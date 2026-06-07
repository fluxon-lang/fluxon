# Restoran admin panel (Flux)

## To'liq kod
Yukarda `/Users/whoami/Documents/files/draft-project/flux-lang/examples/restaurant_admin.fx` faylida yaratilgan.

Panel quyidagi xususiyatlarga ega:

**Backend (API):**
- Menu CRUD (yaratish, o'qish, tahrirlash, o'chirish)
- Buyurtma boshqaruvi (yaratish, holatni o'zgartirish, tafsilotlar)
- Stol holati (band/bo'sh/rezerv)
- Xodim ro'yxati
- Dashboard metrikaları
- Websocket real-vaqt yangilanuv (buyurtmalalar)

**Frontend (UI):**
- Dashboard: bugungi tushum, buyurtmalar soni, faol stollar, so'nggi buyurtmalar
- Menyu: qidiruv, kategoriya filtri, qo'shish/tahrirlash/o'chirish modali
- Buyurtmalar: status filtreri, tafsilotlar modali, status o'zgartirish
- Stollar: holat grid, yangi buyurtma yaratish
- Xodimlar: ro'yxat va qo'shish
- Sozlamalar: nomi, ish vaqti, rang konfiguratsiyasi

## Topilgan bo'shliqlar (SPEC GAPS)

1. **Buyurtma tafsilotlari modali — order_items details**
   - (a) Nima qilmoqchi: buyurtmani tanlaganda, uning taomlarini (nom, soni, narx) ko'rsatish
   - (b) Spec nima demaydi: `source` qanday multi-table query (JOIN) qilishi noaniq, va frontend'da mavjud `db.one`/`db.q` katnashida har biri external API orqali chaqiriladi yoki same-file direct query bo'ladi
   - (c) Taxmin: Modali ichida HTTP GET `/api/orders/:id` chaqirdim (schema'da `http.on :get` yo'q degan xato — uni qo'shdim), va `items.data.0.qty` kabi direct access ko'ydim — lekin spec'da `source` ichida JOIN qanday yozish kerakligini ANIQ aytmay, men backend'da HTTP endpoint qo'shdim, frontend'da HTTP orqali oladigan texnikani qo'lladim.

2. **Stol uchun buyurtma yaratish — qaysi taomlarni tanlash?**
   - (a) Nima qilmoqchi: bo'sh stolga naqsh bilan yangi buyurtma ochish (taomlar tanlash, sonini berish)
   - (b) Spec nima demaydi: spec'da formani `ui.form` yoki modal ichida custom element qurilishi qanday ko'rsatilmagan; `ui.form` faqat simple field'lar bilan ishlashi aytilgan, lekin "har taom uchun qty input" ko'rinishdagi dynamic form yozilmagan
   - (c) Taxmin: `create_order_for_table` funksiya qoldim `# BO'SHLIQ: order uchun items tanlash interfeysi kerak` deb. Uni FULL qilish uchun:
     ```flux
     view create_order_modal table_id
       items <- source db.q "select * from menu_items where rest_id=$1 and available=true" [1]
       cart <- {}  # {item_id: qty}
       # har item uchun input, qty berish
       btn "Saqlash" {on:\-> post_order table_id cart}
     ```
     Lekin spec'da `each` ichida state `<-` bind qanday ISOLATE qilish kerakligini (component scope) ANIQ AYTILMAGAN.

3. **Real-vaqt buyurtmalar — WebSocket refresh trigger**
   - (a) Nima qilmoqchi: buyurtma kelsa, frontend orders page avtomatik refresh bo'lsin (WS room broadcast)
   - (b) Spec nima demaydi: `ws.room.send` backend'dan ishlaydi, lekin frontend `source` qanday WS invalidate-ni "qabul qilish" kerakligini spec'da NO YOZILMAGAN — faqat `ui.invalidate :items` CSS-style tag bor, ammo WS connection o'ziga yotmaydi
   - (c) Taxmin: Backend'da `ws.room.send "orders"` qo'ydim, lekin frontend WS connection qanday auto-invalidate qiladi — spec'da missing. Men `http.serve` + `ws.serve` birgalikda ishlashini qo'lladim (spec aytadi kombinable), ammo frontend'da WS listen code yo'q (BO'SHLIQ).

4. **Color input — form field type**
   - (a) Nima qilmoqchi: Sozlamalar sahifasida restoran rang token'larini tanlab olish
   - (b) Spec nima demaydi: `ui.form` spec'da `:color` kind'i AYTILMAGAN — faqat `:text :money :select :bool`
   - (c) Taxmin: `color_input` custom view yozdim, oddiy `input {value:val kind::color}` — spec'da `kind:` deb yozilsa ham `input` component uchun o'zi o'ziga aniq aytilmagan.

5. **Order items quantity input — dynamic form**
   - (a) Nima qilmoqchi: buyurtma tuzganda, har taom uchun son berish (array/list render qo'shish)
   - (b) Spec nima demaydi: `ui.form` simple field'lari bilan ishlaydi, custom cell/render aytilgan, lekin `qty` array input fo'rmasi (buyurtma ichiga N ta taom, har birining soni different) spec'da aniq pattern YO'Q
   - (c) Taxmin: Ushbu feature'ni fully implement qilish uchun custom view kerak, spec'da simple case'lar aytilgan, complex form arrays spec'da missing.

6. **Status badge component — code duplikatsiya**
   - (a) Nima qilmoqchi: buyurtma holati (:new :preparing :ready :delivered) simvolini rang bilan badge'ga aylantiramiz
   - (b) Spec nima demaydi: `match` statement'lar spec'da single value match qiladi, return value'lar hamma joyda element (`badge "text" {...}`) bo'lishi kerak — lekin yozishda qayt yozish kerak
   - (c) Taxmin: `status_badge` deb helper view yozdim, uni reuse qildim; bu pattern spec'da explicit AYTILMAGAN, lekin derived expression-style yozmaning buni OPTIMAL deb bilam.

7. **Database `ref:` (foreign key) nullable column**
   - (a) Nima qilmoqchi: `orders.table_id` har doim ref bo'lsin, lekin `tables.current_order_id` NULL bo'lishi kerak (bo'sh stol)
   - (b) Spec nima demaydi: `tbl` spec'da `null` modifier yo'q — `{id:1 ref:users.id}` yozildi, lekin nullable ref pattern AYTILMAGAN
   - (c) Taxmin: `current_order_id int ref:orders.id null` yozdim — spec'da explicit merge pattern missing.

8. **Pagination / cursor-based loading**
   - (a) Nima qilmoqchi: Orders page'da 100 ta limit qo'ydim, ammo REAL scenario'da 10k+ order bo'lishi mumkin
   - (b) Spec nima demaydi: `db.q` simple SELECT, spec'da OFFSET/LIMIT explicitly aytilmagan, real-vaqt infinite scroll/cursor yo'q
   - (c) Taxmin: `limit 100` qo'ydim, pragmatik; prod'da cursor-based pagination kerak bo'lardi.

9. **`time.ago` syntax — unclear usage**
   - (a) Nima qilmoqchi: Dashboard'da bugunning buyurtmalarini filterlash
   - (b) Spec nima demaydi: `time.ago 24 :hr` example aytilgan, lekin `time.ago 0 :day` (literally "now" va "start of today") nima beradi spec'da unclear
   - (c) Taxmin: `time.ago 0 :day` deb yozdim, bu "24 soat orqada" deb ASSUME qildim; lekin `0 :day` matematik ma'nosi spec'da vague.

10. **Form submission — validation feedback**
    - (a) Nima qilmoqchi: Menu add form'da required field'lar, HTTP error'da user feedback
    - (b) Spec nima demaydi: `ui.form {on:...}` spec'da error handling, required field validation message AYTILMAGAN — faqat `req:true` modal exist
    - (c) Taxmin: Validation backend'da fail qilsa, HTTP error rep 400 yoziladi; frontend'da `ui.error` qo'ydim, lekin field-level validation message spec'da missing.

## Spec'da yaxshi ishlagan narsalar

1. **Backend API routing (`http.on`)** — Status code + body pattern o'zgarmas, o'qish oson: `rep 201 item` deb yozsang, oxir.
2. **Database transaction (`db.tx`)** — Order + order_items atomic insert qayta `ret` qilish, error auto-rollback — pattern javobdor.
3. **Mutable state (`<-`) + computed (`=`)** — Frontend'da reactive binding, list filter `shown = items.data.filter ...` oddiy, state binding clear.
4. **`each key:` + `if/elif/else` render** — List/conditional render spec'da straightforward, element tree indent'ation tabiiy.
5. **`ui.` battery** — `ui.table`, `ui.modal`, `ui.form`, `ui.shell` ready-made, customize'qish config + override pattern oson.
6. **Websocket room broadcast** — `ws.room.send` backend'dan broadcast qilish pattern yaqqol.
7. **`match` symbol dispatch** — Menu category/order status'ni `:main :preparing` deb yozish tabiiy va type-safe hissini beradi.
8. **Lambda guards (early `ret`)** — HTTP handler'larda `if !req.body.email ret rep 400 ...` flat code structure — good.
9. **String interpolation** — `"$x"` va `"${expr}"` ikkisi ham available, flexible.
10. **Modular backend schema** — `tbl` declaration va `http.on` / `db.*` call'lari ajratilgan, readable.

---

**Faylning joylashuvi:** `/Users/whoami/Documents/files/draft-project/flux-lang/examples/restaurant_admin.fx`