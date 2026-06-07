# Flux Frontend — dizayn

## Asosiy g'oya

Flux frontend qatlami backend bilan bir falsafada quriladi: "bir ish = bir yo'l", kam token, canonical form. React/Vue/Svelte kabi freymvorklar har bir komponent uchun props/state/lifecycle/template to'rt qatlam talab qiladi — Flux esa barchani bitta konstruksiyaga siqadi. `<-` allaqachon mutable binding ma'nosida ishlatiladi; frontend'da u reaktiv state bo'ladi: o'zgarsa — bog'liq DOM avtomatik yangilanadi. Yangi belgi ixtiro qilinmaydi, mavjud Flux sintaksisi kengaytiriladi.

Backend va frontend bitta faylda yashaydi. `tbl` — ma'lumot sxemasi, `http.on` — API, `page` — UI sahifasi: uchala bir-birini ko'radi, glue kod yo'q. `db.q` natijasi to'g'ridan-to'g'ri UI binding'ga uzatiladi, alohida fetch/useEffect/axios kerak emas. Flux runtime server-side render + hidratsiya yoki live-wire uslubida ishlaydi: server state'ni o'zgartiradi, faqat diff jo'natadi — frontend JS bundle minimal yoki nol bo'ladi.

---

## Yangi primitivlar (qo'shimchalar)

### 1. `page` — sahifa e'loni

Vazifasi: URL ga bog'langan UI birlik. `http.on` bilan parallel, lekin UI qaytaradi.

```flux
page :index "/"
  # bu blok = sahifa tanasi
```

- Birinchi argument — ism (routing uchun `sym`), ikkinchi — URL pattern.
- `req` avtomatik scope'da (xuddi `http.on` handler'idek).
- Ichida `view`, `state`, `on` bloklar yashaydi.

Agar URL param bo'lsa:

```flux
page :product "/products/:id"
  # req.params.id mavjud
```

---

### 2. `view` — render bloki

Vazifasi: HTML/UI daraxti. Indentatsiya = ichdalik. Element nomi + space-separated atributlar + matn/blok.

```flux
view
  div .card
    h1 "Salom"
    p .muted "Tavsif"
```

**Sintaksis qoidalari:**

| Narsa | Sintaksis | Misol |
|---|---|---|
| Element | `tag` | `div` `button` `input` |
| Class | `.ism` | `div .card` |
| Bir nechta class | `.a.b` | `div .card.active` |
| ID | `#ism` | `div #main` |
| Atribut | `key:val` | `input type:"text"` |
| Dinamik atribut | `key:expr` | `input value:name` |
| Matn | oxirgi arg yoki blok | `h1 "Salom"` |
| Dinamik matn | `"${expr}"` | `p "${user.name}"` |
| Void element | oddiy | `input type:"text" value:x` |

---

### 3. `state` — reaktiv holat

Vazifasi: sahifa ichidagi reaktiv o'zgaruvchi. `<-` bilan bir xil, lekin reaktiv — o'zgarsa view qayta render bo'ladi.

```flux
page :counter "/"
  state
    count <- 0
    name <- "mehmon"
  view
    p "Soni: ${count}"
    button on:click(\-> count <- count + 1) "+"
```

- `state` bloki faqat `page` ichida.
- `<-` semantikasi bir xil (mutable reassign).
- `state` o'zgaruvchilari `view` da avtomatik ko'rinadi.

---

### 4. `on:event` — event binding

Vazifasi: DOM event'ni lambda'ga bog'lash.

```flux
button on:click(\-> count <- count + 1) "Bosing"
input on:input(\e -> search <- e.value) placeholder:"Qidiring..."
form on:submit(\e -> save e.data)
```

- `on:` prefiksi + event nomi.
- Lambda argument: `e` = event object. `e.value` (input value), `e.data` (form map), `e.key` (keyboard).
- Bir nechta event: `button on:click(h1) on:hover(h2)`.

---

### 5. `bind` — ikki tomonlama bog'lash

Vazifasi: `input` ↔ state o'zgaruvchi. `on:input` + `value:` juftini bir so'zga almashtiradi.

```flux
input bind:email          # email state var bilan ikki tomonlama
input bind:search type:"search"
textarea bind:notes
```

Ekvivalent uzun shakl: `input value:email on:input(\e -> email <- e.value)` — `bind` buni bitta so'zga siqadi.

---

### 6. `each` view ichida — ro'yxat render

Vazifasi: mavjud `each` kalit so'zi view ichida ham ishlaydi. Har element uchun UI blok.

```flux
view
  ul
    each item in products
      li .item
        span "${item.name}"
        span .price "${item.price}"
```

- Indentatsiya saqlangan: `each` bloki = bir `li` takrori.
- `key:item.id` optional — diff uchun:

```flux
each item in products key:item.id
  li "${item.name}"
```

---

### 7. `if` view ichida — shartli render

Mavjud `if/elif/else` — view ichida ham ishlaydi, DOM ni shartli ko'rsatadi.

```flux
view
  if user
    p "Xush kelibsiz, ${user.name}"
  else
    button on:click(\-> login()) "Kirish"
```

---

### 8. `comp` — qayta ishlatiladigan komponent

