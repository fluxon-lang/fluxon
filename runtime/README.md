# Fluxon Runtime

The interpreter for the Fluxon language (Rust, tree-walking). The **language
core** and **all batteries** specified in `docs/fluxon-agent.md` work:
`http` (server + client), `db`, `ai`, `auth`, `ws`, `cron`, `queue`, `reg`.
(`db` currently has only the SQLite backend; `postgres`/`mysql` are stubs.)

## Build and run

```sh
cargo build --release
cargo run -- run examples/demo.fx
# or
./target/release/fluxon run examples/demo.fx
```

### Commands

- `fluxon run <file.fx>` — executes the file (lex → parse → interp). A parse or
  runtime error → `exit 1`.
- `fluxon check <file.fx>` — only checks the syntax (lex + parse, **does not
  run** the code → no side effects). OK → `exit 0`; parse/lex error →
  `exit 2`. This differs from `run`'s `exit 1`, so the caller can tell which
  stage failed (handy for an AI self-repair gate).
- `fluxon --version` or `fluxon -V` — prints the built package version
  (`package.version` from `Cargo.toml`).
- `fluxon --help` or `fluxon -h` — prints usage instructions.

## What works now

The full language core:

- **Types:** int, flt, str, bool, nil, sym, list, map
- **Bindings:** `=` (immutable), `<-` (mutable)
- **Functions:** `fn`, one-liner `->`, lambda `\x ->`, closure, `ret`, last-expression return, recursion
- **Control:** `if`/`elif`/`else`, `each` (list/map/range/str), `skip`/`stop`, `match` (symbol/number/`_`)
- **Operators:** arithmetic (`+ - * / %`), comparison, logical (`& | !`), `??`, `|>`, `..`, member/index (`.` `[]`)
- **String interpolation:** `"$x"`, `"${expr}"`
- **List methods:** `len push has filter map reduce slice join`
- **Map methods:** `len has keys vals set del` + spread `{...m}` + dynamic key `{[k]:v}`
- **Core modules:** `str` (len up low slice split has int str), `math` (floor ceil abs round), `rand` (int str), `json` (enc dec), `time`, `env`, `io`, `fs`, `sh`
- **Batteries:** `http`, `db`, `ai`, `auth`, `ws`, `cron`, `queue`, `reg` — enabled with `use <name>`
- **`log`** — print to stderr
- **Errors:** `fail [status] "..."`, `!` (propagate operator)

The `tbl` schema is read by the `db` battery — used for `CREATE TABLE IF NOT EXISTS`
auto-migration and column type conversion.

### `http` battery (server + client)

```fluxon
use http
http.on :get "/health" \req -> rep 200 {ok:true}
http.on :get "/notes/:id" \req -> rep 200 {id:req.params.id}
http.on :post "/notes" \req -> rep 201 {received:req.body}
http.serve 8080
```

- `http.on :method "/path" handler` — a route. `:get :post :put :del`. In the
  path, `:id` is a parameter (`req.params.id`).
- `req` map: `method path query{} headers{} params{} body`. If `Content-Type:
  application/json`, `body` is automatically decoded into a map.
- `rep status body` — a response. If body is a map/list, auto JSON; if str, text.
- `fail status "msg"` — an error response inside a handler (`{"error":"msg"}` + status).
- `http.serve port` — starts the server, **blocking**. Optional option:
  `http.serve port {max_body: BYTES}` — request body size limit (default
  10 MiB, over it → `413`; `max_body: 0` — unlimited).
- Client: `http.get url`, `http.post url body`, `http.put url body`,
  `http.del url` (body map -> JSON). The result is `{status, body}`; if the
  response is JSON, `body` is decoded.
- `http.get/post/put/del` calls reuse one global Hyper client. So a new client
  isn't built each time on sequential or parallel calls, and Hyper's connection
  pool reuses connections to the same hosts.

**Parallelism:** the server is built on tokio + hyper, each request runs
separately in `spawn_blocking` (truly parallel). The runtime is thread-safe
(`Arc`/`RwLock`), and the global scope is frozen into a lock-free snapshot during
`http.serve`. Example: `examples/server.fx` (test with `curl localhost:8080/health`).
For client API simplicity and pool reuse, `examples/http_client_pool.fx` does
sequential `http.get`s against a local server; the `for ... & ... wait` command
at the top of the file also tests this Fluxon client by running it in parallel.

## Architecture

```
src/
  token.rs    — token types (+ INDENT/DEDENT, string fragments)
  lexer.rs    — source -> tokens; indentation -> INDENT/DEDENT
  ast.rs      — AST nodes
  parser.rs   — tokens -> AST (precedence climbing + paren-free calls)
  value.rs    — runtime values
  interp.rs   — walks the AST and executes (scope, control flow, calls)
  builtins.rs — core modules (str/math/rand/json/time/io/fs/sh) + methods + `rep`
  http_mod.rs — `http` battery: server (on/serve), routing, req/rep, middleware, client
  db_mod.rs   — `db` battery: SQLite, pool, tx, schema auto-migration
  ai_mod.rs   — `ai` battery: LLM (Anthropic Messages API)
  auth_mod.rs — `auth` battery: JWT HS256 + password hash (argon2id)
  ws_mod.rs   — `ws` battery: websocket server, room/data
  cron_mod.rs — `cron` battery: scheduled tasks
  queue_mod.rs— `queue` battery: background job queue
  reg_mod.rs  — `reg` battery: tool registry
  serve_mod.rs— managing deferred servers (http/ws/cron together)
  main.rs     — CLI + integration tests
```

The frontend (lexer/parser/AST) can be reused for a bytecode VM in the future.

## Tests

```sh
cargo test
```

There are ~197 tests right now: Rust unit tests inside modules (`builtins.rs`,
`interp.rs`, `db_mod.rs`, etc.) + integration tests in `src/main.rs::mod tests`
(running `.fx` code and checking the result). In addition, `tests-fx/` has e2e
tests written in Fluxon itself (`run_all.sh`).

## Next step

All batteries in the spec (`http`, `db`, `ai`, `auth`, `ws`, `cron`,
`queue`, `reg`) are implemented. Next steps — deepening the existing batteries
(e.g. a postgres/mysql backend for `db`) and new language features. For the
pattern → [`ARCHITECTURE.md`](../ARCHITECTURE.md).
