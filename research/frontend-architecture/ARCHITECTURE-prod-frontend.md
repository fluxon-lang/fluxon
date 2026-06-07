Yetarli kontekstga egaman. Real kod strukturasini tushundim: element = `{__node}` map, view = `Value::Fn{is_view}`, SSR `node_to_html` sof funksiya, `ui.serve` deferred server, theme/pages/routes state'lari. Endi arxitektura hujjatini tuzaman.

Hozir `attrs_html` `on:`/`bind:` proplarini SSR'da TASHLAB YUBORADI — bu aniq integratsiya nuqtasi. Endi to'liq arxitekturani beraman.

---

# FLUX FRONTEND: AVTOMATIK SSR/CLIENT AJRATISH ARXITEKTURASI

Mavjud kodga (ui_mod.rs `{__node}` map, `node_to_html`, `view=Value::Fn{is_view}`, `ui.serve`) tayanib loyihalashtirilgan. Falsafa: bir ish=bir yo'l, dasturchi hech narsa belgilamaydi, analyzer o'zi qaror qiladi.

## 0. ASOSIY QAROR (TL;DR)

**Flux "Marko-uslub avtomatik partial hydration" + "Astro-uslub island" + "Solid-uslub signal" ni oladi, RSC-uslub server/client chegarasi bilan.**

Sabab dalil bilan (bo'lim 2-da chuqurroq): Flux falsafasi = dasturchi belgilamaydi → **Qwik/Astro/Next RAD** (ular `$`, `client:load`, `'use client'` — qo'lda marker talab qiladi). Marko yagona "to'liq avtomatik" (statik tahlil `<let>`/state izidan island topadi). Flux'da bu iz allaqachon mavjud: **`<-` = reaktiv state, `on:` = event, `act` = handler, `source.reload` = client-trigger**. Demak Flux Marko qila olganidan ham aniqroq qila oladi — chunki Flux'da reaktivlik belgisi (`<-`) til grammatikasida, "guess" qilish kerak emas.

---

## 1. AVTOMATIK AJRATISH MEXANIZMI (eng muhim)

### 1.1 Asosiy qoida: "interaktivlik izi" (taint)

Analyzer har `view` tanasini AST darajasida traverse qiladi va **bitta savol**ga javob beradi: *"Bu element daraxtining biror qismi browserda o'zgaradimi?"*

Element/qism **client island** bo'ladi, FAQAT VA FAQAT u quyidagi izlardan birini o'z ichiga olsa (transitiv):

| Iz | Ma'no | Manba (kod) |
|----|-------|-------------|
| `on:click \-> ...` | event handler | parser `props["on"]` |
| `act nom \-> ...` chaqiruvi | view-lokal handler | `Stmt`/`Expr` act |
| `<-` o'zgaruvchi O'QILISHI | reaktiv state binding | `Expr::Ident` → `<-` bind'ga |
| `bind:value x` | two-way binding | `props["bind"]` |
| `source` `.reload`/`live` | client-trigger reaktiv data | `source` decl |

**Aksincha, qism SOF STATIK (SSR, 0 JS) bo'ladi** agar uning butun subtree'sida hech bir iz bo'lmasa: faqat `=` immutable qiymatlar, literal matn, `each`/`if`/`match` (statik manba ustida), `source.data` (faqat o'qish, reload yo'q).

### 1.2 Reaktivlik grafi (compile-time, Solid/Svelte uslub)

Analyzer view tanasidan **dependency grafi** quradi (Svelte 4-pass naqshi):

```
Pass 1 — Bindlarni yig'ish: har bind nomi -> {kind: Immut(=)|React(<-)|Source, span}
Pass 2 — O'qish/yozishlarni topish: har Expr::Ident -> qaysi bindga tegishli;
         har on:/act tanasida `x <- ...` -> WRITE; element ichida `x` -> READ.
Pass 3 — Dependency edge: React node -> uni o'qiydigan element/expression node.
Pass 4 — Island chegarasi: bir element subtree'sida >=1 React-READ yoki event/act
         bo'lsa -> "interaktiv". Aks holda statik. Eng kichik o'rab turuvchi
         statik bo'lmagan element = ISLAND ildizi (yuqoriga ko'tarish: agar
         element ichidagi matn React state'ga bog'liq bo'lsa, o'sha element island).
```

Bu **`{__node}`ga yangi maydon** qo'shadi: `island_id` (Some=client, None=statik). Yangi `Value` varianti SHART EMAS — http_mod `{__resp}` idiomasi davom etadi (CLAUDE.md invarianti saqlanadi).

### 1.3 Dataflow + xavfsizlik tahlili (RSC-uslub taint)

