# Flux Runtime

Flux tilining interpretatori (Rust, tree-walking). **Til yadrosi** to'liq
ishlaydi, va birinchi battery — **`http`** (server + klient) — qo'shildi.
Qolgan batteries (`db`, `ai`, `ws`, `cron`, `queue`) hali yo'q.

## Qurish va ishga tushirish

```sh
cargo build --release
cargo run -- run examples/demo.fx
# yoki
./target/release/flux run examples/demo.fx
```

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
- **Modullar:** `str` (len up low slice split has int str), `math` (floor ceil abs round), `rand` (int str), `json` (enc dec)
- **`log`** — stderr'ga chiqarish
- **Xatolar:** `fail [status] "..."`, `!` (propagate o'tkazgich)

`tbl` parse qilinadi, lekin hali e'tiborsiz qoldiriladi (`db` battery uchun).

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
- `http.serve port` — serverni **bloklab** ishga tushiradi.
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
  builtins.rs — yadro funksiyalari (modullar + qiymat metodlari + `rep`)
  http_mod.rs — `http` battery: server (on/serve), routing, req/rep, klient
  main.rs     — CLI + integratsiya testlari
```

Frontend (lexer/parser/AST) kelajakda bytecode VM'ga ham qayta ishlatilishi
mumkin.

## Testlar

```sh
cargo test
```

7 ta integratsiya testi `src/main.rs` ichida (fib, list metodlari, map, mutable
each, match, str/modullar, pipe/coalesce).

## Keyingi qadam

Keyingi battery — **`db`** (haqiqiy SQLite/Postgres, `tbl` schema'ni o'qish,
`db.q/one/ins/up/del/put`, `db.tx`). Keyin `ai`, `ws`, `cron`, `queue`.
