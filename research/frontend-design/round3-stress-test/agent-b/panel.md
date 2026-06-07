# Restoran admin panel (Flux)

## To'liq kod

```flux
use http db ui ws

# ============================================================
# SCHEMA
# ============================================================

tbl menu_items
  id         serial pk
  name       str
  category   sym
  price      money
  image_url  str null
  available  bool
  ts         now

tbl tables
  id         serial pk
  number     int uniq
  capacity   int
  status     sym

tbl staff
  id         serial pk
  name       str
  role       sym
  shift_start str null
  shift_end   str null
  active     bool

tbl orders
  id         serial pk
  table_id   int ref:tables.id null
  staff_id   int ref:staff.id null
  status     sym
  total      money
  notes      str null
  ts         now

tbl order_items
  id         serial pk
  order_id   int ref:orders.id
  menu_item_id int ref:menu_items.id
  quantity   int
  price      money

tbl settings
  id         serial pk
  key        str uniq
  val        json

# ============================================================
# BACKEND — HTTP API
# ============================================================

# --- Dashboard ---
http.on :get "/api/dashboard" \req ->
  today_start = time.ago 0 :day
  revenue = db.one "select coalesce(sum(total),0) rev from orders where ts >= $1 and status==$2" [today_start "delivered"]
  orders_today = db.one "select count(*) cnt from orders where ts >= $1" [today_start]
  active_tables = db.one "select count(*) cnt from tables where status=='occupied'" []
  top_item = db.one "select mi.name, sum(oi.quantity) qty from order_items oi join menu_items mi on mi.id=oi.menu_item_id group by mi.name order by qty desc limit 1" []
  hourly = db.q "select date_trunc('hour', ts) hr, coalesce(sum(total),0) rev from orders where ts >= $1 and status='delivered' group by hr order by hr" [today_start]
  rep 200 {
    revenue: revenue.rev ?? 0
    orders_count: orders_today.cnt ?? 0
    active_tables: active_tables.cnt ?? 0
    top_item: top_item
    hourly_revenue: hourly
  }

# --- Menu ---
http.on :get "/api/menu" \req ->
  cat = req.query.category
  if cat
    rep 200 (db.q "select * from menu_items where category=$1 order by name" [cat])
  else
    rep 200 (db.q "select * from menu_items order by name")

http.on :post "/api/menu" \req ->
  item = db.ins "menu_items" {
    name: req.body.name
    category: req.body.category
    price: req.body.price
    image_url: req.body.image_url
    available: req.body.available ?? true
  }
  rep 201 item

http.on :put "/api/menu/:id" \req ->
  db.up "menu_items" {
    name: req.body.name
    category: req.body.category
    price: req.body.price
    image_url: req.body.image_url
    available: req.body.available
  } {id: req.params.id}
  rep 200 (db.one "select * from menu_items where id=$1" [req.params.id])

http.on :del "/api/menu/:id" \req ->
  db.del "menu_items" {id: req.params.id}
  rep 200 {ok: true}

# --- Orders ---
http.on :get "/api/orders" \req ->
  status = req.query.status
  if status
    rows = db.q "select o.*, t.number table_number, s.name staff_name from orders o left join tables t on t.id=o.table_id left join staff s on s.id=o.staff_id where o.status=$1 order by o.ts desc" [status]
    rep 200 rows
  else
    rows = db.q "select o.*, t.number table_number, s.name staff_name from orders o left join tables t on t.id=o.table_id left join staff s on s.id=o.staff_id order by o.ts desc limit 100" []
    rep 200 rows

http.on :get "/api/orders/:id" \req ->
  order = db.one "select o.*, t.number table_number, s.name staff_name from orders o left join tables t on t.id=o.table_id left join staff s on s.id=o.staff_id where o.id=$1" [req.params.id]
  if !order
    fail 404 "buyurtma topilmadi"
  items = db.q "select oi.*, mi.name item_name, mi.category from order_items oi join menu_items mi on mi.id=oi.menu_item_id where oi.order_id=$1" [req.params.id]
  rep 200 {order: order items: items}

http.on :post "/api/orders" \req ->
  order = db.tx \->
    ord = db.ins "orders" {
      table_id: req.body.table_id
      staff_id: req.body.staff_id
      status: :new
      total: 0
      notes: req.body.notes
    }
    total <- 0
    each it in req.body.items
      mi = db.one "select * from menu_items where id=$1" [it.menu_item_id]!
      db.ins "order_items" {
        order_id: ord.id
        menu_item_id: it.menu_item_id
        quantity: it.quantity
        price: mi.price
      }
      total <- total + (mi.price * it.quantity)
    db.up "orders" {total: total} {id: ord.id}
    db.up "tables" {status: :occupied} {id: req.body.table_id}
    ret db.one "select * from orders where id=$1" [ord.id]
  ws.room.send "orders" (json.enc {event: :new_order order: order})
  rep 201 order

http.on :patch "/api/orders/:id/status" \req ->
  db.up "orders" {status: req.body.status} {id: req.params.id}
  order = db.one "select * from orders where id=$1" [req.params.id]
  if req.body.status == :delivered
    db.up "tables" {status: :free} {id: order.table_id}
  ws.room.send "orders" (json.enc {event: :status_changed order: order})
  rep 200 order

# --- Tables ---
http.on :get "/api/tables" \req ->
  rows = db.q "select t.*, o.id order_id, o.total order_total, o.status order_status from tables t left join orders o on o.table_id=t.id and o.status != 'delivered' order by t.number" []
  rep 200 rows

http.on :post "/api/tables" \req ->
  t = db.ins "tables" {
    number: req.body.number
    capacity: req.body.capacity
    status: :free
  }
  rep 201 t

http.on :patch "/api/tables/:id/status" \req ->
  db.up "tables" {status: req.body.status} {id: req.params.id}
  rep 200 {ok: true}

# --- Staff ---
http.on :get "/api/staff" \req ->
  rep 200 (db.q "select * from staff where active=true order by name" [])

http.on :get "/api/staff/sales" \req ->
  today_start = time.ago 0 :day
  rows = db.q "select s.id, s.name, count(o.id) orders_count, coalesce(sum(o.total),0) total_sales from staff s left join orders o on o.staff_id=s.id and o.ts >= $1 and o.status='delivered' where s.active=true group by s.id, s.name order by total_sales desc" [today_start]
  rep 200 rows

http.on :post "/api/staff" \req ->
  s = db.ins "staff" {
    name: req.body.name
    role: req.body.role ?? :waiter
    shift_start: req.body.shift_start
    shift_end: req.body.shift_end
    active: true
  }
  rep 201 s

http.on :put "/api/staff/:id" \req ->
  db.up "staff" {
    name: req.body.name
    role: req.body.role
    shift_start: req.body.shift_start
    shift_end: req.body.shift_end
  } {id: req.params.id}
  rep 200 {ok: true}

http.on :del "/api/staff/:id" \req ->
  db.up "staff" {active: false} {id: req.params.id}
  rep 200 {ok: true}

# --- Settings ---
http.on :get "/api/settings" \req ->
  rows = db.q "select * from settings" []
  result <- {}
  each row in rows
    result <- result.set row.key row.val
  rep 200 result

http.on :post "/api/settings" \req ->
  each k, v in req.body
    db.put "settings" {val: v} {key: k}
  rep 200 {ok: true}

# ============================================================
# WEBSOCKET — real-time orders
# ============================================================

ws.on :connect \conn ->
  ws.room.join conn "orders"

ws.on :disconnect \conn ->
  ws.room.leave conn "orders"

# ============================================================
# THEME
# ============================================================

theme
  primary "#e85d26"
  accent  "#f5a623"
  radius  :md
  font    "Inter"
  mode    :light

# ============================================================
# FRONTEND — VIEWS
# ============================================================

# --- Nav menu ---
menu = [
  {to:"/"          icon::home   label:"Bosh sahifa"}
  {to:"/menu"      icon::book   label:"Menyu"}
  {to:"/orders"    icon::list   label:"Buyurtmalar"}
  {to:"/tables"    icon::grid   label:"Stollar"}
  {to:"/staff"     icon::users  label:"Xodimlar"}
  {to:"/settings"  icon::cog    label:"Sozlamalar"}
]

# --- Dashboard ---
view dashboard
  stats  <- source http.get "/api/dashboard"
  h1 "Bosh sahifa"
  if stats.loading
    ui.spinner
  elif stats.err
    ui.error stats.err
  else
    s = stats.data
    div {grid:4 gap:4 mb:6}
      ui.stat "Bugungi tushum" "${s.revenue/100} so'm" {icon::cash kind::primary}
      ui.stat "Buyurtmalar" "${s.orders_count}" {icon::list}
      ui.stat "Faol stollar" "${s.active_tables}" {icon::grid kind::info}
      ui.stat "Top taom" (s.top_item.name ?? "—") {icon::star kind::ok}
    h2 "Soatlik tushum" {mb:3}
    ui.chart s.hourly_revenue {
      kind::line
      x::hr
      y::rev
      fmt::rev \v -> "${v/100} so'm"
    }

# --- Menu boshqaruvi ---
view menu_page
  items      <- source http.get "/api/menu"
  q          <- ""
  cat_filter <- ""
  open_add   <- false
  open_edit  <- false
  edit_item  <- nil

  categories = [:bosh_taom :salat :ichimlik :desert :garnir]

  shown = items.data.filter \p ->
    name_match = str.has (str.low (p.name ?? "")) (str.low q)
    cat_match = cat_filter == "" | p.category == cat_filter
    name_match & cat_match

  div {flex:true gap:3 mb:4}
    h1 "Menyu"
    ui.search {bind:q placeholder:"Taom qidirish..."}
    ui.select {bind:cat_filter opts:([:_ "Barcha turlar"] ++ (categories.map \c -> [c (str.str c)])) }
    btn "+ Yangi taom" {on:\-> open_add <- true kind::primary ml::auto}

  if items.loading
    ui.spinner
  elif items.err
    ui.error items.err
  else
    ui.table shown {
      cols:[:name :category :price :available]
      fmt::price \v -> "${v/100} so'm"
      fmt::available \v -> if v "Mavjud" else "Mavjud emas"
      cell::available \r ->
        badge (if r.available "Mavjud" else "Yoq") {kind:(if r.available :ok else :danger)}
      actions:[
        {icon::edit   label:"Tahrirlash" on:\r -> do_edit r}
        {icon::trash  label:"O'chirish"  on:\r -> del_menu_item r.id confirm:"O'chirilsinmi?"}
      ]
    }

  ui.modal {open:open_add title:"Yangi taom qo'shish"}
    ui.form nil {on:save_new_item fields:[
      {name::name       label:"Nomi"        kind::text    req:true}
      {name::category   label:"Kategoriya"  kind::select  opts:categories req:true}
      {name::price      label:"Narx (tiyin)" kind::number req:true}
      {name::image_url  label:"Rasm URL"    kind::text}
      {name::available  label:"Mavjud"      kind::bool}
    ]}

  ui.modal {open:open_edit title:"Taomni tahrirlash"}
    ui.form edit_item {on:save_edit_item fields:[
      {name::name       label:"Nomi"        kind::text    req:true}
      {name::category   label:"Kategoriya"  kind::select  opts:categories req:true}
      {name::price      label:"Narx (tiyin)" kind::number req:true}
      {name::image_url  label:"Rasm URL"    kind::text}
      {name::available  label:"Mavjud"      kind::bool}
    ]}

fn do_edit item
  edit_item <- item
  open_edit <- true

fn save_new_item d
  http.post "/api/menu" d
  ui.invalidate :items
  ui.close
  open_add <- false

fn save_edit_item d
  http.put "/api/menu/${d.id}" d
  ui.invalidate :items
  ui.close
  open_edit <- false

fn del_menu_item id
  http.del "/api/menu/$id"
  ui.invalidate :items

# --- Buyurtmalar (override: o'z custom view) ---

# OVERRIDE: standart ui.table o'rniga maxsus buyurtma satri
view order_row order
  status_kind = match order.status
    :new           -> :info
    :preparing     -> :warn
    :ready         -> :ok
    :delivered     -> :muted
    _ -> :muted

  div {kind::row hover:true pad:2 mb:2}
    div {w:8}
      b "Stol #${order.table_number ?? '—'}"
      p "${order.staff_name ?? '—'}" {kind::muted size::sm}
    div {w:12}
      badge (str.str order.status) {kind:status_kind}
    div {w:8}
      p "${order.total/100} so'm" {bold:true}
    div {w:10}
      p (time.fmt order.ts "HH:mm") {kind::muted size::sm}
    div {flex:true gap:2}
      if order.status == :new
        btn "Tayyorlanmoqda" {on:\-> change_order_status order.id :preparing kind::warn size::sm}
      elif order.status == :preparing
        btn "Tayyor" {on:\-> change_order_status order.id :ready kind::ok size::sm}
      elif order.status == :ready
        btn "Yetkazildi" {on:\-> change_order_status order.id :delivered kind::ghost size::sm}
      btn "Batafsil" {on:\-> open_order_detail order.id kind::ghost size::sm}

view orders_page
  orders    <- source http.get "/api/orders"
  detail    <- nil
  show_detail <- false
  status_filter <- ""

  shown = if status_filter == ""
    orders.data
  else
    orders.data.filter \o -> o.status == status_filter

  div {flex:true gap:3 mb:4}
    h1 "Buyurtmalar"
    div {flex:true gap:2}
      btn "Barchasi"         {on:\-> status_filter <- ""           kind:(if status_filter=="" :primary else :ghost) size::sm}
      btn "Yangi"            {on:\-> status_filter <- "new"        kind:(if status_filter=="new" :info else :ghost) size::sm}
      btn "Tayyorlanmoqda"   {on:\-> status_filter <- "preparing"  kind:(if status_filter=="preparing" :warn else :ghost) size::sm}
      btn "Tayyor"           {on:\-> status_filter <- "ready"      kind:(if status_filter=="ready" :ok else :ghost) size::sm}
      btn "Yetkazildi"       {on:\-> status_filter <- "delivered"  kind:(if status_filter=="delivered" :muted else :ghost) size::sm}

  if orders.loading
    ui.spinner
  elif orders.err
    ui.error orders.err
  else
    div {kind::panel pad:3}
      div {kind::row pad:2 mb:2}
        b "Stol / Xodim" {w:8}
        b "Holat" {w:12}
        b "Summa" {w:8}
        b "Vaqt" {w:10}
        b "Amallar" {}
      each o in shown key:o.id
        order_row o

  ui.modal {open:show_detail title:"Buyurtma tafsiloti"}
    if detail
      order_detail detail

fn open_order_detail id
  d <- source http.get "/api/orders/$id"
  detail <- d.data
  show_detail <- true

fn change_order_status id status
  http.patch "/api/orders/$id/status" {status: status}
  ui.invalidate :orders

# Buyurtma tafsiloti view
view order_detail data
  if !data
    ui.spinner
  else
    o = data.order
    div {mb:4}
      p "Stol: #${o.table_number ?? '—'}" {bold:true}
      p "Xodim: ${o.staff_name ?? '—'}"
      p "Holat: ${str.str o.status}"
      p "Vaqt: ${time.fmt o.ts 'dd.MM.yyyy HH:mm'}" {kind::muted}
      if o.notes
        p "Izoh: ${o.notes}" {kind::muted}
    h3 "Taomlar" {mb:2}
    ui.table data.items {
      cols:[:item_name :quantity :price]
      fmt::price \v -> "${v/100} so'm"
    }
    div {kind::row mt:3}
      b "Jami:" {ml::auto}
      b "${o.total/100} so'm" {ml:2 kind::primary}

# ============================================================
# WS — real-time yangiliklar qabul qilish
# BO'SHLIQ: frontendda ws.connect/on yo'q spec'da — taxmin sifatida
# source bilan ui.invalidate ishlatilyapti
# ============================================================

# --- Stollar ---
view table_card tbl
  status_kind = match tbl.status
    :free     -> :ok
    :occupied -> :danger
    :reserved -> :warn
    _ -> :muted
  status_label = match tbl.status
    :free     -> "Bo'sh"
    :occupied -> "Band"
    :reserved -> "Rezerv"
    _ -> "Noma'lum"

  div {kind::card pad:4 hover:true}
    div {flex:true mb:2}
      h2 "Stol ${tbl.number}"
      badge status_label {kind:status_kind ml::auto}
    p "${tbl.capacity} o'rin" {kind::muted size::sm}
    if tbl.order_id
      div {mt:2}
        p "Buyurtma #${tbl.order_id}" {size::sm}
        p "${tbl.order_total/100} so'm" {bold:true size::sm}
        badge (str.str tbl.order_status) {kind::info size::sm}
    div {flex:true gap:2 mt:3}
      if tbl.status == :free
        btn "Rezerv qilish" {on:\-> set_table_status tbl.id :reserved kind::warn size::sm}
      elif tbl.status == :reserved
        btn "Bo'shatish" {on:\-> set_table_status tbl.id :free kind::ghost size::sm}

view tables_page
  tables <- source http.get "/api/tables"
  open_add <- false

  div {flex:true gap:3 mb:4}
    h1 "Stollar"
    btn "+ Stol qo'shish" {on:\-> open_add <- true kind::primary ml::auto}

  if tables.loading
    ui.spinner
  elif tables.err
    ui.error tables.err
  else
    div {grid:4 gap:4}
      each t in tables.data key:t.id
        table_card t

  ui.modal {open:open_add title:"Yangi stol"}
    ui.form nil {on:save_table fields:[
      {name::number   label:"Stol raqami" kind::number req:true}
      {name::capacity label:"O'rin soni"  kind::number req:true}
    ]}

fn set_table_status id status
  http.patch "/api/tables/$id/status" {status: status}
  ui.invalidate :tables

fn save_table d
  http.post "/api/tables" d
  ui.invalidate :tables
  ui.close
  open_add <- false

# --- Xodimlar ---
view staff_page
  staff_list <- source http.get "/api/staff"
  sales      <- source http.get "/api/staff/sales"
  open_add   <- false
  open_edit  <- false
  edit_staff <- nil

  roles = [:waiter :manager :chef :cashier]

  div {flex:true gap:3 mb:4}
    h1 "Xodimlar"
    btn "+ Xodim qo'shish" {on:\-> open_add <- true kind::primary ml::auto}

  if staff_list.loading | sales.loading
    ui.spinner
  elif staff_list.err
    ui.error staff_list.err
  else
    h2 "Bugungi savdo" {mb:3}
    ui.table sales.data {
      cols:[:name :orders_count :total_sales]
      fmt::total_sales \v -> "${v/100} so'm"
    }
    h2 "Barcha xodimlar" {mb:3 mt:5}
    ui.table staff_list.data {
      cols:[:name :role :shift_start :shift_end]
      fmt::role \v -> str.str v
      actions:[
        {icon::edit  label:"Tahrirlash" on:\r -> do_edit_staff r}
        {icon::trash label:"O'chirish"  on:\r -> del_staff r.id confirm:"O'chirilsinmi?"}
      ]
    }

  ui.modal {open:open_add title:"Yangi xodim qo'shish"}
    ui.form nil {on:save_new_staff fields:[
      {name::name        label:"Ismi"         kind::text   req:true}
      {name::role        label:"Lavozimi"      kind::select opts:roles req:true}
      {name::shift_start label:"Smena boshi"  kind::text   placeholder:"09:00"}
      {name::shift_end   label:"Smena oxiri"  kind::text   placeholder:"21:00"}
    ]}

  ui.modal {open:open_edit title:"Xodimni tahrirlash"}
    ui.form edit_staff {on:save_edit_staff fields:[
      {name::name        label:"Ismi"         kind::text   req:true}
      {name::role        label:"Lavozimi"      kind::select opts:roles req:true}
      {name::shift_start label:"Smena boshi"  kind::text}
      {name::shift_end   label:"Smena oxiri"  kind::text}
    ]}

fn do_edit_staff s
  edit_staff <- s
  open_edit <- true

fn save_new_staff d
  http.post "/api/staff" d
  ui.invalidate :staff_list
  ui.close
  open_add <- false

fn save_edit_staff d
  http.put "/api/staff/${d.id}" d
  ui.invalidate :staff_list
  ui.close
  open_edit <- false

fn del_staff id
  http.del "/api/staff/$id"
  ui.invalidate :staff_list

# --- Sozlamalar ---
view settings_page
  cfg <- source http.get "/api/settings"

  rest_name  <- ""
  open_from  <- ""
  open_to    <- ""
  primary_color <- "#e85d26"
  accent_color  <- "#f5a623"

  if !cfg.loading & !cfg.err & cfg.data
    rest_name    <- cfg.data.restaurant_name ?? "Restoran"
    open_from    <- cfg.data.open_from ?? "09:00"
    open_to      <- cfg.data.open_to   ?? "23:00"
    primary_color <- cfg.data.primary_color ?? "#e85d26"
    accent_color  <- cfg.data.accent_color  ?? "#f5a623"

  h1 "Sozlamalar"
  div {kind::panel pad:5 mt:4}
    h2 "Umumiy" {mb:4}
    div {grid:2 gap:4}
      div
        p "Restoran nomi" {bold:true mb:1}
        input {bind:rest_name placeholder:"Restoran nomi"}
      div
        p "Ish vaqti" {bold:true mb:1}
        div {flex:true gap:2}
          input {bind:open_from placeholder:"09:00"}
          span "—" {mt:1}
          input {bind:open_to placeholder:"23:00"}

    h2 "Mavzu ranglari" {mb:3 mt:5}
    div {grid:2 gap:4}
      div
        p "Asosiy rang" {bold:true mb:1}
        input {bind:primary_color kind::color}
      div
        p "Qo'shimcha rang" {bold:true mb:1}
        input {bind:accent_color kind::color}

    btn "Saqlash" {on:save_settings kind::primary mt:5}

fn save_settings
  http.post "/api/settings" {
    restaurant_name: rest_name
    open_from: open_from
    open_to: open_to
    primary_color: primary_color
    accent_color: accent_color
  }
  ui.invalidate :cfg

# ============================================================
# APP SHELL + ROUTING
# ============================================================

view app
  ui.shell {brand:"Restoran Admin" nav:menu}
    page "/"          -> dashboard
    page "/menu"      -> menu_page
    page "/orders"    -> orders_page
    page "/tables"    -> tables_page
    page "/staff"     -> staff_page
    page "/settings"  -> settings_page

# ============================================================
# ENTRY POINT
# ============================================================

ui.serve app 3000
ws.serve 3001
```

