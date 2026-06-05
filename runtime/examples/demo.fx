# Yadro tilning namoyishi — batteries'siz, lekin to'liq mantiq.
# Ishga tushirish:  flux run examples/demo.fx

fn fib n
  if n < 2
    ret n
  (fib (n - 1)) + (fib (n - 2))

each i in 0..10
  log "fib ${i} = ${fib i}"

# List metodlari — canonical naqsh (filter -> map -> reduce)
nums = [1 2 3 4 5 6 7 8 9 10]
evens = nums.filter \x -> x % 2 == 0
squared = evens.map \x -> x * x
total = squared.reduce 0 \acc x -> acc + x
log "juftlar=${evens}"
log "kvadratlar=${squared}"
log "yig'indi=${total}"

# match — symbol dispatch
fn nomi s
  match s
    :new -> "yangi"
    :done -> "tugagan"
    _ -> "noma'lum"

each st in [:new :done :other]
  log "${st} -> ${nomi st}"

# mutable holat + pipe
fn inc x -> x + 1
fn sq x -> x * x
log "pipe: ${5 |> inc |> sq}"

# map operatsiyalari + null-coalesce
user = {ism:"Aziza" yosh:30}
log "ism=${user.ism} email=${user.email ?? "yo'q"}"
log "kalitlar=${user.keys}"
