// ---------------- time ----------------
// All times are UTC text in "YYYY-MM-DD HH:MM:SS" format — EXACTLY the same as
// SQLite CURRENT_TIMESTAMP (the tbl `now` column), so DB filters like
// `created > (time.ago 24 :hr)` work directly.
use crate::builtins::R;
use crate::builtins::args::*;
use crate::interp::Flow;
use crate::value::Value;

pub(crate) fn time_module(func: &str, args: Vec<Value>) -> R {
    match func {
        // current time -> UTC text timestamp
        "now" => Ok(Value::Str(fmt_unix(now_unix()))),
        // time.ago N :unit -> UTC text N units before now
        "ago" => {
            let n = arg_int(&args, 0, "time.ago")?;
            let unit = arg_str(&args, 1, "time.ago")?;
            let secs = unit_secs(&unit).ok_or_else(|| {
                Flow::err(format!(
                    "time.ago: unit must be :sec/:min/:hr/:day, got :{}",
                    unit
                ))
            })?;
            // For large N, n * secs (or the subtraction) overflows i64 — checked.
            let ts = n
                .checked_mul(secs)
                .and_then(|off| now_unix().checked_sub(off))
                .ok_or_else(|| Flow::overflow("time.ago"))?;
            Ok(Value::Str(fmt_unix(ts)))
        }
        // time.in N :unit -> UTC text N units AFTER now (TTL/expiry).
        // The mirror of time.ago — the only difference is the add/subtract sign.
        "in" => {
            let n = arg_int(&args, 0, "time.in")?;
            let unit = arg_str(&args, 1, "time.in")?;
            let secs = unit_secs(&unit).ok_or_else(|| {
                Flow::err(format!(
                    "time.in: unit must be :sec/:min/:hr/:day, got :{}",
                    unit
                ))
            })?;
            let ts = n
                .checked_mul(secs)
                .and_then(|off| now_unix().checked_add(off))
                .ok_or_else(|| Flow::overflow("time.in"))?;
            Ok(Value::Str(fmt_unix(ts)))
        }
        // time.sleep secs -> waits secs seconds (flt too — 0.5 = half a second).
        // For polling/retry backoff: waiting before retrying on an error (to avoid
        // a burst/rate-limit loop). A negative value is clamped to 0
        // (Duration::from_secs_f64 panics on a negative value).
        "sleep" => {
            let secs = arg_num(&args, 0, "time.sleep")?.max(0.0);
            std::thread::sleep(std::time::Duration::from_secs_f64(secs));
            Ok(Value::Nil)
        }
        // time.fmt timestamp "..." -> text formatting.
        // Input: a text timestamp ("YYYY-MM-DD HH:MM:SS", ISO with zone too) or a unix int.
        // Tokens: YYYY MM DD HH mm ss. By default formats the UTC wall-clock.
        //
        // Optional 3rd argument — an IANA zone name: `time.fmt t "HH:mm" "Asia/Tashkent"`.
        // Converts the UTC instant to that zone's local wall-clock (DST aware) and
        // formats it — to show the user a local time.
        "fmt" => {
            let ts = arg_ts(&args, 0, "time.fmt")?;
            let pat = arg_str(&args, 1, "time.fmt")?;
            match args.get(2) {
                Some(_) => {
                    let zone = arg_str(&args, 2, "time.fmt")?;
                    let out = fmt_in_zone(ts, &pat, &zone).ok_or_else(|| {
                        Flow::err(format!("time.fmt: unknown IANA zone name: {}", zone))
                    })?;
                    Ok(Value::Str(out))
                }
                None => Ok(Value::Str(strftime(ts, &pat))),
            }
        }
        // time.parse "2026-06-10T10:00:00Z" -> canonical UTC text timestamp.
        // Normalizes an arbitrary ISO-8601 text (from a client/external API) to the
        // internal canonical "YYYY-MM-DD HH:MM:SS" UTC format — so time.add/time.diff
        // and DB filters work with it directly. Understands "Z", "±HH:MM"/"±HHMM"
        // zones and fractional seconds; text without a zone is taken as UTC.
        //
        // Optional 2nd argument — an IANA zone name: `time.parse "2026-03-08 09:00" "America/New_York"`.
        // In this case the wall-clock time in the text is interpreted in that zone
        // (DST aware) and converted to UTC — not a fixed offset. "09:00 local" maps
        // to the correct UTC every day, including across DST transitions (PRD §6.8).
        "parse" => {
            let s = arg_str(&args, 0, "time.parse")?;
            let ts = match args.get(1) {
                Some(_) => {
                    let zone = arg_str(&args, 1, "time.parse")?;
                    parse_in_zone(&s, &zone).ok_or_else(|| {
                        Flow::err(format!(
                            "time.parse: could not parse time '{}' in zone '{}' \
                             (unknown zone or nonexistent local time during a DST jump)",
                            s, zone
                        ))
                    })?
                }
                None => parse_iso(&s).ok_or_else(|| {
                    Flow::err(format!(
                        "time.parse: could not parse ISO timestamp text: {}",
                        s
                    ))
                })?,
            };
            Ok(Value::Str(fmt_unix(ts)))
        }
        // time.add t N :unit -> returns UTC text with N units ADDED to timestamp t.
        // Unlike time.in: it offsets from an ARBITRARY given time, not from now
        // (e.g. end_at = start_at + duration). If N is negative it subtracts (shifts back).
        "add" => {
            let base = arg_ts(&args, 0, "time.add")?;
            let n = arg_int(&args, 1, "time.add")?;
            let unit = arg_str(&args, 2, "time.add")?;
            let secs = unit_secs(&unit).ok_or_else(|| {
                Flow::err(format!(
                    "time.add: unit must be :sec/:min/:hr/:day, got :{}",
                    unit
                ))
            })?;
            let ts = n
                .checked_mul(secs)
                .and_then(|off| base.checked_add(off))
                .ok_or_else(|| Flow::overflow("time.add"))?;
            Ok(Value::Str(fmt_unix(ts)))
        }
        // time.sub t N :unit -> returns UTC text with N units SUBTRACTED from timestamp t.
        // The mirror of time.add (like the time.ago/time.in pair). A separate function
        // to avoid a negative number being confused with the binary `-` in a bare call —
        // a buffer-inclusive interval start is written as `time.sub start_at 5 :min`.
        "sub" => {
            let base = arg_ts(&args, 0, "time.sub")?;
            let n = arg_int(&args, 1, "time.sub")?;
            let unit = arg_str(&args, 2, "time.sub")?;
            let secs = unit_secs(&unit).ok_or_else(|| {
                Flow::err(format!(
                    "time.sub: unit must be :sec/:min/:hr/:day, got :{}",
                    unit
                ))
            })?;
            let ts = n
                .checked_mul(secs)
                .and_then(|off| base.checked_sub(off))
                .ok_or_else(|| Flow::overflow("time.sub"))?;
            Ok(Value::Str(fmt_unix(ts)))
        }
        // time.diff a b -> (a - b) the difference between two times IN SECONDS (int).
        // A positive result = a is after b (in the future). Divide by a unit
        // (e.g. `(time.diff end start) / 60` -> duration in minutes).
        "diff" => {
            let a = arg_ts(&args, 0, "time.diff")?;
            let b = arg_ts(&args, 1, "time.diff")?;
            Ok(Value::Int(a - b))
        }
        _ => Err(Flow::err(format!("time module has no function '{}'", func))),
    }
}

