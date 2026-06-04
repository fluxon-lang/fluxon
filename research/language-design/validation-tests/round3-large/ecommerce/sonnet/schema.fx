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
  id          serial pk
  customer_id int ref:customers.id
  status      sym

tbl cart_items
  id         serial pk
  cart_id    int ref:carts.id
  product_id int ref:products.id
  qty        int

tbl orders
  id          serial pk
  customer_id int ref:customers.id
  total       flt
  status      sym
  created     now

tbl order_items
  id         serial pk
  order_id   int ref:orders.id
  product_id int ref:products.id
  qty        int
  unit_price flt

tbl reviews
  id          serial pk
  product_id  int ref:products.id
  customer_id int ref:customers.id
  rating      int
  body        str
  created     now
