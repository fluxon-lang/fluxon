# CORS namoyishi (issue #135) — brauzer frontend'i bilan ishlovchi API.
# Ishga tushirish:  fluxon run examples/cors_server.fx
# Sinash (boshqa terminalda):
#   # preflight (brauzer haqiqiy so'rovdan oldin shuni yuboradi):
#   curl -s -i -X OPTIONS localhost:8080/api/notes -H 'Origin: https://app.example.com'
#   # oddiy so'rov — javobda Access-Control-Allow-Origin bo'ladi:
#   curl -s -i localhost:8080/api/notes -H 'Origin: https://app.example.com'

use http

# Bitta deklaratsiya: preflight + barcha javobga CORS header avtomatik.
# Dev uchun hammaga ochiq:
http.cors "*"
# Prod uchun aniq origin'lar + cookie/Authorization:
#   http.cors ["https://app.example.com"] {creds: true}

http.on :get "/api/notes" \req ->
  rep 200 {notes: ["birinchi" "ikkinchi"]}

http.on :post "/api/notes" \req ->
  rep 201 {received: req.body}

log "CORS server 8080-portda ishga tushmoqda..."
http.serve 8080
