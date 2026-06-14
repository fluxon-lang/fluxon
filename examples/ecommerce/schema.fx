# schema.fluxon — all table schemas (tbl)
# Data model for the e-commerce backend.
use db

tbl products
  id          serial pk
  name        str
  description str
  price       flt
  stock       int
  category    str

tbl customers
  id    serial pk
  email str uniq
  name  str

tbl carts
  id       serial pk
  customer int ref:customers.id
  status   sym          # :open | :converted

tbl cart_items
  id      serial pk
  cart    int ref:carts.id
  product int ref:products.id
  qty     int

tbl orders
  id       serial pk
  customer int ref:customers.id
  total    flt
  status   sym          # :placed | :cancelled
  created  now

tbl order_items
  id        serial pk
  order     int ref:orders.id
  product   int ref:products.id
  qty       int
  unit_price flt

tbl reviews
  id       serial pk
  product  int ref:products.id
  customer int ref:customers.id
  rating   int
  body     str
  created  now
