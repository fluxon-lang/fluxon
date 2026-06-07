Yetarli kontekst yig'dim. Endi to'liq tahlilni beraman.

---

# Flux Frontend — Konvergensiya tahlili va yakuniy spec tavsiyasi

Uchala model (opus/sonnet/haiku) ni Flux'ning haqiqiy idiomalariga (`docs/flux-agent.md` v5: `<-`, `each`, `if/elif/else`, `match`, space-args, indentatsiya, `source` yo'q lekin `db.q`, `fn`/`\->`) solishtirib chiqdim. Quyida real dizayn qarori.

---

## 1. KONVERGENSIYA — modellar nimaga BIR XIL keldi

### 1.1 Komponent e'loni — `fn`-ga o'xshash, props = space-args (3/3 KONVERGENSIYA, lekin nom har xil)

Uchovi ham "komponent = parametrli UI qaytaruvchi blok, chaqirish = element kabi space-args" g'oyasiga keldi. Faqat **kalit so'z** har xil:

```flux
# agent-a (opus) — "view"
view greeting name
  h1 "Salom $name"

# agent-b (sonnet) — "comp" (+ default-qiymatli prop: key:default)
comp badge text color:"blue"
  span .badge style:"color:${color}"
    "${text}"

# agent-c (haiku) — "cmp" (+ tip annotatsiya: props:type)
cmp Card title:str content:str
  render ->
    {tag::div class:"card" children:[...]}
```

Konvergensiya nuqtasi juda kuchli: **hammasi `fn`ning UI-varianti**, chaqirish space-args (`greeting "Ali"`, `badge "Yangi" color:"green"`). Divergensiya: agent-c `render ->` oraliq qatlam qo'shdi (eng ko'p token, eng kam idiomatik); agent-a/b to'g'ridan-to'g'ri tana = render (idiomatik, `fn` kabi).

### 1.2 Reaktiv state — `<-` qayta ishlatildi (3/3 TO'LIQ KONVERGENSIYA) ⭐

Bu eng muhim natija. **Uchala model ham yangi belgi ixtiro QILMADI** — Flux'ning mavjud `<-` (mutable bind) ni reaktiv state deb oldi:

```flux
# agent-a (opus)
count <- 0
btn "+1" {on:\-> count <- count + 1}
p "Hozir: $count"          # count o'zgarsa shu p yangilanadi

# agent-b (sonnet) — `state` bloki ichiga o'radi
state
  count <- 0
button on:click(\-> count <- count + 1) "+"

# agent-c (haiku)
count <- 0
count <- count + 1          # update — auto-render
```

Hammasi bir xil semantikani aytdi: **`<-` o'zgarsa, bog'liq DOM avtomatik yangilanadi**. Bu hal qiluvchi konvergensiya — yangi state API o'rganish/yozish yo'q, token tejaladi, Flux falsafasiga ("bir ish = bir yo'l") to'liq mos.

### 1.3 Event — prop ichida lambda (3/3 KONVERGENSIYA, sintaksis 2 xil)

```flux
# agent-a (opus) — element prop-map ichida `on:`
btn "Saqlash" {on:save}
btn "+1" {on:\-> count <- count + 1}

# agent-b (sonnet) — atribut sifatida `on:event(lambda)`
button on:click(\-> count <- count + 1) "Bosing"
input on:input(\e -> search <- e.value)

# agent-c (haiku) — map qiymat `on:{event:\lambda}`
{tag::button on:{click:\-> count <- count + 1}}
```

Konvergensiya: hodisa = `on` + lambda. Divergensiya: a hodisa nomini yashiradi (`btn`→click default), b/c hodisa nomini ataydi (`click`/`input`/`submit`). a eng kam token, lekin "qaysi hodisa" noaniq; b/c aniqroq.

### 1.4 `each` ro'yxat render (3/3 KONVERGENSIYA) ⭐

Uchovi ham mavjud `each`ni qayta ishlatdi, yangi `map`/`for` ixtiro qilmadi:

```flux
# agent-a (opus)
each p in products.data
  card p.name

# agent-b (sonnet) — + key:
each item in products key:item.id
  li "${item.name}"

# agent-c (haiku)
each product in products
  {tag::div text:product.name}
```

