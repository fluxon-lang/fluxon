# Flux Frontend — dizayn

## Asosiy g'oya

Flux'ga **UI qatlami** qo'shish — Flux'ning "bir ish = bir yo'l", kam-token, batteries-included falsafasini frontend'ga o'tkazish. Bugun AI agentlar REST API yozarkan, ertaga complete web-app'ni **bitta .fx faylda** yozishi kerak. 

Frontend qatlami **uch daraja modeli** bilan ishlaydi: (1) **DEFAULT** — tayyor UI komponentlar (shadcn/ui-ga o'xshash), AI hech narsa yozmaydi; (2) **CONFIG** — rang/tema/kichik o'zgarish; (3) **OVERRIDE** — xohlovchilar default komponentlarni o'zi qayta yozib butunlay moslashtiradi. 

Backend va frontend **BIR tilda, BIR faylda** (opsional import bilan alohida faylga ajratish); `http.on` API'ni, `dom` batareyasi UI komponentlarni, `<-` reaktiv stateni boshqaradi. Transpile target: **Bun.js + TypeScript + React**, ishga tushma: `flux run app.fx` → HTTP server + hot reload'li dev server, production: `flux build app.fx` → static HTML + JS.

---

## Yangi primitivlar (qo'shimchalar)

### 1. **`dom` battery** — UI komponent va rendering
```flux
# Komponent e'loni (no props yet)
cmp Button
  render -> {tag::button text:"Click me"}

# Komponent props bilan
cmp Card title:str content:str
  render ->
    {tag::div class:"card" children:[
      {tag::h2 text:title}
      {tag::p text:content}
    ]}

# Render — komponent ichida o'zi ishlatiladi
Button.render()       # → {tag::button ...}
Card.render "Hi" "body"
```
**Sintaksis eslatma:** `cmp` (component) — yangi kalit so'z, `props:type` — o'zgaruvchi: `str int bool [T] {k:T}`. `render` — maxsus metod (aniki foydalanuvchi yozadi yoki default keladi).

### 2. **`dom.html`/`dom.text`/`dom.fragment`** — primitiv element'lar
```flux
# E'lon qilmasdan to'g'ridan-to'g'ri render
http.on :get "/" \req ->
  rep 200 (dom.html "div" {class:"container"} [
    dom.html "h1" {} [dom.text "Gul do'koni"]
  ])

# Yoki shorthand (tag bilan lambda)
div {class:"main"} [
  h1 {} "Sarlavha"
]
```

### 3. **Reaktiv state** — `<-` va `watch`
```flux
# State e'loni (komponenta ichida)
count <- 0
msg <- ""

# Update (istalgan joyda)
count <- count + 1

# State'ni kuzatish va auto-render
watch count \new_val ->
  log "count changed: ${new_val}"
```
**Semantika:** `<-` o'zgartirishdan keyin rendering **avtomatik** quriladi (React-ga o'xshash virtual-dom reconciliation). Server-side: SSR (server), client-side: hydration + signal'lar.

### 4. **Event binding** — `on` atributi
```flux
cmp Counter
  count <- 0
  render ->
    {tag::div children:[
      {tag::button on:{click:\-> count <- count + 1} text:"+"}
      {tag::span text:"${count}"}
    ]}
```
**Sintaksis:** `on:{event:\handler}` — map ichida symbol kalit va lambda qiymat.

### 5. **List rendering** — `each` (existing loop, UI context'da ishlaydi)
```flux
cmp ProductList products:[{id:int name:str price:flt}]
  render ->
    {tag::div children:(products.map \p ->
      {tag::div class:"product" children:[
        {tag::h3 text:p.name}
        {tag::p text:"${p.price}₽"}
      ]}
    )}

# Yoki to'g'ri:
each product in products
  {tag::div text:product.name}
```

