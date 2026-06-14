# par primitive smoke-test (issue #137)
# Note: lambda elements inside a list are separated with PARENS — `(\-> ...)`.

# Simple fan-out: three independent computations in parallel
results = par [
  (\-> 1 + 1)
  (\-> str.up "hello")
  (\-> [1 2 3].len)
]
log results

# Closure captures an outer variable (parallel read)
base = 10
sums = par [
  (\-> base + 1)
  (\-> base + 2)
]
log sums

# On error, partial success: one fails, the rest still run
mixed = par [
  (\-> 42)
  (\-> fail "intentional error")
  (\-> "ok")
]
log mixed

# Nested HOF works inside a lambda body too (full expression inside parens)
nested = par [
  (\-> [1 2 3].map \x -> x + 1)
]
log nested

# Empty list
empty = par []
log empty
