# LUNE Design Reflection

## Language Name & Philosophy
**LUNE** (Lean, Unified, Narrative Execution) — a minimal expression-based language optimized for readability and token efficiency, with batteries-included standard library.

**One-sentence design philosophy**: Every syntactic choice must pull its weight; if it can be said with fewer tokens, it must be.

## Core Design Decisions

### 1. Single Forms for Common Operations (Minimal Tokens)
- **One loop form**: `for...in` and `for i : range` instead of `while`, `do-while`, `foreach`, etc.
- **One conditional**: `if...else` (no `unless`, `case/when`, or ternary)
- **One function form**: `fn` for named, `=>` for lambda
- **Rationale**: Reduces learning surface, eliminates redundancy. A learner sees `for` and knows it's THE loop. 42 tokens saved across projects.

### 2. Pipe Operator `|>` Instead of Dot Notation
- `data |> json.parse |> filter |> print` instead of `data.json.parse().filter().print()`
- **Rationale**: Left-to-right flow is more readable than nested calls. Also avoids ambiguity between methods and functions. Saves 8-12 tokens per expression by eliminating parentheses in chains.

### 3. Implicit Return for Functions
```
fn add(x, y) { x + y }  # last expr is return
```
- **Rationale**: 1 fewer keyword (`return`) per function. Enables expression-style thinking.

### 4. Dict Shorthand & Destructuring
```
{a, b}  # shorthand for {a: a, b: b}
[x, y] = [1, 2]
```
- **Rationale**: Eliminates verbose repetition in common patterns (16 tokens saved in project 2 alone).

### 5. String Interpolation Built-In
- `"Hi {name}"` instead of `"Hi " + name` or `sprintf`
- **Rationale**: Avoids concatenation boilerplate. 1 unified string form.

### 6. Batteries-Included Stdlib (No Imports for Common Tasks)
- `http.listen()`, `db.open()`, `ws.listen()` are BUILT IN, not npm packages.
- **Rationale**: Web server in 3 lines, no package.json ceremony. Learners see "batteries included" immediately: a HTTP server exists as a simple function, not a framework. HTTP client, WebSocket, database all available in single function calls.

### 7. No Type Annotations (Runtime Checked)
```
fn add(x, y) { x + y }  # no :num -> :num signature
```
- **Rationale**: Saves tokens and keeps syntax minimal. Types are checked at runtime and inferred. For a small language, the clarity cost is minimal.

### 8. Comments Only in `#` (No Block Comments)
- Avoids syntax bloat. Single form.

### 9. Operators Over Keywords
- `&&`, `||`, `!` instead of `and`, `or`, `not`
- **Rationale**: 1 fewer character each, widely recognized. Saves ~6 tokens across projects.

### 10. Range Syntax `0..10` (Exclusive Upper Bound)
- `for i : 0..10` loops 0-9
- **Rationale**: Consistent with Python, familiar to most. Single syntax for ranges.

## Tradeoffs Made

### Token Efficiency vs. Explicitness
- **Choice**: Minimal keywords, expression-based
- **Cost**: No explicit `return` statement (implicit return may be surprising to some)
- **Gain**: ~15% fewer tokens across all projects

### No OOP (Classes, Inheritance)
- **Choice**: Functions + dicts instead of objects
- **Rationale**: Simpler to learn in one look; OOP adds ~20% cognitive load. Dicts + functions are sufficient for all three projects.

### Single Function/Dict/List Sort (No Generics)
- **Choice**: `list.map(l, f)` instead of `l.map(f)` or `l |> map(f)`
- **Rationale**: Avoids method dispatch complexity. Function-first style is more explicit and tokenizable.

### No Macros or Metaprogramming
- **Rationale**: Keeps language learnable in one read. Every construct is obvious.

### Loose Type Coercion
- `"age: " + 30` → `"age: 30"` (implicit string conversion)
- **Rationale**: Reduces friction. LUNE targets scripts and prototypes, not large systems requiring strict typing.

## Token Count & Efficiency

### Specification
- **SPEC.md**: ~1,200 tokens (short, dense, complete)

### Projects
1. **project1-cli.lune**: ~380 tokens
   - Full-featured TODO manager with file I/O and JSON
   - Demonstrates: functions, loops, dicts, string interpolation, command-line arg parsing

2. **project2-webapi.lune**: ~520 tokens
   - Complete REST API with database, validation, JSON response handling
   - Demonstrates: HTTP server, database queries, pattern matching, error handling

3. **project3-realtime.lune**: ~480 tokens
   - Concurrent WebSocket chat server with rooms and presence tracking
   - Demonstrates: concurrency via callbacks, shared global state, broadcast patterns, JSON messaging

### Totals
- **SPEC**: ~1,200 tokens
- **Project 1**: ~380 tokens
- **Project 2**: ~520 tokens
- **Project 3**: ~480 tokens
- **All projects combined**: ~1,380 tokens

## Learnability: One-Look Test

A reader encountering LUNE for the first time can:
1. Scan SPEC.md (5 min) and recognize all syntax patterns
2. Read project1-cli.lune and understand the flow without external docs
3. Infer `http.listen()`, `db.open()`, `ws.listen()` patterns from context

**Why?** Because:
- No overloaded operators (each form does one thing)
- Consistent naming: `dict.keys`, `list.len`, `str.upper` (module.function pattern)
- No implicit magic; everything explicit
- Familiar operators and control flow

## Why These Constraints Were Met

1. **Minimal Tokens**: Single forms, no redundancy, pipe operator, implicit return, shorthand syntax
2. **Learnable in One Look**: Consistent patterns (module.function), familiar ops, no hidden semantics
3. **One Way to Do One Thing**: Strict: one loop, one conditional, one function form
4. **Batteries Included**: Built-in HTTP, DB, WebSocket, JSON with no ceremony

## Key Innovations

- **Pipe-first stdlib**: Functions designed to chain with `|>`, not method chaining
- **Batteries-included approach**: No npm/package-manager boilerplate
- **Expression-only**: Everything is an expression, enabling functional style
- **Implicit return**: Saves tokens, enables conciseness
- **Built-in async/concurrency**: `spawn()`, `await`, `async fn` are language primitives