### 6. **Conditional rendering** — `if/elif/else` (existing)
```flux
cmp UserGreeting user:{name:str role:str} nil
  render ->
    if user
      {tag::div text:"Salom, ${user.name}"}
    elif
      {tag::div text:"Kirishingiz kerak"}
```

### 7. **Stil va tema** — `style` atributi + `theme` battery
```flux
# Inline style
{tag::button style:{color:"blue" padding:"10px"}}

# CSS class (default theme'dan)
{tag::div class:"card primary-bg"}

# Theme switching
theme.use :light   # yoki :dark, :custom

# Custom theme
theme.set {
  primary:"#3b82f6"
  secondary:"#10b981"
  bg:"#ffffff"
  text:"#1f2937"
}
```

### 8. **Default komponentlar** — `ui` library
```flux
use ui   # shadcn/ui ga o'xshash tayyor komponentlar

# Button, Card, Modal, Form, Input, Table, etc.
{tag::Button variant:"primary" size:"lg" text:"Savatga qo'sh"}
{tag::Input type:"email" placeholder:"email@example.com"}
{tag::Modal open:true on_close:\-> modal_open <- false} [
  {tag::Card title:"Tasdiqlang"}
]
```
**Runtime tajribasi:** Flux to'g'ridan-to'g'ri React-komponentga transpile qilinadi (default style bilan).

### 9. **Form handling** — `form` battery
```flux
cmp NewProduct
  name <- ""
  price <- 0.0
  render ->
    {tag::form on:{submit:\->
      if (name.len > 0) & (price > 0)
        db.ins "products" {name:name price:(price*100)}
        name <- ""
        price <- 0.0
      else
        fail 400 "Name va narx kerak"
    } children:[
      {tag::Input type:"text" value:name on:{change:\e -> name <- e.target.value}}
      {tag::Input type:"number" value:"${price}" on:{change:\e -> price <- (str.int e.target.value) / 100}}
      {tag::Button type:"submit" text:"Qo'shish"}
    ]}
```

### 10. **Routing** — `route` battery
```flux
use route

route.on "/" \-> rep 200 (Home.render())
route.on "/products" \-> rep 200 (ProductList.render())
route.on "/products/:id" \-> rep 200 (ProductDetail.render req.params.id)

# Client-side navigation
{tag::a href:"/products" on:{click:\e ->
  e.prevent_default()
  route.push "/products"
}}
```

### 11. **Meta va head** — `head` battery
```flux
use head

head.title "Gul do'koni"
head.meta "description" "Yangi gullar internet-do'konida"
head.style "body { font-family: sans-serif; }"
head.script "https://cdn.example.com/lib.js"
```

### 12. **Data binding** — `bind` keyword (opsional syntax sugar)
```flux
# Qisqa variant (syntactic sugar)
cmp LoginForm
  email <- ""
  password <- ""
  render ->
    {tag::form children:[
      {tag::Input bind:email type:"email"}    # o'tini: value:email + on:{change:...}
      {tag::Input bind:password type:"password"}
      {tag::Button type:"submit" text:"Kirish"}
    ]}
```

### 13. **Komponent lifecycle** — `on_mount`, `on_unmount`
```flux
cmp Dashboard
  data <- nil
  on_mount \->
    data <- (db.q "select * from stats")
  render ->
    if data
      render_stats data
    else
      {tag::div text:"Yuklanyapti..."}
```

---

## Default → Config → Override modeli

Bitta komponent, uch darajada:

### DEFAULT (AI hech narsa yozmaydi)
```flux
use ui

cmp Button
  render -> {tag::Button variant:"primary" size:"md" text:"Click"}

# O'z-o'zidan:
# - styling (shadcn/ui orqali)
# - hover/active state
# - accessibility (aria)
# - responsive
```

### CONFIG (tema + kichik o'zgarish)
```flux
use ui

theme.set {
  primary:"#8b5cf6"      # purple
  radius:"8px"
}

cmp Button variant:str size:str text:str
  render -> {tag::Button variant:variant size:size text:text}

# Yoki global config:
ui.config {
  button_variant::secondary
  card_radius::12
}
```

