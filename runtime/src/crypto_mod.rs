// Flux crypto battery — kriptografik primitivlar (issue #131).
//
// Til API:
//   crypto.sha256 s        # -> SHA-256 hex (kichik harf)
//   crypto.hmac key msg    # -> HMAC-SHA256 hex — webhook imzo tekshirish
//   crypto.b64 s           # -> base64 (standart alifbo, padding bilan)
//   crypto.b64d s          # -> base64'ni ochish (UTF-8 matn), yoki err
//   crypto.b64db s         # -> base64'ni ochish (bytes — ikkilik xavfsiz)
//   crypto.hex s           # -> matn baytlarining hex ko'rinishi
//   crypto.uuid            # -> UUID v4 (OS CSPRNG)
//
// Kirishlar str YOKI bytes (issue #132): fayl baytlarini hash'lash/kodlash
// uchun alohida funksiya nomi kerak emas — arg_bytes ikkalasini qabul qiladi.
//
// Primitivlar runtime ichida allaqachon bor edi (`auth` battery JWT uchun
// hmac/sha2/base64 ishlatadi) — bu battery ularni foydalanuvchiga ochadi,
// yangi dependency yo'q. Nega hex chiqish: Stripe/GitHub/Telegram webhook
// imzolari hex'da keladi — `crypto.hmac` natijasi to'g'ridan-to'g'ri
// taqqoslanadi, qo'shimcha konversiya kerak emas.
//
// Holatsiz va Interp'ga muhtoj emas (env o'qimaydi, IO yo'q), lekin auth/ai
// kabi battery sifatida ulanadi (interp::eval_call + Field, lookup tekshiruvi
// bilan): foydalanuvchi `crypto` nomini e'lon qilgan bo'lsa (masalan
// `use ./crypto`), uniki ustun — shartsiz is_module ro'yxatiga kirmaydi.

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

// Dekodlashda padding majburiy emas: tashqi servislar base64'ni ham padding'li,
// ham padding'siz yuboradi — ikkalasini ham qabul qilamiz (bir ish = bir yo'l,
// foydalanuvchi padding haqida o'ylamasin).
const LENIENT_STD: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);
// JWT segmentlari va ko'p webhook'lar url-safe alifboda (`-`/`_`) — standart
// alifbo mos kelmasa shunisi bilan ham urinamiz.
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
                .expect("HMAC har xil kalit uzunligini qabul qiladi");
            mac.update(msg.as_slice());
            Ok(Value::Str(to_hex(&mac.finalize().into_bytes())))
        }
        "b64" => {
            let b = arg_bytes(&args, 0, "crypto.b64")?;
            Ok(Value::Str(STANDARD.encode(b.as_slice())))
        }
        "b64d" => {
            let s = arg_str(&args, 0, "crypto.b64d")?;
            let bytes = decode_lenient(&s, "crypto.b64d")?;
            // Natija matn bo'lishi shart. Ikkilik ma'lumotni jim buzib qaytarish
            // (lossy) xavfli, aniq xato beramiz — ikkilik uchun crypto.b64db bor.
            String::from_utf8(bytes)
                .map(Value::Str)
                .map_err(|_| Flow::err("crypto.b64d: natija UTF-8 matn emas".to_string()))
        }
        // b64d'ning ikkilik juftligi (fs.read/fs.readb naqshi, issue #132):
        // natija bytes — rasm/fayl kabi UTF-8 bo'lmagan yuklamalar uchun.
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
            "crypto.{} yo'q (sha256/hmac/b64/b64d/b64db/hex/uuid)",
            func
        ))),
    }
}

// Lenient base64 dekodlash (b64d/b64db umumiy yo'li): padding ixtiyoriy,
// standart alifbo mos kelmasa url-safe bilan ham urinamiz.
fn decode_lenient(s: &str, who: &str) -> Result<Vec<u8>, Flow> {
    LENIENT_STD
        .decode(s.as_bytes())
        .or_else(|_| LENIENT_URL.decode(s.as_bytes()))
        .map_err(|_| Flow::err(format!("{}: kirish base64 emas", who)))
}

// Baytlarni kichik-harf hex matnga aylantiradi.
fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{:02x}", b);
    }
    s
}

