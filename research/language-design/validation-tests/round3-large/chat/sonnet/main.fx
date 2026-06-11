# main.fluxon — Asosiy kirish nuqtasi
# HTTP va WebSocket serverlarini ishga tushiradi, barcha modullarni ulaydi.

use http
use ./schema
use ./users
use ./channels
use ./messages
use ./ai_features
use ./realtime
use ./cron_jobs

# Port konfiguratsiyasi
http_port = str.int (env.HTTP_PORT ?? "8080")
ws_port   = str.int (env.WS_PORT ?? "8081")

log "=== Fluxon Chat Platform ishga tushmoqda ==="
log "HTTP port: ${http_port}"
log "WS port:   ${ws_port}"
log "DATABASE_URL: ${env.DATABASE_URL ?? '(belgilanmagan)'}"

# Sog'liqni tekshirish endpoint
http.on :get "/health" \req ->
  db_ok = true
  # DB ulanishini tekshirish
  test = db.one "select 1 as ok"
  if test == nil
    db_ok <- false

  status = "ok"
  if !(db_ok)
    status <- "degraded"

  rep 200 {
    status:    status
    db:        db_ok
    http_port: http_port
    ws_port:   ws_port
    ts:        time.now
  }

# API versiyasi
http.on :get "/" \req ->
  rep 200 {
    service: "Fluxon Chat Platform"
    version: "1.0.0"
    endpoints: {
      users:    "/users"
      channels: "/channels"
      health:   "/health"
    }
  }

# Presence (kim online — HTTP fallback)
# GET /channels/:id/presence
http.on :get "/channels/:id/presence" \req ->
  channel_id = str.int req.params.id
  channel    = db.one "select id, name from channels where id=$1" [channel_id]
  if channel == nil
    rep 404 {error:"kanal topilmadi"}

  online = realtime.channel_presence channel_id
  rep 200 {
    channel:      channel.name
    online_users: online
    count:        online.len
  }

# WebSocket serverini ishga tushirish
realtime.start_ws ws_port

# HTTP serverini ishga tushirish (bloklovchi, oxirida)
log "HTTP server ${http_port} portda ishga tushmoqda..."
http.serve http_port
