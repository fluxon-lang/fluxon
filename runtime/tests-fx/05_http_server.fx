# 05 - http server. Routes, params, query, body (JSON), rep, fail.
# Runs blocking; the client (05_http_client.fx) tests it.
use http

# in-memory state (mutable map)
store <- {n:0}

http.on :get "/health" \req ->
  rep 200 {ok:true}

# path parameter
http.on :get "/echo/:id" \req ->
  rep 200 {id:req.params.id}

# query string
http.on :get "/q" \req ->
  rep 200 {name:(req.query.name ?? "none")}

# POST + JSON body -> read as a map
http.on :post "/sum" \req ->
  a = req.body.a
  b = req.body.b
  rep 201 {sum:(a + b)}

# guard-clause: ret + fail inside lambda (error response)
http.on :post "/strict" \req ->
  if !req.body.email
    ret fail 400 "email required"
  rep 201 {email:req.body.email}

# str response (plain text, not JSON)
http.on :get "/text" \req ->
  rep 200 "hello world"

# echoes the request headers (verifies the client sends custom headers).
# req.headers keys are lowercase + '-' -> '_' (x-api-key -> x_api_key).
http.on :get "/echo-headers" \req ->
  rep 200 {key:(req.headers.x_api_key ?? "none") ver:(req.headers.anthropic_version ?? "none")}

# custom response headers (issue #16): 3rd-argument headers map.
# `_` -> `-` (content_type -> Content-Type), name is case-insensitive.
http.on :get "/html" \req ->
  rep 200 "<h1>Hello</h1>" {content_type:"text/html" x_powered_by:"fluxon"}

# repeated Set-Cookie: list value -> each element is a separate header line.
http.on :get "/cookies" \req ->
  rep 200 {ok:true} {set_cookie:["a=1" "b=2"]}

# redirect chain: /r1 -> /r2 -> /dest (client follows with follow:true).
http.on :get "/r1" \req ->
  rep 302 {location:"/r2"}
http.on :get "/r2" \req ->
  rep 302 {location:"/dest"}
http.on :get "/dest" \req ->
  rep 200 {arrived:true}

log "server starting on port 8123..."
http.serve 8123
