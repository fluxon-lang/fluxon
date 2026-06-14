# AI helper — classify a ticket and draft a reply

use ai

# Classifies a ticket: category, priority, confidence level.
# Returns the r._.conf metadata merged into the result map.
exp fn classify subject body
  r = ai.json "Classify the following support request. subject: ${subject}. body: ${body}" {category::other priority::medium}
  ret {category:r.category priority:r.priority conf:r._.conf}

# AI drafts an automatic reply
exp fn draft_reply subject body
  ret ai.ask "Write a professional, concise support reply to the following customer request. Subject: ${subject}. Request: ${body}"
