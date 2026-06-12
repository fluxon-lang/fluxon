// Fluxon interpreter — AST'ni to'g'ridan-to'g'ri bajaruvchi (tree-walking).
//
// Boshqaruv oqimi (ret/skip/stop/fail) Rust `Result`'ining `Err` tarmog'i
// orqali tarqatiladi: oddiy qiymatlar `Ok`, oqim-uzilishlari esa `Flow`.
// Bu `?` operatori bilan tabiiy yuqoriga ko'tariladi.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use parking_lot::RwLock;

use crate::ast::*;
use crate::value::{FnValue, Value};

// Lexical scope: ota-muhitga havola bilan zanjir. Arc<RwLock<>> — closure'lar,
// mutatsiya VA thread'lar orasida ulashish uchun (haqiqiy parallel HTTP).
// RwLock (Mutex emas): qidirish/o'qish ko'p o'quvchiga parallel ruxsat beradi,
// shunda parallel request'lar global scope'dagi funksiyalarni (masalan rekursiv
// `fib`) bir-birini bloklamasdan o'qiydi. Yozish (`<-`, bind) eksklyuziv.
pub type Env = Arc<RwLock<Scope>>;

// Scope zanjirining ota-havolasi. Muhim: ROOT (global) scope barcha thread'lar
// orasida ULASHILADI — uni har lookup'da klonlash/lock qilish atomik
// contention'ning asosiy manbai (cache-line bouncing 8 core'da). Shuning uchun
// root'ga yetadigan zanjir `Parent::Root(env)` ishlatadi: root Arc saqlanadi
// (oraliq scope'lar uni HECH QACHON klonlamaydi), va global muzlatilgandan keyin
// lookup root Arc'ga TEGMASDAN lock-free frozen snapshot'dan o'qiydi.
#[derive(Clone)]
pub enum Parent {
    // Root scope'ning o'zi — yuqorida ota yo'q.
    None,
    // Ota — root (global) scope. MARKER (Arc emas!) — root Arc saqlanmaydi,
    // shuning uchun fn chaqiruvi/scope ochilishida root refcount ATOMIK
    // urilmaydi (cache-line bouncing yo'q). Muzlatilgach lookup frozen
    // snapshot'dan, muzlatilmagan (top-level) holatda `Interp.global` Arc'idan
    // o'qiydi — ikkalasi ham `&self` orqali, klon shart emas.
    Root,
    // Ota — oddiy (root bo'lmagan) scope.
    Scope(Env),
}

