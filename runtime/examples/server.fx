# HTTP battery namoyishi.
# Ishga tushirish:  fluxon run examples/server.fx
# Sinash (boshqa terminalda):
#   curl -s localhost:8080/health
#   curl -s -X POST localhost:8080/notes -H 'Content-Type: application/json' -d '{"title":"salom"}'
#   curl -s localhost:8080/notes/42
#   curl -s -i localhost:8080/boom

use http

http.on :get "/health" \req ->
  rep 200 {ok:true}

http.on :post "/notes" \req ->
  rep 201 {received:req.body}

http.on :get "/notes/:id" \req ->
  rep 200 {id:req.params.id method:req.method}

http.on :get "/boom" \req ->
  fail 422 "yomon so'rov"

log "server 8080-portda ishga tushmoqda..."
http.serve 8080
