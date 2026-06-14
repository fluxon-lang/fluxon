// Fluxon DB battery — db.q/one/ins/up/del/put and db.tx.
//
// ARCHITECTURE: the backend is hidden behind the `Db` trait. Fluxon code (`db.*`)
// never changes; the backend is swapped from a single config point (the
// `$DATABASE_URL` scheme). Today it is fully SQLite (rusqlite, bundled — no server
// needed); postgres/mysql connect additively later (currently an `Err` stub).
//
// Dialect differences (placeholder style, RETURNING, ON CONFLICT, identifier
// quoting, BEGIN/SAVEPOINT syntax) live inside the trait. SQLite supports the `$1`
// placeholder natively, so Fluxon's spec-mandated `$1` style passes through without
// a rewrite.
//
// Transactions are driven manually via BEGIN/COMMIT/ROLLBACK/SAVEPOINT SQL (instead
// of rusqlite's lifetime-bound Transaction type) — that way the tx owns the
// connection (`'static`) and can live in a thread_local context.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use rusqlite::types::{Value as RqVal, ValueRef};
use rusqlite::{Connection, params_from_iter};

use crate::builtins::{json_decode, json_encode};
use crate::interp::{Flow, Interp};
use crate::value::Value;

// --- backend-neutral cell value ---

#[derive(Clone, Debug)]
pub enum SqlVal {
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
    Null,
}

pub type Row = BTreeMap<String, SqlVal>;

// --- tbl column definition (for CREATE TABLE generation) ---

#[derive(Clone)]
pub struct ColDef {
    pub name: String,
    pub type_name: String,
    pub modifiers: Vec<String>,
}

// A required index definition (declared in tbl) — for CREATE INDEX generation.
#[derive(Clone)]
pub struct IndexDef {
    pub table: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

// Existing Fluxon index info in the DB — for the diff (drop). The unique flag is
// encoded in the name (`idx_` vs `uniq_` prefix), so the name alone is enough.
pub struct IndexInfo {
    pub name: String,
}

// FOREIGN KEY constraint: `from` column -> `table`.`to`. The `ref:tbl.col`
// declaration and the actual DB state (pragma_foreign_key_list) are compared in this
// shape — on a difference, migration rebuilds the table (FK cannot be added via ALTER).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ForeignKey {
    pub from: String,
    pub table: String,
    pub to: String,
}

// Extracts a column's FK from the `ref:tbl.col` modifier. None if no modifier.
pub fn coldef_foreign_key(c: &ColDef) -> Option<ForeignKey> {
    column_ref_target(&c.modifiers).map(|(table, to)| ForeignKey {
        from: c.name.clone(),
        table: table.to_string(),
        to: to.to_string(),
    })
}

// --- Db trait: dialect-neutral backend interface ---

pub trait Db: Send + Sync {
    // SELECT-style query; result rows (maps).
    fn query(&self, sql: &str, params: &[SqlVal]) -> Result<Vec<Row>, String>;
    // An operation that returns no rows (up/del); number of affected rows.
    fn exec(&self, sql: &str, params: &[SqlVal]) -> Result<usize, String>;
    // An operation that returns rows (ins/put) — via RETURNING *.
    fn query_returning(&self, sql: &str, params: &[SqlVal]) -> Result<Vec<Row>, String>;

    // --- dialect-specific SQL generation ---
    fn build_insert(&self, table: &str, cols: &[String]) -> String;
    fn build_update(&self, table: &str, set: &[String], whr: &[String]) -> String;
    fn build_delete(&self, table: &str, whr: &[String]) -> String;
    fn build_upsert(&self, table: &str, set: &[String], key: &[String]) -> String;
    fn build_create_table(&self, table: &str, cols: &[ColDef]) -> String;

    // List of a table's columns (name, fluxon-type) — introspected from the DB
    // schema. A process that did not declare `tbl` finds json columns this way
    // (issue #63). Only json is reliably reconstructed (sym/bool are TEXT/INTEGER in
    // SQLite and not textually distinguishable). Empty list if the table is missing.
    fn column_types(&self, table: &str) -> Result<Vec<(String, String)>, String>;

    // Fluxon-managed indexes (per table): name + unique flag. Only `idx_`/`uniq_`
    // prefixed, user-created (origin='c') indexes — auto-migration diffs these. Does
    // NOT touch `sqlite_autoindex_*`/UNIQUE-constraint/pk indexes.
    fn fluxon_indexes(&self, table: &str) -> Result<Vec<IndexInfo>, String>;

    // Table names created by Fluxon, from the `_fluxon_schema` meta-table. DROP TABLE
    // applies only to tables in this list (a non-Fluxon table is preserved).
    fn fluxon_tables(&self) -> Result<Vec<String>, String>;

    // Existing FOREIGN KEY constraints on a table (introspection). Migration compares
    // these with the `ref:tbl.col` declaration — on a difference, rebuild.
    fn foreign_keys(&self, table: &str) -> Result<Vec<ForeignKey>, String>;

    // Fully rebuilds a table (preserving data, into the new schema + FK). FK cannot
    // be added via ALTER — this is called when an existing column needs an FK.
    fn rebuild_table(
        &self,
        table: &str,
        cols: &[ColDef],
        indexes: &[IndexDef],
        ts: u64,
    ) -> Result<(), String>;

    // Opens a transaction — returns a `'static` object that owns the connection.
    fn begin(&self) -> Result<Box<dyn DbTx>, String>;
}

// An active transaction — all db.* calls run on this single connection.
pub trait DbTx: Send {
    fn query(&self, sql: &str, params: &[SqlVal]) -> Result<Vec<Row>, String>;
    fn exec(&self, sql: &str, params: &[SqlVal]) -> Result<usize, String>;
    fn query_returning(&self, sql: &str, params: &[SqlVal]) -> Result<Vec<Row>, String>;
    // Nested tx: via SAVEPOINT.
    fn savepoint(&self, name: &str) -> Result<(), String>;
    fn release(&self, name: &str) -> Result<(), String>; // inner commit
    fn rollback_to(&self, name: &str) -> Result<(), String>; // inner rollback
    fn commit(self: Box<Self>) -> Result<(), String>;
    fn rollback(self: Box<Self>) -> Result<(), String>;
    // Introspects column types via the tx connection — used instead of the global
    // pool so uncommitted DDL is visible (issue #63).
    fn column_types(&self, table: &str) -> Result<Vec<(String, String)>, String>;
}

// ==================== SQLite backend ====================

// Connection pool — holds several connections. Tx-less operations (q/one/ins/
// up/del/put) CHECK OUT a connection from the pool and return it IMMEDIATELY; a tx
// holds the connection until commit/rollback. So even when ONE request is inside a
// tx, another PARALLEL request finds a global connection — no "connection busy"
// problem (user-approved design: each tx gets a separate connection).
//
// For `:memory:`, so that each connection does not end up as a SEPARATE empty DB, we
// use `file::memory:?cache=shared` and keep one "keepalive" connection open (the
// shared-cache DB is dropped when the last connection closes).
struct Pool {
    spec: String,               // the open specification passed to rusqlite
    flags: rusqlite::OpenFlags, // URI mode (shared-cache) when needed
    idle: Mutex<Vec<Connection>>,
    // Keeps the :memory: shared-cache DB alive. Mutex — Connection is not Sync,
    // but Pool (inside Arc<dyn Db>) must be Sync.
    _keepalive: Mutex<Option<Connection>>,
}

impl Pool {
    // Checks out a connection from the pool (opens a new one if none are idle).
    fn checkout(&self) -> Result<Connection, String> {
        if let Some(c) = self.idle.lock().unwrap().pop() {
            return Ok(c);
        }
        self.open_one()
    }
    // Returns a connection to the pool.
    fn checkin(&self, conn: Connection) {
        self.idle.lock().unwrap().push(conn);
    }
    fn open_one(&self) -> Result<Connection, String> {
        let conn = Connection::open_with_flags(&self.spec, self.flags)
            .map_err(|e| format!("sqlite could not be opened ({}): {}", self.spec, e))?;
        // On every connection: WAL, FK, busy_timeout.
        let _ = conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
        );
        Ok(conn)
    }
}

pub struct SqliteDb {
    pool: Arc<Pool>,
}

impl SqliteDb {
    // `rest` — the part of DATABASE_URL after `sqlite:`: a file path or `:memory:`.
    pub fn open(rest: &str) -> Result<Self, String> {
        let is_mem = rest.is_empty() || rest == ":memory:" || rest == "memory:";
        // :memory: -> shared-cache URI (all connections see one DB).
        let (spec, flags) = if is_mem {
            (
                "file::memory:?cache=shared".to_string(),
                rusqlite::OpenFlags::default() | rusqlite::OpenFlags::SQLITE_OPEN_URI,
            )
        } else if rest.starts_with("file:") {
            (
                rest.to_string(),
                rusqlite::OpenFlags::default() | rusqlite::OpenFlags::SQLITE_OPEN_URI,
            )
        } else {
            (rest.to_string(), rusqlite::OpenFlags::default())
        };

        let pool = Pool {
            spec,
            flags,
            idle: Mutex::new(Vec::new()),
            _keepalive: Mutex::new(None),
        };
        // :memory: -> keepalive (so the shared DB is not dropped when the last
        // connection closes).
        if is_mem {
            *pool._keepalive.lock().unwrap() = Some(pool.open_one()?);
        }
        // Open one connection ahead of time and leave it in the pool (so an open
        // error is detected here).
        let first = pool.open_one()?;
        pool.idle.lock().unwrap().push(first);

        Ok(SqliteDb {
            pool: Arc::new(pool),
        })
    }
}

// SqlVal -> rusqlite bind value.
fn to_rqval(v: &SqlVal) -> RqVal {
    match v {
        SqlVal::Int(n) => RqVal::Integer(*n),
        SqlVal::Real(x) => RqVal::Real(*x),
        SqlVal::Text(s) => RqVal::Text(s.clone()),
        SqlVal::Blob(b) => RqVal::Blob(b.clone()),
        SqlVal::Null => RqVal::Null,
    }
}

// rusqlite ValueRef -> SqlVal (when reading).
fn from_ref(r: ValueRef<'_>) -> SqlVal {
    match r {
        ValueRef::Null => SqlVal::Null,
        ValueRef::Integer(n) => SqlVal::Int(n),
        ValueRef::Real(x) => SqlVal::Real(x),
        ValueRef::Text(t) => SqlVal::Text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => SqlVal::Blob(b.to_vec()),
    }
}

// Reads all rows from a single prepared statement as maps.
fn run_query(conn: &Connection, sql: &str, params: &[SqlVal]) -> Result<Vec<Row>, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| sql_err(sql, e))?;
    let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let binds: Vec<RqVal> = params.iter().map(to_rqval).collect();
    let mut rows = stmt
        .query(params_from_iter(binds.iter()))
        .map_err(|e| sql_err(sql, e))?;

    let mut out = Vec::new();
    loop {
        match rows.next() {
            Ok(Some(row)) => {
                let mut m = BTreeMap::new();
                for (i, name) in col_names.iter().enumerate() {
                    let vref = row.get_ref(i).map_err(|e| sql_err(sql, e))?;
                    m.insert(name.clone(), from_ref(vref));
                }
                out.push(m);
            }
            Ok(None) => break,
            Err(e) => return Err(sql_err(sql, e)),
        }
    }
    Ok(out)
}

fn run_exec(conn: &Connection, sql: &str, params: &[SqlVal]) -> Result<usize, String> {
    let binds: Vec<RqVal> = params.iter().map(to_rqval).collect();
    conn.execute(sql, params_from_iter(binds.iter()))
        .map_err(|e| sql_err(sql, e))
}

