# users.flux — Foydalanuvchi REST endpointlari va yordamchi funksiyalar

use http db
use ./schema

# Foydalanuvchi yaratish
# POST /users
# Body: { username, email }
http.on :post "/users" \req ->
  username = req.body.username
  email    = req.body.email

  if username == nil | email == nil
    rep 400 {error:"username va email majburiy"}

  # Takrorlanishni tekshirish
  existing = db.one "select id from users where username=$1 or email=$2" [username email]
  if existing != nil
    rep 409 {error:"username yoki email band"}

  user = db.ins "users" {username:username email:email status::offline}
  rep 201 {ok:true user:user}

# Foydalanuvchini olish
# GET /users/:id
http.on :get "/users/:id" \req ->
  uid = str.int req.params.id
  user = db.one "select id, username, email, status, created from users where id=$1" [uid]
  if user == nil
    rep 404 {error:"foydalanuvchi topilmadi"}
  rep 200 user

# Foydalanuvchini yangilash (status o'zgartirish)
# PUT /users/:id/status
# Body: { status }   (:online | :offline | :away)
http.on :put "/users/:id/status" \req ->
  uid    = str.int req.params.id
  status = req.body.status
  if status == nil
    rep 400 {error:"status majburiy"}
  db.up "users" {status:status} {id:uid}
  rep 200 {ok:true status:status}

# Barcha foydalanuvchilar ro'yxati
# GET /users
http.on :get "/users" \req ->
  users = db.q "select id, username, email, status from users order by username asc"
  rep 200 users

# Eksport: boshqa fayllar uchun yordamchi funksiya
exp fn find_user_by_id uid
  db.one "select id, username, email, status from users where id=$1" [uid]

exp fn user_exists uid
  u = db.one "select id from users where id=$1" [uid]
  u != nil
