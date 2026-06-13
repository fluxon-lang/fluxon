# db_demo.fluxon — db battery demo (SQLite, zero-setup).
# Run (in-memory, nothing to install):
#   DATABASE_URL=sqlite::memory: cargo run -- run examples/db_demo.fx
# or with a file:
#   DATABASE_URL=sqlite:/tmp/fluxon.db cargo run -- run examples/db_demo.fx

use db

# tbl — schema declaration and the SINGLE SOURCE OF TRUTH. When the db opens,
# Fluxon compares `tbl` against the DB's current state (diff) and runs the
# needed DDL itself: a new column → ADD COLUMN, a removed column → DROP COLUMN
# (with backup), an added/removed index → CREATE/DROP INDEX. You only write the
# final shape — no migration SQL to hand-write, and re-deploy is idempotent.
tbl tickets
  id       serial pk
  category sym          # DB: text, Fluxon: symbol
  status   sym index    # filtered often → index (auto name: idx_tickets_status)
  meta     json         # DB: text, Fluxon: map/list

  index(category status)   # multi-column index (space-separated, no commas)

# --- ins: returns the inserted row (RETURNING *) ---
t = db.ins "tickets" {category::billing status::new meta:{tries:0 src:"web"}}
log "inserted id=${t.id} category=${t.category} status=${t.status}"

# a sym column returns a symbol -> match works directly
match t.category
  :billing -> log "  -> billing issue"
  :tech    -> log "  -> technical"
  _        -> log "  -> other"

# a json column returns a map
log "  meta.src=${t.meta.src} meta.tries=${t.meta.tries}"

# --- up: update ---
db.up "tickets" {status::done} {id:t.id}
one = db.one "select * from tickets where id=$1" [t.id]
log "updated status=${one.status}"

# --- q: list; a symbol parameter is auto-converted to text ---
db.ins "tickets" {category::billing status::new meta:{}}
billing = db.q "select * from tickets where category=$1" [:billing]
log "billing tickets count=${billing.len}"

# q without params
all = db.q "select * from tickets"
log "total tickets=${all.len}"

# --- put: upsert (update if present, insert if not) ---
tbl counters
  name  str pk
  label str uniq        # single-column uniq → its own UNIQUE INDEX (auto name)
  hits  int

db.put "counters" {hits:1} {name:"home"}
db.put "counters" {hits:5} {name:"home"}      # exists -> updated
c = db.one "select * from counters where name=$1" ["home"]
log "counter home hits=${c.hits}"             # 5

# --- tx: atomic block, returns the ret value ---
r = db.tx \->
  x = db.ins "tickets" {category::tech status::new meta:{}}
  ret x
log "tx returned id=${r.id}"
# (If a fail/`!` is raised inside tx, the whole block rolls back and the error
#  propagates upward — the rollback tests check this.)

log "DONE"
