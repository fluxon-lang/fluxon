// Fluxon core library (the part without batteries).
//
// Three kinds of call:
//   - global functions (log) — installed into Env (`install`)
//   - module functions (str.up, math.floor, rand.int, json) — `call_module`
//   - value methods (l.push, m.set, not s.up...) — `call_method`
//
// list methods act on the value (.push/.filter), str/math/rand go through the
// module — this mirrors the spec distinction exactly: `l.len` (member) vs
// `str.len s` (module).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::interp::{Env, Flow};
use crate::value::{NativeFn, Value};

type R = Result<Value, Flow>;

// The "N assert" count in the `fluxon test` report. Atomic — http.serve handlers
// may call assert from parallel threads. The test runner resets it before each
// file (files run sequentially, in a single process).
static ASSERT_PASSED: AtomicU64 = AtomicU64::new(0);

pub fn assert_passed_reset() {
    ASSERT_PASSED.store(0, Ordering::Relaxed);
}

pub fn assert_passed() -> u64 {
    ASSERT_PASSED.load(Ordering::Relaxed)
}

// --- install global functions ---
pub fn install(env: &Env) {
    let mut s = env.write();
    let mut add = |name: &str, f: Box<dyn Fn(Vec<Value>) -> R + Send + Sync>| {
        s.set_global(
            name,
            Value::Native(Arc::new(NativeFn {
                name: name.into(),
                func: f,
            })),
        );
    };
    // `log` is NOT a global function — it is a leveled dispatch battery (issue #139):
    // `log.debug`/`log.info`/`log.warn`/`log.err`, bare `log` = info. Because it
    // needs to read $LOG_LEVEL and $LOG_FORMAT it is handled in Interp (log_dispatch),
    // not installed as a global here (same pattern as ai/crypto).
    // assert cond ["message"] — test primitive (issue #136). If the condition is
    // truthy it continues silently (counter +1), otherwise a runtime error —
    // execution stops and `fluxon test` marks the file as FAIL. A condition with
    // operators is written in parens (the bare-call rule): `assert (x == 2) "x is not two"`.
    add(
        "assert",
        Box::new(|args: Vec<Value>| {
            let cond = match args.first() {
                Some(v) => v,
                None => return Err(Flow::err("assert: condition argument required")),
            };
            if cond.truthy() {
                ASSERT_PASSED.fetch_add(1, Ordering::Relaxed);
                return Ok(Value::Nil);
            }
            Err(Flow::err(match args.get(1) {
                Some(msg) => format!("assert failed: {}", msg.to_text()),
                None => "assert failed".to_string(),
            }))
        }),
    );
    // rep status body [headers] — an HTTP response. To avoid adding a new Value
    // variant it is represented as a map with special keys:
    // {__resp:true status:N body:V headers:{...}}. http_mod::value_to_response
    // recognizes these keys.
    //
    // Optional 3rd argument — a map of custom headers (issue #16). It is a
    // separate 3rd arg rather than living in the body so it does not collide with
    // the body: in `rep 200 {ok}` the whole map is the body, so a header could not
    // be read out of the body. A header value can be a str (single header) or a
    // list (repeated header, e.g. multiple Set-Cookie).
    add(
        "rep",
        Box::new(|args: Vec<Value>| {
            let status = match args.first() {
                Some(Value::Int(n)) => *n,
                Some(other) => {
                    return Err(Flow::err(format!(
                        "rep: 1st argument must be status (int), got {}",
                        other.type_name()
                    )));
                }
                None => return Err(Flow::err("rep: status argument required")),
            };
            let body = args.get(1).cloned().unwrap_or(Value::Nil);
            let mut m = BTreeMap::new();
            m.insert("__resp".to_string(), Value::Bool(true));
            m.insert("status".to_string(), Value::Int(status));
            m.insert("body".to_string(), body);
            // If a 3rd argument is present — the headers map. If it is not a map
            // we return an explicit error, because silently ignoring it is
            // misleading for the AI.
            if let Some(h) = args.get(2) {
                match h {
                    Value::Map(_) => {
                        m.insert("headers".to_string(), h.clone());
                    }
                    other => {
                        return Err(Flow::err(format!(
                            "rep: 3rd argument must be headers (map), got {}",
                            other.type_name()
                        )));
                    }
                }
            }
            Ok(Value::Map(m))
        }),
    );
}

// ---------------- log levels (issue #139) ----------------
//
// `log` is a leveled logger: `log.debug`/`log.info`/`log.warn`/`log.err`, bare
// `log` = info. Levels are ordered (debug < info < warn < err) — $LOG_LEVEL sets
// the minimum level, messages BELOW it are silently dropped. With $LOG_FORMAT
// =json each line is a JSON object (time/level/msg) — for log aggregators.
//
// Interp reads the envs (OS env + .env, the db/ai convention) and passes them
// here; this function is stateless and pure formatting/filter logic.

// Level order (smaller = lower). An unknown name is treated as info (safe default).
fn log_level_rank(name: &str) -> u8 {
    match name {
        "debug" => 0,
        "info" => 1,
        "warn" => 2,
        // `err` is canonical, `error` is also accepted (a human may write it in LOG_LEVEL).
        "err" | "error" => 3,
        _ => 1,
    }
}

// Formats a log line. If `min_level` (=$LOG_LEVEL) is given and the message level
// is below it, returns None (filtered — nothing is emitted). With `json`
// (=$LOG_FORMAT=json) a structured line, otherwise `[LEVEL] message`.
pub fn format_log(
    level: &str,
    args: &[Value],
    min_level: Option<&str>,
    json: bool,
) -> Option<String> {
    if let Some(min) = min_level
        && log_level_rank(level) < log_level_rank(min)
    {
        return None;
    }
    let msg: String = args
        .iter()
        .map(|v| v.to_text())
        .collect::<Vec<_>>()
        .join(" ");
    if json {
        // Time is UTC, same format as time.now. json_str escapes correctly —
        // quotes/newlines inside the message do not break the JSON.
        Some(format!(
            "{{\"time\":{},\"level\":{},\"msg\":{}}}",
            json_str(&fmt_unix(now_unix())),
            json_str(level),
            json_str(&msg)
        ))
    } else {
        Some(format!("[{}] {}", level.to_uppercase(), msg))
    }
}

// Emits the format_log result to stderr (silent if filtered). Like the old `log`
// it appends `\n` to stderr — so it does not mix with stdout (io.print/prompt).
pub fn emit_log(level: &str, args: &[Value], min_level: Option<&str>, json: bool) {
    if let Some(line) = format_log(level, args, min_level, json) {
        eprintln!("{}", line);
    }
}

// The info-level Native shim returned when `log` is used as a value (callback
// `xs.each log`, storing `f log`) — for compatibility with the old global `log`
// (issue #139). A direct `log "..."` call goes through the env-aware log_dispatch
// in apply_callee; this shim is only for the value position and runs without
// Interp, so it reads $LOG_LEVEL/$LOG_FORMAT from the OS env (.env is not seen on
// this path — using log as a value is a rare case).
pub fn log_value_shim() -> Value {
    Value::Native(Arc::new(NativeFn {
        name: "log".into(),
        func: Box::new(|args: Vec<Value>| {
            let min = std::env::var("LOG_LEVEL").ok();
            let json = std::env::var("LOG_FORMAT")
                .map(|s| s.eq_ignore_ascii_case("json"))
                .unwrap_or(false);
            emit_log("info", &args, min.as_deref(), json);
            Ok(Value::Nil)
        }),
    }))
}

// --- is it a module name? ---
pub fn is_module(name: &str) -> bool {
    matches!(
        name,
        "str" | "math" | "rand" | "json" | "time" | "io" | "fs" | "sh" | "bytes"
    )
}

// --- module function call ---
pub fn call_module(module: &str, func: &str, args: Vec<Value>) -> R {
    match module {
        "str" => str_module(func, args),
        "math" => math_module(func, args),
        "rand" => rand_module(func, args),
        "json" => json_module(func, args),
        "time" => time_module(func, args),
        "io" => io_module(func, args),
        "fs" => fs_module(func, args),
        "sh" => sh_module(func, args),
        "bytes" => bytes_module(func, args),
        _ => Err(Flow::err(format!("unknown module: {}", module))),
    }
}

// ---------------- str ----------------
fn str_module(func: &str, args: Vec<Value>) -> R {
    match func {
        "len" => {
            let s = arg_str(&args, 0, "str.len")?;
            Ok(Value::Int(s.chars().count() as i64))
        }
        "up" => Ok(Value::Str(arg_str(&args, 0, "str.up")?.to_uppercase())),
        "low" => Ok(Value::Str(arg_str(&args, 0, "str.low")?.to_lowercase())),
        "slice" => {
            let s = arg_str(&args, 0, "str.slice")?;
            let a = arg_int(&args, 1, "str.slice")? as usize;
            let b = arg_int(&args, 2, "str.slice")? as usize;
            let chars: Vec<char> = s.chars().collect();
            let a = a.min(chars.len());
            let b = b.min(chars.len());
            if a >= b {
                return Ok(Value::Str(String::new()));
            }
            Ok(Value::Str(chars[a..b].iter().collect()))
        }
        "split" => {
            let s = arg_str(&args, 0, "str.split")?;
            let sep = arg_str(&args, 1, "str.split")?;
            let parts: Vec<Value> = if sep.is_empty() {
                s.chars().map(|c| Value::Str(c.to_string())).collect()
            } else {
                s.split(&sep).map(|p| Value::Str(p.to_string())).collect()
            };
            Ok(Value::List(parts))
        }
        "has" => {
            let s = arg_str(&args, 0, "str.has")?;
            let sub = arg_str(&args, 1, "str.has")?;
            Ok(Value::Bool(s.contains(&sub)))
        }
        "trim" => Ok(Value::Str(
            arg_str(&args, 0, "str.trim")?.trim().to_string(),
        )),
        "replace" => {
            let s = arg_str(&args, 0, "str.replace")?;
            let old = arg_str(&args, 1, "str.replace")?;
            let new = arg_str(&args, 2, "str.replace")?;
            // With an empty pattern Rust's replace inserts between every char —
            // nobody expects that result, so s is left unchanged.
            if old.is_empty() {
                return Ok(Value::Str(s));
            }
            Ok(Value::Str(s.replace(&old, &new)))
        }
        "starts" => {
            let s = arg_str(&args, 0, "str.starts")?;
            let pre = arg_str(&args, 1, "str.starts")?;
            Ok(Value::Bool(s.starts_with(&pre)))
        }
        "ends" => {
            let s = arg_str(&args, 0, "str.ends")?;
            let suf = arg_str(&args, 1, "str.ends")?;
            Ok(Value::Bool(s.ends_with(&suf)))
        }
        // str.pad s n ch — pads on the left with ch up to length n (padStart):
        // number formatting ("7" → "007") is the main need. Length is in the same
        // unit as str.len (chars), not bytes. If n <= len, s is unchanged.
        "pad" => {
            let s = arg_str(&args, 0, "str.pad")?;
            let n = arg_int(&args, 1, "str.pad")?.max(0) as usize;
            let ch = arg_str(&args, 2, "str.pad")?;
            let Some(c) = ch.chars().next() else {
                return Err(Flow::err("str.pad: 3rd argument must not be empty"));
            };
            let len = s.chars().count();
            if n <= len {
                return Ok(Value::Str(s));
            }
            // Compute the result byte count with checked math and reject if it
            // exceeds isize::MAX (Rust's allocation limit) — otherwise
            // with_capacity would panic with "capacity overflow" instead of a Fluxon error.
            let bytes = (n - len)
                .checked_mul(c.len_utf8())
                .and_then(|b| b.checked_add(s.len()))
                .filter(|&b| b <= isize::MAX as usize)
                .ok_or_else(|| Flow::overflow("str.pad"))?;
            let mut out = String::with_capacity(bytes);
            for _ in 0..(n - len) {
                out.push(c);
            }
            out.push_str(&s);
            Ok(Value::Str(out))
        }
        "repeat" => {
            let s = arg_str(&args, 0, "str.repeat")?;
            let n = arg_int(&args, 1, "str.repeat")?;
            if n < 0 {
                return Err(Flow::err(format!(
                    "str.repeat: repeat count must not be negative ({})",
                    n
                )));
            }
            // Keep the result byte count from exceeding isize::MAX (Rust's
            // allocation limit): even if it fits usize, String::repeat panics with
            // "capacity overflow" — give an explicit Fluxon error instead.
            match s.len().checked_mul(n as usize) {
                Some(b) if b <= isize::MAX as usize => Ok(Value::Str(s.repeat(n as usize))),
                _ => Err(Flow::overflow("str.repeat")),
            }
        }
        "int" => {
            let s = arg_str(&args, 0, "str.int")?;
            match s.trim().parse::<i64>() {
                Ok(n) => Ok(Value::Int(n)),
                Err(_) => Ok(Value::Nil),
            }
        }
        "str" => Ok(Value::Str(arg(&args, 0, "str.str")?.to_text())),
        // str.sym "pending" → :pending. For turning query-string statuses into sym
        // filters (db.eq {status:(str.split q "," |> ...).map str.sym}).
        // Previously a json.dec("\":"+s+"\"") hack was used for this (issue #78).
        // Sym/str are also accepted (idempotent); surrounding whitespace is trimmed.
        "sym" => match arg(&args, 0, "str.sym")? {
            Value::Str(s) => Ok(Value::Sym(s.trim().to_string())),
            Value::Sym(s) => Ok(Value::Sym(s.clone())),
            other => Err(Flow::err(format!(
                "str.sym: str expected, got {}",
                other.type_name()
            ))),
        },
        _ => Err(Flow::err(format!("str module has no function '{}'", func))),
    }
}

