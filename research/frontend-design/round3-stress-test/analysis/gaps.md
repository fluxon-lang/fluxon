Spec'ni o'qib chiqdim va 3 modelning ishini tahlil qildim. Bu pure-analiz vazifa — kod yozish yoki ishga tushirish kerak emas, faqat spec muhandisligi. Quyida batafsil hisobot.

---

# Flux Frontend Spec — Stress-Test Tahlili

3 model (opus/sonnet/haiku) bir xil spec'ni birinchi marta ko'rib restoran admin panelini yozdi. Konvergent bo'shliqlar (2+ model mustaqil topgan) eng jiddiy signal. Quyida ularni jiddiylik bo'yicha tartibladim.

---

## 1. KONVERGENT BO'SHLIQLAR (eng muhim)

### KB-1 — Frontend WS tinglovchisi YO'Q (3/3 model, eng kritik)

**Bo'shliq:** Spec backend WS'ni to'liq beradi (`ws.on :connect/:message/:disconnect`, `ws.room.send`, `ws.room.join`) VA `ui.serve` tavsifida "WS for realtime `source` invalidation" deb yozadi. Lekin **frontend tomonda** kelgan WS xabarini qanday tinglash kerakligini umuman ko'rsatmaydi. Bu spec'ning eng katta yorig'i, chunki "realtime" Flux'ning sotuv nuqtasi (`ui.serve` bo'limida aniq va'da qilingan), lekin amalda yozib bo'lmaydi.

**Modellar qanday turlicha taxmin qildi** (turlicha taxmin = spec noaniq isboti):
- **opus**: yangi funksiya ixtiro qildi — `ui.on_ws "orders" \msg -> ui.invalidate :orders`. Spec'da bunday narsa YO'Q deb o'zi belgiladi.
- **sonnet**: WS'ni butunlay tashladi, faqat `ui.invalidate` (pull) bilan kifoyalandi va izoh qoldirdi: *"source batareyasi WS kanalni ichida avtomatik tinglaydi deb taxmin qildim — ammo bu to'liq real-vaqt emas (push emas, pull)"*. Hatto `ws.serve 3001` ni alohida portga chiqarib yubordi (spec esa BITTA port deydi — bu spec talabini buzdi).
- **haiku**: backend `ws.room.send "orders"` yozdi, lekin frontend listen kodini umuman yoza olmadi, faqat "BO'SHLIQ" deb belgiladi.

Uchta model uch xil yo'l tutdi (yangi API ixtiro / pull-only / hech narsa) — bu spec'ning aniq noaniqligi.

### KB-2 — `source` tag'i `ui.invalidate` bilan qanday bog'lanadi? (2/3: opus, sonnet)

**Bo'shliq:** `items <- source db.q "..."` deb e'lon qilinadi, keyin `ui.invalidate :items` chaqiriladi. `:items` symbol qaysi source'ga bog'lanishi spec'da aytilmagan. Spec misolida tasodifan o'zgaruvchi nomi (`items`) tag bilan (`:items`) bir xil bo'lib qolgan, lekin bu qoidami yoki tasodifmi — aytilmagan.

**Turlicha taxmin:**
- **opus**: *"`<- source` o'zgaruvchi nomidan tag olinadi deb taxmin qildim"*.
- **sonnet**: aynan shu taxminni qildi, lekin alohida belgiladi: *"`<-` o'zgaruvchi nomi avtomatik tag bo'ladimi yoki alohida yozish kerakmi? Aniq aytilmagan"*. Yana ikkinchi savol: ko'p so'zli tag (`:staff_list`) ishlaydimi — bu ham noaniq.

Ikkalasi ham bir xil taxminga keldi, lekin ikkalasi ham bu taxmin ekanligini va spec tasdiqlamasligini ta'kidladi. Bu "lucky default" — spec uni qoidaga aylantirishi shart.

### KB-3 — `fn` ichidan `view`-lokal `<-` state'ni o'zgartirish (2/3: opus, sonnet; haiku ham bilvosita)

**Bo'shliq:** `view` ichidagi `<-` state'ni (`open`, `edit_row`, `detail`) tashqi `fn`dan (`save_product`, `do_edit`) yangilash kerak. Spec'da `<-` faqat `view` ichida e'lon qilinadi, lekin `fn` u state'ga qanday yetadi — scope/closure modeli umuman yo'q.

