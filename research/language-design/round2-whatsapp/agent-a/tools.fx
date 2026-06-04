# AI-callable tools. Each is a plain fn; ai.run discovers args by name.
use db env http

# Send a WhatsApp message via provider (battery: http client).
exp fn send ph body
  db.ins "messages" {cust:nil dir::out body:body}
  http.post "$env.WA_URL/messages" {to:ph text:body key:env.WA_KEY}

exp fn get_customer_history phone
  c = db.one "select * from customers where ph=$1" [phone]
  if !c
    ret {customer:nil orders:[]}
  os = db.q "select * from orders where cust=$1 order by ts desc limit 10" [c.id]
  {customer:c orders:os}

exp fn get_product_catalog owner
  db.q "select name price unit stock from products where owner=$1" [owner]

# Prices ALWAYS from db — we look up each product, never trust AI price.
exp fn create_order items customer delivery
  c = db.one "select * from customers where ph=$1" [customer]!
  ord = db.ins "orders" {cust:c.id status::new total:0 deliv:delivery}
  tot <- 0.0
  ea it in items
    p = db.one "select * from products where owner=$1 & lower(name)=lower($2)" [c.owner it.product]
    if !p
      ask_owner c.owner "Mahsulot topilmadi: ${it.product}. Narxi?"
      skip
    db.ins "order_items" {ord:ord.id prod:p.id qty:it.qty price:p.price}
    tot <- tot + p.price * it.qty
  db.up "orders" {total:tot} {id:ord.id}
  {order:ord.id total:tot}

exp fn update_product owner name price
  db.up "products" {price:price} {owner:owner name:name}
  {updated:name price:price}

# Ask the business owner; routed to their WhatsApp.
exp fn ask_owner owner question
  u = db.one "select * from users where id=$1" [owner]!
  send u.ph "🤖 $question"
  {asked:true}

exp fn schedule_outreach customer when reason
  c = db.one "select * from customers where ph=$1" [customer]!
  db.ins "proactive_outreach" {cust:c.id route:nil reason:reason status::sent}
  {scheduled:when reason:reason}
