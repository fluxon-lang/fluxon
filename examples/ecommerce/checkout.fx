# checkout.fluxon — convert a cart into an order
# IMPORTANT: stock is checked for each item, the total is computed from DB
# prices (the customer's price is not trusted), stock is decremented, the cart
# becomes :converted, and order + order_items are created. If any item is short,
# the entire checkout is cleanly rejected (nothing is changed).
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
        # Step 1: check stock. Collect any shortages.
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
          # Change nothing — reject cleanly.
          rep 409 {error:"out of stock" items:shortages}
        else
          # Step 2: compute the total from DB prices (do not trust the customer).
          total <- 0.0
          each r in rows
            total <- total + (r.price * r.qty)

          # Step 3: create the order.
          order = db.ins "orders" {
            customer:cust
            total:total
            status::placed
          }

          # Step 4: create order_items and decrement stock.
          each r in rows
            db.ins "order_items" {
              order:order.id
              product:r.product
              qty:r.qty
              unit_price:r.price
            }
            db.up "products" {stock: r.stock - r.qty} {id:r.product}

          # Step 5: mark the cart as :converted.
          db.up "carts" {status::converted} {id:cart.id}

          # Final order + its contents.
          items = db.q "select * from order_items where order=$1" [order.id]
          rep 201 {order:order items:items}

# GET /orders/:customer_id — a customer's order history.
http.on :get "/orders/:customer_id" \req ->
  orders = db.q "select * from orders where customer=$1 order by created desc" [req.params.customer_id]
  rep 200 {orders:orders}
