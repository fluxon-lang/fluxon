-- Fluxon Live Chat Server (WebSocket)
-- Clients connect, join rooms, broadcast messages, see who's online
--
-- Protocol (JSON messages from client):
--   {type: "join",    room: "general", name: "alice"}
--   {type: "msg",     room: "general", text: "hello"}
--   {type: "leave",   room: "general"}
--   {type: "online",  room: "general"}   -- request member list
--
-- Server emits:
--   {type: "joined",   room, name, members: [...]}
--   {type: "msg",      room, from, text, ts}
--   {type: "left",     room, name}
--   {type: "online",   room, members: [...]}
--   {type: "error",    reason}

use ws
use json
use env
use time

let PORT = num.parse(env.get("PORT") or "4000")

-- Shared state: map of room -> list of {id, name, client}
let %rooms = {}

-- Helpers

fn get_room $room -> @
  if !rooms.has(room)
    rooms[room] = []
  rooms[room]

fn find_member $room ~client -> %
  @members = get_room(room)
  members.find(fn ~m -> m.client == client)

fn member_names $room -> @
  get_room(room) |> map(fn ~m -> m.name)

fn broadcast_room $room %msg $skip_id
  $payload = json.encode(msg)
  @members = get_room(room)
  each %m in members
    if m.id != skip_id
      m.client.send(payload)

fn send_err ~client $reason
  client.send(json.encode({type: "error", reason: reason}))

-- Generate a simple unique id from time + random
fn new_id -> $
  num.str(time.now()) + "-" + num.str(num.rand(1000, 9999))

-- Connection opened
ws.on "connect" fn ~client ->
  -- attach a fresh id to this client session
  client.meta = {id: new_id(), rooms: []}
  show "Client connected: " + client.meta.id

-- Message received
ws.on "message" fn ~client $raw ->
  try
    ~msg = json.parse!(raw)

    match msg.type
      "join" ->
        $room = msg.room or ""
        $name = msg.name or ""
        if room.len() == 0
          send_err(client, "room required")
          skip
        if name.len() == 0
          send_err(client, "name required")
          skip

        -- Check name not already taken in room
        @existing = get_room(room)
        ?taken = existing.has(fn ~m -> m.name == name)
        if taken
          send_err(client, "name taken in " + room)
          skip

        -- Register member
        %member = {id: client.meta.id, name: name, client: client}
        lock rooms
          get_room(room).push(member)
          client.meta.rooms.push(room)
        unlock rooms

        -- Confirm join to this client
        client.send(json.encode({
          type:    "joined",
          room:    room,
          name:    name,
          members: member_names(room)
        }))

        -- Notify others
        broadcast_room(room, {
          type: "joined",
          room: room,
          name: name,
          members: member_names(room)
        }, client.meta.id)

        show name + " joined #" + room

      "msg" ->
        $room = msg.room or ""
        $text = msg.text or ""
        if room.len() == 0
          send_err(client, "room required")
          skip
        if text.trim().len() == 0
          send_err(client, "empty message")
          skip

        -- Find sender's name
        %m = find_member(room, client)
        if m == nil
          send_err(client, "join room first")
          skip

        #ts = time.now()
        %envelope = {type: "msg", room: room, from: m.name, text: text, ts: ts}
        $payload = json.encode(envelope)

        -- Deliver to all in room (including sender)
        @members = get_room(room)
        each %mb in members
          mb.client.send(payload)

      "leave" ->
        $room = msg.room or ""
        %m = find_member(room, client)
        if m == nil
          skip

        lock rooms
          @kept = get_room(room).filter(fn ~mb -> mb.id != client.meta.id)
          rooms[room] = kept
          client.meta.rooms = client.meta.rooms.filter(fn ~r -> r != room)
        unlock rooms

        -- Notify others
        broadcast_room(room, {
          type:    "left",
          room:    room,
          name:    m.name,
          members: member_names(room)
        }, client.meta.id)

        show m.name + " left #" + room

      "online" ->
        $room = msg.room or ""
        if room.len() == 0
          send_err(client, "room required")
          skip
        client.send(json.encode({
          type:    "online",
          room:    room,
          members: member_names(room)
        }))

      _ ->
        send_err(client, "unknown type: " + msg.type)

  catch $err
    send_err(client, "bad message: " + err)

-- Disconnection: remove from all rooms
ws.on "disconnect" fn ~client ->
  $cid = client.meta.id
  @joined = client.meta.rooms

  each $room in joined
    %m = find_member(room, client)
    if m != nil
      lock rooms
        rooms[room] = get_room(room).filter(fn ~mb -> mb.id != cid)
      unlock rooms
      broadcast_room(room, {
        type:    "left",
        room:    room,
        name:    m.name,
        members: member_names(room)
      }, cid)
      show m.name + " disconnected from #" + room

  show "Client disconnected: " + cid

-- Start
show "Chat server on ws://localhost:" + num.str(PORT)
ws.serve(PORT)
