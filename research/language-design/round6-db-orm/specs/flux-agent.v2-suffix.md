# Flux — language spec (for AI)

Flux: AI-native backend language. One task = one way. Few tokens. Batteries-included.
File extension: `.fx`. Read once, write correct Flux code.

## Basics
- Comment `# to end of line` (no `//`). Statement on a new line (no `;`).
- Block = indentation (2 spaces), no `{}`.
- These are keywords — never name a var/loop/param after one (e.g. `each exp in xs`
  fails; use `e`): `as each elif else exp fail fn if in inf match ret skip stop tbl use`
```flux
if x > 0
  log "positive"
log "outside"
```

## Types
```
42 int · 3.14 flt · "hi" str · true bool · nil · :ok sym (enum/tag)
[1 2 3] list · {a:1 b:2} map        # NO COMMAS, space-separated
```
Str interpolation: `"$x"` (bare var only) or `"${expr}"` (any expr — `.field`, calls).
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
```flux
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
```flux
http.on :post "/x" \req ->
  if !req.body.email
    ret rep 400 {error:"email required"}
  rep 201 (db.ins "t" {...})
```

## Control flow
```flux
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
```flux
each item in list   ·   each i in 1..5   ·   each k, v in map   ·   each i in inf
```
In a loop: `skip` (continue), `stop` (break). `each i in inf` = infinite loop
(i = 0,1,2,...) for REPL / event loops / "repeat until `stop`". `inf` is ONLY
valid as the `each` iterator — not a value.

`match` — value dispatch (symbol/number ONLY, NOT boolean conditions):
```flux
match status
  :new -> log "new"
  _ -> log "default"
```
For boolean conditions (`x > 0.85`) ALWAYS use `if/elif/else`. `match true` = error.

## Errors
```flux
user = db.one "..." [id]!     # ! = on error, auto-propagate up
name = user.name ?? "guest"   # ?? = alternative if nil
fail 422 "insufficient funds" # with status → 422 {error:...} to client
fail "internal error"         # no status → 500
```
`!` propagate, `??` replace nil, `fail [status] "..."` raise. No try/catch —
`fail 4xx` auto-converts an expected error into an HTTP response (code stays flat).

## Modules
```flux
use http db ai json     # batteries, no install
use ./tools             # your file → tools.fn
use ./ai as helper      # ALIAS: on clash with a battery name → helper.fn
exp fn create_order ... # exp = expose externally
```

## Batteries (stdlib — no install)

### http
```flux
http.on :post "/notes" \req -> rep 201 {ok:true}
http.on :get "/notes/:id" \req -> rep 200 {id:req.params.id}
http.serve 8080
```
- Method: `:get :post :put :patch :del`. `req.body` (JSON→map), `req.params.id`,
  `req.query`, `req.headers`. Missing key → `nil`.
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
- Rate limit: `http.limit N :sec|:min|:hr \req -> key` (declared like middleware,
  runs in order; an optional leading path scopes it like `http.before`):
  `http.limit 100 :min \req -> req.ctx.tenant_id` (per-tenant, all routes),
  `http.limit "/api/*" 100 :min \req -> req.headers.x_api_key` (per-key, prefix).
  Over the limit → auto `429` + `Retry-After` (seconds until window resets). Key fn
  nil → falls back to `req.ip`. Fixed-window, in-memory (single instance only).
