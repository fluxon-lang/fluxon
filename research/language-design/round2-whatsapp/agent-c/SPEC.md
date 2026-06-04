# QUILL — Minimalist Multi-Effect Language

**Philosophy**: Terse, single-purpose, batteries-first. Every keyword earns its token count. No redundancy. Reads as pseudocode but runs as code.

## Core Syntax (Minimal)

### Values & Types
```
42              # int
3.14            # float
"hello"         # string (double-quote only)
true false      # bool
null            # null
[1 2 3]         # list: [elem elem...]
{x: 1 y: 2}     # record: {key: val key: val}
```

### Variables & Binding
```
let x = 42
x = x + 1       # reassign
```

### Functions (all curried, 0+ params)
```
fn name(a b) { a + b }
fn zero() { 42 }
name(1 2)       # call: space-separated args
map(inc [1 2 3])  # higher-order: fn as first arg
```

### Control Flow (one of each, canonical)
```
if cond { stmt } else { stmt }
loop i in list { stmt }        # only loop form
cond ? yes : no                # ternary (same as if/else)
```

### Operators (minimal, left-assoc, no precedence surprises)
```
+ - * / %       # arithmetic
== != < > <= >= # comparison
and or not      # boolean (short-circuit)
++              # string/list concat
@ a b           # error: raise b with msg a (custom error)
```

## Module & Import (single form)
```
use "module/name"        # imports all exports from file
use pkg::symbol          # standard library
```

## Error Handling
```
try { risky() } catch e { handle(e) }
@ "error: invalid qty" null   # raise as error
```

## Standard Library (Batteries)

### JSON
```
pkg::json::encode(data)   # → string
pkg::json::decode(string) # → object
```

### HTTP Server (webhook listener)
```
pkg::http::server(port)
  .on("POST", "/webhook", fn(req) {
    body = req.body
    pkg::http::reply(200, {status: "ok"})
  })
  .start()
```

### HTTP Client
```
resp = pkg::http::post("https://api.whatsapp.com/send", headers, body)
resp.status     # int
resp.body       # string
```

### Database (Postgres)
```
db = pkg::db::connect(url)
db.exec("CREATE TABLE users (id int PRIMARY KEY, name text)")
rows = db.query("SELECT * FROM users WHERE id = ?", [id])
db.exec("INSERT INTO users VALUES (?, ?)", [id, name])
db.close()
```

### LLM / AI Calls (streaming + non-streaming)
```
result = pkg::llm::call({
  model: "gpt-4o-mini"
  messages: [{role: "user" content: "classify: ..."}]
  tools: [tool1 tool2]
})
result.text         # string
result.tool_calls   # [{name: "fn" args: {...}}]
result.cost         # float
```

### Env Vars
```
api_key = pkg::env::get("WHATSAPP_API_KEY")
pkg::env::set("MY_VAR", value)
```

### JSON-RPC / Tool Calls (for LLM callbacks)
```
pkg::tools::define(name, desc, schema, fn(args) {
  # args auto-decoded from JSON
  # return becomes tool result JSON
})
```

### Cron Scheduling
```
pkg::cron::job("0 0 * * 0", fn() {  # weekly Sunday midnight
  # runs this fn
})
```

### Time
```
now = pkg::time::now()      # unix timestamp (float)
pkg::time::schedule(when, fn) # schedule once at unix timestamp
```

### File I/O
```
pkg::file::read(path)       # → string
pkg::file::write(path, text)
pkg::file::append(path, text)
```

### Queue / Pubsub (in-memory for now, redis-compatible)
```
q = pkg::queue::new("messages")
q.push({data: val})
item = q.pop()      # blocking
q.subscribe(fn(msg) { handle(msg) })
```

### Logging
```
pkg::log::info("msg")
pkg::log::error("msg")
pkg::log::debug("msg")
```

## Type Hints (optional, non-enforced)
```
fn add(a: int b: int): int { a + b }
let x: string = "hello"
```

## Notable Absences (to meet constraint #3)
- No class/OOP syntax (use records + functions)
- No `var` (only `let`)
- No `while` (only `loop`)
- No `case/switch` (use if/else chains)
- No operator overloading
- No generics syntax (duck-typed at runtime)

## Example: Complete mini-program
```
use pkg::http
use pkg::db

fn greet(name) { "Hello, " ++ name }

let port = 3000
let db_url = pkg::env::get("DATABASE_URL")
db = pkg::db::connect(db_url)

http::server(port)
  .on("POST", "/hello", fn(req) {
    body = pkg::json::decode(req.body)
    msg = greet(body.name)
    http::reply(200, {message: msg})
  })
  .start()
```

## Comments
```
# single-line comment
```

## String Interpolation
```
name = "Alice"
s = "Hello, {name}!"   # interpolation with {}
```
