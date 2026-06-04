# Chat Platform Schema

use db

# Users table
tbl users
  id       serial pk
  username str uniq
  email    str uniq
  status   sym
  created  now

# Channels table (can be public or private)
tbl channels
  id         serial pk
  name       str
  is_private bool
  created_by int ref:users.id
  created    now

# Channel memberships (who is in what channel)
tbl memberships
  id       serial pk
  channel  int ref:channels.id
  user     int ref:users.id
  role     sym
  joined   now

# Messages table
tbl messages
  id      serial pk
  channel int ref:channels.id
  user    int ref:users.id
  body    str
  created now

# Reactions to messages
tbl reactions
  id      serial pk
  message int ref:messages.id
  user    int ref:users.id
  emoji   str
  created now

# Schema initialized
log "Chat schema defined"