fn sql_err(sql: &str, e: rusqlite::Error) -> String {
    format!("db error: {} (query: {})", e, sql.trim())
}

impl Db for SqliteDb {
    fn query(&self, sql: &str, params: &[SqlVal]) -> Result<Vec<Row>, String> {
        // Check out a connection from the pool, use it, return it immediately — no
        // other parallel query (or tx) keeps the global one busy.
        let conn = self.pool.checkout()?;
        let r = run_query(&conn, sql, params);
        self.pool.checkin(conn);
        r
    }
    fn exec(&self, sql: &str, params: &[SqlVal]) -> Result<usize, String> {
        let conn = self.pool.checkout()?;
        let r = run_exec(&conn, sql, params);
        self.pool.checkin(conn);
        r
    }
    fn query_returning(&self, sql: &str, params: &[SqlVal]) -> Result<Vec<Row>, String> {
        // In SQLite, RETURNING is read like an ordinary query.
        self.query(sql, params)
    }

    fn column_types(&self, table: &str) -> Result<Vec<(String, String)>, String> {
        let conn = self.pool.checkout()?;
        let r = sqlite_column_types(&conn, table);
        self.pool.checkin(conn);
        r
    }

    fn fluxon_indexes(&self, table: &str) -> Result<Vec<IndexInfo>, String> {
        let conn = self.pool.checkout()?;
        let r = sqlite_fluxon_indexes(&conn, table);
        self.pool.checkin(conn);
        r
    }

    fn fluxon_tables(&self) -> Result<Vec<String>, String> {
        let conn = self.pool.checkout()?;
        let r = (|| {
            let mut stmt = conn
                .prepare("SELECT table_name FROM _fluxon_schema")
                .map_err(|e| sql_err("_fluxon_schema read", e))?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| sql_err("_fluxon_schema read", e))?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.map_err(|e| sql_err("_fluxon_schema read", e))?);
            }
            Ok(out)
        })();
        self.pool.checkin(conn);
        r
    }

    fn foreign_keys(&self, table: &str) -> Result<Vec<ForeignKey>, String> {
        let conn = self.pool.checkout()?;
        let r = sqlite_foreign_keys(&conn, table);
        self.pool.checkin(conn);
        r
    }

    fn rebuild_table(
        &self,
        table: &str,
        cols: &[ColDef],
        indexes: &[IndexDef],
        ts: u64,
    ) -> Result<(), String> {
        let conn = self.pool.checkout()?;
        let r = sqlite_rebuild_table(&conn, table, cols, indexes, ts);
        self.pool.checkin(conn);
        r
    }

    fn build_insert(&self, table: &str, cols: &[String]) -> String {
        let collist = cols
            .iter()
            .map(|c| q_ident(c))
            .collect::<Vec<_>>()
            .join(",");
        let places = (1..=cols.len())
            .map(|i| format!("${i}"))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "INSERT INTO {} ({}) VALUES ({}) RETURNING *",
            q_ident(table),
            collist,
            places
        )
    }

    fn build_update(&self, table: &str, set: &[String], whr: &[String]) -> String {
        let mut idx = 0;
        let set_clause = set
            .iter()
            .map(|c| {
                idx += 1;
                format!("{}=${}", q_ident(c), idx)
            })
            .collect::<Vec<_>>()
            .join(",");
        let where_clause = whr
            .iter()
            .map(|c| {
                idx += 1;
                format!("{}=${}", q_ident(c), idx)
            })
            .collect::<Vec<_>>()
            .join(" and ");
        format!(
            "UPDATE {} SET {} WHERE {}",
            q_ident(table),
            set_clause,
            where_clause
        )
    }

    fn build_delete(&self, table: &str, whr: &[String]) -> String {
        let where_clause = whr
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{}=${}", q_ident(c), i + 1))
            .collect::<Vec<_>>()
            .join(" and ");
        format!("DELETE FROM {} WHERE {}", q_ident(table), where_clause)
    }

    fn build_upsert(&self, table: &str, set: &[String], key: &[String]) -> String {
        // Insert columns = key ∪ set (key first, deterministic order).
        let mut cols: Vec<String> = key.to_vec();
        for c in set {
            if !cols.contains(c) {
                cols.push(c.clone());
            }
        }
        let collist = cols
            .iter()
            .map(|c| q_ident(c))
            .collect::<Vec<_>>()
            .join(",");
        let places = (1..=cols.len())
            .map(|i| format!("${i}"))
            .collect::<Vec<_>>()
            .join(",");
        let conflict = key.iter().map(|c| q_ident(c)).collect::<Vec<_>>().join(",");
        // ON CONFLICT(key) DO UPDATE SET col=excluded.col (only set columns).
        let do_update = set
            .iter()
            .map(|c| format!("{}=excluded.{}", q_ident(c), q_ident(c)))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT ({}) DO UPDATE SET {} RETURNING *",
            q_ident(table),
            collist,
            places,
            conflict,
            do_update
        )
    }

    fn build_create_table(&self, table: &str, cols: &[ColDef]) -> String {
        build_create_table_sql(table, cols)
    }

    fn begin(&self) -> Result<Box<dyn DbTx>, String> {
        // The tx checks out a separate connection from the POOL — the global pool
        // stays free, other parallel queries run unhindered (user-approved design).
        let conn = self.pool.checkout()?;
        // BEGIN IMMEDIATE — acquires the write lock up front (race-safe, no overdraft).
        if let Err(e) = conn.execute_batch("BEGIN IMMEDIATE") {
            self.pool.checkin(conn);
            return Err(format!("tx could not begin: {e}"));
        }
        Ok(Box::new(SqliteTx {
            conn: Some(conn),
            pool: self.pool.clone(), // connection returns to the pool on commit/rollback
        }))
    }
}

// Introspects SQLite table columns: takes the declared type from pragma_table_info
// and converts it to a Fluxon type name. Empty list if the table is missing.
fn sqlite_column_types(conn: &Connection, table: &str) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare("SELECT name, type FROM pragma_table_info(?1)")
        .map_err(|e| sql_err("pragma_table_info", e))?;
    let rows = stmt
        .query_map([table], |row| {
            let name: String = row.get(0)?;
            let decl: String = row.get(1)?;
            Ok((name, sqlite_decl_to_fluxon_type(&decl)))
        })
        .map_err(|e| sql_err("pragma_table_info", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| sql_err("pragma_table_info", e))?);
    }
    Ok(out)
}

// Reads Fluxon-managed indexes: from pragma_index_list, those with origin='c'
// (CREATE INDEX) AND a `idx_`/`uniq_` prefixed name. Does NOT touch
// `sqlite_autoindex_*` (origin='u'/'pk') and other manually created indexes.
fn sqlite_fluxon_indexes(conn: &Connection, table: &str) -> Result<Vec<IndexInfo>, String> {
    let mut stmt = conn
        .prepare("SELECT name, origin FROM pragma_index_list(?1)")
        .map_err(|e| sql_err("pragma_index_list", e))?;
    let rows = stmt
        .query_map([table], |row| {
            let name: String = row.get(0)?;
            let origin: String = row.get(1)?;
            Ok((name, origin))
        })
        .map_err(|e| sql_err("pragma_index_list", e))?;
    let mut out = Vec::new();
    for r in rows {
        let (name, origin) = r.map_err(|e| sql_err("pragma_index_list", e))?;
        // Only Fluxon-created (CREATE INDEX) + our prefixes.
        if origin == "c" && (name.starts_with("idx_") || name.starts_with("uniq_")) {
            out.push(IndexInfo { name });
        }
    }
    Ok(out)
}

// Existing FOREIGN KEY constraints on a table — via pragma_foreign_key_list.
// Migration compares with the declaration's FK set and rebuilds on a difference.
// If `to` is NULL (a columnless reference to the parent PK) it becomes an empty
// string — our DDL always writes an explicit column, so NULL never occurs in practice.
fn sqlite_foreign_keys(conn: &Connection, table: &str) -> Result<Vec<ForeignKey>, String> {
    let mut stmt = conn
        .prepare("SELECT \"from\", \"table\", \"to\" FROM pragma_foreign_key_list(?1)")
        .map_err(|e| sql_err("pragma_foreign_key_list", e))?;
    let rows = stmt
        .query_map([table], |row| {
            let from: String = row.get(0)?;
            let to_table: String = row.get(1)?;
            let to: Option<String> = row.get(2)?;
            Ok(ForeignKey {
                from,
                table: to_table,
                to: to.unwrap_or_default(),
            })
        })
        .map_err(|e| sql_err("pragma_foreign_key_list", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| sql_err("pragma_foreign_key_list", e))?);
    }
    Ok(out)
}

// Count of FK violations (orphan rows) — pragma_foreign_key_check. Checked at the
// end of a rebuild: if existing data violates the new FK, error instead of silently
// losing it.
fn sqlite_fk_violations(conn: &Connection, table: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT count(*) FROM pragma_foreign_key_check(?1)",
        [table],
        |row| row.get(0),
    )
    .map_err(|e| sql_err("pragma_foreign_key_check", e))
}

// Finds a non-existing (free) rebuild backup name. `ts` has only second precision —
// if a table is rebuilt twice within one second (e.g. one deploy adds `ref:`, soon
// after another removes it) the same name collides. We append a counter until a free
// name is found: `_fk`, `_fk_2`, `_fk_3`... Every backup is kept intentionally, so it
// is not a clobber — a new name.
fn unique_rebuild_backup_name(conn: &Connection, table: &str, ts: u64) -> Result<String, String> {
    let base = format!("_fluxon_bak_{table}_{ts}_fk");
    let mut name = base.clone();
    let mut n = 2;
    while sqlite_table_exists(conn, &name)? {
        name = format!("{base}_{n}");
        n += 1;
    }
    Ok(name)
}

// Whether a table (or view) with this name exists — via sqlite_master.
fn sqlite_table_exists(conn: &Connection, name: &str) -> Result<bool, String> {
    let c: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |row| row.get(0),
        )
        .map_err(|e| sql_err("sqlite_master", e))?;
    Ok(c > 0)
}

