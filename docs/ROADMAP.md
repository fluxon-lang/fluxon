# Fluxon Roadmap — the path to a real, working programming language

> Status: June 2026. 267 green tests in the runtime, all batteries in the spec
> implemented. The current focus is **Phase 0**.

The logic is simple: Phases 0–1 make the language *reliable*, Phase 2 *keeps*
that reliability *automatically*, Phase 3 makes it *useful*, Phase 4 *fast*,
Phase 5 *publicly available*. The phases can run partly in parallel, but you
don't enter 3–5 before 0–1 are done — you can't build an ecosystem on top of
panics in the foundation.

---

## Phase 0 — Stability: closing open bugs *(current phase)*

Open bugs that came out of the full code review, in three waves by importance:

### Wave 1 — crash/DoS (can take the server down)

- [#87](https://github.com/Firdavs9512/fluxon-lang/issues/87) `json.dec` panics on malformed JSON — DoS via request body
- [#91](https://github.com/Firdavs9512/fluxon-lang/issues/91) no http request body size limit — memory DoS
- [#90](https://github.com/Firdavs9512/fluxon-lang/issues/90) no depth limit — unbounded recursion stack overflow abort
- [#89](https://github.com/Firdavs9512/fluxon-lang/issues/89) integer arithmetic overflow panic / silent wrap
- [#88](https://github.com/Firdavs9512/fluxon-lang/issues/88) `extract_from_table` Unicode char-boundary panic
- [#92](https://github.com/Firdavs9512/fluxon-lang/issues/92) no http client + ai timeout — the handler thread hangs forever

### Wave 2 — security

- [#97](https://github.com/Firdavs9512/fluxon-lang/issues/97) `rand` is not cryptographic — token/session IDs are predictable
- [#96](https://github.com/Firdavs9512/fluxon-lang/issues/96) on a cross-origin redirect the `Authorization` header leaks to a foreign host
- [#103](https://github.com/Firdavs9512/fluxon-lang/issues/103) on a db tx error a dirty connection returns to the pool without ROLLBACK

### Wave 3 — silent incorrectness (works wrong without raising an error)

- [#94](https://github.com/Firdavs9512/fluxon-lang/issues/94) `uniq(a, b)` silently drops the multi-column constraint
- [#95](https://github.com/Firdavs9512/fluxon-lang/issues/95) ai: with multiple `tool_use` blocks only the last one is kept
- [#93](https://github.com/Firdavs9512/fluxon-lang/issues/93) / [#98](https://github.com/Firdavs9512/fluxon-lang/issues/98) / [#99](https://github.com/Firdavs9512/fluxon-lang/issues/99) parser-lexer silent errors (`!x`, `m.0.1`, `1..n+1`)
- [#104](https://github.com/Firdavs9512/fluxon-lang/issues/104) `db.up` empty where — malformed SQL
- [#101](https://github.com/Firdavs9512/fluxon-lang/issues/101) repeated headers are lost
- [#105](https://github.com/Firdavs9512/fluxon-lang/issues/105) queue: handler-less jobs busy-loop, and jobs are silently lost on shutdown
- [#100](https://github.com/Firdavs9512/fluxon-lang/issues/100) query string percent-decoding

**Exit criterion:** open issues with the `bug` label = 0, and every fix shipped
with a regression test.

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
  convention, structured logging (the stdout vs stderr split started in `io`).
- **`fluxon fmt`** — canonical form is the language's philosophy, so a formatter
  is mandatory.
- **`fluxon check`** — parse + static check without running (fast feedback for
  the AI agent loop).
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

## Phase 5 — Distribution and v0.1

- **Install:** `curl | sh` + a Homebrew formula, binaries on GitHub Releases.
- **Documentation site + interactive playground** (compiled to WASM it runs in
  the browser too).
- **English translation**
  ([#58](https://github.com/Firdavs9512/fluxon-lang/issues/58)) — for an
  external audience.
- **Versioning the spec:** `fluxon-agent.md` is frozen as v0.1, breaking changes
  only with a version bump. A "real language" means a promise that code written
  today still works tomorrow.
- **Editor tooling:** syntax highlighting (VS Code extension), then an LSP.
