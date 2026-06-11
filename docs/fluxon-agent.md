# Fluxon — language spec (for AI)

Fluxon: AI-native backend language. One task = one way. Few tokens. Batteries-included.
File extension: `.fx`. Read once, write correct Fluxon code.

## Basics
- Comment `# to end of line` (no `//`). Statement on a new line (no `;`).
- Block = indentation (2 spaces), no `{}`.
- These are keywords — never name a var/loop/param after one (e.g. `each exp in xs`
  fails; use `e`): `as each elif else exp fail fn if in inf match ret skip stop tbl use`
```fluxon
if x > 0
  log "positive"
log "outside"
```

## Types
```
42 int · 3.14 flt · "hi" str · true bool · nil · :ok sym (enum/tag)
[1 2 3] list · {a:1 b:2} map        # NO COMMAS, space-separated
bytes — binary data (no literal; comes from fs.readb / crypto.b64db / bytes.of)
```
Str interpolation: `"$x"` (bare var only) or `"${expr}"` (any expr — `.field`, calls).
Multi-line str: `"""` block (prompts, SQL, templates). Content starts on the NEXT line;
common indentation is stripped; closing `"""` on its own line → no trailing `\n`.
Interpolation and escapes work as in `"..."`; `"` needs no escape inside.
```fluxon
prompt = """
  Extract intent from: ${text}
  Answer as JSON: {"intent": "..."}
  """
```
Symbol→text (interp, `str.str`, `+`, `log`) drops the `:` prefix: `str.str :ok` → `"ok"`. Inside a list/map it keeps `:` (`[:a]` → `[:a]`).
Truthy: only `nil`/`false` are false.

## Bindings
```
x = 10              # immutable (DEFAULT)
total <- 0          # mutable; reassign: total <- total + 5
```

## Operators
```
+ - * / %      arithmetic. + also concatenates STRINGS: "a"+"b"→"ab"
== != < <= > >=   ·   & | !  (and/or/not)
??   null-coalesce: a ?? b → a, or b if a is nil
.    member/index: m.key, l.0, l.len, m[k], l[i], l.(i)  (i — computed index)
..   range: 1..5 → [1 2 3 4 5]   ·   |>  pipe: x |> f |> g
```

## Functions
```fluxon
fn add a b
  ret a + b               # ret (early) or last expression (implicit)
fn double x -> x * 2      # one-liner
add 2 3                   # paren-free call; parens only group: f (g x)
fn new_id -> rand.str 8   # no params
new_id()                  # nullary call (empty parens REQUIRED to call)
new_id                    # NOT a call — the function VALUE (for callbacks/reg)
\x -> x * 2               # lambda
```
`ret` works INSIDE a lambda too — guard-clause (instead of deep nesting):
```fluxon
http.on :post "/x" \req ->
  if !req.body.email
    ret rep 400 {error:"email required"}
  rep 201 (db.ins "t" {...})
```

## Control flow
```fluxon
if x > 0
  log "a"
elif x == 0
  log "b"
else
  log "c"
```
Inline `if` = expression (ternary): `pad = if h < 10 ("0" + str.str h) else (str.str h)`.
`else` is required. Calls in the condition need parens: `if (str.len s) > 0 a else b`.
Only loop = `each` (no while/for):
```fluxon
each item in list   ·   each i in 1..5   ·   each k, v in map   ·   each i in inf
```
In a loop: `skip` (continue), `stop` (break). `each i in inf` = infinite loop
(i = 0,1,2,...) for REPL / event loops / "repeat until `stop`". `inf` is ONLY
valid as the `each` iterator — not a value.

`match` — value dispatch (symbol/number ONLY, NOT boolean conditions):
```fluxon
match status
  :new -> log "new"
  _ -> log "default"
```
For boolean conditions (`x > 0.85`) ALWAYS use `if/elif/else`. `match true` = error.

