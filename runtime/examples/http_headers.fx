# Custom javob header'lari (issue #16) — rep'ning ixtiyoriy 3-argument map'i.
#
# Header nomida defis o'rniga `_` yoziladi (Flux map kalitida defis bo'lolmaydi);
# runtime uni `-` ga aylantiradi: content_type → Content-Type. Nom case-insensitive.
# Bu o'qish bilan simmetrik — req.headers'da ham `-` → `_`.
#
# Server bloklaydi; ishga tushirish: cargo run -- run examples/http_headers.fx
# keyin boshqa terminalda: curl -i http://127.0.0.1:8080/html
use http

# 1) Maxsus Content-Type — HTML qaytarish (body str bo'lsa default text/plain).
http.on :get "/html" \req ->
  rep 200 "<h1>Salom Flux</h1>" {content_type:"text/html"}

# 2) Redirect — Location header (302). URL qisqartiruvchi naqshi.
http.on :get "/go" \req ->
  rep 302 nil {location:"https://example.com"}

# 3) Cookie/sessiya o'rnatish. Bir nechta cookie — list qiymat (har biri
#    alohida Set-Cookie qatori; RFC 7230 vergulli ro'yxatni man qiladi).
http.on :get "/login" \req ->
  rep 200 {ok:true} {set_cookie:["session=abc123" "theme=dark"]}

# 4) CSV eksport — maxsus Content-Type + yuklab olish nomi.
http.on :get "/export" \req ->
  rep 200 "id,nom\n1,Ali\n2,Vali" {
    content_type:"text/csv"
    content_disposition:"attachment; filename=\"data.csv\""
  }

log "http://127.0.0.1:8080 — /html /go /login /export"
http.serve 8080
