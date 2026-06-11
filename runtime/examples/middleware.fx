# HTTP middleware + request-scoped context (issue #67, #68).
#
# Multi-tenant API namunasi: himoyalangan /api/* yo'llari avval auth'dan o'tadi,
# auth natijasi (tenant/role) req.ctx orqali handler'ga uzatiladi — har handler'da
# qayta hisoblanmaydi.
#
# Ishga tushirish:  fluxon run examples/middleware.fx
# Sinash:
#   curl localhost:8080/health                       # auth shart emas -> 200
#   curl localhost:8080/api/me                       # auth yo'q -> 401
#   curl -H "Authorization: Bearer t5" localhost:8080/api/me   # -> tenant/role

# http.before — faqat /api/* yo'llariga (prefiks bo'yicha). Authorization
# header'ni tekshiradi; yo'q bo'lsa fail 401 (zanjir to'xtaydi, handler chaqirilmaydi).
# Bor bo'lsa tenant/role'ni aniqlab req.ctx ga yozadi.
http.before "/api/*" \req ->
  token = req.headers.authorization
  if !token
    fail 401 "Authorization header kerak"
  # Haqiqiy ilovada bu yerda token tekshiriladi (db.one ...). Bu yerda
  # soddalashtirilgan tenant/role'ni ctx ga yozamiz.
  req.ctx <- {tenant_id: 5 role: "admin" token: token}

# http.use — barcha route'larga global middleware. Bu yerda oddiy log.
http.use \req ->
  log "${req.method} ${req.path}"

# Himoyalanmagan yo'l — /api/* ga mos emas, auth talab qilmaydi.
http.on :get "/health" \req ->
  rep 200 {ok: true}

# Himoyalangan yo'l — middleware qo'ygan ctx'ni o'qiydi (qayta hisoblamasdan).
http.on :get "/api/me" \req ->
  ctx = req.ctx
  rep 200 {tenant: ctx.tenant_id role: ctx.role}

http.serve 8080
