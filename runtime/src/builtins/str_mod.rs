// ---------------- str ----------------
use crate::builtins::R;
use crate::builtins::args::*;
use crate::interp::Flow;
use crate::value::Value;

pub(crate) fn str_module(func: &str, args: Vec<Value>) -> R {
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
            let chars: Vec<char> = s.chars().collect();
            // The end index is OPTIONAL and defaults to end-of-string, matching the
            // near-universal Python/JS prior (`s[a:]`). `str.slice s a` ≡
            // `str.slice s a (str.len s)`. With it given, bounds clamp as before.
            let b = match args.get(2) {
                Some(_) => arg_int(&args, 2, "str.slice")? as usize,
                None => chars.len(),
            };
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
        // str.cmp a b -> -1 | 0 | 1, lexicographic by Unicode code point. The
        // canonical three-way compare for sorting/pagination cursor keys; `<`/`>`
        // on str work too, this gives a single ordering value. See issue #174.
        "cmp" => {
            let a = arg_str(&args, 0, "str.cmp")?;
            let b = arg_str(&args, 1, "str.cmp")?;
            Ok(Value::Int(match a.cmp(&b) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            }))
        }
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
        // str.url_enc s → RFC 3986 percent-encoding. EVERY byte that is not an
        // unreserved char (A-Z a-z 0-9 - _ . ~) becomes %XX (uppercase hex). This
        // is AWS's exact `UriEncode` for SigV4 query/path segments — note `/` is
        // also encoded, so to keep slashes in an object key, split on "/", encode
        // each segment, and re-join (the s3 package does this). Operates on UTF-8
        // bytes, so non-ASCII keys round-trip.
        "url_enc" => {
            let s = arg_str(&args, 0, "str.url_enc")?;
            let mut out = String::with_capacity(s.len());
            for b in s.bytes() {
                let unreserved =
                    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~';
                if unreserved {
                    out.push(b as char);
                } else {
                    out.push('%');
                    out.push(hex_upper(b >> 4));
                    out.push(hex_upper(b & 0x0f));
                }
            }
            Ok(Value::Str(out))
        }
        _ => Err(Flow::err(format!("str module has no function '{}'", func))),
    }
}

// One hex digit (0-15) as an uppercase ASCII char. SigV4 requires UPPERCASE
// percent-encoding (`%2F`, not `%2f`) — a lowercased escape changes the
// canonical request and produces a SignatureDoesNotMatch error.
fn hex_upper(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'A' + (nibble - 10)) as char,
    }
}
