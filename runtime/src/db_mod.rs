// Flux DB battery — db.q/one/ins/up/del/put va db.tx.
//
// ARXITEKTURA: backend `Db` trait orqasiga yashiringan. Flux kodi (`db.*`) hech
// qachon o'zgarmaydi; backend bitta config nuqtasidan (`$DATABASE_URL` sxemasi)
// almashtiriladi. Bugun to'liq SQLite (rusqlite, bundled — server kerak emas);
// postgres/mysql keyin additiv ulanadi (hozir `Err` stub).
//
// Dialekt farqlari (placeholder uslubi, RETURNING, ON CONFLICT, identifikator
// quoting, BEGIN/SAVEPOINT sintaksisi) trait ichida. SQLite `$1` placeholder'ni
// native qo'llaydi, shuning uchun Flux'ning spec'dagi `$1` uslubi rewrite'siz
// o'tadi.
//
// Tranzaksiya qo'lda BEGIN/COMMIT/ROLLBACK/SAVEPOINT SQL orqali boshqariladi
// (rusqlite'ning lifetime-bog'liq Transaction tipi o'rniga) — shunda tx
// connection'ni egallaydi (`'static`) va thread_local kontekstda yashashi mumkin.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use rusqlite::types::{Value as RqVal, ValueRef};
use rusqlite::{Connection, params_from_iter};

use crate::builtins::{json_decode, json_encode};
use crate::interp::{Flow, Interp};
use crate::value::Value;

// --- backend-neytral hujayra qiymati ---

#[derive(Clone, Debug)]
pub enum SqlVal {
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
    Null,
}

pub type Row = BTreeMap<String, SqlVal>;

// --- tbl ustun ta'rifi (CREATE TABLE generatsiya uchun) ---

#[derive(Clone)]
pub struct ColDef {
    pub name: String,
    pub type_name: String,
    pub modifiers: Vec<String>,
}

// --- Db trait: dialekt-neytral backend interfeysi ---

pub trait Db: Send + Sync {
    // SELECT-uslubidagi so'rov; natija qatorlar (map'lar).
    fn query(&self, sql: &str, params: &[SqlVal]) -> Result<Vec<Row>, String>;
    // Natija qaytarmaydigan amal (up/del); ta'sirlangan qatorlar soni.
    fn exec(&self, sql: &str, params: &[SqlVal]) -> Result<usize, String>;
    // Natija qaytaruvchi amal (ins/put) — RETURNING * orqali.
    fn query_returning(&self, sql: &str, params: &[SqlVal]) -> Result<Vec<Row>, String>;

    // --- dialekt-specific SQL generatsiya ---
    fn build_insert(&self, table: &str, cols: &[String]) -> String;
    fn build_update(&self, table: &str, set: &[String], whr: &[String]) -> String;
    fn build_delete(&self, table: &str, whr: &[String]) -> String;
    fn build_upsert(&self, table: &str, set: &[String], key: &[String]) -> String;
    fn build_create_table(&self, table: &str, cols: &[ColDef]) -> String;

    // Jadval ustunlarining (nom, flux-tip) ro'yxati — DB sxemasidan introspeksiya.
    // `tbl` e'lon qilinmagan process json ustunni shu orqali topadi (issue #63).
    // Faqat json aniq tiklanadi (sym/bool SQLite'da TEXT/INTEGER bo'lib matnan
    // farqlanmaydi). Jadval topilmasa bo'sh ro'yxat.
    fn column_types(&self, table: &str) -> Result<Vec<(String, String)>, String>;

    // Tranzaksiya ochish — connection'ni egallagan `'static` obyekt qaytaradi.
    fn begin(&self) -> Result<Box<dyn DbTx>, String>;
}

