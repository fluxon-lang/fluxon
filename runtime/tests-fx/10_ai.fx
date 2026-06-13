# 10 - ai battery (LLM primitive). NETWORK-FREE test: we do not make real API calls
# (saves tokens + no key in CI). We only test these:
#   - if the `ai` module name is SHADOWED by a variable, it reads as a plain map
#   - the FLUXON SIDE of the tool-loop (running a tool via reg.call) works
#
# A real call to ai.ask/ai.json/ai.run requires $AI_KEY and goes over the network -
# that is tested manually by examples/ai_demo.fx (with a key).

use reg

fails <- 0
fn ok label -> log "ok  ${label}"
fn bad label
  log "FAIL ${label}"
  fails <- fails + 1

# --- shadowing: if `ai` is a variable, it is not the module ---
# ai.ask "..." (with args) checks lookup in eval_call: if it is a variable it does
# not dispatch. ai.ask without args is a Field - read straight from the map.
ai = {ask:"shadowed" model:"none"}
if ai.ask == "shadowed"
  ok "ai shadow: ai.ask read from map field"
else
  bad "ai shadow broke got=${ai.ask}"

if ai.model == "none"
  ok "ai shadow: ai.model = ${ai.model}"
else
  bad "ai.model got=${ai.model}"

# --- tool-loop FLUXON side: simulate the ai.run :call step ---
# ai.run with a model returns {kind::call tool args id}. The loop runs the tool via
# reg.call and appends the result to msgs. Here we craft the model response BY HAND
# and test the reg.call + msgs.push logic (network-free).

reg.add "weather" \args -> "${args.city} is 25 degrees"

# Assume the model "called a tool" (ai.run returns this kind of map):
step = {kind::call tool:"weather" args:{city:"Tashkent"} id:"toolu_1"}

# we declare result outside - `=` is transparent in an if block (updates the outer),
# but if it already exists it is also visible below.
result <- ""
if step.kind == :call
  result <- reg.call step.tool step.args
  if result == "Tashkent is 25 degrees"
    ok "tool-loop: reg.call step result = ${result}"
  else
    bad "tool-loop result got=${result}"
else
  bad "step.kind != :call"

# append the tool result to msgs (grow the conversation history)
msgs <- [{role::user content:"weather?"}]
msgs <- msgs.push {role::tool id:step.id content:result}
if msgs.len == 2 & msgs.1.role == :tool
  ok "tool-loop: msgs history grew (${msgs.len} messages)"
else
  bad "msgs history got len=${msgs.len}"

# --- final step shape ---
final = {kind::final text:"answer ready"}
if final.kind == :final & final.text == "answer ready"
  ok "ai.run :final shape is correct"
else
  bad ":final shape broke"

# --- End ---
if fails == 0
  log "=== 10_ai: ALL PASSED ==="
else
  log "=== 10_ai: ${fails} TESTS FAILED ==="
