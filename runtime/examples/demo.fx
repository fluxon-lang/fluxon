# Core language demonstration — no batteries, but full logic.
# Running it:  fluxon run examples/demo.fx

fn fib n
  if n < 2
    ret n
  (fib (n - 1)) + (fib (n - 2))

each i in 0..10
  log "fib ${i} = ${fib i}"

# List methods — canonical pattern (filter -> map -> reduce)
nums = [1 2 3 4 5 6 7 8 9 10]
evens = nums.filter \x -> x % 2 == 0
squared = evens.map \x -> x * x
total = squared.reduce 0 \acc x -> acc + x
log "evens=${evens}"
log "squares=${squared}"
log "sum=${total}"

# match — symbol dispatch
fn name_of s
  match s
    :new -> "new"
    :done -> "done"
    _ -> "unknown"

each st in [:new :done :other]
  log "${st} -> ${name_of st}"

# mutable state + pipe
fn inc x -> x + 1
fn sq x -> x * x
log "pipe: ${5 |> inc |> sq}"

# map operations + null-coalesce
user = {name:"Aziza" age:30}
log "name=${user.name} email=${user.email ?? "none"}"
log "keys=${user.keys}"
