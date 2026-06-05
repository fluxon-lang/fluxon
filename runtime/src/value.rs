// Flux runtime qiymatlari.
//
// List va Map ulashilgan/o'zgaruvchan bo'lishi mumkin (spec: `m.set`, `l.push`
// yangi qiymat qaytaradi, lekin shared state map'lar `<-` bilan boshqariladi).
// Soddalik uchun list/map'ni Rc<RefCell<...>> bilan emas, oddiy klonlanadigan
// qiymat sifatida saqlaymiz — Flux semantikasi asosan "yangi qiymat qaytarish"
// (persistent) uslubida, mutatsiya esa binding qayta tayinlash orqali.

use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

use crate::ast::Stmt;

#[derive(Clone)]
pub enum Value {
    Int(i64),
    Flt(f64),
    Str(String),
    Bool(bool),
    Nil,
    Sym(String),
    List(Vec<Value>),
    // Tartibni barqaror saqlash uchun BTreeMap (chiqishni deterministik qiladi).
    Map(BTreeMap<String, Value>),
    // Foydalanuvchi funksiyasi (closure): parametrlar, tana, qamrab olingan
    // muhit (lexical scope).
    Fn(Rc<FnValue>),
    // Rust'da yozilgan ichki funksiya (builtin).
    Native(Rc<NativeFn>),
}

pub struct FnValue {
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    pub closure: crate::interp::Env,
    pub name: String,
}

pub struct NativeFn {
    pub name: String,
    pub func: Box<dyn Fn(Vec<Value>) -> Result<Value, crate::interp::Flow>>,
}

impl Value {
    // Flux truthiness: faqat nil va false yolg'on; qolgan hammasi rost.
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
            Value::List(_) => "list",
            Value::Map(_) => "map",
            Value::Fn(_) | Value::Native(_) => "fn",
        }
    }

    // Tenglik — Flux `==` semantikasi.
    pub fn equals(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Flt(a), Value::Flt(b)) => a == b,
            (Value::Int(a), Value::Flt(b)) | (Value::Flt(b), Value::Int(a)) => *a as f64 == *b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Sym(a), Value::Sym(b)) => a == b,
            (Value::Nil, Value::Nil) => true,
            (Value::List(a), Value::List(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.equals(y))
            }
            (Value::Map(a), Value::Map(b)) => {
                a.len() == b.len()
                    && a.iter().all(|(k, v)| b.get(k).map_or(false, |w| v.equals(w)))
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
            Value::Map(m) => {
                write!(f, "{{")?;
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{}:{}", k, v.repr())?;
                }
                write!(f, "}}")
            }
            Value::Fn(fv) => write!(f, "<fn {}>", fv.name),
            Value::Native(nf) => write!(f, "<native {}>", nf.name),
        }
    }
}

impl Value {
    // List/map ichida ko'rinish: stringlar tirnoq bilan, qolgani Display.
    pub fn repr(&self) -> String {
        match self {
            Value::Str(s) => format!("\"{}\"", s),
            other => format!("{}", other),
        }
    }
}