```flux
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

### db (Postgres, $DATABASE_URL auto)
```flux
row  = db.ins "orders" {cust:5 status::new}          # → full row (with id)
db.up "orders" {total:1500} {id:oid}                 # {set} {where}
db.del "cart_items" {id:iid}                          # {where}
db.put "memory" {val:v} {agent:a key:k}               # UPSERT (atomic)
```

Reads are declarative — a flat filter map, no raw SQL:
```flux
rows = db.find "bookings" {tenant_id:tid}             # → list of maps (all matching)
one  = db.get  "bookings" {id:bid tenant_id:tid}      # → first match, or nil
```
A map key = a column; multiple keys are AND-ed. A **list value → `IN (...)`**:
```flux
db.find "bookings" {tenant_id:tid status:[:pending :confirmed]}  # status IN (..)
```
Operators — a **suffix on the key**, `col__op` (ops: `gt ge lt le ne like`):
```flux
db.find "bookings" {tenant_id:tid start_at__ge:t0 start_at__lt:t1}  # >= t0 AND < t1
db.find "resources" {tenant_id:tid capacity__ge:4 name__like:"%lab%"}
```
A bare key (no `__`) means `=`. Order / limit / paging — an **optional second map**:
```flux
db.find "bookings" {tenant_id:tid} {order::start_at limit:50 offset:0}
db.find "bookings" {tenant_id:tid} {order::created desc:true limit:20}
```
`order` = a symbol (column), `desc:true` = descending, `limit`/`offset` = ints.

Aggregation — `db.agg "table" {filter} {spec}`. Spec keys name the output:
```flux
db.agg "bookings" {tenant_id:tid status:[:done :confirmed]}
  {group::resource_id count::n sum__total_cents::revenue order::revenue desc:true}
# → [{resource_id:5 n:12 revenue:48000} ...]
```
Spec keys: `count::out`, `sum__col::out` / `avg__col::out` / `min__col::out` /
`max__col::out`, `group::col` (or list), plus `order`/`desc`/`limit`. No `group`
→ one summary row. For a raw expression (`date(created)`) use `db.q`.

`db.q "raw SQL" [params]` / `db.one` stay available as an escape hatch:
```flux
db.q "select date(created) day, count(*) n from bookings where tenant_id=$1 group by day order by day" [tid]
```

Transaction — atomic, rollback on `fail`/`!`, returns a value:
```flux
res = db.tx \->
  ord = db.ins "orders" {cust:c total:t}
  each it in items
    db.up "products" {stock:it.stock - it.qty} {id:it.id}
  ret ord
```
`db.tx` auto-serializable + retry → "read-check-update" is race-safe. Idempotency:
`uniq` column + ins inside tx (duplicate → rollback).

Schema = `tbl`:
```flux
tbl products
  id    serial pk
  owner int ref:users.id
  price money               # money = integer minor unit (cents), NOT float
  ts    now
```
Types: serial int flt str bool json now sym money (`int` 64-bit). Modifiers:
`pk uniq null ref:tbl.col`. Multi-column: `uniq(agent, key)`.
`json` column: auto map/list on read, auto-encode on write.
`sym` column: text in DB, symbol in Flux (`{status::pending}` filters fine).

### ai (LLM — first-class, key auto-detected)
```flux
txt = ai.ask "question ${x}"                 # → text
r = ai.json "extract: ${text}" {intent::a items:[{product:str qty:int}]}  # → map
```
Metadata: `r._.conf` (0..1), `r._.tokens`, `r._.cost`, `r._.ms`.
```flux
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
`{kind::final text}` or `{kind::call tool args id}`.
```flux
msgs <- [{role::user content:text}]
each i in 1..10
  r = ai.run msgs tools                # tools: [{name desc params}]
  if r.kind == :final
    ret r.text
  # r.kind == :call → model wants a tool
  out = reg.call r.tool r.args         # run the tool by name
  # feed back: assistant tool_use + tool result (id ties them)
  msgs <- msgs.push {role::assistant content:[{type:"tool_use" id:r.id name:r.tool input:r.args}]}
  msgs <- msgs.push {role::tool id:r.id content:(json.enc out)}
```