### OVERRIDE (butunlay qayta yozish)
```flux
cmp CustomButton text:str color:str on_click:\
  render ->
    {tag::button
      style:{
        background:color
        padding:"12px 20px"
        border:"none"
        border_radius:"6px"
        cursor:"pointer"
        font_weight:"bold"
      }
      on:{click:on_click}
      text:text
    }

# Yoki React-qism kiritish (advanced):
cmp AdvancedChart data:[{x:int y:int}]
  render -> {tag::ReactComponent name:"Recharts" props:{data:data}}
```

---

## To'liq gul do'koni dashboard (frontend + backend bir faylda)

```flux
use http db json time ai
use ui route head theme

# ========== SCHEMA ==========
tbl flowers
  id      serial pk
  name    str uniq
  desc    str
  price   int                # cents
  stock   int
  image   str
  category str
  created now

tbl orders
  id      serial pk
  cust_id int ref:customers.id
  status  sym                # :new :processing :sent :delivered
  total   int
  items   json               # [{flower_id qty price_each}]
  created now

tbl customers
  id      serial pk
  name    str
  email   str uniq
  phone   str
  addr    str
  created now

tbl analytics
  id      serial pk
  date    str                # "2024-01-15"
  views   int
  orders  int
  revenue int
  top     str                # top flower name
  created now

# ========== THEME ==========
theme.set {
  primary:"#ec4899"         # pink
  secondary:"#8b5cf6"       # purple
  success:"#10b981"         # green
  danger:"#ef4444"          # red
  bg:"#ffffff"
  text:"#1f2937"
  border:"#e5e7eb"
  radius:"8px"
}

# ========== SHARED STATE ==========
user_id <- nil              # logged-in user
cart_items <- []            # [{flower_id qty}]
modal_open <- false
modal_content <- nil

# ========== COMPONENTS ==========

# Header with nav
cmp Header
  render ->
    {tag::div class:"header" style:{bg:theme.primary color:"white"} children:[
      {tag::div class:"container" children:[
        {tag::h1 text:"Gul do'koni 🌸"}
        {tag::nav children:[
          {tag::a href:"/" text:"Bosh sahifa"}
          {tag::a href:"/products" text:"Mahsulotlar"}
          {tag::a href:"/orders" text:"Buyurtmalarim"}
          {tag::a href:"/settings" text:"Sozlamalar"}
          if user_id
            {tag::a href:"#" on:{click:\-> user_id <- nil} text:"Chiqish"}
          else
            {tag::a href:"/login" text:"Kirish"}
        ]}
      ]}
    ]}

# Product card with add-to-cart
cmp ProductCard flower:{id:int name:str price:int image:str stock:int}
  render ->
    {tag::div class:"product-card" children:[
      {tag::img src:flower.image alt:flower.name style:{height:"200px" object_fit:"cover"}}
      {tag::h3 text:flower.name}
      {tag::p text:"${flower.price / 100}₽"}
      if flower.stock > 0
        {tag::Button
          variant:"primary"
          size:"sm"
          text:"Savatga qo'sh"
          on:{click:\->
            cart_items <- cart_items.push {flower_id:flower.id qty:1}
            modal_content <- {msg:"Savatga qo'shildi", ok:true}
            modal_open <- true
          }
        }
      else
        {tag::div text:"Sotilgan" style:{color:theme.danger}}
    ]}

# Order status badge
cmp StatusBadge status:sym
  color = match status
    :new -> theme.secondary
    :processing -> theme.primary
    :sent -> theme.primary
    :delivered -> theme.success
    _ -> theme.text
  text_val = match status
    :new -> "Yangi"
    :processing -> "Jarayonda"
    :sent -> "Yuborildi"
    :delivered -> "Yetkazildi"
    _ -> "Noma'lum"
  render ->
    {tag::span style:{background:color color:"white" padding:"4px 8px" border_radius:"4px"} text:text_val}

# Modal component
cmp Modal open:bool on_close:\ title:str content:str
  render ->
    if open
      {tag::div class:"modal-overlay" on:{click:on_close} children:[
        {tag::div class:"modal-content" style:{bg:theme.bg padding:"20px" border_radius:theme.radius} children:[
          {tag::h2 text:title}
          {tag::p text:content}
          {tag::Button text:"Yopish" on:{click:on_close}}
        ]}
      ]}
    else
      nil

# ========== PAGES ==========

# Home page — statistics + dashboard
cmp HomePage
  stats <- nil
  chart_data <- nil
  
  on_mount \->
    # Fetch today's stats
    today = time.fmt (time.now) "2006-01-02"
    s = db.one "select * from analytics where date=$1" [today]
    stats <- s ?? {views:0 orders:0 revenue:0 top:"—"}
    
    # Fetch last 30 days for chart
    past = time.ago 30 :day
    past_date = time.fmt past "2006-01-02"
    data = db.q "select date views orders revenue from analytics where created > $1 order by date" [past]
    chart_data <- data
  
  render ->
    {tag::div class:"page" children:[
      Header.render()
      {tag::div class:"container" children:[
        {tag::h2 text:"Bosh sahifa"}
        {tag::div class:"stats-grid" children:[
          {tag::Card title:"Ko'rishlar" content:"${stats.views}"}
          {tag::Card title:"Buyurtmalar" content:"${stats.orders}"}
          {tag::Card title:"Daromad" content:"${stats.revenue / 100}₽"}
          {tag::Card title:"Top mahsulot" content:stats.top}
        ]}
        if chart_data
          {tag::Card title:"Tarixiy tendensiya" children:[
            {tag::SimpleChart data:chart_data x_key:"date" y_key:"orders"}
          ]}
        else
          {tag::div text:"Yuklanyapti..."}
      ]}
    ]}

# Products page — searchable + filterable list
cmp ProductsPage
  flowers <- []
  search <- ""
  category <- ""
  
  on_mount \->
    flowers <- db.q "select * from flowers where stock > 0"
  
  filtered = flowers.filter \f ->
    (search.len == 0 | (f.name.low.has (search.low))) &
    (category.len == 0 | f.category == category)
  
  render ->
    {tag::div class:"page" children:[
      Header.render()
      {tag::div class:"container" children:[
        {tag::h2 text:"Mahsulotlar"}
        {tag::div class:"filters" children:[
          {tag::Input bind:search placeholder:"Qidirish..."}
          {tag::Select value:category on:{change:\e -> category <- e.target.value} children:[
            {tag::option value:"" text:"Barcha toifalar"}
            {tag::option value:"roses" text:"Qizil gullar"}
            {tag::option value:"tulips" text:"Tulipanlar"}
          ]}
        ]}
        {tag::div class:"product-grid" children:(filtered.map \f -> ProductCard.render f)}
      ]}
    ]}

# Orders page — customer orders history
cmp OrdersPage
  orders <- []
  
  on_mount \->
    if user_id
      orders <- db.q "select * from orders where cust_id=$1 order by created desc" [user_id]
  
  render ->
    {tag::div class:"page" children:[
      Header.render()
      {tag::div class:"container" children:[
        {tag::h2 text:"Mening buyurtmalarim"}
        {tag::Table columns:[{key:"id" label:"ID"} {key:"status" label:"Holati"} {key:"total" label:"Summa"}] data:(orders.map \o -> {
          id:"#${o.id}"
          status:(StatusBadge.render o.status)
          total:"${o.total / 100}₽"
        })}
      ]}
    ]}

# Login page
cmp LoginPage
  email <- ""
  password <- ""
  error <- nil
  
  on_submit = \->
    if (email.len > 0) & (password.len > 5)
      # Simple auth (production: bcrypt, JWT)
      cust = db.one "select id from customers where email=$1" [email]
      if cust
        user_id <- cust.id
        route.push "/"
      else
        error <- "Foydalanuvchi topilmadi"
    else
      error <- "Noto'g'ri kiritish"
  
  render ->
    {tag::div class:"page login-page" children:[
      {tag::Card title:"Kirish" children:[
        if error
          {tag::div style:{color:theme.danger} text:error}
        else
          nil
        {tag::Input bind:email type:"email" placeholder:"Email"}
        {tag::Input bind:password type:"password" placeholder:"Parol"}
        {tag::Button text:"Kirish" on:{click:on_submit}}
      ]}
    ]}

# ========== ROUTING ==========

route.on "/" \-> rep 200 (HomePage.render())
route.on "/products" \-> rep 200 (ProductsPage.render())
route.on "/orders" \->
  if user_id
    rep 200 (OrdersPage.render())
  else
    rep 302 {location:"/login"}
route.on "/login" \-> rep 200 (LoginPage.render())

# API endpoints (backend)

http.on :get "/api/products" \req ->
  flowers = db.q "select * from flowers"
  rep 200 flowers

http.on :post "/api/orders" \req ->
  if !user_id
    fail 401 "Not authenticated"
  order = db.ins "orders" {cust_id:user_id status::new total:(req.body.total) items:(json.enc req.body.items)}
  rep 201 order

http.on :post "/api/flowers" \req ->
  # Admin only
  flower = db.ins "flowers" {name:req.body.name desc:req.body.desc price:req.body.price category:req.body.category}
  rep 201 flower

# ========== CRON: Daily analytics ==========

cron.on 0 0 * * * \->
  today = time.fmt (time.now) "2006-01-02"
  view_count = (db.one "select count(*) c from events where date=$1 and type='view'" [today]).c ?? 0
  order_count = (db.one "select count(*) c from orders where date(created)=$1" [today]).c ?? 0
  revenue = (db.one "select sum(total) r from orders where date(created)=$1" [today]).r ?? 0
  top_flower = (db.one "select name from flowers order by sales desc limit 1").name ?? "—"
  
  db.ins "analytics" {date:today views:view_count orders:order_count revenue:revenue top:top_flower}
  log "Analytics updated for ${today}"

# ========== SERVER ==========

head.title "Gul do'koni — Yangi gullar"
head.meta "description" "Taza, rang-barang gullar internetda"
head.style """
.container { max-width: 1200px; margin: 0 auto; padding: 20px; }
.header { padding: 20px; }
.stats-grid { display: grid; grid-template-columns: 1fr 1fr 1fr 1fr; gap: 16px; }
.product-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 16px; }
.product-card { border: 1px solid #e5e7eb; padding: 10px; border-radius: 8px; }
.modal-overlay { position: fixed; inset: 0; background: rgba(0,0,0,0.5); display: flex; align-items: center; justify-content: center; }
.modal-content { background: white; padding: 30px; border-radius: 8px; max-width: 500px; }
.filters { margin-bottom: 20px; display: flex; gap: 10px; }
"""

http.serve 8080
```

