# Flux loyihasiga hissa qo'shish

Rahmat! Flux ochiq manba va hissa qo'shuvchilarni kutamiz. Bu hujjat boshlash
uchun kerak bo'lgan hamma narsani beradi.

> AI agent (Claude Code va h.k.) bilan ishlasangiz — qoidalar va navigatsiya
> [`CLAUDE.md`](CLAUDE.md) da. Runtime ichki tuzilishi: [`ARCHITECTURE.md`](ARCHITECTURE.md).

---

## Til: o'zbekcha

Bu loyiha **o'zbek tilida** olib boriladi. Kod izohlari, commit xabarlari, PR
tavsiflari va hujjatlar o'zbekcha bo'lsin. Texnik atamalar va kod nomlari
(`HashMap`, `db.tx`) asl holida qoladi. Diakritik belgilarni to'g'ri yozing
(`o'`, `g'`).

---

## Talablar

- **Rust** (stable, edition 2024) — [rustup.rs](https://rustup.rs) orqali o'rnating.
- `git`.
- Boshqa hech narsa kerak emas: SQLite **bundled** (tizim kutubxonasi shart emas),
  HTTP/server deps `cargo` bilan keladi.

---

## Tez boshlash

```sh
git clone <repo-url>
cd flux-lang/runtime          # MUHIM: hamma cargo buyrug'i shu yerda

cargo build                   # qurish
cargo test                    # testlar (hozir 26 ta)
cargo run -- run examples/demo.fx   # bir .fx faylni ishga tushirish
```

Repo tuzilishi:

```
flux-lang/
├── runtime/          interpretator (Rust) — ASOSIY ISH SHU YERDA
│   ├── src/          manba kod
│   └── examples/     .fx misollari
├── docs/             til spetsifikatsiyasi (flux-agent.md, flux-human.md)
├── examples/         real loyiha misollari (chat, ecommerce, support-tickets)
└── research/         til qanday dizayn qilingani
```

---

## Ish jarayoni

1. **Branch oching** master'dan. Nom: `battery-<nom>`, `fix-<nom>`,
   `perf-<nom>`, `docs/<nom>`.
2. O'zgarish kiriting + **test yozing** (har yangi xulq uchun).
3. Mahalliy tekshiring (pastdagi "PR tayyorligi" ro'yxati).
4. Commit qiling (o'zbekcha xabar) → PR oching.
5. CI yashil bo'lsin. Review'dan keyin merge qilinadi.

Bir PR = bir mantiqiy o'zgarish. Battery + refactor'ni aralashtirmang.

---

## PR tayyorligi (commit oldidan tekshiring)

`runtime/` ichida:

```sh
cargo build --locked                          # 1. kompilyatsiya
cargo test --locked                           # 2. testlar yashil
cargo fmt --check                             # 3. formatlangan
cargo clippy --all-targets -- -D warnings     # 4. 0 warning
cargo run -- run examples/demo.fx             # 5. smoke-test
```

CI (`.github/workflows/ci.yml`) ubuntu + macOS da shularni tekshiradi:

- **`build-test` job — MAJBURIY.** Qizil bo'lsa merge yo'q.
- **`lint` job** (fmt + clippy) — hozircha non-blocking, lekin **yangi kod 0
  warning** bilan kelishi kutiladi. Eski toza holatni buzmang.

---

## Test yozish

Ikki xil test (batafsil → [`ARCHITECTURE.md`](ARCHITECTURE.md) §6):

- **Rust testlari** — modul ichida `#[cfg(test)] mod ...`, yoki `.fx` kodini run
  qilib natijani tekshiruvchi integratsiya testi `main.rs::mod tests` da
  (`run(src)` yordamchisi).
- **`.fx` e2e testlari** — `runtime/tests-fx/` (Flux'ning o'zida yozilgan,
  `run_all.sh` bilan ishga tushadi). Yangi battery qo'shsangiz shu uslubda.

DB testlari global `DB_TEST_LOCK` mutex bilan serializatsiya qilinadi —
namunani `db_mod.rs` dan ko'ring.

---

## Kod uslubi

- `cargo fmt` standart sozlamasi (edition 2024).
- Izohlar **nega** (why) ni tushuntirsin, **nima** (what) ni emas. Atrofdagi
  kodning uslubiga moslang.
- `unsafe` ishlatmang.
- `Value: Send + Sync` invariantini buzmang (runtime thread-safe).
- Muhim perf/semantik invariantlar [`CLAUDE.md`](CLAUDE.md) §7 da — buzmang.

---

## Nimadan boshlash

- **Yangi battery** (`ai`, `reg`, `ws`, `cron`, `queue`) — spec'da bor, hali
  yo'q. Retsept: [`ARCHITECTURE.md`](ARCHITECTURE.md) §5. `http`/`db` namuna.
- **Misollar/hujjat** yaxshilash.
- **Bug fix** — avval qayta ishlab chiqaradigan test yozing.

Katta o'zgarish boshlashdan oldin issue oching — yo'nalishni kelishib olamiz.

---

## Xulq-atvor

Hurmatli va konstruktiv bo'ling. Savol bering, kichik PR'lar yuboring, bir-biringizga
yordam bering. Bu birga qurilayotgan til.
