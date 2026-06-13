# HTTP rate-limit — a language-level primitive (issue #79).
#
# http.limit works in declaration order like middleware. Syntax:
#   http.limit N :sec|:min|:hr \req -> key          # all routes (like http.use)
#   http.limit "/api/*" N :min  \req -> key          # by prefix (like http.before)
# When the limit is exceeded: automatic 429 + Retry-After (seconds until the
# window ends). If the key function returns nil it falls back to req.ip (so a
# keyless request can be limited too).
#
# Running: fluxon run examples/ratelimit.fx
# Testing (the 4th request returns 429):
#   for i in 1 2 3 4; do curl -i -H "x-api-key: demo" localhost:8080/api/ping; done
#   curl -i localhost:8080/api/ping        # keyless -> limited by IP

# Auth first: write the API key into ctx (in a real app it's verified against db).
# Since this is declared BEFORE http.limit, the key function sees req.ctx.
http.before "/api/*" \req ->
  key = req.headers.x_api_key
  if !key
    fail 401 "x-api-key required"
  req.ctx <- {api_key: key}

# Per-key rate-limit: for /api/* routes, 3 requests/minute per API key.
http.limit "/api/*" 3 :min \req -> req.ctx.api_key

http.on :get "/api/ping" \req ->
  rep 200 {ok: true key: req.ctx.api_key}

# Unlimited route — does not match /api/*.
http.on :get "/health" \req ->
  rep 200 {ok: true}

http.serve 8080
