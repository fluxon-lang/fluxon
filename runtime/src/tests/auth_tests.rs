use super::*;

// --- auth battery (issue #69) ---
//
// A lock for tests that need the $AUTH_SECRET env — so parallel tests do not
// race on the env (the AI_ENV_LOCK pattern).
static AUTH_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn auth_jwt_verify_roundtrip() {
    let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe { std::env::set_var("AUTH_SECRET", "sirli-kalit-123") };
    run(r#"
use auth
token = auth.jwt {sub:"u1" tenant:"t1" role:"admin"}
# a signed JWT — 3 segments (header.payload.signature)
parts = str.split token "."
(parts.len == 3) | (fail "JWT 3 segment not: ${parts.len}")
# verify -> returns the payload map, claims are preserved
claims = auth.verify token
(claims.sub == "u1") | (fail "sub wrong: ${claims.sub}")
(claims.tenant == "t1") | (fail "tenant wrong: ${claims.tenant}")
(claims.role == "admin") | (fail "role wrong: ${claims.role}")
# iat/exp added automatically
(claims.exp > claims.iat) | (fail "exp should be greater than iat")
"#);
}

#[test]
fn auth_verify_buzilgan_token_xato() {
    let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe { std::env::set_var("AUTH_SECRET", "sirli-kalit-123") };
    // A token with a tampered signature -> auth.verify err (in Fluxon `try` is a
    // passthrough, the error stops the run — so we check with expect_err on the
    // Rust side). Adding a character to the token makes the signature mismatch.
    let err = run_source(
        r#"use auth
token = auth.jwt {sub:"u1"}
auth.verify (token + "x")"#,
    )
    .expect_err("a tampered token should error");
    assert!(
        err.contains("signature"),
        "expected signature error, got: {}",
        err
    );
}

#[test]
fn auth_verify_yaroqsiz_shakl_xato() {
    let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe { std::env::set_var("AUTH_SECRET", "sirli-kalit-123") };
    // Fewer than 3 segments — the JWT format is invalid -> err.
    let err = run_source(
        r#"use auth
auth.verify "faqat.ikki""#,
    )
    .expect_err("an invalid format should error");
    assert!(
        err.contains("format") || err.contains("segment"),
        "expected format error, got: {}",
        err
    );
}

#[test]
fn auth_verify_exp_siz_token_rad_etiladi() {
    let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe { std::env::set_var("AUTH_SECRET", "sirli-kalit-123") };
    // An `exp:nil` payload -> auth.jwt `or_insert` does not override nil,
    // i.e. the token is signed without a numeric `exp`. Even if correctly signed,
    // auth.verify must REJECT it (otherwise it would be valid forever —
    // Codex P2). The key is correct, so this is an exp error, not a signature one.
    let err = run_source(
        r#"use auth
token = auth.jwt {sub:"u1" exp:nil}
auth.verify token"#,
    )
    .expect_err("a token without exp should be rejected");
    assert!(
        err.contains("exp") || err.contains("expir"),
        "expected exp-missing error, got: {}",
        err
    );
}

#[test]
fn auth_secret_yoq_bolsa_aniq_xato() {
    let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved = std::env::var("AUTH_SECRET").ok();
    unsafe { std::env::remove_var("AUTH_SECRET") };
    let err = run_source(
        r#"use auth
token = auth.jwt {sub:"u1"}"#,
    )
    .expect_err("a missing $AUTH_SECRET should error");
    if let Some(v) = saved {
        unsafe { std::env::set_var("AUTH_SECRET", v) };
    }
    assert!(
        err.contains("AUTH_SECRET"),
        "expected AUTH_SECRET error, got: {}",
        err
    );
}

#[test]
fn auth_hash_check_roundtrip() {
    // hash/check do not need the env (no lock required).
    run(r#"
use auth
h = auth.hash "user-parol"
# argon2id PHC string
(str.has h "argon2id") | (fail "argon2id hash not: ${h}")
# correct password -> true
(auth.check "user-parol" h) | (fail "check returned false for correct password")
# wrong password -> false
((auth.check "wrong-password" h) == false) | (fail "check returned true for wrong password")
"#);
}

#[test]
fn auth_noma_lum_funksiya_xato() {
    // auth.foo -> reaches dispatch and gives "no auth.foo" (NOT unknown name).
    let err = run_source(r#"auth.foo "x""#).expect_err("an unknown auth function should error");
    assert!(
        err.contains("auth.foo") && !err.contains("unknown name"),
        "auth should reach dispatch and give a function error, got: {}",
        err
    );
}

#[test]
fn auth_ozgaruvchi_modulni_yopadi() {
    // If `auth` is declared as a variable, it is not a module — a plain map.
    run(r#"
auth = {jwt:"shadowed"}
log "auth.jwt = ${auth.jwt}"
"#);
}
