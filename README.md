# Flux

> 🌐 **Language:** English (current) · [O'zbek](README.uz.md)

**A backend programming language that AI agents write well.**

> Philosophy: *"The language adapts to the AI, not the AI to the language."*

Today's programming languages were built for humans. They let you do one thing
a dozen different ways, with syntax that is convenient but token-wasteful, where
even the simplest task requires an extra package. For an AI agent that is noise:
every "decision point" is a potential mistake, every redundant character is
wasted context.

Flux is built differently — by measuring what AI writes easily and reliably, and
shaping the language around that.

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

---

## Core principles

1. **One task = one way (canonical form).** The only way to iterate is `each`.
   There is only one way to output. The AI never wonders "which way should I
   choose?" — there is no choice, so there are fewer mistakes.

2. **Few tokens, but readable.** The syntax is short, but not cryptic. Keywords
   are spelled out in full (`each`, `match`, `else`) — an AI seeing Flux for the
   first time understands it immediately.

3. **Batteries included.** `http`, `db` (transactions + concurrency guarantees),
   `ai`, `reg` (tool registry), `ws`, `cron`, `queue`, `sh` (shell), `json` —
   all built into the language. No `npm install`. At compile time only what is
   used ends up in the binary (tree-shaking).

4. **AI is a first-class primitive.** Calling an LLM is a keyword, not an SDK:
   ```fx
   r = ai.json "extract the order: ${text}" {intent::a items:[{product:str qty:int}]}
   if r._.conf > 0.85
     auto r          # confidence metadata is built into the language
   ```

---

## How this language was designed (methodology)

Flux was built through **stress testing** — with evidence, not guesswork:

1. **Research:** we studied which code patterns AI writes most reliably and with
   the fewest tokens (declarative DSLs, canonical form, batteries — see the
   `research/` folder).
2. **Invention:** several AI models were each given the task "invent a language
   for AI." Independently, multiple models converged on the same ideas — and
   that convergence showed there is a "correct" design.
3. **Testing:** the Flux spec was handed to AI models that had **never seen** the
   language (opus, sonnet, haiku), which were asked to write real projects. Each
   "spec gap" a model hit exposed a real shortcoming of the language.
4. **Refinement:** the gaps found were closed, then re-tested. Over several
   rounds the language deepened — from small utilities (URL shortener) to large
   systems (e-commerce, realtime chat).

This whole process is preserved in full in the `research/` folder.

---

## Repository structure

```
flux-lang/
├── docs/
│   ├── flux-human.md      # detailed guide (for humans, Uzbek)
│   ├── flux-human.en.md   # detailed guide (for humans, English)
│   └── flux-agent.md      # compact spec (for AI agents — ~2700 tokens)
├── examples/              # working example projects
│   ├── support-tickets/   # AI classification + confidence routing
│   ├── ecommerce/         # catalog, cart, checkout (transaction), AI recommendations
│   └── chat/              # realtime websocket, AI moderation
└── research/              # how the language was born — design experiments
    └── language-design/
        ├── round1-invented-langs/   # AIs invent languages
        ├── round2-whatsapp/         # invention driven by a real project
        └── validation-tests/        # testing Flux on fresh AIs
```

---

## Current status

🚧 **Under active development.** A working **runtime** for the language core
exists (Rust, tree-walking interpreter) — it can run `.fx` files.

**Working:**

- Language core: types, bindings (`=`/`<-`), `fn`/lambda/closure, `if`/`each`/
  `match`, operators, string interpolation, `fail`/`!`/`??`/`|>`.
- Core modules: `str`, `math`, `rand`, `json`, `time`, `env`.
- Batteries: **`http`** (server + client), **`db`** (SQLite, transactions,
  schema, auto-migration).

**Not yet (in the spec):** `ai`, `reg`, `ws`, `cron`, `queue`.

Run it:

```sh
cd runtime
cargo run -- run examples/demo.fx
```

---

## Contributing

Flux is open source — we welcome your help.

- **Human contributors:** [`CONTRIBUTING.md`](CONTRIBUTING.md) — setup, build,
  test, PR process.
- **AI agents (Claude Code etc.):** [`CLAUDE.md`](CLAUDE.md) — rules,
  navigation, "what is where".
- **Runtime internals:** [`ARCHITECTURE.md`](ARCHITECTURE.md).

---

## License

MIT

---

> **Note.** Flux is not being built to replace or outcompete existing global
> programming languages. The goal is just one: to be **the programming language
> AI knows best and likes most**.