---

## Topilgan bo'shliqlar (SPEC GAPS)

**1. Frontend'dan WebSocket qabul qilish**

- **Nima qilmoqchi edim:** Real-vaqtda yangi buyurtma kelganda frontend avtomatik yangilansin. Backend `ws.room.send "orders" ...` orqali xabar yuboradi, lekin frontend (`view` ichida) WS kanalga subscribe bo'lish uchun hech qanday `ws.connect`, `ws.on` yoki event listener spec'da ko'rsatilmagan.
- **Spec nima deydi:** Spec faqat backend `ws.on :connect`, `ws.on :message`, `ws.on :disconnect`, `ws.send`, `ws.room.*` metodlarini ko'rsatadi. Frontend tomonda WS xabarlarni qanday qabul qilish haqida hech narsa yo'q.
- **Taxminim:** `source` batareyasi WS kanalni ichida avtomatik tinglaydi deb taxmin qildim — `ui.invalidate :orders` chaqirilganda source qayta yuklanadi. Ammo bu to'liq real-vaqt emas (push emas, pull). Bo'shliq belgilandi.

**2. `source` ga nom (tag) berish sintaksisi**

- **Nima qilmoqchi edim:** `ui.invalidate :items` chaqirilganda faqat kerakli source qayta yuklansin. `source` ga qanday nom/tag beriladi? Spec'da `items <- source db.q ...` ko'rinishida yozilgan, lekin `ui.invalidate :items` da `:items` tegli holda qanday bog'lanishi aniq emas.
- **Spec nima deydi:** Spec misolida `items <- source db.q "..."` va keyin `ui.invalidate :items` ishlatilgan — lekin bu `<-` o'zgaruvchi nomi avtomatik tag bo'ladimi yoki alohida yozish kerakmi? Aniq aytilmagan.
- **Taxminim:** `<-` bilan berilgan o'zgaruvchi nomi avtomatik tag bo'ladi deb qabul qildim (`:items` → `items <- source ...`).