// ---------------- bytes (binary data, issue #132) ----------------
//
// bytes values have no literal syntax — they are created via functions
// (fs.readb, crypto.b64db, bytes.of). str.len counts CHARS, bytes.len counts
// BYTES — deliberately two separate units.
fn bytes_module(func: &str, args: Vec<Value>) -> R {
    match func {
        // bytes.of s -> the UTF-8 bytes of the text. If bytes are given they are
        // returned as-is (idempotent — handy in conversion chains).
        "of" => match arg(&args, 0, "bytes.of")? {
            Value::Bytes(b) => Ok(Value::Bytes(b.clone())),
            Value::Str(s) | Value::Sym(s) => Ok(Value::Bytes(Arc::new(s.clone().into_bytes()))),
            other => Err(Flow::err(format!(
                "bytes.of: argument must be str or bytes, got {}",
                other.type_name()
            ))),
        },
        // bytes.str b -> UTF-8 text; an explicit error on invalid bytes (not a
        // silent corruption — same principle as crypto.b64d).
        "str" => {
            let b = arg_bytes(&args, 0, "bytes.str")?;
            String::from_utf8(b.as_ref().clone())
                .map(Value::Str)
                .map_err(|_| Flow::err("bytes.str: bytes are not valid UTF-8 text".to_string()))
        }
        "len" => Ok(Value::Int(arg_bytes(&args, 0, "bytes.len")?.len() as i64)),
        // bytes.slice b a c — str.slice semantics (bounds clamp, a >= b ->
        // empty), but on byte indices.
        "slice" => {
            let b = arg_bytes(&args, 0, "bytes.slice")?;
            let a = arg_int(&args, 1, "bytes.slice")? as usize;
            let c = arg_int(&args, 2, "bytes.slice")? as usize;
            let a = a.min(b.len());
            let c = c.min(b.len());
            if a >= c {
                return Ok(Value::Bytes(Arc::new(Vec::new())));
            }
            Ok(Value::Bytes(Arc::new(b[a..c].to_vec())))
        }
        _ => Err(Flow::err(format!(
            "bytes module has no function '{}' (of/str/len/slice)",
            func
        ))),
    }
}

// ---------------- math ----------------
fn math_module(func: &str, args: Vec<Value>) -> R {
    let x = arg_num(&args, 0, &format!("math.{}", func))?;
    match func {
        "floor" => Ok(Value::Int(x.floor() as i64)),
        "ceil" => Ok(Value::Int(x.ceil() as i64)),
        "abs" => {
            // int in -> int out, flt in -> flt out.
            // i64::MIN.abs() panics (its positive counterpart does not fit) — checked.
            match &args[0] {
                Value::Int(n) => Ok(Value::Int(
                    n.checked_abs().ok_or_else(|| Flow::overflow("math.abs"))?,
                )),
                _ => Ok(Value::Flt(x.abs())),
            }
        }
        "round" => Ok(Value::Int(x.round() as i64)),
        // min/max return the argument itself — int in stays int (same style as
        // abs), and the type is not lost for mixed int/flt either.
        "min" | "max" => {
            // arg_num checks the second argument is a number (x checked the first).
            let y = arg_num(&args, 1, &format!("math.{}", func))?;
            use std::cmp::Ordering;
            // Lossless comparison: casting an int to f64 rounds neighboring values
            // above 2^53 so they compare equal, and the tie rule then returns the
            // wrong side. int/int — in i64, mixed — via cmp_int_flt; only flt/flt
            // stays in f64 (which is exact there).
            let ord = match (&args[0], &args[1]) {
                (Value::Int(a), Value::Int(b)) => a.cmp(b),
                (Value::Int(a), Value::Flt(b)) => cmp_int_flt(*a, *b),
                (Value::Flt(a), Value::Int(b)) => cmp_int_flt(*b, *a).reverse(),
                // NaN is unordered — treat as Equal (same as sorting).
                _ => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            };
            let pick_first = if func == "min" {
                ord != Ordering::Greater
            } else {
                ord != Ordering::Less
            };
            Ok(if pick_first {
                args[0].clone()
            } else {
                args[1].clone()
            })
        }
        "pow" => {
            let y = arg_num(&args, 1, "math.pow")?;
            match (&args[0], &args[1]) {
                // int ^ non-negative int → int (checked: on overflow a Fluxon
                // error, not a panic; an exponent that does not fit i64 is also overflow).
                (Value::Int(a), Value::Int(b)) if *b >= 0 => {
                    let e = u32::try_from(*b).map_err(|_| Flow::overflow("math.pow"))?;
                    Ok(Value::Int(
                        a.checked_pow(e).ok_or_else(|| Flow::overflow("math.pow"))?,
                    ))
                }
                // negative exponent or a flt involved — the result is flt.
                _ => Ok(Value::Flt(x.powf(y))),
            }
        }
        "sqrt" => {
            // The root of a negative number would yield NaN — Fluxon does not
            // expect a NaN value, so an explicit error instead.
            if x < 0.0 {
                return Err(Flow::err(
                    "math.sqrt: cannot take square root of a negative number",
                ));
            }
            Ok(Value::Flt(x.sqrt()))
        }
        _ => Err(Flow::err(format!("math module has no function '{}'", func))),
    }
}

// Compares an i64 with an f64 losslessly: the i64->f64 cast rounds beyond 2^53,
// so the f64 is first compared against the i64 bounds, then the integer and
// fractional parts are compared separately. NaN — Equal (same convention as
// sorting).
fn cmp_int_flt(a: i64, b: f64) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    if b.is_nan() {
        return Equal;
    }
    // i64::MAX as f64 = 2^63 (rounded up) — from that value on, b is greater than
    // any i64. i64::MIN as f64 = -2^63 is represented exactly.
    if b >= i64::MAX as f64 {
        return Less;
    }
    if b < i64::MIN as f64 {
        return Greater;
    }
    // Now b.trunc() fits in i64 and the cast is lossless.
    match a.cmp(&(b.trunc() as i64)) {
        Equal if b.fract() > 0.0 => Less,
        Equal if b.fract() < 0.0 => Greater,
        ord => ord,
    }
}

// ---------------- rand (dependency-free LCG) ----------------
fn rand_module(func: &str, args: Vec<Value>) -> R {
    match func {
        "int" => {
            let a = arg_int(&args, 0, "rand.int")?;
            let b = arg_int(&args, 1, "rand.int")?;
            if b < a {
                return Err(Flow::err("rand.int: upper bound is less than lower bound"));
            }
            // span in i128: at extreme a/b values (e.g. a very negative,
            // b very positive) b - a + 1 does not fit i64 and would overflow.
            let span = (b as i128) - (a as i128) + 1; // [1, 2^64]
            let r = if span > u64::MAX as i128 {
                next_rand() // full i64 range — any u64 is a valid value
            } else {
                next_rand() % (span as u64)
            };
            // The actual result a + r is always within [a, b] — in two's-complement
            // modular arithmetic wrapping_add gives exactly that value (even if the
            // intermediate sum overflows i64).
            Ok(Value::Int(a.wrapping_add(r as i64)))
        }
        "str" => {
            let n = arg_int(&args, 0, "rand.str")? as usize;
            const ALPHA: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
            let mut out = String::with_capacity(n);
            for _ in 0..n {
                let idx = (next_rand() % ALPHA.len() as u64) as usize;
                out.push(ALPHA[idx] as char);
            }
            Ok(Value::Str(out))
        }
        _ => Err(Flow::err(format!("rand module has no function '{}'", func))),
    }
}

