# ledger.fx — core double-entry ledger operations
#
# SPEC GAP NOTE: All ledger mutations MUST be called from within a db.tx block
# to be atomic. Fluxon db.tx rolls back on any fail/! inside the block, which
# gives us atomicity. However, there is no SELECT FOR UPDATE exposed by Fluxon,
# so concurrent transfers updating the same balance row can race. We perform an
# optimistic balance read + update inside db.tx; the DB's row-level locking at
# the UPDATE statement level provides the last line of defense, but the balance
# check (sufficient funds?) is not atomic with the update without row-locking.
# Risk: two concurrent withdrawals could both pass the balance check before
# either commits. This is documented in spec-gaps.

use db
use ./audit as audit_mod

# Post a single ledger entry (must be called inside db.tx).
# direction: :debit or :credit
# amount: integer minor units (cents)
exp fn post_entry txn_id account_id direction amount
  if amount <= 0
    fail "ledger entry amount must be a positive integer (cents)"
  db.ins "ledger_entries" {
    transaction_id:txn_id
    account_id:account_id
    direction:direction
    amount:amount
  }

# Compute the true balance for an account directly from ledger (the invariant).
# Returns {credits: int, debits: int, net: int} all in integer cents.
exp fn compute_balance account_id
  credits_row = db.one "select coalesce(sum(amount),0) as total from ledger_entries where account_id=$1 and direction='credit'" [account_id]
  debits_row  = db.one "select coalesce(sum(amount),0) as total from ledger_entries where account_id=$1 and direction='debit'"  [account_id]
  credits = credits_row.total ?? 0
  debits  = debits_row.total  ?? 0
  {credits:credits debits:debits net:(credits - debits)}

# Get the cached balance row for an account (fast path).
exp fn get_balance account_id
  db.one "select * from balances where account_id=$1" [account_id]

# Ensure a balance row exists (upsert-like: insert if missing).
# SPEC GAP: No upsert in Fluxon. We do a read-then-insert which is racy on
# first-time creation if two requests race. Only affects initial setup.
exp fn ensure_balance account_id
  existing = db.one "select id from balances where account_id=$1" [account_id]
  if existing == nil
    db.ins "balances" {account_id:account_id available:0 pending:0}

# Adjust the cached balance by a signed integer delta for available.
# Must be called inside db.tx.
exp fn adjust_balance account_id delta
  bal = db.one "select * from balances where account_id=$1" [account_id]
  if bal == nil
    fail "balance row missing for account ${account_id}"
  new_available = bal.available + delta
  if new_available < 0
    fail "insufficient funds: account ${account_id} available ${bal.available} cents, attempted delta ${delta}"
  db.up "balances" {available:new_available} {account_id:account_id}
