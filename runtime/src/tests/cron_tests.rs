use super::*;

#[test]
fn cron_on_registratsiya_xatosiz() {
    // Unquoted 5-field form (a named function). cron.on does not block, the program ends.
    run(r#"
fn check
  log "check"
cron.on 0 * * * * check
"#);
}

#[test]
fn cron_on_lambda_va_murakkab_ifoda() {
    // Inline lambda + a mixed step/range/list expression.
    run(r#"
cron.on */15 9 1,15 * 1-5 \->
  log "har 15 daqiqa, 9-soat, 1 va 15-kun, ish kunlari"
"#);
}

#[test]
fn cron_on_tirnoqli_variant() {
    // A quoted str also works (for humans; not in the AI docs).
    run(r#"
fn report
  log "report"
cron.on "30 9 * * *" report
"#);
}

#[test]
fn cron_on_notogri_ifoda_xato() {
    // There is no minute 99 — cron.on must return an error.
    let err = run_source(
        r#"
fn f
  log "x"
cron.on 99 * * * * f
"#,
    )
    .expect_err("an invalid cron expression should error");
    assert!(
        err.contains("cron") && err.to_lowercase().contains("expression"),
        "expected cron expression error, got: {}",
        err
    );
}

// --- queue battery ---

#[test]
fn cron_argumentsiz_dispatch_ga_yetadi() {
    // `cron.run` argument-less — arrives as a Field and must reach dispatch
    // (otherwise "unknown name: cron"). cron.run blocks, so instead of an existing
    // function we test, with an unknown function, that it reaches dispatch.
    let err = run_source(r#"cron.yoq"#).expect_err("argument-less cron.yoq should error");
    assert!(
        err.contains("cron module") && !err.contains("unknown name"),
        "argument-less cron should reach dispatch, got: {}",
        err
    );
}
