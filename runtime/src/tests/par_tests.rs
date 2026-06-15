use super::*;

// Issue #137: par — language-level parallel fan-out. Takes a list of lambdas,
// calls each one on its own thread, waits for all, results (in input order) are
// each {ok:...} or {err:...}.
// Note: lambda elements inside a list are separated by PARENS — `(\-> ...)`.
// The lexer does not emit a Newline token inside a list/map (`paren_depth>0`), so
// without parens `[\-> a  \-> b]` the first body would swallow the second as an
// argument; the parens delimit the body and a nested HOF (`\-> xs.map \x ->`)
// is not broken either (issue #137 PR review, P2).
#[test]
fn par_asosiy_fan_out() {
    run(r#"
r = par [
  (\-> 1 + 1)
  (\-> str.up "hi")
  (\-> [1 2 3].len)
]
((r.len) == 3) | (fail "par 3 results should be returned")
((r.0.ok) == 2) | (fail "1st result should be {ok:2}")
((r.1.ok) == "HI") | (fail "2nd result should be {ok:HI}")
((r.2.ok) == 3) | (fail "3rd result should be {ok:3}")
"#);
}

// Issue #137: partial success — if one lambda fails the others do not stop;
// the error comes back as {err:message}, order is preserved.
#[test]
fn par_qisman_muvaffaqiyat() {
    run(r#"
r = par [
  (\-> 42)
  (\-> fail "on purpose")
  (\-> "third")
]
((r.0.ok) == 42) | (fail "1st result ok should be set")
((r.1.err) == "on purpose") | (fail "2nd result should be err")
((r.2.ok) == "third") | (fail "3rd result ok should be set")
"#);
}

// Issue #137: a closure can read an outer (loop/scope) variable in parallel.
#[test]
fn par_closure_capture() {
    run(r#"
base = 100
r = par [(\-> base + 1) (\-> base + 2)]
((r.0.ok) == 101) | (fail "closure capture 1 broke")
((r.1.ok) == 102) | (fail "closure capture 2 broke")
"#);
}

// Issue #137: a nested paren-free HOF inside a lambda body (`xs.map \x -> ...`)
// is read in full inside the parens — no P2 regression.
#[test]
fn par_nested_hof() {
    run(r#"
r = par [(\-> [1 2 3].map \x -> x + 1)]
((r.0.ok.0) == 2) | (fail "nested HOF 1-element broke")
((r.0.ok.2) == 4) | (fail "nested HOF 3-element broke")
"#);
}

// Issue #137: empty list -> empty result (no thread is spawned).
#[test]
fn par_bosh_royxat() {
    run(r#"
r = par []
((r.len) == 0) | (fail "par [] should return an empty list")
"#);
}

// Issue #137: a non-lambda element gives a clear error (without spawning a thread).
#[test]
fn par_lambda_bolmagan_element_xato() {
    let e = run_source("par [42]").unwrap_err();
    assert!(
        e.contains("must be a function"),
        "a clear error is expected for a non-lambda par element, got: {}",
        e
    );
}

// Issue #137 (PR review P2): if two par lambdas `use ./m` the same UNCACHED
// module in parallel, both must return {ok:...} — not a false "circular import".
// Because module_loading/current_base are thread-local, parallel imports do not
// see each other as a cycle and the base is not corrupted.
#[test]
fn par_parallel_modul_import_soxta_sikl_yoq() {
    let dir = std::env::temp_dir().join(format!("fluxon_par_mod_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("m.fx"), "exp fn greet n -> \"hello ${n}\"\n").unwrap();
    let main = dir.join("main.fx");
    // Each lambda imports the MODULE FOR THE FIRST TIME on its own thread
    // (cache empty) — Codex reproduction.
    std::fs::write(
        &main,
        r#"
fn load n
  use ./m
  ret m.greet n
r = par [
  (\-> load 1)
  (\-> load 2)
]
((r.0.ok) == "hello 1") | (fail "par module import 1 broke: ${r.0}")
((r.1.ok) == "hello 2") | (fail "par module import 2 broke: ${r.1}")
"#,
    )
    .unwrap();
    let src = std::fs::read_to_string(&main).unwrap();
    let res = run_source_at(&src, &main);
    let _ = std::fs::remove_dir_all(&dir);
    res.unwrap_or_else(|e| panic!("par parallel module import error: {}", e));
}

// Issue #137: if the user declares a variable named `par`, it wins
// (shadowing consistent with the other dispatch batteries).
#[test]
fn par_ozgaruvchi_sifatida_shadow() {
    run(r#"
fn id v -> v
par = (id 7)
(par == 7) | (fail "par did not shadow as a variable")
"#);
}

// Issue #137 (PR review P1): calling par from inside db.tx gives a clear error —
// the new threads do not inherit the CURRENT_TX TLS, so instead of silently
// running outside the tx, it is rejected. (DB test — DB_TEST_LOCK.)
#[test]
fn par_db_tx_ichida_rad_etiladi() {
    with_db_test("par_in_tx", || {
        let e = run_source(
            r#"
use db
tbl t
  id serial pk
db.tx \->
  par [(\-> 1)]
"#,
        )
        .unwrap_err();
        assert!(
            e.contains("cannot be used inside db.tx"),
            "a clear error is expected for par inside db.tx, got: {}",
            e
        );
    });
}
