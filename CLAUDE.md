# CLAUDE.md — How to work on Fluxon (for AI agents)

This file is for Claude Code and other AI agents. Before making any change to the
project, **read this file to the end**. The goal: get a new agent productive fast,
working in the right place and in the right style.

> For human contributors: [`CONTRIBUTING.md`](CONTRIBUTING.md).
> Runtime internals: [`ARCHITECTURE.md`](ARCHITECTURE.md).

---

## 0. What this project is

**Fluxon** — a backend programming language that AI agents write well. Philosophy:
*"The language adapts to the AI, not the AI to the language."* One task = one way
(canonical form), few tokens, batteries-included (`http`/`db`/`ai`/`ws`/...).

This repo has two parts:

- **`runtime/`** — the language interpreter (Rust, tree-walking). **This is where
  the main work happens.** All code, tests, and builds live here.
- **`docs/` + `examples/` + `research/`** — the language specification, examples,
  and how the language was designed (experiments).

---

## 1. Language

Write **code, comments, commit messages, PR titles/descriptions, and
documentation in English.** Technical terms and code identifiers (`HashMap`,
`eval_call`, `db.tx`) stay as-is.

When **talking to the user**, the agent is language-neutral: reply in whatever
language the user writes in. The codebase is English, but the conversation
adapts to the user.

> Note: `research/` and the `*.uz.md` documents are kept in Uzbek on purpose
> (design history and future multi-language docs) — do not translate them.

---

## 2. Where things are (navigation)

Before adding a new battery or making a change, find the relevant file:

| Task | File |
|------|------|
| Token types, `Token.spaced` flag | `runtime/src/token.rs` |
| Lexer: INDENT/DEDENT, string interpolation | `runtime/src/lexer.rs` |
| AST nodes (`Stmt`, `Expr`) | `runtime/src/ast.rs` |
| Parser: precedence climbing, parenless calls | `runtime/src/parser.rs` |
| Value types (`Value`, `NativeFn`) | `runtime/src/value.rs` |
| Interpreter: scope, control flow, dispatch | `runtime/src/interp.rs` |
| Core modules (`str/math/rand/json/time`) | `runtime/src/builtins.rs` |
| `http` battery (server + client) | `runtime/src/http_mod.rs` |
| `ai` battery (LLM — Anthropic Messages API) | `runtime/src/ai_mod.rs` |
| `db` battery (SQLite, tx, schema) | `runtime/src/db_mod.rs` |
| `auth` battery (JWT HS256 + password hash argon2id) | `runtime/src/auth_mod.rs` |
| `crypto` battery (sha256/hmac/b64/hex/uuid) | `runtime/src/crypto_mod.rs` |
| CLI entry point + integration tests | `runtime/src/main.rs` |

**If you need to read the spec:** `docs/fluxon-agent.md` (~2700 tokens, compact —
written for an AI to learn how the language works). More detail:
`docs/fluxon-human.md` (English) or `docs/fluxon-human.uz.md` (Uzbek).

**For the pattern to add/change a battery** → the "Adding a new battery" section
of [`ARCHITECTURE.md`](ARCHITECTURE.md). The pattern already exists in
`http_mod.rs` and `db_mod.rs` — read them as examples.

---

## 3. Build, test, run

**All `cargo` commands run inside `runtime/`** (not at the repo root):

```sh
cd runtime
cargo build                          # build
cargo test                           # all tests
cargo run -- run examples/demo.fx    # run a single .fx file
cargo fmt                            # format
cargo clippy --all-targets -- -D warnings   # lint (must be 0 warnings)
```

`.fx` examples live in `runtime/examples/`. The HTTP/WS server examples
(`server.fx`) open a port and **block** — for a smoke test use `demo.fx`.

---

## 4. What it takes for a green PR

CI (`.github/workflows/ci.yml`) runs on ubuntu + macOS. Before committing,
**check the following locally:**

1. `cargo build --locked` — compiles
2. `cargo test --locked` — all tests green
3. `cargo fmt --check` — formatted
4. `cargo clippy --all-targets -- -D warnings` — 0 warnings
5. `cargo run -- run examples/demo.fx` — smoke test works

> The `build-test` job is **required** (no merge if red). The `lint` job is
> non-blocking for now, but **new code is expected to arrive with 0 warnings** —
> do not break the existing agreement.

Write a **test for every new behavior**. Test conventions:

- **Native (Rust) tests** — inside the relevant module as `#[cfg(test)] mod ...`
  (`builtins.rs`, `interp.rs`, `db_mod.rs`).
- **Integration tests** (run `.fx` code and check the result) — inside the
  `mod tests` of `main.rs`. Use the `run(src)` helper.
- **DB tests** are serialized with the global `DB_TEST_LOCK` mutex (to avoid the
  `DATABASE_URL` env race) — see the example in `db_mod.rs`.

---

## 5. Code style (Rust)

- **Edition 2024.** Default `cargo fmt` settings.
- Comments should explain **why**, not **what** — existing files follow this
  style. Match the comment density of the surrounding code.
- New names and idioms should resemble the surrounding code.
- Do not use `unsafe`. The existing code is fully safe (`db_mod.rs` connection
  pool uses `Arc`, without `unsafe`).
- **Do not break the `Value: Send + Sync` invariant** — the runtime is
  thread-safe (each HTTP request runs on its own thread). Any new value type you
  introduce must be Send+Sync.

---

## 6. Git and commit rules

- **Do not commit directly to master** — always branch + PR.
- Branch name: `battery-<name>`, `perf-<name>`, `docs/<name>`, `fix-<name>`.
- Commit message in **English**, short and precise: what changed and why.
- One PR = one logical change. Do not mix (e.g. battery + refactor).
- Do not `commit`/`push` unless the user asks.

---

## 7. Important invariants (do not break)

These keep the runtime working — think carefully before changing them and guard
them with tests:

- **`=`/`exp`/param are immutable**, `<-` is mutable. A param can be reassigned
  with `<-` (the old behavior is preserved).
- **Closure capture, mutual recursion, shadowing, `each` loop var mutability** —
  these work correctly today; do not introduce regressions.
- **Scope/`Parent` enum** (`interp.rs`): top-level fns hold a `Parent::Root`
  marker (they do not keep the root Arc) — this is the optimization that
  eliminated Arc contention. Do not change it without understanding it →
  [`ARCHITECTURE.md`](ARCHITECTURE.md).
- **`http.serve`** freezes globals with `freeze_globals` (a lock-free snapshot).

---

## 8. Battery status (all ready)

All batteries specified in `docs/fluxon-agent.md` are **implemented** in the
runtime: `http`, `db`, `ai`, `auth`, `crypto`, `ws`, `cron`, `queue`, `reg`
(`ai` — `ai.ask`/`ai.json`/`ai.run`, `$AI_KEY`, via the Anthropic Messages API;
`runtime/src/ai_mod.rs`).

When adding a new battery, treat the spec (`docs/fluxon-agent.md`) as the
**source of truth** — the syntax is defined there. The implementation pattern is
the same as `http`/`db`.

## 9. Unexpected bugs and gaps

Fluxon may still have missing pieces, bugs, or rough edges. When you hit one,
report it back to the repo via a GitHub issue.
