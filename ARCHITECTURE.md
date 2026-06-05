# Flux runtime — arxitektura

Bu hujjat `runtime/` ichidagi interpretator qanday qurilganini tushuntiradi.
Contributor (odam yoki AI) yangi imkoniyat qo'shishdan oldin shu yerni o'qisin.

> Tilning **o'zi** qanday ishlashi (sintaksis/semantika): `docs/flux-agent.md`
> (ixcham) yoki `docs/flux-human.md` (batafsil). Bu hujjat — **implementatsiya**
> haqida.

---

## 1. Umumiy ko'rinish

Flux **tree-walking interpreter** — AST'ni to'g'ridan-to'g'ri yuradi (bytecode/VM
yo'q). Rust edition 2024 da yozilgan. Pipeline:

```
manba (.fx)
  → token.rs        token turlari + Token.spaced flag
  → lexer.rs        belgilarni token'ga; INDENT/DEDENT, string interpolatsiya
  → ast.rs          AST tugunlari: Stmt, Expr
  → parser.rs       precedence climbing + qavssiz chaqirish (juxtaposition)
  → value.rs        runtime qiymatlari: Value, NativeFn
  → interp.rs       AST'ni yurish: scope, control flow (Flow enum), dispatch
  → builtins.rs     yadro modullari (str/math/rand/json/time) + list/map metodlari
```

Batareyalar (`http`, `db`) alohida modullarda (`http_mod.rs`, `db_mod.rs`) va
`interp.rs` dagi dispatch nuqtasidan ulanadi.

CLI kirish: `runtime/src/main.rs` → `flux run fayl.fx`.

---

## 2. Frontend (lexer / parser)

Flux grammatikasi ixcham bo'lgani uchun ikki nozik joy bor — yangi sintaksis
qo'shsangiz bularni esda tuting:

### 2.1 INDENT/DEDENT (lexer.rs)

