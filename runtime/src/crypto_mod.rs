// Fluxon crypto battery — cryptographic primitives (issue #131).
//
// Language API:
//   crypto.sha256 s        # -> SHA-256 hex (lowercase)
//   crypto.hmac key msg    # -> HMAC-SHA256 hex — verify webhook signatures
//   crypto.hmac key msg {raw:true}  # -> HMAC-SHA256 bytes — for binary key
//                                   #    chaining (AWS SigV4 signing-key derivation)
//   crypto.b64 s           # -> base64 (standard alphabet, with padding)
//   crypto.b64d s          # -> decode base64 (UTF-8 text), or err
//   crypto.b64db s         # -> decode base64 (bytes — binary safe)
//   crypto.hex s           # -> hex representation of the text bytes
//   crypto.uuid            # -> UUID v4 (OS CSPRNG)
//
// Inputs are str OR bytes (issue #132): no separate function name is needed to
// hash/encode file bytes — arg_bytes accepts both.
//
// The primitives already existed inside the runtime (the `auth` battery uses
// hmac/sha2/base64 for JWT) — this battery exposes them to the user, with no
// new dependency. Why hex output: Stripe/GitHub/Telegram webhook signatures
// arrive in hex — the `crypto.hmac` result is compared directly, no extra
// conversion needed.
//
// Stateless and does not need Interp (reads no env, no IO), but it is wired in
// as a battery like auth/ai (interp::eval_call + Field, with a lookup check):
// if the user has declared the name `crypto` (for example `use ./crypto`),
// theirs takes precedence — it is not in the unconditional is_module list.

use base64::Engine;
use base64::alphabet;
use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig, STANDARD};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use std::sync::Arc;

use crate::builtins::{arg_bytes, arg_str};
use crate::interp::Flow;
use crate::value::Value;

type R = Result<Value, Flow>;

// Padding is not required when decoding: external services send base64 both
// with and without padding — we accept both (one task = one way, the user
// should not have to think about padding).
const LENIENT_STD: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);
// JWT segments and many webhooks use the url-safe alphabet (`-`/`_`) — if the
// standard alphabet does not match, we also try this one.
const LENIENT_URL: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

pub fn crypto_module(func: &str, args: Vec<Value>) -> R {
    match func {
        "sha256" => {
            let b = arg_bytes(&args, 0, "crypto.sha256")?;
            Ok(Value::Str(to_hex(&Sha256::digest(b.as_slice()))))
        }
        "hmac" => {
            let key = arg_bytes(&args, 0, "crypto.hmac")?;
            let msg = arg_bytes(&args, 1, "crypto.hmac")?;
            let mut mac = Hmac::<Sha256>::new_from_slice(key.as_slice())
                .expect("HMAC accepts keys of any length");
            mac.update(msg.as_slice());
            let tag = mac.finalize().into_bytes();
            // {raw:true} returns the 32 raw bytes instead of hex. AWS SigV4
            // derives the signing key as a chain of HMACs where each result is
            // the binary KEY of the next — hex output would break the chain
            // (`key` already accepts Value::Bytes, so the chain round-trips).
            if raw_opt(&args, 2) {
                Ok(Value::Bytes(Arc::new(tag.to_vec())))
            } else {
                Ok(Value::Str(to_hex(&tag)))
            }
        }
        "b64" => {
            let b = arg_bytes(&args, 0, "crypto.b64")?;
            Ok(Value::Str(STANDARD.encode(b.as_slice())))
        }
        "b64d" => {
            let s = arg_str(&args, 0, "crypto.b64d")?;
            let bytes = decode_lenient(&s, "crypto.b64d")?;
            // The result must be text. Silently corrupting binary data (lossy)
            // is unsafe, so we return an explicit error — crypto.b64db is for binary.
            String::from_utf8(bytes)
                .map(Value::Str)
                .map_err(|_| Flow::err("crypto.b64d: result is not UTF-8 text".to_string()))
        }
        // The binary counterpart of b64d (fs.read/fs.readb pattern, issue #132):
        // result is bytes — for non-UTF-8 payloads like images/files.
        "b64db" => {
            let s = arg_str(&args, 0, "crypto.b64db")?;
            Ok(Value::Bytes(Arc::new(decode_lenient(&s, "crypto.b64db")?)))
        }
        "hex" => {
            let b = arg_bytes(&args, 0, "crypto.hex")?;
            Ok(Value::Str(to_hex(b.as_slice())))
        }
        "uuid" => Ok(Value::Str(uuid_v4())),
        _ => Err(Flow::err(format!(
            "crypto.{} not found (sha256/hmac/b64/b64d/b64db/hex/uuid)",
            func
        ))),
    }
}

