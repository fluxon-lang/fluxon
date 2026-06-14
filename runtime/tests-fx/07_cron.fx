# 07 - cron battery (scheduled background tasks).
# Run: ./target/release/fluxon run tests-fx/07_cron.fx
#
# cron - standard Unix 5-field cron expression (unquoted). `cron.on` registers
# (does not block). This test checks PARSE + REGISTRATION correctness
# (unquoted/quoted/lambda/complex expression). Task EXECUTION is time-dependent -
# that is checked by a native test (cron_mod) and a manual cron_demo.fx smoke test.

fails <- 0
fn ok label -> log "ok  ${label}"
fn bad label
  log "FAIL ${label}"
  fails <- fails + 1

fn job
  log "job done"

# --- Unquoted 5-field + named function ---
# `*` here is NOT multiplication - the parser recognizes the cron expr and collects it into a str.
# cron.on returns nil; if there is no error the registration succeeded.
r1 = cron.on 0 * * * * job
if r1 == nil
  ok "cron.on unquoted 5-field"
else
  bad "cron.on unquoted got=${r1}"

# --- Complex expression: step / list / range mixed ---
r2 = cron.on */15 9 1,15 * 1-5 job
if r2 == nil
  ok "cron.on complex expression (*/15 9 1,15 * 1-5)"
else
  bad "cron.on complex got=${r2}"

# --- Inline lambda (no parameters) ---
r3 = cron.on 30 9 * * * \->
  log "lambda job"
if r3 == nil
  ok "cron.on inline lambda"
else
  bad "cron.on lambda got=${r3}"

# --- Quoted variant (human convenience; not in the AI docs) ---
r4 = cron.on "0 0 * * 0" job
if r4 == nil
  ok "cron.on quoted variant"
else
  bad "cron.on quoted got=${r4}"

# --- End ---
if fails == 0
  log "=== 07_cron: ALL PASSED ==="
else
  log "=== 07_cron: ${fails} TESTS FAILED ==="
