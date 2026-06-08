// Flux auth battery — autentifikatsiya primitivlari (JWT + parol hash).
//
// Til API (issue #69):
//   token  = auth.jwt {sub:user.id tenant:t.id role:"admin"}   # -> imzolangan JWT (str)
//   token  = auth.jwt {sub:user.id} {exp:3600}                 # ixtiyoriy muddat (sekund)
//   claims = auth.verify token                                 # -> payload map, yoki err
//   hash   = auth.hash "parol"                                 # -> argon2id PHC matn
//   ok     = auth.check "parol" hash                           # -> bool (doimiy-vaqt)
//
// Falsafa: "bir ish = bir yo'l", batteries-included. Imzo kaliti `$AUTH_SECRET`
// env'dan AVTO-aniqlanadi (`db` -> $DATABASE_URL, `ai` -> ANTHROPIC_API_KEY
// naqshiga mos) — har chaqiruvda kalit berish shart emas. Kalit yo'q bo'lsa
// `ai` kalitsiz xato bergani kabi ANIQ xato beramiz.
//
// JWT: HS256 (HMAC-SHA256). Simmetrik kalit v1 uchun yetarli; RS256 keyinroq
// (issue ochiq savoli). Standart JWT: base64url(header).base64url(payload).
// base64url(HMAC). `auth.verify` imzo + muddat (exp) ni AVTO tekshiradi.
//
// Parol hash: argon2id (issue afzal ko'rgan). Salt avtomatik (PHC string
// ichida saqlanadi), `auth.check` doimiy-vaqt taqqoslash — bu xavfsizlik
// default'lari til ichida to'g'ri bajariladi, agent xato qila olmaydi.
//
// Holatsiz battery (`ai` kabi): env o'qiydi + hisoblaydi. Kalitni `env_lookup`
// orqali olish uchun Interp'ga muhtoj -> `auth_dispatch` `&self` metodi.

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

