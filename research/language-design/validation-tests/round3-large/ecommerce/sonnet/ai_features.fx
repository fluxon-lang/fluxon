use http db
use ai as llm
use ./schema

# Helper: fetch reviews for a product and build a summary string
fn build_reviews_text product_id
  reviews = db.q "select rating, body from reviews where product_id=$1 order by created desc limit 20" [product_id]
  if reviews.len == 0
    ret ""
  text <- ""
  each r in reviews
    text <- text + "Rating: ${r.rating}/5 — ${r.body}\n"
  text

# POST /products/:id/summarize — generate an AI marketing summary from product reviews
http.on :post "/products/:id/summarize" \req ->
  product_id = str.int req.params.id

  product = db.one "select * from products where id=$1" [product_id]
  if product == nil
    rep 404 {error:"Product not found"}
  else
    reviews_text = build_reviews_text product_id

    if reviews_text == ""
      # No reviews yet — generate a summary from product info alone
      prompt = "Write a compelling 2-3 sentence marketing summary for this product:\nName: ${product.name}\nDescription: ${product.description}\nPrice: $${product.price}\nCategory: ${product.category}"
    else
      prompt = "Write a compelling 2-3 sentence marketing summary for this product based on customer reviews:\nName: ${product.name}\nDescription: ${product.description}\nPrice: $${product.price}\nCategory: ${product.category}\n\nCustomer Reviews:\n${reviews_text}"

    summary = llm.ask prompt
    rep 200 {product_id:product_id summary:summary}

# GET /recommend/:customer_id — AI-powered product recommendations based on order history
http.on :get "/recommend/:customer_id" \req ->
  customer_id = str.int req.params.customer_id

  customer = db.one "select * from customers where id=$1" [customer_id]
  if customer == nil
    rep 404 {error:"Customer not found"}
  else
    # Fetch customer's purchase history with product details
    history = db.q "select p.name, p.category, p.description, oi.qty from order_items oi join orders o on o.id=oi.order_id join products p on p.id=oi.product_id where o.customer_id=$1 order by o.created desc limit 30" [customer_id]

    if history.len == 0
      rep 200 {customer_id:customer_id recommendations:[] message:"No purchase history found"}
    else
      # Build purchase history text
      history_text <- ""
      each item in history
        history_text <- history_text + "- ${item.name} (category: ${item.category}, qty: ${item.qty})\n"

      # Fetch available products to recommend from
      available = db.q "select id, name, category, description, price from products where stock > 0 order by id limit 50"

      available_text <- ""
      each p in available
        available_text <- available_text + "ID:${p.id} ${p.name} (${p.category}) — $${p.price}\n"

      prompt = "Based on this customer's purchase history, recommend 5 products from the available catalog.\n\nPurchase history:\n${history_text}\nAvailable products:\n${available_text}\n\nReturn only JSON."

      result = llm.json prompt {
        recommendations: [{product_id:int name:str reason:str}]
      }

      rep 200 {customer_id:customer_id recommendations:result.recommendations}