// Fully rebuilds a table (SQLite "12-step" pattern): preserves data and migrates it
// to the new schema (with FK). Used when an FK/constraint change cannot be handled
// via ALTER — adding/removing an FK on an existing column.
//
// `PRAGMA foreign_keys=OFF` is set OUTSIDE THE TRANSACTION (a no-op inside a tx).
// Everything is in one transaction — if it is interrupted halfway, ROLLBACK keeps
// the data intact. At the end, foreign_key_check: if there are orphan rows the
// rebuild is aborted.
fn sqlite_rebuild_table(
    conn: &Connection,
    table: &str,
    cols: &[ColDef],
    indexes: &[IndexDef],
    ts: u64,
) -> Result<(), String> {
    // Columns to migrate — the intersection of live and desired (in declared order).
    let desired: std::collections::HashSet<&str> = cols.iter().map(|c| c.name.as_str()).collect();
    let live: Vec<String> = sqlite_column_types(conn, table)?
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let common = live
        .iter()
        .filter(|c| desired.contains(c.as_str()))
        .map(|c| q_ident(c))
        .collect::<Vec<_>>()
        .join(", ");

    let tmp = format!("_fluxon_rebuild_{table}");
    // foreign_keys OFF outside the tx; FK is not enforced during the rebuild
    // (so drop/rename works regardless of parent-child order).
    conn.execute_batch("PRAGMA foreign_keys=OFF")
        .map_err(|e| sql_err("PRAGMA foreign_keys=OFF", e))?;

    let result = (|| -> Result<(), String> {
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| sql_err("BEGIN", e))?;
        // 1. Safety backup (before DROP — protection against an agent mistake). Name
        //    collision avoidance: (a) within one migration a DROP COLUMN may also have
        //    created a backup (`build_backup` with the same `ts`) — `_fk` suffix;
        //    (b) if there are two rebuilds within one second (`ts` has only second
        //    precision) we append a counter until a free name is found (`_fk`, `_fk_2`, ...).
        let bak = unique_rebuild_backup_name(conn, table, ts)?;
        run_exec(
            conn,
            &format!(
                "CREATE TABLE {} AS SELECT * FROM {}",
                q_ident(&bak),
                q_ident(table)
            ),
            &[],
        )?;
        // 2. New table with a temporary name (full desired schema + FK).
        run_exec(conn, &build_create_table_sql(&tmp, cols), &[])?;
        // 3. Copy the common columns (INSERT ... SELECT is safe even when empty).
        if !common.is_empty() {
            run_exec(
                conn,
                &format!(
                    "INSERT INTO {} ({}) SELECT {} FROM {}",
                    q_ident(&tmp),
                    common,
                    common,
                    q_ident(table)
                ),
                &[],
            )?;
        }
        // 4. Drop the old one, rename the new one to the original name.
        run_exec(conn, &format!("DROP TABLE {}", q_ident(table)), &[])?;
        run_exec(
            conn,
            &format!("ALTER TABLE {} RENAME TO {}", q_ident(&tmp), q_ident(table)),
            &[],
        )?;
        // 5. Recreate the indexes (DROP TABLE removed them).
        for idx in indexes {
            run_exec(conn, &build_create_index(idx), &[])?;
        }
        // 6. If orphan rows violate the new FK — error instead of silently losing
        //    them (ROLLBACK).
        let bad = sqlite_fk_violations(conn, table)?;
        if bad > 0 {
            return Err(format!(
                "table `{table}`: {bad} orphan rows violate the FK constraint — rebuild aborted (clean up the data first)"
            ));
        }
        conn.execute_batch("COMMIT")
            .map_err(|e| sql_err("COMMIT", e))?;
        Ok(())
    })();

    if result.is_err() {
        let _ = conn.execute_batch("ROLLBACK");
    }
    // Re-enable FK — so the connection returns to the pool with it ON.
    let _ = conn.execute_batch("PRAGMA foreign_keys=ON");
    result
}

// Maps a declared SQLite type to a Fluxon type name. Currently only json matters
// (sqlval_to_value decodes it into a map/list); the rest comes back as text and
// undergoes no special conversion.
fn sqlite_decl_to_fluxon_type(decl: &str) -> String {
    if decl.eq_ignore_ascii_case("json") {
        "json".to_string()
    } else {
        decl.to_ascii_lowercase()
    }
}

// SQLite identifier quoting: "..." (inner " -> "").
pub(crate) fn q_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

// Converts a tbl column into a SQLite CREATE TABLE definition.
fn sqlite_column_def(c: &ColDef) -> String {
    let has = |m: &str| c.modifiers.iter().any(|x| x == m);
    let sql_type = match c.type_name.as_str() {
        "serial" => "INTEGER",
        "int" | "money" | "now" | "bool" => "INTEGER",
        "flt" => "REAL",
        // json -> declared type "JSON". SQLite stores it as TEXT (a json value is
        // always {}/[] — NUMERIC affinity leaves it as text), but the declared type
        // stays in the DB schema: so that a process which did not declare `tbl` can,
        // when reading, reconstruct via introspection that the column is json (issue #63).
        "json" => "JSON",
        // str/sym and unknown -> TEXT
        _ => "TEXT",
    };
    let mut def = format!("{} {}", q_ident(&c.name), sql_type);
    // serial -> auto-incrementing primary key.
    if c.type_name == "serial" {
        def.push_str(" PRIMARY KEY AUTOINCREMENT");
    } else if has("pk") {
        def.push_str(" PRIMARY KEY");
    }
    // `uniq` is NO LONGER inline UNIQUE — it goes via a separate CREATE UNIQUE INDEX
    // (so migration can later drop/add it). `index` is also ignored in the column DDL
    // (separate CREATE INDEX). So nothing is added here.
    if c.type_name == "now" {
        def.push_str(" DEFAULT CURRENT_TIMESTAMP");
    }
    // `ref:tbl.col` -> column-level REFERENCES (FOREIGN KEY). `PRAGMA
    // foreign_keys=ON` is enabled on every connection (checkout), so the constraint
    // is enforced on ins/up. The column allows NULL (not NOT NULL), so ALTER TABLE
    // ADD COLUMN also works with REFERENCES.
    if let Some((tbl, col)) = column_ref_target(&c.modifiers) {
        def.push_str(&format!(" REFERENCES {}({})", q_ident(tbl), q_ident(col)));
    }
    def
}

// Extracts the FK target (table, column) from the `ref:tbl.col` modifier. None if
// not found. The first `ref:` modifier is used (a column has a single FK).
fn column_ref_target(modifiers: &[String]) -> Option<(&str, &str)> {
    modifiers
        .iter()
        .find_map(|m| m.strip_prefix("ref:"))
        .and_then(|t| t.split_once('.'))
}

// FNV-1a 32-bit hash — DETERMINISTIC (stable across Rust versions/platforms, unlike
// std DefaultHasher). For a collision-free suffix when truncating long index names.
// NOT random — the same input → the same output (required for idempotent migration).
fn fnv1a(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

// Deterministic index name, within the DB identifier limit (PostgreSQL
// NAMEDATALEN=63 bytes — the most restrictive backend). Logical name:
// `idx_<tbl>_<c1>_<c2>...` (`uniq_` for unique). If it fits the limit — use it as is;
// otherwise `<short-prefix>_<fnv8>` (hash from the FULL logical name — even if
// different indexes collapse to the same short prefix, the hash distinguishes them).
// CREATE and DROP call this function — name agreement is required for idempotency.
pub fn index_name(idx: &IndexDef) -> String {
    let prefix = if idx.unique { "uniq" } else { "idx" };
    let logical = format!("{}_{}_{}", prefix, idx.table, idx.columns.join("_"));
    const LIMIT: usize = 63;
    if logical.len() <= LIMIT {
        return logical;
    }
    let hash = format!("{:08x}", fnv1a(&logical)); // 8 hex chars
    // prefix + as much of the logical name as fits + "_<hash>" (total <= LIMIT bytes).
    let keep = LIMIT - (hash.len() + 1);
    // Cut by char to avoid breaking the byte boundary (an ASCII identifier is
    // expected, but to be safe).
    let mut short = String::new();
    for ch in logical.chars() {
        if short.len() + ch.len_utf8() > keep {
            break;
        }
        short.push(ch);
    }
    format!("{}_{}", short, hash)
}

// CREATE [UNIQUE] INDEX IF NOT EXISTS — idempotent.
pub fn build_create_index(idx: &IndexDef) -> String {
    let name = index_name(idx);
    let uniq = if idx.unique { "UNIQUE " } else { "" };
    let cols = idx
        .columns
        .iter()
        .map(|c| q_ident(c))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "CREATE {}INDEX IF NOT EXISTS {} ON {} ({})",
        uniq,
        q_ident(&name),
        q_ident(&idx.table),
        cols
    )
}

// DROP INDEX IF EXISTS — idempotent.
pub fn build_drop_index(name: &str) -> String {
    format!("DROP INDEX IF EXISTS {}", q_ident(name))
}

// ALTER TABLE ... ADD COLUMN (SQLite has no IF NOT EXISTS — migration swallows the
// error in the "duplicate column" case).
pub fn build_add_column(table: &str, c: &ColDef) -> String {
    format!(
        "ALTER TABLE {} ADD COLUMN {}",
        q_ident(table),
        sqlite_column_def(c)
    )
}

// ALTER TABLE ... DROP COLUMN.
pub fn build_drop_column(table: &str, col: &str) -> String {
    format!(
        "ALTER TABLE {} DROP COLUMN {}",
        q_ident(table),
        q_ident(col)
    )
}

// CREATE TABLE IF NOT EXISTS — from a list of ColDef (with FK/REFERENCES). The trait
// method and rebuild also use this function (guaranteeing identical DDL).
pub fn build_create_table_sql(table: &str, cols: &[ColDef]) -> String {
    let coldefs = cols
        .iter()
        .map(sqlite_column_def)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "CREATE TABLE IF NOT EXISTS {} ({})",
        q_ident(table),
        coldefs
    )
}

// Copies the table within the DB before a DROP (safety backup). `ts` — the migration
// time (to make the backup name unique; it need not be idempotent — a re-run creates
// a new backup, which is intentional for safety).
pub fn build_backup(table: &str, ts: u64) -> String {
    let bak = format!("_fluxon_bak_{table}_{ts}");
    format!(
        "CREATE TABLE {} AS SELECT * FROM {}",
        q_ident(&bak),
        q_ident(table)
    )
}

// --- SqliteTx: an active transaction (owns the connection) ---

struct SqliteTx {
    conn: Option<Connection>,
    // The pool, for returning the connection (Arc clone — alive as long as the tx).
    pool: Arc<Pool>,
}

impl SqliteTx {
    fn conn(&self) -> Result<&Connection, String> {
        self.conn.as_ref().ok_or_else(|| "tx is closed".to_string())
    }
    // Returns the connection to the pool on commit/rollback. If COMMIT/ROLLBACK fails
    // (deferred FK violation, SQLITE_BUSY, etc.) the transaction may stay open — if a
    // dirty connection returns to the pool, the next checkout gets "cannot start a
    // transaction within a transaction" or tx-less writes leak into the old open
    // transaction (issue #103). So ROLLBACK first; if that also fails, the connection
    // is not returned to the pool (dropped — closed).
    fn give_back(&mut self) {
        if let Some(conn) = self.conn.take() {
            if !conn.is_autocommit() && conn.execute_batch("ROLLBACK").is_err() {
                return;
            }
            self.pool.checkin(conn);
        }
    }
}

impl DbTx for SqliteTx {
    fn query(&self, sql: &str, params: &[SqlVal]) -> Result<Vec<Row>, String> {
        run_query(self.conn()?, sql, params)
    }
    fn exec(&self, sql: &str, params: &[SqlVal]) -> Result<usize, String> {
        run_exec(self.conn()?, sql, params)
    }
    fn query_returning(&self, sql: &str, params: &[SqlVal]) -> Result<Vec<Row>, String> {
        run_query(self.conn()?, sql, params)
    }
    fn savepoint(&self, name: &str) -> Result<(), String> {
        self.conn()?
            .execute_batch(&format!("SAVEPOINT {}", q_ident(name)))
            .map_err(|e| format!("savepoint: {e}"))
    }
    fn release(&self, name: &str) -> Result<(), String> {
        self.conn()?
            .execute_batch(&format!("RELEASE {}", q_ident(name)))
            .map_err(|e| format!("release: {e}"))
    }
    fn rollback_to(&self, name: &str) -> Result<(), String> {
        // ROLLBACK TO undoes the savepoint but leaves the savepoint on the stack — we
        // clean it up with RELEASE, otherwise it gets confused in the nested case.
        let id = q_ident(name);
        self.conn()?
            .execute_batch(&format!("ROLLBACK TO {id}; RELEASE {id}"))
            .map_err(|e| format!("rollback_to: {e}"))
    }
    fn commit(mut self: Box<Self>) -> Result<(), String> {
        let r = self
            .conn()?
            .execute_batch("COMMIT")
            .map_err(|e| format!("commit: {e}"));
        self.give_back();
        r
    }
    fn rollback(mut self: Box<Self>) -> Result<(), String> {
        let r = self
            .conn()?
            .execute_batch("ROLLBACK")
            .map_err(|e| format!("rollback: {e}"));
        self.give_back();
        r
    }
    fn column_types(&self, table: &str) -> Result<Vec<(String, String)>, String> {
        sqlite_column_types(self.conn()?, table)
    }
}