// UUID v4 (RFC 4122): 16 tasodifiy bayt + versiya/variant bitlari. Manba —
// OS CSPRNG (`rand` moduli bilan bir xil, #97 naqshi), shuning uchun
// to'qnashuv/bashorat xavfi yo'q. uuid crate'siz — format oddiy.
fn uuid_v4() -> String {
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    let mut b = [0u8; 16];
    OsRng.fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40; // versiya = 4
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
            _ => panic!("str kutilgan"),
        }
    }

    // SHA-256 ma'lum vektor (FIPS 180-2: "abc").
    #[test]
    fn sha256_malum_vektor() {
        assert_eq!(
            out_str(crypto_module("sha256", vec![s("abc")])),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // Bo'sh matn ham aniq qiymat beradi.
        assert_eq!(
            out_str(crypto_module("sha256", vec![s("")])),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    // HMAC-SHA256 ma'lum vektor (RFC 4231, test case 2).
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

    #[test]
    fn b64_roundtrip_va_lenient_dekodlash() {
        assert_eq!(
            out_str(crypto_module("b64", vec![s("salom dunyo")])),
            "c2Fsb20gZHVueW8="
        );
        // Padding'li ham, padding'siz ham ochiladi.
        assert_eq!(
            out_str(crypto_module("b64d", vec![s("c2Fsb20gZHVueW8=")])),
            "salom dunyo"
        );
        assert_eq!(
            out_str(crypto_module("b64d", vec![s("c2Fsb20gZHVueW8")])),
            "salom dunyo"
        );
    }

    // JWT segmentlari url-safe alifboda — b64d ularni ham ochadi.
    #[test]
    fn b64d_url_safe_alifbo() {
        // Baytlari standart alifboda '+'/'/' talab qiladigan matn.
        let src = "subjects?_d";
        let enc = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(src.as_bytes());
        assert_eq!(out_str(crypto_module("b64d", vec![s(&enc)])), src);
    }

    #[test]
    fn b64d_xato_holatlar() {
        // base64 bo'lmagan kirish -> aniq xato.
        assert!(matches!(
            crypto_module("b64d", vec![s("bu base64 emas!!!")]),
            Err(Flow::Error(_))
        ));
        // Yaroqli base64, lekin natija UTF-8 emas -> aniq xato (jim buzilmaydi).
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

    // UUID v4 shakli: 8-4-4-4-12, versiya nibble = 4, variant ∈ {8,9,a,b}.
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
                "shakl buzildi: {}",
                u
            );
            assert!(u.chars().all(|c| c == '-' || c.is_ascii_hexdigit()));
            assert_eq!(&u[14..15], "4", "versiya 4 emas: {}", u);
            assert!("89ab".contains(&u[19..20]), "variant noto'g'ri: {}", u);
            assert!(seen.insert(u), "takror UUID — CSPRNG buzildi");
        }
    }

    // bytes kirish (issue #132): bir xil baytlar str bilan bir xil natija beradi —
    // fayl baytlarini hash'lash matnni hash'lash bilan bitta yo'l.
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
            out_str(crypto_module("hmac", vec![s("kalit"), by])),
            out_str(crypto_module("hmac", vec![s("kalit"), s("abc")]))
        );
    }

    // b64db: ikkilik xavfsiz dekodlash — UTF-8 bo'lmagan yuklama bytes qaytadi
    // (b64d shu kirishda ataylab xato beradi).
    #[test]
    fn b64db_ikkilik_aylana() {
        let data = vec![0xff, 0xfe, 0x00, 0x88];
        let enc = STANDARD.encode(&data);
        match crypto_module("b64db", vec![s(&enc)]) {
            Ok(Value::Bytes(b)) => assert_eq!(*b, data),
            _ => panic!("crypto.b64db bytes qaytarishi kerak"),
        }
        // bytes -> b64 -> b64db to'liq aylana.
        let enc2 = out_str(crypto_module(
            "b64",
            vec![Value::Bytes(Arc::new(data.clone()))],
        ));
        match crypto_module("b64db", vec![s(&enc2)]) {
            Ok(Value::Bytes(b)) => assert_eq!(*b, data),
            _ => panic!("b64 -> b64db aylanasi buzildi"),
        }
        // Yaroqsiz base64 — aniq xato.
        assert!(matches!(
            crypto_module("b64db", vec![s("bu base64 emas!!!")]),
            Err(Flow::Error(_))
        ));
    }

    // Argument turi noto'g'ri bo'lsa aniq xato (panic emas).
    #[test]
    fn notogri_argument_aniq_xato() {
        assert!(matches!(
            crypto_module("sha256", vec![Value::Int(5)]),
            Err(Flow::Error(_))
        ));
        assert!(matches!(
            crypto_module("hmac", vec![s("kalit")]),
            Err(Flow::Error(_))
        ));
        assert!(matches!(
            crypto_module("yoq_funksiya", vec![]),
            Err(Flow::Error(_))
        ));
    }
}
