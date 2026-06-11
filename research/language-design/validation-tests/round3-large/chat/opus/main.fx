# main.fluxon — wires the HTTP REST server, the WebSocket realtime server, and
# the cron jobs together. This is the entry point.
#
# Module aliasing note: our local files `cron.fluxon` and `moderation.fluxon` would
# otherwise be fine, but `cron` is also a battery name, so the local file is
# imported `as jobs` per the spec's collision rule.
use http
use ./schema                       # registers tbl declarations
use ./users as users
use ./channels as channels
use ./messages as messages
use ./moderation as mod
use ./realtime as rt
use ./cron as jobs

# ------------------------------------------------------------------
# Helpers
# ------------------------------------------------------------------

# Pull the acting user id from a header (stand-in for real auth/session).
fn actor req
  uid = req.headers.x_user_id
  uid ?? (fail "missing X-User-Id header")
  ret str.int uid

# ------------------------------------------------------------------
# Users
# ------------------------------------------------------------------

# Create a user.
http.on :post "/users" \req ->
  u = users.create_user req.body.username req.body.email
  rep 201 u

# Get a user.
http.on :get "/users/:id" \req ->
  u = users.get_user (str.int req.params.id)
  u ?? (rep 404 {error:"not found"})
  rep 200 u

# ------------------------------------------------------------------
# Channels
# ------------------------------------------------------------------

# Create a channel (creator = X-User-Id).
http.on :post "/channels" \req ->
  uid = actor req
  ch = channels.create_channel req.body.name req.body.is_private uid
  rep 201 ch

# List channels for a user.
http.on :get "/users/:id/channels" \req ->
  rep 200 {channels:(channels.channels_for_user (str.int req.params.id))}

# Join a channel.
http.on :post "/channels/:id/join" \req ->
  uid = actor req
  rep 200 (channels.join_channel (str.int req.params.id) uid)

# Leave a channel.
http.on :post "/channels/:id/leave" \req ->
  uid = actor req
  rep 200 (channels.leave_channel (str.int req.params.id) uid)

# Who is online right now in a channel (from the realtime presence table).
http.on :get "/channels/:id/presence" \req ->
  cid = str.int req.params.id
  rep 200 {channel:cid online:(rt.presence_for cid)}

# ------------------------------------------------------------------
# Messages
# ------------------------------------------------------------------

# Paginated message history: ?before=<msg id> ?limit=<n>.
http.on :get "/channels/:id/messages" \req ->
  cid = str.int req.params.id
  before <- nil
  if req.query.before
    before <- str.int req.query.before
  limit <- nil
  if req.query.limit
    limit <- str.int req.query.limit
  rep 200 {messages:(messages.history cid before limit)}

# Post a message over REST (also goes through AI moderation). On success we also
# broadcast it to any connected realtime clients in the room.
http.on :post "/channels/:id/messages" \req ->
  cid = str.int req.params.id
  uid = actor req
  if !channels.is_member cid uid
    rep 403 {error:"not a member"}
  res = messages.create_message cid uid req.body.body
  if !res.ok
    rep 422 {blocked:true moderation:res.moderation}
  rt.broadcast_rest cid res.message
  rep 201 {message:res.message flagged:(res.flagged ?? false)}

# Add a reaction to a message.
http.on :post "/messages/:id/reactions" \req ->
  uid = actor req
  r = messages.add_reaction (str.int req.params.id) uid req.body.emoji
  rep 201 r

# List reaction counts for a message.
http.on :get "/messages/:id/reactions" \req ->
  rep 200 {reactions:(messages.reactions_for (str.int req.params.id))}

# ------------------------------------------------------------------
# AI
# ------------------------------------------------------------------

# Summarize the last N messages of a channel (?n= defaults to 50).
http.on :post "/channels/:id/summarize" \req ->
  cid = str.int req.params.id
  n <- 50
  if req.body.n
    n <- req.body.n
  rep 200 (mod.summarize_channel cid n)

# Health check.
http.on :get "/health" \req ->
  rep 200 {ok:true}

# ------------------------------------------------------------------
# Boot: cron + websocket server + http server.
# ------------------------------------------------------------------
jobs.install
rt.serve (str.int (env.WS_PORT ?? "8081"))
http.serve (str.int (env.PORT ?? "8080"))