impl Drop for SqliteTx {
    fn drop(&mut self) {
        // If commit/rollback was not called (panic, etc.) — give_back ROLLBACKs the
        // open transaction and returns the connection, otherwise the DB stays locked.
        self.give_back();
    }
}

// ==================== backend selection (single config point) ====================

pub fn open_from_env() -> Result<Arc<dyn Db>, String> {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:fluxon.db".to_string());
    match url.split_once(':') {
        Some(("sqlite", rest)) => Ok(Arc::new(SqliteDb::open(rest)?)),
        Some(("postgres", _)) | Some(("postgresql", _)) => {
            Err("postgres backend not connected yet (use DATABASE_URL=sqlite:...)".to_string())
        }
        Some(("mysql", _)) => {
            Err("mysql backend not connected yet (use DATABASE_URL=sqlite:...)".to_string())
        }
        _ => Err(format!("unknown DATABASE_URL scheme: {url}")),
    }
}

// ==================== Interp dispatch ====================

// The active transaction on the current thread. HTTP runs each request on a separate
// spawn_blocking thread, so thread_local gives correct isolation.
thread_local! {
    static CURRENT_TX: std::cell::RefCell<Option<Box<dyn DbTx>>> =
        const { std::cell::RefCell::new(None) };
    // Nested SAVEPOINT depth (for a unique name).
    static TX_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

// A guard that clears CURRENT_TX even on the panic path during tx_outer. If a
// Rust-level panic occurred inside the lambda, the tx would be left in the
// thread_local; since tokio spawn_blocking threads are reused, the NEXT request could
// keep running inside the old tx (issue #103). The guard removes the tx —
// SqliteTx::Drop ROLLBACKs and returns the connection to the pool.
struct TxClearGuard;
impl Drop for TxClearGuard {
    fn drop(&mut self) {
        // Let take() drop outside the borrow (SqliteTx::Drop does not touch the
        // RefCell, but we separate it as a precaution).
        let tx = CURRENT_TX.with(|c| c.borrow_mut().take());
        drop(tx);
        TX_DEPTH.with(|d| d.set(0));
    }
}

// Whether there is an active transaction (`db.tx`) on the current thread. `par` uses
// this to detect being called from inside a tx: new threads do NOT INHERIT the
// `CURRENT_TX` TLS, so par lambdas would run outside the transaction context
// (read-your-writes/atomicity would break) — par rejects this with an explicit error
// (issue #137 PR review, P1). A SQLite connection is not thread-safe, so the tx
// cannot be shared across threads either — the ban is the correct solution.
pub(crate) fn tx_active() -> bool {
    CURRENT_TX.with(|c| c.borrow().is_some())
}

// Routes to the current tx if there is one, otherwise to the global Db. f — a closure
// that runs against the tx/db.
fn with_db<T>(
    interp: &Interp,
    on_tx: impl FnOnce(&dyn DbTx) -> Result<T, String>,
    on_global: impl FnOnce(&dyn Db) -> Result<T, String>,
) -> Result<T, Flow> {
    let via_tx = CURRENT_TX.with(|cell| {
        let b = cell.borrow();
        b.as_ref().map(|tx| on_tx(tx.as_ref()))
    });
    match via_tx {
        Some(r) => r.map_err(Flow::err),
        None => {
            let db = interp.db()?;
            on_global(db.as_ref()).map_err(Flow::err)
        }
    }
}

impl Interp {
    // db.<func> calls. eval_call routes here.
    pub fn db_dispatch(self: &Arc<Self>, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            "q" => self.db_q(args),
            "one" => self.db_one(args),
            "ins" => self.db_ins(args),
            "up" => self.db_up(args),
            "del" => self.db_del(args),
            "put" => self.db_put(args),
            "tx" => self.db_tx(args),
            // --- declarative read builder (issue #78) ---
            // db.from "t" |> db.eq {...} |> db.cmp :c :ge v |> db.order :c
            //   |> db.limit n |> db.offset m |> db.all|db.first
            // Aggregation: |> db.group :c |> db.count :out |> db.sum :c :out
            //   |> db.count_if {f} :out |> db.sum_if :c {f} :out |> db.agg|db.agg_row
            // Builder state flows through inside a Value::Map with the `__dbq` marker
            // (pipe makes each stage the LAST argument of the next).
            "from" => db_from(args),
            "eq" => db_stage_eq(args),
            "cmp" => db_stage_cmp(args),
            "order" => db_stage_order(args),
            "limit" => db_stage_limit(args),
            "offset" => db_stage_offset(args),
            "group" => db_stage_group(args),
            "count" => db_stage_agg(args, "count", false),
            "sum" => db_stage_agg(args, "sum", false),
            "avg" => db_stage_agg(args, "avg", false),
            "min" => db_stage_agg(args, "min", false),
            "max" => db_stage_agg(args, "max", false),
            "count_if" => db_stage_count_if(args),
            "sum_if" => db_stage_agg_if(args, "sum"),
            "avg_if" => db_stage_agg_if(args, "avg"),
            "all" => self.db_run_query(args, RunMode::All),
            "first" => self.db_run_query(args, RunMode::First),
            "agg" => self.db_run_query(args, RunMode::Agg),
            "agg_row" => self.db_run_query(args, RunMode::AggRow),
            _ => Err(Flow::err(format!("db module has no function '{}'", func))),
        }
    }

    // db.q sql [params?] -> list of rows (maps).
    fn db_q(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let sql = arg_sql(&args, "db.q")?;
        let params = arg_params(&args, 1)?;
        let table = extract_from_table(&sql);
        let rows = with_db(
            self,
            |tx| tx.query(&sql, &params),
            |db| db.query(&sql, &params),
        )?;
        Ok(Value::List(
            rows.into_iter()
                .map(|r| self.row_to_value(table.as_deref(), r))
                .collect(),
        ))
    }

    // db.one sql [params?] -> first row or nil.
    fn db_one(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let sql = arg_sql(&args, "db.one")?;
        let params = arg_params(&args, 1)?;
        let table = extract_from_table(&sql);
        let rows = with_db(
            self,
            |tx| tx.query(&sql, &params),
            |db| db.query(&sql, &params),
        )?;
        match rows.into_iter().next() {
            Some(r) => Ok(self.row_to_value(table.as_deref(), r)),
            None => Ok(Value::Nil),
        }
    }

    // db.ins "table" {map} -> the inserted row.
    fn db_ins(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let table = arg_table(&args, "db.ins")?;
        let map = arg_map(&args, 1, "db.ins")?;
        let (cols, vals) = self.map_to_cols(&table, &map)?;
        if cols.is_empty() {
            return Err(Flow::err("db.ins: empty map — nothing to insert"));
        }
        let sql = self.db_builder(|db| db.build_insert(&table, &cols))?;
        let rows = with_db(
            self,
            |tx| tx.query_returning(&sql, &vals),
            |db| db.query_returning(&sql, &vals),
        )?;
        match rows.into_iter().next() {
            Some(r) => Ok(self.row_to_value(Some(&table), r)),
            None => Ok(Value::Nil),
        }
    }

    // db.up "table" {set} {where} -> nil.
    fn db_up(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let table = arg_table(&args, "db.up")?;
        let set = arg_map(&args, 1, "db.up")?;
        let whr = arg_map(&args, 2, "db.up")?;
        let (set_cols, mut vals) = self.map_to_cols(&table, &set)?;
        let (whr_cols, whr_vals) = self.map_to_cols(&table, &whr)?;
        if set_cols.is_empty() {
            return Err(Flow::err("db.up: update map is empty"));
        }
        // Guard, like in db.del: an empty condition → build_update builds the "WHERE"
        // part with no columns (malformed SQL) and the whole table would be updated.
        // Give a clear error instead of SQLite's raw "incomplete input" message.
        if whr_cols.is_empty() {
            return Err(Flow::err(
                "db.up: condition map is empty — rejected so the whole table is not updated",
            ));
        }
        vals.extend(whr_vals);
        let sql = self.db_builder(|db| db.build_update(&table, &set_cols, &whr_cols))?;
        with_db(self, |tx| tx.exec(&sql, &vals), |db| db.exec(&sql, &vals))?;
        Ok(Value::Nil)
    }

    // db.del "table" {where} -> nil.
    fn db_del(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let table = arg_table(&args, "db.del")?;
        let whr = arg_map(&args, 1, "db.del")?;
        let (whr_cols, vals) = self.map_to_cols(&table, &whr)?;
        if whr_cols.is_empty() {
            return Err(Flow::err(
                "db.del: condition map is empty — rejected so the whole table is not deleted",
            ));
        }
        let sql = self.db_builder(|db| db.build_delete(&table, &whr_cols))?;
        with_db(self, |tx| tx.exec(&sql, &vals), |db| db.exec(&sql, &vals))?;
        Ok(Value::Nil)
    }

    // db.put "table" {set} {key} -> the upserted row.
    fn db_put(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let table = arg_table(&args, "db.put")?;
        let set = arg_map(&args, 1, "db.put")?;
        let key = arg_map(&args, 2, "db.put")?;
        let (set_cols, _) = self.map_to_cols(&table, &set)?;
        let (key_cols, _) = self.map_to_cols(&table, &key)?;
        if key_cols.is_empty() {
            return Err(Flow::err("db.put: key map is empty"));
        }
        // Bind order = the column order in build_upsert: key ∪ set.
        let mut cols: Vec<String> = key_cols.clone();
        for c in &set_cols {
            if !cols.contains(c) {
                cols.push(c.clone());
            }
        }
        // Merged map: key + set (the set column value takes priority).
        let mut merged = key.clone();
        for (k, v) in &set {
            merged.insert(k.clone(), v.clone());
        }
        let vals = self.cols_to_vals(&table, &cols, &merged)?;
        let sql = self.db_builder(|db| db.build_upsert(&table, &set_cols, &key_cols))?;
        let rows = with_db(
            self,
            |tx| tx.query_returning(&sql, &vals),
            |db| db.query_returning(&sql, &vals),
        )?;
        match rows.into_iter().next() {
            Some(r) => Ok(self.row_to_value(Some(&table), r)),
            None => Ok(Value::Nil),
        }
    }

    // db.tx \-> ... — an atomic block. If nested, SAVEPOINT.
    fn db_tx(self: &Arc<Self>, args: Vec<Value>) -> Result<Value, Flow> {
        let lambda = match args.into_iter().next() {
            Some(f @ (Value::Fn(_) | Value::Native(_))) => f,
            _ => {
                return Err(Flow::err("db.tx: argument must be a function (\\-> ...)"));
            }
        };

        let already = CURRENT_TX.with(|c| c.borrow().is_some());
        if already {
            return self.tx_nested(lambda);
        }
        self.tx_outer(lambda)
    }

    // First (outer) tx: BEGIN ... COMMIT/ROLLBACK.
    fn tx_outer(self: &Arc<Self>, lambda: Value) -> Result<Value, Flow> {
        let tx = self.db()?.begin().map_err(Flow::err)?;
        CURRENT_TX.with(|c| *c.borrow_mut() = Some(tx));
        // thread_local is cleared even if the lambda panics (on the normal path
        // the guard is a no-op after the take() below).
        let _guard = TxClearGuard;

        let result = self.apply(lambda, vec![]);

        // We take the tx back from thread_local (commit/rollback takes ownership of it).
        let tx = CURRENT_TX.with(|c| c.borrow_mut().take());
        let tx = match tx {
            Some(tx) => tx,
            None => return Err(Flow::err("internal: tx context was lost")),
        };

        match result {
            Ok(v) => match tx.commit() {
                Ok(()) => Ok(v),
                Err(e) => Err(Flow::err(e)),
            },
            Err(Flow::Return(v)) => match tx.commit() {
                Ok(()) => Ok(v),
                Err(e) => Err(Flow::err(e)),
            },
            Err(flow) => {
                let _ = tx.rollback();
                // skip/stop -> clearer error
                match flow {
                    Flow::Skip | Flow::Stop => Err(Flow::err("skip/stop used inside db.tx")),
                    other => Err(other),
                }
            }
        }
    }

    // Nested tx: SAVEPOINT on top of the current tx.
    fn tx_nested(self: &Arc<Self>, lambda: Value) -> Result<Value, Flow> {
        let depth = TX_DEPTH.with(|d| {
            let n = d.get() + 1;
            d.set(n);
            n
        });
        let name = format!("fluxon_sp_{depth}");

        let sp_res = CURRENT_TX.with(|c| {
            c.borrow()
                .as_ref()
                .map(|tx| tx.savepoint(&name))
                .unwrap_or_else(|| Err("internal: no nested tx context".to_string()))
        });
        if let Err(e) = sp_res {
            TX_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
            return Err(Flow::err(e));
        }

        let result = self.apply(lambda, vec![]);
        TX_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));

        let finalize = |commit: bool| -> Result<(), String> {
            CURRENT_TX.with(|c| {
                c.borrow().as_ref().map_or(Ok(()), |tx| {
                    if commit {
                        tx.release(&name)
                    } else {
                        tx.rollback_to(&name)
                    }
                })
            })
        };

        match result {
            Ok(v) => finalize(true).map(|_| v).map_err(Flow::err),
            Err(Flow::Return(v)) => finalize(true).map(|_| v).map_err(Flow::err),
            Err(flow) => {
                let _ = finalize(false);
                match flow {
                    Flow::Skip | Flow::Stop => Err(Flow::err("skip/stop used inside db.tx")),
                    other => Err(other),
                }
            }
        }
    }

    // Calls the build_* trait method on the global db (SQL generation is
    // backend-dependent, but the dialect is the same even when there is a tx in db.*).
    fn db_builder(&self, f: impl FnOnce(&dyn Db) -> String) -> Result<String, Flow> {
        let db = self.db()?;
        Ok(f(db.as_ref()))
    }

    // Builder terminal: db.all/first/agg/agg_row. Builds SQL from the builder map,
    // executes it, and returns the result according to the mode.
    fn db_run_query(self: &Arc<Self>, args: Vec<Value>, mode: RunMode) -> Result<Value, Flow> {
        let who = match mode {
            RunMode::All => "db.all",
            RunMode::First => "db.first",
            RunMode::Agg => "db.agg",
            RunMode::AggRow => "db.agg_row",
        };
        let (b, _) = take_builder(args, who)?;
        let table = match b.get("table") {
            Some(Value::Str(s)) => s.clone(),
            _ => return Err(Flow::err(format!("{}: no table in builder", who))),
        };

        // Bind values ($1, $2, ...) — in the order they appear in the SQL.
        // In the agg case the conditional-filter binds inside SELECT come BEFORE
        // the WHERE, so we call build_agg_select BEFORE build_where (otherwise the
        // $N numbers would shift).
        let mut binds: Vec<SqlVal> = Vec::new();
        let is_agg = matches!(mode, RunMode::Agg | RunMode::AggRow);
        let select_sql = if is_agg {
            self.build_agg_select(&table, &b, &mut binds)?
        } else {
            format!("SELECT * FROM {}", q_ident(&table))
        };

        // WHERE clauses (binds continue after the SELECT ones).
        let mut where_parts: Vec<String> = Vec::new();
        self.build_where(&table, &b, &mut where_parts, &mut binds)?;

        let mut sql = select_sql;
        if !where_parts.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_parts.join(" AND "));
        }
        // GROUP BY (only when agg + group).
        if is_agg && let Some(Value::List(cols)) = b.get("group") {
            let gb = cols
                .iter()
                .filter_map(|v| {
                    if let Value::Str(s) = v {
                        Some(q_ident(s))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            if !gb.is_empty() {
                sql.push_str(" GROUP BY ");
                sql.push_str(&gb);
            }
        }
        // ORDER BY.
        if let Some(Value::List(o)) = b.get("order")
            && let (Some(Value::Str(col)), desc) = (o.first(), o.get(1))
        {
            let dir = if matches!(desc, Some(Value::Bool(true))) {
                " DESC"
            } else {
                ""
            };
            sql.push_str(&format!(" ORDER BY {}{}", q_ident(col), dir));
        }
        // LIMIT / OFFSET (limit 1 is not required for agg_row/first — we take the
        // result on the code side, but for first we add LIMIT 1).
        if matches!(mode, RunMode::First) {
            sql.push_str(" LIMIT 1");
        } else {
            let limit = if let Some(Value::Int(n)) = b.get("limit") {
                Some(*n)
            } else {
                None
            };
            let offset = if let Some(Value::Int(off)) = b.get("offset") {
                Some(*off)
            } else {
                None
            };
            match (limit, offset) {
                (Some(n), Some(off)) => sql.push_str(&format!(" LIMIT {} OFFSET {}", n, off)),
                (Some(n), None) => sql.push_str(&format!(" LIMIT {}", n)),
                // OFFSET without LIMIT: SQLite requires a LIMIT for OFFSET, which
                // is why it used to be silently ignored. LIMIT -1 = skips off rows
                // out of an unlimited number of rows.
                (None, Some(off)) => sql.push_str(&format!(" LIMIT -1 OFFSET {}", off)),
                (None, None) => {}
            }
        }

        let rows = with_db(
            self,
            |tx| tx.query(&sql, &binds),
            |db| db.query(&sql, &binds),
        )?;

        match mode {
            RunMode::All | RunMode::Agg => Ok(Value::List(
                rows.into_iter()
                    .map(|r| self.row_to_value(Some(&table), r))
                    .collect(),
            )),
            RunMode::First | RunMode::AggRow => match rows.into_iter().next() {
                Some(r) => Ok(self.row_to_value(Some(&table), r)),
                None => Ok(Value::Nil),
            },
        }
    }

    // Builds WHERE clauses and bindings from the builder's eq/cmp filters.
    fn build_where(
        &self,
        table: &str,
        b: &BTreeMap<String, Value>,
        parts: &mut Vec<String>,
        binds: &mut Vec<SqlVal>,
    ) -> Result<(), Flow> {
        // eq: {col:val} — equality; list value → IN (...).
        if let Some(Value::Map(eq)) = b.get("eq") {
            for (col, v) in eq {
                match v {
                    Value::List(items) => {
                        // IN (...) — empty list = always false (1=0).
                        if items.is_empty() {
                            parts.push("1=0".to_string());
                            continue;
                        }
                        let mut places = Vec::with_capacity(items.len());
                        for it in items {
                            binds.push(self.value_to_sqlval(table, col, it)?);
                            places.push(format!("${}", binds.len()));
                        }
                        parts.push(format!("{} IN ({})", q_ident(col), places.join(",")));
                    }
                    // nil → IS NULL (`= NULL` never matches in SQL).
                    Value::Nil => {
                        parts.push(format!("{} IS NULL", q_ident(col)));
                    }
                    _ => {
                        binds.push(self.value_to_sqlval(table, col, v)?);
                        parts.push(format!("{} = ${}", q_ident(col), binds.len()));
                    }
                }
            }
        }
        // cmp: [col, op, val] triples.
        if let Some(Value::List(cmps)) = b.get("cmp") {
            for c in cmps {
                if let Value::List(triple) = c
                    && let (Some(Value::Str(col)), Some(Value::Str(op)), Some(val)) =
                        (triple.first(), triple.get(1), triple.get(2))
                    && let Some(sql_op) = cmp_sql_op(op)
                {
                    binds.push(self.value_to_sqlval(table, col, val)?);
                    parts.push(format!("{} {} ${}", q_ident(col), sql_op, binds.len()));
                }
            }
        }
        Ok(())
    }

    // Builds the aggregation SELECT list: group columns + agg expressions.
    // A conditional agg (count_if/sum_if) becomes SUM(CASE WHEN <filter> THEN ... END) —
    // the filter bindings are added to binds inside SELECT (BEFORE the WHERE).
    fn build_agg_select(
        &self,
        table: &str,
        b: &BTreeMap<String, Value>,
        binds: &mut Vec<SqlVal>,
    ) -> Result<String, Flow> {
        let mut cols: Vec<String> = Vec::new();
        // group columns also appear in the result.
        if let Some(Value::List(g)) = b.get("group") {
            for v in g {
                if let Value::Str(s) = v {
                    cols.push(q_ident(s));
                }
            }
        }
        let aggs = match b.get("aggs") {
            Some(Value::List(a)) if !a.is_empty() => a,
            _ => {
                return Err(Flow::err(
                    "db.agg/agg_row: at least one aggregate is required (db.count/sum/avg/count_if/sum_if)",
                ));
            }
        };
        for a in aggs {
            let Value::List(spec) = a else { continue };
            let kind = str_at(spec, 0);
            let col = str_at(spec, 1);
            let out = str_at(spec, 2);
            let filt = spec.get(3);
            // Expression inside the aggregate: count → *, others → column.
            let inner = if kind == "count" {
                "*".to_string()
            } else {
                q_ident(&col)
            };
            let expr = match filt {
                // Unconditional: COUNT(*) / SUM(col).
                Some(Value::Nil) | None => format!("{}({})", kind.to_uppercase(), inner),
                // Conditional: convert the filter to CASE WHEN and wrap it in the aggregate.
                Some(Value::Map(f)) => {
                    let cond = self.filter_to_case_cond(table, f, binds)?;
                    if kind == "count" {
                        // COUNT(*) FILTER ekvivalenti: SUM(CASE WHEN cond THEN 1 ELSE 0 END).
                        // COALESCE — on an empty result SUM returns NULL, but count_if
                        // must return 0 like COUNT semantics (an empty tenant's
                        // dashboard should show 0, not nil).
                        format!("COALESCE(SUM(CASE WHEN {} THEN 1 ELSE 0 END), 0)", cond)
                    } else {
                        format!(
                            "{}(CASE WHEN {} THEN {} END)",
                            kind.to_uppercase(),
                            cond,
                            inner
                        )
                    }
                }
                _ => return Err(Flow::err("db.agg: internal error — invalid filter type")),
            };
            cols.push(format!("{} AS {}", expr, q_ident(&out)));
        }
        Ok(format!(
            "SELECT {} FROM {}",
            cols.join(", "),
            q_ident(table)
        ))
    }

    // Converts a conditional-aggregate filter into an SQL CASE condition (col = $N AND ...),
    // list value → IN. Bindings are added to binds (order matters — this is
    // inside SELECT, called before the WHERE).
    fn filter_to_case_cond(
        &self,
        table: &str,
        f: &BTreeMap<String, Value>,
        binds: &mut Vec<SqlVal>,
    ) -> Result<String, Flow> {
        let mut conds = Vec::new();
        for (col, v) in f {
            match v {
                Value::List(items) => {
                    if items.is_empty() {
                        conds.push("1=0".to_string());
                        continue;
                    }
                    let mut places = Vec::with_capacity(items.len());
                    for it in items {
                        binds.push(self.value_to_sqlval(table, col, it)?);
                        places.push(format!("${}", binds.len()));
                    }
                    conds.push(format!("{} IN ({})", q_ident(col), places.join(",")));
                }
                // nil → IS NULL.
                Value::Nil => {
                    conds.push(format!("{} IS NULL", q_ident(col)));
                }
                _ => {
                    binds.push(self.value_to_sqlval(table, col, v)?);
                    conds.push(format!("{} = ${}", q_ident(col), binds.len()));
                }
            }
        }
        if conds.is_empty() {
            Ok("1=1".to_string())
        } else {
            Ok(conds.join(" AND "))
        }
    }
}

// The string at position i from the agg spec list (or empty).
fn str_at(spec: &[Value], i: usize) -> String {
    match spec.get(i) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

// --- Value <-> SqlVal and schema-aware conversion ---

impl Interp {
    // Splits a Fluxon map into (column, value) lists. The BTreeMap order is
    // deterministic — it matches the bindings.
    fn map_to_cols(
        &self,
        table: &str,
        map: &BTreeMap<String, Value>,
    ) -> Result<(Vec<String>, Vec<SqlVal>), Flow> {
        let mut cols = Vec::with_capacity(map.len());
        let mut vals = Vec::with_capacity(map.len());
        for (k, v) in map {
            cols.push(k.clone());
            vals.push(self.value_to_sqlval(table, k, v)?);
        }
        Ok((cols, vals))
    }

    // Takes values from the map in the given column order (for upsert).
    fn cols_to_vals(
        &self,
        table: &str,
        cols: &[String],
        map: &BTreeMap<String, Value>,
    ) -> Result<Vec<SqlVal>, Flow> {
        let mut vals = Vec::with_capacity(cols.len());
        for c in cols {
            let v = map.get(c).cloned().unwrap_or(Value::Nil);
            vals.push(self.value_to_sqlval(table, c, &v)?);
        }
        Ok(vals)
    }

    // Fluxon Value -> SqlVal (when writing). map/list into a json column -> json_encode.
    fn value_to_sqlval(&self, table: &str, col: &str, v: &Value) -> Result<SqlVal, Flow> {
        Ok(match v {
            Value::Int(n) => SqlVal::Int(*n),
            Value::Flt(x) => SqlVal::Real(*x),
            Value::Str(s) => SqlVal::Text(s.clone()),
            Value::Bool(b) => SqlVal::Int(if *b { 1 } else { 0 }),
            Value::Nil => SqlVal::Null,
            Value::Sym(s) => SqlVal::Text(s.clone()),
            // bytes -> BLOB (issue #132): SQLite native binary column.
            Value::Bytes(b) => SqlVal::Blob(b.as_ref().clone()),
            Value::List(_) | Value::Map(_) => {
                // When writing, only the in-process tbl registry is checked.
                // DB introspection is for the read side (json decode) — if used
                // here, the old schema-less writing to TEXT columns would break
                // (a process without tbl would be unable to write to a TEXT column).
                let tbl_type = self
                    .schema
                    .read()
                    .get(table)
                    .and_then(|t| t.columns.get(col))
                    .map(|c| c.type_name.clone());
                if tbl_type.as_deref() == Some("json") || tbl_type.is_none() {
                    SqlVal::Text(json_encode(v))
                } else {
                    return Err(Flow::err(format!(
                        "db: cannot write {} to column '{}.{}' (not a json column)",
                        v.type_name(),
                        table,
                        col
                    )));
                }
            }
            // ctx (req.ctx) — usually get_field returns a snapshot Map, but just in
            // case: written to a json column like an ordinary map (snapshot).
            Value::Ctx(c) => {
                let snap = Value::Map(c.lock().unwrap().clone());
                return self.value_to_sqlval(table, col, &snap);
            }
            Value::Fn(_) | Value::Native(_) => {
                return Err(Flow::err("db: cannot write a function to the DB"));
            }
        })
    }

    // Converts a row to a Fluxon map; sym/json/bool are reconstructed per the schema.
    fn row_to_value(&self, table: Option<&str>, row: Row) -> Value {
        let mut m = BTreeMap::new();
        for (col, cell) in row {
            let ty = table.and_then(|t| self.col_type(t, &col));
            m.insert(col, sqlval_to_value(cell, ty.as_deref()));
        }
        Value::Map(m)
    }

    // Gets the column type. The primary source is the schema registry declared
    // with `tbl` in the current process. If that is absent (for example in a
    // two-process setup the reader does not declare tbl) we reconstruct it via
    // introspection from the DB schema — so that a json column returns the same
    // map regardless of the process boundary.
    fn col_type(&self, table: &str, col: &str) -> Option<String> {
        if let Some(t) = self.schema.read().get(table)
            && let Some(c) = t.columns.get(col)
        {
            return Some(c.type_name.clone());
        }
        self.db_col_type(table, col)
    }

    // Finds the column type by introspecting the DB schema (cached per table —
    // no re-query for each row). The DB is already open: this method is only
    // called while converting a result row into a Value.
    //
    // If inside a transaction, the tx connection is used so that uncommitted DDL
    // is visible — the global pool connection cannot see this DDL (issue #63).
    fn db_col_type(&self, table: &str, col: &str) -> Option<String> {
        if let Some(entry) = self.db_schema.read().get(table) {
            return entry.get(col).cloned();
        }
        let raw = CURRENT_TX.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|tx| tx.column_types(table).ok())
        });
        let cols: BTreeMap<String, String> = match raw {
            Some(v) => v.into_iter().collect(),
            None => self
                .db()
                .ok()?
                .column_types(table)
                .unwrap_or_default()
                .into_iter()
                .collect(),
        };
        let result = cols.get(col).cloned();
        self.db_schema.write().insert(table.to_string(), cols);
        result
    }
}

