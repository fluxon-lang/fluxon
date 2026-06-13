// Fluxon auth battery — authentication primitives (JWT + password hash).
//
// Language API (issue #69):
//   token  = auth.jwt {sub:user.id tenant:t.id role:"admin"}   # -> signed JWT (str)
//   token  = auth.jwt {sub:user.id} {exp:3600}                 # optional expiry (seconds)
//   claims = auth.verify token                                 # -> payload map, or err
//   hash   = auth.hash "parol"                                 # -> argon2id PHC text
//   ok     = auth.check "parol" hash                           # -> bool (constant-time)
//
// Philosophy: "one task = one way", batteries-included. The signing key
// `$AUTH_SECRET` is AUTO-detected from env (matching the `db` -> $DATABASE_URL,
// `ai` -> ANTHROPIC_API_KEY pattern) — you do not have to pass a key on every
// call. If the key is missing we return an EXPLICIT error, just like `ai` does
// without a key.
//
// JWT: HS256 (HMAC-SHA256). A symmetric key is enough for v1; RS256 later
// (open question in the issue). Standard JWT: base64url(header).base64url(payload).
// base64url(HMAC). `auth.verify` AUTO-checks the signature + expiry (exp).
//
// Password hash: argon2id (preferred by the issue). Salt is automatic (stored
// inside the PHC string), `auth.check` is a constant-time comparison — these
// security defaults are applied correctly inside the language, the agent
// cannot get them wrong.
//
// Stateless battery (like `ai`): reads env + computes. Needs Interp to fetch
// the key via `env_lookup` -> the `auth_dispatch` `&self` method.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::builtins::{json_decode, json_encode};
use crate::interp::{Flow, Interp};
use crate::value::Value;

type HmacSha256 = Hmac<Sha256>;

// auth.jwt default expiry: 24 hours (seconds). Overridden with `{exp:N}`.
const DEFAULT_EXP_SECS: i64 = 24 * 60 * 60;

