### db (Postgres, $DATABASE_URL auto)
```flux
row  = db.ins "orders" {cust:5 status::new}          # → full row (with id)
db.up "orders" {total:1500} {id:oid}                 # {set} {where}
db.del "cart_items" {id:iid}                          # {where}
db.put "memory" {val:v} {agent:a key:k}               # UPSERT (atomic)
```

Reads — a query builder, piped with `|>`. `db.from "t"` starts it; `db.all`
runs it → list, `db.first` → one row or nil. No raw SQL:
```flux
rows = db.from "bookings" |> db.eq {tenant_id:tid} |> db.all
one  = db.from "bookings" |> db.eq {id:bid tenant_id:tid} |> db.first
```
Stages (each takes the query, returns the query — chain freely):
```flux
db.eq {col:val ...}        # equality, AND-ed. A list value → IN (...)
db.cmp :col :ge t          # one comparison: op ∈ :gt :ge :lt :le :ne :like
db.order :col              #   ascending
db.order :col :desc        #   descending
db.limit n   ·   db.offset n
```
```flux
db.from "bookings"
  |> db.eq {tenant_id:tid status:[:pending :confirmed]}   # status IN (..)
  |> db.cmp :start_at :ge t0
  |> db.cmp :start_at :lt t1
  |> db.order :start_at
  |> db.limit 50 |> db.offset 0
  |> db.all
```
Aggregation — builder stages that set output columns, then `db.agg` runs it:
```flux
db.from "bookings"
  |> db.eq {tenant_id:tid status:[:done :confirmed]}
  |> db.group :resource_id
  |> db.count :n                 # row count → column n
  |> db.sum :total_cents :revenue   # sum(total_cents) → column revenue
  |> db.order :revenue :desc
  |> db.agg                      # → [{resource_id:5 n:12 revenue:48000} ...]
```
Agg stages: `db.count :out`, `db.sum :col :out` / `db.avg :col :out` /
`db.min :col :out` / `db.max :col :out`, `db.group :col`. No `db.group` → one
summary row. For a raw expression (`date(created)`) use `db.q`.

`db.q "raw SQL" [params]` / `db.one` stay as a full escape hatch:
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
`sym` column: text in DB, symbol in Flux (`db.eq {status::pending}` filters fine).
