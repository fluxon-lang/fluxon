# Ticket handlers

use db ai
use ./classify
use ./validate

# POST /tickets — create a new ticket + AI classification + confidence routing
exp fn create req
  b = req.body
  email = b.email
  subject = b.subject ?? ""
  body = b.body

  # Validation: email is valid and body is not empty
  if !(validate.valid_email email)
    ret rep 400 {ok:false error:"invalid email"}
  if !(validate.non_empty body)
    ret rep 400 {ok:false error:"body cannot be empty"}

  # AI classification
  c = classify.classify subject body
  conf = c.conf ?? 0.0

  # Initial status based on confidence
  status <- :needs_review
  if conf > 0.85
    status <- :answered
  elif conf >= 0.6
    status <- :needs_review
  else
    status <- :escalated

  # Save the ticket
  t = db.ins "tickets" {customer_email:email subject:subject body:body category:c.category priority:c.priority status:status ai_confidence:conf}!

  # High confidence — the AI drafts and saves an automatic reply
  if conf > 0.85
    draft = classify.draft_reply subject body
    db.ins "replies" {ticket:t.id author:"ai" body:draft is_ai:true}!

  rep 201 {id:t.id category:t.category priority:t.priority status:t.status}

# GET /tickets — list, optional ?status= and ?priority= filters
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

# GET /tickets/:id — ticket + all its replies
exp fn get req
  id = req.params.id
  t = db.one "select * from tickets where id=$1" [id]!
  if t == nil
    ret rep 404 {ok:false error:"ticket not found"}
  rs = db.q "select * from replies where ticket=$1 order by created" [id]!
  rep 200 {ticket:t replies:rs}

# POST /tickets/:id/reply — agent reply (is_ai false) + status :answered
exp fn reply req
  id = req.params.id
  t = db.one "select * from tickets where id=$1" [id]!
  if t == nil
    ret rep 404 {ok:false error:"ticket not found"}

  b = req.body
  author = b.author ?? "agent"
  body = b.body
  if !(validate.non_empty body)
    ret rep 400 {ok:false error:"body cannot be empty"}

  r = db.ins "replies" {ticket:id author:author body:body is_ai:false}!
  db.up "tickets" {status::answered} {id:id}!

  rep 201 {id:r.id ticket:id status::answered}
