# schema.fluxon — database schemas for the realtime chat platform
# All tables for the chat backend. Loaded once; tbl declarations register
# the schema with the db battery.
use db

# Users of the platform.
tbl users
  id       serial pk
  username str uniq
  email    str uniq
  status   sym            # :online :offline :away — presence/account status

# Channels (chat rooms).
tbl channels
  id         serial pk
  name       str
  is_private bool
  created_by int ref:users.id
  created    now

# Channel membership: which user belongs to which channel and their role.
tbl memberships
  id      serial pk
  channel int ref:channels.id
  user    int ref:users.id
  role    sym            # :owner :admin :member
  joined  now

# Messages posted into a channel.
tbl messages
  id      serial pk
  channel int ref:channels.id
  user    int ref:users.id
  body    str
  status  sym            # :ok :flagged :blocked — moderation outcome
  created now

# Emoji reactions on a message.
tbl reactions
  id      serial pk
  message int ref:messages.id
  user    int ref:users.id
  emoji   str
  created now
