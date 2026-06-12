# Fluxon — Dasturlash Tili (Inson uchun to'liq qo'llanma)

> 🌐 **Til:** O'zbek (joriy) · [English](fluxon-human.md)

> **Fluxon nima?** Fluxon — AI agentlar yaxshi yozadigan, backend tizimlari uchun
> mo'ljallangan dasturlash tili. Falsafasi: *"Til AI'ga moslashadi, AI tilga
> emas."* Har bir ishni qilishning **bitta** aniq yo'li bor, sintaksis kam
> token ishlatadi, va eng kerakli narsalar (HTTP server, ma'lumotlar bazasi,
> AI/LLM chaqiruvi, cron, navbat) — tilning **ichida**, hech qanday paket
> o'rnatmasdan.

Fluxon fayllari `.fx` kengaytmasi bilan saqlanadi.

Bu hujjat — to'liq, batafsil **inson** qo'llanmasi. Agar siz AI agentga Fluxon'ni
o'rgatmoqchi bo'lsangiz, qisqaroq `fluxon-agent.md` faylidan foydalaning.

---

## 0. Asosiy g'oyalar (avval shularni o'qing)

Fluxon'ni boshqa tillardan ajratib turadigan 5 ta tamoyil:

1. **Bir ish = bir yo'l (canonical form).** Boshqa tillarda bir narsani 5 xil
   yozish mumkin (`while`, `for`, `do-while`...). Fluxon'da takrorlash uchun
   **faqat `each`** bor. Ekranga chiqarish uchun **faqat bitta** usul. Bu
   qoidaning sababi: AI har safar "qaysi usulni tanlay?" deb o'ylamaydi —
   tanlov yo'q, demak xato ham kam.

2. **Kam token, lekin tushunarli.** Sintaksis imkon qadar qisqa, lekin
   *shifrli emas*. Kalit so'zlar to'liq yoziladi (`each`, `match`, `else`) —
   chunki Fluxon'ni birinchi marta ko'rgan odam yoki AI ularni darhol tushunishi
   kerak.

3. **Batteries included (hammasi ichida).** `http`, `db`, `ai`, `json`, `cron`,
   `queue` — bularning hammasi tilning standart kutubxonasida. Hech qanday
   `npm install`, `composer require` yo'q. Faqat `use http` deysiz va
   ishlatasiz.

4. **AI — birinchi darajali primitiv.** Boshqa tillarda LLM chaqirish uchun
   SDK o'rnatib, kalit sozlab, JSON parse qilasiz. Fluxon'da `ai.json` bitta
   qatorda matnni strukturali ma'lumotga aylantiradi va ishonch ballini
   qaytaradi.