Ikkinchi, alohida taint: **`source`/`db`/`http.*`/`ai.*`/env/secret = server-only**. Bu qiymatlar grafda "server" deb belgilanadi. Qoida:

- `source` HAR DOIM serverda qoladi (db.q/http/ai ustida) — uning **natijasi** (`.data`) island'ga prop sifatida o'tishi mumkin, lekin **source ifodasi** (SQL, API key, db handle) hech qachon client'ga ketmaydi. Bu RSC `getPostDTO` naqshi: faqat serializable natija o'tadi.
- Agar island server-only qiymatni (db handle, secret) closure'da ushlasa → **compile xato** (`fail "island server-only qiymatni ushlay olmaydi: db.q"`). Bu React `taint` API ekvivalenti, lekin compile-time majburiy.

### 1.4 Aniq misol — qaysi qism qayerga ketadi

```flux
view dashboard
  stats = db.q "select count(*) c from orders"      # SERVER-ONLY (db)
  filter <- ""                                        # REACT state
  items source db.q "select * from products"         # SERVER source

  div
    h1 "Do'kon paneli"                                # STATIK -> SSR HTML
    p "Jami buyurtma: ${stats.c}"                     # STATIK (server data, o'zgarmaydi)

    input {bind:value filter placeholder:"qidir"}     # ISLAND (bind:)
    each p in items.data                              # source.data O'QISH
      if p.name.has filter                            # filter = REACT READ -> ISLAND
        div {kind::panel}
          h3 p.name                                   # island ichida (filter'ga bog'liq qism)
          btn "Sotib ol" {on:click \-> cart.add p}    # ISLAND (on:)
```

