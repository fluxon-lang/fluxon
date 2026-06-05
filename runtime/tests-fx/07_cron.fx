# 07 — cron battery (rejalashtirilgan fon vazifalari).
# Ishga: ./target/release/flux run tests-fx/07_cron.fx
#
# cron — standart Unix 5-maydonli cron ifoda (tirnoqsiz). `cron.on` ro'yxatga
# oladi (bloklamaydi). Bu test PARSE + REGISTRATSIYA to'g'riligini tekshiradi
# (tirnoqsiz/tirnoqli/lambda/murakkab ifoda). Vazifa IJROSI vaqtga bog'liq —
# u native test (cron_mod) va cron_demo.fx qo'lda smoke'da tekshiriladi.

fails <- 0
fn ok label -> log "ok  ${label}"
fn bad label
  log "FAIL ${label}"
  fails <- fails + 1

fn job
  log "ish bajarildi"

# --- Tirnoqsiz 5-maydon + nomli funksiya ---
# `*` bu yerda ko'paytirish EMAS — parser cron ifodani tanib str'ga yig'adi.
# cron.on nil qaytaradi; xato bo'lmasa registratsiya o'tdi.
r1 = cron.on 0 * * * * job
if r1 == nil
  ok "cron.on tirnoqsiz 5-maydon"
else
  bad "cron.on tirnoqsiz got=${r1}"

# --- Murakkab ifoda: step / list / range aralash ---
r2 = cron.on */15 9 1,15 * 1-5 job
if r2 == nil
  ok "cron.on murakkab ifoda (*/15 9 1,15 * 1-5)"
else
  bad "cron.on murakkab got=${r2}"

# --- Inline lambda (parametrsiz) ---
r3 = cron.on 30 9 * * * \->
  log "lambda ish"
if r3 == nil
  ok "cron.on inline lambda"
else
  bad "cron.on lambda got=${r3}"

# --- Tirnoqli variant (inson qulayligi; AI docs'da yo'q) ---
r4 = cron.on "0 0 * * 0" job
if r4 == nil
  ok "cron.on tirnoqli variant"
else
  bad "cron.on tirnoqli got=${r4}"

# --- Yakun ---
if fails == 0
  log "=== 07_cron: HAMMASI O'TDI ==="
else
  log "=== 07_cron: ${fails} TEST YIQILDI ==="
