# Flux Frontend — PR-6 misoli: on:click handler-effekt (server-driven, STATELESS).
#
# `btn "+1" {on:\-> count <- count+1}` — "+1" bosilganda count serverda oshadi.
# AVTOMATIK AJRATISH: `count <- 0` + on:click -> div CLIENT ISLAND (interaktiv),
# h1 SOF SSR (0 JS, statik). Dasturchi hech narsa belgilamaydi.
#
# Mexanizm (STATELESS): client "+1" bosadi -> POST /_fx/event {handler:"#0", state}
# -> server handler tanasini (count <- count+1) view scope'da bajaradi -> island
# re-render -> yangi count HTML'da + data-fx-state (server RAM'da state SAQLAMAYDI).
#
# DIQQAT: bu fayl portni ochib BLOKLAYDI (server) — smoke-test uchun emas.
# Ishga tushirish: cargo run -- run examples/ui_counter.fx, keyin brauzerda och,
# "+1" tugmasini bos -> son oshadi (har bosish = 1 HTTP, holatsiz).

theme
  primary "#4f46e5"
  radius  :lg

view counter
  count <- 0
  h1 "Hisoblagich"
  p "Statik sarlavha — 0 JS (CDN-cacheable)" {kind::muted}
  div {kind::panel}
    p "Joriy son: ${count}"
    btn "+1" {on:\-> count <- count+1}
    btn "-1" {on:\-> count <- count-1}

page "/" -> counter

# Bitta port: SSR sahifa + /_fx/event (server-driven handler).
ui.serve 3778
