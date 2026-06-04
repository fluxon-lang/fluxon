# channels.flux — Kanal yaratish, qo'shilish, chiqish va ro'yxat endpointlari

use http db
use ./schema
use ./users

# Kanal yaratish
# POST /channels
# Body: { name, is_private, created_by }
http.on :post "/channels" \req ->
  name       = req.body.name
  is_private = req.body.is_private ?? false
  created_by = req.body.created_by

  if name == nil | created_by == nil
    rep 400 {error:"name va created_by majburiy"}

  # Kanal nomini tekshirish
  existing = db.one "select id from channels where name=$1" [name]
  if existing != nil
    rep 409 {error:"bu nom band"}

  # Foydalanuvchi mavjudligini tekshirish
  if !(users.user_exists created_by)
    rep 404 {error:"foydalanuvchi topilmadi"}

  channel = db.ins "channels" {name:name is_private:is_private created_by:created_by}

  # Yaratuvchini owner sifatida a'zo qil
  db.ins "memberships" {channel:channel.id user:created_by role::owner}

  rep 201 {ok:true channel:channel}

# Kanallar ro'yxati (foydalanuvchi uchun)
# GET /users/:id/channels
http.on :get "/users/:id/channels" \req ->
  uid = str.int req.params.id
  rows = db.q "select c.id, c.name, c.is_private, c.created_by, c.created, m.role
               from channels c
               join memberships m on m.channel = c.id
               where m.user = $1
               order by c.name asc" [uid]
  rep 200 rows

# Barcha ochiq kanallar ro'yxati
# GET /channels
http.on :get "/channels" \req ->
  channels = db.q "select id, name, is_private, created_by, created from channels where is_private = false order by name asc"
  rep 200 channels

# Kanalga qo'shilish
# POST /channels/:id/join
# Body: { user_id }
http.on :post "/channels/:id/join" \req ->
  channel_id = str.int req.params.id
  user_id    = req.body.user_id

  if user_id == nil
    rep 400 {error:"user_id majburiy"}

  channel = db.one "select id, is_private from channels where id=$1" [channel_id]
  if channel == nil
    rep 404 {error:"kanal topilmadi"}

  if channel.is_private
    rep 403 {error:"bu maxfiy kanal, taklif kerak"}

  if !(users.user_exists user_id)
    rep 404 {error:"foydalanuvchi topilmadi"}

  # Allaqachon a'zomi tekshirish
  already = db.one "select id from memberships where channel=$1 and user=$2" [channel_id user_id]
  if already != nil
    rep 409 {error:"allaqachon a'zo"}

  membership = db.ins "memberships" {channel:channel_id user:user_id role::member}
  rep 200 {ok:true membership:membership}

# Kanaldan chiqish
# POST /channels/:id/leave
# Body: { user_id }
http.on :post "/channels/:id/leave" \req ->
  channel_id = str.int req.params.id
  user_id    = req.body.user_id

  if user_id == nil
    rep 400 {error:"user_id majburiy"}

  # A'zolikni tekshirish
  membership = db.one "select id, role from memberships where channel=$1 and user=$2" [channel_id user_id]
  if membership == nil
    rep 404 {error:"a'zolik topilmadi"}

  # Owner chiqib keta olmaydi (kanalning yagona ownerini himoya qilish)
  if membership.role == :owner
    owner_count = db.one "select count(*) c from memberships where channel=$1 and role=$2" [channel_id "owner"]
    if owner_count.c <= 1
      rep 403 {error:"yagona owner chiqib keta olmaydi, avval boshqaga o'tkazing"}

  db.up "memberships" {role::left} {id:membership.id}
  # Yoki to'liq o'chirish uchun (quyida SQL DELETE — spec'da db.del yo'q, shuning uchun workaround):
  # SPEC GAP: db.del (delete) operatsiyasi ko'rsatilmagan. db.up bilan softdelete qilamiz.
  rep 200 {ok:true}

# Kanal a'zolari ro'yxati
# GET /channels/:id/members
http.on :get "/channels/:id/members" \req ->
  channel_id = str.int req.params.id
  channel = db.one "select id from channels where id=$1" [channel_id]
  if channel == nil
    rep 404 {error:"kanal topilmadi"}

  members = db.q "select u.id, u.username, u.email, u.status, m.role, m.joined
                  from users u
                  join memberships m on m.user = u.id
                  where m.channel = $1
                  order by u.username asc" [channel_id]
  rep 200 members

# Kanal ma'lumotlarini olish
# GET /channels/:id
http.on :get "/channels/:id" \req ->
  channel_id = str.int req.params.id
  channel = db.one "select id, name, is_private, created_by, created from channels where id=$1" [channel_id]
  if channel == nil
    rep 404 {error:"kanal topilmadi"}
  rep 200 channel

# Eksport: boshqa fayllar uchun yordamchi funksiyalar
exp fn channel_exists channel_id
  c = db.one "select id from channels where id=$1" [channel_id]
  c != nil

exp fn is_member channel_id user_id
  m = db.one "select id from memberships where channel=$1 and user=$2" [channel_id user_id]
  m != nil

exp fn get_channel_members channel_id
  db.q "select user from memberships where channel=$1" [channel_id]
