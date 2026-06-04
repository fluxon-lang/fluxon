# Reply endpoints

use http db

fn post_reply req
  # POST /tickets/:id/reply
  id = req.params.id
  body = req.body
  author = body.author
  reply_text = body.body

  # Check ticket exists
  ticket = db.one "select * from tickets where id = $1" [id]
  if !ticket
    ret rep 404 {error:"ticket not found"}

  # Validate input
  if !author
    ret rep 400 {error:"author required"}

  if !reply_text
    ret rep 400 {error:"body required"}

  # Save reply
  reply = db.ins "replies" {
    ticket_id: id
    author: author
    body: reply_text
    is_ai: false
  }

  # Update ticket status to answered
  db.up "tickets" {status:"answered"} {id:id}

  rep 201 {
    reply_id: reply.id
    ticket_id: id
    status:"answered"
  }

exp post_reply
