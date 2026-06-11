# ai_helper.fluxon — AI classification and auto-reply helpers

use ai

# Classify a ticket by subject and body.
# Returns a map with category, priority, and confidence metadata.
exp fn classify_ticket subject body
  prompt = "You are a customer support classifier. Given the following support ticket, classify it.\n\nSubject: ${subject}\nBody: ${body}\n\nRespond with the category (billing, technical, account, or other) and priority (low, medium, or high)."
  r = ai.json prompt {
    category: str
    priority: str
  }
  ret r

# Generate an auto-reply for a ticket given its category and body.
exp fn draft_reply subject body category priority
  prompt = "You are a helpful customer support agent. Write a friendly, professional reply to the following support ticket.\n\nCategory: ${category}\nPriority: ${priority}\nSubject: ${subject}\nCustomer message: ${body}\n\nWrite a clear, empathetic response that addresses their concern."
  txt = ai.ask prompt
  ret txt
