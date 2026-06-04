# Flux — til spetsifikatsiyasi (AI uchun)

Flux: AI-native backend tili. Bir ish = bir yo'l. Kam token. Batteries-included.
Fayl kengaytmasi: `.fx`. Bu spec'ni bir marta o'qib, to'g'ri Flux kodi yoz.

## Asoslar
- Izoh: `# qator oxirigacha`. Faqat shu. `//` yo'q.
- Statement yangi qatorda tugaydi. `;` yo'q.
- Blok = chekinish (2 bo'shliq). `{}` yo'q.
```flux
if x > 0
  log "musbat"
log "tashqari"
```

## Tiplar
```
42        int
3.14      flt
"hi"      str       # interpolatsiya: "$x" yoki "${expr}"
true      bool
nil       nil
[1 2 3]   list      # bo'shliq bilan, VERGUL YO'Q
{a:1 b:2} map       # bo'shliq bilan, VERGUL YO'Q
:ok       sym       # enum/teg
```
Truthy: `nil` va `false` — yolg'on. Boshqa hammasi rost (0, "", [] ham rost).

## Bindings (ikkala turi bor, har xil ish qiladi)
```
x = 10        # o'zgarmas (immutable) — STANDART
total <- 0    # o'zgaruvchan (mutable)
total <- total + 5   # qayta tayinlash faqat <- bilan
```
Qoida: o'zgarmasa `=`, o'zgarsa `<-`.

## Operatorlar
```
+ - * / %                  arifmetik. + STRING'ni ham birlashtiradi: "a"+"b"→"ab"
== != < <= > >=            solishtirish
& | !                      va / yoki / emas
??                         null-coalesce: a ?? b → a, agar a nil bo'lsa b
.                          a'zo/indeks: m.key, list.0, list.len, m[k]
..                         diapazon: 1..5 → [1 2 3 4 5] (ikki chet kiradi)
|>                         pipe: x |> f |> g == g(f(x))
```

## Misol (hammasi birga)
```flux
user = {name:"Aziza" age:30}
nums = [1 2 3]
total <- 0
each n in nums
  total <- total + n
log "Salom ${user.name}, jami ${total}"
port = env.PORT ?? "8080"
```

## Funksiyalar
Argumentlar bo'shliq bilan (vergul yo'q). Qavssiz chaqiriladi.
```flux
fn add a b
  ret a + b

fn double x -> x * 2      # bir qatorli: ->

add 2 3                   # chaqirish, qavssiz
double (add 2 3)          # qavs faqat guruhlash uchun
\x -> x * 2               # lambda
```
Qaytarish: `ret x` (erta) yoki oxirgi ifoda (avtomat).
Lambda ICHIDA ham `ret` ishlaydi — guard-clause uchun (chuqur nesting o'rniga):
```flux
http.on :post "/x" \req ->
  if !req.body.email
    ret rep 400 {error:"email kerak"}    # erta chiqish, qolgani ishlamaydi
  if !req.body.body
    ret rep 400 {error:"body kerak"}
  rep 201 (db.ins "t" {...})              # asosiy yo'l, nesting yo'q
```

## Control flow
```flux
if x > 0
  log "a"
elif x == 0
  log "b"
else
  log "c"
```
Yagona loop = `each` (while/for YO'Q). Ro'yxat/diapazon/map ustidan:
```flux
each item in list
each i in 1..5            # 1,2,3,4,5
each k, v in map
```
Loop ichida: `skip` (continue), `stop` (break). Shart bo'yicha takror kerak
bo'lsa: `each i in 1..n` yoki rekursiya.

Qiymat dispatch = `match` (asosan symbol uchun):
```flux
match status
  :new -> log "yangi"
  :confirmed -> log "ok"
  _ -> log "default"
```
`if` = mantiqiy shart, `match` = qiymatni variantlarga taqsimlash.
DIQQAT: `match` faqat QIYMAT (symbol/son) bilan. Mantiqiy shart (`x > 0.85`)
uchun HAR DOIM `if/elif/else`. `match true` YOZMA — bu xato.

## Errorlar
```flux
user = db.one "..." [id]!     # ! = xato bo'lsa avtomat yuqoriga uzat
name = user.name ?? "mehmon"  # ?? = nil bo'lsa muqobil
p ?? (ask_owner "Narx?"; skip) # nil bo'lsa o'ngdagini bajar
fail "xato matni"             # o'zing xato chiqar
```
Canonical: `!` uzat, `??` nil almashtir, `fail` chiqar. try/catch YO'Q.

## Modullar
```flux
use http db ai json     # batteries, install yo'q
use ./tools             # o'z fayling → tools.fn
use ./ai as helper      # ALIAS: o'z fayling batareya nomi bilan to'qnashsa → helper.fn
exp fn create_order ... # exp = tashqariga ochish
exp limit = 1000
```
Qoida: o'z faylingiz batareya nomini (ai/db/http/cron...) band qilsa, `as` bilan
qayta nomlang — aks holda to'qnashadi.

## Batteries (stdlib — install yo'q)

### http (server + klient)
```flux
use http
http.on :post "/notes" \req -> rep 201 {ok:true}
http.on :get "/notes/:id" \req -> rep 200 {id:req.params.id}
http.serve 8080
```
- `http.on :metod "/yo'l" handler`. Metod: `:get :post :put :patch :del`.
- `req.body` (JSON→map), `req.params.id`, `req.query`, `req.headers`.
- `req.query.x` / `req.body.x` yo'q kalit → `nil` (filter: `if req.query.status ...`).
- `rep status body` — body map bo'lsa avtomat JSON.
- Redirect: `rep 302 {location:url}` — `location` kaliti Location header bo'ladi.
- Route ustunligi: literal yo'l avtomat ustun. `/stats/:code` `/:code` dan
  oldin mos keladi — tartibdan qat'i nazar.
- Klient: `http.get url`, `http.post url body` → `res.status res.body`.

### db (Postgres, $DATABASE_URL avtomat)
```flux
use db
rows = db.q "select * from t where owner=$1" [oid]   # → map ro'yxati
one  = db.one "select * from users where id=$1" [id] # → map yoki nil
row  = db.ins "orders" {cust:5 total:0 status::new}  # → to'liq qator (id bilan)
db.up "orders" {total:1500} {id:oid}                 # {set} {where}
db.del "cart_items" {id:iid}                          # o'chirish {where}
```
Tranzaksiya — ko'p qadamli atomik mutatsiya. Blok ichida xato (`fail`/`!`)
bo'lsa, HAMMA o'zgarish qaytariladi (rollback):
```flux
db.tx \->
  ord = db.ins "orders" {cust:c total:t}
  each it in items
    db.ins "order_items" {ord:ord.id prod:it.id qty:it.qty}
    db.up "products" {stock:it.stock - it.qty} {id:it.id}
  # blok muvaffaqiyatli tugasa — commit; fail bo'lsa — hammasi bekor
```
Parametr `$1 $2...`, qiymat `[...]`. ins/up map kaliti = ustun.
Param'siz so'rovda ro'yxat shart emas: `db.q "select * from links"`.
Aggregat (count/sum) bo'sh jadvalda nil qaytarishi mumkin → `?? 0`:
```flux
r = db.one "select count(*) c, sum(clicks) s from links"
log "links: ${r.c} clicks: ${r.s ?? 0}"
```

Schema = `tbl`:
```flux
tbl products
  id    serial pk
  owner int ref:users.id
  name  str
  price flt
  ts    now
```
Tiplar: serial int flt str bool json now sym. Modifikator: pk uniq null ref:tbl.col.

`sym` tipi (enum uchun): DB'da matn saqlanadi, Flux o'qiganda symbol qaytaradi —
avtomat. Yozish/filter'da symbol avtomat matnga aylanadi:
```flux
tbl tickets
  category sym          # DB: matn, Flux: symbol
  status   sym
db.ins "tickets" {category::billing status::new}   # symbol beriladi
t = db.one "select * from tickets where id=$1" [id]
match t.category        # t.category — symbol, match to'g'ridan ishlaydi
  :billing -> log "to'lov"
  _ -> log "boshqa"
db.q "select * from tickets where category=$1" [:billing]  # filter: symbol → matn
```

### ai (LLM — first-class, $AI_KEY avtomat)
```flux
use ai
txt = ai.ask "savol ${x}"                    # → matn
r = ai.json "buyurtmani ajrat: ${text}" {    # → schema bo'yicha map
  intent: ":new_order|:question|:other"
  items: [{product:str qty:int}]
}
ans = ai.run "javob ber" [get_catalog get_history]  # agentik tool-loop
```
Har ai.* natija `_` metadata olib keladi:
```flux
r._.conf    # ishonch 0..1
r._.tokens  r._.cost  r._.ms
```
Confidence routing:
```flux
if r._.conf > 0.85
  auto r
elif r._.conf >= 0.6
  confirm r
else
  escalate r
```

### list metodlari (qiymat ustida, .metod)
```flux
l.len                    # uzunlik
l.push x                 # element qo'shadi (yangi list qaytaradi)
l.filter \x -> x > 0     # shartga mosini qoldiradi
l.map \x -> x * 2        # har birini o'zgartiradi
l.has x                  # ichida bormi → bool
l.0  l.1                 # indeks
l.slice a b              # a..b oralig'i (b kirmaydi)
l.join ", "              # → str: [1 2 3].join "," → "1,2,3"
l.reduce 0 \acc x -> acc + x   # yig'ish (boshlang'ich, fn)
```
Qo'lda loop o'rniga filter/map/reduce/join ishlat. List qurish: `l.push x`
(`+[x]` EMAS). Matn qurish: `l.join sep` (qo'lda each + akkumulyator EMAS).

### map metodlari (qiymat ustida, .metod)
```flux
m.set k v                # kalit qo'yadi/yangilaydi → yangi map
m.del k                  # kalitni o'chiradi → yangi map
m.has k                  # kalit bormi → bool
m.keys   m.vals          # kalitlar / qiymatlar ro'yxati
m.k   m[k]               # o'qish (m[k] = dinamik kalit)
```
Map'ga yozish: `m.set k v` (`m[k] = v` EMAS — `m[k]` faqat O'QISH).
Shared state (rooms/presence) uchun shu metodlar ishlatiladi.

### str / math / rand (yadro modullari, use kerak emas)
```flux
str.len s                # uzunlik
str.slice s a b          # a..b oralig'i (b kirmaydi): str.slice "salom" 0 3 → "sal"
str.up s   str.low s     # katta/kichik harf
str.split s sep          # → list:  str.split "a,b" "," → ["a" "b"]
str.has s sub            # → bool
str.int s   str.str x    # matn↔son aylantirish
math.floor x  math.ceil x  math.abs x
rand.int a b             # a..b oralig'ida tasodifiy int
rand.str n               # n ta belgili tasodifiy satr (kod generatsiya uchun)
```
Eslatma: list = a'zo (`l.len`, `l.push`), str = modul (`str.len s`) — alohida.

### time (vaqt/sana)
```flux
time.now                 # hozir (timestamp)
time.ago 24 :hr          # 24 soat oldin. Birlik: :sec :min :hr :day
time.fmt t "..."         # formatlash
```
DB so'rovda raw `now() - interval` o'rniga `time.ago` ishlat:
```flux
n = db.one "select count(*) c from tickets where created > $1" [time.ago 24 :hr]
```

### json / env / log
```flux
json.enc v   json.dec s
env.PORT ?? "8080"      # to'g'ridan env.NOM
log "xabar"             # stderr
```

### cron (fe'l)
```flux
cron.wk :sun 18 0 fn    # haftalik: kun soat daqiqa
cron.dy 9 0 fn          # kunlik: soat daqiqa
cron.hr 30 fn           # soatlik: daqiqa
```

### queue (fon)
```flux
queue.push "send" {ph:p body:t}
queue.on "send" \job -> tools.send job.ph job.body
```

### ws (websocket — realtime)
```flux
use ws
ws.on :connect \conn ->                  # conn.id — barqaror unikal id
  conn.data.user = nil                   # conn.data — shu ulanish uchun map
ws.on :message \conn msg ->              # msg — kelgan matn (json.dec qil)
  m = json.dec msg
  ws.send conn (json.enc {ok:true})      # shu ulanishga yuborish
ws.on :disconnect \conn ->
  ws.room.leave conn "ch:5"
ws.serve 9000
```
Xona (room) — broadcast uchun:
```flux
ws.room.join conn "ch:5"                 # ulanishni xonaga qo'shish
ws.room.leave conn "ch:5"
ws.room.send "ch:5" (json.enc {t:"msg" body:b})   # xonadagi HAMMAGA
ws.room.members "ch:5"                    # xonadagi conn'lar ro'yxati (presence)
```
`http.serve` va `ws.serve` birga ishlaydi (har xil port). Shared state
(kim qaysi xonada) `ws.room` ichida — qo'lda map boshqarish shart emas.

## To'liq misol
```flux
use http db

tbl notes
  id   serial pk
  text str
  ts   now

http.on :post "/notes" \req ->
  rep 201 (db.ins "notes" {text:req.body.text})
http.on :get "/notes" \req ->
  rep 200 (db.q "select * from notes order by ts desc")
http.serve 8080
```
