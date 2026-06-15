// Backend-neutral value types and the dialect-neutral `Db`/`DbTx` trait contract.
// These are the shared vocabulary every backend and the interp dispatch speak.

use std::collections::BTreeMap;

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

// Extracts the FK target (table, column) from the `ref:tbl.col` modifier. None if
// not found. The first `ref:` modifier is used (a column has a single FK).
pub(crate) fn column_ref_target(modifiers: &[String]) -> Option<(&str, &str)> {
    modifiers
        .iter()
        .find_map(|m| m.strip_prefix("ref:"))
        .and_then(|t| t.split_once('.'))
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
