# Data model — Postgres tables. Prices live here only.

tbl users
  id    serial pk
  name  str
  ph    str uniq          # owner WhatsApp number
  tz    str
  ts    now

tbl customers
  id    serial pk
  owner int ref:users.id
  name  str
  ph    str uniq
  route str null          # delivery route tag, eg "center-cafes"
  notes str null
  ts    now

tbl products
  id    serial pk
  owner int ref:users.id
  name  str
  price flt               # canonical price — AI never invents this
  unit  str               # "loaf" "kg"
  stock int
  ts    now

tbl orders
  id     serial pk
  cust   int ref:customers.id
  status str              # :new :confirmed :delivered :cancelled
  total  flt
  deliv  str null         # delivery date
  ts     now

tbl order_items
  id    serial pk
  ord   int ref:orders.id
  prod  int ref:products.id
  qty   int
  price flt               # snapshot of products.price at order time

tbl messages              # full audit log
  id    serial pk
  cust  int ref:customers.id null
  dir   str               # :in :out
  body  str
  ts    now

tbl ai_interactions       # AI audit
  id     serial pk
  msg    int ref:messages.id null
  intent str
  conf   flt
  tokens int
  cost   flt
  ms     int
  ts     now

tbl schedule_routes
  id    serial pk
  owner int ref:users.id
  name  str
  day   str               # :mon..:sun delivery day
  ts    now

tbl proactive_outreach
  id     serial pk
  cust   int ref:customers.id
  route  int ref:schedule_routes.id
  reason str
  status str              # :sent :replied :yes :no
  reply  str null
  ts     now
