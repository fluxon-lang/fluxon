# Live voting (Strawpoll) — REST + realtime in ONE process.
#
# We accept votes over HTTP and broadcast the result LIVE over WS to all
# connected clients. http.serve and ws.serve are declared together — both
# run in one shared event loop, so you can call ws.room.send from inside an
# HTTP handler (cross-protocol).
#
# Run: fluxon run examples/poll.fx
#   vote:   curl -X POST localhost:8080/vote -d '{"opt":"yes"}'
#   result: connect to ws://localhost:9000 to see live updates.
#
# Votes are stored in the DB (since globals are frozen, mutable state lives in the db).

use http db

tbl votes
  id  serial pk
  opt str
  ts  now

# When a WS client connects we add it to the "live" room — all broadcasts go here.
ws.on :connect \conn ->
  ws.room.join conn "live"

# HTTP: accept a vote, write to db, broadcast the live result over WS.
http.on :post "/vote" \req ->
  opt = req.body.opt ?? "unknown"
  db.ins "votes" {opt: opt}
  # Read the current tally and send it live to all WS clients.
  rows = db.q "select opt, count(*) c from votes group by opt"
  ws.room.send "live" (json.enc {t: "tally" rows: rows})
  rep 201 {ok: true}

# HTTP: current result (also readable over REST).
http.on :get "/tally" \req ->
  rep 200 (db.q "select opt, count(*) c from votes group by opt")

http.serve 8080
ws.serve 9000
