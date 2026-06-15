use super::*;

// The `ai` tests depend on the env (keys) — we serialize them with a global mutex
// (so other tests do not change the env in parallel). These tests do NOT GO TO THE
// NETWORK: we check that an error is raised BEFORE the API call when the key is missing.
static AI_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn ai_kalit_yoq_bolsa_aniq_xato() {
    let _guard = AI_ENV_LOCK.lock().unwrap();
    // We temporarily remove all key envs (auto-detect must find none). There is no
    // .env in runtime/ -> a clear "key not found" error, no network call. We save
    // the previous values and restore them after the test.
    let saved: Vec<(&str, Option<String>)> = ["AI_KEY", "ANTHROPIC_API_KEY", "OPENAI_API_KEY"]
        .iter()
        .map(|k| (*k, std::env::var(k).ok()))
        .collect();
    for (k, _) in &saved {
        unsafe { std::env::remove_var(k) };
    }
    let err = run_source(r#"x = ai.ask "hello""#).expect_err("a missing key should error");
    // restore the env (so it does not affect other tests).
    for (k, v) in &saved {
        if let Some(val) = v {
            unsafe { std::env::set_var(k, val) };
        }
    }
    assert!(
        err.contains("key not found") || err.contains("key"),
        "expected key-not-found error, got: {}",
        err
    );
}

#[test]
fn ai_noma_lum_funksiya_xato() {
    let _guard = AI_ENV_LOCK.lock().unwrap();
    // ai.foo -> reaches dispatch and gives "no ai.foo" (NOT unknown name).
    // Whether or not a key exists, this comes before checking the function name.
    let err = run_source(r#"ai.foo "x""#).expect_err("an unknown ai function should error");
    assert!(
        err.contains("ai.foo") && !err.contains("unknown name"),
        "ai should reach dispatch and give a function error, got: {}",
        err
    );
}

#[test]
fn ai_ozgaruvchi_modulni_yopadi() {
    // If `ai` is declared as a variable, it is not a module — it is read as a plain
    // map field (unlike http/db, but the ai dispatch lookup checks for it).
    run(r#"
ai = {ask:"shadowed"}
log "ai.ask = ${ai.ask}"
"#);
}

// sh.run -> {stdout stderr code}: the echo output and the success code are correct.
// (Unix-compatible echo, works on CI ubuntu+macOS.)
#[test]
fn sh_run_echo_natija_va_kod() {
    run(r#"
r = sh.run "printf hello"
(r.code == 0) | (fail "code should be 0: ${r.code}")
(r.stdout == "hello") | (fail "stdout wrong: ${r.stdout}")
(r.stderr == "") | (fail "stderr empty should be: ${r.stderr}")
"#);
}

// Non-zero exit -> NOT a Flow::err, it is checked via `code` (the expected result).
#[test]
fn sh_run_nolik_bolmagan_kod_xato_emas() {
    run(r#"
r = sh.run "exit 7"
(r.code == 7) | (fail "code 7 should be: ${r.code}")
"#);
}
