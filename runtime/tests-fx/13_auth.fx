# 13 — auth battery (JWT + parol hash, issue #69).
# Ishga: AUTH_SECRET=sirli-kalit ./target/release/flux run tests-fx/13_auth.fx
#
# auth.jwt {payload} [{exp:N}] -> imzolangan JWT (HS256).
# auth.verify token            -> payload map (imzo + exp avto tekshirilgan), yoki err.
# auth.hash "parol"            -> argon2id PHC matn (salt ichida).
# auth.check "parol" hash      -> bool (doimiy-vaqt taqqoslash).
#
# Imzo kaliti $AUTH_SECRET env'dan AVTO topiladi (db/ai naqshiga mos).

use auth

fails <- 0

fn eq got want label
  if got == want
    log "ok  ${label} = ${got}"
  else
    log "FAIL ${label}: got=${got} want=${want}"
    fails <- fails + 1

fn truthy v label
  if v
    log "ok  ${label}"
  else
    log "FAIL ${label}: got=${v}"
    fails <- fails + 1

# --- Parol hash + check ---
hash = auth.hash "user-parol-123"
truthy (str.has hash "argon2id") "auth.hash argon2id PHC string"
truthy (auth.check "user-parol-123" hash) "auth.check to'g'ri parol -> true"
eq (auth.check "boshqa" hash) false "auth.check noto'g'ri parol -> false"
eq (auth.check "x" "buzuq-hash") false "auth.check buzuq hash -> false (xato emas)"

# Bir xil parol har gal boshqa hash (salt tasodifiy).
h2 = auth.hash "user-parol-123"
truthy (hash != h2) "auth.hash tasodifiy salt (har gal boshqa)"

# --- JWT imzolash + tekshirish (roundtrip) ---
token = auth.jwt {sub: "u1" tenant: "t1" role: "admin"}
parts = str.split token "."
eq parts.len 3 "JWT 3 segment (header.payload.imzo)"

claims = auth.verify token
eq claims.sub "u1" "verify -> sub saqlanadi"
eq claims.tenant "t1" "verify -> tenant saqlanadi"
eq claims.role "admin" "verify -> role saqlanadi"
truthy (claims.exp > claims.iat) "iat/exp avto qo'shildi (exp > iat)"

# --- {exp:N} opt ---
qisqa = auth.jwt {sub: "u2"} {exp: 3600}
qc = auth.verify qisqa
truthy (qc.exp == (qc.iat + 3600)) "{exp:3600} -> exp = iat + 3600"

# --- Yakun ---
if fails == 0
  log "=== 13_auth: HAMMASI O'TDI ==="
else
  log "=== 13_auth: ${fails} TEST YIQILDI ==="
