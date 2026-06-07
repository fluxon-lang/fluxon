This is a pure language-design task — no provider/LLM integration to build, no code to run. I'll produce the design document directly.

# Flux Frontend — dizayn

## Asosiy g'oya

Flux backendning falsafasi "bir ish = bir yo'l, kam token, aytilmagan = oqilona default". Frontend qatlami ham aynan shu prinsip ustiga quriladi: UI — bu **server-driven, reaktiv komponent daraxti**. Mijoz brauzerda alohida JS framework yozmaydi; Flux fayli ham backendni, ham UI'ni bitta `.fx` faylida e'lon qiladi. Yangi belgilar ixtiro qilinmaydi — `<-` reaktiv state'ni anglatadi (mutable bind = o'zgarganda UI qayta render), `each` ro'yxat renderi, `if/elif/else` shartli render, space-args esa komponent chaqirig'i bo'ladi. Ya'ni komponent oddiy `fn`, prop esa oddiy argument.

Eng muhim qism — **moslashuvchanlik uch darajada**. Flux'da `ui` batareyasi shadcn'ga o'xshash tayyor, dizayni mukammal bloklarni (`ui.table`, `ui.form`, `ui.stat`, `ui.shell`) olib keladi. Default holatda mijoz bir-ikki so'z bilan to'liq sahifa oladi (default-by-omission). Xohlasa `theme` orqali rang/shriftni sozlaydi (config). Xohlasa o'sha komponentni o'z `fn`'i bilan to'liq qayta yozadi (override) — chunki tayyor blok ham, mijoz bloki ham bir xil `fn name props -> el` shaklida bo'lgani uchun ular o'rin almasha oladi. Backend bilan organik bog'lanish: `db.q`/`source` to'g'ridan UI state'ga ulanadi, glue (REST fetch, JSON parse, loading-state) avtomatik — chunki runtime data-binding'ni o'zi boshqaradi.

## Yangi primitivlar (qo'shimchalar)

### `view` — komponent e'loni
`fn`ning UI varianti: argument = prop, qaytargani = element daraxti. Indentatsiya = bola elementlar (xuddi `tbl` kabi).
```flux
view greeting name
  h1 "Salom $name"
  p "xush kelibsiz"
```
Chaqirish — space-args, backend `fn` chaqirig'i bilan bir xil: `greeting "Ali"`.

### Element yozuvi — `tag content props`
HTML emas, **canonical element**: `tag` (yoki `ui.*` blok), so'ng matn/bola, so'ng `{props}` map. Bittagina yo'l.
```flux
btn "Saqlash" {on:save kind::primary}
div {pad:4}
  p "matn"
```
Hech qachon yopuvchi teg yo'q — blok indentatsiya bilan tugaydi.

### Reaktiv state — `<-` (yangi belgi YO'Q)
Backenddagi mutable bind aynan UI signali bo'ladi. O'zgarsa — faqat bog'liq joy qayta render qilinadi (fine-grained signals).
```flux
count <- 0
btn "+1" {on:\-> count <- count + 1}
p "Hozir: $count"          # count o'zgarsa shu p yangilanadi
```
Hosila qiymat — oddiy `=` ifoda (computed): `doubled = count * 2`.

### Data-binding — `source` UI'ga ulanadi
`source` = reaktiv ma'lumot manbai (backend `db.q`/`http`/`ai` ustida). Avtomatik loading/error/refetch. Glue kod yo'q.
```flux
products <- source db.q "select * from products order by ts desc"
# products.loading · products.err · products.data · products.reload()
```
Mutatsiyadan keyin avtomatik invalidate: `products.reload()` yoki `source`larni tag bilan bog'lab `ui.invalidate :products`.

### Ro'yxat / shartli render — `each` / `if` qayta ishlatiladi
```flux
each p in products.data
  card p.name
if products.loading
  ui.spinner
elif products.err
  ui.error products.err
```

### Event — prop ichida lambda yoki `fn` qiymati
Yangi sintaksis yo'q: `{on:save}` (fn qiymati) yoki `{on:\-> ...}` (lambda). `on` = asosiy hodisa (btn→click, form→submit, input→change).
```flux
input {bind:name}                    # ikki tomonlama: yozilsa name <- ...
form {on:save}                       # submit → save chaqiriladi
```
`bind:` = two-way binding (state nomi), `on:` = hodisa.

