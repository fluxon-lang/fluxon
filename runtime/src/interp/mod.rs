// Fluxon interpreter — directly executes the AST (tree-walking).
//
// Control flow (ret/skip/stop/fail) is propagated through the `Err` branch of
// Rust's `Result`: plain values are `Ok`, flow-interruptions are `Flow`. This
// bubbles up naturally with the `?` operator.
//
// This file is the module root: it holds the `Interp` struct + lifecycle (new,
// freeze_globals, db/migrate) and wires the submodules together. The 1829-line
// `impl Interp` is split across files using Rust's "one `impl Interp` block per
// file" pattern (same struct, methods in different `impl Interp` blocks):
//
//   scope   — Env/Parent/Scope + impl, Flow, EvalResult, CallDepthGuard
//   exec    — run/run_repl_chunk, exec_block/exec_stmt, =/<- bind, each
//   expr    — eval, lookup, try/catch, if/match
//   call    — binary ops, eval_call/apply_callee dispatch, list HOFs, apply, field/index
//   module  — use ./file loading, register_tbl
//   util    — migration error-swallowing, .env parsing, arithmetic helpers

mod call;
mod exec;
mod expr;
mod module;
mod scope;
mod util;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use parking_lot::RwLock;

use crate::value::Value;

// Re-exports preserving the public surface used by other modules: `value`/
// `par_mod`/`builtins` reach these as `crate::interp::X`.
pub use scope::{Env, EvalResult, Flow, Parent, Scope};

use scope::CURRENT_BASE;
use util::load_dotenv;

