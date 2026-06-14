# reviews.fluxon — product reviews
# POST a review (rating 1-5 is validated), list a product's reviews + average.
use http db

# POST /products/:id/reviews  body: {customer_id, rating, body}
http.on :post "/products/:id/reviews" \req ->
  b = req.body
  pid = req.params.id
  if b.customer_id == nil | b.rating == nil
    rep 400 {error:"customer_id and rating required"}
  elif b.rating < 1 | b.rating > 5
    rep 400 {error:"rating must be between 1 and 5"}
  else
    prod = db.one "select * from products where id=$1" [pid]
    if prod == nil
      rep 404 {error:"product not found"}
    else
      review = db.ins "reviews" {
        product:pid
        customer:b.customer_id
        rating:b.rating
        body:b.body ?? ""
      }
      rep 201 review

# GET /products/:id/reviews — reviews + average rating.
http.on :get "/products/:id/reviews" \req ->
  pid = req.params.id
  reviews = db.q "select * from reviews where product=$1 order by created desc" [pid]
  agg = db.one "select avg(rating) a, count(*) c from reviews where product=$1" [pid]
  rep 200 {
    product:pid
    count:agg.c ?? 0
    average:agg.a ?? 0
    reviews:reviews
  }
