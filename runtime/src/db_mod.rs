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
//
// This file is the module root: it wires the submodules together and re-exports the
// public surface that other modules (interp/par) reach as `crate::db_mod::X`.
//
//   values   — SqlVal/Row/ColDef/IndexDef/ForeignKey + the Db/DbTx traits
//   pool     — the SQLite connection pool, SqliteDb::open, open_from_env
//   sqlite   — value conversion, run_query/run_exec, impl Db for SqliteDb
//   migrate  — schema introspection, table rebuild, DDL + index-name builders
//   tx       — SqliteTx, the thread-local tx context, with_db router
//   interp   — the db.* dispatch, CRUD verbs, tx driver, query builder, conversion

mod interp;
mod migrate;
mod pool;
mod sqlite;
mod tx;
mod values;

// --- public surface (call sites use `crate::db_mod::X`) ---

// The dialect-neutral contract + value types — used by interp (schema/migration)
// and stored on the Interp.
pub use values::{ColDef, Db, IndexDef, SqlVal, coldef_foreign_key};

// Backend selection — interp opens the DB through this single entry point.
pub use pool::open_from_env;

// DDL + index-name builders used by interp's auto-migration.
pub use migrate::{
    build_add_column, build_backup, build_create_index, build_drop_column, build_drop_index,
    index_name,
};
// q_ident is crate-internal (`pub(crate)` in migrate) — interp's DROP TABLE DDL
// reaches it as `crate::db_mod::q_ident`, so re-export it crate-visibly.
pub(crate) use migrate::q_ident;

// par detects "am I inside a tx?" through this (issue #137).
pub(crate) use tx::tx_active;
