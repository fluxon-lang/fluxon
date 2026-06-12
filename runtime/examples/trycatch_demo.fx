# try/catch demo (issue #125) — xatoni ushlab qolib, ishni davom ettirish.
# Ishga: cargo run -- run examples/trycatch_demo.fx

# 1) Fallback: xato chiqsa default qiymat bilan davom et.
fn narx_ol mahsulot
  if mahsulot == "yo'q"
    fail 404 "mahsulot topilmadi: ${mahsulot}"
  ret 100

narx = try
  narx_ol "yo'q"
catch e
  log "ogohlantirish: ${e.message} (status: ${e.status})"
  0                                  # fallback narx
log "narx = ${narx}"                 # → 0

# 2) Custom xato: o'z biznes qoidangizdan fail chiqarish.
fn tekshir items
  if (items.len) != 4
    fail "yuborilgan ma'lumot faqat 4ta bo'lishi kerak"
  ret :ok

xabar = try
  tekshir [1 2 3]
catch e
  e.message
log xabar                            # → yuborilgan ma'lumot faqat 4ta...

# 3) Bir nechta manbadan birinchi ishlaganini olish (re-raise bilan).
fn birlamchi -> fail "birlamchi manba yiqildi"
fn zaxira -> "zaxiradan ma'lumot"

natija = try
  birlamchi()
catch e
  log "birlamchi yiqildi: ${e.message} — zaxiraga o'tamiz"
  try
    zaxira()
  catch e2
    fail "ikkala manba ham yiqildi: ${e2.message}"
log natija                           # → zaxiradan ma'lumot

# 4) Qayta urinish (retry): xato bo'lsa bir necha marta urinish.
urinish <- 0
fn beqaror
  urinish <- urinish + 1
  if urinish < 3
    fail "vaqtinchalik xato (urinish ${urinish})"
  ret "muvaffaqiyat"

javob <- nil
each i in 1..3
  javob <- try
    beqaror()
  catch e
    log "urinish yiqildi: ${e.message}"
    nil
  if javob != nil
    stop
log "yakuniy javob: ${javob}"        # → muvaffaqiyat (3-urinishda)
