use http db auth json time

# Sxema ta'rifi
tbl tenants
  id   serial pk
  name str
  plan sym
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
  id            serial pk
  tenant_id     int ref:tenants.id
  resource_id   int ref:resources.id
  booking_id    int ref:bookings.id
  rating        int
  comment       str
  created       now

# Middleware: JWT dan tenant_id ni ol
http.before "*" \req ->
  auth_header = req.headers.authorization ?? ""
  if !(str.len auth_header) > 0
    fail 401 {error:"authorization required"}
  token = str.slice auth_header 7 (str.len auth_header)
  claims = auth.verify token!
  req.ctx <- {tenant_id:claims.sub}

# 1. POST /resources
http.on :post "/resources" \req ->
  tid = req.ctx.tenant_id
  res = db.ins "resources" {
    tenant_id:tid
    name:req.body.name
    kind:req.body.kind
    capacity:req.body.capacity
    price_cents:req.body.price_cents
    active:req.body.active ?? true
  }
  rep 201 res

# 2. GET /resources (with query filters)
http.on :get "/resources" \req ->
  tid = req.ctx.tenant_id
  filter = {tenant_id:tid}
  if req.query.kind
    filter = filter.set "kind" (json.dec ("\"" + req.query.kind + "\""))
  if req.query.active
    filter = filter.set "active" (req.query.active == "true")
  if req.query.min_capacity
    filter = filter.set "capacity__ge" (str.int req.query.min_capacity)
  opts = {order::id limit:50 offset:0}
  resources = db.find "resources" filter opts
  rep 200 resources

# 3. GET /resources/:id
http.on :get "/resources/:id" \req ->
  tid = req.ctx.tenant_id
  rid = str.int req.params.id
  res = db.get "resources" {id:rid tenant_id:tid}!
  if res == nil
    fail 404 {error:"resource not found"}
  rep 200 res

# 4. POST /bookings (race-safe with transaction)
http.on :post "/bookings" \req ->
  tid = req.ctx.tenant_id
  rid = req.body.resource_id
  start_at = req.body.start_at
  end_at = req.body.end_at

  # txn ichida overlap tekshir va booking yaratadi
  booking = db.tx \->
    # apa mavjud pending/confirmed booking overlap qiladi?
    existing = db.find "bookings" {
      tenant_id:tid
      resource_id:rid
      status:[:pending :confirmed]
    }

    overlap = false
    each ex in existing
      # interval overlap: start_at < end_at VA ex.end_at > start_at
      if (start_at < ex.end_at) & (end_at > ex.start_at)
        overlap = true
        stop

    if overlap
      fail 409 {error:"resource already booked for this time"}

    # bugun yaratish
    ins = db.ins "bookings" {
      tenant_id:tid
      resource_id:rid
      user_email:req.body.user_email
      status::pending
      start_at:start_at
      end_at:end_at
      guests:req.body.guests
      total_cents:req.body.total_cents
    }
    ret ins

  rep 201 booking!

# 5. PATCH /bookings/:id/status
http.on :patch "/bookings/:id/status" \req ->
  tid = req.ctx.tenant_id
  bid = str.int req.params.id
  booking = db.get "bookings" {id:bid tenant_id:tid}!
  if booking == nil
    fail 404 {error:"booking not found"}
  db.up "bookings" {status:req.body.status} {id:bid}
  updated = db.get "bookings" {id:bid}
  rep 200 updated

# 6. GET /bookings (with filters)
http.on :get "/bookings" \req ->
  tid = req.ctx.tenant_id
  filter = {tenant_id:tid}

  if req.query.status
    statuses = str.split req.query.status ","
    status_syms <- []
    each s in statuses
      status_syms <- status_syms.push (json.dec ("\":" + str.low s + "\""))
    filter = filter.set "status" status_syms

  if req.query.resource_id
    filter = filter.set "resource_id" (str.int req.query.resource_id)

  if req.query.from
    filter = filter.set "start_at__ge" req.query.from

  if req.query.to
    filter = filter.set "end_at__le" req.query.to

  limit = str.int (req.query.limit ?? "50")
  offset = str.int (req.query.offset ?? "0")
  opts = {order::start_at limit:limit offset:offset}

  bookings = db.find "bookings" filter opts
  rep 200 bookings