// SqlVal -> Fluxon Value, post-processed by column type.
fn sqlval_to_value(cell: SqlVal, col_type: Option<&str>) -> Value {
    let base = match cell {
        SqlVal::Int(n) => Value::Int(n),
        SqlVal::Real(x) => Value::Flt(x),
        SqlVal::Text(s) => Value::Str(s),
        // BLOB -> bytes (issue #132). It used to be mangled into lossy text — now
        // binary data is returned losslessly (if text is needed: bytes.str).
        SqlVal::Blob(b) => Value::Bytes(std::sync::Arc::new(b)),
        SqlVal::Null => Value::Nil,
    };
    match (col_type, &base) {
        // sym column: DB text -> Fluxon symbol.
        (Some("sym"), Value::Str(s)) => Value::Sym(s.clone()),
        // json column: text -> decoded map/list.
        (Some("json"), Value::Str(s)) => json_decode(s).unwrap_or(base),
        // bool column: int 0/1 -> bool.
        (Some("bool"), Value::Int(n)) => Value::Bool(*n != 0),
        _ => base,
    }
}

// --- argument helpers ---

fn arg_sql(args: &[Value], who: &str) -> Result<String, Flow> {
    match args.first() {
        Some(Value::Str(s)) => Ok(s.clone()),
        _ => Err(Flow::err(format!(
            "{}: 1st argument must be SQL (str)",
            who
        ))),
    }
}

