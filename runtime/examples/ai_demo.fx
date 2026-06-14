# AI demo — LLM primitives (Anthropic Messages API).
#
# Before running, set $AI_KEY (OS env or in the .env file):
#   export AI_KEY=sk-ant-...
#   fluxon run examples/ai_demo.fx
#
# Model: $AI_MODEL ?? "claude-opus-4-8". For a different model:
#   export AI_MODEL=claude-sonnet-4-6
#
# NOTE: this example calls the real API (spends tokens). If there is no key
# it gives a clear error and does not reach the network.

use ai reg

# 1) ai.ask — a simple question, text answer.
answer = ai.ask "In one sentence: why is the Fluxon language good?"
log "ask: ${answer}"

# 2) ai.json — structured output. A schema map is given, the model returns
#    JSON that MATCHES it. The result also contains `_` metadata (conf/tokens/cost/ms).
r = ai.json "Parse this order: 3 apples, 2 loaves of bread" {
  products: [{name:str count:int}]
}
log "json result: ${r.products}"
log "confidence: ${r._.conf}  tokens: ${r._.tokens}  cost: ${r._.cost}  time(ms): ${r._.ms}"

# Decision based on confidence (spec pattern):
if r._.conf > 0.85
  log "high confidence -> accept automatically"
elif r._.conf >= 0.6
  log "medium confidence -> ask for confirmation"
else
  log "low confidence -> hand off to a human"

# 3) ai.run — ONE step of the tool-loop. The model does NOT run the tool itself:
#    the loop is yours (log/cost/confirmation control). You call the tool on the
#    Fluxon side via reg.call and add the result to msgs.

# Register the tool function (reg dynamic dispatch).
reg.add "weather" \args ->
  # In a real case this would do http.get; for the demo, a fixed answer.
  "25 degrees and sunny in ${args.city}"

# Tool definition: name, description, parameters (simple {name:type} -> JSON-schema).
tools = [{
  name: "weather"
  desc: "Current weather in the given city"
  params: {city:str}
}]

# Conversation history — first message.
msgs <- [{role::user content:"What is the weather in Tashkent?"}]

# Tool-loop: spin until the model returns :final (or hits the limit).
each i in 1..10
  r = ai.run msgs tools
  if r.kind == :final
    log "final answer: ${r.text}"
    ret r.text
  # r.kind == :call -> the model wants to call a tool, we run it on the Fluxon side.
  log "tool call: ${r.tool} args=${r.args}"
  result = reg.call r.tool r.args
  # Add the model reply (tool_use) and the tool result to the history.
  msgs <- msgs.push {role::assistant content:[{type:"tool_use" id:r.id name:r.tool input:r.args}]}
  msgs <- msgs.push {role::tool id:r.id content:result}
