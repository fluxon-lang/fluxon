// Flux interpreter — AST'ni to'g'ridan-to'g'ri bajaruvchi (tree-walking).
//
// Boshqaruv oqimi (ret/skip/stop/fail) Rust `Result`'ining `Err` tarmog'i
// orqali tarqatiladi: oddiy qiymatlar `Ok`, oqim-uzilishlari esa `Flow`.
// Bu `?` operatori bilan tabiiy yuqoriga ko'tariladi.

use std::collections::BTreeMap;
use std::collections::HashMap;
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
}

impl Scope {
    pub fn root() -> Env {
        Arc::new(RwLock::new(Scope {
            vars: Vec::new(),
            parent: Parent::None,
            is_root: true,
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
        }))
    }
    // Params soni bilan oldindan o'lchamlangan child (fn chaqiruvi — bind paytida
    // qayta-allocate bo'lmaydi).
    fn child_with_capacity(parent: Parent, cap: usize) -> Env {
        Arc::new(RwLock::new(Scope {
            vars: Vec::with_capacity(cap),
            parent,
            is_root: false,
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
}

pub type EvalResult = Result<Value, Flow>;
type ExecResult = Result<Value, Flow>; // blok oxirgi ifoda qiymatini qaytaradi

pub struct Interp {
    pub global: Env,
    // HTTP battery: ro'yxatga olingan marshrutlar. `http.on` to'ldiradi,
    // `http.serve` o'qiydi. Arc<Mutex> — server thread'lari bilan ulashiladi.
    pub routes: Arc<Mutex<Vec<crate::http_mod::Route>>>,
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
    // tbl schema registry: jadval -> (ustun -> meta). `Stmt::Tbl` to'ldiradi,
    // db natijalarini post-process qilish (sym/json/bool) va auto-migration uchun.
    // Arc<RwLock>: top-level'da yoziladi, parallel request thread'larida o'qiladi.
    pub schema: Arc<RwLock<HashMap<String, BTreeMap<String, ColMeta>>>>,
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
}

// tbl ustun metasi — tip nomi (sym/json/bool konversiya) + modifikatorlar
// (CREATE TABLE: pk/uniq/null).
#[derive(Clone)]
pub struct ColMeta {
    pub type_name: String,
    pub modifiers: Vec<String>,
}

impl Interp {
    pub fn new() -> Self {
        let global = Scope::root();
        crate::builtins::install(&global);
        Interp {
            global,
            routes: Arc::new(Mutex::new(Vec::new())),
            this: OnceLock::new(),
            globals_frozen: OnceLock::new(),
            db: OnceLock::new(),
            schema: Arc::new(RwLock::new(HashMap::new())),
            env_file: OnceLock::new(),
            ws: Arc::new(crate::ws_mod::WsState::new()),
            reg: Arc::new(crate::reg_mod::RegState::new()),
            cron: Arc::new(crate::cron_mod::CronState::new()),
        }
    }

    // `env.NOM` qiymatini topadi. Ustunlik: OS env (std::env) > .env fayl.
    // .env fayl LAZY — birinchi chaqiruvda bir marta o'qiladi va cache'lanadi;
    // `env.X` umuman ishlatilmasa, bu metod chaqirilmaydi -> fayl o'qilmaydi.
    fn env_lookup(&self, name: &str) -> Value {
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

    // schema registry'dagi har jadval uchun CREATE TABLE IF NOT EXISTS.
    fn migrate(&self, db: &dyn crate::db_mod::Db) -> Result<(), Flow> {
        let schema = self.schema.read();
        for (table, cols) in schema.iter() {
            let coldefs: Vec<crate::db_mod::ColDef> = cols
                .iter()
                .map(|(name, meta)| crate::db_mod::ColDef {
                    name: name.clone(),
                    type_name: meta.type_name.clone(),
                    modifiers: meta.modifiers.clone(),
                })
                .collect();
            let sql = db.build_create_table(table, &coldefs);
            db.exec(&sql, &[]).map_err(Flow::err)?;
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
                Stmt::Tbl { name, columns } => self.register_tbl(name, columns),
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
        Ok(())
    }

    // tbl e'lonini schema registry'ga yozadi (ustun -> tip+modifikatorlar).
    fn register_tbl(&self, name: &str, columns: &[TblColumn]) {
        let mut cols = BTreeMap::new();
        for c in columns {
            cols.insert(
                c.name.clone(),
                ColMeta {
                    type_name: c.type_name.clone(),
                    modifiers: c.modifiers.clone(),
                },
            );
        }
        self.schema.write().insert(name.to_string(), cols);
    }

    // Blokni ketma-ket bajaradi; qiymati — oxirgi ifoda (Flux'da blok ifoda).
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
                env.write().define(name, v, false);
                Ok(Value::Nil)
            }
            Stmt::Assign { name, value } => {
                let v = self.eval(value, env)?;
                self.assign(name, v, env)?;
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
            // use — modul import (dispatch nom asosida, ro'yxatga olish shart emas).
            Stmt::Use { .. } => Ok(Value::Nil),
            // tbl — schema registry'ga yoziladi (sym/json konversiya + migration).
            Stmt::Tbl { name, columns } => {
                self.register_tbl(name, columns);
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

    fn exec_each(&self, vars: &[String], iter: &Expr, body: &[Stmt], env: &Env) -> ExecResult {
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
                            out.push_str(&format!("{}", v));
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
                        Value::Int(n) => Ok(Value::Int(-n)),
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
                            i += 1;
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
                // x |> f  ==  f x
                let l = self.eval(lhs, env)?;
                let f = self.eval(rhs, env)?;
                return self.apply(f, vec![l]);
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
            (BinOp::Add, Str(a), b) => Ok(Str(a + &format!("{}", b))),
            (BinOp::Add, a, Str(b)) => Ok(Str(format!("{}", a) + &b)),

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
                let argv = self.eval_args(args, env)?;
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
                    let argv = self.eval_args(args, env)?;
                    return self.arc_self().http_dispatch(name, argv);
                }
                // db — http kabi state'li (connection + tx konteksti); Interp'ga
                // muhtoj. db.tx argumenti lambda bo'lib keladi (Value::Fn).
                if modname == "db" {
                    let argv = self.eval_args(args, env)?;
                    return self.arc_self().db_dispatch(name, argv);
                }
                // ws — http kabi state'li (jonli ulanishlar, handler apply uchun
                // Interp kerak). ws.room.*/ws.data.* esa ikki-bosqichli Field
                // bo'lib keladi — quyiroqda (Field target ichida) ushlanadi.
                if modname == "ws" {
                    let argv = self.eval_args(args, env)?;
                    return self.arc_self().ws_dispatch(name, argv);
                }
                // reg — state'li (funksiya registri); `reg.add`/`reg.call` argument
                // sifatida funksiya/argumentlar oladi. `reg.names` argumentsiz —
                // Field shoxida (quyiroqda) ushlanadi.
                if modname == "reg" {
                    let argv = self.eval_args(args, env)?;
                    return self.reg_dispatch(name, argv);
                }
                // cron — state'li (rejalashtirilgan vazifalar). `cron.on` ifoda + handler
                // oladi, `cron.run` argumentsiz bloklaydi. Ifoda parser'da tirnoqsiz
                // 5-maydonli str sifatida keladi (quyida parser maxsus ushlaydi).
                if modname == "cron" {
                    let argv = self.eval_args(args, env)?;
                    return self.arc_self().cron_dispatch(name, argv);
                }
                if crate::builtins::is_module(modname) {
                    let argv = self.eval_args(args, env)?;
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
                let argv = self.eval_args(args, env)?;
                return self.apply(f, argv);
            }
            let argv = self.eval_args(args, env)?;
            // Yuqori tartibli list metodlari (lambda chaqiradi) — bu yerda,
            // chunki builtins Interp'ga kira olmaydi.
            if let Value::List(xs) = &recv {
                match name.as_str() {
                    "filter" | "map" | "reduce" => {
                        return self.list_hof(xs, name, argv);
                    }
                    _ => {}
                }
            }
            return crate::builtins::call_method(&recv, name, argv);
        }
        let f = self.eval(callee, env)?;
        let argv = self.eval_args(args, env)?;
        self.apply(f, argv)
    }

    // list.filter/map/reduce — funksiya argumentini har element uchun chaqiradi.
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
            (Value::Map(m), Value::Str(key)) => Ok(m.get(key).cloned().unwrap_or(Value::Nil)),
            (Value::Map(m), Value::Sym(key)) => Ok(m.get(key).cloned().unwrap_or(Value::Nil)),
            (Value::Nil, _) => Ok(Value::Nil),
            (t, k) => Err(Flow::err(format!(
                "{}[{}] indekslash qo'llab-quvvatlanmaydi",
                t.type_name(),
                k.type_name()
            ))),
        }
    }
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
    Ok(match op {
        BinOp::Add => Int(a + b),
        BinOp::Sub => Int(a - b),
        BinOp::Mul => Int(a * b),
        BinOp::Div => {
            if b == 0 {
                return Err(Flow::err("nolga bo'lish"));
            }
            Int(a / b)
        }
        BinOp::Mod => {
            if b == 0 {
                return Err(Flow::err("nolga bo'lish (mod)"));
            }
            Int(a % b)
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
