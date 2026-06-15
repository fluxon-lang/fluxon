// The SQLite connection pool and backend selection (the single config point that
// turns `$DATABASE_URL` into an `Arc<dyn Db>`).

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use super::values::Db;

// Connection pool — holds several connections. Tx-less operations (q/one/ins/
// up/del/put) CHECK OUT a connection from the pool and return it IMMEDIATELY; a tx
// holds the connection until commit/rollback. So even when ONE request is inside a
// tx, another PARALLEL request finds a global connection — no "connection busy"
// problem (user-approved design: each tx gets a separate connection).
//
// For `:memory:`, so that each connection does not end up as a SEPARATE empty DB, we
// use `file::memory:?cache=shared` and keep one "keepalive" connection open (the
// shared-cache DB is dropped when the last connection closes).
pub(crate) struct Pool {
    spec: String,               // the open specification passed to rusqlite
    flags: rusqlite::OpenFlags, // URI mode (shared-cache) when needed
    idle: Mutex<Vec<Connection>>,
    // Keeps the :memory: shared-cache DB alive. Mutex — Connection is not Sync,
    // but Pool (inside Arc<dyn Db>) must be Sync.
    _keepalive: Mutex<Option<Connection>>,
}

impl Pool {
    // Checks out a connection from the pool (opens a new one if none are idle).
    pub(crate) fn checkout(&self) -> Result<Connection, String> {
        if let Some(c) = self.idle.lock().unwrap().pop() {
            return Ok(c);
        }
        self.open_one()
    }
    // Returns a connection to the pool.
    pub(crate) fn checkin(&self, conn: Connection) {
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
    // `pub(super)` so the `impl Db for SqliteDb` in sqlite.rs can check connections
    // in/out, and `begin()` can hand the pool Arc to a SqliteTx.
    pub(super) pool: Arc<Pool>,
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
