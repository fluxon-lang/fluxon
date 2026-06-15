// Active-transaction support: the `SqliteTx` that owns its connection, the
// thread-local current-tx context, and the `with_db` router that sends each db.*
// call either to the live tx or to the global pool.

use std::sync::Arc;

use rusqlite::Connection;

use crate::interp::{Flow, Interp};

use super::migrate::{q_ident, sqlite_column_types};
use super::pool::Pool;
use super::sqlite::{run_exec, run_query};
use super::values::{DbTx, Row, SqlVal};

// --- SqliteTx: an active transaction (owns the connection) ---

pub(crate) struct SqliteTx {
    conn: Option<Connection>,
    // The pool, for returning the connection (Arc clone — alive as long as the tx).
    pool: Arc<Pool>,
}

impl SqliteTx {
    // The connection arrives with `BEGIN IMMEDIATE` already issued (see Db::begin).
    pub(crate) fn new(conn: Connection, pool: Arc<Pool>) -> Self {
        SqliteTx {
            conn: Some(conn),
            pool,
        }
    }

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

// ==================== thread-local tx context ====================

// The active transaction on the current thread. HTTP runs each request on a separate
// spawn_blocking thread, so thread_local gives correct isolation.
thread_local! {
    pub(crate) static CURRENT_TX: std::cell::RefCell<Option<Box<dyn DbTx>>> =
        const { std::cell::RefCell::new(None) };
    // Nested SAVEPOINT depth (for a unique name).
    pub(crate) static TX_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

// A guard that clears CURRENT_TX even on the panic path during tx_outer. If a
// Rust-level panic occurred inside the lambda, the tx would be left in the
// thread_local; since tokio spawn_blocking threads are reused, the NEXT request could
// keep running inside the old tx (issue #103). The guard removes the tx —
// SqliteTx::Drop ROLLBACKs and returns the connection to the pool.
pub(crate) struct TxClearGuard;
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
pub(crate) fn with_db<T>(
    interp: &Interp,
    on_tx: impl FnOnce(&dyn DbTx) -> Result<T, String>,
    on_global: impl FnOnce(&dyn crate::db_mod::Db) -> Result<T, String>,
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