---

## Token tahlili

Shu dashboard'i oddiy React + TypeScript + Tailwind'da yozish:

**React variyanti** (~2500 token):
```typescript
// Bitta komponent uchun: state, useEffect, JSX, CSS, form logic, API calls
// 10+ komponentlar → har biri 100-200 token → 1500-2000 token
// Utils, types, hooks → 300-500 token
// Styling (separate CSS) → 300-500 token
// Routing (React Router) → 200 token
// TOTAL: ~2500-3500 token
```

**Flux variyanti** (yuqori): **~950 token**

**Farqi: 2.6x qisqa!**

Sabablari:
1. **Default komponentlar** — `{tag::Button}` yozmagin, albatta `theme.primary` (CSS Flux bilan yotadi)
2. **Reaktiv state** — `count <- 0` + auto-render, `useState + useEffect` kerak emas
3. **Database integratsiyasi** — `db.q`, `db.ins` to'g'ridan-to'g'ri, API yozish kerak emas
4. **Routing** — `route.on` (bir qat), React Router yo'q
5. **Qavs/komma yok** — Flux sintaksisi ixcham
6. **Backend+Frontend bir fayl** — glue kod yo'q

---

## Flux'ga qo'shilishi kerak bo'lgan runtime imkoniyatlari

### 1. **Transpile qo'ng'iroq**
```
flux build app.fx              # → dist/index.html + dist/bundle.js
flux run app.fx --dev          # dev server + hot reload, localhost:8080
flux run app.fx --ssr          # server-side render (istalgan HTTP server)
```