### Stil / tema — `theme` bloki (config darajasi)
Inline stil emas — token'lar (kam token, canonical). `{pad:4 gap:2 kind::primary}` kabi semantik proplar; aniq ranglar `theme`da.
```flux
theme
  primary  "#e84d8a"
  radius   :lg
  font     "Inter"
  mode     :light          # :light :dark :auto
```

### Default komponentlar — `ui.*` batareyasi (shadcn bloklari)
`use ui` bilan keladi, install yo'q. Har biri to'liq, dizayni tayyor, theme'ga bo'ysunadi:
`ui.shell` (sidebar+header layout), `ui.table`, `ui.form`, `ui.stat`, `ui.chart`, `ui.modal`, `ui.search`, `ui.badge`, `ui.btn`, `ui.crud` (CRUD'ning hammasi bitta blokda).

### Override — bir xil shakl, almashtirish
Tayyor blok ham, mijoz komponenti ham `view name props`. O'z nomli `view` e'lon qilsang, runtime uni ishlatadi; `ui.*`ni to'g'ridan-to'g'ri chaqirsang — default. Qisman override uchun `slot`:
```flux
ui.table products.data {cols:[:name :price]
  cell::price \row -> badge "${row.price/100}$" {kind::ok}}   # bitta ustunni override
```

### Routing — `page` (URL = sahifa)
`http.on`ning UI varianti, file-system emas, deklarativ:
```flux
page "/" -> dashboard
page "/products" -> products_page
page "/orders/:id" \params -> order_page params.id
```
`nav "/products" "Mahsulotlar"` = link (SPA, reload yo'q).

## Default → Config → Override modeli

Bitta jadval komponenti uch darajada:

```flux
# (a) DEFAULT — bir so'z, to'liq jadval (ustunlar avtomatik aniqlanadi)
ui.table products.data

# (b) CONFIG — ustun/saralash/qidiruv sozlamalari (token oz)
ui.table products.data {cols:[:name :price :stock] search::name sort::price}

# (c) OVERRIDE — o'z komponenting; ui.* primitivlaridan foydalan, lekin to'liq nazorat
view my_table rows
  div {kind::panel}
    each r in rows
      div {flex:true gap:3 pad:2 hover::row}
        img r.photo {w:8 round:true}
        b r.name
        span "${r.price/100}$" {ml::auto}
        if r.stock == 0
          badge "Tugadi" {kind::danger}
# endi default o'rniga buni ishlat:
my_table products.data
```

Uchala holatda chaqiruv shakli bir xil (`X products.data`), shuning uchun default'dan override'ga o'tish faqat nomni almashtirish — qolgan kod o'zgarmaydi.

## To'liq gul do'koni dashboard (frontend + backend bir faylda)

```flux
use http db ui

# ---------- BACKEND: schema ----------
tbl products
  id    serial pk
  name  str
  price money
  stock int
  photo str null
  cat   sym
  ts    now

tbl orders
  id     serial pk
  cust   str
  total  money
  status sym                 # :new :packed :shipped :done
  ts     now

tbl customers
  id    serial pk
  name  str
  phone str
  spent money
  ts    now

# ---------- BACKEND: API (UI shulardan oziqlanadi) ----------
http.on :get  "/api/products" \req -> rep 200 (db.q "select * from products order by ts desc")
http.on :post "/api/products" \req -> rep 201 (db.ins "products" req.body)
http.on :put  "/api/products/:id" \req -> rep 200 (db.up "products" req.body {id:req.params.id})
http.on :del  "/api/products/:id" \req -> rep 200 (db.del "products" {id:req.params.id})

# ---------- THEME (config darajasi) ----------
theme
  primary "#e84d8a"
  accent  "#9b5de5"
  radius  :lg
  font    "Inter"
  mode    :light

# ---------- LAYOUT: sidebar + header (default shell) ----------
view app
  ui.shell {brand:"Gulzor 🌸" nav:menu}
    page "/"             -> dashboard
    page "/products"     -> products_page
    page "/orders"       -> orders_page
    page "/customers"    -> customers_page
    page "/settings"     -> settings_page

menu = [
  {to:"/"          icon::home    label:"Bosh sahifa"}
  {to:"/products"  icon::box     label:"Mahsulotlar"}
  {to:"/orders"    icon::cart    label:"Buyurtmalar"}
  {to:"/customers" icon::users   label:"Mijozlar"}
  {to:"/settings"  icon::gear    label:"Sozlamalar"}
]

# ---------- SAHIFA 1: DASHBOARD (statistika + grafik) ----------
view dashboard
  stats <- source db.one "select
      (select count(*) from orders where status=$1) new_cnt,
      (select coalesce(sum(total),0) from orders) revenue,
      (select count(*) from products where stock=0) out_stock,
      (select count(*) from customers) custs" [:new]

  h1 "Bosh sahifa"
  div {grid:4 gap:4}                          # 4 ustunli statistika kartalari
    ui.stat "Yangi buyurtma" stats.data.new_cnt   {icon::cart  trend::up}
    ui.stat "Daromad"        "${stats.data.revenue/100}$" {icon::cash kind::primary}
    ui.stat "Tugagan mahsulot" stats.data.out_stock {icon::warn kind::danger}
    ui.stat "Mijozlar"       stats.data.custs    {icon::users}

  div {grid:2 gap:4}
    ui.chart (source db.q "select date(ts) d, sum(total)/100 v from orders group by 1 order by 1") {kind::line title:"Sotuv dinamikasi"}
    ui.chart (source db.q "select cat, count(*) c from products group by cat") {kind::donut title:"Kategoriyalar"}

# ---------- SAHIFA 2: MAHSULOTLAR (CRUD + qidiruv + forma) ----------
view products_page
  items <- source db.q "select * from products order by ts desc"
  q     <- ""
  open  <- false
  edit  <- nil

  shown = items.data.filter \p -> str.has (str.low p.name) (str.low q)

  div {flex:true gap:3 mb:4}
    h1 "Mahsulotlar"
    ui.search {bind:q placeholder:"Qidirish..."}
    btn "+ Yangi" {on:\-> (edit <- nil) (open <- true) kind::primary ml::auto}

  ui.table shown {cols:[:photo :name :cat :price :stock]
    fmt::price \v -> "${v/100}$"
    cell::photo \r -> img r.photo {w:10 round:true}
    cell::stock \r -> badge r.stock {kind:(if r.stock == 0 :danger else :ok)}
    actions:[
      {icon::edit on:\r -> (edit <- r) (open <- true)}
      {icon::trash on:\r -> del_product r.id  confirm:"O'chirilsinmi?"}
    ]}

  # forma modal ichida — yangi yoki tahrir
  ui.modal {open:open title:(if edit "Tahrirlash" else "Yangi mahsulot")}
    ui.form edit {on:save_product
      fields:[
        {name::name  label:"Nomi"      kind::text req:true}
        {name::price label:"Narx (so'm)" kind::money}
        {name::stock label:"Zaxira"    kind::int}
        {name::cat   label:"Kategoriya" kind::select opts:[:atirgul :lola :buket :chinnigul]}
        {name::photo label:"Rasm"      kind::file}
      ]}

fn save_product data
  if data.id
    http.put "/api/products/${data.id}" data
  else
    http.post "/api/products" data
  ui.invalidate :items           # source'ni qayta yuklaydi → jadval yangilanadi
  ui.close

fn del_product id
  http.del "/api/products/$id"
  ui.invalidate :items

# ---------- SAHIFA 3: BUYURTMALAR (jadval + holat + modal) ----------
view orders_page
  orders <- source db.q "select * from orders order by ts desc"
  sel    <- nil

  h1 "Buyurtmalar"
  ui.table orders.data {cols:[:id :cust :total :status :ts]
    fmt::total \v -> "${v/100}$"
    fmt::ts \v -> time.fmt v "dd.MM HH:mm"
    cell::status \r -> badge r.status {kind:(match r.status
      :new -> :info
      :packed -> :warn
      :shipped -> :primary
      :done -> :ok
      _ -> :muted)}
    on::row \r -> sel <- r}

  ui.modal {open:(sel != nil) title:"Buyurtma #${sel.id}" on_close:\-> sel <- nil}
    if sel
      p "Mijoz: ${sel.cust}"
      p "Summa: ${sel.total/100}$"
      div {flex:true gap:2 mt:3}
        each st in [:new :packed :shipped :done]
          btn (str.str st) {kind:(if sel.status == st :primary else :ghost)
            on:\-> set_status sel.id st}

fn set_status id st
  http.put "/api/orders/$id" {status:st}
  ui.invalidate :orders

# ---------- SAHIFA 4: MIJOZLAR ----------
view customers_page
  custs <- source db.q "select * from customers order by spent desc"
  h1 "Mijozlar"
  ui.table custs.data {cols:[:name :phone :spent] fmt::spent \v -> "${v/100}$" search::name}

# ---------- SAHIFA 5: SOZLAMALAR ----------
view settings_page
  cfg <- source db.one "select * from settings where id=1"
  h1 "Sozlamalar"
  ui.form cfg.data {on:\d -> http.put "/api/settings" d
    fields:[
      {name::shop   label:"Do'kon nomi" kind::text}
      {name::theme  label:"Mavzu" kind::select opts:[:light :dark :auto]}
      {name::notify label:"Bildirishnoma" kind::switch}
    ]}

# ---------- ISHGA TUSHIRISH (backend + frontend bitta nuqtadan) ----------
ui.serve app 3000          # http API + UI bir portda, bitta event-loop
```

## Token tahlili

Misol: mahsulotlar CRUD + qidiruv + modal forma jadvali.

- **React + shadcn/ui + react-query + react-hook-form**: `useState`/`useQuery`/`useMutation`, `<Dialog>`, `<Table>` ustun definitsiyalari (`columnHelper`), `fetch` wrapper, JSON parse, loading/error JSX, `onSubmit` handler, invalidatsiya — taxminan **180-250 qator**, har bir `import`, generic tip, `className` string token sarflaydi. Faqat jadval ustun ta'rifi 30+ qator.
- **Flux** `products_page` (yuqorida): **~30 qator**. `ui.table shown {...}` — bitta ifoda formatlash, cell-override va action'lar bilan. Loading/error/fetch/parse butunlay yashirin (`source`).

Taxminan **6-8 barobar** kam. Sabablari, har biri token kamaytiradi:
1. **Default-by-omission** — `ui.table products.data` ustunlarni schema'dan oladi; aytmaganing default.
2. **Glue yo'q** — `source` fetch+cache+loading+invalidate'ni yutadi; React'da bu eng katta boilerplate.
3. **Bir til, bir fayl** — backend tip va frontend tip bir xil; serializatsiya/DTO/API-client qatlami umuman yo'q.
4. **`<-` qayta ishlatilgan** — yangi state API o'rganish/yozish yo'q; mutable bind = signal.
5. **Semantik proplar** — `{kind::primary pad:4}` o'nlab `className` utility'ni almashtiradi, theme markazlashgan.

Dashboard'ning to'liq 5 sahifasi (yuqorida) ~150 qator; ekvivalent React+router+query+chart loyihasi odatda 1000+ qator va o'nlab fayl.

## Flux'ga qo'shilishi kerak bo'lgan runtime imkoniyatlari

- **Transpile target + thin client**: `.fx`dagi `view`/`page`/element daraxti kompilyatsiya vaqtida JS'ga (yoki WASM client'ga) chiqadi. Mijozda kichik Flux-runtime (~signals + DOM patcher) ishlaydi — alohida framework emas. `ui.serve` HTML qobiq + shu client bundle'ni beradi.
- **Fine-grained signals**: `<-` reaktiv tugun bo'ladi; o'zgarganda faqat bog'liq DOM bo'lagi yangilanadi (Solid.js/Svelte uslubi, virtual-DOM diff'siz). `=` computed (memoized).
- **Server-driven data (`source`)**: runtime `source` ifodasini avtomatik backend endpoint'iga ulaydi (same-file `db.q` bo'lsa to'g'ridan generatsiya qilingan RPC orqali; tashqi bo'lsa `http.get`). Cache, dedupe, `loading/err/data`, `reload()`/`ui.invalidate :tag` — runtime mas'uliyati. Bu eng muhim qism: dasturchi fetch yozmaydi.
- **Component registry (override)**: nomli `view` global registry'ga tushadi (xuddi `reg` kabi). Default `ui.*` chaqiruvi shu registry'dan qidiradi — mijoz o'sha nomda `view` e'lon qilsa, override avtomatik kuchga kiradi.
- **`ui.*` batareya kutubxonasi**: theme-token'lar bilan dizayni tayyor, accessible bloklar (shadcn ekvivalenti) runtime/SDK ichida keladi, install yo'q.
- **SPA router**: `page` deklaratsiyalari client-side router + history API'ga; `:param` backenddagi `http.on` bilan bir xil ajratish qoidasi (literal > param).
- **Bitta event-loop**: `ui.serve` HTTP API, statik client va WebSocket (realtime `source` invalidatsiya/push uchun) ni bitta jarayonda — `http.serve`/`ws.serve` bilan birga ishlaydi.