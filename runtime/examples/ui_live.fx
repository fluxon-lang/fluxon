# Flux Frontend — PR-7b misoli: `source live` WS real-time (BIR PORTDA).
#
# `orders <- source live db.q "..."` — live source server WS kanaliga (tag nomi
# bilan) avto-subscribe. Server `ui.push :orders` chaqirsa, BARCHA ulangan
# klientlarning :orders source'i qayta yuklanadi (client WS kod YO'Q).
#
# BIR PORTDA: ui.serve bitta portda SSR sahifa + /_fx/ws (WS upgrade) + REST.
# Alohida ws.serve YO'Q. Client.js o'zi /_fx/ws ga ulanib subscribe qiladi.
#
# DIQQAT: portni ochib BLOKLAYDI (server) — smoke-test emas.
# Ishga tushirish:
#   DATABASE_URL="sqlite::memory:" cargo run -- run examples/ui_live.fx
# Ikki brauzer oynasi och (http://localhost:3780). Birinchisida "+ Buyurtma"
# bos -> ikkala oyna ham avto-yangilanadi (WS push, real-time).

use db ui

tbl ord
  id   serial pk
  name str
  ts   now

db.ins "ord" {name:"Atirgul buketi"}
db.ins "ord" {name:"Lola savati"}

theme
  primary "#e84d8a"
  radius  :lg

view shop
  # live source: WS kanaliga (:orders) avto-subscribe. ui.push :orders -> reload.
  orders <- source live db.q "select * from ord order by id desc"

  h1 "Buyurtmalar — ${orders.data.len} ta"
  div {kind::panel}
    each o in orders.data
      div {kind::row}
        span o.name

# REST: yangi buyurtma qo'shadi va BARCHA klientlarga push qiladi.
http.on :post "/api/order" \req ->
  db.ins "ord" {name: req.body.name ?? "Yangi buyurtma"}
  ui.push :orders                      # BARCHA klientlar :orders source'i reload
  rep 201 {ok: true}

page "/" -> shop

# Bitta port: SSR + /_fx/ws (WS) + /api/* REST.
ui.serve 3780