// Aktiv tranzaksiya — barcha db.* chaqiruvlari shu yagona connection'da bajariladi.
pub trait DbTx: Send {
    fn query(&self, sql: &str, params: &[SqlVal]) -> Result<Vec<Row>, String>;
    fn exec(&self, sql: &str, params: &[SqlVal]) -> Result<usize, String>;
    fn query_returning(&self, sql: &str, params: &[SqlVal]) -> Result<Vec<Row>, String>;
    // Nested tx: SAVEPOINT bilan.
    fn savepoint(&self, name: &str) -> Result<(), String>;
    fn release(&self, name: &str) -> Result<(), String>; // ichki commit
    fn rollback_to(&self, name: &str) -> Result<(), String>; // ichki rollback
    fn commit(self: Box<Self>) -> Result<(), String>;
    fn rollback(self: Box<Self>) -> Result<(), String>;
    // Tx connection'i orqali ustun tiplarini introspeksiya qiladi — uncommitted
    // DDL ko'rinishi uchun global pool o'rniga shu ishlatiladi (issue #63).
    fn column_types(&self, table: &str) -> Result<Vec<(String, String)>, String>;
}

// ==================== SQLite backend ====================

// Connection pool — bir nechta connection saqlaydi. Tx'siz amallar (q/one/ins/
// up/del/put) pooldan connection OLADI va darhol QAYTARADI; tx esa connection'ni
// commit/rollback gacha egallaydi. Shu sababli BIR request tx ichida bo'lsa ham
// boshqa PARALLEL request global connection topadi — "connection band" muammosi
// yo'q (foydalanuvchi tasdiqlagan dizayn: har tx alohida connection).
//
// `:memory:` holatida har connection ALOHIDA bo'sh DB bo'lib qolmasligi uchun
// `file::memory:?cache=shared` ishlatamiz va bitta "keepalive" connection ochiq
// turadi (oxirgi connection yopilsa shared-cache DB o'chadi).
struct Pool {
    spec: String,               // rusqlite ga beriladigan ochish spetsifikatsiyasi
    flags: rusqlite::OpenFlags, // URI rejimi (shared-cache) kerak bo'lganda
    idle: Mutex<Vec<Connection>>,
    // :memory: shared-cache DB'ni tirik tutadi. Mutex — Connection Sync emas,
    // lekin Pool (Arc<dyn Db> ichida) Sync bo'lishi shart.
    _keepalive: Mutex<Option<Connection>>,
}

impl Pool {
    // Pooldan connection oladi (bo'sh bo'lmasa yangi ochadi).
    fn checkout(&self) -> Result<Connection, String> {
        if let Some(c) = self.idle.lock().unwrap().pop() {
            return Ok(c);
        }
        self.open_one()
    }
    // Connection'ni poolga qaytaradi.
    fn checkin(&self, conn: Connection) {
        self.idle.lock().unwrap().push(conn);
    }
    fn open_one(&self) -> Result<Connection, String> {
        let conn = Connection::open_with_flags(&self.spec, self.flags)
            .map_err(|e| format!("sqlite ochilmadi ({}): {}", self.spec, e))?;
        // Har connection'da: WAL, FK, busy_timeout.
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
    // `rest` — DATABASE_URL'ning `sqlite:` dan keyingi qismi: fayl yo'li yoki
    // `:memory:`.
    pub fn open(rest: &str) -> Result<Self, String> {
        let is_mem = rest.is_empty() || rest == ":memory:" || rest == "memory:";
        // :memory: -> shared-cache URI (barcha connection bir DB'ni ko'radi).
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
        // :memory: -> keepalive (oxirgi connection yopilsa shared DB o'chmasin).
        if is_mem {
            *pool._keepalive.lock().unwrap() = Some(pool.open_one()?);
        }
        // Bitta connection oldindan ochib pulda qoldiramiz (ochilish xatosini
        // shu yerda aniqlaymiz).
        let first = pool.open_one()?;
        pool.idle.lock().unwrap().push(first);

        Ok(SqliteDb {
            pool: Arc::new(pool),
        })
    }
}

// SqlVal -> rusqlite bog'lash qiymati.
fn to_rqval(v: &SqlVal) -> RqVal {
    match v {
        SqlVal::Int(n) => RqVal::Integer(*n),
        SqlVal::Real(x) => RqVal::Real(*x),
        SqlVal::Text(s) => RqVal::Text(s.clone()),
        SqlVal::Blob(b) => RqVal::Blob(b.clone()),
        SqlVal::Null => RqVal::Null,
    }
}

// rusqlite ValueRef -> SqlVal (o'qishda).
fn from_ref(r: ValueRef<'_>) -> SqlVal {
    match r {
        ValueRef::Null => SqlVal::Null,
        ValueRef::Integer(n) => SqlVal::Int(n),
        ValueRef::Real(x) => SqlVal::Real(x),
        ValueRef::Text(t) => SqlVal::Text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => SqlVal::Blob(b.to_vec()),
    }
}

// Bitta prepared statement'dan barcha qatorlarni map sifatida o'qiydi.
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
    format!("db xato: {} (so'rov: {})", e, sql.trim())
}

