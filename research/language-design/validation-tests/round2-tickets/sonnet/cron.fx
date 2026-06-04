# cron.flux — scheduled daily report job

use db cron

exp fn register_cron
  cron.dy 8 0 daily_report

fn daily_report
  # Total open tickets (not yet answered or closed)
  open_rows = db.q "select count(*) as cnt from tickets where status != 'answered'"
  total_open = open_rows.0.cnt ?? 0
  log "Daily report — open tickets: ${total_open}"

  # Count by priority
  low_rows    = db.q "select count(*) as cnt from tickets where status != 'answered' and priority = 'low'"
  medium_rows = db.q "select count(*) as cnt from tickets where status != 'answered' and priority = 'medium'"
  high_rows   = db.q "select count(*) as cnt from tickets where status != 'answered' and priority = 'high'"

  low_count    = low_rows.0.cnt ?? 0
  medium_count = medium_rows.0.cnt ?? 0
  high_count   = high_rows.0.cnt ?? 0

  log "  Priority breakdown — low: ${low_count}  medium: ${medium_count}  high: ${high_count}"

  # AI auto-answered in the last 24 hours
  ai_rows = db.q "select count(*) as cnt from tickets where status = 'answered' and ai_confidence > 0.85 and created >= now() - interval '24 hours'"
  ai_answered = ai_rows.0.cnt ?? 0
  log "  AI auto-answered (last 24h): ${ai_answered}"
