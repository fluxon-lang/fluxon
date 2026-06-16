<div align="center">

<!-- To use the website logo instead of the emoji, replace the line below with:
     <img src="assets/logo.png" alt="Fluxon" width="120" /> -->
<h1>🌊 Fluxon</h1>

### The AI-native general-purpose programming language

**A simple, fast, batteries-included language — designed so AI agents write it well, with the LLM built in as a first-class primitive.**

[![Build](https://github.com/fluxon-lang/fluxon/actions/workflows/ci.yml/badge.svg)](https://github.com/fluxon-lang/fluxon/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/fluxon-lang/fluxon?color=blue)](https://github.com/fluxon-lang/fluxon/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)

[**Quickstart**](#install) · [**Docs**](docs/fluxon-human.md) · [**Examples**](examples/) · [**Spec**](docs/fluxon-agent.md) · [**Roadmap**](docs/ROADMAP.md) · [O'zbek](README.uz.md)

</div>

---

> **Philosophy:** *"The language adapts to the AI, not the AI to the language."*

Fluxon is a general-purpose programming language — like Go or Python, you use it
to write scripts, tools, data-processing, services, and full applications. What
makes it different is **who it was designed for**: AI agents.

Today's languages were built for humans. They let you do one thing a dozen ways,
with syntax that's convenient but token-wasteful. For an AI agent that's noise —
every "decision point" is a chance to slip, every redundant character wastes
context. Fluxon takes the opposite path: **one task = one way**, short but
readable syntax, and the things AI-era programs reach for most — including the
LLM itself — built right into the language.

## A taste of the language

Plain, general-purpose code — functions, recursion, list pipelines, pattern
matching. No frameworks, no imports:

```fx
fn fib n
  if n < 2
    ret n
  (fib (n - 1)) + (fib (n - 2))

nums = [1 2 3 4 5 6 7 8 9 10]
evens = nums.filter \x -> x % 2 == 0
log "evens = ${evens}"
log "fib 10 = ${fib 10}"

# pattern matching on symbols
fn status s
  match s
    :new  -> "fresh"
    :done -> "complete"
    _     -> "unknown"
```

## Batteries included

When you do reach for the real world — HTTP, a database, the filesystem — it's
already in the language. Here's a whole web service:

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

That's the entire application — no package installs, no connection code, no
boilerplate.

## The AI is built into the language

Calling an LLM is a keyword, not an SDK. Classify some text, read the built-in
confidence, and branch on it — no dependency, no JSON parsing by hand:

```fx
r = ai.json "classify this ticket: ${text}" {topic::a urgency:int}
if r._.conf > 0.85               # confidence is built into the language
  log "auto-handled · cost: ${r._.cost} · tokens: ${r._.tokens}"
else
  log "low confidence → send to a human"
```

Providers auto-detect from the environment (`ANTHROPIC_API_KEY` → Claude,
`OPENAI_API_KEY` → GPT). And `ai.run` drives tool-using agents one step at a
time — so logging, cost, and approval stay in **your** code, not hidden inside
an SDK.

---

## Why Fluxon

| | |
|---|---|
| 🧩 **General-purpose** | A real language — scripts, CLIs, tools, data work, and full services. Functions, closures, pattern matching, errors, parallelism (`par`), pipes (`\|>`). |
| 🎯 **One task = one way** | The only way to iterate is `each`. One way to output. The AI never wonders "which way should I choose?" — there is no choice, so there are fewer mistakes. |
| ⚡ **Few tokens, still readable** | Short syntax, but never cryptic. Keywords are spelled out in full (`each`, `match`, `else`) — an AI seeing Fluxon for the first time understands it immediately. |
| 🔋 **Batteries included** | `http`, `db`, `ai`, `auth`, `crypto`, `ws`, `cron`, `queue`, `reg`, `sh`, `json` — all built in. No `npm install`. Only what you use ends up in the binary (tree-shaking). |
| 🤖 **AI as a primitive** | Calling an LLM is a keyword, not an SDK. Structured output, confidence, token count, and cost all come back built in. Providers auto-detect from the environment. |

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
   language deepened — from small utilities to large systems.

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
