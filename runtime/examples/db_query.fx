# db read builder (issue #78) — filter/range/order/agg without raw SQL.
# Running it:  DATABASE_URL="sqlite::memory:" cargo run -- run examples/db_query.fx

use db

tbl bookings
  id          serial pk
  tenant_id   int
  resource_id int
  user_email  str
  status      sym
  start_at    str
  guests      int
  total_cents money

# Test data
db.ins "bookings" {tenant_id:1 resource_id:5 user_email:"a@x.uz" status::done      start_at:"2026-06-01" guests:2 total_cents:5000}
db.ins "bookings" {tenant_id:1 resource_id:5 user_email:"a@x.uz" status::confirmed start_at:"2026-06-02" guests:4 total_cents:3000}
db.ins "bookings" {tenant_id:1 resource_id:7 user_email:"b@x.uz" status::pending   start_at:"2026-06-03" guests:1 total_cents:1000}

# 1. IN-filter (list value -> IN) + order + pagination — no raw SQL
rows = db.from "bookings"
  |> db.eq {tenant_id:1 status:[:pending :confirmed]}
  |> db.order :start_at
  |> db.limit 50 |> db.offset 0
  |> db.all
log "1. active bookings: ${rows.len}"

# 2. Range filter (db.cmp) — start_at range
rng = db.from "bookings"
  |> db.eq {tenant_id:1}
  |> db.cmp :start_at :ge "2026-06-02"
  |> db.all
log "2. after 06-02: ${rng.len}"

# 3. db.first — a single row or nil
one = db.from "bookings" |> db.eq {tenant_id:1 resource_id:7} |> db.first
log "3. resource 7 status: ${one.status}"

# 4. Aggregation: group by resource, revenue in descending order
by_res = db.from "bookings"
  |> db.eq {tenant_id:1 status:[:done :confirmed]}
  |> db.group :resource_id
  |> db.count :n |> db.sum :total_cents :revenue
  |> db.order :revenue :desc
  |> db.agg
log "4. top-earning resource: ${by_res.0.resource_id} (${by_res.0.revenue} cent, ${by_res.0.n} booking)"

# 5. Conditional aggregate (overview) — count + revenue by status in one query
ov = db.from "bookings"
  |> db.eq {tenant_id:1}
  |> db.count_if {status::confirmed} :confirmed
  |> db.count_if {status::pending} :pending
  |> db.sum_if :total_cents {status::done} :revenue
  |> db.agg_row
log "5. overview: confirmed=${ov.confirmed} pending=${ov.pending} revenue=${ov.revenue}"

# 6. Query-string statuses to a sym filter: "pending,confirmed" -> [:pending :confirmed]
q = "pending,confirmed"
syms = (str.split q ",").map \s -> str.sym s
filtered = db.from "bookings" |> db.eq {tenant_id:1 status:syms} |> db.all
log "6. str.sym filter: ${filtered.len}"
