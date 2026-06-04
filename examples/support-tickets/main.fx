# Asosiy fayl — routerlar va serverni ulaydi

use http cron env
use ./tickets
use ./report

# Routerlar
http.on :post "/tickets" \req -> tickets.create req
http.on :get "/tickets" \req -> tickets.list req
http.on :get "/tickets/:id" \req -> tickets.get req
http.on :post "/tickets/:id/reply" \req -> tickets.reply req

# Har kuni 08:00 da hisobot
cron.dy 8 0 report.daily_report

# Server
port = env.PORT ?? "8080"
http.serve port