Vazifasi: parametrli UI birlik. `fn` ga o'xshash, lekin view qaytaradi.

```flux
comp badge text color:"blue"
  span .badge style:"color:${color}"
    "${text}"
```

Chaqiruv — xuddi element kabi, space-separated args:

```flux
badge "Yangi" color:"green"
badge "Arxiv"          # color default "blue"
```

Parametrlar: majburiy (faqat ism), ixtiyoriy (default qiymat bilan `key:default`).

---

### 9. `slot` — komponent ichiga kontent uzatish

```flux
comp card title
  div .card
    h3 "${title}"
    div .card-body
      slot          # tashqaridan kontent shu yerga kiradi

# ishlatish:
card "Mahsulot"
  p "Bu yerda tavsif"
  badge "Yangi"
```

---

### 10. `layout` — sahifalar uchun umumiy qolip

```flux
layout :admin
  div .sidebar
    slot :nav
  div .content
    slot          # asosiy kontent

page :dashboard "/" layout::admin
  # avtomatik :admin layout ichida render bo'ladi
```

---

### 11. `theme` — global dizayn tokenlari

```flux
theme
  primary: "#6366f1"
  surface: "#ffffff"
  radius: "8px"
  font: "Inter, sans-serif"
```

View ichida: `style:"color:${theme.primary}"` yoki CSS var sifatida avtomatik `--flux-primary`.

---

### 12. `ui` — default komponentlar (batteries)

Built-in komponentlar, `use ui` bilan:

```flux
use ui
```

Shundan so'ng: `ui.table`, `ui.form`, `ui.modal`, `ui.chart`, `ui.badge`, `ui.button`, `ui.input`, `ui.select`, `ui.sidebar`, `ui.nav`, `ui.stat` — hammasi tayyor.

**Eng muhimi:** `ui.*` komponentlari `override` mexanizmi bilan to'liq almashtirilishi mumkin.

---

### 13. `override` — default komponentni qayta yozish

```flux
override ui.button
  comp button text variant:"primary"
    button .btn."btn-${variant}" on:click(\-> nil)
      "${text}"
```

Endi barcha `ui.button` chaqiruvlari bu komponentni ishlatadi.

---

### 14. `link` — client-side routing

```flux
link "/products" "Mahsulotlar"
link "/products/${id}" "${item.name}" .nav-link
```

Page'lar orasida navigatsiya — full reload yo'q.

---

### 15. `load` — sahifa yuklanishida ma'lumot olish

```flux
page :products "/products"
  load
    products = db.q "select * from products order by name"
    stats = db.one "select count(*) c from products"
  view
    # products va stats bu yerda mavjud
    p "Jami: ${stats.c}"
    each p in products
      ...
```

`load` bloki server-side bajariladi, natija view ga uzatiladi. Alohida API endpoint kerak emas.

---

### 16. `action` — form/button server action

```flux
page :products "/products"
  action :create req
    db.ins "products" req.body
    redirect "/products"

  view
    form action::create method::post
      input bind:name
      button type:"submit" "Saqlash"
```

`action` server-side bajariladi, `redirect` yoki yangilangan `load` data qaytaradi.

---

## Default → Config → Override modeli

Uch daraja bitta komponent — `ui.button` — orqali ko'rsatiladi:

### (a) DEFAULT — hech narsa yozmaslik

```flux
use ui

page :home "/"
  view
    ui.button "Saqlash"
    ui.button "O'chirish" variant:"danger"
```

Tayyor shadcn-unga o'xshash dizayn. Hech qanday CSS, hech qanday config. AI uchun minimal token.

---

### (b) CONFIG — tema va parametrlar

```flux
use ui

theme
  primary: "#10b981"    # yashil brand
  radius: "4px"         # keskinroq burchak
  font: "JetBrains Mono, monospace"

page :home "/"
  view
    ui.button "Saqlash"          # endi yashil, 4px radius
    ui.button "Bekor" variant:"ghost"
```

`theme` bir marta e'lon qilinsa — barcha `ui.*` komponentlar o'zgaradi. Har komponentni alohida ushlamaslik kerak.

---

### (c) OVERRIDE — to'liq nazorat

```flux
use ui

# faqat button ni qayta yoz, qolganlar default qoladi
override ui.button
  comp button text variant:"primary" size:"md" icon:nil
    button
      .px-4.py-2.rounded.font-semibold.transition
      ."bg-emerald-600 hover:bg-emerald-700" if variant == "primary"
      ."bg-gray-100 hover:bg-gray-200 text-gray-800" if variant == "ghost"
      ."text-sm" if size == "sm"
      on:click(\-> nil)
        if icon
          span .mr-2 "${icon}"
        "${text}"
```

Endi `ui.button "Saqlash"` bu custom komponentni ishlatadi. Boshqa `ui.*` komponentlar o'zgarmagan.

**Uch daraja xulosa:**

```
use ui  →  tayyor dastur  (0 ta custom kod)
theme { primary:"#..." }  →  brand moslash  (3-5 satr)
override ui.button { ... }  →  to'liq pixel-perfect  (xohlaganicha)
```

---

## To'liq gul do'koni dashboard (frontend + backend bir faylda)

