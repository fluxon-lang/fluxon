# 09 — fs battery (lokal fayl tizimi primitivlari).
# Ishga: ./target/release/flux run tests-fx/09_fs.fx
#
# fs.read/write/append/exists/ls/del/mkdirp — barchasi haqiqiy fayl tizimida
# ishlaydi. Test noyob vaqtinchalik papkada ishlaydi (parallel run'da
# to'qnashmasligi uchun rand.str bilan nom yasaydi) va oxirida o'zini tozalaydi.

fails <- 0

fn eq got want label
  if got == want
    log "ok  ${label} = ${got}"
  else
    log "FAIL ${label}: got=${got} want=${want}"
    fails <- fails + 1

fn truthy v label
  if v
    log "ok  ${label}"
  else
    log "FAIL ${label}: got=${v}"
    fails <- fails + 1

# --- Noyob ish papkasi ---
base = "/tmp/flux_fs_${rand.str 8}"
r_mk = fs.mkdirp base
eq r_mk :ok "mkdirp -> :ok"
truthy (fs.exists base) "mkdirp keyin papka mavjud"

# --- mkdirp idempotent (ikkinchi marta ham :ok) ---
eq (fs.mkdirp base) :ok "mkdirp idempotent"

# --- write + read aylanasi ---
f = "${base}/a.txt"
eq (fs.write f "salom dunyo") :ok "write -> :ok"
eq (fs.read f) "salom dunyo" "read yozilganni qaytaradi"
truthy (fs.exists f) "write keyin fayl mavjud"

# --- Yo'q faylni o'qish -> nil (xato emas) ---
eq (fs.read "${base}/yoq.txt") nil "yo'q fayl read -> nil"
eq (fs.exists "${base}/yoq.txt") false "yo'q fayl exists -> false"

# --- append: mavjud oxiriga qo'shadi ---
fs.append f " qoldiq"
eq (fs.read f) "salom dunyo qoldiq" "append matnni oxiriga qo'shadi"

# --- append yangi fayl yaratadi ---
g = "${base}/log.txt"
fs.append g "x"
fs.append g "y"
eq (fs.read g) "xy" "append yangi faylni yaratib to'playdi"

# --- ls: saralangan nomlar ro'yxati ---
names = fs.ls base
eq names.len 2 "ls 2 ta yozuv (a.txt, log.txt)"
eq names.0 "a.txt" "ls[0] = a.txt (saralangan)"
eq names.1 "log.txt" "ls[1] = log.txt"

# --- del: faylni o'chiradi ---
eq (fs.del f) :ok "del fayl -> :ok"
eq (fs.exists f) false "del keyin fayl yo'q"

# --- Tozalash: qolgan fayllarni va papkani o'chiramiz ---
fs.del g
fs.del base
eq (fs.exists base) false "del bo'sh papka -> tozalandi"

# --- Yakun ---
if fails == 0
  log "=== 09_fs: HAMMASI O'TDI ==="
else
  log "=== 09_fs: ${fails} TEST YIQILDI ==="
