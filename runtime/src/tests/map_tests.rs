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
