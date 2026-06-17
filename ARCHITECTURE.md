# Fluxon runtime — architecture

This document explains how the interpreter inside `runtime/` is built. A
contributor (human or AI) should read this before adding a new feature.

> How the language **itself** works (syntax/semantics): `docs/fluxon-agent.md`
> (compact) or `docs/fluxon-human.uz.md` (detailed). This document is about the
> **implementation**.

---

## 1. Overview

Fluxon is a **tree-walking interpreter** — it walks the AST directly (no
bytecode/VM). Written in Rust edition 2024. Pipeline:

```
source (.fx)
  → token.rs        token types + Token.spaced flag
  → lexer.rs        characters into tokens; INDENT/DEDENT, string interpolation
  → ast.rs          AST nodes: Stmt, Expr
  → parser.rs       precedence climbing + paren-free calls (juxtaposition)
  → value.rs        runtime values: Value, NativeFn
  → interp.rs       walking the AST: scope, control flow (Flow enum), dispatch
  → builtins.rs     core modules (str/math/rand/json/time) + list/map methods
```

The batteries (`http`, `db`, `ai`, `auth`, `ws`, `cron`, `queue`, `reg`) live in
separate modules (`*_mod.rs`) and hook in from the dispatch point in `interp.rs`.

User modules (`use ./lib/x`) load in `interp/module.rs::run_module_file`, which —
after collecting the `exp`-ed names — validates an optional sibling `.pkg`
manifest (the "battery-shaped module" AI-doc, parsed by `interp/pkg.rs`; see
`docs/pkg-format.md`).

CLI entry: `runtime/src/main.rs` → `fluxon run file.fx`.

---

## 2. Frontend (lexer / parser)

Because Fluxon's grammar is compact, there are two subtle spots — keep them in
mind when adding new syntax:

### 2.1 INDENT/DEDENT (lexer.rs)

Blocks are not `{}` but **indentation** (2 spaces). The lexer emits Python-like
INDENT/DEDENT tokens. **Important fix:** after a multi-line block-lambda
(`\req ->\n  ...`), a `Newline` is pushed after the DEDENTs — otherwise the next
line is swallowed as an argument to the preceding paren-free call
(inside `emit_indentation`).

### 2.2 `:` ambiguity

`key:val` (Colon separator) vs `:sym` (symbol). **Rule:** if `:` is attached to
the preceding atom (ident/number/`)`/`]`/`"`) → Colon, otherwise → Sym.
`status::open` → Colon + Sym.

### 2.3 Paren-free call (juxtaposition) — the `no_app` flag

`f a b` is a paren-free call. But inside a list/map literal this is disabled (the
`no_app` flag): `[a b]` is two elements, not a call of `f`. If a call is needed,
use parens: `{a:(f x)}`.

### 2.4 `Token.spaced` flag

`arr[i]` (a `[` touching the atom → indexing) vs `f "x" [a]` (a spaced `[` → a
separate list argument). In `parse_postfix`: `Tok::LBracket if !self.spaced()` →
index. This powers the `db.one "sql" [params]` spec syntax.

> When adding new syntax: first `token.rs`/`lexer.rs`, then `ast.rs`, then
> `parser.rs`. Write the test as an integration test in `main.rs::mod tests`.

---

## 3. Interpreter (interp.rs)

### 3.1 Scope and the `Parent` enum (perf-critical)

A scope is `Env = Arc<RwLock<Scope>>` (parking_lot RwLock — parallel reads).
`Scope.vars` is a `Vec<(Box<str>, Value, bool)>` (bool = mutable), not a HashMap:
fn/block scopes hold 0–4 names, and a linear scan beats two HashMaps.

