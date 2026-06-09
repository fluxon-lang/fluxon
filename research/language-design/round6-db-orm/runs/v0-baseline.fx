use http db auth time json

# =======================================
# SCHEMA: multi-tenant booking system
# =======================================

tbl tenants
  id      serial pk
  name    str
  plan    sym
  created now

tbl resources
  id          serial pk
  tenant_id   int ref:tenants.id
  name        str
  kind        sym
  capacity    int
  price_cents money
  active      bool

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

tbl reviews
  id          serial pk
  tenant_id   int ref:tenants.id
  resource_id int ref:resources.id
  booking_id  int ref:bookings.id
  rating      int
  comment     str
  created     now

# =======================================
# MIDDLEWARE: JWT tekshiruv + tenant_id
# =======================================

http.before "*" \req ->
  auth_hdr = req.headers.authorization

  if auth_hdr == nil
    fail 401 "Authorization header kerak"

  # "Bearer TOKEN" sxemasini parse qil
  parts = str.split auth_hdr " "
  if (parts.len) != 2
    fail 401 "Bearer token kerak"

  token = parts.1
  claims = auth.verify token!

  # Kontekstga tenant_id qo'y
  req.ctx <- {tenant_id: claims.sub}

# =======================================
# 1. POST /resources
# =======================================

http.on :post "/resources" \req ->
  tid = req.ctx.tenant_id
  body = req.body

  # Validation
  if body.name == nil | body.kind == nil | body.capacity == nil | body.price_cents == nil
    fail 422 "name, kind, capacity, price_cents kerak"

  res = db.ins "resources" {
    tenant_id: tid
    name: body.name
    kind: body.kind
    capacity: body.capacity
    price_cents: body.price_cents
    active: body.active ?? true
  }

  rep 201 res

# =======================================
# 2. GET /resources (filtering)
# =======================================

http.on :get "/resources" \req ->
  tid = req.ctx.tenant_id
  q = req.query

  # SQL tuzish
  sql = "select * from resources where tenant_id=$1"
  params = [tid]

  if q.kind != nil
    sql = sql + " and kind=$" + str.str (len params + 1)
    params = params.push q.kind

  if q.active != nil
    active_bool = q.active == "true"
    sql = sql + " and active=$" + str.str (len params + 1)
    params = params.push active_bool

  if q.min_capacity != nil
    cap = str.int q.min_capacity
    sql = sql + " and capacity>=$" + str.str (len params + 1)
    params = params.push cap

  sql = sql + " order by id"

  rows = db.q sql params

  rep 200 rows

# =======================================
# 3. GET /resources/:id
# =======================================

http.on :get "/resources/:id" \req ->
  tid = req.ctx.tenant_id
  rid = str.int req.params.id

  res = db.one "select * from resources where id=$1 and tenant_id=$2" [rid tid]

  if res == nil
    fail 404 "Resource not found"

  rep 200 res

# =======================================
# 4. POST /bookings (race-safe tx)
# =======================================

http.on :post "/bookings" \req ->
  tid = req.ctx.tenant_id
  body = req.body

  # Validation
  if body.resource_id == nil | body.user_email == nil | body.start_at == nil | body.end_at == nil | body.guests == nil | body.total_cents == nil
    fail 422 "resource_id, user_email, start_at, end_at, guests, total_cents kerak"

  rid = body.resource_id
  start_at = body.start_at
  end_at = body.end_at

  # Transaction: overlap tekshir
  booking = db.tx \->
    # Xuddi shu vaqt oralig'ida :pending yoki :confirmed booking bormi?
    # Interval overlap: start_at < other.end_at AND end_at > other.start_at
    overlaps = db.q "
      select id from bookings
      where tenant_id=$1 and resource_id=$2
        and (status=$3 or status=$4)
        and start_at < $5 and end_at > $6
    " [tid rid :pending :confirmed end_at start_at]

    if (overlaps.len) > 0
      fail 409 "Bu vaqt oralig'ida allaqachon booking bor"

    # Insert yangi booking
    new_booking = db.ins "bookings" {
      tenant_id: tid
      resource_id: rid
      user_email: body.user_email
      status: :pending
      start_at: start_at
      end_at: end_at
      guests: body.guests
      total_cents: body.total_cents
    }

    ret new_booking

  rep 201 booking!

# =======================================
# 5. PATCH /bookings/:id/status
# =======================================

http.on :patch "/bookings/:id/status" \req ->
  tid = req.ctx.tenant_id
  bid = str.int req.params.id
  body = req.body

  if body.status == nil
    fail 422 "status kerak"

  # Booking mavjudmi?
  booking = db.one "select * from bookings where id=$1 and tenant_id=$2" [bid tid]

  if booking == nil
    fail 404 "Booking not found"

  # Update status
  db.up "bookings" {status: body.status} {id: bid}

  updated = db.one "select * from bookings where id=$1" [bid]!

  rep 200 updated

# =======================================
# 6. GET /bookings (filtering + pagination)
# =======================================

