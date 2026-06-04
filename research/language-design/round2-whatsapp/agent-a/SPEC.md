# Flux — Language Spec

Flux is a terse server language for AI-native backends. One way to do one thing.
Indentation = blocks (2 spaces). No semicolons, no braces. Newline ends a statement.

## Comments
`# line comment`

## Values & types
```
42        int
3.14      flt
"hi"      str        # "a $x b" interpolates expressions: "$x" or "${x.y}"
true      bool
nil       nil
[1 2 3]   list       # space-separated
{a:1 b:2} map        # space-separated pairs
:ok       sym        # interned symbol/enum tag
```
Truthy: everything except `nil` and `false`.

## Bindings
`x = expr`           immutable by default
`x <- expr`          mutable; reassign with `x <- newval`

## Operators
`+ - * / %`  arith   `== != < <= > >=`  compare   `& |`  and/or   `!`  not
`?`  null-coalesce: `a ? b` → a unless a is nil, then b
`.`  member/index: `m.key`, `list.0`, `list.len` (length). Dynamic index: `m[k]`.
`..` range: `1..5` → [1 2 3 4 5]
`|>` pipe: `x |> f |> g` == `g(f(x))`

## Functions
`fn name a b -> expr`            single-expr body
```
fn name a b
  ...stmts
  ret x          # explicit return; bare last expr also returns
```
Call: `name a b` (space args). Parens only to group: `f (g x)`.
Lambdas: `\a b -> expr`. Used inline: `map xs \x -> x*2`.

## Control flow
```
if cond
  ...
ef cond       # else-if
  ...
el
  ...

ea x in xs    # each — THE loop. iterate list/range. `skip`=continue `stop`=break
  ...
```
No `while`: loop a range `ea i in 1..n`, or recurse. One loop, one way.
`match`:
```
mt val
  :a -> ...
  :b -> ...
  _  -> ...      # default
```

## Errors
Functions return `ok|err`. `!` propagates, `?:` handles.
```
r = create x!        # ! unwraps ok, propagates err to caller
r = create x ?: e    # on err, bind err to e and run block
  log e
  ret nil
fail "msg"           # raise an err
```

## Modules
`use http db ai json env`   # stdlib, no install. Names then namespaced: `db.get`.
`use ./tools`               # local file → `tools.create_order`
`exp fn ...` / `exp x = ..` exports a name from a file.

## Stdlib (batteries — all built in, zero config)

### http (server)
```
http.on :post "/path" \req -> ...     # req.body(map), req.q(query), req.h(headers)
  rep 200 {ok:true}                   # rep status body(json auto)
http.serve 8080
```
### http client
`res = http.get url` / `http.post url body` → `{status code:int body:any}`

### db (Postgres; $DATABASE_URL auto)
```
db.q "select * from t where id=$1" [id]    # → list of maps
db.one "..." [..]                          # → one map or nil
db.ins "table" {col:val ..}                # insert, → inserted row
db.up "table" {col:val} {id:x}             # update set / where
```
Schema defined in `.flux` via `tbl`:
```
tbl users
  id   serial pk
  name str
  ph   str uniq
  ts   now
```
Types: serial int str flt bool json now(timestamp default now). `pk uniq null ref:tbl.col`.

### ai (LLM; $AI_KEY auto)
```
ai.ask prompt                      # → str
ai.json prompt schema              # → map matching schema (typed extraction)
ai.run prompt tools                # agentic: tools = list of fn refs; auto tool-calling loop
```
Every ai.* returns meta: `r._.tokens r._.cost r._.ms r._.conf` (confidence 0..1).

### json / env / log
`json.enc x` `json.dec s` · `env.X` (env var) · `log x` (stderr)

### cron
`cron.wk :sun 18 0 \-> ...`   # weekly: day, hour, min. also cron.dy(daily h m), cron.hr(min).

### queue
`q.push "name" payload` · `q.on "name" \job -> ...`   # background worker

### time
`now()` → ts · `now() + 7.days` · `ts.fmt "..."` · `:mon..:sun` weekday syms