pub(crate) fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn unit_secs(unit: &str) -> Option<i64> {
    match unit {
        "sec" => Some(1),
        "min" => Some(60),
        "hr" => Some(3600),
        "day" => Some(86_400),
        _ => None,
    }
}

// unix seconds -> (year, month, day, hour, min, sec) UTC.
// civil_from_days: Howard Hinnant's algorithm (dependency-free, constant time).
fn civil(unix: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = unix.div_euclid(86_400);
    let secs_of_day = unix.rem_euclid(86_400);
    let (hh, mm, ss) = (
        (secs_of_day / 3600) as u32,
        ((secs_of_day % 3600) / 60) as u32,
        (secs_of_day % 60) as u32,
    );
    // days: counted from 1970-01-01. Hinnant: starts the era in March.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11] (March=0)
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, hh, mm, ss)
}

pub(crate) fn fmt_unix(unix: i64) -> String {
    let (y, mo, d, h, mi, s) = civil(unix);
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, mo, d, h, mi, s)
}

// "YYYY-MM-DD HH:MM:SS" (or "YYYY-MM-DDTHH:MM:SS") -> unix seconds (UTC).
fn parse_ts(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    // Accept either a full "YYYY-MM-DD HH:MM:SS" base (>= 19) or a date-only
    // "YYYY-MM-DD" (exactly 10) — the latter is treated as midnight, so calendar
    // dates flow through time arithmetic without a hand-appended " 00:00:00".
    // A partial time (e.g. "...HH:MM", 11..18 chars) is rejected: silently
    // dropping the minutes a caller wrote would be a surprising data loss.
    let date_only = b.len() == 10;
    if !date_only && b.len() < 19 {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> { s.get(a..z)?.parse::<i64>().ok() };
    let y = num(0, 4)?;
    let mo = num(5, 7)?;
    let d = num(8, 10)?;
    let (h, mi, se) = if date_only {
        (0, 0, 0)
    } else {
        (num(11, 13)?, num(14, 16)?, num(17, 19)?)
    };
    // Validate the ranges — days_from_civil silently "fixes" an overflow (a
    // nonexistent 02-31 -> 03-03), so we reject it here: a wrong date must not be
    // accepted silently in a booking flow.
    // se 60 — a leap second (ISO allows it) — we accept it.
    if !(1..=12).contains(&mo)
        || !(1..=days_in_month(y, mo)).contains(&d)
        || !(0..=23).contains(&h)
        || !(0..=59).contains(&mi)
        || !(0..=60).contains(&se)
    {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se)
}

// Number of days for the given year/month (leap year aware).
fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
            if leap { 29 } else { 28 }
        }
        _ => 0, // invalid month — the caller already checks mo
    }
}

