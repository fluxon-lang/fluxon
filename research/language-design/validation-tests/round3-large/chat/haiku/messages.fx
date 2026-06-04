# Message management and moderation

use db
use ./ai_service as ai_mod

# Create a message and apply moderation
exp fn create_message channel_id user_id body
  # Check if user is member of channel
  member = db.one "select id from memberships where channel = $1 and user = $2" [channel_id user_id]
  if !member
    fail "User not member of channel"

  # Run moderation check asynchronously (queue it)
  # For now, inline it
  moderation = ai_mod.check_message_moderation body

  if moderation.action == :block
    fail "Message blocked by moderation"

  # If flagged, insert with flagged status
  status <- :ok
  if moderation.action == :flag
    status <- :flagged

  # Create message
  msg = db.ins "messages" {
    channel: channel_id
    user: user_id
    body: body
  }

  ret {
    message: msg
    status: status
    moderation: moderation
  }

# Get message history for a channel (paginated)
exp fn get_channel_history channel_id limit before
  limit_val = limit ?? 20
  limit_val = if limit_val > 100 100 else limit_val

  # Check if before cursor provided
  query <- "select * from messages where channel = $1"
  params <- [channel_id]

  if before != nil
    query <- query + " and id < $2"
    params <- params.push before
    # Shift parameter numbers
    query <- "select * from messages where channel = $1 and id < $2"
    params <- [channel_id before]

  query <- query + " order by created desc limit $3"
  if before != nil
    query <- "select * from messages where channel = $1 and id < $2 order by created desc limit $3"
    params <- [channel_id before limit_val]
  else
    query <- "select * from messages where channel = $1 order by created desc limit $2"
    params <- [channel_id limit_val]

  rows = db.q query params

  # Fetch user info for each message
  messages <- []
  each msg in rows
    user = db.one "select id, username from users where id = $1" [msg.user]
    enriched = {
      id: msg.id
      channel: msg.channel
      user_id: msg.user
      user: user
      body: msg.body
      created: msg.created
    }
    messages <- messages.push enriched

  ret messages

# Add reaction to message
exp fn add_reaction message_id user_id emoji
  msg = db.one "select id from messages where id = $1" [message_id]
  if !msg
    fail "Message not found"

  # Check for existing reaction
  existing = db.one "select id from reactions where message = $1 and user = $2 and emoji = $3" [message_id user_id emoji]
  if existing
    ret {ok:false error:"Already reacted with this emoji"}

  reaction = db.ins "reactions" {
    message: message_id
    user: user_id
    emoji: emoji
  }
  ret reaction

# Get reactions for a message
exp fn get_message_reactions message_id
  reactions = db.q "
    select emoji, count(*) as count, array_agg(u.username) as users
    from reactions r
    join users u on r.user = u.id
    where r.message = $1
    group by emoji
    order by count desc
  " [message_id]
  ret reactions

# Remove reaction
exp fn remove_reaction reaction_id
  db.q "delete from reactions where id = $1" [reaction_id]
  ret {ok:true}

# Get recent messages (for trending/stats)
exp fn get_recent_messages hours limit
  limit_val = limit ?? 50
  since = db.one "select now() - interval '$1 hours' as ts" [hours]

  rows = db.q "
    select id, channel, user, body, created
    from messages
    where created > $1
    order by created desc
    limit $2
  " [time.ago hours :hr limit_val]

  ret rows
