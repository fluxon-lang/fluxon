# Fluxon Roadmap — the path to a real, working programming language

> Status: June 2026. 479 green tests in the runtime, all batteries in the spec
> implemented, and the Phase 0 stability bugs all closed. The current focus is
> **Phase 1** (hardening the core), with **Phase 5** (distribution, beta) work
> starting in parallel.

The logic is simple: Phases 0–1 make the language *reliable*, Phase 2 *keeps*
that reliability *automatically*, Phase 3 makes it *useful*, Phase 4 *fast*,
Phase 5 *publicly available*. The phases can run partly in parallel, but you
don't enter 3–5 before 0–1 are done — you can't build an ecosystem on top of
panics in the foundation.

---

## Phase 0 — Stability: closing open bugs *(done)*

The crash/DoS, security, and silent-incorrectness bugs from the full code review
have all been fixed and shipped with regression tests. Open issues with the
`bug` label are now **0**.

What was closed, by wave:

- **Wave 1 — crash/DoS:** `json.dec` panic on malformed JSON (#87), no request
  body size limit (#91), unbounded recursion stack overflow (#90), integer
  overflow panic (#89), `extract_from_table` Unicode panic (#88), no
  client/`ai` timeout (#92).
- **Wave 2 — security:** non-cryptographic `rand` for tokens (#97),
  `Authorization` leak on cross-origin redirect (#96), dirty connection returned
  to the pool without ROLLBACK on tx error (#103).
- **Wave 3 — silent incorrectness:** `uniq(a, b)` dropping the multi-column
  constraint (#94), `ai` keeping only the last `tool_use` block (#95),
  parser/lexer silent errors `!x` / `m.0.1` / `1..n+1` (#93/#98/#99), `db.up`
  empty-where malformed SQL (#104), lost repeated headers (#101), queue
  handler-less busy-loop and shutdown job loss (#105), query-string
  percent-decoding (#100).

Since the review, several language features also landed: `try`/`catch` (#125),
`assert` + `fluxon test` (#136), an interactive REPL (#138), the `par` parallel
fan-out primitive (#137), and leveled `log` output (#139).

---

## Phase 1 — Hardening the language core (not bug-free, but *predictable*)

What separates a real language from a toy is a definite answer to any input:

- **A guarantee never to panic.** Every panic path in the runtime turns into a
  Fluxon-level error (`err`). To verify, the lexer / parser / `json.dec` are
  fuzzed with `cargo-fuzz` — finding bugs of the #87/#88/#90 class without
  waiting for an issue.
- **Diagnostic quality.** Every error shows line:column + a code snippet +
  "did you maybe mean this". This matters especially for AI agents — the more
  precise the error message, the faster the agent fixes itself (which matches
  the core philosophy of the language).
- **Stack trace.** A runtime error shows the Fluxon-level call chain.
- **Spec ↔ runtime audit.** Is there a test for every sentence in
  `docs/fluxon-agent.md`? When a discrepancy is found, either the spec or the
  runtime is fixed
  ([#81](https://github.com/Firdavs9512/fluxon-lang/issues/81) — the spec
  says "Postgres", the runtime is SQLite — work of this class).
- Close the language gaps found in earlier real-project tests:
  `str` library gaps, dynamic indexing, time arithmetic.

---

## Phase 2 — Reliability infrastructure

- **Continuous fuzzing in CI** (nightly job): lexer, parser, json, http request
  parsing.
- **Expand the `.fx` e2e suite** (`runtime/tests-fx/`) — "bad day" scenarios for
  each battery: network drop, DB lock, large payload.
- **Benchmark suite + regression alert** — a basis for the later move to a VM.
- **Dogfooding harness.** Give an AI agent (with a cheap model) real backend
  tasks and have it write them in Fluxon — every release. This method has found
  the most real bugs so far (the validation-tests methodology in `research/`).

---

## Phase 3 — Production-ready backend language

- **Postgres** real support (currently an `Err` stub) — required for the
  "backend language" claim. Fluxon `db.*` code is backend-neutral, the user code
  doesn't change.
- **Deploy story:** single binary, graceful shutdown, `$PORT`/secrets
  convention. Structured, leveled logging already landed (`log` with
  `debug`/`info`/`warn`/`err` + `$LOG_LEVEL`/`$LOG_FORMAT`, stdout vs stderr
  split) — #139.
- **`fluxon check`** — fast feedback for the AI agent loop. The CLI ships
  `fluxon check <file.fx>` today, but it is **lex + parse only**: it catches
  syntax errors without running, yet syntactically valid code referencing an
  unknown name still exits 0. A real **static/semantic check** (unbound names,
  arity, type-shape) is still to do.
- **`fluxon fmt`** — canonical form is the language's philosophy, so a formatter
  is mandatory. Still to do.
- **Module ecosystem:** `use ./file` exists; instead of versioned packages, for
  now a firm "batteries-included is enough" stance — this is the language's
  distinguishing strength.

---

## Phase 4 — Performance

- Move from the tree-walking interpreter to a **bytecode VM** — but only after
  the Phase 2 benchmarks show "where it's slow". The Arc contention experience
  showed that you can't guess without profiling.
- On the HTTP path, full async or a thread pool instead of a thread per request.

---

## Phase 5 — Distribution and v0.1 *(beta starting)*

- **Install / packaging**
  ([#164](https://github.com/Firdavs9512/fluxon-lang/issues/164)): `curl | sh` +
  binaries on GitHub Releases, then `crates.io` (`cargo install fluxon`), a
  Homebrew tap, Snap, and a PPA.
- **Documentation site + interactive playground** (compiled to WASM it runs in
  the browser too).
- **English translation** ([#58](https://github.com/Firdavs9512/fluxon-lang/issues/58))
  — mostly done: the docs, the CI/templates, and most runtime comments are in
  English. A few leftovers remain (e.g. the `runtime/examples/public/index.html`
  static demo is still Uzbek, and a handful of test names/comments in
  `http_mod.rs`); finish these before calling the task complete.
- **Versioning the spec:** `fluxon-agent.md` is frozen as v0.1, breaking changes
  only with a version bump. A "real language" means a promise that code written
  today still works tomorrow.
- **Editor tooling:** syntax highlighting (VS Code extension), then an LSP.