// auth.jwt default muddat: 24 soat (sekund). `{exp:N}` bilan override qilinadi.
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
                "auth.{} yo'q (jwt/verify/hash/check)",
                func
            ))),
        }
    }

    // `$AUTH_SECRET` ni env'dan oladi (OS env > .env). Yo'q bo'lsa ANIQ xato —
    // `ai` kalitsiz xato bergani kabi, jim ishlamaslik kerak (imzosiz JWT
    // xavfsizlik teshigi bo'lardi).
    fn auth_secret(&self) -> Result<String, Flow> {
        match self.env_lookup("AUTH_SECRET") {
            Value::Str(s) if !s.is_empty() => Ok(s),
            _ => Err(Flow::err(
                "auth: $AUTH_SECRET belgilanmagan — JWT imzolash/tekshirish uchun \
                 .env yoki muhitda AUTH_SECRET belgilang"
                    .to_string(),
            )),
        }
    }

    // auth.jwt {payload} [{exp:sekund}] -> imzolangan JWT matn (HS256).
    // payload map'iga `iat` (berilgan vaqt) va `exp` (muddat) avtomatik
    // qo'shiladi. exp default 24 soat; `{exp:N}` opt bilan override qilinadi
    // (N — hozirdan sekund). Foydalanuvchi payload'da `exp` bersa, u ustun.
    fn auth_jwt(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let payload = match args.first() {
            Some(Value::Map(m)) => m.clone(),
            _ => return Err(Flow::err("auth.jwt: payload (map) kerak".to_string())),
        };
        // Ikkinchi argument — ixtiyoriy {exp:sekund} opt map.
        let exp_secs = match args.get(1) {
            Some(Value::Map(opts)) => match opts.get("exp") {
                Some(v) => as_int(v).ok_or_else(|| {
                    Flow::err("auth.jwt: {exp:N} butun son (sekund) bo'lishi kerak".to_string())
                })?,
                None => DEFAULT_EXP_SECS,
            },
            None | Some(Value::Nil) => DEFAULT_EXP_SECS,
            _ => {
                return Err(Flow::err(
                    "auth.jwt: ikkinchi argument {exp:N} opt bo'lishi kerak".to_string(),
                ));
            }
        };
        let secret = self.auth_secret()?;

        let now = now_unix();
        let mut claims = payload;
        // Standart JWT da'volari: iat (berilgan vaqt), exp (muddat). Foydalanuvchi
        // bularni payload'da o'zi bersa, hurmat qilamiz (override qilmaymiz).
        claims.entry("iat".to_string()).or_insert(Value::Int(now));
        claims
            .entry("exp".to_string())
            .or_insert(Value::Int(now + exp_secs));

        // Header: {alg:"HS256" typ:"JWT"}. BTreeMap tartibi alg<typ — barqaror.
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

    // auth.verify token -> payload map (imzo + exp tekshirilgan), yoki err.
    // Xato holatlari (hammasi Flow::err — handler'da 401 qaytarish oson):
    //   - shakl noto'g'ri (3 segment emas)
    //   - imzo mos kelmaydi (kalit noto'g'ri yoki token buzilgan)
    //   - sonli `exp` yo'q (token muddatsiz — rad etiladi, abadiy bo'lmasin)
    //   - muddat o'tgan (exp <= hozir)
    fn auth_verify(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let token = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => return Err(Flow::err("auth.verify: token (str) kerak".to_string())),
        };
        let secret = self.auth_secret()?;

        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(Flow::err(
                "auth.verify: JWT shakli noto'g'ri (3 segment kutilgan)".to_string(),
            ));
        }
        let signing_input = format!("{}.{}", parts[0], parts[1]);

        // Imzoni qayta hisoblab, doimiy-vaqt taqqoslaymiz (Mac::verify_slice).
        let expected = URL_SAFE_NO_PAD
            .decode(parts[2])
            .map_err(|_| Flow::err("auth.verify: imzo base64url emas".to_string()))?;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .expect("HMAC har xil kalit uzunligini qabul qiladi");
        mac.update(signing_input.as_bytes());
        if mac.verify_slice(&expected).is_err() {
            return Err(Flow::err(
                "auth.verify: imzo noto'g'ri (kalit mos kelmaydi yoki token buzilgan)".to_string(),
            ));
        }

        // Imzo to'g'ri -> payload'ni dekod qilamiz.
        let payload_bytes = URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|_| Flow::err("auth.verify: payload base64url emas".to_string()))?;
        let payload_str = String::from_utf8_lossy(&payload_bytes);
        let claims = match json_decode(&payload_str) {
            Ok(Value::Map(m)) => m,
            _ => return Err(Flow::err("auth.verify: payload JSON map emas".to_string())),
        };

        // Muddat (exp) MAJBURIY — sonli `exp` bo'lmasa rad etamiz. Aks holda
        // `exp`siz (yoki `exp:nil`/sonli-emas) token ABADIY amal qilardi:
        // `auth.jwt` har doim `exp` qo'shadi, lekin tashqi imzolangan yoki
        // qo'lda yasalgan payload `exp`siz kelishi mumkin — o'sha holatda
        // jim "amal qiladi" deyish xavfsizlik teshigi (middleware uni cheksiz
        // qabul qilardi). Shuning uchun `exp` yo'qligi = noto'g'ri token.
        match claims.get("exp").and_then(as_int) {
            Some(exp) if now_unix() >= exp => {
                return Err(Flow::err("auth.verify: token muddati o'tgan".to_string()));
            }
            Some(_) => {}
            None => {
                return Err(Flow::err(
                    "auth.verify: token `exp` (muddat) maydonisiz — rad etildi".to_string(),
                ));
            }
        }
        Ok(Value::Map(claims))
    }
}

// auth.hash "parol" -> argon2id PHC matn (salt ichida). Interp'ga muhtoj emas.
fn auth_hash(args: Vec<Value>) -> Result<Value, Flow> {
    let pw = match args.first() {
        Some(Value::Str(s)) => s.clone(),
        _ => return Err(Flow::err("auth.hash: parol (str) kerak".to_string())),
    };
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map_err(|e| Flow::err(format!("auth.hash: hash xatosi: {}", e)))?
        .to_string();
    Ok(Value::Str(hash))
}

