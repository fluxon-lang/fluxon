use http db auth json time

# Sxema — multi-tenant booking tizimi
tbl tenants
  id    serial pk
  name  str
  plan  sym
  created now

tbl resources
  id           serial pk
  tenant_id    int ref:tenants.id
  name         str
  kind         sym
  capacity     int
  price_cents  money
  active       bool

tbl bookings
  id           serial pk
  tenant_id    int ref:tenants.id
  resource_id  int ref:resources.id
  user_email   str
  status       sym
  start_at     str
  end_at       str
  guests       int
  total_cents  money
  created      now

tbl reviews
  id           serial pk
  tenant_id    int ref:tenants.id
  resource_id  int ref:resources.id
  booking_id   int ref:bookings.id
  rating       int
  comment      str
  created      now

# Middleware — JWT auth'ni tekshir, tenant_id'ni ctx'ga qo'y
http.before "*" \req ->
  auth_header = req.headers.authorization
  if !auth_header
    fail 401 "authorization kerak"

  # "Bearer <token>" formatini parse qil
  parts = str.split auth_header " "
  if (parts.len) != 2 | parts.0 != "Bearer"
    fail 401 "invalid authorization format"

  token = parts.1
  claims = auth.verify token!

  # claims'dan tenant_id olish
  tenant_id = claims.tenant_id
  if !tenant_id
    fail 401 "tenant_id yo'q"

  req.ctx <- {tenant_id: tenant_id}

# ========== RESOURCES ENDPOINTS ==========

# POST /resources — yangi resurs yaratadi
http.on :post "/resources" \req ->
  ctx = req.ctx
  body = req.body

  # Validatsiya
  if !(body.name) | !(body.kind) | !body.capacity | !body.price_cents
    fail 422 "name, kind, capacity, price_cents kerak"

  new_res = db.ins "resources" {
    tenant_id: ctx.tenant_id
    name: body.name
    kind: body.kind
    capacity: body.capacity
    price_cents: body.price_cents
    active: body.active ?? true
  }

  rep 201 new_res

# GET /resources — tenant resurslari ro'yxati (query filtrlari)
http.on :get "/resources" \req ->
  ctx = req.ctx

  # Filtrlarni olish
  kind_filter = req.query.kind
  active_filter = req.query.active
  min_capacity = req.query.min_capacity

  # Query builder'ni boshlash
  q = db.from "resources"
    |> db.eq {tenant_id: ctx.tenant_id}

  # Kind filtri (ixtiyoriy)
  if kind_filter
    q = q |> db.eq {kind: kind_filter}

  # Active filtri (ixtiyoriy, string "true"/"false" bo'ladi)
  if active_filter
    active_bool = if active_filter == "true" true else false
    q = q |> db.eq {active: active_bool}

  # Min capacity filtri (ixtiyoriy)
  if min_capacity
    cap_int = str.int min_capacity
    q = q |> db.cmp :capacity :ge cap_int

  resources = q |> db.all

  rep 200 resources

# GET /resources/:id — bitta resurs
http.on :get "/resources/:id" \req ->
  ctx = req.ctx
  res_id = str.int req.params.id

  res = db.from "resources"
    |> db.eq {id: res_id tenant_id: ctx.tenant_id}
    |> db.first

  if !res
    fail 404 "resurs topilmadi"

  rep 200 res

# ========== BOOKINGS ENDPOINTS ==========

# POST /bookings — yangi booking (race-safe transaction'da)
http.on :post "/bookings" \req ->
  ctx = req.ctx
  body = req.body

  # Validatsiya
  if !body.resource_id | !body.user_email | !body.start_at | !body.end_at | !body.guests | !body.total_cents
    fail 422 "resource_id, user_email, start_at, end_at, guests, total_cents kerak"

  # Resource mavjudligini tekshir
  res = db.from "resources"
    |> db.eq {id: body.resource_id tenant_id: ctx.tenant_id}
    |> db.first

  if !res
    fail 404 "resurs topilmadi"

  # Transaction'da race-safe ish
  booking = db.tx \->
    # Bir xil vaqt oralig'idan `:pending` yoki `:confirmed` booking'larni tekshir
    conflict = db.from "bookings"
      |> db.eq {
        resource_id: body.resource_id
        tenant_id: ctx.tenant_id
        status: [:pending :confirmed]
      }
      |> db.cmp :start_at :lt body.end_at
      |> db.cmp :end_at :gt body.start_at
      |> db.first

    if conflict
      fail 409 "vaqt oraligi band bo'lgan"

    # Yangi booking qo'sh
    new_booking = db.ins "bookings" {
      tenant_id: ctx.tenant_id
      resource_id: body.resource_id
      user_email: body.user_email
      status: :pending
      start_at: body.start_at
      end_at: body.end_at
      guests: body.guests
      total_cents: body.total_cents
    }

    ret new_booking

  rep 201 booking

# PATCH /bookings/:id/status — statusni o'zgartir
http.on :patch "/bookings/:id/status" \req ->
  ctx = req.ctx
  booking_id = str.int req.params.id
  body = req.body

  if !body.status
    fail 422 "status kerak"

  # Booking'ni tekshir
  booking = db.from "bookings"
    |> db.eq {id: booking_id tenant_id: ctx.tenant_id}
    |> db.first

  if !booking
    fail 404 "booking topilmadi"

  # Statusni o'zgartir
  updated = db.up "bookings"
    {status: body.status}
    {id: booking_id}

  rep 200 updated

