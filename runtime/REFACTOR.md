# Fluxon runtime — Refactor plan (module splitting)

> Status: planning. No code is changed by this document — it is the map for a
> series of mechanical, behavior-preserving refactors tracked as GitHub issues.

## Why

Several source files have grown past the point where they are comfortable to
navigate or review. The five largest files account for ~15.7k of the 22.6k lines
in `runtime/src/`:

| File | Lines | Real cause of size |
|------|------:|--------------------|
| `main.rs` | 3962 | ~420 lines CLI + **~3540 lines of integration tests** in one `mod tests` (186 tests) |
| `http_mod.rs` | 3928 | server + client + routing + static + multipart + rate-limit + 94 tests |
| `builtins.rs` | 2937 | 9 built-in modules (`str/math/rand/json/time/io/fs/sh/bytes`) + methods + 65 tests |
| `db_mod.rs` | 2497 | pool + sqlite + migration + tx + query-builder + 13 tests |
| `interp.rs` | 2352 | scope + flow + 1829-line `impl Interp` (stmt/expr/call/module) |

The work is **structural only**: move code into submodules, keep behavior and the
public API identical, keep every test green. Each step is one PR.

## Guiding rules (apply to every step)

- **One PR = one file's split.** Do not mix splitting with feature changes.
- **No behavior change.** `cargo test` must stay green at every step; diffs should
  be ~pure moves (plus `mod`/`use` glue). Prefer `git mv`-like moves so review is
  about placement, not logic.
- **Preserve the public surface.** Where other modules import `http_mod::Foo`, keep
  `pub use` re-exports in the parent module so call sites don't churn. This keeps
  each PR small and decoupled from the others.
- **Tests move with their code.** When a cluster moves to its own file, its unit
  tests move into a `#[cfg(test)] mod tests` in that same file.
- **Green-PR checklist** (from `CLAUDE.md`): `cargo build --locked`,
  `cargo test --locked`, `cargo fmt --check`, `cargo clippy --all-targets -D warnings`,
  `cargo run -- run examples/demo.fx`.

## Test placement decision

`main.rs` is a **binary-only crate** (no `[lib]`), and its 186 integration tests
use the private `run_source()` helper. Moving them to `runtime/tests/` would force
either a lib target or a subprocess harness (slower). Instead:

- Declare `#[cfg(test)] mod tests;` in `main.rs` and put the tests under
  `src/tests/` as topic submodules. Private items stay reachable; no subprocess.
- A shared `src/tests/mod.rs` holds the common helpers (`run`, `with_db_test`,
  `setup_db`/`cleanup_db`, `run_modules`, `temp_module_dir`, `repl_chunk`,
  `AI_ENV_LOCK`) and `pub(crate) use` re-exports them to the topic files.

The same in-file `#[cfg(test)] mod tests` convention is kept for the other modules
(`http_mod`, `builtins`, ...): each new submodule carries its own tests.

---

## Step 1 — `main.rs`: extract the 186 integration tests (do first)

**Lowest risk, biggest line win.** Production CLI code (lines 1–419) stays in
`main.rs`. The `mod tests` block (lines 420–3962) moves to `src/tests/`, one file
per topic.

`main.rs` keeps only:
```rust
#[cfg(test)]
mod tests;          // -> src/tests/mod.rs
```

`src/tests/mod.rs` holds shared helpers + `mod` declarations for each topic file.

Topic files (line ranges are in current `main.rs`; counts approximate):

