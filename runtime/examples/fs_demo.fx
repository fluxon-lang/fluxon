# fs battery demo — lokal fayl tizimi primitivlari.
# Ishga: cargo run -- run examples/fs_demo.fx
#
# Bloklamaydi (server emas) — smoke-test sifatida ham yaroqli. Vaqtinchalik
# papkada ishlaydi va oxirida o'zini tozalaydi.

dir = "/tmp/flux_fs_demo"

# Papkani tayyorlash (idempotent — bor bo'lsa xato emas).
fs.mkdirp dir
log "papka tayyor:" dir

# Konfig yozish (json.enc bilan) va qayta o'qish.
conf = "${dir}/conf.json"
fs.write conf (json.enc {port:8080 name:"flux"})
cfg = json.dec (fs.read conf)
log "o'qilgan port:" cfg.port

# Log fayliga ketma-ket qo'shish.
audit = "${dir}/audit.log"
fs.append audit "boshlandi\n"
fs.append audit "tugadi\n"
log "audit mazmuni:" (fs.read audit)

# Yo'q faylni o'qish — nil (xato emas).
yoq = fs.read "${dir}/yoq.txt"
log "yo'q fayl:" yoq

# Papka ichini ko'rish.
log "fayllar:" (fs.ls dir)

# Tozalash.
fs.del conf
fs.del audit
fs.del dir
log "tozalandi, papka mavjudmi:" (fs.exists dir)
