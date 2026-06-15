use super::*;

// rep's optional 3rd-argument headers map (issue #16). rep simply returns a
// {__resp:true status body headers} map — we read and check its keys in Fluxon
// (actual header writing is in the http_mod tests).
#[test]
fn rep_headers_argumenti() {
    run(r#"
# 2-argument (old form) — no headers key
r = rep 200 {ok:true}
(r.status == 200) | (fail "rep status broke: ${r.status}")
(r.headers == nil) | (fail "headers key appeared in rep without headers")

# 3-argument — a headers map is added. Use `_` instead of a dash (a map key
# cannot contain a dash; on write the runtime turns `_` into `-`). Read with `_` too.
r2 = rep 200 "<h1>Salom</h1>" {content_type:"text/html"}
(r2.headers.content_type == "text/html") | (fail "headers could not be read")

# body map + separate headers — they do not collide
r3 = rep 200 {data:1} {set_cookie:"s=abc"}
(r3.body.data == 1) | (fail "body map broke")
(r3.headers.set_cookie == "s=abc") | (fail "set-cookie could not be read")
"#);
}

// If the 3rd argument is not a map, rep gives a clear error (not silent disregard).
#[test]
fn rep_headers_nomap_xato() {
    let e = run_source(r#"x = rep 200 "body" "notmap""#).unwrap_err();
    assert!(
        e.contains("3rd argument must be headers"),
        "unexpected error: {}",
        e
    );
}

// Issue #173: a bare `rep ...` statement short-circuits the enclosing function
// like `ret`. A guard clause must stop execution — the FIRST rep on the taken
// path wins, not the last rep in the body.
#[test]
fn rep_guard_clause_short_circuit() {
    run(r#"
fn handler ->
  if true
    rep 200 {a:1}
  rep 200 {a:2}
r = handler()
(r.status == 200) | (fail "status broke: ${r.status}")
# Old (buggy) behavior returned {a:2}; the guard's rep must win now.
(r.body.a == 1) | (fail "rep did not short-circuit, got ${r.body.a}")
"#);
}

// Issue #173: `ret rep ...` (the old workaround) keeps working unchanged.
#[test]
fn rep_ret_rep_baribir_ishlaydi() {
    run(r#"
fn handler ->
  if true
    ret rep 200 {a:1}
  rep 200 {a:2}
r = handler()
(r.body.a == 1) | (fail "ret rep broke, got ${r.body.a}")
"#);
}

// Issue #173: `rep` in EXPRESSION position (assignment RHS) is still just a
// value — it does not short-circuit, so a response can be built and inspected.
#[test]
fn rep_expr_pozitsiyada_qiymat() {
    run(r#"
fn build ->
  r = rep 200 {a:1}
  r.body.a + 10
v = build()
(v == 11) | (fail "rep in expr position short-circuited, got ${v}")
"#);
}

// Issue #173 (PR review): the short-circuit must NOT fire when a user binding
// shadows the builtin `rep`. Only the BUILTIN `rep` returns early — a user fn
// named `rep` keeps normal call semantics so the body runs to completion.
#[test]
fn rep_shadow_qilingan_short_circuit_qilmaydi() {
    run(r#"
fn f ->
  rep = \x -> x
  rep 1
  99
(f() == 99) | (fail "shadowed rep short-circuited, got ${f()}")
"#);
}

// Issue #173 (PR review): a `rep` that is the tail of an `if`/`match` branch
// used as a VALUE (assignment RHS) must NOT short-circuit — it stays a value
// so code after the assignment still runs. Only a value-DISCARDED `rep`
// (a guard) returns early.
#[test]
fn rep_if_branch_qiymat_pozitsiyada_short_circuit_qilmaydi() {
    run(r#"
fn handler ->
  resp = if true
    rep 200 {a:1}
  else
    rep 404 {a:2}
  # This line must still run — the `rep` above is the value of `resp`, not a return.
  marker = 1
  resp.body.a + marker
v = handler()
(v == 2) | (fail "rep in value-position if-branch short-circuited, got ${v}")
"#);
}

// Issue #173 (PR review): the guard case still short-circuits even when the
// `if` is in statement position — the value is discarded so the branch `rep`
// returns from the function (regression guard alongside the value-position test).
#[test]
fn rep_guard_if_statement_pozitsiya_short_circuit() {
    run(r#"
fn handler ->
  if true
    rep 200 {a:1}
  log.info "should NOT run"
  rep 200 {a:2}
r = handler()
(r.body.a == 1) | (fail "guard rep did not short-circuit, got ${r.body.a}")
"#);
}

// Even after the inline form was added, the block form (with a call condition)
// must still work — regression check.
#[test]
fn blok_if_inline_qoshilgach_ishlaydi() {
    run(r#"
s = "hi"
out <- "none"
if str.len s > 0
  out <- "full"
else
  out <- "empty"
(out == "full") | (fail "block if broke: ${out}")
"#);
}
