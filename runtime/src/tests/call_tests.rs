use super::*;

// Argument-less (nullary) call: `f()`. Since a paren-free call is defined by its
// argument, this is the only way to call a 0-arity function.
// `f` (paren-free) is the function VALUE, `f()` is a CALL.
#[test]
fn nullary_call() {
    run(r#"
fn new_id
  ret rand.str 8

a = new_id()
b = new_id()
(str.len a == 8) | (fail "new_id() was not called: ${a}")
(a != b) | (fail "each call did not give a new value")

# paren-free: the function value (not called) — boolean truthy
f = new_id
(f != nil) | (fail "bare name should be a function value")

# nullary lambda
g = \->
  ret 42
(g() == 42) | (fail "lambda nullary call did not work: ${g()}")
"#);
}

// Argument-less recursion: `tick()` calls itself. We used to be forced to add a
// dummy argument (`tick n`) — now it is not required.
#[test]
fn nullary_recursion() {
    run(r#"
n <- 0
fn tick
  n <- n + 1
  if n < 3
    tick()
  ret n
(tick() == 3) | (fail "nullary recursion did not work: ${n}")
"#);
}

// Issue #213: a prefix `!`/`-` binds the WHOLE paren-free call that follows, not
// just its callee. `!str.starts a b` ≡ `!(str.starts a b)` and `-math.max a b` ≡
// `-(math.max a b)`. Before the fix the operator grabbed only the callee, leaving
// the args dangling and raising a misleading "argument 1 is missing". This is the
// trap small models hit when writing a Bearer-token guard the natural way.
#[test]
fn prefix_op_binds_whole_parenless_call() {
    run(r#"
auth_h = "Bearer xyz"
# the natural Bearer guard: `!` in front of a 2-arg call, behind a `|`
if !auth_h | !str.starts auth_h "Bearer "
  fail "should have a bearer"
else
  log "ok"
# standalone, no operator: !str.starts a b
(!str.starts "abc" "x") | (fail "!str.starts should be true")
((!str.starts "abc" "ab") == false) | (fail "!str.starts should be false")
# prefix `-` over a multi-arg call
(-math.max 3 5 == -5) | (fail "-math.max should be -5")
# plain unary still works (no following atom)
b = true
(!b == false) | (fail "plain !b")
(-3 == 0 - 3) | (fail "plain -3")
"#);
}

// `f(x)` (a parenthesized call with an argument) is REJECTED — the canonical form is `f x`.
// Empty `()` is only for nullary; one task = one way.
#[test]
fn paren_call_with_arg_errors() {
    let err = run_source(
        r#"
fn g x
  ret x
g(5)
"#,
    )
    .expect_err("f(x) with parenthesized argument should error");
    assert!(err.contains("argument-less"), "unexpected error: {}", err);
}
