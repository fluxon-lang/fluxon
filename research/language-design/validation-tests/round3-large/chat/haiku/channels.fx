# Channel management

use db
use ./users as user_module

# Create a new channel
exp fn create_channel name is_private created_by
  channel = db.ins "channels" {
    name: name
    is_private: is_private
    created_by: created_by
  }

  # Auto-add creator to channel
  db.ins "memberships" {
    channel: channel.id
    user: created_by
    role: :owner
  }

  ret channel

# Get channel by id
exp fn get_channel id
  ret db.one "select * from channels where id = $1" [id]

# List all public channels
exp fn list_public_channels
  ret db.q "select id, name, is_private, created_by from channels where is_private = false order by name"

# List channels for a user (public + ones they're in)
exp fn list_channels_for_user user_id
  ret db.q "
    select distinct c.id, c.name, c.is_private, c.created_by
    from channels c
    left join memberships m on c.id = m.channel and m.user = $1
    where c.is_private = false or m.user = $1
    order by c.name
  " [user_id]

# Join a channel
exp fn join_channel channel_id user_id
  existing = db.one "select id from memberships where channel = $1 and user = $2" [channel_id user_id]
  if existing
    ret {ok:false error:"Already a member"}

  membership = db.ins "memberships" {
    channel: channel_id
    user: user_id
    role: :member
  }
  ret {ok:true membership: membership}

# Leave a channel
exp fn leave_channel channel_id user_id
  db.q "delete from memberships where channel = $1 and user = $2" [channel_id user_id]
  ret {ok:true}

# Get channel members
exp fn get_channel_members channel_id
  members = db.q "
    select u.id, u.username, u.email, u.status, m.role, m.joined
    from memberships m
    join users u on m.user = u.id
    where m.channel = $1
    order by u.username
  " [channel_id]
  ret members

# Get member count for a channel
exp fn get_channel_member_count channel_id
  result = db.one "select count(*) as cnt from memberships where channel = $1" [channel_id]
  ret result.cnt ?? 0

# Check if user is member of channel
exp fn is_channel_member channel_id user_id
  membership = db.one "select id from memberships where channel = $1 and user = $2" [channel_id user_id]
  ret membership != nil

# Get online users in a channel (from presence tracking)
exp fn get_online_users_in_channel channel_id
  # This will be populated by realtime/ws
  # For now, return users with :online status in this channel
  members = db.q "
    select u.id, u.username, u.status
    from memberships m
    join users u on m.user = u.id
    where m.channel = $1 and u.status = :online
    order by u.username
  " [channel_id]
  ret members
