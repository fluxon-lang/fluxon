use super::*;

// Issue #139: leveled log — `log.debug/info/warn/err` and bare `log` (=info)
// are wired into dispatch and work without error (writes to stderr, returns nil).
// Filter/format via $LOG_LEVEL/$LOG_FORMAT; the format logic is unit-tested in
// builtins::log_tests. Here only syntax/dispatch is checked.
#[test]
fn log_darajalari() {
    run(r#"
log "bare = info"
log.debug "tafsilot"
log.info "info"
log.warn "ogohlantirish"
log.err "error"
log.info "interpolatsiya ${1 + 1}"
"#);
}

// Issue #139: `log` continues to work as a value (callback/storage) —
// compatible with the old global `log` Native (an info-level shim). PR #163 review.
#[test]
fn log_qiymat_sifatida_callback() {
    run(r#"
fn call f -> f "by value"
call log
[1, 2, 3].map log
g = log
g "saved function"
"#);
}

// Issue #139: if the user declares a variable named `log`, it wins
// (shadows the battery) — the old shadowing invariant is not broken.
#[test]
fn log_ozgaruvchi_sifatida_shadow() {
    run(r#"
fn log_id v -> v
log = (log_id 42)
(log == 42) | (fail "log did not shadow as a variable")
"#);
}
