# Cron demo — scheduled background tasks together with an HTTP server.
#
# cron.on does not block anything (like http.on it only registers). The last
# blocking call written (here http.serve) keeps the process alive; cron runs
# on schedule in the background.
#
# If there is NO server, write cron.run instead of http.serve (to keep the
# process alive). cron.run and http.serve/ws.serve also work TOGETHER in any
# order — neither blocks the other.

use http

# Background task: leaves a mark once a minute (fast interval for the demo).
fn tick
  log "cron: per-minute tick"

# Background task: a daily report every morning at 9:00.
fn daily
  log "cron: daily report (09:00)"

# Register the scheduled tasks — does not block.
cron.on * * * * * tick           # every minute
cron.on 0 9 * * * daily          # every day at 09:00
cron.on 30 9 * * 1-5 \->          # weekdays at 09:30 (inline lambda)
  log "cron: weekday reminder"

# Simple HTTP endpoint.
http.on :get "/" \req -> rep 200 {ok:true msg:"cron demo is running"}

# The server keeps the process alive; cron ticks in the background.
http.serve 8080

# --- If there is NO server, only cron, it would be written like this: ---
# cron.on * * * * * tick
# cron.run     # takes over the process (blocks)
