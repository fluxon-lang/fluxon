# cart.flux — savatcha endpointlari
# add item, view cart (hisoblangan jami), remove item, update qty.
use http db

# Mijoz uchun ochiq savatcha topadi yoki yangisini yaratadi.
fn get_open_cart customer_id
  c = db.one "select * from carts where customer=$1 and status=$2" [customer_id :open]
  if c == nil
    ret db.ins "carts" {customer:customer_id status::open}
  ret c

# Savatcha tarkibini hisoblab beradi: itemlar + jami narx.
# Har bir item uchun mahsulot narxi va qatorlik subtotal qaytariladi.
fn build_cart_view cart_id
  rows = db.q "select ci.id, ci.product, ci.qty, p.name, p.price, p.stock from cart_items ci join products p on p.id = ci.product where ci.cart=$1 order by ci.id" [cart_id]
  total <- 0.0
  items <- []
  each r in rows
    subtotal = r.price * r.qty
    total <- total + subtotal
    items <- items.push {
      item_id: r.id
      product: r.product
      name: r.name
      price: r.price
      qty: r.qty
      subtotal: subtotal
    }
  ret {cart_id:cart_id items:items total:total}

# POST /carts/:customer_id/items — savatchaga mahsulot qo'shish.
# Agar mahsulot allaqachon savatda bo'lsa — qty ni oshiramiz.
http.on :post "/carts/:customer_id/items" \req ->
  b = req.body
  if b.product == nil | b.qty == nil
    rep 400 {error:"product and qty required"}
  elif b.qty <= 0
    rep 400 {error:"qty must be positive"}
  else
    prod = db.one "select * from products where id=$1" [b.product]
    if prod == nil
      rep 404 {error:"product not found"}
    else
      cart = get_open_cart req.params.customer_id
      existing = db.one "select * from cart_items where cart=$1 and product=$2" [cart.id b.product]
      if existing == nil
        db.ins "cart_items" {cart:cart.id product:b.product qty:b.qty}
      else
        db.up "cart_items" {qty: existing.qty + b.qty} {id:existing.id}
      rep 201 (build_cart_view cart.id)

# GET /carts/:customer_id — savatchani hisoblangan jami bilan ko'rsatish.
http.on :get "/carts/:customer_id" \req ->
  cart = db.one "select * from carts where customer=$1 and status=$2" [req.params.customer_id :open]
  if cart == nil
    rep 200 {cart_id:nil items:[] total:0.0}
  else
    rep 200 (build_cart_view cart.id)

# PUT /carts/items/:item_id — savatdagi item qty sini yangilash.
http.on :put "/carts/items/:item_id" \req ->
  item = db.one "select * from cart_items where id=$1" [req.params.item_id]
  if item == nil
    rep 404 {error:"cart item not found"}
  elif req.body.qty == nil
    rep 400 {error:"qty required"}
  elif req.body.qty <= 0
    # qty 0 yoki manfiy → itemni o'chiramiz.
    db.q "delete from cart_items where id=$1" [item.id]
    rep 200 (build_cart_view item.cart)
  else
    db.up "cart_items" {qty:req.body.qty} {id:item.id}
    rep 200 (build_cart_view item.cart)

# DELETE /carts/items/:item_id — savatdan itemni olib tashlash.
http.on :del "/carts/items/:item_id" \req ->
  item = db.one "select * from cart_items where id=$1" [req.params.item_id]
  if item == nil
    rep 404 {error:"cart item not found"}
  else
    db.q "delete from cart_items where id=$1" [item.id]
    rep 200 (build_cart_view item.cart)
