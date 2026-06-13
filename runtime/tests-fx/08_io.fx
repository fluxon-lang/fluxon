# 08 - io: terminal input/output (read_line, print, prompt).
# Input is fed via stdin: "Firdavs\n42\n" (run_all.sh pipes this).
# io.print/io.prompt write to stdout; the assertion log (stderr) goes separately -
# they do not mix.

fails <- 0

fn eq got want label
  if got == want
    log "ok  ${label} = ${got}"
  else
    log "FAIL ${label}: got=${got} want=${want}"
    fails <- fails + 1

# --- io.prompt: prints, then reads a single line ---
name = io.prompt "Your name: "
eq name "Firdavs" "prompt read 1st line"

# --- io.read_line: next line ---
age = io.read_line
eq age "42" "read_line 2nd line"

# --- io.read_line: nil at EOF ---
last = io.read_line
eq last nil "read_line EOF -> nil"

# --- io.print: returns nil (side effect to stdout) ---
result = io.print ""
eq result nil "print -> nil"

# --- End ---
if fails == 0
  log "=== 08_io: ALL PASSED ==="
else
  log "=== 08_io: ${fails} TESTS FAILED ==="
