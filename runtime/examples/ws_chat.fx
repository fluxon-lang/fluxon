# WS battery demonstration — minimal realtime chat (echo + room broadcast).
# Running:  fluxon run examples/ws_chat.fx
# Testing (in another terminal, websocat needed):
#   websocat ws://localhost:9000
#   > {"t":"join","room":"general","name":"firdavs"}
#   > {"t":"say","room":"general","body":"hello"}     # goes to everyone in the room
# Open two websocat windows, say from one — it appears in the other.

use ws

# New connection — we greet it and mark the name as still unknown.
ws.on :connect \conn ->
  ws.data.set conn :name "anon"
  ws.send conn (json.enc {t:"hello" id:conn.id})

# Incoming message — JSON, the `t` field selects the action.
ws.on :message \conn raw ->
  m = json.dec raw
  if m.t == "join"
    ws.data.set conn :name m.name
    ws.room.join conn m.room
    who = ws.room.members m.room
    ws.room.send m.room (json.enc {t:"joined" name:m.name online:who.len})
  elif m.t == "say"
    name = ws.data.get conn :name
    ws.room.send m.room (json.enc {t:"msg" from:name body:m.body})
  elif m.t == "leave"
    ws.room.leave conn m.room
  else
    ws.send conn (json.enc {t:"error" reason:"unknown action: ${m.t}"})

# Connection dropped — Fluxon cleans up room membership automatically, we just log.
ws.on :disconnect \conn ->
  log "disconnected: ${conn.id}"

log "ws chat on port 9000..."
ws.serve 9000