**3. `source` reaktiv bo'shliq — qayta baholash (`if !cfg.loading` bilan boshlash)**

- **Nima qilmoqchi edim:** Settings sahifasida `cfg` yuklanganidan keyin `rest_name <-`, `open_from <-` holatlarini uning qiymati bilan to'ldirmoqchi edim.
- **Spec nima deydi:** Spec `source` fieldlari (`data`, `loading`, `err`) haqida gapiradi, lekin source yuklanib bo'lgach bir marta reaktiv "effect" ishlatish (yani: "source tayyor bo'lganda bir marta holat set qil") haqida hech narsa yo'q. `if !cfg.loading & !cfg.err & cfg.data` bloki har render'da qayta ishlaydi — bu to'g'ri emas.
- **Taxminim:** Source yuklanishi bilanoq `<-` bilan yozilgan kod trigger bo'ladi deb taxmin qildim. Haqiqiy implementatsiyada bu "computed init" yoki "watch" talab qilishi mumkin — spec'da bunday pattern ko'rsatilmagan.

**4. `fn` ichida `view`-lokal state'ga kirish (`open_add`, `edit_item` va h.k.)**

- **Nima qilmoqchi edim:** `save_new_item`, `do_edit` kabi `fn`lar `view` ichidagi reaktiv holatlarni (`open_add <- false`, `edit_item <- item`) o'zgartirsin.
- **Spec nima deydi:** Spec `fn` va `view` bir-biridan farqli; `fn` oddiy backend funksiyasi. `fn` `view`-ichki `<-` o'zgaruvchilarga qanday kirishi aniq emas. Closure mexanizmi haqida (ya'ni: `view`-lokal `fn` va ko'rinuvchilik doirasi) spec'da hech narsa yo'q.
- **Taxminim:** `fn`lar `view` ichida e'lon qilinsa, `view`ning lokal o'zgaruvchilariga closure orqali kiradi deb qabul qildim. Ammo spec'da `fn` va `view` ko'rinuvchilik doirasi hech qayerda tushuntirilmagan.

