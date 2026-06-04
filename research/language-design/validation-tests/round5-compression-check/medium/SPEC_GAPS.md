# Spec Gaps I Hit

Implementing a complete polls/survey API with AI revealed the following gaps in the Flux spec:

## 1. **Module Import Path Resolution (CRITICAL)**
**Gap:** Spec shows `use ./tools` and `use ./ai as helper` for local file imports, but does NOT specify:
- Whether `.fx` extension is implicit or explicit
- Whether relative paths work with or without extension
- What happens if import fails (should it error, or treat as missing module?)
- Can you import the same file twice? Does it re-execute or cache?

**What I Assumed:** `.fx` extension is implicit; relative paths are file-system relative; import once per program.

**Code:** `use ./schema`, `use ./models`, `use ./api` in multiple files.

---

## 2. **Variable Shadowing Across Modules (AMBIGUOUS)**
**Gap:** When module A imports B which imports C, and A also defines a function, can they have the same name without conflict? The spec says "module functions accessed via `.` notation" but doesn't clarify:
- Namespace collision rules
- Whether function names are globally unique or scoped per module
- Behavior if two modules export a function with the same name

**What I Assumed:** Module prefixing prevents collisions (e.g., `models.create_poll`).

---

## 3. **Cron Function Signature (AMBIGUOUS)**
**Gap:** Spec shows `cron.hr 30 fn` but does NOT specify:
- Function signature: should `fn` take `(hr min sec)` parameters? Or take `(min)` for hourly? Or no parameters?
- Line 226: `cron.wk :sun 18 0 fn (kun soat daqiqa)` suggests parameters `(day hour minute)`, but unclear if ALL cron tasks receive these or only some

**What I Assumed:** Hourly task gets `(hr min)` parameters based on the pattern shown.

**Code:** `fn log_hourly_stats hr min` — assumed it receives hour/minute when triggered.

---

## 4. **ai.ask Return Type (UNDERSPECIFIED)**
**Gap:** Spec says `ai.ask "question"` returns text, but does NOT specify:
- Whether it's just a string or a map with metadata?
- Spec says `r._.conf` exists for `ai.json` output, but does `ai.ask` also have `._.conf`/`._.tokens`/`._.cost`?
- If return is just string, how do you access metadata?

**What I Assumed:** `ai.ask` returns plain string (no metadata). `ai.json` returns map with `._.*` fields.

**Code:** Used `ai.ask` for summaries directly, didn't try to access confidence.

---

## 5. **Function Return Type on Error (UNDERSPECIFIED)**
**Gap:** When you call a function that does `fail 404 "not found"`, what happens?
- Does the calling function also fail immediately?
- Does it propagate as exception or return special value?
- Can you catch/handle `fail`?

**What I Assumed:** `fail` immediately returns HTTP error to client; calling function doesn't continue.

**Code:** `poll = db.one "..." [id]!` — using `!` to assume success or `fail 404`.

---

## 6. **Transaction Rollback on fail (IMPLICIT)**
**Gap:** Spec says `db.tx` rolls back on `fail` or `!` (error propagation), but:
- Does a failed `db.ins` inside `db.tx` automatically rollback?
- If a function called inside `db.tx` does `fail`, does it rollback the whole transaction?
- What about nested `db.tx`?

**What I Assumed:** Any `fail` inside `db.tx` rolls back entire transaction atomically.

**Code:** `models.cast_vote` uses `db.tx` with two operations; if vote response fails, vote count doesn't increment.

---

## 7. **each Loop with Mutable References (AMBIGUOUS)**
**Gap:** Spec shows `each item in list` but in my code, I do:
```flux
each p in polls
  p.total_votes <- db.one "select sum(votes) v from options..." [p.id]
```
**Question:** Can you mutate objects inside `each`? Does `p` refer to the original list element or a copy?

**What I Assumed:** `p` is a reference; mutations are visible after the loop.

---

## 8. **nil Coalescing with Complex Expressions (WORKS BUT STYLE UNCLEAR)**
**Gap:** Spec shows `a ?? b` for null-coalesce, but my code uses:
```flux
option_id:req.body.option_id ?? (fail 400 "...")
```
**Question:** Can `??` chain with `fail` on the right side? Is this idiomatic?

**What I Assumed:** Yes; the right side of `??` can be any expression including `fail`.

---

## 9. **Map Literal with nil Values (UNDERSPECIFIED)**
**Gap:** When building a response, can you return:
```flux
ret {id:poll.id question:poll.question}
```
If `poll.question` is nil, does the key appear in JSON with null, or is it omitted?

**What I Assumed:** Key appears in JSON with null value.

**Code:** All response objects include all fields; nil values become null in JSON.

---

## 10. **String Interpolation Type Coercion (WORKS BUT IMPLICIT)**
**Gap:** Spec shows `"$x"` interpolation but doesn't say:
- What if `x` is not a string? Does it auto-convert?
- Integer, float, bool — all converted to string representation?

**What I Assumed:** All types auto-convert to string in interpolation.

**Code:** `"${opt.votes}"`, `"${poll_id}"` — used directly without `str.str(...)`.

---

## 11. **Symbol Literals in Queries (NEEDS CLARIFICATION)**
**Gap:** Spec shows `[:new]` passed to `db.q`, converting symbol to DB text. But:
- Is this automatic in ALL contexts, or only for `$1 $2` parameters?
- What if you do string comparison with symbol in SQL?

**What I Assumed:** Symbol is auto-converted to text string when passed as parameter.

**Code:** `db.q "select * from t where status=$1" [:open]` — `:open` becomes `"open"` in SQL.

---

## 12. **Metadata Field Access with Undefined Values (MINOR)**
**Gap:** Code does `r._.conf` assuming response has `_` field, but what if:
- Response is just a string (for `ai.ask`)? Does it have `._`?
- Accessing undefined field returns nil?

**What I Assumed:** Only `ai.json` returns metadata; attempting to access `._` on `ai.ask` string would fail (but I avoid this).

---

## Summary

**Most critical gaps:**
1. Module import details (extension, caching, conflicts)
2. Cron function signatures
3. ai.ask metadata vs ai.json
4. Mutable object references in loops

**Workaround approach:**
Implemented with reasonable defaults based on other languages' conventions and spec patterns. All 7 Flux files compile assuming standard semantics.