| File | Tests | Source lines |
|------|------:|--------------|
| `tests/mod.rs` (helpers + decls) | — | helpers at 425, 1783, 1992, 2005, 2905, 2984, 2995, 3877 |
| `tests/log_tests.rs` | 3 | 434–610 |
| `tests/par_tests.rs` | 9 | 467–587 |
| `tests/try_tests.rs` | 12 | 622–769 |
| `tests/range_tests.rs` | 5 | 815–872 |
| `tests/rep_tests.rs` | 9 | 908–1034 |
| `tests/call_tests.rs` | 3 | 1050–1089 |
| `tests/list_tests.rs` | 8 | 1102–1238 |
| `tests/map_tests.rs` | 5 | 1264–1340 |
| `tests/bind_tests.rs` | 7 | 1358–1443 |
| `tests/str_time_tests.rs` | 8 | 1456–1559 |
| `tests/env_json_reg_tests.rs` | 9 | 1576–1695 |
| `tests/pipe_fail_tests.rs` | 4 | 1706–1760 |
| `tests/db_tests.rs` | 8 | 1792–1976 |
| `tests/migrate_tests.rs` | 11 | 2012–2464 |
| `tests/db_tx_tests.rs` | 7 | 2519–2720 |
| `tests/cron_tests.rs` | 4 | 2747–2776 |
| `tests/queue_tests.rs` | 8 | 2796–2865 |
| `tests/ai_sh_tests.rs` | 5 | 2908–2971 |
| `tests/use_module_tests.rs` | 10 | 3017–3166 |
| `tests/math_each_tests.rs` | 5 | 3176–3225 |
| `tests/framework_tests.rs` | 11 | 3239–3422 |
| `tests/sym_tests.rs` | 2 | 3440–3458 |
| `tests/auth_tests.rs` | 8 | 3472–3596 |
| `tests/interp_err_tests.rs` | 4 | 3608–3649 |
| `tests/block_str_tests.rs` | 9 | 3659–3757 |
| `tests/crypto_tests.rs` | 3 | 3770–3796 |
| `tests/bytes_tests.rs` | 6 | 3806–3867 |
| `tests/repl_tests.rs` | 4 | 3884–3924 |
| `tests/str_order_tests.rs` | 2 | 3939–3955 |

**Watch-outs:**
- `run()` (425) calls private `run_source()` — stays reachable via `src/tests/`.
- DB tests share `with_db_test`/`setup_db`/`cleanup_db`; keep them in `mod.rs`.
- AI tests serialize on `AI_ENV_LOCK` (static Mutex) — move to `mod.rs` so the
  lock is one shared instance, not per-file.
- `use_*` tests need `temp_module_dir`/`run_modules`.
- Result target: `main.rs` ~420 lines; each test file 50–300 lines.

---

## Step 2 — `http_mod.rs` → `http/` submodules

Façade `http_mod.rs` keeps `pub use` re-exports; logic moves to `src/http/`.

| New file | ~Lines | Contents |
|----------|-------:|----------|
| `http/routing.rs` | 350 | `Seg`, `Route`, `parse_pattern`, `path_segments`, `normalize_method`, `match_route`, `prefix_matches` |
| `http/request.rs` | 300 | `percent_decode`, `parse_query`, multipart parsing, `build_req` |
| `http/response.rs` | 280 | `value_to_response`, `apply_headers*`, `headers_to_map`, json/text/error builders, `is_resp` |
| `http/static_files.rs` | 320 | `StaticMount`, `safe_join`, `mime_for`, `try_serve_static` |
| `http/limits.rs` | 110 | `LimitBucket`, `LimitState`, rate-limit window/count/response |
| `http/middleware.rs` | 350 | `MwKind`, `Middleware`, `CorsConfig`, CORS finalize/preflight, `run_middleware_chain` |
| `http/client.rs` | 360 | `ClientOpts`, `pooled_http_client`, `client_runtime`, `http_client`, redirect helpers (`pub(crate)` — used by `ai_mod`) |
| `http/server.rs` | 250 | `bind`, `serve_loop`, `handle_request` |
| `http/interp.rs` | 314 | `impl Interp` http_* dispatch + register handlers |

**Watch-outs:** `pooled_http_client`/`client_runtime` are used by `ai_mod` — keep
`pub(crate)` and re-export. `headers_to_map`, `value_to_response`, `apply_headers*`,
`is_resp`, `percent_decode`, `prefix_matches` are cross-cluster — put them in their
home module and `pub(crate)` them. 94 tests split across the modules they cover.

---

## Step 3 — `builtins.rs` → `builtins/` submodules ✅ DONE (PR for #186)

Root `builtins.rs` keeps `install()`, the log subsystem, `is_module`/`call_module`
dispatch, and `R`; the shared `arg*` helpers moved to `builtins/args.rs`; each
built-in module got its own file. Façade is 353 lines (down from 2937); test
count unchanged (456 lib + integration). No external call site touched.

