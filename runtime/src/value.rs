// Fluxon runtime qiymatlari.
//
// List va Map ulashilgan/o'zgaruvchan bo'lishi mumkin (spec: `m.set`, `l.push`
// yangi qiymat qaytaradi, lekin shared state map'lar `<-` bilan boshqariladi).
// Soddalik uchun list/map'ni Rc<RefCell<...>> bilan emas, oddiy klonlanadigan
// qiymat sifatida saqlaymiz — Fluxon semantikasi asosan "yangi qiymat qaytarish"
// (persistent) uslubida, mutatsiya esa binding qayta tayinlash orqali.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use crate::ast::Stmt;

#[derive(Clone)]
pub enum Value {
    Int(i64),
    Flt(f64),
    Str(String),
    Bool(bool),
    Nil,
    Sym(String),
    // Ikkilik ma'lumot (issue #132): fayl tarkibi, HTTP binary body, hash
    // natijalari. Arc — katta fayl klonlashda nusxalanmasin (HTTP javobida
    // body bir necha marta ko'chadi); Send+Sync invarianti ham saqlanadi.
    Bytes(Arc<Vec<u8>>),
    List(Vec<Value>),
    // Tartibni barqaror saqlash uchun BTreeMap (chiqishni deterministik qiladi).
    Map(BTreeMap<String, Value>),
    // Foydalanuvchi funksiyasi (closure): parametrlar, tana, qamrab olingan
    // muhit (lexical scope).
    Fn(Arc<FnValue>),
    // Rust'da yozilgan ichki funksiya (builtin).
    Native(Arc<NativeFn>),
    // Request-scoped context store: `req.ctx` shu yerda turadi (issue #68).
    // Map immutable + klonlanadi, shuning uchun middleware va handler bir xil
    // ctx'ni ko'rishi uchun SHARED mutable kerak — `Arc<Mutex>` aynan shuni
    // beradi (klonlanganda Arc ulashiladi, cell o'sha qoladi). Send+Sync
    // invarianti saqlanadi. Foydalanuvchiga oddiy map ko'rinadi (type_name="map",
    // o'qiganda snapshot Map qaytadi — interp::get_field).
    Ctx(Arc<Mutex<BTreeMap<String, Value>>>),
}

pub struct FnValue {
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    // Closure ota-havolasi. `apply` shundan child scope ochadi — top-level
    // fn'lar uchun bu `Parent::Root`, shuning uchun rekursiv chaqiruvda root
    // Arc klonlanmaydi/lock olinmaydi (atomik contention yo'q). Nested closure
    // runtime'da joriy scope'ni ushlaydi (`Parent::Scope`). Avval to'liq
    // `closure: Env` edi — har `apply` root Arc'ni klonlardi.
    pub parent: crate::interp::Parent,
    pub name: String,
}

pub struct NativeFn {
    pub name: String,
    pub func: Box<dyn Fn(Vec<Value>) -> Result<Value, crate::interp::Flow> + Send + Sync>,
}

// Map'ni `{k:v ...}` ko'rinishida chop etadi (Map va Ctx Display uchun umumiy).
fn write_map(f: &mut fmt::Formatter<'_>, m: &BTreeMap<String, Value>) -> fmt::Result {
    write!(f, "{{")?;
    for (i, (k, v)) in m.iter().enumerate() {
        if i > 0 {
            write!(f, " ")?;
        }
        write!(f, "{}:{}", k, v.repr())?;
    }
    write!(f, "}}")
}

// Ikki map'ni Fluxon `==` semantikasi bilan taqqoslaydi (Map va Ctx uchun umumiy).
fn maps_equal(a: &BTreeMap<String, Value>, b: &BTreeMap<String, Value>) -> bool {
    a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|w| v.equals(w)))
}