// auth.check "parol" hash -> bool. Doimiy-vaqt taqqoslash (argon2 verify).
// Hash buzuq bo'lsa -> false (xato emas; tekshiruv muvaffaqiyatsiz = false).
fn auth_check(args: Vec<Value>) -> Result<Value, Flow> {
    let pw = match args.first() {
        Some(Value::Str(s)) => s.clone(),
        _ => return Err(Flow::err("auth.check: parol (str) kerak".to_string())),
    };
    let hash_str = match args.get(1) {
        Some(Value::Str(s)) => s.clone(),
        _ => return Err(Flow::err("auth.check: hash (str) kerak".to_string())),
    };
    let parsed = match PasswordHash::new(&hash_str) {
        Ok(h) => h,
        // Buzuq/noto'g'ri shakldagi hash — tekshiruv false (xato emas).
        Err(_) => return Ok(Value::Bool(false)),
    };
    let ok = Argon2::default()
        .verify_password(pw.as_bytes(), &parsed)
        .is_ok();
    Ok(Value::Bool(ok))
}

// --- Yordamchilar ---

// JWT header qiymati: {alg:"HS256" typ:"JWT"}.
fn header_value() -> Value {
    let mut h = BTreeMap::new();
    h.insert("alg".to_string(), Value::Str("HS256".to_string()));
    h.insert("typ".to_string(), Value::Str("JWT".to_string()));
    Value::Map(h)
}

// HMAC-SHA256 imzo -> base64url (padding'siz) matn.
fn sign_hs256(signing_input: &str, secret: &[u8]) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret).expect("HMAC har xil kalit uzunligini qabul qiladi");
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

// Value'dan int o'qish (Int yoki Flt). JWT da'volaridagi sonlar uchun.
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

    // Test uchun sobit kalit bilan imzolash/tekshirish (Interp'siz —
    // auth_jwt/auth_verify env'ga muhtoj, lekin imzo/format mantiqini
    // past-darajali yordamchilar orqali sinaymiz).
    const SECRET: &[u8] = b"test-secret-key";

    #[test]
    fn sign_and_verify_roundtrip() {
        let input = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0";
        let sig = sign_hs256(input, SECRET);
        // Qayta hisoblansa bir xil imzo (deterministik).
        assert_eq!(sig, sign_hs256(input, SECRET));
        // Boshqa kalit -> boshqa imzo.
        assert_ne!(sig, sign_hs256(input, b"boshqa-kalit"));
    }

    #[test]
    fn sign_is_base64url_no_pad() {
        let sig = sign_hs256("a.b", SECRET);
        // base64url: '+' '/' '=' bo'lmasligi kerak.
        assert!(!sig.contains('+'));
        assert!(!sig.contains('/'));
        assert!(!sig.contains('='));
    }

    #[test]
    fn header_shape() {
        let h = match header_value() {
            Value::Map(m) => m,
            _ => panic!("map kutilgan"),
        };
        assert!(matches!(h.get("alg"), Some(Value::Str(s)) if s == "HS256"));
        assert!(matches!(h.get("typ"), Some(Value::Str(s)) if s == "JWT"));
    }

    #[test]
    fn hash_and_check_roundtrip() {
        let hash = match auth_hash(vec![Value::Str("parol123".to_string())]) {
            Ok(Value::Str(s)) => s,
            _ => panic!("hash str kutilgan"),
        };
        // PHC string argon2id bilan boshlanadi.
        assert!(hash.starts_with("$argon2id$"));
        // To'g'ri parol -> true.
        let ok = auth_check(vec![
            Value::Str("parol123".to_string()),
            Value::Str(hash.clone()),
        ]);
        assert!(matches!(ok, Ok(Value::Bool(true))));
        // Noto'g'ri parol -> false.
        let bad = auth_check(vec![Value::Str("xato".to_string()), Value::Str(hash)]);
        assert!(matches!(bad, Ok(Value::Bool(false))));
    }

    #[test]
    fn check_handles_garbage_hash() {
        // Buzuq hash -> false (panic/err emas).
        let r = auth_check(vec![
            Value::Str("parol".to_string()),
            Value::Str("buzuq-hash".to_string()),
        ]);
        assert!(matches!(r, Ok(Value::Bool(false))));
    }

    #[test]
    fn hash_includes_random_salt() {
        // Bir xil parol -> har gal boshqa hash (salt tasodifiy).
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
