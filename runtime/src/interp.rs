// Fluxon interpreter — directly executes the AST (tree-walking).
//
// Control flow (ret/skip/stop/fail) is propagated through the `Err` branch of
// Rust's `Result`: plain values are `Ok`, flow-interruptions are `Flow`. This
// bubbles up naturally with the `?` operator.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use parking_lot::RwLock;

use crate::ast::*;
use crate::value::{FnValue, Value};

// Lexical scope: a chain linked to the parent environment. Arc<RwLock<>> — for
// closures, mutation AND sharing across threads (true parallel HTTP). RwLock
// (not Mutex): lookup/read allows many readers in parallel, so parallel
// requests read functions in the global scope (e.g. recursive `fib`) without
// blocking each other. Writes (`<-`, bind) are exclusive.
pub type Env = Arc<RwLock<Scope>>;

// Parent link of the scope chain. Important: the ROOT (global) scope is SHARED
// across all threads — cloning/locking it on every lookup is the main source of
// atomic contention (cache-line bouncing on 8 cores). So the chain reaching the
// root uses `Parent::Root(env)`: the root Arc is preserved (intermediate scopes
// NEVER clone it), and once the global is frozen, lookup reads from a lock-free
// frozen snapshot WITHOUT TOUCHING the root Arc.
#[derive(Clone)]
pub enum Parent {
    // The root scope itself — no parent above.
    None,
    // Parent is the root (global) scope. A MARKER (not an Arc!) — the root Arc
    // is not held, so a fn call / scope opening does not ATOMICALLY bump the
    // root refcount (no cache-line bouncing). Once frozen, lookup reads from the
    // frozen snapshot; when not frozen (top-level) it reads from the
    // `Interp.global` Arc — both via `&self`, no clone needed.
    Root,
    // Parent is a plain (non-root) scope.
    Scope(Env),
}

pub struct Scope {
    // Names — a small VECTOR (not a HashMap). Fn-call / block scopes usually
    // hold 0-4 names; for such a small set a linear scan beats computing a hash
    // + a HashMap allocation, and the per-call allocation is cheap (one Vec
    // buffer instead of two empty HashMaps). Element: (name, value, is-mutable).
    // mutable = whether it can be re-bound with `<-` (`=`/`exp`/param are
    // immutable; `<-` and loop vars are mutable).
    vars: Vec<(Box<str>, Value, bool)>,
    parent: Parent,
    // Is this scope the root (global)? When lookup reaches the root, if the
    // Interp has frozen the global it reads from a lock-free snapshot (no
    // parallel contention).
    is_root: bool,
    // Is this scope an fn/lambda call boundary? An `=` bind looking up an outer
    // variable stops here (function isolation/shadowing). if/each/match blocks
    // are `false` — they are lexically TRANSPARENT: inside them an `=` can
    // update an outer variable (within the same fn).
    is_fn_boundary: bool,
}

impl Scope {
    pub fn root() -> Env {
        Arc::new(RwLock::new(Scope {
            vars: Vec::new(),
            parent: Parent::None,
            is_root: true,
            is_fn_boundary: false,
        }))
    }
    // A new (empty) child scope under the given `Parent` link. `apply`/`if`/
    // `each`/`match` open scopes through this. IMPORTANT: it does NOT LOCK the
    // parent — the link type (Root/Scope) comes from the caller, so a recursive
    // fn call never touches the root Arc at all (no contention).
    fn child(parent: Parent) -> Env {
        Arc::new(RwLock::new(Scope {
            vars: Vec::new(),
            parent,
            is_root: false,
            is_fn_boundary: false, // if/each/match — transparent block
        }))
    }
    // A child pre-sized by the number of params (fn call — no re-allocation
    // during bind).
    fn child_with_capacity(parent: Parent, cap: usize) -> Env {
        Arc::new(RwLock::new(Scope {
            vars: Vec::with_capacity(cap),
            parent,
            is_root: false,
            is_fn_boundary: true, // fn/lambda call — isolation boundary
        }))
    }
    // Turns the `env` Arc into a parent link for a child (a single lock, only to
    // learn `is_root`). Top-level code (if/each/match in the global env) goes
    // through this — single-threaded, contention-free. A fn call instead uses
    // `FnValue.parent` (Parent) directly and does not enter this path.
    fn parent_link(env: &Env) -> Parent {
        if env.read().is_root {
            Parent::Root
        } else {
            Parent::Scope(env.clone())
        }
    }
    // A child under the given env (combines the two above).
    fn child_of(env: &Env) -> Env {
        Scope::child(Scope::parent_link(env))
    }
    // Declares a name. If it already exists, updates value + mutable
    // (shadow/re-bind — the old HashMap insert semantics: last one wins).
    fn define(&mut self, name: &str, v: Value, mutable: bool) {
        for slot in self.vars.iter_mut() {
            if &*slot.0 == name {
                slot.1 = v;
                slot.2 = mutable;
                return;
            }
        }
        self.vars.push((name.into(), v, mutable));
    }
    // Reads a name's value (from the last declaration — scanning back to front).
    fn get(&self, name: &str) -> Option<&Value> {
        self.vars
            .iter()
            .rev()
            .find(|(n, _, _)| &**n == name)
            .map(|(_, v, _)| v)
    }
    // For `<-`: finds the mutable slot. Returns (slot, is-mutable).
    fn get_mut_entry(&mut self, name: &str) -> Option<(&mut Value, bool)> {
        self.vars
            .iter_mut()
            .rev()
            .find(|(n, _, _)| &**n == name)
            .map(|(_, v, m)| (v, *m))
    }
    // For installing builtins: sets an immutable value on a global name.
    pub fn set_global(&mut self, name: &str, v: Value) {
        self.define(name, v, false);
    }
}

// Flow-interruption signals and errors. All travel on the `Err` side.
pub enum Flow {
    Return(Value),
    Skip,
    Stop,
    // fail [status] message — a business or internal error.
    Fail {
        status: Option<i64>,
        message: String,
    },
    // A plain runtime error (type mismatch, unknown variable, ...).
    Error(String),
}

impl Flow {
    pub fn err(msg: impl Into<String>) -> Flow {
        Flow::Error(msg.into())
    }

    // The single error for i64 arithmetic going out of range (issue #89). Used
    // together with checked_*: instead of a debug panic and a silent wrap in
    // release, it gives the same explicit runtime error in both modes.
    pub fn overflow(who: &str) -> Flow {
        Flow::Error(format!("{}: number out of range (i64)", who))
    }
}

pub type EvalResult = Result<Value, Flow>;
type ExecResult = Result<Value, Flow>; // a block returns its last expression's value

// Maximum depth for Fluxon-level fn calls. The native stack grows in segments
// via `stacker::maybe_grow`, so the real limit is this counter: on reaching the
// limit it's a graceful Flow::err, not an abort. 1000 is in the same ballpark
// as Python's default recursion limit; real backend code does not recurse
// deeper than this, while infinite recursion is caught quickly.
const MAX_CALL_DEPTH: usize = 1000;

// stacker parameters: the red zone must be larger than the native stack that
// can be used within one Fluxon call (until the next check) — measured at
// ~15KB/level in a debug build. The segment size — each allocation fits ~130
// levels, so a few segments suffice for 1000 levels.
const STACK_RED_ZONE: usize = 128 * 1024;
const STACK_GROW_SIZE: usize = 2 * 1024 * 1024;

