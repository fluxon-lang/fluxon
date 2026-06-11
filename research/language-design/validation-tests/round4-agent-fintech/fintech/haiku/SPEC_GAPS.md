# Spec Gaps & Workarounds in Fintech Backend

## Critical Gaps Encountered

### 1. **Integer Money Math — Absence of Decimal Type**
**Problem:** The spec provides `int` and `flt` (float) but NO decimal/bigint type. Financial systems must NEVER use floats for money due to rounding errors. The spec says nothing about overflow or precision.

**What I did:**
- Used `int` for all amounts in **cents** (minor currency units)
- Stored `balance_cents`, `amount_cents` throughout
- Assumed 64-bit integers (typical for int), which safely handles ~$92M per account
- Assumed no currency subdivision finer than cents (works for USD, EUR, etc; would fail for Bitcoin/satoshis)

**Risk:** If amounts exceed int64 max or if a currency uses smaller units (e.g., millicents), overflow is silent. The spec provides no `checked_add` or overflow detection.

---

### 2. **Transaction Atomicity & Rollback — Unclear Guarantees**
**Problem:** The spec shows `db.tx` syntax but does NOT document:
- What exactly triggers rollback (caught errors? explicit fail?)
- Whether database constraints are checked within tx
- Whether concurrent transactions on same row are serialized or can race
- Whether a partial write failure leaves the tx in limbo

**What I did:**
- Assumed `fail` or `!` inside tx causes full rollback
- Assumed `db.ins`, `db.up`, `db.del` within tx are atomically applied or fully rolled back
- Treated tx as Postgres-like: ACID, serializable isolation (safest assumption)
- Did NOT rely on database constraints (UNIQUE, NOT NULL) for correctness because spec doesn't guarantee they're enforced within tx

**Risk:** If Fluxon's tx is actually just a session/context wrapper without real atomicity, concurrent transfers could lead to:
- Double-spending (both transfers see balance = 1000, both subtract, final = -1000)
- Orphaned ledger entries (debit created, tx fails, credit never created)

---

### 3. **Concurrency & Row-Level Locking — No Language Support**
**Problem:** Spec provides no `SELECT FOR UPDATE`, `LOCK TABLE`, `atomic_decrement`, or concurrency primitives. For racing transfers:

```
Thread A: balance = 1000, wants to transfer 600
Thread B: balance = 1000, wants to transfer 600
Both pass balance check, both transfer → balance = -200 (WRONG)
```

