# CORS demonstration (issue #135) — an API that works with a browser frontend.
# Running it:  fluxon run examples/cors_server.fx
# Testing (in another terminal):
#   # preflight (the browser sends this before the real request):
#   curl -s -i -X OPTIONS localhost:8080/api/notes -H 'Origin: https://app.example.com'
#   # a simple request — the response will contain Access-Control-Allow-Origin:
#   curl -s -i localhost:8080/api/notes -H 'Origin: https://app.example.com'

use http

# One declaration: preflight + CORS header on every response automatically.
# Open to everyone for dev:
http.cors "*"
# For prod, explicit origins + cookie/Authorization:
#   http.cors ["https://app.example.com"] {creds: true}

http.on :get "/api/notes" \req ->
  rep 200 {notes: ["first" "second"]}

http.on :post "/api/notes" \req ->
  rep 201 {received: req.body}

log "CORS server starting on port 8080..."
http.serve 8080