```flux
use http db ai ui ws

# ============================================================
# SCHEMA
# ============================================================

tbl products
  id       serial pk
  name     str
  category str
  price    money
  stock    int
  image    str null
  ts       now

tbl orders
  id         serial pk
  customer   str
  total      money
  status     sym
  items      json
  note       str null
  ts         now

tbl customers
  id      serial pk
  name    str
  email   str uniq
  phone   str null
  city    str null
  ts      now

tbl settings
  id     serial pk
  key    str uniq
  value  json

# ============================================================
# THEME
# ============================================================

theme
  primary:  "#6366f1"
  success:  "#22c55e"
  danger:   "#ef4444"
  warning:  "#f59e0b"
  surface:  "#ffffff"
  bg:       "#f8fafc"
  border:   "#e2e8f0"
  muted:    "#94a3b8"
  font:     "Inter, sans-serif"
  radius:   "10px"

# ============================================================
# CUSTOM KOMPONENTLAR
# ============================================================

comp stat_card label value trend:nil color:"indigo"
  div .bg-white.rounded-xl.p-6.shadow-sm.border.border-slate-100
    div .flex.justify-between.items-start
      div
        p .text-sm.text-slate-500.mb-1 "${label}"
        p .text-3xl.font-bold.text-slate-800 "${value}"
      if trend != nil
        span
          ."text-sm font-medium px-2 py-1 rounded-full"
          ."bg-green-100 text-green-700" if trend >= 0
          ."bg-red-100 text-red-700" if trend < 0
          if trend >= 0
            "+${trend}%"
          else
            "${trend}%"

comp status_badge val
  span
    ."text-xs font-semibold px-2.5 py-1 rounded-full"
    ."bg-blue-100 text-blue-700"   if val == :new
    ."bg-yellow-100 text-yellow-700" if val == :processing
    ."bg-green-100 text-green-700" if val == :delivered
    ."bg-red-100 text-red-700"     if val == :cancelled
    match val
      :new        -> "Yangi"
      :processing -> "Jarayonda"
      :delivered  -> "Yetkazildi"
      :cancelled  -> "Bekor"
      _           -> "${val}"

comp nav_item href label icon active:false
  a .flex.items-center.gap-3.px-4.py-2.5.rounded-lg.cursor-pointer
    ."bg-indigo-600 text-white font-medium" if active
    ."text-slate-600 hover:bg-slate-100" if !active
    href:href
    span .text-lg "${icon}"
    span .text-sm "${label}"

comp product_row item
  tr .border-b.hover:bg-slate-50
    td .px-4.py-3
      div .flex.items-center.gap-3
        div .w-10.h-10.rounded-lg.bg-indigo-50.flex.items-center.justify-center
          span "🌸"
        div
          p .font-medium.text-slate-800 "${item.name}"
          p .text-xs.text-slate-400 "${item.category}"
    td .px-4.py-3.text-slate-700 "${item.price}"
    td .px-4.py-3
      span
        ."font-medium"
        ."text-green-600" if item.stock > 10
        ."text-yellow-600" if item.stock <= 10 & item.stock > 0
        ."text-red-600" if item.stock == 0
        "${item.stock} dona"
    td .px-4.py-3
      div .flex.gap-2
        ui.button "Tahrir" variant:"ghost" size:"sm"
          on:click(\-> selected_product <- item)
        ui.button "O'chir" variant:"danger-ghost" size:"sm"
          on:click(\-> delete_product item.id)

comp order_row ord
  tr .border-b.hover:bg-slate-50
    td .px-4.py-3.font-mono.text-sm.text-slate-600 "#${ord.id}"
    td .px-4.py-3.font-medium "${ord.customer}"
    td .px-4.py-3
      ui.select
        bind:ord_status_edit
        options:[{v::new l:"Yangi"} {v::processing l:"Jarayonda"} {v::delivered l:"Yetkazildi"} {v::cancelled l:"Bekor"}]
        value:ord.status
        on:change(\e -> update_order_status ord.id e.value)
    td .px-4.py-3.font-semibold "${ord.total} so'm"
    td .px-4.py-3.text-slate-500 "${ord.ts}"
    td .px-4.py-3
      ui.button "Batafsil" variant:"ghost" size:"sm"
        on:click(\-> open_order_modal ord)

# ============================================================
# LAYOUT
# ============================================================

layout :admin
  div .flex.h-screen.bg-slate-50 style:"font-family:${theme.font}"

    # Sidebar
    aside .w-64.bg-white.border-r.border-slate-200.flex.flex-col.shrink-0
      div .px-6.py-5.border-b.border-slate-100
        div .flex.items-center.gap-3
          span .text-2xl "🌺"
          div
            p .font-bold.text-slate-800 "FleurAdmin"
            p .text-xs.text-slate-400 "Gul do'koni boshqaruv"
      nav .flex-1.p-4.flex.flex-col.gap-1
        nav_item "/" "Bosh sahifa" "📊" active:(current_page == :index)
        nav_item "/products" "Mahsulotlar" "🌹" active:(current_page == :products)
        nav_item "/orders" "Buyurtmalar" "📦" active:(current_page == :orders)
        nav_item "/customers" "Mijozlar" "👥" active:(current_page == :customers)
        nav_item "/settings" "Sozlamalar" "⚙️" active:(current_page == :settings)
      div .p-4.border-t.border-slate-100
        div .flex.items-center.gap-3.px-3.py-2.rounded-lg.bg-slate-50
          div .w-8.h-8.rounded-full.bg-indigo-100.flex.items-center.justify-center
            span .text-sm "👤"
          div
            p .text-sm.font-medium.text-slate-700 "Admin"
            p .text-xs.text-slate-400 "Superuser"

    # Asosiy kontent
    div .flex-1.overflow-auto
      slot

# ============================================================
# 1. BOSH SAHIFA — Statistika + Grafik
# ============================================================

page :index "/" layout::admin
  load
    total_products = db.one "select count(*) c from products"
    total_orders   = db.one "select count(*) c from orders"
    total_revenue  = db.one "select coalesce(sum(total),0) s from orders where status != $1" [:cancelled]
    new_orders     = db.one "select count(*) c from orders where status=$1" [:new]
    recent_orders  = db.q "select * from orders order by ts desc limit 8"
    weekly_sales   = db.q """
      select date_trunc('day', ts) d, count(*) c, sum(total) s
      from orders
      where ts > now() - interval '7 days'
      group by d order by d
    """
    top_products   = db.q """
      select p.name, count(*) sold
      from orders o, jsonb_array_elements(o.items) item
      join products p on p.id = (item->>'id')::int
      group by p.name order by sold desc limit 5
    """

  state
    chart_mode <- :revenue    # :revenue | :orders

  view
    div .p-8

      # Header
      div .mb-8
        h1 .text-2xl.font-bold.text-slate-800 "Xush kelibsiz 👋"
        p .text-slate-500 "Gul do'koni umumiy ko'rinishi"

      # Stat kartalar
      div .grid.grid-cols-4.gap-5.mb-8
        stat_card "Jami mahsulot"  "${total_products.c} ta"  trend:3
        stat_card "Jami buyurtma" "${total_orders.c} ta"     trend:12
        stat_card "Daromad"       "${total_revenue.s} so'm"  trend:8   color:"green"
        stat_card "Yangi"         "${new_orders.c} ta"       color:"amber"

      # Grafik + Top mahsulotlar
      div .grid.grid-cols-3.gap-6.mb-8

        # Haftalik grafik (2/3)
        div .col-span-2.bg-white.rounded-xl.p-6.shadow-sm.border.border-slate-100
          div .flex.justify-between.items-center.mb-6
            h2 .font-semibold.text-slate-800 "Haftalik tahlil"
            div .flex.gap-2
              ui.button "Daromad" size:"sm"
                variant:(if chart_mode == :revenue then "primary" else "ghost")
                on:click(\-> chart_mode <- :revenue)
              ui.button "Buyurtma" size:"sm"
                variant:(if chart_mode == :orders then "primary" else "ghost")
                on:click(\-> chart_mode <- :orders)
          ui.chart
            type: :bar
            data: weekly_sales
            x:    "d"
            y:    (if chart_mode == :revenue then "s" else "c")
            color: theme.primary
            height: 240

        # Top mahsulotlar (1/3)
        div .bg-white.rounded-xl.p-6.shadow-sm.border.border-slate-100
          h2 .font-semibold.text-slate-800.mb-4 "Top mahsulotlar"
          div .flex.flex-col.gap-3
            each p in top_products
              div .flex.justify-between.items-center
                span .text-sm.text-slate-700 "${p.name}"
                span .text-sm.font-semibold.text-indigo-600 "${p.sold} ta"

      # So'nggi buyurtmalar
      div .bg-white.rounded-xl.shadow-sm.border.border-slate-100
        div .px-6.py-4.border-b.border-slate-100.flex.justify-between.items-center
          h2 .font-semibold.text-slate-800 "So'nggi buyurtmalar"
          link "/orders" "Hammasini ko'rish →" .text-sm.text-indigo-600
        table .w-full
          thead
            tr .bg-slate-50.text-left.text-xs.text-slate-500.uppercase
              th .px-4.py-3 "ID"
              th .px-4.py-3 "Mijoz"
              th .px-4.py-3 "Holat"
              th .px-4.py-3 "Summa"
              th .px-4.py-3 "Sana"
              th .px-4.py-3 ""
          tbody
            each ord in recent_orders
              order_row ord

# ============================================================
# 2. MAHSULOTLAR — CRUD + Qidiruv + Forma
# ============================================================

page :products "/products" layout::admin
  load
    all_products = db.q "select * from products order by name"
    categories   = db.q "select distinct category from products order by category"

  state
    search          <- ""
    selected_cat    <- "barchasi"
    selected_product <- nil      # tahrirlash uchun
    show_form       <- false
    form_name       <- ""
    form_category   <- ""
    form_price      <- ""
    form_stock      <- ""
    products        <- all_products

  action :save req
    if req.body.id
      db.up "products"
        {name:req.body.name category:req.body.category price:req.body.price stock:req.body.stock}
        {id:req.body.id}
    else
      db.ins "products" {name:req.body.name category:req.body.category price:req.body.price stock:req.body.stock}
    redirect "/products"

  action :delete req
    db.del "products" {id:req.body.id}
    redirect "/products"

  fn filter_products list q cat
    list
      |> \l -> if q != "" then l.filter \p -> str.has (str.low p.name) (str.low q) else l
      |> \l -> if cat != "barchasi" then l.filter \p -> p.category == cat else l

  fn delete_product id
    products <- products.filter \p -> p.id != id
    http.post "/products/delete" {id:id}

  fn open_edit p
    selected_product <- p
    form_name     <- p.name
    form_category <- p.category
    form_price    <- str.str p.price
    form_stock    <- str.str p.stock
    show_form     <- true

  fn reset_form
    selected_product <- nil
    form_name     <- ""
    form_category <- ""
    form_price    <- ""
    form_stock    <- ""
    show_form     <- false

  view
    div .p-8

      # Header
      div .flex.justify-between.items-center.mb-8
        div
          h1 .text-2xl.font-bold.text-slate-800 "Mahsulotlar"
          p .text-slate-500 "${products.len} ta mahsulot"
        ui.button "+ Yangi mahsulot" on:click(\-> show_form <- true)

      # Filtr qatori
      div .flex.gap-4.mb-6
        div .flex-1
          ui.input
            bind:search
            placeholder:"Mahsulot nomi bo'yicha qidiring..."
            icon:"🔍"
            on:input(\e ->
              search <- e.value
              products <- filter_products all_products search selected_cat
            )
        div .flex.gap-2
          each cat in [{v:"barchasi" l:"Barchasi"}].push(categories.map \c -> {v:c.category l:c.category})
            ui.button "${cat.l}" size:"sm"
              variant:(if selected_cat == cat.v then "primary" else "ghost")
              on:click(\->
                selected_cat <- cat.v
                products <- filter_products all_products search selected_cat
              )

      # Jadval
      div .bg-white.rounded-xl.shadow-sm.border.border-slate-100
        if products.len == 0
          div .py-16.text-center
            p .text-4xl.mb-3 "🌿"
            p .text-slate-500 "Mahsulot topilmadi"
        else
          table .w-full
            thead
              tr .bg-slate-50.text-left.text-xs.text-slate-500.uppercase
                th .px-4.py-3 "Mahsulot"
                th .px-4.py-3 "Narx"
                th .px-4.py-3 "Zaxira"
                th .px-4.py-3 ""
            tbody
              each item in products key:item.id
                product_row item

      # Forma modal
      if show_form
        div .fixed.inset-0.bg-black/40.z-50.flex.items-center.justify-center
          on:click(\e -> if e.target == e.currentTarget then reset_form())
          div .bg-white.rounded-2xl.shadow-2xl.w-full.max-w-md.p-8
            div .flex.justify-between.items-center.mb-6
              h2 .text-xl.font-bold.text-slate-800
                if selected_product then "Mahsulotni tahrirlash" else "Yangi mahsulot"
              button .text-slate-400.hover:text-slate-600 on:click(\-> reset_form()) "✕"
            form action::save method::post
              if selected_product
                input type:"hidden" name:"id" value:selected_product.id
              div .flex.flex-col.gap-4
                div
                  label .text-sm.font-medium.text-slate-700.mb-1 "Nomi *"
                  ui.input bind:form_name name:"name" placeholder:"Atirgul, lola..." required:true
                div
                  label .text-sm.font-medium.text-slate-700.mb-1 "Kategoriya *"
                  ui.input bind:form_category name:"category" placeholder:"Kesik gullar, Guldasta..." required:true
                div .grid.grid-cols-2.gap-4
                  div
                    label .text-sm.font-medium.text-slate-700.mb-1 "Narx (so'm) *"
                    ui.input bind:form_price name:"price" type:"number" placeholder:"15000" required:true
                  div
                    label .text-sm.font-medium.text-slate-700.mb-1 "Zaxira (dona) *"
                    ui.input bind:form_stock name:"stock" type:"number" placeholder:"100" required:true
                div .flex.justify-end.gap-3.pt-2
                  ui.button "Bekor" variant:"ghost" on:click(\-> reset_form())
                  ui.button "Saqlash" type:"submit"

# ============================================================
# 3. BUYURTMALAR — Jadval + Holat + Modal
# ============================================================

page :orders "/orders" layout::admin
  load
    orders     = db.q "select * from orders order by ts desc"
    order_stats = db.one """
      select
        count(*) filter (where status='new') new_c,
        count(*) filter (where status='processing') proc_c,
        count(*) filter (where status='delivered') del_c,
        count(*) filter (where status='cancelled') canc_c
      from orders
    """

  state
    filter_status <- "barchasi"
    modal_order   <- nil
    orders_list   <- orders

  action :update_status req
    db.up "orders" {status:req.body.status} {id:req.body.id}
    redirect "/orders"

  fn update_order_status id new_status
    orders_list <- orders_list.map \o ->
      if o.id == id then o.set "status" new_status else o
    http.post "/orders/update_status" {id:id status:new_status}

  fn open_order_modal ord
    modal_order <- ord

  view
    div .p-8

      # Header
      div .mb-8
        h1 .text-2xl.font-bold.text-slate-800 "Buyurtmalar"
        p .text-slate-500 "Barcha buyurtmalarni boshqaring"

      # Status kartalari
      div .grid.grid-cols-4.gap-4.mb-8
        div .bg-white.rounded-xl.p-4.border.border-blue-100
          p .text-2xl.font-bold.text-blue-600 "${order_stats.new_c}"
          p .text-sm.text-slate-500 "Yangi"
        div .bg-white.rounded-xl.p-4.border.border-yellow-100
          p .text-2xl.font-bold.text-yellow-600 "${order_stats.proc_c}"
          p .text-sm.text-slate-500 "Jarayonda"
        div .bg-white.rounded-xl.p-4.border.border-green-100
          p .text-2xl.font-bold.text-green-600 "${order_stats.del_c}"
          p .text-sm.text-slate-500 "Yetkazildi"
        div .bg-white.rounded-xl.p-4.border.border-red-100
          p .text-2xl.font-bold.text-red-600 "${order_stats.canc_c}"
          p .text-sm.text-slate-500 "Bekor"

      # Filtr
      div .flex.gap-2.mb-6
        each st in [{v:"barchasi" l:"Barchasi"} {v:"new" l:"Yangi"} {v:"processing" l:"Jarayonda"} {v:"delivered" l:"Yetkazildi"} {v:"cancelled" l:"Bekor"}]
          ui.button "${st.l}" size:"sm"
            variant:(if filter_status == st.v then "primary" else "ghost")
            on:click(\->
              filter_status <- st.v
              orders_list <- if st.v == "barchasi" then orders else orders.filter \o -> o.status == st.v
            )

      # Jadval
      div .bg-white.rounded-xl.shadow-sm.border.border-slate-100
        table .w-full
          thead
            tr .bg-slate-50.text-left.text-xs.text-slate-500.uppercase
              th .px-4.py-3 "Buyurtma"
              th .px-4.py-3 "Mijoz"
              th .px-4.py-3 "Holat"
              th .px-4.py-3 "Summa"
              th .px-4.py-3 "Sana"
              th .px-4.py-3 ""
          tbody
            each ord in orders_list key:ord.id
              order_row ord

      # Batafsil modal
      if modal_order != nil
        div .fixed.inset-0.bg-black/40.z-50.flex.items-center.justify-center
          div .bg-white.rounded-2xl.shadow-2xl.w-full.max-w-lg.p-8
            div .flex.justify-between.items-center.mb-6
              h2 .text-xl.font-bold "Buyurtma #${modal_order.id}"
              button .text-slate-400 on:click(\-> modal_order <- nil) "✕"
            div .space-y-4
              div .flex.justify-between
                span .text-slate-500 "Mijoz"
                span .font-medium "${modal_order.customer}"
              div .flex.justify-between
                span .text-slate-500 "Holat"
                status_badge modal_order.status
              div .flex.justify-between
                span .text-slate-500 "Summa"
                span .font-bold.text-lg "${modal_order.total} so'm"
              if modal_order.items
                div
                  p .text-slate-500.mb-2 "Mahsulotlar:"
                  div .bg-slate-50.rounded-lg.p-3.space-y-2
                    each it in modal_order.items
                      div .flex.justify-between.text-sm
                        span "${it.name} × ${it.qty}"
                        span "${it.price} so'm"
              if modal_order.note
                div
                  p .text-slate-500.mb-1 "Izoh:"
                  p .text-sm.bg-amber-50.p-3.rounded-lg "${modal_order.note}"
            div .flex.justify-end.gap-3.mt-6
              ui.button "Yopish" variant:"ghost" on:click(\-> modal_order <- nil)

# ============================================================
# 4. MIJOZLAR
# ============================================================

page :customers "/customers" layout::admin
  load
    customers  = db.q "select c.*, count(o.id) orders from customers c left join orders o on o.customer=c.name group by c.id order by c.name"

  state
    search    <- ""
    show_form <- false
    form_name  <- ""
    form_email <- ""
    form_phone <- ""
    form_city  <- ""
    cust_list  <- customers

  action :add_customer req
    db.ins "customers" {name:req.body.name email:req.body.email phone:req.body.phone city:req.body.city}
    redirect "/customers"

  view
    div .p-8

      div .flex.justify-between.items-center.mb-8
        div
          h1 .text-2xl.font-bold.text-slate-800 "Mijozlar"
          p .text-slate-500 "${cust_list.len} ta mijoz"
        ui.button "+ Yangi mijoz" on:click(\-> show_form <- true)

      div .mb-6
        ui.input
          bind:search
          placeholder:"Ism yoki email bo'yicha qidiring..."
          icon:"🔍"
          on:input(\e ->
            search <- e.value
            cust_list <- if e.value == ""
              then customers
              else customers.filter \c ->
                (str.has (str.low c.name) (str.low e.value)) |
                (str.has (str.low c.email) (str.low e.value))
          )

      div .bg-white.rounded-xl.shadow-sm.border.border-slate-100
        table .w-full
          thead
            tr .bg-slate-50.text-left.text-xs.text-slate-500.uppercase
              th .px-4.py-3 "Ism"
              th .px-4.py-3 "Email"
              th .px-4.py-3 "Telefon"
              th .px-4.py-3 "Shahar"
              th .px-4.py-3 "Buyurtmalar"
          tbody
            each c in cust_list key:c.id
              tr .border-b.hover:bg-slate-50
                td .px-4.py-3
                  div .flex.items-center.gap-3
                    div .w-8.h-8.rounded-full.bg-indigo-100.flex.items-center.justify-center.text-sm.font-bold.text-indigo-600
                      "${str.slice c.name 0 1}"
                    span .font-medium "${c.name}"
                td .px-4.py-3.text-slate-600 "${c.email}"
                td .px-4.py-3.text-slate-600 "${c.phone ?? "—"}"
                td .px-4.py-3.text-slate-600 "${c.city ?? "—"}"
                td .px-4.py-3
                  span .bg-indigo-50.text-indigo-700.text-xs.font-semibold.px-2.py-1.rounded-full
                    "${c.orders} ta"

      if show_form
        div .fixed.inset-0.bg-black/40.z-50.flex.items-center.justify-center
          div .bg-white.rounded-2xl.p-8.w-full.max-w-md
            div .flex.justify-between.mb-6
              h2 .text-xl.font-bold "Yangi mijoz"
              button on:click(\-> show_form <- false) "✕"
            form action::add_customer method::post
              div .flex.flex-col.gap-4
                ui.input bind:form_name name:"name" placeholder:"To'liq ism" required:true
                ui.input bind:form_email name:"email" type:"email" placeholder:"email@misol.uz" required:true
                ui.input bind:form_phone name:"phone" placeholder:"+998 90 000 00 00"
                ui.input bind:form_city name:"city" placeholder:"Toshkent"
                div .flex.justify-end.gap-3
                  ui.button "Bekor" variant:"ghost" on:click(\-> show_form <- false)
                  ui.button "Qo'shish" type:"submit"

# ============================================================
# 5. SOZLAMALAR
# ============================================================

page :settings "/settings" layout::admin
  load
    cfg = db.one "select value from settings where key=$1" ["shop_config"]
    config <- cfg.value ?? {shop_name:"FleurAdmin" currency:"so'm" tax_rate:12 notify_email:""}

  state
    saved <- false
    shop_name    <- config.shop_name
    currency     <- config.currency
    tax_rate     <- str.str config.tax_rate
    notify_email <- config.notify_email

  action :save_settings req
    db.put "settings" {value:{
      shop_name:   req.body.shop_name
      currency:    req.body.currency
      tax_rate:    str.int req.body.tax_rate
      notify_email:req.body.notify_email
    }} {key:"shop_config"}
    redirect "/settings"

  view
    div .p-8.max-w-2xl

      div .mb-8
        h1 .text-2xl.font-bold.text-slate-800 "Sozlamalar"
        p .text-slate-500 "Do'kon konfiguratsiyasi"

      div .bg-white.rounded-xl.shadow-sm.border.border-slate-100.p-8
        form action::save_settings method::post
          div .flex.flex-col.gap-6

            div
              label .text-sm.font-medium.text-slate-700.block.mb-2 "Do'kon nomi"
              ui.input bind:shop_name name:"shop_name" placeholder:"FleurAdmin"

            div
              label .text-sm.font-medium.text-slate-700.block.mb-2 "Valyuta"
              ui.select
                bind:currency
                name:"currency"
                options:[{v:"so'm" l:"So'm (UZS)"} {v:"USD" l:"Dollar (USD)"} {v:"EUR" l:"Yevro (EUR)"}]

            div
              label .text-sm.font-medium.text-slate-700.block.mb-2 "Soliq foizi (%)"
              ui.input bind:tax_rate name:"tax_rate" type:"number" placeholder:"12"

            div
              label .text-sm.font-medium.text-slate-700.block.mb-2 "Bildirishnoma email"
              ui.input bind:notify_email name:"notify_email" type:"email" placeholder:"admin@fleur.uz"

            div .pt-2.border-t.border-slate-100.flex.justify-end
              ui.button "Sozlamalarni saqlash" type:"submit"

# ============================================================
# BACKEND API (live-update uchun)
# ============================================================

http.on :post "/orders/update_status" \req ->
  db.up "orders" {status:req.body.status} {id:req.body.id}
  ws.room.send "admin" (json.enc {type:"order_updated" id:req.body.id status:req.body.status})
  rep 200 {ok:true}

http.on :post "/products/delete" \req ->
  db.del "products" {id:req.body.id}
  rep 200 {ok:true}

http.on :get "/api/stats" \req ->
  revenue = db.one "select coalesce(sum(total),0) s from orders where ts > $1 and status != $2" [time.ago 30 :day :cancelled]
  rep 200 {revenue:revenue.s}

ws.on :connect \conn ->
  ws.room.join conn "admin"

ws.serve 9000
http.serve 8080
```