pub struct Interp {
    pub global: Env,
    // HTTP battery: registered routes. `http.on` fills it, `http.serve` reads
    // it. Arc<Mutex> — shared with server threads.
    pub routes: Arc<Mutex<Vec<crate::http_mod::Route>>>,
    // HTTP middleware (issue #67). `http.use` (for all routes) and `http.before`
    // (by path prefix) both append to THIS ONE list in order — so the chain runs
    // in DECLARATION ORDER (even when use/before are mixed; e.g. if before writes
    // req.ctx, a use logger declared after it sees the ctx). It runs BEFORE the
    // route handler; if one returns `fail`/`rep` the chain stops. Like `routes`,
    // top-level fills it, server threads read it.
    pub middlewares: Arc<Mutex<Vec<crate::http_mod::Middleware>>>,
    // CORS config (issue #135). `http.cors` sets it, server threads read it: when
    // enabled, an OPTIONS preflight is answered automatically and
    // `Access-Control-Allow-*` headers are added to every response. None — CORS
    // off (default, no headers added). Like `routes`, top-level fills it, server
    // threads read it.
    pub cors: Arc<Mutex<Option<crate::http_mod::CorsConfig>>>,
    // Static file mounts (issue #134). `http.static` fills it, server threads
    // read it: when no exact route is found, a file is served from the folder
    // matching the prefix (route priority: exact route > static). Like `routes`,
    // top-level fills it, server threads read it.
    pub statics: Arc<Mutex<Vec<crate::http_mod::StaticMount>>>,
    // A weak self-reference: `http.serve` needs `Arc<Interp>` to call handlers on
    // server threads. `eval_call` (&self) recovers it from here. `new_arc` sets
    // it.
    this: OnceLock<Weak<Interp>>,
    // Frozen global snapshot. Set when `http.serve` is called — after that the
    // top-level code is done and the global does not change. When `lookup`
    // reaches the root it reads from this LOCK-FREE (shared via Arc, no read
    // lock), so parallel requests do not block each other on global lookup.
    pub(crate) globals_frozen: OnceLock<Arc<HashMap<String, Value>>>,
    // DB battery: lazily-opened backend (one per process, selected via
    // `$DATABASE_URL`). Opened on the first `db.*` call + auto-migration.
    db: OnceLock<Arc<dyn crate::db_mod::Db>>,
    // tbl schema registry: table -> meta (columns + order + indexes). `Stmt::Tbl`
    // fills it, used for post-processing db results (sym/json/bool) and
    // auto-migration (diff: ADD/DROP COLUMN, CREATE/DROP INDEX). Arc<RwLock>:
    // written at top-level, read on parallel request threads.
    pub schema: Arc<RwLock<HashMap<String, TableMeta>>>,
    // Cache of column types introspected from the DB schema (table -> column ->
    // fluxon-type). A process that did NOT declare `tbl` (e.g. a reader in a
    // two-process setup) reconstructs a json column via this cache when `schema`
    // is empty — so json returns the same map regardless of the process boundary
    // (issue #63).
    pub(crate) db_schema: RwLock<HashMap<String, BTreeMap<String, String>>>,
    // .env file cache: LAZY — only on the first use of `env.X` is the `.env` in
    // the current directory read and parsed. If `env.X` is never used, the file
    // is NOT read (same philosophy as DB lazy-open). Priority: OS env > .env file
    // (the real environment variable matters on deploy).
    env_file: OnceLock<HashMap<String, String>>,
    // WS battery: event handlers + live connection/room/session state. Like http
    // `routes`, top-level code (`ws.on`) fills it, `ws.serve` threads read/write
    // it. Arc — shared with server threads.
    pub ws: Arc<crate::ws_mod::WsState>,
    // reg battery: name -> function registry (dynamic dispatch). `reg.add` fills
    // it, `reg.call` reads it (from any thread — even inside an http/ws handler).
    pub reg: Arc<crate::reg_mod::RegState>,
    // cron battery: scheduled tasks + a scheduler background thread. `cron.on`
    // registers (non-blocking), the background thread reads and calls the handler
    // on time.
    pub cron: Arc<crate::cron_mod::CronState>,
    // queue battery: a background queue + a single FIFO worker thread.
    // `queue.push` adds work (non-blocking), `queue.on` registers a handler; the
    // worker pulls from the queue and runs them in order.
    pub queue: Arc<crate::queue_mod::QueueState>,
    // Pending (deferred) servers: `http.serve`/`ws.serve` do not block
    // immediately, they add a server description here. Once top-level code is
    // done (end of `run`), they are all spawned on ONE shared tokio runtime — so
    // HTTP + WS run together in one process and `ws.room.send` can be called from
    // inside an HTTP handler.
    pub pending_servers: Arc<Mutex<Vec<crate::serve_mod::PendingServer>>>,
    // Cache for `use ./file` user modules: canonical path -> module namespace
    // (`Value::Map`). A module imported twice is not re-executed — it's run once
    // and the result is stored here (idempotent).
    pub(crate) module_cache: Mutex<HashMap<PathBuf, Value>>,
    // module_loading (cycle-detection stack) and current_base (current file's
    // directory) are NOT a process-wide Mutex but THREAD-LOCAL
    // (CURRENT_BASE/MODULE_LOADING, in `scope`). Reason: `par` calls each lambda
    // in a separate thread — if two lambdas `use ./m` the same uncached module, a
    // process-wide stack would show one's in-flight path as a cycle to the other
    // (a false "circular import"), and parallel base save/restore would corrupt
    // each other (issue #137 PR review). A cycle happens within one IMPORT CHAIN
    // (one thread), and the base is also the current execution context — both are
    // thread-local by nature (like CURRENT_TX). module_cache, by contrast, stays
    // SHARED: a module is loaded once and all threads share it.
}

// tbl column meta — type name (sym/json/bool conversion) + modifiers
// (CREATE TABLE: pk/uniq/null).
#[derive(Clone)]
pub struct ColMeta {
    pub type_name: String,
    pub modifiers: Vec<String>,
}

// tbl table meta — columns (name -> meta), declaration order (for stable ADD
// COLUMN) and indexes (for CREATE/DROP INDEX diff).
#[derive(Clone, Default)]
pub struct TableMeta {
    pub columns: BTreeMap<String, ColMeta>,
    pub col_order: Vec<String>,
    pub indexes: Vec<crate::db_mod::IndexDef>,
}

