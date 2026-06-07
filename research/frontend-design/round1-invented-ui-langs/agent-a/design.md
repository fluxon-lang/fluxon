I'll design a new UI language for this task. Let me write the complete response.

# Til nomi: Petal

## Asosiy g'oya

Petal — UI yozish uchun **deklarativ, reaktiv va "data-first"** til. Asosiy g'oya shundaki: HTML/JSX'da struktura, JS'da mantiq, CSS'da stil — uchta alohida joyda yashaydi va dasturchi (yoki AI) doimiy ravishda ular orasida sakraydi. Petal'da esa **bir komponent = bir blok**, va shu blok ichida state, ko'rinish, hodisa va stil **bir-biriga yaqin, lokal** turadi. Lekin JSX'dan farqli o'laroq, Petal **indentatsiyaga asoslangan daraxt** (Python kabi) ishlatadi — yopiluvchi teglar yo'q, shuning uchun matn shovqini kam, AI uchun "tegni yopishni unutish" xatosi mavjud emas.

Ikkinchi asosiy g'oya — **reaktivlik avtomatik**. `state` ichidagi har qanday qiymat o'zgarsa, uni ishlatgan har bir joy o'zi qayta chiziladi; hech qanday `useState`, `setState`, `ref`, yoki "dependency array" yo'q. Data binding ikki tomonlama: `bind` kalit so'zi input bilan state'ni bog'laydi. Backend bilan aloqa tilga **birinchi darajali** kiritilgan: `source` blogi REST/GraphQL endpoint'ni reaktiv state sifatida e'lon qiladi (yuklanish, xato, qayta yuklash holatlari avtomatik). Stil esa komponent ichida `style:` bilan yoziladi va **dizayn tokenlar** (`@theme`) orqali markazlashtiriladi. Bu uch narsa — lokal joylashuv, avtomatik reaktivlik, va integratsiyalangan data — AI uchun kod yozishni soddalashtiradi, chunki har bir komponent o'zini o'zi tushuntiradi va boshqa fayllarga bog'liqlik minimal.

## Sintaksis qoidalari

**Komponent / view:** `view Nom(parametrlar):` bilan e'lon qilinadi. Indentatsiya daraxtni belgilaydi. Element yozish: `tag .class #id "matn"`.

**State:** komponent ichida `state:` bloki. Reaktiv — o'zgarsa, UI yangilanadi. Hosila qiymat `derive nom = ifoda` (computed).

**Event:** `on hodisa -> harakat`. Masalan `on click -> count += 1`. Funksiya: `fn nom(args): ...`.

**Data binding:** `bind value <-> state.field` (ikki tomonlama). Bir tomonlama: `text: state.x` yoki `{state.x}` interpolatsiya.

**Stil:** element ostida `style:` bloki yoki inline `style(color: red)`. Global tokenlar `@theme { ... }`.

**Ro'yxat / loop:** `for item in collection:` — ostidagi blok har element uchun takrorlanadi. `key: item.id`.

