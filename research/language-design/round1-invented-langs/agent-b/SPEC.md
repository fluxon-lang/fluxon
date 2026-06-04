# Flux Language Specification

**Flux** — a terse, readable language where structure IS meaning.
Extension: `.fx` | Philosophy: *Minimum syntax, maximum intent.*

---

## 1. Core Rules

- Indentation (2 spaces) defines blocks — no braces, no `end`
- Sigils declare type at definition; omit sigil when referencing
- Implicit return: last expression in a fn is its value
- `;` never needed — newline terminates a statement
- Comments: `--` to end of line

---

## 2. Types & Sigils

| Sigil | Type    | Example                         |
|-------|---------|---------------------------------|
| `$`   | string  | `$name = "ada"`                 |
| `#`   | number  | `#count = 42`                   |
| `?`   | bool    | `?flag = true`                  |
| `@`   | list    | `@items = [1, 2, 3]`            |
| `%`   | map     | `%opts = {a: 1, b: "x"}`       |
| `~`   | any     | `~val = json.parse(raw)`        |

Sigil only appears on **first assignment**. References use bare name.

```
$city = "Paris"
$msg = "Hello " + city    -- bare 'city', no sigil
```

---

## 3. Functions

```
fn greet $name -> $
  "Hello " + name          -- implicit return

fn add #a #b -> #
  a + b
```

- `fn <name> [params] -> <return-sigil>` — typed return hint is optional but good
- Params use sigils: `$name`, `#count`, `@list`, `%map`, `~anything`
- Call: `greet "world"` or `add 3 4` — no parens needed for single/chained calls
- Parens for grouping or multi-arg disambiguation: `add(3, 4)`

---

## 4. Control Flow

**Conditional** — `if / elif / else`:
```
if x > 10
  show "big"
elif x > 5
  show "mid"
else
  show "small"
```

**One loop form** — `each`:
```
each #i in 1..10         -- range (inclusive)
  show i

each $item in items      -- iterate list
  show item

each ?running            -- while-style: loop while bool is true
  ~data = fetch()
  ?running = data != nil
```

**Break / skip**:
```
break        -- exit loop
skip         -- continue to next iteration
```

---

## 5. Error Handling

```
try
  ~data = db.query!("SELECT * FROM t")   -- ! = may throw
  show data
catch $err
  show "Failed: " + err
```

- `!` suffix on a call signals it can throw — purely documentary/linting hint
- `fail $msg` — throw an error from user code
- `ok ~val` / `err $msg` — result-pair return (optional pattern)

---

## 6. Pipeline Operator `|>`

Chain transforms without nesting:

```
@result = items |> filter(fn ~x -> x > 2) |> map(fn ~x -> x * 10)
```

---

## 7. Pattern Matching

```
match status
  200 -> show "ok"
  404 -> show "not found"
  _   -> show "other: " + status
```

---

## 8. Maps & Lists

```
%user = {name: "ada", age: 30}
show user.name              -- dot access
user.age = 31               -- mutation
@keys = user.keys()

@nums = [1, 2, 3]
nums.push(4)
#len = nums.len()
~first = nums[0]
@sliced = nums[1..3]
```

---

## 9. Modules

No package manager. Standard library is built-in. User modules are files.

```
use fs                     -- stdlib module
use http                   -- stdlib module
use ./utils                -- local file: utils.fx
use ./models/user          -- local file: models/user.fx

-- Import specific names:
use db { connect, query }
```

Exported names: any top-level `fn` or `let` is exported automatically.

`let` declares a module-level constant (no sigil, value inferred):

```
let VERSION = "1.0.0"
let PORT = 8080
```

---

## 10. Standard Library (Batteries)

### `show` — print (built-in, no import)
```
show "hello"
show val
```

### `env` — environment variables
```
$port = env.get("PORT") or "3000"
```

### `fs` — file I/O
```
use fs
$raw = fs.read!("data.json")
fs.write!("out.txt", content)
?exists = fs.exists("file.txt")
```

### `json` — JSON encode/decode
```
use json
~data = json.parse!(raw)
$str = json.encode(data)
```

### `args` — CLI arguments
```
use args
$cmd = args[0]
@rest = args[1..]
```

### `http` — HTTP server
```
use http
http.get "/hello" fn req res ->
  res.send "Hello world"
http.serve 3000
```

Routes: `http.get`, `http.post`, `http.put`, `http.delete`
`req.params`, `req.body`, `req.query` — request data
`res.send`, `res.json`, `res.status(#).json(~)` — responses

### `db` — database (SQLite by default, Postgres with URL)
```
use db
db.open "app.db"                              -- SQLite
db.open env.get("DB_URL")                    -- Postgres/MySQL by url
@rows = db.query!("SELECT * FROM notes")
db.exec!("INSERT INTO notes VALUES (?)", [val])
```

### `ws` — WebSockets
```
use ws
ws.on "connect" fn client ->
  client.send "welcome"
ws.on "message" fn client $msg ->
  ws.broadcast msg
ws.serve 4000
```

### `time` — timers & dates
```
use time
time.now()           -- current unix ms
time.sleep(500)      -- ms
```

### String methods (built-in on $ values)
```
str.upper() / str.lower() / str.trim()
str.split($sep) -> @
str.has($sub) -> ?
str.replace($from, $to) -> $
str.len() -> #
```

### Number methods
```
#x = num.floor(3.7)
$s = num.str(42)
```

### List methods (built-in on @ values)
```
list.push(~val)
list.pop() -> ~
list.len() -> #
list.has(~val) -> ?
list.filter(fn) -> @
list.map(fn) -> @
list.find(fn) -> ~
list.del(#i)           -- delete by index
```

### Map methods (built-in on % values)
```
map.keys() -> @
map.vals() -> @
map.has($key) -> ?
map.del($key)
```

---

## 11. Concurrency

`go` spawns a non-blocking task (goroutine-style):

```
go
  ~result = slow.op()
  show result
```

`lock` / `unlock` for shared state:

```
let %state = {}
lock state
  state.users = []
unlock state
```

---

## 12. Quick Reference

```
$x = "val"          -- string var
#n = 42             -- number var
?b = false          -- bool var
@a = [1,2,3]        -- list var
%m = {k: "v"}       -- map var
fn f $a -> $        -- function
if / elif / else    -- conditional
each x in list      -- loop (all forms)
match x / _ ->      -- pattern match
try / catch $e      -- errors
fail $msg           -- throw
use mod             -- import
go { ... }          -- async task
show val            -- print
x |> fn(...)        -- pipeline
```
