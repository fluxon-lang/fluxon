# 08 — io: terminal input/output (read_line, print, prompt).
# Stdin orqali kiritma beriladi: "Firdavs\n42\n" (run_all.sh shuni quvuraydi).
# io.print/io.prompt stdout'ga yozadi; tasdiqlash log (stderr) orqali ketadi —
# ular aralashmaydi.

fails <- 0

fn eq got want label
  if got == want
    log "ok  ${label} = ${got}"
  else
    log "FAIL ${label}: got=${got} want=${want}"
    fails <- fails + 1

# --- io.prompt: chiqarib, keyin bitta satr o'qiydi ---
ism = io.prompt "Isming: "
eq ism "Firdavs" "prompt read 1-satr"

# --- io.read_line: keyingi satr ---
yosh = io.read_line
eq yosh "42" "read_line 2-satr"

# --- io.read_line: EOF'da nil ---
oxiri = io.read_line
eq oxiri nil "read_line EOF -> nil"

# --- io.print: nil qaytaradi (yon ta'sir stdout) ---
natija = io.print ""
eq natija nil "print -> nil"

# --- Yakun ---
if fails == 0
  log "=== 08_io: HAMMASI O'TDI ==="
else
  log "=== 08_io: ${fails} TEST YIQILDI ==="
