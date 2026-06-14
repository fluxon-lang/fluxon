# io demo — interactive terminal: prompt/read/output without a newline.
#
# `log` always appends `\n` to stderr; for an interactive CLI (REPL, agent, wizard)
# you need `io`:
#   io.prompt msg   — prints msg WITHOUT \n, reads a single line -> str
#   io.read_line    — a single line from stdin -> str (EOF -> nil)
#   io.print s      — print WITHOUT \n (to build a prompt)
#
# Running:        fluxon run examples/io_repl.fx
# Testing (pipe): printf 'Aziza\n5\n' | fluxon run examples/io_repl.fx

# --- One-shot wizard: ask for name + age ---
name = io.prompt "Your name: "
io.print "Hello, "
io.print name
io.print "!\n"

age = io.prompt "Your age: "
log "So you are ${age} years old."

# --- REPL loop: infinite loop `each i in inf` (i = 0,1,2,...) ---
# No `while`/`for`; `inf` is only meaningful as an `each` iterator.
# On EOF (nil) or typing "exit" we leave with `stop`.
log "--- echo REPL (Ctrl-D or 'exit' to quit) ---"
each i in inf
  line = io.prompt "echo> "
  if line == nil
    log "bye!"
    stop
  if line == "exit"
    log "bye!"
    stop
  log "you: ${line}"
