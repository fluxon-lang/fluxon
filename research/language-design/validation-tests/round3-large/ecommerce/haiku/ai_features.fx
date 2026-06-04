# AI features - summarize product from reviews, recommend based on history

use http db ai

exp fn summarize_product req
  product_id = str.int req.params.id!

  product = db.one "select * from products where id=$1" [product_id]!
  reviews = db.q "select rating, body from reviews where product=$1 order by created desc limit 10" [product_id]

  if reviews.len == 0
    fail "No reviews available for summarization"

  review_text = ""
  each review in reviews
    review_text = review_text + "Rating: ${review.rating}/5"
    if review.body != nil
      review_text = review_text + " - ${review.body}"
    review_text = review_text + "\n"

  prompt = "Summarize these customer reviews for the product '${product.name}' into a compelling marketing summary (2-3 sentences):\n${review_text}"

  summary = ai.ask prompt

  ret {product_id:product_id summary:summary}

exp fn recommend_products req
  customer_id = str.int req.params.customer_id!

  customer = db.one "select * from customers where id=$1" [customer_id]!

  order_history = db.q "select distinct p.id, p.name, p.category from orders o join order_items oi on o.id=oi.order join products p on oi.product=p.id where o.customer=$1 order by o.created desc limit 5" [customer_id]

  if order_history.len == 0
    ret {recommendations:[]}

  categories_str = ""
  each item in order_history
    categories_str = categories_str + item.category + ", "

  prompt = "Based on a customer's purchase history in categories: ${categories_str}, recommend 5 products from our catalog. The customer bought: "
  each item in order_history
    prompt = prompt + item.name + ", "
  prompt = prompt + ". Give recommendations as a JSON array of product names (no explanation, just names in order of relevance)."

  recs = ai.json prompt {
    recommendations: ["str"]
  }

  ret recs
