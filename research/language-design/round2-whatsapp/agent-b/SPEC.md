# Nol — Language Specification

**Philosophy**: Structured English commands. Every line is a declaration or an action. No punctuation ceremony.

---

## 1. Core Syntax Rules

- **No semicolons.** Newlines terminate statements.
- **No braces.** Indentation (2 spaces) defines blocks.
- **No type annotations.** Types inferred always.
- **Comments**: `-- text` (inline or full-line).
- **Strings**: `"double-quoted"`, multiline with `"""..."""`.
- **Interpolation**: `"Hello {name}"`.
- **Numbers**: `42`, `3.14`. Booleans: `yes`, `no`. Null: `nil`.
- **Lists**: `[a, b, c]`. Maps: `{key: val, key2: val2}`.

---

## 2. Variables & Assignment

```
let x = 42
let name = "Firdavs"
let items = [1, 2, 3]
let config = {host: "localhost", port: 5432}
```

`let` is the only assignment form. Re-assign with `let x = new_val`.

---

## 3. Functions

```
fn greet(name)
  return "Hello {name}"

fn add(a, b)
  return a + b
```

No `def`, no `func`, no `function` — only `fn`. Last expression is NOT auto-returned; always use `return`.

---

## 4. Control Flow

```
-- Conditional (one form only)
if condition
  ...
else if other
  ...
else
  ...

-- Loop over collection
each item in list
  ...

-- Loop with index
each item, i in list
  ...

-- While-style loop
loop
  break if done
  ...
```

No `for`, no `while`, no `do`. Only `if`, `each`, `loop`.

---

## 5. Error Handling

```
try
  let result = risky_op()
catch err
  log "Error: {err.message}"
```

Functions that can fail return either a value or raise. No checked exceptions.

---

## 6. Pattern Matching

```
match value
  "new_order"  -> handle_order()
  "question"   -> handle_question()
  _            -> handle_other()
```

`_` is the wildcard/default case. Only `match` — no `switch`, no `case`.

---

## 7. Modules & Imports

```
import tools
import schema.orders as orders_table
```

One `import` form. Files in same directory are importable by name without path.

---

## 8. HTTP Server

```
serve 3000
  post "/webhook"
    handle_webhook(body)

  get "/health"
    return {status: "ok"}
```

`body` is the parsed JSON request body (auto-parsed). Response is the return value (auto-serialized to JSON).

---

## 9. Database (Postgres)

```
-- Connect (called once in main)
db.connect(env.DATABASE_URL)

-- Query returning rows
let rows = db.query("SELECT * FROM orders WHERE customer_id = ?", [id])

-- Single row
let row = db.one("SELECT * FROM users WHERE id = ?", [id])

-- Execute (insert/update/delete), returns affected rows or inserted id
let id = db.exec("INSERT INTO orders(customer_id) VALUES(?)", [cid])

-- Transaction
db.tx
  db.exec("UPDATE products SET price = ? WHERE id = ?", [price, pid])
  db.exec("INSERT INTO order_items(...) VALUES(...)", [...])
```

---

## 10. LLM / AI

```
-- Simple completion
let reply = ai.complete(prompt)

-- Structured extraction: returns parsed map matching schema
let result = ai.extract(prompt, {
  intent: "string",
  items: [{product: "string", qty: "number"}],
  confidence: "number"
})

-- Tool-use: AI can call named functions
let result = ai.run(prompt, tools: [get_catalog, create_order])
```

`ai.extract` validates the output against the schema and retries once on failure.

---

## 11. HTTP Client

```
let resp = http.post("https://api.example.com/send", {
  headers: {"Authorization": "Bearer {env.TOKEN}"},
  body: {to: phone, text: message}
})

let data = http.get("https://api.example.com/resource")
```

Returns parsed JSON automatically. `resp.status`, `resp.body` available.

---

## 12. Cron / Scheduling

```
cron "0 9 * * MON"
  weekly_outreach()

cron "0 20 * * SUN"
  sunday_briefing()
```

Standard cron expressions. Block runs in background worker.

---

## 13. Queue

```
queue.push("outreach", {customer_id: 42, reason: "weekly"})

queue.on "outreach"
  process_outreach(job.data)
```

In-process queue with named channels. `job.data` is the payload.

---

## 14. Environment & Logging

```
env.DATABASE_URL        -- read env var
env.get("KEY", "default") -- with default

log "message"           -- stdout with timestamp
log.warn "message"
log.error "message"
```

---

## 15. JSON

```
let obj = json.parse(raw_string)
let str = json.dump(obj)
```

DB results, HTTP bodies, `ai.extract` outputs are already maps — no manual parse needed in most cases.

---

## 16. Type Summary

| Value     | Literal         |
|-----------|-----------------|
| String    | `"text"`        |
| Number    | `42`, `3.14`    |
| Boolean   | `yes`, `no`     |
| Null      | `nil`           |
| List      | `[a, b]`        |
| Map       | `{k: v}`        |

Map fields accessed with `.`: `user.name`, `order.items[0].qty`.

---

## 17. String Ops & Guards

```
-- Null-safe access
let name = user.name or "Unknown"

-- Existence check
if user.phone
  ...

-- Comparison
if score > 0.85
  ...
if intent == "new_order"
  ...
```

Boolean operators: `and`, `or`, `not`. No `&&`, `||`, `!`.