// Reads the optional `{raw:true}` flag from an opts map at position `i`.
// Absent map / missing key / falsey value all mean "no" — so the default
// (hex output) is preserved for every existing caller.
fn raw_opt(args: &[Value], i: usize) -> bool {
    match args.get(i) {
        Some(Value::Map(m)) => m.get("raw").map(|v| v.truthy()).unwrap_or(false),
        _ => false,
    }
}

// Lenient base64 decoding (shared path for b64d/b64db): padding optional, and
// if the standard alphabet does not match we also try url-safe.
fn decode_lenient(s: &str, who: &str) -> Result<Vec<u8>, Flow> {
    LENIENT_STD
        .decode(s.as_bytes())
        .or_else(|_| LENIENT_URL.decode(s.as_bytes()))
        .map_err(|_| Flow::err(format!("{}: input is not base64", who)))
}

// Converts bytes to lowercase hex text.
fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{:02x}", b);
    }
    s
}

// UUID v4 (RFC 4122): 16 random bytes + version/variant bits. Source is the
// OS CSPRNG (same as the `rand` module, #97 pattern), so there is no collision
// or predictability risk. Without the uuid crate — the format is simple.
fn uuid_v4() -> String {
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    let mut b = [0u8; 16];
    OsRng.fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40; // version = 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant = RFC 4122
    let h = to_hex(&b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Value {
        Value::Str(v.to_string())
    }

    fn out_str(r: R) -> String {
        match r {
            Ok(Value::Str(x)) => x,
            _ => panic!("str expected"),
        }
    }

    // SHA-256 known vector (FIPS 180-2: "abc").
    #[test]
    fn sha256_malum_vektor() {
        assert_eq!(
            out_str(crypto_module("sha256", vec![s("abc")])),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // Empty text also yields a known value.
        assert_eq!(
            out_str(crypto_module("sha256", vec![s("")])),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    // HMAC-SHA256 known vector (RFC 4231, test case 2).
    #[test]
    fn hmac_malum_vektor() {
        assert_eq!(
            out_str(crypto_module(
                "hmac",
                vec![s("Jefe"), s("what do ya want for nothing?")]
            )),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    // {raw:true} returns the bytes whose hex equals the default output — same
    // computation, only the wrapping differs.
    #[test]
    fn hmac_raw_matches_hex() {
        let hex = out_str(crypto_module("hmac", vec![s("k"), s("msg")]));
        let raw = match crypto_module(
            "hmac",
            vec![
                s("k"),
                s("msg"),
                Value::Map(std::collections::BTreeMap::from([(
                    "raw".to_string(),
                    Value::Bool(true),
                )])),
            ],
        ) {
            Ok(Value::Bytes(b)) => b,
            _ => panic!("expected bytes"),
        };
        assert_eq!(to_hex(&raw), hex);
        assert_eq!(raw.len(), 32);
    }

    // The whole point of {raw:true}: AWS SigV4 derives the signing key as a
    // chain of HMACs where each result is the binary KEY of the next. This is
    // the official AWS worked example (Signature V4 test suite): secret
    // wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY, 20150830 / us-east-1 / iam.
    // If raw chaining were broken (hex key reuse), the final hex would differ.
    #[test]
    fn sigv4_signing_key_derivation() {
        let raw = |key: Value, msg: &str| -> Arc<Vec<u8>> {
            match crypto_module(
                "hmac",
                vec![
                    key,
                    s(msg),
                    Value::Map(std::collections::BTreeMap::from([(
                        "raw".to_string(),
                        Value::Bool(true),
                    )])),
                ],
            ) {
                Ok(Value::Bytes(b)) => b,
                _ => panic!("expected bytes"),
            }
        };
        let k_date = raw(
            s("AWS4wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
            "20150830",
        );
        let k_region = raw(Value::Bytes(k_date), "us-east-1");
        let k_service = raw(Value::Bytes(k_region), "iam");
        let k_signing = raw(Value::Bytes(k_service), "aws4_request");
        assert_eq!(
            to_hex(&k_signing),
            "2c94c0cf5378ada6887f09bb697df8fc0affdb34ba1cdd5bda32b664bd55b73c"
        );
    }

    #[test]
    fn b64_roundtrip_va_lenient_dekodlash() {
        assert_eq!(
            out_str(crypto_module("b64", vec![s("hello world")])),
            "aGVsbG8gd29ybGQ="
        );
        // Decodes both with and without padding.
        assert_eq!(
            out_str(crypto_module("b64d", vec![s("aGVsbG8gd29ybGQ=")])),
            "hello world"
        );
        assert_eq!(
            out_str(crypto_module("b64d", vec![s("aGVsbG8gd29ybGQ")])),
            "hello world"
        );
    }

    // JWT segments use the url-safe alphabet — b64d decodes them too.
    #[test]
    fn b64d_url_safe_alifbo() {
        // Text whose bytes require '+'/'/' in the standard alphabet.
        let src = "subjects?_d";
        let enc = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(src.as_bytes());
        assert_eq!(out_str(crypto_module("b64d", vec![s(&enc)])), src);
    }

    #[test]
    fn b64d_xato_holatlar() {
        // Non-base64 input -> explicit error.
        assert!(matches!(
            crypto_module("b64d", vec![s("this is not base64!!!")]),
            Err(Flow::Error(_))
        ));
        // Valid base64, but the result is not UTF-8 -> explicit error (no silent corruption).
        let bad = STANDARD.encode([0xff, 0xfe, 0x00]);
        assert!(matches!(
            crypto_module("b64d", vec![s(&bad)]),
            Err(Flow::Error(_))
        ));
    }

    #[test]
    fn hex_kodlash() {
        assert_eq!(out_str(crypto_module("hex", vec![s("abz")])), "61627a");
        assert_eq!(out_str(crypto_module("hex", vec![s("")])), "");
    }

    // UUID v4 shape: 8-4-4-4-12, version nibble = 4, variant in {8,9,a,b}.
    #[test]
    fn uuid_shakli_va_unikalligi() {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for _ in 0..100 {
            let u = out_str(crypto_module("uuid", vec![]));
            assert_eq!(u.len(), 36);
            let parts: Vec<&str> = u.split('-').collect();
            assert_eq!(
                parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
                vec![8, 4, 4, 4, 12],
                "shape broken: {}",
                u
            );
            assert!(u.chars().all(|c| c == '-' || c.is_ascii_hexdigit()));
            assert_eq!(&u[14..15], "4", "version is not 4: {}", u);
            assert!("89ab".contains(&u[19..20]), "invalid variant: {}", u);
            assert!(seen.insert(u), "duplicate UUID — CSPRNG broken");
        }
    }

    // bytes input (issue #132): the same bytes yield the same result as a str —
    // hashing file bytes is the same path as hashing text.
    #[test]
    fn bytes_kirish_str_bilan_bir_xil() {
        let by = Value::Bytes(Arc::new(b"abc".to_vec()));
        assert_eq!(
            out_str(crypto_module("sha256", vec![by.clone()])),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            out_str(crypto_module("b64", vec![by.clone()])),
            out_str(crypto_module("b64", vec![s("abc")]))
        );
        assert_eq!(out_str(crypto_module("hex", vec![by.clone()])), "616263");
        assert_eq!(
            out_str(crypto_module("hmac", vec![s("key"), by])),
            out_str(crypto_module("hmac", vec![s("key"), s("abc")]))
        );
    }

    // b64db: binary-safe decoding — a non-UTF-8 payload returns bytes
    // (b64d deliberately errors on this same input).
    #[test]
    fn b64db_ikkilik_aylana() {
        let data = vec![0xff, 0xfe, 0x00, 0x88];
        let enc = STANDARD.encode(&data);
        match crypto_module("b64db", vec![s(&enc)]) {
            Ok(Value::Bytes(b)) => assert_eq!(*b, data),
            _ => panic!("crypto.b64db must return bytes"),
        }
        // bytes -> b64 -> b64db full round trip.
        let enc2 = out_str(crypto_module(
            "b64",
            vec![Value::Bytes(Arc::new(data.clone()))],
        ));
        match crypto_module("b64db", vec![s(&enc2)]) {
            Ok(Value::Bytes(b)) => assert_eq!(*b, data),
            _ => panic!("b64 -> b64db roundtrip broken"),
        }
        // Invalid base64 — explicit error.
        assert!(matches!(
            crypto_module("b64db", vec![s("this is not base64!!!")]),
            Err(Flow::Error(_))
        ));
    }

    // Wrong argument type yields an explicit error (not a panic).
    #[test]
    fn notogri_argument_aniq_xato() {
        assert!(matches!(
            crypto_module("sha256", vec![Value::Int(5)]),
            Err(Flow::Error(_))
        ));
        assert!(matches!(
            crypto_module("hmac", vec![s("key")]),
            Err(Flow::Error(_))
        ));
        assert!(matches!(
            crypto_module("no_func", vec![]),
            Err(Flow::Error(_))
        ));
    }
}