**Turlicha taxmin:**
- **opus**: bu ishlamaydigan workaround yozishga majbur bo'ldi — `reg.call "set_order_detail" {detail:d.body}` va hatto `log` bilan. O'zi *"bu ishlamaydi, faqat bo'shliqni belgilash uchun"* dedi.
- **sonnet**: closure deb taxmin qildi — `fn do_edit` ichida `edit_item <- item` to'g'ridan-to'g'ri yozdi va *"fn view ichida e'lon qilinsa closure orqali kiradi deb qabul qildim. Ammo spec'da fn va view ko'rinuvchilik doirasi hech qayerda tushuntirilmagan"* dedi. Lekin uning `fn`lari `view`dan TASHQARIDA e'lon qilingan — ya'ni closure ishlamaydi, bu jim xato.
- **haiku**: *"each ichida state `<-` bind qanday ISOLATE qilish kerakligini (component scope) ANIQ AYTILMAGAN"* — scope modeli yo'qligini boshqa rakursdan topdi.

Bu KB-1 dan keyingi eng jiddiy bo'shliq: modal ochish/yopish, edit-row tanlash — har qanday CRUD UI'ning yuragi, va hech kim uni ishonchli yoza olmadi.

### KB-4 — `ui.chart` props butunlay yo'q (2/3: opus, sonnet)

**Bo'shliq:** Spec `ui.chart`'ni faqat blok ro'yxatida sanaydi, bironta props misoli yo'q. Dashboard grafigi har qanday admin panelda bor.

**Turlicha taxmin:**
- **opus**: `ui.chart chart.data {x::day y::rev kind::bar}` — *"sof taxmin"*.
- **sonnet**: `ui.chart s.hourly_revenue {kind::line x::hr y::rev fmt::rev \v -> ...}` — *"to'liq ixtiro"*.

Ikkalasi `x::` `y::` `kind::` ni topdi (yaqin), lekin biri `:bar` biri `:line` default, va sonnet `fmt::` ham qo'shdi. Yaqin taxminlar — demak intuitiv, spec'ga rasmiylashtirish oson.

### KB-5 — `ui.select` mustaqil ishlatish + `opts` formati (2/3: opus, sonnet)

**Bo'shliq:** Spec `ui.select`'ni faqat `ui.form` ichidagi field sifatida (`kind::select opts:[...]`) ko'rsatadi. Mustaqil komponent sifatida `ui.select {bind:cat opts:[...]}` ishlatish tasdiqlanmagan, va `opts` faqat symbol massivimi yoki `[value label]` juftlik bo'la oladimi — aytilmagan.

**Turlicha taxmin:**
- **opus**: `ui.select {bind:cat opts:[:all :starter ...]}` — oddiy symbol massivi, "all" ni symbol qildi.
- **sonnet**: `opts:([:_ "Barcha turlar"] ++ (categories.map \c -> [c (str.str c)]))` — label kerak bo'lgani uchun `[symbol "label"]` juftliklarni ixtiro qildi va `++` konkatenatsiya bilan murakkablashtirdi. *"`[symbol "label"]` ko'rinishdagi tuple array — taxmin, spec'da aniq emas"*.

Bu muhim divergensiya: label-li select real ehtiyoj, lekin spec faqat "yalang'och symbol" beradi → sonnet murakkab workaround yozdi.

---

## 2. JIDDIY YAKKA BO'SHLIQLAR

**YB-1 — `source` ni dinamik parametr bilan ochish (opus, haiku)** — "tugma bosilganda `id` bo'yicha detail yuklash". opus: *"dinamik id bilan source ochish naqshi spec'da yo'q"*. haiku: stol uchun buyurtma ochishda aynan shu muammoga urildi. Bu KB-3 bilan bog'liq, lekin alohida: imperativ "hozir shu id bilan fetch qil" naqshi yetishmaydi.

**YB-2 — `source` yuklangach BIR MARTA effekt (sonnet)** — settings sahifasida `cfg` yuklangach `<-` state'larni to'ldirish kerak edi. sonnet `if !cfg.loading & cfg.data` blokini yozdi va o'zi tan oldi: *"bu blok har render'da qayta ishlaydi — bu to'g'ri emas"*. Spec'da `watch`/`effect` yo'q (atayin olib tashlangan), lekin "data kelgach state'ga ko'chir" real ehtiyoj. Bu odatda derived `=` bilan hal qilinadi — spec buni ko'rsatmagan.

**YB-3 — `ui.close` semantikasi (sonnet)** — qaysi modalni yopadi? sonnet himoya uchun `ui.close` dan keyin yana `open_add <- false` yozdi. Bir nechta modal bo'lganda `ui.close` qaysi birini yopishi noaniq.

