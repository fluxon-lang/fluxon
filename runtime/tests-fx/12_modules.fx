# 12 - User modules: `use ./file`, `as alias`, exports, closures,
# nested import, cache (issue #45). Paths are relative to this file's directory.

fails <- 0

fn eq got want label
  if got == want
    log "ok  ${label} = ${got}"
  else
    log "FAIL ${label}: got=${got} want=${want}"
    fails <- fails + 1

# --- Basic import: exp value and function ---
use ./mod_math
eq mod_math.pi 3 "exp value"
eq (mod_math.add 2 5) 7 "exp function"

# --- Closure: a module fn accesses the module-level private `base` ---
eq (mod_math.from_base 5) 105 "module closure"

# --- A module-private name is not in the namespace ---
eq mod_math.base nil "private name hidden"

# --- as alias ---
use ./mod_math as m
eq (m.add 10 1) 11 "alias function"

# --- Nested import: mod_nested -> mod_math ---
use ./mod_nested
eq (mod_nested.double 21) 42 "nested import"

# --- Cache: mod_math used twice, same namespace ---
eq mod_math.pi m.pi "cache - same value"

# --- par + module import (issue #137 PR review): par lambdas import a module
# on separate threads. Since module_loading/current_base is thread-local,
# parallel import does not give a false "cyclic import" and base is passed
# through correctly (including a nested-dir module). Both lambdas return {ok:...}. ---
fn par_load n
  use ./mod_math
  ret mod_math.add n 1
prl = par [(\-> par_load 10) (\-> par_load 20)]
eq prl.0.ok 11 "par module import 1"
eq prl.1.ok 21 "par module import 2"

# --- End ---
if fails == 0
  log "=== 12_modules: ALL PASSED ==="
else
  log "=== 12_modules: ${fails} TESTS FAILED ==="
