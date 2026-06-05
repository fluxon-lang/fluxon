# 05 — http klient: 05_http_server.fx ga so'rovlar yuborib javoblarni tekshiradi.
# Server 8123-portda ishlab turishi kerak.
use http

fails <- 0
fn eq got want label
  if got == want
    log "ok  ${label} = ${got}"
  else
    log "FAIL ${label}: got=${got} want=${want}"
    fails <- fails + 1

base = "http://127.0.0.1:8123"

# GET /health → {ok:true}
r = http.get "${base}/health"
eq r.status 200 "GET /health status"
eq r.body.ok true "GET /health body.ok"

# GET /echo/:id → param
r2 = http.get "${base}/echo/42"
eq r2.status 200 "GET /echo/:id status"
eq r2.body.id "42" "GET /echo param"

# GET /q?name=Ali → query
r3 = http.get "${base}/q?name=Ali"
eq r3.body.name "Ali" "GET query param"

# query'siz → default
r4 = http.get "${base}/q"
eq r4.body.name "yo'q" "GET query missing → default"

# POST /sum {a,b} → JSON body o'qiladi
r5 = http.post "${base}/sum" {a:7 b:8}
eq r5.status 201 "POST /sum status 201"
eq r5.body.sum 15 "POST /sum body parsed (7+8)"

# POST /strict bo'sh → fail 400
r6 = http.post "${base}/strict" {}
eq r6.status 400 "POST /strict missing email → 400"
eq r6.body.error "email kerak" "fail message in body.error"

# POST /strict email bilan → 201
r7 = http.post "${base}/strict" {email:"a@b.uz"}
eq r7.status 201 "POST /strict valid → 201"
eq r7.body.email "a@b.uz" "POST /strict echoes email"

# GET /text → matn javob (JSON emas)
r8 = http.get "${base}/text"
eq r8.status 200 "GET /text status"
eq r8.body "salom dunyo" "GET /text plain body"

# res.headers — javob header'lari map sifatida o'qiladi (kichik harf kalit)
# tireli kalit (content-type) uchun m[k] indeks ishlatamiz (m.k emas).
r9 = http.get "${base}/health"
ct = r9.headers["content-type"]
eq (str.has ct "application/json") true "res.headers content-type"

# follow:false (default) — 302 xom qaytadi, Location header o'qiladi
r10 = http.get "${base}/r1"
eq r10.status 302 "default follow off → xom 302"
eq r10.headers.location "/r2" "res.headers.location o'qildi"

# follow:true — redirect zanjiri kuzatiladi (/r1 → /r2 → /dest)
r11 = http.get "${base}/r1" {follow:true}
eq r11.status 200 "follow:true → yakuniy 200"
eq r11.body.arrived true "follow:true → /dest tanasi"
eq r11.hops 2 "follow:true → 2 hop sanaldi"

if fails == 0
  log "=== 05_http: HAMMASI O'TDI ==="
else
  log "=== 05_http: ${fails} TEST YIQILDI ==="