impl Db for SqliteDb {
    fn query(&self, sql: &str, params: &[SqlVal]) -> Result<Vec<Row>, String> {
        // Pooldan connection olamiz, ishlatamiz, darhol qaytaramiz — boshqa
        // parallel so'rov (yoki tx) global'ni band qilmaydi.
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
        // SQLite'da RETURNING oddiy query kabi o'qiladi.
        self.query(sql, params)
    }

    fn column_types(&self, table: &str) -> Result<Vec<(String, String)>, String> {
        let conn = self.pool.checkout()?;
        let r = sqlite_column_types(&conn, table);
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
        // Insert ustunlari = key ∪ set (key birinchi, deterministik tartib).
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
        // ON CONFLICT(key) DO UPDATE SET col=excluded.col (faqat set ustunlari).
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

    fn begin(&self) -> Result<Box<dyn DbTx>, String> {
        // Tx POOL'dan alohida connection oladi — global pool band qolmaydi, boshqa
        // parallel so'rovlar bemalol ishlaydi (foydalanuvchi tasdiqlagan dizayn).
        let conn = self.pool.checkout()?;
        // BEGIN IMMEDIATE — write-lock'ni oldindan oladi (race-safe, overdraft yo'q).
        if let Err(e) = conn.execute_batch("BEGIN IMMEDIATE") {
            self.pool.checkin(conn);
            return Err(format!("tx boshlanmadi: {e}"));
        }
        Ok(Box::new(SqliteTx {
            conn: Some(conn),
            pool: self.pool.clone(), // commit/rollback'da connection poolga qaytadi
        }))
    }
}

// SQLite jadval ustunlarini introspeksiya qiladi: pragma_table_info'dan e'lon
// qilingan tipni olib Flux tip nomiga aylantiradi. Jadval bo'lmasa bo'sh ro'yxat.
fn sqlite_column_types(conn: &Connection, table: &str) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare("SELECT name, type FROM pragma_table_info(?1)")
        .map_err(|e| sql_err("pragma_table_info", e))?;
    let rows = stmt
        .query_map([table], |row| {
            let name: String = row.get(0)?;
            let decl: String = row.get(1)?;
            Ok((name, sqlite_decl_to_flux_type(&decl)))
        })
        .map_err(|e| sql_err("pragma_table_info", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| sql_err("pragma_table_info", e))?);
    }
    Ok(out)
}

// E'lon qilingan SQLite tipini Flux tip nomiga moslaydi. Hozir faqat json
// ahamiyatli (sqlval_to_value uni map/list'ga dekod qiladi); qolgani matn bo'lib
// qaytadi va maxsus konversiyaga tushmaydi.
fn sqlite_decl_to_flux_type(decl: &str) -> String {
    if decl.eq_ignore_ascii_case("json") {
        "json".to_string()
    } else {
        decl.to_ascii_lowercase()
    }
}

// SQLite identifikator quoting: "..." (ichidagi " -> "").
fn q_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

