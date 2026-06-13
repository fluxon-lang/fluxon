# 11 - sh battery (external shell commands).
# Run: ./target/release/fluxon run tests-fx/11_sh.fx
#
# sh.run cmd -> {stdout: str  stderr: str  code: int}. The command goes through the shell,
# so `&&` and pipes (|) work. These tests assume a Unix shell (sh) -
# CI runs on ubuntu+macOS.

fails <- 0

fn eq got want label
  if got == want
    log "ok  ${label} = ${got}"
  else
    log "FAIL ${label}: got=${got} want=${want}"
    fails <- fails + 1

fn truthy v label
  if v
    log "ok  ${label}"
  else
    log "FAIL ${label}: got=${v}"
    fails <- fails + 1

# --- Simple call: stdout + code ---
r = sh.run "printf hello"
eq r.stdout "hello" "printf stdout"
eq r.code 0 "success code 0"
eq r.stderr "" "stderr empty"

# --- Non-zero exit: NOT a Flow::err, signaled via code ---
bad = sh.run "exit 5"
eq bad.code 5 "exit 5 -> code 5"

# --- stderr captured separately (does not mix with stdout) ---
e = sh.run "printf error 1>&2"
eq e.stderr "error" "stderr captured"
eq e.stdout "" "stdout empty on error"

# --- Shell features: `&&` sequential commands ---
chain = sh.run "printf a && printf b"
eq chain.stdout "ab" "&& sequential commands"
eq chain.code 0 "&& code 0"

# --- Pipe (|) works ---
pipe = sh.run "printf 'one\ntwo\nthree' | wc -l"
truthy (str.int pipe.stdout >= 2) "pipe (wc -l) worked"

# --- End ---
if fails == 0
  log "=== 11_sh: ALL PASSED ==="
else
  log "=== 11_sh: ${fails} TESTS FAILED ==="
