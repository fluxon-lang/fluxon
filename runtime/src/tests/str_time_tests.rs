use super::*;

// Issue #126: str.trim/replace/starts/ends/pad/repeat — the str functions every
// real project needs on day one.
#[test]
fn str_trim_replace_starts_ends_pad_repeat() {
    run(r#"
(str.trim "  hello  " == "hello") | (fail "str.trim")
(str.trim "hello" == "hello") | (fail "str.trim unchanged")
(str.replace "a-b-c" "-" "+" == "a+b+c") | (fail "str.replace")
(str.replace "abc" "x" "y" == "abc") | (fail "str.replace not found")
(str.replace "abc" "" "y" == "abc") | (fail "str.replace empty pattern")
(str.starts "/api/users" "/api") | (fail "str.starts true")
((str.starts "/api" "/web") == false) | (fail "str.starts false")
(str.ends "file.fx" ".fx") | (fail "str.ends true")
((str.ends "file.fx" ".rs") == false) | (fail "str.ends false")
(str.pad "7" 3 "0" == "007") | (fail "str.pad")
(str.pad "1234" 3 "0" == "1234") | (fail "str.pad long unchanged")
(str.pad "ab" 4 " " == "  ab") | (fail "str.pad whitespace")
(str.repeat "ab" 3 == "ababab") | (fail "str.repeat")
(str.repeat "ab" 0 == "") | (fail "str.repeat zero")
"#);
}

// str.url_enc — RFC 3986 percent-encoding (the AWS SigV4 `UriEncode`). Reserved
// chars become uppercase %XX; unreserved (A-Za-z0-9-_.~) pass through; `/` and
// space are encoded; non-ASCII goes byte-by-byte over its UTF-8 encoding.
#[test]
fn str_url_enc() {
    run(r#"
(str.url_enc "abc" == "abc") | (fail "url_enc unreserved")
(str.url_enc "a-_.~z" == "a-_.~z") | (fail "url_enc unreserved set")
(str.url_enc "my file.png" == "my%20file.png") | (fail "url_enc space")
(str.url_enc "a/b" == "a%2Fb") | (fail "url_enc slash uppercase")
(str.url_enc "a=b&c" == "a%3Db%26c") | (fail "url_enc query chars")
(str.url_enc "é" == "%C3%A9") | (fail "url_enc utf8")
(str.url_enc "" == "") | (fail "url_enc empty")
"#);
}

// str.repeat with a negative number and str.pad with an empty filler — a clear
// error (not a silent wrong result).
// Issue #213: str.slice's end index is OPTIONAL and defaults to end-of-string,
// matching the Python/JS prior (`s[a:]`). Small models assume slice-to-end; the
// mandatory 3rd arg was a needless trap.
#[test]
fn str_slice_to_end_default() {
    run(r#"
(str.slice "Bearer xyz" 7 == "xyz") | (fail "slice to end")
(str.slice "hello" 0 == "hello") | (fail "slice from 0 to end")
(str.slice "hello" 5 == "") | (fail "slice at len is empty")
(str.slice "hello" 9 == "") | (fail "slice past len is empty")
# explicit end still works exactly as before
(str.slice "hello" 1 3 == "el") | (fail "slice with explicit end")
(str.slice "hello" 1 (str.len "hello") == "ello") | (fail "the old to-end idiom")
# unicode: indices are in chars, not bytes
(str.slice "héllo" 1 == "éllo") | (fail "slice unicode to end")
"#);
}

#[test]
fn str_repeat_negative_and_pad_empty_fail() {
    assert!(run_source(r#"str.repeat "a" (0 - 1)"#).is_err());
    assert!(run_source(r#"str.pad "a" 3 """#).is_err());
    // Even if the bytes fit in usize, exceeding isize::MAX (the allocation limit)
    // gives a Fluxon error, not a panic (PR #151 review).
    assert!(run_source(r#"str.repeat "aa" 4611686018427387904"#).is_err());
    assert!(run_source(r#"str.pad "x" 4611686018427387904 "🙂""#).is_err());
}

#[test]
fn time_module_fmt_and_roundtrip() {
    // time.fmt is deterministic with a unix int: 1700000000 = 2023-11-14 22:13:20 UTC.
    // We check the time.now/time.ago text format ("YYYY-MM-DD HH:MM:SS") and
    // round-trip it through fmt.
    run(r#"
d = time.fmt 1700000000 "YYYY-MM-DD"
(d == "2023-11-14") | (fail "fmt sana wrong: ${d}")
t = time.fmt 1700000000 "HH:mm:ss"
(t == "22:13:20") | (fail "fmt vaqt wrong: ${t}")
n = time.now
(str.len n == 19) | (fail "time.now uzunligi 19 not: ${n}")
back = time.fmt n "YYYY"
(str.len back == 4) | (fail "time.now -> fmt yil 4 raqam not")
"#);
}

#[test]
fn time_ago_is_earlier() {
    // time.ago is before now: the ISO text format is lexicographic = chronological,
    // so a DB filter (`created > $1`) works correctly in SQL. Here we prove the
    // chronological order by comparing the year/month/day parts.
    run(r#"
now = time.now
past = time.ago 1 :day
ny = str.int (time.fmt now "YYYYMMDDHHmmss")
py = str.int (time.fmt past "YYYYMMDDHHmmss")
(py < ny) | (fail "time.ago kelajakda: past=${past} now=${now}")
"#);
}

#[test]
fn time_in_is_later() {
    // time.in is after now (for TTL/expiry). The mirror of time.ago:
    // ISO text is lexicographic = chronological, so the `expires > $now`
    // SQL filter works correctly. We compare the year/month/.../sec parts.
    run(r#"
now = time.now
soon = time.in 1 :hr
ny = str.int (time.fmt now "YYYYMMDDHHmmss")
sy = str.int (time.fmt soon "YYYYMMDDHHmmss")
(sy > ny) | (fail "time.in in the past: soon=${soon} now=${now}")
"#);
}

#[test]
fn time_parse_add_diff_booking_flow() {
    // Issue #65: the client gives an ISO `start_at` and `duration_minutes` ->
    // the server computes `end_at`. The e2e scenario of the booking core.
    run(r#"
start_at = time.parse "2026-06-10T10:00:00Z"
(start_at == "2026-06-10 10:00:00") | (fail "parse wrong: ${start_at}")
end_at = time.add start_at 30 :min
(end_at == "2026-06-10 10:30:00") | (fail "add wrong: ${end_at}")
mins = (time.diff end_at start_at) / 60
(mins == 30) | (fail "diff wrong: ${mins}")
# buffer-inclusive interval: start - 5min (time.sub — the mirror of add)
buf_start = time.sub start_at 5 :min
(buf_start == "2026-06-10 09:55:00") | (fail "time.sub wrong: ${buf_start}")
"#);
}

#[test]
fn time_parse_handles_iso_offset() {
    // ISO text with an offset is brought to UTC (+05:00 -> the time is 5 hours earlier).
    run(r#"
t = time.parse "2026-06-10T15:00:00+05:00"
(t == "2026-06-10 10:00:00") | (fail "mintaqa UTC ga kelmadi: ${t}")
"#);
}

#[test]
fn time_parse_fmt_iana_zone_dst() {
    // Issue #80: DST-aware conversion with an IANA zone name. "09:00 local"
    // maps to different UTC in winter and summer — not a fixed offset.
    run(r#"
# winter (EST = UTC-5): 09:00 local -> 14:00 UTC
w = time.parse "2026-01-15 09:00:00" "America/New_York"
(w == "2026-01-15 14:00:00") | (fail "winter DST wrong: ${w}")
# summer (EDT = UTC-4): the exact same wall-clock -> 13:00 UTC
s = time.parse "2026-07-15 09:00:00" "America/New_York"
(s == "2026-07-15 13:00:00") | (fail "summer DST wrong: ${s}")
# reverse path: UTC instant -> the zone's wall-clock (for display)
back = time.fmt s "HH:mm" "America/New_York"
(back == "09:00") | (fail "fmt zone wrong: ${back}")
"#);
}