# GET /bookings — tenant booking'lari (kompleks filtrlar)
http.on :get "/bookings" \req ->
  ctx = req.ctx

  # Filtrlarni olish
  status_filter = req.query.status
  resource_id = req.query.resource_id
  from_date = req.query.from
  to_date = req.query.to
  limit_str = req.query.limit ?? "50"
  offset_str = req.query.offset ?? "0"

  limit_val = str.int limit_str
  offset_val = str.int offset_str

  # Query'ni boshlash
  q = db.from "bookings"
    |> db.eq {tenant_id: ctx.tenant_id}

  # Status filtri — vergul-ajratilgan statust'lar (IN)
  if status_filter
    # status_filter "pending,confirmed" bo'lsa, statust'lar listi yaratish
    # Murakkab: split + map + har biri sym
    # Sodda yondashish: raw db.q'dan foydalanish
    # Lekin spetsifikatsiya db.from|db.eq orqali bildirir, shuning uchun qilinadi:
    statuses = str.split status_filter ","
    status_syms = []
    each s in statuses
      trimmed = str.slice s 0 (str.len s)
      status_syms = status_syms.push (if trimmed == "pending" :pending
                                      elif trimmed == "confirmed" :confirmed
                                      elif trimmed == "cancelled" :cancelled
                                      elif trimmed == "done" :done
                                      else nil)
    # SQLite LIST support uchun status_syms list qilib yuborish
    q = q |> db.eq {status: status_syms}

  # Resource_id filtri
  if resource_id
    res_id = str.int resource_id
    q = q |> db.eq {resource_id: res_id}

  # Start_at oraligi
  if from_date
    q = q |> db.cmp :start_at :ge from_date

  if to_date
    q = q |> db.cmp :start_at :lt to_date

  # Order va pagination
  q = q
    |> db.order :start_at
    |> db.limit limit_val
    |> db.offset offset_val

  bookings = q |> db.all

  rep 200 bookings

# ========== ANALYTICS ENDPOINTS ==========

# GET /stats/overview — umumiy statistika
http.on :get "/stats/overview" \req ->
  ctx = req.ctx

  # db.q raw SQL — aggregation murakkabroq (bir nechta sanoq, har xil filtrlar)
  result = db.q
    "SELECT
      COUNT(*) as total_bookings,
      SUM(CASE WHEN status = 'confirmed' THEN 1 ELSE 0 END) as confirmed,
      SUM(CASE WHEN status = 'cancelled' THEN 1 ELSE 0 END) as cancelled,
      SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END) as pending,
      SUM(CASE WHEN status = 'done' THEN total_cents ELSE 0 END) as revenue_cents,
      AVG(CASE WHEN status IN ('confirmed', 'done') THEN guests ELSE NULL END) as avg_guests
     FROM bookings
     WHERE tenant_id = $1"
    [ctx.tenant_id]

  # Alohida query: active resources
  active_res = db.from "resources"
    |> db.eq {tenant_id: ctx.tenant_id active: true}
    |> db.all

  overview = {
    total_bookings: (result.0.total_bookings) ?? 0
    confirmed: (result.0.confirmed) ?? 0
    cancelled: (result.0.cancelled) ?? 0
    pending: (result.0.pending) ?? 0
    revenue_cents: (result.0.revenue_cents) ?? 0
    avg_guests: (result.0.avg_guests) ?? 0
    active_resources: active_res.len
  }

  rep 200 overview

# GET /stats/by-resource — har resurs uchun statistika
http.on :get "/stats/by-resource" \req ->
  ctx = req.ctx

  # Raw SQL — resurs bo'yicha agregate, reyting o'rtachasi
  result = db.q
    "SELECT
      r.id as resource_id,
      r.name,
      COUNT(DISTINCT b.id) as bookings_count,
      SUM(CASE WHEN b.status = 'done' THEN b.total_cents ELSE 0 END) as revenue_cents,
      AVG(CASE WHEN rev.rating IS NOT NULL THEN rev.rating ELSE NULL END) as avg_rating
     FROM resources r
     LEFT JOIN bookings b ON r.id = b.resource_id AND r.tenant_id = b.tenant_id
       AND b.status IN ('confirmed', 'done')
     LEFT JOIN reviews rev ON r.id = rev.resource_id AND r.tenant_id = rev.tenant_id
     WHERE r.tenant_id = $1
     GROUP BY r.id, r.name
     HAVING COUNT(DISTINCT b.id) > 0
     ORDER BY revenue_cents DESC"
    [ctx.tenant_id]

  rep 200 result

# GET /stats/daily?days=30 — kunlik statistika
http.on :get "/stats/daily" \req ->
  ctx = req.ctx
  days_str = req.query.days ?? "30"
  days = str.int days_str

  # Raw SQL — kun bo'yicha guruhlash (SQLite date function)
  result = db.q
    "SELECT
      DATE(created) as day,
      COUNT(*) as bookings,
      SUM(CASE WHEN status = 'done' THEN total_cents ELSE 0 END) as revenue_cents
     FROM bookings
     WHERE tenant_id = $1
       AND created >= datetime('now', '-' || $2 || ' days')
     GROUP BY DATE(created)
     ORDER BY day ASC"
    [ctx.tenant_id days]

  rep 200 result

# GET /stats/top-customers?limit=10 — eng ko'p sarflagan mijozlar
http.on :get "/stats/top-customers" \req ->
  ctx = req.ctx
  limit_str = req.query.limit ?? "10"
  limit = str.int limit_str

  # Raw SQL — user_email bo'yicha guruhla, `:done` filtrida
  result = db.q
    "SELECT
      user_email,
      COUNT(*) as bookings,
      SUM(total_cents) as total_spent_cents
     FROM bookings
     WHERE tenant_id = $1 AND status = 'done'
     GROUP BY user_email
     ORDER BY total_spent_cents DESC
     LIMIT $2"
    [ctx.tenant_id limit]

  rep 200 result

# Server tuning
http.serve 8080
