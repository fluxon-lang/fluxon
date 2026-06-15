use super::*;

// Issue #130: """ block string — the common indentation is stripped, if the
// closing """ is on its own line there is no trailing \n.
#[test]
fn blok_satr_dedent_va_trailing_yoq() {
    run(r#"
s = """
  hello
  world
  """
(s == "hello\nworld") | (fail "block string dedent error: ${s}")
"#);
}

// Issue #130: ${expr} and $ident interpolation work inside a block string.
#[test]
fn blok_satr_interpolatsiya() {
    run(r#"
name = "fluxon"
n = 2
s = """
  hello ${name}!
  n+1 = ${n + 1}
  short: $name
  """
(s == "hello fluxon!\nn+1 = 3\nshort: fluxon") | (fail "block string interpolation error: ${s}")
"#);
}

// Issue #130: an empty line becomes \n, `"` and `""` are free without escaping —
// JSON/HTML snippets are written directly.
#[test]
fn blok_satr_bosh_qator_va_tirnoq() {
    run(r#"
s = """
  a "quoted"

  {"json": true}
  """
(s == "a \"quoted\"\n\n{\"json\": true}") | (fail "block string quote/empty line error: ${s}")
"#);
}

// Issue #130: lines deeper than the minimal indentation keep their relative
// position (so the inner structure of SQL/a prompt is not broken).
#[test]
fn blok_satr_nisbiy_chekinish() {
    run(r#"
s = """
  SELECT *
    FROM t
  """
(s == "SELECT *\n  FROM t") | (fail "relative indentation should be preserved needed: ${s}")
"#);
}

// Issue #130: the closing """ may also come at the end of a content line.
#[test]
fn blok_satr_kontent_qatorida_yopilish() {
    run(r#"
s = """
  one line"""
(s == "one line") | (fail "closing on a content line error: ${s}")
"#);
}

// Issue #130: a block string also works inside an indented block (an fn body) —
// the lines within the string do not emit INDENT/DEDENT.
#[test]
fn blok_satr_fn_ichida() {
    run(r#"
fn f x ->
  s = """
    inner ${x}
    """
  ret s
((f "a") == "inner a") | (fail "block string inside fn error")
"#);
}

// Issue #130: if three consecutive quotes are needed, write \""".
#[test]
fn blok_satr_escape_uchta_tirnoq() {
    run(r#"
s = """
  three: \"""
  """
(s == "three: \"\"\"") | (fail "escape quote error: ${s}")
"#);
}

// Issue #130: text on the same line after the opening """ — a clear error
// (the one canonical way: content starts on a new line).
#[test]
fn blok_satr_ochilishda_matn_xato() {
    let err =
        run_source("s = \"\"\"matn\nx\"\"\"\n").expect_err("text on the opening line should error");
    assert!(err.contains("a new line"), "unexpected error: {}", err);
}

// Issue #130: an unterminated block string gives a clear error (with the opening line).
#[test]
fn blok_satr_yopilmagan_xato() {
    let err =
        run_source("s = \"\"\"\n  abc\n").expect_err("an unterminated block string should error");
    assert!(
        err.contains("unterminated block string") && err.contains("on line 1"),
        "unexpected error: {}",
        err
    );
}
