use http db
use ./schema
use ./products
use ./cart
use ./checkout
use ./reviews
use ./ai_features as ai_routes
use ./cron_jobs

http.on :post "/customers" \req ->
  email = req.body.email
  name  = req.body.name
  if email == nil
    rep 400 {error:"email is required"}
  elif name == nil
    rep 400 {error:"name is required"}
  else
    existing = db.one "select id from customers where email=$1" [email]
    if existing != nil
      rep 409 {error:"Email already registered"}
    else
      customer = db.ins "customers" {email:email name:name}
      rep 201 customer

http.on :get "/customers/:id" \req ->
  id       = str.int req.params.id
  customer = db.one "select * from customers where id=$1" [id]
  if customer == nil
    rep 404 {error:"Customer not found"}
  else
    rep 200 customer

# Health check
http.on :get "/health" \req ->
  rep 200 {status:"ok" service:"ecommerce-api"}

port = env.PORT ?? "8080"
http.serve (str.int port)
