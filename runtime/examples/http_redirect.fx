# HTTP klient: response header'lari + redirect kuzatuvi misoli (issue #13).
#
# Ikki yondashuv ko'rsatiladi:
#   1) follow:true — klient redirectni o'zi kuzatadi (kam kod).
#   2) Qo'lda loop — har hop nazorat ostida (res.headers.location o'qib).
#
# Ishga tushirish: redirect qaytaruvchi server kerak. Soddalik uchun shu fayl
# o'z ichida server ko'taradi (/short → /long), keyin klient bilan tekshiradi.
# Real holatda url tashqi bo'ladi (bit.ly va h.k.).

use http

# --- demonstratsion server: /short 302 bilan /long'ga yo'naltiradi ---
http.on :get "/short" \req ->
  rep 302 {location:"/long"}
http.on :get "/long" \req ->
  rep 200 {final:true msg:"manzilga yetdik"}

# Server alohida thread'da; klient sinovini shu jarayonda ham ishga tushirib
# bo'lmaydi (http.serve bloklaydi), shuning uchun bu fayl SERVER sifatida ishlaydi.
# Klient qismini boshqa terminalda quyidagicha sinang:
#
#   res = http.get "http://127.0.0.1:8088/short" {follow:true}
#   log "status=${res.status} hops=${res.hops} body=${res.body.msg}"
#
#   # qo'lda kuzatish (hops'ni o'zingiz sanaysiz):
#   r = http.get "http://127.0.0.1:8088/short"
#   if r.status >= 300 & r.status < 400
#     loc = r.headers.location
#     log "redirect → ${loc}"

log "redirect demo server 8088-portda..."
http.serve 8088
