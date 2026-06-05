# Flux — Dasturlash Tili (Inson uchun to'liq qo'llanma)

> **Flux nima?** Flux — AI agentlar yaxshi yozadigan, backend tizimlari uchun
> mo'ljallangan dasturlash tili. Falsafasi: *"Til AI'ga moslashadi, AI tilga
> emas."* Har bir ishni qilishning **bitta** aniq yo'li bor, sintaksis kam
> token ishlatadi, va eng kerakli narsalar (HTTP server, ma'lumotlar bazasi,
> AI/LLM chaqiruvi, cron, navbat) — tilning **ichida**, hech qanday paket
> o'rnatmasdan.

Flux fayllari `.fx` kengaytmasi bilan saqlanadi.

Bu hujjat — to'liq, batafsil **inson** qo'llanmasi. Agar siz AI agentga Flux'ni
o'rgatmoqchi bo'lsangiz, qisqaroq `flux-agent.md` faylidan foydalaning.

---

## 0. Asosiy g'oyalar (avval shularni o'qing)

Flux'ni boshqa tillardan ajratib turadigan 5 ta tamoyil:

1. **Bir ish = bir yo'l (canonical form).** Boshqa tillarda bir narsani 5 xil
   yozish mumkin (`while`, `for`, `do-while`...). Flux'da takrorlash uchun
   **faqat `each`** bor. Ekranga chiqarish uchun **faqat bitta** usul. Bu
   qoidaning sababi: AI har safar "qaysi usulni tanlay?" deb o'ylamaydi —
   tanlov yo'q, demak xato ham kam.

2. **Kam token, lekin tushunarli.** Sintaksis imkon qadar qisqa, lekin
   *shifrli emas*. Kalit so'zlar to'liq yoziladi (`each`, `match`, `else`) —
   chunki Flux'ni birinchi marta ko'rgan odam yoki AI ularni darhol tushunishi
   kerak.

3. **Batteries included (hammasi ichida).** `http`, `db`, `ai`, `json`, `cron`,
   `queue` — bularning hammasi tilning standart kutubxonasida. Hech qanday
   `npm install`, `composer require` yo'q. Faqat `use http` deysiz va
   ishlatasiz.

4. **AI — birinchi darajali primitiv.** Boshqa tillarda LLM chaqirish uchun
   SDK o'rnatib, kalit sozlab, JSON parse qilasiz. Flux'da `ai.json` bitta
   qatorda matnni strukturali ma'lumotga aylantiradi va ishonch ballini
   qaytaradi.

