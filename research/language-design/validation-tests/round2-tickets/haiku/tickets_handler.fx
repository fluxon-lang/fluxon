# Ticket endpoints

use http db
use ./ai_helper
use ./validation

fn create_ticket req
  # POST /tickets
  body = req.body
  email = body.email
  subject = body.subject
  body_text = body.body

  # Validate input
  validation = validate_ticket_input email subject body_text
  if !validation.ok
    ret rep 400 {error:validation.error}

  # Classify ticket with AI
  classification = classify_ticket subject body_text
  category = classification.category
  priority = classification.priority
  confidence = classification.confidence

  # Determine status based on confidence
  status = nil
  match true
    confidence > 0.85
      status = "answered"
    confidence >= 0.6
      status = "needs_review"
    _
      status = "escalated"

  # Save ticket to database
  ticket = db.ins "tickets" {
    customer_email: email
    subject: subject
    body: body_text
    category: category
    priority: priority
    status: status
    ai_confidence: confidence
  }

  # If confidence high, auto-draft and save reply
  if confidence > 0.85
    draft = auto_reply_draft subject body_text category
    db.ins "replies" {
      ticket_id: ticket.id
      author: "AI Support"
      body: draft
      is_ai: true
    }

  rep 201 {
    id: ticket.id
    category: category
    priority: priority
    status: status
    ai_confidence: confidence
  }

fn list_tickets req
  # GET /tickets with optional filters
  status_filter = req.query.status
  priority_filter = req.query.priority

  query = "select * from tickets"
  params = []
  conditions = []

  if status_filter
    conditions = [... conditions, "status = $" + str.str (params.len + 1)]
    params = [... params, status_filter]

  if priority_filter
    conditions = [... conditions, "priority = $" + str.str (params.len + 1)]
    params = [... params, priority_filter]

  if conditions.len > 0
    where = str.join conditions " and "
    query = query + " where " + where

  query = query + " order by created desc"

  tickets = db.q query params
  rep 200 {tickets: tickets}

fn get_ticket req
  # GET /tickets/:id
  id = req.params.id
  ticket = db.one "select * from tickets where id = $1" [id]

  if !ticket
    ret rep 404 {error:"ticket not found"}

  replies = db.q "select * from replies where ticket_id = $1 order by created asc" [id]

  rep 200 {
    ticket: ticket
    replies: replies
  }

exp create_ticket list_tickets get_ticket