### auth (JWT + password hash, $AUTH_SECRET auto)
```flux
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

### reg (function registry — dynamic dispatch)
Store/call a function by STRING name (for agent tools — NOT a `match`-switch,
added at runtime):
```flux
reg.add "calc" \args -> args.a + args.b
out = reg.call "calc" {a:2 b:3}      # → 5
reg.has "calc"   ·   reg.names
```

### list methods (.method)
```flux
l.len · l.push x · l.filter \x->x>0 · l.map \x->x*2 · l.has x · l.0
l.slice a b · l.join ", " · l.reduce 0 \acc x -> acc + x
l.index x → birinchi indeks yoki -1 · l.find \x->x>4 → birinchi mos element yoki nil
```
Build a list: `l.push x` (NOT `+[x]`). Build a string: `l.join sep`.

### map methods (.method)
```flux
m.set k v · m.del k · m.has k · m.keys · m.vals · m.k · m[k]
```
Write to a map: `m.set k v` (`m[k]` is READ only). Shared state via this.

### str / math / rand (core, no use needed)
```flux
str.len s · str.slice s a b · str.up s · str.low s · str.split s sep → list
str.has s sub → bool · str.int s · str.str x
math.floor x · math.ceil x · math.abs x · rand.int a b · rand.str n
```
List length `l.len` (member), string length `str.len s` (module).

### time
```flux
time.now · time.ago 24 :hr · time.in 60 :min (:sec :min :hr :day) · time.fmt t "..."
time.sleep 1 · time.sleep 0.5   # secs kutadi (flt ham) — polling/retry backoff
time.parse "2026-06-10T10:00:00Z"   # ixtiyoriy ISO matn -> kanonik UTC timestamp ("Z"/"±HH:MM")
time.add t 30 :min · time.sub t 5 :min   # IXTIYORIY vaqtdan offset (now emas): end_at = start_at + dur
time.diff a b                       # (a - b) sekundda (int); / 60 -> daqiqa
db.one "select count(*) c from t where created > $1" [time.ago 24 :hr]
```

### json / env / log
```flux
json.enc v · json.dec s · env.PORT ?? "8080" · log "message"
```

### io (terminal input/output)
`log` always adds `\n` to stderr; for an interactive CLI (REPL, agent, wizard):
```flux
io.read_line          # one line from stdin → str (blocks until Enter); EOF → nil
io.print s            # print to stdout WITHOUT `\n` (for building prompts)
io.prompt msg         # print msg, then io.read_line → str (shorthand)
```
REPL loop — `each i in inf` (infinite), `stop` on EOF/exit:
```flux
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
```flux
fs.read path           # → str, or nil if file missing
fs.write path content  # overwrites (prior content lost) → :ok
fs.append path content # appends to end (creates file if missing) → :ok
fs.exists path         # file OR directory exists → bool
fs.ls path             # names inside a directory (sorted, name only) → [str]
fs.del path            # file or EMPTY directory → :ok (no recursive delete)
fs.mkdirp path         # creates with intermediate dirs, idempotent → :ok
```
```flux
if !(fs.exists "data")
  fs.mkdirp "data"
fs.write "data/conf.json" (json.enc {port:8080})
cfg = json.dec (fs.read "data/conf.json")
```

### sh (external shell command)
Runs a command through the shell (`sh -c` / on Windows `cmd /C`), so `&&`, pipes (`|`),
glob all work. `code == 0` is success; a non-zero exit is NOT a `Flow::err` — check `code`.
```flux
sh.run cmd             # → {stdout: str  stderr: str  code: int}
```
```flux
r = sh.run "git status --short"
if r.code == 0
  log r.stdout
else
  fail "git failed: ${r.stderr}"
```
Dangerous commands are NOT blocked — that's the caller's responsibility.

### cron (background task)
Standard Unix 5-field (minute hour day month weekday), UNQUOTED — `*` is the cron char:
```flux
cron.on 0 * * * * check_prices    # at the top of every hour · fn or \-> lambda
cron.on 30 9 * * 1-5 \-> report    # weekdays 09:30
```
`cron.on` doesn't block (registers, like `http.on`). With a server (`http.serve`/
`ws.serve`) cron runs in the background; in a server-less script `cron.run` holds the
process. `cron.run` and `http.serve`/`ws.serve` combine in ANY order — none blocks the
others (all share one event-loop at top-level's end).

### queue (background)
Offload heavy work — `push`/`on` don't block, a worker runs FIFO:
```flux
queue.on "send" \job -> tools.send job.ph job.body   # job = push payload
queue.push "send" {ph:p body:t}                       # payload optional
```
If push is written before the handler, the job waits in the queue. queue has no `run`
of its own — a worker runs in the background while `http.serve`/`ws.serve`/`cron.run` holds the process.

### ws (websocket — realtime)
```flux
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
```flux
use http db

tbl notes
  id   serial pk
  text str
  ts   now

http.on :post "/notes" \req ->
  rep 201 (db.ins "notes" {text:req.body.text})
http.on :get "/notes" \req ->
  rep 200 (db.q "select * from notes order by ts desc")
http.serve 8080
```
