use super::*;

// issue #57: when a symbol turns into TEXT the `:` prefix is dropped
// (interpolation, str.str, `+` concatenation). The symbol literal syntax
// (`:florist`) is unchanged — only the text representation is without `:`.
#[test]
fn sym_to_text_colon_tashlanadi() {
    run(r#"
s = :florist
# interpolation
(("v/${s}") == "v/florist") | (fail "interpolation: ${"v/${s}"}")
# str.str
((str.str s) == "florist") | (fail "str.str: ${str.str s}")
# `+` concatenation (both sides)
(("p/" + s) == "p/florist") | (fail "left + : ${"p/" + s}")
((s + "/q") == "florist/q") | (fail "right + : ${s + "/q"}")
# symbol literal and comparison are UNCHANGED
(s == :florist) | (fail "symbol comparison broke")
"#);
}

// INSIDE a list/map a symbol KEEPS the `:` prefix — there a symbol must be
// distinguishable from a string (repr differs from the text representation).
#[test]
fn sym_repr_listda_colon_saqlaydi() {
    run(r#"
xs = [:a "b"]
((str.str xs) == "[:a \"b\"]") | (fail "list repr: ${str.str xs}")
"#);
}
