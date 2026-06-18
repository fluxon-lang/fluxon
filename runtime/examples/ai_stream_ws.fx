# AI streaming over WebSocket (issue #201).
#
# A realtime chat where the AI answer is relayed to the browser TOKEN BY TOKEN:
# the `ai.stream` callback `ws.send`s each chunk to the asking connection as it
# arrives, so the client renders the reply progressively (a chat-stream UX) — it
# never waits for the whole answer.
#
# Running:  export ANTHROPIC_API_KEY=sk-ant-...  &&  fluxon run examples/ai_stream_ws.fx
# Testing (websocat in another terminal):
#   websocat ws://localhost:9001
#   > explain async in one sentence
#   < {"t":"chunk","text":"Async ..."}     (many chunk messages, in order)
#   < {"t":"done"}
#
# NOTE: `ai.stream` runs on the handler thread and blocks IT until the answer
# finishes — but only that one connection's handler; other connections are
# unaffected (each runs on its own thread, the runtime invariant).

use ws ai

ws.on :connect \conn ->
  ws.send conn (json.enc {t:"ready"})

ws.on :message \conn raw ->
  # Each text frame is treated as a prompt. Stream the answer back as it streams
  # in: one ws message per chunk, then a final {t:"done"}.
  ai.stream raw \chunk ->
    ws.send conn (json.enc {t:"chunk" text:chunk})
  ws.send conn (json.enc {t:"done"})

ws.on :disconnect \conn ->
  log "disconnected: ${conn.id}"

log "AI streaming chat on port 9001..."
ws.serve 9001
