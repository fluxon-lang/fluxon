# cron.fluxon — scheduled jobs. Hourly report of active channels & message volume.
use db cron

# Compute and log activity for the last hour: per-channel message counts plus a
# total. "Active" = a channel that received at least one message in the window.
exp fn hourly_report
  since = time.ago 1 :hr
  rows = db.q "select c.id, c.name, count(m.id) c from channels c join messages m on m.channel = c.id where m.created > $1 group by c.id, c.name order by c desc" [since]
  total <- 0
  each r in rows
    total <- total + r.c
  log "[cron] hourly report: ${rows.len} active channels, ${total} messages in last hour"
  each r in rows
    log "[cron]   #${r.name} (id=${r.id}): ${r.c} messages"
  ret {active_channels:rows.len total_messages:total}

# Register the schedule: run at minute 0 of every hour.
exp fn install
  cron.hr 0 hourly_report
