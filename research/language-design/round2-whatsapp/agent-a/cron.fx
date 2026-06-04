# Weekly proactive outreach + Sunday-evening owner briefing.
use db cron ai json
use ./tools

# Sat 10:00 — ask every routed customer if they want this week's delivery.
exp fn register
  cron.wk :sat 10 0 outreach
  cron.wk :sun 18 0 briefing

fn outreach
  routes = db.q "select * from schedule_routes"
  ea rt in routes
    custs = db.q "select * from customers where owner=$1 & route=$2" [rt.owner rt.name]
    ea c in custs
      tools.send c.ph "Salom ${c.name}! Bu hafta ($rt.day) yetkazib berishni xohlaysizmi? (ha/yo'q)"
      db.ins "proactive_outreach" {cust:c.id route:rt.id reason:"weekly" status::sent}

# Sun 18:00 — summarize tomorrow's route and DM the owner.
fn briefing
  owners = db.q "select * from users"
  ea u in owners
    rows = db.q "select p.* from proactive_outreach p
      join customers c on c.id=p.cust
      where c.owner=$1 & p.status=:yes & p.ts > now()-interval '7 days'" [u.id]
    if !rows
      skip
    items <- 0
    ea r in rows
      o = db.one "select sum(qty) n from order_items oi
        join orders od on od.id=oi.ord where od.cust=$1
        order by od.ts desc limit 1" [r.cust]
      items <- items + (o.n ? o.n : 0)
    # Let AI phrase a warm briefing from the numbers (no prices invented).
    txt = ai.ask "Ertangi marshrut bo'yicha ega uchun qisqa brifing yoz:
      ${rows.len} mijoz, $items dona non. O'zbekcha, do'stona, 1-2 jumla."
    tools.send u.ph "📋 $txt"
