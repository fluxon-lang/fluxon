// ---------------- json ----------------
use std::collections::BTreeMap;

use crate::builtins::R;
use crate::builtins::args::*;
use crate::interp::Flow;
use crate::value::Value;

pub(crate) fn json_module(func: &str, args: Vec<Value>) -> R {
    match func {
        "enc" => Ok(Value::Str(json_encode(arg(&args, 0, "json.enc")?))),
        "dec" => {
            let s = arg_str(&args, 0, "json.dec")?;
            json_decode(&s)
        }
        _ => Err(Flow::err(format!("json module has no function '{}'", func))),
    }
}

// Encodes a map into a JSON object (shared by Map and Ctx).
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
        // JSON has no Infinity/NaN — like JSON.stringify we emit `null` (otherwise
        // "inf"/"NaN" is rejected by strict parsers).
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
        // JSON has no binary type — bytes become a base64 string (lossless; more
        // useful than null/corrupted text).
        Value::Bytes(b) => {
            use base64::Engine;
            json_str(&base64::engine::general_purpose::STANDARD.encode(b.as_slice()))
        }
        // ctx is encoded like a plain map (snapshot) — when it lands in a response body.
        Value::Ctx(c) => json_encode_map(&c.lock().unwrap()),
        Value::Fn(_) | Value::Native(_) => "null".into(),
    }
}

pub(crate) fn json_str(s: &str) -> String {
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
            // The remaining control chars (0x00-0x1F) cannot appear raw per the
            // JSON spec — we escape them as \u00XX (otherwise the output is invalid JSON).
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

// Minimal JSON decoder (enough for the core version).
pub fn json_decode(s: &str) -> R {
    let mut p = JsonParser {
        b: s.as_bytes(),
        i: 0,
    };
    p.skip_ws();
    let v = p.value()?;
    p.skip_ws();
    // No garbage may remain after the value — `"1 garbage"` now returns an error
    // (previously it silently returned `1`).
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
            // Check `null` char by char — previously `nqqq` also silently returned nil.
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
        // Avoid going out of bounds and panicking on truncated input (e.g. `{`) —
        // untrusted external data (an HTTP body) must not crash us.
        if self.i >= self.b.len() {
            return Err(Flow::err("json: unexpected end"));
        }
        if self.b[self.i] != b'"' {
            return Err(Flow::err("json: string expected"));
        }
        self.i += 1;
        // Collect the result as BYTES, then convert to a UTF-8 str at the end —
        // multi-byte chars (emoji, accented letters) get CORRUPTED (mojibake) when
        // read byte-by-byte with `as char`. \u escapes are decoded to a char and
        // written as UTF-8.
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
                    // If the string ends with `\` (e.g. `"ab\`) do not go out of
                    // bounds reading the escape byte — otherwise a panic.
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
                            // \uXXXX — a 16-bit code unit. A surrogate pair
                            // (\uD800..DBFF + \uDC00..DFFF) yields a single char
                            // (emoji and everything outside the BMP arrives this way).
                            let hi = self.hex4()?;
                            let ch = if (0xD800..=0xDBFF).contains(&hi) {
                                // high surrogate -> we expect the low surrogate.
                                if self.b.get(self.i) == Some(&b'\\')
                                    && self.b.get(self.i + 1) == Some(&b'u')
                                {
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    let cp = 0x10000
                                        + (((hi as u32 - 0xD800) << 10) | (lo as u32 - 0xDC00));
                                    char::from_u32(cp).unwrap_or('\u{FFFD}')
                                } else {
                                    '\u{FFFD}' // unpaired surrogate
                                }
                            } else {
                                char::from_u32(hi as u32).unwrap_or('\u{FFFD}')
                            };
                            // append the char as UTF-8 bytes.
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        other => out.push(other), // unknown escape — the byte itself
                    }
                }
                // A plain byte (ASCII or part of a multi-byte UTF-8 sequence) —
                // appended as-is, str conversion happens at the end.
                _ => out.push(c),
            }
        }
        Err(Flow::err("json: unterminated string"))
    }

    // Reads 4 hex digits from the current position and returns a u16 (for \uXXXX).
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
    // We follow the JSON number grammar strictly: [-] int [frac] [exp].
    // The previous version swallowed invalid numbers like `+5`, `1.2.3`.
    fn number(&mut self) -> R {
        let start = self.i;
        let mut is_float = false;
        // optional negative sign — JSON allows only '-' (not '+')
        if self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        // integer part: '0' or digits starting from 1-9
        match self.b.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(c) if c.is_ascii_digit() => {
                while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                    self.i += 1;
                }
            }
            _ => return Err(Flow::err("json: invalid number")),
        }
        // fractional part: at least one digit after '.'
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
        // exponent: e/E [+/-] at least one digit
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

