# Resly — Multi-tenant booking + analytics backend
# O'zbek tilida Flux'da yozilgan SaaS backend
# Ishga tushirish: export AUTH_SECRET=test && flux run resly.fx
# DATABASE_URL avto topiladi

use http db auth json time

# ===== SCHEMA =====

tbl tenants
  id    serial pk
  name  str
  plan  sym
  created now

tbl resources
  id          serial pk
  tenant_id   int ref:tenants.id
  name        str
  kind        sym
  capacity    int
  price_cents money
  active      bool
  uniq(tenant_id, name)

tbl bookings
  id          serial pk
  tenant_id   int ref:tenants.id
  resource_id int ref:resources.id
  user_email  str
  status      sym
  start_at    str
  end_at      str
  guests      int
  total_cents money
  created     now
  uniq(tenant_id, resource_id, start_at, end_at)

tbl reviews
  id          serial pk
  tenant_id   int ref:tenants.id
  resource_id int ref:resources.id
  booking_id  int ref:bookings.id
  rating      int
  comment     str null
  created     now

# ===== MIDDLEWARE — JWT tekshirish va tenant isolyatsiya =====

http.before "/api/*" \req ->
  token = req.headers.authorization
  if !token
    fail 401 "Authorization header kerak"

  # Token verification
  claims = auth.verify token!

  # req.ctx ga tenant_id qo'y (barcha handler'lar foydalanadi)
  req.ctx <- {tenant_id: claims.tenant}

# ===== HELPER FUNCTIONS =====

fn extract_tenant req
  ctx = req.ctx
  ret ctx.tenant_id

# ===== ENDPOINTS: RESOURCES =====

# 1. POST /resources — yangi resurs yaratadi
http.on :post "/api/resources" \req ->
  tenant_id = extract_tenant req
  body = req.body

  # Validatsiya
  if !body.name || !body.kind || !body.capacity || !body.price_cents
    fail 422 "name, kind, capacity, price_cents kerak"

  row = db.ins "resources" {
    tenant_id: tenant_id
    name: body.name
    kind: body.kind
    capacity: body.capacity
    price_cents: body.price_cents
    active: body.active ?? true
  }

  rep 201 row

# 2. GET /resources — tenant resurslari ro'yxati, filtri bilan
http.on :get "/api/resources" \req ->
  tenant_id = extract_tenant req

  # Filtrlar
  kind_filter = req.query.kind
  active_filter = req.query.active
  min_capacity = req.query.min_capacity

  # DSL'ni dinamik tuzamiz
  where_parts <- []
  params <- {tid: tenant_id}

  where_parts <- where_parts.push "tenant_id = :tid"

  if kind_filter
    where_parts <- where_parts.push "kind = :kind"
    params.set :kind kind_filter

  if active_filter
    where_parts <- where_parts.push "active = :active"
    params.set :active (active_filter == "true")

  if min_capacity
    where_parts <- where_parts.push "capacity >= :min_cap"
    params.set :min_cap (str.int min_capacity)

  where_str = where_parts.join " and "

  rows = db.find "resources" where_str params!
  rep 200 rows

# 3. GET /resources/:id — bitta resurs
http.on :get "/api/resources/:id" \req ->
  tenant_id = extract_tenant req
  rid = str.int req.params.id

  row = db.get "resources" "id = :id and tenant_id = :tid" {id: rid tid: tenant_id}

  if !row
    fail 404 "Resurs topilmadi"

  rep 200 row

# ===== ENDPOINTS: BOOKINGS =====

# 4. POST /bookings — band qilish (race-safe transaction)
http.on :post "/api/bookings" \req ->
  tenant_id = extract_tenant req
  body = req.body

  # Validatsiya
  if !body.resource_id || !body.user_email || !body.start_at || !body.end_at || !body.guests || !body.total_cents
    fail 422 "resource_id, user_email, start_at, end_at, guests, total_cents kerak"

  # Transaction'da race-safe'ni ta'minla
  result = db.tx \->
    # Buning orasida boshqa booking istalmagan-mi?
    existing = db.find "bookings" "resource_id = :rid and tenant_id = :tid and status in (:st) and start_at < :end_at and end_at > :start_at" {
      rid: body.resource_id
      tid: tenant_id
      st: [:pending :confirmed]
      start_at: body.end_at
      end_at: body.start_at
    }!

    if existing.len > 0
      fail 409 "Resurs shu vaqtda band qilingan"

    # Booking qo'sh
    booking = db.ins "bookings" {
      tenant_id: tenant_id
      resource_id: body.resource_id
      user_email: body.user_email
      status: :pending
      start_at: body.start_at
      end_at: body.end_at
      guests: body.guests
      total_cents: body.total_cents
    }!

    ret booking

  rep 201 result

# 5. PATCH /bookings/:id/status — statusni o'zgartiradi
http.on :patch "/api/bookings/:id/status" \req ->
  tenant_id = extract_tenant req
  bid = str.int req.params.id
  body = req.body

  if !body.status
    fail 422 "status kerak"

  # Booking mavjud va tenant'niki-mi?
  booking = db.get "bookings" "id = :id and tenant_id = :tid" {id: bid tid: tenant_id}
  if !booking
    fail 404 "Booking topilmadi"

  db.up "bookings" {status: body.status} {id: bid}!

  updated = db.get "bookings" "id = :id" {id: bid}!
  rep 200 updated

