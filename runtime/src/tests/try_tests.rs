use super::*;

// Issue #93: in `log !x` the `!` used to stick to the callee as a postfix Try —
// `Call(Try(log), [x])` — the negation silently disappeared. Now a `!` after
// whitespace starts an argument as a prefix not.
#[test]
fn chaqiruv_argumentida_prefiks_not() {
    run(r#"
x = false
(!x) | (fail "parenthesized prefix not broke")
fn id v -> v
((id !x) == true) | (fail "in call argument !x was not negated")
y = true
((id !y) == false) | (fail "in call argument !y was not negated")
fn second a b -> b
((second x !y) == false) | (fail "prefix not in the second argument broke")
"#);
}

// Issue #93 (regression guard): an attached `!` is still a postfix Try as before —
// it sticks to the value and stays a passthrough on success.
#[test]
fn tutash_bang_postfix_try_qoladi() {
    run(r#"
fn safe v -> v
a = (safe 5)!
(a == 5) | (fail "postfix try passthrough broke")
"#);
}

// Issue #125: try/catch — catches an error raised by `fail` and continues
// as a value. The catch variable binds to a {message, status} map.
#[test]
fn try_catch_fail_statusli_ushlaydi() {
    run(r#"
r = try
  fail 422 "invalid data"
catch e
  (e.message == "invalid data") | (fail "catch message broke")
  (e.status == 422) | (fail "catch status broke")
  "fallback"
(r == "fallback") | (fail "catch body value did not return")
"#);
}

// Status-less fail and a runtime error — both are caught; without a status
// e.status is nil.
#[test]
fn try_catch_runtime_xato_va_statussiz() {
    run(r#"
r = try
  fail "boom"
catch e
  (e.status == nil) | (fail "status should be nil for fail without status should be")
  e.message
(r == "boom") | (fail "fail message without status was not caught")

# runtime errors (divide by zero) are caught too
r2 = try
  1 / 0
catch e
  (e.status == nil) | (fail "runtime error status should be nil should be")
  "ushlandi"
(r2 == "ushlandi") | (fail "runtime error was not caught")
"#);
}

// On success the body's last expression returns; catch does not run.
#[test]
fn try_catch_muvaffaqiyatda_body_qiymati() {
    run(r#"
r = try
  40 + 2
catch
  0
(r == 42) | (fail "body value on success did not return")
"#);
}

// ret/skip/stop flow signals pass through try — catch does not catch them.
#[test]
fn try_catch_oqim_signallarini_ushlamaydi() {
    run(r#"
fn f
  try
    ret "early"
  catch
    ret "caught"
((f()) == "early") | (fail "ret from inside try was caught (wrong)")

total <- 0
each i in 1..5
  try
    if i == 3
      skip
    if i == 5
      stop
    total <- total + i
  catch
    fail "skip/stop should not be caught"
(total == 7) | (fail "skip/stop try ichida broke: ${total}")
"#);
}

// Nested try, and a re-fail (re-raise) from inside catch goes to the outer try.
#[test]
fn try_catch_ichmaich_va_qayta_fail() {
    run(r#"
r = try
  try
    fail "inner"
  catch e
    fail "outer: ${e.message}"
catch e
  e.message
(r == "outer: inner") | (fail "nested try or re-fail broke")
"#);
}

// Issue #90: infinite recursion must return a graceful runtime error instead
// of a stack overflow ABORT (so an HTTP handler does not kill the whole server).
#[test]
fn cheksiz_rekursiya_graceful_xato() {
    let e = run_source("fn f n -> f (n + 1)\nf 0").unwrap_err();
    assert!(e.contains("recursion too deep"), "unexpected error: {}", e);
}

// Issue #90: after a limit error the depth counter fully resets —
// the next execution on the same thread starts clean (RAII guard).
#[test]
fn rekursiya_limitdan_keyin_tiklanish() {
    assert!(run_source("fn f n -> f (n + 1)\nf 0").is_err());
    run(r#"
fn g x -> x + 1
((g 1) == 2) | (fail "call after the limit broke")
"#);
}

// Issue #90: ~2000 nested parens used to abort the parser with a stack overflow.
// Now exceeding the limit (256) is a clear parse error; 200 levels still work.
#[test]
fn chuqur_qavs_parse_limiti() {
    let deep = format!("x = {}1{}", "(".repeat(300), ")".repeat(300));
    let e = check_source(&deep).unwrap_err();
    assert!(e.contains("too deep"), "unexpected error: {}", e);

    let ok = format!("x = {}1{}", "(".repeat(200), ")".repeat(200));
    check_source(&ok).unwrap_or_else(|e| panic!("200 levels should pass: {}", e));
}

// Issue #89: on int arithmetic overflow, instead of a panic (debug) / silent
// wrap (release), both modes return the same Fluxon error.
#[test]
fn int_overflow_xato_panic_emas() {
    // + overflow (used to panic in debug)
    let e = run_source("log (9223372036854775806 + 2)").unwrap_err();
    assert!(e.contains("number out of range"), "unexpected error: {}", e);
    // i64::MIN / -1 — used to panic even in release in Rust
    let e = run_source(
        r#"
a = 0 - 9223372036854775807 - 1
log (a / (0 - 1))
"#,
    )
    .unwrap_err();
    assert!(e.contains("number out of range"), "unexpected error: {}", e);
    // i64::MIN % -1 — same family
    let e = run_source(
        r#"
a = 0 - 9223372036854775807 - 1
log (a % (0 - 1))
"#,
    )
    .unwrap_err();
    assert!(e.contains("number out of range"), "unexpected error: {}", e);
    // unary minus too: -(i64::MIN) does not fit
    let e = run_source(
        r#"
a = 0 - 9223372036854775807 - 1
log (-a)
"#,
    )
    .unwrap_err();
    assert!(e.contains("number out of range"), "unexpected error: {}", e);
    // * and - are checked too
    assert!(run_source("log (4611686018427387904 * 2)").is_err());
    assert!(run_source("log (0 - 9223372036854775807 - 2)").is_err());
    // Ordinary arithmetic works as before
    run(r#"
((2 + 3) == 5) | (fail "sum broke")
((7 / 2) == 3) | (fail "division broke")
((7 % 2) == 1) | (fail "mod broke")
((-(5)) == (0 - 5)) | (fail "unary minus broke")
"#);
}
