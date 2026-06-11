# Static fayl tarqatish namoyishi (issue #134) — frontend build + API birga.
# Ishga tushirish:  fluxon run examples/static_server.fx
# (yon papka examples/public/ namuna fayllar bilan birga keladi)
# Sinash (boshqa terminalda):
#   curl -s -i localhost:8080/assets/app.css     # fayl + Content-Type avtomatik
#   curl -s -i localhost:8080/                   # SPA: dist/index.html
#   curl -s -i localhost:8080/api/health         # aniq route static'dan ustun

use http

# Prefiks -> papka: /assets/app.css -> ./public/app.css
# Katalog skript fayli joylashgan papkaga nisbatan hal qilinadi.
http.static "/assets" "./public"

# SPA rejimi: topilmagan yo'l ./dist/index.html ga tushadi
# (frontend router o'zi hal qiladi). Aniq route'lar baribir ustun.
# http.static "/" "./dist" {spa: true}

http.on :get "/api/health" \req ->
  rep 200 {ok: true}

log "Static server 8080-portda ishga tushmoqda..."
http.serve 8080
