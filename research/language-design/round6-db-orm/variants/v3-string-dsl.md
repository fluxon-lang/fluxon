### db (Postgres, $DATABASE_URL auto)
```flux
row  = db.ins "orders" {cust:5 status::new}          # → full row (with id)
db.up "orders" {total:1500} {id:oid}                 # {set} {where}
db.del "cart_items" {id:iid}                          # {where}
db.put "memory" {val:v} {agent:a key:k}               # UPSERT (atomic)
```

Reads — table, a short WHERE-DSL string, and a named-params map. NO `select`,
NO `from`, NO `order by`/`limit` SQL: just the predicate. Named params `:name`
bind from the map (never string-concat values):
```flux
rows = db.find "bookings" "tenant_id = :tid" {tid:tid}      # → list of maps
one  = db.get  "bookings" "id = :id and tenant_id = :tid" {id:bid tid:tid}  # → map or nil
```
The DSL allows: `= != < <= > >=`, `and`, `in`, `like`. A **list param → `IN`**:
```flux
db.find "bookings" "tenant_id = :tid and status in :st" {tid:tid st:[:pending :confirmed]}
db.find "bookings" "tenant_id = :tid and start_at >= :a and start_at < :b" {tid:tid a:t0 b:t1}
```
Order / limit / paging — an **optional trailing options map**:
```flux
db.find "bookings" "tenant_id = :tid" {tid:tid} {order::start_at limit:50 offset:0}
db.find "bookings" "tenant_id = :tid" {tid:tid} {order::created desc:true limit:20}
```
`order` = a symbol (column), `desc:true` = descending, `limit`/`offset` = ints.

Aggregation — `db.agg "table" "where" {params} {spec}`. Spec names the outputs;
`group` groups; reuse `order`/`desc`/`limit`:
```flux
db.agg "bookings" "tenant_id = :tid and status in :st" {tid:tid st:[:done :confirmed]}
  {group::resource_id count::n sum::total_cents:revenue order::revenue desc:true}
# → [{resource_id:5 n:12 revenue:48000} ...]
```
Spec keys: `count::out`, `sum::col:out` / `avg::col:out` / `min::col:out` /
`max::col:out`, `group::col` (or list), plus `order`/`desc`/`limit`. Empty where
`""` = all rows. For a raw expression (`date(created)`) use `db.q`.

`db.q "raw SQL" [params]` / `db.one` (positional `$1`) stay as a full escape hatch:
```flux
db.q "select date(created) day, count(*) n from bookings where tenant_id=$1 group by day order by day" [tid]
```

Transaction — atomic, rollback on `fail`/`!`, returns a value:
```flux
res = db.tx \->
  ord = db.ins "orders" {cust:c total:t}
  each it in items
    db.up "products" {stock:it.stock - it.qty} {id:it.id}
  ret ord
```
`db.tx` auto-serializable + retry → "read-check-update" is race-safe. Idempotency:
`uniq` column + ins inside tx (duplicate → rollback).

Schema = `tbl`:
```flux
tbl products
  id    serial pk
  owner int ref:users.id
  price money               # money = integer minor unit (cents), NOT float
  ts    now
```
Types: serial int flt str bool json now sym money (`int` 64-bit). Modifiers:
`pk uniq null ref:tbl.col`. Multi-column: `uniq(agent, key)`.
`json` column: auto map/list on read, auto-encode on write.
`sym` column: text in DB, symbol in Flux (`status in :st` filters fine).