agent-b'ning `key:item.id` qo'shimchasi diff uchun aqlli (faqat u o'yladi).

### 1.5 `if/elif/else` shartli render (3/3 KONVERGENSIYA)

Hammasi mavjud `if`ni qayta ishlatdi. agent-b qo'shimcha inline shaklni ham kiritdi (`."bg-green" if trend >= 0`) — bu Flux'da YO'Q konstruksiya (postfix if), divergensiya/xavf.

### 1.6 Tema — `theme` bloki (3/3 KONVERGENSIYA)

```flux
# agent-a (opus) — sym/qiymat
theme
  primary  "#e84d8a"
  radius   :lg
  mode     :light

# agent-b (sonnet) — map-stil key:val
theme
  primary: "#6366f1"
  radius: "10px"

# agent-c (haiku) — theme.set {}
theme.set {primary:"#3b82f6" bg:"#ffffff"}
```

Konvergensiya: global dizayn tokenlari bitta `theme` deklaratsiyasida. Divergensiya: a — space-separated (Flux `tbl` idiomasi), b — `key:` ikki nuqta (Flux map idiomasi, lekin blok ichida `:` Flux'da yo'q), c — `theme.set` chaqiruv (imperativ, kam idiomatik).

### 1.7 Default UI kutubxonasi — `ui.*` (3/3 KONVERGENSIYA) ⭐

Uchovi ham shadcn-uslubidagi tayyor bloklarni `use ui` orqali keltirdi:

```flux
# agent-a:  ui.table, ui.form, ui.stat, ui.shell, ui.modal, ui.chart, ui.crud
# agent-b:  ui.table, ui.form, ui.modal, ui.chart, ui.button, ui.input, ui.select
# agent-c:  ui Button, Card, Modal, Table, Form, Input
```

### 1.8 Routing — deklarativ, `http.on` parallel (3/3 KONVERGENSIYA)

```flux
# agent-a (opus) — `page "url" -> view`
page "/products" -> products_page
page "/orders/:id" \params -> order_page params.id

# agent-b (sonnet) — `page :name "url"` + tana
page :products "/products" layout::admin
  load
    ...

# agent-c (haiku) — route.on
route.on "/" "/products" "/orders"
```

Konvergensiya: URL = sahifa, file-system emas, `:param` backenddagi `http.on` bilan bir xil. agent-a/b `page` kalit so'ziga keldi (kuchli konvergensiya).

### 1.9 Data ulanishi — bu yerda DIVERGENSIYA (pastga qarang, 1.10 emas)

- **agent-a**: `products <- source db.q "..."` — yangi `source` primitivi, reaktiv (`.loading/.err/.data/.reload()`), avtomatik invalidatsiya.
- **agent-b**: `load` bloki (`page` ichida server-side, natija view'ga uzatiladi) + `action` bloki (form server action).
- **agent-c**: oddiy `http.on` API + client `fetch` (eng kam o'ylangan).

Bu eng katta divergensiya — 6-bo'limda hal qilinadi.

---

## 2. `<-` REAKTIVLIK — hal qiluvchi natija

**Uchala model ham `<-`ni qayta ishlatdi, hech kim yangi reaktiv belgi/blok ixtiro qilmadi.** Bu maksimal token tejash va Flux falsafasiga mos. Yagona farq — *o'rab qo'yish*:

- **agent-a**: o'ramaydi — `count <- 0` to'g'ridan-to'g'ri `view` tanasida (eng kam token, eng tabiiy, `fn` ichidagi mutable bind kabi).
- **agent-b**: `state` blokiga o'raydi (`state` → `count <- 0`). Ortiqcha indentatsiya + kalit so'z, lekin "qaysi binding reaktiv UI state" ni aniq belgilaydi.
- **agent-c**: o'ramaydi, lekin `watch` qo'shadi (Flux'da `<-` allaqachon o'zgarishni biladi — `watch` ortiqcha, anti-pattern).

**Qaror:** agent-a yondashuvi (`state` bloksiz) g'olib. Sabab: Flux'da `fn` ichidagi `<-` allaqachon mutable; runtime uni signal qilib kompilyatsiya qilsa, dasturchi uchun hech qanday yangi sintaksis yo'q. `state` bloki ortiqcha token va "ikkinchi yo'l" (canonical formni buzadi). `watch` esa keraksiz — reaktivlik avtomatik.

---

## 3. DEFAULT → CONFIG → OVERRIDE — kim eng yaxshi yechdi

Bu foydalanuvchining ENG MUHIM talabi. Taqqoslash:

### agent-a (opus) — eng nafis, "bir xil shakl" printsipi ⭐
Override mexanizmi `reg` (mavjud Flux registry!) ustiga quriladi:
```flux
# DEFAULT
ui.table products.data
# CONFIG
ui.table products.data {cols:[:name :price] search::name sort::price}
# OVERRIDE — o'sha nomda `view` e'lon qil, runtime registry'dan almashtiradi
view my_table rows
  ...
my_table products.data
```
**Eng kuchli g'oya:** tayyor blok ham, mijoz komponenti ham bir xil `view name props` shaklida → chaqiruv o'zgarmaydi, faqat nom almashadi. Qisman override uchun `slot`/`cell::` override. Bu mavjud `reg` (dynamic dispatch) bilan organik bog'lanadi.

### agent-b (sonnet) — eng aniq, alohida `override` kalit so'z
```flux
# DEFAULT
ui.button "Saqlash"
# CONFIG
theme
  primary: "#10b981"
# OVERRIDE — yangi kalit so'z
override ui.button
  comp button text variant:"primary"
    button .btn."btn-${variant}" ...
```
Aniq va o'qiladigan, lekin **yangi `override` kalit so'zi** Flux falsafasiga qarshi ("kam belgi, kam kalit so'z"). agent-a buni `reg` bilan yangi kalit so'zsiz qildi.

### agent-c (haiku) — eng zaif
`cmp CustomButton {...}` deb butunlay qayta yozish — qisman override yo'q, slot yo'q, mexanizm noaniq.

**Qaror:** **agent-a g'olib** — uch daraja bir xil chaqiruv shakli + `reg`-asosli override (yangi kalit so'zsiz) eng nafis va eng Flux-idiomatik. agent-b'ning `theme` config darajasi va aniq `field`/`variant` g'oyalari qo'shimcha sifatida olinadi.

---

## 4. TOKEN — kim eng kam bilan eng to'liq dashboard yozdi

| Model | To'liq dashboard | Sahifa | Yondashuv |
|-------|-----------------|--------|-----------|
| **agent-a (opus)** | **~150 qator, ~3654 tok (butun javob)** | 5 sahifa to'liq + backend API + schema | `source` + `ui.*` default-by-omission |
| agent-b (sonnet) | ~380 qator, ~9557 tok | 5 sahifa, lekin har element Tailwind class qo'lda | `load`/`action` + qo'lda Tailwind |
| agent-c (haiku) | ~950 tok (faqat tavsif, kod yetarli emas) | sxematik | yetarli emas |

**agent-a eng zich.** Sababi — uning `ui.*` bloklari haqiqatan default-by-omission:
```flux
ui.table products.data        # ustunlar schema'dan, loading/fetch yashirin
ui.stat "Daromad" "${stats.data.revenue/100}$" {icon::cash kind::primary}
```
agent-b texnik jihatdan to'liqroq va realroq (Tailwind class'lar bilan haqiqiy dizayn), lekin har `div .bg-white.rounded-xl.p-6.shadow-sm` qatori token yeydi — bu "default" emas, qo'lda stillash. agent-b o'zining token jadvalida bitta stat-karta uchun React 420 → Flux 65 token (6.5x) ko'rsatdi, bu ishonarli, lekin uning dashboard'i agent-a'nikidan 2.6x ko'p token sarfladi, chunki Tailwind utility class'larni qo'lda yozdi.

**Asosiy dars:** eng kam token = (1) default-by-omission `ui.*` + (2) `source`/`load` glue-yo'q data + (3) `<-` qayta ishlatish + (4) semantik proplar (`{kind::primary}`) Tailwind string'lar o'rniga. agent-a to'rttasini ham qildi; agent-b faqat (2),(3) ni qildi, (1),(4) o'rniga Tailwind yozdi.

---

## 5. DIVERGENSIYA va XAVFLAR

| Joy | Divergensiya | Xavf / canonical-buzilish |
|-----|-------------|---------------------------|
| **Element sintaksisi** | a: `tag content {props}` · b: `div .class key:val "text"` · c: `{tag::div ...}` map | **c eng yomon** — `{tag::div children:[...]}` map sintaksisi token-og'ir va ichma-ich, Flux'ning indentatsiya-blok falsafasini buzadi. b'ning `.class` qisqa lekin Tailwind'ga bog'laydi. a'ning `tag content {props}` eng Flux-idiomatik (`tbl`/space-args kabi). |
| **CSS modeli** | a: semantik proplar (`{kind::primary pad:4}`) + `theme` · b: to'g'ridan-to'g'ri Tailwind class | **b XAVF** — Tailwind'ga qattiq bog'lanish, har element token og'ir, "ikki xil yo'l" (theme + class). a'ning semantik propi token-yengil va theme-markazlashgan. |
| **state o'rami** | b: `state` bloki · a/c: yalang `<-` | **b ortiqcha** — `state` ikkinchi yo'l, canonical formni buzadi. |
| **`watch`** (faqat c) | reaktivlikni qo'lda kuzatish | **c anti-pattern** — `<-` allaqachon avtomatik; `watch` keraksiz belgi. |
| **`render ->`** (faqat c) | komponent ichida oraliq metod | **c ortiqcha** — `fn` tana = qaytarish kabi, `render ->` qatlam token yeydi. |
| **postfix `if`** (faqat b) | `."bg-green" if trend >= 0` | **b XAVF** — Flux'da postfix if YO'Q. Yangi grammatika, "ikkinchi if shakli" — canonical buzilishi. |
| **data ulanishi** | a: `source` (reaktiv, client) · b: `load`/`action` (server-driven) · c: `fetch` | Arxitektura darajasidagi eng katta divergensiya (6-bo'limda hal qilinadi). |

**Eng jiddiy xavf:** Tailwind class'larni til sintaksisiga aralashtirish (agent-b). Bu Flux'ni Tailwind'ga abadiy bog'laydi, token og'irlashtiradi va "default UI" g'oyasini buzadi (agar har element class yozsa, default qayerda?). Flux falsafasi: semantik proplar + `theme`, Tailwind esa runtime'ning *ichki transpile detali* bo'lishi kerak, til yuzasida emas.

---

## 6. YAKUNIY FLUX FRONTEND SPEC TAVSIYASI

Konvergensiya (`<-`, `each`, `if`, space-args, `ui.*`, `page`) + Flux falsafasi (canonical form, kam token, default-by-omission, `reg`/`source` qayta ishlatish) asosida. Asos — **agent-a**, agent-b'dan `field`/config aniqligi, agent-b'dan `load`/`action` server modeli (lekin idiomatik shaklda).

### Primitiv 1 — `view` (komponent e'loni)

`fn`ning UI varianti. Tana = element daraxti (oxirgi ifoda = qaytariladigan UI, `fn` kabi). Prop = space-args, default-qiymatli prop `name:default` (agent-b'dan).

**Sintaksis:**
```flux
view name prop1 prop2:default
  <element daraxti>
```
**Namuna:**
```flux
view stat label value icon:nil
  div {kind::card pad:4}
    p value {size::xl bold:true}
    p label {kind::muted}
    if icon
      span icon
# chaqirish — space-args, fn bilan bir xil:
stat "Daromad" "$1200" icon::cash
```

### Primitiv 2 — Element yozuvi: `tag content {props}`

Bitta canonical shakl (agent-a). HTML teg yo'q, **canonical element**: teg nomi, so'ng matn/bola, so'ng ixtiyoriy `{props}` map. Bola = indentatsiya (Flux blok idiomasi, `tbl` kabi). Yopuvchi teg YO'Q.

**Sintaksis:**
```flux
tag "matn" {prop:val}      # bitta qatorda
tag {prop:val}             # bolalar indentatsiyada
  child1
  child2
```
**Namuna:**
```flux
div {kind::panel gap:3}
  h1 "Mahsulotlar"
  p "${items.len} ta" {kind::muted}
  btn "+ Yangi" {on:add kind::primary}
```
Asosiy teglar: `div p h1 h2 h3 span btn img input a ul li form`. Boshqasi `tag "raw" name` orqali. **Tailwind YO'Q** — proplar semantik (`kind:: pad: gap: size: bold:`), aniq ranglar `theme`da. (agent-b'ning Tailwind aralashtirishi RAD etiladi.)

### Primitiv 3 — Reaktiv state: yalang `<-` (yangi belgi/blok YO'Q) ⭐

Flux'ning mavjud mutable bind'i `view` ichida = reaktiv signal. O'zgarsa, faqat bog'liq DOM yangilanadi. `state` bloki YO'Q (agent-b RAD), `watch` YO'Q (agent-c RAD). Hosila qiymat = `=` (computed/memoized).

**Namuna:**
```flux
view counter
  n <- 0                          # reaktiv state
  doubled = n * 2                 # computed (n o'zgarsa qayta hisoblanadi)
  p "Soni: $n, ikki barobar: $doubled"
  btn "+1" {on:\-> n <- n + 1}
```

### Primitiv 4 — Event: prop ichida `on:` lambda yoki fn-qiymat

Yangi sintaksis yo'q — prop-map ichida. Element-default hodisa (`btn`→click, `form`→submit, `input`→change) `on:` bilan; aniq hodisa kerak bo'lsa `on::click`/`on::input`. Lambda argumenti `e` (`e.value e.data e.key`).

**Namuna:**
```flux
btn "Saqlash" {on:save}                  # save = fn qiymat (click)
btn "+1" {on:\-> n <- n + 1}             # lambda (click)
input {on::input \e -> q <- e.value}     # aniq hodisa
form {on:submit_handler}                 # form → submit
```

### Primitiv 5 — `bind:` (ikki tomonlama binding)

`input value + on:input` juftini bitta so'zga (agent-b'dan, kuchli token tejash). `bind:x` = state nomi.

**Namuna:**
```flux
q <- ""
input {bind:q placeholder:"Qidirish..."}     # = value:q + on:input(\e -> q <- e.value)
```

### Primitiv 6 — `each` / `if` (mavjud, qayta ishlatish)

Yangi narsa yo'q. `each` ro'yxat render, `key:` ixtiyoriy (agent-b, diff uchun). `if/elif/else` shartli render. **Postfix `if` YO'Q** (agent-b RAD).

**Namuna:**
```flux
each p in items key:p.id
  div {kind::row}
    b p.name
    if p.stock == 0
      badge "Tugadi" {kind::danger}
    else
      span "${p.stock} dona"
```

### Primitiv 7 — `theme` (config darajasi)

Global dizayn tokenlari. Flux `tbl` idiomasiga mos space-separated (agent-a), `key:` ikki nuqta blokda YO'Q (agent-b'ning `primary:` shakli Flux blok grammatikasiga zid). `theme.set` imperativ chaqiruv ham YO'Q (agent-c).