**The `Parent` enum — an Arc contention optimization (don't break it):**

```rust
enum Parent { None, Root, Scope(Env) }
```

Top-level fns do **not** hold the root Arc, only a `Parent::Root` **marker**. On
`Parent::Root`, `lookup` reads from the frozen (`freeze_globals`) lock-free
snapshot if frozen, otherwise `self.global.clone()`. The result: 8 threads no
longer collide on a single root cache line → negative scaling turned positive.

> The history: previously every fn call atomically cloned the root Arc refcount →
> `Arc::drop_slow` + `lock_shared_slow` contention on 8 threads. Don't revert
> `Parent` to `Option<Env>` without understanding this — it's a regression.

### 3.2 Control flow — the `Flow` enum

Early exits (`ret`, `skip`, `stop`, `fail`, `!`) are propagated upward via Rust's
`Result`/`Flow` enum (`EvalResult`). `fail` → `Flow::Fail` → turns into a JSON
error in the HTTP response.

### 3.3 Dispatch — where batteries hook in

`eval_call` (`interp.rs`) routes by looking at the module name:

```rust
// inside interp.rs::eval_call (roughly):
if modname == "http" { return self.arc_self().http_dispatch(name, argv); }
if modname == "db"   { return self.arc_self().db_dispatch(name, argv); }
if is_module(modname) { return call_module(modname, name, argv); }  // str/math/...
```

`arc_self()` rebuilds an `Arc<Interp>` from `&self` (via
`this: OnceLock<Weak<Interp>>`). This is needed to pass `Interp` to threads in
spawn_blocking.

**A no-argument module function** (`time.now`) arrives in the parser not as a
`Call` but as a `Field`. In the `Expr::Field` handler, if
`is_module(id) && lookup(id).is_err()`, it calls `call_module(id, name, vec![])`
with no arguments.

### 3.4 `tbl` schema registry

`Stmt::Tbl` → `register_tbl` (writes the schema into `Interp.schema`). In `run()`,
`FnDecl` and `Tbl` are **hoisted** (registered up front). The schema is for the
`db` battery: `sym`/`json` column conversion, auto-migration.

---

## 4. Batteries

### 4.1 `http` (http_mod.rs)

- Server: tokio + hyper 1.x. Every request runs inside **`spawn_blocking`** → so
  Fluxon's synchronous interpreter doesn't block the tokio workers, and it runs
  truly in parallel (`Value: Send+Sync` guarantees this).
- `http.on :method "/path/:id" \req -> ...` — Route/Seg, `match_route`.
- `rep status body` — a `{__resp:true status body}` map (builtins).
- Client: `http.get/post/put/del` — a pooled hyper Client.
- `http.serve port` / `ws.serve port` / `cron.run` **do not block immediately** —
  they add a deferred descriptor to the `Interp.pending_servers` list. Once
  top-level code finishes, `serve_mod::run_pending` takes over: if there's a
  network server (http/ws), it freezes the globals once with `freeze_globals`
  (a lock-free snapshot), and ONE shared tokio runtime `spawn`s each server and
  blocks (the cron scheduler on its own background thread). If only `cron.run`
  (no server): no runtime/freeze is NEEDED — the main thread is just put to
  sleep. So HTTP + WS + cron.run run together in **any order**; an HTTP handler
  can call `ws.room.send` and reach WS connections (shared `Interp`). Because
  this is the single place, every blocking "run" (fixed in #18 and #42) doesn't
  kill the others.

### 4.2 `db` (db_mod.rs)

- **Hidden behind the `Db` trait.** Fluxon code (`db.*`) never changes; the
  backend is chosen from the `$DATABASE_URL` scheme (`sqlite:`/`postgres:`
  /`mysql:`). Default is **SQLite** (`rusqlite` bundled — no server needed).
- `postgres`/`mysql` are an `Err` stub for now — later they plug in
  **additively** in `open_from_env`. An agent doesn't get tangled up in a
  separate package for another db.
- A connection **pool** (`Mutex<Vec<Connection>>`). A tx takes a separate
  connection → other queries aren't blocked during a tx.
- `db.tx \-> ...` — `BEGIN IMMEDIATE` (race-safe). Nested tx → SAVEPOINT.
  `fail`/`!` → rollback.
- `tbl` → `CREATE TABLE IF NOT EXISTS` **auto-migration** (zero-setup).

---

## 5. Adding a new battery (recipe)

The most common contributor task. Follow the `http_mod.rs`/`db_mod.rs` pattern:

1. **Read the spec.** Battery syntax is specified in `docs/fluxon-agent.md` —
   this is the **source of truth**. Don't invent syntax yourself.
2. **Create a new module file**: `runtime/src/<name>_mod.rs`. Inside:
   `impl Interp { fn <name>_dispatch(&self, func: &str, args: Vec<Value>) -> ... }`
   plus a helper per function.
3. **Add `mod <name>_mod;`** to `main.rs`.
4. **Hook up dispatch** (`interp.rs::eval_call`): alongside the `http`/`db` line,
   `if modname == "<name>" { return self.arc_self().<name>_dispatch(name, argv); }`.
   If it has a no-argument function (like `time.now`), also in the `Expr::Field`
   handler. If it's a pure core module (no IO, like `str`/`math`) — adding it to
   `builtins.rs` `is_module`/`call_module` is enough.
5. **If you need a dependency**, add it to `Cargo.toml`. Write a comment
   explaining **why** it's needed (existing deps are commented this way).
6. **Test:** a native test inside the module + an integration test in
   `main.rs::mod tests`. If it's IO/server, verify it by actually running it.
7. **Don't break `Value: Send + Sync`.** If you introduce a new value type, make
   it Send+Sync.

> Note: the "no dependencies" rule applies only to the **Fluxon language user**
> (they don't `npm install`). INSIDE the runtime, Rust crates are OK.

---

## 6. Test strategy

Two layers:

- **Rust unit/integration tests** (`cargo test`) — `#[cfg(test)]` inside a module
  + `main.rs::mod tests` (running `.fx` code and checking the result, the
  `run(src)` helper). DB tests are serialized with `DB_TEST_LOCK`.
- **`.fx` e2e tests** (`runtime/tests-fx/`) — written in Fluxon **itself**,
  asserting from the user's point of view. Run via `run_all.sh`. When you add a
  new battery, add an `NN_*.fx` file in this style.

---

## 7. Battery status

**All batteries** specified in `docs/fluxon-agent.md` are implemented:

| Battery | Module | Note |
|---------|-------|------|
| `http` | `http_mod.rs` | server + client + middleware |
| `db` | `db_mod.rs` | SQLite, pool, tx, auto-migration (postgres/mysql stub) |
| `ai` | `ai_mod.rs` | Anthropic Messages API |
| `auth` | `auth_mod.rs` | JWT HS256 + password hash (argon2id) |
| `crypto` | `crypto_mod.rs` | sha256/hmac/b64/hex/uuid (pure module — via call_module) |
| `ws` | `ws_mod.rs` | websocket server, room/data |
| `cron` | `cron_mod.rs` | scheduled tasks |
| `queue` | `queue_mod.rs` | background job queue |
| `reg` | `reg_mod.rs` | tool registry |

Next steps — deepening the existing batteries (e.g. a postgres/mysql backend for
`db`) and new language features. The new-battery pattern is in §5.