pub struct Scope {
    // Nomlar — kichik VEKTOR (HashMap emas). Fn chaqiruvi/blok scope'lari odatda
    // 0-4 nom ushlaydi; bunday kichik to'plamda linear scan hash hisoblash +
    // HashMap allocation'idan tezroq, va per-call allocation arzon (bitta Vec
    // buffer, ikkita bo'sh HashMap o'rniga). Element: (nom, qiymat, mutable-mi).
    // mutable = `<-` bilan qayta tayinlanishi mumkinmi (`=`/`exp`/param immutable;
    // `<-` va loop var mutable).
    vars: Vec<(Box<str>, Value, bool)>,
    parent: Parent,
    // Bu scope root (global)mi? lookup root'ga yetganda, agar Interp global'ni
    // muzlatgan bo'lsa, lock-free snapshot'dan o'qiydi (parallel contention yo'q).
    is_root: bool,
    // Bu scope fn/lambda chaqiruvi chegarasimi? `=` bind tashqi o'zgaruvchini
    // qidirganda shu yerda to'xtaydi (funksiya izolyatsiyasi/shadowing). if/each/
    // match bloklari `false` — ular leksik jihatdan SHAFFOF: ichida `=` bilan
    // tashqi (bir xil fn ichidagi) o'zgaruvchini yangilash mumkin.
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
    // Berilgan `Parent` havola ostida yangi (bo'sh) child scope. `apply`/`if`/
    // `each`/`match` shu orqali scope ochadi. MUHIM: parent'ni LOCK QILMAYDI —
    // havola turi (Root/Scope) chaqiruvchidan keladi, shuning uchun rekursiv
    // fn chaqiruvida root Arc'ga umuman tegilmaydi (contention yo'q).
    fn child(parent: Parent) -> Env {
        Arc::new(RwLock::new(Scope {
            vars: Vec::new(),
            parent,
            is_root: false,
            is_fn_boundary: false, // if/each/match — shaffof blok
        }))
    }
    // Params soni bilan oldindan o'lchamlangan child (fn chaqiruvi — bind paytida
    // qayta-allocate bo'lmaydi).
    fn child_with_capacity(parent: Parent, cap: usize) -> Env {
        Arc::new(RwLock::new(Scope {
            vars: Vec::with_capacity(cap),
            parent,
            is_root: false,
            is_fn_boundary: true, // fn/lambda chaqiruvi — izolyatsiya chegarasi
        }))
    }
    // `env` Arc'ni child uchun ota-havolaga aylantiradi (faqat `is_root` ni
    // bilish uchun bitta lock). Top-level kod (if/each/match global env'da) shu
    // orqali boradi — single-threaded, contentionsiz. Fn chaqiruvi esa
    // `FnValue.parent` (Parent) ni to'g'ridan ishlatadi, bu yo'lga kirmaydi.
    fn parent_link(env: &Env) -> Parent {
        if env.read().is_root {
            Parent::Root
        } else {
            Parent::Scope(env.clone())
        }
    }
    // Berilgan env ostida child (yuqoridagi ikkisini birlashtiradi).
    fn child_of(env: &Env) -> Env {
        Scope::child(Scope::parent_link(env))
    }
    // Nomni e'lon qiladi. Allaqachon mavjud bo'lsa qiymat+mutable'ni yangilaydi
    // (shadow/qayta-bind — eski HashMap insert semantikasi: oxirgisi g'olib).
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
    // Nom qiymatini o'qiydi (oxirgi e'londan — orqadan oldinga scan).
    fn get(&self, name: &str) -> Option<&Value> {
        self.vars
            .iter()
            .rev()
            .find(|(n, _, _)| &**n == name)
            .map(|(_, v, _)| v)
    }
    // `<-` uchun: o'zgaruvchan slot'ni topadi. (slot, mutable-mi) qaytaradi.
    fn get_mut_entry(&mut self, name: &str) -> Option<(&mut Value, bool)> {
        self.vars
            .iter_mut()
            .rev()
            .find(|(n, _, _)| &**n == name)
            .map(|(_, v, m)| (v, *m))
    }
    // Builtins o'rnatish uchun: global nomga immutable qiymat qo'yadi.
    pub fn set_global(&mut self, name: &str, v: Value) {
        self.define(name, v, false);
    }
}

// Oqim-uzilish signallari va xatolar. Hammasi `Err` tomonida sayohat qiladi.
pub enum Flow {
    Return(Value),
    Skip,
    Stop,
    // fail [status] message — biznes yoki ichki xato.
    Fail {
        status: Option<i64>,
        message: String,
    },
    // Oddiy runtime xato (tip mosligi, noma'lum o'zgaruvchi, ...).
    Error(String),
}

impl Flow {
    pub fn err(msg: impl Into<String>) -> Flow {
        Flow::Error(msg.into())
    }

    // i64 arifmetikasi chegaradan oshganda yagona xato (issue #89). checked_*
    // bilan birga ishlatiladi: debug'dagi panic va release'dagi jim wrap o'rniga
    // ikkala rejimda ham bir xil, oshkora runtime xato beradi.
    pub fn overflow(who: &str) -> Flow {
        Flow::Error(format!("{}: son chegaradan oshdi (i64)", who))
    }
}

pub type EvalResult = Result<Value, Flow>;
type ExecResult = Result<Value, Flow>; // blok oxirgi ifoda qiymatini qaytaradi

// Fluxon-darajadagi fn chaqiriqlar uchun maksimal chuqurlik. Native stack
// `stacker::maybe_grow` bilan segmentlab o'sadi, shuning uchun haqiqiy chegara
// shu hisoblagich: limitga yetganda abort emas, graceful Flow::err. 1000 —
// Python'ning default rekursiya limiti bilan bir xil tartibda; real backend
// kodi bundan chuqur rekursiya qilmaydi, cheksiz rekursiya esa tez ushlanadi.
const MAX_CALL_DEPTH: usize = 1000;

// stacker parametrlari: red zone — bir Fluxon chaqirig'i ichida (keyingi
// tekshiruvgacha) ishlatilishi mumkin bo'lgan native stack'dan kattaroq bo'lishi
// shart (debug build'da ~15KB/daraja o'lchangan). Segment hajmi — har ajratish
// ~130 darajani sig'diradi, 1000 daraja uchun bir nechta segment yetadi.
const STACK_RED_ZONE: usize = 128 * 1024;
const STACK_GROW_SIZE: usize = 2 * 1024 * 1024;

