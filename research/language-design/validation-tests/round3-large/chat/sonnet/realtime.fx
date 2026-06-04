# realtime.flux — WebSocket realtime server
#
# SPEC GAP HUJJATI:
# Flux spec'da WebSocket primitivlari to'liq aniqlanmagan.
# Spec faqat queue.push / queue.on ni ta'riflaydi. WS uchun quyidagilarni
# IXTIRO QILDIM (spec'da yo'q):
#
#   ws.on :connect  \conn -> ...     — yangi ulanish
#   ws.on :message  \conn msg -> ... — xabar keldi (msg = parse qilingan map)
#   ws.on :disconnect \conn -> ...   — ulanish uzildi
#   ws.send conn payload             — bitta ulanishga yuborish
#   ws.serve port                    — WS serverni ishga tushirish
#   conn.id                          — ulanish unikal ID
#
# Umumiy holat (shared mutable state) uchun:
#   rooms <- {}   — mutable global map (channel_id -> [conn_id list])
#   presence <- {} — mutable global map (conn_id -> {user_id channel_id})
#   conns <- {}   — mutable global map (conn_id -> conn object)
#
# Bu spec'da ko'rsatilmagan, lekin mutable global binding (<-) orqali
# amalga oshirildi. Concurrency xavfi: spec lock/mutex mexanizmini
# ta'riflamagan — bu katta kamchilik.
#
# Broadcasting uchun (spec'da yo'q):
#   ws.broadcast room_id payload    — xonaga yuborish
# Yoki qo'lda: har a'zoga ws.send orqali loop.

use db queue
use ./channels as ch
use ./messages as msg_mod
use ./users

# ─── Global holat ───────────────────────────────────────────────────────────
# channel_id (str key) -> [conn_id list]
rooms <- {}

# conn_id -> {user_id: int, channel_id: int, username: str}
presence <- {}

# conn_id -> conn (ws connection object)
conns <- {}
# ────────────────────────────────────────────────────────────────────────────

# Yordamchi: foydalanuvchini kanalga qo'shish (ichki holat)
fn join_room conn_id channel_id
  key    = str.str channel_id
  members = rooms[key] ?? []
  if !(members.has conn_id)
    rooms <- {key: members.push conn_id}

# Yordamchi: foydalanuvchini kanaldan olib chiqish (ichki holat)
fn leave_room conn_id channel_id
  key     = str.str channel_id
  members = rooms[key] ?? []
  updated = members.filter \cid -> cid != conn_id
  rooms   <- {key: updated}

# Yordamchi: xonaga xabar tarqatish
fn broadcast_to_room channel_id payload
  key     = str.str channel_id
  members = rooms[key] ?? []
  each cid in members
    c = conns[cid]
    if c != nil
      ws.send c payload

# Yordamchi: bitta ulanishga xato yuborish
fn send_error conn payload
  ws.send conn {type:"error" data:payload}

# Yordamchi: bitta ulanishga muvaffaqiyat yuborish
fn send_ok conn payload
  ws.send conn {type:"ok" data:payload}

# ─── WebSocket hodisalari ────────────────────────────────────────────────────

# Yangi ulanish
ws.on :connect \conn ->
  conns <- {(conn.id): conn}
  log "WS ulanish: ${conn.id}"
  ws.send conn {type:"welcome" data:{conn_id:conn.id}}

# Ulanish uzildi
ws.on :disconnect \conn ->
  info = presence[conn.id]
  if info != nil
    # Kanaldan chiqish va presence yangilash
    leave_room conn.id info.channel_id

    # Boshqa a'zolarga bildirish
    broadcast_to_room info.channel_id {
      type: "presence"
      data: {
        event:    "left"
        user_id:  info.user_id
        username: info.username
        channel:  info.channel_id
      }
    }

    # Foydalanuvchi statusini offline qil
    db.up "users" {status::offline} {id:info.user_id}

    # Presence'dan o'chirish
    # SPEC GAP: map'dan kalit o'chirish primitivi yo'q.
    # Workaround: nil qo'yish orqali "bo'sh" qilish
    presence <- {(conn.id): nil}

  conns <- {(conn.id): nil}
  log "WS uzildi: ${conn.id}"

