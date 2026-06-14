# sh battery demo — running external shell commands.
# Run: cargo run -- run examples/sh_demo.fx
#
# Non-blocking (not a server) — also usable as a smoke-test.

# sh.run cmd -> {stdout: str  stderr: str  code: int}.
# The command goes through the shell, so `&&`, pipe (|), glob work.

# Simple call: on success code == 0.
r = sh.run "echo hello world"
log "stdout:" r.stdout
log "code:" r.code

# Checking success — the code == 0 convention.
g = sh.run "git --version"
if g.code == 0
  log "git present:" g.stdout
else
  log "git not found:" g.stderr

# Shell features: sequential commands and pipe.
files = sh.run "ls /tmp | head -3"
log "files:\n${files.stdout}"

# A failing command is NOT Flow::err — you tell from the code.
bad = sh.run "exit 2"
log "failing command code:" bad.code

# stderr is captured separately.
err = sh.run "ls /no-such-dir-for-sure"
log "error stream:" err.stderr
log "error code:" err.code
