# 03b — db.tx rollback: fail ichida bo'lsa butun blok bekor + xato yuqoriga.
# Bu fayl ATAYLAB xato bilan tugaydi (exit != 0). Tashqi skript ikki narsani
# tekshiradi: (1) "ROLLBACK-OK" chiqdi (qator qo'shilmadi), (2) exit kodi != 0.
#
# Ishga: DATABASE_URL=sqlite:/tmp/fluxon_rb_test.db ./target/release/fluxon run tests-fx/03b_db_rollback.fx
# (fayl-asosli db kerak: tx ichidagi yozuv rollback bo'lganini keyin tekshirib bo'lmaydi
#  in-memory bilan ham bo'ladi, chunki rollback ayni jarayonda ko'rinadi.)

use db

tbl items
  id   serial pk
  name str

db.ins "items" {name:"asl"}
before = (db.q "select * from items").len    # 1

# tx ichida ins + fail → rollback kutiladi
db.tx \->
  db.ins "items" {name:"ghost"}
  inside = (db.q "select * from items").len   # tx ichida 2 ko'rinadi
  log "tx ichida qator soni = ${inside}"
  fail "ataylab rollback"

# BU YERGA YETIB KELMAYDI — fail yuqoriga ko'tariladi va dastur to'xtaydi.
log "BU CHIQMASLIGI KERAK"
