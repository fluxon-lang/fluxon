use http db auth json time

# Sxema
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
  uniq(tenant_id name)

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
  uniq(tenant_id resource_id start_at)

tbl reviews
  id          serial pk
  tenant_id   int ref:tenants.id
  resource_id int ref:resources.id
  booking_id  int ref:bookings.id
  rating      int
  comment     str null
  created     now

# Middleware: JWT tekshirish va tenant_id o'rnatish
http.before "/*" \req ->
  auth_header = req.headers.authorization ?? ""
  if !(str.has auth_header "Bearer ")
    fail 401 "authorization required"

  token = str.slice auth_header 7 (str.len auth_header)
  claims = auth.verify token!

  tenant_id = claims.tenant_id!
  req.ctx <- {tenant_id: tenant_id}

# POST /resources — yangi resurs yaratadi
http.on :post "/resources" \req ->
  ctx = req.ctx
  tid = ctx.tenant_id

  b = req.body
  name = b.name!
  kind = b.kind!
  capacity = b.capacity!
  price_cents = b.price_cents!
  active = b.active ?? true

  res = db.ins "resources" {
    tenant_id: tid
    name: name
    kind: kind
    capacity: capacity
    price_cents: price_cents
    active: active
  }

  rep 201 res

# GET /resources — tenant resurslari ro'yxati
http.on :get "/resources" \req ->
  ctx = req.ctx
  tid = ctx.tenant_id

  q = db.from "resources" |> db.eq {tenant_id: tid}

  # Query filtrlari
  kind_filter = req.query.kind
  if kind_filter != nil
    kind_sym = str.sym kind_filter
    q = q |> db.eq {kind: kind_sym}

  active_filter = req.query.active
  if active_filter != nil
    is_active = active_filter == "true"
    q = q |> db.eq {active: is_active}

  min_cap = req.query.min_capacity
  if min_cap != nil
    cap_int = str.int min_cap
    q = q |> db.cmp :capacity :ge cap_int

  rows = q |> db.all
  rep 200 rows

# GET /resources/:id — bitta resurs
http.on :get "/resources/:id" \req ->
  ctx = req.ctx
  tid = ctx.tenant_id
  rid = str.int (req.params.id!)

  row = db.from "resources"
    |> db.eq {tenant_id: tid id: rid}
    |> db.first

  if row == nil
    fail 404 "resource not found"

  rep 200 row

# POST /bookings — band qilish (race-safe)
http.on :post "/bookings" \req ->
  ctx = req.ctx
  tid = ctx.tenant_id

  b = req.body
  resource_id = b.resource_id!
  user_email = b.user_email!
  status = b.status ?? :pending
  start_at = b.start_at!
  end_at = b.end_at!
  guests = b.guests!
  total_cents = b.total_cents!

  # Resurs mavjudligini tekshir
  res = db.from "resources"
    |> db.eq {tenant_id: tid id: resource_id}
    |> db.first!

  # Race-safe: transaction ichida tekshir va yaratish
  booking = db.tx \->
    # Ustma-ust vaqt oralig'ida pending yoki confirmed booking bormi?
    conflict = db.from "bookings"
      |> db.eq {
        tenant_id: tid
        resource_id: resource_id
        status: [:pending :confirmed]
      }
      |> db.cmp :start_at :lt end_at
      |> db.cmp :end_at :gt start_at
      |> db.first

    if conflict != nil
      fail 409 "resource not available for this time"

    # Yangi booking yaratish
    new_booking = db.ins "bookings" {
      tenant_id: tid
      resource_id: resource_id
      user_email: user_email
      status: status
      start_at: start_at
      end_at: end_at
      guests: guests
      total_cents: total_cents
    }

    ret new_booking

  rep 201 booking

# PATCH /bookings/:id/status — statusni o'zgartirish
http.on :patch "/bookings/:id/status" \req ->
  ctx = req.ctx
  tid = ctx.tenant_id
  bid = str.int (req.params.id!)

  new_status = req.body.status!

  # Booking mavjudligini tekshir
  booking = db.from "bookings"
    |> db.eq {tenant_id: tid id: bid}
    |> db.first!

  # Statusni yangilash
  db.up "bookings" {status: new_status} {id: bid}

  # Yangilangan booking qaytarish
  updated = db.from "bookings"
    |> db.eq {id: bid}
    |> db.first

  rep 200 updated

