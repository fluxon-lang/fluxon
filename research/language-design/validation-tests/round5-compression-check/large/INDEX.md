# Job Board Platform — File Index

## Overview
Production-grade job matching + hiring platform in Fluxon. 564 lines across 9 modules + schema. Demonstrates transactions, AI integration, WebSocket realtime, cron scheduling, and modular Fluxon architecture.

## Files

### Schema & Core
**schema.fx** (47 lines)
- Database tables: companies, jobs, candidates, applications, notifications
- Column types: serial pk, str, money (cents), sym (enums), now (auto-timestamp), flt
- Relations: jobs → companies, applications → jobs + candidates, notifications → user_id

### Business Logic Modules

**companies.fx** (29 lines)
- `create_company(req)` — POST /companies
- `list_companies(req)` — GET /companies
- `get_company(req)` — GET /companies/:id

**candidates.fx** (54 lines)
- `create_candidate(req)` — POST /candidates
- `get_candidate(req)` — GET /candidates/:id
- `update_candidate(req)` — PUT /candidates/:id
- `list_candidates(req)` — GET /candidates

**jobs.fx** (79 lines)
- `create_job(req)` — POST /companies/:company_id/jobs
- `list_jobs(req)` — GET /jobs (with ?status=, ?search=, ?page=)
- `get_job(req)` — GET /jobs/:id
- `summarize_job(req)` — POST /jobs/:id/summarize (AI-generated candidate-facing summary, cached)

**matching.fx** (38 lines)
- `score_match(job, candidate)` — Uses ai.json to rate resume against job (0-1 + reasons)
- `determine_status(score)` — Routes: > 0.85 → :shortlisted, ≥ 0.6 → :review, < 0.6 → :rejected
- `confidence_reason(conf)` — Explains AI confidence level

**applications.fx** (140 lines)
- `apply_to_job(req)` — POST /jobs/:job_id/apply
  - Atomic transaction: application + AI match + notification
  - Prevents duplicate applies (409 Conflict)
- `get_candidate_applications(req)` — GET /candidates/:cand_id/applications
- `get_job_applications(req)` — GET /jobs/:job_id/applications (ranked by match score)
- `update_application_status(req)` — PUT /applications/:id/status + realtime notification
- `get_application(req)` — GET /applications/:id (full details)

**realtime.fx** (51 lines)
- `setup_websocket()` — WebSocket event handlers
  - `:connect` — initialize conn.data.user_id
  - `:message` → subscribe candidate to `user:<id>` room
  - `:disconnect` → cleanup
- `notify_candidate(user_id, body)` — Send WebSocket message to user room
- `create_notification(user_id, body)` — Persist + broadcast
- `mark_notification_read(notif_id)` — Update DB
- `get_notifications(user_id)` — List notifications for candidate

**cron_jobs.fx** (40 lines)
- `setup_daily_cron()` — Daily 9:00 AM reporting
  - Open jobs count
  - Today's applications
  - Shortlist rate (%)
  - Logs to stdout (can be extended to store in DB)

**main.fx** (86 lines)
- HTTP route registration (POST/GET/PUT across 5 resource types)
- WebSocket setup (9000)
- Cron initialization
- Server startup (HTTP 8080 + WS 9000)

---

## API Surface (26 Endpoints)

### Companies (3)
- POST /companies — create
- GET /companies — list
- GET /companies/:id — detail

### Candidates (4)
- POST /candidates — create
- GET /candidates — list
- GET /candidates/:id — detail
- PUT /candidates/:id — update

### Jobs (4)
- POST /companies/:company_id/jobs — create
- GET /jobs — list with search + status filter + pagination
- GET /jobs/:id — detail
- POST /jobs/:id/summarize — AI summary

### Applications (5)
- POST /jobs/:job_id/apply — apply (atomic + AI match + notification)
- GET /candidates/:cand_id/applications — candidate's applications
- GET /jobs/:job_id/applications — job's applications (ranked by match score)
- GET /applications/:id — application detail
- PUT /applications/:id/status — update status + notify

### Notifications (via WebSocket)
- ws://localhost:9000
- Message: {action: :subscribe, user_id: <id>}
- Receives: {type: :notification, body: "...", timestamp: ...}

---

## Key Technical Features Demonstrated

