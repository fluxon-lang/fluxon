use super::*;

// Issue #131: the crypto battery is accessible from Fluxon code — both a call
// with arguments (Call) and the argument-less `crypto.uuid` (Field) work.
#[test]
fn crypto_battery_fluxon_kodidan() {
    run(r#"
h = crypto.sha256 "abc"
(h == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad") | (fail "sha256 broke: ${h}")
sig = crypto.hmac "Jefe" "what do ya want for nothing?"
(sig == "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843") | (fail "hmac broke: ${sig}")
((crypto.b64d (crypto.b64 "hello world")) == "hello world") | (fail "b64 roundtrip broke")
((crypto.hex "abz") == "61627a") | (fail "hex broke")
u = crypto.uuid
((str.len u) == 36) | (fail "uuid uzunligi broke: ${u}")
(u != crypto.uuid) | (fail "uuid takrorlandi")
"#);
}

// Issue #131: crypto.b64d gives a clear error on invalid input (not a panic).
#[test]
fn crypto_b64d_xato_beradi() {
    let err = run_source("crypto.b64d \"this is not base64!!!\"")
        .expect_err("invalid base64 should error");
    assert!(err.contains("base64"), "unexpected error: {}", err);
}

// Issue #131 (review): if the user has declared the name `crypto`
// (e.g. a `use ./crypto` module), it is not the battery — theirs wins. Same
// shadowing behavior as auth/ai, on both the Call and the Field path.
#[test]
fn crypto_lokal_nom_battery_dan_ustun() {
    run(r#"
crypto = {sha256: \s -> "meniki ${s}" uuid: 7}
((crypto.sha256 "x") == "meniki x") | (fail "lokal crypto.sha256 column did not happen")
((crypto.uuid) == 7) | (fail "lokal crypto.uuid column did not happen")
"#);
}
