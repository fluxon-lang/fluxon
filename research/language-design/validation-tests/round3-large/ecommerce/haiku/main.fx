# Main e-commerce backend API

use http db
use ./products
use ./cart
use ./checkout
use ./reviews
use ./ai_features as ai_svc
use ./cron_tasks

# Product endpoints
http.on :get "/products" \req ->
  result = products.list_products req
  rep 200 result

http.on :get "/products/:id" \req ->
  product = products.get_product req
  rep 200 product

http.on :post "/products" \req ->
  product = products.create_product req
  rep 201 product

http.on :put "/products/:id/stock" \req ->
  updated = products.update_stock req
  rep 200 updated

# Cart endpoints
http.on :post "/customers/:customer_id/cart/items" \req ->
  result = cart.add_to_cart req
  rep 200 result

http.on :get "/customers/:customer_id/cart" \req ->
  cart_view = cart.view_cart req
  rep 200 cart_view

http.on :delete "/cart/items/:item_id" \req ->
  result = cart.remove_from_cart req
  rep 200 result

http.on :put "/cart/items/:item_id" \req ->
  result = cart.update_cart_qty req
  rep 200 result

# Checkout
http.on :post "/checkout" \req ->
  customer_id = str.int req.body.customer_id!
  order = checkout.process_checkout customer_id
  rep 201 order

http.on :get "/orders/:order_id" \req ->
  order_data = checkout.get_order req
  rep 200 order_data

# Reviews
http.on :post "/products/:product_id/reviews" \req ->
  review = reviews.post_review req
  rep 201 review

http.on :get "/products/:product_id/reviews" \req ->
  result = reviews.list_reviews req
  rep 200 result

# AI features
http.on :post "/products/:id/summarize" \req ->
  result = ai_svc.summarize_product req
  rep 200 result

http.on :get "/recommend/:customer_id" \req ->
  recs = ai_svc.recommend_products req
  rep 200 recs

# Health check
http.on :get "/health" \req ->
  rep 200 {status::ok}

# Start server
http.serve 8080
