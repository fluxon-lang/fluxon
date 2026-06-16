use super::*;

#[test]
fn mutable_and_each() {
    run(r#"
total <- 0
each n in [10 20 30]
  total <- total + n
log "total=${total}"
"#);
}

// if/each/match blocks are lexically TRANSPARENT: an inner `=` updates the outer
// (same-fn) variable — like other languages, no clone is taken. This makes the
// accumulator pattern natural (before, an `=` inside a block silently created a
// new local -> the outer stayed nil).
#[test]
fn bind_in_block_updates_outer() {
    run(r#"
best <- nil
top <- 0
each e in [{n:"a" v:3} {n:"b" v:7} {n:"c" v:2}]
  if e.v > top
    top = e.v
    best = e
(top == 7) | (fail "top wrong: ${top}")
(best.n == "b") | (fail "best wrong: ${best.n}")
"#);
}

// Python model: there is no immutability. An outer `=` variable IS updated by a
// re-`=` from inside a transparent block (`if`/`each`/`match`) — same as the
// accumulator pattern, never an error.
#[test]
fn rebind_in_block_updates_outer() {
    run(r#"
x = 10
if true
  x = 20
(x == 20) | (fail "= inside block did not update outer x: ${x}")
"#);
}

// And a plain re-`=` at the same level just reassigns (no immutability error).
#[test]
fn rebind_same_level_ok() {
    run(r#"
x = 1
x = 2
(x == 2) | (fail "re-= did not update x: ${x}")
"#);
}

// fn/lambda BOUNDARY: an inner `=` creates a new LOCAL, not the outer variable
// (shadowing/isolation). The outer value is unchanged.
#[test]
fn bind_in_fn_shadows_not_mutates() {
    run(r#"
x = 100
f = \n ->
  x = 5
  x + n
(f 1 == 6) | (fail "fn local x did not work")
(x == 100) | (fail "= inside fn changed outer x: ${x}")
"#);
}

// `<-` (assign), however, CROSSES the fn boundary — closure capture is preserved
// (`=` stops at the boundary, `<-` does not: the clear difference between them).
#[test]
fn assign_crosses_fn_boundary_capture() {
    run(r#"
counter <- 0
inc = \n ->
  counter <- counter + n
inc 5
inc 3
(counter == 8) | (fail "closure capture did not work: ${counter}")
"#);
}

#[test]
fn match_symbols() {
    run(r#"
fn label s
  match s
    :new -> "new"
    :done -> "done"
    _ -> "other"

log (label :new)
log (label :x)
"#);
}

#[test]
fn string_and_modules() {
    run(r#"
s = "Salom Dunyo"
log (str.up s)
log "len=${str.len s} floor=${math.floor 3.7}"
parts = str.split "a,b,c" ","
log "parts=${parts} joined=${parts.join "-"}"
"#);
}
