use super::*;

#[test]
fn migrate_add_column_idempotent() {
    // Adding a new column to tbl -> ADD COLUMN; old rows are preserved;
    // re-deploy is idempotent (does not fail).
    let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = setup_db("fluxon_mig_addcol.db");

    // Deploy 1: a two-column table + one row.
    run_source("use db\ntbl t\n  id serial pk\n  a int\ndb.ins \"t\" {a:1}\n")
        .unwrap_or_else(|e| panic!("deploy1: {}", e));

    // Deploy 2: new column `b` added. It must be an ADD COLUMN, and the old row
    // must be preserved (b NULL).
    run_source(
        r#"
use db
tbl t
  id serial pk
  a  int
  b  str
old = db.one "select * from t where a=1"
(old != nil) | (fail "old row should be preserved needed")
(old.b == nil) | (fail "new column b NULL should be")
db.ins "t" {a:2 b:"hi"}
(db.one "select b from t where a=2").b == "hi" | (fail "write to new column")
"#,
    )
    .unwrap_or_else(|e| panic!("deploy2 add column: {}", e));

    // Deploy 3: the exact same schema — idempotent, does not fail.
    run_source("use db\ntbl t\n  id serial pk\n  a int\n  b str\n")
        .unwrap_or_else(|e| panic!("deploy3 idempotent: {}", e));

    cleanup_db(&path);
}

#[test]
fn migrate_drop_column_with_backup() {
    // Removing a column from tbl -> DROP COLUMN + a _fluxon_bak_* backup table
    // remains with the old data.
    let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = setup_db("fluxon_mig_dropcol.db");

    run_source("use db\ntbl t\n  id serial pk\n  a int\n  b str\ndb.ins \"t\" {a:1 b:\"keep\"}\n")
        .unwrap_or_else(|e| panic!("deploy1: {}", e));

    // Deploy 2: column `b` removed -> DROP COLUMN. A query for `b` errors
    // (column gone), but the backup table keeps `b="keep"`.
    run_source(
        r#"
use db
tbl t
  id serial pk
  a  int
# column b is gone now -> DROP COLUMN
baks = db.q "select name from sqlite_master where type='table' and name like '_fluxon_bak_t_%'"
(baks.len >= 1) | (fail "backup table should be created needed")
"#,
    )
    .unwrap_or_else(|e| panic!("deploy2 drop column: {}", e));

    // Deploy 3: the exact same (b-less) schema — `b` is already gone, DROP COLUMN
    // is attempted on a missing column, but idempotent: silent pass, no failure.
    run_source("use db\ntbl t\n  id serial pk\n  a int\n")
        .unwrap_or_else(|e| panic!("deploy3 drop idempotent: {}", e));

    cleanup_db(&path);
}

#[test]
fn migrate_drop_table_only_fluxon_managed() {
    // If a tbl is removed from the source entirely -> DROP TABLE + backup, but
    // ONLY a Fluxon-created table (in _fluxon_schema). A non-Fluxon table is preserved.
    let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = setup_db("fluxon_mig_droptbl.db");

    // Deploy 1: Fluxon creates table `a` + a manual, non-Fluxon `manual` table.
    run_source(
        r#"
use db
tbl a
  id serial pk
  n  int
db.ins "a" {n:1}
db.q "CREATE TABLE manual (x int)"
db.q "INSERT INTO manual VALUES (42)"
"#,
    )
    .unwrap_or_else(|e| panic!("deploy1: {}", e));

    // Deploy 2: tbl `a` removed (but another tbl exists — the registry is NOT
    // empty). `a` must be DROPped, `manual` must be preserved.
    run_source(
        r#"
use db
tbl b
  id serial pk
gone = db.q "select name from sqlite_master where type='table' and name='a'"
(gone.len == 0) | (fail "a tablei DROP should be")
kept = db.q "select name from sqlite_master where type='table' and name='manual'"
(kept.len == 1) | (fail "manual table should be preserved needed (not created by Fluxon)")
(db.one "select x from manual").x == 42 | (fail "manual data should be preserved needed")
baks = db.q "select name from sqlite_master where type='table' and name like '_fluxon_bak_a_%'"
(baks.len >= 1) | (fail "backup for a should be created needed")
"#,
    )
    .unwrap_or_else(|e| panic!("deploy2 drop table: {}", e));

    cleanup_db(&path);
}

