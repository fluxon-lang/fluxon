use super::*;

#[test]
fn queue_on_push_registratsiya_xatosiz() {
    // queue.on registers a handler, queue.push adds a job — neither blocks, the
    // program ends (the worker keeps running in the background). The handler takes
    // a single `job` map argument.
    run(r#"
queue.on "send" \job ->
  log "sending: ${job.ph}"
queue.push "send" {ph:"+99890" body:"hello"}
"#);
}

#[test]
fn queue_push_payloadsiz() {
    // Payload is optional — if omitted, job is Nil.
    run(r#"
queue.on "tozala" \job ->
  log "cleaned"
queue.push "tozala"
"#);
}

#[test]
fn queue_handlersiz_push_dastur_tugaydi() {
    // Issue #105: a job whose handler is never registered must not block the
    // program from exiting — run() ends normally with a warning (in the old
    // busy-loop the job spun forever).
    run(r#"queue.push "orphan" {x:1}"#);
}

#[test]
fn queue_drain_handler_haqiqatan_ishlaydi() {
    // Issue #105: the queue is drained before run() returns — that the handler
    // actually ran is checked via the DB without a RACE (before, you could not
    // guarantee the worker background thread had finished).
    let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = setup_db("fluxon_queue_drain.db");

    run(r#"
use db
tbl jobs
  id  serial pk
  nom str
queue.on "yoz" \job ->
  db.ins "jobs" {nom:job.nom}
queue.push "yoz" {nom:"a"}
queue.push "yoz" {nom:"b"}
"#);

    // The first run() ended with a drain — both jobs MUST be in the DB.
    run(r#"
use db
((db.q "select * from jobs").len == 2) | (fail "queue jobs were not executed")
"#);

    cleanup_db(&path);
}

#[test]
fn queue_push_nom_str_bolmasa_xato() {
    // The 1st argument, the job name, must be a str.
    let err = run_source(r#"queue.push 5"#).expect_err("a non-str name should error");
    assert!(
        err.contains("queue.push"),
        "expected queue.push error, got: {}",
        err
    );
}

#[test]
fn queue_argumentsiz_dispatch_ga_yetadi() {
    // An argument-less `queue.X` (it arrives as a Field, not a Call) must reach
    // module dispatch — so the `queue` ident is not looked up as a variable and
    // does not give "unknown name". We test with an unknown function: if it reaches
    // dispatch, a "no ... in queue module" error comes (NOT unknown name). [cron.run regression]
    let err = run_source(r#"queue.yoq"#).expect_err("argument-less queue.yoq should error");
    assert!(
        err.contains("queue module") && !err.contains("unknown name"),
        "argument-less queue should reach dispatch, got: {}",
        err
    );
}

#[test]
fn queue_on_handler_fn_bolmasa_xato() {
    // The 2nd argument, the handler, must be an fn.
    let err = run_source(r#"queue.on "send" 5"#).expect_err("a non-fn handler should error");
    assert!(
        err.contains("queue.on"),
        "expected queue.on error, got: {}",
        err
    );
}
