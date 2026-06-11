# realtime.fluxon — WebSocket realtime layer: rooms, presence, broadcast,
# typing indicators, and the connect/message/disconnect lifecycle.
#
# ============================ SPEC GAP WARNING =============================
# The Fluxon spec does NOT define a websocket battery. It gives http (request/
# response only) and queue (queue.push / queue.on). I therefore INVENT a `ws`
# battery whose surface mirrors the http battery's style as closely as the spec
# allows. Every `ws.*` call below is improvised and is documented in the gaps
# section of the report. The shape assumed:
#
#   ws.on :connect    \conn -> ...   # new socket; conn.id is a stable id
#   ws.on :message    \conn msg -> ...   # msg is JSON-decoded to a map
#   ws.on :disconnect \conn -> ...
#   ws.send conn map                 # send one frame (map → JSON) to one socket
#   ws.serve 8081                    # start the ws server on a port
#
# For broadcast I do NOT assume a native "room" primitive (the spec has none).
# Instead I keep shared mutable state in module-level `<-` bindings and fan out
# by iterating the sockets in a room and calling ws.send on each. The queue
# battery is used to decouple persistence/broadcast so a slow socket can't block
# the sender.
# ==========================================================================
use db queue
use ./users as users
use ./channels as channels
use ./messages as messages

# NOTE: `ws` is referenced as a battery (ws.on / ws.send / ws.serve) but the
# spec defines no such module, so there is no `use ws` to write. See gaps.

# -------------------------------------------------------------------------
# Shared mutable state (single-process). Maps are treated as mutable
# containers reassigned via `<-`.
#
#   conns    : conn.id -> {conn:<conn> user:<user-id> rooms:[channel-id ...]}
#   rooms    : channel-id -> [conn.id ...]   (who is connected & joined)
#   presence : channel-id -> [user-id ...]   (distinct online users per room)
# -------------------------------------------------------------------------
conns    <- {}
rooms    <- {}
presence <- {}

# --- small map helpers (abstraction boundary for missing map verbs) ---
# SPEC GAP: the spec gives list methods (.push/.filter/.map) but defines NO map
# mutation verbs, NO map spread/merge, and NO computed/dynamic-key writes (only
# `m[k]` *reads* are shown). The shared `conns`/`rooms`/`presence` dictionaries
# need all three. These two helpers are the ONLY place that gap appears, so they
# are the single point to adapt to a real map API. I assume:
#   - a spread/merge map literal `{...m key:v}` (parallel to list building), and
#   - that `[k]:v` inside it writes a *computed* key taken from variable k.
# Both assumptions are invented; see the gaps report.

# Set key k (a runtime value) to value v in map m, returning a new map.
fn map_set m k v
  ret {...m [k]:v}

# Remove key k from map m, returning a new map.
fn map_del m k
  out <- {}
  each ek, ev in m
    if ek != k
      out <- {...out [ek]:ev}
  ret out

# -------------------------------------------------------------------------
# Room membership of live sockets.
# -------------------------------------------------------------------------

# Add a connection id to a channel room (live socket tracking).
fn room_add channel_id conn_id
  cur = rooms[channel_id] ?? []
  if cur.has conn_id
    ret nil
  rooms <- map_set rooms channel_id (cur.push conn_id)

# Remove a connection id from a channel room.
fn room_del channel_id conn_id
  cur = rooms[channel_id] ?? []
  rooms <- map_set rooms channel_id (cur.filter \c -> c != conn_id)

# -------------------------------------------------------------------------
# Presence: distinct online user ids per channel.
# -------------------------------------------------------------------------

# Recompute presence for a channel from the live sockets currently in its room.
fn recompute_presence channel_id
  conn_ids = rooms[channel_id] ?? []
  uids <- []
  each cid in conn_ids
    entry = conns[cid]
    if entry
      if !uids.has entry.user
        uids <- uids.push entry.user
  presence <- map_set presence channel_id uids
  ret uids

# Who is online in a channel (list of user ids).
exp fn presence_for channel_id
  ret presence[channel_id] ?? []

# -------------------------------------------------------------------------
# Broadcast. Send `payload` (a map) to every live socket in a channel room.
# `except` is an optional conn id to skip (e.g. the sender). We push the work
# onto the queue so the calling handler returns immediately and a slow client
# cannot stall the producer.
# -------------------------------------------------------------------------
fn broadcast channel_id payload except
  conn_ids = rooms[channel_id] ?? []
  each cid in conn_ids
    if cid != except
      queue.push "ws_send" {conn_id:cid payload:payload}

# The queue worker that actually writes a frame to a socket. Looking up the
# live conn handle by id keeps the queue job small/serializable.
queue.on "ws_send" \job ->
  entry = conns[job.conn_id]
  if entry
    ws.send entry.conn job.payload

# -------------------------------------------------------------------------
# Lifecycle: connect.
# -------------------------------------------------------------------------
ws.on :connect \conn ->
  conns <- map_set conns conn.id {conn:conn user:nil rooms:[]}
  ws.send conn {type:"hello" conn_id:conn.id need:"auth"}