## Errors
```fluxon
user = db.one "..." [id]!     # ! = on error, auto-propagate up
name = user.name ?? "guest"   # ?? = alternative if nil
fail 422 "insufficient funds" # with status → 422 {error:...} to client
fail "internal error"         # no status → 500
```
`!` propagate, `??` replace nil, `fail [status] "..."` raise. No try/catch —
`fail 4xx` auto-converts an expected error into an HTTP response (code stays flat).

## Modules
```fluxon
use http db ai json     # batteries, no install
use ./tools             # your file → tools.fn
use ./ai as helper      # ALIAS: on clash with a battery name → helper.fn
exp fn create_order ... # exp = expose externally
```

## Batteries (stdlib — no install)

### http
```fluxon
http.on :post "/notes" \req -> rep 201 {ok:true}
http.on :get "/notes/:id" \req -> rep 200 {id:req.params.id}
http.serve 8080
```
- Method: `:get :post :put :patch :del`. `req.body` (JSON→map), `req.params.id`,
  `req.query`, `req.headers`. Missing key → `nil`. Reading repeated headers
  (req and res): values joined `", "` (`cookie`: `"; "`); repeated `set-cookie`
  in `res.headers` → list.
- `rep status body` (map→auto JSON). Redirect: `rep 302 {location:url}`.
- Custom headers: optional 3rd arg map — `rep 200 "<h1>" {content_type:"text/html"}`.
  Key `_`→`-` (`content_type`→`Content-Type`); name case-insensitive. Repeated
  header (multiple Set-Cookie): list value — `rep 200 nil {set_cookie:["a=1" "b=2"]}`.
- Route priority: a literal path auto-wins (`/stats/:c` > `/:c`).
- Middleware (runs before handlers, in declaration order): `http.use \req -> ...`
  (all routes) or `http.before "/api/*" \req -> ...` (path prefix; `*` only at end,
  `"/api/*"` matches `/api` and below at segment boundary; no `*` → exact path).
  If a middleware returns `fail`/`rep`, the chain stops and that response is sent
  (e.g. auth reject) — handler not called.
- Request-scoped context: `req.ctx <- {tenant_id:5 role:"admin"}` (middleware
  writes), `ctx = req.ctx` (handler reads). Lives for THIS request, shared between
  middleware and handler — compute auth once, not per handler. Per-request, isolated.