// tbl ustunini SQLite CREATE TABLE ta'rifiga aylantiradi.
fn sqlite_column_def(c: &ColDef) -> String {
    let has = |m: &str| c.modifiers.iter().any(|x| x == m);
    let sql_type = match c.type_name.as_str() {
        "serial" => "INTEGER",
        "int" | "money" | "now" | "bool" => "INTEGER",
        "flt" => "REAL",
        // json -> e'lon qilingan tip "JSON". SQLite uni TEXT sifatida saqlaydi
        // (json qiymat doim {}/[] — NUMERIC affinity uni matn qoldiradi), lekin
        // e'lon tipi DB sxemasida qoladi: `tbl` e'lon qilinmagan process o'qiganda
        // introspeksiya orqali ustun json ekanini tiklash uchun (issue #63).
        "json" => "JSON",
        // str/sym va noma'lum -> TEXT
        _ => "TEXT",
    };
    let mut def = format!("{} {}", q_ident(&c.name), sql_type);
    // serial -> avtomat o'suvchi primary key.
    if c.type_name == "serial" {
        def.push_str(" PRIMARY KEY AUTOINCREMENT");
    } else if has("pk") {
        def.push_str(" PRIMARY KEY");
    }
    if has("uniq") {
        def.push_str(" UNIQUE");
    }
    if c.type_name == "now" {
        def.push_str(" DEFAULT CURRENT_TIMESTAMP");
    }
    def
}

// --- SqliteTx: aktiv tranzaksiya (connection'ni egallaydi) ---

struct SqliteTx {
    conn: Option<Connection>,
    // Connection'ni qaytarish uchun pool (Arc klon — tx yashagancha tirik).
    pool: Arc<Pool>,
}

impl SqliteTx {
    fn conn(&self) -> Result<&Connection, String> {
        self.conn.as_ref().ok_or_else(|| "tx yopilgan".to_string())
    }
    // commit/rollback'da connection'ni poolga qaytaradi.
    fn give_back(&mut self) {
        if let Some(conn) = self.conn.take() {
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
        // ROLLBACK TO savepoint'ni bekor qiladi, lekin savepoint'ni stack'da
        // qoldiradi — RELEASE bilan tozalaymiz, aks holda nested holatda chalkashadi.
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
        // Agar commit/rollback chaqirilmagan bo'lsa (panik va h.k.) — rollback qilib
        // connection'ni qaytaramiz, aks holda DB qulflanib qoladi.
        if let Some(conn) = self.conn.take() {
            let _ = conn.execute_batch("ROLLBACK");
            self.pool.checkin(conn);
        }
    }
}

// ==================== backend tanlash (yagona config nuqtasi) ====================

pub fn open_from_env() -> Result<Arc<dyn Db>, String> {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:flux.db".to_string());
    match url.split_once(':') {
        Some(("sqlite", rest)) => Ok(Arc::new(SqliteDb::open(rest)?)),
        Some(("postgres", _)) | Some(("postgresql", _)) => {
            Err("postgres backend hali ulanmagan (DATABASE_URL=sqlite:... ishlating)".to_string())
        }
        Some(("mysql", _)) => {
            Err("mysql backend hali ulanmagan (DATABASE_URL=sqlite:... ishlating)".to_string())
        }
        _ => Err(format!("noma'lum DATABASE_URL sxemasi: {url}")),
    }
}

// ==================== Interp dispatch ====================

// Joriy thread'dagi aktiv tranzaksiya. HTTP har request'ni alohida spawn_blocking
// thread'da bajaradi, shuning uchun thread_local to'g'ri izolyatsiya beradi.
thread_local! {
    static CURRENT_TX: std::cell::RefCell<Option<Box<dyn DbTx>>> =
        const { std::cell::RefCell::new(None) };
    // Nested SAVEPOINT chuqurligi (unikal nom uchun).
    static TX_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

// Joriy tx bo'lsa unga yo'naltiradi, aks holda global Db'ga. f — tx/db ustida
// ishlaydigan closure.
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
    // db.<func> chaqiruvlari. eval_call shu yerga yo'naltiradi.
    pub fn db_dispatch(self: &Arc<Self>, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            "q" => self.db_q(args),
            "one" => self.db_one(args),
            "ins" => self.db_ins(args),
            "up" => self.db_up(args),
            "del" => self.db_del(args),
            "put" => self.db_put(args),
            "tx" => self.db_tx(args),
            _ => Err(Flow::err(format!("db modulida '{}' funksiyasi yo'q", func))),
        }
    }