#[test]
fn migrate_index_create_and_drop() {
    // An index declaration -> CREATE INDEX; removing it -> DROP INDEX. uniq(a b) ->
    // a duplicate insert errors. sqlite_autoindex_* is left untouched.
    let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = setup_db("fluxon_mig_index.db");

    // Deploy 1: a single index + a multi-column unique.
    run_source(
            r#"
use db
tbl bookings
  id          serial pk
  resource_id int
  status      sym index
  start_at    str
  uniq(resource_id start_at)
idx = db.q "select name from sqlite_master where type='index' and name='idx_bookings_status'"
(idx.len == 1) | (fail "idx_bookings_status yaratilishi needed")
uniq = db.q "select name from sqlite_master where type='index' and name='uniq_bookings_resource_id_start_at'"
(uniq.len == 1) | (fail "uniq index should be created needed")
db.ins "bookings" {resource_id:5 status::done start_at:"2026-06-01"}
"#,
        )
        .unwrap_or_else(|e| panic!("deploy1 index: {}", e));

    // uniq violation: the same (resource_id start_at) -> error.
    let dup = run_source(
        r#"
use db
tbl bookings
  id          serial pk
  resource_id int
  status      sym index
  start_at    str
  uniq(resource_id start_at)
db.ins "bookings" {resource_id:5 status::pending start_at:"2026-06-01"}
"#,
    );
    assert!(
        dup.is_err(),
        "uniq(resource_id start_at) duplicate insert should error"
    );

    // Deploy 2: the status index removed -> DROP INDEX. uniq stays.
    run_source(
            r#"
use db
tbl bookings
  id          serial pk
  resource_id int
  status      sym
  start_at    str
  uniq(resource_id start_at)
dropped = db.q "select name from sqlite_master where type='index' and name='idx_bookings_status'"
(dropped.len == 0) | (fail "idx_bookings_status DROP should be")
kept = db.q "select name from sqlite_master where type='index' and name='uniq_bookings_resource_id_start_at'"
(kept.len == 1) | (fail "uniq index should be preserved needed")
"#,
        )
        .unwrap_or_else(|e| panic!("deploy2 drop index: {}", e));

    cleanup_db(&path);
}

#[test]
fn migrate_drop_indexed_column() {
    // REGRESSION (code review): when an indexed column is removed, the stale
    // index must be dropped BEFORE the column DROP — otherwise in some SQLite
    // states DROP COLUMN is rejected with "error in index ... no such column"
    // and the deploy cannot migrate. Both single and composite index.
    let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = setup_db("fluxon_mig_dropidxcol.db");

    // Deploy 1: an indexed `status` column + a composite index(a status).
    run_source(
        r#"
use db
tbl t
  id     serial pk
  a      int
  status sym index
  index(a status)
db.ins "t" {a:1 status::x}
"#,
    )
    .unwrap_or_else(|e| panic!("deploy1: {}", e));

    // Deploy 2: column `status` removed. The old idx_t_status and idx_t_a_status
    // are still in the DB — the migration must not fail (the stale index is
    // dropped first), then DROP COLUMN must work.
    run_source(
        r#"
use db
tbl t
  id serial pk
  a  int
gone = db.q "select name from sqlite_master where type='index' and name='idx_t_status'"
(gone.len == 0) | (fail "idx_t_status DROP should be")
comp = db.q "select name from sqlite_master where type='index' and name='idx_t_a_status'"
(comp.len == 0) | (fail "idx_t_a_status (depending on status) DROP should be")
# the status column is really gone
cols = db.q "select name from pragma_table_info('t') where name='status'"
(cols.len == 0) | (fail "status columni DROP should be")
"#,
    )
    .unwrap_or_else(|e| panic!("deploy2 drop indexed column: {}", e));

    cleanup_db(&path);
}

