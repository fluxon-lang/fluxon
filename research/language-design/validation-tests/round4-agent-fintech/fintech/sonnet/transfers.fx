# transfers.fx — atomic double-entry transfer between accounts
#
# The transfer operation is the most correctness-critical path:
#   1. Idempotency check (return early if key seen before)
#   2. Validate accounts exist, are active, same currency
#   3. Fraud / daily-limit check
#   4. db.tx: debit source, credit destination, update both balance caches,
#      record the transfer row, write audit log — all atomic.
#
# SPEC GAP (concurrency): Fluxon does NOT expose SELECT FOR UPDATE or any row
# locking primitive. The balance check (sufficient funds?) and the balance
# update both happen inside db.tx, but without an explicit row lock the
# database may allow two concurrent transactions to read the same balance
# before either commits. Under high concurrency this can allow overdrafts.
# The risk is mitigated by the DB's serializable isolation (if configured),
# but Fluxon gives us no way to request it explicitly.

use db
use ./ledger as ledger_mod
use ./audit  as audit_mod
use ./fraud  as fraud_mod
use ./idempotency as idem

exp fn execute_transfer from_id to_id amount_cents currency actor idempotency_key
  # ── 1. Idempotency ────────────────────────────────────────────────────────
  existing = idem.find_transfer_by_key idempotency_key
  if existing != nil
    ret {transfer:existing idempotent:true}

  if amount_cents <= 0
    fail "transfer amount must be a positive integer (cents)"

  # ── 2. Validate accounts ─────────────────────────────────────────────────
  from_acct = db.one "select * from accounts where id=$1" [from_id]
  if from_acct == nil
    fail "source account ${from_id} not found"
  if from_acct.status != :active
    fail "source account ${from_id} is not active"

  to_acct = db.one "select * from accounts where id=$1" [to_id]
  if to_acct == nil
    fail "destination account ${to_id} not found"
  if to_acct.status != :active
    fail "destination account ${to_id} is not active"

  # ── 3. Currency check ─────────────────────────────────────────────────────
  if from_acct.currency != to_acct.currency
    fail "currency mismatch: source is ${from_acct.currency}, destination is ${to_acct.currency}"
  if from_acct.currency != currency
    fail "requested currency ${currency} does not match account currency ${from_acct.currency}"

  # ── 4. Daily limit check (outside tx — informational; enforced again inside) ──
  fraud_mod.check_daily_limit from_id amount_cents

  # ── 5. Atomic transfer inside db.tx ──────────────────────────────────────
  result <- nil
  db.tx \->
    # Re-check daily limit inside tx for consistency
    fraud_mod.check_daily_limit from_id amount_cents

    # Debit source (will fail if insufficient funds via adjust_balance)
    ledger_mod.adjust_balance from_id (0 - amount_cents)

    # Credit destination
    ledger_mod.adjust_balance to_id amount_cents

    # Record the top-level transaction
    txn = db.ins "transactions" {
      kind::transfer
      status::completed
      idempotency_key:idempotency_key
    }

    # Two balanced ledger entries (the double-entry invariant)
    debit_entry  = ledger_mod.post_entry txn.id from_id :debit  amount_cents
    credit_entry = ledger_mod.post_entry txn.id to_id   :credit amount_cents

    # Record the transfer record
    transfer = db.ins "transfers" {
      from_account:from_id
      to_account:to_id
      amount:amount_cents
      currency:currency
      status::completed
      idempotency_key:idempotency_key
    }

    # Capture before/after balances for audit
    from_bal_after = ledger_mod.get_balance from_id
    to_bal_after   = ledger_mod.get_balance to_id
    from_before = {account_id:from_id available:(from_bal_after.available + amount_cents)}
    to_before   = {account_id:to_id   available:(to_bal_after.available  - amount_cents)}

    audit_mod.write_audit actor "transfer_debit"  "account:${from_id}" from_before {account_id:from_id available:from_bal_after.available}
    audit_mod.write_audit actor "transfer_credit" "account:${to_id}"   to_before   {account_id:to_id   available:to_bal_after.available}
    audit_mod.write_audit actor "transfer"        "transfer:${transfer.id}" {} {from_account:from_id to_account:to_id amount:amount_cents currency:currency}

    result <- {
      transfer:transfer
      transaction:txn
      debit_entry:debit_entry
      credit_entry:credit_entry
      idempotent:false
    }
  result

# HTTP route for transfers
http.on :post "/transfers" \req ->
  if !req.body.from_account
    ret rep 400 {error:"from_account required"}
  if !req.body.to_account
    ret rep 400 {error:"to_account required"}
  if !req.body.amount
    ret rep 400 {error:"amount required (integer cents)"}
  if !req.body.currency
    ret rep 400 {error:"currency required"}
  if !req.body.idempotency_key
    ret rep 400 {error:"idempotency_key required"}
  actor = req.headers.x_actor ?? "system"
  result = execute_transfer req.body.from_account req.body.to_account req.body.amount req.body.currency actor req.body.idempotency_key
  rep 200 result

http.on :get "/transfers/:id" \req ->
  t = db.one "select * from transfers where id=$1" [req.params.id]
  if t == nil
    ret rep 404 {error:"transfer not found"}
  rep 200 t
