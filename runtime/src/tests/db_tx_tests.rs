use super::*;

#[test]
fn db_tx_commit_returns_value() {
    with_db_test("tx_commit", || {
        run(r#"
use db
tbl t
  id serial pk
  n  int
r = db.tx \->
  x = db.ins "t" {n:7}
  ret x
(r.n == 7) | (fail "tx ret valuei n=7")
(db.one "select count(*) c from t").c == 1 | (fail "1 row commit should be")
"#);
    });
}

#[test]
fn db_tx_rollback_on_fail() {
    // fail inside tx -> the whole block rolls back; the error propagates upward
    // and the first (tx-less) ins is preserved, while the ins inside the tx is
    // rolled back. FILE-backed temp DB: persists between two run_source calls (a
    // memory DB is gone when the first Interp drops). The verifying run is a SEPARATE Interp.
    let path = std::env::temp_dir().join("fluxon_tx_rollback_test.db");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard is held.
    unsafe {
        std::env::set_var("DATABASE_URL", format!("sqlite:{}", path.display()));
    }

    let err = run_source(
        r#"
use db
tbl t
  id serial pk
  n  int
db.ins "t" {n:1}
db.tx \->
  db.ins "t" {n:2}
  fail "on purpose"
"#,
    )
    .unwrap_err();
    assert!(
        err.contains("on purpose"),
        "expected fail message, got: {}",
        err
    );

    // A separate (new) Interp/pool — the file DB is preserved. If rollback worked,
    // only the tx-less ins (n:1) remains, the one inside the tx (n:2) does not.
    run_source(
        r#"
use db
tbl t
  id serial pk
  n  int
(db.one "select count(*) c from t").c == 1 | (fail "1 row should remain after rollback needed")
"#,
    )
    .unwrap_or_else(|e| panic!("rollback tekshiruvi: {}", e));

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn db_json_col_cross_process_decode() {
    // Issue #63: a json column must return a map even in a process where `tbl` is
    // NOT declared. Two SEPARATE Interps (= two processes) over one FILE DB: the
    // first writes (with tbl), the second reads without tbl — DB introspection
    // recovers that the column is json and gives a map (before, a raw string came
    // back and row.body.x errored).
    let path = std::env::temp_dir().join("fluxon_json_xproc_test.db");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard is held.
    unsafe {
        std::env::set_var("DATABASE_URL", format!("sqlite:{}", path.display()));
    }

    // Writer process: declares tbl + writes a json map (which also contains a list).
    run_source(
        r#"
use db
tbl t
  k    sym
  body json
db.ins "t" {k::a body:{x:1 y:[1 2 3]}}
"#,
    )
    .unwrap_or_else(|e| panic!("yozish: {}", e));

    // Reader process: NO tbl — only reads. The json must come back as a map.
    run_source(
        r#"
use db
row = db.one "select * from t where k=$1" [:a]
(row.body.x == 1) | (fail "json column should decode as a map (x)")
(row.body.y.len == 3) | (fail "inner json list should also be restored needed (y)")
"#,
    )
    .unwrap_or_else(|e| panic!("read: {}", e));

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn db_json_schema_less_write_to_text_col() {
    // Regression: a process where tbl is NOT declared must be able to write a
    // map/list to a TEXT column. Before, DB introspection returned the TEXT column
    // as Some("text") and the write path errored with "not a json column" — now the
    // write side uses only the tbl registry, so schema-less writes work for a process without tbl.
    //
    // Scenario: the first process creates a `str` (TEXT) column; the second process
    // writes a map with NO tbl — this used to error with "not a json column".
    let path = std::env::temp_dir().join("fluxon_schemaless_write_test.db");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::set_var("DATABASE_URL", format!("sqlite:{}", path.display()));
    }

    // First process: creates a table with a str (TEXT) column and writes one row
    // (db.ins does a lazy DB open + migrate — the table is created right here).
    run_source(
        r#"
use db
tbl t3
  id   serial pk
  body str
db.ins "t3" {body:"init"}
"#,
    )
    .unwrap_or_else(|e| panic!("jadval yaratish: {}", e));

    // Second process: NO tbl — must write a map to the TEXT column (schema-less).
    run_source(
        r#"
use db
db.ins "t3" {body:{x:42 y:[1 2]}}
row = db.one "select body from t3 limit 1"
row.body | (fail "body should not be empty needed")
"#,
    )
    .unwrap_or_else(|e| panic!("schema-less yozish: {}", e));

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn db_tx_nested_savepoint() {
    // Inner tx (SAVEPOINT). The inner block returns a ret value, the outer commits.
    with_db_test("tx_nested", || {
        run(r#"
use db
tbl t
  id serial pk
  n  int
r = db.tx \->
  db.ins "t" {n:1}
  inner = db.tx \->
    x = db.ins "t" {n:2}
    ret x
  ret inner
(r.n == 2) | (fail "nested tx ret valuei n=2")
(db.one "select count(*) c from t").c == 2 | (fail "ikkala ins commit should be")
"#);
    });
}

#[test]
fn db_put_upsert() {
    with_db_test("put_upsert", || {
        run(r#"
use db
tbl counters
  name str pk
  hits int
db.put "counters" {hits:1} {name:"x"}
db.put "counters" {hits:9} {name:"x"}
c = db.one "select * from counters where name=$1" ["x"]
(c.hits == 9) | (fail "upsert hits=9 should be")
n = (db.q "select * from counters").len
(n == 1) | (fail "upsert should not create a duplicate needed")
"#);
    });
}

#[test]
fn db_uniq_violation_rolls_back_tx() {
    // A uniq violation inside tx -> rollback (the idempotency pattern).
    with_db_test("uniq_violation", || {
        let err = run_source(
            r#"
use db
tbl txns
  id   serial pk
  ikey str uniq
db.ins "txns" {ikey:"k1"}
db.tx \->
  db.ins "txns" {ikey:"k1"}
"#,
        )
        .unwrap_err();
        // The uniq violation is raised as a db error.
        assert!(
            err.to_lowercase().contains("unique") || err.contains("db error"),
            "expected uniq violation error, got: {}",
            err
        );
    });
}

// --- cron battery ---
