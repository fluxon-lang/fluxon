# messages.flux — Xabar yuborish, tarix va reaksiya endpointlari

use http db queue
use ./schema
use ./channels
use ./users
use ./moderation

# Xabar yuborish (HTTP fallback — asosiy yo'l WS orqali)
# POST /channels/:id/messages
# Body: { user_id, body }
http.on :post "/channels/:id/messages" \req ->
  channel_id = str.int req.params.id
  user_id    = req.body.user_id
  body       = req.body.body

  if user_id == nil | body == nil
    rep 400 {error:"user_id va body majburiy"}

  if !(channels.channel_exists channel_id)
    rep 404 {error:"kanal topilmadi"}

  if !(channels.is_member channel_id user_id)
    rep 403 {error:"siz bu kanalga a'zo emassiz"}

  if str.len body == 0
    rep 400 {error:"xabar bo'sh bo'lmasligi kerak"}

  # AI moderatsiya
  mod = moderation.moderate_message body channel_id user_id

  if !(mod.allowed)
    rep 403 {
      error:      "xabar bloklandi"
      reason:     mod.reason
      action:     mod.action
      confidence: mod.confidence
    }

  # Xabarni saqlash
  is_flagged = mod.action == :flagged
  msg = db.ins "messages" {
    channel:    channel_id
    user:       user_id
    body:       body
    is_blocked: false
    is_flagged: is_flagged
    created:    time.now
  }

  # Realtime broadcast uchun navbatga qo'shish
  queue.push "broadcast_message" {
    channel_id: channel_id
    message:    msg
    user_id:    user_id
  }

  if is_flagged
    rep 201 {ok:true message:msg warning:"xabar ko'rib chiqish uchun belgilandi"}

  rep 201 {ok:true message:msg}

# Xabar tarixi (sahifalash bilan)
# GET /channels/:id/messages?before=&limit=
http.on :get "/channels/:id/messages" \req ->
  channel_id = str.int req.params.id
  limit_raw  = req.query.limit ?? "50"
  before_raw = req.query.before

  limit = str.int limit_raw
  if limit > 100
    limit <- 100
  if limit < 1
    limit <- 20

  channel = db.one "select id from channels where id=$1" [channel_id]
  if channel == nil
    rep 404 {error:"kanal topilmadi"}

  rows = nil
  if before_raw != nil
    before_id = str.int before_raw
    rows <- db.q "select m.id, m.channel, m.user, m.body, m.is_flagged, m.created,
                         u.username
                  from messages m
                  join users u on u.id = m.user
                  where m.channel = $1
                    and m.is_blocked = false
                    and m.id < $2
                  order by m.created desc
                  limit $3" [channel_id before_id limit]
  else
    rows <- db.q "select m.id, m.channel, m.user, m.body, m.is_flagged, m.created,
                         u.username
                  from messages m
                  join users u on u.id = m.user
                  where m.channel = $1
                    and m.is_blocked = false
                  order by m.created desc
                  limit $2" [channel_id limit]

  rep 200 {messages:rows count:rows.len}

# Reaksiya qo'shish
# POST /messages/:id/reactions
# Body: { user_id, emoji }
http.on :post "/messages/:id/reactions" \req ->
  message_id = str.int req.params.id
  user_id    = req.body.user_id
  emoji      = req.body.emoji

  if user_id == nil | emoji == nil
    rep 400 {error:"user_id va emoji majburiy"}

  # Xabar mavjudligini tekshirish
  msg = db.one "select id, channel from messages where id=$1 and is_blocked=false" [message_id]
  if msg == nil
    rep 404 {error:"xabar topilmadi"}

  # A'zolikni tekshirish
  if !(channels.is_member msg.channel user_id)
    rep 403 {error:"siz bu kanalga a'zo emassiz"}

  # Bir xil reaksiya ikki marta qo'yilmasin
  existing = db.one "select id from reactions where message=$1 and user=$2 and emoji=$3" [message_id user_id emoji]
  if existing != nil
    rep 409 {error:"bu reaksiya allaqachon qo'yilgan"}

  reaction = db.ins "reactions" {message:message_id user:user_id emoji:emoji}

  # Realtime broadcast uchun navbatga qo'shish
  queue.push "broadcast_reaction" {
    channel_id: msg.channel
    reaction:   reaction
    message_id: message_id
    user_id:    user_id
    emoji:      emoji
  }

  rep 201 {ok:true reaction:reaction}

# Reaksiyani o'chirish
# DEL /messages/:id/reactions/:emoji
# SPEC GAP: :del metod mavjud (http.on :del ...) lekin parametrli route + body
# kombinatsiyasida user_id ni query yoki body dan olish kerak — biz query ishlatamiz
http.on :del "/messages/:id/reactions/:emoji" \req ->
  message_id = str.int req.params.id
  emoji      = req.params.emoji
  user_id    = str.int (req.query.user_id ?? "0")

  if user_id == 0
    rep 400 {error:"user_id majburiy"}

  reaction = db.one "select id from reactions where message=$1 and user=$2 and emoji=$3" [message_id user_id emoji]
  if reaction == nil
    rep 404 {error:"reaksiya topilmadi"}

  # SPEC GAP: db.del yo'q — softdelete yoki raw SQL workaround kerak
  # Biz db.up orqali flaglash bilan o'chiramiz (softdelete pattern)
  # Yoki: db.q bilan DELETE (qiymat qaytarmaydi)
  db.q "DELETE FROM reactions WHERE id=$1" [reaction.id]
  rep 200 {ok:true}

# Xabar reaksiyalari ro'yxati
# GET /messages/:id/reactions
http.on :get "/messages/:id/reactions" \req ->
  message_id = str.int req.params.id
  msg = db.one "select id from messages where id=$1" [message_id]
  if msg == nil
    rep 404 {error:"xabar topilmadi"}

  reactions = db.q "select r.id, r.emoji, r.user, u.username
                    from reactions r
                    join users u on u.id = r.user
                    where r.message = $1" [message_id]
  rep 200 reactions

# Eksport: WS moduli uchun xabar saqlash funksiyasi
exp fn save_message channel_id user_id body
  mod = moderation.moderate_message body channel_id user_id

  if !(mod.allowed)
    ret {ok:false error:"xabar bloklandi" reason:mod.reason action:mod.action}

  is_flagged = mod.action == :flagged
  msg = db.ins "messages" {
    channel:    channel_id
    user:       user_id
    body:       body
    is_blocked: false
    is_flagged: is_flagged
    created:    time.now
  }
  ret {ok:true message:msg flagged:is_flagged}
