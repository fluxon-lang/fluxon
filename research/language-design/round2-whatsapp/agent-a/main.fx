# Wire it all together: DB + webhook server + cron + reply queue.
use http db q
use ./webhook
use ./cron
use ./tools

# DB connects from $DATABASE_URL automatically on first query — nothing to do.

# Incoming provider webhook.
http.on :post "/wa/webhook" webhook.handle

# Background send queue so the webhook responds fast.
q.on "send" \job -> tools.send job.ph job.body

# Register weekly outreach + Sunday briefing.
cron.register

log "Fluxon ops assistant up on :8080"
http.serve 8080
