# Flux Runtime

Flux tilining interpretatori (Rust, tree-walking). **Til yadrosi** va
`docs/flux-agent.md` da spetsifikatsiyalangan **barcha batareyalar** ishlaydi:
`http` (server + klient), `db`, `ai`, `auth`, `ws`, `cron`, `queue`, `reg`.
(`db` hozircha faqat SQLite backend; `postgres`/`mysql` stub.)

## Qurish va ishga tushirish

```sh
cargo build --release
cargo run -- run examples/demo.fx
# yoki
./target/release/flux run examples/demo.fx
```

### Buyruqlar

- `flux run <fayl.fx>` — faylni bajaradi (lex → parse → interp). Parse yoki
  runtime xato → `exit 1`.
- `flux check <fayl.fx>` — faqat sintaksisni tekshiradi (lex + parse, kodni
  **bajarmaydi** → side-effect yo'q). To'g'ri → `exit 0`; parse/lex xato →
  `exit 2`. Bu `run`ning `exit 1`idan farqli, shuning uchun chaqiruvchi qaysi
  bosqichda yiqilganini bila oladi (AI self-repair gate uchun qulay).

## Hozir nima ishlaydi

Til yadrosining to'liq qismi:

- **Tiplar:** int, flt, str, bool, nil, sym, list, map
- **Bindings:** `=` (o'zgarmas), `<-` (o'zgaruvchan)
- **Funksiyalar:** `fn`, bir qatorli `->`, lambda `\x ->`, closure, `ret`, oxirgi-ifoda qaytarish, rekursiya
- **Boshqaruv:** `if`/`elif`/`else`, `each` (list/map/range/str), `skip`/`stop`, `match` (symbol/son/`_`)
- **Operatorlar:** arifmetik (`+ - * / %`), taqqoslash, mantiqiy (`& | !`), `??`, `|>`, `..`, member/indeks (`.` `[]`)
- **String interpolatsiya:** `"$x"`, `"${expr}"`
- **List metodlari:** `len push has filter map reduce slice join`
- **Map metodlari:** `len has keys vals set del` + spread `{...m}` + dinamik kalit `{[k]:v}`
- **Yadro modullari:** `str` (len up low slice split has int str), `math` (floor ceil abs round), `rand` (int str), `json` (enc dec), `time`, `env`, `io`, `fs`, `sh`
- **Batareyalar:** `http`, `db`, `ai`, `auth`, `ws`, `cron`, `queue`, `reg` — `use <nom>` bilan ulanadi
- **`log`** — stderr'ga chiqarish
- **Xatolar:** `fail [status] "..."`, `!` (propagate o'tkazgich)

`tbl` schema `db` battery tomonidan o'qiladi — `CREATE TABLE IF NOT EXISTS`
auto-migration va ustun tip konversiyasi uchun ishlatiladi.

### `http` battery (server + klient)

```flux
use http
http.on :get "/health" \req -> rep 200 {ok:true}
http.on :get "/notes/:id" \req -> rep 200 {id:req.params.id}
http.on :post "/notes" \req -> rep 201 {received:req.body}
http.serve 8080
```

- `http.on :metod "/yo'l" handler` — marshrut. `:get :post :put :del`. Yo'lda
  `:id` — parametr (`req.params.id`).
- `req` map: `method path query{} headers{} params{} body`. `Content-Type:
  application/json` bo'lsa `body` avtomat map'ga dekod bo'ladi.
- `rep status body` — javob. body map/list bo'lsa avtomat JSON, str bo'lsa matn.
- `fail status "msg"` — handler ichida xato javob (`{"error":"msg"}` + status).
- `http.serve port` — serverni **bloklab** ishga tushiradi. Ixtiyoriy opsiya:
  `http.serve port {max_body: BAYT}` — so'rov tanasi o'lcham chegarasi (default
  10 MiB, oshsa `413`; `max_body: 0` — cheklovsiz).
- Klient: `http.get url`, `http.post url body`, `http.put url body`,
  `http.del url` (body map -> JSON). Natija `{status, body}`; javob JSON
  bo'lsa `body` dekod qilinadi.
- `http.get/post/put/del` chaqiruvlari bitta global Hyper klientni qayta
  ishlatadi. Shu sabab ketma-ket yoki parallel chaqiruvlarda yangi klient har
  safar qurilmaydi, Hyper connection pool esa bir xil hostlarga ulanishlarni
  qayta ishlatadi.

**Parallellik:** server tokio + hyper ustida, har request `spawn_blocking`'da
alohida bajariladi (haqiqiy parallel). Runtime thread-safe (`Arc`/`RwLock`),
global scope `http.serve` paytida lock-free snapshot'ga muzlatiladi. Misol:
`examples/server.fx` (`curl localhost:8080/health` bilan sinaladi). Klient
API soddaligi va pool reuse uchun `examples/http_client_pool.fx` lokal serverga
ketma-ket `http.get` qiladi; fayl boshidagi `for ... & ... wait` komandasi shu
Flux klientini parallel ishga tushirib ham tekshiradi.

## Arxitektura

```
src/
  token.rs    — token tiplari (+ INDENT/DEDENT, string bo'laklari)
  lexer.rs    — manba -> tokenlar; indentatsiya -> INDENT/DEDENT
  ast.rs      — AST tugunlari
  parser.rs   — tokenlar -> AST (precedence climbing + qavssiz chaqirish)
  value.rs    — runtime qiymatlar
  interp.rs   — AST'ni walk qilib bajaruvchi (scope, control flow, chaqiruv)
  builtins.rs — yadro modullari (str/math/rand/json/time/io/fs/sh) + metodlar + `rep`
  http_mod.rs — `http` battery: server (on/serve), routing, req/rep, middleware, klient
  db_mod.rs   — `db` battery: SQLite, pool, tx, schema auto-migration
  ai_mod.rs   — `ai` battery: LLM (Anthropic Messages API)
  auth_mod.rs — `auth` battery: JWT HS256 + parol hash (argon2id)
  ws_mod.rs   — `ws` battery: websocket server, room/data
  cron_mod.rs — `cron` battery: rejalashtirilgan vazifalar
  queue_mod.rs— `queue` battery: fon ishlari navbati
  reg_mod.rs  — `reg` battery: tool registry
  serve_mod.rs— deferred serverlarni boshqarish (http/ws/cron birga)
  main.rs     — CLI + integratsiya testlari
```

Frontend (lexer/parser/AST) kelajakda bytecode VM'ga ham qayta ishlatilishi
mumkin.

## Testlar

```sh
cargo test
```

Hozir ~197 ta test bor: modul ichidagi Rust unit testlari (`builtins.rs`,
`interp.rs`, `db_mod.rs` va h.k.) + `src/main.rs::mod tests` dagi integratsiya
testlari (`.fx` kodini run qilib natijani tekshirish). Bundan tashqari
`tests-fx/` da Flux'ning o'zida yozilgan e2e testlar (`run_all.sh`).

## Keyingi qadam

Spec'dagi barcha batareyalar (`http`, `db`, `ai`, `auth`, `ws`, `cron`,
`queue`, `reg`) implementatsiya qilingan. Keyingi ishlar — mavjud
batareyalarni chuqurlashtirish (masalan `db` uchun postgres/mysql backend) va
yangi til imkoniyatlari. Naqsh uchun → [`ARCHITECTURE.md`](../ARCHITECTURE.md).