### 1. Transactions (db.tx)
```fluxon
result = db.tx \->
  app = db.ins "applications" {...}
  db.one "check duplicate" ...!
  rt.create_notification cand_id msg
  ret {application:app}
```
- Atomicity: all-or-nothing
- Auto-rollback on fail/!
- Serializable isolation (no locks needed)

### 2. AI Integration (ai.json + confidence)
```fluxon
match_result = ai.json prompt {score:flt reasons:str}
if match_result._.conf > 0.85
  status = :shortlisted
```
- Type-safe AI responses
- Confidence scoring (0-1)
- Metadata tracking (tokens, cost, latency)

### 3. WebSocket Rooms (realtime broadcast)
```fluxon
ws.room.join conn "user:5"
ws.room.send "user:5" msg
```
- Per-user notification channels
- Automatic presence tracking
- JSON serialization automatic

### 4. Cron Scheduling (cron.dy)
```fluxon
cron.dy 9 0 \day hour minute -> ...
```
- Daily recurring jobs
- Aggregation queries in cron

### 5. Symbol Enums (type-safe statuses)
```fluxon
tbl jobs
  status sym
match status
  :open -> ...
  :closed -> ...
```
- Auto-convert DB string ↔ Fluxon symbol
- Pattern matching on symbols
- Filter queries with symbol params

### 6. Money Type (currency in cents)
```fluxon
salary_min money
salary_max money
```
- Stored as integers (no float precision loss)
- Safe for financial calculations

### 7. Schema Relations (foreign keys)
```fluxon
job_id int ref:jobs.id
```
- Declarative references
- ! operator validates existence on read

### 8. Modular Imports (namespace management)
```fluxon
use ./matching as match_mod
use ./realtime as rt
match_mod.score_match job candidate
rt.create_notification user_id msg
```
- File-based modules
- Aliasing for disambiguation
- Only exp functions exported

---

## Testing Example Sequence

1. **Create Company**
   ```bash
   POST /companies
   {"name": "TechCorp", "website": "https://techcorp.com"}
   → Company ID = 1
   ```

2. **Create Job**
   ```bash
   POST /companies/1/jobs
   {"title": "Backend Engineer", "description": "Rust...", "salary_min": 100000, "salary_max": 150000}
   → Job ID = 1
   ```

3. **Create Candidate**
   ```bash
   POST /candidates
   {"name": "Alice", "email": "alice@ex.com", "skills": "Rust, Postgres", "resume": "10yr..."}
   → Candidate ID = 1
   ```

4. **Apply to Job**
   ```bash
   POST /jobs/1/apply
   {"candidate_id": 1}
   → AI matches (0.87 score) → Status = :shortlisted
   → Notification created + broadcast to ws://localhost:9000 user:1
   ```

5. **Subscribe to Notifications**
   ```
   ws://localhost:9000
   Send: {"action": :subscribe, "user_id": 1}
   Receive: {"type": :notification, "body": "Application status: shortlisted", ...}
   ```

6. **Get Job Summary**
   ```bash
   POST /jobs/1/summarize
   → AI generates: "Seeking experienced Rust backend engineer... [2-3 sentences]"
   → Cached in application_summaries table
   ```

7. **Update Application Status**
   ```bash
   PUT /applications/1/status
   {"status": :accepted}
   → Status updated → Notification sent to candidate
   ```

8. **Cron Report** (daily 9:00 AM)
   ```
   Log: Open jobs: 5, Today's applications: 12, Shortlist rate: 18%
   ```

---

## Lines of Code Breakdown
- schema.fx: 47 (database definitions)
- companies.fx: 29 (CRUD)
- candidates.fx: 54 (CRUD + update)
- jobs.fx: 79 (CRUD + search + AI summary + caching)
- matching.fx: 38 (AI scoring + routing logic)
- applications.fx: 140 (apply with tx + status + notifications)
- realtime.fx: 51 (WebSocket rooms + notifications)
- cron_jobs.fx: 40 (daily analytics)
- main.fx: 86 (route registration + server startup)
- **Total: 564 lines**

---

## Compilation & Execution

```bash
# Assumes Fluxon compiler installed
fluxon build -o job-board main.fx

# Set environment
export DATABASE_URL="postgres://..."
export AI_KEY="sk-..."

# Run
./job-board
# HTTP: localhost:8080
# WebSocket: localhost:9000
```

---

See **README.md** for feature overview and **SPEC_GAPS.txt** for Fluxon specification validation.