**Sintaksis:**
```flux
theme
  primary "#e84d8a"
  radius  :lg
  font    "Inter"
  mode    :light          # :light :dark :auto
```

### Primitiv 8 — `ui.*` (default batareya — `use ui`)

Tayyor, dizayni mukammal, theme'ga bo'ysunuvchi, accessible bloklar. Install yo'q (Flux batteries falsafasi). Default-by-omission: argument bermasa schema'dan oladi.

Asosiy bloklar: `ui.shell` (sidebar+header layout), `ui.table`, `ui.form`, `ui.stat`, `ui.chart`, `ui.modal`, `ui.input`, `ui.select`, `ui.search`, `ui.badge`, `ui.btn`.

**Default → Config:**
```flux
ui.table products                                          # DEFAULT (ustun schema'dan)
ui.table products {cols:[:name :price] search::name}       # CONFIG
ui.form product {on:save fields:[                          # CONFIG (agent-b field modeli)
  {name::name label:"Nomi" kind::text req:true}
  {name::price label:"Narx" kind::money}
]}
```

### Primitiv 9 — OVERRIDE: bir xil shakl + `reg` (yangi kalit so'z YO'Q) ⭐

agent-a'ning eng nafis g'oyasi. Tayyor `ui.X` ham, mijoz `view`'i ham bir xil `view name props` shaklida → o'sha nomda `view` e'lon qilinsa, runtime registry (`reg`) uni ishlatadi. agent-b'ning `override` kalit so'zi RAD (yangi belgi). Qisman override = `cell::`/`slot`.