# Xabar keldi
# Har bir xabar JSON map bo'lib keladi:
#   { type: "auth" | "join" | "leave" | "message" | "typing" | "ping", data: {...} }
ws.on :message \conn raw_msg ->
  # SPEC GAP: ws xabarini JSON'dan parse qilish — spec'da ws.on qanday raw
  # ma'lumot berishi aniq emas. json.dec orqali decode qilamiz.
  msg = json.dec raw_msg

  if msg == nil
    send_error conn {message:"yaroqsiz JSON"}
    ret nil

  t = msg.type

  match t
    "auth"    -> handle_auth conn msg.data
    "join"    -> handle_join conn msg.data
    "leave"   -> handle_leave conn msg.data
    "message" -> handle_message conn msg.data
    "typing"  -> handle_typing conn msg.data
    "ping"    -> ws.send conn {type:"pong" data:{ts:time.now}}
    _         -> send_error conn {message:"noma'lum xabar turi: ${t}"}

# ─── Xabar handlerlari ───────────────────────────────────────────────────────

# Autentifikatsiya
# data: { user_id }
fn handle_auth conn data
  user_id = data.user_id
  if user_id == nil
    send_error conn {message:"user_id majburiy"}
    ret nil

  user = users.find_user_by_id user_id
  if user == nil
    send_error conn {message:"foydalanuvchi topilmadi"}
    ret nil

  # Presence'ni yangilash
  existing = presence[conn.id]
  channel_id = nil
  if existing != nil
    channel_id <- existing.channel_id

  presence <- {(conn.id): {user_id:user_id username:user.username channel_id:channel_id}}

  # Foydalanuvchi statusini online qil
  db.up "users" {status::online} {id:user_id}

  send_ok conn {message:"autentifikatsiya muvaffaqiyatli" user_id:user_id username:user.username}

# Kanalga qo'shilish
# data: { channel_id }
fn handle_join conn data
  info = presence[conn.id]
  if info == nil | info.user_id == nil
    send_error conn {message:"avval autentifikatsiya qiling"}
    ret nil

  channel_id = data.channel_id
  if channel_id == nil
    send_error conn {message:"channel_id majburiy"}
    ret nil

  # DB a'zoligini tekshirish
  if !(ch.is_member channel_id info.user_id)
    send_error conn {message:"siz bu kanalga a'zo emassiz"}
    ret nil

  # Oldingi kanaldan chiqish
  if info.channel_id != nil & info.channel_id != channel_id
    leave_room conn.id info.channel_id
    broadcast_to_room info.channel_id {
      type: "presence"
      data: {event:"left" user_id:info.user_id username:info.username channel:info.channel_id}
    }

  # Yangi kanalga kirish
  join_room conn.id channel_id
  presence <- {(conn.id): {user_id:info.user_id username:info.username channel_id:channel_id}}

  # Kanalda kim borligini hisoblash
  online_users = get_channel_online_users channel_id

  # Kanalga kelgani haqida xabar tarqatish
  broadcast_to_room channel_id {
    type: "presence"
    data: {
      event:        "joined"
      user_id:      info.user_id
      username:     info.username
      channel:      channel_id
      online_count: online_users.len
    }
  }

  send_ok conn {
    message:      "kanalga qo'shildingiz"
    channel_id:   channel_id
    online_users: online_users
  }

