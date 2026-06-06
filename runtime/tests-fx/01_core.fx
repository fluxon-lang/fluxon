# 01 — Yadro til: tiplar, binding, funksiya, control flow, match, operatorlar.
# Har bir blok kutilgan natija bilan solishtiriladi; xato bo'lsa "FAIL" chiqadi.

fails <- 0

fn eq got want label
  if got == want
    log "ok  ${label} = ${got}"
  else
    log "FAIL ${label}: got=${got} want=${want}"
    fails <- fails + 1

# --- Tiplar ---
eq 42 42 "int"
eq 3.5 3.5 "flt"
eq "hi" "hi" "str"
eq true true "bool"
eq :ok :ok "sym"
eq nil nil "nil"

# --- Binding: o'zgarmas vs o'zgaruvchan ---
x = 10
eq x 10 "= immutable"
total <- 0
total <- total + 5
total <- total + 5
eq total 10 "<- mutable reassign"

# --- Operatorlar ---
eq (2 + 3 * 4) 14 "precedence * over +"
eq (10 % 3) 1 "modulo"
eq (10 / 4) 2 "int div"          # butun bo'linish
eq ("a" + "b") "ab" "str concat"
eq (5 > 3 & 2 < 4) true "and"
eq (5 < 3 | 2 < 4) true "or"
eq (!false) true "not"
eq (nil ?? "x") "x" "coalesce nil"
eq (7 ?? "x") 7 "coalesce non-nil"

# --- Diapazon ---
eq (1..5) [1 2 3 4 5] "range"

# --- Funksiya: ret, oxirgi-ifoda, bir qatorli, lambda, closure, rekursiya ---
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

# closure — tashqi o'zgaruvchini ushlaydi
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
eq (sign (-3)) "neg" "else"     # qavssiz chaqiruvda manfiy arg → qavs shart

# --- each: list, range, map, skip/stop ---
sum <- 0
each n in [1 2 3 4]
  sum <- sum + n
eq sum 10 "each list"

acc <- 0
each i in 1..3
  acc <- acc + i
eq acc 6 "each range"

# skip (continue) — juftlarni o'tkaz
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

# --- match: symbol va son ---
fn nomi s
  match s
    :new -> "yangi"
    :done -> "tugagan"
    _ -> "boshqa"
eq (nomi :new) "yangi" "match sym hit"
eq (nomi :x) "boshqa" "match sym default"

fn numword n
  match n
    1 -> "bir"
    2 -> "ikki"
    _ -> "ko'p"
eq (numword 2) "ikki" "match int"

# --- pipe ---
fn inc x -> x + 1
fn sq x -> x * x
eq (5 |> inc |> sq) 36 "pipe"

# --- Argumentsiz (nullary) chaqiruv: f() ---
fn answer -> 42
eq (answer()) 42 "nullary chaqiruv"
# qavssiz nom = funksiya qiymati; () bilan chaqiriladi
fv = answer
eq (fv()) 42 "nullary qiymat keyin chaqiruv"
# lambda nullary
lam = \-> 7
eq (lam()) 7 "lambda nullary"

# --- String interpolatsiya ---
nm = "Aziza"
eq "salom ${nm}" "salom Aziza" "interp expr"
eq "1+1=${1 + 1}" "1+1=2" "interp calc"

# --- Yakun ---
if fails == 0
  log "=== 01_core: HAMMASI O'TDI ==="
else
  log "=== 01_core: ${fails} TEST YIQILDI ==="