impl Value {
    // Fluxon truthiness: faqat nil va false yolg'on; qolgan hammasi rost.
    pub fn truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "int",
            Value::Flt(_) => "flt",
            Value::Str(_) => "str",
            Value::Bool(_) => "bool",
            Value::Nil => "nil",
            Value::Sym(_) => "sym",
            Value::Bytes(_) => "bytes",
            Value::List(_) => "list",
            Value::Map(_) => "map",
            // ctx foydalanuvchiga oddiy map ko'rinadi — ichki tipni oshkor qilmaymiz.
            Value::Ctx(_) => "map",
            Value::Fn(_) | Value::Native(_) => "fn",
        }
    }

    // Tenglik — Fluxon `==` semantikasi.
    pub fn equals(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Flt(a), Value::Flt(b)) => a == b,
            (Value::Int(a), Value::Flt(b)) | (Value::Flt(b), Value::Int(a)) => *a as f64 == *b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Sym(a), Value::Sym(b)) => a == b,
            (Value::Bytes(a), Value::Bytes(b)) => a == b,
            (Value::Nil, Value::Nil) => true,
            (Value::List(a), Value::List(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.equals(y))
            }
            (Value::Map(a), Value::Map(b)) => maps_equal(a, b),
            // ctx oddiy map kabi taqqoslanadi (snapshot orqali). MUHIM: ikki lock'ni
            // bir vaqtda USHLAMAYMIZ — `req == req` (yoki klon) da a va b bir xil
            // Arc<Mutex> bo'lishi mumkin; ikkinchi lock o'sha non-reentrant mutex'ni
            // qayta olib deadlock qilardi. Avval bir xil Arc'ni ptr_eq bilan
            // qisqa-tutamiz, aks holda har birini ALOHIDA snapshot qilib taqqoslaymiz.
            (Value::Ctx(a), Value::Ctx(b)) => {
                if Arc::ptr_eq(a, b) {
                    return true;
                }
                let sa = a.lock().unwrap().clone();
                let sb = b.lock().unwrap().clone();
                maps_equal(&sa, &sb)
            }
            (Value::Ctx(a), Value::Map(b)) | (Value::Map(b), Value::Ctx(a)) => {
                let sa = a.lock().unwrap().clone();
                maps_equal(&sa, b)
            }
            _ => false,
        }
    }
}

// Foydalanuvchiga ko'rinadigan format (log uchun).
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{}", n),
            Value::Flt(x) => {
                // butun float bo'lsa ham nuqta ko'rsatamiz (1.0), aks holda oddiy
                if x.fract() == 0.0 && x.is_finite() {
                    write!(f, "{:.1}", x)
                } else {
                    write!(f, "{}", x)
                }
            }
            Value::Str(s) => write!(f, "{}", s),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Nil => write!(f, "nil"),
            Value::Sym(s) => write!(f, ":{}", s),
            // Xom baytlarni matnga quyish xavfli (terminal/log buziladi) —
            // o'lchamli qisqa belgi chiqaramiz. Matn kerak bo'lsa: bytes.str b.
            Value::Bytes(b) => write!(f, "<bytes {}>", b.len()),
            Value::List(items) => {
                write!(f, "[")?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{}", v.repr())?;
                }
                write!(f, "]")
            }
            Value::Map(m) => write_map(f, m),
            // ctx oddiy map kabi chop etiladi (snapshot).
            Value::Ctx(c) => write_map(f, &c.lock().unwrap()),
            Value::Fn(fv) => write!(f, "<fn {}>", fv.name),
            Value::Native(nf) => write!(f, "<native {}>", nf.name),
        }
    }
}

impl Value {
    // Matnli ko'rinish: qiymat STRING'ga aylantirilganda ishlatiladi
    // (interpolatsiya, str.str, `+` birlashtirish, log). Symbol bu yerda `:`
    // prefiksisiz nomini beradi — `:` sintaksis belgisi, qiymatning matn
    // ko'rinishi emas (issue #57). Qolgan turlar Display bilan bir xil.
    pub fn to_text(&self) -> String {
        match self {
            Value::Sym(s) => s.clone(),
            other => format!("{}", other),
        }
    }

    // List/map ichida ko'rinish: stringlar tirnoq bilan, qolgani Display.
    // Bu yerda symbol `:` prefiksini SAQLAYDI — list/map ichida symbol
    // string'dan (yoki boshqa turdan) ajralib turishi kerak.
    pub fn repr(&self) -> String {
        match self {
            Value::Str(s) => format!("\"{}\"", s),
            other => format!("{}", other),
        }
    }
}
