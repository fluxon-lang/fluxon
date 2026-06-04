# products.flux — mahsulot endpointlari
# list (filter + pagination), get one, create, update stock.
use http db

# Sahifa hajmi (har sahifada nechta mahsulot).
page_size = 20

# GET /products  ?category=  ?search=  ?page=
# Filterlarni dinamik quramiz. Param indekslari ($1, $2...) ketma-ket.
http.on :get "/products" \req ->
  page = str.int (req.query.page ?? "1")
  offset = (page - 1) * page_size

  # WHERE shartlarini va parametrlarni dinamik to'playmiz.
  conds <- []
  params <- []
  idx <- 0

  if req.query.category != nil
    idx <- idx + 1
    conds <- conds.push "category = $${idx}"
    params <- params.push req.query.category

  if req.query.search != nil
    idx <- idx + 1
    conds <- conds.push "name ilike $${idx}"
    params <- params.push "%${req.query.search}%"

  # WHERE bo'limini yig'amiz (shart bo'lsa). Indeks bo'yicha " and " bilan ulaymiz.
  where <- ""
  if conds.len > 0
    joined <- ""
    pos <- 0
    each c in conds
      if pos == 0
        joined <- c
      else
        joined <- joined + " and " + c
      pos <- pos + 1
    where <- " where " + joined

  # LIMIT va OFFSET parametrlarini qo'shamiz.
  idx <- idx + 1
  lim_idx = idx
  idx <- idx + 1
  off_idx = idx
  params <- params.push page_size
  params <- params.push offset

  sql = "select * from products" + where + " order by id limit $${lim_idx} offset $${off_idx}"
  rows = db.q sql params
  rep 200 {page:page page_size:page_size items:rows}

# GET /products/:id — bitta mahsulot.
http.on :get "/products/:id" \req ->
  p = db.one "select * from products where id=$1" [req.params.id]
  if p == nil
    rep 404 {error:"product not found"}
  else
    rep 200 p

# POST /products — yangi mahsulot yaratish.
http.on :post "/products" \req ->
  b = req.body
  if b.name == nil | b.price == nil
    rep 400 {error:"name and price required"}
  else
    p = db.ins "products" {
      name: b.name
      description: b.description ?? ""
      price: b.price
      stock: b.stock ?? 0
      category: b.category ?? "uncategorized"
    }
    rep 201 p

# PUT /products/:id/stock — faqat zaxirani yangilash.
http.on :put "/products/:id/stock" \req ->
  p = db.one "select * from products where id=$1" [req.params.id]
  if p == nil
    rep 404 {error:"product not found"}
  elif req.body.stock == nil
    rep 400 {error:"stock required"}
  else
    db.up "products" {stock:req.body.stock} {id:req.params.id}
    updated = db.one "select * from products where id=$1" [req.params.id]
    rep 200 updated
