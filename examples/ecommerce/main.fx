# main.flux — kirish nuqtasi
# Barcha modullarni ulaydi va serverni 8080 portda ishga tushiradi.
# Har bir modul yuklanganda o'zining http.on yo'nalishlarini ro'yxatdan o'tkazadi.
use http

# Sxema (tbl) + endpoint modullari.
use ./schema
use ./products
use ./cart
use ./checkout
use ./reviews
use ./aifeatures
use ./jobs

# Sog'liq tekshiruvi.
http.on :get "/health" \req -> rep 200 {status::ok}

# Cron vazifalarini ro'yxatdan o'tkazamiz.
jobs.register_jobs

# Serverni ishga tushiramiz.
log "E-commerce API 8080 portda ishga tushmoqda..."
http.serve 8080