// Converts an arbitrary ISO-8601 text to unix seconds (UTC). Built on top of
// parse_ts: first reads the date+time base ("YYYY-MM-DD[ T]HH:MM:SS"), then from
// the part after the 19th char understands an optional fractional second (".sss"
// — dropped, since we work at second precision) and a time zone ("Z", "±HH:MM",
// "±HHMM", "±HH"). With no zone it is taken as UTC. The text time is local ->
// UTC = time - offset. Timestamps are ASCII, so byte index = char index (boundary safe).
pub(crate) fn parse_iso(s: &str) -> Option<i64> {
    let s = s.trim();
    let base = parse_ts(s)?; // date+time (>= 19 chars) or a date-only midnight (10 chars)
    // The remainder (fractional second / zone) starts right after the base; a
    // date-only string has nothing after it.
    let base_len = if s.len() == 10 { 10 } else { 19 };
    let mut rest = &s[base_len..];
    // skip the fractional second (".123") — we work at second precision.
    if let Some(after_dot) = rest.strip_prefix('.') {
        let digits = after_dot.bytes().take_while(|b| b.is_ascii_digit()).count();
        rest = &after_dot[digits..];
    }
    let offset = match rest.chars().next() {
        None => 0,                  // no zone -> UTC
        Some('Z') | Some('z') => 0, // Zulu (UTC)
        Some(sign @ ('+' | '-')) => {
            // ignore ":" and take only the digits: HHMM or HH.
            let digits: String = rest[1..].chars().filter(|c| c.is_ascii_digit()).collect();
            let (hh, mm) = match digits.len() {
                2 => (digits.parse::<i64>().ok()?, 0),
                4 => (
                    digits[0..2].parse::<i64>().ok()?,
                    digits[2..4].parse::<i64>().ok()?,
                ),
                _ => return None,
            };
            let off = hh * 3600 + mm * 60;
            if sign == '-' { -off } else { off }
        }
        _ => return None, // unrecognized remainder -> invalid text
    };
    Some(base - offset)
}

// (year, month, day) UTC -> days since 1970-01-01 (Hinnant inverse).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 }; // March=0
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn strftime(unix: i64, pat: &str) -> String {
    let (y, mo, d, h, mi, s) = civil(unix);
    strftime_fields(y, mo, d, h, mi, s, pat)
}

