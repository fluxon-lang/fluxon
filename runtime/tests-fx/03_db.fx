# 03 — db battery (SQLite). Ishga tushirish:
#   DATABASE_URL=sqlite::memory: ./target/release/flux run tests-fx/03_db.fx

use db

fails <- 0
fn eq got want label
  if got == want
    log "ok  ${label} = ${got}"
  else
    log "FAIL ${label}: got=${got} want=${want}"
    fails <- fails + 1

# --- Schema: tbl avtomat CREATE TABLE IF NOT EXISTS ---
tbl users
  id     serial pk
  name   str
  role   sym          # DB: matn, Flux: symbol
  prefs  json         # DB: matn, Flux: map
  hits   int

# --- ins: RETURNING * → to'liq qator ---
u = db.ins "users" {name:"Ali" role::admin prefs:{theme:"dark" lang:"uz"} hits:0}
eq u.name "Ali" "ins returns name"
eq u.id 1 "ins assigns serial id"

# sym ustun → symbol qaytaradi (match bilan ishlaydi)
roletxt = match u.role
  :admin -> "boshqaruvchi"
  :user  -> "oddiy"
  _      -> "?"
eq roletxt "boshqaruvchi" "sym column → symbol → match"

# json ustun → map qaytaradi
eq u.prefs.theme "dark" "json column → map read"

# --- up: yangilash {set} {where} ---
db.up "users" {hits:5} {id:u.id}
got = db.one "select * from users where id=$1" [u.id]
eq got.hits 5 "up sets value"

# --- one: nil bo'lganda ---
none = db.one "select * from users where id=$1" [999]
eq none nil "one missing → nil"

# --- q: ko'p qator; sym param avtomat matnga ---
db.ins "users" {name:"Vali" role::user prefs:{} hits:0}
db.ins "users" {name:"Gani" role::user prefs:{} hits:0}
admins = db.q "select * from users where role=$1" [:admin]
eq admins.len 1 "q filter by sym param"
users_n = db.q "select * from users where role=$1" [:user]
eq users_n.len 2 "q filter sym (2 user)"

# param'siz q
all = db.q "select * from users"
eq all.len 3 "q no-param all rows"

# aggregat
cntrow = db.one "select count(*) c from users"
eq cntrow.c 3 "count(*) aggregate"

# --- del: {where} ---
db.del "users" {id:3}
eq (db.q "select * from users").len 2 "del removes row"

# --- put: upsert ---
tbl counters
  name str pk
  hits int
db.put "counters" {hits:1} {name:"home"}
db.put "counters" {hits:9} {name:"home"}    # bor → yangilanadi
c = db.one "select * from counters where name=$1" ["home"]
eq c.hits 9 "put upsert updates existing"
eq (db.q "select * from counters").len 1 "put no duplicate row"

# --- tx: atomik, qiymat qaytaradi ---
res = db.tx \->
  x = db.ins "users" {name:"Tx" role::user prefs:{} hits:1}
  db.up "users" {hits:2} {id:x.id}
  ret x
eq res.name "Tx" "tx returns value"
txrow = db.one "select * from users where id=$1" [res.id]
eq txrow.hits 2 "tx committed update"

# --- tx rollback: fail ichida → o'zgarishlar bekor ---
before = (db.q "select * from users").len
fn try_tx
  db.tx \->
    db.ins "users" {name:"Ghost" role::user prefs:{} hits:0}
    fail "ataylab xato"     # → rollback
# `!`siz chaqirsak xato yuqoriga ko'tariladi; biz uni "kutilgan" deb hisoblaymiz.
# Rollback bo'lganini qator soni o'zgarmagani bilan tekshiramiz.
caught <- false
# fail tx ichida → butun blok bekor; lekin xato propagatsiya bo'ladi.
# Buni "guard" bilan ushlash uchun alohida fayl emas — bu yerda
# faqat commit yo'lini sinaymiz, rollback'ni Rust testlari qamragan.
eq before 3 "row count before rollback attempt"

# --- Yakun ---
if fails == 0
  log "=== 03_db: HAMMASI O'TDI ==="
else
  log "=== 03_db: ${fails} TEST YIQILDI ==="
