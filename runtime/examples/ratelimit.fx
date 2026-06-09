# HTTP rate-limit — til darajasidagi primitiv (issue #79).
#
# http.limit middleware kabi deklaratsiya tartibida ishlaydi. Sintaksis:
#   http.limit N :sec|:min|:hr \req -> kalit          # barcha yo'lga (http.use kabi)
#   http.limit "/api/*" N :min  \req -> kalit          # prefiks bo'yicha (http.before kabi)
# Limit oshsa avtomatik 429 + Retry-After (oyna tugashigacha soniya). Kalit
# funksiyasi nil qaytarsa req.ip'ga qaytadi (kalitsiz so'rovni ham cheklash uchun).
#
# Ishga tushirish:  flux run examples/ratelimit.fx
# Sinash (4-so'rov 429 qaytaradi):
#   for i in 1 2 3 4; do curl -i -H "x-api-key: demo" localhost:8080/api/ping; done
#   curl -i localhost:8080/api/ping        # kalitsiz -> IP bo'yicha cheklanadi

# Avval auth: API kalitni ctx'ga yozamiz (haqiqiy ilovada db'dan tekshiriladi).
# Bu http.limit'dan OLDIN e'lon qilingani uchun kalit funksiyasi req.ctx'ni ko'radi.
http.before "/api/*" \req ->
  key = req.headers.x_api_key
  if !key
    fail 401 "x-api-key kerak"
  req.ctx <- {api_key: key}

# Per-key rate-limit: /api/* yo'llariga, har API kalit uchun 3 so'rov/daqiqa.
http.limit "/api/*" 3 :min \req -> req.ctx.api_key

http.on :get "/api/ping" \req ->
  rep 200 {ok: true key: req.ctx.api_key}

# Limitsiz yo'l — /api/* ga mos emas.
http.on :get "/health" \req ->
  rep 200 {ok: true}

http.serve 8080