# Kanaldan chiqish
# data: { channel_id }
fn handle_leave conn data
  info = presence[conn.id]
  if info == nil | info.user_id == nil
    send_error conn {message:"avval autentifikatsiya qiling"}
    ret nil

  channel_id = data.channel_id ?? info.channel_id
  if channel_id == nil
    send_error conn {message:"channel_id majburiy"}
    ret nil

  leave_room conn.id channel_id
  presence <- {(conn.id): {user_id:info.user_id username:info.username channel_id:nil}}

  broadcast_to_room channel_id {
    type: "presence"
    data: {event:"left" user_id:info.user_id username:info.username channel:channel_id}
  }

  send_ok conn {message:"kanaldan chiqdingiz" channel_id:channel_id}

# Xabar yuborish
# data: { body }
fn handle_message conn data
  info = presence[conn.id]
  if info == nil | info.user_id == nil
    send_error conn {message:"avval autentifikatsiya qiling"}
    ret nil

  if info.channel_id == nil
    send_error conn {message:"avval kanalga qo'shiling"}
    ret nil

  body = data.body
  if body == nil | str.len body == 0
    send_error conn {message:"xabar bo'sh bo'lmasligi kerak"}
    ret nil

  # Xabarni saqlash (moderatsiya bilan)
  result = msg_mod.save_message info.channel_id info.user_id body

  if !(result.ok)
    send_error conn {
      message:    "xabar bloklandi"
      reason:     result.reason
      action:     result.action
    }
    ret nil

  saved_msg = result.message

  # Xonaga tarqatish
  payload = {
    type: "message"
    data: {
      id:        saved_msg.id
      channel:   info.channel_id
      user_id:   info.user_id
      username:  info.username
      body:      body
      flagged:   result.flagged
      created:   saved_msg.created
    }
  }

  broadcast_to_room info.channel_id payload

  # Agar flaglangan bo'lsa foydalanuvchiga ogohlantirish
  if result.flagged
    ws.send conn {
      type: "warning"
      data: {message:"xabaringiz ko'rib chiqish uchun belgilandi"}
    }

# Yozish ko'rsatkichi
# data: { is_typing: bool }
fn handle_typing conn data
  info = presence[conn.id]
  if info == nil | info.user_id == nil
    ret nil

  if info.channel_id == nil
    ret nil

  is_typing = data.is_typing ?? false

  # Barcha xona a'zolariga yozyapti signalini yuborish (o'zidan tashqari)
  key     = str.str info.channel_id
  members = rooms[key] ?? []
  each cid in members
    if cid != conn.id
      c = conns[cid]
      if c != nil
        ws.send c {
          type: "typing"
          data: {
            user_id:   info.user_id
            username:  info.username
            channel:   info.channel_id
            is_typing: is_typing
          }
        }

# Kanalda online foydalanuvchilar ro'yxati
fn get_channel_online_users channel_id
  key     = str.str channel_id
  members = rooms[key] ?? []
  result  <- []
  each cid in members
    info = presence[cid]
    if info != nil & info.user_id != nil
      result <- result.push {user_id:info.user_id username:info.username}
  result

# ─── Queue konsumerlari (HTTP yoki cron tomonidan yuborilgan xabarlar) ────────

# HTTP POST orqali yuborilgan xabarlarni WS orqali tarqatish
queue.on "broadcast_message" \job ->
  broadcast_to_room job.channel_id {
    type: "message"
    data: {
      id:       job.message.id
      channel:  job.channel_id
      user_id:  job.user_id
      body:     job.message.body
      created:  job.message.created
    }
  }

# Reaksiyalarni tarqatish
queue.on "broadcast_reaction" \job ->
  broadcast_to_room job.channel_id {
    type: "reaction"
    data: {
      message_id: job.message_id
      user_id:    job.user_id
      emoji:      job.emoji
      channel:    job.channel_id
    }
  }

# Eksport: WS serverni ishga tushirish funksiyasi
exp fn start_ws port
  ws.serve port
  log "WS server ${port} portda ishga tushdi"

# Kanalda online foydalanuvchilar (tashqaridan chaqirish uchun)
exp fn channel_presence channel_id
  get_channel_online_users channel_id