---

## Token tahlili

**Aniq taqqoslash — bitta stat karta komponenti:**

| Yondashuv | Kod satrlari | Taxminiy token |
|---|---|---|
| React (JSX + TypeScript) | ~35 satr | ~420 token |
| Vue 3 (Composition API) | ~28 satr | ~350 token |
| Svelte | ~20 satr | ~240 token |
| **Flux (bu dizayn)** | **7 satr** | **~65 token** |

```flux
# Flux — 7 satr, ~65 token
comp stat_card label value trend:nil color:"indigo"
  div .bg-white.rounded-xl.p-6.shadow-sm
    p .text-sm.text-slate-500 "${label}"
    p .text-3xl.font-bold "${value}"
    if trend != nil
      span ."text-green-600" if trend >= 0 else ."text-red-600"
        "${trend}%"
```

```tsx
// React — 35 satr, ~420 token
interface StatCardProps {
  label: string; value: string;
  trend?: number; color?: string;
}
export const StatCard: React.FC<StatCardProps> = ({
  label, value, trend, color = "indigo"
}) => (
  <div className="bg-white rounded-xl p-6 shadow-sm">
    <p className="text-sm text-slate-500">{label}</p>
    <p className="text-3xl font-bold">{value}</p>
    {trend !== undefined && (
      <span className={trend >= 0 ? "text-green-600" : "text-red-600"}>
        {trend}%
      </span>
    )}
  </div>
);
```