# 7. GET /stats/overview
http.on :get "/stats/overview" \req ->
  tid = req.ctx.tenant_id

  # Hammasi (to'liq sanoq)
  total = db.agg "bookings" {tenant_id:tid}
    {count::total}
  total_count = (total.0.total) ?? 0

  # Status bo'yicha
  confirmed = db.agg "bookings" {tenant_id:tid status::confirmed}
    {count::c}
  confirmed_count = (confirmed.0.c) ?? 0

  cancelled = db.agg "bookings" {tenant_id:tid status::cancelled}
    {count::c}
  cancelled_count = (cancelled.0.c) ?? 0

  pending = db.agg "bookings" {tenant_id:tid status::pending}
    {count::c}
  pending_count = (pending.0.c) ?? 0

  # Revenue: :done bookings
  rev = db.agg "bookings" {tenant_id:tid status::done}
    {sum__total_cents::rev}
  revenue_cents = (rev.0.rev) ?? 0

  # Avg guests: :confirmed + :done
  avg_g = db.agg "bookings" {tenant_id:tid status:[:confirmed :done]}
    {avg__guests::avg_guests}
  avg_guests = (avg_g.0.avg_guests) ?? nil

  # Active resources
  active_res = db.agg "resources" {tenant_id:tid active:true}
    {count::c}
  active_resources = (active_res.0.c) ?? 0

  rep 200 {
    total_bookings:total_count
    confirmed:confirmed_count
    cancelled:cancelled_count
    pending:pending_count
    revenue_cents:revenue_cents
    avg_guests:avg_guests
    active_resources:active_resources
  }

# 8. GET /stats/by-resource
http.on :get "/stats/by-resource" \req ->
  tid = req.ctx.tenant_id

  # Raw SQL: har resurs uchun stats (faqat minimal 1 booking)
  rows = db.q
    "select
      r.id,
      r.name,
      count(distinct b.id) as bookings_count,
      coalesce(sum(case when b.status = :done then b.total_cents else 0 end), 0) as revenue_cents,
      coalesce(avg(rev.rating), null) as avg_rating
    from resources r
    left join bookings b on b.resource_id = r.id and b.tenant_id = r.tenant_id
      and b.status in (:confirmed, :done)
    left join reviews rev on rev.resource_id = r.id and rev.tenant_id = r.tenant_id
    where r.tenant_id = $1
    group by r.id, r.name
    having count(distinct b.id) > 0
    order by revenue_cents desc"
    [tid]

  rep 200 rows

# 9. GET /stats/daily
http.on :get "/stats/daily" \req ->
  tid = req.ctx.tenant_id
  days = str.int (req.query.days ?? "30")

  rows = db.q
    "select
      date(created) as day,
      count(distinct id) as bookings,
      coalesce(sum(total_cents), 0) as revenue_cents
    from bookings
    where tenant_id = $1 and status in (:confirmed, :done)
      and created >= datetime('now', '-' || $2 || ' days')
    group by date(created)
    order by day asc"
    [tid days]

  rep 200 rows

# 10. GET /stats/top-customers
http.on :get "/stats/top-customers" \req ->
  tid = req.ctx.tenant_id
  limit = str.int (req.query.limit ?? "10")

  rows = db.q
    "select
      user_email,
      count(distinct id) as bookings,
      sum(total_cents) as total_spent_cents
    from bookings
    where tenant_id = $1 and status = :done
    group by user_email
    order by total_spent_cents desc
    limit $2"
    [tid limit]

  rep 200 rows

http.serve 8080
