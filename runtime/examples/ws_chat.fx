# WS battery namoyishi — minimal realtime chat (echo + xona broadcast).
# Ishga tushirish:  fluxon run examples/ws_chat.fx
# Sinash (boshqa terminalda, websocat kerak):
#   websocat ws://localhost:9000
#   > {"t":"join","room":"general","name":"firdavs"}
#   > {"t":"say","room":"general","body":"salom"}     # xonadagi hammaga ketadi
# Ikkita websocat oynasini ochib, bir oynadan say qiling — ikkinchisida ko'rinadi.

use ws

# Yangi ulanish — kutib olamiz va ismni hali noma'lum deb belgilaymiz.
ws.on :connect \conn ->
  ws.data.set conn :name "anon"
  ws.send conn (json.enc {t:"hello" id:conn.id})

# Kelgan xabar — JSON, `t` maydoni amalni tanlaydi.
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
    ws.send conn (json.enc {t:"error" reason:"noma'lum amal: ${m.t}"})

# Ulanish uzildi — Fluxon xona a'zoligini avtomat tozalaydi, biz faqat loglaymiz.
ws.on :disconnect \conn ->
  log "uzildi: ${conn.id}"

log "ws chat 9000-portda..."
ws.serve 9000
