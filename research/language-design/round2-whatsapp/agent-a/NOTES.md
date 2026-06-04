# Flux — Design Notes

## Philosophy
A terse, batteries-included server language where the AI/LLM is a first-class
primitive, not a library you wire up. Indentation blocks, newline statements,
space-separated args — the shape of the language is shaped like the backends
people actually build today.

## Key decisions

**AI as a keyword, not a SDK.** `ai.json prompt schema` does typed extraction in
one line and returns confidence on `r._.conf`. The whole confidence-routing core
(classify + extract + confidence) is a *single* call. This is the biggest
token-saver: no client setup, no JSON-mode flags, no retry boilerplate. `ai.run`
gives an agentic tool-loop by just handing it a list of fn refs.

**Audit metadata rides along.** Every `ai.*` result carries `_.tokens _.cost _.ms
_.conf`. The `ai_interactions` table fills itself from one map — no manual timing
or token math.

**DB with no ORM ceremony.** `db.ins "table" {..}`, `db.up`, `db.one`, `db.q`.
Tables declared with `tbl` in the same language as the code. `$DATABASE_URL` is
read implicitly, so there is zero connection code in `main.flux`.

**Webhook = one line.** `http.on :post "/path" handler` + `http.serve`. `rep`
auto-encodes JSON. No router, no middleware tax.

**Cron is a verb.** `cron.wk :sun 18 0 fn` reads like English. The whole weekly
outreach + Sunday briefing fits in ~30 lines.

## Token efficiency reasoning
- Space-separated args (`send ph body`) kill comma+paren noise vs `send(ph, body)`.
- `!` for error-propagation replaces multi-line `if err != nil { return err }`.
- Two-letter control keywords (`ea ef el mt wh→removed`) and stdlib verbs
  (`db.q db.ins ai.json cron.wk`) keep call sites dense but readable.
- Symbols (`:new_order`) avoid quoting enums everywhere.
- String interpolation `"${x}"` removes concat chains.

## One-way-to-do-one-thing enforcement
- ONE loop: `ea`. Dropped `while` (range/recurse covers it).
- ONE conditional family: `if/ef/el` and `mt` for value dispatch — distinct jobs.
- ONE output to user (`rep` for HTTP, `log` for stderr) — they are different sinks,
  so they are allowed to look different per constraint 3.
- ONE bind (`=`) vs ONE mutable bind (`<-`) — distinct operations, distinct syntax.

## Tradeoffs / where the project stressed Flux
- **Significant whitespace + deep nesting**: the `briefing` cron with a join and a
  loop pushes indentation; a richer query helper or pipelines would flatten it.
- **No native pattern-binding in `match`** (e.g. destructuring order shapes) — I
  kept `mt` to bare tag matching to preserve one-look simplicity, at the cost of
  manual field access.
- **Single-tenant shortcut** (`users limit 1`) — multi-tenant routing would need
  an owner resolved from the inbound number; the language handles it fine, the
  demo just didn't need it.
- **`_.conf` confidence is provider-magic**: Flux assumes the LLM battery returns
  calibrated confidence. Real life needs a logprob/self-eval strategy; the spec
  hides that behind the battery deliberately.
- Multi-line SQL strings work but blunt the terseness win — a query builder was
  tempting but would have violated "one way to do one thing".

## Verdict
The AI-native batteries paid off hardest exactly where this project is heaviest:
the classify/extract/route core and the cron briefing are each tiny. DB and
webhook plumbing nearly vanished. The main pressure point is nesting depth in
data-heavy procedures.
