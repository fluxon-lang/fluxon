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

# so'rov header'larini echo qiladi (klient custom header yuborishini tekshiradi).
# req.headers kalitlari kichik harf + '-' → '_' (x-api-key → x_api_key).
http.on :get "/echo-headers" \req ->
  rep 200 {key:(req.headers.x_api_key ?? "yo'q") ver:(req.headers.anthropic_version ?? "yo'q")}

# custom javob header'lari (issue #16): 3-argument headers map.
# `_` → `-` (content_type → Content-Type), nom case-insensitive.
http.on :get "/html" \req ->
  rep 200 "<h1>Salom</h1>" {content_type:"text/html" x_powered_by:"flux"}

# takror Set-Cookie: list qiymat → har element alohida sarlavha qatori.
http.on :get "/cookies" \req ->
  rep 200 {ok:true} {set_cookie:["a=1" "b=2"]}

# redirect zanjiri: /r1 → /r2 → /dest (klient follow:true bilan kuzatadi).
http.on :get "/r1" \req ->
  rep 302 {location:"/r2"}
http.on :get "/r2" \req ->
  rep 302 {location:"/dest"}
http.on :get "/dest" \req ->
  rep 200 {arrived:true}

log "server 8123-portda ishga tushyapti..."
http.serve 8123
