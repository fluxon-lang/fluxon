# Flux — Design Notes & Tradeoffs

## Language Name
**Flux** (`.fx`) — reflects flow, pipelines, and the idea that data moves through the program.

---

## Core Design Decisions

### 1. Sigils for type clarity (`$`, `#`, `?`, `@`, `%`, `~`)

**Why sigils?** They eliminate the need for `var`, `let`, `const`, `string`, `int`, `bool`, `Array`, `Map` keywords entirely. One character at first assignment encodes type. The cost: unfamiliar look at first glance. The gain: every declaration is 1-3 tokens shorter, and at a glance you know what kind of value you're dealing with without reading the RHS.

Sigil only appears on **first write** — not on reads. This avoids sigil noise on every reference (unlike Perl/Ruby which put sigils on every read).

`~` (any/dynamic) is the escape hatch — used for JSON parse results, DB rows, anything where static type isn't known. It's ugly enough to be noticeable, encouraging you to assign real types when you can.

**Tradeoff**: Sigils vs no-sigils. A sigil-free language is cleaner visually but requires more keywords or type inference. Given the "one-look learnability" goal, sigils are more self-documenting than inference.

---

### 2. One loop form: `each`

`for`, `while`, `do-while`, `forEach`, `loop` all do the same thing conceptually: repeat while some condition holds. **Flux unifies them under `each`:**

- `each x in list` — iterate a list/map
- `each #i in 1..10` — numeric range
- `each ?cond` — while-style (loop while bool true)

This eliminates 3 keywords and 2 syntax forms. The tradeoff: `each ?cond` for while-loops looks slightly unusual, but it reads as "each [time the condition is] true" which is accurate.

---

### 3. No parentheses for function calls (optional parens)

`show "hello"` instead of `show("hello")`. `greet name` instead of `greet(name)`. Parens are only required when disambiguation is needed (multi-argument calls in expression position, or chaining).

This shaves 2 tokens per call — in a program with 50 function calls that's 100 characters of pure syntax noise eliminated.

**Tradeoff**: parsing ambiguity. `f a b` — is that `f(a, b)` or `f(a)(b)`? Flux treats it as `f(a, b)`. Chained calls need parens: `f(g(x))`. This is the same tradeoff ML/Haskell makes, and it works well once internalized.

---

### 4. Implicit return (last expression)

No `return` keyword. The last expression in a function body is its value. This is standard in functional languages and saves a keyword per function.

**Tradeoff**: less explicit. But the `-> type` hint on functions signals "this produces a value" and makes it clear a return is happening.

---

### 5. Batteries-included stdlib design

The core design principle for the stdlib: **zero ceremony**. Compare:

*Node.js HTTP server (minimal):*
```js
const http = require('http')
const server = http.createServer((req, res) => { res.end('hi') })
server.listen(3000)
```

*Flux:*
```
use http
http.get "/" fn req res -> res.send "hi"
http.serve 3000
```

Achieved by: route registration is a direct call (not a class, not a builder pattern, not middleware chains). DB connection is one line. WebSocket is event-driven with three hooks.

The `use` statement doesn't need a string path for stdlib modules — just the name. Local files use `./` prefix. No `package.json`, no `go.mod`, no `requirements.txt`.

---

### 6. `try/catch` over result types

Flux uses `try/catch` rather than `Result<T, E>` or `Option<T>` types. Reasoning: result types require significant type system machinery (generics, monadic binding, etc.) which contradicts "learnable in one look." `try/catch` is universally understood and needs no explanation. The `!` suffix on potentially-failing calls (`db.query!`, `fs.read!`) acts as a visible warning without requiring the caller to handle it immediately.

**Tradeoff**: less safe than forced result types. But the `!` convention provides visibility, and `try/catch` is zero-boilerplate.

---

### 7. `lock/unlock` over async/await

For the concurrency story, `go` for spawning and `lock/unlock` for mutual exclusion. This is more like Go than JavaScript. Reasoning: `async/await` requires every function touching async state to be marked `async`, which propagates virally through the codebase. `go` + `lock` keeps the async boundary explicit only where it matters (the spawn site and the shared state access).

**Tradeoff**: more error-prone than async/await (easy to forget `unlock`). A future version could use `with lock(state)` block syntax to auto-unlock, but that adds syntax complexity.

---

### 8. Pipeline operator `|>`

`|>` allows left-to-right chaining without nesting or temp variables. `items |> filter(f) |> map(g)` reads naturally. The tradeoff is that it's a non-standard operator, but it appears in Elixir, F#, and is proposed for JS, so it has prior art and is intuitive.

---

## Token Efficiency Analysis

Flux eliminates these tokens vs typical languages:

| Eliminated token | Saves per use | Typical uses in project |
|-----------------|---------------|------------------------|
| `var`/`let`/`const` | 1 | ~30 |
| `return` | 1 | ~15 |
| `{` `}` block delimiters | 2 | ~40 |
| `(` `)` on calls | 2 | ~50 |
| `;` line terminators | 1 | ~60 |
| `string`/`int`/`bool` type names | 1-2 | ~20 |
| `for`/`while`/`forEach` unification | saves 2 keywords | — |

Rough estimate: **~250–300 tokens saved per 100-line program** vs equivalent TypeScript/Go.

---

## Approximate Token Counts (by word count proxy)

| File | Lines | ~Words | ~Tokens |
|------|-------|--------|---------|
| SPEC.md | ~200 | ~900 | ~1200 |
| project1-cli.fx | ~75 | ~220 | ~320 |
| project2-webapi.fx | ~80 | ~230 | ~340 |
| project3-realtime.fx | ~130 | ~360 | ~520 |
| NOTES.md | ~120 | ~700 | ~950 |

---

## What I Would Change

1. **Pattern matching** could be more powerful — add destructuring: `match {name, age} = user`
2. **String interpolation** — `"Hello {name}"` instead of `"Hello " + name` would save tokens
3. **Auto-unlock** — `with lock(x)` block for safer concurrency
4. **Type inference** — could eliminate sigils entirely with a Hindley-Milner system, but that's complex to spec in 3000 tokens
5. **Async I/O** — the current model is synchronous-looking but the runtime should handle I/O async under the hood (like Go's goroutines over I/O)
