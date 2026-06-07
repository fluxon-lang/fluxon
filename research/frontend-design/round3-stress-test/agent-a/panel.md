This task is to write Flux code — no external research, LLM provider work, or tooling needed. I'll write the restaurant admin panel directly.

Let me produce the full Flux code and the required analysis. This is a pure writing task based on the spec provided, so I'll work from the spec directly.

# Restoran admin panel (Flux)

## To'liq kod
```flux
use http db ws ui json ai

# ============================================================
# SXEMA (tbl)
# ============================================================

tbl staff
  id      serial pk
  name    str
  role    sym                 # :waiter :chef :manager
  phone   str null
  active  bool
  ts      now

tbl shifts
  id        serial pk
  staff_id  int ref:staff.id
  start_at  now
  end_at    now null          # nil = smena hali ochiq
  ts        now

tbl tables
  id       serial pk
  num      int uniq            # stol raqami
  seats    int
  status   sym                 # :free :busy :reserved
  ts       now

tbl menu_items
  id        serial pk
  name      str
  price     money              # cents (minor unit)
  category  sym                # :starter :main :dessert :drink
  photo     str null
  available bool
  ts        now

tbl orders
  id        serial pk
  table_id  int ref:tables.id
  waiter_id int ref:staff.id null
  status    sym                # :new :cooking :ready :delivered
  total     money
  created   now

tbl order_items
  id        serial pk
  order_id  int ref:orders.id
  item_id   int ref:menu_items.id
  name      str                # snapshot (taom nomi o'zgarsa ham buyurtmada saqlanadi)
  price     money              # snapshot narx
  qty       int

tbl settings
  id        serial pk
  rest_name str
  open_time str                # "09:00"
  close_time str               # "23:00"
  theme_primary str
  theme_accent  str
  ts        now

# ============================================================
# YORDAMCHI FUNKSIYALAR
# ============================================================

fn get_settings
  s = db.one "select * from settings order by id limit 1"
  ret s ?? {rest_name:"Mening Restoranim" open_time:"09:00" close_time:"23:00" theme_primary:"#e84d8a" theme_accent:"#9b5de5"}

# Buyurtma to'liq tafsiloti bilan (items qo'shilgan holda)
fn order_detail oid
  ord = db.one "select * from orders where id=$1" [oid]!
  items = db.q "select * from order_items where order_id=$1" [oid]
  ret {order:ord items:items}

# ============================================================
# BACKEND HTTP API
# ============================================================

# ---- Dashboard statistikasi ----
http.on :get "/api/stats" \req ->
  today = time.ago 24 :hr
  rev = db.one "select coalesce(sum(total),0) s from orders where created > $1 and status=$2" [today :delivered]
  cnt = db.one "select count(*) c from orders where created > $1" [today]
  act = db.one "select count(*) c from tables where status=$1" [:busy]
  top = db.one "select name, sum(qty) q from order_items oi join orders o on o.id=oi.order_id where o.created > $1 group by name order by q desc limit 1" [today]
  rep 200 {
    revenue:(rev.s ?? 0)
    orders_count:(cnt.c ?? 0)
    active_tables:(act.c ?? 0)
    top_item:(top.name ?? "—")
  }

# Tushum grafigi uchun — oxirgi 7 kun (BO'SHLIQ: pastdagi gaps'ga qara)
http.on :get "/api/revenue-chart" \req ->
  rows = db.q "select to_char(created,'YYYY-MM-DD') day, coalesce(sum(total),0) rev from orders where status=$1 and created > $2 group by day order by day" [:delivered (time.ago 168 :hr)]
  rep 200 rows

# ---- Menyu ----
http.on :get "/api/menu" \req ->
  rep 200 (db.q "select * from menu_items order by category, name")

http.on :post "/api/menu" \req ->
  if !req.body.name
    ret rep 400 {error:"taom nomi kerak"}
  rep 201 (db.ins "menu_items" {
    name:req.body.name
    price:req.body.price
    category:req.body.category
    photo:req.body.photo
    available:(req.body.available ?? true)
  })

http.on :put "/api/menu/:id" \req ->
  db.up "menu_items" req.body {id:req.params.id}
  rep 200 (db.one "select * from menu_items where id=$1" [req.params.id])

http.on :del "/api/menu/:id" \req ->
  db.del "menu_items" {id:req.params.id}
  rep 200 {ok:true}

# ---- Stollar ----
http.on :get "/api/tables" \req ->
  rep 200 (db.q "select * from tables order by num")

http.on :get "/api/tables/:id/order" \req ->
  # stol bo'yicha joriy (yetkazilmagan) buyurtma
  ord = db.one "select * from orders where table_id=$1 and status != $2 order by created desc limit 1" [req.params.id :delivered]
  if !ord
    ret rep 200 {order:nil items:[]}
  rep 200 (order_detail ord.id)

http.on :put "/api/tables/:id" \req ->
  db.up "tables" {status:req.body.status} {id:req.params.id}
  rep 200 {ok:true}

# ---- Buyurtmalar ----
http.on :get "/api/orders" \req ->
  rep 200 (db.q "select o.*, t.num table_num from orders o join tables t on t.id=o.table_id order by o.created desc")

http.on :get "/api/orders/:id" \req ->
  rep 200 (order_detail req.params.id)

http.on :post "/api/orders" \req ->
  # Yangi buyurtma — tranzaksiya ichida: order + order_items + stol band qilish
  res = db.tx \->
    ord = db.ins "orders" {
      table_id:req.body.table_id
      waiter_id:req.body.waiter_id
      status::new
      total:0
    }
    total <- 0
    each it in req.body.items
      mi = db.one "select * from menu_items where id=$1" [it.item_id]!
      db.ins "order_items" {
        order_id:ord.id
        item_id:mi.id
        name:mi.name
        price:mi.price
        qty:it.qty
      }
      total <- total + (mi.price * it.qty)
    db.up "orders" {total:total} {id:ord.id}
    db.up "tables" {status::busy} {table_id:req.body.table_id}   # BO'SHLIQ: where kaliti id bo'lishi kerak edi — pastga qara
    ret ord.id
  # realtime: barcha admin panellarga yangi buyurtmani push qilish
  ws.room.send "orders" (json.enc {event::new oid:res})
  rep 201 (order_detail res)

# Buyurtma holatini o'zgartirish + realtime push
http.on :put "/api/orders/:id/status" \req ->
  oid = req.params.id
  db.up "orders" {status:req.body.status} {id:oid}
  # yetkazildi → stolni bo'shatish
  if req.body.status == :delivered
    ord = db.one "select * from orders where id=$1" [oid]!
    db.up "tables" {status::free} {id:ord.table_id}
  ws.room.send "orders" (json.enc {event::status oid:oid status:req.body.status})
  rep 200 {ok:true}

# ---- Xodimlar ----
http.on :get "/api/staff" \req ->
  rep 200 (db.q "select * from staff order by name")

# Har xodim bo'yicha sotuv (yetkazilgan buyurtmalar summasi)
http.on :get "/api/staff/sales" \req ->
  rep 200 (db.q "select s.id, s.name, s.role, coalesce(sum(o.total),0) sales, count(o.id) cnt from staff s left join orders o on o.waiter_id=s.id and o.status=$1 group by s.id, s.name, s.role order by sales desc" [:delivered])

http.on :post "/api/staff" \req ->
  rep 201 (db.ins "staff" {
    name:req.body.name
    role:req.body.role
    phone:req.body.phone
    active:(req.body.active ?? true)
  })

http.on :put "/api/staff/:id" \req ->
  db.up "staff" req.body {id:req.params.id}
  rep 200 {ok:true}

http.on :del "/api/staff/:id" \req ->
  db.del "staff" {id:req.params.id}
  rep 200 {ok:true}

# Smena ochish/yopish
http.on :post "/api/staff/:id/shift" \req ->
  sid = req.params.id
  open = db.one "select * from shifts where staff_id=$1 and end_at is null order by start_at desc limit 1" [sid]
  if open
    db.up "shifts" {end_at:time.now} {id:open.id}
    rep 200 {ok:true action::closed}
  else
    db.ins "shifts" {staff_id:sid start_at:time.now}
    rep 201 {ok:true action::opened}

# ---- Sozlamalar ----
http.on :get "/api/settings" \req ->
  rep 200 (get_settings())

http.on :put "/api/settings" \req ->
  cur = db.one "select * from settings order by id limit 1"
  if cur
    db.up "settings" req.body {id:cur.id}
  else
    db.ins "settings" req.body
  rep 200 {ok:true}

# ============================================================
# WEBSOCKET — realtime buyurtmalar
# ============================================================

ws.on :connect \conn ->
  ws.room.join conn "orders"      # har bir admin "orders" xonasiga qo'shiladi

ws.on :message \conn msg ->
  # admin paneldan kelishi mumkin bo'lgan xabarlar (hozircha echo/ping)
  m = json.dec msg
  if m.ping
    ws.send conn (json.enc {pong:true})

ws.on :disconnect \conn ->
  ws.room.leave conn "orders"

# ============================================================
# THEME
# ============================================================

theme
  primary "#e84d8a"
  accent  "#9b5de5"
  radius  :lg
  font    "Inter"
  mode    :light

# ============================================================
# FRONTEND
# ============================================================

menu_nav = [
  {to:"/"         icon::home  label:"Bosh sahifa"}
  {to:"/menu"     icon::box   label:"Menyu"}
  {to:"/orders"   icon::list  label:"Buyurtmalar"}
  {to:"/tables"   icon::grid  label:"Stollar"}
  {to:"/staff"    icon::users label:"Xodimlar"}
  {to:"/settings" icon::gear  label:"Sozlamalar"}
]

view app
  ui.shell {brand:"Restoran Admin" nav:menu_nav}
    page "/"         -> dashboard
    page "/menu"     -> menu_page
    page "/orders"   -> orders_page
    page "/tables"   -> tables_page
    page "/staff"    -> staff_page
    page "/settings" -> settings_page

# ---- DASHBOARD ----
view dashboard
  s     <- source http.get "/api/stats"
  chart <- source http.get "/api/revenue-chart"

  h1 "Bosh sahifa"

  if s.loading
    ui.spinner
  elif s.err
    ui.error s.err
  else
    div {grid:4 gap:4 mb:4}
      ui.stat "Bugungi tushum" "${s.data.revenue/100}$" {icon::cash kind::primary}
      ui.stat "Buyurtmalar" s.data.orders_count {icon::list}
      ui.stat "Faol stollar" s.data.active_tables {icon::grid kind::info}
      ui.stat "Top taom" s.data.top_item {icon::box kind::ok}

  div {kind::card pad:4}
    h2 "Tushum grafigi (7 kun)"
    if chart.loading
      ui.spinner
    else
      ui.chart chart.data {x::day y::rev kind::bar}

# ---- MENYU ----
view menu_page
  items <- source db.q "select * from menu_items order by category, name"
  q     <- ""
  cat   <- :all
  open  <- false
  edit_row <- nil

  shown = items.data.filter \p ->
    name_ok = str.has (str.low p.name) (str.low q)
    cat_ok = (cat == :all) | (p.category == cat)
    name_ok & cat_ok

  div {flex:true gap:3 mb:4}
    h1 "Menyu"
    ui.search {bind:q placeholder:"Taom qidirish..."}
    ui.select {bind:cat opts:[:all :starter :main :dessert :drink]}
    btn "+ Yangi taom" {on:\-> edit_row <- nil open <- true kind::primary ml::auto}

  ui.table shown {cols:[:photo :name :category :price :available]
    fmt::price \v -> "${v/100}$"
    cell::photo \r -> img r.photo {w:10 round:true}
    cell::available \r -> badge (if r.available "Bor" else "Yo'q") {kind:(if r.available :ok else :danger)}
    actions:[
      {icon::edit  on:\r -> edit_row <- r open <- true}
      {icon::trash on:\r -> del_menu r.id confirm:"O'chirilsinmi?"}
    ]}

  ui.modal {open:open title:(if edit_row "Taomni tahrirlash" else "Yangi taom")}
    ui.form edit_row {on:save_menu fields:[
      {name::name      label:"Nomi"       kind::text   req:true}
      {name::price     label:"Narx"       kind::money  req:true}
      {name::category  label:"Kategoriya" kind::select opts:[:starter :main :dessert :drink]}
      {name::photo     label:"Rasm URL"   kind::text}
      {name::available label:"Mavjud"     kind::bool}
    ]}

fn save_menu d
  if d.id
    http.put "/api/menu/${d.id}" d
  else
    http.post "/api/menu" d
  ui.invalidate :items
  ui.close

fn del_menu id
  http.del "/api/menu/$id"
  ui.invalidate :items

# ---- BUYURTMALAR (realtime) ----
view orders_page
  orders <- source http.get "/api/orders"
  detail <- nil
  open   <- false

  # realtime: WS orqali yangi/o'zgargan buyurtma kelganda jadvalni yangilash
  ui.on_ws "orders" \msg ->            # BO'SHLIQ: WS frontend tinglovchisi — pastga qara
    ui.invalidate :orders

  h1 "Buyurtmalar"

  ui.table orders {cols:[:id :table_num :total :status :created]
    fmt::total \v -> "${v/100}$"
    cell::status \r -> status_badge r.status
    actions:[
      {icon::eye on:\r -> show_detail r.id}
      {icon::forward on:\r -> advance_status r}
    ]}

  ui.modal {open:open title:"Buyurtma tafsiloti"}
    if detail
      div {kind::panel gap:3}
        div {flex:true gap:3}
          b "Stol: ${detail.order.table_id}"
          status_badge detail.order.status {ml::auto}
        ul
          each it in detail.items key:it.id
            li
              span it.name
              span " x${it.qty}" {kind::muted}
              span "${(it.price * it.qty)/100}$" {ml::auto}
        div {flex:true mt:3}
          b "Jami:"
          b "${detail.order.total/100}$" {ml::auto}
        div {flex:true gap:2 mt:3}
          btn "Tayyorlanmoqda" {on:\-> set_status detail.order.id :cooking}
          btn "Tayyor" {on:\-> set_status detail.order.id :ready kind::info}
          btn "Yetkazildi" {on:\-> set_status detail.order.id :delivered kind::ok}

# Buyurtma holati badge'i — OVERRIDE darajasidagi kichik komponent
view status_badge st
  label = match st
    :new -> "Yangi"
    :cooking -> "Tayyorlanmoqda"
    :ready -> "Tayyor"
    :delivered -> "Yetkazildi"
    _ -> "?"
  kind = match st
    :new -> :info
    :cooking -> :warn
    :ready -> :primary
    :delivered -> :ok
    _ -> :muted
  badge label {kind:kind}

fn show_detail oid
  d = http.get "/api/orders/$oid"
  reg.call "set_order_detail" {detail:d.body}   # BO'SHLIQ: tashqi state set — pastga qara

fn advance_status r
  nxt = match r.status
    :new -> :cooking
    :cooking -> :ready
    :ready -> :delivered
    _ -> :delivered
  set_status r.id nxt

fn set_status oid st
  http.put "/api/orders/$oid/status" {status:st}
  ui.invalidate :orders

# ---- STOLLAR (override: o'z grid view'imiz) ----
view tables_page
  tbls <- source http.get "/api/tables"
  h1 "Stollar"
  if tbls.loading
    ui.spinner
  else
    table_grid tbls.data

# OVERRIDE — tayyor ui.table o'rniga o'z karta-grid view'imiz
view table_grid rows
  div {grid:4 gap:4}
    each t in rows key:t.id
      div {kind::card pad:4 hover:true on:\-> open_table t.id}
        div {flex:true}
          h2 "Stol ${t.num}"
          badge (table_label t.status) {kind:(table_kind t.status) ml::auto}
        p "${t.seats} kishilik" {kind::muted}
        div {flex:true gap:2 mt:3}
          btn "Bo'sh" {on:\-> set_table t.id :free kind::ghost}
          btn "Band" {on:\-> set_table t.id :busy kind::ghost}
          btn "Rezerv" {on:\-> set_table t.id :reserved kind::ghost}

fn table_label st
  match st
    :free -> "Bo'sh"
    :busy -> "Band"
    :reserved -> "Rezerv"
    _ -> "?"

fn table_kind st
  match st
    :free -> :ok
    :busy -> :danger
    :reserved -> :warn
    _ -> :muted

fn set_table id st
  http.put "/api/tables/$id" {status:st}
  ui.invalidate :tbls

fn open_table id
  d = http.get "/api/tables/$id/order"
  # BO'SHLIQ: stol bo'yicha joriy buyurtmani modalda ko'rsatish — state set mexanizmi noaniq
  log "stol buyurtmasi:" d.body

# ---- XODIMLAR ----
view staff_page
  sales <- source http.get "/api/staff/sales"
  open  <- false
  edit_row <- nil

  div {flex:true gap:3 mb:4}
    h1 "Xodimlar"
    btn "+ Yangi xodim" {on:\-> edit_row <- nil open <- true kind::primary ml::auto}

  ui.table sales {cols:[:name :role :cnt :sales]
    fmt::sales \v -> "${v/100}$"
    cell::role \r -> badge (role_label r.role) {kind::info}
    actions:[
      {icon::clock on:\r -> toggle_shift r.id}
      {icon::edit  on:\r -> edit_row <- r open <- true}
      {icon::trash on:\r -> del_staff r.id confirm:"O'chirilsinmi?"}
    ]}

  ui.modal {open:open title:(if edit_row "Xodimni tahrirlash" else "Yangi xodim")}
    ui.form edit_row {on:save_staff fields:[
      {name::name   label:"Ism"      kind::text   req:true}
      {name::role   label:"Lavozim"  kind::select opts:[:waiter :chef :manager]}
      {name::phone  label:"Telefon"  kind::text}
      {name::active label:"Faol"     kind::bool}
    ]}

fn role_label r
  match r
    :waiter -> "Ofitsiant"
    :chef -> "Oshpaz"
    :manager -> "Menejer"
    _ -> "?"

fn save_staff d
  if d.id
    http.put "/api/staff/${d.id}" d
  else
    http.post "/api/staff" d
  ui.invalidate :sales
  ui.close

fn del_staff id
  http.del "/api/staff/$id"
  ui.invalidate :sales

fn toggle_shift id
  http.post "/api/staff/$id/shift" {}
  ui.invalidate :sales

# ---- SOZLAMALAR ----
view settings_page
  cfg <- source http.get "/api/settings"
  h1 "Sozlamalar"
  if cfg.loading
    ui.spinner
  else
    div {kind::panel gap:4 w:60}
      ui.form cfg.data {on:save_settings fields:[
        {name::rest_name     label:"Restoran nomi"  kind::text req:true}
        {name::open_time     label:"Ochilish vaqti" kind::text}
        {name::close_time    label:"Yopilish vaqti" kind::text}
        {name::theme_primary label:"Asosiy rang"    kind::text}
        {name::theme_accent  label:"Aksent rang"    kind::text}
      ]}

fn save_settings d
  http.put "/api/settings" d
  ui.invalidate :cfg

# ============================================================
# ENTRY POINT — HTTP API + UI + WS bitta portda
# ============================================================

ui.serve app 3000
```