// ---------------- time ----------------
// All times are UTC text in "YYYY-MM-DD HH:MM:SS" format — EXACTLY the same as
// SQLite CURRENT_TIMESTAMP (the tbl `now` column), so DB filters like
// `created > (time.ago 24 :hr)` work directly.
fn time_module(func: &str, args: Vec<Value>) -> R {
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

// ---------------- io ----------------
// Terminal input/output. `log` always appends `\n` to stderr; an interactive CLI
// (REPL, agent, wizard) needs to read from stdin and a prompt without `\n`. The
// prompt and input go through stdout/stdin (log is stderr — they must not mix).
fn io_module(func: &str, args: Vec<Value>) -> R {
    use std::io::Write;
    match func {
        // io.read_line -> a single line from stdin (the trailing \n is removed).
        // EOF (Ctrl-D, end of a pipe) -> nil, so the caller stops the loop.
        "read_line" => {
            let mut line = String::new();
            match std::io::stdin().read_line(&mut line) {
                Ok(0) => Ok(Value::Nil),
                Ok(_) => {
                    // strip the trailing \n (and Windows \r)
                    let trimmed = line.trim_end_matches(['\n', '\r']);
                    Ok(Value::Str(trimmed.to_string()))
                }
                Err(e) => Err(Flow::err(format!("io.read_line: {}", e))),
            }
        }
        // io.print s -> write to stdout WITHOUT a \n (to show a prompt).
        // Flush immediately — otherwise the prompt stays in the buffer and the user
        // does not see it before typing input.
        "print" => {
            let s = arg_str(&args, 0, "io.print")?;
            let mut out = std::io::stdout();
            out.write_all(s.as_bytes())
                .and_then(|_| out.flush())
                .map_err(|e| Flow::err(format!("io.print: {}", e)))?;
            Ok(Value::Nil)
        }
        // io.prompt msg -> prints msg without a \n, then reads a single line.
        // A convenient shorthand for io.print + io.read_line.
        "prompt" => {
            let s = arg_str(&args, 0, "io.prompt")?;
            let mut out = std::io::stdout();
            out.write_all(s.as_bytes())
                .and_then(|_| out.flush())
                .map_err(|e| Flow::err(format!("io.prompt: {}", e)))?;
            io_module("read_line", vec![])
        }
        _ => Err(Flow::err(format!("io module has no function '{}'", func))),
    }
}

// ---------------- fs (local file system) ----------------
//
// Convention: on success a useful value (text/bool/list) or the :ok sym; on a
// real IO error a Flow::err — so the cause is not lost (the io battery is like this).
// The only exception: fs.read returns nil when the file is missing (an expected
// case, not an error — handy for folding the "does the file exist?" check into read).
fn fs_module(func: &str, args: Vec<Value>) -> R {
    match func {
        // fs.read path -> the file text (str), or nil if the file is missing.
        // Flow::err on a non-UTF-8 file or a permission error.
        "read" => {
            let path = arg_str(&args, 0, "fs.read")?;
            match std::fs::read_to_string(&path) {
                Ok(s) => Ok(Value::Str(s)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Nil),
                Err(e) => Err(Flow::err(format!("fs.read {}: {}", path, e))),
            }
        }
        // fs.readb path -> the file bytes (bytes), or nil if missing. The binary
        // counterpart of fs.read (issue #132) — non-UTF-8 files like images/PDFs
        // error in fs.read, and are read through this instead.
        "readb" => {
            let path = arg_str(&args, 0, "fs.readb")?;
            match std::fs::read(&path) {
                Ok(b) => Ok(Value::Bytes(Arc::new(b))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Nil),
                Err(e) => Err(Flow::err(format!("fs.readb {}: {}", path, e))),
            }
        }
        // fs.write path content -> overwrites the file (previous content is lost).
        // Intermediate directories must exist (use fs.mkdirp if needed).
        // content is str OR bytes — no separate "writeb" is needed for writing,
        // because the source type does not change the path (unlike reading).
        "write" => {
            let path = arg_str(&args, 0, "fs.write")?;
            let content = arg_bytes(&args, 1, "fs.write")?;
            std::fs::write(&path, content.as_slice())
                .map_err(|e| Flow::err(format!("fs.write {}: {}", path, e)))?;
            Ok(Value::Sym("ok".into()))
        }
        // fs.append path content -> appends to the end of an existing file (creates
        // it if missing). content is str or bytes (same as fs.write).
        "append" => {
            use std::io::Write;
            let path = arg_str(&args, 0, "fs.append")?;
            let content = arg_bytes(&args, 1, "fs.append")?;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| Flow::err(format!("fs.append {}: {}", path, e)))?;
            f.write_all(content.as_slice())
                .map_err(|e| Flow::err(format!("fs.append {}: {}", path, e)))?;
            Ok(Value::Sym("ok".into()))
        }
        // fs.exists path -> bool (whether a file OR directory exists).
        "exists" => {
            let path = arg_str(&args, 0, "fs.exists")?;
            Ok(Value::Bool(std::path::Path::new(&path).exists()))
        }
        // fs.ls path -> a list of names inside the directory [str] (just the name,
        // not the full path). Sorted so the order is deterministic.
        "ls" => {
            let path = arg_str(&args, 0, "fs.ls")?;
            let entries = std::fs::read_dir(&path)
                .map_err(|e| Flow::err(format!("fs.ls {}: {}", path, e)))?;
            let mut names = Vec::new();
            for entry in entries {
                let entry = entry.map_err(|e| Flow::err(format!("fs.ls {}: {}", path, e)))?;
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
            names.sort();
            Ok(Value::List(names.into_iter().map(Value::Str).collect()))
        }
        // fs.del path -> deletes a file or an empty directory -> :ok.
        // If the directory is not empty, Flow::err (recursive delete is deliberately
        // absent — safer, so a whole tree is not accidentally removed).
        "del" => {
            let path = arg_str(&args, 0, "fs.del")?;
            let p = std::path::Path::new(&path);
            let res = if p.is_dir() {
                std::fs::remove_dir(p)
            } else {
                std::fs::remove_file(p)
            };
            res.map_err(|e| Flow::err(format!("fs.del {}: {}", path, e)))?;
            Ok(Value::Sym("ok".into()))
        }
        // fs.mkdirp path -> creates the directory (with the needed intermediate dirs) -> :ok.
        // Not an error if the directory already exists (idempotent).
        "mkdirp" => {
            let path = arg_str(&args, 0, "fs.mkdirp")?;
            std::fs::create_dir_all(&path)
                .map_err(|e| Flow::err(format!("fs.mkdirp {}: {}", path, e)))?;
            Ok(Value::Sym("ok".into()))
        }
        _ => Err(Flow::err(format!("fs module has no function '{}'", func))),
    }
}

// ---------------- sh (external shell commands) ----------------
//
// sh.run cmd -> {stdout: str  stderr: str  code: int}.
// The command is run through the SHELL (Unix: `sh -c`, Windows: `cmd /C`) — so
// shell features like `cd x && cargo build`, pipes (`|`), `&&`, glob work (in
// issue #26 Sonnet guessed exactly this pattern). Needed for a coding agent,
// CI scripts, build automation.
//
// `code == 0` is success (the Unix convention). If the process dies from a signal
// (no exit code on Unix) code = -1. The command ITSELF failing (a non-zero code)
// is NOT a Flow::err — that is an expected result, the caller checks via `code`.
// Only when the process cannot be started at all (e.g. the shell is not found) Flow::err.
//
// Blocking dangerous commands is deliberately ABSENT — that is the user's responsibility (issue #26).
fn sh_module(func: &str, args: Vec<Value>) -> R {
    match func {
        "run" => {
            let cmd = arg_str(&args, 0, "sh.run")?;
            let mut command;
            #[cfg(windows)]
            {
                command = std::process::Command::new("cmd");
                command.arg("/C").arg(&cmd);
            }
            #[cfg(not(windows))]
            {
                command = std::process::Command::new("sh");
                command.arg("-c").arg(&cmd);
            }
            let output = command
                .output()
                .map_err(|e| Flow::err(format!("sh.run: could not start command: {}", e)))?;
            // Read stdout/stderr as lossy UTF-8 — no panic even on binary output
            // (unlike the json decoder, there is no text guarantee here).
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            // a process ended by a signal has code None -> -1.
            let code = output.status.code().unwrap_or(-1) as i64;
            let mut m = BTreeMap::new();
            m.insert("stdout".to_string(), Value::Str(stdout));
            m.insert("stderr".to_string(), Value::Str(stderr));
            m.insert("code".to_string(), Value::Int(code));
            Ok(Value::Map(m))
        }
        _ => Err(Flow::err(format!("sh module has no function '{}'", func))),
    }
}

fn now_unix() -> i64 {
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

fn fmt_unix(unix: i64) -> String {
    let (y, mo, d, h, mi, s) = civil(unix);
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, mo, d, h, mi, s)
}

// "YYYY-MM-DD HH:MM:SS" (or "YYYY-MM-DDTHH:MM:SS") -> unix seconds (UTC).
fn parse_ts(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> { s.get(a..z)?.parse::<i64>().ok() };
    let y = num(0, 4)?;
    let mo = num(5, 7)?;
    let d = num(8, 10)?;
    let h = num(11, 13)?;
    let mi = num(14, 16)?;
    let se = num(17, 19)?;
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
fn parse_iso(s: &str) -> Option<i64> {
    let s = s.trim();
    let base = parse_ts(s)?; // first 19 chars (date + time); len >= 19 guaranteed
    let mut rest = &s[19..];
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

// Wall-clock matnni IANA zonada (DST hisobga olinib) talqin qilib UTC sekundga
// o'giradi. parse_ts asosini (sana+vaqt, mintaqasiz) o'qiydi, so'ng o'sha maydonlarni
// zonaning local vaqti deb hisoblaydi — fiksrlangan offset emas, shuning uchun
// yoz/qish (DST) o'tishi to'g'ri ishlaydi.
//
// DST chetlari: bahorgi sakrashda mavjud bo'lmagan local vaqt (masalan 02:30) -> None
// (chaqiruvchi xato beradi). Kuzgi takror (vaqt ikki marta) holatda ertaroq (DST-li)
// instant tanlanadi — booking uchun deterministik va xavfsiz default.
fn parse_in_zone(s: &str, zone: &str) -> Option<i64> {
    use chrono::offset::LocalResult;
    use chrono::{NaiveDate, TimeZone};
    let tz: chrono_tz::Tz = zone.parse().ok()?;
    // parse_ts wall-clock'ni "soxta UTC" sekund sifatida beradi; civil bilan
    // maydonlarga qaytarib, zonada qayta talqin qilamiz.
    let (y, mo, d, h, mi, se) = civil(parse_ts(s)?);
    let naive = NaiveDate::from_ymd_opt(y as i32, mo, d)?.and_hms_opt(h, mi, se)?;
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Some(dt.timestamp()),
        LocalResult::Ambiguous(earlier, _later) => Some(earlier.timestamp()),
        LocalResult::None => None,
    }
}

// UTC instant'ni IANA zonaning local wall-clock'iga (DST hisobga olinib) o'tkazib
// formatlaydi. Noma'lum zona nomida None.
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

// OS kriptografik CSPRNG (getrandom orqali, `auth` battery'dagi OsRng bilan
// bir xil manba). Avval thread-local xorshift edi, seed = system time (nanos) —
// seed bashorat qilinardi va bir nanosekundda ochilgan ikki thread bir xil
// ketma-ketlik olardi. `rand.str` token/session-ID generatsiyaga tabiiy
// ishlatilgani uchun (#97) rand butunlay kriptografik xavfsiz manbaga o'tdi:
// bir ish = bir yo'l — alohida "xavfsiz rand" o'rganishga hojat yo'q.
fn next_rand() -> u64 {
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    OsRng.next_u64()
}

// ---------------- json ----------------
fn json_module(func: &str, args: Vec<Value>) -> R {
    match func {
        "enc" => Ok(Value::Str(json_encode(arg(&args, 0, "json.enc")?))),
        "dec" => {
            let s = arg_str(&args, 0, "json.dec")?;
            json_decode(&s)
        }
        _ => Err(Flow::err(format!("json module has no function '{}'", func))),
    }
}

// Map'ni JSON obyektga kodlaydi (Map va Ctx uchun umumiy).
fn json_encode_map(m: &std::collections::BTreeMap<String, Value>) -> String {
    let parts: Vec<String> = m
        .iter()
        .map(|(k, val)| format!("{}:{}", json_str(k), json_encode(val)))
        .collect();
    format!("{{{}}}", parts.join(","))
}

pub fn json_encode(v: &Value) -> String {
    match v {
        Value::Int(n) => n.to_string(),
        // JSON'da Infinity/NaN yo'q — JSON.stringify kabi `null` chiqaramiz
        // (aks holda "inf"/"NaN" qat'iy parserlarda rad etiladi).
        Value::Flt(x) => {
            if x.is_finite() {
                x.to_string()
            } else {
                "null".into()
            }
        }
        Value::Bool(b) => b.to_string(),
        Value::Nil => "null".into(),
        Value::Str(s) | Value::Sym(s) => json_str(s),
        Value::List(items) => {
            let parts: Vec<String> = items.iter().map(json_encode).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Map(m) => json_encode_map(m),
        // JSON'da ikkilik tur yo'q — baytlar base64 matn bo'lib tushadi
        // (yo'qotishsiz; null/buzilgan matndan foydaliroq).
        Value::Bytes(b) => {
            use base64::Engine;
            json_str(&base64::engine::general_purpose::STANDARD.encode(b.as_slice()))
        }
        // ctx oddiy map kabi kodlanadi (snapshot) — javob body'siga tushsa.
        Value::Ctx(c) => json_encode_map(&c.lock().unwrap()),
        Value::Fn(_) | Value::Native(_) => "null".into(),
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            // Qolgan control belgilar (0x00–0x1F) JSON spec'da xom kelolmaydi —
            // \u00XX shaklida escape qilamiz (aks holda chiqish invalid JSON).
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

// Minimal JSON dekoder (yadro versiyasi uchun yetarli).
pub fn json_decode(s: &str) -> R {
    let mut p = JsonParser {
        b: s.as_bytes(),
        i: 0,
    };
    p.skip_ws();
    let v = p.value()?;
    p.skip_ws();
    // Qiymatdan keyin chiqindi qolmasligi kerak — `"1 garbage"` endi xato beradi
    // (avval jim `1` qaytarardi).
    if p.i < p.b.len() {
        return Err(Flow::err("json: extra text after value"));
    }
    Ok(v)
}

struct JsonParser<'a> {
    b: &'a [u8],
    i: usize,
}
impl<'a> JsonParser<'a> {
    fn skip_ws(&mut self) {
        while self.i < self.b.len() && (self.b[self.i] as char).is_whitespace() {
            self.i += 1;
        }
    }
    fn value(&mut self) -> R {
        self.skip_ws();
        if self.i >= self.b.len() {
            return Err(Flow::err("json: unexpected end"));
        }
        match self.b[self.i] {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => Ok(Value::Str(self.string()?)),
            b't' | b'f' => self.boolean(),
            // `null`ni harf-baharf tekshiramiz — avval `nqqq` ham jim nil berardi.
            b'n' => {
                if self.b[self.i..].starts_with(b"null") {
                    self.i += 4;
                    Ok(Value::Nil)
                } else {
                    Err(Flow::err("json: invalid value (null expected)"))
                }
            }
            _ => self.number(),
        }
    }
    fn object(&mut self) -> R {
        self.i += 1; // {
        let mut m = BTreeMap::new();
        self.skip_ws();
        if self.i < self.b.len() && self.b[self.i] == b'}' {
            self.i += 1;
            return Ok(Value::Map(m));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            if self.i >= self.b.len() || self.b[self.i] != b':' {
                return Err(Flow::err("json: ':' expected"));
            }
            self.i += 1;
            let val = self.value()?;
            m.insert(key, val);
            self.skip_ws();
            match self.b.get(self.i) {
                Some(b',') => {
                    self.i += 1;
                }
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                _ => return Err(Flow::err("json: ',' or '}' expected")),
            }
        }
        Ok(Value::Map(m))
    }
    fn array(&mut self) -> R {
        self.i += 1; // [
        let mut out = Vec::new();
        self.skip_ws();
        if self.i < self.b.len() && self.b[self.i] == b']' {
            self.i += 1;
            return Ok(Value::List(out));
        }
        loop {
            let v = self.value()?;
            out.push(v);
            self.skip_ws();
            match self.b.get(self.i) {
                Some(b',') => {
                    self.i += 1;
                }
                Some(b']') => {
                    self.i += 1;
                    break;
                }
                _ => return Err(Flow::err("json: ',' or ']' expected")),
            }
        }
        Ok(Value::List(out))
    }
    fn string(&mut self) -> Result<String, Flow> {
        // Kesilgan input (masalan `{`) bilan chegaradan o'tib panic bo'lmasin —
        // ishonchsiz tashqi ma'lumot (HTTP body) crash qildirmasligi shart.
        if self.i >= self.b.len() {
            return Err(Flow::err("json: unexpected end"));
        }
        if self.b[self.i] != b'"' {
            return Err(Flow::err("json: string expected"));
        }
        self.i += 1;
        // Natijani BAYTLAR sifatida yig'amiz, oxirida UTF-8 str'ga aylantiramiz —
        // ko'p baytli belgilar (emoji, o'zbekcha o'/g') bayt-bayt `as char` bilan
        // BUZILADI (mojibake). \u escape'lari esa char'ga dekodlanib UTF-8 yoziladi.
        let mut out: Vec<u8> = Vec::new();
        while self.i < self.b.len() {
            let c = self.b[self.i];
            self.i += 1;
            match c {
                b'"' => {
                    return String::from_utf8(out)
                        .map_err(|_| Flow::err("json: string is invalid UTF-8"));
                }
                b'\\' => {
                    // Satr `\` bilan tugab qolsa (masalan `"ab\`) escape baytini
                    // o'qishda chegaradan o'tmaylik — aks holda panic.
                    if self.i >= self.b.len() {
                        return Err(Flow::err("json: unexpected end"));
                    }
                    let e = self.b[self.i];
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0C),
                        b'u' => {
                            // \uXXXX — 16-bitli kod birligi. Surrogate juftligi
                            // (\uD800..DBFF + \uDC00..DFFF) bitta belgini beradi
                            // (emoji va BMP'dan tashqari hamma narsa shunday keladi).
                            let hi = self.hex4()?;
                            let ch = if (0xD800..=0xDBFF).contains(&hi) {
                                // yuqori surrogate -> past surrogatni kutamiz.
                                if self.b.get(self.i) == Some(&b'\\')
                                    && self.b.get(self.i + 1) == Some(&b'u')
                                {
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    let cp = 0x10000
                                        + (((hi as u32 - 0xD800) << 10) | (lo as u32 - 0xDC00));
                                    char::from_u32(cp).unwrap_or('\u{FFFD}')
                                } else {
                                    '\u{FFFD}' // juftsiz surrogate
                                }
                            } else {
                                char::from_u32(hi as u32).unwrap_or('\u{FFFD}')
                            };
                            // char'ni UTF-8 baytlar sifatida qo'shamiz.
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        other => out.push(other), // noma'lum escape — baytni o'zini
                    }
                }
                // Oddiy bayt (ASCII yoki ko'p baytli UTF-8 ketma-ketligining bir
                // qismi) — o'z holicha qo'shiladi, str konversiyasi oxirida.
                _ => out.push(c),
            }
        }
        Err(Flow::err("json: unterminated string"))
    }

    // Joriy pozitsiyadan 4 hex raqamni o'qib u16 qaytaradi (\uXXXX uchun).
    fn hex4(&mut self) -> Result<u16, Flow> {
        if self.i + 4 > self.b.len() {
            return Err(Flow::err("json: \\u requires 4 hex digits"));
        }
        let slice = &self.b[self.i..self.i + 4];
        let s = std::str::from_utf8(slice).map_err(|_| Flow::err("json: \\u invalid"))?;
        let v = u16::from_str_radix(s, 16).map_err(|_| Flow::err("json: \\u invalid hex"))?;
        self.i += 4;
        Ok(v)
    }
    fn boolean(&mut self) -> R {
        if self.b[self.i..].starts_with(b"true") {
            self.i += 4;
            Ok(Value::Bool(true))
        } else if self.b[self.i..].starts_with(b"false") {
            self.i += 5;
            Ok(Value::Bool(false))
        } else {
            Err(Flow::err("json: invalid bool"))
        }
    }
    // JSON son grammatikasini qat'iy tutamiz: [-] int [frac] [exp].
    // Avvalgi versiya `+5`, `1.2.3` kabi yaroqsiz sonlarni yutardi.
    fn number(&mut self) -> R {
        let start = self.i;
        let mut is_float = false;
        // ixtiyoriy manfiy belgi — JSON faqat '-' ruxsat beradi ('+' emas)
        if self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        // butun qism: '0' yoki 1-9 dan boshlanuvchi raqamlar
        match self.b.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(c) if c.is_ascii_digit() => {
                while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                    self.i += 1;
                }
            }
            _ => return Err(Flow::err("json: invalid number")),
        }
        // kasr qismi: '.' dan keyin kamida bitta raqam
        if self.b.get(self.i) == Some(&b'.') {
            is_float = true;
            self.i += 1;
            if !self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                return Err(Flow::err("json: invalid number"));
            }
            while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                self.i += 1;
            }
        }
        // eksponent: e/E [+/-] kamida bitta raqam
        if matches!(self.b.get(self.i), Some(b'e') | Some(b'E')) {
            is_float = true;
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            if !self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                return Err(Flow::err("json: invalid number"));
            }
            while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                self.i += 1;
            }
        }
        let text = std::str::from_utf8(&self.b[start..self.i]).unwrap_or("");
        if is_float {
            text.parse::<f64>()
                .map(Value::Flt)
                .map_err(|_| Flow::err("json: invalid number"))
        } else {
            text.parse::<i64>()
                .map(Value::Int)
                .map_err(|_| Flow::err("json: invalid number"))
        }
    }
}

