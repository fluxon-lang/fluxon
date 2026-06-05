# 06 — reg battery (funksiya registri, dinamik dispatch).
# Ishga: ./target/release/flux run tests-fx/06_reg.fx
#
# reg — funksiyani STRING nomi bilan saqlash/chaqirish. Asosiy foydalanish:
# AI agent tool-loop'lari (model tool nomini tanlaydi, kod reg.call bilan bajaradi).

fails <- 0
fn ok label -> log "ok  ${label}"
fn bad label
  log "FAIL ${label}"
  fails <- fails + 1

# --- reg.add + reg.call: nom bilan saqlash va chaqirish ---
# closure args (map) oladi — agent tool argumentlari shu naqshda keladi.
reg.add "calc" \args -> args.a + args.b
out = reg.call "calc" {a:2 b:3}
if out == 5
  ok "reg.call calc = ${out}"
else
  bad "reg.call calc got=${out}"

# string natija (interpolatsiya closure ichida)
reg.add "greet" \args -> "salom ${args.nom}"
g = reg.call "greet" {nom:"Aziza"}
if g == "salom Aziza"
  ok "reg.call greet = ${g}"
else
  bad "reg.call greet got=${g}"

# --- reg.has: nom ro'yxatda bormi (bool) ---
if reg.has "calc"
  ok "reg.has calc = true"
else
  bad "reg.has calc false bo'ldi"

if (reg.has "yoq") == false
  ok "reg.has yoq = false"
else
  bad "reg.has yoq true bo'ldi"

# --- reg.names: ro'yxatdagi nomlar (alifbo tartibida, barqaror) ---
ns = reg.names
if ns.len == 2 & ns.0 == "calc" & ns.1 == "greet"
  ok "reg.names = ${ns}"
else
  bad "reg.names got=${ns}"

# --- reg.add ustiga yozadi (tool yangilash holati) ---
reg.add "calc" \args -> args.a * args.b
out2 = reg.call "calc" {a:4 b:5}
if out2 == 20
  ok "reg.add ustiga yozdi: calc = ${out2}"
else
  bad "reg.add ustiga yozmadi got=${out2}"

# nom soni o'zgarmadi (ustiga yozish yangi yozuv qo'shmaydi)
if reg.names.len == 2
  ok "reg.names ustiga yozgach ham 2 ta"
else
  bad "reg.names ustiga yozgach ${reg.names.len} ta"

# --- Yakun ---
if fails == 0
  log "=== 06_reg: HAMMASI O'TDI ==="
else
  log "=== 06_reg: ${fails} TEST YIQILDI ==="
