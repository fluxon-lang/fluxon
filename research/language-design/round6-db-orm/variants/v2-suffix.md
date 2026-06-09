### db (Postgres, $DATABASE_URL auto)
```flux
row  = db.ins "orders" {cust:5 status::new}          # → full row (with id)
db.up "orders" {total:1500} {id:oid}                 # {set} {where}
db.del "cart_items" {id:iid}                          # {where}
db.put "memory" {val:v} {agent:a key:k}               # UPSERT (atomic)
```

Reads are declarative — a flat filter map, no raw SQL:
```flux
rows = db.find "bookings" {tenant_id:tid}             # → list of maps (all matching)
one  = db.get  "bookings" {id:bid tenant_id:tid}      # → first match, or nil
```
A map key = a column; multiple keys are AND-ed. A **list value → `IN (...)`**:
```flux
db.find "bookings" {tenant_id:tid status:[:pending :confirmed]}  # status IN (..)
```
Operators — a **suffix on the key**, `col__op` (ops: `gt ge lt le ne like`):
```flux
db.find "bookings" {tenant_id:tid start_at__ge:t0 start_at__lt:t1}  # >= t0 AND < t1
db.find "resources" {tenant_id:tid capacity__ge:4 name__like:"%lab%"}
```
A bare key (no `__`) means `=`. Order / limit / paging — an **optional second map**:
```flux
db.find "bookings" {tenant_id:tid} {order::start_at limit:50 offset:0}
db.find "bookings" {tenant_id:tid} {order::created desc:true limit:20}
```
`order` = a symbol (column), `desc:true` = descending, `limit`/`offset` = ints.

Aggregation — `db.agg "table" {filter} {spec}`. Spec keys name the output:
```flux
db.agg "bookings" {tenant_id:tid status:[:done :confirmed]}
  {group::resource_id count::n sum__total_cents::revenue order::revenue desc:true}
# → [{resource_id:5 n:12 revenue:48000} ...]
```
Spec keys: `count::out`, `sum__col::out` / `avg__col::out` / `min__col::out` /
`max__col::out`, `group::col` (or list), plus `order`/`desc`/`limit`. No `group`
→ one summary row. For a raw expression (`date(created)`) use `db.q`.

`db.q "raw SQL" [params]` / `db.one` stay available as an escape hatch:
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
`sym` column: text in DB, symbol in Flux (`{status::pending}` filters fine).
