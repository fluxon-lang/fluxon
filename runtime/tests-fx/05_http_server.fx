# 05 — http server. Marshrutlar, params, query, body (JSON), rep, fail.
# Bloklab ishlaydi; klient (05_http_client.fx) test qiladi.
use http

# in-memory holat (mutable map)
store <- {n:0}

http.on :get "/health" \req ->
  rep 200 {ok:true}

# yo'l parametri
http.on :get "/echo/:id" \req ->
  rep 200 {id:req.params.id}

# query string
http.on :get "/q" \req ->
  rep 200 {name:(req.query.name ?? "yo'q")}

# POST + JSON body → map sifatida o'qiladi
http.on :post "/sum" \req ->
  a = req.body.a
  b = req.body.b
  rep 201 {sum:(a + b)}

# guard-clause: lambda ichida ret + fail (xato javob)
http.on :post "/strict" \req ->
  if !req.body.email
    ret fail 400 "email kerak"
  rep 201 {email:req.body.email}

# str javob (JSON emas, matn)
http.on :get "/text" \req ->
  rep 200 "salom dunyo"

# redirect zanjiri: /r1 → /r2 → /dest (klient follow:true bilan kuzatadi).
http.on :get "/r1" \req ->
  rep 302 {location:"/r2"}
http.on :get "/r2" \req ->
  rep 302 {location:"/dest"}
http.on :get "/dest" \req ->
  rep 200 {arrived:true}

log "server 8123-portda ishga tushyapti..."
http.serve 8123
