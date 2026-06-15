use super::*;

// Issue #174: ordering operators (`<` `>` `<=` `>=`) compare strings
// lexicographically (Unicode code point order). Previously errored with
// "Lt operator cannot be applied to str and str".
#[test]
fn str_ordering_operatorlari() {
    run(r#"
(("a" < "b") == true) | (fail "a < b should be true")
(("b" < "a") == false) | (fail "b < a should be false")
(("a" <= "a") == true) | (fail "a <= a should be true")
(("b" > "a") == true) | (fail "b > a should be true")
(("a" >= "b") == false) | (fail "a >= b should be false")
# prefix is less than the longer string
(("ab" < "abc") == true) | (fail "ab < abc should be true")
# timestamps as strings sort correctly (the booking-API use case)
(("2026-09-01 10:00:00" < "2026-09-01 11:00:00") == true) | (fail "earlier timestamp should be less")
"#);
}

// Issue #174: str.cmp a b -> -1 | 0 | 1 — canonical three-way compare.
#[test]
fn str_cmp_uch_tomonlama() {
    run(r#"
((str.cmp "a" "b") == -1) | (fail "cmp a b should be -1")
((str.cmp "b" "a") == 1) | (fail "cmp b a should be 1")
((str.cmp "a" "a") == 0) | (fail "cmp a a should be 0")
"#);
}
