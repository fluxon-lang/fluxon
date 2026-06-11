# Chat Platform Main Server
# HTTP REST API + WebSocket realtime layer

use http
use db
use json

# Import modules
use ./schema
use ./users as user_mod
use ./channels as ch_mod
use ./messages as msg_mod
use ./realtime as rt_mod
use ./ai_service as ai_mod
use ./cron_jobs

# =================================================================
# Authentication helper (in real app, use JWT)
# =================================================================

fn get_user_from_req req
  user_id_str = req.headers.user_id
  if !user_id_str
    ret nil
  user_id = str.int user_id_str
  user = user_mod.get_user user_id
  ret user

fn require_auth req
  user = get_user_from_req req
  if !user
    rep 401 {error:"Unauthorized"}
  ret user

# =================================================================
# USER ENDPOINTS
# =================================================================

http.on :post "/users" \req ->
  username = req.body.username
  email = req.body.email

  if !username | !email
    rep 400 {error:"username and email required"}

  user = user_mod.create_user username email
  rep 201 user

http.on :get "/users/:id" \req ->
  user_id = str.int req.params.id
  user = user_mod.get_user user_id
  if !user
    rep 404 {error:"User not found"}
  rep 200 user

http.on :get "/users" \req ->
  users = user_mod.list_users
  rep 200 {users: users}

http.on :put "/users/:id/status" \req ->
  user_id = str.int req.params.id
  status = req.body.status
  if !status
    rep 400 {error:"status required"}

  user = user_mod.set_user_status user_id status
  rep 200 user

# =================================================================
# CHANNEL ENDPOINTS
# =================================================================

http.on :post "/channels" \req ->
  user = require_auth req
  if !user
    stop

  name = req.body.name
  is_private = req.body.is_private ?? false

  if !name
    rep 400 {error:"name required"}

  channel = ch_mod.create_channel name is_private user.id
  rep 201 channel

http.on :get "/channels" \req ->
  user = require_auth req
  if !user
    stop

  channels = ch_mod.list_channels_for_user user.id
  rep 200 {channels: channels}

http.on :get "/channels/:id" \req ->
  channel_id = str.int req.params.id
  channel = ch_mod.get_channel channel_id
  if !channel
    rep 404 {error:"Channel not found"}

  members = ch_mod.get_channel_members channel_id
  rep 200 {channel: channel members: members}

http.on :get "/channels/:id/members" \req ->
  channel_id = str.int req.params.id
  members = ch_mod.get_channel_members channel_id
  rep 200 {members: members count: members.len}

http.on :post "/channels/:id/join" \req ->
  user = require_auth req
  if !user
    stop

  channel_id = str.int req.params.id
  result = ch_mod.join_channel channel_id user.id
  if !result.ok
    rep 400 result
  rep 200 {ok:true membership: result.membership}

http.on :post "/channels/:id/leave" \req ->
  user = require_auth req
  if !user
    stop

  channel_id = str.int req.params.id
  ch_mod.leave_channel channel_id user.id
  rep 200 {ok:true}

# =================================================================
# MESSAGE ENDPOINTS
# =================================================================

http.on :post "/channels/:id/messages" \req ->
  user = require_auth req
  if !user
    stop

  channel_id = str.int req.params.id
  body = req.body.body

  if !body
    rep 400 {error:"body required"}

  msg_result = msg_mod.create_message channel_id user.id body

  # Broadcast to realtime
  rt_mod.ws_send_message channel_id user.id body

  if msg_result.status == :flagged
    rep 202 {message: msg_result.message status: msg_result.status warning:"Message flagged for moderation"}
  else
    rep 201 msg_result.message

http.on :get "/channels/:id/messages" \req ->
  channel_id = str.int req.params.id
  limit_str = req.query.limit
  before_str = req.query.before

  limit = if limit_str (str.int limit_str) else 20
  before = if before_str (str.int before_str) else nil

  messages = msg_mod.get_channel_history channel_id limit before
  rep 200 {messages: messages}

http.on :post "/messages/:id/reactions" \req ->
  user = require_auth req
  if !user
    stop

  message_id = str.int req.params.id
  emoji = req.body.emoji

  if !emoji
    rep 400 {error:"emoji required"}

  reaction = msg_mod.add_reaction message_id user.id emoji
  rep 201 reaction

http.on :get "/messages/:id/reactions" \req ->
  message_id = str.int req.params.id
  reactions = msg_mod.get_message_reactions message_id
  rep 200 {reactions: reactions}

# =================================================================
# AI ENDPOINTS
# =================================================================

http.on :post "/channels/:id/summarize" \req ->
  user = require_auth req
  if !user
    stop

  channel_id = str.int req.params.id
  last_n = req.body.last_n ?? 50

  # Check if user is member
  is_member = ch_mod.is_channel_member channel_id user.id
  if !is_member
    rep 403 {error:"Not a member of this channel"}

  summary = ai_mod.summarize_channel channel_id last_n
  rep 200 summary

http.on :get "/channels/:id/topics" \req ->
  user = require_auth req
  if !user
    stop

  channel_id = str.int req.params.id
  is_member = ch_mod.is_channel_member channel_id user.id
  if !is_member
    rep 403 {error:"Not a member of this channel"}

  topics = ai_mod.get_channel_topics channel_id 100
  rep 200 topics

# =================================================================
# REALTIME / WEBSOCKET SIMULATION
# (In real Fluxon, would use http.on :ws or similar)
# We use HTTP endpoints that simulate WebSocket operations
# =================================================================

http.on :post "/ws/connect" \req ->
  user = require_auth req
  if !user
    stop

  channel_id = str.int req.body.channel_id
  result = rt_mod.ws_user_connect channel_id user.id
  rep 200 result

http.on :post "/ws/disconnect" \req ->
  user = require_auth req
  if !user
    stop

  channel_id = str.int req.body.channel_id
  result = rt_mod.ws_user_disconnect channel_id user.id
  rep 200 result

http.on :get "/channels/:id/active-users" \req ->
  channel_id = str.int req.params.id
  active = rt_mod.get_active_users channel_id
  rep 200 {active_users: active}

http.on :get "/channels/:id/typing" \req ->
  channel_id = str.int req.params.id
  typing = rt_mod.get_typing_users channel_id
  rep 200 {typing_users: typing}

http.on :post "/channels/:id/typing" \req ->
  user = require_auth req
  if !user
    stop

  channel_id = str.int req.params.id
  action = req.body.action

  if action == :start
    rt_mod.ws_typing_start channel_id user.id
  elif action == :stop
    rt_mod.ws_typing_stop channel_id user.id

  rep 200 {ok:true}

# =================================================================
# HEALTH CHECK
# =================================================================

http.on :get "/health" \req ->
  rep 200 {status:"ok" timestamp:time.now}

# =================================================================
# STATS / ADMIN
# =================================================================

http.on :get "/stats" \req ->
  user_count = db.one "select count(*) as cnt from users"
  ch_count = db.one "select count(*) as cnt from channels"
  msg_count = db.one "select count(*) as cnt from messages"
  reaction_count = db.one "select count(*) as cnt from reactions"

  stats = {
    users: user_count.cnt ?? 0
    channels: ch_count.cnt ?? 0
    messages: msg_count.cnt ?? 0
    reactions: reaction_count.cnt ?? 0
    timestamp: time.now
  }
  rep 200 stats

# =================================================================
# Start server
# =================================================================

port = env.PORT ?? "8080"
log "Chat server starting on port ${port}"
http.serve (str.int port)
