# Flux Language Spec (`.fx`)

Flux is a terse, batteries-included scripting language. One way to do each thing.
Read this once and you can write it.

## 1. Lexical basics
- Comments: `# to end of line`.
- Statements end at newline. No semicolons.
- Blocks use `:` to open and indentation (2 spaces) to nest — like a tree. A block ends when indentation decreases.
- Strings: `"hi"`, interpolate with `$name` or `${expr}`. Raw multiline strings also use `"..."`.
- Numbers: `42`, `3.14`. Bool: `T` / `F`. Nothing: `nil`.

## 2. Bindings
```
x = 10          # mutable binding (the only kind)
name = "ada"
list = [1, 2, 3]
map  = {a: 1, b: 2}     # keys are bare identifiers or "strings"
```
There is exactly one assignment form (`=`). Compound: `+=  -=  *=  /=`.

## 3. Types
`int float str bool list map fn nil`. Dynamic typing. Truthiness: `nil`, `0`, `""`, `[]`, `{}` are false; everything else true.

Common ops: `+ - * / %`, compare `== != < <= > >=`, logic `and or not`,
membership `in`, range `a..b` (inclusive). Index `list[0]`, `map.key` or `map["key"]`.

## 4. Control flow
One conditional, one loop. `?` is if, `|` is else-branch.
```
? x > 0:
  say "pos"
| x == 0:
  say "zero"
|:
  say "neg"
```
One loop: `@@` iterates anything (range, list, map). Optional index/key.
```
@@ i in 1..5:        # 1,2,3,4,5
  say i
@@ k, v in map:      # map: key, value
  say "$k=$v"
@@ item in list:
  ? item == 7: stop      # stop = break
  ? item < 0: skip       # skip = continue
```
A bare condition loop: `@@ cond:` repeats while cond is true.

## 5. Functions
`fn` defines, last expression (or `ret`) returns. Arrow `\` for lambdas.
```
fn add a b:
  ret a + b

double = \x: x * 2
say add(2, 3)        # 5
say double(4)        # 8
```
Default args: `fn greet name="world":`. Variadic: `fn sum *nums:`.

## 6. Errors
`!` raises, `try/catch` via `?!`. An error value has `.msg`.
```
? bad: ! "boom"          # raise
?! risky():              # try-block
  say "ok"
|! e:                    # catch, e is the error
  say "failed: ${e.msg}"
```

## 7. Modules
`use` imports a file or stdlib module; names live under the module alias.
```
use math            # stdlib
use "./util.fx" as u
say math.sqrt(9)
say u.helper()
```
Everything top-level in a file is exported automatically.

## 8. Batteries (stdlib, all under `@`)
The global `@` object holds the runtime/stdlib. No imports needed for these.

### Print / IO
```
say x                 # the one print (adds newline)
line = @in()          # read one stdin line
@args                 # list of CLI args (after program name)
@env("PORT", "8080")  # env var with default
```

### JSON
```
@json.enc(value)      # -> str
@json.dec(str)        # -> value
```

### Files
```
@fs.read(path)        # -> str (raises if missing)
@fs.write(path, str)
@fs.exists(path)      # -> bool
```

### HTTP server  (handler gets a req, returns a map {status, json|text})
```
srv = @web()
srv.get("/notes", \req: {json: notes})
srv.post("/notes/:id", \req: ...)   # req.params.id, req.body (parsed json), req.query
srv.del("/notes/:id", \req: ...)    # also: srv.put
srv.run(8080)
```
Response map keys: `status` (default 200), `json` OR `text`, `headers`.

### Database  (one-line connect; sqlite/postgres by URL)
```
db = @db("sqlite://app.db")
db.run("create table t(id int)")          # execute, no rows
rows = db.q("select * from t where id=?", id)   # query -> list of maps
db.q("insert into t(id) values(?)", 1)
```

### WebSocket server
```
ws = @ws()
ws.on("open",    \c:    ...)     # c = connection; c.id, c.send(str), c.data (per-conn map)
ws.on("message", \c, m: ...)     # m = received string
ws.on("close",   \c:    ...)
ws.run(9000)
# broadcast helper: ws.all() -> list of live connections
```

### Misc
```
@now()                # unix seconds (int)
@uid()                # short unique id string
math.sqrt math.floor  # via `use math`
str.split str.join str.upper str.trim   # via `use str`, e.g. str.split("a,b", ",")
str.int("42") str.str(42)               # parse / stringify
list.len(x)  list.push(x, v)  list.del(x, i)   # via `use list`
map.keys(m)  map.has(m, k)  map.del(m, k)      # via `use map`
```

## 9. Full tiny program
```
use list
todos = []
list.push(todos, "buy milk")
@@ t in todos:
  say "- $t"
```
That's the whole language.