**What I did:**
- Relied on `db.tx` to serialize everything (hope it's SERIALIZABLE isolation)
- Inside transfer, I check balance, then immediately insert ledger entries, all in ONE tx
- Assumed this ordering is atomic: check → debit → credit → ledger update
- Did NOT implement explicit locking because Fluxon spec provides no syntax for it

**Risk:** CRITICAL. If tx isolation is weaker than SERIALIZABLE, transfers can race. This is the #1 correctness issue.

---

### 4. **Idempotency Key Check-and-Insert Race**
**Problem:** For idempotency (same request key → same result, no double-charge), I need:
```
if idempotency_log[key] exists:
  return cached_result
else:
  insert(key, result)  ← race: another thread inserts between check and insert
```

The spec provides no `INSERT ... ON CONFLICT` or atomic `compare_and_swap`.

**What I did:**
```
existing = idempotency.check_and_lock key  # queries DB
if existing → return it
# else proceed to transfer (RACE WINDOW HERE)
db.ins "idempotency_log" ...
```

Added DB-level UNIQUE constraint on idempotency_key (assuming the schema file works) so the second thread's insert fails. But the spec doesn't say how this is handled — does `db.ins` raise an error? Return nil? Return a tuple?

**Risk:** CRITICAL. If constraint violation doesn't raise an error, two concurrent requests with same key could both execute transfers.

---

### 5. **Type Conversion & String-to-Int Parsing**
**Problem:** HTTP query/body params are always strings. The spec has `str.int s` to parse, but doesn't say:
- What if `str.int "notanumber"`? Raises error? Returns nil? Returns 0?
- Does it raise an exception that propagates or fail-safe?

**What I did:**
- Assumed `str.int s` returns an int or fails (throws)
- Wrapped all conversions in explicit `if !value` checks
- Manually type-checked before using (e.g., `if from_id == to_id`)

**Risk:** If `str.int` returns 0 for bad input (silent), transfers could go to account 0. I added guards, but spec doesn't guarantee safety.

---

### 6. **Nil vs Empty vs Error — No Unified Error Model**
**Problem:** Spec has `??` (nil coalesce) and `!` (error propagation) but no structured errors. Functions can return:
- nil (not found? or actually nothing?)
- bool (success/fail?)
- map with error key? (guess)

**What I did:**
- Functions return nil if not found (e.g., `get_account`)
- Functions return a map with `{blocked:bool reason:str}` for fraud checks (guess)
- Functions fail loudly for invariant violations (e.g., `fail "currency mismatch"`)
- HTTP handlers return JSON with `{error:str}` (standard REST, not spec)

**Risk:** No caller can reliably distinguish between "not found", "validation failed", and "database error". A middleware would need to catch all, but spec has no middleware/exception handling.

---

### 7. **Constraints & Invariants — No Language-Level Enforcement**
**Problem:** Double-entry accounting depends on: every debit has a matching credit, no orphaned entries, balance = sum(ledger). The spec provides no `assert`, `invariant`, or compile-time checks.

**What I did:**
- Created matching debit + credit ledger entries in single tx
- Added daily reconciliation cron to detect discrepancies post-facto
- Assumed database constraints exist (but spec doesn't define them explicitly)
- NO COMPILE-TIME CHECKS: if someone later edits ledger_entries table directly, invariants silently break

**Risk:** Data corruption is undetectable until reconciliation runs. In production, this is fine (banks do this). But the spec doesn't provide tools to PREVENT violations, only detect them.

---

### 8. **Floating-Point in Fraud Scoring**
**Problem:** I need fraud risk scores 0.0..1.0, but `flt` type's precision is unspecified. Also, accumulating floats:
```
risk_score <- 0.1
risk_score = risk_score + 0.2  # now 0.3, or 0.30000000001?
```

**What I did:**
- Used `flt` for risk_score (0..1)
- Added cap at 1.0 to prevent accumulation errors
- Returned floats to AI module and HTTP (hoping IEEE 754 is close enough)

**Risk:** Fraud decisions might drift (score 0.94999 wrongly blocked because 0.95 > 0.94999). Better to multiply by 100 and use int percentiles, but this is a workaround, not spec-mandated.

---

### 9. **JSON Serialization of State — Spec Doesn't Define Schema**
**Problem:** Audit log stores `before_json` and `after_json` as JSON blobs. The spec has `json.enc v` but doesn't say:
- What's the schema?
- How are nested objects handled?
- Are symbols preserved or stringified?
- Can a deeply nested object > max JSON size?

**What I did:**
- Used `json.enc acc` to serialize account/transfer objects
- Assumed all fields are JSON-serializable (ints, strs, bools)
- Assumed symbols are stringified ("active" not :active in JSON)
- Assumed no size limits

**Risk:** If JSON schema mismatches or sizes exceed limits, audit becomes useless. No schema validation is available in the spec.

---

### 10. **Shared Mutable State for Idempotency Cache**
**Problem:** I declared `idempotency_cache <- {}` (mutable) at module level for fast lookups. The spec doesn't say:
- Are module-level bindings truly mutable across all requests?
- Does each request get its own scope or share state?
- Is concurrent mutation safe?

**What I did:**
- Treated it as a shared cache (hope it works)
- Added DB lookup as fallback
- Used `.set k v` to "mutate" (which returns new map, not true mutation)

**Risk:** If each HTTP request is isolated, the cache is useless. If global mutable state exists but isn't thread-safe, concurrent requests corrupt it. The spec is silent on both.

---

### 11. **Date/Time Formatting — `time.fmt` Unspecified**
**Problem:** I used `time.fmt time.now "%Y-%m-%d"` for reconciliation date, but the spec example doesn't show the format string syntax. Is it strftime? Go time? Custom?

**What I did:**
- Assumed standard strftime (`%Y-%m-%d`)
- Provided the format explicitly

**Risk:** If `time.fmt` uses different syntax (e.g., "2006-01-02" Go style), reconciliation dates will be wrong or error.

---

### 12. **No Prepared Statements for SQL Injection — Only Parameterization**
**Problem:** The spec shows `$1 $2` params, which are parameterized (safe). But I have no way to:
- Validate table/column names (e.g., in dynamic schema)
- Reject malicious params at compile time

**What I did:**
- Used only hardcoded table/column names
- All user input passed as params (`$1`), never interpolated into SQL

**Risk:** Safe from SQL injection but no verification in the spec.

---

### 13. **Error Propagation with `!` — Unclear for Partial Failures**
**Problem:** If `db.q` inside a loop fails on the 3rd row, does `!` propagate the error? Is the list partially populated?

**What I did:**
- Assumed `!` stops execution immediately and bubbles up
- Assumed tx rollback undoes everything
- Did NOT use `!` in loops, only on single queries

**Risk:** If `!` is ignored or delayed, partially-executed loops corrupt data.

---

## Summary Table

| Gap | Severity | Impact | Workaround |
|-----|----------|--------|-----------|
| No decimal type | CRITICAL | Silent overflow, rounding errors | Use int cents, cap at int64 |
| No atomicity guarantees | CRITICAL | Double-spending, orphaned entries | Assume SERIALIZABLE tx |
| No row locking | CRITICAL | Racing transfers | Rely on tx serialization |
| No atomic check-insert | CRITICAL | Idempotency key races | Add DB UNIQUE constraint |
| No error types | HIGH | Silent failures, wrong semantics | Manual checking, assume fail propagates |
| No invariant checking | HIGH | Data corruption undetected | Daily reconciliation |
| Type conversion unclear | MEDIUM | Silent zeroing, wrong conversions | Add guards |
| Float precision | MEDIUM | Fraud score drift | Cap manually |
| JSON schema unspecified | MEDIUM | Audit unreliable | Assume JSON serializes all |
| Mutable global state | MEDIUM | Cache isolation unknown | Assume shared, fall back to DB |
| Time formatting | LOW | Wrong dates | Assume strftime |
| Error propagation in loops | MEDIUM | Partial execution | Avoid `!` in loops |

## What the Spec Does Well

1. ✅ **Pipe operator** (`|>`) makes data flow readable
2. ✅ **Lambda shorthand** (`\x ->`) is concise
3. ✅ **Parameterized queries** prevent SQL injection
4. ✅ **batteries included** (http, db, ai, json, cron) — no dependency hell
5. ✅ **Symbols** (`sym` type) make enums safer than strings
6. ✅ **Early return** (`ret`) reduces nesting
7. ✅ **Mutable bindings** (`<-`) are explicit (not auto-promoted like some languages)

## Recommendations for Spec v2

1. **Add `decimal` type** for money: `100_000_cents` literal syntax
2. **Document `db.tx` isolation level**: SERIALIZABLE? READ_COMMITTED? Row locks?
3. **Add `try/catch` or `Result` type**: Distinguish errors from nil
4. **Provide `INSERT ... ON CONFLICT` equivalent**: Atomic idempotency
5. **Clarify `str.int` on parse failure**: Error vs nil vs 0
6. **Document type conversion rules**: esp. symbol ↔ string in queries
7. **Specify JSON schema validation**: enforce types before storing
8. **Clarify scope of mutable state**: request-scoped vs global?
9. **Add assertions/invariants**: compile-time or runtime checks
10. **Document number overflow semantics**: silent wrap? exception? saturate?

---

**Written by**: Claude Haiku 4.5  
**Date**: 2026-06-04  
**Files**: 9 Fluxon modules + 1 schema file + HTTP server  
**Correctness Level**: Production-ready IF spec guarantees hold; HIGH RISK if tx/concurrency/type conversion deviate
