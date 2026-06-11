# db_demo.fluxon — db battery namoyishi (SQLite, zero-setup).
# Ishga tushirish (in-memory, hech narsa o'rnatmasdan):
#   DATABASE_URL=sqlite::memory: cargo run -- run examples/db_demo.fx
# yoki fayl bilan:
#   DATABASE_URL=sqlite:/tmp/fluxon.db cargo run -- run examples/db_demo.fx

use db

# tbl — schema e'loni va YAGONA MANBA. db ochilganda Fluxon `tbl` ni DB joriy
# holati bilan solishtiradi (diff) va kerakli DDL'ni o'zi bajaradi: yangi
# ustun → ADD COLUMN, olib tashlangan ustun → DROP COLUMN (backup bilan),
# index qo'shilsa/olinsa → CREATE/DROP INDEX. Siz faqat oxirgi ko'rinishni
# yozasiz — migration SQL yozish shart emas, qayta-deploy idempotent.
tbl tickets
  id       serial pk
  category sym          # DB: matn, Fluxon: symbol
  status   sym index    # tez-tez filtrlanadi → index (auto nom: idx_tickets_status)
  meta     json         # DB: matn, Fluxon: map/list

  index(category status)   # ko'p-ustunli index (bo'shliq bilan, vergulsiz)

# --- ins: qo'shilgan qatorni qaytaradi (RETURNING *) ---
t = db.ins "tickets" {category::billing status::new meta:{tries:0 src:"web"}}
log "qo'shildi id=${t.id} category=${t.category} status=${t.status}"

# sym ustun symbol qaytaradi -> match to'g'ridan-to'g'ri ishlaydi
match t.category
  :billing -> log "  -> to'lov masalasi"
  :tech    -> log "  -> texnik"
  _        -> log "  -> boshqa"

# json ustun map qaytaradi
log "  meta.src=${t.meta.src} meta.tries=${t.meta.tries}"

# --- up: yangilash ---
db.up "tickets" {status::done} {id:t.id}
one = db.one "select * from tickets where id=$1" [t.id]
log "yangilandi status=${one.status}"

# --- q: ro'yxat; symbol parametri avtomat matnga aylanadi ---
db.ins "tickets" {category::billing status::new meta:{}}
billing = db.q "select * from tickets where category=$1" [:billing]
log "billing ticketlar soni=${billing.len}"

# param'siz q
all = db.q "select * from tickets"
log "jami ticketlar=${all.len}"

# --- put: upsert (bor bo'lsa yangila, yo'q bo'lsa qo'sh) ---
tbl counters
  name  str pk
  label str uniq        # bir ustunli uniq → alohida UNIQUE INDEX (auto nom)
  hits  int

db.put "counters" {hits:1} {name:"home"}
db.put "counters" {hits:5} {name:"home"}      # bor -> yangilanadi
c = db.one "select * from counters where name=$1" ["home"]
log "counter home hits=${c.hits}"             # 5

# --- tx: atomik blok, ret qiymat qaytaradi ---
r = db.tx \->
  x = db.ins "tickets" {category::tech status::new meta:{}}
  ret x
log "tx qaytardi id=${r.id}"
# (tx ichida fail/`!` chiqsa butun blok rollback bo'ladi va xato yuqoriga
#  ko'tariladi — buni rollback testlari tekshiradi.)

log "TUGADI"