5. **Ahamiyatli bo'shliq (indentation).** Bloklar `{}` qavslar bilan emas,
   **chekinish (2 bo'shliq)** bilan ajratiladi — xuddi Python kabi. Bu ortiqcha
   belgilarni olib tashlaydi.

---

## 1. Leksik asoslar

### Izohlar (comments)
Faqat bitta turdagi izoh bor — `#` belgisidan qator oxirigacha:
```flux
# Bu izoh
x = 5   # Bu ham izoh
```
Flux'da `//` yoki `/* */` **yo'q**. Bitta usul — `#`.

### Statementlar
Har bir statement **yangi qatorda** tugaydi. Nuqtali vergul (`;`) **kerak emas**
va ishlatilmaydi:
```flux
x = 5
y = 10
```

### Bloklar
Blok `{}` bilan emas, **chekinish** bilan ochiladi. Har daraja — **2 bo'shliq**.
Chekinish kamayganda blok tugaydi:
```flux
if x > 0
  log "musbat"
  log "ikkinchi qator ham blok ichida"
log "blokdan tashqari"
```

---

## 2. Qiymatlar va tiplar

Flux'da quyidagi asosiy tiplar bor:

| Yozuv | Tip | Izoh |
|-------|-----|------|
| `42` | `int` | Butun son |
| `3.14` | `flt` | Kasrli son (float) |
| `"salom"` | `str` | Matn (string) |
| `true` / `false` | `bool` | Mantiqiy qiymat |
| `nil` | `nil` | "Hech narsa" / bo'shliq |
| `[1 2 3]` | `list` | Ro'yxat — elementlar **bo'shliq** bilan ajraladi |
| `{a:1 b:2}` | `map` | Kalit-qiymat juftliklari — **bo'shliq** bilan ajraladi |
| `:ok` | `sym` | Belgi (symbol) — enum/teg uchun |

### Muhim nozikliklar

**Ro'yxat va map'da vergul YO'Q.** Elementlar bo'shliq bilan ajraladi. Bu
ataylab — vergullar token isrof qiladi:
```flux
nums = [1 2 3 4]
user = {name:"Aziza" age:30 active:true}
```

**Matn ichida o'zgaruvchi qo'yish (interpolation).** `"${...}"` orqali ifodani
matn ichiga joylashtirasiz:
```flux
name = "Aziza"
log "Salom ${name}!"              # → Salom Aziza!
log "Jami: ${price * qty} so'm"   # ifoda ham bo'ladi
```
Oddiy o'zgaruvchi uchun qisqartirib `"$name"` ham yozsa bo'ladi, lekin ifoda
uchun `${...}` shart.

**Belgilar (symbols) — enum o'rniga.** Holatlarni ifodalash uchun matn
o'rniga belgi ishlating. `:new`, `:confirmed` — bu `"new"` matnidan token
arzonroq va aniqroq:
```flux
status = :confirmed
dir = :in
```

**Truthiness (rost/yolg'on qiymati).** `nil` va `false` — yolg'on. Qolgan
hamma narsa (shu jumladan `0`, `""`, bo'sh ro'yxat) — **rost**. Bu sodda
qoida ataylab: faqat ikki narsa yolg'on.

---

## 3. O'zgaruvchilar (bindings)

Flux'da **ikki** xil bog'lash bor, va ular **boshqa ish** qiladi (shuning
uchun ikkitasi bo'lishi canonical qoidaga zid emas):

### `=` — o'zgarmas (immutable)
Bir marta qiymat beriladi, keyin o'zgartirib bo'lmaydi:
```flux
x = 10
name = "Aziza"
```
Bu **standart** holat. Ko'pchilik qiymatlar o'zgarmaydi.

### `<-` — o'zgaruvchan (mutable)
Qiymatini keyin o'zgartirish mumkin bo'lgan o'zgaruvchi. Qayta tayinlash ham
`<-` bilan:
```flux
total <- 0.0
total <- total + 5.0     # qayta tayinlash
```

> **Qoida:** agar qiymat o'zgarmasa — `=` ishlating. Faqat haqiqatan
> o'zgaradigan narsalar uchun `<-`. Bu kod o'qishini osonlashtiradi: `<-`
> ko'rsangiz, "bu o'zgaradi" deb bilasiz.

---

## 4. Operatorlar

### Arifmetik
```flux
+   -   *   /   %        # qo'shish, ayirish, ko'paytirish, bo'lish, qoldiq
```
**`+` string'larni ham birlashtiradi.** Operandlar son bo'lsa — qo'shadi,
matn bo'lsa — ulaydi:
```flux
1 + 2          # → 3
"sal" + "om"   # → "salom"
```
Tip o'zi farqni belgilaydi — bitta operator, ikki tabiiy ish.

### Solishtirish
```flux
==  !=  <  <=  >  >=
```

### Mantiqiy
```flux
&    # va (and)
|    # yoki (or)
!    # emas (not) — qiymat oldida: !x
```

### Maxsus operatorlar

**`??` — null-coalesce.** Chap tomon `nil` bo'lsa, o'ng tomonni beradi:
```flux
port = env.PORT ?? "8080"     # PORT yo'q bo'lsa, "8080"
name = user.name ?? "mehmon"
```

**`.` — a'zoga murojaat / indeks.** Map kaliti, ro'yxat indeksi, uzunlik:
```flux
user.name        # map kaliti
list.0           # ro'yxatning birinchi elementi
list.len         # uzunlik
m[key]           # dinamik kalit (o'zgaruvchi orqali)
```

**`..` — diapazon (range).** Ikkala chet ham kiradi:
```flux
1..5             # [1 2 3 4 5]
```

**`|>` — quvur (pipe).** Qiymatni funksiyaga uzatadi, ichма-ich yozuvni
yo'qotadi:
```flux
result = data |> clean |> format
# bu g'a teng: format(clean(data))
```

---

---

## 5. Funksiyalar

Funksiya `fn` bilan e'lon qilinadi. Argumentlar **bo'shliq** bilan ajraladi
(vergul yo'q):

```flux
fn add a b
  ret a + b
```

### Bir qatorli funksiya
Agar tana bitta ifoda bo'lsa, `->` bilan bir qatorda yozsa bo'ladi:
```flux
fn double x -> x * 2
```

### Qaytarish (return)
Ikki usul, lekin ular bir xil natija beradi:
- `ret x` — aniq qaytarish
- **Oxirgi ifoda** — avtomat qaytariladi (`ret`siz)

```flux
fn add a b
  a + b            # oxirgi ifoda — avtomat qaytadi

fn check x
  if x > 0
    ret "musbat"   # erta qaytish uchun ret kerak
  "nomusbat"       # oxirgi ifoda
```

> **Eslatma:** `ret` faqat **erta** (o'rtada) qaytish kerak bo'lganda
> ishlatiladi. Oxirida — shunchaki ifodani yozing.

**`ret` lambda ichida ham ishlaydi.** Bu — HTTP handlerlarda eng muhim.
Validatsiya uchun chuqur `if/elif/else` piramidasi o'rniga **guard-clause**
(erta chiqish) yozing — kod tekis qoladi:
```flux
# ❌ Chuqur nesting (yomon):       ✅ Guard-clause (yaxshi):
http.on :post "/x" \req ->        http.on :post "/x" \req ->
  if req.body.email                 if !req.body.email
    if req.body.body                  ret rep 400 {error:"email kerak"}
      rep 201 (...)                 if !req.body.body
    else                              ret rep 400 {error:"body kerak"}
      rep 400 {...}                 rep 201 (db.ins "t" {...})
  else
    rep 400 {...}
```

### Funksiyani chaqirish
Argumentlar bo'shliq bilan, qavssiz:
```flux
add 2 3            # → 5
double 4           # → 8
```
Qavs faqat **guruhlash** uchun kerak (funksiya natijasini boshqasiga uzatish):
```flux
double (add 2 3)   # avval add 2 3 = 5, keyin double 5 = 10
```

### Lambda (anonim funksiya)
`\` belgisi bilan, inline ishlatiladi:
```flux
\x -> x * 2
each_map nums \x -> x * 2    # har elementni 2 ga ko'paytirish
```

---

## 6. Boshqaruv oqimi (control flow)

### Shartlar: `if` / `elif` / `else`
```flux
if x > 0
  log "musbat"
elif x == 0
  log "nol"
else
  log "manfiy"
```
Kalit so'zlar **to'liq** yoziladi (`elif`, `else`) — bir qarashda tushunarli
bo'lishi uchun.

### Takrorlash: `each` (yagona loop)
Flux'da **faqat bitta** loop bor — `each`. U ro'yxat, diapazon yoki map
ustidan yuradi. `while`, `for`, `do-while` **yo'q**:

```flux
each item in list           # ro'yxat elementlari
  log item

each i in 1..5              # diapazon: 1,2,3,4,5
  log i

each k, v in map            # map: kalit va qiymat
  log "$k = $v"
```

Loop ichida:
- `skip` — keyingi iteratsiyaga o'tish (boshqa tillarda `continue`)
- `stop` — loopdan chiqish (boshqa tillarda `break`)

```flux
each n in nums
  if n < 0
    skip          # manfiylarni o'tkazib yuborish
  if n > 100
    stop          # 100 dan oshsa to'xtash
  log n
```

> **"While qani?"** Agar shart bo'yicha takrorlash kerak bo'lsa: diapazon
> ustidan yuring (`each i in 1..n`) yoki rekursiya ishlating. Bitta loop —
> bitta yo'l.

### Qiymat bo'yicha tanlash: `match`
Bir qiymatni bir nechta variant bilan solishtirish. Asosan belgilar (symbols)
uchun:
```flux
match status
  :new -> log "yangi"
  :confirmed -> log "tasdiqlangan"
  :cancelled -> log "bekor"
  _ -> log "noma'lum"        # _ = standart (default)
```
`match` va `if` — **boshqa ish** qiladi: `if` mantiqiy shart uchun, `match`
bir qiymatni variantlarga taqsimlash uchun. Shuning uchun ikkalasi ham bor.

> **⚠️ Muhim:** `match` faqat **qiymat** (symbol yoki son) bilan ishlaydi.
> Mantiqiy shart (`conf > 0.85` kabi) uchun **har doim `if/elif/else`**
> ishlating. `match true` deb yozib, ostiga shartlar qo'yish — **xato**,
> bunday qilmang:
> ```flux
> # NOTO'G'RI:
> match true
>   conf > 0.85 -> ...
> # TO'G'RI:
> if conf > 0.85
>   ...
> ```

---

## 7. Xatolar (error handling)

Flux'da funksiya muvaffaqiyat (`ok`) yoki xato (`err`) qaytarishi mumkin. Xato
bilan ishlashning **bitta** asosiy usuli — `!` operatori, va `nil` uchun `??`.

### `!` — xatoni avtomat yuqoriga uzatish
Funksiya nomidan keyin `!` qo'ysangiz: agar u xato qaytarsa, xato **avtomat**
chaqiruvchiga uzatiladi (siz qo'lda tekshirmaysiz). Agar muvaffaqiyatli bo'lsa,
natijani oladi:
```flux
fn process id
  user = db.one "select * from users where id=$1" [id]!
  # agar db.one xato qaytarsa, process ham shu xatoni qaytaradi —
  # keyingi qator umuman ishlamaydi
  log user.name
```
Bu `if err != nil { return err }` ko'p qatorli naqshni **bitta belgiga**
qisqartiradi.

### `??` — nil bo'lsa muqobil
Agar qiymat `nil` bo'lsa (xato emas, shunchaki bo'sh), `??` bilan muqobil
bering:
```flux
name = user.name ?? "mehmon"
each it in items
  p = db.one "...narx..." [it.product]
  p ?? (ask_owner "Narx?"; skip)    # p nil bo'lsa — so'ra va o'tkaz
  log p.price
```

### `fail` — xato chiqarish
O'z kodingizdan xato ko'tarish:
```flux
if qty < 1
  fail "miqdor noto'g'ri"
```

**`fail` status kodi bilan — kutilgan xatolar uchun.** HTTP handler ichida
`fail` ga status kodini bersangiz, u **avtomat** o'sha statusli javobga
aylanadi. Bu — `try/catch` o'rnini bosadi: kutilgan xatoda chuqur nesting
o'rniga shunchaki `fail` qiling:
```flux
http.on :post "/transfer" \req ->
  acc = db.one "select * from accounts where id=$1" [req.body.from]
  if acc.balance < req.body.amount
    fail 422 "balans yetarli emas"     # → mijozga 422 {error:"balans yetarli emas"}
  # ... asosiy yo'l, nesting yo'q
```
- `fail 4xx "xabar"` — **kutilgan** (biznes) xato → o'sha statusli JSON javob.
- `fail "xabar"` (status'siz) — **kutilmagan** xato → 500.

> **Canonical:** `!` = xatoni uzat, `??` = nil'ni almashtir, `fail` = xato
> chiqar (status bilan yoki status'siz). Har belgi bitta ma'no. `try/catch`
> **yo'q** — `fail`+status uning o'rnini bosadi, kod tekis qoladi.

---

## 8. Modullar (import / export)

### `use` — modul chaqirish
Standart kutubxona yoki o'z faylingizni chaqirasiz. O'rnatish (`install`) yo'q:
```flux
use http db ai json        # standart batteries — bo'shliq bilan ko'p modul
use ./tools                # o'z faylingiz → tools.funksiya
```
Chaqirilgandan keyin nomlar modul ostida: `db.one`, `http.serve`,
`tools.create_order`.

**`as` — qayta nomlash (alias).** Agar o'z faylingiz batareya nomi bilan bir
xil bo'lsa (masalan `ai.flux` fayl va `ai` batareyasi), to'qnashuv bo'ladi.
`as` bilan o'z modulingizni qayta nomlang:
```flux
use ai                     # batareya
use ./ai as helper         # o'z faylingiz → helper.classify (to'qnashmaydi)
```
**Qoida:** o'z fayllaringizga batareya nomini (`ai db http cron`...) bermang,
yoki bersangiz `as` bilan qayta nomlang.

### `exp` — eksport qilish
Faylingizdagi funksiya yoki qiymatni boshqa fayllar uchun ochish:
```flux
exp fn create_order items customer
  ...
exp price_limit = 1000
```
Faqat `exp` bilan belgilangan narsalar tashqaridan ko'rinadi.

---

## 9. Batteries — standart kutubxona

Bu — Flux'ning eng kuchli qismi. Eng kerakli narsalarning **hammasi** tilning
ichida. Hech narsa o'rnatmaysiz — faqat `use` qilasiz va ishlatasiz.

### 9.1 `http` — server va klient

**Server.** Marshrutni (route) bitta qatorda e'lon qilasiz:
```flux
use http

http.on :post "/notes" \req -> rep 201 {ok:true}
http.on :get "/notes/:id" \req -> rep 200 {id:req.params.id}
http.serve 8080
```
- `http.on :metod "/yo'l" handler` — marshrut. Metod belgi (`:get :post :put
  :patch :del`).
- Handler — lambda. Argument `req`:
  - `req.body` — JSON tanasi (avtomat map'ga aylantirilgan)
  - `req.params.id` — yo'ldagi `:id`
  - `req.query` — so'rov parametrlari (`?key=val`)
  - `req.headers` — sarlavhalar
- `rep status body` — javob. `body` map bo'lsa, **avtomat JSON** bo'ladi.
- `http.serve port` — serverni ishga tushiradi.

**Redirect (yo'naltirish).** Maxsus fe'l yo'q — `rep` bilan 302 status va
`location` kalitini berasiz; u Location header'ga aylanadi:
```flux
http.on :get "/:code" \req ->
  link = db.one "select * from links where code=$1" [req.params.code]
  link ?? (rep 404 {error:"topilmadi"})
  rep 302 {location:link.url}
```

**Route ustunligi.** Agar ikki marshrut bir-biriga to'g'ri kelsa (`/:code` va
`/stats/:code`), **literal (aniq) yo'l avtomat ustun** bo'ladi — yozish
tartibidan qat'i nazar. `/stats/:code` har doim `/:code` dan oldin tekshiriladi.

**Klient.** Tashqi API chaqirish:
```flux
res = http.get "https://api.example.com/data"
res = http.post url {key:"val"}      # tana avtomat JSON
# res.status, res.body, res.headers (map, kalit kichik harf)
loc = res.headers.location           # yoki res.headers["content-type"]
```

Redirect (3xx) **default kuzatilmaydi** — `res.status` 30x, `res.headers.location`
o'qiladi. Avtomat kuzatish kerak bo'lsa opsiya map qo'shing:
```flux
res = http.get url {follow:true}         # 3xx → Location'ga ergashadi
res = http.get url {follow:true max:5}   # hop limiti (default 10)
# res.hops — necha marta redirect bo'lgani
```
`max`'dan oshsa xato. Opsiya oxirgi argument: `http.post url body {follow:true}`.

### 9.2 `db` — ma'lumotlar bazasi (Postgres)

Ulanish **avtomat**: `$DATABASE_URL` muhit o'zgaruvchisidan o'qiladi. Hech
qanday ulanish kodi yozmaysiz.

```flux
use db

# So'rov — natija map'lar ro'yxati
rows = db.q "select * from products where owner=$1" [owner_id]

# Bitta qator (yoki nil)
user = db.one "select * from users where id=$1" [id]

# Qo'shish — qo'shilgan qatorni qaytaradi
row = db.ins "orders" {cust:5 total:0 status::new}

# Yangilash — db.up "jadval" {o'zgartirish} {shart}
db.up "orders" {total:1500} {id:order_id}

# O'chirish — db.del "jadval" {shart}
db.del "cart_items" {id:item_id}

# UPSERT — db.put "jadval" {o'zgartirish} {kalit}
# kalit bo'yicha bor bo'lsa yangilaydi, yo'q bo'lsa qo'shadi (atomik)
db.put "agent_memory" {val:v} {agent:aid key:k}
```

> **`db.put` nega kerak?** "Bor bo'lsa yangila, yo'q bo'lsa qo'sh" naqshi
> (memory, cache, hisoblagich) uchun. Buni qo'lda `db.one` + `if` + `db.ins`
> bilan qilsa, ikki parallel so'rov ikkalasi ham "yo'q" deb ko'rib, ikki marta
> qo'shishi mumkin (race). `db.put` buni atomik qiladi.

**Tranzaksiya — `db.tx`.** Ko'p qadamli mutatsiya **atomik** bo'lishi kerak
bo'lsa (masalan checkout: buyurtma + qatorlar + stok kamaytirish), `db.tx`
blokiga o'rang. Blok ichida xato (`fail` yoki `!`) chiqsa, **hamma** o'zgarish
**qaytariladi** (rollback) — DB hech qachon yarim holatda qolmaydi:
```flux
db.tx \->
  ord = db.ins "orders" {cust:c.id total:total}
  each it in items
    db.ins "order_items" {ord:ord.id prod:it.id qty:it.qty price:it.price}
    db.up "products" {stock:it.stock - it.qty} {id:it.id}
  db.up "carts" {status::converted} {id:cart.id}
  # blok oxirigacha yetsa — commit. O'rtada fail bo'lsa — hammasi bekor.
```

`db.tx` qiymat ham qaytaradi (`ret` orqali):
```flux
ord = db.tx \->
  o = db.ins "orders" {...}
  ret o            # blok qiymati tashqariga
```

**Concurrency (parallel so'rovlar) kafolati.** `db.tx` avtomat eng kuchli
izolyatsiyada ishlaydi va konflikt bo'lsa **avtomat qayta uriniladi**. Bu
shuni anglatadiki, "o'qib → tekshirib → o'zgartirish" naqshi xavfsiz. Masalan,
bir hisobdan ikki parallel pul yechish — ikkalasi ham bir balansni ko'rib,
ikkalasi ham o'tib ketmaydi (overdraft bo'lmaydi):
```flux
db.tx \->
  acc = db.one "select * from accounts where id=$1" [aid]
  if acc.balance < amt
    fail 422 "balans yetarli emas"
  db.up "accounts" {balance:acc.balance - amt} {id:aid}   # race-xavfsiz
```
> Boshqa tillarda buning uchun `SELECT FOR UPDATE`, lock, mutex yozish kerak.
> Flux'da — kerak emas, `db.tx` o'zi kafolatlaydi. "Til AI'ga moslashadi":
> AI lock haqida o'ylamaydi, shunchaki `db.tx` ichiga yozadi.

**Idempotency — bir amalni ikki marta bajarmaslik.** Pul ko'chirish kabi
joylarda mijoz so'rovni qayta yuborishi mumkin. Unikal kalit (`uniq` ustun)
bilan himoyalang: avval mavjudini tekshiring, keyin tranzaksiya ichida kalitni
yozing — dublikat bo'lsa `uniq` xato → tx rollback:
```flux
old = db.one "select * from transactions where ikey=$1" [key]
old ?? (ret old)              # allaqachon bajarilgan → eski natijani qaytar
db.tx \->
  db.ins "transactions" {ikey:key amount:amt ...}   # dublikat → uniq → rollback
  # ... pul ko'chirish
```
> Bu — e-commerce checkout kabi joylar uchun **majburiy**. Tranzaksiyasiz
> o'rtada xato bo'lsa, ba'zi stok kamaygan, lekin buyurtma yaratilmagan
> holat qoladi.
- Parametrlar `$1, $2...` orqali, qiymatlar ro'yxat sifatida `[...]`.
- `db.ins`/`db.up`'da map kalitlari — ustun nomlari.
- **Param'siz so'rov** — ro'yxat shart emas: `db.q "select * from links"`.
- **Aggregat (count/sum)** bo'sh jadvalda `nil` qaytarishi mumkin —
  `?? 0` bilan himoyalang:
  ```flux
  r = db.one "select count(*) c, sum(clicks) s from links"
  log "links: ${r.c}, clicks: ${r.s ?? 0}"
  ```

**Schema e'loni — `tbl`.** Jadvallarni Flux'ning o'zida e'lon qilasiz:
```flux
tbl products
  id    serial pk
  owner int ref:users.id
  name  str
  price flt
  ts    now
```
Tip kalit so'zlari: `serial int flt str bool json now sym money`. Modifikatorlar:
`pk` (primary key), `uniq`, `null`, `ref:jadval.ustun` (tashqi kalit).
Ko'p ustunli unikal: jadval tanasida `uniq(agent, key)` (ikki ustun birga
unikal — masalan har agent uchun har kalit faqat bir marta).

**`json` ustun** — o'qiganda **avtomat map/list** bo'ladi (string emas,
`json.dec` shart emas); yozganda map/list avtomat enkod qilinadi.

**`money` tipi — pul uchun.** Pul HECH QACHON `flt` (float) bo'lmasligi kerak —
float yaxlitlash xatosi pulni buzadi. `money` — butun **minor birlik** (tiyin,
sent): `15000` = 150.00 so'm. Hamma pul-math `money`/`int` bilan (`int` 64-bit):
```flux
tbl accounts
  id      serial pk
  balance money       # tiyinda, masalan 15000 = 150.00
total = price * qty   # int math, float emas
```

**`sym` tipi — enum uchun.** Bu Flux'ning chiroyli yechimi. Ustun `sym`
bo'lsa: DB'da **matn** saqlanadi, lekin Flux uni o'qiganda avtomat **symbol**
qaytaradi. Yozish va filtrlashda symbol avtomat matnga aylanadi. Shunda `match`
to'g'ridan-to'g'ri ishlaydi:
```flux
tbl tickets
  category sym         # DB: matn ("billing"), Flux: symbol (:billing)
  status   sym

# Yozish: symbol berasiz, DB matn saqlaydi
db.ins "tickets" {category::billing status::new}

# O'qish: schema sym desa, Flux symbol qaytaradi
t = db.one "select * from tickets where id=$1" [id]
match t.category       # t.category — symbol, shuning uchun match ishlaydi
  :billing -> log "to'lov masalasi"
  :technical -> log "texnik"
  _ -> log "boshqa"

# Filtrlash: symbol uzatiladi, avtomat matnga aylanadi
db.q "select * from tickets where category=$1" [:billing]
```
**Bitta qoida:** `sym` ustun — DB'da matn, Flux'da symbol, aylanish avtomat.

### 9.3 `ai` — LLM (birinchi darajali primitiv)

Bu Flux'ni boshqa tillardan ajratib turadigan eng katta narsa. LLM — kalit
so'z, SDK emas. Kalit `$AI_KEY` dan avtomat o'qiladi.

```flux
use ai

# Oddiy savol-javob → matn
javob = ai.ask "Bu xabarni o'zbekchaga tarjima qil: ${text}"

# Strukturali ajratish (typed extraction) → schema bo'yicha map
schema = {
  intent: ":new_order|:question|:other"
  items: [{product:str qty:int}]
}
r = ai.json "Buyurtmani ajrat: ${text}" schema
# r.intent, r.items[0].product ...

```

**Audit metadata — avtomat.** Har bir `ai.*` natijasi `_` ostida metadata
olib keladi:
```flux
r = ai.json prompt schema
log r._.conf        # ishonch balli (0..1)
log r._.tokens      # ishlatilgan token
log r._.cost        # narx
log r._.ms          # kechikish (millisekund)
```
Bu ishonch-asosli yo'naltirish (confidence routing) uchun asosiy:
```flux
if r._.conf > 0.85
  auto_javob r          # yuqori ishonch → avtomat
elif r._.conf >= 0.6
  egadan_sora r         # o'rta → tasdiq so'ra
else
  egaga_yubor r         # past → to'liq eskalatsiya
```

> **Eslatma:** `_.conf` — LLM batareyasi qaytaradigan kalibrlangan ishonch.
> Real hayotda buni logprob yoki self-eval bilan ta'minlash kerak; til buni
> batareya ortida yashiradi.

**`ai.run` — agent tool-loop (BIR qadam).** AI tool ishlatmoqchi bo'lsa,
`ai.run` uni **o'zi bajarmaydi** — sizga *nima qilmoqchiligini* qaytaradi.
Siz tool'ni bajarib (logging, narx, tasdiq bilan), natijani qaytarib berasiz.
Loop **qo'lda** — bu sizga to'liq nazorat beradi:
```flux
msgs <- [{role::user content:text}]
each i in 1..10                          # maksimum 10 qadam
  r = ai.run msgs tools                  # tools: [{name desc params}] ro'yxati
  if r.kind == :final
    ret r.text                           # AI tugadi → final javob
  # r.kind == :call → AI tool chaqirmoqchi
  out = reg.call r.tool r.args           # tool'ni nomi bilan bajar (pastга qara)
  log "tool ${r.tool}: ${r._.ms}ms"      # logging/cost/tasdiq shu yerda
  msgs <- msgs.push {role::tool name:r.tool content:(json.enc out)}
```
> `ai.run` ataylab bir qadamli. Agar AI'ning tool chaqiruvlarini avtomat,
> nazoratsiz bajartirsa, logging/narx/tasdiq qila olmas edingiz. Loop sizniki —
> shuning uchun har tool chaqiruvini ko'rasiz va boshqarasiz.

### 9.4 `reg` — funksiya registri (dinamik dispatch)

Funksiyani **string nomi bilan** saqlash va chaqirish. Agent tool'lari uchun
zarur: AI sizga tool **nomini** (matn) beradi, siz uni funksiyaga aylantirib
chaqirishingiz kerak.

```flux
reg.add "calc" \args -> args.a + args.b          # nom → funksiya
reg.add "search" \args -> http.get "/s?q=${args.q}"

out = reg.call "calc" {a:2 b:3}                  # nomi bilan chaqir → 5
reg.has "search"                                  # ro'yxatda bormi → bool
reg.names                                         # barcha nomlar ro'yxati
```

> **Nega `reg` kerak?** Boshqacha bo'lsa, AI'dan kelgan tool nomini
> `match name` (hardcoded switch) bilan bajarish kerak edi — har yangi tool
> uchun kodni o'zgartirib. `reg` bilan tool'lar **runtime'da** qo'shiladi
> (`reg.add`), AI istalganini `reg.call` bilan chaqiradi. Agent platforma
> aynan shusiz qurib bo'lmaydi.

### 9.5 `list` metodlari, `str` / `math` / `rand` / `time` — yadro

Bularning hammasi **yadro** — `use` qilmasdan ishlaydi (xuddi `log` kabi).

**`list` — ro'yxat metodlari** (qiymat ustida, `.metod`):
```flux
l.len                  # uzunlik
l.push x               # element qo'shadi → yangi ro'yxat
l.filter \x -> x > 0   # shartga mosini qoldiradi → yangi ro'yxat
l.map \x -> x * 2      # har birini o'zgartiradi → yangi ro'yxat
l.has x                # ichida bormi → bool
l.0  l.1               # indeks bo'yicha element
l.slice a b            # a..b oralig'i (b kirmaydi) → yangi ro'yxat
l.join ", "            # → matn: [1 2 3].join "," → "1,2,3"
l.reduce 0 \acc x -> acc + x   # yig'ish: (boshlang'ich qiymat, funksiya)
```

> **Muhim:** ro'yxat qurish uchun `l.push x` ishlating, `l + [x]` **emas**.
> Filtrlash uchun qo'lda `each` loop o'rniga `l.filter`, matn qurish uchun
> qo'lda akkumulyator o'rniga `l.join`:
> ```flux
> # Qo'lda (uzun):              Metod bilan (toza):
> result <- []                  result = items.filter \t -> t.active
> each t in items
>   if t.active
>     result <- result.push t
>
> text <- ""                    text = names.join ", "
> each n in names
>   text <- text + n + ", "
> ```

**`map` — kalit-qiymat metodlari** (qiymat ustida, `.metod`):
```flux
m.set k v              # kalit qo'yadi/yangilaydi → yangi map
m.del k                # kalitni o'chiradi → yangi map
m.has k                # kalit bormi → bool
m.keys                 # kalitlar ro'yxati
m.vals                 # qiymatlar ro'yxati
m.key   m[k]           # o'qish (m[k] — dinamik, o'zgaruvchi kalit)
```
> **Muhim:** map'ga **yozish** uchun `m.set k v` ishlating. `m[k]` faqat
> **o'qiydi** (yozmaydi). Bu list bilan izchil: list'da `push`, map'da `set`.
> Shared state (masalan, realtime'da kim qaysi xonada) shu metodlar bilan
> boshqariladi.

**`str` — matn funksiyalari:**
```flux
str.len s              # uzunlik (son)
str.slice s 0 3        # 0..3 oralig'i (3 kirmaydi): "salom" → "sal"
str.up s               # KATTA HARF
str.low s              # kichik harf
str.split s ","        # ajratish → ro'yxat: "a,b" → ["a" "b"]
str.has s "qism"       # ichida bormi → bool
str.int "42"           # matn → son
str.str 42             # son → matn
```

> **Nega `str.len s` ro'yxatdagi `list.len` dan farqli?** Ro'yxat uzunligi —
> a'zo (`list.len`), matn uzunligi — modul funksiyasi (`str.len s`). Sabab:
> ro'yxat va matn alohida tiplar, va ularning operatsiyalari aralashmasligi
> kerak. Ikkalasi bir xil `.len` bo'lsa chalkashardi.

**`math` — matematika:**
```flux
math.floor 3.7         # → 3
math.ceil 3.2          # → 4
math.abs -5            # → 5
```

**`rand` — tasodifiy:**
```flux
rand.int 1 100         # 1..100 oralig'ida tasodifiy butun son
rand.str 6             # 6 ta belgili tasodifiy satr (qisqa kod uchun ideal)
```

**`time` — vaqt va sana:**
```flux
time.now               # hozirgi vaqt (timestamp)
time.ago 24 :hr        # 24 birlik oldingi vaqt. Birlik: :sec :min :hr :day
time.fmt t "..."       # timestamp'ni matnga formatlash
```
> DB so'rovida raw `now() - interval '24 hours'` yozish o'rniga `time.ago`
> ishlating — toza va xavfsiz:
> ```flux
> r = db.one "select count(*) c from tickets where created > $1" [time.ago 24 :hr]
> ```

### 9.6 `json`
```flux
use json
s = json.enc value     # qiymat → JSON matn
v = json.dec str       # JSON matn → qiymat
```

### 9.7 `env` — muhit o'zgaruvchilari
```flux
port = env.PORT ?? "8080"      # to'g'ridan-to'g'ri env.NOM
key = env.AI_KEY
```

### 9.8 `cron` — rejalashtirish
Standart **Unix 5-maydonli** cron ifoda: `daqiqa soat kun oy hafta-kuni`. Har
AI agent shu formatni biladi (crontab, GitHub Actions, ...). `cron.on` ifodani
**tirnoqsiz** o'qiydi — `*` bu yerda ko'paytirish emas, cron belgisi:
```flux
use cron
cron.on 0 * * * * check_prices    # har soat boshida (daqiqa=0)
cron.on 30 9 * * * daily_check    # har kun 09:30
cron.on 0 18 * * 0 briefing       # yakshanba (0) 18:00
cron.on */15 * * * * poll         # har 15 daqiqada
cron.on 0 9 * * 1-5 \->           # ish kunlari 09:00 (inline lambda)
  log "ish kuni"
```
Maydonlar: `*` har qiymat, `*/N` har N, `A-B` diapazon, `A,B,C` ro'yxat.
Hafta-kuni: 0=yakshanba ... 6=shanba.

`cron.on` **bloklamaydi** — `http.on` kabi faqat ro'yxatga oladi va scheduler
fonda ishga tushadi. Server (`http.serve`/`ws.serve`) processni tirik ushlaydi,
cron fonda o'z vaqtida ishlaydi. Tartib: `cron.on` lar `http.serve` dan **oldin**.

Faqat-cron skript (server yo'q) uchun — `cron.run` processni o'z qo'liga oladi:
```flux
cron.on 0 9 * * * daily_check
cron.run                          # bloklaydi: dastur tugamaydi, cron ishlayveradi
```

> Qulaylik: ifodani tirnoq bilan ham yozsa bo'ladi (`cron.on "0 9 * * *" f`) —
> natija bir xil. AI uchun kanonik shakl tirnoqsiz (kam token).

### 9.9 `queue` — fon navbati
Webhook tez javob berishi uchun og'ir ishni fonga uzatasiz:
```flux
use queue

queue.on "send" \job -> tools.send job.ph job.body   # ishlovchi (handler)
queue.push "send" {ph:phone body:text}               # navbatga qo'shish
```

- `queue.on <nom> <handler>` — shu nomli ishlar uchun ishlovchi. Handler bittagina
  `job` argumenti oladi — bu `queue.push`'ga berilgan payload (map).
- `queue.push <nom> <payload>` — navbatga ish qo'shadi. Payload ixtiyoriy
  (berilmasa `nil`). **Bloklamaydi** — darhol qaytadi, ish fonda bajariladi.
- Ishlar **bitta worker thread'da, FIFO (kelgan tartibda)** bajariladi —
  ketma-ketlik kafolatlangan. Handler ichidagi xato worker'ni o'ldirmaydi.
- `push` `on`'dan oldin yozilsa, ish **navbatda kutadi** va handler ro'yxatga
  olingach bajariladi (tartibga bog'liq emas).
- Worker fon thread'i — server (`http.serve`/`ws.serve`) yoki `cron.run` processni
  ushlab turganda navbatni qayta ishlaydi. Faqat-queue skriptda processni ushlash
  uchun shu bloklovchi chaqiruvlardan biri kerak.

### 9.10 `ws` — websocket (realtime)

Real-time ilovalar (chat, jonli yangilanish) uchun. `http` so'rov-javob bo'lsa,
`ws` doimiy ikki tomonlama ulanish.

```flux
use ws

ws.on :connect \conn ->         # yangi ulanish. conn.id — barqaror unikal id
  ws.data.set conn :user nil    # ws.data — SHU ulanish uchun sessiya holati

ws.on :message \conn msg ->     # msg — kelgan matn (JSON bo'lsa json.dec qiling)
  m = json.dec msg
  ws.send conn (json.enc {ok:true})    # SHU ulanishga javob

ws.on :disconnect \conn ->
  ws.room.leave conn "ch:5"

ws.serve 9000
```

- `ws.on :hodisa handler` — hodisa: `:connect`, `:message`, `:disconnect`.
  `:message` handler `\conn msg ->` (msg — kelgan **matn**), qolganlari `\conn ->`.
- `ws.send conn matn` — SHU ulanishga yuboradi (matn; JSON kerak bo'lsa `json.enc`).
- `ws.data.set conn :kalit qiymat` / `ws.data.get conn :kalit` — SHU ulanish
  uchun sessiya holati (Flux ulanish uzilguncha saqlaydi, uzilganda tozalaydi).
- `ws.serve port` — serverni ishga tushiradi (bloklovchi).

**Xona (room) — broadcast uchun.** Bir guruhga bir vaqtda yuborish. Flux
xonalarni o'zi boshqaradi — siz qo'lda "kim qaysi xonada" map'ini yuritmaysiz:
```flux
ws.room.join conn "ch:5"                          # ulanishni xonaga qo'shish
ws.room.leave conn "ch:5"                         # xonadan chiqarish
ws.room.send "ch:5" (json.enc {t:"msg" body:b})   # xonadagi HAMMAGA yuborish
ws.room.members "ch:5"                            # xonadagilar (presence uchun)
```

> `http.serve` va `ws.serve` **birga** ishlaydi (har xil portda). Xona
> a'zoligi va presence — `ws.room` ichida boshqariladi, qo'lda shared-state
> map kerak emas.

### 9.11 `log` — stderr'ga chiqarish
```flux
log "xabar"          # diagnostika uchun stderr'ga
```

---

## 10. To'liq kichik dastur (hammasi birga)

```flux
use http db ai json

tbl notes
  id   serial pk
  text str
  ts   now

http.on :post "/notes" \req ->
  note = db.ins "notes" {text:req.body.text}
  rep 201 note

http.on :get "/notes" \req ->
  rep 200 (db.q "select * from notes order by ts desc")

log "server :8080 da"
http.serve 8080
```

Mana butun til. `use` qiling, `tbl` bilan jadval, `http.on` bilan marshrut,
`db` bilan saqlash — paket yo'q, ulanish kodi yo'q, boilerplate yo'q.