impl Interp {
    pub fn new() -> Self {
        let global = Scope::root();
        crate::builtins::install(&global);
        Interp {
            global,
            routes: Arc::new(Mutex::new(Vec::new())),
            middlewares: Arc::new(Mutex::new(Vec::new())),
            cors: Arc::new(Mutex::new(None)),
            statics: Arc::new(Mutex::new(Vec::new())),
            this: OnceLock::new(),
            globals_frozen: OnceLock::new(),
            db: OnceLock::new(),
            schema: Arc::new(RwLock::new(HashMap::new())),
            db_schema: RwLock::new(HashMap::new()),
            env_file: OnceLock::new(),
            ws: Arc::new(crate::ws_mod::WsState::new()),
            reg: Arc::new(crate::reg_mod::RegState::new()),
            cron: Arc::new(crate::cron_mod::CronState::new()),
            queue: Arc::new(crate::queue_mod::QueueState::new()),
            pending_servers: Arc::new(Mutex::new(Vec::new())),
            module_cache: Mutex::new(HashMap::new()),
            // module_loading/current_base — thread-local (see comment above).
        }
    }

    // Sets the top-level file's directory — `use ./file` paths are resolved
    // relative to it. main.rs calls this once before `run`.
    pub fn set_base(&self, dir: &std::path::Path) {
        CURRENT_BASE.with(|b| *b.borrow_mut() = dir.to_path_buf());
    }

    // The directory of the currently executing file. pub(crate): `http.static`
    // resolves a relative directory (`"./public"`) by the same rule as
    // `use ./file` — relative to the script file's directory.
    pub(crate) fn base_dir(&self) -> PathBuf {
        CURRENT_BASE.with(|b| b.borrow().clone())
    }

    // Looks up the value of `env.NAME`. Priority: OS env (std::env) > .env file.
    // The .env file is LAZY — read once on the first call and cached; if `env.X`
    // is never used, this method is not called -> the file is not read.
    // pub(crate): the `ai` battery reads `$AI_KEY`/`$AI_MODEL` this way (OS env >
    // .env) — the same priority rule as `env.X`.
    pub(crate) fn env_lookup(&self, name: &str) -> Value {
        if let Ok(v) = std::env::var(name) {
            return Value::Str(v); // OS env wins
        }
        let file = self.env_file.get_or_init(load_dotenv);
        match file.get(name) {
            Some(v) => Value::Str(v.clone()),
            None => Value::Nil, // not found -> `?? "default"`
        }
    }

    // log.<level> / bare `log` -> leveled log (issue #139). $LOG_LEVEL filters by
    // a minimum level, $LOG_FORMAT=json gives a structured (JSON) line.
    // env_lookup sees OS env + .env file (same convention as db/ai), so it's not
    // call_module — it needs the Interp.
    pub(crate) fn log_dispatch(&self, level: &str, argv: Vec<Value>) -> EvalResult {
        let min = match self.env_lookup("LOG_LEVEL") {
            Value::Str(s) => Some(s),
            _ => None,
        };
        let json = matches!(
            self.env_lookup("LOG_FORMAT"),
            Value::Str(s) if s.eq_ignore_ascii_case("json")
        );
        crate::builtins::emit_log(level, &argv, min.as_deref(), json);
        Ok(Value::Nil)
    }

    // Lazily opens the DB backend (on the first `db.*`). On opening it replays
    // the tbl schema registry and runs auto-migration (`CREATE TABLE IF NOT
    // EXISTS`) — tables declared with `tbl` appear with zero setup.
    pub fn db(&self) -> Result<Arc<dyn crate::db_mod::Db>, Flow> {
        if let Some(d) = self.db.get() {
            return Ok(d.clone());
        }
        let d = crate::db_mod::open_from_env().map_err(Flow::err)?;
        self.migrate(d.as_ref())?;
        // Race: if another thread also opened it, drop ours.
        let _ = self.db.set(d);
        Ok(self.db.get().unwrap().clone())
    }