thread_local! {
    // Fluxon call depth on the current thread. Thread-local: each HTTP request
    // runs in its own spawn_blocking thread — one request's recursion does not
    // count toward another's. A field cannot be added to Interp (&self, Sync — a
    // Cell is not possible).
    static CALL_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };

    // For `use ./file`: the current file's directory and the cycle-detection
    // stack — THREAD-LOCAL (not an Interp field). `par` calls each lambda in a
    // separate thread, so parallel module loading must not corrupt each other's
    // base / in-flight stack. The base defaults to the current working directory
    // (`set_base` pins it to the top-level file); `par` snapshots the parent
    // thread's base into the new thread. The loading stack starts empty on each
    // thread (each par lambda is an independent import chain). module_cache, by
    // contrast, is shared in Interp — a loaded module is shared.
    static CURRENT_BASE: std::cell::RefCell<PathBuf> =
        std::cell::RefCell::new(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    static MODULE_LOADING: std::cell::RefCell<Vec<PathBuf>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

// RAII guard: bumps the counter on enter, decrements on Drop. Without Drop, on
// an error (`?`) or panic path the counter would stay elevated and poison
// subsequent requests once the spawn_blocking thread is reused.
struct CallDepthGuard;

impl CallDepthGuard {
    fn enter(fname: &str) -> Result<CallDepthGuard, Flow> {
        CALL_DEPTH.with(|d| {
            let depth = d.get();
            if depth >= MAX_CALL_DEPTH {
                return Err(Flow::err(format!(
                    "recursion too deep: '{}' call reached the {} level limit",
                    fname, MAX_CALL_DEPTH
                )));
            }
            d.set(depth + 1);
            Ok(CallDepthGuard)
        })
    }
}

impl Drop for CallDepthGuard {
    fn drop(&mut self) {
        CALL_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

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
    globals_frozen: OnceLock<Arc<HashMap<String, Value>>>,
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
    module_cache: Mutex<HashMap<PathBuf, Value>>,
    // module_loading (cycle-detection stack) and current_base (current file's
    // directory) are NOT a process-wide Mutex but THREAD-LOCAL
    // (CURRENT_BASE/MODULE_LOADING, below). Reason: `par` calls each lambda in a
    // separate thread — if two lambdas `use ./m` the same uncached module, a
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
    fn log_dispatch(&self, level: &str, argv: Vec<Value>) -> EvalResult {
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
                    swallow_benign(db.exec(&build_add_column(table, &coldef(col)), &[]))?;
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
                    swallow_benign(db.exec(&build_drop_column(table, dbcol), &[]))?;
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

    pub fn run(&self, prog: &Program) -> Result<(), String> {
        // First pass: pre-register top-level fn/tbl declarations (hoisting), so
        // they can call each other regardless of order and the schema is ready
        // before any `db.*` call.
        for stmt in prog {
            match stmt {
                Stmt::FnDecl {
                    name, params, body, ..
                } => {
                    let f = Value::Fn(Arc::new(FnValue {
                        params: params.clone(),
                        body: body.clone(),
                        // Top-level fn — parent is root (a marker, not an Arc).
                        parent: Parent::Root,
                        name: name.clone(),
                    }));
                    self.global.write().define(name, f, false);
                }
                Stmt::Tbl {
                    name,
                    columns,
                    indexes,
                } => self.register_tbl(name, columns, indexes),
                _ => {}
            }
        }
        for stmt in prog {
            // fn/tbl already registered — we do not re-execute them.
            if matches!(stmt, Stmt::FnDecl { .. } | Stmt::Tbl { .. }) {
                continue;
            }
            match self.exec_stmt(stmt, &self.global.clone()) {
                Ok(_) => {}
                Err(Flow::Error(e)) => return Err(e),
                Err(Flow::Fail { status, message }) => {
                    let pfx = status.map(|s| format!("[{}] ", s)).unwrap_or_default();
                    return Err(format!("fail: {}{}", pfx, message));
                }
                Err(Flow::Return(_)) => {} // top-level ret — ignored
                Err(Flow::Skip) | Err(Flow::Stop) => {
                    return Err("skip/stop used outside a loop".into());
                }
            }
        }
        // Top-level is done — if there are pending servers (http.serve/ws.serve),
        // start them all on one shared event-loop and block. With no server it
        // returns immediately (a plain script ends normally). run_pending only
        // returns Flow::Error (when it cannot build the tokio runtime).
        if let Err(Flow::Error(e)) = crate::serve_mod::run_pending(&self.arc_self()) {
            return Err(e);
        }
        // With no server run_pending returns immediately — before exiting we wait
        // for background-queue work to finish (issue #105: a queue-only script
        // must not exit without doing the work). With a server, run_pending
        // blocks and we never reach here at all.
        self.queue_wait_drain();
        Ok(())
    }

    // The REPL executes one entered block and returns the last expression's
    // VALUE (for printing) — whereas `run` returns `()`. It is called repeatedly
    // on the same interp object, so declarations (`x = 1`, `fn f ...`) persist
    // across chunks: everything lives in `self.global`. Unlike `run`, here
    // `run_pending`/`queue_wait_drain` are NOT called — even if a REPL line has
    // `http.serve`, the event-loop must not start on each chunk and block the
    // prompt (interactive session, not a script). fn/tbl hoisting is the same as
    // `run` — within a chunk they can call each other regardless of order.
    pub fn run_repl_chunk(&self, prog: &Program) -> Result<Value, String> {
        for stmt in prog {
            match stmt {
                Stmt::FnDecl {
                    name, params, body, ..
                } => {
                    let f = Value::Fn(Arc::new(FnValue {
                        params: params.clone(),
                        body: body.clone(),
                        parent: Parent::Root,
                        name: name.clone(),
                    }));
                    self.global.write().define(name, f, false);
                }
                Stmt::Tbl {
                    name,
                    columns,
                    indexes,
                } => self.register_tbl(name, columns, indexes),
                _ => {}
            }
        }
        let mut last = Value::Nil;
        for stmt in prog {
            if matches!(stmt, Stmt::FnDecl { .. } | Stmt::Tbl { .. }) {
                continue;
            }
            match self.exec_stmt(stmt, &self.global.clone()) {
                Ok(v) => last = v,
                Err(Flow::Error(e)) => return Err(e),
                Err(Flow::Fail { status, message }) => {
                    let pfx = status.map(|s| format!("[{}] ", s)).unwrap_or_default();
                    return Err(format!("fail: {}{}", pfx, message));
                }
                Err(Flow::Return(_)) => {} // top-level ret — e'tiborsiz
                Err(Flow::Skip) | Err(Flow::Stop) => {
                    return Err("skip/stop used outside a loop".into());
                }
            }
        }
        Ok(last)
    }

    // tbl e'lonini schema registry'ga yozadi (ustunlar + tartib + indekslar).
    fn register_tbl(&self, name: &str, columns: &[TblColumn], indexes: &[TblIndex]) {
        let mut cols = BTreeMap::new();
        let mut col_order = Vec::with_capacity(columns.len());
        for c in columns {
            cols.insert(
                c.name.clone(),
                ColMeta {
                    type_name: c.type_name.clone(),
                    modifiers: c.modifiers.clone(),
                },
            );
            col_order.push(c.name.clone());
        }
        let idx_defs = indexes
            .iter()
            .map(|i| crate::db_mod::IndexDef {
                table: name.to_string(),
                columns: i.columns.clone(),
                unique: i.unique,
            })
            .collect();
        self.schema.write().insert(
            name.to_string(),
            TableMeta {
                columns: cols,
                col_order,
                indexes: idx_defs,
            },
        );
    }

    // `use ./fayl` — foydalanuvchi modulini yuklab namespace `Value::Map` qaytaradi.
    // Yo'l joriy fayl katalogiga (`current_base`) nisbatan hal qilinadi. Cache va
    // sikllik import himoyasi shu yerda. Faqat `exp` qilingan nomlar namespace'ga
    // kiradi (qolganlari modul-private).
    fn load_module(&self, rel_path: &str) -> EvalResult {
        // 1. To'liq yo'lni quramiz: base + nisbiy yo'l, .fx kengaytmasi qo'shamiz.
        let base = self.base_dir();
        let mut full = base.join(rel_path);
        if full.extension().is_none() {
            full.set_extension("fx");
        }
        // canonicalize: cache/sikl kaliti barqaror bo'lishi uchun (symlink/`..`
        // normallashtiriladi). Fayl yo'q bo'lsa shu yerda xato beradi.
        let canon = full
            .canonicalize()
            // Xato xabarida foydalanuvchi yozган yo'lni ko'rsatamiz (`./greet`),
            // normallashtirilmagan to'liq yo'lni emas — o'qishga qulayroq.
            .map_err(|e| Flow::err(format!("module not found '{}': {}", rel_path, e)))?;

        // 2. Cache hit — qayta bajarmaymiz (idempotent import).
        if let Some(v) = self.module_cache.lock().unwrap().get(&canon) {
            return Ok(v.clone());
        }

        // 3. Sikllik import: agar bu modul SHU THREAD'DA hozir yuklanish
        //    jarayonida bo'lsa (A -> B -> A), to'xtaymiz — aks holda cheksiz
        //    rekursiya. Steki thread-local: `par` parallel import'lari bir-birini
        //    sikl deb ko'rmaydi (har lambda mustaqil zanjir).
        let cycle = MODULE_LOADING.with(|l| {
            let loading = l.borrow();
            loading.contains(&canon).then(|| {
                loading
                    .iter()
                    .chain(std::iter::once(&canon))
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            })
        });
        if let Some(chain) = cycle {
            return Err(Flow::err(format!("circular import: {}", chain)));
        }
        MODULE_LOADING.with(|l| l.borrow_mut().push(canon.clone()));

        // 4. Faylni bajaramiz. Natijadan qat'i nazar steki'dan olib tashlaymiz.
        let result = self.run_module_file(&canon);
        MODULE_LOADING.with(|l| {
            l.borrow_mut().pop();
        });
        let ns = result?;

        // 5. Cache'ga yozamiz (closure Arc'lar shared — ikkinchi import klon oladi).
        self.module_cache.lock().unwrap().insert(canon, ns.clone());
        Ok(ns)
    }

    // Modul faylini o'qib parse qilib, alohida modul scope'da bajaradi va
    // `exp` qilingan nomlardan namespace `Value::Map` quradi. `current_base`'ni
    // modul katalogiga vaqtincha o'rnatadi (nested import uchun), tugagach tiklaydi.
    fn run_module_file(&self, canon: &std::path::Path) -> EvalResult {
        let src = std::fs::read_to_string(canon).map_err(|e| {
            Flow::err(format!(
                "could not read module '{}': {}",
                canon.display(),
                e
            ))
        })?;
        let toks = crate::lexer::lex(&src).map_err(Flow::err)?;
        let prog = crate::parser::parse(toks).map_err(Flow::err)?;

        // Modul scope — global'ning child'i: builtin'lar (`log`/`rep`) va top-level
        // fn'lar lookup zanjiri orqali ko'rinadi, lekin modulning o'z `exp`/`=`
        // nomlari avval qidiriladi (shadowing — izolyatsiya yetarli).
        let mod_scope = Scope::child_of(&self.global);

        // base'ni modul katalogiga o'rnatamiz — modul ichidagi `use ./...` shu
        // modulga nisbatan hal qilinsin. Save/restore: nested import qaytib
        // chiqqanda ota-modul base'i tiklanadi (xato yo'lida ham).
        let prev_base = self.base_dir();
        if let Some(dir) = canon.parent() {
            self.set_base(dir);
        }
        let exec = self.exec_module_body(&prog, &mod_scope);
        self.set_base(&prev_base);
        exec?;

        // Faqat eksport qilingan nomlarni yig'amiz: `exp NAME =` va `exp fn`.
        let exported = collect_exported(&prog);
        let mut ns = BTreeMap::new();
        for (name, v, _) in mod_scope.read().vars.iter() {
            if exported.contains(&**name) {
                ns.insert(name.to_string(), v.clone());
            }
        }
        Ok(Value::Map(ns))
    }

    // Modul tanasini berilgan scope'da bajaradi. `run`dan farqi:
    //  • fn'lar `Parent::Scope(mod_scope)` HAQIQIY Arc bilan saqlanadi
    //    (Parent::Root marker EMAS) — shunda modul fn'i apply qilinganda
    //    import qiluvchi global'ga emas, o'z modul scope'iga (`exp greeting`)
    //    boradi. Bu closure capture'ning to'g'ri ishlashi uchun MAJBURIY.
    //  • `run_pending` chaqirmaydi — modul ichidagi `http.serve`/`ws.serve`
    //    bir xil Interp'ning `pending_servers`'iga qo'shiladi (chunki
    //    `arc_self` o'sha Interp), top-level oxirida bir marta ishga tushadi.
    //
    // Eslatma (ataylab qabul qilingan leak): modul scope o'z `vars`ida fn'larni,
    // fn'lar esa `Parent::Scope(mod_scope)` orqali modul scope'ni ushlaydi —
    // Arc sikli. Modullar process umri davomida tirik kerak (HTTP handler'lar
    // ulardan foydalanadi), shuning uchun bu drop bo'lmasligi maqsadga muvofiq.
    fn exec_module_body(&self, prog: &Program, scope: &Env) -> Result<(), Flow> {
        // Hoisting — fn/tbl oldindan ro'yxatga (tartibdan qat'i nazar bir-birini
        // chaqira oladi). `run`dagidan farqi: parent modul scope (Arc).
        for stmt in prog {
            match stmt {
                Stmt::FnDecl {
                    name, params, body, ..
                } => {
                    let f = Value::Fn(Arc::new(FnValue {
                        params: params.clone(),
                        body: body.clone(),
                        parent: Scope::parent_link(scope),
                        name: name.clone(),
                    }));
                    scope.write().define(name, f, false);
                }
                Stmt::Tbl {
                    name,
                    columns,
                    indexes,
                } => self.register_tbl(name, columns, indexes),
                _ => {}
            }
        }
        for stmt in prog {
            if matches!(stmt, Stmt::FnDecl { .. } | Stmt::Tbl { .. }) {
                continue;
            }
            match self.exec_stmt(stmt, scope) {
                Ok(_) => {}
                Err(Flow::Error(e)) => return Err(Flow::Error(e)),
                Err(Flow::Fail { status, message }) => {
                    let pfx = status.map(|s| format!("[{}] ", s)).unwrap_or_default();
                    return Err(Flow::err(format!("fail: {}{}", pfx, message)));
                }
                Err(Flow::Return(_)) => {} // modul top-level ret — e'tiborsiz
                Err(Flow::Skip) | Err(Flow::Stop) => {
                    return Err(Flow::err("skip/stop used outside a loop"));
                }
            }
        }
        Ok(())
    }

    // Blokni ketma-ket bajaradi; qiymati — oxirgi ifoda (Fluxon'da blok ifoda).
    fn exec_block(&self, stmts: &[Stmt], env: &Env) -> ExecResult {
        let mut last = Value::Nil;
        for s in stmts {
            last = self.exec_stmt(s, env)?;
        }
        Ok(last)
    }

    fn exec_stmt(&self, stmt: &Stmt, env: &Env) -> ExecResult {
        match stmt {
            Stmt::Bind { name, value } => {
                let v = self.eval(value, env)?;
                self.bind(name, v, env)?;
                Ok(Value::Nil)
            }
            Stmt::Assign { target, value } => {
                let v = self.eval(value, env)?;
                match target.as_ref() {
                    // `x <- v` — oddiy o'zgaruvchi qayta tayinlash (eski yo'l).
                    Expr::Ident(name) => self.assign(name, v, env)?,
                    // `req.ctx <- v` — shared ctx cell'ga yozish (issue #68).
                    Expr::Field { target: obj, name } => {
                        let obj_val = self.eval(obj, env)?;
                        self.assign_field(&obj_val, name, v)?;
                    }
                    _ => {
                        return Err(Flow::err("'<-' left side must be a variable or '.field'"));
                    }
                }
                Ok(Value::Nil)
            }
            Stmt::ExpBind { name, value } => {
                let v = self.eval(value, env)?;
                // exp bind — eksport qilinadigan global; immutable (`=` kabi).
                env.write().define(name, v, false);
                Ok(Value::Nil)
            }
            Stmt::FnDecl {
                name, params, body, ..
            } => {
                let f = Value::Fn(Arc::new(FnValue {
                    params: params.clone(),
                    body: body.clone(),
                    parent: Scope::parent_link(env),
                    name: name.clone(),
                }));
                env.write().define(name, f, false);
                Ok(Value::Nil)
            }
            Stmt::Ret(opt) => {
                let v = match opt {
                    Some(e) => self.eval(e, env)?,
                    None => Value::Nil,
                };
                Err(Flow::Return(v))
            }
            Stmt::Skip => Err(Flow::Skip),
            Stmt::Stop => Err(Flow::Stop),
            Stmt::Fail { status, message } => {
                let st = match status {
                    Some(e) => match self.eval(e, env)? {
                        Value::Int(n) => Some(n),
                        other => {
                            return Err(Flow::err(format!(
                                "fail status must be an int, got {}",
                                other.type_name()
                            )));
                        }
                    },
                    None => None,
                };
                let msg = self.eval(message, env)?;
                Err(Flow::Fail {
                    status: st,
                    message: format!("{}", msg),
                })
            }
            Stmt::Each { vars, iter, body } => self.exec_each(vars, iter, body, env),
            Stmt::Expr(e) => self.eval(e, env),
            // use — modul import. Ikki xil:
            //  • Batareya (`use http`, `use db`) — dispatch nom asosida ishlaydi,
            //    ro'yxatga olish SHART EMAS, shuning uchun no-op.
            //  • Foydalanuvchi fayli (`use ./tools`, `use ../lib/x as y`) — faylni
            //    o'qib, alohida modul scope'da bajarib, `exp` qilingan nomlarni
            //    `tools.nom` (yoki alias) ostida joriy scope'ga bog'laydi.
            Stmt::Use { items } => {
                for item in items {
                    // Nisbiy yo'l (`.`/`..` bilan boshlanadi) — foydalanuvchi fayli.
                    // Aks holda batareya nomi (no-op, eski xatti-harakat).
                    if !is_user_module_path(&item.path) {
                        continue;
                    }
                    let ns = self.load_module(&item.path)?;
                    // Bog'lash nomi: alias bo'lsa o'sha, aks holda yo'l "bazasi"
                    // (`./lib/greet` -> `greet`).
                    let name = item
                        .alias
                        .clone()
                        .unwrap_or_else(|| module_basename(&item.path));
                    env.write().define(&name, ns, false);
                }
                Ok(Value::Nil)
            }
            // tbl — schema registry'ga yoziladi (sym/json konversiya + migration).
            Stmt::Tbl {
                name,
                columns,
                indexes,
            } => {
                self.register_tbl(name, columns, indexes);
                Ok(Value::Nil)
            }
        }
    }

    // `<-` qayta tayinlash: o'zgaruvchini scope zanjirida topib yangilaydi.
    // Topilmasa — joriy scope'da mutable sifatida yaratadi.
    fn assign(&self, name: &str, v: Value, env: &Env) -> Result<(), Flow> {
        let mut cur = env.clone();
        loop {
            // Bitta write lock ostida: nomni topib yangilash YOKI keyingi ota'ni
            // olish (avval write + alohida read — ikki lock har leveldda edi).
            let parent = {
                let mut s = cur.write();
                if let Some((slot, mutable)) = s.get_mut_entry(name) {
                    if !mutable {
                        return Err(Flow::err(format!(
                            "'{}' is immutable (declared with =), cannot be changed with '<-'",
                            name
                        )));
                    }
                    *slot = v;
                    return Ok(());
                }
                s.parent.clone()
            };
            match parent {
                Parent::Scope(p) => cur = p,
                // Ota — root (marker). Muzlatilgandan keyin global FROZEN
                // (immutable snapshot) — root'ga TEGMAYMIZ. Agar nom global
                // sifatida mavjud bo'lsa, uni handler ichidan `<-` bilan
                // o'zgartirib bo'lmaydi: ANIQ xato beramiz (jim shadow EMAS —
                // dasturchi jim muvaffaqiyatsizlikka uchramasin). Nom yangi bo'lsa
                // joriy scope'da lokal yaratamiz. Muzlatilmagan (top-level) bo'lsa
                // `Interp.global` ni odatdagidek qidiramiz/o'zgartiramiz.
                Parent::Root => {
                    if let Some(frozen) = self.globals_frozen.get() {
                        if frozen.contains_key(name) {
                            return Err(Flow::err(format!(
                                "'{}' global is frozen (server is running) — \
                                 cannot be changed with '<-' from inside a handler; \
                                 use db for shared mutable state",
                                name
                            )));
                        }
                        break;
                    }
                    cur = self.global.clone();
                }
                Parent::None => break,
            }
        }
        // yangi mutable o'zgaruvchi
        env.write().define(name, v, true);
        Ok(())
    }

    // `obj.field <- v` — member tayinlash. Hozircha FAQAT shared ctx cell'ga
    // yozish qo'llanadi (`req.ctx <- {...}`, issue #68). `obj` = `req` (Map),
    // `field` = "ctx" → req map'ining "ctx" kaliti `Value::Ctx(Arc<Mutex>)`
    // saqlaydi. `obj` (Map) klonlanadi, lekin ichidagi `Value::Ctx` Arc ulashiladi,
    // shuning uchun klon orqali ham asl Mutex cell'ga yozamiz — middleware yozgan
    // ctx'ni handler bir xil cell'da ko'radi. Oddiy Map immutable bo'lib qoladi:
    // `Value::Ctx` bo'lmagan maydonga yozish rad etiladi.
    fn assign_field(&self, obj: &Value, field: &str, v: Value) -> Result<(), Flow> {
        if let Value::Map(m) = obj
            && let Some(Value::Ctx(cell)) = m.get(field)
        {
            // ctx butunlay almashtiriladi (yangi map yoziladi). Yozilayotgan
            // qiymat map (yoki boshqa ctx snapshot'i) bo'lishi kerak.
            let new_map = match v {
                Value::Map(nm) => nm,
                Value::Ctx(c) => c.lock().unwrap().clone(),
                other => {
                    return Err(Flow::err(format!(
                        "req.{} <- expects a map, got {}",
                        field,
                        other.type_name()
                    )));
                }
            };
            *cell.lock().unwrap() = new_map;
            return Ok(());
        }
        Err(Flow::err(format!(
            "'.{}' cannot be assigned with '<-' (only a context field like req.ctx can be changed)",
            field
        )))
    }

    // `=` bind: o'zgaruvchini JORIY FUNKSIYA ICHIDAGI scope zanjirida qidiradi.
    // if/each/match bloklari leksik jihatdan shaffof — ular ichidagi `=` tashqi
    // (bir xil fn'dagi) o'zgaruvchini yangilaydi, boshqa tillar kabi. Qidiruv
    // fn/lambda chegarasida (`is_fn_boundary`) to'xtaydi: fn ichida `=` tashqi
    // global'ni emas, yangi LOCAL yaratadi (izolyatsiya/shadowing). Topilgan
    // o'zgaruvchi immutable (`=`) bo'lsa — xato (immutability saqlanadi, `<-` bilan
    // bir xil qoida). Topilmasa joriy scope'da yangi IMMUTABLE local yaratadi.
    fn bind(&self, name: &str, v: Value, env: &Env) -> Result<(), Flow> {
        let mut cur = env.clone();
        loop {
            let (parent, at_boundary) = {
                let mut s = cur.write();
                if let Some((slot, mutable)) = s.get_mut_entry(name) {
                    if !mutable {
                        return Err(Flow::err(format!(
                            "'{}' is immutable (declared with =); cannot be \
                             reassigned even from inside a block (declare it with `<-`)",
                            name
                        )));
                    }
                    *slot = v;
                    return Ok(());
                }
                // fn/lambda chegarasiga yetdik — bu fn'dan tashqariga chiqmaymiz.
                (s.parent.clone(), s.is_fn_boundary)
            };
            if at_boundary {
                break;
            }
            match parent {
                Parent::Scope(p) => cur = p,
                // Root — top-level global. Muzlatilmagan bo'lsa global'da qidirsak
                // ham bo'ladi, lekin `=` semantikasi: joriy scope'da yangi local
                // yaratish (top-level'da `cur` allaqachon global). Tashqi global'ni
                // qidirish uchun zanjir davom etadi.
                Parent::Root => {
                    if self.globals_frozen.get().is_some() {
                        break; // muzlatilgan global — yangi local yaratamiz
                    }
                    cur = self.global.clone();
                }
                Parent::None => break,
            }
        }
        // yangi immutable o'zgaruvchi (joriy scope'da)
        env.write().define(name, v, false);
        Ok(())
    }

    fn exec_each(&self, vars: &[String], iter: &Expr, body: &[Stmt], env: &Env) -> ExecResult {
        // `each i in inf` — cheksiz loop (REPL/event-loop uchun). i = 0,1,2,...
        // `stop`/`skip` bilan boshqariladi. Eager Vec yig'maydi (cheksiz bo'lardi).
        if matches!(iter, Expr::Inf) {
            return self.exec_each_inf(vars, body, env);
        }
        let iterable = self.eval(iter, env)?;
        let items: Vec<(Option<Value>, Value)> = match iterable {
            Value::List(xs) => xs.into_iter().map(|x| (None, x)).collect(),
            Value::Map(m) => m
                .into_iter()
                .map(|(k, v)| (Some(Value::Str(k)), v))
                .collect(),
            Value::Str(s) => s
                .chars()
                .map(|c| (None, Value::Str(c.to_string())))
                .collect(),
            other => {
                return Err(Flow::err(format!(
                    "each only iterates over list/map/range/str, got {}",
                    other.type_name()
                )));
            }
        };
        for (key, val) in items {
            let loop_env = Scope::child_of(env);
            {
                let mut s = loop_env.write();
                // Loop o'zgaruvchilari mutable (tana ichida `<-` mumkin; har
                // iteratsiyada qayta o'rnatiladi).
                if vars.len() == 2 {
                    // each k, v in map
                    let k = key.unwrap_or(Value::Nil);
                    s.define(&vars[0], k, true);
                    s.define(&vars[1], val, true);
                } else {
                    // each x in list  — map ustida bo'lsa, qiymat
                    s.define(&vars[0], val, true);
                }
            }
            match self.exec_block(body, &loop_env) {
                Ok(_) => {}
                Err(Flow::Skip) => continue,
                Err(Flow::Stop) => break,
                Err(other) => return Err(other),
            }
        }
        Ok(Value::Nil)
    }

    // `each i in inf` — cheksiz takror. Hisoblagich i 0 dan boshlab har
    // iteratsiyada 1 ga ortadi (i64 overflow'da to'xtaydi — amalda yetib
    // bormaydi). `stop` chiqaradi, `skip` keyingisiga o'tadi.
    fn exec_each_inf(&self, vars: &[String], body: &[Stmt], env: &Env) -> ExecResult {
        if vars.len() != 1 {
            return Err(Flow::err(
                "each ... in inf expects a single variable (each i in inf)",
            ));
        }
        let mut i: i64 = 0;
        loop {
            let loop_env = Scope::child_of(env);
            {
                let mut s = loop_env.write();
                // Loop o'zgaruvchisi mutable (tana ichida `<-` mumkin).
                s.define(&vars[0], Value::Int(i), true);
            }
            match self.exec_block(body, &loop_env) {
                Ok(_) => {}
                Err(Flow::Skip) => {}
                Err(Flow::Stop) => break,
                Err(other) => return Err(other),
            }
            match i.checked_add(1) {
                Some(n) => i = n,
                None => break, // i64 chegarasi — amalda yetib bo'lmaydi
            }
        }
        Ok(Value::Nil)
    }

    // ---------------- ifodalarni baholash ----------------
    pub fn eval(&self, e: &Expr, env: &Env) -> EvalResult {
        match e {
            Expr::Int(n) => Ok(Value::Int(*n)),
            Expr::Flt(x) => Ok(Value::Flt(*x)),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Nil => Ok(Value::Nil),
            Expr::Sym(s) => Ok(Value::Sym(s.clone())),
            Expr::Str(pieces) => {
                let mut out = String::new();
                for p in pieces {
                    match p {
                        StrPiece::Lit(s) => out.push_str(s),
                        StrPiece::Expr(e) => {
                            let v = self.eval(e, env)?;
                            out.push_str(&v.to_text());
                        }
                    }
                }
                Ok(Value::Str(out))
            }
            Expr::Ident(name) => match self.lookup(name, env) {
                Ok(v) => Ok(v),
                // `log` qiymat sifatida (callback `xs.each log`, `f log`) — eski
                // global `log` bilan moslik uchun info-darajali shim (issue #139).
                // To'g'ridan-to'g'ri `log "..."` chaqiruvi apply_callee'da oldinroq
                // ushlanadi (bu yo'lga tushmaydi). `log` o'zgaruvchi e'lon qilingan
                // bo'lsa lookup Ok beradi — u ustun.
                Err(e) => {
                    if name == "log" {
                        Ok(crate::builtins::log_value_shim())
                    } else {
                        Err(e)
                    }
                }
            },
            Expr::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    out.push(self.eval(it, env)?);
                }
                Ok(Value::List(out))
            }
            Expr::Map(entries) => {
                let mut m = BTreeMap::new();
                for entry in entries {
                    match entry {
                        MapEntry::Pair { key, value } => {
                            m.insert(key.clone(), self.eval(value, env)?);
                        }
                        MapEntry::Dynamic { key, value } => {
                            let k = self.eval(key, env)?;
                            let ks = match k {
                                Value::Str(s) => s,
                                Value::Sym(s) => s,
                                other => format!("{}", other),
                            };
                            m.insert(ks, self.eval(value, env)?);
                        }
                        MapEntry::Spread(src) => {
                            let v = self.eval(src, env)?;
                            if let Value::Map(other) = v {
                                for (k, val) in other {
                                    m.insert(k, val);
                                }
                            } else {
                                return Err(Flow::err(format!(
                                    "map spread (...) only works with a map, got {}",
                                    v.type_name()
                                )));
                            }
                        }
                    }
                }
                Ok(Value::Map(m))
            }
            Expr::Unary { op, expr } => {
                let v = self.eval(expr, env)?;
                match op {
                    UnOp::Not => Ok(Value::Bool(!v.truthy())),
                    UnOp::Neg => match v {
                        // i64::MIN ni teskarilab bo'lmaydi — int_arith bilan bir xil xato.
                        Value::Int(n) => Ok(Value::Int(
                            n.checked_neg().ok_or_else(|| Flow::overflow("-"))?,
                        )),
                        Value::Flt(x) => Ok(Value::Flt(-x)),
                        other => Err(Flow::err(format!(
                            "'-' only applies to a number, got {}",
                            other.type_name()
                        ))),
                    },
                }
            }
            Expr::Binary { op, lhs, rhs } => self.eval_binary(*op, lhs, rhs, env),
            Expr::Range { start, end } => {
                let a = self.eval(start, env)?;
                let b = self.eval(end, env)?;
                match (a, b) {
                    (Value::Int(s), Value::Int(e)) => {
                        let mut out = Vec::new();
                        let mut i = s;
                        while i <= e {
                            out.push(Value::Int(i));
                            // end = i64::MAX bo'lsa i += 1 toshib ketardi —
                            // oxirgi element qo'shilgach to'xtaymiz.
                            match i.checked_add(1) {
                                Some(n) => i = n,
                                None => break,
                            }
                        }
                        Ok(Value::List(out))
                    }
                    (a, b) => Err(Flow::err(format!(
                        "range (..) requires integers, got {}..{}",
                        a.type_name(),
                        b.type_name()
                    ))),
                }
            }
            // inf faqat `each i in inf` da ma'noli — qiymat sifatida ishlatib bo'lmaydi.
            Expr::Inf => Err(Flow::err(
                "inf is only used in `each i in inf` (not a value)",
            )),
            Expr::Field { target, name } => {
                // `env.PORT` — muhit o'zgaruvchisi. `env` built-in ident bo'lib,
                // o'zgaruvchi sifatida e'lon QILINMAGAN bo'lsa, std::env'dan o'qiymiz.
                // Foydalanuvchi `env` nomli o'zgaruvchi yaratsa, u ustun bo'ladi.
                if let Expr::Ident(id) = target.as_ref() {
                    if id == "env" && self.lookup(id, env).is_err() {
                        // OS env > .env fayl (lazy o'qiladi, faqat shu yerdan).
                        return Ok(self.env_lookup(name));
                    }
                    // Argument'siz modul funksiyasi: `time.now` Call emas, Field
                    // bo'lib keladi. Modul nomi o'zgaruvchi sifatida e'lon
                    // qilinmagan bo'lsa, argument'siz modul funksiyasi sifatida
                    // chaqiramiz. (str/math/rand argument talab qiladi; time.now —
                    // yagona argumentsizi, lekin umumiy tutamiz.)
                    if crate::builtins::is_module(id) && self.lookup(id, env).is_err() {
                        return crate::builtins::call_module(id, name, vec![]);
                    }
                    // `log.info` argumentsiz (xabarsiz) -> Field bo'lib keladi.
                    // `log` o'zgaruvchi emas bo'lsa bo'sh xabarli darajaga
                    // yo'naltiramiz (issue #139). Noma'lum daraja — aniq xato.
                    if id == "log" && self.lookup(id, env).is_err() {
                        return match name.as_str() {
                            "debug" | "info" | "warn" | "err" => self.log_dispatch(name, vec![]),
                            _ => Err(Flow::err(format!(
                                "log.{} does not exist (debug/info/warn/err)",
                                name
                            ))),
                        };
                    }
                    // `reg.names` argumentsiz -> Call emas, Field bo'lib keladi
                    // (time.now kabi). `reg` o'zgaruvchi sifatida e'lon qilinmagan
                    // bo'lsa, argumentsiz reg funksiyasi sifatida chaqiramiz.
                    if id == "reg" && self.lookup(id, env).is_err() {
                        return self.reg_dispatch(name, vec![]);
                    }
                    // `crypto.uuid` argumentsiz -> Call emas, Field bo'lib keladi
                    // (time.now kabi). `crypto` e'lon qilinmagan bo'lsa battery
                    // funksiyasi sifatida chaqiramiz.
                    if id == "crypto" && self.lookup(id, env).is_err() {
                        return crate::crypto_mod::crypto_module(name, vec![]);
                    }
                    // `cron.run` argumentsiz -> Call emas, Field bo'lib keladi. cron
                    // o'zgaruvchi sifatida e'lon qilinmagan bo'lsa, argumentsiz cron
                    // funksiyasi (run) sifatida chaqiramiz. (Aks holda `cron` ident
                    // o'zgaruvchi deb qidirilib "noma'lum nom" beradi.)
                    if id == "cron" && self.lookup(id, env).is_err() {
                        return self.arc_self().cron_dispatch(name, vec![]);
                    }
                    // queue ham state'li modul — argumentsiz chaqiruvi (kelajakda)
                    // shu yerda ushlanadi; aks holda `queue` ident o'zgaruvchi deb
                    // qidirilib "noma'lum nom" beradi.
                    if id == "queue" && self.lookup(id, env).is_err() {
                        return self.arc_self().queue_dispatch(name, vec![]);
                    }
                }
                let t = self.eval(target, env)?;
                self.get_field(&t, name, env)
            }
            Expr::Index { target, key } => {
                let t = self.eval(target, env)?;
                let k = self.eval(key, env)?;
                self.get_index(&t, &k)
            }
            Expr::Lambda { params, body } => Ok(Value::Fn(Arc::new(FnValue {
                params: params.clone(),
                body: body.clone(),
                parent: Scope::parent_link(env),
                name: "<lambda>".to_string(),
            }))),
            Expr::Call { callee, args } => self.eval_call(callee, args, env),
            Expr::Try(inner) => {
                // expr! — agar inner fail/err qaytarsa, yuqoriga uzatamiz;
                // muvaffaqiyatli bo'lsa qiymatni qaytaramiz. Yadroda Fail/Error
                // baribir Err sifatida ko'tariladi, shuning uchun bu o'tkazgich.
                self.eval(inner, env)
            }
            Expr::TryCatch {
                body,
                catch_var,
                catch_body,
            } => self.eval_try(body, catch_var.as_deref(), catch_body, env),
            Expr::If(ifx) => self.eval_if(ifx, env),
            Expr::Match(mx) => self.eval_match(mx, env),
            Expr::Fail { status, message } => {
                let st = match status {
                    Some(e) => match self.eval(e, env)? {
                        Value::Int(n) => Some(n),
                        other => {
                            return Err(Flow::err(format!(
                                "fail status must be an int, got {}",
                                other.type_name()
                            )));
                        }
                    },
                    None => None,
                };
                let msg = self.eval(message, env)?;
                Err(Flow::Fail {
                    status: st,
                    message: format!("{}", msg),
                })
            }
        }
    }

    fn lookup(&self, name: &str, env: &Env) -> EvalResult {
        // Muzlatilgan global snapshot'ni bir marta lock-free olamiz (OnceLock
        // o'qishi atomik yuklash — qulf emas).
        let frozen = self.globals_frozen.get();
        let mut cur = env.clone();
        loop {
            // Har leveldagi scope'ni BITTA read lock ostida ko'ramiz: ham
            // o'zgaruvchini qidiramiz, ham keyingi ota'ni olamiz. (Avval ikkita
            // alohida `cur.read()` bor edi — har biri parking_lot RwLock atomik
            // operatsiyasi; parallel request'lar global root'da urilardi.)
            let parent = {
                let s = cur.read();
                // root scope'ning O'ZI muzlatilgan bo'lsa — lock-free snapshot.
                if s.is_root
                    && let Some(frozen) = frozen
                {
                    return frozen
                        .get(name)
                        .cloned()
                        .ok_or_else(|| Flow::err(format!("unknown name: {}", name)));
                }
                if let Some(v) = s.get(name) {
                    return Ok(v.clone());
                }
                s.parent.clone()
            };
            match parent {
                Parent::None => return Err(Flow::err(format!("unknown name: {}", name))),
                Parent::Scope(p) => cur = p,
                Parent::Root => {
                    // Ota — root (marker). Muzlatilgan bo'lsa root Arc'ga TEGMASDAN
                    // frozen snapshot'dan o'qiymiz — parallel request'lar bu yerda
                    // urilmaydi (atomik contention yo'q). Aks holda (top-level,
                    // muzlatilmagan) `Interp.global` Arc'iga o'tamiz — klon shart
                    // emas, `&self` orqali kelyapti.
                    if let Some(frozen) = frozen {
                        return frozen
                            .get(name)
                            .cloned()
                            .ok_or_else(|| Flow::err(format!("unknown name: {}", name)));
                    }
                    cur = self.global.clone();
                }
            }
        }
    }

    // try/catch (issue #125). Tana o'z scope'ida ishlaydi; `fail` (Flow::Fail)
    // yoki runtime xato (Flow::Error) ko'tarilsa — uni ushlaymiz va catch tanasini
    // ishga tushiramiz. ret/skip/stop oqim-signallari ushlanmaydi: ular try'dan
    // o'tib funksiya/loop'ni boshqaradi (xato emas, oqim). catch o'zgaruvchisi
    // bo'lsa, unga {message, status} map'i bog'lanadi (status — int yoki nil).
    fn eval_try(
        &self,
        body: &[Stmt],
        catch_var: Option<&str>,
        catch_body: &[Stmt],
        env: &Env,
    ) -> EvalResult {
        let inner = Scope::child_of(env);
        match self.exec_block(body, &inner) {
            Ok(v) => Ok(v),
            Err(Flow::Fail { status, message }) => {
                self.run_catch(catch_var, status, message, catch_body, env)
            }
            Err(Flow::Error(message)) => self.run_catch(catch_var, None, message, catch_body, env),
            // ret/skip/stop — oqim-signallari, ushlanmaydi.
            Err(other) => Err(other),
        }
    }

    // catch tanasini xato map'i bilan ishga tushiradi.
    fn run_catch(
        &self,
        catch_var: Option<&str>,
        status: Option<i64>,
        message: String,
        catch_body: &[Stmt],
        env: &Env,
    ) -> EvalResult {
        let inner = Scope::child_of(env);
        if let Some(name) = catch_var {
            let mut m = BTreeMap::new();
            m.insert("message".to_string(), Value::Str(message));
            m.insert(
                "status".to_string(),
                status.map(Value::Int).unwrap_or(Value::Nil),
            );
            inner.write().define(name, Value::Map(m), false);
        }
        self.exec_block(catch_body, &inner)
    }

    fn eval_if(&self, ifx: &IfExpr, env: &Env) -> EvalResult {
        for (cond, block) in &ifx.arms {
            if self.eval(cond, env)?.truthy() {
                let inner = Scope::child_of(env);
                return self.exec_block(block, &inner);
            }
        }
        if let Some(eb) = &ifx.else_block {
            let inner = Scope::child_of(env);
            return self.exec_block(eb, &inner);
        }
        Ok(Value::Nil)
    }

    fn eval_match(&self, mx: &MatchExpr, env: &Env) -> EvalResult {
        let subj = self.eval(&mx.subject, env)?;
        for arm in &mx.arms {
            let matched = match &arm.pattern {
                MatchPat::Wildcard => true,
                MatchPat::Sym(s) => matches!(&subj, Value::Sym(v) if v == s),
                MatchPat::Int(n) => matches!(&subj, Value::Int(v) if v == n),
            };
            if matched {
                let inner = Scope::child_of(env);
                return self.exec_block(&arm.body, &inner);
            }
        }
        Ok(Value::Nil)
    }

    fn eval_binary(&self, op: BinOp, lhs: &Expr, rhs: &Expr, env: &Env) -> EvalResult {
        // Qisqa-tutashuv (short-circuit) operatorlari
        match op {
            BinOp::And => {
                let l = self.eval(lhs, env)?;
                if !l.truthy() {
                    return Ok(l);
                }
                return self.eval(rhs, env);
            }
            BinOp::Or => {
                let l = self.eval(lhs, env)?;
                if l.truthy() {
                    return Ok(l);
                }
                return self.eval(rhs, env);
            }
            BinOp::Coalesce => {
                let l = self.eval(lhs, env)?;
                if matches!(l, Value::Nil) {
                    return self.eval(rhs, env);
                }
                return Ok(l);
            }
            BinOp::Pipe => {
                // x |> f      ==  f x       (f — funksiya qiymati yoki lambda)
                // x |> f a b  ==  f a b x   (rhs chaqiruv bo'lsa, x OXIRGI argument)
                //
                // Ikkinchi shakl pipe'ni qisman-chaqiruvga aylantiradi: `db.from "t"
                // |> db.eq {...}` da `db.eq {...}` rhs Call bo'lib keladi, biz uni
                // darhol baholamay, lhs'ni args oxiriga qo'shib `eval_call` qilamiz.
                // Shu sabab db.*/str.* kabi modul dispatch'lari ham tabiiy ishlaydi
                // (eval_call ularni maxsus yo'naltiradi). Mavjud `x |> str.up` endi
                // ishlaydi — avval u rhs'ni argumentsiz chaqirib xato berardi.
                let l = self.eval(lhs, env)?;
                match rhs {
                    // `x |> f a b` => `f a b x`: lhs args oxiriga qo'shiladi.
                    Expr::Call { callee, args } => {
                        let mut argv = self.eval_args(args, env)?;
                        argv.push(l);
                        return self.apply_callee(callee, argv, env);
                    }
                    // `x |> str.up` / `x |> db.all` => argumentsiz modul/metod
                    // chaqiruvi, lhs yagona argument. Field'ni qiymat sifatida
                    // baholab bo'lmaydi (modul funksiyasi qiymat emas), shuning
                    // uchun to'g'ridan-to'g'ri apply_callee.
                    Expr::Field { .. } => {
                        return self.apply_callee(rhs, vec![l], env);
                    }
                    // rhs oddiy funksiya qiymati/lambda/ident: f x.
                    _ => {
                        let f = self.eval(rhs, env)?;
                        return self.apply(f, vec![l]);
                    }
                }
            }
            _ => {}
        }
        let l = self.eval(lhs, env)?;
        let r = self.eval(rhs, env)?;
        self.binary_values(op, l, r)
    }

    fn binary_values(&self, op: BinOp, l: Value, r: Value) -> EvalResult {
        use Value::*;
        match op {
            BinOp::Eq => return Ok(Bool(l.equals(&r))),
            BinOp::Ne => return Ok(Bool(!l.equals(&r))),
            _ => {}
        }
        // Taqqoslash va arifmetika
        match (op, l, r) {
            // + string birlashtirish
            (BinOp::Add, Str(a), Str(b)) => Ok(Str(a + &b)),
            (BinOp::Add, Str(a), b) => Ok(Str(a + &b.to_text())),
            (BinOp::Add, a, Str(b)) => Ok(Str(a.to_text() + &b)),

            // int-int arifmetika
            (op, Int(a), Int(b)) => int_arith(op, a, b),
            // aralash/float arifmetika
            (op, a, b) if is_num(&a) && is_num(&b) => flt_arith(op, to_f64(&a), to_f64(&b)),

            (op, a, b) => Err(Flow::err(format!(
                "{:?} operator cannot be applied to {} and {}",
                op,
                a.type_name(),
                b.type_name()
            ))),
        }
    }

    // ---------------- chaqiruv ----------------
    fn eval_call(&self, callee: &Expr, args: &[Expr], env: &Env) -> EvalResult {
        let argv = self.eval_args(args, env)?;
        self.apply_callee(callee, argv, env)
    }

    // Argumentlar ALLAQACHON baholangan holatda callee'ni chaqiradi. eval_call va
    // pipe (`x |> f a` => `f a x`) shu yagona nuqtaga keladi — dispatch mantig'i
    // bir joyda. `argv` chaqiruv argumentlari (pipe holatida lhs oxiriga qo'shilgan).
    fn apply_callee(&self, callee: &Expr, argv: Vec<Value>, env: &Env) -> EvalResult {
        // Metod chaqiruvi: target.method arg...  -> Field bo'lib keladi.
        if let Expr::Field { target, name } = callee {
            // Ikki-bosqichli modul namespace'i: ws.room.* / ws.data.* —
            // target'ning o'zi Field{Ident("ws"), "room"/"data"}. `Ident` shoxiga
            // tushmaydi, shuning uchun bu yerda alohida ushlaymiz (ws — state'li,
            // Interp kerak). Hozircha faqat `ws` namespace'i ichki guruhli.
            if let Expr::Field {
                target: inner,
                name: sub,
            } = target.as_ref()
                && let Expr::Ident(root) = inner.as_ref()
                && root == "ws"
            {
                return match sub.as_str() {
                    "room" => self.arc_self().ws_room_dispatch(name, argv),
                    "data" => self.arc_self().ws_data_dispatch(name, argv),
                    _ => Err(Flow::err(format!("ws.{} group does not exist", sub))),
                };
            }
            // module.func (str.up, math.floor, ...) — `str` o'zgaruvchi emas,
            // shuning uchun target'ni baholashdan OLDIN modulni tekshiramiz.
            if let Expr::Ident(modname) = target.as_ref() {
                // http — state'li va Interp'ga (handler apply uchun) muhtoj,
                // shuning uchun call_module emas, http_dispatch'ga yo'naltiramiz.
                if modname == "http" {
                    return self.arc_self().http_dispatch(name, argv);
                }
                // db — http kabi state'li (connection + tx konteksti); Interp'ga
                // muhtoj. db.tx argumenti lambda bo'lib keladi (Value::Fn).
                if modname == "db" {
                    return self.arc_self().db_dispatch(name, argv);
                }
                // ws — http kabi state'li (jonli ulanishlar, handler apply uchun
                // Interp kerak). ws.room.*/ws.data.* esa ikki-bosqichli Field
                // bo'lib keladi — quyiroqda (Field target ichida) ushlanadi.
                if modname == "ws" {
                    return self.arc_self().ws_dispatch(name, argv);
                }
                // reg — state'li (funksiya registri); `reg.add`/`reg.call` argument
                // sifatida funksiya/argumentlar oladi. `reg.names` argumentsiz —
                // Field shoxida (quyiroqda) ushlanadi.
                if modname == "reg" {
                    return self.reg_dispatch(name, argv);
                }
                // cron — state'li (rejalashtirilgan vazifalar). `cron.on` ifoda + handler
                // oladi, `cron.run` argumentsiz bloklaydi. Ifoda parser'da tirnoqsiz
                // 5-maydonli str sifatida keladi (quyida parser maxsus ushlaydi).
                if modname == "cron" {
                    return self.arc_self().cron_dispatch(name, argv);
                }
                // queue — state'li (fon navbati). `queue.push` nom+payload oladi,
                // `queue.on` nom+handler oladi. Worker handler'ni apply qiladi —
                // shuning uchun Interp'ga muhtoj (call_module emas).
                if modname == "queue" {
                    return self.arc_self().queue_dispatch(name, argv);
                }
                // ai — LLM primitiv (Anthropic). `$AI_KEY`ni env_lookup orqali
                // o'qish uchun Interp'ga muhtoj (call_module emas). Holatsiz —
                // har chaqiruv mustaqil https POST. `ai` o'zgaruvchi sifatida
                // e'lon qilingan bo'lsa, modul emas — o'zgaruvchi sifatida ko'riladi.
                if modname == "ai" && self.lookup(modname, env).is_err() {
                    return self.ai_dispatch(name, argv);
                }
                // auth — autentifikatsiya primitivlari (JWT + parol hash). `ai`
                // kabi holatsiz; `$AUTH_SECRET`ni env_lookup orqali o'qish uchun
                // Interp'ga muhtoj (call_module emas). `auth` o'zgaruvchi sifatida
                // e'lon qilingan bo'lsa, modul emas — o'zgaruvchi ustun.
                if modname == "auth" && self.lookup(modname, env).is_err() {
                    return self.auth_dispatch(name, argv);
                }
                // crypto — kriptografik primitivlar (issue #131). Holatsiz va
                // Interp'ga muhtoj emas, lekin auth/ai kabi battery: `crypto`
                // nomi e'lon qilingan bo'lsa (masalan `use ./crypto`), u ustun —
                // shuning uchun shartsiz is_module ro'yxatiga kirmaydi.
                if modname == "crypto" && self.lookup(modname, env).is_err() {
                    return crate::crypto_mod::crypto_module(name, argv);
                }
                // log — darajali logger (issue #139). `log.debug/info/warn/err`.
                // `log` global emas; foydalanuvchi `log` o'zgaruvchi e'lon qilmagan
                // bo'lsa shu yerda ushlanadi. Noma'lum daraja — aniq xato.
                if modname == "log" && self.lookup(modname, env).is_err() {
                    return match name.as_str() {
                        "debug" | "info" | "warn" | "err" => self.log_dispatch(name, argv),
                        _ => Err(Flow::err(format!(
                            "log.{} does not exist (debug/info/warn/err)",
                            name
                        ))),
                    };
                }
                if crate::builtins::is_module(modname) {
                    return crate::builtins::call_module(modname, name, argv);
                }
            }
            let recv = self.eval(target, env)?;
            // Avval haqiqiy map maydoni funksiya bo'lsa (masalan map ichidagi
            // lambda) — uni chaqiramiz; aks holda builtin metod.
            if let Value::Map(m) = &recv
                && let Some(v @ (Value::Fn(_) | Value::Native(_))) = m.get(name)
            {
                let f = v.clone();
                return self.apply(f, argv);
            }
            // Yuqori tartibli list metodlari (lambda chaqiradi) — bu yerda,
            // chunki builtins Interp'ga kira olmaydi.
            if let Value::List(xs) = &recv {
                match name.as_str() {
                    "filter" | "map" | "reduce" | "find" | "any" | "all" | "sort" => {
                        return self.list_hof(xs, name, argv);
                    }
                    _ => {}
                }
            }
            return crate::builtins::call_method(&recv, name, argv);
        }
        // Bare `log "..."` — darajasiz chaqiruv = info (issue #139). `log` global
        // emas (pure dispatch battery); foydalanuvchi `log` o'zgaruvchi e'lon
        // qilmagan bo'lsa shu yerda ushlanadi, aks holda o'zgaruvchi ustun.
        if let Expr::Ident(id) = callee
            && id == "log"
            && self.lookup(id, env).is_err()
        {
            return self.log_dispatch("info", argv);
        }
        // par [\-> ... \-> ...] — til-darajasidagi parallel fan-out (issue #137).
        // Lambdalar ro'yxatini har birini ALOHIDA thread'da chaqiradi, hammasini
        // kutadi va natijalar ro'yxatini (kirish tartibida) qaytaradi. `par` global
        // emas (pure primitiv); foydalanuvchi `par` o'zgaruvchi e'lon qilmagan
        // bo'lsa shu yerda ushlanadi, aks holda o'zgaruvchi ustun.
        if let Expr::Ident(id) = callee
            && id == "par"
            && self.lookup(id, env).is_err()
        {
            return self.arc_self().par_run(argv);
        }
        let f = self.eval(callee, env)?;
        self.apply(f, argv)
    }

    // Yuqori tartibli list metodlari (filter/map/reduce/find/any/all/sort) —
    // funksiya argumentini element(lar) uchun chaqiradi.
    fn list_hof(&self, xs: &[Value], method: &str, args: Vec<Value>) -> EvalResult {
        match method {
            "filter" => {
                let f = args
                    .into_iter()
                    .next()
                    .ok_or_else(|| Flow::err("list.filter: function argument required"))?;
                let mut out = Vec::new();
                for x in xs {
                    if self.apply(f.clone(), vec![x.clone()])?.truthy() {
                        out.push(x.clone());
                    }
                }
                Ok(Value::List(out))
            }
            "map" => {
                let f = args
                    .into_iter()
                    .next()
                    .ok_or_else(|| Flow::err("list.map: function argument required"))?;
                let mut out = Vec::with_capacity(xs.len());
                for x in xs {
                    out.push(self.apply(f.clone(), vec![x.clone()])?);
                }
                Ok(Value::List(out))
            }
            "reduce" => {
                let mut it = args.into_iter();
                let mut acc = it
                    .next()
                    .ok_or_else(|| Flow::err("list.reduce: initial value required"))?;
                let f = it
                    .next()
                    .ok_or_else(|| Flow::err("list.reduce: function argument required"))?;
                for x in xs {
                    acc = self.apply(f.clone(), vec![acc, x.clone()])?;
                }
                Ok(acc)
            }
            "find" => {
                // Predikatga mos birinchi elementni qaytaradi; topilmasa nil.
                // (list.index -1 berib pozitsiya beradi; find esa qiymatni.)
                let f = args
                    .into_iter()
                    .next()
                    .ok_or_else(|| Flow::err("list.find: function argument required"))?;
                for x in xs {
                    if self.apply(f.clone(), vec![x.clone()])?.truthy() {
                        return Ok(x.clone());
                    }
                }
                Ok(Value::Nil)
            }
            "any" => {
                // Birinchi mosda to'xtaydi (short-circuit) — filter+len aylanma
                // yo'lidan farqli, qolgan elementlar uchun predikat chaqirilmaydi.
                let f = args
                    .into_iter()
                    .next()
                    .ok_or_else(|| Flow::err("list.any: function argument required"))?;
                for x in xs {
                    if self.apply(f.clone(), vec![x.clone()])?.truthy() {
                        return Ok(Value::Bool(true));
                    }
                }
                Ok(Value::Bool(false))
            }
            "all" => {
                // Birinchi nomosda to'xtaydi; bo'sh list uchun true (vacuous).
                let f = args
                    .into_iter()
                    .next()
                    .ok_or_else(|| Flow::err("list.all: function argument required"))?;
                for x in xs {
                    if !self.apply(f.clone(), vec![x.clone()])?.truthy() {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(true))
            }
            "sort" => {
                // Komparatorli sort: \a b -> son (manfiy: a oldin, musbat: b
                // oldin, 0: teng) — JS uslubi. Argumentsiz `l.sort` Field bo'lib
                // builtins'dagi tabiiy tartibga tushadi; bu yerga faqat Call
                // (argumentli) keladi, lekin bo'sh argv ham defensive qo'llanadi.
                let Some(f) = args.into_iter().next() else {
                    return crate::builtins::sort_default(xs);
                };
                let sorted = crate::builtins::sort_values(xs.to_vec(), &mut |a, b| match self
                    .apply(f.clone(), vec![a.clone(), b.clone()])?
                {
                    Value::Int(n) => Ok(n.cmp(&0)),
                    Value::Flt(x) => Ok(x.partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal)),
                    other => Err(Flow::err(format!(
                        "list.sort: comparator must return a number (negative/0/positive), got {}",
                        other.type_name()
                    ))),
                })?;
                Ok(Value::List(sorted))
            }
            _ => unreachable!(),
        }
    }

    fn eval_args(&self, args: &[Expr], env: &Env) -> Result<Vec<Value>, Flow> {
        let mut out = Vec::with_capacity(args.len());
        for a in args {
            out.push(self.eval(a, env)?);
        }
        Ok(out)
    }

    pub fn apply(&self, f: Value, args: Vec<Value>) -> EvalResult {
        match f {
            Value::Native(nf) => (nf.func)(args),
            Value::Fn(fv) => {
                if args.len() != fv.params.len() {
                    return Err(Flow::err(format!(
                        "{}: expected {} arguments, got {}",
                        fv.name,
                        fv.params.len(),
                        args.len()
                    )));
                }
                // Chuqurlik limiti: cheksiz rekursiya stack overflow'da butun
                // process'ni ABORT qiladi (panic emas — spawn_blocking ham
                // qutqarmaydi). Limit shu abort'dan ancha oldin graceful
                // Flow::err qaytaradi. Guard RAII — xato/panic yo'lida ham
                // hisoblagich to'g'ri kamayadi (issue #90).
                let _depth = CallDepthGuard::enter(&fv.name)?;
                // Native stack kam qolgan bo'lsa yangi segment ajratamiz (rustc
                // yondashuvi): chuqur (lekin limit ichidagi) rekursiya 2MB'lik
                // spawn_blocking/test thread'ida ham overflow qilmaydi — haqiqiy
                // chegara faqat MAX_CALL_DEPTH bo'lib qoladi.
                stacker::maybe_grow(STACK_RED_ZONE, STACK_GROW_SIZE, || {
                    // Params soni bilan oldindan o'lchamlangan child — bind paytida
                    // Vec qayta-allocate bo'lmaydi. Params mutable: tana ichida `<-`
                    // bilan o'zgartirilishi mumkin (avval ruxsat etilardi).
                    let call_env = Scope::child_with_capacity(fv.parent.clone(), fv.params.len());
                    {
                        let mut s = call_env.write();
                        for (p, a) in fv.params.iter().zip(args) {
                            // `define` ishlatamiz (xom push emas): parser takror
                            // param'ni rad etadi, lekin define defensive — agar nom
                            // baribir takrorlansa write/read bitta slot'da qoladi
                            // (define-oldindan / get-orqadan zidligi yuzaga kelmaydi).
                            // Params kichik (0-4), O(n²) arzon. Mutable: tana `<-` qila oladi.
                            s.define(p, a, true);
                        }
                    }
                    match self.exec_block(&fv.body, &call_env) {
                        Ok(v) => Ok(v),                // oxirgi ifoda — qaytadi
                        Err(Flow::Return(v)) => Ok(v), // erta ret
                        Err(other) => Err(other),      // fail/err/skip/stop
                    }
                })
            }
            other => Err(Flow::err(format!(
                "{} is not callable (not a function)",
                other.type_name()
            ))),
        }
    }

    // ---------------- maydon / indeks ----------------
    fn get_field(&self, t: &Value, name: &str, _env: &Env) -> EvalResult {
        match t {
            Value::Map(m) => {
                // Avval haqiqiy kalit; bo'lmasa argumentsiz metod (keys/vals/len).
                if let Some(v) = m.get(name) {
                    // ctx cell'ni o'qisa — snapshot Map qaytaramiz (handler oddiy
                    // map ko'rsin, ichki Ctx tipini emas).
                    if let Value::Ctx(cell) = v {
                        return Ok(Value::Map(cell.lock().unwrap().clone()));
                    }
                    return Ok(v.clone());
                }
                if matches!(name, "keys" | "vals" | "len") {
                    return crate::builtins::call_method(t, name, vec![]);
                }
                Ok(Value::Nil)
            }
            // .len kabi argumentsiz metodlar maydon sifatida ham ishlaydi.
            Value::List(_) | Value::Str(_) => crate::builtins::call_method(t, name, vec![]),
            Value::Nil => Ok(Value::Nil), // nil.x -> nil (xavfsiz navigatsiya)
            other => Err(Flow::err(format!(
                "{} type has no field '.{}'",
                other.type_name(),
                name
            ))),
        }
    }

    fn get_index(&self, t: &Value, k: &Value) -> EvalResult {
        match (t, k) {
            (Value::List(xs), Value::Int(i)) => {
                let idx = *i;
                if idx < 0 || idx as usize >= xs.len() {
                    Ok(Value::Nil)
                } else {
                    Ok(xs[idx as usize].clone())
                }
            }
            // ctx kalitini o'qisa get_field bilan izchil — snapshot Map qaytaramiz.
            (Value::Map(m), Value::Str(key)) | (Value::Map(m), Value::Sym(key)) => {
                match m.get(key) {
                    Some(Value::Ctx(cell)) => Ok(Value::Map(cell.lock().unwrap().clone())),
                    other => Ok(other.cloned().unwrap_or(Value::Nil)),
                }
            }
            (Value::Nil, _) => Ok(Value::Nil),
            (t, k) => Err(Flow::err(format!(
                "{}[{}] indexing is not supported",
                t.type_name(),
                k.type_name()
            ))),
        }
    }
}

// `use` yo'li foydalanuvchi faylimi yoki batareyami? Foydalanuvchi modullari
// nisbiy yo'l bilan beriladi (`./tools`, `../lib/x`). Batareyalar oddiy nom
// (`http`, `db`) — ular dispatch nom asosida ishlaydi, fayl yuklanmaydi.
// ADD/DROP COLUMN xatosini "allaqachon bor/yo'q" holatida yutadi (SQLite'da bu
// DDL'lar IF [NOT] EXISTS qo'llab-quvvatlamaydi). Idempotentlik uchun: ustun
// allaqachon mavjud (user qo'shgan / rename'ning yangi tomoni) yoki allaqachon
// yo'q (user o'chirgan / rename'ning eski tomoni) bo'lsa — migration yiqilmaydi.
// Boshqa BARCHA xatolar (masalan, sintaksis, tip) ko'tariladi.
fn swallow_benign(res: Result<usize, String>) -> Result<(), Flow> {
    match res {
        Ok(_) => Ok(()),
        Err(msg) => {
            let m = msg.to_lowercase();
            if m.contains("duplicate column name") || m.contains("no such column") {
                Ok(()) // allaqachon kerakli holatda — tinch o't
            } else {
                Err(Flow::err(msg))
            }
        }
    }
}

fn is_user_module_path(path: &str) -> bool {
    path.starts_with("./") || path.starts_with("../") || path == "." || path == ".."
}

// Modul yo'lidan bog'lash nomini chiqaradi: oxirgi segment, `.fx` siz.
// `./lib/greet` -> `greet`, `./tools` -> `tools`.
fn module_basename(path: &str) -> String {
    let last = path.rsplit('/').next().unwrap_or(path);
    last.strip_suffix(".fx").unwrap_or(last).to_string()
}

// Modul dasturidan eksport qilingan top-level nomlarni yig'adi: `exp NAME = ...`
// va `exp fn NAME`. Faqat shular namespace'ga kiradi — qolgan `=`/`fn` lar
// modul-private.
fn collect_exported(prog: &Program) -> HashSet<String> {
    let mut set = HashSet::new();
    for stmt in prog {
        match stmt {
            Stmt::ExpBind { name, .. } => {
                set.insert(name.clone());
            }
            Stmt::FnDecl {
                name,
                exported: true,
                ..
            } => {
                set.insert(name.clone());
            }
            _ => {}
        }
    }
    set
}

// Joriy katalogdagi `.env` faylini o'qiydi va parse qiladi. Fayl yo'q bo'lsa
// yoki o'qib bo'lmasa — bo'sh map (xato emas; .env ixtiyoriy). Format:
//   KEY=VALUE        # izoh
//   export KEY=VALUE   (export prefiksi e'tiborga olinmaydi)
//   KEY="qiymat"  /  KEY='qiymat'   (tashqi qo'shtirnoq/apostrof olinadi)
// Bo'sh qatorlar va `#` bilan boshlanadigan qatorlar tashlanadi.
fn load_dotenv() -> HashMap<String, String> {
    match std::fs::read_to_string(".env") {
        Ok(c) => parse_dotenv(&c),
        Err(_) => HashMap::new(), // .env yo'q -> bo'sh (ixtiyoriy)
    }
}

// .env matn -> map. load_dotenv'dan ajratilgan (test qilinadigan sof funksiya).
fn parse_dotenv(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // `export KEY=VAL` -> `KEY=VAL`
        let line = line.strip_prefix("export ").map(str::trim).unwrap_or(line);
        let Some((key, val)) = line.split_once('=') else {
            continue; // `=` yo'q -> noto'g'ri qator, tashlaymiz
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let val = val.trim();
        // Tashqi juft qo'shtirnoq yoki apostrofni olib tashlaymiz.
        let val = if val.len() >= 2
            && ((val.starts_with('"') && val.ends_with('"'))
                || (val.starts_with('\'') && val.ends_with('\'')))
        {
            &val[1..val.len() - 1]
        } else {
            val
        };
        map.insert(key.to_string(), val.to_string());
    }
    map
}

// ---- arifmetika yordamchilari ----
fn is_num(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Flt(_))
}
fn to_f64(v: &Value) -> f64 {
    match v {
        Value::Int(n) => *n as f64,
        Value::Flt(x) => *x,
        _ => 0.0,
    }
}

