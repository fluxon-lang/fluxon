# Flux — language spec (for AI)

Flux: AI-native backend language. One task = one way. Few tokens. Batteries-included.
File extension: `.fx`. Read once, write correct Flux code.

## Basics
- Comment `# to end of line` (no `//`). Statement on a new line (no `;`).
- Block = indentation (2 spaces), no `{}`.
- These are keywords — never name a var/loop/param after one (e.g. `each exp in xs`
  fails; use `e`): `as each elif else exp fail fn if in match ret skip stop tbl use`
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
.    member/index: m.key, l.0, l.len, m[k]
..   range: 1..5 → [1 2 3 4 5]   ·   |>  pipe: x |> f |> g
```

## Functions
```flux
fn add a b
  ret a + b               # ret (early) or last expression (implicit)
fn double x -> x * 2      # one-liner
add 2 3                   # paren-free call; parens only group: f (g x)
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
Only loop = `each` (no while/for):
```flux
each item in list   ·   each i in 1..5   ·   each k, v in map
```
In a loop: `skip` (continue), `stop` (break). Conditional repeat: `each i in 1..n`
or recursion.

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
- Route priority: a literal path auto-wins (`/stats/:c` > `/:c`).
- Client: `http.get url`, `http.post url body` → `res.status res.body res.headers`.
  `res.headers` (map, lowercase keys): `res.headers.location`, also `m[k]`.
  Redirects not followed by default; opt-in: `http.get url {follow:true max:10}`
  → follows, `res.hops` (hop count). `max` default 10.
  Custom request header: `{headers:{"x-api-key":KEY "anthropic-version":"2023-06-01"}}`
  (symmetric with req/res.headers; a user value overrides the auto `content-type`).

### db (Postgres, $DATABASE_URL auto)
```flux
rows = db.q "select * from t where owner=$1" [oid]   # → list of maps
one  = db.one "select * from users where id=$1" [id] # → map or nil
row  = db.ins "orders" {cust:5 status::new}          # → full row (with id)
db.up "orders" {total:1500} {id:oid}                 # {set} {where}
db.del "cart_items" {id:iid}                          # {where}
db.put "memory" {val:v} {agent:a key:k}               # UPSERT (atomic)
```
Params `$1 $2`, values `[...]`. No params: `db.q "select * from links"`.
Aggregate may be nil → `?? 0`: `db.one "select count(*) c, sum(x) s from t"`.

Transaction — atomic, rollback on `fail`/`!`, returns a value:
```flux
res = db.tx \->
  ord = db.ins "orders" {cust:c total:t}
  each it in items
    db.up "products" {stock:it.stock - it.qty} {id:it.id}
  ret ord
```
`db.tx` auto-serializable + retry → "read-check-update" is race-safe (no lock
needed). Idempotency: `uniq` column + ins inside tx (duplicate → rollback):
```flux
old = db.one "select * from txns where ikey=$1" [key]
old ?? (ret old)
db.tx \-> db.ins "txns" {ikey:key ...}   # duplicate → uniq error → rollback
```

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
`sym` column: text in DB, symbol in Flux (auto-converts):
```flux
db.ins "tickets" {status::new}
t = db.one "select * from tickets where id=$1" [id]
match t.status
  :new -> ...
db.q "select * from t where status=$1" [:new]    # filter: symbol → text
```

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
time.now · time.ago 24 :hr (:sec :min :hr :day) · time.fmt t "..."
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
REPL loop — no `each`/`while`, via recursion (EOF → `nil` → stop):
```flux
repl = \n ->
  line = io.prompt "you> "
  if line == nil
    ret nil                # EOF (Ctrl-D) — exit
  log "reply:" line
  repl n
repl 0
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
`ws.serve`) cron runs in the background; in a server-less script `cron.run` holds the process.

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
`http.serve` and `ws.serve` run together.

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
