# LUNE Language Specification

**LUNE** (Lean, Unified, Narrative Execution) is a minimal expression language optimized for readability and terse syntax with batteries included.

## Core Syntax

### Literals & Types
- **Numbers**: `42`, `3.14`, `0xFF` (hex), `0b101` (binary)
- **Strings**: `"hello"` or `'hello'` (both identical). Interpolation: `"x={x}"`
- **Booleans**: `true`, `false`
- **Null**: `null`
- **Lists**: `[1, 2, 3]`
- **Dicts**: `{a: 1, b: 2}` (shorthand: `{a, b}` → `{a: a, b: b}`)

### Variables & Assignment
```
x = 5          # assignment
x += 2         # compound ops: +=, -=, *=, /=, //=, %=
```

### Expressions & Operators
All operators return values. Operator precedence is standard.
```
1 + 2 * 3      # arithmetic: +, -, *, /, //, %, **
"a" + "b"      # string concat
[1] + [2]      # list concat
a == b         # comparison: ==, !=, <, >, <=, >=
a && b         # logic: &&, ||, !
a |> f         # pipe: pass a to function f
```

### Functions
```
fn add(x, y) { x + y }      # def function
fn greet(name = "World") { "Hi {name}" }  # default args
(x) => x * 2   # lambda (arrow)
```

Function calls: `add(3, 4)`. Last expression is return value.

### Control Flow
Only ONE form per construct (no if/unless redundancy):

```
if x > 0 { "pos" } else { "neg" }    # single if-else
while x < 10 { x += 1 }               # single loop
for item in list { ... }              # iterate
for i : 0..10 { ... }                 # range (0 to 9)
break                                  # exit loop
continue                               # next iteration
```

### Pattern Matching (destructuring)
```
[a, b] = [1, 2]
{x, y} = {x: 1, y: 2}
```

### Error Handling
```
try { risky() } catch e { handle(e) }
throw "message"
```

### Modules & Import
```
use http              # import built-in module
use ./utils { fn1 }   # import from file
pub fn export_me() {} # exported function
```

## Standard Library (Batteries Included)

### Collections
- **list.len(l)** → length
- **list.push(l, x)** → append, return list
- **list.map(l, f)** → apply f to each
- **list.filter(l, f)** → keep where f is true
- **list.find(l, f)** → first match or null
- **list.join(l, sep)** → string
- **dict.keys(d)**, **dict.values(d)**, **dict.get(d, key, default)**
- **dict.merge(d1, d2)** → combine dicts

### String
- **str.len(s)**, **str.upper(s)**, **str.lower(s)**
- **str.trim(s)**, **str.split(s, delim)**, **str.join(l, sep)**
- **str.replace(s, old, new)**, **str.contains(s, sub)**, **str.starts_with(s, prefix)**
- **str.index(s, sub)** → position or -1

### I/O
- **print(val)** → console output (with newline)
- **file.read(path)** → string
- **file.write(path, content)** → null
- **file.append(path, content)** → null
- **file.exists(path)** → bool

### JSON
- **json.parse(s)** → value
- **json.stringify(v, pretty=false)** → string

### HTTP (web server & client)
- **http.listen(port, handler)** → start server, handler(req) returns {status, body}
- **http.get(url)** → {status, body, headers}
- **http.post(url, body, headers={})** → {status, body, headers}

### Database (SQLite by default)
- **db.open(path)** → connection
- **conn.exec(sql, params=[])** → null (insert/update/delete)
- **conn.query(sql, params=[])** → list of dicts (select)
- **conn.close()** → null

### WebSocket
- **ws.listen(port, handlers)** → start server
  - handlers: `{onConnect, onMessage, onClose}`
  - each handler(client, data/null) returns null
- **ws.client(url)** → {send(msg), close()}

### Environment & System
- **env.get(key, default="")** → value
- **env.set(key, val)** → null
- **sys.now()** → unix timestamp (float)
- **sys.sleep(ms)** → null
- **sys.exec(cmd)** → {code, stdout, stderr}

### Async/Concurrency
- **async fn work() { ... }** → coroutine
- **await fn()** → wait for coroutine
- **spawn(fn)** → background task (fire-forget)

## Types (implicit, checked at runtime)
- `num`, `str`, `bool`, `list`, `dict`, `null`, `fn`
- Type coercion: loose where sensible (string concat with any type)
- **is_type(val)** → `"num" | "str" | "bool" | "list" | "dict" | "null" | "fn"`

## Example Programs

### FizzBuzz
```
for i : 1..101 {
  msg = if i % 15 == 0 { "FizzBuzz" }
        else if i % 3 == 0 { "Fizz" }
        else if i % 5 == 0 { "Buzz" }
        else { i }
  print(msg)
}
```

### HTTP Server (5 lines)
```
handler = (req) => {status: 200, body: "Hello"}
http.listen(8000, handler)
print("Server on :8000")
```

### Query Database
```
db = db.open("test.db")
rows = db.query("SELECT * FROM users WHERE age > ?", [18])
print(rows)
db.close()
```

## Comments
```
# single-line comment
```

## Formatting & Style
- Indentation: 2 spaces (not enforced, but convention)
- Statements separated by newline or `;`
- Use `|>` (pipe) operator to chain calls: `data |> process |> format`