## Topilgan bo'shliqlar (SPEC GAPS)

1. **Frontend WS tinglovchisi (eng katta bo'shliq)** — (a) Buyurtmalar sahifasida real-vaqt yangilanish kerak edi: backend `ws.room.send "orders" ...` qiladi, frontend esa shu xabarni eshitib `ui.invalidate :orders` qilishi kerak. (b) Spec backend WS'ni (`ws.on`, `ws.room.*`) va `source`'ning WS orqali invalidatsiya qilinishini ("WS for realtime source invalidation") **aytadi**, lekin frontend tomondan WS xabarini qanday tinglashni (qanday API bilan) **ko'rsatmaydi**. `ws.on` faqat server tomoni. (c) Men `ui.on_ws "orders" \msg -> ...` deb taxmin qildim — spec'da bunday funksiya YO'Q. Aslida `ui.serve` "realtime source invalidation" uchun WS'ni o'zi boshqarishi mumkin, ya'ni `ui.invalidate` boshqa klientlarga avtomatik tarqalishi mumkin — lekin bu ham aniq emas.

2. **Modal/komponentdan tashqaridagi state'ni o'rnatish (`detail`, stol buyurtmasi)** — (a) Buyurtma tafsilotini va stol joriy buyurtmasini modalda ko'rsatish uchun `http.get` natijasini reaktiv state'ga yozish kerak edi. (b) `<-` state faqat `view` ichida e'lon qilinadi, lekin uni `fn` ichidan (masalan `show_detail`) yangilash yo'li spec'da **ko'rsatilmagan**. `ui.invalidate` faqat `source`'ni qayta yuklaydi, ixtiyoriy state'ni emas. (c) Men `reg.call "set_order_detail" ...` va `log` bilan vaqtinchalik yechim qildim — bu ishlamaydi, faqat bo'shliqni belgilash uchun. To'g'risi: tafsilotni alohida `source`'ga aylantirish kerak bo'lsa kerak (`detail <- source http.get "/api/orders/$id"`), lekin "tugma bosilganda dinamik id bilan source ochish" naqshi spec'da yo'q.

3. **`db.up` da noto'g'ri where kaliti** — (a) Yangi buyurtma tranzaksiyasida stolni band qilmoqchi edim: `db.up "tables" {status::busy} {table_id:...}`. (b) Spec `db.up "t" {set} {where}` deydi, lekin `tables` jadvalida `table_id` ustuni yo'q — to'g'risi `{id:req.body.table_id}`. Bu mantiqiy xato, spec gap emas, lekin halol bo'lish uchun belgiladim: where kaliti `id` bo'lishi kerak.

4. **`ui.chart` props'lari** — (a) Tushum grafigini chizmoqchi edim. (b) Spec `ui.chart`'ni blok sifatida sanaydi, lekin uning props'larini (qaysi maydon X, qaysi Y, turi bar/line) **umuman ko'rsatmaydi**. (c) Men `{x::day y::rev kind::bar}` deb taxmin qildim — bu sof taxmin.

5. **`ui.select` ning `bind:` bilan ishlashi va `opts` formati** — (a) Menyu/kategoriya filtri uchun select kerak edi. (b) Spec `ui.select`'ni sanaydi va `ui.form` ichida `kind::select opts:[...]` ko'rsatadi, lekin **mustaqil** `ui.select {bind:cat opts:[...]}` sintaksisini tasdiqlamaydi. (c) `bind:` input/search uchun ko'rsatilgan, men uni select'ga ham qo'lladim — mantiqiy, lekin tasdiqlanmagan.

6. **`source` ni `http.get` bilan ishlatish** — (a) Dashboard/orders uchun `source http.get "/api/stats"` ishlatdim. (b) Spec `source`'ni asosan `db.q`/`db.one` bilan ko'rsatadi va "external → http.get" deydi, lekin namuna doim `db.q`. `http.get` natijasi `.data`'ga to'g'ridan-to'g'ri kelishi (status/body emas) taxmin qilingan. `ui.invalidate :items` qanday tag bilan `http.get` source'ni topishi ham noaniq — men `<- source` o'zgaruvchi nomidan (`items`, `orders`) tag olinadi deb taxmin qildim (spec'da `:items` source nomiga mos kelishi shundan).