**Butun dashboard taqqoslash:**

| | Flux | React+Next.js ekvivalenti |
|---|---|---|
| Fayl soni | 1 | 25-40 (pages, components, api routes, types, hooks) |
| Jami satr | ~380 | ~1800-2500 |
| Token (taxmin) | ~4,200 | ~22,000-30,000 |
| Backend/frontend integratsiya | nol glue kod | fetch/axios + API routes |
| **Qisqarish** | — | **~5-7x kam token** |

Asosiy token tejash manbalari:
- `load` bloki = `getServerSideProps` + API route + `useEffect` + `useState` — to'rttasini bitta blokka siqadi.
- `action` = `<form onSubmit>` + `preventDefault` + `fetch POST` + `router.push` — hammasini ikki satrga.
- `bind:x` = `value={x} onChange={e => setX(e.target.value)}` — 6x qisqa.
- `comp` = `interface Props + export const + return` deklaratsiyasiz.
- `layout` = `_app.tsx` + `<Layout>` wrapper — avtomatik.

---

## Flux'ga qo'shilishi kerak bo'lgan runtime imkoniyatlari

### 1. Transpile target: "zero-JS by default, sprinkle JS when needed"

Runtime ikki rejimda ishlaydi:

**Server-driven (default):** `state` o'zgaruvchilari server-side saqlanadi. `on:click` server'ga minimal WebSocket/fetch jo'natadi, server diff hisoblab, faqat o'zgargan DOM qismini qaytaradi. Mijoz tomonida vanilla JS fragment patcher (< 5 KB) — React/Vue bundle yo'q. Bu Phoenix LiveView / Hotwire Turbo uslubi.