    // db.q sql [params?] -> qatorlar ro'yxati (map'lar).
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

    // db.one sql [params?] -> birinchi qator yoki nil.
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

    // db.ins "table" {map} -> qo'shilgan qator.
    fn db_ins(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let table = arg_table(&args, "db.ins")?;
        let map = arg_map(&args, 1, "db.ins")?;
        let (cols, vals) = self.map_to_cols(&table, &map)?;
        if cols.is_empty() {
            return Err(Flow::err("db.ins: bo'sh map — hech narsa qo'shilmaydi"));
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
            return Err(Flow::err("db.up: o'zgartirish map'i bo'sh"));
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
                "db.del: shart map'i bo'sh — butun jadval o'chmasligi uchun rad etildi",
            ));
        }
        let sql = self.db_builder(|db| db.build_delete(&table, &whr_cols))?;
        with_db(self, |tx| tx.exec(&sql, &vals), |db| db.exec(&sql, &vals))?;
        Ok(Value::Nil)
    }

    // db.put "table" {set} {key} -> upsert qatori.
    fn db_put(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let table = arg_table(&args, "db.put")?;
        let set = arg_map(&args, 1, "db.put")?;
        let key = arg_map(&args, 2, "db.put")?;
        let (set_cols, _) = self.map_to_cols(&table, &set)?;
        let (key_cols, _) = self.map_to_cols(&table, &key)?;
        if key_cols.is_empty() {
            return Err(Flow::err("db.put: kalit map'i bo'sh"));
        }
        // Bog'lash tartibi = build_upsert dagi ustun tartibi: key ∪ set.
        let mut cols: Vec<String> = key_cols.clone();
        for c in &set_cols {
            if !cols.contains(c) {
                cols.push(c.clone());
            }
        }
        // Birlashgan map: key + set (set ustun qiymati ustivor).
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

    // db.tx \-> ... — atomik blok. Nested bo'lsa SAVEPOINT.
    fn db_tx(self: &Arc<Self>, args: Vec<Value>) -> Result<Value, Flow> {
        let lambda = match args.into_iter().next() {
            Some(f @ (Value::Fn(_) | Value::Native(_))) => f,
            _ => {
                return Err(Flow::err(
                    "db.tx: argument funksiya (\\-> ...) bo'lishi kerak",
                ));
            }
        };

        let already = CURRENT_TX.with(|c| c.borrow().is_some());
        if already {
            return self.tx_nested(lambda);
        }
        self.tx_outer(lambda)
    }

    // Birinchi (tashqi) tx: BEGIN ... COMMIT/ROLLBACK.
    fn tx_outer(self: &Arc<Self>, lambda: Value) -> Result<Value, Flow> {
        let tx = self.db()?.begin().map_err(Flow::err)?;
        CURRENT_TX.with(|c| *c.borrow_mut() = Some(tx));

        let result = self.apply(lambda, vec![]);

        // tx'ni thread_local'dan qaytarib olamiz (commit/rollback uni egallaydi).
        let tx = CURRENT_TX.with(|c| c.borrow_mut().take());
        let tx = match tx {
            Some(tx) => tx,
            None => return Err(Flow::err("ichki: tx konteksti yo'qoldi")),
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
                // skip/stop -> aniqroq xato
                match flow {
                    Flow::Skip | Flow::Stop => Err(Flow::err("db.tx ichida skip/stop ishlatildi")),
                    other => Err(other),
                }
            }
        }
    }

    // Nested tx: joriy tx ustida SAVEPOINT.
    fn tx_nested(self: &Arc<Self>, lambda: Value) -> Result<Value, Flow> {
        let depth = TX_DEPTH.with(|d| {
            let n = d.get() + 1;
            d.set(n);
            n
        });
        let name = format!("flux_sp_{depth}");

        let sp_res = CURRENT_TX.with(|c| {
            c.borrow()
                .as_ref()
                .map(|tx| tx.savepoint(&name))
                .unwrap_or_else(|| Err("ichki: nested tx konteksti yo'q".to_string()))
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
                    Flow::Skip | Flow::Stop => Err(Flow::err("db.tx ichida skip/stop ishlatildi")),
                    other => Err(other),
                }
            }
        }
    }

    // build_* trait metodini global db ustida chaqiradi (SQL generatsiya
    // backend'ga bog'liq, lekin db.* ichida tx bo'lsa ham bir xil dialekt).
    fn db_builder(&self, f: impl FnOnce(&dyn Db) -> String) -> Result<String, Flow> {
        let db = self.db()?;
        Ok(f(db.as_ref()))
    }
}