fn int_arith(op: BinOp, a: i64, b: i64) -> EvalResult {
    use Value::*;
    // checked_*: overflow'da debug panic / release jim wrap o'rniga ikkala
    // rejimda bir xil Fluxon xatosi. i64::MIN / -1 (va % -1) Rust'da release'da
    // ham panic berardi — checked_div/checked_rem uni ham ushlaydi.
    Ok(match op {
        BinOp::Add => Int(a.checked_add(b).ok_or_else(|| Flow::overflow("+"))?),
        BinOp::Sub => Int(a.checked_sub(b).ok_or_else(|| Flow::overflow("-"))?),
        BinOp::Mul => Int(a.checked_mul(b).ok_or_else(|| Flow::overflow("*"))?),
        BinOp::Div => {
            if b == 0 {
                return Err(Flow::err("division by zero"));
            }
            Int(a.checked_div(b).ok_or_else(|| Flow::overflow("/"))?)
        }
        BinOp::Mod => {
            if b == 0 {
                return Err(Flow::err("division by zero (mod)"));
            }
            Int(a.checked_rem(b).ok_or_else(|| Flow::overflow("%"))?)
        }
        BinOp::Lt => Bool(a < b),
        BinOp::Le => Bool(a <= b),
        BinOp::Gt => Bool(a > b),
        BinOp::Ge => Bool(a >= b),
        _ => return Err(Flow::err("internal: unexpected int operator")),
    })
}

