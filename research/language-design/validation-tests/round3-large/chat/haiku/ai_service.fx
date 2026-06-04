# AI-powered features: summarization and moderation

use ai
use db

# Check message for toxicity/spam using ai.json
exp fn check_message_moderation body
  result = ai.json "
    Classify this message. Respond with confidence 0-1.
    Return: action (block, flag, ok), reason, confidence.
    Message: ${body}
  " {
    action: ":block|:flag|:ok"
    reason: "str"
    confidence: "flt"
  }

  # confidence is in result._.conf, but we also requested confidence field
  conf = result.confidence ?? result._.conf ?? 0.5

  action <- result.action
  if conf > 0.85 & (action == :block | action == :flag)
    # High confidence — enforce
    ret {
      action: action
      reason: result.reason ?? "Auto-moderation"
      confidence: conf
    }
  elif conf >= 0.6 & action == :block
    # Medium confidence — flag instead of block
    ret {
      action: :flag
      reason: result.reason ?? "Potential violation (medium confidence)"
      confidence: conf
    }
  else
    # Low confidence or :ok — let through
    ret {
      action: :ok
      reason: result.reason ?? "Passed moderation"
      confidence: conf
    }

# Summarize messages in a channel
exp fn summarize_channel channel_id last_n
  n = last_n ?? 50

  # Fetch recent messages
  messages = db.q "
    select u.username, m.body
    from messages m
    join users u on m.user = u.id
    where m.channel = $1
    order by m.created desc
    limit $2
  " [channel_id n]

  # Build context string
  context <- ""
  each msg in messages
    context <- context + "${msg.username}: ${msg.body}\n"

  if str.len context > 5000
    context <- str.slice context 0 5000

  # Ask AI for summary
  summary = ai.ask "
    Summarize this chat conversation in 2-3 sentences:
    ${context}
  "

  ret {
    channel_id: channel_id
    message_count: messages.len
    summary: summary
    generated: time.now
  }

# Generate topic suggestions based on messages
exp fn get_channel_topics channel_id last_n
  n = last_n ?? 100

  messages = db.q "
    select m.body
    from messages m
    where m.channel = $1
    order by m.created desc
    limit $2
  " [channel_id n]

  context <- ""
  each msg in messages
    context <- context + "${msg.body} "

  if str.len context > 3000
    context <- str.slice context 0 3000

  topics = ai.json "
    Extract 3-5 main topics from this conversation:
    ${context}
  " {
    topics: "[str]"
    confidence: "flt"
  }

  ret topics

# Detect spam/bot patterns
exp fn detect_spam_user user_id hours
  recent = db.q "
    select count(*) as cnt, count(distinct channel) as channels
    from messages
    where user = $1 and created > $2
  " [user_id time.ago hours :hr]

  row = recent.0 ?? {cnt:0 channels:0}
  msg_count = row.cnt ?? 0
  channel_count = row.channels ?? 0

  # Heuristic: > 50 messages in 1 hour = likely spam
  # Or: > 10 channels in 1 hour = likely spam
  is_spam = msg_count > 50 | channel_count > 10

  ret {
    user_id: user_id
    message_count: msg_count
    channel_count: channel_count
    is_likely_spam: is_spam
    checked: time.now
  }
