# AI helper for ticket classification

use ai

fn classify_ticket subject body
  # Ask AI to classify the ticket and return structured data
  prompt = "Classify this support ticket:
Subject: ${subject}
Body: ${body}

Extract: category (one of: billing, technical, account, other), priority (low, medium, high), and confidence (0-1).
Return as JSON with keys: category, priority, confidence."

  result = ai.json prompt {category:str priority:str confidence:flt}
  result

fn auto_reply_draft subject body category
  # Generate an auto-reply based on category
  prompt = "Write a short, professional support reply for this ticket category: ${category}
Subject: ${subject}
Body: ${body}

Keep it to 2-3 sentences. Be helpful but acknowledge we'll investigate further if needed."

  reply = ai.ask prompt
  reply

exp classify_ticket auto_reply_draft
