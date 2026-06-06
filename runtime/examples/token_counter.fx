# Token hisoblagich — docs va misollar Claude uchun qancha token sarflashini O'LCHAYDI.
#
# Nega: character-base taxmin (char/4) aniq emas — Flux docs'ida (o'zbekcha matn +
# kod) char/token nisbati ~1.9, ya'ni char/4 taxmini qariyb 2 baravar past
# ko'rsatadi. Aniq raqam uchun Anthropic'ning count_tokens API'si kerak (bepul,
# faqat RPM limiti bor).
#
# Bu tool Flux'ning o'zida yozilgan (dogfooding): fs.read/fs.ls + http.post +
# custom header (x-api-key). HTTPS, fs va so'rov header'lari batareyalardan keladi.
#
# Ishga tushirish (runtime/ ichida):
#   ANTHROPIC_API_KEY=sk-... cargo run -- run examples/token_counter.fx

use http

key = env.ANTHROPIC_API_KEY
if key == nil
  fail "ANTHROPIC_API_KEY env o'rnatilmagan (count_tokens API uchun kerak)"

# count_tokens API har doim shu model va versiya bilan ishlaydi.
model = "claude-opus-4-8"

# Bitta matn uchun aniq token sonini qaytaradi (yoki xato bo'lsa fail).
count_tokens = \text ->
  res = http.post "https://api.anthropic.com/v1/messages/count_tokens" {
    model: model
    messages: [{role: "user" content: text}]
  } {
    headers: {
      "x-api-key": key
      "anthropic-version": "2023-06-01"
    }
  }
  if res.status != 200
    fail "count_tokens xato (status ${res.status}): ${json.enc res.body}"
  res.body.input_tokens

# char/token nisbatini "1.93" ko'rinishida formatlaydi (2 kasr xona).
# Butun arifmetika: (chars*100)/tokens → 193 → "1.93".
ratio_str = \chars tokens ->
  r = math.floor ((chars * 100) / tokens)
  whole = math.floor (r / 100)
  frac = r % 100
  # frac bir xonali bo'lsa old nol qo'shamiz ("1.9" emas "1.09").
  if frac < 10
    "${whole}.0${frac}"
  else
    "${whole}.${frac}"

# Bitta faylni o'lchaydi va natija map'ini qaytaradi {path chars tokens}.
# Fayl yo'q bo'lsa nil (chaqiruvchi o'tkazib yuboradi).
measure = \path ->
  text = fs.read path
  if text == nil
    ret nil
  chars = str.len text
  tokens = count_tokens text
  {path: path chars: chars tokens: tokens}

# Papkadagi barcha .md (yoki berilgan kengaytma) fayllarni o'lchaydi.
# dir oxirida "/" bo'lishi shart emas — qo'shib beramiz.
measure_dir = \dir ext ->
  names = fs.ls dir
  out <- []
  each name in names
    if str.has name ext
      m = measure "${dir}/${name}"
      if m != nil
        out <- out.push m
  out

# --- Skanerlash: docs/*.md va examples/*.fx (worktree ildizidan nisbiy) ---
log "=== Token hisobi (model: ${model}) ==="
log ""

groups = [
  {label: "docs (*.md)" dir: "../docs" ext: ".md"}
  {label: "examples (*.fx)" dir: "examples" ext: ".fx"}
]

grand_tokens <- 0
grand_chars <- 0

each g in groups
  log "--- ${g.label} ---"
  rows = measure_dir g.dir g.ext
  sub_tokens <- 0
  sub_chars <- 0
  each row in rows
    sub_tokens <- sub_tokens + row.tokens
    sub_chars <- sub_chars + row.chars
    log "  ${row.path}"
    log "    belgilar: ${row.chars}  ·  tokenlar: ${row.tokens}  ·  char/tok: ${ratio_str row.chars row.tokens}"
  log "  guruh jami: ${sub_tokens} tok (${sub_chars} belgi, char/4 taxmini: ${math.floor (sub_chars / 4)})"
  log ""
  grand_tokens <- grand_tokens + sub_tokens
  grand_chars <- grand_chars + sub_chars

log "=== UMUMIY JAMI ==="
log "belgilar: ${grand_chars}"
log "tokenlar: ${grand_tokens}  (aniq, Claude)"
log "char/4 taxmini: ${math.floor (grand_chars / 4)} tok  (xato!)"
log "haqiqiy char/tok nisbati: ${ratio_str grand_chars grand_tokens}"
