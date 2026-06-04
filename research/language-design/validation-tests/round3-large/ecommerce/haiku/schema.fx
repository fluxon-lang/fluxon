# Schema definitions for e-commerce backend

tbl products
  id       serial pk
  name     str
  description str null
  price    flt
  stock    int
  category str
  created  now

tbl customers
  id       serial pk
  email    str uniq
  name     str
  created  now

tbl carts
  id       serial pk
  customer int ref:customers.id
  status   sym
  created  now
  updated  now

tbl cart_items
  id       serial pk
  cart     int ref:carts.id
  product  int ref:products.id
  qty      int
  created  now

tbl orders
  id       serial pk
  customer int ref:customers.id
  total    flt
  status   sym
  created  now
  updated  now

tbl order_items
  id       serial pk
  order    int ref:orders.id
  product  int ref:products.id
  qty      int
  unit_price flt
  created  now

tbl reviews
  id       serial pk
  product  int ref:products.id
  customer int ref:customers.id
  rating   int
  body     str null
  created  now
