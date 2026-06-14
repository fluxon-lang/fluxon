# Fluxon

> 🌐 **Language:** English (current) · [O'zbek](README.uz.md)

**An AI-native programming language — one that AI agents write well, and that
makes AI a first-class part of the backend.**

> Philosophy: *"The language adapts to the AI, not the AI to the language."*

Today's programming languages were built for humans. They let you do one thing
a dozen different ways, with syntax that is convenient but token-wasteful, where
even the simplest task requires an extra package. For an AI agent that is noise:
every "decision point" is a potential mistake, every redundant character is
wasted context. And calling an LLM — the thing AI-era backends do constantly —
means dragging in an SDK, wiring keys, and parsing JSON by hand.

Fluxon is built differently — by measuring what AI writes easily and reliably,
shaping the language around that, and making the LLM a keyword (`ai.ask` /
`ai.json` / `ai.run`) instead of a dependency.

```fx
use http db

tbl notes
  id   serial pk
  text str
  ts   now

http.on :post "/notes" \req ->
  rep 201 (db.ins "notes" {text:req.body.text})

http.on :get "/notes" \req ->
  rep 200 (db.q "select * from notes order by ts desc")

http.serve 8080
```

That is the whole application. No package installs, no connection code, no
boilerplate.

And the AI is right there in the language. Classify a request, read the built-in
confidence, and route on it — no SDK, no JSON parsing:

```fx
use http ai

http.on :post "/triage" \req ->
  r = ai.json "classify this ticket: ${req.body.text}" {topic::a urgency:int}
  if r._.conf > 0.85
    rep 200 {auto:true result:r}      # confidence is built into the language
  else
    rep 200 {auto:false review:true}  # low confidence → send to a human

http.serve 8080
```

Need a tool-using agent? `ai.run` returns one step of the loop and hands control
back to you — so logging, cost, and approval stay in your code, not hidden in an
SDK.

---

## Core principles

1. **One task = one way (canonical form).** The only way to iterate is `each`.
   There is only one way to output. The AI never wonders "which way should I
   choose?" — there is no choice, so there are fewer mistakes.

2. **Few tokens, but readable.** The syntax is short, but not cryptic. Keywords
   are spelled out in full (`each`, `match`, `else`) — an AI seeing Fluxon for the
   first time understands it immediately.

3. **Batteries included.** `http`, `db` (transactions + concurrency guarantees),
   `ai`, `reg` (tool registry), `ws`, `cron`, `queue`, `sh` (shell), `json` —
   all built into the language. No `npm install`. At compile time only what is
   used ends up in the binary (tree-shaking).

4. **AI is a first-class primitive.** Calling an LLM is a keyword, not an SDK —
   structured output, confidence, token count, and cost all come back built in:
   ```fx
   r = ai.json "extract the order: ${text}" {intent::a items:[{product:str qty:int}]}
   if r._.conf > 0.85          # confidence metadata is built into the language
     log "cost: ${r._.cost} · tokens: ${r._.tokens}"
   ```
   Providers are auto-detected from the environment (`ANTHROPIC_API_KEY` → Claude,
   `OPENAI_API_KEY` → GPT) — nothing to configure. `ai.run` drives tool-using
   agents, with the loop (and its logging/cost/approval) staying in your code.

---

## How this language was designed (methodology)

Fluxon was built through **stress testing** — with evidence, not guesswork:

1. **Research:** we studied which code patterns AI writes most reliably and with
   the fewest tokens (declarative DSLs, canonical form, batteries — see the
   `research/` folder).
2. **Invention:** several AI models were each given the task "invent a language
   for AI." Independently, multiple models converged on the same ideas — and
   that convergence showed there is a "correct" design.
3. **Testing:** the Fluxon spec was handed to AI models that had **never seen** the
   language (opus, sonnet, haiku), which were asked to write real projects. Each
   "spec gap" a model hit exposed a real shortcoming of the language.
4. **Refinement:** the gaps found were closed, then re-tested. Over several
   rounds the language deepened — from small utilities (URL shortener) to large
   systems (e-commerce, realtime chat).

This whole process is preserved in full in the `research/` folder.

---

## Repository structure

```
fluxon-lang/
├── docs/
│   ├── fluxon-human.md      # detailed guide (for humans, English)
│   ├── fluxon-human.uz.md   # detailed guide (for humans, Uzbek)
│   ├── fluxon-agent.md      # compact spec (for AI agents — ~2700 tokens)
│   └── ROADMAP.md           # phases, what's done, what's planned
├── examples/              # working example projects
│   ├── support-tickets/   # AI classification + confidence routing
│   ├── ecommerce/         # catalog, cart, checkout (transaction), AI recommendations
│   └── chat/              # realtime websocket, AI moderation
└── research/              # how the language was born — design experiments
    └── language-design/
        ├── round1-invented-langs/   # AIs invent languages
        ├── round2-whatsapp/         # invention driven by a real project
        └── validation-tests/        # testing Fluxon on fresh AIs
```

---

## Current status

**Beta.** The language core and every battery in the spec are implemented and
covered by 479 passing tests. The runtime (Rust, tree-walking interpreter) runs
`.fx` files, serves HTTP/WebSocket, talks to a database, and drives LLM agents
today.

**Working:**

- Language core: types, bindings (`=`/`<-`), `fn`/lambda/closure, `if`/`each`/
  `match`, operators, string interpolation, errors (`fail`/`!`/`??`),
  `try`/`catch`, `par` (parallel fan-out), and the `|>` pipe.
- Core modules: `str`, `math`, `rand`, `json`, `time`, `env`, `io`, `fs`, `sh`,
  leveled `log`, plus `assert` + a built-in `fluxon test` runner and an
  interactive REPL.
- Batteries (all of them): **`http`** (server + client + middleware + static),
  **`db`** (SQLite, transactions, schema, auto-migration, query builder),
  **`ai`** (LLM — `ai.ask`/`ai.json`/`ai.run`, Anthropic + OpenAI auto-detect,
  tool-loop, confidence/token/cost metadata, retry + timeout), **`auth`** (JWT +
  password hashing), **`crypto`**, **`ws`** (websocket), **`cron`**, **`queue`**,
  **`reg`** (tool registry for agents).

The CLI ships `fluxon run`, `fluxon check` (lex + parse, no semantic check yet),
`fluxon test`, and an interactive `fluxon repl`. What's still on the roadmap
(Postgres/MySQL backends, semantic/static checking, `fluxon fmt`, packaging, an
LSP) is tracked in [`docs/ROADMAP.md`](docs/ROADMAP.md).

Run it:

```sh
cd runtime
cargo run -- run examples/demo.fx
```

---

## Contributing

Fluxon is open source — we welcome your help.

- **Human contributors:** [`CONTRIBUTING.md`](CONTRIBUTING.md) — setup, build,
  test, PR process.
- **AI agents (Claude Code etc.):** [`CLAUDE.md`](CLAUDE.md) — rules,
  navigation, "what is where".
- **Runtime internals:** [`ARCHITECTURE.md`](ARCHITECTURE.md).

---

## License

MIT

---

> **Note.** Fluxon is not being built to replace or outcompete existing global
> programming languages. The goal is just one: to be **the programming language
> AI knows best and likes most**.