# GET /bookings — tenant booking'lari
http.on :get "/bookings" \req ->
  ctx = req.ctx
  tid = ctx.tenant_id

  q = db.from "bookings" |> db.eq {tenant_id: tid}

  # Status filtri (vergul bilan)
  status_param = req.query.status
  if status_param != nil
    status_strs = str.split status_param ","
    status_syms = status_strs.map \s -> str.sym s
    q = q |> db.eq {status: status_syms}

  # Resource filtri
  resource_param = req.query.resource_id
  if resource_param != nil
    res_id = str.int resource_param
    q = q |> db.eq {resource_id: res_id}

  # Vaqt oralig'i filtri
  from_param = req.query.from
  if from_param != nil
    q = q |> db.cmp :start_at :ge from_param

  to_param = req.query.to
  if to_param != nil
    q = q |> db.cmp :start_at :lt to_param

  # Sorting, limit, offset
  q = q |> db.order :start_at

  limit_val = req.query.limit ?? "50"
  limit_int = str.int limit_val
  q = q |> db.limit limit_int

  offset_val = req.query.offset ?? "0"
  offset_int = str.int offset_val
  q = q |> db.offset offset_int

  rows = q |> db.all
  rep 200 rows

# GET /stats/overview — taniqlik statistikasi
http.on :get "/stats/overview" \req ->
  ctx = req.ctx
  tid = ctx.tenant_id

  # Barcha statuslar bo'yicha sanoq
  all_stats = db.from "bookings"
    |> db.eq {tenant_id: tid}
    |> db.count_if {status: [:pending]} :pending
    |> db.count_if {status: [:confirmed]} :confirmed
    |> db.count_if {status: [:cancelled]} :cancelled
    |> db.count_if {status: [:done]} :done
    |> db.count :total
    |> db.agg_row

  # Yo'q bo'lgan maydonlari null => 0 bilan to'ldirish
  pending = all_stats.pending ?? 0
  confirmed = all_stats.confirmed ?? 0
  cancelled = all_stats.cancelled ?? 0
  done = all_stats.done ?? 0
  total = all_stats.total ?? 0

  # Revenue: :done booking'lar yig'indisi
  rev_result = db.from "bookings"
    |> db.eq {tenant_id: tid status: [:done]}
    |> db.sum :total_cents :revenue
    |> db.agg_row
  revenue_cents = rev_result.revenue ?? 0

  # O'rtacha mehmon soni (:confirmed + :done)
  guests_result = db.from "bookings"
    |> db.eq {tenant_id: tid status: [:confirmed :done]}
    |> db.avg :guests :avg_guests
    |> db.agg_row

  avg_guests_val = guests_result.avg_guests
  avg_guests = if avg_guests_val == nil
    nil
  else
    avg_guests_val

  # Faol resurslar soni
  active_count = db.from "resources"
    |> db.eq {tenant_id: tid active: true}
    |> db.count :c
    |> db.agg_row
  active_resources = active_count.c ?? 0

  stats = {
    total_bookings: total
    pending: pending
    confirmed: confirmed
    cancelled: cancelled
    done: done
    revenue_cents: revenue_cents
    avg_guests: avg_guests
    active_resources: active_resources
  }

  rep 200 stats

# GET /stats/by-resource — har resurs bo'yicha statistika
http.on :get "/stats/by-resource" \req ->
  ctx = req.ctx
  tid = ctx.tenant_id

  # Raw SQL: resurs statistikasi va rating o'rtachasi
  rows = db.q "
    select
      r.id as resource_id,
      r.name,
      count(distinct b.id) as bookings_count,
      coalesce(sum(case when b.status = 'done' then b.total_cents else 0 end), 0) as revenue_cents,
      coalesce(avg(rv.rating), null) as avg_rating
    from resources r
    left join bookings b on r.id = b.resource_id and r.tenant_id = b.tenant_id and b.status in ('done', 'confirmed')
    left join reviews rv on r.id = rv.resource_id and r.tenant_id = rv.tenant_id
    where r.tenant_id = $1
    group by r.id, r.name
    having count(distinct b.id) > 0
    order by revenue_cents desc
  " [tid]

  rep 200 rows

# GET /stats/daily — kunlik statistika
http.on :get "/stats/daily" \req ->
  ctx = req.ctx
  tid = ctx.tenant_id

  days_param = req.query.days ?? "30"
  days_int = str.int days_param

  # Raw SQL: kunlik guruhlash (SQLite date() ichida)
  rows = db.q "
    select
      date(created) as day,
      count(*) as bookings,
      coalesce(sum(case when status in ('done', 'confirmed') then total_cents else 0 end), 0) as revenue_cents
    from bookings
    where tenant_id = $1 and created >= datetime('now', '-' || $2 || ' days')
    group by date(created)
    order by day asc
  " [tid days_int]

  rep 200 rows

# GET /stats/top-customers — eng ko'p sarflagan mijozlar
http.on :get "/stats/top-customers" \req ->
  ctx = req.ctx
  tid = ctx.tenant_id

  limit_param = req.query.limit ?? "10"
  limit_int = str.int limit_param

  # Raw SQL: user_email bo'yicha guruhlash
  rows = db.q "
    select
      user_email,
      count(*) as bookings,
      sum(total_cents) as total_spent_cents
    from bookings
    where tenant_id = $1 and status = 'done'
    group by user_email
    order by total_spent_cents desc
    limit $2
  " [tid limit_int]

  rep 200 rows

# Server ishga tushirish
http.serve 8080
