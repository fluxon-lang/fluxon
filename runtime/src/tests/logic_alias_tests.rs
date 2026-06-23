use super::*;

// Issue #216: `&&` and `||` are accepted as exact aliases for `&` and `|`
// (logical AND / OR). Small models reach for the C/JS/Go doubled form
// reflexively; the canonical Fluxon form stays single `&`/`|`.

#[test]
fn double_amp_is_logical_and() {
    run(r#"
(true && true) | (fail "&& should be true")
((false && (fail "must short-circuit")) == false) | (fail "&& false broke")
# Mixed with the canonical single form — same precedence, same result.
((1 == 1 && 2 == 2) == (1 == 1 & 2 == 2)) | (fail "&& != &")
"#);
}

#[test]
fn double_pipe_is_logical_or() {
    run(r#"
(false || true) | (fail "|| should be true")
(true || (fail "must short-circuit")) | (fail "|| true broke")
((false || false) == false) | (fail "|| false broke")
# The `cond || (fail ..)` guard form works just like `cond | (fail ..)`.
(1 == 1) || (fail "|| guard broke")
"#);
}

// The original FizzBuzz from the issue: `&&` inside an `if` condition.
#[test]
fn fizzbuzz_with_double_amp() {
    run(r#"
out = []
each n in 1..20
  if n % 3 == 0 && n % 5 == 0
    out <- out.push "FizzBuzz"
  elif n % 3 == 0
    out <- out.push "Fizz"
  elif n % 5 == 0
    out <- out.push "Buzz"
  else
    out <- out.push (str.str n)
(out[14] == "FizzBuzz") | (fail "15 should be FizzBuzz, got ${out[14]}")
(out[2] == "Fizz") | (fail "3 should be Fizz, got ${out[2]}")
(out[4] == "Buzz") | (fail "5 should be Buzz, got ${out[4]}")
(out[0] == "1") | (fail "1 should be 1, got ${out[0]}")
"#);
}
