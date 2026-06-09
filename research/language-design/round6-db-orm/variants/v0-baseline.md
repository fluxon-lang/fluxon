### db (Postgres, $DATABASE_URL auto)
```flux
rows = db.q "select * from t where owner=$1" [oid]   # → list of maps
one  = db.one "select * from users where id=$1" [id] # → map or nil
row  = db.ins "orders" {cust:5 status::new}          # → full row (with id)
db.up "orders" {total:1500} {id:oid}                 # {set} {where}
db.del "cart_items" {id:iid}                          # {where}
db.put "memory" {val:v} {agent:a key:k}               # UPSERT (atomic)
```
Params `$1 $2`, values `[...]`. No params: `db.q "select * from links"`.
Aggregate may be nil → `?? 0`: `db.one "select count(*) c, sum(x) s from t"`.

For reads (`db.q`/`db.one`) write raw SQL. Build the WHERE clause yourself; for
a multi-value match use `or` (one `$N` per value — there is no list parameter):
```flux
db.q "select * from bookings where tenant_id=$1 and (status=$2 or status=$3) order by start_at limit $4" [tid :pending :confirmed 50]
```
Group/aggregate is raw SQL too:
```flux
db.q "select resource_id, count(*) c, sum(total_cents) rev from bookings where tenant_id=$1 group by resource_id order by rev desc" [tid]
```

Transaction — atomic, rollback on `fail`/`!`, returns a value:
```flux
res = db.tx \->
  ord = db.ins "orders" {cust:c total:t}
  each it in items
    db.up "products" {stock:it.stock - it.qty} {id:it.id}
  ret ord
```
`db.tx` auto-serializable + retry → "read-check-update" is race-safe (no lock
needed). Idempotency: `uniq` column + ins inside tx (duplicate → rollback):
```flux
old = db.one "select * from txns where ikey=$1" [key]
old ?? (ret old)
db.tx \-> db.ins "txns" {ikey:key ...}   # duplicate → uniq error → rollback
```

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
`sym` column: text in DB, symbol in Flux (auto-converts):
```flux
db.ins "tickets" {status::new}
t = db.one "select * from tickets where id=$1" [id]
match t.status
  :new -> ...
db.q "select * from t where status=$1" [:new]    # filter: symbol → text
```