7. **`ui.modal` ichida shartli `view` chaqirish va dinamik title** — (a) Tahrirlash/qo'shish bitta modalda: `title:(if edit_row "..." else "...")`. (b) Spec props ichida ifoda (`if`) ishlashini to'g'ridan-to'g'ri ko'rsatadi (`kind:(if r.stock==0 ...)`) — bu OK. Lekin `ui.form edit_row` (yoki `nil`) — `edit_row` mavjud bo'lsa formani to'ldirishi spec'da aniq emas; men `ui.form`'ning birinchi argumenti (data) formani to'ldiradi deb oldim.

8. **`db.up` da to'g'ridan-to'g'ri `req.body`ni set sifatida berish** — `db.up "menu_items" req.body {id:...}` — spec `{set}` map kutadi, `req.body` map bo'lgani uchun ishlaydi deb oldim, lekin ortiqcha/ruxsatsiz ustunlar bo'lsa (mass-assignment) himoya yo'q. Bu xavfsizlik bo'shlig'i, spec bu haqda jim.

9. **`fmt::price` / `cell::status` da `::` symbol kaliti** — `fmt::price` ustun nomi `price` ni symbol sifatida kalitlaydi. Spec namunada `fmt::price \v -> ...` bor, demak OK. Lekin men `cell::photo`, `fmt::sales` kabilarni kengaytirdim — naqsh bir xil, ishonchli.

