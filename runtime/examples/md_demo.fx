# tui.md demo — render Markdown in the terminal.
#
# Run it:
#     cargo run -- run examples/md_demo.fx
#
# Colors show on a real terminal (a tty). Piped/redirected output is clean text
# (no escapes) — try `cargo run -- run examples/md_demo.fx | cat` to see that.
# Force colors through a pipe with FORCE_COLOR=1 (for asciinema / `| less -R`).
#
# NOTE: Fluxon strings are single-line, so the sample below is built with `\n`
# escapes — exactly what `ai.ask`/`ai.run` hand you as one Markdown string.

sample = "# Fluxon Markdown Render\n\nFluxon makes **async** feel *synchronous*. Here are the key points:\n\n## Core ideas\n\n- Each request runs on its own `thread`\n- No *await* keyword needed\n- Shared state is frozen via [freeze_globals](https://docs.fluxon.dev)\n\n### Ordered steps\n\n1. Parse the request\n2. Run the handler\n   - inside a `db.tx`\n   - with **rollback** on error\n3. Return the response\n\n> The language adapts to the AI, not the AI to the language.\n\n```rust\nfn handler(req) {\n    db.tx(|| insert(req.body))\n}\n```\n\n---\n\nThat is all."

tui.print (tui.md sample)
