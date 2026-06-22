use super::*;

#[test]
fn map_operations() {
    run(r#"
u = {name:"Aziza" age:30}
u2 = u.set "age" 31
log "keys=${u.keys} hasName=${u.has "name"} age=${u2.age}"
"#);
}

// Issue #129: m.merge other — merges two maps (other wins).
#[test]
fn map_merge() {
    run(r#"
# the main pattern: default config + user override
defaults = {host:"localhost" port:8080 debug:false}
user = {port:3000 debug:true}
cfg = defaults.merge user

# keys from other win
(cfg.port == 3000) | (fail "merge: other key did not win: ${cfg.port}")
(cfg.debug == true) | (fail "merge: debug override did not happen")
# a key not present in other keeps its original value
(cfg.host == "localhost") | (fail "merge: host lost: ${cfg.host}")
(cfg.len == 3) | (fail "merge: key count wrong: ${cfg.len}")

# the original maps are unchanged (consistent with set/del — a new map is returned)
(defaults.port == 8080) | (fail "merge: original map changed")
((user.has "host") == false) | (fail "merge: other map changed")

# merge with an empty map — returns itself
((defaults.merge {}).len == 3) | (fail "merge: with empty map broke")
(({}.merge defaults).port == 8080) | (fail "merge: merge from empty map broke")
"#);
}

// map.merge with a non-map argument returns an understandable error.
#[test]
fn map_merge_notogri_argument() {
    let e = run_source(r#"({a:1}).merge 42"#).unwrap_err();
    assert!(e.contains("map.merge"), "unexpected error text: {}", e);
}

// A bare type name in a schema map's value position (`{a:str b:int}`) turns into
// a sym — as the docs promise (`ai.json {product:str qty:int}`). Because `str` is
// also a module name, it used to give an "unknown name: str" error.
#[test]
fn schema_bare_type_names() {
    run(r#"
schema = {product:str qty:int price:flt active:bool data:json tag:sym}
(schema.product == :str) | (fail "product :str not: ${schema.product}")
(schema.qty == :int) | (fail "qty :int not: ${schema.qty}")
(schema.price == :flt) | (fail "price :flt not")
(schema.active == :bool) | (fail "active :bool not")
(schema.data == :json) | (fail "data :json not")
(schema.tag == :sym) | (fail "tag :sym not")

# a map inside a nested list should work too (`{items:[{product:str qty:int}]}`)
nested = {items:[{product:str qty:int}]}
row = nested.items.0
(row.product == :str) | (fail "nested product :str not")
(row.qty == :int) | (fail "nested qty :int not")

# regression: an ident that is NOT a type name is still resolved as a variable
x = 5
m = {n:x}
(m.n == 5) | (fail "oddiy variable value broke: ${m.n}")

# regression: a str module call as a value is not broken
up = str.up "hello"
(up == "HELLO") | (fail "str.up broke: ${up}")
"#);
}

// Issue #220 — index assignment `m[k] = v` for maps. This is the canonical
// map-write form (every small model reaches for it); it mutates the map held by
// the variable in place, using `=` BIND lookup (function-local, transparent
// through if/each/match) so the accumulator pattern works.
#[test]
fn map_index_assign() {
    run(r#"
cnt = {}
cnt["a"] = 1
cnt["b"] = 2
cnt["a"] = (cnt["a"] ?? 0) + 10
(cnt.a == 11) | (fail "index assign a: ${cnt.a}")
(cnt.b == 2) | (fail "index assign b: ${cnt.b}")
(cnt.len == 2) | (fail "index assign count: ${cnt.len}")
"#);
}

// The canonical accumulator from the issue: counting words into a map inside an
// `each` loop. The `each` block is transparent, so `cnt[w] =` updates the outer
// `cnt` (not a fresh per-iteration local).
#[test]
fn map_index_assign_accumulator() {
    run(r#"
words = ["red" "blue" "red" "green" "blue" "red"]
cnt = {}
each w in words
  cnt[w] = (cnt[w] ?? 0) + 1
(cnt.red == 3) | (fail "red != 3: ${cnt.red}")
(cnt.blue == 2) | (fail "blue != 2: ${cnt.blue}")
(cnt.green == 1) | (fail "green != 1: ${cnt.green}")
"#);
}

// A computed (variable) key works the same as a literal one.
#[test]
fn map_index_assign_computed_key() {
    run(r#"
m = {}
k = "dyn"
m[k] = 42
(m.dyn == 42) | (fail "computed key: ${m.dyn}")
"#);
}

// `m.field = v` — the field (dot) form mutates the same way as `m["field"]`.
#[test]
fn map_field_assign() {
    run(r#"
m = {a:1 b:2}
m.a = 99
(m.a == 99) | (fail "field assign: ${m.a}")
(m.b == 2) | (fail "field assign sibling changed: ${m.b}")
"#);
}

// Writing to a name that does not exist yet auto-creates the map (so `cnt["a"] =
// 1` works without a preceding `cnt = {}`).
#[test]
fn map_index_assign_creates_var() {
    run(r#"
fresh["x"] = 5
(fresh.x == 5) | (fail "auto-create: ${fresh.x}")
"#);
}

// A deep write auto-creates intermediate map levels (`cfg["db"]["port"] = 8080`).
#[test]
fn map_index_assign_nested() {
    run(r#"
cfg = {}
cfg["db"]["port"] = 8080
cfg["db"]["host"] = "localhost"
(cfg.db.port == 8080) | (fail "nested port: ${cfg.db.port}")
(cfg.db.host == "localhost") | (fail "nested host: ${cfg.db.host}")
"#);
}

// Computed keys in a chained write evaluate root→leaf (left→right), matching
// normal `target`-before-`key` index reads. With a side-effecting key generator
// `m[next()][next()] = 1` must build `m.k1.k2`, NOT `m.k2.k1`.
#[test]
fn map_index_assign_key_eval_order() {
    run(r#"
n <- 0
fn next
  n <- n + 1
  ret "k${n}"
m = {}
m[next()][next()] = 1
(m.k1.k2 == 1) | (fail "chained key eval order wrong: ${m}")
((m.has "k2") == false) | (fail "outer key should be k1 not k2: ${m}")
"#);
}

// Maps are value types throughout Fluxon (like `.set`/`.push` returning a new
// value): mutating a map passed into a function does NOT affect the caller's map.
#[test]
fn map_index_assign_is_value_local() {
    run(r#"
m = {n:1}
fn bump x
  x["n"] = 99
  ret x
bumped = bump m
(bumped.n == 99) | (fail "callee did not see write: ${bumped.n}")
(m.n == 1) | (fail "caller map was mutated (should be a value copy): ${m.n}")
"#);
}

// A failed deep write must not leak the empty parent maps it auto-created. Since
// `try/catch` resumes, a half-applied write would corrupt the map. The pre-check
// makes the write all-or-nothing: after a caught error the map is untouched.
#[test]
fn map_index_assign_failed_write_no_leak() {
    run(r#"
m = {a:1}
try
  m["x"]["y"][0] = 1
catch e
  nil
(m.a == 1) | (fail "existing key lost: ${m.a}")
((m.has "x") == false) | (fail "failed write leaked auto-created parent: ${m}")
(m.len == 1) | (fail "map grew on a failed write: ${m}")
"#);
}

// Indexing into a non-collection with a string key is a loud error, not silent.
#[test]
fn map_index_assign_wrong_type_errors() {
    let e = run_source(
        r#"
x = 5
x["k"] = 1
"#,
    )
    .unwrap_err();
    assert!(e.contains("int"), "unexpected error text: {}", e);
}

// Issue #98 — nested numeric index `m.0.1`. The lexer used to greedily swallow
// `.1` as `Flt(0.1)` (not knowing it was in a `.` member context). Now a number
// after a member index does not start a float: `m.0.1` ≡ `(m.0).1`.
#[test]
fn nested_numeric_index() {
    run(r#"
m = [[1 2] [3 4]]
(m.0.1 == 2) | (fail "m.0.1 != 2: ${m.0.1}")
(m.1.0 == 3) | (fail "m.1.0 != 3: ${m.1.0}")

# three-level nested index too
deep = [[[7 8]]]
(deep.0.0.1 == 8) | (fail "deep.0.0.1 != 8: ${deep.0.0.1}")

# regression: ordinary float literals are not broken
(0.5 + 0.5 == 1.0) | (fail "float literal broke")
fs = [0.5 1.5]
(fs.1 == 1.5) | (fail "float element broke: ${fs.1}")
"#);
}
