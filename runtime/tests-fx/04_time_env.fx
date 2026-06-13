# 04 - time + env battery.
# Run: FLUXON_TEST_VAR=salom PORT=9090 ./target/release/fluxon run tests-fx/04_time_env.fx

fails <- 0
fn ok label -> log "ok  ${label}"
fn bad label
  log "FAIL ${label}"
  fails <- fails + 1

# --- time.now: UTC text timestamp (DB-compatible "YYYY-MM-DD HH:MM:SS", 19 chars) ---
# NOTE: time.* returns a STRING; in Fluxon `<` does not work on strings, so we
# compare ordering as a number - the year via str.slice + str.int.
t = time.now
if (str.len t) == 19
  ok "time.now format len 19 = ${t}"
else
  bad "time.now wrong len=${str.len t} (${t})"

# separators in the right places
if (str.slice t 4 5) == "-" & (str.slice t 10 11) == " " & (str.slice t 13 14) == ":"
  ok "time.now separators (- space :)"
else
  bad "time.now separators"

# --- time.ago: a point in the past differs from now and is earlier ---
past = time.ago 24 :hr
if past != t
  ok "time.ago 24:hr differs from now (${past})"
else
  bad "time.ago == now"

# units work - all are valid 19-char timestamps
a_sec = time.ago 30 :sec
a_min = time.ago 10 :min
a_day = time.ago 2 :day
if (str.len a_sec) == 19 & (str.len a_min) == 19 & (str.len a_day) == 19
  ok "time.ago :sec/:min/:day all valid timestamps"
else
  bad "time.ago units format"

# ordering: 2 days ago year/month <= 30 seconds ago (date part).
# Turn the date into a number: YYYYMMDD -> int, lexicographic = chronological.
fn datenum ts
  y = str.int (str.slice ts 0 4)
  mo = str.int (str.slice ts 5 7)
  d = str.int (str.slice ts 8 10)
  y * 10000 + mo * 100 + d
if (datenum a_day) <= (datenum a_sec)
  ok "2:day date <= 30:sec date (${datenum a_day} <= ${datenum a_sec})"
else
  bad "time.ago ordering"

# --- time.parse: arbitrary ISO text -> canonical UTC timestamp ---
start_at = time.parse "2026-06-10T10:00:00Z"
if start_at == "2026-06-10 10:00:00"
  ok "time.parse ISO 'Z' -> ${start_at}"
else
  bad "time.parse Z got=${start_at}"

# offset ISO (+05:00) is converted to UTC (time is 5 hours earlier)
tz = time.parse "2026-06-10T15:00:00+05:00"
if tz == "2026-06-10 10:00:00"
  ok "time.parse '+05:00' -> UTC ${tz}"
else
  bad "time.parse offset got=${tz}"

# --- time.add: offset from an arbitrary time (booking: end_at = start_at + duration) ---
end_at = time.add start_at 30 :min
if end_at == "2026-06-10 10:30:00"
  ok "time.add 30:min -> ${end_at}"
else
  bad "time.add got=${end_at}"

# --- time.sub: mirror of add - shifts backward (buffer before) ---
buf = time.sub start_at 5 :min
if buf == "2026-06-10 09:55:00"
  ok "time.sub 5:min -> ${buf}"
else
  bad "time.sub got=${buf}"

# --- time.diff: difference between two times in seconds (int) ---
mins = (time.diff end_at start_at) / 60
if mins == 30
  ok "time.diff -> ${mins} minutes"
else
  bad "time.diff got=${mins}"

# --- env.NAME: environment variable ---
v = env.FLUXON_TEST_VAR
if v == "salom"
  ok "env.FLUXON_TEST_VAR = ${v}"
else
  bad "env.FLUXON_TEST_VAR got=${v}"

# missing variable -> nil, ?? default works
port = env.PORT ?? "8080"
if port == "9090"
  ok "env.PORT (set) = ${port}"
else
  bad "env.PORT got=${port}"

missing = env.DEFINITELY_NOT_SET ?? "default"
if missing == "default"
  ok "env missing -> ?? default"
else
  bad "env missing default got=${missing}"

# --- End ---
if fails == 0
  log "=== 04_time_env: ALL PASSED ==="
else
  log "=== 04_time_env: ${fails} TESTS FAILED ==="
