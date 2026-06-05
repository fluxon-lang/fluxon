# 04 — time + env battery.
# Ishga: FLUX_TEST_VAR=salom PORT=9090 ./target/release/flux run tests-fx/04_time_env.fx

fails <- 0
fn ok label -> log "ok  ${label}"
fn bad label
  log "FAIL ${label}"
  fails <- fails + 1

# --- time.now: UTC matn timestamp (DB-mos "YYYY-MM-DD HH:MM:SS", 19 belgi) ---
# ESLATMA: time.* STRING qaytaradi; Flux'da `<` string'ga ishlamaydi, shuning
# uchun tartibni son sifatida — yilni str.slice + str.int bilan — solishtiramiz.
t = time.now
if (str.len t) == 19
  ok "time.now format len 19 = ${t}"
else
  bad "time.now wrong len=${str.len t} (${t})"

# ajratuvchilar to'g'ri joyda
if (str.slice t 4 5) == "-" & (str.slice t 10 11) == " " & (str.slice t 13 14) == ":"
  ok "time.now separators (- space :)"
else
  bad "time.now separators"

# --- time.ago: o'tmishdagi nuqta now'dan farqli va oldinroq ---
past = time.ago 24 :hr
if past != t
  ok "time.ago 24:hr differs from now (${past})"
else
  bad "time.ago == now"

# birliklar ishlaydi — barchasi 19-belgili to'g'ri timestamp
a_sec = time.ago 30 :sec
a_min = time.ago 10 :min
a_day = time.ago 2 :day
if (str.len a_sec) == 19 & (str.len a_min) == 19 & (str.len a_day) == 19
  ok "time.ago :sec/:min/:day all valid timestamps"
else
  bad "time.ago units format"

# tartib: 2 kun oldin yili/oyi 30 soniya oldindan kichik-yoki-teng (sana qismi).
# Sanani son qilib olamiz: YYYYMMDD -> int, leksikografik = xronologik.
fn datenum ts
  y = str.int (str.slice ts 0 4)
  mo = str.int (str.slice ts 5 7)
  d = str.int (str.slice ts 8 10)
  y * 10000 + mo * 100 + d
if (datenum a_day) <= (datenum a_sec)
  ok "2:day date <= 30:sec date (${datenum a_day} <= ${datenum a_sec})"
else
  bad "time.ago ordering"

# --- env.NOM: muhit o'zgaruvchisi ---
v = env.FLUX_TEST_VAR
if v == "salom"
  ok "env.FLUX_TEST_VAR = ${v}"
else
  bad "env.FLUX_TEST_VAR got=${v}"

# yo'q o'zgaruvchi → nil, ?? default ishlaydi
port = env.PORT ?? "8080"
if port == "9090"
  ok "env.PORT (set) = ${port}"
else
  bad "env.PORT got=${port}"

missing = env.DEFINITELY_NOT_SET ?? "default"
if missing == "default"
  ok "env missing → ?? default"
else
  bad "env missing default got=${missing}"

# --- Yakun ---
if fails == 0
  log "=== 04_time_env: HAMMASI O'TDI ==="
else
  log "=== 04_time_env: ${fails} TEST YIQILDI ==="
