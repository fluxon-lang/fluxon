# Queue demo — fon navbati (background jobs) HTTP server bilan birga.
#
# Falsafa: webhook TEZ javob qaytarishi kerak. Og'ir ishni (SMS yuborish, hisobot
# tayyorlash) fonga uzatamiz — `queue.push` darhol qaytadi, ish fon worker
# thread'ida ketma-ket (FIFO) bajariladi.
#
# `queue.on` handler ro'yxatga oladi (bloklamaydi), `queue.push` ish qo'shadi
# (bloklamaydi). Handler bittagina `job` argumenti oladi — bu `queue.push`'ga
# berilgan payload (map). Eng oxirgi bloklovchi chaqiruv (bu yerda http.serve)
# processni tirik ushlaydi; worker fonda navbatni qayta ishlaydi.

use http queue

# Ishlovchi: "send" nomli ishlar shu yerda bajariladi. job — push qilingan payload.
queue.on "send" \job ->
  log "SMS yuborilmoqda -> ${job.ph}: ${job.body}"

# Ishlovchi: "report" nomli og'ir ish (masalan hisobot generatsiyasi).
queue.on "report" \job ->
  log "hisobot tayyorlanmoqda: ${job.kind}"

# Webhook: kelgan so'rovni navbatga uzatib, DARHOL javob qaytaradi.
# Klient kutib turmaydi — og'ir ish fonda bajariladi.
http.on :post "/notify" \req ->
  queue.push "send" {ph:req.body.ph body:req.body.text}
  rep 202 {queued:true}

# Boshqa endpoint: hisobotni fonga uzatadi.
http.on :post "/report" \req ->
  queue.push "report" {kind:req.body.kind}
  rep 202 {queued:true}

# Server processni ushlab turadi; worker fonda navbatni qayta ishlaydi.
http.serve 8080

# --- Server YO'Q, faqat queue bo'lsa: push'lardan keyin processni ushlash uchun
# bloklovchi chaqiruv kerak. http.serve / ws.serve / cron.run dan birini ishlating.
