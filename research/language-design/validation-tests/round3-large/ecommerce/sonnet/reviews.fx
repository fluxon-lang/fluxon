use http db
use ./schema

# POST /products/:id/reviews — submit a review for a product
# Body: { customer_id, rating (1-5), body }
http.on :post "/products/:id/reviews" \req ->
  product_id  = str.int req.params.id
  customer_id = req.body.customer_id
  rating      = req.body.rating
  body        = req.body.body ?? ""

  if customer_id == nil
    rep 400 {error:"customer_id is required"}
  elif rating == nil
    rep 400 {error:"rating is required"}
  elif rating < 1 | rating > 5
    rep 400 {error:"rating must be between 1 and 5"}
  else
    product = db.one "select id from products where id=$1" [product_id]
    if product == nil
      rep 404 {error:"Product not found"}
    else
      customer = db.one "select id from customers where id=$1" [customer_id]
      if customer == nil
        rep 404 {error:"Customer not found"}
      else
        # Check if customer has purchased this product (optional validation)
        has_purchased = db.one "select oi.id from order_items oi join orders o on o.id=oi.order_id where o.customer_id=$1 and oi.product_id=$2 limit 1" [customer_id product_id]
        if has_purchased == nil
          rep 403 {error:"You can only review products you have purchased"}
        else
          # Check for duplicate review
          existing = db.one "select id from reviews where product_id=$1 and customer_id=$2" [product_id customer_id]
          if existing != nil
            rep 409 {error:"You have already reviewed this product"}
          else
            review = db.ins "reviews" {product_id:product_id customer_id:customer_id rating:rating body:body}
            rep 201 review

# GET /products/:id/reviews — list all reviews for a product with average rating
http.on :get "/products/:id/reviews" \req ->
  product_id = str.int req.params.id

  product = db.one "select id, name from products where id=$1" [product_id]
  if product == nil
    rep 404 {error:"Product not found"}
  else
    reviews = db.q "select r.*, c.name as customer_name from reviews r join customers c on c.id=r.customer_id where r.product_id=$1 order by r.created desc" [product_id]
    avg_row = db.one "select avg(rating) as avg_rating, count(*) as total from reviews where product_id=$1" [product_id]
    avg_rating = avg_row.avg_rating ?? 0.0
    total      = avg_row.total ?? 0
    rep 200 {product_id:product_id product_name:product.name reviews:reviews avg_rating:avg_rating total_reviews:total}

