use http db
use ./schema

# Helper: get or create an active cart for a customer
fn get_or_create_cart customer_id
  cart = db.one "select * from carts where customer_id=$1 and status=$2" [customer_id "open"]
  if cart != nil
    ret cart
  db.ins "carts" {customer_id:customer_id status::open}

# Helper: compute cart totals by joining cart_items with products
fn cart_with_totals cart_id
  items = db.q "select ci.id, ci.cart_id, ci.product_id, ci.qty, p.name, p.price, (ci.qty * p.price) as line_total from cart_items ci join products p on p.id=ci.product_id where ci.cart_id=$1 and ci.qty > 0" [cart_id]
  total <- 0.0
  each item in items
    total <- total + item.line_total
  {cart_id:cart_id items:items total:total}

# POST /cart/items — add item to cart (requires customer_id, product_id, qty in body)
http.on :post "/cart/items" \req ->
  customer_id = req.body.customer_id
  product_id  = req.body.product_id
  qty         = req.body.qty ?? 1

  if customer_id == nil
    rep 400 {error:"customer_id is required"}
  elif product_id == nil
    rep 400 {error:"product_id is required"}
  else
    # Validate customer exists
    customer = db.one "select id from customers where id=$1" [customer_id]
    if customer == nil
      rep 404 {error:"Customer not found"}
    else
      # Validate product exists and has enough stock
      product = db.one "select * from products where id=$1" [product_id]
      if product == nil
        rep 404 {error:"Product not found"}
      elif product.stock < qty
        rep 400 {error:"Insufficient stock" available:product.stock}
      else
        cart = get_or_create_cart customer_id
        # Check if item already in cart
        existing = db.one "select * from cart_items where cart_id=$1 and product_id=$2" [cart.id product_id]
        if existing != nil
          new_qty = existing.qty + qty
          db.up "cart_items" {qty:new_qty} {id:existing.id}
        else
          db.ins "cart_items" {cart_id:cart.id product_id:product_id qty:qty}
        rep 200 (cart_with_totals cart.id)

# GET /cart/:customer_id — view cart with computed totals
http.on :get "/cart/:customer_id" \req ->
  customer_id = str.int req.params.customer_id
  customer    = db.one "select id from customers where id=$1" [customer_id]
  if customer == nil
    rep 404 {error:"Customer not found"}
  else
    cart = db.one "select * from carts where customer_id=$1 and status=$2" [customer_id "open"]
    if cart == nil
      rep 200 {cart_id:nil items:[] total:0.0}
    else
      rep 200 (cart_with_totals cart.id)

# DELETE /cart/items/:item_id — remove a specific item from cart
http.on :del "/cart/items/:item_id" \req ->
  item_id = str.int req.params.item_id
  item    = db.one "select * from cart_items where id=$1" [item_id]
  if item == nil
    rep 404 {error:"Cart item not found"}
  else
    # Spec has no db.del — soft-delete by setting qty=0; cart_with_totals filters qty>0
    db.up "cart_items" {qty:0} {id:item_id}
    rep 200 (cart_with_totals item.cart_id)

# PUT /cart/items/:item_id — update qty of a cart item
http.on :put "/cart/items/:item_id" \req ->
  item_id = str.int req.params.item_id
  qty     = req.body.qty

  if qty == nil
    rep 400 {error:"qty is required"}
  elif qty < 1
    rep 400 {error:"qty must be at least 1"}
  else
    item = db.one "select * from cart_items where id=$1" [item_id]
    if item == nil
      rep 404 {error:"Cart item not found"}
    else
      # Check stock
      product = db.one "select * from products where id=$1" [item.product_id]
      if product.stock < qty
        rep 400 {error:"Insufficient stock" available:product.stock}
      else
        db.up "cart_items" {qty:qty} {id:item_id}
        rep 200 (cart_with_totals item.cart_id)

exp fn get_or_create_cart
exp fn cart_with_totals
