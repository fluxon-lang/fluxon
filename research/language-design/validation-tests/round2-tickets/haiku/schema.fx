# Database schemas for customer support ticket system

tbl tickets
  id          serial pk
  customer_email str
  subject     str
  body        str
  category    str
  priority    str
  status      str
  ai_confidence flt
  created     now

tbl replies
  id        serial pk
  ticket_id int ref:tickets.id
  author    str
  body      str
  is_ai     bool
  created   now
