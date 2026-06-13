# par primitivi smoke-test (issue #137)
# Eslatma: list ichidagi lambda elementlar QAVS bilan ajraladi — `(\-> ...)`.

# Oddiy fan-out: uchta mustaqil hisob parallel
results = par [
  (\-> 1 + 1)
  (\-> str.up "salom")
  (\-> [1 2 3].len)
]
log results

# Closure tashqi o'zgaruvchini ushlaydi (parallel o'qish)
base = 10
sums = par [
  (\-> base + 1)
  (\-> base + 2)
]
log sums

# Xato bo'lsa qisman muvaffaqiyat: bittasi fail, qolganlar ishlaydi
mixed = par [
  (\-> 42)
  (\-> fail "qasddan xato")
  (\-> "ok")
]
log mixed

# Nested HOF lambda body ichida ham ishlaydi (qavs ichi to'liq ifoda)
nested = par [
  (\-> [1 2 3].map \x -> x + 1)
]
log nested

# Bo'sh ro'yxat
empty = par []
log empty
