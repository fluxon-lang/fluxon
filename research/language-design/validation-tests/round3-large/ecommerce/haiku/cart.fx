# Cart endpoints

use http db

exp fn get_or_create_cart customer_id
  cart = db.one "select * from carts where customer=$1 and status=:active" [customer_id]
  if cart != nil
    ret cart

  new_cart = db.ins "carts" {customer:customer_id status::active}
  ret new_cart

exp fn add_to_cart req
  customer_id = str.int req.params.customer_id!
  product_id = str.int req.body.product_id!
  qty = str.int (req.body.qty ?? "1")!

  if qty <= 0
    fail "Quantity must be positive"

  product = db.one "select * from products where id=$1" [product_id]!
  if product.stock < qty
    fail "Not enough stock"

  cart = get_or_create_cart customer_id

  existing = db.one "select * from cart_items where cart=$1 and product=$2" [cart.id product_id]
  if existing != nil
    new_qty = existing.qty + qty
    if product.stock < new_qty
      fail "Not enough stock for updated quantity"
    db.up "cart_items" {qty:new_qty} {id:existing.id}
  else
    db.ins "cart_items" {cart:cart.id product:product_id qty:qty}

  ret {ok:true}

exp fn view_cart req
  customer_id = str.int req.params.customer_id!
  cart = db.one "select * from carts where customer=$1 and status=:active" [customer_id]

  if cart == nil
    ret {cart:nil items:[] subtotal:0 total:0}

  items = db.q "select ci.id, ci.qty, p.id as product_id, p.name, p.price from cart_items ci join products p on ci.product=p.id where ci.cart=$1" [cart.id]

  subtotal <- 0
  each item in items
    subtotal <- subtotal + (item.price * item.qty)

  tax = subtotal * 0.1
  total = subtotal + tax

  ret {cart:cart items:items subtotal:subtotal tax:tax total:total}

exp fn remove_from_cart req
  cart_item_id = str.int req.params.item_id!
  db.up "cart_items" {qty:0} {id:cart_item_id}
  ret {ok:true}

exp fn update_cart_qty req
  cart_item_id = str.int req.params.item_id!
  new_qty = str.int req.body.qty!

  if new_qty <= 0
    db.up "cart_items" {qty:0} {id:cart_item_id}
  else
    item = db.one "select * from cart_items where id=$1" [cart_item_id]!
    product = db.one "select * from products where id=$1" [item.product]!
    if product.stock < new_qty
      fail "Not enough stock"
    db.up "cart_items" {qty:new_qty} {id:cart_item_id}

  ret {ok:true}
