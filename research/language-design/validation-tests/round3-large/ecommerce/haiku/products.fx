# Product endpoints

use http db

exp fn list_products req
  category = req.query.category ?? nil
  search = req.query.search ?? nil
  page = str.int (req.query.page ?? "1")
  limit = 20
  offset = (page - 1) * limit

  query = "select * from products where 1=1"
  params = []

  if category != nil
    query = query + " and category = $" + str.str (params.len + 1)
    params = params.push category

  if search != nil
    search_term = "%" + search + "%"
    query = query + " and (name ilike $" + str.str (params.len + 1) + " or description ilike $" + str.str (params.len + 2) + ")"
    params = params.push search_term
    params = params.push search_term

  query = query + " order by created desc limit $" + str.str (params.len + 1) + " offset $" + str.str (params.len + 2)
  params = params.push limit
  params = params.push offset

  rows = db.q query params
  ret {items:rows page:page}

exp fn get_product req
  id = str.int req.params.id!
  product = db.one "select * from products where id=$1" [id]!
  ret product

exp fn create_product req
  name = req.body.name!
  description = req.body.description ?? nil
  price = req.body.price!
  stock = req.body.stock!
  category = req.body.category!

  if price < 0
    fail "Price must be non-negative"
  if stock < 0
    fail "Stock must be non-negative"

  product = db.ins "products" {
    name: name
    description: description
    price: price
    stock: stock
    category: category
  }
  ret product

exp fn update_stock req
  id = str.int req.params.id!
  new_stock = req.body.stock!

  if new_stock < 0
    fail "Stock must be non-negative"

  updated = db.up "products" {stock:new_stock} {id:id}
  ret updated
