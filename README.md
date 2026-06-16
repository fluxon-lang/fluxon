<div align="center">

<!-- To use the website logo instead of the emoji, replace the line below with:
     <img src="assets/logo.png" alt="Fluxon" width="120" /> -->
<h1>🌊 Fluxon</h1>

### The AI-native programming language

**A backend language that AI agents write well — and that makes the LLM a first-class part of the backend.**

[![Build](https://github.com/fluxon-lang/fluxon/actions/workflows/ci.yml/badge.svg)](https://github.com/fluxon-lang/fluxon/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/fluxon-lang/fluxon?color=blue)](https://github.com/fluxon-lang/fluxon/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)

[**Quickstart**](#install) · [**Docs**](docs/fluxon-human.md) · [**Examples**](examples/) · [**Spec**](docs/fluxon-agent.md) · [**Roadmap**](docs/ROADMAP.md) · [O'zbek](README.uz.md)

</div>

---

> **Philosophy:** *"The language adapts to the AI, not the AI to the language."*

Today's languages were built for humans. They let you do one thing a dozen ways,
with syntax that's convenient but token-wasteful, where even the simplest task
needs an extra package. For an AI agent that's noise — every "decision point" is
a chance to slip, every redundant character wastes context. And calling an LLM,
the thing AI-era backends do constantly, means dragging in an SDK, wiring keys,
and parsing JSON by hand.

**Fluxon is built differently** — by measuring what AI writes easily and
reliably, shaping the language around that, and making the LLM a keyword
(`ai.ask` / `ai.json` / `ai.run`) instead of a dependency.

## A whole app in one file

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

That's the entire application. No package installs, no connection code, no
boilerplate.

## The AI is right there in the language

Classify a request, read the built-in confidence, and route on it — no SDK, no
JSON parsing:

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
back to you — so logging, cost, and approval stay in **your** code, not hidden
inside an SDK.

---

## Why Fluxon

| | |
|---|---|
| 🎯 **One task = one way** | The only way to iterate is `each`. One way to output. The AI never wonders "which way should I choose?" — there is no choice, so there are fewer mistakes. |
| ⚡ **Few tokens, still readable** | Short syntax, but never cryptic. Keywords are spelled out in full (`each`, `match`, `else`) — an AI seeing Fluxon for the first time understands it immediately. |
| 🔋 **Batteries included** | `http`, `db`, `ai`, `auth`, `crypto`, `ws`, `cron`, `queue`, `reg`, `sh`, `json` — all built in. No `npm install`. Only what you use ends up in the binary (tree-shaking). |
| 🤖 **AI as a primitive** | Calling an LLM is a keyword, not an SDK. Structured output, confidence, token count, and cost all come back built in. Providers are auto-detected from the environment. |

```fx
r = ai.json "extract the order: ${text}" {intent::a items:[{product:str qty:int}]}
if r._.conf > 0.85          # confidence metadata is built into the language
  log "cost: ${r._.cost} · tokens: ${r._.tokens}"
```

Providers auto-detect from the environment (`ANTHROPIC_API_KEY` → Claude,
`OPENAI_API_KEY` → GPT) — nothing to configure.

---

## Install

**Linux / macOS** — one line (downloads the latest release, verifies its
checksum, and installs it onto your PATH):

```sh
curl -fsSL https://raw.githubusercontent.com/fluxon-lang/fluxon/master/install.sh | sh
```

**Windows** (PowerShell):

```powershell
irm https://raw.githubusercontent.com/fluxon-lang/fluxon/master/install.ps1 | iex
```

Then run a file:

```sh
fluxon run hello.fx        # run a .fx file
fluxon repl                # interactive REPL
fluxon --help              # all commands
```

<details>
<summary><b>Other install options</b></summary>

Pin a specific version with `FLUXON_VERSION=v0.1.0` (or `$env:FLUXON_VERSION` on
Windows). Prefer a manual download? Grab the archive for your platform from the
[releases page](https://github.com/fluxon-lang/fluxon/releases).

**From source** (Rust toolchain required):

```sh
cd runtime
cargo run -- run examples/demo.fx
# or install the binary:  cargo install --path runtime
```

</details>

---

## Status — Beta

The language core and **every battery in the spec** are implemented and covered
by **479 passing tests**. The runtime (Rust, tree-walking interpreter) runs
`.fx` files, serves HTTP/WebSocket, talks to a database, and drives LLM agents
today.

<details>
<summary><b>What works right now</b></summary>

- **Language core:** types, bindings (`=`/`<-`), `fn`/lambda/closure,
  `if`/`each`/`match`, operators, string interpolation, errors (`fail`/`!`/`??`),
  `try`/`catch`, `par` (parallel fan-out), and the `|>` pipe.
- **Core modules:** `str`, `math`, `rand`, `json`, `time`, `env`, `io`, `fs`,
  `sh`, leveled `log`, plus `assert` + a built-in `fluxon test` runner and an
  interactive REPL.
- **Batteries (all of them):** **`http`** (server + client + middleware +
  static), **`db`** (SQLite, transactions, schema, auto-migration, query
  builder), **`ai`** (LLM — `ai.ask`/`ai.json`/`ai.run`, Anthropic + OpenAI
  auto-detect, tool-loop, confidence/token/cost metadata, retry + timeout),
  **`auth`** (JWT + password hashing), **`crypto`**, **`ws`** (websocket),
  **`cron`**, **`queue`**, **`reg`** (tool registry for agents).

The CLI ships `fluxon run`, `fluxon check` (lex + parse), `fluxon test`, and an
interactive `fluxon repl`.

</details>

What's still on the roadmap (Postgres/MySQL backends, semantic/static checking,
`fluxon fmt`, packaging, an LSP) is tracked in
[`docs/ROADMAP.md`](docs/ROADMAP.md).

---

## How the language was designed

Fluxon was built through **stress testing** — with evidence, not guesswork:

1. **Research** — we studied which code patterns AI writes most reliably and
   with the fewest tokens (declarative DSLs, canonical form, batteries).
2. **Invention** — several AI models were each asked to "invent a language for
   AI." Independently, multiple models converged on the same ideas — and that
   convergence showed there is a "correct" design.
3. **Testing** — the spec was handed to AI models that had **never seen** the
   language (opus, sonnet, haiku) and asked to build real projects. Each "spec
   gap" a model hit exposed a real shortcoming.
4. **Refinement** — the gaps were closed, then re-tested. Over several rounds the
   language deepened — from small utilities to large systems (e-commerce,
   realtime chat).

The whole process is preserved in the [`research/`](research/) folder.

---

## Explore

| Path | What's inside |
|------|---------------|
| [`docs/fluxon-agent.md`](docs/fluxon-agent.md) | Compact spec for AI agents (~2700 tokens) |
| [`docs/fluxon-human.md`](docs/fluxon-human.md) | Detailed guide for humans |
| [`examples/support-tickets/`](examples/support-tickets/) | AI classification + confidence routing |
| [`examples/ecommerce/`](examples/ecommerce/) | Catalog, cart, checkout (transaction), AI recommendations |
| [`examples/chat/`](examples/chat/) | Realtime websocket + AI moderation |
| [`research/`](research/) | How the language was born — design experiments |

---

## Contributing

Fluxon is open source — we welcome your help.

- **Human contributors:** [`CONTRIBUTING.md`](CONTRIBUTING.md) — setup, build,
  test, PR process.
- **AI agents (Claude Code etc.):** [`CLAUDE.md`](CLAUDE.md) — rules, navigation,
  "what is where".
- **Runtime internals:** [`ARCHITECTURE.md`](ARCHITECTURE.md).

---

## License

[MIT](LICENSE)

<div align="center">

---

*Fluxon isn't built to replace or outcompete existing programming languages.
The goal is just one: to be **the language AI knows best and likes most**.*

</div>