### 2. **Runtime ta'minoti (Node.js + Bun.js bilan)**
- **Transpiler** (`flux_transpile` subcommand): Flux AST → TypeScript + React (via babel/esbuild)
- **Virtual DOM reconciliation**: `<-` o'zgarishi → React `useState`'ga map, `watch` → `useEffect`'ga
- **Event delegation**: `on:{click:\->...}` → React `onClick` handler
- **Hydration**: SSR'dan kelgan HTML + client-side state ulanishi

### 3. **Default komponentlar library**
```
flux/ui/components/:
  Button.tsx
  Card.tsx
  Input.tsx
  Modal.tsx
  Table.tsx
  Select.tsx
  Form.tsx
  ...
```
Runtime o'z ichiga oladi, Flux'da `use ui` → bundled.

### 4. **Theme system**
- CSS variable'lar (`:root { --color-primary: #... }`)
- Runtime'da `theme.set {...}` → CSS generate (tailwind'dan o'xshash)

### 5. **Routing engine**
- Client + server routing bir kalit so'z (`route.on`)
- SPA mode (client-side nav) + SSR mode (server render)

### 6. **Lifecycle hooks**
- `on_mount` → `useEffect(() => ..., [])` 
- `on_unmount` → cleanup function
- `watch` → `useEffect(() => ..., [dep])`

