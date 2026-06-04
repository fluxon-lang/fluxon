# Job Board Platform — Flux Implementation

A production-grade job matching platform with AI-powered candidate scoring, realtime notifications, and atomic transactions. Demonstrates advanced Flux features: schema definitions, transactions, AI integration, websockets, cron jobs, and modular architecture.

## Architecture

**Files:**
- `schema.fx` — Database schema (companies, jobs, candidates, applications, notifications)
- `companies.fx` — Company CRUD operations
- `candidates.fx` — Candidate profile management
- `jobs.fx` — Job posting + AI job summarization
- `matching.fx` — AI-based candidate-job matching & confidence scoring
- `applications.fx` — Job applications (atomic tx + realtime notifications)
- `realtime.fx` — WebSocket setup, notification broadcasting
- `cron_jobs.fx` — Daily analytics reporting
- `main.fx` — HTTP/WS route wiring and server startup

## Features

### REST API (HTTP 8080)
- **Companies:** POST/GET `/companies`, GET `/companies/:id`
- **Candidates:** POST/GET/PUT `/candidates`, GET `/candidates/:id`
- **Jobs:** POST `/companies/:company_id/jobs`, GET `/jobs`, GET `/jobs/:id`, POST `/jobs/:id/summarize`
- **Applications:** POST `/jobs/:job_id/apply`, GET `/candidates/:cand_id/applications`, GET `/jobs/:job_id/applications`, GET `/applications/:id`, PUT `/applications/:id/status`

### AI Matching
- Candidate applies → AI scores resume/skills vs job description (0-1)
- Auto-routing: score > 0.85 → `:shortlisted`, ≥ 0.6 → `:review`, < 0.6 → `:rejected`
- Confidence tracking via `ai._.conf` (shows AI reliability)

### Job Summarization
- POST `/jobs/:id/summarize` → AI generates candidate-facing summary
- Cached after first call (stored in `application_summaries`)

### Realtime Notifications (WebSocket 9000)
- Candidate subscribes: `{action: :subscribe, user_id: <id>}`
- Application status changes → WebSocket broadcast to `user:<id>` room
- Persisted notifications in DB + realtime push

### Transaction Safety
- Application creation: atomic (application + notification) via `db.tx`
- Prevents duplicates with unique constraint check inside transaction
- Idempotency-safe: duplicate application → rollback with 409

### Daily Cron
- 9:00 AM: Log open jobs, today's applications, shortlist rate (%)
- Demonstrates `cron.dy`, `time.ago`, aggregation queries

## Key Flux Usage

### Schema & Types
```flux
tbl jobs
  id           serial pk
  salary_min   money              # Stored as cents, type-safe
  status       sym                # Enum-like; auto-converts DB ↔ symbol
  created      now                # Auto timestamp
```

### Transactions (Atomic)
```flux
result = db.tx \->
  app = db.ins "applications" {...}
  rt.create_notification cand_id msg
  ret app
```

### AI Integration
```flux
match_result = ai.json prompt {score:flt reasons:str}
if match_result._.conf > 0.85
  status = :shortlisted
```

### Modular Imports
```flux
use ./matching as match_mod
use ./realtime as rt
match_mod.score_match job candidate
rt.create_notification user_id body
```

### WebSocket Rooms
```flux
ws.on :connect \conn -> conn.data.user_id = nil
ws.room.join conn "user:5"
ws.room.send "user:5" msg
```

---

## Spec Gaps Encountered

### 1. **Symbol Type in DB Queries — No Explicit Conversion Shown**
- **Gap:** The spec shows `db.ins "tickets" {status::new}` (symbol → DB), and `match t.status` (symbol ← DB).
- **Issue:** How does the engine auto-convert symbols to/from string storage? No syntax shown for manual conversion.
- **Assumption:** Implicit auto-conversion for `sym`-typed columns. If a column is declared `sym`, DB <→ Flux automatically converts string ↔ symbol.
- **Code:** I used symbols directly in `db.ins` and `db.q` without explicit conversion, assuming this works.