Bloklar `{}` emas, **chekinish** (2 bo'shliq) bilan. Lexer Python kabi
INDENT/DEDENT token chiqaradi. **Muhim tuzatish:** ko'p qatorli blok-lambda
(`\req ->\n  ...`) dan keyin DEDENT'lardan so'ng `Newline` push qilinadi —
aks holda keyingi qator oldingi qavssiz-chaqiruvning argumenti deb yutiladi
(`emit_indentation` ichida).

### 2.2 `:` noaniqligi

`key:val` (Colon ajratuvchi) vs `:sym` (symbol). **Qoida:** `:` oldingi atomga
(ident/son/`)`/`]`/`"`) yopishgan bo'lsa → Colon, aks holda → Sym.
`status::open` → Colon + Sym.

### 2.3 Qavssiz chaqirish (juxtaposition) — `no_app` flag

`f a b` qavssiz chaqiruv. Lekin list/map literal ichida bu o'chiriladi (`no_app`
bayrog'i): `[a b]` ikki element, `f` chaqiruvi emas. Chaqiruv kerak bo'lsa qavs:
`{a:(f x)}`.

### 2.4 `Token.spaced` flag

`arr[i]` (tutash `[` → indekslash) vs `f "x" [a]` (bo'shliqli `[` → alohida
list argument). `parse_postfix` da: `Tok::LBracket if !self.spaced()` → indeks.
Bu `db.one "sql" [params]` spec sintaksisini ishlatadi.

> Yangi sintaksis qo'shganda: avval `token.rs`/`lexer.rs`, keyin `ast.rs`,
> keyin `parser.rs`. Test'ni `main.rs::mod tests` ga integratsiya sifatida yozing.

---

## 3. Interpreter (interp.rs)

### 3.1 Scope va `Parent` enum (perf-kritik)

Scope `Env = Arc<RwLock<Scope>>` (parking_lot RwLock — parallel read).
`Scope.vars` — `Vec<(Box<str>, Value, bool)>` (bool = mutable), HashMap emas:
fn/blok scope'lari 0-4 nom ushlaydi, linear scan ikki HashMap'dan tez.

**`Parent` enum — Arc contention optimizatsiyasi (buzmang):**

```rust
enum Parent { None, Root, Scope(Env) }
```

Top-level fn'lar root Arc'ni **saqlamaydi**, faqat `Parent::Root` **marker**
ushlaydi. `lookup` `Parent::Root` da: muzlatilgan (`freeze_globals`) bo'lsa
lock-free snapshot'dan o'qiydi, aks holda `self.global.clone()`. Natija: 8 thread
bitta root cache line'da urishmaydi → manfiy scaling musbatga aylandi.

> Bu tarix: avval har fn chaqiruv root Arc refcount'ini atomik klonlardi → 8
> thread'da `Arc::drop_slow` + `lock_shared_slow` contention. Tushunmasdan
> `Parent`'ni `Option<Env>`'ga qaytarmang — regressiya.

### 3.2 Control flow — `Flow` enum

Erta chiqish (`ret`, `skip`, `stop`, `fail`, `!`) Rust `Result`/`Flow` enum
orqali yuqoriga uzatiladi (`EvalResult`). `fail` → `Flow::Fail` → HTTP javobda
JSON xatoga aylanadi.

### 3.3 Dispatch — battery'lar qayerga ulanadi

`eval_call` (`interp.rs`) — modul nomini ko'rib yo'naltiradi:

```rust
// interp.rs::eval_call ichida (taxminiy):
if modname == "http" { return self.arc_self().http_dispatch(name, argv); }
if modname == "db"   { return self.arc_self().db_dispatch(name, argv); }
if is_module(modname) { return call_module(modname, name, argv); }  // str/math/...
```

`arc_self()` — `&self` dan `Arc<Interp>` tiklaydi (`this: OnceLock<Weak<Interp>>`
orqali). Bu spawn_blocking'da `Interp`'ni thread'larga uzatish uchun kerak.

**Argument'siz modul funksiyasi** (`time.now`) parser'da `Call` emas, `Field`
bo'lib keladi. `Expr::Field` handler'da `is_module(id) && lookup(id).is_err()`
bo'lsa argument'siz `call_module(id, name, vec![])` chaqiriladi.

### 3.4 `tbl` schema registry

`Stmt::Tbl` → `register_tbl` (schema'ni `Interp.schema` ga yozadi). `run()` da
`FnDecl` va `Tbl` **hoisting** qilinadi (oldindan ro'yxatdan o'tadi). Schema
`db` battery uchun: `sym`/`json` ustun konversiyasi, auto-migration.

---

## 4. Batareyalar

### 4.1 `http` (http_mod.rs)

- Server: tokio + hyper 1.x. Har request **`spawn_blocking`** ichida bajariladi
  → Flux'ning sinxron interpretatori tokio worker'larini bloklamaydi, real
  parallel ishlaydi (`Value: Send+Sync` shuni ta'minlaydi).
- `http.on :method "/path/:id" \req -> ...` — Route/Seg, `match_route`.
- `rep status body` — `{__resp:true status body}` map (builtins).
- Klient: `http.get/post/put/del` — pooled hyper Client.
- `http.serve port` global'ni `freeze_globals` bilan muzlatadi (lock-free).

### 4.2 `db` (db_mod.rs)

- **`Db` trait orqasiga yashiringan.** Flux kodi (`db.*`) hech qachon
  o'zgarmaydi; backend `$DATABASE_URL` sxemasidan tanlanadi (`sqlite:`/`postgres:`
  /`mysql:`). Default **SQLite** (`rusqlite` bundled — server kerak emas).
- `postgres`/`mysql` hozir `Err` stub — keyin `open_from_env` da **additiv**
  ulanadi. Agent boshqa db uchun alohida paketda chalkashmaydi.
- Connection **pool** (`Mutex<Vec<Connection>>`). Tx alohida connection oladi →
  tx davomida boshqa so'rovlar bloklanmaydi.
- `db.tx \-> ...` — `BEGIN IMMEDIATE` (race-safe). Nested tx → SAVEPOINT.
  `fail`/`!` → rollback.
- `tbl` → `CREATE TABLE IF NOT EXISTS` **auto-migration** (zero-setup).

---

## 5. Yangi battery qo'shish (retsept)

Eng ko'p takrorlanadigan contributor ishi. `http_mod.rs`/`db_mod.rs` naqshini
takrorlang:

1. **Spec'ni o'qing.** `docs/flux-agent.md` da battery sintaksisi belgilangan —
   bu **manba haqiqat**. O'zingizdan sintaksis o'ylab topmang.
2. **Yangi modul fayli** yarating: `runtime/src/<nom>_mod.rs`. Ichida:
   `impl Interp { fn <nom>_dispatch(&self, func: &str, args: Vec<Value>) -> ... }`
   va har funksiya uchun yordamchi.
3. **`main.rs`** ga `mod <nom>_mod;` qo'shing.
4. **Dispatch ulang** (`interp.rs::eval_call`): `http`/`db` qatorida
   `if modname == "<nom>" { return self.arc_self().<nom>_dispatch(name, argv); }`.
   Argument'siz funksiya bo'lsa (`time.now` kabi) `Expr::Field` handler'iga ham.
   Toza yadro moduli bo'lsa (IO'siz, `str`/`math` kabi) — `builtins.rs`
   `is_module`/`call_module` ga qo'shish yetadi.
5. **Dependency** kerak bo'lsa `Cargo.toml` ga qo'shing. Izoh bilan **nega**
   kerakligini yozing (mavjud deps shunday izohlangan).
6. **Test:** native test modul ichida + integratsiya testi `main.rs::mod tests`.
   IO/server bo'lsa, real ishga tushirib tekshiring.
7. **`Value: Send + Sync` ni buzmang.** Yangi qiymat turi kiritsangiz Send+Sync.

> Eslatma: dependency "yo'q" qoidasi faqat **Flux tili foydalanuvchisiga**
> taalluqli (ular `npm install` qilmaydi). Runtime ICHIDA Rust crate'lar OK.

---

## 6. Test strategiyasi

Ikki qatlam:

- **Rust unit/integratsiya testlari** (`cargo test`) — modul ichidagi
  `#[cfg(test)]` + `main.rs::mod tests` (`.fx` kodini run qilib natija tekshirish,
  `run(src)` yordamchisi). DB testlari `DB_TEST_LOCK` bilan serializatsiya.
- **`.fx` e2e testlari** (`runtime/tests-fx/`, PR #10 — master'ga kirmoqda) —
  Flux'ning **o'zida** yozilgan, foydalanuvchi nuqtai-nazaridan tasdiqlovchi
  testlar. `run_all.sh` bilan ishga tushadi. Yangi battery qo'shsangiz shu uslubda
  `NN_*.fx` fayl qo'shing.

---

## 7. Spec'da bor, hali yo'q

`ai`, `reg`, `ws`, `cron`, `queue` — `docs/flux-agent.md` da spetsifikatsiyalangan,
implementatsiya kutilmoqda. Tavsiya etilgan tartib va naqsh CLAUDE.md §8 da.