**YB-4 — `input kind::color` (sonnet, haiku)** — spec form field turlari `:text :money :select :bool` (va `:number`?). `:color` yo'q. Ikki model rang tanlovchi kerak bo'lganda ixtiro qildi.

**YB-5 — Dinamik/massiv form (haiku)** — buyurtmaga N ta taom + har biriga qty. `ui.form` faqat tekis field'lar. Repeating/array field naqshi yo'q. Bu jiddiy: `ui.form`'ning chegarasi.

**YB-6 — `match` expression sifatida (sonnet, haiku belgiladi)** — `kind = match st ...` qiymat qaytaradimi? Spec faqat statement-render misoli beradi. Aslida `if/else` props ichida expression sifatida ko'rsatilgan (`kind:(if ...)`), lekin `match` uchun bunday misol yo'q. Uchala model ham `match`'ni expression sifatida ishlatdi (status→kind/label) — demak juda kerakli, lekin tasdiqlanmagan.

---

## 3. KODDAGI XATOLAR — model adashgani vs spec yetishmagani

### Model adashgan (spec aniq edi):
- **opus — `db.up "tables" {status::busy} {table_id:...}`**: where kaliti `table_id` yozdi, `id` bo'lishi kerak edi. opus o'zi tan oldi. Spec `{where}` map ekanini aniq beradi — bu sof model xatosi, spec gap emas.
- **sonnet — `ws.serve 3001` alohida portda**: spec `ui.serve` BITTA portda HTTP+UI+WS deb aniq aytadi. sonnet ikkinchi port ochib spec falsafasini buzdi. Bu model xatosi (lekin KB-1 noaniqligidan kelib chiqqan — frontend WS yo'qligi uni chalkashtirgan).
- **haiku — "schema'da `http.on :get` yo'q degan xato"**: haiku o'zicha endpoint yetishmaydi deb o'ylab qo'shdi; aslida `http.on` spec'da bor. Model chalkashligi.

### Spec haqiqatan yetishmagan (yuqoridagi KB/YB hammasi):
- frontend WS, source-tag bog'lanishi, fn→view state scope, ui.chart props, ui.select opts, dinamik source, source-effect, ui.close, color field, array form. Bular model xatosi emas — yozish mumkin emas edi.

### Chegaraviy (spec aytadi, lekin yetarli emas):
- **`source http.get`**: spec "external → http.get" deydi, lekin BARCHA misol `db.q`. `http.get` natijasi to'g'ridan-to'g'ri `.data`ga keladimi yoki `.body` orqalimi — uchala model `.data` deb taxmin qildi (opus belgiladi). Spec buni misol bilan tasdiqlashi kerak.
- **`:patch` metodi**: sonnet ishlatdi, o'zi *"spec backend handler sifatida :patch'ni aniq ko'rsatmagan"* dedi. Spec metod ro'yxatini to'liq sanashi kerak.

---

## 4. REAL-VAQT (ws) INTEGRATSIYASI — alohida xulosa

Bu **eng katta tizimli bo'shliq** (KB-1). Spec backend-WS va "realtime invalidation" va'dasini beradi, lekin frontend-WS ni butunlay tashlab ketgan. Natija: **bironta model ishlaydigan realtime yoza olmadi.**
- opus → yo'q API ixtiro qildi (`ui.on_ws`)
- sonnet → realtime'dan voz kechdi (pull-only), spec'ning bitta-port qoidasini ham buzdi
- haiku → backend yozdi, frontend'ni bo'sh qoldirdi

Spec va'da bergan, lekin bajara olmaydigan funksiya — bu eng yomon spec holati (yolg'on va'da). Darhol yopilishi shart. Tuzatish quyida (ST-1).

---

## 5. DEFAULT → CONFIG → OVERRIDE — amalda ishladimi?

**Default va Config a'lo ishladi.** Uchala model ham `ui.table products` (default) va `ui.table shown {cols fmt cell actions}` (config) ni muammosiz, izchil yozdi. Bu spec'ning eng kuchli qismi.

**Override — qisman, va `reg` ishlatilMADI.** Spec deydi: "Named `view`s register in `reg` ... that's the override mechanism". Lekin:
- opus va sonnet override qildi (`table_grid`, `order_row`, `status_badge` o'z `view`lari), lekin **hech biri `reg` ga aniq ro'yxatdan o'tkazmadi** — shunchaki yangi nomli `view` yozib, uni chaqirdi. Ya'ni amalda "override" emas, "yangi komponent yozish + uni chaqirish" bo'ldi.
- Bu shuni ko'rsatadi: spec'dagi "`reg` orqali override" jumlasi **amalda hech narsani anglatmaydi** — modellar `ui.table` o'rniga o'z view'ini chaqirdi, `ui.table`'ni "almashtirmadi". Agar 10 joyda `ui.table` chaqirilsa, hammasini qo'lda almashtirish kerak — bu "override" emas.

**Xulosa:** Default→Config ajoyib. "Override = reg" da'vosi gapda bor, kodda yo'q. Spec yo `reg` bilan haqiqiy override misolini berishi, YOKI "override = shunchaki o'z view'ingni yoz va chaqir" deb halol aytishi kerak (hozirgi `reg` jumlasi chalg'ituvchi). Partial override (`cell::`/`fmt::`/`slot`) ham — `slot` hech qayerda misol bilan ko'rsatilmagan, hech kim ishlatmadi.

---

## 6. SPEC TUZATISHLARI (eng muhim qism)

Quyida docs'ga to'g'ridan-to'g'ri qo'shiladigan, ixcham, Flux-falsafasiga sodiq matnlar. Token byudjetiga rioya qildim (har biri kichik).

---

### ST-1 — Frontend WS / realtime (KB-1, eng kritik). Yangi: `live` source + `ui.on`

Falsafa: yangi keyword emas, mavjud `source`'ni kengaytirish. `live` modifikatori = source WS kanalga avtomatik ulanadi, server `ui.push :tag` qilganda o'zini qayta yuklaydi. Imperativ ehtiyoj uchun `ui.on`.

`source` bo'limiga qo'shiladigan matn:

```
## Realtime source — `live`
A `source` marked `live` auto-subscribes to the server WS channel named by its
tag. When the server calls `ui.push :tag`, every connected client's matching
source reloads — no client WS code. `ui.push` is the broadcast twin of the
local `ui.invalidate`.

  orders <- source live db.q "select * from orders order by ts desc"
  # server side, after a mutation:
  fn save_order d
    db.ins "orders" d
    ui.push :orders          # ALL clients' :orders source reloads (via WS)

Raw WS messages (no source): `ui.on :tag \msg -> ...` inside a view.
  ui.on :orders \msg -> log msg.event
`ui.serve` owns the WS channel — same port, no `ws.serve` needed for this.
```

Bu KB-1 ni to'liq yopadi, sonnet'ning ikki-port xatosini oldini oladi, opus'ning `ui.on_ws` ixtirosini rasmiylashtiradi (`ui.on`).

---

### ST-2 — Source tag qoidasi (KB-2). Bitta jumla yetadi.

`source` bo'limidagi `ui.invalidate` qatoriga qo'shish:

```
A source's tag IS its bind name: `items <- source ...` registers tag `:items`.
`ui.invalidate :items` / `ui.push :items` / `ui.on :items` all refer to it.
Multi-word names work: `staff_list <- source ...` → `:staff_list`.
```

Bu "lucky default"ni rasmiy qoidaga aylantiradi va sonnet'ning ko'p-so'zli tag savoliga javob beradi.

---

### ST-3 — `fn` → `view` state scope (KB-3, ikkinchi eng kritik). Yangi: `act` view-handler.

Bu eng nozik. Falsafa: `<-` state `view`ga tegishli; tashqi `fn` unga yeta olmaydi (bu to'g'ri — global mutatsiya yomon). Yechim: handler `view` ichida lambda bo'lsin (eng oddiy), VA bir necha qatorli handler uchun `view` ichida nomli `act` e'lon qilinsin.

`view` yoki Events bo'limiga qo'shish:

```
## Handlers & state scope
`<-` state is LOCAL to its `view`. An outside `fn` cannot mutate it. Write the
handler inline as a lambda, or name it with `act` INSIDE the view (closes over
state):
  view menu_page
    open <- false
    edit <- nil
    act open_new            # multi-line handler, sees `open`/`edit`
      edit <- nil
      open <- true
    btn "+ Yangi" {on:open_new}
    ui.modal {open:open}
      ui.form edit {on:save}    # `save` may be a plain fn (data in, http out)
A plain top-level `fn` is for data/IO (http/db), takes args, returns — it never
touches view state. After a server mutation it calls `ui.invalidate`/`ui.push`.
```

`act` = "action" = view-ichidagi state'ga yeta oladigan nomli handler. Yangi token kam, `fn`/`view` ajratimini halol qiladi, opus'ning `reg.call` workaroundini va sonnet'ning noto'g'ri-scope closuresini ikkalasini ham yopadi.

---

### ST-4 — Dinamik source + post-load → state (YB-1, YB-2). Derived `=` bilan.

ST-3 ostiga qo'shish:

```
Dynamic source: bind a source to a reactive arg — it refetches when the arg
changes (no imperative fetch):
  sel <- nil                              # selected id
  detail <- source if sel db.one "select * from orders where id=$1" [sel]
  act show \r -> sel <- r.id             # click → source refetches
Use derived `=` to read loaded data (recomputes when data arrives) instead of
copying into state:
  name = cfg.data.rest_name ?? ""        # NOT: name <- ... on load
```

Bu YB-1 (dinamik fetch) va YB-2 (yuklangach-effekt) ni ikkalasini bitta idioma bilan yopadi — `watch` qo'shmasdan.

---

### ST-5 — `ui.chart` props (KB-4). Mavjud `cols/fmt` naqshini takrorlash.

`ui.*` bo'limiga, `ui.table` yonidan:

```
ui.chart data {kind::line x::day y::rev fmt::rev \v -> "${v/100}$"}
  # kind:: :line :bar :area · x::/y:: = field syms · fmt:: per series (as table)
```

Bir qator. Ikkala model topgan `x:: y:: kind::` ni rasmiylashtiradi, `fmt::` ni `ui.table` bilan izchil qiladi.

---

### ST-6 — `ui.select` opts + label (KB-5). `[val "label"]` juftligini rasmiy qil.

`ui.*` bo'limiga:

```
ui.select {bind:cat opts:[:all :main :drink]}        # symbols = label from name
ui.select {bind:cat opts:[[:all "Barchasi"] [:main "Asosiy"]]}  # [val label]
```

sonnet'ning `++`/`map` murakkabligini keraksiz qiladi.

---

### ST-7 — Form field turlari to'liq ro'yxati (YB-4) + array field (YB-5).

`ui.form` misoliga `kind::` ro'yxatini qo'shish:

```
field kind:: :text :money :number :select :bool :color :date :textarea
Repeating field (N-rows, e.g. order lines): kind::list with sub-fields.
  {name::items label:"Taomlar" kind::list fields:[
    {name::item kind::select opts:menu} {name::qty kind::number}]}  # value = array
```

YB-4 (color) va YB-5 (haiku'ning eng katta to'sig'i — array form) ni yopadi.

---

### ST-8 — `match` expression + metod ro'yxati (YB-6, chegaraviy xatolar). Bir-bir jumla.

`match` reused deb aytilgan joyga (frontend spec boshida) qo'shish:
```
`match`/`if` are EXPRESSIONS (return the matched arm) — usable in `=`, props, args:
  kind = match st (:new -> :info  :done -> :ok  _ -> :muted)
```
`source` bo'limiga:
```
`source http.get "/url"` → `.data` is the parsed body directly (not {status,body}).
```
`page`/http metodlar uchun (backend spec'da): `:get :post :put :patch :del` to'liq sanab qo'yish.

---

### ST-9 — Override haqiqatini halol qilish (Bo'lim 5).

Hozirgi "Named views register in `reg` — that's the override mechanism" jumlasi chalg'ituvchi (hech kim `reg` ishlatmadi). Ikki variant — men 2-ni tavsiya qilaman (kamroq token, halol):

```
# OVERRIDE — write a view with your own name, call it instead of ui.table.
# To replace a ui.* block GLOBALLY (every call site at once), register it:
reg ui.table my_table        # now every `ui.table` renders via my_table
```

Agar `reg` global-almashtirish'ni qo'llab-quvvatlamasa, jumlani olib tashlab, shunchaki "write your own view and call it" deyish kerak — chunki hozirgi da'vo amalda yolg'on.

---

## Yakuniy ustuvorlik (yopish tartibi)

1. **ST-1 (frontend WS / `live` + `ui.on`)** — va'da qilingan, bajarib bo'lmaydigan funksiya. Eng shoshilinch.
2. **ST-3 (`act` / fn-view scope)** — har qanday CRUD UI buzilgan. 
3. **ST-2 (source tag qoidasi)** — bir jumla, katta noaniqlikni yopadi.
4. **ST-4 (dinamik source + derived)** — modal/detail naqshlari uchun zarur.
5. **ST-5/6/7 (chart/select/form-kinds)** — ixtiro qilingan API'larni rasmiylashtirish.
6. **ST-8/9 (match-expr, metodlar, override halolligi)** — kichik aniqliklar.

ST-1..ST-4 frontend'ning haqiqiy yoriqlari (yozib bo'lmaydigan narsalar). ST-5..ST-9 esa "ishladi-yu, taxmin bilan" turidagi — rasmiylashtirish. Hammasi birga ~600 token, frontend spec'ni 3k chegarasida ushlab turadi.