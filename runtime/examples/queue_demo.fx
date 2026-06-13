# Queue demo — background jobs together with an HTTP server.
#
# Philosophy: a webhook must respond FAST. We push heavy work (sending SMS,
# preparing a report) to the background — `queue.push` returns immediately, the
# work runs sequentially (FIFO) on the background worker thread.
#
# `queue.on` registers a handler (non-blocking), `queue.push` adds work
# (non-blocking). A handler takes a single `job` argument — the payload given
# to `queue.push` (a map). The last blocking call (here http.serve) keeps the
# process alive; the worker processes the queue in the background.

use http queue

# Worker: jobs named "send" run here. job — the pushed payload.
queue.on "send" \job ->
  log "sending SMS -> ${job.ph}: ${job.body}"

# Worker: heavy job named "report" (e.g. report generation).
queue.on "report" \job ->
  log "preparing report: ${job.kind}"

# Webhook: hands the incoming request to the queue and responds IMMEDIATELY.
# The client does not wait — heavy work runs in the background.
http.on :post "/notify" \req ->
  queue.push "send" {ph:req.body.ph body:req.body.text}
  rep 202 {queued:true}

# Another endpoint: hands a report to the background.
http.on :post "/report" \req ->
  queue.push "report" {kind:req.body.kind}
  rep 202 {queued:true}

# The server keeps the process alive; the worker processes the queue in the background.
http.serve 8080

# --- NO server, queue only: to keep the process alive after the pushes you
# need a blocking call. Use one of http.serve / ws.serve / cron.run.