thread_local! {
    // Joriy thread'dagi Fluxon chaqiriq chuqurligi. Thread-local: har HTTP request
    // o'z spawn_blocking thread'ida — bir request'ning rekursiyasi boshqasini
    // sanamaydi. Interp'ga maydon qo'shib bo'lmaydi (&self, Sync — Cell mumkin emas).
    static CALL_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// RAII guard: enter'da hisoblagichni oshiradi, Drop'da kamaytiradi. Drop'siz
// xato (`?`) yoki panic yo'lida hisoblagich oshib qolar va spawn_blocking
// thread'i qayta ishlatilganda keyingi request'larni zaharlar edi.
struct CallDepthGuard;

impl CallDepthGuard {
    fn enter(fname: &str) -> Result<CallDepthGuard, Flow> {
        CALL_DEPTH.with(|d| {
            let depth = d.get();
            if depth >= MAX_CALL_DEPTH {
                return Err(Flow::err(format!(
                    "rekursiya juda chuqur: '{}' chaqirig'ida {} darajalik limitga yetildi",
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
    // HTTP battery: ro'yxatga olingan marshrutlar. `http.on` to'ldiradi,
    // `http.serve` o'qiydi. Arc<Mutex> — server thread'lari bilan ulashiladi.
    pub routes: Arc<Mutex<Vec<crate::http_mod::Route>>>,
    // HTTP middleware (issue #67). `http.use` (barcha route'ga) va `http.before`
    // (yo'l prefiks bo'yicha) ikkalasi SHU BITTA ro'yxatga ketma-ket qo'shiladi —
    // shunda zanjir DEKLARATSIYA TARTIBIDA ishlaydi (use/before aralashganda ham,
    // masalan before req.ctx yozsa, undan keyin e'lon qilingan use logger ctx'ni
    // ko'radi). Route handler'dan OLDIN ishlaydi; biri `fail`/`rep` qaytarsa zanjir
    // to'xtaydi. `routes` kabi top-level to'ldiradi, server thread'lari o'qiydi.
    pub middlewares: Arc<Mutex<Vec<crate::http_mod::Middleware>>>,
    // CORS sozlamasi (issue #135). `http.cors` o'rnatadi, server thread'lari
    // o'qiydi: yoqilgan bo'lsa OPTIONS preflight avtomatik javob oladi va har
    // javobga `Access-Control-Allow-*` header'lar qo'shiladi. None — CORS o'chiq
    // (default, hech qanday header qo'shilmaydi). `routes` kabi top-level
    // to'ldiradi, server thread'lari o'qiydi.
    pub cors: Arc<Mutex<Option<crate::http_mod::CorsConfig>>>,
    // Static fayl mount'lari (issue #134). `http.static` to'ldiradi, server
    // thread'lari o'qiydi: aniq route topilmaganda prefiksga mos papkadan fayl
    // beriladi (route prioriteti: aniq route > static). `routes` kabi top-level
    // to'ldiradi, server thread'lari o'qiydi.
    pub statics: Arc<Mutex<Vec<crate::http_mod::StaticMount>>>,
    // O'ziga zaif havola: `http.serve` handler'larni server thread'larida
    // chaqirishi uchun `Arc<Interp>` kerak. `eval_call` (&self) shu yerdan
    // qayta tiklaydi. `new_arc` o'rnatadi.
    this: OnceLock<Weak<Interp>>,
    // Muzlatilgan global snapshot. `http.serve` chaqirilganda o'rnatiladi —
    // shundan keyin top-level kod tugagan, global o'zgarmaydi. `lookup` root'ga
    // yetganda LOCK-FREE shundan o'qiydi (Arc orqali ulashilgan, read lock yo'q),
    // shuning uchun parallel request'lar global qidiruvda bir-birini bloklamaydi.
    globals_frozen: OnceLock<Arc<HashMap<String, Value>>>,
    // DB battery: lazy ochilgan backend (jarayonga bitta, `$DATABASE_URL` bilan
    // tanlanadi). Birinchi `db.*` chaqiruvida ochiladi + auto-migration.
    db: OnceLock<Arc<dyn crate::db_mod::Db>>,
    // tbl schema registry: jadval -> meta (ustunlar + tartib + indekslar).
    // `Stmt::Tbl` to'ldiradi, db natijalarini post-process qilish (sym/json/bool)
    // va auto-migration (diff: ADD/DROP COLUMN, CREATE/DROP INDEX) uchun.
    // Arc<RwLock>: top-level'da yoziladi, parallel request thread'larida o'qiladi.
    pub schema: Arc<RwLock<HashMap<String, TableMeta>>>,
    // DB sxemasidan introspeksiya qilingan ustun tiplari cache'i (jadval -> ustun
    // -> fluxon-tip). `tbl` e'lon QILINMAGAN process (masalan ikki-process setup'da
    // o'qigich) `schema` bo'sh bo'lganda json ustunni shu cache orqali tiklaydi —
    // shunda json process chegarasidan qat'i nazar bir xil map qaytaradi (issue #63).
    pub(crate) db_schema: RwLock<HashMap<String, BTreeMap<String, String>>>,
    // .env fayl cache: LAZY — faqat birinchi `env.X` ishlatilganda joriy
    // katalogdagi `.env` o'qiladi va parse qilinadi. `env.X` umuman bo'lmasa,
    // fayl O'QILMAYDI (DB lazy-open bilan bir xil falsafa). Ustunlik: OS env >
    // .env fayl (deployda real muhit o'zgaruvchisi muhim).
    env_file: OnceLock<HashMap<String, String>>,
    // WS battery: hodisa handler'lari + jonli ulanishlar/xonalar/sessiya holati.
    // http `routes` kabi top-level kod (`ws.on`) to'ldiradi, `ws.serve` thread'lari
    // o'qiydi/yozadi. Arc — server thread'lari bilan ulashiladi.
    pub ws: Arc<crate::ws_mod::WsState>,
    // reg battery: nom -> funksiya registri (dinamik dispatch). `reg.add` to'ldiradi,
    // `reg.call` o'qiydi (istalgan thread'dan — http/ws handler ichidan ham).
    pub reg: Arc<crate::reg_mod::RegState>,
    // cron battery: rejalashtirilgan vazifalar + scheduler fon thread'i. `cron.on`
    // ro'yxatga oladi (bloklamaydi), fon thread o'qib o'z vaqtida handler chaqiradi.
    pub cron: Arc<crate::cron_mod::CronState>,
    // queue battery: fon navbati + bitta FIFO worker thread'i. `queue.push` ish
    // qo'shadi (bloklamaydi), `queue.on` handler ro'yxatga oladi; worker navbatdan
    // olib ketma-ket bajaradi.
    pub queue: Arc<crate::queue_mod::QueueState>,
    // Kutilayotgan (deferred) serverlar: `http.serve`/`ws.serve` darhol bloklamaydi,
    // balki bu yerga server tavsifini qo'shadi. Top-level kod tugagach (`run` oxiri)
    // hammasi BITTA umumiy tokio runtime'da spawn qilinadi — shunda HTTP + WS bir
    // jarayonda birga ishlaydi va `ws.room.send` HTTP handler ichidan chaqirila oladi.
    pub pending_servers: Arc<Mutex<Vec<crate::serve_mod::PendingServer>>>,
    // `use ./fayl` foydalanuvchi modullari uchun cache: canonical yo'l -> modul
    // namespace (`Value::Map`). Bir modul ikki marta import qilinsa qayta
    // bajarilmaydi — bir marta run qilinib natija shu yerda saqlanadi (idempotent).
    module_cache: Mutex<HashMap<PathBuf, Value>>,
    // Hozir yuklanayotgan modullar steki (canonical yo'llar) — sikllik importni
    // (A -> B -> A) aniqlash uchun. Modul run boshida push, tugaganda pop.
    module_loading: Mutex<Vec<PathBuf>>,
    // Joriy bajarilayotgan faylning katalogi. `use ./fayl` yo'lini shunga nisbatan
    // hal qiladi. Nested import uchun save/restore steki kabi ishlaydi: modul run
    // qilinganda uning katalogiga o'rnatiladi, tugagach tiklanadi.
    current_base: Mutex<PathBuf>,
}

// tbl ustun metasi — tip nomi (sym/json/bool konversiya) + modifikatorlar
// (CREATE TABLE: pk/uniq/null).
#[derive(Clone)]
pub struct ColMeta {
    pub type_name: String,
    pub modifiers: Vec<String>,
}

// tbl jadval metasi — ustunlar (nom -> meta), e'lon tartibi (barqaror ADD COLUMN
// uchun) va indekslar (CREATE/DROP INDEX diff uchun).
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
            module_loading: Mutex::new(Vec::new()),
            // Boshlang'ich base — joriy ish katalogi. `set_base` top-level fayl
            // katalogiga aniqlashtiradi (main.rs).
            current_base: Mutex::new(
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            ),
        }
    }

    // Top-level faylning katalogini o'rnatadi — `use ./fayl` yo'llari shunga
    // nisbatan hal qilinadi. main.rs `run`dan oldin bir marta chaqiradi.
    pub fn set_base(&self, dir: &std::path::Path) {
        *self.current_base.lock().unwrap() = dir.to_path_buf();
    }

    // Joriy bajarilayotgan faylning katalogi. pub(crate): `http.static` nisbiy
    // katalogni (`"./public"`) `use ./fayl` bilan bir xil qoidada — skript fayli
    // katalogiga nisbatan — hal qiladi.
    pub(crate) fn base_dir(&self) -> PathBuf {
        self.current_base.lock().unwrap().clone()
    }

    // `env.NOM` qiymatini topadi. Ustunlik: OS env (std::env) > .env fayl.
    // .env fayl LAZY — birinchi chaqiruvda bir marta o'qiladi va cache'lanadi;
    // `env.X` umuman ishlatilmasa, bu metod chaqirilmaydi -> fayl o'qilmaydi.
    // pub(crate): `ai` battery `$AI_KEY`/`$AI_MODEL`ni shu yo'l bilan (OS env >
    // .env) o'qiydi — `env.X` bilan bir xil ustunlik qoidasi.
    pub(crate) fn env_lookup(&self, name: &str) -> Value {
        if let Ok(v) = std::env::var(name) {
            return Value::Str(v); // OS env ustun
        }
        let file = self.env_file.get_or_init(load_dotenv);
        match file.get(name) {
            Some(v) => Value::Str(v.clone()),
            None => Value::Nil, // topilmadi -> `?? "default"`
        }
    }

    // DB backend'ni lazy ochadi (birinchi `db.*` da). Ochilganda tbl schema
    // registry'ni replay qilib auto-migration (`CREATE TABLE IF NOT EXISTS`)
    // bajaradi — `tbl` e'lon qilingan jadvallar zero-setup paydo bo'ladi.
    pub fn db(&self) -> Result<Arc<dyn crate::db_mod::Db>, Flow> {
        if let Some(d) = self.db.get() {
            return Ok(d.clone());
        }
        let d = crate::db_mod::open_from_env().map_err(Flow::err)?;
        self.migrate(d.as_ref())?;
        // Race: agar boshqa thread ham ochgan bo'lsa, biznikini tashlaymiz.
        let _ = self.db.set(d);
        Ok(self.db.get().unwrap().clone())
    }

    // Deklarativ auto-migration: `tbl` = DB schemasi uchun YAGONA MANBA. Joriy
    // DB holatini introspeksiya qilib `tbl` registry bilan farqini (diff)
    // hisoblaydi va kerakli DDL'ni bajaradi:
    //   - yangi jadval -> CREATE TABLE
    //   - yangi ustun  -> ADD COLUMN          (idempotent: bor bo'lsa jim pass)
    //   - olib tashlangan ustun -> BACKUP + DROP COLUMN  (yo'q bo'lsa jim pass)
    //   - index e'loni -> CREATE/DROP INDEX IF [NOT] EXISTS
    //   - olib tashlangan jadval -> BACKUP + DROP TABLE   (faqat Fluxon yaratganlar)
    //
    // KRITIK: idempotent va user manual SQL bilan birga ishlaganda yiqilmaydi —
    // "kerakli holatga keltir, allaqachon shunday bo'lsa tinch o't". DROP'lardan
    // oldin jadval DB ichida `_fluxon_bak_*` ga nusxalanadi (agent xatosiga himoya).
    fn migrate(&self, db: &dyn crate::db_mod::Db) -> Result<(), Flow> {
        use crate::db_mod::{
            ColDef, SqlVal, build_add_column, build_backup, build_create_index, build_drop_column,
            build_drop_index, coldef_foreign_key, index_name,
        };

        // 0. Fluxon boshqaradigan jadvallar reyestri (xavfsiz DROP uchun).
        db.exec(
            "CREATE TABLE IF NOT EXISTS _fluxon_schema (table_name TEXT PRIMARY KEY)",
            &[],
        )
        .map_err(Flow::err)?;

        // Backup nomi uchun migratsiya vaqti (unix secs). Faqat backup nomini
        // noyob qilish uchun — index nomlaridan FARQLI, bu yerda determinizm
        // shart emas.
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let schema = self.schema.read();

        // 1. Har registry jadval uchun: CREATE + ustun/index diff.
        for (table, meta) in schema.iter() {
            // ColDef'lar e'lon tartibida (barqaror ADD COLUMN).
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

            // DB'dagi joriy ustunlar.
            let db_cols: HashSet<String> = db
                .column_types(table)
                .map_err(Flow::err)?
                .into_iter()
                .map(|(n, _)| n)
                .collect();

            // 2. ADD COLUMN: registry'da bor, DB'da yo'q.
            for col in &meta.col_order {
                if !db_cols.contains(col) {
                    swallow_benign(db.exec(&build_add_column(table, &coldef(col)), &[]))?;
                }
            }

            // 3. ESKIRGAN INDEX DROP — ustun DROP'idan OLDIN. Sabab: index'lanган
            //    ustun olib tashlansa, eski `idx_<tbl>_<col>` hali DB'da turadi va
            //    ba'zi SQLite holatlarida `DROP COLUMN` "error in index ... after
            //    drop column: no such column" bilan rad etiladi -> deploy migrate
            //    qila olmaydi. Shu sabab avval kerak BO'LMAGAN Fluxon index'larini
            //    tashlaymiz, keyin ustunni xavfsiz drop qilamiz.
            let want_names: HashSet<String> = meta.indexes.iter().map(index_name).collect();
            for info in db.fluxon_indexes(table).map_err(Flow::err)? {
                if !want_names.contains(&info.name) {
                    db.exec(&build_drop_index(&info.name), &[])
                        .map_err(Flow::err)?;
                }
            }

            // 4. DROP COLUMN: DB'da bor, registry'da yo'q. BACKUP (jadval bo'yicha
            //    bir marta) -> DROP COLUMN (yo'q bo'lsa jim pass).
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

            // 5. YANGI INDEX CREATE — ustun DROP'idan KEYIN (yangi ustunlar
            //    allaqachon mavjud). IF NOT EXISTS idempotent.
            for idx in &meta.indexes {
                db.exec(&build_create_index(idx), &[]).map_err(Flow::err)?;
            }
        }

        // 5.5 FK RECONCILE — ALOHIDA pass (barcha jadval/ustun yaratilgandan keyin,
        //     parent jadval mavjudligi kafolatlanadi). DB'dagi HAQIQIY FK to'plamini
        //     (introspeksiya) `ref:tbl.col` deklaratsiyasi bilan solishtiramiz:
        //     faqat kodga emas, eski holatga ham qaraymiz. Farq bo'lsa (mavjud
        //     ustunga FK qo'shilgan/olib tashlangan) ALTER yetmaydi — jadvalni
        //     rebuild qilamiz (ma'lumot saqlanadi). Yangi ustun FK'si ADD COLUMN'da
        //     allaqachon qo'llangan; bu pass faqat mavjud ustunlar farqini yopadi.
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

        // 6. DROP TABLE: `_fluxon_schema` da bor, registry'da yo'q (source'dan
        //    tbl olib tashlangan). BACKUP -> DROP -> reyestrdan o'chir.
        //
        // MUHIM: registry BUTUNLAY bo'sh bo'lsa (hech qanday `tbl` e'lon
        //    qilinmagan), DROP'ni o'tkazib yuboramiz — bunday process schema
        //    dirijyori EMAS (faqat o'qiydi/yozadi, masalan ikki-process setup).
        //    Aks holda u boshqa process yaratgan barcha jadvalni o'chirib yuborardi.
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

    // Global scope'ni lock-free snapshot'ga muzlatadi. `http.serve` server'ni
    // ishga tushirishdan oldin chaqiradi. Bir marta — keyin global o'qish
    // lock'siz bo'ladi. (Top-level kod tugagan, mutatsiya kutilmaydi.)
    pub fn freeze_globals(&self) {
        // Frozen snapshot HASHMAP — global katta (builtin'lar + fn'lar), va u har
        // request'da O(1) qidiriladi. Global Vec'dan (oxirgi e'lon g'olib) quramiz.
        let mut snap: HashMap<String, Value> = HashMap::new();
        for (name, v, _) in self.global.read().vars.iter() {
            snap.insert(name.to_string(), v.clone());
        }
        let _ = self.globals_frozen.set(Arc::new(snap));
    }

    // Interp'ni Arc'ga o'rab, o'ziga zaif havolani o'rnatadi.
    pub fn new_arc() -> Arc<Self> {
        let arc = Arc::new(Self::new());
        let _ = arc.this.set(Arc::downgrade(&arc));
        arc
    }

    // `&self` dan `Arc<Interp>` ni qayta tiklaydi (http.serve uchun).
    pub fn arc_self(&self) -> Arc<Interp> {
        self.this
            .get()
            .and_then(|w| w.upgrade())
            .expect("Interp Arc orqali yaratilishi kerak (new_arc)")
    }

    pub fn run(&self, prog: &Program) -> Result<(), String> {
        // Birinchi o'tish: top-level fn/tbl e'lonlarini oldindan ro'yxatga olamiz
        // (hoisting), shunda tartibdan qat'i nazar bir-birini chaqira oladi va
        // har qanday `db.*` chaqiruvidan oldin schema tayyor bo'ladi.
        for stmt in prog {
            match stmt {
                Stmt::FnDecl {
                    name, params, body, ..
                } => {
                    let f = Value::Fn(Arc::new(FnValue {
                        params: params.clone(),
                        body: body.clone(),
                        // Top-level fn — ota root (marker, Arc emas).
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
            // fn/tbl allaqachon ro'yxatda — qayta bajarmaymiz.
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
                Err(Flow::Return(_)) => {} // top-level ret — e'tiborsiz
                Err(Flow::Skip) | Err(Flow::Stop) => {
                    return Err("skip/stop loop tashqarisida ishlatildi".into());
                }
            }
        }
        // Top-level tugadi — kutilayotgan serverlar (http.serve/ws.serve) bo'lsa,
        // hammasini bitta umumiy event-loopda ishga tushirib bloklaymiz. Server
        // bo'lmasa darhol qaytadi (oddiy skript normal tugaydi).
        // run_pending faqat Flow::Error qaytaradi (tokio runtime qura olmasa).
        if let Err(Flow::Error(e)) = crate::serve_mod::run_pending(&self.arc_self()) {
            return Err(e);
        }
        // Server yo'q bo'lsa run_pending darhol qaytadi — chiqishdan oldin fon
        // navbatidagi ishlar tugashini kutamiz (issue #105: faqat-queue skript
        // ishlarni bajarmasdan chiqib ketmasin). Server bo'lsa run_pending
        // bloklaydi va bu yerga umuman yetib kelmaymiz.
        self.queue_wait_drain();
        Ok(())
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
        let base = self.current_base.lock().unwrap().clone();
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
            .map_err(|e| Flow::err(format!("modul topilmadi '{}': {}", rel_path, e)))?;

        // 2. Cache hit — qayta bajarmaymiz (idempotent import).
        if let Some(v) = self.module_cache.lock().unwrap().get(&canon) {
            return Ok(v.clone());
        }

        // 3. Sikllik import: agar bu modul hozir yuklanish jarayonida bo'lsa
        //    (A -> B -> A), to'xtaymiz — aks holda cheksiz rekursiya.
        {
            let loading = self.module_loading.lock().unwrap();
            if loading.contains(&canon) {
                let chain: Vec<String> = loading
                    .iter()
                    .chain(std::iter::once(&canon))
                    .map(|p| p.display().to_string())
                    .collect();
                return Err(Flow::err(format!("sikllik import: {}", chain.join(" -> "))));
            }
        }
        self.module_loading.lock().unwrap().push(canon.clone());

        // 4. Faylni bajaramiz. Natijadan qat'i nazar steki'dan olib tashlaymiz.
        let result = self.run_module_file(&canon);
        self.module_loading.lock().unwrap().pop();
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
                "modulni o'qib bo'lmadi '{}': {}",
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
        let prev_base = self.current_base.lock().unwrap().clone();
        if let Some(dir) = canon.parent() {
            *self.current_base.lock().unwrap() = dir.to_path_buf();
        }
        let exec = self.exec_module_body(&prog, &mod_scope);
        *self.current_base.lock().unwrap() = prev_base;
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
                    return Err(Flow::err("skip/stop loop tashqarisida ishlatildi"));
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
                        return Err(Flow::err(
                            "'<-' chap tomoni o'zgaruvchi yoki '.maydon' bo'lishi kerak",
                        ));
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
                                "fail status int bo'lishi kerak, {} berildi",
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
                            "'{}' o'zgarmas (=) e'lon qilingan, '<-' bilan o'zgartirib bo'lmaydi",
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
                                "'{}' global muzlatilgan (server ishga tushgan) — \
                                 handler ichidan '<-' bilan o'zgartirib bo'lmaydi; \
                                 ulashilgan o'zgaruvchan holat uchun db'dan foydalaning",
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
                        "req.{} <- map kutadi, {} berildi",
                        field,
                        other.type_name()
                    )));
                }
            };
            *cell.lock().unwrap() = new_map;
            return Ok(());
        }
        Err(Flow::err(format!(
            "'.{}' ga '<-' bilan tayinlab bo'lmaydi (faqat req.ctx kabi kontekst maydoni o'zgartiriladi)",
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
                            "'{}' o'zgarmas (=) e'lon qilingan; blok ichidan ham \
                             qayta tayinlab bo'lmaydi (uni `<-` bilan e'lon qiling)",
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
                    "each faqat list/map/range/str ustidan yuradi, {} berildi",
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
                "each ... in inf bitta o'zgaruvchi kutadi (each i in inf)",
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
            Expr::Ident(name) => self.lookup(name, env),
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
                                    "map spread (...) faqat map bilan ishlaydi, {} berildi",
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
                            "'-' faqat songa, {} berildi",
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
                        "range (..) butun son talab qiladi, {}..{} berildi",
                        a.type_name(),
                        b.type_name()
                    ))),
                }
            }
            // inf faqat `each i in inf` da ma'noli — qiymat sifatida ishlatib bo'lmaydi.
            Expr::Inf => Err(Flow::err(
                "inf faqat `each i in inf` da ishlatiladi (qiymat emas)",
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
                                "fail status int bo'lishi kerak, {} berildi",
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
                        .ok_or_else(|| Flow::err(format!("noma'lum nom: {}", name)));
                }
                if let Some(v) = s.get(name) {
                    return Ok(v.clone());
                }
                s.parent.clone()
            };
            match parent {
                Parent::None => return Err(Flow::err(format!("noma'lum nom: {}", name))),
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
                            .ok_or_else(|| Flow::err(format!("noma'lum nom: {}", name)));
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
                "{:?} operatori {} va {} ga qo'llab bo'lmaydi",
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
                    _ => Err(Flow::err(format!("ws.{} guruhi yo'q", sub))),
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
                    .ok_or_else(|| Flow::err("list.filter: funksiya argumenti kerak"))?;
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
                    .ok_or_else(|| Flow::err("list.map: funksiya argumenti kerak"))?;
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
                    .ok_or_else(|| Flow::err("list.reduce: boshlang'ich qiymat kerak"))?;
                let f = it
                    .next()
                    .ok_or_else(|| Flow::err("list.reduce: funksiya argumenti kerak"))?;
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
                    .ok_or_else(|| Flow::err("list.find: funksiya argumenti kerak"))?;
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
                    .ok_or_else(|| Flow::err("list.any: funksiya argumenti kerak"))?;
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
                    .ok_or_else(|| Flow::err("list.all: funksiya argumenti kerak"))?;
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
                        "list.sort: komparator son qaytarishi kerak (manfiy/0/musbat), {} qaytardi",
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
                        "{}: {} ta argument kutilgan, {} berildi",
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
                "{} chaqirib bo'lmaydi (funksiya emas)",
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
                "{} tipida '.{}' maydoni yo'q",
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
                "{}[{}] indekslash qo'llab-quvvatlanmaydi",
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
                return Err(Flow::err("nolga bo'lish"));
            }
            Int(a.checked_div(b).ok_or_else(|| Flow::overflow("/"))?)
        }
        BinOp::Mod => {
            if b == 0 {
                return Err(Flow::err("nolga bo'lish (mod)"));
            }
            Int(a.checked_rem(b).ok_or_else(|| Flow::overflow("%"))?)
        }
        BinOp::Lt => Bool(a < b),
        BinOp::Le => Bool(a <= b),
        BinOp::Gt => Bool(a > b),
        BinOp::Ge => Bool(a >= b),
        _ => return Err(Flow::err("ichki: kutilmagan int operatori")),
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
        _ => return Err(Flow::err("ichki: kutilmagan flt operatori")),
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
