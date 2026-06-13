# HTTP middleware + request-scoped context (issue #67, #68).
#
# Multi-tenant API example: protected /api/* routes go through auth first,
# the auth result (tenant/role) is passed to the handler via req.ctx — it is not
# recomputed in each handler.
#
# Running: fluxon run examples/middleware.fx
# Testing:
#   curl localhost:8080/health                       # no auth needed -> 200
#   curl localhost:8080/api/me                       # no auth -> 401
#   curl -H "Authorization: Bearer t5" localhost:8080/api/me   # -> tenant/role

# http.before — only for /api/* routes (by prefix). Checks the Authorization
# header; if missing, fail 401 (chain stops, handler not called).
# If present, determines tenant/role and writes them to req.ctx.
http.before "/api/*" \req ->
  token = req.headers.authorization
  if !token
    fail 401 "Authorization header required"
  # In a real app the token is verified here (db.one ...). Here we write a
  # simplified tenant/role into ctx.
  req.ctx <- {tenant_id: 5 role: "admin" token: token}

# http.use — global middleware for all routes. Here just a simple log.
http.use \req ->
  log "${req.method} ${req.path}"

# Unprotected route — does not match /api/*, requires no auth.
http.on :get "/health" \req ->
  rep 200 {ok: true}

# Protected route — reads the ctx set by middleware (without recomputing).
http.on :get "/api/me" \req ->
  ctx = req.ctx
  rep 200 {tenant: ctx.tenant_id role: ctx.role}

http.serve 8080
