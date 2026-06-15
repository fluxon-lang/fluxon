// SQLite schema introspection, the "12-step" table rebuild, and the DDL builders
// used by auto-migration (CREATE/ALTER/INDEX) plus the deterministic index-name hash.

use rusqlite::Connection;

use super::sqlite::{run_exec, sql_err};
use super::values::{ColDef, ForeignKey, IndexDef, IndexInfo, column_ref_target};

// Introspects SQLite table columns: takes the declared type from pragma_table_info
// and converts it to a Fluxon type name. Empty list if the table is missing.
pub(crate) fn sqlite_column_types(
    conn: &Connection,
    table: &str,
) -> Result<Vec<(String, String)>, String> {
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
pub(crate) fn sqlite_fluxon_indexes(
    conn: &Connection,
    table: &str,
) -> Result<Vec<IndexInfo>, String> {
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
pub(crate) fn sqlite_foreign_keys(
    conn: &Connection,
    table: &str,
) -> Result<Vec<ForeignKey>, String> {
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
pub(crate) fn sqlite_rebuild_table(
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
pub(crate) fn sqlite_column_def(c: &ColDef) -> String {
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
