use http db
use ./schema

# POST /checkout — convert an open cart into an order
# Body: { customer_id: int }
# 1. Fetch active cart and all its items (with product prices from DB)
# 2. Validate every item has enough stock — fail entire checkout if any is short
# 3. Compute total from DB prices (never trust client-supplied prices)
# 4. Decrement stock for each item
# 5. Mark cart as :converted
# 6. Insert order + all order_items
http.on :post "/checkout" \req ->
  customer_id = req.body.customer_id

  if customer_id == nil
    rep 400 {error:"customer_id is required"}
  else
    customer = db.one "select id, name, email from customers where id=$1" [customer_id]
    if customer == nil
      rep 404 {error:"Customer not found"}
    else
      cart = db.one "select * from carts where customer_id=$1 and status=$2" [customer_id "open"]
      if cart == nil
        rep 400 {error:"No active cart found for this customer"}
      else
        # Fetch cart items joined with live product data
        items = db.q "select ci.id as item_id, ci.product_id, ci.qty, p.name, p.price, p.stock from cart_items ci join products p on p.id=ci.product_id where ci.cart_id=$1 and ci.qty > 0" [cart.id]

        if items.len == 0
          rep 400 {error:"Cart is empty"}
        else
          # Phase 1: validate all stock BEFORE mutating anything
          out_of_stock <- nil
          each item in items
            if item.stock < item.qty
              out_of_stock <- {product:item.name available:item.stock requested:item.qty}
              stop

          if out_of_stock != nil
            rep 400 {error:"Insufficient stock" detail:out_of_stock}
          else
            # Phase 2: compute total from DB prices
            total <- 0.0
            each item in items
              total <- total + (item.price * item.qty)

            # Phase 3: create the order record
            order = db.ins "orders" {customer_id:customer_id total:total status::pending}

            # Phase 4: insert order_items and decrement stock
            each item in items
              db.ins "order_items" {order_id:order.id product_id:item.product_id qty:item.qty unit_price:item.price}
              new_stock = item.stock - item.qty
              db.up "products" {stock:new_stock} {id:item.product_id}

            # Phase 5: mark cart as converted
            db.up "carts" {status::converted} {id:cart.id}

            # Return the full order with its items
            order_items = db.q "select oi.*, p.name from order_items oi join products p on p.id=oi.product_id where oi.order_id=$1" [order.id]
            rep 201 {order:order items:order_items}

# GET /orders/:id — retrieve a single order with its items
http.on :get "/orders/:id" \req ->
  id    = str.int req.params.id
  order = db.one "select * from orders where id=$1" [id]
  if order == nil
    rep 404 {error:"Order not found"}
  else
    items = db.q "select oi.*, p.name from order_items oi join products p on p.id=oi.product_id where oi.order_id=$1" [id]
    rep 200 {order:order items:items}

# GET /orders — list orders for a customer via ?customer_id=
http.on :get "/orders" \req ->
  customer_id = req.query.customer_id
  if customer_id == nil
    rep 400 {error:"customer_id query param is required"}
  else
    cid    = str.int customer_id
    orders = db.q "select * from orders where customer_id=$1 order by created desc" [cid]
    rep 200 {orders:orders}
