# Flux

**AI agentlar yaxshi yozadigan backend dasturlash tili.**

> Falsafa: *"Til AI'ga moslashadi, AI tilga emas."*

Hozirgi dasturlash tillari odamlar uchun yaratilgan. Ularda bir ishni o'nlab
yo'l bilan qilish mumkin, sintaksis qulay lekin token-isrofgar, va eng oddiy
narsa ham qo'shimcha paket talab qiladi. AI agent uchun bu — shovqin: har
"tanlov nuqtasi" potensial xato, har ortiqcha belgi sarflangan kontekst.

Flux boshqacha qurilgan — AI nimani oson va ishonchli yozishini o'lchab, tilni
shunga moslab.

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

---

## Asosiy tamoyillar

1. **Bir ish = bir yo'l (canonical form).** Takrorlash uchun faqat `each`.
   Chiqarish uchun faqat bitta usul. AI "qaysi yo'lni tanlay?" deb o'ylamaydi —
   tanlov yo'q, xato kam.

2. **Kam token, lekin tushunarli.** Sintaksis qisqa, lekin shifrli emas.
   Kalit so'zlar to'liq (`each`, `match`, `else`) — Flux'ni birinchi marta
   ko'rgan AI ham darhol tushunadi.

3. **Batteries included.** `http`, `db` (tranzaksiya + concurrency kafolati),
   `ai`, `reg` (tool registry), `ws`, `cron`, `queue`, `sh` (shell), `json` — hammasi tilning
   ichida. `npm install` yo'q. Compile vaqtida faqat ishlatilgani binary'ga
   kiradi (tree-shaking).

4. **AI — birinchi darajali primitiv.** LLM chaqirish — kalit so'z, SDK emas:
   ```fx
   r = ai.json "buyurtmani ajrat: ${text}" {intent::a items:[{product:str qty:int}]}
   if r._.conf > 0.85
     auto r          # ishonch metadata tilning ichida
   ```

---

## Bu til qanday dizayn qilindi (metodologiya)

Flux **stress-test orqali** qurildi — taxmin bilan emas, dalil bilan:

1. **Tadqiqot:** AI qaysi kod-naqshlarni eng ishonchli va kam token bilan
   yozishini o'rgandik (deklarativ DSL'lar, canonical form, batteries —
   `research/` papkasiga qarang).
2. **Ixtiro:** turli AI modellariga "AI uchun til ixtiro qil" topshirig'i
   berildi. Mustaqil ravishda bir nechta model bir xil g'oyalarga keldi —
   konvergensiya "to'g'ri" dizayn borligini ko'rsatdi.
3. **Sinov:** Flux spec'i tilni **hech ko'rmagan** AI modellariga berilib
   (opus, sonnet, haiku), real loyihalar yozdirildi. Har model topgan
   "spec bo'shliqlari" tilning haqiqiy kamchiligini ko'rsatdi.
4. **Sayqal:** topilgan bo'shliqlar yopildi, qayta sinaldi. Bir necha raundda
   til chuqurlashdi — kichik utilitalardan (URL qisqartiruvchi) katta
   tizimlargacha (e-commerce, realtime chat).

Bu jarayon `research/` papkasida to'liq saqlangan.

---

## Repo tuzilishi

```
flux-lang/
├── docs/
│   ├── flux-human.md      # batafsil qo'llanma (odamlar uchun)
│   └── flux-agent.md      # ixcham spec (AI agent uchun — ~2700 token)
├── examples/              # ishlaydigan misol loyihalar
│   ├── support-tickets/   # AI klassifikatsiya + confidence routing
│   ├── ecommerce/         # katalog, savat, checkout (tranzaksiya), AI tavsiya
│   └── chat/              # realtime websocket, AI moderatsiya
└── research/              # til qanday tug'ilgani — dizayn eksperimentlari
    └── language-design/
        ├── round1-invented-langs/   # AI'lar til ixtiro qiladi
        ├── round2-whatsapp/         # real loyiha bilan ixtiro
        └── validation-tests/        # Flux'ni toza AI'larda sinash
```

---

## Hozirgi holat

🚧 **Faol ishlab chiqilmoqda.** Til yadrosi ishlaydigan **runtime** (Rust,
tree-walking interpreter) mavjud — `.fx` fayllarni ishga tushira oladi.

**Ishlaydi:**

- Til yadrosi: tiplar, bindings (`=`/`<-`), `fn`/lambda/closure, `if`/`each`/
  `match`, operatorlar, string interpolatsiya, `fail`/`!`/`??`/`|>`.
- Yadro modullari: `str`, `math`, `rand`, `json`, `time`, `env`.
- Batareyalar: **`http`** (server + klient), **`db`** (SQLite, tranzaksiya,
  schema, auto-migration).

**Hali yo'q (spec'da bor):** `ai`, `reg`, `ws`, `cron`, `queue`.

Ishga tushirish:

```sh
cd runtime
cargo run -- run examples/demo.fx
```

---

## Hissa qo'shish

Flux ochiq manba — yordamingizni kutamiz.

- **Odam contributor'lar:** [`CONTRIBUTING.md`](CONTRIBUTING.md) — setup, build,
  test, PR jarayoni.
- **AI agentlar (Claude Code va h.k.):** [`CLAUDE.md`](CLAUDE.md) — qoidalar,
  navigatsiya, "qayer nima".
- **Runtime ichki tuzilishi:** [`ARCHITECTURE.md`](ARCHITECTURE.md).

---

## Litsenziya

MIT

---

> **Eslatma.** Flux mavjud global dasturlash tillarini almashtirish yoki
> ulardan o'tib ketish uchun yaratilmayapti. Maqsad bitta: **AI eng yaxshi
> biladigan va yoqtiradigan dasturlash tili** bo'lish.
