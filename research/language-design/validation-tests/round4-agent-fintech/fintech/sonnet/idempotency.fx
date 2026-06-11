# idempotency.fx — idempotency key check-and-lock helpers
#
# SPEC GAP NOTE: Fluxon provides db.ins but does NOT expose INSERT ... ON CONFLICT
# or SELECT FOR UPDATE. True atomic check-and-insert on a unique key requires
# the database to enforce uniqueness; we rely on the `idempotency_key uniq`
# constraint on `transactions` and `transfers` tables. If db.ins raises an error
# because the key already exists, the caller must catch that — but Fluxon has NO
# try/catch. We work around this by doing a db.one lookup BEFORE the insert. This
# introduces a TOCTOU race window. Under concurrent load two identical keys could
# both pass the lookup and race to insert; only one will succeed (DB unique
# constraint), the other will receive an unhandled runtime error. The correct fix
# would be upsert / ON CONFLICT DO NOTHING with a returned flag, which Fluxon does
# not expose. This is documented in the spec-gaps section.

use db

# Returns the existing transaction row if key was already used, else nil.
exp fn find_txn_by_key key
  db.one "select * from transactions where idempotency_key=$1" [key]

# Returns the existing transfer row if key was already used, else nil.
exp fn find_transfer_by_key key
  db.one "select * from transfers where idempotency_key=$1" [key]
