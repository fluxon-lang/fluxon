# Fluxon `.pkg` manifest format

A **battery-shaped module** is a reusable Fluxon module (`use ./lib/s3`) that
ships an optional sibling `.pkg` manifest. The manifest's mandatory `doc` block
is the micro-equivalent of a built-in battery's entry in `fluxon-agent.md`: a
short, canonical doc the AI agent reads **instead of** the implementation code.

This solves a *reuse + knowledge-packaging* problem (issue #202): write the code
once, and let the agent understand and use it correctly and cheaply. It is
deliberately **not** a package manager — there is no transitive resolution and
no version ranges.

## Location

The manifest lives next to the module file, with the `.pkg` extension:

```
lib/s3/s3.fx   →  lib/s3/s3.pkg     # package-style layout
lib/foo.fx     →  lib/foo.pkg       # single-file module
```

## Format

Line-oriented, with exactly two required keys. Lines starting with `#` are
comments; blank lines are ignored. **Unknown keys are an error** (so typos like
`nam` or `doc:` surface instead of being silently dropped).

```
name s3
doc """
  ### s3 (object storage upload)
  WHAT: upload a file to S3/R2 + presigned URL.
  CANONICAL:
    use ./lib/s3
    url = s3.upload "bucket" "key.png" bytes {content_type:"image/png"}!
  GOTCHAS:
    - content_type is required, else the browser downloads instead of rendering.
    - never put `../` in the key.
  DEPENDS: crypto http   # AWS Signature V4
  """
```

- **`name`** — the package name (one value after the keyword).
- **`doc`** — a triple-quoted block string. The opening `"""` must be alone on
  its line (content starts on the next line); the closing `"""` is alone on its
  own line. The block is **dedented** (the smallest common leading indentation is
  stripped), so you can indent the doc to taste.

### Doc conventions

`WHAT` / `CANONICAL` / `GOTCHAS` / `DEPENDS` are **conventions inside the
free-text doc**, not parsed keys — mirror a battery's entry in `fluxon-agent.md`:

- **WHAT** — one line: what the module does.
- **CANONICAL** — the one canonical way to use it. Reference exported names as
  `name.fn` (e.g. `s3.upload`). The runtime cross-checks these against the
  module's actual `exp`-orts (see below).
- **GOTCHAS** — the non-obvious correctness traps.
- **DEPENDS** — which **built-in batteries** the module uses (`crypto http`).
  A package may depend ONLY on built-in batteries — never on another package.
  This flat graph is what keeps Fluxon out of the npm/pip dependency-hell trap.

## Validation (load-time, soft)

When a module is loaded with `use ./...`, the runtime looks for a sibling
`.pkg`. The policy is intentionally lenient so existing modules keep working:

| Situation | Result |
|---|---|
| No `.pkg` sibling | Module loads (backward compatible) |
| `.pkg` present, valid doc | Module loads |
| `.pkg` present, **empty doc** | **Load fails** — the AI-doc is mandatory |
| `.pkg` present, **malformed** (e.g. unterminated `doc` block) | **Load fails** |
| `.pkg` present but **unreadable** (invalid UTF-8, a directory, permissions) | **Load fails** |
| `CANONICAL` references a name not `exp`-orted | **Warning** on stderr, still loads |

Only a genuine *file-not-found* is the backward-compatible no-manifest case;
any other read failure means a manifest is present but unusable and is surfaced.
The `CANONICAL` reference check resolves names against the manifest's own `name`
field (so a vendored `aws.fx` carrying `name s3` is checked against `s3.`).

The empty-doc and malformed cases are hard errors because a manifest that exists
on purpose but carries no usable doc defeats the entire point. The missing-`exp`
case is only a warning: the doc may legitimately mention a not-yet-implemented
form, and breaking the load over a doc typo would be too aggressive.

## How this differs from npm/pip

| | npm/pip | Fluxon `.pkg` modules |
|---|---|---|
| What the agent reads | code / long README | mandatory short canonical doc |
| Transitive deps | A→B→C→D hell | only batteries; package→package forbidden |
| Versioning | `^1.2` (nondeterministic) | exact pin / local vendoring |
| Quality | scattered | doc mandatory — no doc = invalid |
| Philosophy | many ways | `one task = one way` |

## Phases

This is **phase 2** of issue #202: the format plus load-time validation. The doc
is validated and dropped — it is not stored in-process. A later **phase 3** skill
scans the project for `.pkg` files and injects their `doc` blocks into the
agent's context (like `CLAUDE.md`), so the agent never reads the implementation.
That skill re-reads the plain `.pkg` file directly; the format never changes.