**Namuna:**
```flux
# DEFAULT:  ui.table products
# OVERRIDE: o'sha shaklda o'z view'ing → chaqiruv o'zgarmaydi
view my_table rows
  div {kind::panel}
    each r in rows key:r.id
      div {kind::row hover:true}
        b r.name
        span "${r.price/100}$" {ml::auto}
my_table products

# QISMAN override (bitta ustun):
ui.table products {cols:[:name :price]
  cell::price \r -> badge "${r.price/100}$" {kind::ok}}
```

### Primitiv 10 — `page` (routing, deklarativ)

`http.on`ning UI varianti, file-system emas. `:param` backend bilan bir xil ajratish (literal > param — Flux route priority). agent-a/b konvergensiyasi.

**Sintaksis:**
```flux
page "/" -> dashboard
page "/products" -> products_page
page "/orders/:id" \params -> order_page params.id
nav "/products" "Mahsulotlar"     # SPA link (reload yo'q)
```

### Primitiv 11 — Data ulanishi: `source` (reaktiv) — DIVERGENSIYANI HAL QILISH ⭐

Bu eng katta divergensiya edi. **Qaror: agent-a'ning `source`'i asos, agent-b'ning server-side g'oyasi runtime'ga.**

`source` = reaktiv ma'lumot manbai, backend `db.q`/`http`/`ai` ustida. Avtomatik `loading`/`err`/`data`/`reload()`. **Glue kod yo'q** (eng katta token tejash). Mutatsiyadan keyin `ui.invalidate :tag` yoki `.reload()`.

