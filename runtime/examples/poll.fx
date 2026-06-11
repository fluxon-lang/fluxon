# Jonli ovoz berish (Strawpoll) — REST + realtime BIR jarayonda.
#
# HTTP'da ovoz qabul qilamiz, natijani WS orqali barcha ulangan klientlarga
# JONLI broadcast qilamiz. http.serve va ws.serve birga e'lon qilinadi — ikkalasi
# bitta umumiy event-loopda ishlaydi, shuning uchun HTTP handler ichidan
# ws.room.send chaqirish mumkin (cross-protocol).
#
# Ishga: fluxon run examples/poll.fx
#   ovoz:   curl -X POST localhost:8080/vote -d '{"opt":"ha"}'
#   natija: ws://localhost:9000 ga ulanib jonli yangilanishni ko'ring.
#
# Ovozlar DB'da saqlanadi (global muzlatilgani uchun mutable holat db'da yashaydi).

use http db

tbl votes
  id  serial pk
  opt str
  ts  now

# WS klient ulanganda "live" xonasiga qo'shamiz — barcha broadcast shu yerga.
ws.on :connect \conn ->
  ws.room.join conn "live"

# HTTP: ovoz qabul qilamiz, db'ga yozamiz, jonli natijani WS'ga broadcast qilamiz.
http.on :post "/vote" \req ->
  opt = req.body.opt ?? "noma'lum"
  db.ins "votes" {opt: opt}
  # Joriy sanoqni o'qib, barcha WS klientlarga jonli yuboramiz.
  rows = db.q "select opt, count(*) c from votes group by opt"
  ws.room.send "live" (json.enc {t: "tally" rows: rows})
  rep 201 {ok: true}

# HTTP: joriy natija (REST orqali ham o'qish mumkin).
http.on :get "/tally" \req ->
  rep 200 (db.q "select opt, count(*) c from votes group by opt")

http.serve 8080
ws.serve 9000
