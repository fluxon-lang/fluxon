# Token counter — MEASURES how many tokens the docs and examples cost for Claude.
#
# Why: the character-based estimate (char/4) is inaccurate — in the Fluxon docs
# (Uzbek text + code) the char/token ratio is ~1.9, i.e. the char/4 estimate is
# off by nearly 2x on the low side. For an exact number you need Anthropic's
# count_tokens API (free, only has an RPM limit).
#
# This tool is written in Fluxon itself (dogfooding): fs.read/fs.ls + http.post +
# a custom header (x-api-key). HTTPS, fs and request headers come from batteries.
#
# Running (inside runtime/):
#   ANTHROPIC_API_KEY=sk-... cargo run -- run examples/token_counter.fx

use http

key = env.ANTHROPIC_API_KEY
if key == nil
  fail "ANTHROPIC_API_KEY env is not set (needed for the count_tokens API)"

# The count_tokens API always works with this model and version.
model = "claude-opus-4-8"

# Returns the exact token count for one text (or fails on error).
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
    fail "count_tokens error (status ${res.status}): ${json.enc res.body}"
  res.body.input_tokens

# Formats the char/token ratio as "1.93" (2 decimal places).
# Integer arithmetic: (chars*100)/tokens -> 193 -> "1.93".
ratio_str = \chars tokens ->
  r = math.floor ((chars * 100) / tokens)
  whole = math.floor (r / 100)
  frac = r % 100
  # If frac is single-digit, add a leading zero ("1.09" not "1.9").
  if frac < 10
    "${whole}.0${frac}"
  else
    "${whole}.${frac}"

# Measures one file and returns a result map {path chars tokens}.
# If the file is missing, nil (the caller skips it).
measure = \path ->
  text = fs.read path
  if text == nil
    ret nil
  chars = str.len text
  tokens = count_tokens text
  {path: path chars: chars tokens: tokens}

# Measures all .md (or the given extension) files in a folder.
# A trailing "/" on dir is not required — we add it.
measure_dir = \dir ext ->
  names = fs.ls dir
  out <- []
  each name in names
    if str.has name ext
      m = measure "${dir}/${name}"
      if m != nil
        out <- out.push m
  out

# --- Scanning: docs/*.md and examples/*.fx (relative to the worktree root) ---
log "=== Token count (model: ${model}) ==="
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
    log "    chars: ${row.chars}  |  tokens: ${row.tokens}  |  char/tok: ${ratio_str row.chars row.tokens}"
  log "  group total: ${sub_tokens} tok (${sub_chars} chars, char/4 estimate: ${math.floor (sub_chars / 4)})"
  log ""
  grand_tokens <- grand_tokens + sub_tokens
  grand_chars <- grand_chars + sub_chars

log "=== GRAND TOTAL ==="
log "chars: ${grand_chars}"
log "tokens: ${grand_tokens}  (exact, Claude)"
log "char/4 estimate: ${math.floor (grand_chars / 4)} tok  (wrong!)"
log "real char/tok ratio: ${ratio_str grand_chars grand_tokens}"
