use super::*;

// Issue #89: when the range end was i64::MAX, `i += 1` used to overflow —
// now it stops after the last element.
#[test]
fn range_i64_max_chegarasida_toxtaydi() {
    run(r#"
m = 9223372036854775806
r = m..(m + 1)
(r.len == 2) | (fail "range length wrong: ${r.len}")
"#);
}

#[test]
fn fib_recursion() {
    run(r#"
fn fib n
  if n < 2
    ret n
  (fib (n - 1)) + (fib (n - 2))

each i in 0..10
  log "fib ${i} = ${fib i}"
"#);
}

// Issue #99: `..` binds LOWER than arithmetic, but HIGHER than pipe/comparison.
// `1..n+1` = `1..(n+1)` (natural for AI), it used to be `(1..n)+1` and gave a
// runtime error. Pipe, on the other hand, wraps the whole range.
#[test]
fn range_ustuvorligi() {
    run(r#"
n = 3
# end side: +1 applies only to n, not the whole range
(1..n+1 == [1 2 3 4]) | (fail "1..n+1 wrong")
# end side: -1
(0..n-1 == [0 1 2]) | (fail "0..n-1 wrong")
# arithmetic on both sides
(2*1..2+1 == [2 3]) | (fail "2*1..2+1 wrong")
# works inside an each loop too, without error
sum <- 0
each i in 1..n+1
  sum <- sum + i
(sum == 10) | (fail "each 1..n+1 sum wrong: ${sum}")
"#);
}

// Issue #99 (review): pipe binds LOWER than range, so
// `1..3 |> f` = `(1..3) |> f` — the built range is passed to f, without parens.
#[test]
fn range_pipe_butun_diapazonni_uzatadi() {
    run(r#"
fn total xs
  xs.reduce 0 \acc x -> acc + x
# pipe applies to the whole range (1..3 = [1 2 3]), not the end side
(1..3 |> total == 6) | (fail "pipe range wrong")
"#);
}

// Inline if (ternary equivalent): `if cond a else b` returns a single value.
// Issue #66 — a compact conditional expression (for places like leading-zero formatting).
#[test]
fn inline_if_ifoda() {
    run(r#"
# the main example from the issue: leading-zero formatting
h = 5
pad = if h < 10 ("0" + str.str h) else (str.str h)
(pad == "05") | (fail "inline if value did not give: ${pad}")

# else branch when the condition is false
x = 20
pad2 = if x < 10 ("0" + str.str x) else (str.str x)
(pad2 == "20") | (fail "else branch did not work: ${pad2}")

# simple branches without parens
y = if h > 3 "big" else "small"
(y == "big") | (fail "branch without parens did not work: ${y}")

# else-if chain (nested inline if)
g = if h == 0 "zero" else if h < 0 "negative" else "positive"
(g == "positive") | (fail "else-if chain did not work: ${g}")

# a call as the condition, inside parens
s = "hi"
r = if (str.len s) > 0 "full" else "empty"
(r == "full") | (fail "parenthesized condition did not work: ${r}")

# using it inside a larger expression
n = 7
msg = "son " + (if n % 2 == 0 "juft" else "toq")
(msg == "son toq") | (fail "inner inline if did not work: ${msg}")
"#);
}