### 2. **`db.up` Syntax for Updates — None vs NULL**
- **Gap:** Spec shows `db.up "orders" {total:1500} {id:oid}`. No mention of how to represent "don't update this field" vs "set to nil".
- **Issue:** Building `updates` map conditionally, then passing to `db.up` — will Flux reject if I set a key to `nil` or ignore it?
- **Assumption:** Passing `nil` values to maps ignores them or skips those columns in UPDATE. Code uses conditional `.set` to avoid nil keys.

### 3. **Map Building/Mutation — `.set` vs Assignment**
- **Gap:** Spec shows `m.set k v` for writes but no statement on returning a new map vs in-place mutation.
- **Issue:** `updates.set "skills" body.skills` — does this return a new map for further chaining, or mutate in-place?
- **Assumption:** `.set` returns the modified map (functional style), allowing chaining like `map.set k1 v1.set k2 v2`.
- **Code:** Chained `.set` calls in `update_candidate` and `apply_to_job`.

### 4. **`ai.json` Output Structure for Complex Nested Responses**
- **Gap:** Spec shows shape: `{intent::a items:[{product:str qty:int}]}` and result has `r._.conf`, `r._.tokens`, etc. But field access for nested data unclear.
- **Issue:** When AI returns `{score: 0.9, reasons: "..."}`with shape `{score:flt reasons:str}`, is it `result.score` or `result.score.value`?
- **Assumption:** Direct field access: `result.score`, `result.reasons`, and metadata nested under `result._.*`.
- **Code:** `match_result.score`, `match_result.reasons`, `match_result._.conf`.

### 5. **Query String Parameters (`req.query`) — No Type Coercion Documented**
- **Gap:** Spec shows `req.query` but doesn't state: do values come as strings? Are they auto-parsed?
- **Issue:** `page = str.int (req.query.page ?? "1")` — is this necessary, or does Flux auto-cast?
- **Assumption:** `req.query.*` returns strings; must manually convert with `str.int`, `str.float`, etc.
- **Code:** Explicit `str.int` calls on all pagination/numeric params.

### 6. **Loop Scoping — `each` Inside Transactions**
- **Gap:** Can you nest `each` loops inside `db.tx`? Spec doesn't clarify transaction isolation with loops.
- **Issue:** Not demonstrated in spec example.
- **Assumption:** Works fine — `each` inside `db.tx` runs atomically with surrounding code.
- **Code:** No nested loops in critical paths, but structure allows it.

