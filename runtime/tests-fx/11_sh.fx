# 11 — sh battery (tashqi shell buyruqlari).
# Ishga: ./target/release/flux run tests-fx/11_sh.fx
#
# sh.run cmd -> {stdout: str  stderr: str  code: int}. Buyruq shell orqali boradi,
# shuning uchun `&&`, quvur (|) ishlaydi. Bu testlar Unix shell (sh) ni nazarda
# tutadi — CI ubuntu+macOS da ishlaydi.

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

# --- Oddiy chaqiruv: stdout + code ---
r = sh.run "printf salom"
eq r.stdout "salom" "printf stdout"
eq r.code 0 "muvaffaqiyat kodi 0"
eq r.stderr "" "stderr bo'sh"

# --- Non-zero exit: Flow::err EMAS, code orqali bilinadi ---
bad = sh.run "exit 5"
eq bad.code 5 "exit 5 -> code 5"

# --- stderr alohida tutiladi (stdout bilan aralashmaydi) ---
e = sh.run "printf xato 1>&2"
eq e.stderr "xato" "stderr tutildi"
eq e.stdout "" "xato vaqtida stdout bo'sh"

# --- Shell xususiyatlari: `&&` ketma-ket buyruq ---
chain = sh.run "printf a && printf b"
eq chain.stdout "ab" "&& ketma-ket buyruq"
eq chain.code 0 "&& kodi 0"

# --- Quvur (|) ishlaydi ---
pipe = sh.run "printf 'bir\nikki\nuch' | wc -l"
truthy (str.int pipe.stdout >= 2) "quvur (wc -l) ishladi"

# --- Yakun ---
if fails == 0
  log "=== 11_sh: HAMMASI O'TDI ==="
else
  log "=== 11_sh: ${fails} TEST YIQILDI ==="
