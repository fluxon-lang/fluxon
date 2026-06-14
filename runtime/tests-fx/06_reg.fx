# 06 - reg battery (function registry, dynamic dispatch).
# Run: ./target/release/fluxon run tests-fx/06_reg.fx
#
# reg - store/call a function by its STRING name. Main use:
# AI agent tool-loops (the model picks a tool name, the code runs it via reg.call).

fails <- 0
fn ok label -> log "ok  ${label}"
fn bad label
  log "FAIL ${label}"
  fails <- fails + 1

# --- reg.add + reg.call: store by name and call ---
# the closure takes args (a map) - agent tool arguments arrive in this shape.
reg.add "calc" \args -> args.a + args.b
out = reg.call "calc" {a:2 b:3}
if out == 5
  ok "reg.call calc = ${out}"
else
  bad "reg.call calc got=${out}"

# string result (interpolation inside the closure)
reg.add "greet" \args -> "hello ${args.name}"
g = reg.call "greet" {name:"Aziza"}
if g == "hello Aziza"
  ok "reg.call greet = ${g}"
else
  bad "reg.call greet got=${g}"

# --- reg.has: is the name in the registry (bool) ---
if reg.has "calc"
  ok "reg.has calc = true"
else
  bad "reg.has calc was false"

if (reg.has "nope") == false
  ok "reg.has nope = false"
else
  bad "reg.has nope was true"

# --- reg.names: names in the registry (alphabetical, stable) ---
ns = reg.names
if ns.len == 2 & ns.0 == "calc" & ns.1 == "greet"
  ok "reg.names = ${ns}"
else
  bad "reg.names got=${ns}"

# --- reg.add overwrites (tool-update case) ---
reg.add "calc" \args -> args.a * args.b
out2 = reg.call "calc" {a:4 b:5}
if out2 == 20
  ok "reg.add overwrote: calc = ${out2}"
else
  bad "reg.add did not overwrite got=${out2}"

# the name count is unchanged (overwriting does not add a new entry)
if reg.names.len == 2
  ok "reg.names still 2 after overwrite"
else
  bad "reg.names ${reg.names.len} after overwrite"

# --- End ---
if fails == 0
  log "=== 06_reg: ALL PASSED ==="
else
  log "=== 06_reg: ${fails} TESTS FAILED ==="