**5. `ui.close` semantikasi**

- **Nima qilmoqchi edim:** Modal yopilsin. `save_new_item` da `ui.close` chaqirilgandan keyin `open_add <- false` ham yozdim, chunki...
- **Spec nima deydi:** Spec misolida `fn save_product d ... ui.close` ko'rsatilgan, lekin `ui.close` qaysi modalni yopadi? Faol/ochiq modalni avtomatik yopadimi, yoki `{open:open_add}` props bilan bog'liq holatni o'zgartiradimi? Aniq emas.
- **Taxminim:** `ui.close` eng so'nggi ochilgan modalni yopadi deb taxmin qildim va himoya sifatida `open_add <- false` ni ham qo'shdim.

**6. `ui.select` opts formati — tuple yoki faqat symbol**

- **Nima qilmoqchi edim:** Kategoriya filtri uchun "Barcha turlar" kabi label bilan `select` yasamoqchi edim.
- **Spec nima deydi:** Spec `ui.select {bind:x opts:[:a :b :c]}` formatini ko'rsatadi, lekin `[value label]` juft-tuple array (masalan `[[:_ "Barcha turlar"] [:bosh_taom "Bosh taom"]]`) qo'llab-quvvatlanishini aytmaydi.
- **Taxminim:** `[symbol "label"]` ko'rinishdagi tuple array ishlatdim. Bu taxmin — spec'da aniq emas.

