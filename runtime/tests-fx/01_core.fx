# 01 - Core language: types, binding, functions, control flow, match, operators.
# Each block is compared against the expected result; on mismatch "FAIL" is printed.

fails <- 0

fn eq got want label
  if got == want
    log "ok  ${label} = ${got}"
  else
    log "FAIL ${label}: got=${got} want=${want}"
    fails <- fails + 1

# --- Types ---
eq 42 42 "int"
eq 3.5 3.5 "flt"
eq "hi" "hi" "str"
eq true true "bool"
eq :ok :ok "sym"
eq nil nil "nil"

# --- Binding: local (=) vs reach-out (<-) ---
x = 10
eq x 10 "= local bind"
total <- 0
total <- total + 5
total <- total + 5
eq total 10 "<- reassign"

# --- Operators ---
eq (2 + 3 * 4) 14 "precedence * over +"
eq (10 % 3) 1 "modulo"
eq (10 / 4) 2 "int div"          # integer division
eq ("a" + "b") "ab" "str concat"
eq (5 > 3 & 2 < 4) true "and"
eq (5 < 3 | 2 < 4) true "or"
eq (!false) true "not"
eq (nil ?? "x") "x" "coalesce nil"
eq (7 ?? "x") 7 "coalesce non-nil"

# --- Range ---
eq (1..5) [1 2 3 4 5] "range"

# --- Function: ret, last-expr, one-line, lambda, closure, recursion ---
fn add a b
  ret a + b
eq (add 2 3) 5 "fn ret"

fn last_expr a b
  a * b
eq (last_expr 3 4) 12 "fn last-expr"

fn double x -> x * 2
eq (double 21) 42 "fn one-line"

dbl = \x -> x * 2
eq (dbl 5) 10 "lambda"

# closure - captures the outer variable
fn adder n
  \x -> x + n
add5 = adder 5
eq (add5 10) 15 "closure capture"

fn fib n
  if n < 2
    ret n
  (fib (n - 1)) + (fib (n - 2))
eq (fib 10) 55 "recursion fib"

# --- Control flow: if/elif/else ---
fn sign n
  if n > 0
    "pos"
  elif n == 0
    "zero"
  else
    "neg"
eq (sign 5) "pos" "if"
eq (sign 0) "zero" "elif"
eq (sign (-3)) "neg" "else"     # negative arg in paren-free call -> parens required

# --- each: list, range, map, skip/stop ---
sum <- 0
each n in [1 2 3 4]
  sum <- sum + n
eq sum 10 "each list"

acc <- 0
each i in 1..3
  acc <- acc + i
eq acc 6 "each range"

# skip (continue) - skip even numbers
odds <- 0
each i in 1..6
  if i % 2 == 0
    skip
  odds <- odds + i
eq odds 9 "each skip (1+3+5)"

# stop (break)
cnt <- 0
each i in 1..100
  if i > 3
    stop
  cnt <- cnt + 1
eq cnt 3 "each stop"

# each map: k, v
m = {a:1 b:2 c:3}
msum <- 0
each k, v in m
  msum <- msum + v
eq msum 6 "each map k,v"

# each i in inf - infinite loop, ends with stop, i increments from 0 (issue #27)
isum <- 0
each i in inf
  if i == 5
    stop
  isum <- isum + i
eq isum 10 "each inf stop (0+1+2+3+4)"

# inf + skip: count odd numbers, stop at 10
odds2 <- 0
each i in inf
  if i >= 10
    stop
  if i % 2 == 0
    skip
  odds2 <- odds2 + 1
eq odds2 5 "each inf skip (odd 1,3,5,7,9)"

# --- match: symbol and number ---
fn nameof s
  match s
    :new -> "new"
    :done -> "done"
    _ -> "other"
eq (nameof :new) "new" "match sym hit"
eq (nameof :x) "other" "match sym default"

fn numword n
  match n
    1 -> "one"
    2 -> "two"
    _ -> "many"
eq (numword 2) "two" "match int"

# --- pipe ---
fn inc x -> x + 1
fn sq x -> x * x
eq (5 |> inc |> sq) 36 "pipe"

# --- Nullary call: f() ---
fn answer -> 42
eq (answer()) 42 "nullary call"
# paren-free name = function value; called with ()
fv = answer
eq (fv()) 42 "nullary value then call"
# lambda nullary
lam = \-> 7
eq (lam()) 7 "lambda nullary"

# --- String interpolation ---
nm = "Aziza"
eq "hello ${nm}" "hello Aziza" "interp expr"
eq "1+1=${1 + 1}" "1+1=2" "interp calc"

# --- try/catch (issue #125) ---
# a fail with a status is caught, catch variable binds to {message, status}
tc1 = try
  fail 422 "invalid"
catch e
  eq e.message "invalid" "catch message"
  eq e.status 422 "catch status"
  "fallback"
eq tc1 "fallback" "try catch fallback value"
# on success the body value is returned, catch does not run
tc2 = try
  40 + 2
catch
  0
eq tc2 42 "try success value"
# fail without status and runtime error - status nil
tc3 = try
  fail "boom"
catch e
  e.status
eq tc3 nil "fail without status -> status nil"
# ret control-signal passes through try (catch does not catch it)
fn tc_ret
  try
    ret "early"
  catch
    ret "caught"
eq (tc_ret()) "early" "ret passes through try"

# --- par: parallel fan-out (issue #137) ---
# Note: lambda elements inside a list are separated by PARENS - `(\-> ...)`.
pr = par [
  (\-> 1 + 1)
  (\-> str.up "hi")
  (\-> [1 2 3].len)
]
eq pr.len 3 "par result count"
eq pr.0.ok 2 "par 1st result ok"
eq pr.1.ok "HI" "par 2nd result ok"
eq pr.2.ok 3 "par 3rd result ok"
# partial success: the one that failed gives {err}, the rest {ok}
pmix = par [(\-> 42) (\-> fail "boom") (\-> "z")]
eq pmix.0.ok 42 "par partial 1-ok"
eq pmix.1.err "boom" "par partial 2-err"
eq pmix.2.ok "z" "par partial 3-ok"
# closure capture reads the outer variable in parallel
pbase = 100
pcap = par [(\-> pbase + 1) (\-> pbase + 2)]
eq pcap.0.ok 101 "par closure capture 1"
eq pcap.1.ok 102 "par closure capture 2"
# nested paren-free HOF lambda inside a body (full expression inside parens)
pnest = par [(\-> [1 2 3].map \x -> x + 1)]
eq pnest.0.ok.0 2 "par nested HOF 1"
eq pnest.0.ok.2 4 "par nested HOF 3"
# empty list -> empty result
eq (par []).len 0 "par empty list"

# --- End ---
if fails == 0
  log "=== 01_core: ALL PASSED ==="
else
  log "=== 01_core: ${fails} TESTS FAILED ==="