// --- Value <-> SqlVal va schema-aware konversiya ---

impl Interp {
    // Flux map'ni (ustun, qiymat) ro'yxatlariga ajratadi. BTreeMap tartibi
    // deterministik — bog'lash bilan mos keladi.
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

    // Berilgan ustunlar tartibida map'dan qiymatlarni oladi (upsert uchun).
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

    // Flux Value -> SqlVal (yozishda). json ustunga map/list -> json_encode.
    fn value_to_sqlval(&self, table: &str, col: &str, v: &Value) -> Result<SqlVal, Flow> {
        Ok(match v {
            Value::Int(n) => SqlVal::Int(*n),
            Value::Flt(x) => SqlVal::Real(*x),
            Value::Str(s) => SqlVal::Text(s.clone()),
            Value::Bool(b) => SqlVal::Int(if *b { 1 } else { 0 }),
            Value::Nil => SqlVal::Null,
            Value::Sym(s) => SqlVal::Text(s.clone()),
            Value::List(_) | Value::Map(_) => {
                // Faqat json ustunga ruxsat (yoki schema noma'lum bo'lsa ham
                // qulaylik uchun ruxsat berib enkod qilamiz).
                if self.col_type(table, col).as_deref() == Some("json")
                    || self.col_type(table, col).is_none()
                {
                    SqlVal::Text(json_encode(v))
                } else {
                    return Err(Flow::err(format!(
                        "db: '{}.{}' ustuniga {} yozib bo'lmaydi (json ustun emas)",
                        table,
                        col,
                        v.type_name()
                    )));
                }
            }
            Value::Fn(_) | Value::Native(_) => {
                return Err(Flow::err("db: funksiyani DB'ga yozib bo'lmaydi"));
            }
        })
    }

    // Qatorni Flux map'ga aylantiradi, schema bo'yicha sym/json/bool tiklanadi.
    fn row_to_value(&self, table: Option<&str>, row: Row) -> Value {
        let mut m = BTreeMap::new();
        for (col, cell) in row {
            let ty = table.and_then(|t| self.col_type(t, &col));
            m.insert(col, sqlval_to_value(cell, ty.as_deref()));
        }
        Value::Map(m)
    }

    // Ustun tipini oladi. Birlamchi manba — joriy process'da `tbl` bilan e'lon
    // qilingan schema registry. U bo'lmasa (masalan ikki-process setup'da
    // o'qigich tbl e'lon qilmaydi) DB sxemasidan introspeksiya bilan tiklaymiz —
    // shunda json ustun process chegarasidan qat'i nazar bir xil map qaytaradi.
    fn col_type(&self, table: &str, col: &str) -> Option<String> {
        if let Some(cols) = self.schema.read().get(table)
            && let Some(c) = cols.get(col)
        {
            return Some(c.type_name.clone());
        }
        self.db_col_type(table, col)
    }