// Builds text from date/time fields — extracted so the UTC (civil) and
// zone-aware (fmt_in_zone) paths use the same token set.
fn strftime_fields(y: i64, mo: u32, d: u32, h: u32, mi: u32, s: u32, pat: &str) -> String {
    pat.replace("YYYY", &format!("{:04}", y))
        .replace("MM", &format!("{:02}", mo))
        .replace("DD", &format!("{:02}", d))
        .replace("HH", &format!("{:02}", h))
        .replace("mm", &format!("{:02}", mi))
        .replace("ss", &format!("{:02}", s))
}

// Interprets a wall-clock string in an IANA zone (DST aware) and converts it to
// UTC seconds. Reads the parse_ts base (date+time, no zone), then treats those
// fields as the zone's local time — not a fixed offset, so summer/winter (DST)
// transitions work correctly.
//
// DST edges: during a spring-forward jump a nonexistent local time (e.g. 02:30)
// -> None (the caller returns an error). On a fall-back repeat (the time occurs
// twice) the earlier (DST) instant is chosen — a deterministic, safe default for
// booking.
fn parse_in_zone(s: &str, zone: &str) -> Option<i64> {
    use chrono::offset::LocalResult;
    use chrono::{NaiveDate, TimeZone};
    let tz: chrono_tz::Tz = zone.parse().ok()?;
    // parse_ts gives the wall-clock as "fake UTC" seconds; we turn it back into
    // fields with civil and re-interpret them in the zone.
    let (y, mo, d, h, mi, se) = civil(parse_ts(s)?);
    let naive = NaiveDate::from_ymd_opt(y as i32, mo, d)?.and_hms_opt(h, mi, se)?;
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Some(dt.timestamp()),
        LocalResult::Ambiguous(earlier, _later) => Some(earlier.timestamp()),
        LocalResult::None => None,
    }
}

