# chat.fx — websocket live chat. Join rooms, broadcast, see who's online.
# Protocol: client sends JSON. Server replies JSON.
#   {t:"join", room:"x", name:"ada"}
#   {t:"msg",  text:"hi"}
# Server -> {t:"joined"|"msg"|"left"|"who"|"error", ...}
use list
use map
use str

# shared state: room name -> list of connections
rooms = {}

fn members room:
  ? map.has(rooms, room): ret rooms[room]
  rooms[room] = []
  ret rooms[room]

fn send c obj:
  c.send(@json.enc(obj))

fn bcast room obj:
  @@ peer in members(room):
    send(peer, obj)

fn names room:
  out = []
  @@ peer in members(room):
    list.push(out, peer.data.name)
  ret out

ws = @ws()

ws.on("open", \c:
  c.data.room = nil
  c.data.name = nil)

ws.on("message", \c, m:
  ?!:
    msg = @json.dec(m)
  |! e:
    send(c, {t: "error", error: "bad json"})
    ret nil

  ? msg.t == "join":
    ? msg.room == nil or msg.name == nil:
      ret send(c, {t: "error", error: "room and name required"})
    c.data.room = msg.room
    c.data.name = msg.name
    list.push(members(msg.room), c)
    send(c, {t: "joined", room: msg.room, who: names(msg.room)})
    bcast(msg.room, {t: "who", who: names(msg.room)})

  | msg.t == "msg":
    ? c.data.room == nil:
      ret send(c, {t: "error", error: "join a room first"})
    bcast(c.data.room, {t: "msg", from: c.data.name, text: msg.text, ts: @now()})

  |:
    send(c, {t: "error", error: "unknown type"}))

ws.on("close", \c:
  room = c.data.room
  ? room == nil: ret nil
  ppl = members(room)
  @@ i, peer in ppl:
    ? peer.id == c.id:
      list.del(ppl, i)
      stop
  bcast(room, {t: "left", name: c.data.name, who: names(room)}))

say "chat ws on :9000"
ws.run(9000)