    // DB sxemasini introspeksiya qilib ustun tipini topadi (jadval bo'yicha
    // cache'lanadi — har qator uchun qayta so'rov bo'lmaydi). DB allaqachon ochiq:
    // bu metod faqat natija qatorini Value'ga aylantirish paytida chaqiriladi.
    //
    // Tranzaksiya ichida bo'lsa, uncommitted DDL ko'rinishi uchun tx connection
    // ishlatiladi — global pool connection bu DDL ni ko'ra olmaydi (issue #63).
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

// SqlVal -> Flux Value, ustun tipi bo'yicha post-process.
fn sqlval_to_value(cell: SqlVal, col_type: Option<&str>) -> Value {
    let base = match cell {
        SqlVal::Int(n) => Value::Int(n),
        SqlVal::Real(x) => Value::Flt(x),
        SqlVal::Text(s) => Value::Str(s),
        SqlVal::Blob(b) => Value::Str(String::from_utf8_lossy(&b).into_owned()),
        SqlVal::Null => Value::Nil,
    };
    match (col_type, &base) {
        // sym ustun: DB matn -> Flux symbol.
        (Some("sym"), Value::Str(s)) => Value::Sym(s.clone()),
        // json ustun: matn -> dekod qilingan map/list.
        (Some("json"), Value::Str(s)) => json_decode(s).unwrap_or(base),
        // bool ustun: int 0/1 -> bool.
        (Some("bool"), Value::Int(n)) => Value::Bool(*n != 0),
        _ => base,
    }
}

// --- argument yordamchilari ---

fn arg_sql(args: &[Value], who: &str) -> Result<String, Flow> {
    match args.first() {
        Some(Value::Str(s)) => Ok(s.clone()),
        _ => Err(Flow::err(format!(
            "{}: 1-argument SQL (str) bo'lishi kerak",
            who
        ))),
    }
}

fn arg_table(args: &[Value], who: &str) -> Result<String, Flow> {
    match args.first() {
        Some(Value::Str(s)) => Ok(s.clone()),
        _ => Err(Flow::err(format!(
            "{}: 1-argument jadval nomi (str) bo'lishi kerak",
            who
        ))),
    }
}

fn arg_map(args: &[Value], i: usize, who: &str) -> Result<BTreeMap<String, Value>, Flow> {
    match args.get(i) {
        Some(Value::Map(m)) => Ok(m.clone()),
        _ => Err(Flow::err(format!(
            "{}: {}-argument map ({{...}}) bo'lishi kerak",
            who,
            i + 1
        ))),
    }
}

// db.q/one ning 2-argumenti: ixtiyoriy params ro'yxati.
fn arg_params(args: &[Value], i: usize) -> Result<Vec<SqlVal>, Flow> {
    match args.get(i) {
        None | Some(Value::Nil) => Ok(vec![]),
        Some(Value::List(xs)) => xs.iter().map(param_to_sqlval).collect(),
        Some(other) => Err(Flow::err(format!(
            "db: parametrlar ro'yxat ([...]) bo'lishi kerak, {} berildi",
            other.type_name()
        ))),
    }
}

// Param qiymatini SqlVal'ga (schema'siz — q/one params ustunsiz).
fn param_to_sqlval(v: &Value) -> Result<SqlVal, Flow> {
    Ok(match v {
        Value::Int(n) => SqlVal::Int(*n),
        Value::Flt(x) => SqlVal::Real(*x),
        Value::Str(s) => SqlVal::Text(s.clone()),
        Value::Bool(b) => SqlVal::Int(if *b { 1 } else { 0 }),
        Value::Nil => SqlVal::Null,
        Value::Sym(s) => SqlVal::Text(s.clone()), // symbol -> matn (filter mosligi)
        Value::List(_) | Value::Map(_) => SqlVal::Text(json_encode(v)),
        Value::Fn(_) | Value::Native(_) => {
            return Err(Flow::err(
                "db: funksiyani parametr sifatida uzatib bo'lmaydi",
            ));
        }
    })
}

// SQL'dan asosiy jadval nomini ajratadi: lowercase ` from ` dan keyingi
// identifikator. Join/alias'da cheklov — eng keng tarqalgan `from <table>` holati
// uchun sym/json konversiya ishlaydi.
fn extract_from_table(sql: &str) -> Option<String> {
    let lower = sql.to_lowercase();
    let pos = lower.find(" from ")?;
    let after = &sql[pos + 6..];
    let tok: String = after
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if tok.is_empty() { None } else { Some(tok) }
}
