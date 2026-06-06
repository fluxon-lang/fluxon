# sh battery demo — tashqi shell buyruqlarini ishga tushirish.
# Ishga: cargo run -- run examples/sh_demo.fx
#
# Bloklamaydi (server emas) — smoke-test sifatida ham yaroqli.

# sh.run cmd -> {stdout: str  stderr: str  code: int}.
# Buyruq shell orqali boradi, shuning uchun `&&`, quvur (|), glob ishlaydi.

# Oddiy chaqiruv: muvaffaqiyatda code == 0.
r = sh.run "echo salom dunyo"
log "stdout:" r.stdout
log "code:" r.code

# Muvaffaqiyatni tekshirish — code == 0 konvensiyasi.
g = sh.run "git --version"
if g.code == 0
  log "git bor:" g.stdout
else
  log "git topilmadi:" g.stderr

# Shell xususiyatlari: ketma-ket buyruqlar va quvur.
files = sh.run "ls /tmp | head -3"
log "fayllar:\n${files.stdout}"

# Muvaffaqiyatsiz buyruq Flow::err EMAS — code orqali bilinadi.
bad = sh.run "exit 2"
log "muvaffaqiyatsiz buyruq kodi:" bad.code

# stderr alohida tutiladi.
err = sh.run "ls /yoq-papka-aniq"
log "xato oqimi:" err.stderr
log "xato kodi:" err.code
