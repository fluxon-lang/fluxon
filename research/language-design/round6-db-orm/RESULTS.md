# Round 6 — db ORM dizayn taqqoslash (natijalar)

**Maqsad (issue #78):** `db.q`/`db.one` xom SQL o'rniga deklarativ o'qish qatlami.
Dizaynni o'zimiz tanlamadik — **5 xil dizaynni haiku agentga berib**, qaysi birini
agent bir o'qishda eng to'g'ri va kam tokenli yozishini o'lchadik.

## Metodika

- Bitta og'ir PRD (`PRD.md`): multi-tenant booking + 4 analitika endpoint
  (IN-filtr, vaqt-range, GROUP BY, sum/avg, order+limit+offset, race-safe tx).
- 5 variant — har biri uchun **to'liq `flux-agent.md`**, faqat `db` bo'limi
  almashtirilgan (`specs/flux-agent.v*.md`).
- Har variant uchun bitta **haiku** agent spec'ni o'qib `.fx` yozdi
  (`runs/*.fx`) + o'z-hisobot (qaysi qism qiyin, xom SQL'ga qochdimi).
- Bitta tahlilchi 5 natijani yonma-yon baholadi (`runs/judge.json`).

## Variantlar

| | Dizayn | Misol |
|--|--------|-------|
| v0 | baseline (xom SQL) | `db.q "select ... where status=$1 or status=$2"` |
| v1 | nested-map operator | `db.find "t" {status:[:a :b] start_at:{ge:t}}` |
| v2 | Django-suffiks | `db.find "t" {status:[:a] start_at__ge:t}` |
| v3 | string-DSL + named | `db.find "t" "status in :st" {st:[...]}` |
| v4 | pipe/builder | `db.from "t" \|> db.eq {...} \|> db.cmp :col :ge t \|> db.all` |

## Reyting (tahlilchi, 0–10)

| variant | correctness | token | readability | escape-hatch |
|---------|:--:|:--:|:--:|--|
| v0-baseline | 7.5 | **3** | 4.5 | hamma o'qish xom SQL (dizayn) |
| v1-nested-map | 7.0 | 7.5 | 8.0 | overview'ni `limit:999999` + Flux-loop bilan buzdi |
| v2-suffix | 6.0 | **8** | 7.5 | `db.agg` to'g'ri; lekin xom SQL ichiga `:sym` yozdi (BUG) |
| v3-string-dsl | 5.5 | 6.5 | 6.5 | `db.one` ga named param berdi (BUG); `/api/` prefiks PRD'dan chetladi |
| **v4-builder** | **8.5** | 7.0 | **8.5** | intizomli — xom SQL faqat JOIN/CASE/date'da, $1 to'g'ri |

**G'olib: v4-builder** (pipe/chain).

## Eng muhim topilmalar (agent xulqidan)

1. **list→IN qoidasi shart.** IN bo'lgan har variant (`v1/v2/v3/v4`) buni
   to'g'ri yozdi; IN'i yo'q v0 esa qo'lda `$N` placeholder-string yasadi — bu
   butun maydondagi eng xato-xavfli, eng ko'p-tokenli konstruksiya.

2. **Xom-SQL escape-hatch defekt manbai.** v2 va v3 xom `db.q`/`db.one` ichiga
   `:sym`/`:named` param yozdi — lekin escape-hatch faqat pozitsion `$1`. Demak
   deklarativ qatlam **umumiy holatlarni qoplashi** kerak, aks holda agent xom
   SQL'ga qochib u yerda yangi xato qiladi.

3. **Conditional aggregate yo'qligi hammani xom SQL'ga majburladi.**
   `/stats/overview` (status bo'yicha sanoq + `:done` revenue + avg_guests) —
   bironta variant ham buni faqat-deklarativ qila olmadi. v4 buni bitta xom
   `SUM(CASE WHEN...)` round-trip bilan eng aqlli yechdi; v0/v3 esa 6 ta alohida
   `db.one` qildi; v1 butun jadvalni yuklab Flux'da sanadi (eng yomon).

4. **Status-string→symbol konversiyasi og'riq nuqtasi.** v0/v2/v3 da
   `json.dec ("\":" + s + "\"")` hack, v4 da uzun `if/elif` zanjiri ko'rindi.
   `str.sym` yo'qligi sezildi.

5. **JOIN/`date()` har doim xom SQL talab qildi** — bu kutilgan va to'g'ri;
   escape-hatch shu uchun qoladi.

## Tahlilchi tavsiyasi (hybrid)

1. **O'qish:** v4 pipe-builder asosiy yuza —
   `db.from "t" |> db.eq {col:val col:[..]→IN} |> db.cmp :col :op v |> db.order :col [:desc] |> db.limit n |> db.offset n |> (db.all|db.first)`.
   `db.eq` da list→IN qoidasini saqlash.
2. **Aggregatsiya:** v2'ning deklarativ `db.agg` ergonomikasini olish +
   **conditional aggregate** (`db.count_if`/`db.sum_if` yoki `where`-scoped) —
   shunda `/stats/overview` ga xom SQL kerak bo'lmaydi.
3. **`str.sym`** (string→symbol) qo'shish — `json.dec` hack'ini yo'q qiladi.
4. **`db.q`/`db.one` yagona escape-hatch** bo'lib qoladi, **faqat pozitsion `$1`**;
   ichida `:name` token bo'lsa ogohlantirish/rad etish (v2/v3 bug klassi).

**Natija:** v4 builder (filtr/paging) + v2-uslub deklarativ agg + conditional
aggregate + `str.sym`. Bu kombinatsiya bilan AI `/bookings` va `/stats/overview`
ni **xom SQL'siz** yozadi; `db.q` faqat ko'p-jadvalli JOIN va `date()` uchun qoladi.

## Fayllar
- `PRD.md` — test task
- `variants/v*.md` — har dizaynning db bo'limi
- `specs/flux-agent.v*.md` — har variant uchun to'liq spec
- `runs/*.fx` — agent yozgan kodlar
- `runs/judge.json` — to'liq baholash
- `run-experiment.mjs` — workflow skripti (args embed'siz manba)