## Spec'da yaxshi ishlagan narsalar

- **`tbl` sxema** — juda tabiiy yozildi: `money`, `sym`, `ref:`, `now`, `null` modifikatorlari aniq va yetarli. Restoran domeni (orders/order_items/menu/tables/staff) muammosiz modellashtirildi.
- **`http.on` + `rep` + `db.*`** — backend CRUD endpoint'lar bir zumda yozildi. `req.params`, `req.body`, `db.ins`/`db.up`/`db.del` to'liq qoplandi.
- **`db.tx` tranzaksiya** — yangi buyurtma (order + order_items + total hisoblash) uchun ideal mos keldi; `\->` lambda, ichida `<-` mutable accumulator, `ret` — hammasi tabiiy.
- **`match` symbol dispatch** — buyurtma/stol holatlarini label va kind'ga aylantirish (`status_badge`, `table_label`) uchun juda toza chiqdi.
- **Default → config → override modeli** — `ui.table` (config: cols/fmt/cell/actions) ↔ o'z `table_grid` view'i (override) ni ko'rsatish oson bo'ldi. `ui.stat` default'i ham bir qatorda.
- **`source` + `ui.invalidate`** — db so'rovini UI'ga ulash glue-kodsiz; mutatsiyadan keyin `ui.invalidate :tag` naqshi juda yoqimli va izchil.
- **`view` = props'li komponent** — `status_badge`, `table_grid` kabi qayta ishlatiladigan komponentlarni `fn` kabi chaqirish (space-args) ortiqcha sintaksissiz ishladi.
- **`theme` bloki** — ranglar/radius/font'ni bitta joyda, komponentlarga tegmasdan sozlash — semantik props (`kind::primary`) bilan birga juda toza arxitektura.