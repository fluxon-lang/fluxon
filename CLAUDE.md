# CLAUDE.md — Flux loyihasida ishlash qoidalari (AI agentlar uchun)

Bu fayl Claude Code va boshqa AI agentlar uchun. Loyihada o'zgarish kiritishdan
oldin **shu faylni oxirigacha o'qing**. Maqsad: yangi agent boshlang'ich
holatdan tezroq chiqib, to'g'ri joyda, to'g'ri uslubda ish qilsin.

> Odam contributor'lar uchun: [`CONTRIBUTING.md`](CONTRIBUTING.md).
> Runtime ichki tuzilishi: [`ARCHITECTURE.md`](ARCHITECTURE.md).

---

## 0. Bu loyiha nima

**Flux** — AI agentlar yaxshi yozadigan backend dasturlash tili. Falsafa:
*"Til AI'ga moslashadi, AI tilga emas."* Bir ish = bir yo'l (canonical form),
kam token, batteries-included (`http`/`db`/`ai`/`ws`/...).

Bu repo ikki qismdan iborat:

- **`runtime/`** — tilning interpretatori (Rust, tree-walking). **Asosiy ish shu
  yerda.** Hamma kod, test, build shu papkada.
- **`docs/` + `examples/` + `research/`** — til spetsifikatsiyasi, misollar va til
  qanday dizayn qilingani (eksperimentlar).

---

## 1. Til (MUHIM): hamma narsa o'zbekcha

Bu loyiha o'zbek tilida olib boriladi. **Doim o'zbek tilida yozing:**

- Kod izohlari (`// ...`, `# ...`)
- Commit xabarlari
- PR sarlavha va tavsiflari
- Hujjatlar
- Foydalanuvchi bilan muloqot

Texnik atamalar va kod identifikatorlari (`HashMap`, `eval_call`, `db.tx`) asl
holida qoladi. O'zbek tilining barcha diakritik belgilarini to'g'ri yozing
(`o'`, `g'`, `'`) — ASCII bilan almashtirmang.

---

## 2. Qayerda nima (navigatsiya)

Yangi battery yoki o'zgarish kiritishdan oldin tegishli faylni biling:

| Vazifa | Fayl |
|--------|------|
| Token turlari, `Token.spaced` flag | `runtime/src/token.rs` |
| Lexer: INDENT/DEDENT, string interpolatsiya | `runtime/src/lexer.rs` |
| AST tugunlari (`Stmt`, `Expr`) | `runtime/src/ast.rs` |
| Parser: precedence climbing, qavssiz chaqirish | `runtime/src/parser.rs` |
| Qiymat turlari (`Value`, `NativeFn`) | `runtime/src/value.rs` |
| Interpreter: scope, control flow, dispatch | `runtime/src/interp.rs` |
| Yadro modullari (`str/math/rand/json/time`) | `runtime/src/builtins.rs` |
| `http` battery (server + klient) | `runtime/src/http_mod.rs` |
| `db` battery (SQLite, tx, schema) | `runtime/src/db_mod.rs` |
| CLI kirish nuqtasi + integratsiya testlari | `runtime/src/main.rs` |

**Spec'ni o'qish kerak bo'lsa:** `docs/flux-agent.md` (~2700 token, ixcham —
til qanday ishlashini AI uchun yozilgan). Batafsil: `docs/flux-human.md`.

**Battery qo'shish/o'zgartirish naqshi** uchun → [`ARCHITECTURE.md`](ARCHITECTURE.md)
ning "Yangi battery qo'shish" bo'limi. Naqsh `http_mod.rs` va `db_mod.rs` da
allaqachon mavjud — ularni namuna sifatida o'qing.

---

## 3. Build, test, ishga tushirish

