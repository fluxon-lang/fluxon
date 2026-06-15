use super::*;

// Issue #132: bytes type basics — of/str/len/slice, equality, Display.
#[test]
fn bytes_turi_asoslari() {
    run(r#"
b = bytes.of "hello"
((bytes.len b) == 5) | (fail "bytes.len broke")
((bytes.str b) == "hello") | (fail "bytes.str broke")
(b == (bytes.of "hello")) | (fail "bytes equality broke")
(b != (bytes.of "other")) | (fail "bytes inequality broke")
part = bytes.slice b 0 2
((bytes.str part) == "he") | (fail "bytes.slice broke")
("${b}" == "<bytes 5>") | (fail "bytes interpolation representation broke: ${b}")
"#);
}

// Issue #132: bytes.len measures BYTES, str.len measures CHARACTERS — the
// difference shows in text with diacritics (’ U+2019 = 3 bytes, 1 character).
#[test]
fn bytes_len_bayt_str_len_belgi() {
    run(r#"
s = "o’zbek"
((str.len s) == 6) | (fail "str.len belgi sanashi needed")
((bytes.len (bytes.of s)) == 8) | (fail "bytes.len bayt sanashi needed")
"#);
}

// Issue #132: integration with crypto — b64db binary decoding, bytes inputs
// give the same result as str.
#[test]
fn bytes_crypto_integratsiya() {
    run(r#"
data = crypto.b64db "AP/+iA=="
((bytes.len data) == 4) | (fail "b64db uzunlik broke")
((crypto.b64 data) == "AP/+iA==") | (fail "bytes b64 aylanasi broke")
((crypto.sha256 (bytes.of "abc")) == (crypto.sha256 "abc")) | (fail "sha256 bytes/str differ")
"#);
}

// Issue #132: bytes.str gives a clear error on non-UTF-8 bytes (not silent corruption).
#[test]
fn bytes_str_yaroqsiz_utf8_xato() {
    let err =
        run_source("bytes.str (crypto.b64db \"//4=\")").expect_err("invalid UTF-8 should error");
    assert!(err.contains("UTF-8"), "unexpected error: {}", err);
}

// Issue #132: a binary round-trip with fs — bytes are written, fs.readb returns
// exactly those bytes (the image/PDF scenario).
#[test]
fn bytes_fs_integratsiya() {
    run(r#"
yol = "/tmp/fluxon_bytes_it_" + (rand.str 10) + ".bin"
fs.write yol (crypto.b64db "AP/+iA==")
b = fs.readb yol
((bytes.len b) == 4) | (fail "fs.readb uzunlik broke")
((crypto.b64 b) == "AP/+iA==") | (fail "fs ikkilik aylanasi broke")
fs.del yol
((fs.readb yol) == nil) | (fail "deleted fayl nil should be")
"#);
}

// Issue #132: json.enc encodes bytes as base64 text (without loss).
#[test]
fn bytes_json_enc_base64() {
    run(r#"
b = crypto.b64db "AP/+iA=="
((json.enc {fayl:b}) == "{\"fayl\":\"AP/+iA==\"}") | (fail "json.enc bytes broke")
"#);
}
