# Contributing to Fluxon

Thank you! Fluxon is open source and we welcome contributors. This document
gives you everything you need to get started.

> If you work with an AI agent (Claude Code etc.) — rules and navigation are in
> [`CLAUDE.md`](CLAUDE.md). Runtime internals: [`ARCHITECTURE.md`](ARCHITECTURE.md).

---

## Language

Code comments, commit messages, PR descriptions, and documentation are written
in **English**. Technical terms and code identifiers (`HashMap`, `db.tx`) stay
as-is. Conversation in issues and PRs can be in whatever language is comfortable
for the participants.

---

## Requirements

- **Rust** (stable, edition 2024) — install via [rustup.rs](https://rustup.rs).
- `git`.
- Nothing else needed: SQLite is **bundled** (no system library required),
  HTTP/server deps come with `cargo`.

---

## Quick start

```sh
git clone <repo-url>
cd fluxon-lang/runtime          # IMPORTANT: every cargo command runs here

cargo build                   # build
cargo test                    # tests (197 right now)
cargo run -- run examples/demo.fx   # run a single .fx file
```

Repository structure:

```
fluxon-lang/
├── runtime/          interpreter (Rust) — THE MAIN WORK IS HERE
│   ├── src/          source code
│   └── examples/     .fx examples
├── docs/             language spec (fluxon-agent.md, fluxon-human.md)
├── examples/         real project examples (chat, ecommerce, support-tickets)
└── research/         how the language was designed
```

---

## Workflow

1. **Open a branch** from master. Name: `battery-<name>`, `fix-<name>`,
   `perf-<name>`, `docs/<name>`.
2. Make the change + **write a test** (for every new behavior).
3. Check locally (the "PR readiness" list below).
4. Commit (a clear message) → open a PR.
5. CI should be green. After review it gets merged.

One PR = one logical change. Don't mix a battery + a refactor.

---

## PR readiness (check before committing)

Inside `runtime/`:

```sh
cargo build --locked                          # 1. compiles
cargo test --locked                           # 2. tests green
cargo fmt --check                             # 3. formatted
cargo clippy --all-targets -- -D warnings     # 4. 0 warnings
cargo run -- run examples/demo.fx             # 5. smoke test
```

CI (`.github/workflows/ci.yml`) checks these on ubuntu + macOS:

- **`build-test` job — MANDATORY.** No merge if it's red.
- **`lint` job** (fmt + clippy) — currently non-blocking, but **new code is
  expected to arrive with 0 warnings**. Don't break the existing clean state.

---

## Writing tests

Two kinds of test (details → [`ARCHITECTURE.md`](ARCHITECTURE.md) §6):

- **Rust tests** — inside a module via `#[cfg(test)] mod ...`, or an integration
  test in `main.rs::mod tests` that runs `.fx` code and checks the result
  (the `run(src)` helper).
- **`.fx` e2e tests** — in `runtime/tests-fx/` (written in Fluxon itself, run
  via `run_all.sh`). Follow this style when you add a new battery.

DB tests are serialized with the global `DB_TEST_LOCK` mutex — see the example
in `db_mod.rs`.

---

## Code style

- `cargo fmt` default settings (edition 2024).
- Comments explain **why**, not **what**. Match the style of the surrounding
  code.
- Don't use `unsafe`.
- Don't break the `Value: Send + Sync` invariant (the runtime is thread-safe).
- Important perf/semantic invariants are in [`CLAUDE.md`](CLAUDE.md) §7 — don't
  break them.

---

## Where to start

- **Deepen an existing battery** — all batteries in the spec (`http`, `db`,
  `ai`, `auth`, `ws`, `cron`, `queue`, `reg`) are implemented; extend them or
  add a new language feature. Recipe: [`ARCHITECTURE.md`](ARCHITECTURE.md) §5.
  `http`/`db` are the templates.
- **Improve examples/docs**.
- **Bug fix** — first write a test that reproduces it.

Before starting a large change, open an issue — let's agree on the direction.

---

## Conduct

Be respectful and constructive. Ask questions, send small PRs, help each other.
This is a language we're building together.