### 7. **Form auto-binding**
- `bind:state_var` → `value={stateVar}` + `onChange={e => setStateVar(...)}`
- Validation hooks (opsional)

### 8. **Asset handling**
- `{tag::img src:"path"}` — `public/` papkasidan yoki URL
- Static files (`/public`) automatic serve

### 9. **Meta tags (SSR uchun)**
- `head.title`, `head.meta`, `head.style`, `head.script` → `<head>` generate

### 10. **Bundling & deployment**
- Statik export: HTML + JS → CDN/GitHub Pages
- Node.js server: `flux run app.fx` → standalone executable
- Docker: Flux built-in Dockerfile template

---

## Arxitektura — Flux Frontend qatlamining ichki tutilishi

### Transpiler pipeline:
```
app.fx (Flux source)
  ↓
Parse & analyze (AST'dan `cmp` e'lon + `<-` state'ni aniqlash)
  ↓
JavaScript/TypeScript AST generate (React functional component + hooks)
  ↓
Esbuild bundle
  ↓
dist/index.html (Bun.js + Tailwind inject) + dist/bundle.js
```

### Runtime state management:
```
Flux `<-` (mutable binding)
  ↓
React `useState` (internal)
  ↓
Virtual DOM reconciliation (React)
  ↓
DOM update
```

### Example transpilation:
```flux
cmp Counter
  count <- 0
  render ->
    {tag::div children:[
      {tag::button on:{click:\-> count <- count + 1} text:"+"}
      {tag::span text:"${count}"}
    ]}
```

↓ Transpiles to:

