# 09 - fs battery (local file system primitives).
# Run: ./target/release/fluxon run tests-fx/09_fs.fx
#
# fs.read/write/append/exists/ls/del/mkdirp - all operate on the real file system.
# The test runs in a unique temporary directory (named with rand.str so parallel
# runs do not collide) and cleans up after itself.

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

# --- Unique working directory ---
base = "/tmp/fluxon_fs_${rand.str 8}"
r_mk = fs.mkdirp base
eq r_mk :ok "mkdirp -> :ok"
truthy (fs.exists base) "directory exists after mkdirp"

# --- mkdirp idempotent (:ok the second time too) ---
eq (fs.mkdirp base) :ok "mkdirp idempotent"

# --- write + read cycle ---
f = "${base}/a.txt"
eq (fs.write f "hello world") :ok "write -> :ok"
eq (fs.read f) "hello world" "read returns what was written"
truthy (fs.exists f) "file exists after write"

# --- Reading a missing file -> nil (not an error) ---
eq (fs.read "${base}/none.txt") nil "missing file read -> nil"
eq (fs.exists "${base}/none.txt") false "missing file exists -> false"

# --- append: adds to the end of an existing file ---
fs.append f " rest"
eq (fs.read f) "hello world rest" "append adds text to the end"

# --- append creates a new file ---
g = "${base}/log.txt"
fs.append g "x"
fs.append g "y"
eq (fs.read g) "xy" "append creates and accumulates a new file"

# --- ls: sorted list of names ---
names = fs.ls base
eq names.len 2 "ls 2 entries (a.txt, log.txt)"
eq names.0 "a.txt" "ls[0] = a.txt (sorted)"
eq names.1 "log.txt" "ls[1] = log.txt"

# --- del: deletes the file ---
eq (fs.del f) :ok "del file -> :ok"
eq (fs.exists f) false "file gone after del"

# --- Cleanup: delete the remaining files and the directory ---
fs.del g
fs.del base
eq (fs.exists base) false "del empty directory -> cleaned up"

# --- End ---
if fails == 0
  log "=== 09_fs: ALL PASSED ==="
else
  log "=== 09_fs: ${fails} TESTS FAILED ==="