**Client hydration (ixtiyoriy):** `page` annotatsiyasi bilan `client:true` qo'shilsa, Flux runtime o'sha sahifani Signals-based reaktiv JS'ga transpile qiladi (Preact Signals yoki SolidJS uslubi). Faqat kerak bo'lganda.

### 2. Signals-based reaktivlik

`state` bloki ichidagi `<-` o'zgaruvchilar runtime'da Signals (fine-grained reaktivlik) sifatida kompilyatsiya qilinadi. Butun sahifa re-render bo'lmaydi — faqat `${count}` ni ishlatgan DOM node'lar yangilanadi. Bu Svelte/SolidJS kompilator yondashuvi.

### 3. `load` — server component pattern

`load` bloki faqat server'da bajariladi, natija HTML ichiga inline qilinadi yoki JSON stream sifatida keladi. `db.q` to'g'ridan-to'g'ri `load` ichida — alohida API endpoint kompilyator tomonidan avtomatik yaratiladi (ixtiyoriy expose bilan).

### 4. `action` — progressive enhancement

`action` bloki HTML native `<form method="POST">` ga kompilyatsiya qilinadi — JavaScript o'chiq bo'lsa ham ishlaydi. JS yoqilganda, runtime uni `fetch` + optimistic UI ga upgrade qiladi.

