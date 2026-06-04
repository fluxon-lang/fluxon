use ws db json

# WebSocket connection handler
exp fn setup_websocket
  ws.on :connect \conn ->
    conn.data.user_id = nil
    log "WS connect: ${conn.id}"

  ws.on :message \conn msg ->
    m = json.dec msg
    if m.action == :subscribe
      user_id = m.user_id
      conn.data.user_id = user_id
      room = "user:${user_id}"
      ws.room.join conn room
      log "User ${user_id} subscribed to notifications"

  ws.on :disconnect \conn ->
    if conn.data.user_id
      room = "user:${conn.data.user_id}"
      ws.room.leave conn room
      log "User disconnected from notifications"

# Send notification to a candidate (via websocket)
exp fn notify_candidate user_id body
  room = "user:${user_id}"
  msg = json.enc {
    type::notification
    body:body
    timestamp:(time.now)
  }
  ws.room.send room msg

# Create and persist a notification
exp fn create_notification user_id body
  notification = db.ins "notifications" {
    user_id:user_id
    body:body
    read:false
  }
  notify_candidate user_id body
  ret notification

# Mark notification as read
exp fn mark_notification_read notif_id
  db.up "notifications" {read:true} {id:notif_id}

# Get all notifications for a user
exp fn get_notifications user_id
  notifications = db.q "select * from notifications where user_id=$1 order by created desc" [user_id]
  ret notifications