// ---------------- qiymat metodlari (list/map) ----------------
pub fn call_method(recv: &Value, method: &str, args: Vec<Value>) -> R {
    match recv {
        Value::List(xs) => list_method(xs, method, args),
        Value::Map(m) => map_method(m, method, args),
        Value::Str(_) => Err(Flow::err(format!(
            "str methods are called via the module: str.{} s (not a value method)",
            method
        ))),
        other => Err(Flow::err(format!(
            "{} type has no '.{}' method",
            other.type_name(),
            method
        ))),
    }
}

fn list_method(xs: &[Value], method: &str, args: Vec<Value>) -> R {
    match method {
        "len" => Ok(Value::Int(xs.len() as i64)),
        "push" => {
            let mut new = xs.to_vec();
            new.push(arg(&args, 0, "list.push")?.clone());
            Ok(Value::List(new))
        }
        "has" => {
            let needle = arg(&args, 0, "list.has")?;
            Ok(Value::Bool(xs.iter().any(|v| v.equals(needle))))
        }
        "index" => {
            // Birinchi mos elementning indeksi; topilmasa -1 (bool'dan farqli,
            // index pozitsiya beradi — list.has bilan juftlik).
            let needle = arg(&args, 0, "list.index")?;
            let i = xs
                .iter()
                .position(|v| v.equals(needle))
                .map(|i| i as i64)
                .unwrap_or(-1);
            Ok(Value::Int(i))
        }
        "join" => {
            let sep = arg_str(&args, 0, "list.join")?;
            let parts: Vec<String> = xs.iter().map(|v| format!("{}", v)).collect();
            Ok(Value::Str(parts.join(&sep)))
        }
        "slice" => {
            let a = arg_int(&args, 0, "list.slice")? as usize;
            let b = arg_int(&args, 1, "list.slice")? as usize;
            let a = a.min(xs.len());
            let b = b.min(xs.len());
            if a >= b {
                return Ok(Value::List(vec![]));
            }
            Ok(Value::List(xs[a..b].to_vec()))
        }
        // Argumentsiz sort — tabiiy tartib (son/matn). Komparatorli varianti
        // funksiya argument olgani uchun interp'dagi list_hof orqali keladi.
        "sort" => sort_default(xs),
        "reverse" => {
            let mut new = xs.to_vec();
            new.reverse();
            Ok(Value::List(new))
        }
        "uniq" => {
            // Birinchi uchragan nusxa qoladi (tartib saqlanadi). Value hash'siz,
            // shuning uchun equals bilan chiziqli qidiruv — list'lar kichik.
            let mut out: Vec<Value> = Vec::new();
            for x in xs {
                if !out.iter().any(|v| v.equals(x)) {
                    out.push(x.clone());
                }
            }
            Ok(Value::List(out))
        }
        "flat" => {
            // Bir daraja tekislaydi: ichki list elementlari ochiladi, qolganlar
            // o'z holicha — chuqur rekursiya kerak bo'lsa flat'ni zanjirlash mumkin.
            let mut out = Vec::new();
            for x in xs {
                match x {
                    Value::List(inner) => out.extend(inner.iter().cloned()),
                    other => out.push(other.clone()),
                }
            }
            Ok(Value::List(out))
        }
        "zip" => {
            let other = arg(&args, 0, "list.zip")?;
            let Value::List(ys) = other else {
                return Err(Flow::err(format!(
                    "list.zip: argument must be a list, got {}",
                    other.type_name()
                )));
            };
            // Qisqasi tugaganda to'xtaydi — ortiqcha elementlar tashlanadi.
            Ok(Value::List(
                xs.iter()
                    .zip(ys)
                    .map(|(a, b)| Value::List(vec![a.clone(), b.clone()]))
                    .collect(),
            ))
        }
        // filter/map/reduce/find/any/all — funksiya argument oladi; interp uni
        // shu yerda chaqira olmaydi (apply Interp'da). Shuning uchun bu metodlar
        // maxsus ishlov talab qiladi — pastdagi izohga qarang.
        "filter" | "map" | "reduce" | "find" | "any" | "all" => Err(Flow::err(format!(
            "internal: list.{} is handled via a separate path",
            method
        ))),
        _ => Err(Flow::err(format!(
            "list method '{}' does not exist",
            method
        ))),
    }
}

