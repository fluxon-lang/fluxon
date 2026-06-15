// ---------------- bytes (binary data, issue #132) ----------------
//
// bytes values have no literal syntax — they are created via functions
// (fs.readb, crypto.b64db, bytes.of). str.len counts CHARS, bytes.len counts
// BYTES — deliberately two separate units.
use std::sync::Arc;

use crate::builtins::R;
use crate::builtins::args::*;
use crate::interp::Flow;
use crate::value::Value;

pub(crate) fn bytes_module(func: &str, args: Vec<Value>) -> R {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::{is_module, json_encode};

    fn b(v: &[u8]) -> Value {
        Value::Bytes(Arc::new(v.to_vec()))
    }

    // of/str round-trip: text -> bytes -> text (with diacritics too — UTF-8).
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

    // bytes.of is idempotent on bytes (no re-wrapping).
    #[test]
    fn of_idempotent() {
        match bytes_module("of", vec![b(&[1, 2, 3])]) {
            Ok(Value::Bytes(by)) => assert_eq!(*by, vec![1, 2, 3]),
            _ => panic!("bytes.of must return the bytes as-is"),
        }
    }

    // bytes.str returns an explicit error on invalid UTF-8 (not silently corrupted).
    #[test]
    fn str_invalid_utf8_errors() {
        match bytes_module("str", vec![b(&[0xff, 0xfe])]) {
            Err(Flow::Error(msg)) => assert!(msg.contains("UTF-8")),
            _ => panic!("bytes.str must error on invalid UTF-8"),
        }
    }

    // bytes.len counts BYTES (unlike str.len which counts chars) — an accented
    // letter is 1 char but its byte count differs.
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

    // bytes.slice has str.slice semantics: clamp, a >= b -> empty.
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

    // Equality, display and types — the Value-level contract.
    #[test]
    fn value_contract() {
        assert!(b(&[1, 2]).equals(&b(&[1, 2])));
        assert!(!b(&[1, 2]).equals(&b(&[1, 3])));
        assert!(!b(&[1, 2]).equals(&Value::Str("\u{1}\u{2}".into())));
        assert_eq!(b(&[1, 2]).type_name(), "bytes");
        // Display does not leak raw bytes — a sized marker.
        assert_eq!(format!("{}", b(&[1, 2, 3])), "<bytes 3>");
        assert!(b(&[]).truthy()); // empty bytes is truthy too (like an empty str)
    }

    // In JSON, bytes become a base64 string (lossless).
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
