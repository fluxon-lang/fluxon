use http db json auth time

# ============================================================================
# SCHEMA: multi-tenant booking + analytics
# ============================================================================

tbl tenants
  id    serial pk
  name  str
  plan  sym
  created now

tbl resources
  id            serial pk
  tenant_id     int ref:tenants.id
  name          str
  kind          sym
  capacity      int
  price_cents   money
  active        bool

tbl bookings
  id            serial pk
  tenant_id     int ref:tenants.id
  resource_id   int ref:resources.id
  user_email    str
  status        sym
  start_at      str
  end_at        str
  guests        int
  total_cents   money
  created       now

tbl reviews
  id            serial pk
  tenant_id     int ref:tenants.id
  resource_id   int ref:resources.id
  booking_id    int ref:bookings.id
  rating        int
  comment       str
  created       now

# ============================================================================
# MIDDLEWARE: JWT validation, tenant isolation
# ============================================================================

http.before "*" \req ->
  auth_header = req.headers.authorization ?? ""
  if !(str.len auth_header)
    fail 401 "authorization kerak"

  # Umuman: "Bearer <token>" formatini kutamiz
  parts = str.split auth_header " "
  if (parts.len) != 2
    fail 401 "bearer token kerak"

  token = parts.1
  claims = auth.verify token!
  req.ctx <- {tenant_id: claims.sub}

# ============================================================================
# 1. POST /resources — yangi resurs yaratadi
# ============================================================================

http.on :post "/resources" \req ->
  tid = req.ctx.tenant_id
  body = req.body

  # Validate
  if !body.name | !body.kind | !body.capacity | !body.price_cents
    fail 422 "name, kind, capacity, price_cents kerak"

  r = db.ins "resources" {
    tenant_id: tid
    name: body.name
    kind: body.kind
    capacity: body.capacity
    price_cents: body.price_cents
    active: body.active ?? true
  }
  rep 201 r

# ============================================================================
# 2. GET /resources — tenant resurslari, ixtiyoriy filtrlar
# ============================================================================

http.on :get "/resources" \req ->
  tid = req.ctx.tenant_id
  q = req.query

  # Build filter map
  filter = {tenant_id: tid}

  if q.kind
    filter.set :kind (str.low q.kind)

  if q.active
    a_val = if q.active == "true" true else false
    filter.set :active a_val

  if q.min_capacity
    cap_int = str.int q.min_capacity!
    filter.set :capacity {ge: cap_int}

  rows = db.find "resources" filter
  rep 200 rows

# ============================================================================
# 3. GET /resources/:id — bitta resurs
# ============================================================================

http.on :get "/resources/:id" \req ->
  tid = req.ctx.tenant_id
  rid = str.int req.params.id!

  r = db.get "resources" {id: rid tenant_id: tid}
  if !r
    fail 404 "resurs topilmadi"
  rep 200 r

# ============================================================================
# 4. POST /bookings — band qilish (race-safe, transaction)
# ============================================================================

http.on :post "/bookings" \req ->
  tid = req.ctx.tenant_id
  body = req.body

  # Validate
  if !body.resource_id | !body.user_email | !body.start_at | !body.end_at | !body.guests | !body.total_cents
    fail 422 "resource_id, user_email, start_at, end_at, guests, total_cents kerak"

  # Transaction: race-safe overlap check
  booking = db.tx \->
    # Check: bu vaqt oralig'ida biron :pending yoki :confirmed booking bor-yo'qmi?
    conflict = db.find "bookings" {
      tenant_id: tid
      resource_id: body.resource_id
      status: [:pending :confirmed]
      start_at: {lt: body.end_at}
      end_at: {gt: body.start_at}
    } {limit: 1}

    if (conflict.len) > 0
      fail 409 "bu vaqtda allaqachon band"

    # Insert
    ret db.ins "bookings" {
      tenant_id: tid
      resource_id: body.resource_id
      user_email: body.user_email
      status: body.status ?? :pending
      start_at: body.start_at
      end_at: body.end_at
      guests: body.guests
      total_cents: body.total_cents
    }

  rep 201 booking

# ============================================================================
# 5. PATCH /bookings/:id/status — statusni o'zgartiradi
# ============================================================================

http.on :patch "/bookings/:id/status" \req ->
  tid = req.ctx.tenant_id
  bid = str.int req.params.id!
  body = req.body

  if !body.status
    fail 422 "status kerak"

  b = db.get "bookings" {id: bid tenant_id: tid}
  if !b
    fail 404 "booking topilmadi"

  db.up "bookings" {status: body.status} {id: bid tenant_id: tid}

  updated = db.get "bookings" {id: bid}
  rep 200 updated

# ============================================================================
# 6. GET /bookings — tenant booking'lari, filtrlar + pagination
# ============================================================================

