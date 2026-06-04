# Cron tasks - daily at 02:00 for low-stock alerts and revenue reporting

use cron db time log

fn log_low_stock
  low_stock = db.q "select id, name, stock from products where stock < 10 order by stock asc"
  log "LOW STOCK ALERT: ${low_stock.len} products have stock < 10"
  each product in low_stock
    log "  - ${product.name}: ${product.stock} units"

fn log_daily_revenue
  yesterday = time.ago 24 :hr
  revenue_result = db.one "select count(*) as order_count, sum(total) as daily_revenue from orders where created > $1 and status != :cancelled" [yesterday]

  orders_count = revenue_result.order_count ?? 0
  daily_revenue = revenue_result.daily_revenue ?? 0

  log "DAILY REVENUE (last 24h): ${orders_count} orders, total: $${daily_revenue}"

fn daily_report
  log "=== DAILY REPORT ==="
  log_low_stock
  log_daily_revenue
  log "=== END REPORT ==="

cron.dy 2 0 daily_report