fn flt_arith(op: BinOp, a: f64, b: f64) -> EvalResult {
    use Value::*;
    Ok(match op {
        BinOp::Add => Flt(a + b),
        BinOp::Sub => Flt(a - b),
        BinOp::Mul => Flt(a * b),
        BinOp::Div => Flt(a / b),
        BinOp::Mod => Flt(a % b),
        BinOp::Lt => Bool(a < b),
        BinOp::Le => Bool(a <= b),
        BinOp::Gt => Bool(a > b),
        BinOp::Ge => Bool(a >= b),
        _ => return Err(Flow::err("internal: unexpected flt operator")),
    })
}

#[cfg(test)]
mod dotenv_tests {
    use super::parse_dotenv;

    #[test]
    fn parses_basic_and_comments() {
        let m = parse_dotenv("# izoh\nPORT=8080\n\nNAME=Aziza   \n  # yana izoh\nEMPTY=\n");
        assert_eq!(m.get("PORT").map(String::as_str), Some("8080"));
        assert_eq!(m.get("NAME").map(String::as_str), Some("Aziza"));
        assert_eq!(m.get("EMPTY").map(String::as_str), Some(""));
        assert_eq!(m.len(), 3); // izohlar/bo'sh qatorlar tashlandi
    }

    #[test]
    fn strips_quotes_and_export() {
        let m = parse_dotenv("export KEY=\"qiymat\"\nTOKEN='abc123'\nURL=http://x?a=1&b=2\n");
        assert_eq!(m.get("KEY").map(String::as_str), Some("qiymat"));
        assert_eq!(m.get("TOKEN").map(String::as_str), Some("abc123"));
        // = belgisi qiymat ichida bo'lsa, faqat BIRINCHI = ajratadi
        assert_eq!(m.get("URL").map(String::as_str), Some("http://x?a=1&b=2"));
    }

    #[test]
    fn skips_malformed_lines() {
        let m = parse_dotenv("noequalsign\n=novalue\nGOOD=ok\n");
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("GOOD").map(String::as_str), Some("ok"));
    }
}
