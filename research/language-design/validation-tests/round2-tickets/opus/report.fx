# Kunlik hisobot cron'i

use db

# Ochiq ticketlar, ustuvorlik bo'yicha soni, so'nggi 24 soatda AI auto-javob soni
exp fn daily_report
  open = db.q "select count(*) as n from tickets where status != 'answered'"!
  open_n = open.0.n ?? 0

  high = db.q "select count(*) as n from tickets where priority='high'"!
  med  = db.q "select count(*) as n from tickets where priority='medium'"!
  low  = db.q "select count(*) as n from tickets where priority='low'"!

  high_n = high.0.n ?? 0
  med_n = med.0.n ?? 0
  low_n = low.0.n ?? 0

  # So'nggi 24 soatda AI tomonidan avtomatik javob berilgan ticketlar
  ai_ans = db.q "select count(distinct ticket) as n from replies where is_ai=true and created > now() - interval '24 hours'"!
  ai_n = ai_ans.0.n ?? 0

  log "Kunlik hisobot — Ochiq ticketlar: ${open_n}"
  log "Ustuvorlik bo'yicha — high: ${high_n}, medium: ${med_n}, low: ${low_n}"
  log "So'nggi 24 soatda AI auto-javob: ${ai_n}"
