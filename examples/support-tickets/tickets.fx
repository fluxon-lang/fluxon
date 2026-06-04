# Ticket handlerlari

use db ai
use ./classify
use ./validate

# POST /tickets — yangi ticket yaratish + AI klassifikatsiya + confidence routing
exp fn create req
  b = req.body
  email = b.email
  subject = b.subject ?? ""
  body = b.body

  # Validatsiya: email to'g'ri va body bo'sh emas
  if !(validate.valid_email email)
    ret rep 400 {ok:false error:"noto'g'ri email"}
  if !(validate.non_empty body)
    ret rep 400 {ok:false error:"body bo'sh bo'lishi mumkin emas"}

  # AI klassifikatsiya
  c = classify.classify subject body
  conf = c.conf ?? 0.0

  # Boshlang'ich status confidence bo'yicha
  status <- :needs_review
  if conf > 0.85
    status <- :answered
  elif conf >= 0.6
    status <- :needs_review
  else
    status <- :escalated

  # Ticketni saqlash
  t = db.ins "tickets" {customer_email:email subject:subject body:body category:c.category priority:c.priority status:status ai_confidence:conf}!

  # Yuqori ishonch — AI avtomat javob qoralaydi va saqlaydi
  if conf > 0.85
    draft = classify.draft_reply subject body
    db.ins "replies" {ticket:t.id author:"ai" body:draft is_ai:true}!

  rep 201 {id:t.id category:t.category priority:t.priority status:t.status}

# GET /tickets — ro'yxat, ixtiyoriy ?status= va ?priority= filtrlari
exp fn list req
  status = req.query.status
  priority = req.query.priority

  rows <- nil
  if status != nil & priority != nil
    rows <- db.q "select * from tickets where status=$1 and priority=$2 order by created desc" [status priority]!
  elif status != nil
    rows <- db.q "select * from tickets where status=$1 order by created desc" [status]!
  elif priority != nil
    rows <- db.q "select * from tickets where priority=$1 order by created desc" [priority]!
  else
    rows <- db.q "select * from tickets order by created desc"!

  rep 200 {tickets:rows}

# GET /tickets/:id — ticket + barcha javoblari
exp fn get req
  id = req.params.id
  t = db.one "select * from tickets where id=$1" [id]!
  if t == nil
    ret rep 404 {ok:false error:"ticket topilmadi"}
  rs = db.q "select * from replies where ticket=$1 order by created" [id]!
  rep 200 {ticket:t replies:rs}

# POST /tickets/:id/reply — agent javobi (is_ai false) + status :answered
exp fn reply req
  id = req.params.id
  t = db.one "select * from tickets where id=$1" [id]!
  if t == nil
    ret rep 404 {ok:false error:"ticket topilmadi"}

  b = req.body
  author = b.author ?? "agent"
  body = b.body
  if !(validate.non_empty body)
    ret rep 400 {ok:false error:"body bo'sh bo'lishi mumkin emas"}

  r = db.ins "replies" {ticket:id author:author body:body is_ai:false}!
  db.up "tickets" {status::answered} {id:id}!

  rep 201 {id:r.id ticket:id status::answered}