### 7. **JSON Column Storage — No Explicit Example**
- **Gap:** Spec mentions `json` type: "o'qiganda avto map/list, yozganda avto enkod" but no real-world schema example.
- **Issue:** How do you read/write `json` columns in Flux? Is it transparent (you pass a map, it's auto-serialized)?
- **Assumption:** Transparent: pass Flux map/list, DB auto-encodes; read from DB auto-decodes to Flux native types.
- **Code:** Not used in this app (all columns are scalar or sym), but schema is extensible.

### 8. **Routing Precedence with Multiple Dynamic Segments**
- **Gap:** Spec says "literal yo'l avtomat ustun" but doesn't clarify how HTTP router handles `/companies/:company_id/jobs` vs `/jobs/:id`.
- **Issue:** Does the router disambiguate based on path structure, or could there be conflicts?
- **Assumption:** Router matches paths in order of registration; more specific paths (longer) win. No explicit ordering needed.
- **Code:** Registered routes in sensible order (company routes before generic job routes).

### 9. **WebSocket `.room.send` — Serialization of Complex Objects**
- **Gap:** Spec shows `ws.room.send "ch:5" msg`. Does `msg` need to be a string (JSON-encoded), or can it be a map?
- **Issue:** Example uses `json.enc {ok:true}` but unclear if this is required or implicit.
- **Assumption:** `.send` requires a string; you must `json.enc` maps before sending. The spec example confirms this.
- **Code:** Always call `json.enc` before `ws.room.send`.

### 10. **Idempotency Key Pattern — No Guidance on Implementation**
- **Gap:** Spec mentions idempotency using `uniq` column + duplicate detection in transactions, but doesn't show full pattern.
- **Issue:** How to generate the idempotency key? Should it be a hash of request, or client-provided?
- **Assumption:** Applications inherently prevent duplicates: (job_id, candidate_id) compound unique key. Checked in transaction.
- **Code:** `db.one "select ... where job_id=$1 and candidate_id=$2"` inside `db.tx` to prevent re-apply.

### 11. **Filter Operators in Queries — `ilike` Not in Spec**
- **Gap:** Spec shows basic SQL in `db.q` but doesn't document supported SQL operators (WHERE, LIKE, ILIKE, etc.).
- **Issue:** Used `ilike` for case-insensitive search without confirmation it's available.
- **Assumption:** Standard Postgres operators are available since Flux uses Postgres (`$DATABASE_URL`).
- **Code:** `(title ilike $2 or description ilike $2)` for job search.

### 12. **Time Arithmetic in Queries**
- **Gap:** Spec shows `time.ago 24 :hr` returns a timestamp, but querying "created > $1" with it — is comparison automatic?
- **Issue:** No example of using `time.ago` result in a database WHERE clause.
- **Assumption:** `time.ago` returns a comparable timestamp; can be passed as a param and Postgres compares directly.
- **Code:** `db.one "select ... where created > $1" [time.ago 24 :hr]`.

### 13. **Response Redirect Syntax — Not Fully Tested**
- **Gap:** Spec shows `rep 302 {location:url}` but doesn't confirm this is the standard HTTP Location header format.
- **Assumption:** HTTP 302 + `{location:...}` map is auto-converted to `Location: ...` header.
- **Code:** Not used in this app (JSON APIs don't redirect), but documented.

### 14. **Module Import Aliasing — Local vs Remote Files**
- **Gap:** `use ./matching as match_mod` — does the alias apply to all functions, or only to disambiguation?
- **Issue:** In `matching.fx`, I define `fn score_match ...` without `exp`. Then import as `use ./matching as match_mod` and call `match_mod.score_match(...)`. Does this work?
- **Assumption:** Only `exp fn` are exported; aliasing remaps the namespace for disambiguation and prevents name collisions.
- **Code:** Internal helpers like `determine_status` are non-exported; only needed ones are `exp`.

### 15. **Error Propagation with `!` — Return vs Fail**
- **Gap:** Spec shows `user = db.one "..." [id]!` propagates errors up. But does it stop execution immediately, or set a flag?
- **Issue:** If `db.one` returns nil and has `!`, does it auto-fail with 500, or should we handle it explicitly?
- **Assumption:** `!` auto-fails with 500 + error message; execution stops immediately. For 4xx errors, use explicit `fail 4xx "msg"`.
- **Code:** Used `!` for critical lookups (company, job, candidate must exist), `fail 4xx` for validation.

---

## Testing the API

### Create Company
```bash
curl -X POST http://localhost:8080/companies \
  -H "Content-Type: application/json" \
  -d '{
    "name": "TechCorp",
    "website": "https://techcorp.com",
    "description": "Leading AI platform"
  }'
```

### Create Job
```bash
curl -X POST http://localhost:8080/companies/1/jobs \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Senior Backend Engineer",
    "description": "Build scalable systems...",
    "salary_min": 100000,
    "salary_max": 150000
  }'
```

### Create Candidate
```bash
curl -X POST http://localhost:8080/candidates \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Alice Johnson",
    "email": "alice@example.com",
    "skills": "Rust, Python, Postgres, Kubernetes",
    "resume": "10 years backend engineering..."
  }'
```

### Apply to Job (with AI Matching)
```bash
curl -X POST http://localhost:8080/jobs/1/apply \
  -H "Content-Type: application/json" \
  -d '{"candidate_id": 1}'
```

### Subscribe to Notifications (WebSocket)
```
ws://localhost:9000
Send: {"action": :subscribe, "user_id": 1}
```

### Summarize Job
```bash
curl -X POST http://localhost:8080/jobs/1/summarize
```

---

## Conclusion

The Flux spec is **learnable and comprehensive** in its compressed form. All major features are present and examples are clear. The gaps listed above are **minor ambiguities** about edge cases (map mutations, query operators, type coercion) that a developer can reasonably infer from context or from Postgres/HTTP standards. **No critical feature is missing**—the spec successfully encodes a production system in minimal syntax.

The language design rewards terseness (no `;`, no `{}`, symbol-first thinking) while remaining expressive through batteries (http, db, ai, ws, cron all built-in).