impl Interp {
    // auth.jwt / auth.verify / auth.hash / auth.check dispatch.
    pub fn auth_dispatch(&self, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            "jwt" => self.auth_jwt(args),
            "verify" => self.auth_verify(args),
            "hash" => auth_hash(args),
            "check" => auth_check(args),
            _ => Err(Flow::err(format!(
                "auth.{} not found (jwt/verify/hash/check)",
                func
            ))),
        }
    }

    // Reads `$AUTH_SECRET` from env (OS env > .env). If absent, an EXPLICIT
    // error — like `ai` erroring without a key, it must not work silently (an
    // unsigned JWT would be a security hole).
    fn auth_secret(&self) -> Result<String, Flow> {
        match self.env_lookup("AUTH_SECRET") {
            Value::Str(s) if !s.is_empty() => Ok(s),
            _ => Err(Flow::err(
                "auth: $AUTH_SECRET is not set — set AUTH_SECRET in .env or the \
                 environment to sign/verify JWTs"
                    .to_string(),
            )),
        }
    }

    // auth.jwt {payload} [{exp:seconds}] -> signed JWT text (HS256).
    // `iat` (issued-at time) and `exp` (expiry) are added to the payload map
    // automatically. exp defaults to 24 hours; overridden with the `{exp:N}`
    // opt (N — seconds from now). If the user supplies `exp` in the payload,
    // theirs takes precedence.
    fn auth_jwt(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let payload = match args.first() {
            Some(Value::Map(m)) => m.clone(),
            _ => return Err(Flow::err("auth.jwt: payload (map) required".to_string())),
        };
        // The second argument is an optional {exp:seconds} opt map.
        let exp_secs = match args.get(1) {
            Some(Value::Map(opts)) => match opts.get("exp") {
                Some(v) => as_int(v).ok_or_else(|| {
                    Flow::err("auth.jwt: {exp:N} must be an integer (seconds)".to_string())
                })?,
                None => DEFAULT_EXP_SECS,
            },
            None | Some(Value::Nil) => DEFAULT_EXP_SECS,
            _ => {
                return Err(Flow::err(
                    "auth.jwt: second argument must be an {exp:N} opt".to_string(),
                ));
            }
        };
        let secret = self.auth_secret()?;

        let now = now_unix();
        let mut claims = payload;
        // Standard JWT claims: iat (issued-at time), exp (expiry). If the user
        // supplies these in the payload, we honor them (do not override).
        claims.entry("iat".to_string()).or_insert(Value::Int(now));
        claims
            .entry("exp".to_string())
            .or_insert(Value::Int(now + exp_secs));

        // Header: {alg:"HS256" typ:"JWT"}. BTreeMap order alg<typ — stable.
        let header = json_encode(&header_value());
        let payload_json = json_encode(&Value::Map(claims));

        let signing_input = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header.as_bytes()),
            URL_SAFE_NO_PAD.encode(payload_json.as_bytes())
        );
        let sig = sign_hs256(&signing_input, secret.as_bytes());
        Ok(Value::Str(format!("{}.{}", signing_input, sig)))
    }

    // auth.verify token -> payload map (signature + exp checked), or err.
    // Error cases (all Flow::err — easy to return 401 in a handler):
    //   - wrong format (not 3 segments)
    //   - signature mismatch (wrong key or tampered token)
    //   - no numeric `exp` (token has no expiry — rejected, must not be eternal)
    //   - expired (exp <= now)
    fn auth_verify(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let token = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => return Err(Flow::err("auth.verify: token (str) required".to_string())),
        };
        let secret = self.auth_secret()?;

        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(Flow::err(
                "auth.verify: invalid JWT format (3 segments expected)".to_string(),
            ));
        }
        let signing_input = format!("{}.{}", parts[0], parts[1]);

        // Recompute the signature and compare in constant time (Mac::verify_slice).
        let expected = URL_SAFE_NO_PAD
            .decode(parts[2])
            .map_err(|_| Flow::err("auth.verify: signature is not base64url".to_string()))?;
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any length");
        mac.update(signing_input.as_bytes());
        if mac.verify_slice(&expected).is_err() {
            return Err(Flow::err(
                "auth.verify: invalid signature (key mismatch or token tampered)".to_string(),
            ));
        }

        // Signature is valid -> decode the payload.
        let payload_bytes = URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|_| Flow::err("auth.verify: payload is not base64url".to_string()))?;
        let payload_str = String::from_utf8_lossy(&payload_bytes);
        let claims = match json_decode(&payload_str) {
            Ok(Value::Map(m)) => m,
            _ => {
                return Err(Flow::err(
                    "auth.verify: payload is not a JSON map".to_string(),
                ));
            }
        };

        // Expiry (exp) is MANDATORY — if there is no numeric `exp` we reject it.
        // Otherwise a token without `exp` (or with `exp:nil`/non-numeric) would
        // be valid FOREVER: `auth.jwt` always adds `exp`, but an externally
        // signed or hand-crafted payload may arrive without `exp` — silently
        // saying it "is valid" in that case is a security hole (middleware would
        // accept it indefinitely). So a missing `exp` = an invalid token.
        match claims.get("exp").and_then(as_int) {
            Some(exp) if now_unix() >= exp => {
                return Err(Flow::err("auth.verify: token has expired".to_string()));
            }
            Some(_) => {}
            None => {
                return Err(Flow::err(
                    "auth.verify: token has no `exp` (expiry) field — rejected".to_string(),
                ));
            }
        }
        Ok(Value::Map(claims))
    }
}

// auth.hash "parol" -> argon2id PHC text (salt embedded). Does not need Interp.
fn auth_hash(args: Vec<Value>) -> Result<Value, Flow> {
    let pw = match args.first() {
        Some(Value::Str(s)) => s.clone(),
        _ => return Err(Flow::err("auth.hash: password (str) required".to_string())),
    };
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map_err(|e| Flow::err(format!("auth.hash: hash error: {}", e)))?
        .to_string();
    Ok(Value::Str(hash))
}