// Tabiiy tartibda saralash: son (int/flt aralash) va matn/sym bir jinsli
// bo'lsa ishlaydi; aralash tiplar uchun komparator berish talab qilinadi.
pub fn sort_default(xs: &[Value]) -> R {
    let sorted = sort_values(xs.to_vec(), &mut |a, b| {
        use std::cmp::Ordering;
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => Ok(x.cmp(y)),
            // NaN tartibsiz — Equal deb olamiz (saralash yiqilmasin).
            (Value::Flt(x), Value::Flt(y)) => Ok(x.partial_cmp(y).unwrap_or(Ordering::Equal)),
            (Value::Int(x), Value::Flt(y)) => {
                Ok((*x as f64).partial_cmp(y).unwrap_or(Ordering::Equal))
            }
            (Value::Flt(x), Value::Int(y)) => {
                Ok(x.partial_cmp(&(*y as f64)).unwrap_or(Ordering::Equal))
            }
            (Value::Str(x), Value::Str(y)) => Ok(x.cmp(y)),
            (Value::Sym(x), Value::Sym(y)) => Ok(x.cmp(y)),
            (a, b) => Err(Flow::err(format!(
                "list.sort: cannot compare {} and {} — provide a comparator: l.sort \\a b -> ...",
                a.type_name(),
                b.type_name()
            ))),
        }
    })?;
    Ok(Value::List(sorted))
}

// Stable merge sort — std sort_by o'rniga, chunki komparator Fluxon funksiyasi
// bo'lganda xato (Flow) qaytishi mumkin: std sort xato yo'lida Equal qaytarsak
// "total order buzildi" deb panic qilishi mumkin. Bu yo'l xatoni toza ko'taradi.
pub fn sort_values<F>(mut xs: Vec<Value>, cmp: &mut F) -> Result<Vec<Value>, Flow>
where
    F: FnMut(&Value, &Value) -> Result<std::cmp::Ordering, Flow>,
{
    let len = xs.len();
    if len <= 1 {
        return Ok(xs);
    }
    let right = xs.split_off(len / 2);
    let left = sort_values(xs, cmp)?;
    let right = sort_values(right, cmp)?;
    let mut out = Vec::with_capacity(len);
    let mut li = left.into_iter().peekable();
    let mut ri = right.into_iter().peekable();
    loop {
        match (li.peek(), ri.peek()) {
            // Teng bo'lsa chap (asl tartibda oldingi) birinchi — stable.
            (Some(a), Some(b)) => {
                if cmp(a, b)? == std::cmp::Ordering::Greater {
                    out.push(ri.next().unwrap());
                } else {
                    out.push(li.next().unwrap());
                }
            }
            (Some(_), None) => out.push(li.next().unwrap()),
            (None, Some(_)) => out.push(ri.next().unwrap()),
            (None, None) => break,
        }
    }
    Ok(out)
}

fn map_method(m: &BTreeMap<String, Value>, method: &str, args: Vec<Value>) -> R {
    match method {
        "len" => Ok(Value::Int(m.len() as i64)),
        "has" => {
            let k = key_of(arg(&args, 0, "map.has")?);
            Ok(Value::Bool(m.contains_key(&k)))
        }
        "keys" => Ok(Value::List(
            m.keys().map(|k| Value::Str(k.clone())).collect(),
        )),
        "vals" => Ok(Value::List(m.values().cloned().collect())),
        "set" => {
            let k = key_of(arg(&args, 0, "map.set")?);
            let v = arg(&args, 1, "map.set")?.clone();
            let mut new = m.clone();
            new.insert(k, v);
            Ok(Value::Map(new))
        }
        "del" => {
            let k = key_of(arg(&args, 0, "map.del")?);
            let mut new = m.clone();
            new.remove(&k);
            Ok(Value::Map(new))
        }
        "merge" => {
            // other'dagi kalitlar ustun keladi (set semantikasi bilan izchil:
            // keyin yozilgan g'olib) — default config + override naqshi uchun.
            let other = match arg(&args, 0, "map.merge")? {
                Value::Map(o) => o.clone(),
                other => {
                    return Err(Flow::err(format!(
                        "map.merge: argument must be a map, got {}",
                        other.type_name()
                    )));
                }
            };
            let mut new = m.clone();
            new.extend(other);
            Ok(Value::Map(new))
        }
        _ => Err(Flow::err(format!("map method '{}' does not exist", method))),
    }
}

fn key_of(v: &Value) -> String {
    match v {
        Value::Str(s) | Value::Sym(s) => s.clone(),
        other => format!("{}", other),
    }
}

// ---------------- argument yordamchilari ----------------
fn arg<'a>(args: &'a [Value], i: usize, who: &str) -> Result<&'a Value, Flow> {
    args.get(i)
        .ok_or_else(|| Flow::err(format!("{}: argument {} is missing", who, i + 1)))
}
pub(crate) fn arg_str(args: &[Value], i: usize, who: &str) -> Result<String, Flow> {
    match arg(args, i, who)? {
        Value::Str(s) => Ok(s.clone()),
        Value::Sym(s) => Ok(s.clone()),
        other => Err(Flow::err(format!(
            "{}: argument {} must be str, got {}",
            who,
            i + 1,
            other.type_name()
        ))),
    }
}
// Ikkilik argumentni o'qiydi. str/sym ham qabul qilinadi (UTF-8 baytlari) —
// crypto kabi iste'molchilar matn va bytes'ni bitta yo'l bilan qabul qilsin
// (AI ikki alohida funksiya nomini o'rganmasin).
pub(crate) fn arg_bytes(args: &[Value], i: usize, who: &str) -> Result<Arc<Vec<u8>>, Flow> {
    match arg(args, i, who)? {
        Value::Bytes(b) => Ok(b.clone()),
        Value::Str(s) | Value::Sym(s) => Ok(Arc::new(s.clone().into_bytes())),
        other => Err(Flow::err(format!(
            "{}: argument {} must be bytes or str, got {}",
            who,
            i + 1,
            other.type_name()
        ))),
    }
}
fn arg_int(args: &[Value], i: usize, who: &str) -> Result<i64, Flow> {
    match arg(args, i, who)? {
        Value::Int(n) => Ok(*n),
        other => Err(Flow::err(format!(
            "{}: argument {} must be int, got {}",
            who,
            i + 1,
            other.type_name()
        ))),
    }
}
// Timestamp argumentini unix sekundga o'qiydi: matn (ISO/kanonik, mintaqa ham)
// yoki to'g'ridan-to'g'ri unix int. time.fmt/add/diff bir xil kirishni qabul qilsin.
fn arg_ts(args: &[Value], i: usize, who: &str) -> Result<i64, Flow> {
    match arg(args, i, who)? {
        Value::Str(s) => parse_iso(s)
            .ok_or_else(|| Flow::err(format!("{}: could not parse timestamp text: {}", who, s))),
        Value::Int(n) => Ok(*n),
        other => Err(Flow::err(format!(
            "{}: argument {} must be timestamp (str/int), got {}",
            who,
            i + 1,
            other.type_name()
        ))),
    }
}
fn arg_num(args: &[Value], i: usize, who: &str) -> Result<f64, Flow> {
    match arg(args, i, who)? {
        Value::Int(n) => Ok(*n as f64),
        Value::Flt(x) => Ok(*x),
        other => Err(Flow::err(format!(
            "{}: argument {} must be a number, got {}",
            who,
            i + 1,
            other.type_name()
        ))),
    }
}

#[cfg(test)]
mod log_tests {
    use super::*;

    fn s(x: &str) -> Value {
        Value::Str(x.to_string())
    }

    // Darajasiz (filtrsiz) — har xabar `[LEVEL] matn` shaklida chiqadi.
    #[test]
    fn text_format_prefiks() {
        assert_eq!(
            format_log("info", &[s("hello")], None, false),
            Some("[INFO] hello".to_string())
        );
        assert_eq!(
            format_log("err", &[s("failed")], None, false),
            Some("[ERR] failed".to_string())
        );
    }

    // Bir nechta argument bo'sh joy bilan birlashadi (eski `log` xatti-harakati).
    #[test]
    fn text_multi_arg() {
        assert_eq!(
            format_log("warn", &[s("a"), Value::Int(2), s("b")], None, false),
            Some("[WARN] a 2 b".to_string())
        );
    }

    // $LOG_LEVEL filtri: minimal darajadan PAST xabarlar None (jim).
    #[test]
    fn level_filter() {
        // min=warn -> debug/info jim, warn/err chiqadi.
        assert_eq!(format_log("debug", &[s("x")], Some("warn"), false), None);
        assert_eq!(format_log("info", &[s("x")], Some("warn"), false), None);
        assert!(format_log("warn", &[s("x")], Some("warn"), false).is_some());
        assert!(format_log("err", &[s("x")], Some("warn"), false).is_some());
    }

    // `error` ham `err` kabi tartiblanadi (LOG_LEVEL'da odam yozishi mumkin).
    #[test]
    fn error_alias() {
        assert_eq!(format_log("warn", &[s("x")], Some("error"), false), None);
        assert!(format_log("err", &[s("x")], Some("error"), false).is_some());
    }

    // Noma'lum LOG_LEVEL info kabi qaraladi — debug filtrlanadi, info o'tadi.
    #[test]
    fn unknown_min_level_info() {
        assert_eq!(format_log("debug", &[s("x")], Some("qqq"), false), None);
        assert!(format_log("info", &[s("x")], Some("qqq"), false).is_some());
    }

    // JSON rejim: strukturali qator, daraja va xabar to'g'ri, tirnoq escape qilinadi.
    #[test]
    fn json_format() {
        let line = format_log("warn", &[s("failed")], None, true).unwrap();
        assert!(
            line.starts_with("{\"time\":\""),
            "time field missing: {}",
            line
        );
        assert!(
            line.contains("\"level\":\"warn\""),
            "level missing: {}",
            line
        );
        assert!(
            line.contains("\"msg\":\"failed\""),
            "message missing: {}",
            line
        );
        // Read back via the decoder — confirms it is valid JSON.
        let Ok(Value::Map(m)) = json_decode(&line) else {
            panic!("invalid JSON: {}", line);
        };
        assert!(matches!(m.get("level"), Some(Value::Str(s)) if s == "warn"));
        assert!(matches!(m.get("msg"), Some(Value::Str(s)) if s == "failed"));
    }

    // JSON xabar ichidagi tirnoq/yangi qator JSON'ni buzmaydi (escape).
    #[test]
    fn json_escapes_message() {
        let line = format_log("info", &[s("a\"b\nc")], None, true).unwrap();
        assert!(json_decode(&line).is_ok(), "escape broken: {}", line);
    }

    // JSON rejimda ham filtr ishlaydi.
    #[test]
    fn json_respects_filter() {
        assert_eq!(format_log("debug", &[s("x")], Some("info"), true), None);
    }
}

#[cfg(test)]
mod rand_tests {
    use super::*;

    // rand.int chegaralar ichida qoladi (a..=b), span=1 ham (a==b).
    #[test]
    fn int_in_range() {
        for _ in 0..1000 {
            let Ok(Value::Int(v)) = rand_module("int", vec![Value::Int(10), Value::Int(20)]) else {
                panic!("rand.int must return an int");
            };
            assert!((10..=20).contains(&v), "out of range: {}", v);
        }
        let Ok(Value::Int(v)) = rand_module("int", vec![Value::Int(7), Value::Int(7)]) else {
            panic!("rand.int must return an int");
        };
        assert_eq!(v, 7); // span=1 -> doim a
    }

