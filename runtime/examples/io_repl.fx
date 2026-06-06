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

# --- REPL tsikli: cheksiz loop `each i in inf` (i = 0,1,2,...) ---
# `while`/`for` yo'q; `inf` faqat `each` iteratori sifatida ma'noli.
# EOF (nil) yoki "exit" yozilsa `stop` bilan chiqamiz.
log "--- echo REPL (chiqish uchun Ctrl-D yoki 'exit') ---"
each i in inf
  satr = io.prompt "echo> "
  if satr == nil
    log "xayr!"
    stop
  if satr == "exit"
    log "xayr!"
    stop
  log "siz: ${satr}"