    // Declarative auto-migration: `tbl` = the SINGLE SOURCE OF TRUTH for the DB
    // schema. It introspects the current DB state, computes the diff against the
    // `tbl` registry, and runs the necessary DDL:
    //   - new table   -> CREATE TABLE
    //   - new column  -> ADD COLUMN          (idempotent: silent pass if present)
    //   - removed column -> BACKUP + DROP COLUMN  (silent pass if absent)
    //   - index declaration -> CREATE/DROP INDEX IF [NOT] EXISTS
    //   - removed table -> BACKUP + DROP TABLE   (only Fluxon-created ones)
    //
    // CRITICAL: idempotent and does not break when coexisting with user manual
    // SQL — "bring it to the desired state, pass quietly if already so". Before
    // DROPs the table is copied to `_fluxon_bak_*` inside the DB (protection
    // against agent mistakes).
    fn migrate(&self, db: &dyn crate::db_mod::Db) -> Result<(), Flow> {
        use crate::db_mod::{
            ColDef, SqlVal, build_add_column, build_backup, build_create_index, build_drop_column,
            build_drop_index, coldef_foreign_key, index_name,
        };

        // 0. Registry of Fluxon-managed tables (for safe DROP).
        db.exec(
            "CREATE TABLE IF NOT EXISTS _fluxon_schema (table_name TEXT PRIMARY KEY)",
            &[],
        )
        .map_err(Flow::err)?;

        // Migration time for the backup name (unix secs). Only to make the
        // backup name unique — UNLIKE index names, determinism is not required
        // here.
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let schema = self.schema.read();

        // 1. For each registry table: CREATE + column/index diff.
        for (table, meta) in schema.iter() {
            // ColDefs in declaration order (stable ADD COLUMN).
            let coldef = |col: &str| -> ColDef {
                let m = &meta.columns[col];
                ColDef {
                    name: col.to_string(),
                    type_name: m.type_name.clone(),
                    modifiers: m.modifiers.clone(),
                }
            };
            let coldefs: Vec<ColDef> = meta.col_order.iter().map(|c| coldef(c)).collect();
            db.exec(&db.build_create_table(table, &coldefs), &[])
                .map_err(Flow::err)?;
            db.exec(
                "INSERT OR IGNORE INTO _fluxon_schema(table_name) VALUES (?1)",
                &[SqlVal::Text(table.clone())],
            )
            .map_err(Flow::err)?;

            // Current columns in the DB.
            let db_cols: HashSet<String> = db
                .column_types(table)
                .map_err(Flow::err)?
                .into_iter()
                .map(|(n, _)| n)
                .collect();

            // 2. ADD COLUMN: in the registry, not in the DB.
            for col in &meta.col_order {
                if !db_cols.contains(col) {
                    util::swallow_benign(db.exec(&build_add_column(table, &coldef(col)), &[]))?;
                }
            }

            // 3. DROP STALE INDEXES — BEFORE the column DROP. Reason: if an
            //    indexed column is removed, the old `idx_<tbl>_<col>` still exists
            //    in the DB and in some SQLite cases `DROP COLUMN` is rejected with
            //    "error in index ... after drop column: no such column" -> deploy
            //    cannot migrate. So we first drop Fluxon indexes that are NO
            //    LONGER needed, then safely drop the column.
            let want_names: HashSet<String> = meta.indexes.iter().map(index_name).collect();
            for info in db.fluxon_indexes(table).map_err(Flow::err)? {
                if !want_names.contains(&info.name) {
                    db.exec(&build_drop_index(&info.name), &[])
                        .map_err(Flow::err)?;
                }
            }

            // 4. DROP COLUMN: in the DB, not in the registry. BACKUP (once per
            //    table) -> DROP COLUMN (silent pass if absent).
            let mut backed_up = false;
            for dbcol in &db_cols {
                if !meta.columns.contains_key(dbcol) {
                    if !backed_up {
                        db.exec(&build_backup(table, ts), &[]).map_err(Flow::err)?;
                        backed_up = true;
                    }
                    util::swallow_benign(db.exec(&build_drop_column(table, dbcol), &[]))?;
                }
            }

            // 5. CREATE NEW INDEXES — AFTER the column DROP (new columns already
            //    exist). IF NOT EXISTS is idempotent.
            for idx in &meta.indexes {
                db.exec(&build_create_index(idx), &[]).map_err(Flow::err)?;
            }
        }

        // 5.5 FK RECONCILE — a SEPARATE pass (after all tables/columns are
        //     created, so the parent table is guaranteed to exist). We compare the
        //     ACTUAL FK set in the DB (introspection) against the `ref:tbl.col`
        //     declaration: we look not only at the code but at the existing state.
        //     If they differ (an FK added/removed on an existing column) ALTER is
        //     not enough — we rebuild the table (data preserved). A new column's FK
        //     was already applied in ADD COLUMN; this pass only closes the
        //     difference for existing columns.
        for (table, meta) in schema.iter() {
            let coldefs: Vec<ColDef> = meta
                .col_order
                .iter()
                .map(|c| ColDef {
                    name: c.clone(),
                    type_name: meta.columns[c].type_name.clone(),
                    modifiers: meta.columns[c].modifiers.clone(),
                })
                .collect();
            let desired: HashSet<_> = coldefs
                .iter()
                .filter_map(coldef_foreign_key)
                .map(|fk| (fk.from, fk.table, fk.to))
                .collect();
            let live: HashSet<_> = db
                .foreign_keys(table)
                .map_err(Flow::err)?
                .into_iter()
                .map(|fk| (fk.from, fk.table, fk.to))
                .collect();
            if desired != live {
                db.rebuild_table(table, &coldefs, &meta.indexes, ts)
                    .map_err(Flow::err)?;
            }
        }

        // 6. DROP TABLE: in `_fluxon_schema`, not in the registry (tbl removed
        //    from source). BACKUP -> DROP -> remove from the registry.
        //
        // IMPORTANT: if the registry is COMPLETELY empty (no `tbl` declared at
        //    all), we skip the DROP — such a process is NOT the schema conductor
        //    (it only reads/writes, e.g. a two-process setup). Otherwise it would
        //    drop every table created by another process.
        if schema.is_empty() {
            return Ok(());
        }
        for table in db.fluxon_tables().map_err(Flow::err)? {
            if !schema.contains_key(&table) {
                db.exec(&build_backup(&table, ts), &[]).map_err(Flow::err)?;
                db.exec(
                    &format!("DROP TABLE IF EXISTS {}", crate::db_mod::q_ident(&table)),
                    &[],
                )
                .map_err(Flow::err)?;
                db.exec(
                    "DELETE FROM _fluxon_schema WHERE table_name = ?1",
                    &[SqlVal::Text(table)],
                )
                .map_err(Flow::err)?;
            }
        }
        Ok(())
    }

    // Freezes the global scope into a lock-free snapshot. `http.serve` calls it
    // before starting the server. Once — after that reading the global is
    // lock-free. (Top-level code is done, no mutation expected.)
    pub fn freeze_globals(&self) {
        // The frozen snapshot is a HASHMAP — the global is large (builtins +
        // fn's) and it's looked up O(1) on every request. We build it from the
        // global Vec (last declaration wins).
        let mut snap: HashMap<String, Value> = HashMap::new();
        for (name, v, _) in self.global.read().vars.iter() {
            snap.insert(name.to_string(), v.clone());
        }
        let _ = self.globals_frozen.set(Arc::new(snap));
    }

    // Wraps the Interp in an Arc and sets the weak self-reference.
    pub fn new_arc() -> Arc<Self> {
        let arc = Arc::new(Self::new());
        let _ = arc.this.set(Arc::downgrade(&arc));
        arc
    }

    // Recovers `Arc<Interp>` from `&self` (for http.serve).
    pub fn arc_self(&self) -> Arc<Interp> {
        self.this
            .get()
            .and_then(|w| w.upgrade())
            .expect("Interp must be created via Arc (new_arc)")
    }
}
