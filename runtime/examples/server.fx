# HTTP battery demonstration.
# Running:           fluxon run examples/server.fx
# Testing (in another terminal):
#   curl -s localhost:8080/health
#   curl -s -X POST localhost:8080/notes -H 'Content-Type: application/json' -d '{"title":"hello"}'
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
  fail 422 "bad request"

log "server starting on port 8080..."
http.serve 8080