# 6. GET /bookings — tenant booking'lari, filtrlar bilan
http.on :get "/api/bookings" \req ->
  tenant_id = extract_tenant req

  # Filtrlar
  status_str = req.query.status
  resource_id = req.query.resource_id
  from_date = req.query.from
  to_date = req.query.to
  limit = (str.int (req.query.limit ?? "50"))
  offset = (str.int (req.query.offset ?? "0"))

  where_parts <- ["tenant_id = :tid"]
  params <- {tid: tenant_id}

  if status_str
    # Vergul bilan ajratilgan statuslar (IN)
    status_list = str.split status_str ","
    where_parts <- where_parts.push "status in :st"
    # Symbol'larga o'girish
    st_syms <- []
    each s in status_list
      st_syms <- st_syms.push (json.dec ("\":" + str.low s + "\""))
    params.set :st st_syms

  if resource_id
    where_parts <- where_parts.push "resource_id = :rid"
    params.set :rid (str.int resource_id)

  if from_date
    where_parts <- where_parts.push "start_at >= :from"
    params.set :from from_date

  if to_date
    where_parts <- where_parts.push "start_at < :to"
    params.set :to to_date

  where_str = where_parts.join " and "

  rows = db.find "bookings" where_str params {order: :start_at limit: limit offset: offset}!

  rep 200 rows

# ===== ENDPOINTS: ANALYTICS =====

# 7. GET /stats/overview — jamiiy statistika
http.on :get "/api/stats/overview" \req ->
  tenant_id = extract_tenant req

  # Hammasi
  total = db.one "select count(*) c from bookings where tenant_id = :tid" {tid: tenant_id}!

  # Status bo'yicha
  confirmed = db.one "select count(*) c from bookings where tenant_id = :tid and status = :st" {tid: tenant_id st: :confirmed}!
  cancelled = db.one "select count(*) c from bookings where tenant_id = :tid and status = :st" {tid: tenant_id st: :cancelled}!
  pending = db.one "select count(*) c from bookings where tenant_id = :tid and status = :st" {tid: tenant_id st: :pending}!

  # Revenue (`:done` bo'yicha)
  revenue_row = db.one "select coalesce(sum(total_cents), 0) total from bookings where tenant_id = :tid and status = :st" {tid: tenant_id st: :done}!

  # O'rtacha mehmon (`:confirmed` + `:done`)
  avg_row = db.one "select coalesce(avg(guests), 0) avg_g from bookings where tenant_id = :tid and status in (:st)" {tid: tenant_id st: [:confirmed :done]}!

  # Aktiv resurslar
  active = db.one "select count(*) c from resources where tenant_id = :tid and active = true" {tid: tenant_id}!

  rep 200 {
    total_bookings: total.c
    confirmed: confirmed.c
    cancelled: cancelled.c
    pending: pending.c
    revenue_cents: revenue_row.total
    avg_guests: (str.int (math.floor avg_row.avg_g))
    active_resources: active.c
  }

# 8. GET /stats/by-resource — resurs bo'yicha
http.on :get "/api/stats/by-resource" \req ->
  tenant_id = extract_tenant req

  # Har resurs uchun qator: bookings_count, revenue, avg_rating
  rows = db.q "
    select
      r.id resource_id,
      r.name,
      count(b.id) bookings_count,
      coalesce(sum(case when b.status = 'done' then b.total_cents else 0 end), 0) revenue_cents,
      coalesce(avg(rev.rating), null) avg_rating
    from resources r
    left join bookings b on r.id = b.resource_id and r.tenant_id = b.tenant_id and b.status in ('confirmed', 'done')
    left join reviews rev on r.id = rev.resource_id and r.tenant_id = rev.tenant_id
    where r.tenant_id = $1
    group by r.id, r.name
    having count(b.id) > 0
    order by revenue_cents desc
  " [tenant_id]!

  rep 200 rows

# 9. GET /stats/daily — kunlik statistika
http.on :get "/api/stats/daily" \req ->
  tenant_id = extract_tenant req
  days = (str.int (req.query.days ?? "30"))

  rows = db.q "
    select
      date(created) day,
      count(*) bookings,
      coalesce(sum(case when status = 'done' then total_cents else 0 end), 0) revenue_cents
    from bookings
    where tenant_id = $1 and created >= datetime('now', '-' || $2 || ' days')
    group by day
    order by day asc
  " [tenant_id days]!

  rep 200 rows

# 10. GET /stats/top-customers — eng ko'p sarflagan mijozlar
http.on :get "/api/stats/top-customers" \req ->
  tenant_id = extract_tenant req
  limit = (str.int (req.query.limit ?? "10"))

  rows = db.q "
    select
      user_email,
      count(*) bookings,
      sum(total_cents) total_spent_cents
    from bookings
    where tenant_id = $1 and status = 'done'
    group by user_email
    order by total_spent_cents desc
    limit $2
  " [tenant_id limit]!

  rep 200 rows

# ===== SERVER =====

http.serve 8080
