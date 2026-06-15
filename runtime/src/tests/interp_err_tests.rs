use super::*;

// Issue #106: a parse error inside string interpolation must point to the
// original line (not collapse to "on line 1") and must arrive with the
// "inside interpolation:" prefix.
#[test]
fn interp_parse_xatosi_asl_qatorni_korsatadi() {
    let err = run_source("log \"a\"\nlog \"b\"\nlog \"c\"\nlog \"d\"\nlog \"${x +}\"\n")
        .expect_err("a broken interpolation expression should error");
    assert!(
        err.contains("on line 5"),
        "error should point to the original line (5), got: {}",
        err
    );
    assert!(
        err.contains("inside interpolation"),
        "the parse error should also carry the 'inside interpolation' prefix, got: {}",
        err
    );
}

// Issue #106: a lex error also preserves the original line. A multi-line
// expression does not break the line count either — the inner string opens on line 3.
#[test]
fn interp_lex_xatosi_asl_qatorni_korsatadi() {
    let err = run_source("log \"a\"\nlog \"b\"\nlog \"v=${\"x\ny\"}\"\n")
        .expect_err("a multi-line inner string should error");
    assert!(
        err.contains("inside interpolation") && err.contains("on line 3"),
        "the lex error should point to the original line (3), got: {}",
        err
    );
}

// Issue #106: the ${...} boundary accounts for inner string literals —
// a `}` inside a string does not close the interpolation early.
#[test]
fn interp_ichki_string_qavsni_yopmaydi() {
    run(r#"
x = "v: ${"inner } brace"}"
(x == "v: inner } brace") | (fail "inner string brace wrong ishlandi: ${x}")
"#);
}

// Issue #106: an escaped quote (\") inside an inner string does not close the
// string, and the `}` after it does not close the interpolation either.
#[test]
fn interp_ichki_string_escape_tirnoq() {
    run(r#"
x = "x=${"a\"}b"}"
(x == "x=a\"}b") | (fail "escaped quote wrong ishlandi: ${x}")
"#);
}
