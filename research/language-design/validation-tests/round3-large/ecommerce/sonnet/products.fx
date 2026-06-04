use http db
use ./schema

# List products with optional ?category= and ?search= filters and ?page= pagination
http.on :get "/products" \req ->
  page     = str.int (req.query.page ?? "1")
  per_page = 20
  offset   = (page - 1) * per_page

  category = req.query.category
  search   = req.query.search

  if category != nil & search != nil
    pattern = "%${search}%"
    rows = db.q "select * from products where category=$1 and (name ilike $2 or description ilike $2) order by id limit $3 offset $4" [category pattern per_page offset]
    rep 200 {products:rows page:page per_page:per_page}
  elif category != nil
    rows = db.q "select * from products where category=$1 order by id limit $2 offset $3" [category per_page offset]
    rep 200 {products:rows page:page per_page:per_page}
  elif search != nil
    pattern = "%${search}%"
    rows = db.q "select * from products where name ilike $1 or description ilike $1 order by id limit $2 offset $3" [pattern per_page offset]
    rep 200 {products:rows page:page per_page:per_page}
  else
    rows = db.q "select * from products order by id limit $1 offset $2" [per_page offset]
    rep 200 {products:rows page:page per_page:per_page}

# Get single product by id
http.on :get "/products/:id" \req ->
  id      = str.int req.params.id
  product = db.one "select * from products where id=$1" [id]
  if product == nil
    rep 404 {error:"Product not found"}
  else
    rep 200 product

# Create a new product
http.on :post "/products" \req ->
  name        = req.body.name
  description = req.body.description ?? ""
  price       = req.body.price
  stock       = req.body.stock ?? 0
  category    = req.body.category ?? "general"

  if name == nil
    rep 400 {error:"name is required"}
  elif price == nil
    rep 400 {error:"price is required"}
  else
    created = db.ins "products" {name:name description:description price:price stock:stock category:category}
    rep 201 created

# Update product stock
http.on :put "/products/:id/stock" \req ->
  id    = str.int req.params.id
  stock = req.body.stock

  if stock == nil
    rep 400 {error:"stock value is required"}
  else
    product = db.one "select * from products where id=$1" [id]
    if product == nil
      rep 404 {error:"Product not found"}
    else
      db.up "products" {stock:stock} {id:id}
      updated = db.one "select * from products where id=$1" [id]
      rep 200 updated