**Shart:** `if ifoda:` / `elif` / `else:`. Inline: `show: ifoda` (element ko'rsatiladi/yashiriladi).

**Backend / data:** `source nom = get "url"` reaktiv resurs yaratadi; `nom.data`, `nom.loading`, `nom.error`, `nom.reload()`. Mutatsiya: `post "url" body: {...}`.

**Routing:** `route "/path" -> View`. Navigatsiya: `go("/path")`.

## To'liq dashboard kodi

```petal
# ============================================================
#  Petal — Gul do'koni admin dashboard
# ============================================================

@theme:
  color.bg        = "#fbf7f4"
  color.surface   = "#ffffff"
  color.primary   = "#d6336c"      # gul-pushti
  color.primary-2 = "#f06595"
  color.text      = "#2b2b2b"
  color.muted     = "#8a8a8a"
  color.success   = "#2f9e44"
  color.warn      = "#e8590c"
  radius          = 14px
  shadow          = "0 6px 24px rgba(0,0,0,.06)"
  font            = "Inter, system-ui, sans-serif"

# --- Global do'kon sozlamalari (butun app bo'ylab reaktiv) -----
store Shop:
  name      = "Lola Gullari"
  open      = "09:00"
  close     = "21:00"
  accent    = "#d6336c"

# --- Backend resurslari (reaktiv, avto loading/error) ----------
source orders   = get "/api/orders"
source products = get "/api/products"
source customers= get "/api/customers"
source stats    = get "/api/stats/today"   # {revenue, count, topFlower, chart[]}

# ============================================================
#  ROOT — layout + routing
# ============================================================
view App:
  state:
    section = "home"          # active sidebar item

  layout .shell:
    style:
      display: grid
      grid-template-columns: 248px 1fr
      min-height: 100vh
      background: @color.bg
      color: @color.text
      font-family: @color.font  # token

    Sidebar(active: state.section, onNav: fn(s): state.section = s)

    main .content:
      style: padding: 28px; overflow-y: auto

      if state.section == "home":     Home
      elif state.section == "products": Products
      elif state.section == "orders":   Orders
      elif state.section == "customers":Customers
      elif state.section == "settings": Settings

# ============================================================
#  SIDEBAR
# ============================================================
view Sidebar(active, onNav):
  aside .sidebar:
    style:
      background: @color.surface
      border-right: 1px solid #eee
      padding: 22px 16px
      display: flex
      flex-direction: column
      gap: 6px

    div .brand:
      style: display:flex; align-items:center; gap:10px; margin-bottom:24px
      span "🌷"  style(font-size: 26px)
      h1 {Shop.name}
        style: font-size: 18px; font-weight: 700; color: @color.primary

    # menyu — ro'yxat orqali
    state:
      items = [
        {id:"home",      label:"Bosh sahifa",  icon:"📊"},
        {id:"products",  label:"Mahsulotlar",  icon:"🌹"},
        {id:"orders",    label:"Buyurtmalar",  icon:"📦"},
        {id:"customers", label:"Mijozlar",     icon:"👤"},
        {id:"settings",  label:"Sozlamalar",   icon:"⚙️"},
      ]

    for it in items:
      key: it.id
      button .nav-item:
        on click -> onNav(it.id)
        class.active: active == it.id        # shartli klass
        style:
          display:flex; align-items:center; gap:12px
          padding:11px 14px; border-radius:@radius
          border:none; background:transparent; cursor:pointer
          font-size:15px; color:@color.text; text-align:left
        style.active:                          # .active holati uchun
          background:@color.primary; color:#fff
        span {it.icon}
        span {it.label}

# ============================================================
#  BOSH SAHIFA — statistika + grafik
# ============================================================
view Home:
  section:
    h2 "Bugungi savdo"  style(margin-bottom: 18px; font-size: 24px)

    if stats.loading:
      Skeleton(rows: 3)
    elif stats.error:
      Alert(kind:"error", text: stats.error.message, onRetry: stats.reload)
    else:
      div .cards:
        style: display:grid; grid-template-columns:repeat(3,1fr); gap:18px
        StatCard(title:"Jami daromad", value:`{stats.data.revenue} so'm`, icon:"💰")
        StatCard(title:"Buyurtmalar",  value: stats.data.count,          icon:"📦")
        StatCard(title:"Eng ko'p sotilgan", value: stats.data.topFlower, icon:"🌟")

      Card:
        style: margin-top:22px
        h3 "Haftalik daromad"  style(margin-bottom:14px)
        Chart(data: stats.data.chart, x:"day", y:"amount")

# Qayta ishlatiladigan statistika kartasi
view StatCard(title, value, icon):
  Card:
    style: display:flex; align-items:center; gap:16px
    div .ic style(font-size:30px) {icon}
    div:
      p {title}  style(color:@color.muted; font-size:13px)
      strong {value} style(font-size:26px; font-weight:700)

# Oddiy ustunli diagramma — built-in primitive ustida
view Chart(data, x, y):
  derive max = data.map(d -> d[y]).max()
  div .chart:
    style: display:flex; align-items:flex-end; gap:14px; height:200px
    for d in data:
      key: d[x]
      div .bar-wrap style(flex:1; text-align:center):
        div .bar:
          style:
            height: `{(d[y] / max) * 100}%`
            background: linear-gradient(@color.primary-2, @color.primary)
            border-radius: 8px 8px 0 0
            transition: height .4s ease
          # interaktiv tooltip
          on hover -> this.tip = true
          tooltip show: this.tip  text: `{d[y]} so'm`
        small {d[x]} style(color:@color.muted)

# ============================================================
#  MAHSULOTLAR
# ============================================================
view Products:
  state:
    query   = ""
    stockOnly = false
    editing = none          # tahrirlanayotgan mahsulot yoki none
    showForm = false

  derive filtered = products.data
      .filter(p -> p.name.lower().has(query.lower()))
      .filter(p -> !stockOnly or p.stock > 0)

  fn save(form):
    if form.id:
      post `/api/products/{form.id}` method:"PUT" body: form
    else:
      post "/api/products" body: form
    products.reload()
    state.showForm = false
    state.editing  = none

  fn remove(id):
    if confirm("O'chirilsinmi?"):
      post `/api/products/{id}` method:"DELETE"
      products.reload()

  section:
    div .head:
      style: display:flex; justify-content:space-between; align-items:center
      h2 "Mahsulotlar"
      button .primary "＋ Yangi gul"
        on click -> { state.editing = none; state.showForm = true }
        style: background:@color.primary; color:#fff; border:none
               padding:10px 18px; border-radius:@radius; cursor:pointer

    # qidiruv / filtr
    div .filters style(display:flex; gap:12px; margin:18px 0):
      input .search:
        placeholder: "Gul nomi bo'yicha qidirish..."
        bind value <-> state.query
        style: flex:1; padding:10px 14px; border:1px solid #ddd; border-radius:@radius
      label style(display:flex; align-items:center; gap:8px):
        input type:"checkbox" bind checked <-> state.stockOnly
        span "Faqat omborda bor"

    if products.loading: Skeleton(rows:4)
    else:
      div .grid:
        style: display:grid; grid-template-columns:repeat(auto-fill,minmax(220px,1fr)); gap:18px
        for p in filtered:
          key: p.id
          Card .product:
            img src:p.image alt:p.name
              style: width:100%; height:150px; object-fit:cover; border-radius:10px
            h4 {p.name}  style(margin-top:10px)
            p `{p.price} so'm`  style(color:@color.primary; font-weight:700)
            p:
              text: `Omborda: {p.stock} dona`
              style.warn: p.stock < 5      # kam qolganda qizil
              style(color:@color.muted; font-size:13px)
            style.warn for p: color:@color.warn  # token-shartli stil
            div .actions style(display:flex; gap:8px; margin-top:10px):
              button "Tahrir"  on click -> { state.editing = p; state.showForm = true }
              button "O'chir"  on click -> remove(p.id)  style(color:@color.warn)

    # qo'shish/tahrirlash modali
    if state.showForm:
      ProductForm(item: state.editing, onSave: save, onClose: fn(): state.showForm = false)

# Mahsulot formasi (qo'shish + tahrirlash bitta)
view ProductForm(item, onSave, onClose):
  state:
    form = item ?? {name:"", price:0, stock:0, image:""}   # ?? = default

  Modal(title: item ? "Mahsulotni tahrirlash" : "Yangi gul", onClose: onClose):
    field "Nom":     input bind value <-> state.form.name
    field "Narx":    input type:"number" bind value <-> state.form.price
    field "Ombor":   input type:"number" bind value <-> state.form.stock
    field "Rasm URL":input bind value <-> state.form.image
    div .modal-actions style(display:flex; gap:10px; justify-content:flex-end; margin-top:18px):
      button "Bekor"  on click -> onClose()
      button .primary "Saqlash"  on click -> onSave(state.form)

# ============================================================
#  BUYURTMALAR
# ============================================================
view Orders:
  state:
    selected = none      # ochilgan buyurtma tafsiloti
    filterStatus = "all"

  state:
    statuses = ["all","yangi","tayyorlanmoqda","yetkazilmoqda","bajarildi"]

  derive rows = orders.data
      .filter(o -> state.filterStatus == "all" or o.status == state.filterStatus)

  fn setStatus(order, st):
    post `/api/orders/{order.id}/status` body: {status: st}
    orders.reload()

  section:
    h2 "Buyurtmalar"

    div .tabs style(display:flex; gap:8px; margin:16px 0):
      for s in statuses:
        key: s
        button:
          text: s
          class.active: state.filterStatus == s
          on click -> state.filterStatus = s
          style.active: background:@color.primary; color:#fff

    table .orders:
      style: width:100%; border-collapse:collapse; background:@color.surface
             border-radius:@radius; overflow:hidden; box-shadow:@shadow
      thead:
        tr:
          th "ID"  th "Mijoz"  th "Gullar"  th "Jami"  th "Holat"  th ""
      tbody:
        for o in rows:
          key: o.id
          tr:
            on click -> state.selected = o
            style: cursor:pointer; border-top:1px solid #f0f0f0
            td `#{o.id}`
            td {o.customer.name}
            td {o.items.length} " ta"
            td `{o.total} so'm`
            td:
              StatusBadge(status: o.status)
            td:
              # holatni o'zgartirish (jadval ichida)
              select bind value <-> o.status  on change -> setStatus(o, o.status)
                on click -> stop()    # qatorga bosishni to'xtatish
                for s in statuses.skip(1):
                  option value:s {s}

    # tafsilot modali
    if state.selected != none:
      OrderDetail(order: state.selected, onClose: fn(): state.selected = none,
                  onStatus: setStatus)

view StatusBadge(status):
  derive c = match status:
    "yangi"          -> @color.primary
    "tayyorlanmoqda" -> @color.warn
    "yetkazilmoqda"  -> "#1c7ed6"
    "bajarildi"      -> @color.success
    _                -> @color.muted
  span .badge:
    text: status
    style: background:`{c}22`; color:c; padding:4px 12px
           border-radius:999px; font-size:12px; font-weight:600

view OrderDetail(order, onClose, onStatus):
  Modal(title:`Buyurtma #{order.id}`, onClose:onClose):
    p `Mijoz: {order.customer.name}`
    p `Telefon: {order.customer.phone}`
    p `Manzil: {order.address}`
    hr
    h4 "Gullar:"
    for it in order.items:
      key: it.id
      div .line style(display:flex; justify-content:space-between; padding:6px 0):
        span `{it.name} × {it.qty}`
        span `{it.price * it.qty} so'm`
    hr
    div style(display:flex; justify-content:space-between; font-weight:700):
      span "Jami:"  span `{order.total} so'm`
    div .modal-actions style(margin-top:16px; display:flex; gap:8px):
      button .primary "Tayyor deb belgilash"
        on click -> { onStatus(order, "bajarildi"); onClose() }

# ============================================================
#  MIJOZLAR
# ============================================================
view Customers:
  state:
    opened = none

  section:
    h2 "Mijozlar"
    div .grid style(display:grid; grid-template-columns:repeat(auto-fill,minmax(260px,1fr)); gap:16px; margin-top:16px):
      for c in customers.data:
        key: c.id
        Card:
          on click -> state.opened = (state.opened == c.id ? none : c.id)
          style: cursor:pointer
          div style(display:flex; align-items:center; gap:12px):
            div .avatar style(width:42px; height:42px; border-radius:50%; background:@color.primary-2; color:#fff; display:grid; place-items:center) {c.name[0]}
            div:
              strong {c.name}
              p {c.phone} style(color:@color.muted; font-size:13px)
          p `Jami buyurtmalar: {c.orderCount} • {c.totalSpent} so'm`
             style(margin-top:10px; font-size:13px)

          # tarix — kengaytiriladigan
          if state.opened == c.id:
            div .history style(margin-top:12px; border-top:1px solid #eee; padding-top:10px):
              h5 "Buyurtmalar tarixi"
              for h in c.history:
                key: h.id
                div style(display:flex; justify-content:space-between; font-size:13px; padding:4px 0):
                  span {h.date}
                  span `{h.total} so'm`
                  StatusBadge(status: h.status)

# ============================================================
#  SOZLAMALAR
# ============================================================
view Settings:
  state:
    draft = { name: Shop.name, open: Shop.open, close: Shop.close, accent: Shop.accent }
    saved = false

  fn apply():
    Shop.name   = state.draft.name      # global store yangilanadi -> butun UI reaktiv
    Shop.open   = state.draft.open
    Shop.close  = state.draft.close
    Shop.accent = state.draft.accent
    @theme.color.primary = state.draft.accent   # mavzu rangini live o'zgartirish
    post "/api/settings" body: state.draft
    state.saved = true

  section style(max-width:560px):
    h2 "Sozlamalar"
    Card style(margin-top:16px):
      field "Do'kon nomi":  input bind value <-> state.draft.name
      div style(display:flex; gap:14px):
        field "Ochilish":   input type:"time" bind value <-> state.draft.open
        field "Yopilish":   input type:"time" bind value <-> state.draft.close
      field "Mavzu rangi":
        div style(display:flex; gap:10px; align-items:center):
          input type:"color" bind value <-> state.draft.accent
          # jonli ko'rinish
          span .preview style(background:state.draft.accent; width:40px; height:40px; border-radius:8px)
      button .primary "Saqlash"  on click -> apply()  style(margin-top:16px)
      if state.saved:
        p "✓ Saqlandi"  style(color:@color.success; margin-top:8px)

# ============================================================
#  UMUMIY (reusable) PRIMITIVELAR
# ============================================================
view Card:
  div .card:
    style:
      background:@color.surface; border-radius:@radius
      box-shadow:@shadow; padding:18px
    slot     # ichki kontent shu yerga joylashadi

view field(label):
  label .field style(display:block; margin-bottom:12px):
    span {label} style(display:block; font-size:13px; color:@color.muted; margin-bottom:5px)
    slot

view Modal(title, onClose):
  div .overlay:
    on click -> onClose()
    style: position:fixed; inset:0; background:rgba(0,0,0,.4)
           display:grid; place-items:center; z-index:50
    div .modal:
      on click -> stop()    # ichki bosishlar overlay'ga o'tmasin
      style: background:@color.surface; border-radius:@radius
             padding:24px; width:min(480px,92vw); box-shadow:@shadow
      div style(display:flex; justify-content:space-between; align-items:center; margin-bottom:14px):
        h3 {title}
        button "✕"  on click -> onClose()  style(border:none; background:none; cursor:pointer)
      slot

view Alert(kind, text, onRetry):
  div .alert:
    style.error: background:"#fff0f0"; color:@color.warn
    style: padding:14px; border-radius:@radius
    span {text}
    if onRetry: button "Qayta urinish"  on click -> onRetry()

view Skeleton(rows):
  div:
    for i in range(rows):
      key: i
      div .sk style(height:60px; border-radius:@radius; margin-bottom:12px
                    background:linear-gradient(90deg,#eee,#f5f5f5,#eee)
                    animation: shimmer 1.2s infinite)

# ============================================================
#  ROUTING (ixtiyoriy — URL bilan bo'limlar)
# ============================================================
routes:
  "/"           -> App
  "/products"   -> App(section:"products")
  "/orders"     -> App(section:"orders")

mount App -> "#root"
```

## O'z-o'zini baholash

**Kuchli tomonlari:**
- **Lokal joylashuv (locality of behavior):** har bir komponentda struktura, state, event va stil yonma-yon — AI butun mantiqni bitta blok ichida ko'radi, fayllar orasida kontekst yo'qotmaydi.
- **Avtomatik reaktivlik:** `useState`/`setState`/dependency array yo'q. Bu AI tez-tez qiladigan xatolarni (eskirgan closure, yetishmayotgan dependency, `key` unutish — bu yerda `key:` aniq talab qilinadi) kamaytiradi.
- **Birinchi darajali data:** `source ... = get "url"` `.loading`/`.error`/`.reload()` bilan — AI loading/error holatlarini unutmasligi uchun til o'zi turtki beradi.
- **Indentatsiya daraxti:** yopiluvchi teglar yo'q, demak "tegni yopmaslik" yoki noto'g'ri joylashtirish xatosi tuzilmaviy jihatdan imkonsiz.
- **Integratsiyalangan stil tokenlari (`@theme`):** ranglarni jonli o'zgartirish (sozlamalardagi mavzu rangi) deklarativ — AI dizayn izchilligini oson saqlaydi.

**Zaif tomonlari:**
- **Indentatsiyaga sezgirlik:** chuqur ichma-ich daraxtlarda probel xatolari Python kabi qiyinchilik tug'diradi; mexanik tahrirlash mo'rt bo'lishi mumkin.
- **"Sehrli" reaktivlik:** qiymat qachon va nega qayta chizilishini debug qilish qiyin bo'lishi mumkin — yashirin bog'liqliklar ko'rinmaydi.
- **Mutatsiya + reaktivlik:** jadvalda `bind value <-> o.status` to'g'ridan-to'g'ri resurs ma'lumotini o'zgartiradi; haqiqiy implementatsiyada optimistik yangilanish/rollback semantikasi aniqroq belgilanishi kerak.
- **Ekotizim yo'qligi:** yangi til — kutubxonalar, tip tekshiruvi, IDE qo'llab-quvvatlashi nolga teng; jiddiy loyihada bu real to'siq.

**Nega AI buni oson yozadi:** sintaksis takrorlanuvchi va bashorat qilinadigan (`view`, `state:`, `on ... ->`, `for ... in`), bo'lak shovqini (boilerplate) kam, va har bir tushuncha bitta aniq kalit so'zga ega — bu modelning grammatikani izchil ushlab turishini osonlashtiradi.