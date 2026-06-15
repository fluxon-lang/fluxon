// SQLite value conversion, the low-level `run_query`/`run_exec` primitives, and the
// `impl Db for SqliteDb` that wires the dialect-neutral trait to rusqlite.

use std::collections::BTreeMap;

use rusqlite::types::{Value as RqVal, ValueRef};
use rusqlite::{Connection, params_from_iter};

use super::migrate::{
    build_create_table_sql, q_ident, sqlite_column_types, sqlite_fluxon_indexes,
    sqlite_foreign_keys, sqlite_rebuild_table,
};
use super::pool::SqliteDb;
use super::tx::SqliteTx;
use super::values::{ColDef, Db, DbTx, ForeignKey, IndexDef, IndexInfo, Row, SqlVal};

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
pub(crate) fn run_query(
    conn: &Connection,
    sql: &str,
    params: &[SqlVal],
) -> Result<Vec<Row>, String> {
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

pub(crate) fn run_exec(conn: &Connection, sql: &str, params: &[SqlVal]) -> Result<usize, String> {
    let binds: Vec<RqVal> = params.iter().map(to_rqval).collect();
    conn.execute(sql, params_from_iter(binds.iter()))
        .map_err(|e| sql_err(sql, e))
}

pub(crate) fn sql_err(sql: &str, e: rusqlite::Error) -> String {
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
        Ok(Box::new(SqliteTx::new(conn, self.pool.clone())))
    }
}
