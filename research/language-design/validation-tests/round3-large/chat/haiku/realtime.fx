# Realtime WebSocket handling
# NOTE: This file demonstrates Fluxon's queue/pubsub patterns.
# True WebSocket/bidirectional streaming is NOT fully specified in the Fluxon spec.
# We improvise here using queue as a message bus.

use db
use queue
use ./messages as msg_module
use ./channels as ch_module

# Global connection registry (MUTABLE)
# Maps: channel_id -> [user_ids]
# This is in-memory; in production, use Redis or a real message broker.
active_connections <- {}

# Global typing indicators: channel_id -> {user_id: timestamp}
typing_indicators <- {}

# User presence per channel: channel_id -> {user_id: timestamp}
presence_per_channel <- {}

# WebSocket message types that realtime handler will respond to
# (In a real implementation, http.on :ws would exist; we model it via queue)

# Connect a user to a channel
exp fn ws_user_connect channel_id user_id
  # Check membership
  is_member = ch_module.is_channel_member channel_id user_id
  if !is_member
    ret {ok:false error:"Not a member"}

  # Add to active connections
  current <- active_connections[channel_id] ?? []
  if !current.has user_id
    current <- current.push user_id
    active_connections[channel_id] <- current

  # Update presence
  pc <- presence_per_channel[channel_id] ?? {}
  pc[user_id] <- time.now
  presence_per_channel[channel_id] <- pc

  # Set user status to online
  db.up "users" {status: :online} {id: user_id}

  # Notify others
  broadcast_event channel_id {
    type: :user_joined
    user_id: user_id
    channel_id: channel_id
    timestamp: time.now
  }

  ret {ok:true user_id: user_id channel_id: channel_id}

# Disconnect user from channel
exp fn ws_user_disconnect channel_id user_id
  # Remove from active connections
  current <- active_connections[channel_id] ?? []
  new_list <- []
  each uid in current
    if uid != user_id
      new_list <- new_list.push uid
  active_connections[channel_id] <- new_list

  # Remove from presence
  pc <- presence_per_channel[channel_id] ?? {}
  if pc[user_id]
    # In a real language, delete pc[user_id], but Fluxon maps don't have delete
    # Workaround: rebuild map without this key
    new_pc <- {}
    each k, v in pc
      if k != user_id
        new_pc[k] <- v
    presence_per_channel[channel_id] <- new_pc

  # Broadcast disconnect
  broadcast_event channel_id {
    type: :user_left
    user_id: user_id
    channel_id: channel_id
    timestamp: time.now
  }

  ret {ok:true}

# Send a message in realtime (persisted + broadcast)
exp fn ws_send_message channel_id user_id body
  # Create message (with moderation)
  msg_result = msg_module.create_message channel_id user_id body
  msg = msg_result.message

  if msg_result.status == :flagged
    # Notify moderators (in real app, queue this for review)
    queue.push "moderate_message" {
      message_id: msg.id
      reason: msg_result.moderation.reason
      confidence: msg_result.moderation.confidence
    }

  # Broadcast to all in channel
  broadcast_event channel_id {
    type: :message
    message: {
      id: msg.id
      user_id: msg.user
      body: msg.body
      created: msg.created
    }
    timestamp: time.now
  }

  ret msg

# Typing indicator
exp fn ws_typing_start channel_id user_id
  ti <- typing_indicators[channel_id] ?? {}
  ti[user_id] <- time.now
  typing_indicators[channel_id] <- ti

  broadcast_event channel_id {
    type: :typing
    user_id: user_id
    timestamp: time.now
  }

  ret {ok:true}

# Stop typing
exp fn ws_typing_stop channel_id user_id
  ti <- typing_indicators[channel_id] ?? {}
  # Remove user from typing (workaround: rebuild map)
  new_ti <- {}
  each k, v in ti
    if k != user_id
      new_ti[k] <- v
  typing_indicators[channel_id] <- new_ti

  broadcast_event channel_id {
    type: :typing_stop
    user_id: user_id
  }

  ret {ok:true}

# Get who's typing in a channel
exp fn get_typing_users channel_id
  ti <- typing_indicators[channel_id] ?? {}
  typing_list <- []

  each user_id, timestamp in ti
    # Filter out old typing indicators (older than 5 seconds)
    age = time.now - timestamp
    if age < 5000  # milliseconds, rough estimate
      typing_list <- typing_list.push user_id

  ret typing_list

# Get active users in a channel (presence)
exp fn get_active_users channel_id
  conns <- active_connections[channel_id] ?? []
  ret conns

# Broadcast event to all users in a channel
# (In real Fluxon, this would push to a message queue or WebSocket server)
fn broadcast_event channel_id event
  # Queue the event for broadcasting
  # In production: pub/sub system, Redis, or actual WebSocket server
  queue.push "broadcast" {
    channel_id: channel_id
    event: event
  }

# Cleanup old typing indicators (call periodically)
exp fn cleanup_typing_indicators
  each ch_id, ti in typing_indicators
    new_ti <- {}
    each uid, ts in ti
      age = time.now - ts
      if age < 10000  # Keep 10 seconds
        new_ti[uid] <- ts
    typing_indicators[ch_id] <- new_ti
  ret {cleaned: true}

# Cleanup old presence (call periodically)
exp fn cleanup_presence
  each ch_id, pc in presence_per_channel
    # In a real app, timeout inactive users after 5 minutes
    ret {cleaned: true}
