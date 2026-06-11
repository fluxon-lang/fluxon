# fraud.fx — fraud and limit checks for transfers
#
# Daily per-account transfer limit in cents (e.g. 1,000,000 = $10,000.00)
# SPEC GAP: No typed constants or config system in Fluxon beyond env vars.
# We read from env with a fallback.

use db ai env

DAILY_LIMIT_CENTS = str.int (env.DAILY_LIMIT_CENTS ?? "1000000")

# Check that the from_account has not exceeded the daily transfer limit.
# Raises (fail) if over limit. Must be called inside db.tx so that the
# amount counted here is consistent with the transaction being committed.
exp fn check_daily_limit account_id amount_cents
  since = time.ago 1 :day
  result = db.one "select coalesce(sum(t.amount),0) as day_total from transfers t where t.from_account=$1 and t.status='completed' and t.created > $2" [account_id since]
  day_total = result.day_total ?? 0
  if (day_total + amount_cents) > DAILY_LIMIT_CENTS
    fail "daily transfer limit exceeded: account ${account_id} used ${day_total} cents today, limit is ${DAILY_LIMIT_CENTS} cents"

# AI-powered fraud scoring. Returns {score: flt, reasons: [str], flagged: bool}.
# score 0.0 = no risk, 1.0 = certain fraud.
exp fn score_transfer transfer_row from_account to_account
  prompt = "You are a fraud detection system. Analyze this transfer and return a fraud risk score from 0.0 (no risk) to 1.0 (certain fraud) along with reasons. Transfer details: from_account=${from_account.id} owner=${from_account.owner} to_account=${to_account.id} owner=${to_account.owner} amount_cents=${transfer_row.amount} currency=${transfer_row.currency}. Respond with score and array of reasons."
  r = ai.json prompt {
    score: "flt"
    reasons: ["str"]
    flagged: "bool"
  }
  r

# Check fraud score and raise if it is dangerously high (>= 0.9).
exp fn enforce_fraud_check transfer_row from_account to_account
  score_result = score_transfer transfer_row from_account to_account
  if score_result.score >= 0.9
    reasons_str = score_result.reasons.join "; "
    fail "transfer blocked by fraud detection (score ${score_result.score}): ${reasons_str}"
  score_result