# -------------------------------------------------------------------------
# Lifecycle: message. The client sends a JSON object whose `type` field selects
# the action. We dispatch on that string with if/elif (see note below).
#   {type:"auth"    username:..}
#   {type:"join"    channel:..}
#   {type:"leave"   channel:..}
#   {type:"say"     channel:.. body:..}
#   {type:"typing"  channel:..}
# -------------------------------------------------------------------------
ws.on :message \conn msg ->
  entry = conns[conn.id]
  if !entry
    ws.send conn {type:"error" error:"unknown connection"}
    ret nil
  # msg.type is a JSON string. The spec restricts `match` to symbols/numbers and
  # gives no string→symbol cast, so we dispatch with plain string comparison.
  kind = msg.type
  if kind == "auth"
    handle_auth conn entry msg
  elif kind == "join"
    handle_join conn entry msg
  elif kind == "leave"
    handle_leave conn entry msg
  elif kind == "say"
    handle_say conn entry msg
  elif kind == "typing"
    handle_typing conn entry msg
  else
    ws.send conn {type:"error" error:"unknown type ${msg.type}"}

# Authenticate this socket and mark the user :online.
fn handle_auth conn entry msg
  u = users.authenticate msg.username
  conns <- map_set conns conn.id {conn:conn user:u.id rooms:(entry.rooms)}
  users.set_status u.id :online
  ws.send conn {type:"authed" user:{id:u.id username:u.username}}

# Join a channel room: verify membership, register the live socket, broadcast a
# presence update, and send the joiner the current presence + recent history.
fn handle_join conn entry msg
  if !entry.user
    ws.send conn {type:"error" error:"auth required"}
    ret nil
  channel_id = msg.channel
  if !channels.is_member channel_id entry.user
    ws.send conn {type:"error" error:"not a member of ${channel_id}"}
    ret nil
  room_add channel_id conn.id
  cur = conns[conn.id]
  if !cur.rooms.has channel_id
    conns <- map_set conns conn.id (map_set cur "rooms" (cur.rooms.push channel_id))
  online = recompute_presence channel_id
  broadcast channel_id {type:"presence" channel:channel_id online:online} nil
  recent = messages.history channel_id nil 30
  ws.send conn {type:"joined" channel:channel_id online:online history:recent}

# Leave a channel room (socket stays connected).
fn handle_leave conn entry msg
  channel_id = msg.channel
  room_del channel_id conn.id
  cur = conns[conn.id]
  if cur
    conns <- map_set conns conn.id (map_set cur "rooms" (cur.rooms.filter \c -> c != channel_id))
  online = recompute_presence channel_id
  broadcast channel_id {type:"presence" channel:channel_id online:online} nil
  ws.send conn {type:"left" channel:channel_id}

# Send a message: persist (with AI moderation), then broadcast to the room.
fn handle_say conn entry msg
  if !entry.user
    ws.send conn {type:"error" error:"auth required"}
    ret nil
  channel_id = msg.channel
  if !channels.is_member channel_id entry.user
    ws.send conn {type:"error" error:"not a member"}
    ret nil
  res = messages.create_message channel_id entry.user msg.body
  if !res.ok
    # Blocked by moderation — tell only the sender.
    ws.send conn {type:"blocked" reason:res.moderation.reason label:res.moderation.label}
    ret nil
  row = res.message
  frame = {
    type:"message"
    channel:channel_id
    id:row.id
    user:entry.user
    body:row.body
    status:row.status
    created:row.created
  }
  # Acknowledge to the sender (with flag info), broadcast to everyone else.
  ws.send conn {...frame ack:true flagged:(res.flagged ?? false)}
  broadcast channel_id frame conn.id

# Typing indicator: ephemeral, never persisted. Broadcast to others only.
fn handle_typing conn entry msg
  if !entry.user
    ret nil
  channel_id = msg.channel
  broadcast channel_id {type:"typing" channel:channel_id user:entry.user} conn.id

# -------------------------------------------------------------------------
# Lifecycle: disconnect. Pull the socket out of every room it was in, mark the
# user :offline if they have no other live sockets, and update presence.
# -------------------------------------------------------------------------
ws.on :disconnect \conn ->
  entry = conns[conn.id]
  if !entry
    ret nil
  affected = entry.rooms ?? []
  each channel_id in affected
    room_del channel_id conn.id
  conns <- map_del conns conn.id

  # Is this user still connected on any other socket?
  uid = entry.user
  still_online <- false
  each cid, e in conns
    if e.user == uid
      still_online <- true
  if uid
    if !still_online
      users.set_status uid :offline

  # Refresh presence for every affected channel.
  each channel_id in affected
    online = recompute_presence channel_id
    broadcast channel_id {type:"presence" channel:channel_id online:online} nil

# -------------------------------------------------------------------------
# Broadcast a message that originated over REST (not a websocket) to the live
# sockets in the channel room, so realtime clients still see it instantly.
# -------------------------------------------------------------------------
exp fn broadcast_rest channel_id row
  frame = {
    type:"message"
    channel:channel_id
    id:row.id
    user:row.user
    body:row.body
    status:row.status
    created:row.created
  }
  broadcast channel_id frame nil

# -------------------------------------------------------------------------
# Start the websocket server. Called from main.
# -------------------------------------------------------------------------
exp fn serve port
  ws.serve port