http.on :get "/bookings" \req ->
  tid = req.ctx.tenant_id
  q = req.query

  filter = {tenant_id: tid}
  opts = {limit: 50 offset: 0}

  # Status filter: vergul bilan "pending,confirmed" → IN
  if q.status
    statuses = str.split q.status ","
    filter.set :status statuses

  # Resource filter
  if q.resource_id
    rid = str.int q.resource_id!
    filter.set :resource_id rid

  # Date range
  if q.from | q.to
    date_filter = {}
    if q.from
      date_filter.set :ge q.from
    if q.to
      date_filter.set :lt q.to
    filter.set :start_at date_filter

  # Pagination
  if q.limit
    opts.set :limit (str.int q.limit!)
  if q.offset
    opts.set :offset (str.int q.offset!)

  opts.set :order :start_at

  rows = db.find "bookings" filter opts
  rep 200 rows

# ============================================================================
# 7. GET /stats/overview — :done/:confirmed asos'ida
# ============================================================================

http.on :get "/stats/overview" \req ->
  tid = req.ctx.tenant_id

  # Hammasi
  all_bookings = db.find "bookings" {tenant_id: tid} {limit: 999999}
  total = all_bookings.len

  # Status bo'yicha sanoq
  confirmed <- 0
  cancelled <- 0
  pending <- 0
  done <- 0
  each b in all_bookings
    if b.status == :confirmed
      confirmed <- confirmed + 1
    elif b.status == :cancelled
      cancelled <- cancelled + 1
    elif b.status == :pending
      pending <- pending + 1
    elif b.status == :done
      done <- done + 1

  # Revenue: :done yig'indisi
  revenue_cents <- 0
  avg_guests_sum <- 0
  count_for_avg <- 0
  each b in all_bookings
    if b.status == :done
      revenue_cents <- revenue_cents + b.total_cents
    if b.status == :confirmed | b.status == :done
      avg_guests_sum <- avg_guests_sum + b.guests
      count_for_avg <- count_for_avg + 1

  avg_guests = if count_for_avg > 0 (avg_guests_sum / count_for_avg) else nil

  # Active resources soni
  active_resources = db.find "resources" {tenant_id: tid active: true}

  rep 200 {
    total_bookings: total
    confirmed: confirmed
    cancelled: cancelled
    pending: pending
    done: done
    revenue_cents: revenue_cents
    avg_guests: avg_guests
    active_resources: (active_resources.len)
  }

# ============================================================================
# 8. GET /stats/by-resource — har resurs uchun statistika
# ============================================================================

http.on :get "/stats/by-resource" \req ->
  tid = req.ctx.tenant_id

  # Raw SQL: db.find bilan uzoq bo'ladi, agg'ni ham cheklangan
  # Spec'da agg mavjud lekin multi-join uchun to'g'ri emas
  rows = db.q "
    select
      r.id resource_id,
      r.name,
      count(case when b.status in ('confirmed', 'done') then 1 end) bookings_count,
      coalesce(sum(case when b.status = 'done' then b.total_cents else 0 end), 0) revenue_cents,
      case when count(rev.id) > 0 then cast(avg(rev.rating) as int) else null end avg_rating
    from resources r
    left join bookings b on b.resource_id = r.id and b.tenant_id = r.tenant_id
    left join reviews rev on rev.resource_id = r.id and rev.tenant_id = r.tenant_id
    where r.tenant_id = $1
    group by r.id, r.name
    having count(case when b.status in ('confirmed', 'done') then 1 end) > 0
    order by revenue_cents desc
  " [tid]

  rep 200 rows

# ============================================================================
# 9. GET /stats/daily?days=30 — kunlik qatorlar
# ============================================================================

http.on :get "/stats/daily" \req ->
  tid = req.ctx.tenant_id
  q = req.query
  days_param = if q.days (str.int q.days!) else 30

  # Raw SQL: date() funksiyasi kerak, db.find bilan qiyinchilik
  rows = db.q "
    select
      date(created) as day,
      count(*) bookings,
      coalesce(sum(case when status = 'done' then total_cents else 0 end), 0) revenue_cents
    from bookings
    where tenant_id = $1
      and created >= datetime('now', '-' || $2 || ' days')
    group by date(created)
    order by day asc
  " [tid days_param]

  rep 200 rows

# ============================================================================
# 10. GET /stats/top-customers?limit=10 — eng ko'p sarflagan mijozlar
# ============================================================================

http.on :get "/stats/top-customers" \req ->
  tid = req.ctx.tenant_id
  q = req.query
  limit_param = if q.limit (str.int q.limit!) else 10

  rows = db.q "
    select
      user_email,
      count(*) bookings,
      coalesce(sum(total_cents), 0) total_spent_cents
    from bookings
    where tenant_id = $1 and status = 'done'
    group by user_email
    order by total_spent_cents desc
    limit $2
  " [tid limit_param]

  rep 200 rows

# ============================================================================
# START SERVER
# ============================================================================

http.serve 8080
