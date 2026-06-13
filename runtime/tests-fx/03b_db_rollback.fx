# 03b - db.tx rollback: a fail inside reverts the whole block + error propagates up.
# This file INTENTIONALLY ends with an error (exit != 0). The outer script
# checks two things: (1) "ROLLBACK-OK" printed (row not added), (2) exit code != 0.
#
# Run: DATABASE_URL=sqlite:/tmp/fluxon_rb_test.db ./target/release/fluxon run tests-fx/03b_db_rollback.fx
# (a file-based db is needed: a write inside tx cannot be checked afterwards once
#  rolled back; in-memory also works since rollback is visible within the same process.)

use db

tbl items
  id   serial pk
  name str

db.ins "items" {name:"original"}
before = (db.q "select * from items").len    # 1

# inside tx: ins + fail -> rollback expected
db.tx \->
  db.ins "items" {name:"ghost"}
  inside = (db.q "select * from items").len   # inside tx shows 2
  log "row count inside tx = ${inside}"
  fail "intentional rollback"

# THIS LINE IS NEVER REACHED - the fail propagates up and the program stops.
log "THIS SHOULD NOT PRINT"