#[cfg(test)]
mod tests {
    use super::*;

    // Control chars (0x00-0x1F) must be escaped as \u00XX —
    // issue #102: previously ones like 0x08 came out raw and produced invalid JSON.
    #[test]
    fn control_chars_escaped() {
        let s = Value::Str("a\u{08}b\u{01}c".into());
        // 0x08 -> \b (short form), 0x01 -> the general \u escape
        assert_eq!(json_encode(&s), "\"a\\bb\\u0001c\"");
    }

    // \f and \b in short form; the round-trip must work with the decoder.
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

    // Infinity/NaN -> null (JSON.stringify behavior), not "inf"/"NaN".
    #[test]
    fn non_finite_floats_become_null() {
        assert_eq!(json_encode(&Value::Flt(f64::INFINITY)), "null");
        assert_eq!(json_encode(&Value::Flt(f64::NEG_INFINITY)), "null");
        assert_eq!(json_encode(&Value::Flt(f64::NAN)), "null");
        // an ordinary float is unchanged
        assert_eq!(json_encode(&Value::Flt(1.5)), "1.5");
    }

    // Decoder: garbage after a value returns an error (previously silently accepted).
    #[test]
    fn trailing_garbage_rejected() {
        assert!(json_decode("1 garbage").is_err());
        assert!(json_decode("{} extra").is_err());
        // valid JSON ending with whitespace is accepted
        assert!(matches!(json_decode("1  \n"), Ok(Value::Int(1))));
    }

    // Decoder: invalid `null`-like text returns an error (previously returned nil).
    #[test]
    fn invalid_null_rejected() {
        assert!(json_decode("nqqq").is_err());
        assert!(matches!(json_decode("null"), Ok(Value::Nil)));
    }

    // Decoder: invalid numbers are rejected (leading '+', two dots...).
    #[test]
    fn strict_number_grammar() {
        assert!(json_decode("+5").is_err());
        assert!(json_decode("1.2.3").is_err());
        assert!(json_decode("01").is_err());
        assert!(json_decode("1.").is_err());
        assert!(json_decode("1e").is_err());
        // valid numbers work
        assert!(matches!(json_decode("-5"), Ok(Value::Int(-5))));
        assert!(matches!(json_decode("1.5e3"), Ok(Value::Flt(_))));
        assert!(matches!(json_decode("0"), Ok(Value::Int(0))));
    }

    // Decoder: truncated/broken JSON returns an error, not a panic (issue #87).
    // External input (an HTTP body) must not crash the interpreter.
    #[test]
    fn truncated_json_no_panic() {
        // truncated before a string opens and closes: `{` -> ends where a key is expected
        assert!(json_decode("{").is_err());
        // an opened, unclosed string
        assert!(json_decode("\"").is_err());
        assert!(json_decode("\"ab").is_err());
        // a string ending with `\` (going out of bounds reading the escape byte)
        assert!(json_decode("\"ab\\").is_err());
        // an opened, unclosed array/object is also an error
        assert!(json_decode("[").is_err());
        assert!(json_decode("[1,").is_err());
        assert!(json_decode("{\"k\"").is_err());
    }
}