#[test]
fn migrate_pipe_modifier_creates_unique_index() {
    // `email str index|uniq` -> a single UNIQUE index is created (uniq subsumes
    // it), a duplicate insert errors.
    let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = setup_db("fluxon_mig_pipe.db");

    run_source(
        r#"
use db
tbl users
  id    serial pk
  email str index|uniq
ui = db.q "select name from sqlite_master where type='index' and name='uniq_users_email'"
(ui.len == 1) | (fail "uniq_users_email should be created needed")
db.ins "users" {email:"a@x.uz"}
"#,
    )
    .unwrap_or_else(|e| panic!("deploy1 pipe: {}", e));

    let dup = run_source(
        r#"
use db
tbl users
  id    serial pk
  email str index|uniq
db.ins "users" {email:"a@x.uz"}
"#,
    );
    assert!(dup.is_err(), "index|uniq duplicate email should error");

    cleanup_db(&path);
}

#[test]
fn migrate_multi_column_uniq_constraint() {
    // Issue #94: `uniq(a, b)` (comma-separated) creates a multi-column UNIQUE
    // constraint — NOT a fake "uniq" column. A duplicate (a,b) pair errors.
    with_db_test("multi_uniq", || {
        // 1. No fake `uniq` column: the table must contain only a, b.
        run(r#"
use db
tbl t
  a str
  b str
  uniq(a, b)
n = (db.q "select count(*) c from pragma_table_info('t')").0.c
(n == 2) | (fail "table should have only 2 columns (a, b) — no phantom uniq column")
ui = db.q "select name from sqlite_master where type='index' and name='uniq_t_a_b'"
(ui.len == 1) | (fail "uniq_t_a_b unique index should be created")
db.ins "t" {a:"x" b:"y"}
"#);

        // 2. A duplicate (a, b) pair violates the UNIQUE constraint. Both inserts
        //    are in one source — so the shared-memory db is not lost between runs.
        let dup = run_source(
            r#"
use db
tbl t
  a str
  b str
  uniq(a, b)
db.ins "t" {a:"x" b:"y"}
db.ins "t" {a:"x" b:"y"}
"#,
        );
        assert!(dup.is_err(), "duplicate (a, b) should violate uniq");
    });
}

#[test]
fn fk_ref_modifier_enforced() {
    // Issue #94 (related): the `ref:tbl.col` FK modifier is now enforced —
    // an insert referencing a non-existent parent row errors.
    with_db_test("fk_ref", || {
        // Valid FK: the parent row exists — the insert passes.
        run(r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int ref:users.id
  title str
db.ins "users" {name:"ali"}
p = db.ins "posts" {owner:1 title:"hello"}
(p.id == 1) | (fail "valid FK insert should pass needed")
"#);

        // Orphan FK: owner=999 does not exist -> FOREIGN KEY constraint failed.
        let orphan = run_source(
            r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int ref:users.id
  title str
db.ins "posts" {owner:999 title:"orphan"}
"#,
        );
        assert!(orphan.is_err(), "orphan FK insert should error");
    });
}

#[test]
fn migrate_adds_fk_to_existing_column_via_rebuild() {
    // Issue #94 (codex review): FK must apply not only to a NEW table — also to
    // an existing column in an EXISTING table. The old state (DB introspection) is
    // compared with the declaration, and on a difference the table is rebuilt. Data
    // is preserved, autoincrement continues, FK is enforced.
    let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = setup_db("fluxon_fk_rebuild.db");

    // Deploy 1: posts without an FK, with data.
    run_source(
        r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int
  title str
db.ins "users" {name:"ali"}
db.ins "posts" {owner:1 title:"a"}
db.ins "posts" {owner:1 title:"b"}
"#,
    )
    .unwrap_or_else(|e| panic!("deploy1: {}", e));

    // Deploy 2: ref:users.id added to the existing `owner` column -> rebuild.
    run_source(
        r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int ref:users.id
  title str
rows = db.q "select count(*) c from posts"
(rows.0.c == 2) | (fail "rebuild should preserve data needed (2 row)")
fk = db.q "select count(*) c from pragma_foreign_key_list('posts')"
(fk.0.c == 1) | (fail "posts should have FK after rebuild")
n = db.ins "posts" {owner:1 title:"c"}
(n.id == 3) | (fail "autoincrement should continue needed (id=3)")
"#,
    )
    .unwrap_or_else(|e| panic!("deploy2 rebuild: {}", e));

    // Now an orphan insert is rejected (FK enforced).
    let orphan = run_source(
        r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int ref:users.id
  title str
db.ins "posts" {owner:404 title:"orphan"}
"#,
    );
    assert!(
        orphan.is_err(),
        "orphan FK insert should error after rebuild"
    );

    cleanup_db(&path);
}

#[test]
fn migrate_drop_column_and_add_fk_same_deploy() {
    // Codex review: if one migration both DROPs a column and adds a ref to an
    // existing column — the DROP COLUMN backup (`_fluxon_bak_<t>_<ts>`) and the
    // rebuild backup must NOT COLLIDE in name (rebuild uses a `_fk` suffix).
    let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = setup_db("fluxon_drop_and_fk.db");

    // Deploy 1: an `old` column exists, no ref.
    run_source(
        r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int
  title str
  old   str
db.ins "users" {name:"a"}
db.ins "posts" {owner:1 title:"x" old:"old"}
"#,
    )
    .unwrap_or_else(|e| panic!("deploy1 drop+fk: {}", e));

    // Deploy 2: DROP `old` + add a ref to `owner` (one migration).
    run_source(
        r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int ref:users.id
  title str
n = db.q "select count(*) c from posts"
(n.0.c == 1) | (fail "data should be preserved needed (1 row)")
fk = db.q "select count(*) c from pragma_foreign_key_list('posts')"
(fk.0.c == 1) | (fail "FK should be added needed")
cols = db.q "select count(*) c from pragma_table_info('posts')"
(cols.0.c == 3) | (fail "old column DROPped, 3 columns should remain needed")
"#,
    )
    .unwrap_or_else(|e| panic!("deploy2 drop+fk (backup collision?): {}", e));

    cleanup_db(&path);
}

#[test]
fn migrate_fk_rebuild_aborts_on_orphan_data() {
    // If existing data has an orphan row, the FK-adding rebuild does NOT silently
    // lose it — it gives a clear error and the data stays intact via ROLLBACK.
    let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = setup_db("fluxon_fk_orphan.db");

    run_source(
        r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int
  title str
db.ins "users" {name:"a"}
db.ins "posts" {owner:1 title:"ok"}
db.ins "posts" {owner:777 title:"orphan"}
"#,
    )
    .unwrap_or_else(|e| panic!("deploy1 orphan: {}", e));

    // adding a ref -> the orphan row violates the FK -> migrate errors (rebuild aborts).
    let res = run_source(
        r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int ref:users.id
  title str
db.q "select 1 x"
"#,
    );
    assert!(res.is_err(), "FK rebuild should abort on orphan data");

    // The data and the old (FK-less) schema must be preserved.
    run_source(
        r#"
use db
n = db.q "select count(*) c from posts"
(n.0.c == 2) | (fail "rollback should preserve data needed (2 row)")
fk = db.q "select count(*) c from pragma_foreign_key_list('posts')"
(fk.0.c == 0) | (fail "FK should not be added after abort needed")
"#,
    )
    .unwrap_or_else(|e| panic!("verify orphan: {}", e));

    cleanup_db(&path);
}
