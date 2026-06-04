# messages.flux — message persistence, history (paginated), reactions.
use db
use ./moderation as mod

# Persist a message after running it through AI moderation.
# Returns a map describing what happened:
#   {ok:true  message:<row> moderation:<result>}                 — stored
#   {ok:false blocked:true  moderation:<result>}                 — toxic, rejected
# Flagged messages ARE stored (status :flagged) but the caller is told so it
# can route them for human review.
exp fn create_message channel_id user_id body
  if !body
    fail "message body required"
  if str.len body > 4000
    fail "message too long"

  m = mod.moderate body

  match m.action
    :block ->
      ret {ok:false blocked:true moderation:m}
    :flag ->
      row = db.ins "messages" {channel:channel_id user:user_id body:body status::flagged}
      ret {ok:true message:row moderation:m flagged:true}
    _ ->
      row = db.ins "messages" {channel:channel_id user:user_id body:body status::ok}
      ret {ok:true message:row moderation:m}

# Fetch one message by id.
exp fn get_message id
  ret db.one "select * from messages where id=$1" [id]

# Paginated history for a channel. Cursor pagination via `before` (a message id);
# returns up to `limit` messages older than the cursor, newest-first.
# before == nil → most recent page.
exp fn history channel_id before limit
  lim <- limit ?? 50
  if lim > 200
    lim <- 200
  if before
    ret db.q "select m.id, m.channel, m.user, m.body, m.status, m.created, u.username from messages m join users u on u.id = m.user where m.channel = $1 and m.id < $2 and m.status != 'blocked' order by m.id desc limit $3" [channel_id before lim]
  ret db.q "select m.id, m.channel, m.user, m.body, m.status, m.created, u.username from messages m join users u on u.id = m.user where m.channel = $1 and m.status != 'blocked' order by m.id desc limit $2" [channel_id lim]

# Add (or no-op if duplicate) a reaction to a message.
exp fn add_reaction message_id user_id emoji
  if !emoji
    fail "emoji required"
  msg = get_message message_id
  msg ?? (fail "message not found")
  existing = db.one "select * from reactions where message=$1 and user=$2 and emoji=$3" [message_id user_id emoji]
  if existing
    ret existing
  ret db.ins "reactions" {message:message_id user:user_id emoji:emoji}

# All reactions on a message, grouped emoji → count.
exp fn reactions_for message_id
  rows = db.q "select emoji, count(*) c from reactions where message=$1 group by emoji" [message_id]
  ret rows