// auth.check "parol" hash -> bool. Constant-time comparison (argon2 verify).
// If the hash is corrupt -> false (not an error; a failed check = false).
fn auth_check(args: Vec<Value>) -> Result<Value, Flow> {
    let pw = match args.first() {
        Some(Value::Str(s)) => s.clone(),
        _ => return Err(Flow::err("auth.check: password (str) required".to_string())),
    };
    let hash_str = match args.get(1) {
        Some(Value::Str(s)) => s.clone(),
        _ => return Err(Flow::err("auth.check: hash (str) required".to_string())),
    };
    let parsed = match PasswordHash::new(&hash_str) {
        Ok(h) => h,
        // Corrupt/malformed hash — check is false (not an error).
        Err(_) => return Ok(Value::Bool(false)),
    };
    let ok = Argon2::default()
        .verify_password(pw.as_bytes(), &parsed)
        .is_ok();
    Ok(Value::Bool(ok))
}

// --- Helpers ---

// JWT header value: {alg:"HS256" typ:"JWT"}.
fn header_value() -> Value {
    let mut h = BTreeMap::new();
    h.insert("alg".to_string(), Value::Str("HS256".to_string()));
    h.insert("typ".to_string(), Value::Str("JWT".to_string()));
    Value::Map(h)
}

// HMAC-SHA256 signature -> base64url (no padding) text.
fn sign_hs256(signing_input: &str, secret: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts keys of any length");
    mac.update(signing_input.as_bytes());
    let sig = mac.finalize().into_bytes();
    URL_SAFE_NO_PAD.encode(sig)
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// Reads an int from a Value (Int or Flt). For numbers in JWT claims.
fn as_int(v: &Value) -> Option<i64> {
    match v {
        Value::Int(n) => Some(*n),
        Value::Flt(x) => Some(*x as i64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Sign/verify with a fixed key for testing (without Interp —
    // auth_jwt/auth_verify need env, but we test the signature/format logic
    // through the low-level helpers).
    const SECRET: &[u8] = b"test-secret-key";

    #[test]
    fn sign_and_verify_roundtrip() {
        let input = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0";
        let sig = sign_hs256(input, SECRET);
        // Recomputing yields the same signature (deterministic).
        assert_eq!(sig, sign_hs256(input, SECRET));
        // A different key -> a different signature.
        assert_ne!(sig, sign_hs256(input, b"other-key"));
    }

    #[test]
    fn sign_is_base64url_no_pad() {
        let sig = sign_hs256("a.b", SECRET);
        // base64url: must not contain '+' '/' '='.
        assert!(!sig.contains('+'));
        assert!(!sig.contains('/'));
        assert!(!sig.contains('='));
    }

    #[test]
    fn header_shape() {
        let h = match header_value() {
            Value::Map(m) => m,
            _ => panic!("map expected"),
        };
        assert!(matches!(h.get("alg"), Some(Value::Str(s)) if s == "HS256"));
        assert!(matches!(h.get("typ"), Some(Value::Str(s)) if s == "JWT"));
    }

    #[test]
    fn hash_and_check_roundtrip() {
        let hash = match auth_hash(vec![Value::Str("password123".to_string())]) {
            Ok(Value::Str(s)) => s,
            _ => panic!("hash str expected"),
        };
        // The PHC string starts with argon2id.
        assert!(hash.starts_with("$argon2id$"));
        // Correct password -> true.
        let ok = auth_check(vec![
            Value::Str("password123".to_string()),
            Value::Str(hash.clone()),
        ]);
        assert!(matches!(ok, Ok(Value::Bool(true))));
        // Wrong password -> false.
        let bad = auth_check(vec![Value::Str("wrong".to_string()), Value::Str(hash)]);
        assert!(matches!(bad, Ok(Value::Bool(false))));
    }

    #[test]
    fn check_handles_garbage_hash() {
        // Corrupt hash -> false (not a panic/err).
        let r = auth_check(vec![
            Value::Str("password".to_string()),
            Value::Str("garbage-hash".to_string()),
        ]);
        assert!(matches!(r, Ok(Value::Bool(false))));
    }

    #[test]
    fn hash_includes_random_salt() {
        // Same password -> a different hash each time (random salt).
        let h1 = match auth_hash(vec![Value::Str("x".to_string())]) {
            Ok(Value::Str(s)) => s,
            _ => panic!(),
        };
        let h2 = match auth_hash(vec![Value::Str("x".to_string())]) {
            Ok(Value::Str(s)) => s,
            _ => panic!(),
        };
        assert_ne!(h1, h2);
    }
}
