# Fluxon Roadmap — haqiqiy ishlaydigan dasturlash tiliga yo'l

> Holat: 2026-yil iyun. Runtime'da 267 ta yashil test, spec'dagi barcha
> batareyalar implementatsiya qilingan. Hozirgi fokus — **Faza 0**.

Mantiq oddiy: Faza 0–1 tilni *ishonchli* qiladi, Faza 2 ishonchlilikni
*avtomatik ushlab turadi*, Faza 3 uni *foydali* qiladi, Faza 4 *tez*,
Faza 5 *ommaga ochiq*. Fazalarni qisman parallel olib borish mumkin, lekin
0–1 tugamasdan 3–5 ga kirishilmaydi — poydevordagi panic'lar ustiga
ekotizim qurib bo'lmaydi.

---

## Faza 0 — Barqarorlik: ochiq bug'larni yopish *(hozirgi faza)*

To'liq kod-revyudan chiqqan ochiq bug'lar, ahamiyat bo'yicha uch to'lqinda:

### 1-to'lqin — crash/DoS (server'ni yiqitadigan)

- [#87](https://github.com/Firdavs9512/fluxon-lang/issues/87) `json.dec` buzuq JSON'da panic — request body bilan DoS
- [#91](https://github.com/Firdavs9512/fluxon-lang/issues/91) http request body o'lcham chegarasi yo'q — xotira DoS
- [#90](https://github.com/Firdavs9512/fluxon-lang/issues/90) chuqurlik limiti yo'q — cheksiz rekursiya stack overflow abort
- [#89](https://github.com/Firdavs9512/fluxon-lang/issues/89) integer arifmetika overflow panic / jim wrap
- [#88](https://github.com/Firdavs9512/fluxon-lang/issues/88) `extract_from_table` Unicode char-boundary panic
- [#92](https://github.com/Firdavs9512/fluxon-lang/issues/92) http klient + ai timeout yo'q — handler thread abadiy qotadi

### 2-to'lqin — xavfsizlik

- [#97](https://github.com/Firdavs9512/fluxon-lang/issues/97) `rand` kriptografik emas — token/session-ID bashorat qilinadi
- [#96](https://github.com/Firdavs9512/fluxon-lang/issues/96) cross-origin redirect'da `Authorization` header begona host'ga ketadi
- [#103](https://github.com/Firdavs9512/fluxon-lang/issues/103) db tx xatosida iflos connection ROLLBACK'siz poolga qaytadi

### 3-to'lqin — jim noto'g'rilik (xato bermasdan noto'g'ri ishlaydi)

- [#94](https://github.com/Firdavs9512/fluxon-lang/issues/94) `uniq(a, b)` ko'p-ustunli cheklovni jim yo'qotadi
- [#95](https://github.com/Firdavs9512/fluxon-lang/issues/95) ai: ko'p `tool_use` blokida faqat oxirgisi qoladi
- [#93](https://github.com/Firdavs9512/fluxon-lang/issues/93) / [#98](https://github.com/Firdavs9512/fluxon-lang/issues/98) / [#99](https://github.com/Firdavs9512/fluxon-lang/issues/99) parser-lexer jim xatolari (`!x`, `m.0.1`, `1..n+1`)
- [#104](https://github.com/Firdavs9512/fluxon-lang/issues/104) `db.up` bo'sh where — malformed SQL
- [#101](https://github.com/Firdavs9512/fluxon-lang/issues/101) takror header'lar yo'qoladi
- [#105](https://github.com/Firdavs9512/fluxon-lang/issues/105) queue: handler'siz ish busy-loop, tugashda ishlar jim yo'qoladi
- [#100](https://github.com/Firdavs9512/fluxon-lang/issues/100) query string percent-decoding

**Chiqish mezoni:** ochiq `bug` label'li issue = 0, va har bir fix
regression test bilan kelgan.

---

## Faza 1 — Til yadrosini qotirish (xatosiz emas, *bashoratli* qilish)

Haqiqiy tilni o'yinchoqdan ajratadigan narsa — har qanday kirishda aniq javob:

- **Hech qachon panic qilmaslik kafolati.** Runtime'dagi har bir panic yo'li
  Fluxon-darajadagi xatoga (`err`) aylanadi. Tekshirish uchun `cargo-fuzz`
  bilan lexer / parser / `json.dec` fuzz qilinadi — #87/#88/#90 sinfidagi
  bug'larni issue kutmasdan topadi.
- **Diagnostika sifati.** Har xato satr:ustun + kod parchasi + "balki shuni
  nazarda tutdingizmi" ko'rinishida. AI agent uchun bu ayniqsa muhim — xato
  xabari qancha aniq bo'lsa, agent shuncha tez o'zini tuzatadi (tilning
  asosiy falsafasiga to'g'ri keladi).
- **Stack trace.** Runtime xatoda Fluxon-darajadagi chaqiruv zanjiri ko'rinadi.
- **Spec ↔ runtime auditi.** `docs/fluxon-agent.md` dagi har bir jumla uchun
  test bormi? Farq topilsa yo spec, yo runtime tuzatiladi
  ([#81](https://github.com/Firdavs9512/fluxon-lang/issues/81) — spec
  "Postgres" deydi, runtime SQLite — shu sinfdagi ish).
- Avvalgi real-loyiha sinovlarida topilgan til kamchiliklarini yopish:
  `str` kutubxonasi bo'shliqlari, dynamic indexing, time arifmetikasi.

---

## Faza 2 — Ishonchlilik infratuzilmasi

- **Fuzzing CI'da doimiy** (nightly job): lexer, parser, json, http request
  parsing.
- **`.fx` e2e suite kengaytirish** (`runtime/tests-fx/`) — har battery uchun
  "yomon kun" stsenariylari: tarmoq uzilishi, DB lock, katta payload.
- **Benchmark suite + regression alert** — keyinroq VM'ga o'tishda asos.
- **Dogfooding harness.** AI agentga (arzon model bilan) real backend
  topshiriqlar berib Fluxon'da yozdirish — har relizda. Bu usul shu paytgacha
  eng ko'p haqiqiy bug topgan (`research/` dagi validation-tests metodikasi).

---

## Faza 3 — Production-ready backend tili

- **Postgres** haqiqiy qo'llab-quvvatlash (hozir `Err` stub) — "backend
  tili" da'vosi uchun shart. Fluxon `db.*` kodi backend-neytral, foydalanuvchi
  kodi o'zgarmaydi.
- **Deploy hikoyasi:** bitta binary, graceful shutdown, `$PORT`/secrets
  konvensiyasi, structured logging (stdout vs stderr ajratish `io` da
  boshlangan).
- **`fluxon fmt`** — canonical form tilning falsafasi, demak formatter
  majburiy.
- **`fluxon check`** — ishga tushirmasdan parse + statik tekshiruv (AI agent
  loop'i uchun tezkor feedback).
- **Modul ekotizimi:** `use ./fayl` bor; versiyalangan paketlar o'rniga
  hozircha qat'iy "batteries-included yetadi" pozitsiyasi — bu tilning
  farqlovchi kuchi.

---

## Faza 4 — Performans

- Tree-walking interpreter'dan **bytecode VM** ga o'tish — lekin faqat
  Faza 2 benchmark'lari "qayerda sekin"ni ko'rsatgandan keyin. Arc
  contention tajribasi ko'rsatdiki, profiling'siz taxmin qilib bo'lmaydi.
- HTTP yo'lida har-request-thread o'rniga to'liq async yoki thread-pool.

---

## Faza 5 — Tarqatish va v0.1

- **Install:** `curl | sh` + Homebrew formula, GitHub Releases'da
  binary'lar.
- **Hujjatlar sayti + interaktiv playground** (WASM'ga kompilyatsiya
  qilinsa browser'da ham ishlaydi).
- **Inglizcha tarjima**
  ([#58](https://github.com/Firdavs9512/fluxon-lang/issues/58)) — tashqi
  auditoriya uchun.
- **Spec'ni versiyalash:** `fluxon-agent.md` v0.1 deb muzlatiladi, breaking
  change faqat versiya bilan. "Haqiqiy til" degani — bugun yozilgan kod
  ertaga ham ishlaydi degan va'da.
- **Editor tooling:** syntax highlighting (VS Code extension), keyin LSP.
