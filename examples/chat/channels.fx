# channels.fluxon — channel CRUD, membership (join/leave), listing.
use db

# Create a channel owned by created_by. Creator is auto-joined as :owner.
exp fn create_channel name is_private created_by
  if !name
    fail "channel name required"
  owner = db.one "select * from users where id=$1" [created_by]
  owner ?? (fail "creator not found")
  ch = db.ins "channels" {name:name is_private:(is_private ?? false) created_by:created_by}
  db.ins "memberships" {channel:ch.id user:created_by role::owner}
  ret ch

# Fetch a channel by id (nil if missing).
exp fn get_channel id
  ret db.one "select * from channels where id=$1" [id]

# Is a user a member of a channel?
exp fn is_member channel_id user_id
  m = db.one "select * from memberships where channel=$1 and user=$2" [channel_id user_id]
  ret m != nil

# Join a channel as a plain :member. Idempotent — re-joining is a no-op.
exp fn join_channel channel_id user_id
  ch = get_channel channel_id
  ch ?? (fail "channel not found")
  if is_member channel_id user_id
    ret {ok:true already:true}
  db.ins "memberships" {channel:channel_id user:user_id role::member}
  ret {ok:true already:false}

# Leave a channel. Removes the membership row.
exp fn leave_channel channel_id user_id
  m = db.one "select * from memberships where channel=$1 and user=$2" [channel_id user_id]
  m ?? (fail "not a member")
  db.q "delete from memberships where id=$1" [m.id]
  ret {ok:true}

# List all channels a given user belongs to, with their role.
exp fn channels_for_user user_id
  ret db.q "select c.id, c.name, c.is_private, c.created, m.role from channels c join memberships m on m.channel = c.id where m.user = $1 order by c.created desc" [user_id]

# List the user ids that are members of a channel (used by realtime rooms).
exp fn member_ids channel_id
  rows = db.q "select user from memberships where channel=$1" [channel_id]
  ret rows.map \r -> r.user