### 5. `comp` — server component vs client component

```flux
comp badge text        # server component (default) — stateless, HTML render
comp counter           # client component (state bor → avtomatik client)
  state
    n <- 0
```

Kompilyator `state` borligini ko'rib, avtomatik client-side komponent sifatida belgilaydi.

### 6. `ui.*` runtime registry

Built-in `ui.*` komponentlar runtime registrida saqlanadi. `override` kompilyatsiya vaqtida registry'ni yangilaydi — tree-shaking ishlaydi (ishlatilmagan `ui.*` bundle'ga kirmaydi).

### 7. CSS chiqishi

View ichidagi `.class.names` Tailwind CSS utility class'lari sifatida interpret qilinadi. Runtime Tailwind CDN yoki lokal purge+build ishlatadi. `theme {}` bloki Tailwind `theme.extend` ga yoki CSS custom properties (`--flux-primary`) ga transpile qilinadi.

**Xulosa:** Flux frontend runtime = Phoenix LiveView arxitekturasi (server-driven, minimal JS) + Svelte kompilatori (Signals, fine-grained update) + Tailwind (utility CSS) + HTML native forms (progressive enhancement). Hech biri yangi ixtiro emas — lekin Flux ularni bitta, minimal-token, organik sintaksisda birlashtiradi.