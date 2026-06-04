# tickets.flux — HTTP route handlers for the ticket system

use http db
use ./ai_helper

# POST /tickets — create a new ticket with AI classification
exp fn handle_create_ticket req
  email   = req.body.email
  subject = req.body.subject
  body    = req.body.body

  # Validate required fields
  if email == nil | email == ""
    ret rep 400 {error: "email is required"}
  if body == nil | body == ""
    ret rep 400 {error: "body is required"}

  # Use AI to classify the ticket
  classification = ai_helper.classify_ticket subject body

  category      = classification.category ?? "other"
  priority      = classification.priority ?? "low"
  confidence    = classification._.conf ?? 0.0

  # Map string category to symbol representation stored as string
  cat_val = category
  pri_val = priority

  # Determine initial status based on AI confidence
  status <- "pending"
  if confidence > 0.85
    status <- "answered"
  elif confidence >= 0.6
    status <- "needs_review"
  else
    status <- "escalated"

  # Save the ticket to the database
  ticket = db.ins "tickets" {
    customer_email: email
    subject: subject
    body: body
    category: cat_val
    priority: pri_val
    status: status
    ai_confidence: confidence
  }

  # If high confidence, AI auto-drafts and saves a reply
  if confidence > 0.85
    ai_reply_body = ai_helper.draft_reply subject body cat_val pri_val
    db.ins "replies" {
      ticket: ticket.id
      author: "AI Assistant"
      body: ai_reply_body
      is_ai: true
    }

  ret rep 201 {
    id: ticket.id
    category: ticket.category
    priority: ticket.priority
    status: ticket.status
  }

# GET /tickets — list all tickets with optional status/priority filters
exp fn handle_list_tickets req
  status_filter   = req.query.status
  priority_filter = req.query.priority

  rows <- db.q "select * from tickets order by created desc"

  # Apply status filter if provided
  if status_filter != nil & status_filter != ""
    filtered <- []
    each t in rows
      if t.status == status_filter
        filtered <- filtered + [t]
    rows <- filtered

  # Apply priority filter if provided
  if priority_filter != nil & priority_filter != ""
    filtered2 <- []
    each t in rows
      if t.priority == priority_filter
        filtered2 <- filtered2 + [t]
    rows <- filtered2

  ret rep 200 {tickets: rows}

# GET /tickets/:id — get a single ticket with all its replies
exp fn handle_get_ticket req
  id = req.params.id

  ticket = db.one "select * from tickets where id=$1" [id]
  if ticket == nil
    ret rep 404 {error: "ticket not found"}

  replies = db.q "select * from replies where ticket=$1 order by timestamp asc" [id]

  ret rep 200 {
    ticket: ticket
    replies: replies
  }

# POST /tickets/:id/reply — human agent posts a reply
exp fn handle_add_reply req
  id     = req.params.id
  author = req.body.author
  body   = req.body.body

  # Check ticket exists
  ticket = db.one "select * from tickets where id=$1" [id]
  if ticket == nil
    ret rep 404 {error: "ticket not found"}

  # Save the human reply
  db.ins "replies" {
    ticket: id
    author: author
    body: body
    is_ai: false
  }

  # Mark ticket as answered
  db.up "tickets" {status: "answered"} {id: id}

  ret rep 201 {ok: true}