**Namuna:**
```flux
view products_page
  items <- source db.q "select * from products order by ts desc"
  q     <- ""
  shown = items.data.filter \p -> str.has (str.low p.name) (str.low q)

  input {bind:q placeholder:"Qidirish..."}
  if items.loading
    ui.spinner
  else
    ui.table shown {cols:[:name :price :stock]}

fn save_product d
  http.post "/api/products" d
  ui.invalidate :items            # source qayta yuklanadi → jadval yangilanadi
```

Nima uchun `source` g'olib (agent-b'ning `load`/`action` emas): `source` mavjud Flux `db.q`/`http` chaqiruvi ustiga yupqa qatlam — yangi grammatika minimal. `load`/`action` esa `page` ichida ikki yangi blok turi (ko'proq kalit so'z). LEKIN agent-b'ning eng yaxshi g'oyasi — **same-file `db.q` bo'lsa, runtime avtomatik server endpoint generatsiya qiladi** — buni `source` ostida qabul qilamiz (pastdagi runtime tavsiyasi).

### Yangi kalit so'zlar ro'yxati (minimal — Flux falsafasi)

Faqat 4 ta yangi kalit so'z: **`view`** (komponent), **`theme`** (config), **`page`** (routing), **`source`** (data). Plus mavjud `each`/`if`/`elif`/`else`/`match`/`<-`/`=` qayta ishlatiladi, `ui.*` esa oddiy modul (kalit so'z emas). RAD etilganlar: `state`, `watch`, `render`, `override`, `comp`/`cmp`, `cmp`-tip annotatsiya, postfix `if`, `{tag::}` map element, Tailwind class.