http.on :get "/bookings" \req ->
  tid = req.ctx.tenant_id
  q = req.query

  sql = "select * from bookings where tenant_id=$1"
  params = [tid]

  # Status filter (vergul bilan "pending,confirmed")
  if q.status != nil
    statuses = str.split q.status ","
    sql = sql + " and status in ("
    each i in 0..(statuses.len)
      if i > 0
        sql = sql + ","
      sql = sql + "$" + str.str (len params + i + 1)
    sql = sql + ")"
    each s in statuses
      params = params.push (json.dec ("\":" + s + "\""))

  # Resource filter
  if q.resource_id != nil
    rid = str.int q.resource_id
    sql = sql + " and resource_id=$" + str.str (len params + 1)
    params = params.push rid

  # Date range
  if q.from != nil
    sql = sql + " and start_at>=$" + str.str (len params + 1)
    params = params.push q.from

  if q.to != nil
    sql = sql + " and end_at<=$" + str.str (len params + 1)
    params = params.push q.to

  # Pagination
  limit = str.int (q.limit ?? "50")
  offset = str.int (q.offset ?? "0")

  sql = sql + " order by start_at asc limit $" + str.str (len params + 1) + " offset $" + str.str (len params + 2)
  params = params.push limit
  params = params.push offset

  rows = db.q sql params

  rep 200 rows

# =======================================
# 7. GET /stats/overview
# =======================================

http.on :get "/stats/overview" \req ->
  tid = req.ctx.tenant_id

  # Hammasi
  total_row = db.one "select count(*) c from bookings where tenant_id=$1" [tid]!
  total_bookings = total_row.c ?? 0

  # Status counts
  confirmed_row = db.one "select count(*) c from bookings where tenant_id=$1 and status=$2" [tid :confirmed]!
  confirmed = confirmed_row.c ?? 0

  cancelled_row = db.one "select count(*) c from bookings where tenant_id=$1 and status=$2" [tid :cancelled]!
  cancelled = cancelled_row.c ?? 0

  pending_row = db.one "select count(*) c from bookings where tenant_id=$1 and status=$2" [tid :pending]!
  pending = pending_row.c ?? 0

  # Revenue (faqat :done)
  rev_row = db.one "select coalesce(sum(total_cents), 0) rev from bookings where tenant_id=$1 and status=$2" [tid :done]!
  revenue_cents = rev_row.rev ?? 0

  # Avg guests (:confirmed + :done)
  avg_row = db.one "select coalesce(avg(guests), 0) avg_guests from bookings where tenant_id=$1 and (status=$2 or status=$3)" [tid :confirmed :done]!
  avg_guests = avg_row.avg_guests ?? 0

  # Active resources
  active_row = db.one "select count(*) c from resources where tenant_id=$1 and active=true" [tid]!
  active_resources = active_row.c ?? 0

  rep 200 {
    total_bookings: total_bookings
    confirmed: confirmed
    cancelled: cancelled
    pending: pending
    revenue_cents: revenue_cents
    avg_guests: avg_guests
    active_resources: active_resources
  }

# =======================================
# 8. GET /stats/by-resource
# =======================================

http.on :get "/stats/by-resource" \req ->
  tid = req.ctx.tenant_id

  rows = db.q "
    select
      r.id resource_id,
      r.name,
      count(distinct b.id) bookings_count,
      coalesce(sum(case when b.status=$1 then b.total_cents else 0 end), 0) revenue_cents,
      coalesce(avg(rv.rating), null) avg_rating
    from resources r
    left join bookings b on b.resource_id=r.id and b.tenant_id=r.tenant_id
      and (b.status=$2 or b.status=$3)
    left join reviews rv on rv.resource_id=r.id and rv.tenant_id=r.tenant_id
    where r.tenant_id=$4
    group by r.id, r.name
    having count(distinct b.id) > 0
    order by revenue_cents desc
  " [
    :done
    :done
    :confirmed
    tid
  ]

  rep 200 rows

# =======================================
# 9. GET /stats/daily
# =======================================

http.on :get "/stats/daily" \req ->
  tid = req.ctx.tenant_id
  q = req.query
  days_param = str.int (q.days ?? "30")

  rows = db.q "
    select
      date(created) day,
      count(*) bookings,
      coalesce(sum(case when status=$1 then total_cents else 0 end), 0) revenue_cents
    from bookings
    where tenant_id=$2 and (status=$3 or status=$4)
      and created >= datetime('now', '-' || $5 || ' days')
    group by date(created)
    order by day asc
  " [
    :done
    tid
    :done
    :confirmed
    days_param
  ]

  rep 200 rows

# =======================================
# 10. GET /stats/top-customers
# =======================================

http.on :get "/stats/top-customers" \req ->
  tid = req.ctx.tenant_id
  q = req.query
  limit_param = str.int (q.limit ?? "10")

  rows = db.q "
    select
      user_email,
      count(*) bookings,
      sum(total_cents) total_spent_cents
    from bookings
    where tenant_id=$1 and status=$2
    group by user_email
    order by total_spent_cents desc
    limit $3
  " [
    tid
    :done
    limit_param
  ]

  rep 200 rows

# =======================================
# SERVER
# =======================================

http.serve 8080