5. **Ahamiyatli bo'shliq (indentation).** Bloklar `{}` qavslar bilan emas,
   **chekinish (2 bo'shliq)** bilan ajratiladi — xuddi Python kabi. Bu ortiqcha
   belgilarni olib tashlaydi.

---

## 1. Leksik asoslar

### Izohlar (comments)
Faqat bitta turdagi izoh bor — `#` belgisidan qator oxirigacha:
```fluxon
# Bu izoh
x = 5   # Bu ham izoh
```
Fluxon'da `//` yoki `/* */` **yo'q**. Bitta usul — `#`.

### Statementlar
Har bir statement **yangi qatorda** tugaydi. Nuqtali vergul (`;`) **kerak emas**
va ishlatilmaydi:
```fluxon
x = 5
y = 10
```

### Bloklar
Blok `{}` bilan emas, **chekinish** bilan ochiladi. Har daraja — **2 bo'shliq**.
Chekinish kamayganda blok tugaydi:
```fluxon
if x > 0
  log "musbat"
  log "ikkinchi qator ham blok ichida"
log "blokdan tashqari"
```

---

## 2. Qiymatlar va tiplar

Fluxon'da quyidagi asosiy tiplar bor:

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
| — | `bytes` | Ikkilik ma'lumot (binary) — literal yo'q, funksiyadan keladi |

### Muhim nozikliklar

**Ikkilik ma'lumot (`bytes`).** Rasm, PDF, arxiv kabi matn bo'lmagan
ma'lumotlar uchun. Literal sintaksisi yo'q — qiymat funksiyalardan keladi:
`fs.readb yo'l` (faylni ikkilik o'qish), `crypto.b64db s` (base64'ni ikkilik
ochish), `bytes.of s` (matn → UTF-8 baytlari). Asosiy amallar:
```fluxon
b = fs.readb "rasm.png"     # bytes (fayl yo'q bo'lsa nil)
bytes.len b                  # bayt soni (str.len esa BELGI sanaydi)
bytes.str b                  # bytes → matn (UTF-8 bo'lmasa aniq xato)
bytes.slice b 0 4            # qism baytlar
fs.write "nusxa.png" b       # fs.write/append str ham, bytes ham oladi
rep 200 b {content_type:"image/png"}   # HTTP'da xom ikkilik javob
```
Log/interpolatsiyada bytes `<bytes N>` ko'rinishida chiqadi — xom baytlar
matnga sizib chiqmaydi. `crypto.sha256`/`b64`/`hex` kirishlari str yoki bytes.

**Ro'yxat va map'da vergul YO'Q.** Elementlar bo'shliq bilan ajraladi. Bu
ataylab — vergullar token isrof qiladi:
```fluxon
nums = [1 2 3 4]
user = {name:"Aziza" age:30 active:true}
```

**Matn ichida o'zgaruvchi qo'yish (interpolation).** `"${...}"` orqali ifodani
matn ichiga joylashtirasiz:
```fluxon
name = "Aziza"
log "Salom ${name}!"              # → Salom Aziza!
log "Jami: ${price * qty} so'm"   # ifoda ham bo'ladi
```
Oddiy o'zgaruvchi uchun qisqartirib `"$name"` ham yozsa bo'ladi, lekin ifoda
uchun `${...}` shart.

**Ko'p qatorli matn (blok satr).** Uzun prompt, SQL yoki shablon uchun `"""`
ishlatiladi. Kontent keyingi qatordan boshlanadi, qatorlarning umumiy
chekinishi avtomatik kesiladi — blok kod ichida tabiiy joylashadi:
```fluxon
prompt = """
  Sen yordamchi agentsan.
  Foydalanuvchi savoli: ${savol}
  """
```
Yopuvchi `"""` o'z qatorida tursa, matn oxirida `\n` qolmaydi. Interpolatsiya
va `\n`/`\t` escape'lar oddiy satrdagidek ishlaydi; `"` belgisi esa
escape'siz erkin yoziladi (JSON/HTML parchalari uchun qulay).

**Belgilar (symbols) — enum o'rniga.** Holatlarni ifodalash uchun matn
o'rniga belgi ishlating. `:new`, `:confirmed` — bu `"new"` matnidan token
arzonroq va aniqroq:
```fluxon
status = :confirmed
dir = :in
```
Belgi matnga aylanganda (interpolatsiya, `str.str`, `+`, `log`) `:` prefiksi
tushib qoladi — qiymat `florist`, `:` esa sintaksis belgisi: `str.str :florist`
→ `"florist"`, `"yo'l/${:florist}"` → `"yo'l/florist"`. Ro'yxat/map ichida esa
`:` saqlanadi (`[:a]` → `[:a]`), chunki u yerda belgi matndan ajralib turishi kerak.

**Truthiness (rost/yolg'on qiymati).** `nil` va `false` — yolg'on. Qolgan
hamma narsa (shu jumladan `0`, `""`, bo'sh ro'yxat) — **rost**. Bu sodda
qoida ataylab: faqat ikki narsa yolg'on.

---

## 3. O'zgaruvchilar (bindings)

Fluxon'da **ikki** xil bog'lash bor, va ular **boshqa ish** qiladi (shuning
uchun ikkitasi bo'lishi canonical qoidaga zid emas):

### `=` — o'zgarmas (immutable)
Bir marta qiymat beriladi, keyin o'zgartirib bo'lmaydi:
```fluxon
x = 10
name = "Aziza"
```
Bu **standart** holat. Ko'pchilik qiymatlar o'zgarmaydi.

### `<-` — o'zgaruvchan (mutable)
Qiymatini keyin o'zgartirish mumkin bo'lgan o'zgaruvchi. Qayta tayinlash ham
`<-` bilan:
```fluxon
total <- 0.0
total <- total + 5.0     # qayta tayinlash
```

> **Qoida:** agar qiymat o'zgarmasa — `=` ishlating. Faqat haqiqatan
> o'zgaradigan narsalar uchun `<-`. Bu kod o'qishini osonlashtiradi: `<-`
> ko'rsangiz, "bu o'zgaradi" deb bilasiz.

---

## 4. Operatorlar

### Arifmetik
```fluxon
+   -   *   /   %        # qo'shish, ayirish, ko'paytirish, bo'lish, qoldiq
```
**`+` string'larni ham birlashtiradi.** Operandlar son bo'lsa — qo'shadi,
matn bo'lsa — ulaydi:
```fluxon
1 + 2          # → 3
"sal" + "om"   # → "salom"
```
Tip o'zi farqni belgilaydi — bitta operator, ikki tabiiy ish.

### Solishtirish
```fluxon
==  !=  <  <=  >  >=
```

### Mantiqiy
```fluxon
&    # va (and)
|    # yoki (or)
!    # emas (not) — qiymat oldida: !x
```

### Maxsus operatorlar

**`??` — null-coalesce.** Chap tomon `nil` bo'lsa, o'ng tomonni beradi:
```fluxon
port = env.PORT ?? "8080"     # PORT yo'q bo'lsa, "8080"
name = user.name ?? "mehmon"
```

**`.` — a'zoga murojaat / indeks.** Map kaliti, ro'yxat indeksi, uzunlik:
```fluxon
user.name        # map kaliti
list.0           # ro'yxatning birinchi elementi
list.len         # uzunlik
m[key]           # dinamik kalit (o'zgaruvchi orqali)
list[i]          # hisoblangan indeks (ifoda bilan)
list.(i)         # `.` orqali hisoblangan indeks — list[i] bilan bir xil
```

**`..` — diapazon (range).** Ikkala chet ham kiradi:
```fluxon
1..5             # [1 2 3 4 5]
```

**`|>` — quvur (pipe).** Qiymatni funksiyaga uzatadi, ichма-ich yozuvni
yo'qotadi:
```fluxon
result = data |> clean |> format
# bu g'a teng: format(clean(data))
```

---

---

## 5. Funksiyalar

Funksiya `fn` bilan e'lon qilinadi. Argumentlar **bo'shliq** bilan ajraladi
(vergul yo'q):

```fluxon
fn add a b
  ret a + b
```

### Bir qatorli funksiya
Agar tana bitta ifoda bo'lsa, `->` bilan bir qatorda yozsa bo'ladi:
```fluxon
fn double x -> x * 2
```

### Qaytarish (return)
Ikki usul, lekin ular bir xil natija beradi:
- `ret x` — aniq qaytarish
- **Oxirgi ifoda** — avtomat qaytariladi (`ret`siz)

```fluxon
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
```fluxon
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
```fluxon
add 2 3            # → 5
double 4           # → 8
```
Qavs faqat **guruhlash** uchun kerak (funksiya natijasini boshqasiga uzatish):
```fluxon
double (add 2 3)   # avval add 2 3 = 5, keyin double 5 = 10
```

**Argumentsiz funksiya — bo'sh qavs `()` bilan chaqiriladi.** Qavssiz chaqirish
argument bilan aniqlangani uchun, parametri yo'q funksiyani chaqirishning yagona
yo'li shu. Bu nom (qiymat) bilan chaqiruvni aniq ajratadi:
```fluxon
fn new_id -> rand.str 8
new_id()           # CHAQIRUV → har safar yangi tasodifiy id
new_id             # CHAQIRMAYDI → funksiya QIYMATI (callback/reg uchun)
```
> `f(x)` (qavs ichida argument) **ishlamaydi** — canonical shakl `f x`. Bo'sh
> `()` faqat argumentsiz chaqiruv uchun (bir ish = bir yo'l).

### Lambda (anonim funksiya)
`\` belgisi bilan, inline ishlatiladi:
```fluxon
\x -> x * 2
each_map nums \x -> x * 2    # har elementni 2 ga ko'paytirish
```

---

## 6. Boshqaruv oqimi (control flow)

### Shartlar: `if` / `elif` / `else`
```fluxon
if x > 0
  log "musbat"
elif x == 0
  log "nol"
else
  log "manfiy"
```
Kalit so'zlar **to'liq** yoziladi (`elif`, `else`) — bir qarashda tushunarli
bo'lishi uchun.

`if` **ifoda sifatida** ham ishlaydi (ternary ekvivalenti): bir qatorda qiymat
qaytaradi. `else` majburiy. Shartdagi qavssiz chaqiruvni qavsga oling.

```fluxon
pad = if h < 10 ("0" + str.str h) else (str.str h)   # leading-zero
turi = if n % 2 == 0 "juft" else "toq"                # oddiy tanlov
r    = if (str.len s) > 0 "to'la" else "bo'sh"        # chaqiruvli shart → qavs
```

### Takrorlash: `each` (yagona loop)
Fluxon'da **faqat bitta** loop bor — `each`. U ro'yxat, diapazon yoki map
ustidan yuradi. `while`, `for`, `do-while` **yo'q**:

```fluxon
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

```fluxon
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
```fluxon
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
> ```fluxon
> # NOTO'G'RI:
> match true
>   conf > 0.85 -> ...
> # TO'G'RI:
> if conf > 0.85
>   ...
> ```

---

## 7. Xatolar (error handling)

Fluxon'da funksiya muvaffaqiyat (`ok`) yoki xato (`err`) qaytarishi mumkin. Xato
bilan ishlashning **bitta** asosiy usuli — `!` operatori, va `nil` uchun `??`.

### `!` — xatoni avtomat yuqoriga uzatish
Funksiya nomidan keyin `!` qo'ysangiz: agar u xato qaytarsa, xato **avtomat**
chaqiruvchiga uzatiladi (siz qo'lda tekshirmaysiz). Agar muvaffaqiyatli bo'lsa,
natijani oladi:
```fluxon
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
```fluxon
name = user.name ?? "mehmon"
each it in items
  p = db.one "...narx..." [it.product]
  p ?? (ask_owner "Narx?"; skip)    # p nil bo'lsa — so'ra va o'tkaz
  log p.price
```

### `fail` — xato chiqarish
O'z kodingizdan xato ko'tarish:
```fluxon
if qty < 1
  fail "miqdor noto'g'ri"
```

**`fail` status kodi bilan — kutilgan xatolar uchun.** HTTP handler ichida
`fail` ga status kodini bersangiz, u **avtomat** o'sha statusli javobga
aylanadi. Bu — `try/catch` o'rnini bosadi: kutilgan xatoda chuqur nesting
o'rniga shunchaki `fail` qiling:
```fluxon
http.on :post "/transfer" \req ->
  acc = db.one "select * from accounts where id=$1" [req.body.from]
  if acc.balance < req.body.amount
    fail 422 "balans yetarli emas"     # → mijozga 422 {error:"balans yetarli emas"}
  # ... asosiy yo'l, nesting yo'q
```
- `fail 4xx "xabar"` — **kutilgan** (biznes) xato → o'sha statusli JSON javob.
- `fail "xabar"` (status'siz) — **kutilmagan** xato → 500.

### `try` / `catch` — xatoni ushlab qolish
Ko'pincha xatoni yuqoriga uzatish (`!`) yoki HTTP javobga aylantirish
(`fail 4xx`) yetarli. Lekin ba'zan xatoni **ushlab qolib, ishni davom
ettirish** kerak — tashqi API yiqilsa default qiymat berish, qayta urinish,
xatoni log qilib so'rovni davom ettirish. Shuning uchun `try`/`catch`:
```fluxon
user = try
  api.get "https://..."!        # shu yerda xato chiqsa?
catch e
  log "api yiqildi: ${e.message}"  # e = {message, status}
  cached_user                       # → catch tanasining qiymati
```
- `catch e` — `e` ga `{message, status}` map'i bog'lanadi. `status` — `fail`
  status kodi, statussiz `fail` yoki runtime xatoda esa `nil`.
- `catch` (o'zgaruvchisiz) — xatoni e'tiborsiz qoldiradi.
- `if` kabi `try`/`catch` ham **ifoda**: muvaffaqiyatda `try` tanasi, xatoda
  `catch` tanasi qiymatini qaytaradi.
- `ret`/`skip`/`stop` — oqim-signallari, **xato emas**: `try`'dan o'tib ketadi,
  ushlanmaydi.
- `catch` ichidan `fail` bilan xatoni qayta ko'tarish mumkin (re-raise).

> **Canonical:** `!` = xatoni uzat, `??` = nil'ni almashtir, `fail` = xato
> chiqar (status bilan yoki status'siz), `try`/`catch` = xatoni ushlab davom
> et. Kutilgan so'rov xatolari uchun avval `fail 4xx`'ni tanlang (kod tekis
> qoladi); `try`/`catch` — tiklanish va davom etish zarur bo'lgandagina.

---

## 8. Modullar (import / export)

### `use` — modul chaqirish
Standart kutubxona yoki o'z faylingizni chaqirasiz. O'rnatish (`install`) yo'q:
```fluxon
use http db ai json        # standart batteries — bo'shliq bilan ko'p modul
use ./tools                # o'z faylingiz → tools.funksiya
```
Chaqirilgandan keyin nomlar modul ostida: `db.one`, `http.serve`,
`tools.create_order`.

**`as` — qayta nomlash (alias).** Agar o'z faylingiz batareya nomi bilan bir
xil bo'lsa (masalan `ai.fluxon` fayl va `ai` batareyasi), to'qnashuv bo'ladi.
`as` bilan o'z modulingizni qayta nomlang:
```fluxon
use ai                     # batareya
use ./ai as helper         # o'z faylingiz → helper.classify (to'qnashmaydi)
```
**Qoida:** o'z fayllaringizga batareya nomini (`ai db http cron`...) bermang,
yoki bersangiz `as` bilan qayta nomlang.

### `exp` — eksport qilish
Faylingizdagi funksiya yoki qiymatni boshqa fayllar uchun ochish:
```fluxon
exp fn create_order items customer
  ...
exp price_limit = 1000
```
Faqat `exp` bilan belgilangan narsalar tashqaridan ko'rinadi.

---

## 9. Batteries — standart kutubxona

Bu — Fluxon'ning eng kuchli qismi. Eng kerakli narsalarning **hammasi** tilning
ichida. Hech narsa o'rnatmaysiz — faqat `use` qilasiz va ishlatasiz.

### 9.1 `http` — server va klient

**Server.** Marshrutni (route) bitta qatorda e'lon qilasiz:
```fluxon
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
- `http.serve port {max_body: BAYT}` — so'rov tanasi o'lcham chegarasini sozlaydi
  (DoS himoyasi). Default `10 MiB` (10485760 bayt); chegaradan oshsa server
  `413 Payload Too Large` qaytaradi va tanani xotiraga yig'maydi. `max_body: 0`
  chegarani o'chiradi (cheklovsiz — faqat ishonchli ichki tarmoq orqasida).

**Fayl yuklash (`multipart/form-data`).** Brauzer formasi yoki `curl -F` yuborgan
fayllar `req.files` ro'yxatiga tushadi, oddiy form maydonlari esa `req.body` ga
(JSON bilan simmetrik):
```flux
http.on :post "/upload" \req ->
  f = req.files.0
  fs.write f.filename f.content
  rep 201 {saved:f.filename size:f.size}
```
- Har fayl: `{name filename content size}`. `content` — UTF-8 matn bo'lsa str,
  ikkilik (rasm, PDF) bo'lsa bytes; `size` — doim **bayt** soni.
- `req.files` doim ro'yxat — multipart bo'lmasa bo'sh (`each` tekshiruvsiz ishlaydi).
- `max_body` chegarasi multipart'ga ham tegishli.

**Redirect (yo'naltirish).** Maxsus fe'l yo'q — `rep` bilan 302 status va
`location` kalitini berasiz; u Location header'ga aylanadi:
```fluxon
http.on :get "/:code" \req ->
  link = db.one "select * from links where code=$1" [req.params.code]
  link ?? (rep 404 {error:"topilmadi"})
  rep 302 {location:link.url}
```

**Route ustunligi.** Agar ikki marshrut bir-biriga to'g'ri kelsa (`/:code` va
`/stats/:code`), **literal (aniq) yo'l avtomat ustun** bo'ladi — yozish
tartibidan qat'i nazar. `/stats/:code` har doim `/:code` dan oldin tekshiriladi.

**Klient.** Tashqi API chaqirish:
```fluxon
res = http.get "https://api.example.com/data"
res = http.post url {key:"val"}      # tana avtomat JSON
# res.status, res.body, res.headers (map, kalit kichik harf)
loc = res.headers.location           # yoki res.headers["content-type"]
```

Redirect (3xx) **default kuzatilmaydi** — `res.status` 30x, `res.headers.location`
o'qiladi. Avtomat kuzatish kerak bo'lsa opsiya map qo'shing:
```fluxon
res = http.get url {follow:true}         # 3xx → Location'ga ergashadi
res = http.get url {follow:true max:5}   # hop limiti (default 10)
# res.hops — necha marta redirect bo'lgani
```
`max`'dan oshsa xato. Opsiya oxirgi argument: `http.post url body {follow:true}`.

**Custom so'rov header'lari.** Autentifikatsiya talab qiladigan API'lar uchun
(`x-api-key`, `Authorization`, `anthropic-version`...) opsiya map'iga `headers`
qo'shing — bu javobdagi `res.headers` bilan simmetrik:
```fluxon
res = http.post "https://api.anthropic.com/v1/messages" body {
  headers: {
    "x-api-key": env.ANTHROPIC_API_KEY
    "anthropic-version": "2023-06-01"
  }
}
```
Header qiymati str bo'lmasa matnga aylantiriladi; `nil` qiymatli header
tashlanadi. Foydalanuvchi `content-type` bersa, avtomatik `application/json`
o'rniga o'sha ishlatiladi.

**Timeout (default 30s).** Qotgan upstream so'rovni abadiy bloklamasligi uchun
har bir klient so'rovi standart 30 soniya timeout bilan ishlaydi. Sozlash:
```fluxon
res = http.get url {timeout: 5}   # 5 soniyada javob bo'lmasa xato
res = http.get url {timeout: 0}   # timeout'siz (faqat ishonchli upstream uchun)
```
Server tomonda ham header o'qish uchun 30s timeout bor (slowloris-uslubdagi sekin
ulanishlardan himoya). LLM so'rovlari (`ai.ask`/`ai.json`/`ai.run`) default 120s
timeout bilan ishlaydi — `$AI_TIMEOUT` (soniya) bilan sozlanadi. LLM API
vaqtinchalik xato qaytarsa (429 rate-limit / 529 overloaded) so'rov **bir marta**
avtomatik qayta uriniladi (server `Retry-After` bersa unga amal qilinadi,
bo'lmasa 2s kutiladi); boshqa xatolar darhol qaytadi.

### 9.2 `db` — ma'lumotlar bazasi (Postgres)

Ulanish **avtomat**: `$DATABASE_URL` muhit o'zgaruvchisidan o'qiladi. Hech
qanday ulanish kodi yozmaysiz.

```fluxon
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
```fluxon
db.tx \->
  ord = db.ins "orders" {cust:c.id total:total}
  each it in items
    db.ins "order_items" {ord:ord.id prod:it.id qty:it.qty price:it.price}
    db.up "products" {stock:it.stock - it.qty} {id:it.id}
  db.up "carts" {status::converted} {id:cart.id}
  # blok oxirigacha yetsa — commit. O'rtada fail bo'lsa — hammasi bekor.
```

`db.tx` qiymat ham qaytaradi (`ret` orqali):
```fluxon
ord = db.tx \->
  o = db.ins "orders" {...}
  ret o            # blok qiymati tashqariga
```

**Concurrency (parallel so'rovlar) kafolati.** `db.tx` avtomat eng kuchli
izolyatsiyada ishlaydi va konflikt bo'lsa **avtomat qayta uriniladi**. Bu
shuni anglatadiki, "o'qib → tekshirib → o'zgartirish" naqshi xavfsiz. Masalan,
bir hisobdan ikki parallel pul yechish — ikkalasi ham bir balansni ko'rib,
ikkalasi ham o'tib ketmaydi (overdraft bo'lmaydi):
```fluxon
db.tx \->
  acc = db.one "select * from accounts where id=$1" [aid]
  if acc.balance < amt
    fail 422 "balans yetarli emas"
  db.up "accounts" {balance:acc.balance - amt} {id:aid}   # race-xavfsiz
```
> Boshqa tillarda buning uchun `SELECT FOR UPDATE`, lock, mutex yozish kerak.
> Fluxon'da — kerak emas, `db.tx` o'zi kafolatlaydi. "Til AI'ga moslashadi":
> AI lock haqida o'ylamaydi, shunchaki `db.tx` ichiga yozadi.

**Idempotency — bir amalni ikki marta bajarmaslik.** Pul ko'chirish kabi
joylarda mijoz so'rovni qayta yuborishi mumkin. Unikal kalit (`uniq` ustun)
bilan himoyalang: avval mavjudini tekshiring, keyin tranzaksiya ichida kalitni
yozing — dublikat bo'lsa `uniq` xato → tx rollback:
```fluxon
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
  ```fluxon
  r = db.one "select count(*) c, sum(clicks) s from links"
  log "links: ${r.c}, clicks: ${r.s ?? 0}"
  ```

**Schema e'loni — `tbl`.** Jadvallarni Fluxon'ning o'zida e'lon qilasiz:
```fluxon
tbl products
  id     serial pk
  owner  int ref:users.id
  name   str
  price  money
  status sym index|uniq      # bir ustunda ko'p modifikator → pipe `|`
  ts     now

  index(owner status)        # ko'p-ustunli index (bo'shliq bilan, vergulsiz)
  uniq(owner price)          # ko'p-ustunli unikal
```
Tip kalit so'zlari: `serial int flt str bool json now sym money`. Modifikatorlar:
`pk` (primary key), `uniq`, `index`, `null`, `ref:jadval.ustun` (tashqi kalit).

**Index va unikal.** Bir ustun uchun ustun oxirida so'z-modifikator: `index`,
`uniq`. Bir ustunda **ikkala** modifikator kerak bo'lsa kanonik shakl `|` (pipe):
`status sym index|uniq`. Bo'shliqli shakl (`index uniq`) ham qabul qilinadi.
**Ko'p-ustunli** uchun alohida qavsli qator: `index(a b)`, `uniq(a b)` — default
bo'shliq bilan ajratiladi (vergulsiz, tejaymiz); vergul ixtiyoriy ham qabul:
`index(a, b)`. **Index nomi avtomatik** (`idx_<jadval>_<ustunlar>` /
`uniq_<...>`) — siz nom o'ylab topmaysiz. Juda uzun nom (DB limiti 63 bayt)
avtomatik qisqartiriladi (deterministik hash suffiks bilan), kod yiqilmaydi.

**Deklarativ migration — `tbl` = yagona manba.** Siz faqat `tbl` ning oxirgi
ko'rinishini yozasiz; Fluxon DB joriy holati bilan farqini hisoblab kerakli DDL'ni
**o'zi** bajaradi:
- yangi ustun → `ADD COLUMN`;
- `tbl`dan olib tashlangan ustun → `DROP COLUMN` (avval jadval `_fluxon_bak_*` ga
  backup qilinadi);
- `tbl` butunlay olib tashlansa → `DROP TABLE` (backup bilan; **faqat Fluxon
  yaratgan** jadvallar — qo'lda yaratilgan jadvalga tegilmaydi);
- index qo'shilsa/olinsa → `CREATE/DROP INDEX`.

Migration **idempotent** — bir xil `tbl` ni qayta deploy qilish xavfsiz, hech
narsa buzilmaydi. Schema o'zgarishi uchun SQL yozish shart emas. Tip o'zgartirish
yoki rename avtomatik EMAS — buni qo'lda `db.q "ALTER TABLE ..."` bilan qilasiz,
Fluxon undan keyin sinxronlaydi.

**`json` ustun** — o'qiganda **avtomat map/list** bo'ladi (string emas,
`json.dec` shart emas); yozganda map/list avtomat enkod qilinadi.

**`money` tipi — pul uchun.** Pul HECH QACHON `flt` (float) bo'lmasligi kerak —
float yaxlitlash xatosi pulni buzadi. `money` — butun **minor birlik** (tiyin,
sent): `15000` = 150.00 so'm. Hamma pul-math `money`/`int` bilan (`int` 64-bit):
```fluxon
tbl accounts
  id      serial pk
  balance money       # tiyinda, masalan 15000 = 150.00
total = price * qty   # int math, float emas
```

**`sym` tipi — enum uchun.** Bu Fluxon'ning chiroyli yechimi. Ustun `sym`
bo'lsa: DB'da **matn** saqlanadi, lekin Fluxon uni o'qiganda avtomat **symbol**
qaytaradi. Yozish va filtrlashda symbol avtomat matnga aylanadi. Shunda `match`
to'g'ridan-to'g'ri ishlaydi:
```fluxon
tbl tickets
  category sym         # DB: matn ("billing"), Fluxon: symbol (:billing)
  status   sym

# Yozish: symbol berasiz, DB matn saqlaydi
db.ins "tickets" {category::billing status::new}

# O'qish: schema sym desa, Fluxon symbol qaytaradi
t = db.one "select * from tickets where id=$1" [id]
match t.category       # t.category — symbol, shuning uchun match ishlaydi
  :billing -> log "to'lov masalasi"
  :technical -> log "texnik"
  _ -> log "boshqa"

# Filtrlash: symbol uzatiladi, avtomat matnga aylanadi
db.q "select * from tickets where category=$1" [:billing]
```
**Bitta qoida:** `sym` ustun — DB'da matn, Fluxon'da symbol, aylanish avtomat.

### 9.3 `ai` — LLM (birinchi darajali primitiv)

Bu Fluxon'ni boshqa tillardan ajratib turadigan eng katta narsa. LLM — kalit
so'z, SDK emas. **Provayder avtomatik aniqlanadi** (OS env yoki `.env`) — hech
narsa sozlash shart emas:

- `ANTHROPIC_API_KEY` bo'lsa → Claude (default `claude-opus-4-8`)
- `OPENAI_API_KEY` bo'lsa → GPT (default `gpt-4o`)
- Ikkalasi bo'lsa Anthropic ustun. Override: `$AI_PROVIDER` (`anthropic|openai`),
  `$AI_KEY` (umumiy kalit), `$AI_MODEL` (model nomi).

Bu `OPENAI_API_KEY`/`ANTHROPIC_API_KEY` kabi keng tarqalgan standart nomlarga
moslashadi — boshqa toollar bilan bir xil `.env` ishlaydi.

```fluxon
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
```fluxon
r = ai.json prompt schema
log r._.conf        # ishonch balli (0..1)
log r._.tokens      # ishlatilgan token
log r._.cost        # narx
log r._.ms          # kechikish (millisekund)
```
Bu ishonch-asosli yo'naltirish (confidence routing) uchun asosiy:
```fluxon
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
```fluxon
msgs <- [{role::user content:text}]
each i in 1..10                          # maksimum 10 qadam
  r = ai.run msgs tools                  # tools: [{name desc params}] ro'yxati
  if r.kind == :final
    ret r.text                           # AI tugadi → final javob
  # r.kind == :call → AI tool chaqirmoqchi. Model parallel bir nechta tool
  # chaqirishi mumkin → hammasi r.calls'da; HAR biriga natija qaytar.
  each c in r.calls
    out = reg.call c.tool c.args         # tool'ni nomi bilan bajar (pastга qara)
    log "tool ${c.tool}"                 # logging/cost/tasdiq shu yerda
    msgs <- msgs.push {role::tool id:c.id content:(json.enc out)}
```
> `r.tool`/`r.args`/`r.id` — orqaga moslik uchun `r.calls[0]` bilan bir xil
> (bitta tool bo'lsa eski kod ishlayveradi). Lekin parallel chaqiruvda HAR bir
> `tool_use_id` uchun natija qaytarmasangiz, keyingi so'rov 400 oladi.
> `ai.run` ataylab bir qadamli. Agar AI'ning tool chaqiruvlarini avtomat,
> nazoratsiz bajartirsa, logging/narx/tasdiq qila olmas edingiz. Loop sizniki —
> shuning uchun har tool chaqiruvini ko'rasiz va boshqarasiz.

### 9.4 `reg` — funksiya registri (dinamik dispatch)

Funksiyani **string nomi bilan** saqlash va chaqirish. Agent tool'lari uchun
zarur: AI sizga tool **nomini** (matn) beradi, siz uni funksiyaga aylantirib
chaqirishingiz kerak.

```fluxon
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
```fluxon
l.len                  # uzunlik
l.push x               # element qo'shadi → yangi ro'yxat
l.filter \x -> x > 0   # shartga mosini qoldiradi → yangi ro'yxat
l.map \x -> x * 2      # har birini o'zgartiradi → yangi ro'yxat
l.has x                # ichida bormi → bool
l.index x              # birinchi mos elementning indeksi, topilmasa -1
l.find \x -> x > 4     # predikatga mos birinchi element, topilmasa nil
l.0  l.1               # indeks bo'yicha element
l.slice a b            # a..b oralig'i (b kirmaydi) → yangi ro'yxat
l.join ", "            # → matn: [1 2 3].join "," → "1,2,3"
l.reduce 0 \acc x -> acc + x   # yig'ish: (boshlang'ich qiymat, funksiya)
l.sort                 # tabiiy tartib (son yoki matn) → yangi ro'yxat
l.sort \a b -> a.p - b.p   # komparator son qaytaradi: manfiy → a oldin
l.reverse              # teskari tartib → yangi ro'yxat
l.uniq                 # takrorlarni olib tashlaydi (birinchisi qoladi)
l.flat                 # bir daraja tekislaydi: [[1 2] [3]] → [1 2 3]
l.zip other            # juftlash: [1 2].zip ["a" "b"] → [[1 "a"] [2 "b"]]
l.any \x -> x > 4      # birortasi mosmi → bool (birinchi mosda to'xtaydi)
l.all \x -> x > 0      # hammasi mosmi → bool (birinchi nomosda to'xtaydi)
```

> **Muhim:** ro'yxat qurish uchun `l.push x` ishlating, `l + [x]` **emas**.
> Filtrlash uchun qo'lda `each` loop o'rniga `l.filter`, matn qurish uchun
> qo'lda akkumulyator o'rniga `l.join`:
> ```fluxon
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
```fluxon
m.set k v              # kalit qo'yadi/yangilaydi → yangi map
m.del k                # kalitni o'chiradi → yangi map
m.merge other          # ikki map'ni birlashtiradi (other ustun) → yangi map
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
```fluxon
str.len s              # uzunlik (son)
str.slice s 0 3        # 0..3 oralig'i (3 kirmaydi): "salom" → "sal"
str.up s               # KATTA HARF
str.low s              # kichik harf
str.split s ","        # ajratish → ro'yxat: "a,b" → ["a" "b"]
str.has s "qism"       # ichida bormi → bool
str.int "42"           # matn → son
str.str 42             # son → matn
str.trim "  s  "       # bosh/oxir bo'shliqni kesadi → "s"
str.replace s "-" "+"  # hamma "-" ni "+" ga almashtiradi
str.starts s "/api"    # prefiks bilan boshlanadimi → bool
str.ends s ".fx"       # suffiks bilan tugaydimi → bool
str.pad "7" 3 "0"      # CHAPdan to'ldiradi → "007"
str.repeat "ab" 3      # takrorlash → "ababab"
```

> **Nega `str.len s` ro'yxatdagi `list.len` dan farqli?** Ro'yxat uzunligi —
> a'zo (`list.len`), matn uzunligi — modul funksiyasi (`str.len s`). Sabab:
> ro'yxat va matn alohida tiplar, va ularning operatsiyalari aralashmasligi
> kerak. Ikkalasi bir xil `.len` bo'lsa chalkashardi.

**`math` — matematika:**
```fluxon
math.floor 3.7         # → 3
math.ceil 3.2          # → 4
math.abs -5            # → 5
math.min 3 7           # → 3 (int kirsa int qaytadi)
math.max 3 7           # → 7
math.pow 2 10          # → 1024 (int ^ manfiy bo'lmagan int → int)
math.sqrt 9            # → 3.0 (doim flt; manfiy kirish — xato)
```

**`rand` — tasodifiy:**
```fluxon
rand.int 1 100         # 1..100 oralig'ida tasodifiy butun son
rand.str 6             # 6 ta belgili tasodifiy satr (qisqa kod uchun)
```

`rand` OS kriptografik CSPRNG'idan foydalanadi, shuning uchun chiqishi
bashorat qilinmaydi. Ammo **uzunlik ham muhim**: `rand.str 6` atigi ~36 bit
entropiya beradi (62⁶) — qisqa kod uchun yetarli, lekin sir uchun brute-force
qilinadi. Session-ID, token va boshqa sirlar uchun kamida `rand.str 24`
ishlating (~140+ bit).

**`time` — vaqt va sana:**
```fluxon
time.now               # hozirgi vaqt (timestamp)
time.ago 24 :hr        # 24 birlik oldingi vaqt. Birlik: :sec :min :hr :day
time.in  60 :min       # 60 birlik keyingi vaqt (TTL/expiry). Birlik bir xil
time.fmt t "..."       # timestamp'ni matnga formatlash
time.sleep 1           # 1 soniya kutadi (flt ham — 0.5). Polling/retry backoff
time.parse "2026-06-10T10:00:00Z"   # ixtiyoriy ISO matn -> kanonik UTC timestamp ("Z"/"±HH:MM")
time.add t 30 :min     # IXTIYORIY vaqtdan offset (now emas): end_at = start_at + davomiylik
time.sub t 5 :min      # time.add ko'zgusi — orqaga siljitadi (masalan buffer before)
time.diff a b          # (a - b) farq sekundda (int); / 60 -> daqiqa
```
> `time.in`/`time.ago` (**hozirdan** offset) bilan `time.add`/`time.sub`
> (**ixtiyoriy** berilgan vaqtdan offset) farqi: booking server mijoz bergan
> `start_at` dan `end_at = time.add start_at 30 :min` ni hisoblaydi.
> DB so'rovida raw `now() - interval '24 hours'` yozish o'rniga `time.ago`
> ishlating — toza va xavfsiz:
> ```fluxon
> r = db.one "select count(*) c from tickets where created > $1" [time.ago 24 :hr]
> ```

**Davomiylik va interval retseptlari** (interval arifmetikasi BOR — `time.add`/`diff` mavjud):
```fluxon
end_at = time.add start_at dur :min            # davomiylik: start + dur daqiqa
mins   = (time.diff end_at start_at) / 60       # ikki vaqt orasi -> daqiqa
overlap = a.start < b.end & a.end > b.start     # ikki interval kesishadimi (bool)
buf_start = time.sub start_at 15 :min           # buffer: boshidan 15 daqiqa oldin
```

**IANA zona / DST** — `time.parse` ixtiyoriy zona nomini, `time.fmt` esa 3-argument
sifatida zonani oladi. Wall-clock ↔ UTC konversiya DST-aware (fiksrlangan offset
EMAS), shuning uchun "har kuni 09:00 local" yoz/qish o'tishida ham to'g'ri UTC
instant'ga tushadi:
```fluxon
utc = time.parse "2026-07-15 09:00:00" "Asia/Tashkent"   # local wall-clock -> UTC
loc = time.fmt utc "HH:mm" "America/New_York"             # UTC instant -> zona wall-clock
```
> Bahorgi sakrash oynasidagi wall-clock vaqt (masalan soat sakraydigan tunda `02:30`)
> mavjud emas — xato beradi; noma'lum zona nomi ham xato beradi.

### 9.6 `json`
```fluxon
use json
s = json.enc value     # qiymat → JSON matn
v = json.dec str       # JSON matn → qiymat
```

### 9.7 `env` — muhit o'zgaruvchilari
```fluxon
port = env.PORT ?? "8080"      # to'g'ridan-to'g'ri env.NOM
key = env.AI_KEY
```

### 9.8 `cron` — rejalashtirish
Standart **Unix 5-maydonli** cron ifoda: `daqiqa soat kun oy hafta-kuni`. Har
AI agent shu formatni biladi (crontab, GitHub Actions, ...). `cron.on` ifodani
**tirnoqsiz** o'qiydi — `*` bu yerda ko'paytirish emas, cron belgisi:
```fluxon
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
```fluxon
cron.on 0 9 * * * daily_check
cron.run                          # bloklaydi: dastur tugamaydi, cron ishlayveradi
```

> Qulaylik: ifodani tirnoq bilan ham yozsa bo'ladi (`cron.on "0 9 * * *" f`) —
> natija bir xil. AI uchun kanonik shakl tirnoqsiz (kam token).

### 9.9 `queue` — fon navbati
Webhook tez javob berishi uchun og'ir ishni fonga uzatasiz:
```fluxon
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

```fluxon
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
  uchun sessiya holati (Fluxon ulanish uzilguncha saqlaydi, uzilganda tozalaydi).
- `ws.serve port` — serverni ishga tushiradi (bloklovchi).

**Xona (room) — broadcast uchun.** Bir guruhga bir vaqtda yuborish. Fluxon
xonalarni o'zi boshqaradi — siz qo'lda "kim qaysi xonada" map'ini yuritmaysiz:
```fluxon
ws.room.join conn "ch:5"                          # ulanishni xonaga qo'shish
ws.room.leave conn "ch:5"                         # xonadan chiqarish
ws.room.send "ch:5" (json.enc {t:"msg" body:b})   # xonadagi HAMMAGA yuborish
ws.room.members "ch:5"                            # xonadagilar (presence uchun)
```

> `http.serve` va `ws.serve` **birga** ishlaydi (har xil portda). Xona
> a'zoligi va presence — `ws.room` ichida boshqariladi, qo'lda shared-state
> map kerak emas.

### 9.11 `log` — stderr'ga chiqarish
```fluxon
log "xabar"          # diagnostika uchun stderr'ga
```

---

## 10. To'liq kichik dastur (hammasi birga)

```fluxon
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