---

### RUNTIME TAVSIYASI (transpile target / server-driven / signals)

Uchala modelning runtime g'oyalarini birlashtirib, Flux'ning mavjud Rust runtime + falsafasiga moslab:

1. **Fine-grained signals (Solid/Svelte uslubi, virtual-DOM YO'Q).** `<-` reaktiv tugun → kompilyatsiya vaqtida signalga aylanadi; o'zgarganda faqat bog'liq DOM bo'lagi yangilanadi. `=` = memoized computed. Virtual-DOM diff (agent-c React g'oyasi) RAD — Flux'ning "kam token, aniq" falsafasiga signals mosroq va yengilroq client beradi. (agent-a va agent-b ikkovi ham signals tomon.)

2. **Transpile target + yupqa client (React EMAS).** `.fx`dagi `view`/`page`/element daraxti → JS (yoki WASM) + ~5KB Flux signal+DOM-patcher client. Alohida framework yuklanmaydi. agent-c'ning "React'ga transpile" g'oyasi RAD — React bundle og'ir va Flux'ning "batteries, no install, kam token" falsafasiga zid. agent-b ham React'dan voz kechib Phoenix-LiveView+Svelte-kompilator tomon ketdi (to'g'ri yo'nalish).

3. **Server-driven default + client-hydration opt-in (agent-b'ning eng yaxshi g'oyasi).** Default holatda `state`/`source` server-side, `on:` minimal WebSocket/fetch yuboradi, server diff'ni qaytaradi (LiveView uslubi → JS ~0). Og'ir interaktivlik kerak bo'lsa `page "/x" {client:true}` → o'sha sahifa signals-JS'ga to'liq transpile (hydration). Bu Flux'ning bitta event-loop arxitekturasiga (http.serve + ws.serve birga) tabiiy o'tiradi.

4. **`source` → avtomatik RPC.** Same-file `db.q`/`db.one` bo'lsa, runtime avtomatik backend endpoint generatsiya qiladi (agent-b g'oyasi); tashqi bo'lsa `http.get`. Cache, dedupe, `loading/err/data`, `reload()`/`ui.invalidate :tag` — runtime mas'uliyati. Dasturchi fetch/useEffect YOZMAYDI. Realtime invalidatsiya mavjud `ws` batareyasi ustida.

5. **Override = `reg` (mavjud registry).** Nomli `view` global `reg`'ga tushadi; `ui.X` chaqiruvi `reg`'dan qidiradi → mijoz o'sha nomda `view` e'lon qilsa, override avtomatik. Yangi mexanizm YO'Q — Flux'ning `reg` (dynamic dispatch) idiomasi to'g'ridan-to'g'ri qayta ishlatiladi.

6. **`ui.serve app port` — bitta nuqta.** HTTP API + UI client bundle + WS bitta portda, bitta event-loop (mavjud `http.serve`/`ws.serve` bilan birga). CSS: `theme` tokenlari → CSS custom properties (`--flux-primary`); semantik proplar (`{kind::primary pad:4}`) → runtime ichidagi tayyor stil tizimi (Tailwind ixtiyoriy ichki detal, til yuzasida YO'Q).

**Bir jumlada:** Flux frontend = signals (Svelte/Solid kompilator) + server-driven-default/hydration-opt-in (Phoenix LiveView) + `source` glue-yo'q data + `reg`-asosli override — hammasi mavjud Flux idiomalari (`<-`, `each`, `if`, space-args, `reg`, `db.q`, bir event-loop) ustiga 4 ta yangi kalit so'z (`view`/`theme`/`page`/`source`) bilan qurilgan, React'siz, Tailwind'ni til yuzasiga chiqarmasdan.

---

**Tegishli fayllar:**
- Mavjud frontend doc (qayta yozilishi kerak — hozir agent-c'ning React/`{tag::}` map yondashuvi, RAD etilgan): `/Users/whoami/Documents/files/draft-project/flux-lang/docs/flux-frontend.md`
- Manba spec (idiomalar shu yerdan): `/Users/whoami/Documents/files/draft-project/flux-lang/docs/flux-agent.md`
- Runtime (signals + DOM-patcher + `ui.*` batareya bu yerga qo'shiladi): `/Users/whoami/Documents/files/draft-project/flux-lang/runtime/src/` (naqsh: `http_mod.rs`, `reg_mod.rs`, `ws_mod.rs`)