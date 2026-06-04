# Scheduled cron jobs

use db cron log

fn daily_ticket_stats
  # Daily stats at 08:00

  # Count open/pending tickets
  open_result = db.one "select count(*) as count from tickets where status in ('needs_review', 'escalated')" []
  open_count = open_result.count ?? 0

  # Count by priority
  priority_result = db.q "select priority, count(*) as cnt from tickets group by priority" []
  priority_str = ""
  each p in priority_result
    priority_str = priority_str + " " + p.priority + ":" + str.str (p.cnt)

  # Count AI auto-answered in last 24h
  ai_answered = db.one "select count(*) as count from tickets where status = 'answered' and ai_confidence > 0.85 and created > now() - interval '24 hours'" []
  ai_count = ai_answered.count ?? 0

  log "=== Daily Ticket Report ==="
  log "Open tickets: ${open_count}"
  log "By priority:${priority_str}"
  log "AI auto-answered (24h): ${ai_count}"

fn schedule_cron_jobs
  # Register daily stats at 08:00
  cron.dy 8 0 daily_ticket_stats

exp schedule_cron_jobs