- `req.ip` — client IP (TCP peer; behind a proxy this is the proxy's IP).
- CORS: `http.cors "*"` (any origin, dev) or `http.cors ["https://app.example.com"]`
  (allowlist), optional `{creds:true}` (cookies/Authorization; with `"*"` the
  response echoes the request Origin, as browsers require). `OPTIONS` preflight is
  answered automatically (204 + `Access-Control-Allow-*`); every response (incl.
  404) gets the headers. Opts: `methods`/`headers` (str) and `max_age` (seconds).
- Rate limit: `http.limit N :sec|:min|:hr \req -> key` (declared like middleware,
  runs in order; an optional leading path scopes it like `http.before`):
  `http.limit 100 :min \req -> req.ctx.tenant_id` (per-tenant, all routes),
  `http.limit "/api/*" 100 :min \req -> req.headers.x_api_key` (per-key, prefix).
  Over the limit → auto `429` + `Retry-After` (seconds until window resets). Key fn
  nil → falls back to `req.ip`. Fixed-window, in-memory (single instance only).
- Body size limit (DoS guard): `http.serve 8080 {max_body: BYTES}`. Default 10 MiB;
  over the limit → `413 Payload Too Large` (body not buffered). `max_body: 0`
  disables the limit (unlimited — only behind a trusted internal network).
- File upload (`multipart/form-data`): files → `req.files` (list of
  `{name filename content size}`), plain form fields → `req.body` (map, symmetric
  with JSON). `content` is str for UTF-8 text, bytes for binary; `size` is BYTE
  count. `req.files` is always a list (empty when not multipart). `max_body`
  applies to multipart too.
  `f = req.files.0` → `fs.write f.filename f.content` saves an upload.
```fluxon
http.before "/api/*" \req ->
  if !req.headers.authorization
    fail 401 "auth kerak"
  req.ctx <- {tenant_id: 5 role: "admin"}
http.on :get "/api/me" \req ->
  ctx = req.ctx
  rep 200 {tenant: ctx.tenant_id}
```
- Client: `http.get url`, `http.post url body` → `res.status res.body res.headers`.
  `res.headers` (map, lowercase keys): `res.headers.location`, also `m[k]`.
  Redirects not followed by default; opt-in: `http.get url {follow:true max:10}`
  → follows, `res.hops` (hop count). `max` default 10.
  Custom request header: `{headers:{"x-api-key":KEY "anthropic-version":"2023-06-01"}}`
  (symmetric with req/res.headers; a user value overrides the auto `content-type`).
  Request timeout (default 30s): `http.get url {timeout: 5}` (seconds); a hung
  upstream → error instead of blocking forever. `timeout: 0` disables (trusted only).
  Server also enforces a 30s header-read timeout (slowloris guard).

### db (Postgres, $DATABASE_URL auto)
```fluxon
row  = db.ins "orders" {cust:5 status::new}          # → full row (with id)
db.up "orders" {total:1500} {id:oid}                 # {set} {where}
db.del "cart_items" {id:iid}                          # {where}
db.put "memory" {val:v} {agent:a key:k}               # UPSERT (atomic)
```

Reads — a query builder, piped with `|>`. `db.from "t"` starts; `db.all` → list,
`db.first` → one row or nil. NO raw SQL for ordinary filters:
```fluxon
rows = db.from "bookings" |> db.eq {tenant_id:tid} |> db.all
one  = db.from "bookings" |> db.eq {id:bid tenant_id:tid} |> db.first
```
Stages (each takes the query, returns the query — chain freely):
```fluxon
db.eq {col:val ...}        # equality, AND-ed. A LIST value → IN (...)
db.cmp :col :ge t          # one comparison: op ∈ :gt :ge :lt :le :ne :like
db.order :col   ·   db.order :col :desc
db.limit n   ·   db.offset n
```
```fluxon
db.from "bookings"
  |> db.eq {tenant_id:tid status:[:pending :confirmed]}   # status IN (..)
  |> db.cmp :start_at :ge t0  |> db.cmp :start_at :lt t1
  |> db.order :start_at |> db.limit 50 |> db.offset 0
  |> db.all
```
Aggregation — set output columns, then `db.agg` (grouped → list) or `db.agg_row`
(one summary row):
```fluxon
db.from "bookings" |> db.eq {tenant_id:tid status:[:done :confirmed]}
  |> db.group :resource_id |> db.count :n |> db.sum :total_cents :rev
  |> db.order :rev :desc |> db.agg          # → [{resource_id:5 n:12 rev:48000} ...]
```
Agg stages: `db.count :out` · `db.sum/avg/min/max :col :out` · `db.group :col`.
Conditional aggregates (status-filtered counts/sums in ONE query — for overviews):
```fluxon
db.from "bookings" |> db.eq {tenant_id:tid}
  |> db.count_if {status::confirmed} :confirmed
  |> db.sum_if :total_cents {status::done} :revenue
  |> db.agg_row     # → {confirmed:7 revenue:91000}
```
A list-of-symbols filter from a query string (`?status=a,b`):
`(str.split q.status ",").map \s -> str.sym s` → `db.eq {status:syms}`.

`db.q`/`db.one` stay as the escape hatch for what the builder can't express —
multi-table JOINs and raw expressions like `date()`. POSITIONAL `$1` ONLY
(never `:name` inside these):
```fluxon
rows = db.q "select * from t where owner=$1" [oid]   # → list of maps
one  = db.one "select * from users where id=$1" [id] # → map or nil
db.q "select date(created) day, count(*) n from bookings where tenant_id=$1 group by day order by day" [tid]
```
No params: `db.q "select * from links"`.

Transaction — atomic, rollback on `fail`/`!`, returns a value:
```fluxon
res = db.tx \->
  ord = db.ins "orders" {cust:c total:t}
  each it in items
    db.up "products" {stock:it.stock - it.qty} {id:it.id}
  ret ord
```
`db.tx` auto-serializable + retry → "read-check-update" is race-safe (no lock
needed). Idempotency: `uniq` column + ins inside tx (duplicate → rollback):
```fluxon
old = db.one "select * from txns where ikey=$1" [key]
old ?? (ret old)
db.tx \-> db.ins "txns" {ikey:key ...}   # duplicate → uniq error → rollback
```

Schema = `tbl`:
```fluxon
tbl products
  id     serial pk
  owner  int ref:users.id
  price  money              # money = integer minor unit (cents), NOT float
  status sym index|uniq     # multiple modifiers on one column → pipe `|`
  ts     now

  index(owner status)       # multi-column index, space-separated (no commas)
  uniq(owner price)         # multi-column unique
```
Types: serial int flt str bool json now sym money (`int` 64-bit). Modifiers:
`pk uniq null index ref:tbl.col`. Multi-column: `index(a b)`, `uniq(a b)`.
Index names are automatic (`idx_<tbl>_<cols>` / `uniq_<tbl>_<cols>`).
`ref:tbl.col` → enforced FOREIGN KEY (`PRAGMA foreign_keys=ON`); inserting a
child row whose parent is missing fails. Adding/removing `ref:` on an existing
column auto-migrates via a safe table rebuild (data preserved; aborts if current
rows would violate the new constraint).

`tbl` is the single source of truth — auto-migration diffs it against the DB:
new column → `ADD COLUMN`, removed column → `DROP COLUMN` (backed up first),
removed `tbl` → `DROP TABLE` (backed up; only Fluxon-managed tables), index added/
removed → `CREATE/DROP INDEX`. Idempotent: re-deploying the same `tbl` is safe.
Just write the latest `tbl` — no migration SQL needed.
`json` column: auto map/list on read, auto-encode on write.
`sym` column: text in DB, symbol in Fluxon (auto-converts):
```fluxon
db.ins "tickets" {status::new}
t = db.from "tickets" |> db.eq {id:id} |> db.first
match t.status
  :new -> ...
db.from "t" |> db.eq {status::new} |> db.all    # filter: symbol value, no SQL
```

### ai (LLM — first-class, key auto-detected)
```fluxon
txt = ai.ask "question ${x}"                 # → text
r = ai.json "extract: ${text}" {intent::a items:[{product:str qty:int}]}  # → map
```
Metadata: `r._.conf` (0..1), `r._.tokens`, `r._.cost`, `r._.ms`.
```fluxon
if r._.conf > 0.85
  auto r
elif r._.conf >= 0.6
  confirm r
else
  escalate r
```
Provider auto-detected from env (OS env > .env), nothing to configure:
`ANTHROPIC_API_KEY` → Claude (default `claude-opus-4-8`); `OPENAI_API_KEY` → GPT
(default `gpt-4o`). Both present → Anthropic wins. Override: `$AI_PROVIDER`
(`anthropic|openai`), `$AI_KEY` (provider-agnostic), `$AI_MODEL`.

`ai.run` — ONE step of a tool-loop (doesn't execute; returns what it wants to do;
the loop is yours → logging/cost/approval control). Returns one of:
`{kind::final text}` or `{kind::call tool args id calls:[{tool args id} ...]}`.
The model may call several tools in parallel → all are in `calls`; `tool/args/id`
mirror `calls[0]` (back-compat). Return a tool result for EACH call, else the
next request 400s (a missing tool_use_id has no tool_result).
```fluxon
msgs <- [{role::user content:text}]
each i in 1..10
  r = ai.run msgs tools                # tools: [{name desc params}]
  if r.kind == :final
    ret r.text
  # r.kind == :call → model wants tools; iterate every call
  each c in r.calls
    out = reg.call c.tool c.args        # run the tool by name
    # feed back: assistant tool_use + tool result (id ties them)
    msgs <- msgs.push {role::assistant content:[{type:"tool_use" id:c.id name:c.tool input:c.args}]}
    msgs <- msgs.push {role::tool id:c.id content:(json.enc out)}
```

### auth (JWT + password hash, $AUTH_SECRET auto)
```fluxon
token = auth.jwt {sub:user.id tenant:t.id role:"admin"}   # → signed JWT (HS256)
token = auth.jwt {sub:user.id} {exp:3600}                 # optional expiry (seconds; default 24h)
claims = auth.verify token        # → payload map (signature + exp checked), or err
hash = auth.hash "user-parol"     # → argon2id hash (salt embedded)
ok = auth.check "user-parol" hash # → bool (constant-time)
```
Signing key auto-detected from `$AUTH_SECRET` (OS env > .env), like `db`/`ai` —
missing → explicit error. `auth.verify` returns `err` on bad signature, expired
token, OR a token with no numeric `exp` (a token must expire — one without `exp`
is rejected, not accepted forever). `iat`/`exp` are added to the payload
automatically. Catch with `!`/propagate → 401 in a handler. Pairs with middleware:
verify in `http.before`, put claims in `req.ctx`, read in the handler.

### crypto (hash / hmac / base64 / uuid)
```fluxon
crypto.sha256 s        # → SHA-256 hex (lowercase)
crypto.hmac key msg    # → HMAC-SHA256 hex — verify webhook signatures (Stripe/GitHub/Telegram)
crypto.b64 s           # → base64 (standard alphabet, padded)
crypto.b64d s          # → decode base64 (padding optional, url-safe accepted), or err
crypto.b64db s         # → decode base64 to bytes (binary-safe: images/files)
crypto.hex s           # → hex of the input's bytes
crypto.uuid            # → UUID v4 (crypto-secure source)
```
`crypto.hmac`/`crypto.sha256` return lowercase hex — compare directly with the
signature header a webhook sends. `crypto.b64d` errs on invalid base64 or
non-UTF-8 output (no silent corruption) — for binary payloads use `crypto.b64db`.
Inputs (`sha256`/`hmac`/`b64`/`hex`) take str OR bytes — hash a file the same way.

### reg (function registry — dynamic dispatch)
Store/call a function by STRING name (for agent tools — NOT a `match`-switch,
added at runtime):
```fluxon
reg.add "calc" \args -> args.a + args.b
out = reg.call "calc" {a:2 b:3}      # → 5
reg.has "calc"   ·   reg.names
```

### list methods (.method)
```fluxon
l.len · l.push x · l.filter \x->x>0 · l.map \x->x*2 · l.has x · l.0
l.slice a b · l.join ", " · l.reduce 0 \acc x -> acc + x
l.index x → birinchi indeks yoki -1 · l.find \x->x>4 → birinchi mos element yoki nil
l.sort → natural order (nums/strs) · l.sort \a b -> a.p - b.p (comparator → number: neg = a first)
l.reverse · l.uniq (first kept) · l.flat (one level) · l.zip other → [[a b] ...]
l.any \x->x>4 → bool (short-circuit) · l.all \x->x>0 → bool
```
Build a list: `l.push x` (NOT `+[x]`). Build a string: `l.join sep`.
Sort in memory: `l.sort` (NOT a `db.order` round-trip).

### map methods (.method)
```fluxon
m.set k v · m.del k · m.has k · m.keys · m.vals · m.k · m[k]
m.merge other → new map, other's keys win (defaults + override)
```
Write to a map: `m.set k v` (`m[k]` is READ only). Shared state via this.

### str / math / rand (core, no use needed)
```fluxon
str.len s · str.slice s a b · str.up s · str.low s · str.split s sep → list
str.has s sub → bool · str.int s · str.str x
str.trim s · str.replace s old new · str.starts s pre → bool · str.ends s suf → bool
str.pad s n ch → pads LEFT to n chars ("7"→"007") · str.repeat s n
math.floor x · math.ceil x · math.abs x · math.round x
math.min a b · math.max a b · math.pow a b · math.sqrt x → flt
rand.int a b · rand.str n
```
List length `l.len` (member), string length `str.len s` (module).

### time
All times — UTC text `"YYYY-MM-DD HH:MM:SS"` (same as SQLite `CURRENT_TIMESTAMP`).
```fluxon
time.now · time.ago 24 :hr · time.in 60 :min (:sec :min :hr :day) · time.fmt t "..."
time.sleep 1 · time.sleep 0.5   # waits secs (flt too) — polling/retry backoff
time.parse "2026-06-10T10:00:00Z"   # arbitrary ISO text -> canonical UTC timestamp ("Z"/"±HH:MM")
time.add t 30 :min · time.sub t 5 :min   # offset from ANY time (not now): end_at = start_at + dur
time.diff a b                       # (a - b) in seconds (int); / 60 -> minutes
db.from "t" |> db.cmp :created :gt (time.ago 24 :hr) |> db.count :c |> db.agg_row
```
**Duration/interval recipes** (interval arithmetic EXISTS — `time.add`/`diff` are here):
```fluxon
end_at = time.add start_at dur :min          # duration: start + dur minutes
mins   = (time.diff end_at start_at) / 60     # gap between two times -> minutes
overlap = a.start < b.end & a.end > b.start   # do two intervals overlap? (bool)
buf_start = time.sub start_at 15 :min         # buffer: 15 min before start
```
**IANA timezone / DST** — `time.parse`/`time.fmt` take an optional zone arg (NOT a fixed offset):
```fluxon
utc = time.parse "2026-07-15 09:00:00" "Asia/Tashkent"  # local wall-clock -> UTC (DST-aware)
loc = time.fmt utc "HH:mm" "America/New_York"            # UTC instant -> zone wall-clock
```

### json / env / log
```fluxon
json.enc v · json.dec s · env.PORT ?? "8080" · log "message"
```

### io (terminal input/output)
`log` always adds `\n` to stderr; for an interactive CLI (REPL, agent, wizard):
```fluxon
io.read_line          # one line from stdin → str (blocks until Enter); EOF → nil
io.print s            # print to stdout WITHOUT `\n` (for building prompts)
io.prompt msg         # print msg, then io.read_line → str (shorthand)
```
REPL loop — `each i in inf` (infinite), `stop` on EOF/exit:
```fluxon
each i in inf
  line = io.prompt "you> "
  if line == nil               # EOF (Ctrl-D)
    stop
  if line == "exit"
    stop
  log "reply:" line
```

### fs (local filesystem)
Naming in `db.*` style (`fs.read`/`fs.del`). On error `Flow::err` (catch with try);
`fs.read` is the only exception — `nil` if the file is missing:
```fluxon
fs.read path           # → str, or nil if file missing
fs.readb path          # → bytes (binary read: image/PDF), or nil if missing
fs.write path content  # overwrites (str OR bytes) → :ok
fs.append path content # appends to end (str OR bytes; creates file if missing) → :ok
fs.exists path         # file OR directory exists → bool
fs.ls path             # names inside a directory (sorted, name only) → [str]
fs.del path            # file or EMPTY directory → :ok (no recursive delete)
fs.mkdirp path         # creates with intermediate dirs, idempotent → :ok
```
```fluxon
if !(fs.exists "data")
  fs.mkdirp "data"
fs.write "data/conf.json" (json.enc {port:8080})
cfg = json.dec (fs.read "data/conf.json")
```

### bytes (binary data — core, no use needed)
For files / HTTP binaries (image, PDF, gzip). No literal syntax — values come
from `fs.readb`, `crypto.b64db`, `bytes.of`. Logs/interp show `<bytes N>`
(raw bytes never leak into text).
```fluxon
bytes.of s        # str → bytes (UTF-8); bytes → itself
bytes.str b       # bytes → str (err if not UTF-8 — no silent corruption)
bytes.len b       # BYTE count (str.len counts CHARS)
bytes.slice b a c # sub-bytes [a..c) (clamped, like str.slice)
```
- HTTP: `rep 200 b {content_type:"image/png"}` sends raw bytes (default
  `application/octet-stream`). A non-UTF-8 request/response body (`req.body`,
  client `res.body`) arrives as bytes; text stays str as before.
- db: bytes ↔ BLOB column. `json.enc` encodes bytes as base64 text.

### sh (external shell command)
Runs a command through the shell (`sh -c` / on Windows `cmd /C`), so `&&`, pipes (`|`),
glob all work. `code == 0` is success; a non-zero exit is NOT a `Flow::err` — check `code`.
```fluxon
sh.run cmd             # → {stdout: str  stderr: str  code: int}
```
```fluxon
r = sh.run "git status --short"
if r.code == 0
  log r.stdout
else
  fail "git failed: ${r.stderr}"
```
Dangerous commands are NOT blocked — that's the caller's responsibility.

### cron (background task)
Standard Unix 5-field (minute hour day month weekday), UNQUOTED — `*` is the cron char:
```fluxon
cron.on 0 * * * * check_prices    # at the top of every hour · fn or \-> lambda
cron.on 30 9 * * 1-5 \-> report    # weekdays 09:30
```
`cron.on` doesn't block (registers, like `http.on`). With a server (`http.serve`/
`ws.serve`) cron runs in the background; in a server-less script `cron.run` holds the
process. `cron.run` and `http.serve`/`ws.serve` combine in ANY order — none blocks the
others (all share one event-loop at top-level's end).

### queue (background)
Offload heavy work — `push`/`on` don't block, a worker runs FIFO:
```fluxon
queue.on "send" \job -> tools.send job.ph job.body   # job = push payload
queue.push "send" {ph:p body:t}                       # payload optional
```
If push is written before the handler, the job waits in the queue. queue has no `run`
of its own — a worker runs in the background while `http.serve`/`ws.serve`/`cron.run` holds the process.

### ws (websocket — realtime)
```fluxon
ws.on :connect \conn -> ws.data.set conn :user nil   # conn.id stable; ws.data = session
ws.on :message \conn msg ->                    # msg — incoming TEXT (str)
  m = json.dec msg
  ws.send conn (json.enc {ok:true})            # to this connection (text sent)
ws.on :disconnect \conn -> ws.room.leave conn "ch:5"
ws.serve 9000
```
Session: `ws.data.set conn :key value` · `ws.data.get conn :key` (this connection, cleared on disconnect).
Room (broadcast): `ws.room.join conn "ch:5"` · `ws.room.leave conn "ch:5"` ·
`ws.room.send "ch:5" msg` (to all) · `ws.room.members "ch:5"` (presence).
`http.serve` and `ws.serve` run together in ONE process — declare both; they don't
block until top-level ends, then share one event-loop. An HTTP handler can call
`ws.room.send` to push realtime updates (REST + realtime, e.g. live poll/chat).

## Full example
```fluxon
use http db

tbl notes
  id   serial pk
  text str
  ts   now

http.on :post "/notes" \req ->
  rep 201 (db.ins "notes" {text:req.body.text})
http.on :get "/notes" \req ->
  rep 200 (db.from "notes" |> db.order :ts :desc |> db.all)
http.serve 8080
```
