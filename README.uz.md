<div align="center">

<!-- Emoji o'rniga website logosini ishlatish uchun quyidagi qatorni almashtiring:
     <img src="assets/logo.png" alt="Fluxon" width="120" /> -->
<h1>🌊 Fluxon</h1>

### AI-native dasturlash tili

**AI agentlar yaxshi yozadigan, va LLM'ni backend'ning birinchi darajali qismiga aylantiradigan backend tili.**

[![Build](https://github.com/fluxon-lang/fluxon/actions/workflows/ci.yml/badge.svg)](https://github.com/fluxon-lang/fluxon/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/fluxon-lang/fluxon?color=blue)](https://github.com/fluxon-lang/fluxon/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)

[**Tez boshlash**](#ornatish) · [**Hujjatlar**](docs/fluxon-human.uz.md) · [**Misollar**](examples/) · [**Spec**](docs/fluxon-agent.md) · [**Yo'l xaritasi**](docs/ROADMAP.md) · [English](README.md)

</div>

---

> **Falsafa:** *"Til AI'ga moslashadi, AI tilga emas."*

Hozirgi dasturlash tillari odamlar uchun yaratilgan. Ularda bir ishni o'nlab
yo'l bilan qilish mumkin, sintaksis qulay lekin token-isrofgar, va eng oddiy
narsa ham qo'shimcha paket talab qiladi. AI agent uchun bu — shovqin: har
"tanlov nuqtasi" potensial xato, har ortiqcha belgi sarflangan kontekst. Va
LLM chaqirish — AI davridagi backend'lar doim qiladigan ish — SDK tortish, kalit
ulash va JSON'ni qo'lda parse qilish demakdir.

**Fluxon boshqacha qurilgan** — AI nimani oson va ishonchli yozishini o'lchab,
tilni shunga moslab, va LLM'ni dependency emas, kalit so'z qilib (`ai.ask` /
`ai.json` / `ai.run`).

## Butun ilova — bitta faylda

```fx
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

Mana butun ilova. Paket o'rnatish yo'q, ulanish kodi yo'q, boilerplate yo'q.

## AI to'g'ridan-to'g'ri tilning ichida

So'rovni klassifikatsiya qiling, tilning ichidagi ishonch (confidence) ni o'qing
va shunga qarab yo'naltiring — SDK yo'q, JSON parse qilish yo'q:

```fx
use http ai

http.on :post "/triage" \req ->
  r = ai.json "bu murojaatni klassifikatsiya qil: ${req.body.text}" {topic::a urgency:int}
  if r._.conf > 0.85
    rep 200 {auto:true result:r}      # ishonch tilning ichiga qurilgan
  else
    rep 200 {auto:false review:true}  # past ishonch → odamga yuborish

http.serve 8080
```

Tool ishlatadigan agent kerakmi? `ai.run` loop'ning bitta qadamini qaytaradi va
boshqaruvni **sizga** qaytaradi — logging, narx va tasdiqlash SDK ichida emas,
sizning kodingizda qoladi.

---

## Nega Fluxon

| | |
|---|---|
| 🎯 **Bir ish = bir yo'l** | Takrorlash uchun faqat `each`. Chiqarish uchun faqat bitta usul. AI "qaysi yo'lni tanlay?" deb o'ylamaydi — tanlov yo'q, xato kam. |
| ⚡ **Kam token, lekin tushunarli** | Sintaksis qisqa, lekin shifrli emas. Kalit so'zlar to'liq (`each`, `match`, `else`) — Fluxon'ni birinchi marta ko'rgan AI ham darhol tushunadi. |
| 🔋 **Batteries included** | `http`, `db`, `ai`, `auth`, `crypto`, `ws`, `cron`, `queue`, `reg`, `sh`, `json` — hammasi tilning ichida. `npm install` yo'q. Faqat ishlatilgani binary'ga kiradi (tree-shaking). |
| 🤖 **AI — primitiv** | LLM chaqirish — kalit so'z, SDK emas. Strukturalangan natija, ishonch, token soni va narx hammasi tilning ichidan qaytadi. Provayderlar muhitdan avtomatik aniqlanadi. |

```fx
r = ai.json "buyurtmani ajrat: ${text}" {intent::a items:[{product:str qty:int}]}
if r._.conf > 0.85          # ishonch metadata tilning ichida
  log "narx: ${r._.cost} · token: ${r._.tokens}"
```

Provayderlar muhitdan avtomatik aniqlanadi (`ANTHROPIC_API_KEY` → Claude,
`OPENAI_API_KEY` → GPT) — sozlash shart emas.

---

## O'rnatish

**Linux / macOS** — bitta qator (eng so'nggi release'ni yuklab oladi,
checksum'ini tekshiradi va PATH'ga o'rnatadi):

```sh
curl -fsSL https://raw.githubusercontent.com/fluxon-lang/fluxon/master/install.sh | sh
```

**Windows** (PowerShell):

```powershell
irm https://raw.githubusercontent.com/fluxon-lang/fluxon/master/install.ps1 | iex
```

So'ng faylni ishga tushiring:

```sh
fluxon run hello.fx        # .fx faylni ishga tushirish
fluxon repl                # interaktiv REPL
fluxon --help              # barcha buyruqlar
```

<details>
<summary><b>Boshqa o'rnatish usullari</b></summary>

Aniq versiyani o'rnatish uchun `FLUXON_VERSION=v0.1.0` (Windows'da
`$env:FLUXON_VERSION`). Qo'lda yuklamoqchimisiz? Platformangiz uchun arxivni
[releases sahifasi](https://github.com/fluxon-lang/fluxon/releases)dan oling.

**Manbadan** (Rust toolchain kerak):

```sh
cd runtime
cargo run -- run examples/demo.fx
# yoki binary'ni o'rnatish:  cargo install --path runtime
```

</details>

---

## Holat — Beta

Til yadrosi va **spec'dagi barcha batareyalar** implement qilingan, **479 ta
o'tadigan test** bilan qoplangan. Runtime (Rust, tree-walking interpreter) bugun
`.fx` fayllarni ishga tushiradi, HTTP/WebSocket xizmat qiladi, ma'lumotlar
bazasi bilan ishlaydi va LLM agentlarni boshqaradi.

<details>
<summary><b>Hozir nima ishlaydi</b></summary>

- **Til yadrosi:** tiplar, bindings (`=`/`<-`), `fn`/lambda/closure,
  `if`/`each`/`match`, operatorlar, string interpolatsiya, xatolar
  (`fail`/`!`/`??`), `try`/`catch`, `par` (parallel fan-out), `|>` pipe.
- **Yadro modullari:** `str`, `math`, `rand`, `json`, `time`, `env`, `io`, `fs`,
  `sh`, darajalangan `log`, hamda `assert` + ichki `fluxon test` runner va
  interaktiv REPL.
- **Batareyalar (barchasi):** **`http`** (server + klient + middleware +
  static), **`db`** (SQLite, tranzaksiya, schema, auto-migration, query builder),
  **`ai`** (LLM — `ai.ask`/`ai.json`/`ai.run`, Anthropic + OpenAI auto-detect,
  tool-loop, confidence/token/narx metadata, retry + timeout), **`auth`** (JWT +
  parol hash), **`crypto`**, **`ws`** (websocket), **`cron`**, **`queue`**,
  **`reg`** (agentlar uchun tool registry).

CLI'da `fluxon run`, `fluxon check` (lex + parse), `fluxon test` va interaktiv
`fluxon repl` bor.

</details>

Hali yo'l xaritasida turgani (Postgres/MySQL backendlari, semantik/statik
tekshiruv, `fluxon fmt`, paketlash, LSP) [`docs/ROADMAP.md`](docs/ROADMAP.md) da
kuzatiladi.

---

## Bu til qanday dizayn qilindi

Fluxon **stress-test orqali** qurildi — taxmin bilan emas, dalil bilan:

1. **Tadqiqot** — AI qaysi kod-naqshlarni eng ishonchli va kam token bilan
   yozishini o'rgandik (deklarativ DSL'lar, canonical form, batteries).
2. **Ixtiro** — turli AI modellariga "AI uchun til ixtiro qil" topshirig'i
   berildi. Mustaqil ravishda bir nechta model bir xil g'oyalarga keldi —
   konvergensiya "to'g'ri" dizayn borligini ko'rsatdi.
3. **Sinov** — spec tilni **hech ko'rmagan** AI modellariga (opus, sonnet,
   haiku) berilib, real loyihalar yozdirildi. Har model topgan "spec bo'shliqlari"
   tilning haqiqiy kamchiligini ko'rsatdi.
4. **Sayqal** — topilgan bo'shliqlar yopildi, qayta sinaldi. Bir necha raundda
   til chuqurlashdi — kichik utilitalardan katta tizimlargacha (e-commerce,
   realtime chat).

Bu jarayon [`research/`](research/) papkasida to'liq saqlangan.

---

## Ko'rib chiqing

| Yo'l | Ichida nima bor |
|------|-----------------|
| [`docs/fluxon-agent.md`](docs/fluxon-agent.md) | AI agentlar uchun ixcham spec (~2700 token) |
| [`docs/fluxon-human.uz.md`](docs/fluxon-human.uz.md) | Odamlar uchun batafsil qo'llanma |
| [`examples/support-tickets/`](examples/support-tickets/) | AI klassifikatsiya + confidence routing |
| [`examples/ecommerce/`](examples/ecommerce/) | Katalog, savat, checkout (tranzaksiya), AI tavsiya |
| [`examples/chat/`](examples/chat/) | Realtime websocket + AI moderatsiya |
| [`research/`](research/) | Til qanday tug'ilgani — dizayn eksperimentlari |

---

## Hissa qo'shish

Fluxon ochiq manba — yordamingizni kutamiz.

- **Odam contributor'lar:** [`CONTRIBUTING.md`](CONTRIBUTING.md) — setup, build,
  test, PR jarayoni.
- **AI agentlar (Claude Code va h.k.):** [`CLAUDE.md`](CLAUDE.md) — qoidalar,
  navigatsiya, "qayer nima".
- **Runtime ichki tuzilishi:** [`ARCHITECTURE.md`](ARCHITECTURE.md).

---

## Litsenziya

[MIT](LICENSE)

<div align="center">

---

*Fluxon mavjud global dasturlash tillarini almashtirish yoki ulardan o'tib ketish
uchun yaratilmayapti. Maqsad bitta: **AI eng yaxshi biladigan va yoqtiradigan
dasturlash tili** bo'lish.*

</div>
