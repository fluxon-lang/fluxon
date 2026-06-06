# Foydalanuvchi modullari — `use ./fayl` (issue #45).
# Yo'l shu faylning katalogiga nisbatan hal qilinadi; `.fx` kengaytmasi avto.

use ./greet                 # greet.fx -> greet.* namespace

log (greet.hello "Aziza")   # Salom, Aziza!
log "til: ${greet.lang}"    # til: o'zbekcha

# `as` bilan qayta nomlash (batareya nomi bilan to'qnashuvni oldini oladi).
use ./greet as g
log (g.hello "Bobur")       # Salom, Bobur!

# Modul-private nom (`prefix`) namespace'da yo'q — nil qaytaradi.
(greet.prefix == nil) | (fail "private nom eksport qilinmasligi kerak")
log "ok: private nom yashirin"
