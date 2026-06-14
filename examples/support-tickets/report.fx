# Daily report cron

use db

# Open tickets, counts by priority, and AI auto-replies in the last 24 hours
exp fn daily_report
  open = db.q "select count(*) as n from tickets where status != 'answered'"!
  open_n = open.0.n ?? 0

  high = db.q "select count(*) as n from tickets where priority='high'"!
  med  = db.q "select count(*) as n from tickets where priority='medium'"!
  low  = db.q "select count(*) as n from tickets where priority='low'"!

  high_n = high.0.n ?? 0
  med_n = med.0.n ?? 0
  low_n = low.0.n ?? 0

  # Tickets answered automatically by the AI in the last 24 hours
  ai_ans = db.q "select count(distinct ticket) as n from replies where is_ai=true and created > now() - interval '24 hours'"!
  ai_n = ai_ans.0.n ?? 0

  log "Daily report — Open tickets: ${open_n}"
  log "By priority — high: ${high_n}, medium: ${med_n}, low: ${low_n}"
  log "AI auto-replies in the last 24 hours: ${ai_n}"
