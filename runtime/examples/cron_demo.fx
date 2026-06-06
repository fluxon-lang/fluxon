# Cron demo — rejalashtirilgan fon vazifalari HTTP server bilan birga.
#
# cron.on hech narsani bloklamaydi (http.on kabi faqat ro'yxatga oladi). Eng
# oxirgi yozilgan bloklovchi chaqiruv (bu yerda http.serve) processni tirik
# ushlaydi; cron fonda o'z vaqtida ishlaydi.
#
# Server YO'Q bo'lsa, http.serve o'rniga cron.run yoziladi (processni ushlab
# turish uchun). cron.run va http.serve/ws.serve ixtiyoriy tartibda BIRGA ham
# ishlaydi — hech biri ikkinchisini bloklamaydi.

use http

# Fon vazifa: har daqiqada bir marta belgi qoldiradi (demo uchun tez interval).
fn tick
  log "cron: har daqiqalik tik"

# Fon vazifa: har kuni ertalab 9:00 da kunlik hisobot.
fn daily
  log "cron: kunlik hisobot (09:00)"

# Rejalashtirilgan vazifalarni ro'yxatga olamiz — bloklamaydi.
cron.on * * * * * tick           # har daqiqa
cron.on 0 9 * * * daily          # har kun 09:00
cron.on 30 9 * * 1-5 \->          # ish kunlari 09:30 (inline lambda)
  log "cron: ish kuni eslatmasi"

# Oddiy HTTP endpoint.
http.on :get "/" \req -> rep 200 {ok:true msg:"cron demo ishlayapti"}

# Server processni ushlab turadi; cron fonda tiktaklaydi.
http.serve 8080

# --- Server YO'Q, faqat cron bo'lsa shunday yozilardi: ---
# cron.on * * * * * tick
# cron.run     # processni o'z qo'liga oladi (bloklaydi)
