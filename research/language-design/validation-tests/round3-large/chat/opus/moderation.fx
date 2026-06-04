# moderation.flux — AI features: auto-moderation classifier + channel summary.
# Uses the ai battery (ai.json for structured classification, ai.ask for summary).
use ai db

# Classify a message body for moderation.
# Returns a map: {label:sym confidence:flt reason:str}
# label is one of :toxic :spam :ok
fn classify body
  r = ai.json "Classify this chat message for moderation. Decide if it is toxic, spam, or ok. Message: ${body}" {
    label: ":toxic|:spam|:ok"
    confidence: "flt"
    reason: "str"
  }
  ret r

# Decide moderation outcome for a message body.
# Returns one of:
#   {action::block label:.. confidence:..}   — toxic + high confidence
#   {action::flag  label:.. confidence:..}   — medium confidence problem
#   {action::allow label:.. confidence:..}   — fine
# We combine the model's own self-reported confidence (r.confidence) with the
# ai metadata confidence (r._.conf) and use the lower of the two to be safe.
exp fn moderate body
  r = classify body
  self_conf = r.confidence ?? 0.0
  meta_conf = r._.conf ?? 0.0
  conf <- self_conf
  if meta_conf < conf
    conf <- meta_conf
  label = r.label ?? :ok

  if label == :ok
    ret {action::allow label:label confidence:conf reason:(r.reason ?? "")}

  # label is :toxic or :spam — route by confidence.
  if conf > 0.85
    ret {action::block label:label confidence:conf reason:(r.reason ?? "")}
  elif conf >= 0.6
    ret {action::flag label:label confidence:conf reason:(r.reason ?? "")}
  else
    # Low confidence → let it through but record the label.
    ret {action::allow label:label confidence:conf reason:(r.reason ?? "")}

# Summarize the last n messages of a channel into a short paragraph.
exp fn summarize_channel channel_id n
  rows = db.q "select m.body, u.username from messages m join users u on u.id = m.user where m.channel = $1 and m.status != 'blocked' order by m.created desc limit $2" [channel_id n]
  if rows.len == 0
    ret {summary:"No messages to summarize." count:0}
  # rows are newest-first; prepend each so the transcript ends up oldest-first.
  lines <- ""
  each r in rows
    lines <- "${r.username}: ${r.body}\n" + lines
  summary = ai.ask "Summarize the following chat conversation in 2-4 sentences. Focus on topics and decisions.\n\n${lines}"
  ret {summary:summary count:rows.len}