fn arg_table(args: &[Value], who: &str) -> Result<String, Flow> {
    match args.first() {
        Some(Value::Str(s)) => Ok(s.clone()),
        _ => Err(Flow::err(format!(
            "{}: 1st argument must be a table name (str)",
            who
        ))),
    }
}

fn arg_map(args: &[Value], i: usize, who: &str) -> Result<BTreeMap<String, Value>, Flow> {
    match args.get(i) {
        Some(Value::Map(m)) => Ok(m.clone()),
        _ => Err(Flow::err(format!(
            "{}: argument {} must be a map ({{...}})",
            who,
            i + 1
        ))),
    }
}

// The 2nd argument of db.q/one: an optional list of params.
fn arg_params(args: &[Value], i: usize) -> Result<Vec<SqlVal>, Flow> {
    match args.get(i) {
        None | Some(Value::Nil) => Ok(vec![]),
        Some(Value::List(xs)) => xs.iter().map(param_to_sqlval).collect(),
        Some(other) => Err(Flow::err(format!(
            "db: parameters must be a list ([...]), got {}",
            other.type_name()
        ))),
    }
}

// Param value to SqlVal (schema-less — q/one params have no column).
fn param_to_sqlval(v: &Value) -> Result<SqlVal, Flow> {
    Ok(match v {
        Value::Int(n) => SqlVal::Int(*n),
        Value::Flt(x) => SqlVal::Real(*x),
        Value::Str(s) => SqlVal::Text(s.clone()),
        Value::Bool(b) => SqlVal::Int(if *b { 1 } else { 0 }),
        Value::Nil => SqlVal::Null,
        Value::Sym(s) => SqlVal::Text(s.clone()), // symbol -> text (filter compatibility)
        Value::Bytes(b) => SqlVal::Blob(b.as_ref().clone()), // bytes -> BLOB (issue #132)
        Value::List(_) | Value::Map(_) => SqlVal::Text(json_encode(v)),
        // ctx — JSON text like an ordinary map (snapshot). json_encode handles this.
        Value::Ctx(_) => SqlVal::Text(json_encode(v)),
        Value::Fn(_) | Value::Native(_) => {
            return Err(Flow::err("db: cannot pass a function as a parameter"));
        }
    })
}

