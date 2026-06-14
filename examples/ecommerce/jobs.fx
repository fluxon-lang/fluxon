# jobs.fluxon — background jobs (cron)
# Every day at 02:00: low-stock products and the last 24 hours of revenue.
# The file is named "jobs" (not "cron") to avoid clashing with the battery.
use db cron

# Daily report job.
fn daily_report
  # Low-stock products (stock < 10).
  low = db.q "select id, name, stock from products where stock < 10 order by stock asc"
  log "[cron] Low-stock products: ${low.len}"
  each p in low
    log "[cron]   - #${p.id} ${p.name}: stock ${p.stock}"

  # Last 24 hours of revenue (placed orders).
  r = db.one "select sum(total) s, count(*) c from orders where created > $1 and status=$2" [time.ago 24 :hr :placed]
  log "[cron] Last 24 hours: ${r.c ?? 0} orders, revenue ${r.s ?? 0}"

# Run every day at 02:00.
exp fn register_jobs
  cron.dy 2 0 daily_report
