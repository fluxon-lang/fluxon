use db cron
use ./schema

# Helper: format a product row for log output
fn fmt_product p
  "[id:${p.id}] ${p.name} — stock: ${p.stock}, category: ${p.category}"

# Daily job at 02:00 — report low-stock products and last-24h revenue
fn daily_report
  log "=== Daily Ecommerce Report ==="

  # Low-stock products (stock < 10)
  low_stock = db.q "select id, name, stock, category from products where stock < 10 order by stock asc"
  log "Low-stock products (stock < 10): ${low_stock.len} items"
  each p in low_stock
    log "  WARNING: ${fmt_product p}"

  # Total revenue in the last 24 hours
  cutoff = time.ago 24 :hr
  revenue_row = db.one "select sum(total) as total_revenue, count(*) as order_count from orders where created > $1" [cutoff]
  total_revenue = revenue_row.total_revenue ?? 0.0
  order_count   = revenue_row.order_count ?? 0

  log "Revenue last 24h: $${total_revenue} from ${order_count} orders"

  # Breakdown by top-selling categories in last 24h
  top_categories = db.q "select p.category, sum(oi.qty) as units_sold, sum(oi.qty * oi.unit_price) as revenue from order_items oi join orders o on o.id=oi.order_id join products p on p.id=oi.product_id where o.created > $1 group by p.category order by revenue desc limit 5" [cutoff]

  if top_categories.len > 0
    log "Top categories (last 24h):"
    each cat in top_categories
      log "  ${cat.category}: ${cat.units_sold} units — $${cat.revenue}"
  else
    log "No orders in the last 24h."

  log "=== End of Report ==="

# Register the cron: every day at 02:00
cron.dy 2 0 daily_report
