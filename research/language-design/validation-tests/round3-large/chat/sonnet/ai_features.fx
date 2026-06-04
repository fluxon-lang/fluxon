# ai_features.flux — AI qo'shimcha funksiyalar: kanal xulasasi

use http db
use ai as ai_svc
use ./channels as ch

# Kanal xulasasi
# POST /channels/:id/summarize
# Body: { limit (optional, default 50) }
http.on :post "/channels/:id/summarize" \req ->
  channel_id = str.int req.params.id
  limit_raw  = req.body.limit ?? 50
  limit      = limit_raw
  if limit > 200
    limit <- 200
  if limit < 5
    limit <- 5

  channel = db.one "select id, name from channels where id=$1" [channel_id]
  if channel == nil
    rep 404 {error:"kanal topilmadi"}

  # So'nggi N ta xabarni olish
  rows = db.q "select m.body, u.username, m.created
               from messages m
               join users u on u.id = m.user
               where m.channel = $1 and m.is_blocked = false
               order by m.created desc
               limit $2" [channel_id limit]

  if rows.len == 0
    rep 200 {summary:"Bu kanalda hali xabarlar yo'q." channel:channel.name}

  # Xabarlarni matn formatida birlashtirish
  # SPEC GAP: list'dan matn qurish uchun reduce/join primitivi yo'q.
  # each loop + string concatenation bilan qilamiz.
  chat_text <- ""
  each row in rows
    line = "[${row.username}]: ${row.body}\n"
    chat_text <- chat_text + line

  summary = ai_svc.ask "Quyidagi chat tarixini qisqa va aniq xulosa qil (o'zbek tilida).
Faqat asosiy mavzular va muhim fikrlarni yoz. 3-5 jumla yetarli.

Kanal: ${channel.name}
Xabarlar soni: ${rows.len}

Chat tarixi:
${chat_text}"

  rep 200 {
    ok:       true
    channel:  channel.name
    summary:  summary
    based_on: rows.len
  }

# Kanal statistikasi
# GET /channels/:id/stats
http.on :get "/channels/:id/stats" \req ->
  channel_id = str.int req.params.id

  channel = db.one "select id, name from channels where id=$1" [channel_id]
  if channel == nil
    rep 404 {error:"kanal topilmadi"}

  # Xabarlar soni
  msg_count = db.one "select count(*) c from messages where channel=$1 and is_blocked=false" [channel_id]

  # A'zolar soni
  member_count = db.one "select count(*) c from memberships where channel=$1" [channel_id]

  # So'nggi 24 soatdagi xabarlar
  recent_count = db.one "select count(*) c from messages where channel=$1 and created > $2 and is_blocked=false" [channel_id (time.ago 24 :hr)]

  # Eng faol foydalanuvchi
  top_user = db.one "select u.username, count(*) c from messages m join users u on u.id=m.user where m.channel=$1 group by u.id, u.username order by c desc limit 1" [channel_id]

  rep 200 {
    channel:        channel.name
    total_messages: msg_count.c ?? 0
    total_members:  member_count.c ?? 0
    last_24h:       recent_count.c ?? 0
    top_user:       top_user.username ?? "yo'q"
  }

# AI xabar tahlili (admin uchun)
# POST /channels/:id/analyze
# Body: { limit }
http.on :post "/channels/:id/analyze" \req ->
  channel_id = str.int req.params.id
  limit      = req.body.limit ?? 30

  channel = db.one "select id, name from channels where id=$1" [channel_id]
  if channel == nil
    rep 404 {error:"kanal topilmadi"}

  rows = db.q "select body from messages where channel=$1 and is_blocked=false order by created desc limit $2" [channel_id limit]

  if rows.len == 0
    rep 200 {analysis:"Ma'lumot yo'q."}

  chat_text <- ""
  each row in rows
    chat_text <- chat_text + row.body + "\n"

  analysis = ai_svc.json "Quyidagi chat xabarlarini tahlil qil:
${chat_text}

Kanal: ${channel.name}" {
    sentiment:    ":positive|:neutral|:negative"
    main_topics:  [str]
    spam_ratio:   flt
    toxicity:     flt
    summary:      str
  }

  rep 200 {
    ok:       true
    channel:  channel.name
    analysis: analysis
  }
