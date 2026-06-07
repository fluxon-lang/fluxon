# Flux Frontend — 3-BOSQICH misoli: page routing + ui.serve (SSR).
#
# `page "/yo'l" -> view` URL'ni view'ga bog'laydi. `ui.serve port` bitta portda
# HTML sahifa (page) + REST API (http.on) beradi. Brauzer http://localhost:3777/
# ochsa SSR HTML (theme CSS bilan) keladi.
#
# DIQQAT: bu fayl portni ochib BLOKLAYDI (server) — smoke-test uchun emas.
# Ishga tushirish: cargo run -- run examples/ui_serve.fx, keyin brauzerda och.

theme
  primary "#e84d8a"
  radius  :lg
  muted   "#888"

view home
  h1 "Gulzor"
  p "Eng yaxshi gullar shu yerda" {kind::muted}
  each g in ["Atirgul" "Lola" "Chinnigul"]
    div {kind::panel}
      h2 g

# 1-param view — req (params/query) oladi.
view product req
  h1 "Mahsulot #${req.params.id}"
  p "Tafsilot sahifasi"

# REST API — UI bilan BIR portda (/api prefiksli).
http.on :get "/api/health" \req -> rep 200 {ok:true}

# page marshrutlari (URL = sahifa).
page "/" -> home
page "/product/:id" \req -> product req

# Bitta port: SSR sahifa + REST + (kelajakda) WS.
ui.serve 3777