// Extracts the main table name from SQL: the identifier after ` from `.
// A limitation with join/alias — sym/json conversion works for the most common
// `from <table>` case.
//
// The search is done directly over the `char`s of the original `sql`: `to_lowercase()`
// changes the byte length for some characters (for example `İ` U+0130 → `i̇`),
// so applying a byte index obtained from the lowercase version to the original text
// leads to a char-boundary panic (issue #88). In addition, a ` from ` inside a
// string literal (`'...'`) is ignored — otherwise the wrong table name would be taken.
fn extract_from_table(sql: &str) -> Option<String> {
    let chars: Vec<char> = sql.chars().collect();
    let n = chars.len();
    let mut in_str = false; // whether we are inside an SQL string literal opened with `'`
    let mut i = 0;
    // i+5 — the last whitespace of ` from `; chars up to this index must exist.
    while i + 5 < n {
        let c = chars[i];
        if in_str {
            // `''` (a doubled apostrophe inside a literal) is also tracked correctly here:
            // the first `'` closes the literal, the next one reopens it.
            if c == '\'' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == '\'' {
            in_str = true;
            i += 1;
            continue;
        }
        // <whitespace> f r o m <whitespace> — case-insensitive.
        if c.is_whitespace()
            && chars[i + 1].eq_ignore_ascii_case(&'f')
            && chars[i + 2].eq_ignore_ascii_case(&'r')
            && chars[i + 3].eq_ignore_ascii_case(&'o')
            && chars[i + 4].eq_ignore_ascii_case(&'m')
            && chars[i + 5].is_whitespace()
        {
            let tok: String = chars[i + 6..]
                .iter()
                .copied()
                .skip_while(|c| c.is_whitespace())
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !tok.is_empty() {
                return Some(tok);
            }
        }
        i += 1;
    }
    None
}

// ==================== declarative read builder (issue #78) ====================
//
// The builder state is stored inside a Value::Map — without introducing a new
// Value variant (Send+Sync, json/display invariants are preserved automatically).
// The `__dbq` marker key distinguishes it from an ordinary map. Pipe makes each
// stage the LAST argument of the next db.* (`q |> db.eq {...}` => `db.eq {...} q`),
// so the stage functions take the builder from the END of args.

const DBQ_MARKER: &str = "__dbq";

// Starts the builder map: with only the table name.
fn db_from(args: Vec<Value>) -> Result<Value, Flow> {
    let table = match args.first() {
        Some(Value::Str(s)) => s.clone(),
        _ => {
            return Err(Flow::err(
                "db.from: 1st argument must be a table name (str)",
            ));
        }
    };
    let mut b = BTreeMap::new();
    b.insert(DBQ_MARKER.to_string(), Value::Bool(true));
    b.insert("table".to_string(), Value::Str(table));
    Ok(Value::Map(b))
}

// Splits off the builder map from the END of the arguments (pipe lhs is the last
// argument). Returns: (builder_map, remaining_arguments). If the builder is not
// found, a clear error — the stage was called without a pipe / without db.from.
fn take_builder(
    mut args: Vec<Value>,
    who: &str,
) -> Result<(BTreeMap<String, Value>, Vec<Value>), Flow> {
    match args.pop() {
        Some(Value::Map(m)) if m.contains_key(DBQ_MARKER) => Ok((m, args)),
        _ => Err(Flow::err(format!(
            "{}: builder not found — use it as `db.from \"t\" |> {}`",
            who, who
        ))),
    }
}

// Adds an element to a list field inside the builder (creates it if absent).
fn push_into(b: &mut BTreeMap<String, Value>, key: &str, item: Value) {
    match b.get_mut(key) {
        Some(Value::List(xs)) => xs.push(item),
        _ => {
            b.insert(key.to_string(), Value::List(vec![item]));
        }
    }
}

// db.eq {col:val ...} — equality filters (AND). List value → IN.
fn db_stage_eq(args: Vec<Value>) -> Result<Value, Flow> {
    let (mut b, rest) = take_builder(args, "db.eq")?;
    let filt = match rest.first() {
        Some(Value::Map(m)) => m.clone(),
        _ => return Err(Flow::err("db.eq: 1st argument must be a map ({...})")),
    };
    // We add to the existing eq map (multiple db.eq calls are possible).
    let mut eq = match b.remove("eq") {
        Some(Value::Map(m)) => m,
        _ => BTreeMap::new(),
    };
    for (k, v) in filt {
        eq.insert(k, v);
    }
    b.insert("eq".to_string(), Value::Map(eq));
    Ok(Value::Map(b))
}

// db.cmp :col :op val — a single comparison (:gt :ge :lt :le :ne :like).
fn db_stage_cmp(args: Vec<Value>) -> Result<Value, Flow> {
    let (mut b, rest) = take_builder(args, "db.cmp")?;
    if rest.len() < 3 {
        return Err(Flow::err("db.cmp: :col :op val — 3 arguments required"));
    }
    let col = arg_col(&rest[0], "db.cmp")?;
    let op = arg_sym(&rest[1], "db.cmp op")?;
    if cmp_sql_op(&op).is_none() {
        return Err(Flow::err(format!(
            "db.cmp: unknown operator :{} (:gt :ge :lt :le :ne :like)",
            op
        )));
    }
    let val = rest[2].clone();
    // The [col, op, val] triple into the cmp list.
    push_into(
        &mut b,
        "cmp",
        Value::List(vec![Value::Str(col), Value::Str(op), val]),
    );
    Ok(Value::Map(b))
}

// db.order :col [:desc].
fn db_stage_order(args: Vec<Value>) -> Result<Value, Flow> {
    let (mut b, rest) = take_builder(args, "db.order")?;
    let col = match rest.first() {
        Some(v) => arg_col(v, "db.order")?,
        None => return Err(Flow::err("db.order: :col argument required")),
    };
    let desc = matches!(rest.get(1), Some(Value::Sym(s)) if s == "desc");
    b.insert(
        "order".to_string(),
        Value::List(vec![Value::Str(col), Value::Bool(desc)]),
    );
    Ok(Value::Map(b))
}

fn db_stage_limit(args: Vec<Value>) -> Result<Value, Flow> {
    let (mut b, rest) = take_builder(args, "db.limit")?;
    let n = arg_int(rest.first(), "db.limit")?;
    // A negative LIMIT means "unlimited" in SQLite — unexpected behavior. Reject it explicitly.
    if n < 0 {
        return Err(Flow::err("db.limit: negative value not allowed"));
    }
    b.insert("limit".to_string(), Value::Int(n));
    Ok(Value::Map(b))
}

fn db_stage_offset(args: Vec<Value>) -> Result<Value, Flow> {
    let (mut b, rest) = take_builder(args, "db.offset")?;
    let n = arg_int(rest.first(), "db.offset")?;
    if n < 0 {
        return Err(Flow::err("db.offset: negative value not allowed"));
    }
    b.insert("offset".to_string(), Value::Int(n));
    Ok(Value::Map(b))
}

// db.group :col (or a list of sym/str).
fn db_stage_group(args: Vec<Value>) -> Result<Value, Flow> {
    let (mut b, rest) = take_builder(args, "db.group")?;
    let cols: Vec<Value> = match rest.first() {
        Some(Value::List(xs)) => xs
            .iter()
            .map(|v| arg_col(v, "db.group").map(Value::Str))
            .collect::<Result<_, _>>()?,
        Some(v) => vec![Value::Str(arg_col(v, "db.group")?)],
        None => return Err(Flow::err("db.group: :col argument required")),
    };
    b.insert("group".to_string(), Value::List(cols));
    Ok(Value::Map(b))
}

// db.count :out  /  db.sum :col :out  (and avg/min/max).
fn db_stage_agg(args: Vec<Value>, kind: &str, _cond: bool) -> Result<Value, Flow> {
    let (mut b, rest) = take_builder(args, &format!("db.{kind}"))?;
    // count: only :out; others: :col :out.
    let (col, out) = if kind == "count" {
        let out = match rest.first() {
            Some(v) => arg_col(v, "db.count")?,
            None => return Err(Flow::err("db.count: :out (output name) argument required")),
        };
        (String::new(), out)
    } else {
        if rest.len() < 2 {
            return Err(Flow::err(format!(
                "db.{kind}: :col :out — 2 arguments required"
            )));
        }
        (arg_col(&rest[0], "db.agg")?, arg_col(&rest[1], "db.agg")?)
    };
    // [kind, col, out, filter (or nil)].
    push_into(
        &mut b,
        "aggs",
        Value::List(vec![
            Value::Str(kind.to_string()),
            Value::Str(col),
            Value::Str(out),
            Value::Nil,
        ]),
    );
    Ok(Value::Map(b))
}

// db.count_if {filter} :out — conditional count (COUNT(*) FILTER ... CASE WHEN).
fn db_stage_count_if(args: Vec<Value>) -> Result<Value, Flow> {
    let (mut b, rest) = take_builder(args, "db.count_if")?;
    if rest.len() < 2 {
        return Err(Flow::err(
            "db.count_if: {filter} :out — 2 arguments required",
        ));
    }
    let filt = match &rest[0] {
        Value::Map(m) => Value::Map(m.clone()),
        _ => {
            return Err(Flow::err(
                "db.count_if: 1st argument must be a filter map ({...})",
            ));
        }
    };
    let out = arg_col(&rest[1], "db.count_if")?;
    push_into(
        &mut b,
        "aggs",
        Value::List(vec![
            Value::Str("count".to_string()),
            Value::Str(String::new()),
            Value::Str(out),
            filt,
        ]),
    );
    Ok(Value::Map(b))
}

// db.sum_if :col {filter} :out — conditional sum/average.
fn db_stage_agg_if(args: Vec<Value>, kind: &str) -> Result<Value, Flow> {
    let (mut b, rest) = take_builder(args, &format!("db.{kind}_if"))?;
    if rest.len() < 3 {
        return Err(Flow::err(format!(
            "db.{kind}_if: :col {{filter}} :out — 3 arguments required"
        )));
    }
    let col = arg_col(&rest[0], "db.agg_if")?;
    let filt = match &rest[1] {
        Value::Map(m) => Value::Map(m.clone()),
        _ => {
            return Err(Flow::err(format!(
                "db.{kind}_if: 2nd argument must be a filter map ({{...}})"
            )));
        }
    };
    let out = arg_col(&rest[2], "db.agg_if")?;
    push_into(
        &mut b,
        "aggs",
        Value::List(vec![
            Value::Str(kind.to_string()),
            Value::Str(col),
            Value::Str(out),
            filt,
        ]),
    );
    Ok(Value::Map(b))
}

// Terminal mode: how the result is returned.
#[derive(Clone, Copy, PartialEq)]
enum RunMode {
    All,    // list of maps
    First,  // a single map or nil
    Agg,    // list of maps by group
    AggRow, // a single agg row (no group)
}

// Sym operator to SQL operator.
fn cmp_sql_op(op: &str) -> Option<&'static str> {
    Some(match op {
        "gt" => ">",
        "ge" => ">=",
        "lt" => "<",
        "le" => "<=",
        "ne" => "!=",
        "like" => "like",
        _ => return None,
    })
}

// --- builder argument helpers ---

// Column name: sym (:col) or str ("col").
fn arg_col(v: &Value, who: &str) -> Result<String, Flow> {
    match v {
        Value::Sym(s) | Value::Str(s) => Ok(s.clone()),
        _ => Err(Flow::err(format!(
            "{}: column name must be a sym (:col) or str, got {}",
            who,
            v.type_name()
        ))),
    }
}

fn arg_sym(v: &Value, who: &str) -> Result<String, Flow> {
    match v {
        Value::Sym(s) => Ok(s.clone()),
        _ => Err(Flow::err(format!("{}: must be a sym (:op)", who))),
    }
}

