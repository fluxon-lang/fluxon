# Flux — til spetsifikatsiyasi (AI uchun)

Flux: AI-native backend tili. Bir ish = bir yo'l. Kam token. Batteries-included.
Fayl kengaytmasi: `.fx`. Bir marta o'qib, to'g'ri Flux kodi yoz.

## Asoslar
- Izoh `# qator oxirigacha` (`//` yo'q). Statement yangi qatorda (`;` yo'q).
- Blok = chekinish (2 bo'shliq), `{}` yo'q.
```flux
if x > 0
  log "musbat"
log "tashqari"
```

## Tiplar
```
42 int · 3.14 flt · "hi" str · true bool · nil · :ok sym (enum/teg)
[1 2 3] list · {a:1 b:2} map        # VERGUL YO'Q, bo'shliq bilan
```
Str interpolatsiya: `"$x"` yoki `"${expr}"`. Truthy: faqat `nil`/`false` yolg'on.

## Bindings
```
x = 10              # o'zgarmas (STANDART)
total <- 0          # o'zgaruvchan; qayta tayinlash: total <- total + 5
```

## Operatorlar
```
+ - * / %      arifmetik. + STRING'ni ham qo'shadi: "a"+"b"→"ab"
== != < <= > >=   ·   & | !  (va/yoki/emas)
??   null-coalesce: a ?? b → a, agar a nil bo'lsa b
.    a'zo/indeks: m.key, l.0, l.len, m[k]
..   diapazon: 1..5 → [1 2 3 4 5]   ·   |>  pipe: x |> f |> g
```

## Funksiyalar
```flux
fn add a b
  ret a + b               # ret (erta) yoki oxirgi ifoda (avtomat)
fn double x -> x * 2      # bir qatorli
add 2 3                   # qavssiz chaqirish; qavs faqat guruhlash: f (g x)
\x -> x * 2               # lambda
```
Lambda ICHIDA ham `ret` ishlaydi — guard-clause (chuqur nesting o'rniga):
```flux
http.on :post "/x" \req ->
  if !req.body.email
    ret rep 400 {error:"email kerak"}
  rep 201 (db.ins "t" {...})
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
Yagona loop = `each` (while/for YO'Q):
```flux
each item in list   ·   each i in 1..5   ·   each k, v in map
```
Loop ichida `skip` (continue), `stop` (break). Shartli takror: `each i in 1..n`
yoki rekursiya.

`match` — qiymat dispatch (FAQAT symbol/son, mantiqiy shart EMAS):
```flux
match status
  :new -> log "yangi"
  _ -> log "default"
```
Mantiqiy shart (`x > 0.85`) uchun HAR DOIM `if/elif/else`. `match true` = xato.

## Errorlar
```flux
user = db.one "..." [id]!     # ! = xato bo'lsa avtomat yuqoriga uzat
name = user.name ?? "mehmon"  # ?? = nil bo'lsa muqobil
fail 422 "balans yetmadi"     # status bilan → mijozga 422 {error:...}
fail "ichki xato"             # status'siz → 500
```
`!` uzat, `??` nil almashtir, `fail [status] "..."` chiqar. try/catch YO'Q —
`fail 4xx` kutilgan xatoni avto HTTP javobga aylantiradi (kod tekis qoladi).

## Modullar
```flux
use http db ai json     # batteries, install yo'q
use ./tools             # o'z fayling → tools.fn
use ./ai as helper      # ALIAS: batareya nomi bilan to'qnashsa → helper.fn
exp fn create_order ... # exp = tashqariga ochish
```

## Batteries (stdlib — install yo'q)

### http
```flux
http.on :post "/notes" \req -> rep 201 {ok:true}
http.on :get "/notes/:id" \req -> rep 200 {id:req.params.id}
http.serve 8080
```
- Metod: `:get :post :put :patch :del`. `req.body` (JSON→map), `req.params.id`,
  `req.query`, `req.headers`. Yo'q kalit → `nil`.
- `rep status body` (map→avtomat JSON). Redirect: `rep 302 {location:url}`.
- Route ustunligi: literal yo'l avtomat ustun (`/stats/:c` > `/:c`).
- Klient: `http.get url`, `http.post url body` → `res.status res.body`.

### db (Postgres, $DATABASE_URL avtomat)
```flux
rows = db.q "select * from t where owner=$1" [oid]   # → map ro'yxati
one  = db.one "select * from users where id=$1" [id] # → map yoki nil
row  = db.ins "orders" {cust:5 status::new}          # → to'liq qator (id bilan)
db.up "orders" {total:1500} {id:oid}                 # {set} {where}
db.del "cart_items" {id:iid}                          # {where}
db.put "memory" {val:v} {agent:a key:k}               # UPSERT (atomik)
```
Param `$1 $2`, qiymat `[...]`. Param'siz: `db.q "select * from links"`.
Aggregat nil bo'lsa `?? 0`: `db.one "select count(*) c, sum(x) s from t"`.

Tranzaksiya — atomik, `fail`/`!` da rollback, qiymat qaytaradi:
```flux
res = db.tx \->
  ord = db.ins "orders" {cust:c total:t}
  each it in items
    db.up "products" {stock:it.stock - it.qty} {id:it.id}
  ret ord
```
`db.tx` avto-serializable + retry → "o'qi-tekshir-yangila" race-safe (lock
kerak emas). Idempotency: `uniq` ustun + tx ichida ins (dublikat → rollback):
```flux
old = db.one "select * from txns where ikey=$1" [key]
old ?? (ret old)
db.tx \-> db.ins "txns" {ikey:key ...}   # dublikat → uniq xato → rollback
```

Schema = `tbl`:
```flux
tbl products
  id    serial pk
  owner int ref:users.id
  price money               # pul = butun minor birlik (tiyin), float EMAS
  ts    now
```
Tiplar: serial int flt str bool json now sym money (`int` 64-bit). Modifikator:
`pk uniq null ref:tbl.col`. Ko'p ustunli: `uniq(agent, key)`.
`json` ustun: o'qiganda avto map/list, yozganda avto enkod.
`sym` ustun: DB'da matn, Flux'da symbol (avto aylanadi):
```flux
db.ins "tickets" {status::new}
t = db.one "select * from tickets where id=$1" [id]
match t.status
  :new -> ...
db.q "select * from t where status=$1" [:new]    # filter: symbol → matn
```

### ai (LLM — first-class, $AI_KEY avtomat)
```flux
txt = ai.ask "savol ${x}"                    # → matn
r = ai.json "ajrat: ${text}" {intent::a items:[{product:str qty:int}]}  # → map
```
Metadata: `r._.conf` (0..1), `r._.tokens`, `r._.cost`, `r._.ms`.
```flux
if r._.conf > 0.85
  auto r
elif r._.conf >= 0.6
  confirm r
else
  escalate r
```
`ai.run` — BIR qadam tool-loop (o'zi bajarmaydi, nima qilmoqchini qaytaradi;
loop sizniki → logging/cost/tasdiq nazorati):
```flux
msgs <- [{role::user content:text}]
each i in 1..10
  r = ai.run msgs tools                # tools: [{name desc params}]
  if r.kind == :final
    ret r.text
  out = reg.call r.tool r.args         # tool'ni nomi bilan bajar
  msgs <- msgs.push {role::tool name:r.tool content:(json.enc out)}
```

### reg (funksiya registri — dinamik dispatch)
Funksiyani STRING nomi bilan saqla/chaqir (agent tool'lari uchun — `match`-switch
EMAS, runtime'da qo'shiladi):
```flux
reg.add "calc" \args -> args.a + args.b
out = reg.call "calc" {a:2 b:3}      # → 5
reg.has "calc"   ·   reg.names
```

### list metodlari (.metod)
```flux
l.len · l.push x · l.filter \x->x>0 · l.map \x->x*2 · l.has x · l.0
l.slice a b · l.join ", " · l.reduce 0 \acc x -> acc + x
```
List qurish: `l.push x` (`+[x]` EMAS). Matn qurish: `l.join sep`.

### map metodlari (.metod)
```flux
m.set k v · m.del k · m.has k · m.keys · m.vals · m.k · m[k]
```
Map'ga yozish: `m.set k v` (`m[k]` faqat O'QISH). Shared state shu bilan.

### str / math / rand (yadro, use kerak emas)
```flux
str.len s · str.slice s a b · str.up s · str.low s · str.split s sep → list
str.has s sub → bool · str.int s · str.str x
math.floor x · math.ceil x · math.abs x · rand.int a b · rand.str n
```
list uzunligi `l.len` (a'zo), str uzunligi `str.len s` (modul).

### time
```flux
time.now · time.ago 24 :hr (:sec :min :hr :day) · time.fmt t "..."
db.one "select count(*) c from t where created > $1" [time.ago 24 :hr]
```

### json / env / log
```flux
json.enc v · json.dec s · env.PORT ?? "8080" · log "xabar"
```

### cron (fon vazifa)
Standart Unix 5-maydon (daqiqa soat kun oy hafta), TIRNOQSIZ — `*` cron belgisi:
```flux
cron.on 0 * * * * check_prices    # har soat boshida · fn yoki \-> lambda
cron.on 30 9 * * 1-5 \-> report    # ish kunlari 09:30
```
`cron.on` bloklamaydi (`http.on` kabi ro'yxatga oladi). Server (`http.serve`/
`ws.serve`) bo'lsa cron fonda ishlaydi; server yo'q skriptda `cron.run` processni ushlaydi.

### queue (fon)
```flux
queue.push "send" {ph:p body:t}
queue.on "send" \job -> tools.send job.ph job.body
```

### ws (websocket — realtime)
```flux
ws.on :connect \conn -> ws.data.set conn :user nil   # conn.id barqaror; ws.data = sessiya
ws.on :message \conn msg ->                    # msg — kelgan MATN (str)
  m = json.dec msg
  ws.send conn (json.enc {ok:true})            # shu ulanishga (matn yuboriladi)
ws.on :disconnect \conn -> ws.room.leave conn "ch:5"
ws.serve 9000
```
Sessiya: `ws.data.set conn :kalit qiymat` · `ws.data.get conn :kalit` (shu ulanish, uzilgach tozalanadi).
Xona (broadcast): `ws.room.join conn "ch:5"` · `ws.room.leave conn "ch:5"` ·
`ws.room.send "ch:5" msg` (hammaga) · `ws.room.members "ch:5"` (presence).
`http.serve` va `ws.serve` birga ishlaydi.

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
