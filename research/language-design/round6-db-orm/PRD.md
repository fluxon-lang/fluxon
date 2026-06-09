# PRD — "Resly" multi-tenant booking + analytics backend

Sen Flux tilida (`.fx`) backend yozasan. Sintaksis va batareyalar uchun berilgan
til spetsifikatsiyasiga **qat'iy amal qil** — bu o'sha tilning yagona manbai.
Faqat bitta `.fx` fayl yoz. Tashqi kutubxona yo'q — hammasi batareyalar ichida.

Bu — resurs band qilish (booking) SaaS'ining backend'i. Ko'p ijarachi
(multi-tenant): har bir so'rov `tenant_id` bo'yicha izolyatsiya qilinadi.

## Sxema (tbl bilan e'lon qil)

- **tenants**: id, name, plan (sym: `:free`/`:pro`/`:scale`), created
- **resources**: id, tenant_id, name, kind (sym: `:room`/`:desk`/`:equipment`),
  capacity (int), price_cents (money), active (bool)
- **bookings**: id, tenant_id, resource_id, user_email (str), status
  (sym: `:pending`/`:confirmed`/`:cancelled`/`:done`), start_at (now-tipida
  emas — `str` ISO vaqt sifatida sақla), end_at (str), guests (int),
  total_cents (money), created (now)
- **reviews**: id, tenant_id, resource_id, booking_id, rating (int 1..5),
  comment (str null), created (now)

`tenant_id` bo'yicha tegishli ustunlarni `uniq`/`ref` bilan to'g'ri bog'la.

## CRUD endpointlar (REST)

Hammasi `Authorization` header'dagi JWT'dan `tenant_id` oladi (middleware'da
tekshir, `req.ctx`'ga qo'y). JWT'siz so'rov → 401.

1. `POST /resources` — yangi resurs yaratadi (tenant'niki).
2. `GET /resources` — tenant resurslari ro'yxati. Query filtri:
   `?kind=room&active=true&min_capacity=4` — barchasi ixtiyoriy, birga ishlashi
   kerak.
3. `GET /resources/:id` — bitta resurs (boshqa tenant'niki → 404).
4. `POST /bookings` — band qilish. **Race-safe** bo'lishi shart: bir xil
   `resource_id` + ustma-ust vaqt oralig'ida `:pending`/`:confirmed` booking
   bo'lsa — 409 qaytar (ikki kishi bir vaqtni band qilolmaydi). Transaction
   ishlat.
5. `PATCH /bookings/:id/status` — statusni o'zgartiradi (`{status::confirmed}`).
6. `GET /bookings` — tenant booking'lari. Filtrlar (barchasi ixtiyoriy, birga):
   - `?status=pending,confirmed` — **bir nechta status** (vergul bilan; IN).
   - `?resource_id=5`
   - `?from=2026-06-01T00:00:00Z&to=2026-06-30T23:59:59Z` — start_at oralig'i.
   - Natija `start_at` bo'yicha o'sish tartibida, `?limit=` (default 50) va
     `?offset=` bilan sahifalansin.

## Analitika / statistika endpointlar (OG'IR qism)

Hammasi tenant doirasida, faqat `:done`/`:confirmed` booking'lar hisobga olinadi
(aks holda aytilgan).

7. `GET /stats/overview` — bitta JSON:
   - `total_bookings` (hammasi), `confirmed`, `cancelled`, `pending` (status
     bo'yicha sanoq).
   - `revenue_cents` — `:done` booking'lar `total_cents` yig'indisi.
   - `avg_guests` — o'rtacha mehmon soni (`:confirmed`+`:done`).
   - `active_resources` — `active=true` resurslar soni.

8. `GET /stats/by-resource` — har resurs uchun bitta qator:
   `{resource_id, name, bookings_count, revenue_cents, avg_rating}`.
   - `bookings_count` — `:done`+`:confirmed` soni.
   - `revenue_cents` — `:done` yig'indisi.
   - `avg_rating` — reviews o'rtachasi (review yo'q → null).
   - Faqat kamida bitta booking'i bor resurslar. `revenue_cents` kamayish
     tartibida.

9. `GET /stats/daily?days=30` — oxirgi N kun (default 30) uchun kunlik qator:
   `{day, bookings, revenue_cents}` — `created` sanasi bo'yicha guruhlangan,
   kun o'sish tartibida. (SQLite'da sanani `date(created)` bilan ol.)

10. `GET /stats/top-customers?limit=10` — eng ko'p sarflagan mijozlar:
    `{user_email, bookings, total_spent_cents}` — `:done` bo'yicha, `total_spent`
    kamayish tartibida, `limit` bilan.

## Talablar

- Pul doim integer (cent) — `money`/`int`, float emas.
- Status filtri `IN (...)` bilan ishlasin (xom `OR` yozma agar til imkon bersa).
- Har endpoint xato holatini to'g'ri qaytarsin (404/409/401/422).
- `http.serve 8080` bilan tugat.
- Faqat spetsifikatsiyada ko'rsatilgan sintaksisni ishlat. Shubha bo'lsa,
  spec'dagi `db` bo'limini namuna qil.

Faqat to'liq `.fx` kodni qaytar (izohlar bilan), boshqa hech narsa yozma.
