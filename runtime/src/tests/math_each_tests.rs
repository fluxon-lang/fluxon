use super::*;

// Issue #128: math.min/max/pow/sqrt — a check through the .fx surface.
#[test]
fn math_min_max_pow_sqrt() {
    run(r#"
(math.min 3 7 == 3) | (fail "min wrong")
(math.max 3 7 == 7) | (fail "max wrong")
(math.min 3 2.5 == 2.5) | (fail "mixed int/float min wrong")
(math.pow 2 10 == 1024) | (fail "pow wrong")
(math.sqrt 9 == 3.0) | (fail "sqrt wrong")
"#);
}

// `each i in inf` — an infinite loop. `stop` exits it, `i` increases from 0.
// For the REPL/event-loop (issue #27): the model used to resort to the 1..1000
// trick; now there is a natural infinite repeat.
#[test]
fn each_inf_stop_va_hisoblagich() {
    run(r#"
sum <- 0
each i in inf
  if i == 5
    stop
  sum <- sum + i
(sum == 10) | (fail "0+1+2+3+4 = 10 should be: ${sum}")
"#);
}

// `skip` in an infinite loop moves to the next iteration (i still increases).
#[test]
fn each_inf_skip() {
    run(r#"
cnt <- 0
each i in inf
  if i >= 10
    stop
  if i % 2 == 0
    skip
  cnt <- cnt + 1
(cnt == 5) | (fail "odd sonlar 1,3,5,7,9 = 5 ta: ${cnt}")
"#);
}

// inf cannot be used as a value — only in `each i in inf`.
#[test]
fn inf_qiymat_sifatida_xato() {
    let err = run_source("x = inf\n").expect_err("inf as a value should error");
    assert!(err.contains("inf"), "unexpected error: {}", err);
}

// `each k, v in inf` — two variables are meaningless (a plain infinite counter).
#[test]
fn each_inf_ikki_ozgaruvchi_xato() {
    let err =
        run_source("each k, v in inf\n  stop\n").expect_err("two variables with inf should error");
    assert!(
        err.contains("a single variable"),
        "unexpected error: {}",
        err
    );
}

// --- `fluxon check` (parse only, issue #55) ---
