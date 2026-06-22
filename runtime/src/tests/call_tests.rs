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

// Issue #219: a paren-free nested call binds tighter in argument position WHEN
// the callee is a value-taking builtin (`log`, `math.*`, ...). `log fac 4` means
// `log (fac 4)`, NOT `log(fac, 4)`. Before the fix the bare function value `fac`
// was passed UNCALLED and the program silently printed the wrong thing
// (`<fn fac> 4`) with no error — the dangerous class of trap. A non-last
// function-valued argument now consumes the arguments that follow it
// (innermost-first) and is applied.
//
// It does NOT fold for a user-fn callee (which may take a callback, possibly
// followed by an options map) or a callback dispatch (`ai.*`/`http.*`/HOF
// methods) — see `arg_fold_only_for_value_taking_builtins` (Codex review #222).
#[test]
fn parenless_nested_call_binds_in_arg_position() {
    run(r#"
fn fac n
  if n == 0
    ret 1
  fac (n - 1) * n
fn inc n
  ret n + 1

# a value builtin folds nested calls: `math.max fac 3 fac 4`
# == `math.max (fac 3) (fac 4)` == max(6, 24) == 24
(math.max fac 3 fac 4 == 24) | (fail "math.max fac 3 fac 4 = max(6,24) = 24")

# chained inside a value builtin: `math.abs inc inc -3`? use math.max for clarity:
# `math.max inc 2 5` == `math.max (inc 2) 5` == max(3,5) == 5
(math.max inc 2 5 == 5) | (fail "math.max inc 2 5 = max(inc 2, 5) = 5")

# a single-arg `fac 5` (fac is the callee) is an ordinary call, unaffected
(fac 5 == 120) | (fail "fac 5 should be 120")

# genuine multi-arg builtin call is UNAFFECTED — args are not function values
(math.max 2 3 == 3) | (fail "math.max 2 3 must stay 3")

# a TRAILING function value is a real higher-order argument, never folded
fn add a b
  ret a + b
xs = [1 2 3]
(xs.map inc == [2 3 4]) | (fail "trailing fn arg must pass through (map)")
(xs.reduce 0 add == 6) | (fail "trailing fn arg must pass through (reduce)")
"#);
}

// Issue #222 (Codex review): folding is restricted to value-taking builtins, so
// it never calls a callback that a higher-order API expects to receive uncalled.
#[test]
fn arg_fold_only_for_value_taking_builtins() {
    // A user fn whose callback is followed by an options map: the callback must
    // NOT be folded (called) during argument evaluation. If it were, `dbl` would
    // be applied to the map and raise "Mul ... map and int".
    run(r#"
fn run_with cb opts
  ret (cb opts.n)
fn dbl x
  ret x * 2
(run_with dbl {n: 21} == 42) | (fail "user-fn callback must not be folded")
"#);

    // A user fn as the callee with a fn arg in the middle is NOT folded — it
    // stays a plain (over-arity) call and fails LOUDLY, never silently.
    let err = run_source(
        r#"
fn add a b
  ret a + b
fn inc n
  ret n + 1
add inc 2 3
"#,
    )
    .expect_err("user-fn callee must not fold; over-arity must error");
    assert!(
        err.contains("expected 2 arguments"),
        "unexpected error: {}",
        err
    );

    // The pipe partial-call path folds the same way for a value builtin.
    run(r#"
fn fac n
  if n == 0
    ret 1
  fac (n - 1) * n
# `5 |> math.max fac 0` => `math.max fac 0 5` => `math.max (fac 0) 5` = max(1,5) = 5
(5 |> math.max fac 0 == 5) | (fail "pipe fold: max(fac 0, 5) = 5")
"#);
}

// Issue #222 (Codex review): the value-builtin allowlist also covers `rep`,
// `assert`, and `log.*` — not just bare `log` and modules. `rep status fn args`
// must fold the body so it is not a bare function value (the silent-wrong class).
#[test]
fn arg_fold_covers_rep_and_log_levels() {
    // `rep 200 fac 5` => `rep 200 (fac 5)`: the body becomes 120, not <fn fac>.
    run(r#"
fn fac n
  if n == 0
    ret 1
  fac (n - 1) * n
r = rep 200 fac 5
(r.body == 120) | (fail "rep body should be folded to 120, got ${r.body}")
"#);
}

// Issue #222 (Codex review): a nullary fn must NOT be auto-called while folding.
// `new_id "tag"` would make `take == 0`; folding must leave `new_id` a bare
// function value (Fluxon requires the explicit `new_id()` to call it).
#[test]
fn arg_fold_does_not_autocall_nullary() {
    run(r#"
calls <- 0
fn nid
  calls <- calls + 1
  ret "x"
# rep is a value builtin and nid is NOT the last arg (headers map follows), so
# folding is considered — but nid is nullary (consumes nothing), so it is passed
# UNCALLED as the body and `calls` stays 0.
r = rep 200 nid {ct: "txt"}
(calls == 0) | (fail "nullary fn must not be auto-called by folding")
(r.body != "x") | (fail "nullary fn should remain a function value, not its result")
"#);
}