**Hamma `cargo` buyruqlari `runtime/` ichida ishlaydi** (root'da emas):

```sh
cd runtime
cargo build                          # qurish
cargo test                           # barcha testlar (hozir 26 ta)
cargo run -- run examples/demo.fx    # bir .fx faylni ishga tushirish
cargo fmt                            # formatlash
cargo clippy --all-targets -- -D warnings   # lint (0 warning bo'lsin)
```

`.fx` misollar `runtime/examples/` da. HTTP/WS server misollari (`server.fx`)
portni ochib **bloklaydi** — smoke-test uchun `demo.fx` ishlating.

---

## 4. PR yashil bo'lishi uchun nima kerak

CI (`.github/workflows/ci.yml`) ubuntu + macOS da ishlaydi. Commit qilishdan
oldin **mahalliy ravishda quyidagilarni tekshiring:**

1. `cargo build --locked` — kompilyatsiya bo'ladi
2. `cargo test --locked` — barcha testlar yashil
3. `cargo fmt --check` — formatlanган
4. `cargo clippy --all-targets -- -D warnings` — 0 warning
5. `cargo run -- run examples/demo.fx` — smoke-test ishlaydi

> `build-test` job **majburiy** (qizil bo'lsa merge yo'q). `lint` job hozircha
> non-blocking, lekin **yangi kod 0 warning** bilan kelishi kutiladi — eski
> kelishuvni buzmang.

Har yangi xulq-atvor uchun **test yozing**. Test konvensiyasi:

- **Native (Rust) testlar** — tegishli modul ichida `#[cfg(test)] mod ...`
  (`builtins.rs`, `interp.rs`, `db_mod.rs`).
- **Integratsiya testlari** (`.fx` kodini run qilib natijani tekshirish) —
  `main.rs` ning `mod tests` ichida. `run(src)` yordamchisidan foydalaning.
- **DB testlari** global `DB_TEST_LOCK` mutex bilan serializatsiya qilinadi
  (`DATABASE_URL` env race oldini olish) — namunani `db_mod.rs` dan ko'ring.

---

## 5. Kod uslubi (Rust)

- **Edition 2024.** `cargo fmt` standart sozlamasi.
- Izohlar **nima uchun** (why) ni tushuntirsin, **nima** (what) ni emas —
  mavjud fayllar shu uslubda. Atrofdagi kodning izoh zichligiga moslang.
- Yangi nom va idiomalar atrofdagi kodga o'xshasin.
- `unsafe` ishlatmang. Mavjud kod butunlay xavfsiz (`db_mod.rs` connection pool
  ham `Arc` bilan, `unsafe`'siz).
- **`Value: Send + Sync` invariantini buzmang** — runtime thread-safe (har HTTP
  request alohida thread'da). Yangi qiymat turi kiritsangiz Send+Sync bo'lsin.

---

## 6. Git va commit qoidalari

- Master'ga to'g'ridan-to'g'ri **commit qilmang** — har doim branch + PR.
- Branch nomi: `battery-<nom>`, `perf-<nom>`, `docs/<nom>`, `fix-<nom>`.
- Commit xabari **o'zbekcha**, qisqa va aniq: nima o'zgardi va nega.
- Bir PR = bir mantiqiy o'zgarish. Aralashtirmang (masalan, battery + refactor).
- Foydalanuvchi so'ramaguncha `commit`/`push` qilmang.

---

## 7. Muhim invariantlar (buzmang)

Bular runtime'ning ishlashini ta'minlaydi — o'zgartirishdan oldin yaxshilab
o'ylang va test bilan himoyalang:

- **`=`/`exp`/param immutable**, `<-` mutable. Param `<-` bilan o'zgartirilishi
  mumkin (eski xatti-harakat saqlangan).
- **Closure capture, o'zaro rekursiya, shadowing, `each` loop var mutability** —
  hozir to'g'ri ishlaydi, regressiya kiritmang.
- **Scope/`Parent` enum** (`interp.rs`): top-level fn'lar `Parent::Root` marker
  ushlaydi (root Arc'ni saqlamaydi) — bu Arc contention'ni yo'qotgan optimizatsiya.
  Buni tushunmasdan o'zgartirmang → [`ARCHITECTURE.md`](ARCHITECTURE.md).
- **`http.serve`** global'ni `freeze_globals` bilan muzlatadi (lock-free snapshot).

---

## 8. Spec'da bor, lekin hali yo'q (kelajak ishlar)

`docs/flux-agent.md` da quyidagi batareyalar spetsifikatsiyalangan, lekin
runtime'da **hali implementatsiya qilinmagan:**

- `ai` (LLM primitiv — `ai.ask`/`ai.json`/`ai.run`, `$AI_KEY`)

Bularni qo'shganda spec (`docs/flux-agent.md`) ni **manba haqiqat** deb oling —
sintaksis u yerda belgilangan. Implementatsiya naqshi `http`/`db` bilan bir xil.

## 9. Kutilmagan xatolar va kamchiliklar

Hozirda `flux` ichida yetishmayotgan qismlar bo'lishi mumkin, yoki xato va kamchiliklar bo'lishi mumkin
bunday holatda chiqgan muamo haqida github issue yordamida repoga bildirish kerak bo'ladi. 