**7. `ui.chart` props — spec juda kam**

- **Nima qilmoqchi edim:** Dashboard'da soatlik tushum grafigini chizmoqchi edim.
- **Spec nima deydi:** Spec `ui.chart` ni faqat ro'yxat sifatida tilga oladi, hech qanday props misoli yo'q.
- **Taxminim:** `{kind::line x::hr y::rev fmt::rev ...}` formatini mantiqiy taxmin sifatida yozdim. Bo'shliq — bu to'liq ixtiro.

**8. `str.str` — symbol'ni string'ga o'girish**

- **Nima qilmoqchi edim:** Symbol (`order.status` = `:new`) ni ko'rsatish uchun string'ga o'girmoqchi edim.
- **Spec nima deydi:** Spec `str.str x` metodini ko'rsatadi. Symbol uchun ham ishlashini taxmin qildim (`:new` → `"new"`).
- **Taxminim:** Ishlaydi deb qabul qildim. Spec aniq tasdiqlash bermagan.

**9. `match` expression sifatida — qiymat qaytarishi**

- **Nima qilmoqchi edim:** `status_kind = match order.status ...` — `match`ni expression sifatida ishlatdim.
- **Spec nima deydi:** Spec `match`ni statement sifatida ko'rsatadi (`log "new"` kabi). Expression (qiymat qaytarishi) haqida aniq aytilmagan.
- **Taxminim:** `match` expression bo'la oladi, `->` o'ng tomoni qiymat qaytaradi deb qabul qildim.