fn arg_int(v: Option<&Value>, who: &str) -> Result<i64, Flow> {
    match v {
        Some(Value::Int(n)) => Ok(*n),
        _ => Err(Flow::err(format!("{}: int argument required", who))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The tests in this module open a real DB. `:memory:` is actually one shared
    // shared-cache DB ACROSS THE PROCESS (Pool note: `file::memory:?cache=shared`) —
    // so tests that do `CREATE TABLE` get a table-name collision
    // ("table ... already exists") or a shared-cache table-lock ("database
    // table is locked") when run in parallel, which makes them flaky (issue #145).
    // Solution: each test should use a UNIQUELY named shared-cache memory DB (the
    // pool opens multiple connections → shared-cache is required; a unique name →
    // tests do not see each other). `mem_db(name)` centralizes this pattern; the
    // name must be unique within the test.
    fn mem_db(name: &str) -> SqliteDb {
        SqliteDb::open(&format!("file:{name}?mode=memory&cache=shared")).unwrap()
    }

    fn idx(table: &str, cols: &[&str], unique: bool) -> IndexDef {
        IndexDef {
            table: table.to_string(),
            columns: cols.iter().map(|s| s.to_string()).collect(),
            unique,
        }
    }

    #[test]
    fn index_name_short_passthrough() {
        // A short name does not change — the full logical name is used.
        assert_eq!(
            index_name(&idx("bookings", &["status"], false)),
            "idx_bookings_status"
        );
        assert_eq!(
            index_name(&idx("bookings", &["resource_id", "start_at"], true)),
            "uniq_bookings_resource_id_start_at"
        );
    }

    #[test]
    fn index_name_long_truncates_deterministically() {
        // A long name exceeding 63 bytes -> truncated + hash suffix.
        let long_table = "very_long_table_name_for_appointments_and_bookings";
        let long = idx(
            long_table,
            &["resource_identifier", "starting_at_timestamp"],
            false,
        );
        let n1 = index_name(&long);
        let n2 = index_name(&long);
        assert_eq!(n1, n2, "must be deterministic");
        assert!(
            n1.len() <= 63,
            "must fit within the limit: {} ({})",
            n1,
            n1.len()
        );
        assert!(n1.starts_with("idx_"), "prefix must be preserved: {}", n1);
    }

    #[test]
    fn index_name_long_no_collision() {
        // Two different long indexes that fall onto the same short prefix do not
        // collide (the hash is derived from the full logical name).
        let t = "extremely_long_table_name_that_definitely_exceeds_the_limit_xx";
        let a = index_name(&idx(t, &["column_alpha"], false));
        let b = index_name(&idx(t, &["column_beta"], false));
        assert_ne!(
            a, b,
            "different columns must yield different names: {} vs {}",
            a, b
        );
    }

    #[test]
    fn index_name_unique_vs_nonunique_differ() {
        // uniq and index on the same columns yield different names (prefixes).
        let u = index_name(&idx("t", &["a"], true));
        let i = index_name(&idx("t", &["a"], false));
        assert_ne!(u, i);
        assert!(u.starts_with("uniq_"));
        assert!(i.starts_with("idx_"));
    }

    fn col(name: &str, ty: &str, mods: &[&str]) -> ColDef {
        ColDef {
            name: name.to_string(),
            type_name: ty.to_string(),
            modifiers: mods.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn coldef_fk_parsing() {
        // The `ref:tbl.col` modifier parses correctly into a ForeignKey.
        let c = col("owner", "int", &["ref:users.id"]);
        assert_eq!(
            coldef_foreign_key(&c),
            Some(ForeignKey {
                from: "owner".into(),
                table: "users".into(),
                to: "id".into(),
            })
        );
        assert_eq!(coldef_foreign_key(&col("title", "str", &[])), None);
    }

    #[test]
    fn column_def_emits_references() {
        // ref:tbl.col -> the column DDL must contain REFERENCES.
        let ddl = sqlite_column_def(&col("owner", "int", &["ref:users.id"]));
        assert!(
            ddl.contains("REFERENCES \"users\"(\"id\")"),
            "must contain REFERENCES: {ddl}"
        );
    }

    #[test]
    fn rebuild_preserves_data_and_adds_fk() {
        // rebuild_table: adds an FK to an existing column, preserves the data,
        // and the foreign_keys() introspection sees the new FK.
        let db = mem_db("rebuild_fk_test");
        // Parent + child (without FK) + data.
        db.exec(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)",
            &[],
        )
        .unwrap();
        db.exec("INSERT INTO users (id, name) VALUES (1, 'a')", &[])
            .unwrap();
        db.exec(
            "CREATE TABLE posts (id INTEGER PRIMARY KEY, owner INTEGER, title TEXT)",
            &[],
        )
        .unwrap();
        db.exec(
            "INSERT INTO posts (id, owner, title) VALUES (1, 1, 'x')",
            &[],
        )
        .unwrap();

        assert!(
            db.foreign_keys("posts").unwrap().is_empty(),
            "no FK at the start"
        );

        // Rebuild: ref:users.id on owner.
        let cols = vec![
            col("id", "serial", &["pk"]),
            col("owner", "int", &["ref:users.id"]),
            col("title", "str", &[]),
        ];
        db.rebuild_table("posts", &cols, &[], 42).unwrap();

        // The FK was added.
        let fks = db.foreign_keys("posts").unwrap();
        assert_eq!(fks.len(), 1);
        assert_eq!(fks[0].from, "owner");
        assert_eq!(fks[0].table, "users");
        // The data was preserved.
        let rows = db.query("SELECT title FROM posts", &[]).unwrap();
        assert_eq!(rows.len(), 1);
        // The FK is now enforced (an orphan insert is rejected).
        let orphan = db.exec("INSERT INTO posts (owner, title) VALUES (999, 'y')", &[]);
        assert!(orphan.is_err(), "orphan insert must violate the FK");
    }

    #[test]
    fn rebuild_twice_same_ts_no_backup_collision() {
        // Codex review: if a table is rebuilt twice within one second (the same `ts`)
        // (add ref -> remove it shortly after), the backup names must not collide.
        let db = mem_db("rebuild_ts_test");
        db.exec("CREATE TABLE users (id INTEGER PRIMARY KEY)", &[])
            .unwrap();
        db.exec("INSERT INTO users (id) VALUES (1)", &[]).unwrap();
        db.exec(
            "CREATE TABLE posts (id INTEGER PRIMARY KEY, owner INTEGER)",
            &[],
        )
        .unwrap();
        db.exec("INSERT INTO posts (id, owner) VALUES (1, 1)", &[])
            .unwrap();

        let with_fk = vec![
            col("id", "serial", &["pk"]),
            col("owner", "int", &["ref:users.id"]),
        ];
        let no_fk = vec![col("id", "serial", &["pk"]), col("owner", "int", &[])];

        // 1st rebuild: adds the FK (same ts=7).
        db.rebuild_table("posts", &with_fk, &[], 7).unwrap();
        assert_eq!(db.foreign_keys("posts").unwrap().len(), 1);
        // 2nd rebuild: removes the FK with the EXACT same ts — must pass without a collision.
        db.rebuild_table("posts", &no_fk, &[], 7).unwrap();
        assert!(db.foreign_keys("posts").unwrap().is_empty());

        // Both backups are preserved (different names: `_fk` and `_fk_2`).
        let baks = db
            .query(
                "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE '_fluxon_bak_posts_7_fk%' ORDER BY name",
                &[],
            )
            .unwrap();
        assert_eq!(
            baks.len(),
            2,
            "two rebuilds must leave two separate backups"
        );
        // The data is preserved.
        assert_eq!(db.query("SELECT id FROM posts", &[]).unwrap().len(), 1);
    }

    #[test]
    fn extract_from_table_basic() {
        // Simple cases: case insensitivity and following clauses (where) are separated.
        assert_eq!(
            extract_from_table("select * from users"),
            Some("users".to_string())
        );
        assert_eq!(
            extract_from_table("SELECT id FROM Bookings WHERE id = 1"),
            Some("Bookings".to_string())
        );
        assert_eq!(extract_from_table("select 1"), None);
    }

    #[test]
    fn extract_from_table_unicode_no_panic() {
        // Issue #88: characters that change byte length under lowercase
        // (for example `İ` U+0130) must not lead to a char-boundary panic.
        assert_eq!(
            extract_from_table("select İİ from té"),
            Some("té".to_string())
        );
        // There must be no panic when there is Unicode before the table name too.
        assert_eq!(
            extract_from_table("select * from naïve_таблица"),
            Some("naïve_таблица".to_string())
        );
    }

    #[test]
    fn extract_from_table_ignores_string_literal() {
        // Issue #88 addition: a ` from ` inside a string literal (`'...'`) must not
        // be taken as the table name — the name in the actual FROM clause is found.
        assert_eq!(
            extract_from_table("select * from posts where body like '% from secret %'"),
            Some("posts".to_string())
        );
        // If there is a ` from ` inside a literal but no FROM outside it — None.
        assert_eq!(extract_from_table("select '% from x %'"), None);
    }

    // Gets an int value from a row (SqlVal is not PartialEq — via match).
    fn row_int(rows: &[Row], col: &str) -> i64 {
        match rows[0].get(col) {
            Some(SqlVal::Int(n)) => *n,
            other => panic!("{col} should have been an int: {other:?}"),
        }
    }

    #[test]
    fn commit_failure_returns_clean_connection_to_pool() {
        // Issue #103: if COMMIT errors (on a deferred FK violation the transaction
        // stays open) the connection must be ROLLBACK'd and returned to the pool —
        // otherwise the next begin() gets "cannot start a transaction within a
        // transaction".
        let db = mem_db("commit_failure_clean_conn");
        db.exec("CREATE TABLE p (id INTEGER PRIMARY KEY)", &[])
            .unwrap();
        db.exec(
            "CREATE TABLE c (pid INTEGER REFERENCES p(id) DEFERRABLE INITIALLY DEFERRED)",
            &[],
        )
        .unwrap();

        let tx = db.begin().unwrap();
        // Deferred FK: the violation is detected at COMMIT time and COMMIT fails.
        tx.exec("INSERT INTO c (pid) VALUES (999)", &[]).unwrap();
        assert!(
            tx.commit().is_err(),
            "deferred FK violation must cause a COMMIT error"
        );

        // Returned CLEAN to the connection pool: a new tx opens (if it were dirty,
        // a "within a transaction" error would appear here)...
        let tx2 = db
            .begin()
            .unwrap_or_else(|e| panic!("dirty connection returned to the pool: {e}"));
        // ...and the orphan record was rolled back (it did not leak into the old open tx).
        let rows = tx2.query("SELECT count(*) AS n FROM c", &[]).unwrap();
        assert_eq!(row_int(&rows, "n"), 0, "orphan record must be rolled back");
        tx2.rollback().unwrap();
    }

    #[test]
    fn global_query_works_while_tx_holds_connection() {
        // The core promise of the pool design: while a tx holds a connection, a
        // global (tx-less) query keeps working by taking ANOTHER connection from the
        // pool and does not see the uncommitted record. A WAL snapshot is needed —
        // we use a file DB (shared-cache :memory: does not support WAL). We add the
        // PID to the file name: so the file does not collide even if two `cargo test`
        // processes run at once.
        let path = std::env::temp_dir().join(format!(
            "fluxon_dbmod_pool_promise_{}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let db = SqliteDb::open(path.to_str().unwrap()).unwrap();
        db.exec("CREATE TABLE t (id INTEGER)", &[]).unwrap();
        db.exec("INSERT INTO t (id) VALUES (1)", &[]).unwrap();

        let tx = db.begin().unwrap();
        tx.exec("INSERT INTO t (id) VALUES (2)", &[]).unwrap();

        // tx is still open — the global query is not blocked, it sees the old snapshot.
        let rows = db.query("SELECT count(*) AS n FROM t", &[]).unwrap();
        assert_eq!(
            row_int(&rows, "n"),
            1,
            "uncommitted record must not be visible"
        );

        tx.commit().unwrap();
        let rows = db.query("SELECT count(*) AS n FROM t", &[]).unwrap();
        assert_eq!(
            row_int(&rows, "n"),
            2,
            "record must be visible after commit"
        );
    }

    #[test]
    fn tx_guard_clears_thread_local_on_panic() {
        // Issue #103 (related): if a Rust-level panic happens inside the lambda the
        // guard must clear CURRENT_TX — so that when the spawn_blocking thread is
        // reused, the next request does not stay inside the old tx.
        let db = mem_db("tx_guard_clears_tl");
        db.exec("CREATE TABLE t (id INTEGER)", &[]).unwrap();

        let tx = db.begin().unwrap();
        CURRENT_TX.with(|c| *c.borrow_mut() = Some(tx));
        let r = std::panic::catch_unwind(|| {
            let _guard = TxClearGuard;
            panic!("artificial panic");
        });
        assert!(r.is_err());
        assert!(
            CURRENT_TX.with(|c| c.borrow().is_none()),
            "guard must clear CURRENT_TX"
        );
        // tx was rolled back via Drop and returned to the connection pool — a new tx
        // opens without trouble.
        let tx2 = db.begin().unwrap();
        tx2.rollback().unwrap();
    }
}
