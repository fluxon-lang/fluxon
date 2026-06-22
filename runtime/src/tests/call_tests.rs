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
# CRITICAL: a unary in ARGUMENT position binds only its atom — it must NOT
# swallow the following argument (regression caught in PR #214 review). Here
# `!ok` is one arg and "msg" is a separate arg.
fn two a b
  ret "${a}|${b}"
ok = false
(two !ok "msg" == "true|msg") | (fail "unary arg must not swallow next arg")
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

// Issue #219: a paren-free nested call binds tighter in argument position.
// `log fac 4` means `log (fac 4)`, NOT `log(fac, 4)`. Before the fix the bare
// function value `fac` was passed UNCALLED and the program silently printed the
// wrong thing (`<fn fac> 4`) with no error — the dangerous class of trap. Now a
// non-last function-valued argument consumes the arguments that follow it
// (innermost-first) and is applied.
#[test]
fn parenless_nested_call_binds_in_arg_position() {
    run(r#"
fn fac n
  if n == 0
    ret 1
  fac (n - 1) * n
fn inc n
  ret n + 1
fn add a b
  ret a + b

# the exact issue example: `log fac 5` == `log (fac 5)` == 120, no bare fn value
(fac 5 == 120) | (fail "fac 5 should be 120")

# a user fn consumes exactly its own arity, leaving the rest for the outer call
(add inc 2 3 == 6) | (fail "add inc 2 3 should be add (inc 2) 3 = 6")

# chained: `inc inc 2` == `inc (inc 2)` == 4
(inc inc 2 == 4) | (fail "inc inc 2 should be 4")

# genuine multi-arg calls are UNAFFECTED — `2` is not a function value
(add 2 3 == 5) | (fail "add 2 3 must stay 5")

# a TRAILING function value is a real higher-order argument, never folded
xs = [1 2 3]
(xs.map inc == [2 3 4]) | (fail "trailing fn arg must pass through (map)")
(xs.reduce 0 add == 6) | (fail "trailing fn arg must pass through (reduce)")
"#);
}