| New file | ~Lines | Contents |
|----------|-------:|----------|
| `builtins/str_mod.rs` | 160 | `str_module` + tests |
| `builtins/bytes_mod.rs` | 100 | `bytes_module` + tests |
| `builtins/math_mod.rs` | 100 | `math_module`, `cmp_int_flt` + tests |
| `builtins/rand_mod.rs` | 60 | `rand_module`, `next_rand` + tests |
| `builtins/time_mod.rs` | 570 | `time_module` + civil/parse/zone helpers + tests |
| `builtins/io_mod.rs` | 60 | `io_module` + tests |
| `builtins/fs_mod.rs` | 120 | `fs_module` + tests |
| `builtins/sh_mod.rs` | 60 | `sh_module` + tests |
| `builtins/json_mod.rs` | 350 | `json_module`, `json_encode/decode`, `JsonParser` + tests |
| `builtins/methods.rs` | 250 | `call_method`, `list_method`, `map_method`, sort helpers |

**Watch-outs:** `arg*` helpers are used by every module — keep them `pub(crate)`
in the root or a small `builtins/args.rs`. `json_encode`/`json_decode` are `pub`
and used by `http`/`db`/`ai` — preserve via re-export. Root target ~520 lines.

---

## Step 4 — `db_mod.rs` → `db/` submodules

| New file | ~Lines | Contents |
|----------|-------:|----------|
| `db/values.rs` | 120 | `SqlVal`, `Row`, `ColDef`, `IndexDef`, `ForeignKey`, `Db`/`DbTx` traits |
| `db/pool.rs` | 130 | `Pool`, `SqliteDb::open`, `open_from_env` |
| `db/sqlite.rs` | 230 | value conversion, `run_query`/`run_exec`, `impl Db for SqliteDb` |
| `db/migrate.rs` | 380 | introspection, `sqlite_rebuild_table`, all DDL builders, `index_name`/`fnv1a` |
| `db/tx.rs` | 155 | `SqliteTx`, `impl DbTx`, `CURRENT_TX` thread_local, `TxClearGuard`, `with_db` |
| `db/interp.rs` | 1070 | `db_dispatch`, `db_q/one/ins/up/del`, tx outer/nested, query builder stages, value↔SQL |
| (tests stay in each module) | 357 | 13 tests using `mem_db` helper |

**Watch-outs:** the `Db`/`DbTx` traits and `SqlVal` are the shared contract — put
them in `db/values.rs` and re-export. `index_name`, `q_ident`, `build_*` are
`pub`/`pub(crate)` (used by migration tests + interp) — preserve. `interp.rs` is
still ~1070 lines; a later optional step can split builder stages from dispatch.

---

## Step 5 — `interp.rs` → `interp/` submodules (optional / lower priority)

The 1829-line `impl Interp` can be split across files using Rust's "impl block per
file" pattern (same struct, methods in different `impl Interp` blocks).

| New file | ~Lines | Contents |
|----------|-------:|----------|
| `interp/scope.rs` | 200 | `Env`, `Parent`, `Scope` + impl, `Flow`, `CallDepthGuard` |
| `interp/mod.rs` (root) | 200 | `Interp` struct, `ColMeta`/`TableMeta`, lifecycle (`new`, `freeze_globals`) |
| `interp/exec.rs` | 350 | `run`, `run_repl_chunk`, statement execution, `exec_each` |
| `interp/expr.rs` | 300 | `eval`, `eval_if`, `eval_match`, try/catch, short-circuit |
| `interp/call.rs` | 400 | `eval_binary`, `eval_call`, list HOFs, `apply`, `get_field`, `get_index` |
| `interp/module.rs` | 170 | `load_module`, `register_tbl`, module caching |
| `interp/util.rs` | 150 | module paths, `.env` parsing, arithmetic + dotenv tests |

**Watch-out:** this is the highest-traffic file (every other module's dispatch is
`impl Interp`). Do it last, and keep method visibility identical.

---

## Sequencing

```
Step 1 (main.rs tests)        ─ independent, do first, lowest risk
Step 2 (http_mod)  ┐
Step 3 (builtins)  ├─ independent of each other; any order, parallelizable
Step 4 (db_mod)    ┘
Step 5 (interp)               ─ last (touched by all dispatch); optional
```

Each step lands as its own `fix-refactor-<area>` branch + PR.