    // rand.str so'ralgan uzunlikda va faqat alfanumerik belgilardan iborat.
    #[test]
    fn str_len_and_alphabet() {
        let Ok(Value::Str(s)) = rand_module("str", vec![Value::Int(32)]) else {
            panic!("rand.str must return a string");
        };
        assert_eq!(s.chars().count(), 32);
        assert!(s.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    // Issue #89: chekka chegaralarda span hisobi (b - a + 1) i64 dan toshardi
    // va panic berardi. Endi i128 oraliq — to'liq i64 diapazoni ham ishlaydi.
    #[test]
    fn int_extreme_bounds_no_overflow() {
        for &(a, b) in &[
            (i64::MIN, i64::MAX),     // span = 2^64 (u64 ga ham sig'maydi)
            (i64::MIN, i64::MIN + 5), // juda manfiy tor diapazon
            (i64::MAX - 5, i64::MAX), // juda musbat tor diapazon
            (-3, i64::MAX),           // span > i64::MAX
        ] {
            for _ in 0..50 {
                let Ok(Value::Int(v)) = rand_module("int", vec![Value::Int(a), Value::Int(b)])
                else {
                    panic!("rand.int must return an int ({}..{})", a, b);
                };
                assert!((a..=b).contains(&v), "out of range: {}", v);
            }
        }
    }

    // Kriptografik manba: ketma-ket ikki token bir xil emas (bashorat qilinmas).
    // Eski xorshift'da bir nanosekundda ochilgan thread'lar bir xil olardi.
    #[test]
    fn tokens_are_unique() {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for _ in 0..100 {
            let Ok(Value::Str(s)) = rand_module("str", vec![Value::Int(24)]) else {
                panic!("rand.str must return a string");
            };
            assert!(seen.insert(s), "duplicate token produced — CSPRNG broken");
        }
    }
}

#[cfg(test)]
mod math_tests {
    use super::*;

    // Issue #89: i64::MIN.abs() panic berardi (musbat juftligi i64 ga sig'maydi).
    // Endi Fluxon xatosi; oddiy qiymatlar avvalgidek ishlaydi.
    #[test]
    fn abs_min_is_error_not_panic() {
        let r = math_module("abs", vec![Value::Int(i64::MIN)]);
        let Err(Flow::Error(msg)) = r else {
            panic!("math.abs i64::MIN must return an error");
        };
        assert!(msg.contains("number out of range"), "error text: {}", msg);
        assert!(matches!(
            math_module("abs", vec![Value::Int(-7)]),
            Ok(Value::Int(7))
        ));
    }

    // Issue #128: min/max argument turini saqlaydi — int kirsa int chiqadi,
    // aralash int/flt da g'olibning asl turi qaytadi.
    #[test]
    fn min_max_turni_saqlaydi() {
        assert!(matches!(
            math_module("min", vec![Value::Int(3), Value::Int(7)]),
            Ok(Value::Int(3))
        ));
        assert!(matches!(
            math_module("max", vec![Value::Int(3), Value::Int(7)]),
            Ok(Value::Int(7))
        ));
        // aralash: flt kichik bo'lsa flt qaytadi, int katta bo'lsa int.
        assert!(matches!(
            math_module("min", vec![Value::Int(3), Value::Flt(2.5)]),
            Ok(Value::Flt(v)) if v == 2.5
        ));
        assert!(matches!(
            math_module("max", vec![Value::Int(3), Value::Flt(2.5)]),
            Ok(Value::Int(3))
        ));
        // teng qiymatlarda birinchi argument qaytadi (deterministik).
        assert!(matches!(
            math_module("min", vec![Value::Int(5), Value::Flt(5.0)]),
            Ok(Value::Int(5))
        ));
        // 2^53 dan katta qo'shni int'lar f64 da bir xil yaxlitlanadi —
        // int/int yo'l i64 da aniq taqqoslashi kerak (PR #152 review).
        assert!(matches!(
            math_module("min", vec![Value::Int(i64::MAX), Value::Int(i64::MAX - 1)]),
            Ok(Value::Int(v)) if v == i64::MAX - 1
        ));
        assert!(matches!(
            math_module("max", vec![Value::Int(i64::MAX - 1), Value::Int(i64::MAX)]),
            Ok(Value::Int(v)) if v == i64::MAX
        ));
        // Aralash int/flt da ham yaxlitlanish bo'lmasin (PR #152 review):
        // 2^53+1 (int) f64 ga o'tsa 2^53 ga teng chiqib qolardi.
        let big = (1i64 << 53) + 1; // 9007199254740993
        let big_f = (1i64 << 53) as f64; // 9007199254740992.0
        assert!(matches!(
            math_module("min", vec![Value::Int(big), Value::Flt(big_f)]),
            Ok(Value::Flt(v)) if v == big_f
        ));
        assert!(matches!(
            math_module("max", vec![Value::Flt(big_f), Value::Int(big)]),
            Ok(Value::Int(v)) if v == big
        ));
        // flt i64 oralig'idan tashqarida bo'lsa ham to'g'ri tomon yutadi.
        assert!(matches!(
            math_module("max", vec![Value::Int(i64::MAX), Value::Flt(1e19)]),
            Ok(Value::Flt(v)) if v == 1e19
        ));
        assert!(matches!(
            math_module("min", vec![Value::Int(i64::MIN), Value::Flt(-1e19)]),
            Ok(Value::Flt(v)) if v == -1e19
        ));
        // kasr qismi hal qiluvchi bo'lgan holat: 3 < 3.5, -3 > -3.5.
        assert!(matches!(
            math_module("max", vec![Value::Int(3), Value::Flt(3.5)]),
            Ok(Value::Flt(v)) if v == 3.5
        ));
        assert!(matches!(
            math_module("max", vec![Value::Int(-3), Value::Flt(-3.5)]),
            Ok(Value::Int(-3))
        ));
    }

    // Issue #128: int ^ manfiy bo'lmagan int → int (checked), overflow'da
    // panic emas Fluxon xatosi; manfiy daraja yoki flt aralashsa flt.
    #[test]
    fn pow_int_flt_va_overflow() {
        assert!(matches!(
            math_module("pow", vec![Value::Int(2), Value::Int(10)]),
            Ok(Value::Int(1024))
        ));
        assert!(matches!(
            math_module("pow", vec![Value::Int(2), Value::Int(-1)]),
            Ok(Value::Flt(v)) if v == 0.5
        ));
        assert!(matches!(
            math_module("pow", vec![Value::Flt(2.0), Value::Int(3)]),
            Ok(Value::Flt(v)) if v == 8.0
        ));
        // 2^63 i64 ga sig'maydi — overflow xatosi.
        let r = math_module("pow", vec![Value::Int(2), Value::Int(63)]);
        let Err(Flow::Error(msg)) = r else {
            panic!("math.pow overflow must return an error");
        };
        assert!(msg.contains("number out of range"), "error text: {}", msg);
        // u32 ga sig'maydigan daraja ham overflow (panic emas).
        assert!(math_module("pow", vec![Value::Int(2), Value::Int(u32::MAX as i64 + 1)]).is_err());
    }

    // Issue #128: sqrt doim flt qaytaradi; manfiy kirish NaN emas, aniq xato.
    #[test]
    fn sqrt_flt_va_manfiy_xato() {
        assert!(matches!(
            math_module("sqrt", vec![Value::Int(9)]),
            Ok(Value::Flt(v)) if v == 3.0
        ));
        assert!(matches!(
            math_module("sqrt", vec![Value::Flt(2.25)]),
            Ok(Value::Flt(v)) if v == 1.5
        ));
        let r = math_module("sqrt", vec![Value::Int(-4)]);
        let Err(Flow::Error(msg)) = r else {
            panic!("math.sqrt of a negative number must return an error");
        };
        assert!(msg.contains("negative"), "error text: {}", msg);
    }
}

#[cfg(test)]
mod time_tests {
    use super::*;

    // Ma'lum unix nuqtalar (UTC) — chrono'siz civil algoritmini tekshiramiz.
    #[test]
    fn civil_known_points() {
        assert_eq!(fmt_unix(0), "1970-01-01 00:00:00"); // epoch
        assert_eq!(fmt_unix(1_700_000_000), "2023-11-14 22:13:20");
        // 2024-02-29 (kabisa yili) — 12:00:00 UTC
        assert_eq!(fmt_unix(1_709_208_000), "2024-02-29 12:00:00");
    }

    #[test]
    fn parse_then_fmt_roundtrip() {
        for &u in &[0i64, 1_700_000_000, 1_709_208_000, 4_102_444_800] {
            let s = fmt_unix(u);
            assert_eq!(parse_ts(&s), Some(u), "round-trip broken: {}", s);
        }
        // 'T' ajratuvchi ham qo'llab-quvvatlanadi (ISO).
        assert_eq!(parse_ts("2023-11-14T22:13:20"), Some(1_700_000_000));
    }

    #[test]
    fn ago_subtracts_units() {
        let now = now_unix();
        // 24 soat = 1 kun: ikki yo'l bir xil natija (matn).
        assert_eq!(fmt_unix(now - 24 * 3600), fmt_unix(now - 86_400));
    }

    #[test]
    fn parse_rejects_garbage() {
        assert_eq!(parse_ts("hello"), None);
        assert_eq!(parse_ts("2023-11-14"), None); // too short (no time)
    }

    #[test]
    fn in_adds_units() {
        // time.in kelajakni, time.ago o'tmishni beradi — natija hozirdan keyin/oldin.
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
        // 1 soat keyin > hozir > 1 soat oldin (sekundlik yumaloqlash chetga surilmaydi).
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
        // Qisqa flt kechikish — int emas, kasr ham qabul qilinishini tekshiramiz.
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
        // Manfiy qiymat panic bermasligi kerak — 0 ga klamp qilinadi.
        let r = time_module("sleep", vec![Value::Int(-5)]);
        assert!(
            matches!(r, Ok(Value::Nil)),
            "negative sleep must return nil"
        );
    }

    #[test]
    fn parse_iso_handles_z_and_offsets() {
        // "Z" -> UTC; "+HH:MM"/"-HH:MM" mintaqa UTC ga keltiriladi.
        let z = parse_iso("2026-06-10T10:00:00Z").expect("Z must parse");
        assert_eq!(parse_iso("2026-06-10 10:00:00"), Some(z)); // no zone = UTC
        // +05:00: matndagi vaqt mahalliy, UTC 5 soat oldin.
        assert_eq!(parse_iso("2026-06-10T15:00:00+05:00"), Some(z));
        // -05:00: UTC 5 soat keyin.
        assert_eq!(parse_iso("2026-06-10T05:00:00-05:00"), Some(z));
        // "+HHMM" (ikki nuqtasiz) va kasr sekund ham o'qilsin.
        assert_eq!(parse_iso("2026-06-10T15:00:00.123+0500"), Some(z));
    }

    #[test]
    fn time_parse_normalizes_to_canonical_utc() {
        // time.parse ISO matnni kanonik "YYYY-MM-DD HH:MM:SS" UTC ga keltiradi.
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
        // Mavjud bo'lmagan sana/vaqt jimgina "tuzatilmasin" — rad etilsin
        // (days_from_civil overflow'ni normalizatsiya qiladi, biz oldini olamiz).
        assert_eq!(parse_ts("2026-02-31T10:00:00Z"), None); // fevralda 31 yo'q
        assert_eq!(parse_ts("2026-02-29 00:00:00"), None); // 2026 kabisa emas
        assert_eq!(parse_ts("2026-13-01 00:00:00"), None); // 13-oy yo'q
        assert_eq!(parse_ts("2026-00-10 00:00:00"), None); // 0-oy yo'q
        assert_eq!(parse_ts("2026-06-00 00:00:00"), None); // 0-kun yo'q
        assert_eq!(parse_ts("2026-06-10 24:00:00"), None); // 24-soat yo'q
        assert_eq!(parse_ts("2026-06-10 10:60:00"), None); // 60-daqiqa yo'q
        // Haqiqiy chekka holatlar QABUL qilinadi:
        assert!(parse_ts("2024-02-29 00:00:00").is_some()); // 2024 kabisa
        assert!(parse_ts("2026-12-31 23:59:60").is_some()); // kabisa sekund (60)
    }

    #[test]
    fn time_parse_rejects_impossible_date() {
        let r = time_module("parse", vec![Value::Str("2026-02-31T10:00:00Z".into())]);
        assert!(r.is_err(), "02-31 does not exist — must return an error");
    }

    #[test]
    fn time_add_offsets_arbitrary_timestamp() {
        // Issue #65 yadrosi: start_at + duration -> end_at.
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
        // Manfiy N orqaga siljitadi.
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

    // Issue #89: n * secs ko'paytmasi (yoki yakuniy yig'indi) i64 dan toshsa
    // panic/jim wrap emas, Fluxon xatosi qaytadi — to'rttala offset funksiyada.
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
        // time.sub — add ning ko'zgusi: berilgan vaqtdan orqaga siljitadi.
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
        // diff a b = a - b sekundda; musbat = a kelajakda.
        let r = time_module(
            "diff",
            vec![
                Value::Str("2026-06-10 10:30:00".into()),
                Value::Str("2026-06-10 10:00:00".into()),
            ],
        );
        assert!(matches!(r, Ok(Value::Int(1800))), "30 minutes = 1800 sec");
        // Teskari tartib manfiy beradi.
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
        // Aralash format: biri ISO mintaqali, biri kanonik — ikkisi ham UTC ga keladi.
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
        // Bir xil wall-clock (12:00 local) DST'da turli UTC offset beradi:
        // qishda America/New_York = UTC-5 (EST), yozda UTC-4 (EDT). Fiksrlangan
        // offset DEB hisoblamaslik isboti — issue #80 yadrosi.
        let winter = parse_in_zone("2026-01-15 12:00:00", "America/New_York").unwrap();
        assert_eq!(fmt_unix(winter), "2026-01-15 17:00:00"); // EST: +5 UTC
        let summer = parse_in_zone("2026-07-15 12:00:00", "America/New_York").unwrap();
        assert_eq!(fmt_unix(summer), "2026-07-15 16:00:00"); // EDT: +4 UTC
    }

    #[test]
    fn parse_in_zone_rejects_spring_forward_gap() {
        // 2026-03-08 02:00 -> 03:00 sakraydi: 02:30 local mavjud emas -> None.
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
        // UTC instant -> zona wall-clock (DST hisobga olinib).
        let winter = parse_in_zone("2026-01-15 12:00:00", "America/New_York").unwrap();
        assert_eq!(
            fmt_in_zone(winter, "YYYY-MM-DD HH:mm", "America/New_York").unwrap(),
            "2026-01-15 12:00"
        );
        // Asia/Tashkent doimiy +5 (DST yo'q) — 17:00 UTC -> 22:00 local.
        let utc = parse_ts("2026-06-10 17:00:00").unwrap();
        assert_eq!(fmt_in_zone(utc, "HH:mm", "Asia/Tashkent").unwrap(), "22:00");
    }

    #[test]
    fn time_parse_with_zone_module_level() {
        // time.parse'ning ixtiyoriy 2-argument (zona) yo'li UTC kanonik beradi.
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
        // time.fmt'ning ixtiyoriy 3-argument (zona) local wall-clock beradi.
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

#[cfg(test)]
mod io_tests {
    use super::*;

    // io.print qiymat sifatida nil qaytaradi (stdout'ga yozish — yon ta'sir).
    // Test stdout'ga "" yozadi (bo'sh) — kuzatuvchi chiqishni ifloslamaydi.
    // (Value/Flow Debug derive qilmaydi — unwrap o'rniga match.)
    #[test]
    fn print_returns_nil() {
        match io_module("print", vec![Value::Str(String::new())]) {
            Ok(Value::Nil) => {}
            _ => panic!("io.print must return nil"),
        }
    }

    // io.print argument str bo'lishini talab qiladi.
    #[test]
    fn print_requires_str_arg() {
        assert!(io_module("print", vec![Value::Int(5)]).is_err());
        assert!(io_module("print", vec![]).is_err());
    }

    // Noma'lum io funksiyasi aniq xato beradi. (Flow Debug derive qilmaydi —
    // xato matniga Flow::Error ichidan kiramiz.)
    #[test]
    fn unknown_func_errors() {
        match io_module("nope", vec![]) {
            Err(Flow::Error(msg)) => assert!(msg.contains("io module")),
            _ => panic!("expected Flow::Error"),
        }
    }

    // io modul sifatida tanilishi kerak (argumentsiz Field dispatch shunга tayanadi).
    #[test]
    fn io_is_module() {
        assert!(is_module("io"));
    }
}

#[cfg(test)]
mod fs_tests {
    use super::*;

    // Har test uchun noyob vaqtinchalik papka (boshqa testlar bilan to'qnashmasin).
    // Process pid + test nomi yetarli noyob — testlar parallel ishlasa ham.
    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("fluxon_fs_test_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&p); // oldingi qoldiqni tozalash
        std::fs::create_dir_all(&p).expect("tmp dir not created");
        p
    }

    fn path_str(dir: &std::path::Path, name: &str) -> String {
        dir.join(name).to_string_lossy().into_owned()
    }

    // write + read aylanasi: yozilgan matn aynan o'qiladi.
    #[test]
    fn write_then_read() {
        let dir = tmp_dir("write_read");
        let f = path_str(&dir, "a.txt");
        match fs_module(
            "write",
            vec![Value::Str(f.clone()), Value::Str("hello".into())],
        ) {
            Ok(Value::Sym(s)) if s == "ok" => {}
            _ => panic!("fs.write must return :ok"),
        }
        match fs_module("read", vec![Value::Str(f)]) {
            Ok(Value::Str(s)) => assert_eq!(s, "hello"),
            _ => panic!("fs.read must return the written text"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Yo'q faylni o'qish nil qaytaradi (xato emas) — issue talabi.
    #[test]
    fn read_missing_is_nil() {
        let dir = tmp_dir("read_missing");
        let f = path_str(&dir, "missing.txt");
        match fs_module("read", vec![Value::Str(f)]) {
            Ok(Value::Nil) => {}
            _ => panic!("missing file must return nil"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // append bo'lmagan faylni yaratadi va ketma-ket qo'shadi.
    #[test]
    fn append_accumulates() {
        let dir = tmp_dir("append");
        let f = path_str(&dir, "log.txt");
        let _ = fs_module(
            "append",
            vec![Value::Str(f.clone()), Value::Str("a".into())],
        );
        let _ = fs_module(
            "append",
            vec![Value::Str(f.clone()), Value::Str("b".into())],
        );
        match fs_module("read", vec![Value::Str(f)]) {
            Ok(Value::Str(s)) => assert_eq!(s, "ab"),
            _ => panic!("append must accumulate text"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // exists: mavjud fayl true, yo'q fayl false.
    #[test]
    fn exists_reflects_reality() {
        let dir = tmp_dir("exists");
        let f = path_str(&dir, "present.txt");
        let _ = fs_module("write", vec![Value::Str(f.clone()), Value::Str("x".into())]);
        match fs_module("exists", vec![Value::Str(f)]) {
            Ok(Value::Bool(true)) => {}
            _ => panic!("existing file must be true"),
        }
        match fs_module("exists", vec![Value::Str(path_str(&dir, "missing.txt"))]) {
            Ok(Value::Bool(false)) => {}
            _ => panic!("missing file must be false"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ls: papka ichidagi nomlarni saralangan holda qaytaradi.
    #[test]
    fn ls_lists_sorted_names() {
        let dir = tmp_dir("ls");
        let _ = fs_module(
            "write",
            vec![Value::Str(path_str(&dir, "b.txt")), Value::Str("".into())],
        );
        let _ = fs_module(
            "write",
            vec![Value::Str(path_str(&dir, "a.txt")), Value::Str("".into())],
        );
        match fs_module("ls", vec![Value::Str(dir.to_string_lossy().into_owned())]) {
            Ok(Value::List(items)) => {
                let names: Vec<String> = items
                    .iter()
                    .map(|v| match v {
                        Value::Str(s) => s.clone(),
                        _ => panic!("ls must return a list of str"),
                    })
                    .collect();
                assert_eq!(names, vec!["a.txt".to_string(), "b.txt".to_string()]);
            }
            _ => panic!("ls must return a list"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // del: faylni o'chiradi, keyin exists false bo'ladi.
    #[test]
    fn del_removes_file() {
        let dir = tmp_dir("del");
        let f = path_str(&dir, "o.txt");
        let _ = fs_module("write", vec![Value::Str(f.clone()), Value::Str("x".into())]);
        match fs_module("del", vec![Value::Str(f.clone())]) {
            Ok(Value::Sym(s)) if s == "ok" => {}
            _ => panic!("fs.del must return :ok"),
        }
        match fs_module("exists", vec![Value::Str(f)]) {
            Ok(Value::Bool(false)) => {}
            _ => panic!("deleted file must not exist"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // mkdirp: rekursiv papka yaratadi va idempotent (ikkinchi marta ham :ok).
    #[test]
    fn mkdirp_recursive_and_idempotent() {
        let dir = tmp_dir("mkdirp");
        let nested = path_str(&dir, "a/b/c");
        match fs_module("mkdirp", vec![Value::Str(nested.clone())]) {
            Ok(Value::Sym(s)) if s == "ok" => {}
            _ => panic!("fs.mkdirp must return :ok"),
        }
        assert!(std::path::Path::new(&nested).is_dir());
        // ikkinchi chaqiruv xato bermasligi kerak (idempotent)
        match fs_module("mkdirp", vec![Value::Str(nested)]) {
            Ok(Value::Sym(s)) if s == "ok" => {}
            _ => panic!("mkdirp must be idempotent"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Noma'lum fs funksiyasi aniq xato beradi.
    #[test]
    fn unknown_func_errors() {
        match fs_module("nope", vec![]) {
            Err(Flow::Error(msg)) => assert!(msg.contains("fs module")),
            _ => panic!("expected Flow::Error"),
        }
    }

    // fs modul sifatida tanilishi kerak.
    #[test]
    fn fs_is_module() {
        assert!(is_module("fs"));
    }

    // Ikkilik aylana (issue #132): bytes yoziladi, fs.readb aynan o'sha
    // baytlarni qaytaradi — UTF-8 bo'lmagan tarkib ham buzilmaydi.
    #[test]
    fn write_bytes_then_readb() {
        let dir = tmp_dir("write_readb");
        let f = path_str(&dir, "bin.dat");
        let data = vec![0xff, 0x00, 0xfe, 0x88, 0x01];
        match fs_module(
            "write",
            vec![Value::Str(f.clone()), Value::Bytes(Arc::new(data.clone()))],
        ) {
            Ok(Value::Sym(s)) if s == "ok" => {}
            _ => panic!("fs.write with bytes must return :ok"),
        }
        match fs_module("readb", vec![Value::Str(f.clone())]) {
            Ok(Value::Bytes(b)) => assert_eq!(*b, data),
            _ => panic!("fs.readb must return bytes"),
        }
        // Matnli fayl ham readb bilan o'qiladi (baytlari).
        match fs_module("read", vec![Value::Str(f)]) {
            Err(Flow::Error(_)) => {} // UTF-8 emas — fs.read aniq xato beradi
            _ => panic!("fs.read must error on a non-UTF-8 file"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // fs.readb yo'q faylda nil (fs.read bilan simmetrik).
    #[test]
    fn readb_missing_is_nil() {
        let dir = tmp_dir("readb_missing");
        match fs_module("readb", vec![Value::Str(path_str(&dir, "missing.bin"))]) {
            Ok(Value::Nil) => {}
            _ => panic!("missing file must return nil"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // fs.append bytes bilan ham ishlaydi (str + bytes aralash yozish).
    #[test]
    fn append_bytes() {
        let dir = tmp_dir("append_bytes");
        let f = path_str(&dir, "mix.dat");
        let _ = fs_module(
            "write",
            vec![Value::Str(f.clone()), Value::Str("ab".into())],
        );
        let _ = fs_module(
            "append",
            vec![Value::Str(f.clone()), Value::Bytes(Arc::new(vec![0xff]))],
        );
        match fs_module("readb", vec![Value::Str(f)]) {
            Ok(Value::Bytes(b)) => assert_eq!(*b, vec![b'a', b'b', 0xff]),
            _ => panic!("fs.readb must return bytes"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod bytes_tests {
    use super::*;

    fn b(v: &[u8]) -> Value {
        Value::Bytes(Arc::new(v.to_vec()))
    }

    // of/str aylanasi: matn -> baytlar -> matn (diakritikali ham — UTF-8).
    #[test]
    fn of_str_roundtrip() {
        let src = "hello \u{1F600} world";
        match bytes_module("of", vec![Value::Str(src.into())]) {
            Ok(Value::Bytes(by)) => {
                assert_eq!(by.as_slice(), src.as_bytes());
                match bytes_module("str", vec![Value::Bytes(by)]) {
                    Ok(Value::Str(s)) => assert_eq!(s, src),
                    _ => panic!("bytes.str must return a string"),
                }
            }
            _ => panic!("bytes.of must return bytes"),
        }
    }

    // bytes.of bytes'da idempotent (qayta o'rash yo'q).
    #[test]
    fn of_idempotent() {
        match bytes_module("of", vec![b(&[1, 2, 3])]) {
            Ok(Value::Bytes(by)) => assert_eq!(*by, vec![1, 2, 3]),
            _ => panic!("bytes.of must return the bytes as-is"),
        }
    }

    // bytes.str yaroqsiz UTF-8'da aniq xato (jim buzilmaydi).
    #[test]
    fn str_invalid_utf8_errors() {
        match bytes_module("str", vec![b(&[0xff, 0xfe])]) {
            Err(Flow::Error(msg)) => assert!(msg.contains("UTF-8")),
            _ => panic!("bytes.str must error on invalid UTF-8"),
        }
    }

    // bytes.len BAYT sanaydi (str.len belgi sanashidan farqli) — "o'" 2 belgi,
    // lekin ' (U+2019) 3 bayt.
    #[test]
    fn len_counts_bytes() {
        match bytes_module("len", vec![b(&[1, 2, 3, 4])]) {
            Ok(Value::Int(4)) => {}
            _ => panic!("bytes.len must return 4"),
        }
        match bytes_module("len", vec![Value::Str("\u{1F600}".into())]) {
            Ok(Value::Int(n)) => assert_eq!(n, "\u{1F600}".len() as i64),
            _ => panic!("bytes.len must return the byte count of a str"),
        }
    }

    // bytes.slice str.slice semantikasi: clamp, a >= b -> bo'sh.
    #[test]
    fn slice_clamps() {
        match bytes_module(
            "slice",
            vec![b(&[1, 2, 3, 4]), Value::Int(1), Value::Int(3)],
        ) {
            Ok(Value::Bytes(by)) => assert_eq!(*by, vec![2, 3]),
            _ => panic!("bytes.slice must return bytes"),
        }
        match bytes_module("slice", vec![b(&[1, 2, 3]), Value::Int(2), Value::Int(100)]) {
            Ok(Value::Bytes(by)) => assert_eq!(*by, vec![3]),
            _ => panic!("bytes.slice must clamp the bound"),
        }
        match bytes_module("slice", vec![b(&[1, 2, 3]), Value::Int(2), Value::Int(1)]) {
            Ok(Value::Bytes(by)) => assert!(by.is_empty()),
            _ => panic!("a >= b must return empty bytes"),
        }
    }

    // Tenglik, ko'rinish va turlar — Value darajasidagi shartnoma.
    #[test]
    fn value_contract() {
        assert!(b(&[1, 2]).equals(&b(&[1, 2])));
        assert!(!b(&[1, 2]).equals(&b(&[1, 3])));
        assert!(!b(&[1, 2]).equals(&Value::Str("\u{1}\u{2}".into())));
        assert_eq!(b(&[1, 2]).type_name(), "bytes");
        // Display xom baytlarni oqizmaydi — o'lchamli belgi.
        assert_eq!(format!("{}", b(&[1, 2, 3])), "<bytes 3>");
        assert!(b(&[]).truthy()); // bo'sh bytes ham rost (bo'sh str kabi)
    }

    // JSON'da bytes base64 matn bo'lib tushadi (yo'qotishsiz).
    #[test]
    fn json_encodes_base64() {
        assert_eq!(json_encode(&b(&[0xff, 0x00])), "\"/wA=\"");
    }

    #[test]
    fn bytes_is_module() {
        assert!(is_module("bytes"));
    }

    #[test]
    fn unknown_func_errors() {
        match bytes_module("nope", vec![]) {
            Err(Flow::Error(msg)) => assert!(msg.contains("bytes module")),
            _ => panic!("expected Flow::Error"),
        }
    }
}

#[cfg(test)]
mod sh_tests {
    use super::*;

    // Buyruq turlarini matn sifatida olish (xato matnlarini soddalashtirish uchun).
    fn run(cmd: &str) -> BTreeMap<String, Value> {
        match sh_module("run", vec![Value::Str(cmd.into())]) {
            Ok(Value::Map(m)) => m,
            other => panic!("sh.run must return a map, got {:?}", other.is_ok()),
        }
    }

    // Oddiy echo: stdout to'g'ri, code 0, stderr bo'sh.
    #[test]
    fn echo_stdout_and_code() {
        let m = run("echo hello");
        match m.get("stdout") {
            Some(Value::Str(s)) => assert_eq!(s.trim_end(), "hello"),
            _ => panic!("stdout must be str"),
        }
        assert!(matches!(m.get("code"), Some(Value::Int(0))));
        match m.get("stderr") {
            Some(Value::Str(s)) => assert!(s.is_empty()),
            _ => panic!("stderr must be str"),
        }
    }

    // Non-zero exit: buyruq muvaffaqiyatsiz -> Flow::err EMAS, code != 0.
    #[test]
    fn nonzero_exit_is_not_error() {
        let m = run("exit 3");
        assert!(matches!(m.get("code"), Some(Value::Int(3))));
    }

    // stderr alohida tutiladi (stdout bilan aralashmaydi).
    #[test]
    fn stderr_captured_separately() {
        let m = run("echo error 1>&2");
        match m.get("stderr") {
            Some(Value::Str(s)) => assert_eq!(s.trim_end(), "error"),
            _ => panic!("stderr must be str"),
        }
        match m.get("stdout") {
            Some(Value::Str(s)) => assert!(s.is_empty()),
            _ => panic!("stdout must be str"),
        }
    }

    // Shell xususiyatlari (`&&`, quvur) ishlaydi — buyruq shell orqali boradi.
    #[test]
    fn shell_features_work() {
        let m = run("echo one && echo two");
        match m.get("stdout") {
            Some(Value::Str(s)) => {
                assert!(s.contains("one") && s.contains("two"));
            }
            _ => panic!("stdout must be str"),
        }
        assert!(matches!(m.get("code"), Some(Value::Int(0))));
    }

    // Noma'lum sh funksiyasi aniq xato beradi.
    #[test]
    fn unknown_func_errors() {
        match sh_module("nope", vec![]) {
            Err(Flow::Error(msg)) => assert!(msg.contains("sh module")),
            _ => panic!("expected Flow::Error"),
        }
    }

    // sh modul sifatida tanilishi kerak.
    #[test]
    fn sh_is_module() {
        assert!(is_module("sh"));
    }
}

#[cfg(test)]
mod json_tests {
    use super::*;

    // Control belgilar (0x00–0x1F) \u00XX shaklida escape bo'lishi kerak —
    // issue #102: avval 0x08 kabilar xom chiqib invalid JSON berardi.
    #[test]
    fn control_chars_escaped() {
        let s = Value::Str("a\u{08}b\u{01}c".into());
        // 0x08 -> \b (qisqa shakl), 0x01 -> umumiy \u escape 
        assert_eq!(json_encode(&s), "\"a\\bb\\u0001c\"");
    }

    // \f va \b qisqa shaklda; round-trip dekoder bilan ishlashi kerak.
    #[test]
    fn backspace_formfeed_roundtrip() {
        let s = Value::Str("x\u{0C}y\u{08}z".into());
        let enc = json_encode(&s);
        assert_eq!(enc, "\"x\\fy\\bz\"");
        match json_decode(&enc) {
            Ok(Value::Str(out)) => assert_eq!(out, "x\u{0C}y\u{08}z"),
            other => panic!("round-trip broken: {:?}", other.is_ok()),
        }
    }

    // Infinity/NaN -> null (JSON.stringify xulqi), "inf"/"NaN" emas.
    #[test]
    fn non_finite_floats_become_null() {
        assert_eq!(json_encode(&Value::Flt(f64::INFINITY)), "null");
        assert_eq!(json_encode(&Value::Flt(f64::NEG_INFINITY)), "null");
        assert_eq!(json_encode(&Value::Flt(f64::NAN)), "null");
        // oddiy float o'zgarishsiz qoladi
        assert_eq!(json_encode(&Value::Flt(1.5)), "1.5");
    }

    // Dekoder: qiymatdan keyin chiqindi xato beradi (avval jim qabul qilinardi).
    #[test]
    fn trailing_garbage_rejected() {
        assert!(json_decode("1 garbage").is_err());
        assert!(json_decode("{} extra").is_err());
        // bo'sh joy bilan tugagan to'g'ri JSON esa qabul qilinadi
        assert!(matches!(json_decode("1  \n"), Ok(Value::Int(1))));
    }

    // Dekoder: noto'g'ri `null`-o'xshash matn xato beradi (avval nil berardi).
    #[test]
    fn invalid_null_rejected() {
        assert!(json_decode("nqqq").is_err());
        assert!(matches!(json_decode("null"), Ok(Value::Nil)));
    }

    // Dekoder: yaroqsiz sonlar rad etiladi (boshida '+', ikkita nuqta...).
    #[test]
    fn strict_number_grammar() {
        assert!(json_decode("+5").is_err());
        assert!(json_decode("1.2.3").is_err());
        assert!(json_decode("01").is_err());
        assert!(json_decode("1.").is_err());
        assert!(json_decode("1e").is_err());
        // to'g'ri sonlar ishlaydi
        assert!(matches!(json_decode("-5"), Ok(Value::Int(-5))));
        assert!(matches!(json_decode("1.5e3"), Ok(Value::Flt(_))));
        assert!(matches!(json_decode("0"), Ok(Value::Int(0))));
    }

    // Dekoder: kesilgan/buzuq JSON panic emas, xato qaytaradi (issue #87).
    // Tashqi input (HTTP body) interpreterni yiqitmasligi shart.
    #[test]
    fn truncated_json_no_panic() {
        // satr ochilib tugamasdan kesilgan: `{` -> kalit kutilgan joyda tugash
        assert!(json_decode("{").is_err());
        // ochilib yopilmagan satr
        assert!(json_decode("\"").is_err());
        assert!(json_decode("\"ab").is_err());
        // satr `\` bilan tugab qolgan (escape baytini o'qishda chegaradan o'tish)
        assert!(json_decode("\"ab\\").is_err());
        // ochilib tugamagan massiv/obyekt ham xato
        assert!(json_decode("[").is_err());
        assert!(json_decode("[1,").is_err());
        assert!(json_decode("{\"k\"").is_err());
    }
}
