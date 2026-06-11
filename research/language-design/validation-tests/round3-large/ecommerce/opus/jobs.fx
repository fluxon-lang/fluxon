# jobs.fluxon — fon vazifalar (cron)
# Har kuni 02:00 da: kam zaxiradagi mahsulotlar va so'nggi 24 soat daromadi.
# Fayl nomi "cron" emas (batareya bilan to'qnashmasin uchun) — jobs.
use db cron

# Kunlik hisobot vazifasi.
fn daily_report
  # Kam zaxiradagi mahsulotlar (stock < 10).
  low = db.q "select id, name, stock from products where stock < 10 order by stock asc"
  log "[cron] Kam zaxiradagi mahsulotlar: ${low.len} ta"
  each p in low
    log "[cron]   - #${p.id} ${p.name}: zaxira ${p.stock}"

  # So'nggi 24 soat daromadi (placed buyurtmalar).
  r = db.one "select sum(total) s, count(*) c from orders where created > $1 and status=$2" [time.ago 24 :hr :placed]
  log "[cron] So'nggi 24 soat: ${r.c ?? 0} buyurtma, daromad ${r.s ?? 0}"

# Har kuni 02:00 da ishga tushir.
exp fn register_jobs
  cron.dy 2 0 daily_report