Analyzer natijasi:
- `h1`, `p "Jami buyurtma"` → **statik HTML**, 0 JS. CDN-cacheable.
- `input` + `each ... if filter` bloki → **bitta island** (`island_id=1`), chunki `filter` (React) o'qiladi va `each` natijasi unga bog'liq. Bu island browserda qayta render qilinadi.
- `btn on:click` → island ichidagi event → handler chunk.
- `db.q`, `source` ifodasi → **serverda qoladi**, faqat `stats.c` (skalyar) va `items.data` (mahsulot ro'yxati DTO) wire orqali o'tadi.

Muhim nuance: `each p in items.data` — agar `filter` bo'lmaganida (sof `each` + statik) butun blok SSR bo'lardi va 0 JS ketardi. `filter` READ butun `each` blokini island'ga ko'taradi (chunki ro'yxat browserda filtrlanadi). Bu **avtomatik** — dasturchi `client:load` yozmaydi.

---

## 2. CLIENT KOD QAYERDAN KELADI

### Qaror: **GIBRID — kichik universal client runtime (JS) + per-island serializatsiyalangan "recipe", transpiler EMAS (1-faza), keyin selektiv transpile (2-faza)**

Uchta variant solishtiruvi:

| Variant | Flux falsafasiga mosligi | Performance | Hukm |
|---------|--------------------------|-------------|------|
| (a) Flux→JS to'liq transpile | Yangi backend (JS codegen) — katta murakkablik, Rust↔JS semantika farqi | Eng yaxshi runtime | Hozir RAD (token/murakkablik) |
| (b) Flux interpreter browserda (WASM) | Bitta semantika (Rust interp→WASM) — falsafaga mos | WASM yuklash ~100KB+, AI uchun ortiqcha | RAD (og'ir) |
| (c) **Gibrid: universal JS runtime + island recipe** | Yagona ~6-10KB client, server-driven default | Minimal JS, statik CDN | **TANLANDI** |

### Hydration emas, "server-driven + selektiv resumability" (Phoenix LiveView + Qwik aralash)

Memory'dagi qaror (LiveView-uslub) to'g'ri, lekin aniqlashtiramiz:

1. **Default rejim — server-driven (LiveView).** Island'da event bo'lsa (`on:click`), client minimal JS event'ni serverga WS orqali yuboradi, server view'ning O'SHA island'ini qayta render qiladi (Rust interp, tez), faqat diff'ni qaytaradi, client DOM-patch qiladi. **Client'da Flux logikasi YO'Q** — handler tanasi (`cart.add p`) serverda ishlaydi. Bu eng kam JS va falsafaga eng mos (handler = backend logikasi, db'ga yetadi).

2. **Selektiv client-only (resumability) — faqat sof-UI state uchun.** Agar island handler'i SERVER-FREE bo'lsa (faqat `<-` state'ni o'zgartiradi, db/http/ai/source TEGMAYDI — masalan `count <- count + 1`, modal ochish/yopish, input filter), analyzer uni **client-da bajariladigan** deb belgilaydi va o'sha kichik handler'ni JS recipe sifatida serializatsiya qiladi. Network'siz, instant. Bu Qwik resumability'ning soddalashtirilgan, **avtomatik aniqlanadigan** versiyasi.

Bu ajratish ham AVTOMATIK (dataflow taint'dan): handler serverga tegadimi-yo'qmi — graf biladi.

**Dalil:** Flux handler'lari ko'pincha backend'ga yetadi (db.put, cart.add → db). Bu RSC/LiveView falsafasi (logika serverda, secret oqmaydi). Lekin sof-UI o'zaro ta'sirlar (toggle, filter, counter) network kerak qilmasligi kerak → ularni client'da qoldirish. Solid signals fine-grained DOM update'i client-only island ichida ishlatiladi.

### Client runtime tarkibi (`ui_client.js`, `include_str!`, ~8KB)

- Event delegation (Qwikloader naqshi): bitta global listener, `data-fx-on="island:event"` o'qiydi.
- WS ulanish (server-driven handler'lar uchun) — mavjud ws_mod bilan bitta portda.
- DOM-patch applier (diff qabul qiladi).
- Mikro signal tizimi (client-only island'lar uchun): `data-fx-bind` → DOM node, signal o'zgarsa surgical update (Solid uslub, virtual-DOM yo'q).

---

## 3. SERVER/CLIENT CHEGARASI (xavfsizlik, wire-format)

### 3.1 Server qism hech qachon client'ga ketmaydi

- `source`, `db.*`, `ai.*`, `http.* (server fetch)`, `reg.*`, env — **graf "server" tugun**. Bu kod **client bundle'ga umuman kiritilmaydi** (tree-shaking: client recipe faqat `island` deb belgilangan handler/expression'lardan boshlanadi, entry-point sifatida).
- RSC naqshi: server view tanasi **bajariladi** (SSR), faqat **natija** (HTML + serializatsiyalangan island state) chiqadi. Manba kod (SQL, prompt, key) hech qachon emas.
- Server-driven handler'lar uchun esa **handler tanasi serverda qoladi** — client faqat "island 1, event click, payload {product_id}" yuboradi. Bu eng kuchli kafolat: handler kodi browserda mavjud emas.

### 3.2 Wire-format (RSC Flight naqshining soddalashtirilgani)

Ikki yo'nalish:

**SSR → browser (dastlabki yuklash):**
```html
<div data-fx-island="1" data-fx-state='{"filter":"","items":[{...DTO...}]}'>
  ...SSR HTML...
</div>
<script>window.__fx = {ws:"/_fx",islands:{1:{handlers:["filter_input"],mode:"server"}}}</script>
```
- `data-fx-state` — faqat O'SHA island uchun kerakli serializatsiyalangan state (Marko field-level naqshi: faqat ishlatilgan property'lar, butun source emas).
- `mode:"server"` yoki `mode:"client"` — analyzer qarori.

**Browser → server (server-driven event):**
```json
{"island":1,"event":"click","handler":"cart_add","args":{"id":42}}
```
Server o'sha island'ni qayta render → diff qaytaradi:
```json
{"island":1,"patch":[{"sel":".cart-count","text":"3"}]}
```

Serializatsiya Flux `Value` ustida (json_mod allaqachon bor: Map/List/Int/Str → JSON; Fn/Native serializatsiya QILINMAYDI → agar island state'da Fn bo'lsa compile xato).

---

## 4. ARXITEKTURA + RUNTIME O'ZGARISHLARI (Rust)

### 4.1 Yangi bosqich: ANALYZER (kompilyatsiya oldidan)

Hozir pipeline: `token→lexer→ast→parser→interp`. Yangi:
```
token→lexer→ast→parser → [ANALYZER] → interp(SSR) + client-codegen
```

**Yangi fayl: `runtime/src/ui_analyze.rs`** — sof AST→AST tahlil (interp'siz):
- `analyze_view(view: &FnValue) -> ViewPlan`
- `ViewPlan { islands: Vec<IslandPlan>, server_only: HashSet<BindId>, static_subtrees: ... }`
- Reaktivlik grafi + taint (1.2, 1.3).

Bu **kompilyatsiya bosqichi**, har request emas — bir marta startup'da (yoki `flux build`'da). Natija `Interp.view_plans: Arc<HashMap<ViewName, ViewPlan>>`.

### 4.2 SSR — hot-path interpreterdan chiqadi (yarim-kompilyatsiya)

Hozir `node_to_html` har request interp qiladi. O'zgarish: analyzer **statik subtree'larni bir marta** render qilib **template cache** (string bo'laklar + "teshik"lar). Marko "compiled template + walks" naqshi:
```
SSR(request) = static_prefix + render_dynamic(hole_1) + static_mid + ...
```
Statik qismlar string konstanta (interp ishtirok etmaydi). Faqat dinamik teshiklar (source.data, server state) interp bilan to'ldiriladi. Bu interp'ni hot-path'dan asosan chiqaradi.

### 4.3 Client codegen

`ui_client_gen.rs` — `IslandPlan` → JS recipe (client-only handler'lar uchun). Bu **mikro-transpiler** faqat island handler tanasi uchun (butun til EMAS — masalan `count <- count+1`, `open <- !open`). Server-driven handler'lar uchun codegen YO'Q (faqat marker). Bu transpiler hajmini minimal saqlaydi.

### 4.4 `attrs_html` integratsiyasi

Hozir ui_mod.rs:479 `on:`/`bind:` ni TASHLAYDI. O'zgarish: island elementga `data-fx-island`, `data-fx-on`, `data-fx-bind` marker qo'shiladi (analyzer bergan `island_id` bilan).

### 4.5 Build pipeline

- `flux run app.fx` — dev: analyzer + SSR + client JS inline (hot reload).
- **`flux build app.fx`** (YANGI) — production: (1) analyzer, (2) statik sahifalarni pre-render → `.html` (CDN), (3) client JS bundle → `_fx/app.[hash].js`, (4) server binarь (interp + dinamik routes). 
- `ui.serve [app] port {mode::prod}` — prod rejimi: statik template cache, gzip, ETag, statik fayllar `Cache-Control: immutable`.

---

## 5. PERFORMANCE (katta trafik)

1. **Statik qism → CDN/edge cache.** `flux build` sof-statik sahifa/fragmentlarni `.html` qiladi → 0 JS, `Cache-Control: public, immutable`. Bu trafik ko'pini interpreterdan butunlay chetlatadi.
2. **Template cache (interp hot-path'dan chiqadi).** 4.2 — statik string bo'laklar + faqat dinamik teshik. SSR ≈ string concat + bir nechta interp eval, butun view qayta-interp emas.
3. **Minimal client JS.** Default server-driven → island'da faqat ~8KB universal runtime + kichik recipe. Astro/Marko "faqat interaktiv qismga JS" — Flux'da avtomatik. Statik sahifa = 0 JS.
4. **Server-driven diff** — event'da butun sahifa emas, faqat island diff (kichik WS payload). LiveView naqshi, katta trafik uchun WS bitta ulanish.
5. **Per-island lazy** — `source live` bo'lmagan island'lar `client:visible` ekvivalenti bilan kech faollashadi (IntersectionObserver, Astro naqshi) — analyzer "below-fold" heuristikasi emas, lekin `source`siz island'lar idle'da resume bo'ladi.
6. **DTO serializatsiya minimal** — Marko field-level: island state'ga faqat ishlatilgan property'lar (graf biladi qaysi field o'qiladi).

---

## 6. BOSQICHMA-BOSQICH REJA

Mavjud 4-bosqich (signals/reaktivlik) **o'rniga emas, qayta tartiblanadi**: analyzer 4-bosqich oldiga qo'yiladi, chunki signals'siz ham analyzer statik/island ajratishni bera oladi (avval SSR-only island = server-driven).

| PR | Mazmun | Xavf |
|----|--------|------|
| **PR-4a** | `ui_analyze.rs`: reaktivlik grafi + taint. `<-`/`on:`/`act`/`source` izini topish, `island_id` ni `{__node}`ga qo'shish. Interp'ga ta'sir yo'q (faqat tahlil). Test: AST→ViewPlan snapshot. | O'rta — graf to'g'riligi |
| **PR-4b** | `attrs_html` island markerlari (`data-fx-*`) + `window.__fx` state inject. SSR HTML island chegaralarini ko'rsatadi. Hali interaktivlik yo'q. | Past |
| **PR-5** | Server-driven: `ui_client.js` (event delegation+WS+DOM-patch), WS event→island re-render→diff. **Eng katta xavf** (bir portda HTTP+WS upgrade, island re-render izolyatsiyasi, state restore serverda). | **YUQORI** |
| **PR-6** | Client-only resumability: serverga tegmaydigan handler'lar (`count<-count+1`, toggle) → mikro JS recipe (`ui_client_gen.rs`) + signal DOM update. | Yuqori (mikro-transpiler doirasi) |
| **PR-7** | `source` → server-only RPC + `live`/`ui.push` realtime (WS broadcast → island reload). | O'rta (PR-5 ustida) |
| **PR-8** | `flux build` prod: statik pre-render, JS bundle, template cache, CDN headerlar. | O'rta |
| **PR-9** | `ui.*` batareya (table/form/modal/...) — har biri o'z island/statik profilini analyzer'ga e'lon qiladi. | Past (oldingi ustida) |

### Eng katta texnik xavflar
1. **Island state'ni serverda restore qilish** (server-driven re-render uchun source/state'ni qayta hisoblash kerak — yoki state'ni WS sessiyada saqlash). LiveView buni server-state bilan hal qiladi → memory bosim katta trafikda. **Yumshatish:** stateless re-render (state client'dan event bilan keladi) yoki qisqa-umrli session.
2. **Graf to'g'riligi** — island chegarasini juda keng (ko'p JS) yoki juda tor (buzilgan reaktivlik) olish. Stress-test driven (memory metodologiyasi): docs'siz modellarga yozdirib chegara xatolarini topish.
3. **Mikro-transpiler doirasi** (PR-6) — qaysi Flux qism JS'ga compile bo'ladi? Doirani QAT'IY cheklash (faqat `<-` arifmetika/toggle), qolgani server-driven. Doira kengaysa transpiler = to'liq Flux→JS (RAD qilingan).
4. **Bir portda HTTP+WS** (memory'da ham eng katta xavf deb belgilangan) — hyper upgrade, ws_mod bilan event-loop integratsiya.

---

## 7. OCHIQ QARORLAR (foydalanuvchi hal qilishi kerak)

1. **Server-driven vs client-only default'i** — men taklif: handler server'ga tegsa server-driven, tegmasa client. Lekin "hammasi server-driven (LiveView)" soddaroq (PR-6'ni keyinga surish, transpiler xavfi yo'q). Tezroq MVP uchun **hammasini server-driven qilib boshlash**ni tavsiya qilaman, client-only optimizatsiyasini keyin. Tasdiqlaysizmi?

2. **State serverda yoki client'da** — server-driven re-render uchun: (a) state WS-session'da serverda (LiveView, RAM bosim), yoki (b) stateless — client har event'da state'ni yuboradi (RAM yo'q, payload kattaroq, xavfsizlik: client state'ni soxtalashtirishi mumkin). Katta trafik uchun (b) yaxshiroq deb o'ylayman.

3. **`flux build` zarurmi yoki `ui.serve {mode::prod}` yetarlimi** — alohida build artifact (CDN deploy) vs hammasi bitta serverdan. Statik CDN kerak bo'lsa `flux build` shart.

4. **WASM variant (b)ni butunlay rad etamizmi** — agar kelajakda murakkab client-only logika kerak bo'lsa WASM zaxira variant bo'la oladi. Hozir rad, lekin yopiq emas.

5. **Analyzer xatosi qattiqmi yumshoqmi** — island server-only qiymat ushlasa: compile xato (qattiq, xavfsiz) yoki ogohlantirib server-driven'ga tushirish (yumshoq, qulay)? Xavfsizlik uchun **qattiq** tavsiya.

---

### Mavjud kod bilan birlashish xulosasi
- **Yangi Value varianti YO'Q** — `{__node}`ga `island_id` maydoni (idioma saqlanadi).
- **Yangi fayllar:** `ui_analyze.rs` (graf+taint), `ui_client.js` (include_str!), `ui_client_gen.rs` (PR-6).
- **O'zgaradi:** `ui_mod.rs` (`attrs_html` markerlari, `node_to_html` template-cache), `interp.rs` (`view_plans` state, analyzer chaqiruvi startup'da), `serve_mod.rs`/`ws_mod.rs` (server-driven WS).
- **Saqlanadigan invariantlar:** Send+Sync, `=`/`<-` semantika, Parent::Root optimizatsiyasi, http.serve freeze_globals.

Tegishli fayllar:
- `/Users/whoami/Documents/files/draft-project/flux-lang-frontend/runtime/src/ui_mod.rs` (SSR, `attrs_html`:470 — `on:`/`bind:` integratsiya nuqtasi)
- `/Users/whoami/Documents/files/draft-project/flux-lang-frontend/runtime/src/interp.rs` (`exec_view_body`:588, `collect_view_nodes`:604, `theme`/`pages`/`routes` state)
- `/Users/whoami/Documents/files/draft-project/flux-lang-frontend/runtime/src/value.rs` (`FnValue.is_view`:40)
- `/Users/whoami/Documents/files/draft-project/flux-lang-frontend/docs/FRONTEND-IMPLEMENTATION-PLAN.md` (mavjud reja — bu hujjat 4-6 bosqichni yangilaydi)