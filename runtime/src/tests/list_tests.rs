use super::*;

#[test]
fn list_methods() {
    run(r#"
nums = [1 2 3 4 5]
evens = nums.filter \x -> x % 2 == 0
doubled = evens.map \x -> x * 2
total = doubled.reduce 0 \acc x -> acc + x
log "evens=${evens} doubled=${doubled} total=${total}"
"#);
}

// list.index gives the position (-1 if not found), list.find gives the first
// element matching the predicate (nil if not found). has is bool, index is the
// position — a pair.
#[test]
fn list_index_and_find() {
    run(r#"
names = ["catalog_manager" "order_extractor" "billing"]
(names.index "order_extractor" == 1) | (fail "index did not find: ${names.index "order_extractor"}")
(names.index "yoq" == -1) | (fail "none element -1 did not give")

nums = [3 1 4 1 5 9]
(nums.index 4 == 2) | (fail "int index: ${nums.index 4}")

# find: the first element matching the predicate
big = nums.find \x -> x > 4
(big == 5) | (fail "find did not return the matching element: ${big}")
none = nums.find \x -> x > 99
(none == nil) | (fail "find should return nil when nothing matches: ${none}")

# using index for comparison (issue source: block order)
a = names.index "catalog_manager"
b = names.index "billing"
(a < b) | (fail "index comparison did not work: ${a} ${b}")
"#);
}

// Issue #127: list.sort — natural order without an argument (number/string),
// arbitrary order with a comparator. The original list is unchanged (immutable values).
#[test]
fn list_sort() {
    run(r#"
nums = [3 1 4 1 5]
s = nums.sort
(s == [1 1 3 4 5]) | (fail "natural sort: ${s}")
(nums == [3 1 4 1 5]) | (fail "sort modified the original list: ${nums}")

# comparator: returns a number (negative: a first) — descending order
d = nums.sort \a b -> b - a
(d == [5 4 3 1 1]) | (fail "comparator sort: ${d}")

# strings sort lexicographically
names = ["banan" "olma" "anor"].sort
(names == ["anor" "banan" "olma"]) | (fail "str sort: ${names}")

# mixed int/flt numeric order
mixed = [2 1.5 1].sort
(mixed == [1 1.5 2]) | (fail "mixed number sort: ${mixed}")

# edge cases
([].sort == []) | (fail "empty list sort")
([7].sort == [7]) | (fail "single element sort")
"#);
}

// Issue #127: sort with a comparator is stable — equal elements keep their
// original order (sorting map records gathered from several sources by a field).
#[test]
fn list_sort_stable_va_maplar() {
    run(r#"
items = [{n:"b" p:2} {n:"a" p:1} {n:"c" p:1}]
sorted = items.sort \a b -> a.p - b.p
ns = sorted.map \x -> x.n
(ns == ["a" "c" "b"]) | (fail "stable map sort: ${ns}")
"#);
}

// Issue #127: sort error paths — mixed types without a comparator, a comparator
// that does not return a number, a zip argument that is not a list.
#[test]
fn list_sort_zip_xatolari() {
    let e = run_source(r#"x = [1 "a"].sort"#).unwrap_err();
    assert!(e.contains("cannot compare"), "unexpected error: {}", e);

    let e = run_source(r#"x = [1 2].sort \a b -> "x""#).unwrap_err();
    assert!(
        e.contains("must return a number"),
        "unexpected error: {}",
        e
    );

    let e = run_source("x = [1 2].zip 5").unwrap_err();
    assert!(e.contains("must be a list"), "unexpected error: {}", e);
}

// Issue #127: reverse/uniq/flat/zip — pure list methods.
#[test]
fn list_reverse_uniq_flat_zip() {
    run(r#"
([1 2 3].reverse == [3 2 1]) | (fail "reverse did not work")
([1 2 1 3 2].uniq == [1 2 3]) | (fail "uniq did not work")

# flat flattens one level; a non-list element stays as-is
([[1 2] [3] 4].flat == [1 2 3 4]) | (fail "flat did not work")

# zip stops when the shorter one runs out
z = [1 2 3].zip ["a" "b"]
(z == [[1 "a"] [2 "b"]]) | (fail "zip did not work: ${z}")
"#);
}

// Issue #127: any/all predicate methods — instead of the filter+len workaround.
#[test]
fn list_any_all() {
    run(r#"
nums = [1 2 3]
a1 = nums.any \x -> x > 2
a1 | (fail "any did not return true on a match")
a2 = nums.any \x -> x > 9
(a2 == false) | (fail "any did not return false without a match")

b1 = nums.all \x -> x > 0
b1 | (fail "all did not return true when all match")
b2 = nums.all \x -> x > 1
(b2 == false) | (fail "all did not return false on a mismatch")

# empty list: any false, all true (vacuous)
e1 = [].any \x -> x
(e1 == false) | (fail "empty any false not")
e2 = [].all \x -> x
e2 | (fail "empty all true not")
"#);
}

// Computed index: both `xs.(expr)` and `xs[expr]` must work.
// Issue #64 — an expression index, not a literal, for pagination/getting the last element.
#[test]
fn hisoblangan_indeks() {
    run(r#"
xs = ["a" "b" "c"]
i = xs.len - 1

# .(expr) form — get the last element with a computed index
last = xs.(i)
(last == "c") | (fail ".(i) oxirgi elementni did not give: ${last}")

# a full expression inside
(xs.(xs.len - 1) == "c") | (fail "xs.(xs.len - 1) did not work")

# the bracket form gives the same result
(xs[i] == "c") | (fail "xs[i] did not work")

# indexing a map with a computed key (str)
m = {name: "Ali" age: 30}
k = "name"
(m.(k) == "Ali") | (fail "m.(k) did not work: ${m.(k)}")

# out of bounds -> nil (existing get_index behavior)
(xs.(99) == nil) | (fail "chegaradan tashqari indeks nil did not give")
"#);
}
