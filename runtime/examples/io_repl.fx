# io demo — interaktiv terminal: prompt/o'qish/yangi qatorsiz chiqarish.
#
# `log` har doim stderr'ga `\n` qo'shadi; interaktiv CLI (REPL, agent, wizard)
# uchun `io` kerak:
#   io.prompt msg   — msg'ni \n SIZ chiqarib, bitta satr o'qiydi → str
#   io.read_line    — stdin'dan bitta satr → str (EOF → nil)
#   io.print s      — \n SIZ chiqarish (prompt qurish uchun)
#
# Ishga tushirish:  flux run examples/io_repl.fx
# Sinash (quvur):   printf 'Aziza\n5\n' | flux run examples/io_repl.fx

# --- Bir martalik wizard: ism + yosh so'raymiz ---
ism = io.prompt "Isming: "
io.print "Salom, "
io.print ism
io.print "!\n"

yosh = io.prompt "Yoshing: "
log "Demak sen ${yosh} yoshdasan."

# --- REPL tsikli: `while`/`for` yo'q — rekursiya bilan, EOF (nil) to'xtatadi ---
# (argumentsiz funksiyani qavssiz chaqirib bo'lmaydi, shu sababli dummy `n`.)
repl = \n ->
  satr = io.prompt "echo> "
  if satr == nil
    log "xayr!"
    ret nil
  log "siz: ${satr}"
  repl n

log "--- echo REPL (chiqish uchun Ctrl-D) ---"
repl 0
