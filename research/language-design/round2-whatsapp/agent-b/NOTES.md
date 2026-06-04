# NOTES — Nol Language Design Decisions

## Language Name & Philosophy

**Nol** ("zero" in Uzbek/Russian) — *Every line is a command; nothing is noise.*

---

## Key Design Decisions

### 1. Newline-terminated, indentation-scoped
Python-style but stricter: no colons after `fn`, `if`, `each`, `loop`. The keyword alone is enough signal. Saves 1 token per block header, and there are many block headers in real code.

### 2. Only `let` for assignment — always
No `var`, `const`, `mut`. Re-assigning is just another `let`. This violates purity purists but makes the rule dead simple: if you're naming a value, write `let`. Immutability is opt-in via convention (`let` + never reassigning), not enforced by syntax.

### 3. Three control-flow keywords only: `if`, `each`, `loop`
- `each` covers both iteration over lists and map entries (`each k, v in map`).
- `loop` + `break if` replaces `while`.
- No `for`, no `do...while`, no `repeat`. One way, always.

### 4. Batteries via dot-namespaced stdlib
`db`, `ai`, `http`, `queue`, `env`, `json` — all lowercase, one-word namespaces. No imports needed for stdlib. Only user modules need `import`. Keeps the top of every file clean.

### 5. `ai.extract(prompt, schema)` is the central innovation
The schema is a Nol map literal — the same syntax developers already know for data. The runtime validates and retries once. This collapses what would be 20+ lines of JSON schema definition + parse + validate into a single expression.

### 6. `db.tx` block for transactions
No callback hell, no `BEGIN`/`COMMIT` ceremony. Indent under `db.tx`, done.

### 7. `serve port` block for HTTP
Replaces Express-style boilerplate: no `app = new Server()`, no `app.listen()`, no `router.post(path, (req, res) => {...})`. The block *is* the router.

### 8. `cron "expr"` block
Cron expressions are universally understood; wrapping them in a block makes scheduling look identical to other declarations. No imports, no `node-cron.schedule(...)`.

---

## Token Efficiency Reasoning

| Construct | Nol | Python equiv | Savings |
|-----------|-----|-------------|---------|
| Function definition | `fn f(x)` | `def f(x):` | -1 token (no colon) |
| If block | `if x` | `if x:` | -1 token |
| Each loop | `each x in y` | `for x in y:` | -1 token |
| DB query | `db.query(sql, params)` | 4-6 lines (connect/cursor/execute/fetch) | ~5x shorter |
| HTTP route | `post "/path"` | 2-3 lines | ~2x shorter |
| AI extract | `ai.extract(p, schema)` | 10-20 lines | ~10x shorter |
| Cron job | `cron "0 9 * * MON"` + block | 3-5 lines | ~3x shorter |

---

## Tradeoffs

- **No static types**: The constraint "learnable in one look" made annotations feel like noise for this domain. Runtime errors over compile-time safety. Acceptable for a scripting/glue-code context.
- **No explicit return type**: Functions return whatever `return` says. Helps terse reading; hurts documentation. Mitigated by descriptive function names.
- **`let` for reassignment** feels odd to typed-language programmers. The tradeoff: zero cognitive overhead to decide `var` vs `const` vs `let`.
- **No async/await syntax**: The runtime handles concurrency internally (webhook handlers run concurrently, cron runs in background workers, queue workers run in a pool). The language surface is synchronous. This is the biggest "batteries" assumption — real implementation would need a runtime like Deno or a custom interpreter.
- **`continue` is borrowed**: Used in `cron.nol` inside an `each` loop. Minor inconsistency (not declared in SPEC). Real revision: add `skip` as the canonical form.

---

## How the Real Project Stressed the Language

1. **Uzbek strings**: Nol handles Unicode natively (strings are UTF-8). No issues.
2. **Confidence routing**: The `if`/`else if` chain for three confidence bands is clear and explicit. One-way to do branching enforced naturally.
3. **Multi-owner architecture**: Forced the tools to always take `owner_id` as a parameter, making it obvious in every tool call that data is scoped.
4. **`ON CONFLICT DO NOTHING`** in raw SQL: Nol's `db.exec` passes SQL verbatim — escape hatch for complex SQL is always available. No ORM magic needed.
5. **Owner reply parsing** (`"HA +998..."`) needed `starts_with` and `after` string operations — these would be in the stdlib but weren't formally specced. Real iteration would add them to the SPEC.
6. **`add_days`, `today`, `this_monday`, `now`, `uptime`** — temporal and system builtins assumed available. SPEC should have a "builtins" section for these.

---

## Approximate Token Counts (GPT-4o tokeniser)

| File | ~Tokens |
|------|---------|
| SPEC.md | ~1 400 |
| schema.nol | ~580 |
| webhook.nol | ~870 |
| tools.nol | ~760 |
| cron.nol | ~700 |
| main.nol | ~650 |
| whatsapp.nol | ~280 |
| NOTES.md | ~700 |
| **Total** | **~5 940** |
