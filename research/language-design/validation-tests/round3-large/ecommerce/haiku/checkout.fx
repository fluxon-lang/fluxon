# Checkout logic - validates stock and creates order

use db

exp fn process_checkout customer_id
  # Get active cart
  cart = db.one "select * from carts where customer=$1 and status=:active" [customer_id]
  if cart == nil
    fail "No active cart found"

  # Get all cart items with product info
  items = db.q "select ci.id as item_id, ci.qty, p.id as product_id, p.price, p.stock from cart_items ci join products p on ci.product=p.id where ci.cart=$1 and ci.qty > 0" [cart.id]

  if items.len == 0
    fail "Cart is empty"

  # Validate stock for all items BEFORE making any changes
  each item in items
    if item.stock < item.qty
      fail "Product out of stock: product_id=${item.product_id} needed=${item.qty} available=${item.stock}"

  # Compute total from DB prices (never trust client)
  total <- 0
  each item in items
    total <- total + (item.price * item.qty)

  tax = total * 0.1
  final_total = total + tax

  # Create the order
  order = db.ins "orders" {
    customer: customer_id
    total: final_total
    status: :new
  }

  # Create order items and decrement stock
  each item in items
    db.ins "order_items" {
      order: order.id
      product: item.product_id
      qty: item.qty
      unit_price: item.price
    }

    new_stock = item.stock - item.qty
    db.up "products" {stock:new_stock} {id:item.product_id}

  # Mark cart as converted
  db.up "carts" {status::completed} {id:cart.id}

  ret order

exp fn get_order req
  order_id = str.int req.params.order_id!
  order = db.one "select * from orders where id=$1" [order_id]!
  items = db.q "select * from order_items where order=$1" [order_id]
  ret {order:order items:items}