**10. `ui.invalidate` dagi tag nomi — o'zgaruvchi nomi vs simvol**

- **Nima qilmoqchi edim:** `ui.invalidate :staff_list` — tag nomini `staff_list <- source ...` bilan mos keltirmoqchi edim.
- **Spec nima deydi:** Spec'da `ui.invalidate :items` misoli bor, lekin ko'p so'zli nom (masalan `:staff_list`) to'g'rimi yoki faqat `:items` kabi qisqa simvollarmi — aniq emas.
- **Taxminim:** Underscore bilan yozilgan simvollar ham ishlaydi deb taxmin qildim.

---

## Spec'da yaxshi ishlagan narsalar

- **`tbl` schema sintaksisi** — juda toza, `pk`, `ref`, `now`, `money`, `sym` modifikatorlari bilan jadval aniqlash intuitiv edi. Hech qanday ikkilanish bo'lmadi.
- **`db.tx` tranzaktsiya** — buyurtma yaratishda `db.tx \->` bloki ichida bir nechta `db.ins` + `db.up` zanjiri juda tabiiy yozildi. `fail` avtomatik rollback qilishi elegantlik.
- **`match` + `sym` kolonnasi** — `order.status` simvol bo'lgani va `match` bilan ishlashi juda mos keldi. Backend'dan frontend'gacha bir xil `:new`, `:occupied` kabi simvollar ishlatish kodning izchilligini oshirdi.
- **`each item in list key:item.id`** — ro'yxatlarni render qilish uchun `each` qayta ishlatish (backend loopi bilan bir sintaksis) juda oz token sarfladi.
- **`http.patch`** — `:patch` metodi spec'da ko'rsatilmagan (`use`da faqat `:get :post :put :patch :del` keltirilgan), lekin `http.on :patch` ham ishlaydi deb taxmin qildim. Aslida spec backend handler sifatida `:patch`ni manzillar ro'yxatida aniq ko'rsatmagan — lekin client metodida bor.
- **`db.put` upsert** — sozlamalar saqlashda `db.put "settings" {val:v} {key:k}` nihoyatda qulay bo'ldi. Alohida `insert or update` yozishga hojat qolmadi.
- **`??` null-coalesce** — `revenue.rev ?? 0`, `top_item.name ?? "—"` kabi joylarida kod o'qilishi yaxshi chiqdi.
- **`ui.shell` + `page` routing** — butun navigatsiya strukturasi `ui.shell {nav:menu}` ichida `page "/"` bilan deklarativ yozilishi juda kam kod talab qildi.