```typescript
function Counter() {
  const [count, setCount] = useState(0);
  return (
    <div>
      <button onClick={() => setCount(count + 1)}>+</button>
      <span>{count}</span>
    </div>
  );
}
```

---

## Flux Frontend spec additions (summary table)

| Konstruksiya | Sintaksis | Maqsad | Misol |
|---|---|---|---|
| Component | `cmp Name [props]` | UI element e'loni | `cmp Button text:str` |
| Render | `render -> {...}` | komponent output | `render -> {tag::button}` |
| Reaktiv state | `var <- value` | o'zgaruvchan UI holati | `count <- 0` |
| State watch | `watch var \v -> ...` | state o'zgarishi | `watch count log` |
| Event | `on:{event:\handler}` | user action | `on:{click:\-> count <- count + 1}` |
| Element | `{tag::name attr:val}` | DOM element | `{tag::button text:"Click"}` |
| List map | `list.map \x -> ...` | rendering arrays | `items.map \i -> {tag::li text:i}` |
| Style | `style:{k:v}` | inline CSS | `style:{color:"red"}` |
| Class | `class:"name"` | CSS class | `class:"btn primary"` |
| Lifecycle | `on_mount \-> ...` | initialization | `on_mount \-> data <- db.q "..."` |
| Binding | `bind:state` | two-way binding | `{tag::Input bind:email}` |
| Route | `route.on path handler` | navigation | `route.on "/about" \-> ...` |
| Theme | `theme.set {...}` | colors/typography | `theme.primary` |
| Head | `head.title str` | meta tags | `head.title "App"` |

---

## Misollar — qo'shimcha ishlari

### Login + protected route
```flux
authenticated <- false
token <- nil

cmp ProtectedPage
  render ->
    if authenticated
      {tag::div text:"Yashiringan ma'lumot"}
    else
      {tag::div text:"Ruxsat yo'q, kiring"}

route.on "/protected" \->
  if authenticated
    rep 200 (ProtectedPage.render())
  else
    rep 302 {location:"/login"}
```

### Form with validation
```flux
cmp SignupForm
  email <- ""
  password <- ""
  errors <- {}
  
  validate = \->
    errs = {}
    if email.len < 5
      errs.set "email" "Email qisqa"
    if password.len < 8
      errs.set "password" "Parol kamida 8 ta belgi"
    errors <- errs
    errs.keys.len == 0
  
  render ->
    {tag::form on:{submit:\->
      if (validate())
        db.ins "users" {email:email password:password}
        route.push "/"
    } children:[
      {tag::Input bind:email placeholder:"email@example.com"}
      if errors.has "email"
        {tag::span style:{color:"red"} text:errors.email}
      {tag::Input bind:password type:"password" placeholder:"Parol"}
      {tag::Button type:"submit" text:"Ro'yxatdan o't"}
    ]}
```

### Real-time updates (WebSocket)
```flux
use ws

cmp LiveOrders
  orders <- []
  
  on_mount \->
    ws.on :message \msg ->
      order = json.dec msg
      orders <- orders.push order
  
  render ->
    {tag::div children:(orders.map \o ->
      {tag::div text:"Order #${o.id}: ${o.status}"}
    )}
```

---

## FAQ

**S:** `dom` + `http` birga ishlaydi mi?
**J:** Ha. `route.on "/" \-> rep 200 (Page.render())` — backend + frontend bir kalit so'z, bir handler.

**S:** CSS framework qaysi?
**J:** Default: Tailwind (built-in `theme.set`). SCSS/Stylus → future. Inline `style:` ham OK.

**S:** TypeScript yoki JavaScript?
**J:** Transpile'r TypeScript chiqaradi (type-safe), lekin `use js` bilan asl JS'ga ham.

**S:** Server-side rendering haminchi?
**J:** Flux `render` → React functional component → `ReactDOMServer.renderToString()` → HTML string.

**S:** Deployment?
**J:** `flux build` → static files (CDN) yoki `flux run` → standalone server.

