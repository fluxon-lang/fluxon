# checkout.fluxon — savatchani buyurtmaga aylantirish
# MUHIM: har item uchun zaxira tekshiriladi, jami DB narxidan hisoblanadi
# (mijoz narxiga ishonilmaydi), zaxira kamaytiriladi, savat :converted
# bo'ladi, order + order_items yaratiladi. Bironta item yetishmasa — butun
# checkout toza rad etiladi (hech narsa o'zgartirilmaydi).
use http db

# POST /checkout  body: {customer_id: int}
http.on :post "/checkout" \req ->
  cust = req.body.customer_id
  if cust == nil
    rep 400 {error:"customer_id required"}
  else
    cart = db.one "select * from carts where customer=$1 and status=$2" [cust :open]
    if cart == nil
      rep 404 {error:"no open cart"}
    else
      rows = db.q "select ci.product, ci.qty, p.name, p.price, p.stock from cart_items ci join products p on p.id = ci.product where ci.cart=$1" [cart.id]
      if rows.len == 0
        rep 400 {error:"cart is empty"}
      else
        # 1-bosqich: zaxirani tekshiramiz. Yetishmaganlarni yig'amiz.
        shortages <- []
        each r in rows
          if r.stock < r.qty
            shortages <- shortages.push {
              product:r.product
              name:r.name
              requested:r.qty
              available:r.stock
            }

        if shortages.len > 0
          # Hech narsa o'zgartirmaymiz — toza rad etamiz.
          rep 409 {error:"out of stock" items:shortages}
        else
          # 2-bosqich: jamini DB narxlaridan hisoblaymiz (mijozga ishonmaymiz).
          total <- 0.0
          each r in rows
            total <- total + (r.price * r.qty)

          # 3-bosqich: buyurtmani yaratamiz.
          order = db.ins "orders" {
            customer:cust
            total:total
            status::placed
          }

          # 4-bosqich: order_items yaratamiz va zaxirani kamaytiramiz.
          each r in rows
            db.ins "order_items" {
              order:order.id
              product:r.product
              qty:r.qty
              unit_price:r.price
            }
            db.up "products" {stock: r.stock - r.qty} {id:r.product}

          # 5-bosqich: savatni :converted ga o'tkazamiz.
          db.up "carts" {status::converted} {id:cart.id}

          # Yakuniy buyurtma + tarkibi.
          items = db.q "select * from order_items where order=$1" [order.id]
          rep 201 {order:order items:items}

# GET /orders/:customer_id — mijoz buyurtmalari tarixi.
http.on :get "/orders/:customer_id" \req ->
  orders = db.q "select * from orders where customer=$1 order by created desc" [req.params.customer_id]
  rep 200 {orders:orders}
