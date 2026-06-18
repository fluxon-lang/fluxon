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
    // Optional `synchronous` override (e.g. "NORMAL"). Default (None) leaves
    // SQLite at its safe default (FULL under WAL) — durability over speed.
    // Opt in to NORMAL/OFF only when you knowingly trade durability for write
    // throughput (e.g. caches, benchmarks). Parsed once from DATABASE_URL.
    synchronous: Option<String>,
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
        // Optional durability override. Default leaves SQLite at FULL (safe).
        // The value is already validated in `parse_synchronous`, so this PRAGMA
        // cannot carry arbitrary user input (no injection).
        if let Some(sync) = &self.synchronous {
            let _ = conn.execute_batch(&format!("PRAGMA synchronous={sync};"));
        }
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
        // Pull off a Fluxon-level `?synchronous=...` knob before handing the rest
        // to rusqlite. We strip it ourselves (rather than relying on SQLite URI
        // parsing) so it works for both plain paths and `file:` URIs, and so the
        // value can be validated against SQL injection.
        let (rest, synchronous) = parse_synchronous(rest)?;
        let rest = rest.as_str();

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
            synchronous,
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

// Strips a `synchronous=<level>` query parameter out of the sqlite `rest` string
// and validates it. Returns (rest_without_that_param, Option<level>).
//
// Accepts it as the sole `?synchronous=...` or as one of several `?a=b&...`
// params; any OTHER params are preserved on the returned `rest` (so `file:` URI
// options still reach rusqlite). Default — no param — yields None, leaving SQLite
// at its safe FULL default.
//
// The level is validated against the fixed SQLite set, so the value spliced into
// the `PRAGMA synchronous=...` batch can never be arbitrary input.
fn parse_synchronous(rest: &str) -> Result<(String, Option<String>), String> {
    let Some((base, query)) = rest.split_once('?') else {
        return Ok((rest.to_string(), None));
    };
    let mut sync = None;
    let mut kept = Vec::new();
    for pair in query.split('&') {
        match pair.split_once('=') {
            Some((k, v)) if k.eq_ignore_ascii_case("synchronous") => {
                let level = v.to_ascii_uppercase();
                // SQLite's full set. OFF/NORMAL trade durability for write speed.
                if !matches!(level.as_str(), "OFF" | "NORMAL" | "FULL" | "EXTRA") {
                    return Err(format!(
                        "invalid synchronous={v}: expected OFF, NORMAL, FULL, or EXTRA"
                    ));
                }
                sync = Some(level);
            }
            _ => kept.push(pair),
        }
    }
    // Rebuild `rest` without the synchronous param (keep other query options).
    let rebuilt = if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", kept.join("&"))
    };
    Ok((rebuilt, sync))
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

#[cfg(test)]
mod tests {
    use super::parse_synchronous;

    #[test]
    fn no_param_keeps_path_and_defaults_to_none() {
        let (rest, sync) = parse_synchronous("bench.db").unwrap();
        assert_eq!(rest, "bench.db");
        assert_eq!(sync, None);
    }

    #[test]
    fn synchronous_is_stripped_and_uppercased() {
        let (rest, sync) = parse_synchronous("bench.db?synchronous=normal").unwrap();
        assert_eq!(rest, "bench.db");
        assert_eq!(sync.as_deref(), Some("NORMAL"));
    }

    #[test]
    fn other_query_params_are_preserved() {
        // synchronous removed, cache=shared kept (so `file:` URI options survive).
        let (rest, sync) = parse_synchronous("file:bench.db?cache=shared&synchronous=OFF").unwrap();
        assert_eq!(rest, "file:bench.db?cache=shared");
        assert_eq!(sync.as_deref(), Some("OFF"));
    }

    #[test]
    fn invalid_level_is_rejected() {
        // Guards the PRAGMA splice against arbitrary input.
        let err = parse_synchronous("bench.db?synchronous=DROP TABLE").unwrap_err();
        assert!(err.contains("invalid synchronous"), "got: {err}");
    }
}
