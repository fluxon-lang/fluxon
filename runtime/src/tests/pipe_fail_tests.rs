use super::*;

#[test]
fn fail_as_expr_and_guard() {
    // fail in an expression context (guard) — breaks the flow, propagates upward.
    let err = run_source(
        r#"
fn check x
  x > 0 | (fail 422 "must be positive")
  "ok"
log (check 5)
log (check 0)
"#,
    )
    .unwrap_err();
    assert!(err.contains("422"), "expected 422, got: {}", err);
}

#[test]
fn pipe_and_coalesce() {
    run(r#"
fn inc x -> x + 1
fn sq x -> x * x
r = 3 |> inc |> sq
log "r=${r}"
m = {a:1}
log "missing=${m.b ?? "none"}"
"#);
}

// Multi-line pipe: if a line starts with `|>`, it continues the previous
// expression (builder-chain readability, issue #78). Only `|>` — not `|` (Or).
#[test]
fn multiline_pipe_continuation() {
    run(r#"
fn inc x -> x + 1
fn dbl x -> x * 2
# stages on new lines, leading |>
r = 5
  |> inc
  |> dbl
  |> inc
(r == 13) | (fail "multi-line pipe wrong: ${r}")
# continues across a comment and a blank line too
r2 = 10
  |> inc

  # a comment here
  |> dbl
(r2 == 22) | (fail "pipe continuation through comment/empty line broke: ${r2}")
"#);
}

// Pipe partial application: `x |> f a b` => `f a b x` (lhs is the LAST argument).
// Drives the builder/chain pattern. An argument-less function value and an
// argument-less module call (`|> str.up`) keep the old behavior.
#[test]
fn pipe_partial_application() {
    run(r#"
fn addto base n -> base + n
# call with arguments: lhs is appended as the last argument
(5 |> addto 100) == 105 | (fail "pipe call with arguments did not work")
# chain
(3 |> addto 10 |> addto 100) == 113 | (fail "pipe zanjir did not work")
# argument-less module call (old behavior must be preserved)
("hello" |> str.up) == "HELLO" | (fail "pipe argumentsiz modul chaqiruvi broke")
# lambda (old behavior)
(5 |> \n -> n * 2) == 10 | (fail "pipe lambda broke")
"#);
}

// --- db battery tests (in-memory SQLite, a separate DB per Interp) ---