// Converts a UTC instant to the IANA zone's local wall-clock (DST aware) and
// formats it. None for an unknown zone name.
fn fmt_in_zone(unix: i64, pat: &str, zone: &str) -> Option<String> {
    use chrono::{Datelike, TimeZone, Timelike, Utc};
    let tz: chrono_tz::Tz = zone.parse().ok()?;
    let dt = Utc.timestamp_opt(unix, 0).single()?.with_timezone(&tz);
    Some(strftime_fields(
        dt.year() as i64,
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
        pat,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Known unix points (UTC) — we check the chrono-free civil algorithm.
    #[test]
    fn civil_known_points() {
        assert_eq!(fmt_unix(0), "1970-01-01 00:00:00"); // epoch
        assert_eq!(fmt_unix(1_700_000_000), "2023-11-14 22:13:20");
        // 2024-02-29 (leap year) — 12:00:00 UTC
        assert_eq!(fmt_unix(1_709_208_000), "2024-02-29 12:00:00");
    }

    #[test]
    fn parse_then_fmt_roundtrip() {
        for &u in &[0i64, 1_700_000_000, 1_709_208_000, 4_102_444_800] {
            let s = fmt_unix(u);
            assert_eq!(parse_ts(&s), Some(u), "round-trip broken: {}", s);
        }
        // the 'T' separator is supported too (ISO).
        assert_eq!(parse_ts("2023-11-14T22:13:20"), Some(1_700_000_000));
    }

    #[test]
    fn ago_subtracts_units() {
        let now = now_unix();
        // 24 hours = 1 day: both paths give the same result (text).
        assert_eq!(fmt_unix(now - 24 * 3600), fmt_unix(now - 86_400));
    }

    #[test]
    fn parse_rejects_garbage() {
        assert_eq!(parse_ts("hello"), None);
        // A bare date is now accepted as midnight (issue #175); a *partial* time
        // (date + truncated clock) is still rejected.
        assert_eq!(parse_ts("2023-11-14"), parse_ts("2023-11-14 00:00:00"));
        assert_eq!(parse_ts("2023-11-14 09:3"), None);
    }

    #[test]
    fn in_adds_units() {
        // time.in gives the future, time.ago the past — the result is after/before now.
        let now = now_unix();
        let Ok(Value::Str(f)) = time_module("in", vec![Value::Int(1), Value::Str("hr".into())])
        else {
            panic!("time.in must return a string");
        };
        let Ok(Value::Str(p)) = time_module("ago", vec![Value::Int(1), Value::Str("hr".into())])
        else {
            panic!("time.ago must return a string");
        };
        let (Some(fu), Some(pu)) = (parse_ts(&f), parse_ts(&p)) else {
            panic!("could not parse timestamps");
        };
        // 1 hour after > now > 1 hour before (a one-second rounding does not shift it off).
        assert!(
            fu >= now + 3600 - 1 && fu <= now + 3600 + 1,
            "time.in incorrect: {}",
            f
        );
        assert!(
            pu >= now - 3600 - 1 && pu <= now - 3600 + 1,
            "time.ago incorrect: {}",
            p
        );
    }

    #[test]
    fn in_rejects_bad_unit() {
        let r = time_module("in", vec![Value::Int(1), Value::Str("year".into())]);
        assert!(r.is_err(), "unknown unit must return an error");
    }

    #[test]
    fn sleep_waits_and_returns_nil() {
        use std::time::Instant;
        // A short flt delay — we check that a fraction is accepted too, not just int.
        let t0 = Instant::now();
        let r = time_module("sleep", vec![Value::Flt(0.05)]);
        let elapsed = t0.elapsed();
        assert!(matches!(r, Ok(Value::Nil)), "time.sleep must return nil");
        assert!(
            elapsed.as_millis() >= 45,
            "time.sleep must wait at least the expected duration: {:?}",
            elapsed
        );
    }

    #[test]
    fn sleep_negative_clamps_to_zero() {
        // A negative value must not panic — it is clamped to 0.
        let r = time_module("sleep", vec![Value::Int(-5)]);
        assert!(
            matches!(r, Ok(Value::Nil)),
            "negative sleep must return nil"
        );
    }

    #[test]
    fn parse_iso_handles_z_and_offsets() {
        // "Z" -> UTC; "+HH:MM"/"-HH:MM" zone is converted to UTC.
        let z = parse_iso("2026-06-10T10:00:00Z").expect("Z must parse");
        assert_eq!(parse_iso("2026-06-10 10:00:00"), Some(z)); // no zone = UTC
        // +05:00: the text time is local, UTC is 5 hours earlier.
        assert_eq!(parse_iso("2026-06-10T15:00:00+05:00"), Some(z));
        // -05:00: UTC is 5 hours later.
        assert_eq!(parse_iso("2026-06-10T05:00:00-05:00"), Some(z));
        // "+HHMM" (without the colon) and a fractional second are read too.
        assert_eq!(parse_iso("2026-06-10T15:00:00.123+0500"), Some(z));
    }

    #[test]
    fn time_parse_normalizes_to_canonical_utc() {
        // time.parse normalizes ISO text to canonical "YYYY-MM-DD HH:MM:SS" UTC.
        let Ok(Value::Str(s)) =
            time_module("parse", vec![Value::Str("2026-06-10T10:00:00Z".into())])
        else {
            panic!("time.parse must return a string");
        };
        assert_eq!(s, "2026-06-10 10:00:00");
    }

    #[test]
    fn time_parse_rejects_garbage() {
        let r = time_module("parse", vec![Value::Str("hello".into())]);
        assert!(r.is_err(), "invalid text must return an error");
    }

    #[test]
    fn parse_ts_rejects_impossible_dates() {
        // A nonexistent date/time must not be silently "fixed" — it must be rejected
        // (days_from_civil normalizes the overflow, we prevent it).
        assert_eq!(parse_ts("2026-02-31T10:00:00Z"), None); // no Feb 31
        assert_eq!(parse_ts("2026-02-29 00:00:00"), None); // 2026 is not a leap year
        assert_eq!(parse_ts("2026-13-01 00:00:00"), None); // no month 13
        assert_eq!(parse_ts("2026-00-10 00:00:00"), None); // no month 0
        assert_eq!(parse_ts("2026-06-00 00:00:00"), None); // no day 0
        assert_eq!(parse_ts("2026-06-10 24:00:00"), None); // no hour 24
        assert_eq!(parse_ts("2026-06-10 10:60:00"), None); // no minute 60
        // Real edge cases are ACCEPTED:
        assert!(parse_ts("2024-02-29 00:00:00").is_some()); // 2024 leap
        assert!(parse_ts("2026-12-31 23:59:60").is_some()); // leap second (60)
    }

    #[test]
    fn parse_ts_accepts_date_only_as_midnight() {
        // Issue #175: a date-only "YYYY-MM-DD" is treated as midnight, so calendar
        // dates flow through time arithmetic without appending " 00:00:00".
        assert_eq!(parse_ts("2026-06-25"), parse_ts("2026-06-25 00:00:00"));
        assert_eq!(parse_iso("2026-06-25"), parse_iso("2026-06-25 00:00:00"));
        // Date-only still validates the calendar (no silent overflow).
        assert_eq!(parse_ts("2026-02-29"), None); // 2026 is not a leap year
        assert_eq!(parse_ts("2026-13-01"), None); // no month 13
        // A truncated time is rejected rather than silently dropped.
        assert_eq!(parse_ts("2026-06-25 09:00"), None);
        assert_eq!(parse_ts("2026-06-25 09"), None);
    }

    #[test]
    fn time_add_accepts_date_only() {
        // Issue #175: time.add on a bare date works (treated as midnight).
        let Ok(Value::Str(end)) = time_module(
            "add",
            vec![
                Value::Str("2026-06-25".into()),
                Value::Int(30),
                Value::Str("min".into()),
            ],
        ) else {
            panic!("time.add must accept a date-only string");
        };
        assert_eq!(end, "2026-06-25 00:30:00");
    }

    #[test]
    fn time_parse_rejects_impossible_date() {
        let r = time_module("parse", vec![Value::Str("2026-02-31T10:00:00Z".into())]);
        assert!(r.is_err(), "02-31 does not exist — must return an error");
    }

    #[test]
    fn time_add_offsets_arbitrary_timestamp() {
        // Core of issue #65: start_at + duration -> end_at.
        let Ok(Value::Str(end)) = time_module(
            "add",
            vec![
                Value::Str("2026-06-10 10:00:00".into()),
                Value::Int(30),
                Value::Str("min".into()),
            ],
        ) else {
            panic!("time.add must return a string");
        };
        assert_eq!(end, "2026-06-10 10:30:00");
        // A negative N shifts backward.
        let Ok(Value::Str(before)) = time_module(
            "add",
            vec![
                Value::Str("2026-06-10 10:00:00".into()),
                Value::Int(-2),
                Value::Str("hr".into()),
            ],
        ) else {
            panic!("time.add must return a string");
        };
        assert_eq!(before, "2026-06-10 08:00:00");
    }

    #[test]
    fn time_add_rejects_bad_unit() {
        let r = time_module(
            "add",
            vec![
                Value::Str("2026-06-10 10:00:00".into()),
                Value::Int(1),
                Value::Str("year".into()),
            ],
        );
        assert!(r.is_err(), "unknown unit must return an error");
    }

    // Issue #89: if the n * secs product (or the final sum) overflows i64, a
    // Fluxon error is returned instead of a panic/silent wrap — in all four offset
    // functions.
    #[test]
    fn time_offsets_overflow_is_error() {
        let big = Value::Int(i64::MAX / 2);
        let day = Value::Str("day".into());
        for func in ["ago", "in"] {
            let r = time_module(func, vec![big.clone(), day.clone()]);
            let Err(Flow::Error(msg)) = r else {
                panic!("time.{} must return an error on overflow", func);
            };
            assert!(msg.contains("number out of range"), "error text: {}", msg);
        }
        let base = Value::Str("2026-06-10 10:00:00".into());
        for func in ["add", "sub"] {
            let r = time_module(func, vec![base.clone(), big.clone(), day.clone()]);
            let Err(Flow::Error(msg)) = r else {
                panic!("time.{} must return an error on overflow", func);
            };
            assert!(msg.contains("number out of range"), "error text: {}", msg);
        }
    }

    #[test]
    fn time_sub_offsets_backward() {
        // time.sub — the mirror of add: shifts backward from a given time.
        let Ok(Value::Str(s)) = time_module(
            "sub",
            vec![
                Value::Str("2026-06-10 10:00:00".into()),
                Value::Int(5),
                Value::Str("min".into()),
            ],
        ) else {
            panic!("time.sub must return a string");
        };
        assert_eq!(s, "2026-06-10 09:55:00");
    }

    #[test]
    fn time_diff_returns_seconds() {
        // diff a b = a - b in seconds; positive = a is in the future.
        let r = time_module(
            "diff",
            vec![
                Value::Str("2026-06-10 10:30:00".into()),
                Value::Str("2026-06-10 10:00:00".into()),
            ],
        );
        assert!(matches!(r, Ok(Value::Int(1800))), "30 minutes = 1800 sec");
        // The reverse order gives a negative.
        let r = time_module(
            "diff",
            vec![
                Value::Str("2026-06-10 10:00:00".into()),
                Value::Str("2026-06-10 10:30:00".into()),
            ],
        );
        assert!(matches!(r, Ok(Value::Int(-1800))));
    }

    #[test]
    fn time_diff_accepts_iso_with_offset() {
        // Mixed format: one ISO with a zone, one canonical — both come to UTC.
        let r = time_module(
            "diff",
            vec![
                Value::Str("2026-06-10T15:30:00+05:00".into()), // = 10:30 UTC
                Value::Str("2026-06-10 10:00:00".into()),
            ],
        );
        assert!(matches!(r, Ok(Value::Int(1800))));
    }

    #[test]
    fn parse_in_zone_is_dst_aware() {
        // The same wall-clock (12:00 local) gives a different UTC offset under DST:
        // in winter America/New_York = UTC-5 (EST), in summer UTC-4 (EDT). Proof of
        // NOT treating it as a fixed offset — core of issue #80.
        let winter = parse_in_zone("2026-01-15 12:00:00", "America/New_York").unwrap();
        assert_eq!(fmt_unix(winter), "2026-01-15 17:00:00"); // EST: +5 UTC
        let summer = parse_in_zone("2026-07-15 12:00:00", "America/New_York").unwrap();
        assert_eq!(fmt_unix(summer), "2026-07-15 16:00:00"); // EDT: +4 UTC
    }

    #[test]
    fn parse_in_zone_rejects_spring_forward_gap() {
        // 2026-03-08 02:00 -> 03:00 jumps: 02:30 local does not exist -> None.
        assert_eq!(
            parse_in_zone("2026-03-08 02:30:00", "America/New_York"),
            None
        );
    }

    #[test]
    fn parse_in_zone_rejects_unknown_zone() {
        assert_eq!(parse_in_zone("2026-01-15 12:00:00", "Mars/Olympus"), None);
    }

    #[test]
    fn fmt_in_zone_converts_utc_to_local() {
        // UTC instant -> zone wall-clock (DST aware).
        let winter = parse_in_zone("2026-01-15 12:00:00", "America/New_York").unwrap();
        assert_eq!(
            fmt_in_zone(winter, "YYYY-MM-DD HH:mm", "America/New_York").unwrap(),
            "2026-01-15 12:00"
        );
        // Asia/Tashkent is a constant +5 (no DST) — 17:00 UTC -> 22:00 local.
        let utc = parse_ts("2026-06-10 17:00:00").unwrap();
        assert_eq!(fmt_in_zone(utc, "HH:mm", "Asia/Tashkent").unwrap(), "22:00");
    }

    #[test]
    fn time_parse_with_zone_module_level() {
        // time.parse's optional 2nd argument (zone) path gives canonical UTC.
        let Ok(Value::Str(s)) = time_module(
            "parse",
            vec![
                Value::Str("2026-07-15 09:00:00".into()),
                Value::Str("America/New_York".into()),
            ],
        ) else {
            panic!("time.parse with zone must return a string");
        };
        assert_eq!(s, "2026-07-15 13:00:00"); // EDT (+4) -> UTC
    }

    #[test]
    fn time_fmt_with_zone_module_level() {
        // time.fmt's optional 3rd argument (zone) gives the local wall-clock.
        let Ok(Value::Str(s)) = time_module(
            "fmt",
            vec![
                Value::Str("2026-07-15 13:00:00".into()),
                Value::Str("HH:mm".into()),
                Value::Str("America/New_York".into()),
            ],
        ) else {
            panic!("time.fmt with zone must return a string");
        };
        assert_eq!(s, "09:00"); // 13:00 UTC -> EDT 09:00
    }
}
