# HTTP client: response headers + redirect following example (issue #13).
#
# Two approaches are shown:
#   1) follow:true — the client follows the redirect itself (less code).
#   2) Manual loop — every hop under control (reading res.headers.location).
#
# Running: a server returning a redirect is needed. For simplicity this file
# brings up its own server (/short -> /long), then checks it with the client.
# In real cases the url would be external (bit.ly etc.).

use http

# --- demonstration server: /short redirects to /long with 302 ---
http.on :get "/short" \req ->
  rep 302 {location:"/long"}
http.on :get "/long" \req ->
  rep 200 {final:true msg:"reached the destination"}

# The server is on a separate thread; the client test cannot be run in this
# same process (http.serve blocks), so this file runs as a SERVER. Try the
# client part in another terminal like this:
#
#   res = http.get "http://127.0.0.1:8088/short" {follow:true}
#   log "status=${res.status} hops=${res.hops} body=${res.body.msg}"
#
#   # manual following (you count the hops yourself):
#   r = http.get "http://127.0.0.1:8088/short"
#   if r.status >= 300 & r.status < 400
#     loc = r.headers.location
#     log "redirect -> ${loc}"

log "redirect demo server on port 8088..."
http.serve 8088
