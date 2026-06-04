# Reviews endpoints

use http db

exp fn post_review req
  product_id = str.int req.params.product_id!
  customer_id = str.int req.body.customer_id!
  rating = str.int req.body.rating!
  body = req.body.body ?? nil

  if rating < 1 | rating > 5
    fail "Rating must be between 1 and 5"

  product = db.one "select * from products where id=$1" [product_id]!

  review = db.ins "reviews" {
    product: product_id
    customer: customer_id
    rating: rating
    body: body
  }

  ret review

exp fn list_reviews req
  product_id = str.int req.params.product_id!

  reviews = db.q "select * from reviews where product=$1 order by created desc" [product_id]

  avg_result = db.one "select avg(rating) as avg_rating from reviews where product=$1" [product_id]
  avg_rating = avg_result.avg_rating ?? 0

  ret {reviews:reviews average_rating:avg_rating}
