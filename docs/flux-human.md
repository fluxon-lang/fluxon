# Flux — The Programming Language (Complete guide for humans)

> 🌐 **Language:** English (current) · [O'zbek](flux-human.uz.md)

> **What is Flux?** Flux is a programming language designed for backend systems
> that AI agents write well. Its philosophy: *"The language adapts to the AI, not
> the AI to the language."* There is **one** clear way to do each thing, the
> syntax uses few tokens, and the things you need most (HTTP server, database,
> AI/LLM calls, cron, queues) are built **into** the language — with no package
> installs.

Flux files are saved with the `.fx` extension.

This document is the complete, detailed **human** guide. If you want to teach
Flux to an AI agent, use the shorter `flux-agent.md` file.

---

## 0. Core ideas (read these first)

The 5 principles that set Flux apart from other languages:

1. **One task = one way (canonical form).** In other languages you can write the
   same thing 5 ways (`while`, `for`, `do-while`...). In Flux there is **only
   `each`** for iteration. There is **only one** way to print to the screen. The
   reason for this rule: the AI does not think "which method should I choose?"
   each time — there is no choice, so there are fewer mistakes.

2. **Few tokens, but readable.** The syntax is as short as possible, but *not
   cryptic*. Keywords are spelled out in full (`each`, `match`, `else`) — because
   a human or AI seeing Flux for the first time must understand them immediately.

3. **Batteries included (everything built in).** `http`, `db`, `ai`, `json`,
   `cron`, `queue` — all of these are in the standard library. No `npm install`,
   no `composer require`. You just say `use http` and use it.

4. **AI is a first-class primitive.** In other languages, calling an LLM means
   installing an SDK, configuring a key, and parsing JSON. In Flux, `ai.json`
   turns text into structured data in a single line and returns a confidence
   score.

5. **Significant whitespace (indentation).** Blocks are separated not by `{}`
   braces but by **indentation (2 spaces)** — just like Python. This removes
   redundant characters.

---

## 1. Lexical basics

### Comments
There is only one kind of comment — from a `#` character to the end of the line:
```flux
# This is a comment
x = 5   # This is also a comment
```
Flux has **no** `//` or `/* */`. One way — `#`.

### Statements
Each statement ends at a **new line**. A semicolon (`;`) is **not needed** and is
not used:
```flux
x = 5
y = 10
```

### Blocks
A block is opened not with `{}` but with **indentation**. Each level is **2
spaces**. The block ends when the indentation decreases:
```flux
if x > 0
  log "positive"
  log "this line is also inside the block"
log "outside the block"
```

---

## 2. Values and types

Flux has the following basic types:

| Notation | Type | Description |
|-------|-----|------|
| `42` | `int` | Integer |
| `3.14` | `flt` | Fractional number (float) |
| `"hello"` | `str` | Text (string) |
| `true` / `false` | `bool` | Boolean value |
| `nil` | `nil` | "Nothing" / emptiness |
| `[1 2 3]` | `list` | List — elements separated by **spaces** |
| `{a:1 b:2}` | `map` | Key-value pairs — separated by **spaces** |
| `:ok` | `sym` | Symbol — for enums/tags |
| — | `bytes` | Binary data — no literal, comes from functions |

### Important subtleties

**Binary data (`bytes`).** For non-text data like images, PDFs, archives.
There is no literal syntax — values come from functions: `fs.readb path`
(binary file read), `crypto.b64db s` (binary-safe base64 decode),
`bytes.of s` (text → its UTF-8 bytes). Core operations:
```flux
b = fs.readb "logo.png"     # bytes (nil if the file is missing)
bytes.len b                  # BYTE count (str.len counts CHARS)
bytes.str b                  # bytes → text (explicit error if not UTF-8)
bytes.slice b 0 4            # sub-bytes
fs.write "copy.png" b        # fs.write/append accept str or bytes
rep 200 b {content_type:"image/png"}   # raw binary HTTP response
```
In logs/interpolation bytes render as `<bytes N>` — raw bytes never leak into
text. `crypto.sha256`/`b64`/`hex` inputs take str or bytes.

**No commas in lists and maps.** Elements are separated by spaces. This is
intentional — commas waste tokens:
```flux
nums = [1 2 3 4]
user = {name:"Aziza" age:30 active:true}
```

**Putting a variable inside text (interpolation).** With `"${...}"` you embed an
expression inside text:
```flux
name = "Aziza"
log "Hello ${name}!"               # → Hello Aziza!
log "Total: ${price * qty} so'm"   # an expression also works
```
For a simple variable you can shorten it to `"$name"`, but for an expression
`${...}` is required.

**Multi-line text (block strings).** For long prompts, SQL, or templates use
`"""`. Content starts on the next line, and the common indentation of the
lines is stripped automatically — so the block sits naturally inside indented
code:
```flux
prompt = """
  You are a helpful agent.
  User question: ${question}
  """
```
If the closing `"""` is on its own line, the text has no trailing `\n`.
Interpolation and `\n`/`\t` escapes work as in normal strings; `"` can be
written freely without escaping (handy for JSON/HTML fragments).

**Symbols — instead of enums.** To represent states, use a symbol instead of
text. `:new`, `:confirmed` — these are cheaper in tokens and clearer than the
text `"new"`:
```flux
status = :confirmed
dir = :in
```
When a symbol is converted to text (interpolation, `str.str`, `+`, `log`) the
`:` prefix is dropped — the value is `florist`, the `:` is a syntax marker:
`str.str :florist` → `"florist"`, `"path/${:florist}"` → `"path/florist"`.
Inside a list/map, the `:` is kept (`[:a]` → `[:a]`), because there a symbol
needs to stand out from text.

**Truthiness.** `nil` and `false` are falsy. Everything else (including `0`,
`""`, and the empty list) is **truthy**. This simple rule is intentional: only
two things are falsy.

---

## 3. Variables (bindings)

Flux has **two** kinds of binding, and they do **different things** (which is why
having two does not violate the canonical rule):

### `=` — immutable
A value is assigned once, then cannot be changed:
```flux
x = 10
name = "Aziza"
```
This is the **default** case. Most values do not change.

### `<-` — mutable
A variable whose value can be changed later. Reassignment is also done with `<-`:
```flux
total <- 0.0
total <- total + 5.0     # reassignment
```

> **Rule:** if a value does not change — use `=`. Use `<-` only for things that
> truly change. This makes the code easier to read: when you see `<-`, you know
> "this changes".

---

## 4. Operators

### Arithmetic
```flux
+   -   *   /   %        # add, subtract, multiply, divide, remainder
```
**`+` also concatenates strings.** If the operands are numbers it adds; if they
are text it joins:
```flux
1 + 2          # → 3
"hel" + "lo"   # → "hello"
```
The type itself decides the difference — one operator, two natural behaviors.

### Comparison
```flux
==  !=  <  <=  >  >=
```

### Logical
```flux
&    # and
|    # or
!    # not — before a value: !x
```

### Special operators

**`??` — null-coalesce.** If the left side is `nil`, it gives the right side:
```flux
port = env.PORT ?? "8080"     # if PORT is missing, "8080"
name = user.name ?? "guest"
```

**`.` — member access / index.** Map key, list index, length:
```flux
user.name        # map key
list.0           # first element of the list
list.len         # length
m[key]           # dynamic key (via a variable)
list[i]          # computed index (via an expression)
list.(i)         # computed index through `.` — same as list[i]
```

**`..` — range.** Both ends are inclusive:
```flux
1..5             # [1 2 3 4 5]
```

**`|>` — pipe.** Passes a value into a function, removing nested notation:
```flux
result = data |> clean |> format
# this is equivalent to: format(clean(data))
```

---

---

## 5. Functions

A function is declared with `fn`. Arguments are separated by **spaces** (no
commas):

```flux
fn add a b
  ret a + b
```

### Single-line function
If the body is a single expression, you can write it on one line with `->`:
```flux
fn double x -> x * 2
```

### Return
Two ways, but they give the same result:
- `ret x` — explicit return
- **The last expression** is returned automatically (without `ret`)

```flux
fn add a b
  a + b            # the last expression — returned automatically

fn check x
  if x > 0
    ret "positive"   # ret is needed for an early return
  "non-positive"     # the last expression
```

> **Note:** `ret` is used only when you need an **early** (mid-function) return.
> At the end — just write the expression.

**`ret` also works inside a lambda.** This matters most in HTTP handlers. Instead
of a deep `if/elif/else` pyramid for validation, write a **guard clause** (early
exit) — the code stays flat:
```flux
# ❌ Deep nesting (bad):           ✅ Guard clause (good):
http.on :post "/x" \req ->        http.on :post "/x" \req ->
  if req.body.email                 if !req.body.email
    if req.body.body                  ret rep 400 {error:"email required"}
      rep 201 (...)                 if !req.body.body
    else                              ret rep 400 {error:"body required"}
      rep 400 {...}                 rep 201 (db.ins "t" {...})
  else
    rep 400 {...}
```

### Calling a function
Arguments are separated by spaces, without parentheses:
```flux
add 2 3            # → 5
double 4           # → 8
```
Parentheses are only needed for **grouping** (passing the result of one function
into another):
```flux
double (add 2 3)   # first add 2 3 = 5, then double 5 = 10
```

**A no-argument function is called with empty parentheses `()`.** Since a
call without parentheses is defined by its arguments, this is the only way to
call a function that has no parameters. This clearly distinguishes a name (value)
from a call:
```flux
fn new_id -> rand.str 8
new_id()           # CALL → a new random id each time
new_id             # NOT a call → the function VALUE (for callback/reg)
```
> `f(x)` (argument inside parentheses) **does not work** — the canonical form is
> `f x`. Empty `()` is only for a no-argument call (one task = one way).

### Lambda (anonymous function)
With the `\` character, used inline:
```flux
\x -> x * 2
each_map nums \x -> x * 2    # multiply each element by 2
```

---

## 6. Control flow

### Conditions: `if` / `elif` / `else`
```flux
if x > 0
  log "positive"
elif x == 0
  log "zero"
else
  log "negative"
```
Keywords are spelled out **in full** (`elif`, `else`) — so they are
understandable at a glance.

`if` also works as an **expression** (ternary equivalent): it returns a value on
one line. The `else` branch is required. Wrap calls in the condition in parens.

```flux
pad = if h < 10 ("0" + str.str h) else (str.str h)   # leading-zero
kind = if n % 2 == 0 "even" else "odd"               # simple choice
r    = if (str.len s) > 0 "full" else "empty"        # call condition → parens
```

### Iteration: `each` (the only loop)
Flux has **only one** loop — `each`. It iterates over a list, range, or map.
There is **no** `while`, `for`, or `do-while`:

```flux
each item in list           # list elements
  log item

each i in 1..5              # range: 1,2,3,4,5
  log i

each k, v in map            # map: key and value
  log "$k = $v"
```

Inside a loop:
- `skip` — move to the next iteration (in other languages `continue`)
- `stop` — exit the loop (in other languages `break`)

```flux
each n in nums
  if n < 0
    skip          # skip negatives
  if n > 100
    stop          # stop if over 100
  log n
```

> **"Where is while?"** If you need to repeat based on a condition: iterate over
> a range (`each i in 1..n`) or use recursion. One loop — one way.

### Selecting by value: `match`
Comparing one value against several variants. Mostly for symbols:
```flux
match status
  :new -> log "new"
  :confirmed -> log "confirmed"
  :cancelled -> log "cancelled"
  _ -> log "unknown"         # _ = default
```
`match` and `if` do **different things**: `if` is for a boolean condition,
`match` is for distributing one value across variants. That is why both exist.

> **⚠️ Important:** `match` only works with a **value** (symbol or number). For a
> boolean condition (like `conf > 0.85`) **always use `if/elif/else`**. Writing
> `match true` and putting conditions under it is **wrong** — do not do this:
> ```flux
> # WRONG:
> match true
>   conf > 0.85 -> ...
> # CORRECT:
> if conf > 0.85
>   ...
> ```

---

## 7. Errors (error handling)

In Flux a function can return success (`ok`) or an error (`err`). The **one**
primary way to work with errors is the `!` operator, and `??` for `nil`.

### `!` — automatically propagate the error upward
If you put `!` after a function name: if it returns an error, the error is
**automatically** propagated to the caller (you do not check it by hand). If it
succeeds, you get the result:
```flux
fn process id
  user = db.one "select * from users where id=$1" [id]!
  # if db.one returns an error, process also returns that error —
  # the next line never runs
  log user.name
```
This shrinks the multi-line `if err != nil { return err }` pattern into **a
single character**.

### `??` — an alternative if nil
If a value is `nil` (not an error, just empty), provide an alternative with `??`:
```flux
name = user.name ?? "guest"
each it in items
  p = db.one "...price..." [it.product]
  p ?? (ask_owner "Price?"; skip)    # if p is nil — ask and skip
  log p.price
```

### `fail` — raise an error
Raise an error from your own code:
```flux
if qty < 1
  fail "invalid quantity"
```

**`fail` with a status code — for expected errors.** If you give `fail` a status
code inside an HTTP handler, it **automatically** turns into a response with that
status. This replaces `try/catch`: for an expected error, instead of deep nesting
just `fail`:
```flux
http.on :post "/transfer" \req ->
  acc = db.one "select * from accounts where id=$1" [req.body.from]
  if acc.balance < req.body.amount
    fail 422 "insufficient balance"     # → 422 {error:"insufficient balance"} to the client
  # ... the main path, no nesting
```
- `fail 4xx "message"` — an **expected** (business) error → a JSON response with
  that status.
- `fail "message"` (no status) — an **unexpected** error → 500.

> **Canonical:** `!` = propagate the error, `??` = replace nil, `fail` = raise an
> error (with or without a status). Each marker has one meaning. There is **no**
> `try/catch` — `fail`+status replaces it, and the code stays flat.

---

## 8. Modules (import / export)

### `use` — import a module
You import the standard library or your own file. There is no installation
(`install`):
```flux
use http db ai json        # standard batteries — multiple modules with spaces
use ./tools                # your own file → tools.function
```
After importing, names live under the module: `db.one`, `http.serve`,
`tools.create_order`.

**`as` — renaming (alias).** If your own file has the same name as a battery (for
example an `ai.flux` file and the `ai` battery), there is a clash. Rename your
own module with `as`:
```flux
use ai                     # the battery
use ./ai as helper         # your own file → helper.classify (no clash)
```
**Rule:** do not give your own files battery names (`ai db http cron`...), or if
you do, rename them with `as`.

### `exp` — export
Expose a function or value from your file to other files:
```flux
exp fn create_order items customer
  ...
exp price_limit = 1000
```
Only things marked with `exp` are visible from the outside.

---

## 9. Batteries — the standard library

This is Flux's most powerful part. **All** the most-needed things are built into
the language. You install nothing — you just `use` it and go.

### 9.1 `http` — server and client

**Server.** You declare a route on a single line:
```flux
use http

http.on :post "/notes" \req -> rep 201 {ok:true}
http.on :get "/notes/:id" \req -> rep 200 {id:req.params.id}
http.serve 8080
```
- `http.on :method "/path" handler` — a route. The method is a symbol (`:get
  :post :put :patch :del`).
- The handler is a lambda. Its argument is `req`:
  - `req.body` — the JSON body (automatically converted to a map)
  - `req.params.id` — the `:id` in the path
  - `req.query` — query parameters (`?key=val`)
  - `req.headers` — headers
- `rep status body` — the response. If `body` is a map, it **automatically**
  becomes JSON.
- `http.serve port` — starts the server.
- `http.serve port {max_body: BYTES}` — configures the request body size limit
  (DoS guard). Default `10 MiB` (10485760 bytes); over the limit the server
  returns `413 Payload Too Large` without buffering the body. `max_body: 0`
  disables the limit (unlimited — only behind a trusted internal network).

**File upload (`multipart/form-data`).** Files sent by a browser form or
`curl -F` land in `req.files`, plain form fields in `req.body` (symmetric with
JSON):
```flux
http.on :post "/upload" \req ->
  f = req.files.0
  fs.write f.filename f.content
  rep 201 {saved:f.filename size:f.size}
```
- Each file: `{name filename content size}`. `content` is a str for UTF-8 text,
  bytes for binary (image, PDF); `size` is always the **byte** count.
- `req.files` is always a list — empty when the request is not multipart
  (`each` works without a nil check).
- The `max_body` limit applies to multipart bodies too.

**Redirect.** There is no special verb — with `rep` you give a 302 status and a
`location` key; it becomes the Location header:
```flux
http.on :get "/:code" \req ->
  link = db.one "select * from links where code=$1" [req.params.code]
  link ?? (rep 404 {error:"not found"})
  rep 302 {location:link.url}
```

**Route precedence.** If two routes overlap (`/:code` and `/stats/:code`), the
**literal (exact) path automatically wins** — regardless of the order written.
`/stats/:code` is always checked before `/:code`.

**Client.** Calling an external API:
```flux
res = http.get "https://api.example.com/data"
res = http.post url {key:"val"}      # the body becomes JSON automatically
# res.status, res.body, res.headers (a map, keys lowercased)
loc = res.headers.location           # or res.headers["content-type"]
```

A redirect (3xx) is **not followed by default** — `res.status` is 30x, and
`res.headers.location` is read. If you need automatic following, add an options
map:
```flux
res = http.get url {follow:true}         # 3xx → follows Location
res = http.get url {follow:true max:5}   # hop limit (default 10)
# res.hops — how many redirects happened
```
Exceeding `max` is an error. The options map is the last argument:
`http.post url body {follow:true}`.

**Custom request headers.** For APIs that require authentication (`x-api-key`,
`Authorization`, `anthropic-version`...), add `headers` to the options map — this
is symmetric with `res.headers` in the response:
```flux
res = http.post "https://api.anthropic.com/v1/messages" body {
  headers: {
    "x-api-key": env.ANTHROPIC_API_KEY
    "anthropic-version": "2023-06-01"
  }
}
```
If a header value is not a string it is converted to text; a header with a `nil`
value is dropped. If the user provides `content-type`, that is used instead of
the automatic `application/json`.

### 9.2 `db` — database (Postgres)

The connection is **automatic**: it is read from the `$DATABASE_URL` environment
variable. You write no connection code.

```flux
use db

# Query — the result is a list of maps
rows = db.q "select * from products where owner=$1" [owner_id]

# A single row (or nil)
user = db.one "select * from users where id=$1" [id]

# Insert — returns the inserted row
row = db.ins "orders" {cust:5 total:0 status::new}

# Update — db.up "table" {changes} {condition}
db.up "orders" {total:1500} {id:order_id}

# Delete — db.del "table" {condition}
db.del "cart_items" {id:item_id}

# UPSERT — db.put "table" {changes} {key}
# updates if it exists by key, inserts if not (atomic)
db.put "agent_memory" {val:v} {agent:aid key:k}
```

> **Why is `db.put` needed?** For the "update if it exists, insert if not"
> pattern (memory, cache, counters). If you did this by hand with `db.one` + `if`
> + `db.ins`, two parallel requests might both see "not there" and insert twice
> (a race). `db.put` makes it atomic.

**Transactions — `db.tx`.** If a multi-step mutation must be **atomic** (for
example checkout: order + line items + decrementing stock), wrap it in a `db.tx`
block. If an error (`fail` or `!`) occurs inside the block, **all** changes are
**rolled back** — the DB never stays in a half-finished state:
```flux
db.tx \->
  ord = db.ins "orders" {cust:c.id total:total}
  each it in items
    db.ins "order_items" {ord:ord.id prod:it.id qty:it.qty price:it.price}
    db.up "products" {stock:it.stock - it.qty} {id:it.id}
  db.up "carts" {status::converted} {id:cart.id}
  # if it reaches the end of the block — commit. If a fail happens midway — all cancelled.
```

`db.tx` can also return a value (via `ret`):
```flux
ord = db.tx \->
  o = db.ins "orders" {...}
  ret o            # the block value goes outside
```

**Concurrency (parallel requests) guarantee.** `db.tx` automatically runs at the
strongest isolation and **automatically retries** on conflict. This means the
"read → check → modify" pattern is safe. For example, two parallel withdrawals
from one account — both see the same balance, and they do not both go through (no
overdraft):
```flux
db.tx \->
  acc = db.one "select * from accounts where id=$1" [aid]
  if acc.balance < amt
    fail 422 "insufficient balance"
  db.up "accounts" {balance:acc.balance - amt} {id:aid}   # race-safe
```
> In other languages you would write `SELECT FOR UPDATE`, locks, or mutexes for
> this. In Flux it is not needed — `db.tx` guarantees it itself. "The language
> adapts to the AI": the AI does not think about locks, it just writes inside
> `db.tx`.

**Idempotency — not performing the same operation twice.** In places like money
transfers, a client may resend a request. Protect it with a unique key (a `uniq`
column): first check whether it exists, then write the key inside a transaction —
if it is a duplicate, the `uniq` error → tx rollback:
```flux
old = db.one "select * from transactions where ikey=$1" [key]
old ?? (ret old)              # already done → return the old result
db.tx \->
  db.ins "transactions" {ikey:key amount:amt ...}   # duplicate → uniq → rollback
  # ... transfer the money
```
> This is **mandatory** for places like e-commerce checkout. Without a
> transaction, if an error happens midway, you can end up with some stock
> decremented but no order created.
- Parameters via `$1, $2...`, values passed as a list `[...]`.
- In `db.ins`/`db.up`, the map keys are column names.
- **A query without parameters** does not need a list: `db.q "select * from links"`.
- An **aggregate (count/sum)** can return `nil` on an empty table — protect it
  with `?? 0`:
  ```flux
  r = db.one "select count(*) c, sum(clicks) s from links"
  log "links: ${r.c}, clicks: ${r.s ?? 0}"
  ```

**Schema declaration — `tbl`.** You declare tables in Flux itself:
```flux
tbl products
  id     serial pk
  owner  int ref:users.id
  name   str
  price  money
  status sym index|uniq      # multiple modifiers on one column → pipe `|`
  ts     now

  index(owner status)        # multi-column index (space-separated, no commas)
  uniq(owner price)          # multi-column unique
```
Type keywords: `serial int flt str bool json now sym money`. Modifiers: `pk`
(primary key), `uniq`, `index`, `null`, `ref:table.column` (foreign key).

**Indexes and uniqueness.** For a single column, append a word modifier: `index`,
`uniq`. To put **both** on one column the canonical form is `|` (pipe):
`status sym index|uniq`. The spaced form (`index uniq`) is also accepted. For
**multi-column**, use a separate parenthesized line: `index(a b)`, `uniq(a b)` —
space-separated by default (no commas, to save tokens); a comma is also accepted:
`index(a, b)`. **Index names are automatic** (`idx_<table>_<cols>` /
`uniq_<...>`) — you never invent a name. A name that is too long (DB limit is 63
bytes) is automatically shortened (with a deterministic hash suffix); your code
never breaks.

**Declarative migration — `tbl` is the single source of truth.** You only write
the latest shape of the `tbl`; Flux diffs it against the current DB and runs the
necessary DDL **itself**:
- new column → `ADD COLUMN`;
- column removed from `tbl` → `DROP COLUMN` (the table is first backed up to
  `_flux_bak_*`);
- a `tbl` removed entirely → `DROP TABLE` (with backup; **only Flux-managed**
  tables — a manually created table is never touched);
- index added/removed → `CREATE/DROP INDEX`.

Migration is **idempotent** — re-deploying the same `tbl` is safe, nothing
breaks. No migration SQL needed for schema changes. Type changes and renames are
**not** automatic — do those manually with `db.q "ALTER TABLE ..."`, and Flux
syncs the rest afterward.

**A `json` column** — when read it **automatically becomes a map/list** (not a
string, no need for `json.dec`); when written, a map/list is automatically
encoded.

**The `money` type — for money.** Money should NEVER be a `flt` (float) — float
rounding errors corrupt money. `money` is a whole number of **minor units**
(tiyin, cents): `15000` = 150.00 so'm. All money math uses `money`/`int` (`int`
is 64-bit):
```flux
tbl accounts
  id      serial pk
  balance money       # in tiyin, e.g. 15000 = 150.00
total = price * qty   # int math, not float
```

**The `sym` type — for enums.** This is Flux's elegant solution. If a column is
`sym`: the DB stores **text**, but when Flux reads it, it automatically returns a
**symbol**. On writing and filtering, a symbol is automatically converted to
text. Then `match` works directly:
```flux
tbl tickets
  category sym         # DB: text ("billing"), Flux: symbol (:billing)
  status   sym

# Writing: you give a symbol, the DB stores text
db.ins "tickets" {category::billing status::new}

# Reading: if the schema says sym, Flux returns a symbol
t = db.one "select * from tickets where id=$1" [id]
match t.category       # t.category is a symbol, so match works
  :billing -> log "billing matter"
  :technical -> log "technical"
  _ -> log "other"

# Filtering: a symbol is passed, automatically converted to text
db.q "select * from tickets where category=$1" [:billing]
```
**One rule:** a `sym` column — text in the DB, a symbol in Flux, conversion
automatic.

### 9.3 `ai` — LLM (a first-class primitive)

This is the biggest thing that sets Flux apart from other languages. The LLM is a
keyword, not an SDK. **The provider is detected automatically** (OS env or
`.env`) — you configure nothing:

- if `ANTHROPIC_API_KEY` is set → Claude (default `claude-opus-4-8`)
- if `OPENAI_API_KEY` is set → GPT (default `gpt-4o`)
- if both are set, Anthropic wins. Override: `$AI_PROVIDER`
  (`anthropic|openai`), `$AI_KEY` (a shared key), `$AI_MODEL` (the model name).

This adapts to common standard names like `OPENAI_API_KEY`/`ANTHROPIC_API_KEY` —
the same `.env` works with other tools.

```flux
use ai

# Simple question-and-answer → text
answer = ai.ask "Translate this message into English: ${text}"

# Structured extraction (typed extraction) → a map according to the schema
schema = {
  intent: ":new_order|:question|:other"
  items: [{product:str qty:int}]
}
r = ai.json "Extract the order: ${text}" schema
# r.intent, r.items[0].product ...

```

**Audit metadata — automatic.** Each `ai.*` result carries metadata under `_`:
```flux
r = ai.json prompt schema
log r._.conf        # confidence score (0..1)
log r._.tokens      # tokens used
log r._.cost        # cost
log r._.ms          # latency (milliseconds)
```
This is the basis for confidence routing:
```flux
if r._.conf > 0.85
  auto_answer r         # high confidence → automatic
elif r._.conf >= 0.6
  ask_owner r           # medium → ask for confirmation
else
  escalate_to_owner r   # low → full escalation
```

> **Note:** `_.conf` is the calibrated confidence returned by the LLM battery. In
> real life this should be backed by logprobs or self-eval; the language hides
> this behind the battery.

**`ai.run` — agent tool loop (ONE step).** If the AI wants to use a tool,
`ai.run` does **not** execute it itself — it returns to you *what it wants to
do*. You run the tool (with logging, cost, confirmation) and return the result.
The loop is **manual** — this gives you full control:
```flux
msgs <- [{role::user content:text}]
each i in 1..10                          # maximum 10 steps
  r = ai.run msgs tools                  # tools: a list of [{name desc params}]
  if r.kind == :final
    ret r.text                           # AI is done → final answer
  # r.kind == :call → the AI wants to call tools. The model may call several
  # in parallel → all are in r.calls; return a result for EACH one.
  each c in r.calls
    out = reg.call c.tool c.args         # run the tool by name (see below)
    log "tool ${c.tool}"                 # logging/cost/confirmation goes here
    msgs <- msgs.push {role::tool id:c.id content:(json.enc out)}
```
> `r.tool`/`r.args`/`r.id` mirror `r.calls[0]` for back-compat (single-tool code
> still works). But on parallel calls you must return a result for every
> `tool_use_id`, otherwise the next request 400s.
> `ai.run` is intentionally single-step. If you let the AI's tool calls run
> automatically and uncontrolled, you could not do logging/cost/confirmation. The
> loop is yours — so you see and control every tool call.

### 9.4 `reg` — function registry (dynamic dispatch)

Storing and calling a function **by its string name**. Essential for agent tools:
the AI gives you the tool **name** (text), and you must turn it into a function
and call it.

```flux
reg.add "calc" \args -> args.a + args.b          # name → function
reg.add "search" \args -> http.get "/s?q=${args.q}"

out = reg.call "calc" {a:2 b:3}                  # call by name → 5
reg.has "search"                                  # is it in the registry → bool
reg.names                                         # a list of all names
```

> **Why is `reg` needed?** Otherwise, you would have to execute the tool name
> coming from the AI with `match name` (a hardcoded switch) — changing the code
> for each new tool. With `reg`, tools are added **at runtime** (`reg.add`), and
> the AI calls any of them with `reg.call`. You simply cannot build an agent
> platform without this.

### 9.5 `list` methods, `str` / `math` / `rand` / `time` — the core

All of these are **core** — they work without `use` (just like `log`).

**`list` — list methods** (on a value, `.method`):
```flux
l.len                  # length
l.push x               # adds an element → a new list
l.filter \x -> x > 0   # keeps those matching the condition → a new list
l.map \x -> x * 2      # transforms each → a new list
l.has x                # is it inside → bool
l.index x              # index of the first matching element, -1 if not found
l.find \x -> x > 4     # first element matching the predicate, nil if not found
l.0  l.1               # element by index
l.slice a b            # the a..b range (b excluded) → a new list
l.join ", "            # → text: [1 2 3].join "," → "1,2,3"
l.reduce 0 \acc x -> acc + x   # accumulate: (initial value, function)
l.sort                 # natural order (numbers or strings) → a new list
l.sort \a b -> a.p - b.p   # comparator returns a number: negative → a first
l.reverse              # reversed order → a new list
l.uniq                 # removes duplicates (first occurrence kept)
l.flat                 # flattens one level: [[1 2] [3]] → [1 2 3]
l.zip other            # pairs up: [1 2].zip ["a" "b"] → [[1 "a"] [2 "b"]]
l.any \x -> x > 4      # does any match → bool (stops at first match)
l.all \x -> x > 0      # do all match → bool (stops at first mismatch)
```

> **Important:** to build a list use `l.push x`, **not** `l + [x]`. To filter, use
> `l.filter` instead of a manual `each` loop; to build text, use `l.join` instead
> of a manual accumulator:
> ```flux
> # Manual (long):              With methods (clean):
> result <- []                  result = items.filter \t -> t.active
> each t in items
>   if t.active
>     result <- result.push t
>
> text <- ""                    text = names.join ", "
> each n in names
>   text <- text + n + ", "
> ```

**`map` — key-value methods** (on a value, `.method`):
```flux
m.set k v              # sets/updates a key → a new map
m.del k                # removes a key → a new map
m.merge other          # merges two maps (other's keys win) → a new map
m.has k                # is the key present → bool
m.keys                 # a list of keys
m.vals                 # a list of values
m.key   m[k]           # read (m[k] — dynamic, variable key)
```
> **Important:** to **write** to a map use `m.set k v`. `m[k]` only **reads**
> (does not write). This is consistent with lists: `push` for a list, `set` for a
> map. Shared state (for example, who is in which room in realtime) is managed
> with these methods.

**`str` — text functions:**
```flux
str.len s              # length (number)
str.slice s 0 3        # the 0..3 range (3 excluded): "hello" → "hel"
str.up s               # UPPERCASE
str.low s              # lowercase
str.split s ","        # split → a list: "a,b" → ["a" "b"]
str.has s "part"       # is it inside → bool
str.int "42"           # text → number
str.str 42             # number → text
str.trim "  s  "       # strips leading/trailing whitespace → "s"
str.replace s "-" "+"  # replaces every "-" with "+"
str.starts s "/api"    # starts with prefix → bool
str.ends s ".fx"       # ends with suffix → bool
str.pad "7" 3 "0"      # pads on the LEFT → "007"
str.repeat "ab" 3      # repeat → "ababab"
```

> **Why is `str.len s` different from `list.len` on a list?** List length is a
> member (`list.len`), text length is a module function (`str.len s`). The reason:
> a list and text are separate types, and their operations should not mix. If both
> were the same `.len` it would be confusing.

**`math` — math:**
```flux
math.floor 3.7         # → 3
math.ceil 3.2          # → 4
math.abs -5            # → 5
math.min 3 7           # → 3 (ints in, int out)
math.max 3 7           # → 7
math.pow 2 10          # → 1024 (int ^ non-negative int → int)
math.sqrt 9            # → 3.0 (always flt; negative input is an error)
```

**`rand` — random:**
```flux
rand.int 1 100         # a random integer in the range 1..100
rand.str 6             # a random string of 6 characters (short codes)
```

`rand` is backed by the OS cryptographic CSPRNG, so its output is not
predictable. But **length matters too**: `rand.str 6` yields only ~36 bits of
entropy (62⁶) — fine for a short code, but brute-forceable as a secret. For
session IDs, tokens, and other secrets use at least `rand.str 24` (~140+ bits).

**`time` — time and date:**
```flux
time.now               # the current time (timestamp)
time.ago 24 :hr        # the time 24 units ago. Units: :sec :min :hr :day
time.in  60 :min       # the time 60 units later (TTL/expiry). Same units
time.fmt t "..."       # format a timestamp into text
time.sleep 1           # waits 1 second (flt too — 0.5). Polling/retry backoff
time.parse "2026-06-10T10:00:00Z"   # arbitrary ISO text -> canonical UTC timestamp ("Z"/"±HH:MM")
time.add t 30 :min     # offset from ANY timestamp (not now): end_at = start_at + duration
time.sub t 5 :min      # mirror of time.add — shift backward (e.g. buffer before)
time.diff a b          # (a - b) difference in seconds (int); / 60 -> minutes
```
> Difference between `time.in`/`time.ago` (offset from **now**) and
> `time.add`/`time.sub` (offset from **any** given timestamp): a booking server
> computes `end_at = time.add start_at 30 :min` from a client-supplied `start_at`.
> Instead of writing raw `now() - interval '24 hours'` in a DB query, use
> `time.ago` — it is clean and safe:
> ```flux
> r = db.one "select count(*) c from tickets where created > $1" [time.ago 24 :hr]
> ```

**Duration & interval recipes** (interval arithmetic IS available — `time.add`/`diff` exist):
```flux
end_at = time.add start_at dur :min            # duration: start + dur minutes
mins   = (time.diff end_at start_at) / 60       # gap between two times -> minutes
overlap = a.start < b.end & a.end > b.start     # do two intervals overlap? (bool)
buf_start = time.sub start_at 15 :min           # buffer: 15 min before start
```

**IANA timezone / DST** — `time.parse` takes an optional zone name; `time.fmt`
takes an optional zone as a third argument. Wall-clock ↔ UTC conversion is
DST-aware (NOT a fixed offset), so "09:00 local every day" lands on the correct
UTC instant across summer/winter transitions:
```flux
utc = time.parse "2026-07-15 09:00:00" "Asia/Tashkent"   # local wall-clock -> UTC
loc = time.fmt utc "HH:mm" "America/New_York"             # UTC instant -> zone wall-clock
```
> A wall-clock time in a spring-forward gap (e.g. `02:30` on the night clocks
> jump) does not exist and raises an error; an unknown zone name raises too.

### 9.6 `json`
```flux
use json
s = json.enc value     # value → JSON text
v = json.dec str       # JSON text → value
```

### 9.7 `env` — environment variables
```flux
port = env.PORT ?? "8080"      # directly env.NAME
key = env.AI_KEY
```

### 9.8 `cron` — scheduling
A standard **Unix 5-field** cron expression: `minute hour day month weekday`.
Every AI agent knows this format (crontab, GitHub Actions, ...). `cron.on` reads
the expression **without quotes** — here `*` is not multiplication, it is a cron
marker:
```flux
use cron
cron.on 0 * * * * check_prices    # at the start of every hour (minute=0)
cron.on 30 9 * * * daily_check    # every day at 09:30
cron.on 0 18 * * 0 briefing       # Sunday (0) at 18:00
cron.on */15 * * * * poll         # every 15 minutes
cron.on 0 9 * * 1-5 \->           # weekdays at 09:00 (inline lambda)
  log "weekday"
```
Fields: `*` any value, `*/N` every N, `A-B` a range, `A,B,C` a list. Weekday:
0=Sunday ... 6=Saturday.

`cron.on` **does not block** — like `http.on` it just registers, and the
scheduler runs in the background. A server (`http.serve`/`ws.serve`) keeps the
process alive, and cron runs in the background at its scheduled times. Order:
`cron.on` calls go **before** `http.serve`.

For a cron-only script (no server) — `cron.run` takes over the process:
```flux
cron.on 0 9 * * * daily_check
cron.run                          # blocks: the program does not end, cron keeps running
```

> Convenience: you can also write the expression with quotes (`cron.on "0 9 * *
> *" f`) — the result is the same. For an AI the canonical form is without quotes
> (fewer tokens).

### 9.9 `queue` — background queue
So a webhook can respond quickly, you offload heavy work to the background:
```flux
use queue

queue.on "send" \job -> tools.send job.ph job.body   # the handler
queue.push "send" {ph:phone body:text}               # add to the queue
```

- `queue.on <name> <handler>` — the handler for jobs with this name. The handler
  takes a single `job` argument — this is the payload given to `queue.push` (a
  map).
- `queue.push <name> <payload>` — adds a job to the queue. The payload is optional
  (if not given, `nil`). It **does not block** — it returns immediately, and the
  job runs in the background.
- Jobs run **on a single worker thread, FIFO (in arrival order)** — ordering is
  guaranteed. An error inside a handler does not kill the worker.
- If `push` is written before `on`, the job **waits in the queue** and runs once
  the handler is registered (order-independent).
- The worker is a background thread — it processes the queue while a server
  (`http.serve`/`ws.serve`) or `cron.run` holds the process. In a queue-only
  script you need one of these blocking calls to hold the process.

### 9.10 `ws` — websocket (realtime)

For real-time applications (chat, live updates). Where `http` is
request-response, `ws` is a persistent two-way connection.

```flux
use ws

ws.on :connect \conn ->         # a new connection. conn.id — a stable unique id
  ws.data.set conn :user nil    # ws.data — session state for THIS connection

ws.on :message \conn msg ->     # msg — the incoming text (if JSON, json.dec it)
  m = json.dec msg
  ws.send conn (json.enc {ok:true})    # reply to THIS connection

ws.on :disconnect \conn ->
  ws.room.leave conn "ch:5"

ws.serve 9000
```

- `ws.on :event handler` — events: `:connect`, `:message`, `:disconnect`. The
  `:message` handler is `\conn msg ->` (msg — the incoming **text**), the others
  are `\conn ->`.
- `ws.send conn text` — sends to THIS connection (text; if you need JSON,
  `json.enc`).
- `ws.data.set conn :key value` / `ws.data.get conn :key` — session state for
  THIS connection (Flux keeps it until the connection drops, and clears it on
  disconnect).
- `ws.serve port` — starts the server (blocking).

**Rooms — for broadcast.** Sending to a group at once. Flux manages rooms itself
— you do not maintain a manual "who is in which room" map:
```flux
ws.room.join conn "ch:5"                          # add the connection to a room
ws.room.leave conn "ch:5"                         # remove it from the room
ws.room.send "ch:5" (json.enc {t:"msg" body:b})   # send to EVERYONE in the room
ws.room.members "ch:5"                            # the room members (for presence)
```

> `http.serve` and `ws.serve` work **together** (on different ports). Room
> membership and presence are managed inside `ws.room` — no manual shared-state
> map is needed.

### 9.11 `log` — printing to stderr
```flux
log "message"          # to stderr for diagnostics
```

---

## 10. A complete small program (all together)

```flux
use http db ai json

tbl notes
  id   serial pk
  text str
  ts   now

http.on :post "/notes" \req ->
  note = db.ins "notes" {text:req.body.text}
  rep 201 note

http.on :get "/notes" \req ->
  rep 200 (db.q "select * from notes order by ts desc")

log "server on :8080"
http.serve 8080
```

Here is the whole language. `use` it, declare a table with `tbl`, a route with
`http.on`, storage with `db` — no packages, no connection code, no boilerplate.
