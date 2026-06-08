# auth battery — JWT imzolash/tekshirish + parol hash (issue #69).
#
# Imzo kaliti $AUTH_SECRET env'dan AVTO topiladi (db -> $DATABASE_URL,
# ai -> ANTHROPIC_API_KEY naqshiga mos). Hech narsa sozlash shart emas:
#
#   export AUTH_SECRET=sirli-kalit
#   flux run examples/auth_demo.fx
#
# (Server BLOKLAMAYDI — bu demo to'g'ridan-to'g'ri chiqib ketadi. Haqiqiy
#  auth oqimi uchun examples/middleware.fx ga qarang.)

use auth

# --- Parol hash (ro'yxatdan o'tish / kirish) ---
# Salt avtomatik, doimiy-vaqt taqqoslash — agent xato qila olmaydi.
hash = auth.hash "user-paroli-123"
log "hash: ${hash}"

ok = auth.check "user-paroli-123" hash
log "to'g'ri parol -> ${ok}"                  # true

xato = auth.check "boshqa-parol" hash
log "noto'g'ri parol -> ${xato}"              # false

# --- JWT imzolash (login muvaffaqiyatli bo'lgach) ---
# payload map'iga iat (berilgan vaqt) + exp (muddat, default 24 soat) avto qo'shiladi.
token = auth.jwt {sub: "u_42" tenant: "acme" role: "admin"}
log "token: ${token}"

# Qisqa muddat (1 soat) kerak bo'lsa — ikkinchi {exp:sekund} opt:
qisqa = auth.jwt {sub: "u_42"} {exp: 3600}
log "1 soatlik token uzunligi: ${(str.split qisqa ".").len} segment"

# --- JWT tekshirish (har himoyalangan so'rovda) ---
# Imzo + muddat (exp) AVTOMATIK tekshiriladi. Noto'g'ri bo'lsa err -> 401.
claims = auth.verify token
log "claims.sub = ${claims.sub}"
log "claims.tenant = ${claims.tenant}"
log "claims.role = ${claims.role}"

log "auth demo tugadi"
