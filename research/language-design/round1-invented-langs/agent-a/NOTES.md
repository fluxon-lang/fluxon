# Flux — design notes

## Goal
A language an AI can learn in one look, that writes web/DB/realtime apps with
near-zero boilerplate, and whose own syntax burns as few tokens as possible.

## Key decisions and why

**One sigil for the stdlib (`@`).** Every batteries-included capability lives
under a single global object: `@web`, `@db`, `@ws`, `@json`, `@fs`, `@env`,
`@args`, `@now`, `@uid`. This means zero imports for the things that matter most,
and the `@` prefix makes "this is runtime magic" visually obvious. One namespace
to remember instead of dozens of import paths.

**Picked exactly one of each redundant construct.** One print (`say`), one loop
(`@@`), one conditional (`?`/`|`), one assignment (`=`). No `while`+`for`+`foreach`,
no `print`/`println`/`printf`. `@@` unifies range / list / map / while by what
follows it. This kills the "which form do I use?" decision and shrinks the spec.

**Symbols for the highest-frequency keywords.** `?` (if), `|` (else), `@@` (loop),
`\` (lambda), `!` (raise), `?!`/`|!` (try/catch). High-frequency tokens get the
shortest spelling — classic information-theory argument: frequent symbols should
be short. Lower-frequency things (`fn`, `use`, `ret`, `stop`, `skip`) stay as short
words because clarity matters more than length when they appear rarely.

**Indentation blocks with a leading `:`.** No `{}`/`end` pairs to balance, no
closing tokens at all. A block costs one `:` and the newlines you already pay for.
This removes a whole class of tokens versus brace languages.

**String interpolation built in (`$x`, `${expr}`).** Almost every real program
formats strings; making it a first-class, one-character feature avoids `+`-concat
chains and `format(...)` calls everywhere.

**Handlers are just lambdas returning a map.** A web route returns
`{status, json|text}`; a ws handler mutates `c.data` and calls `c.send`. No
framework classes, decorators, or response-builder objects. The response *shape*
is the API. This is what makes "web server in ~5 lines" real.

**Auto-export everything top-level.** No `export`/`pub` keyword — `use` just
exposes a file's top-level names under an alias. One less concept.

## Tradeoffs (terseness vs clarity)
- `?`/`|` for if/else is the riskiest call: it's very terse but `|:` as "else"
  takes a beat to learn. I judged it worth it because conditionals are everywhere,
  and the `?`/`|`/`|cond:`/`|:` ladder is regular (always: branch, more branches,
  default). One look, one rule.
- Reused `?!`/`|!` to mirror `?`/`|` so try/catch rhymes with if/else instead of
  being a separate vocabulary. Pattern reuse lowers the learning cost more than a
  fresh keyword would.
- Kept real words for `fn`, `use`, `ret`, `and`, `or`, `not`, `in`, `stop`,
  `skip`. Going full-symbol (e.g. APL-style) would cut tokens but break the
  one-look-readability constraint, which I weighted higher than raw minimalism.
- Single mutable binding (`=`, no `let`/`const`/`var`) trades compile-time safety
  for spec size. Acceptable for a scripting language.

## What I'd add next
Pattern matching (could fold into `?`), typed annotations as optional sugar, and
a `@kv` cache battery. None were needed for the three projects, so they stayed out
to keep the spec under one screen.
