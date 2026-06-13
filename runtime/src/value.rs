// Fluxon runtime values.
//
// List and Map may be shared/mutable (spec: `m.set`, `l.push` return a new
// value, but shared-state maps are managed via `<-`). For simplicity we store
// list/map as plain cloneable values rather than Rc<RefCell<...>> — Fluxon
// semantics are mostly in the "return a new value" (persistent) style, with
// mutation done by re-binding.

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
    // Binary data (issue #132): file contents, HTTP binary body, hash results.
    // Arc keeps large files from being copied on clone (a body is moved several
    // times in an HTTP response); the Send+Sync invariant is preserved too.
    Bytes(Arc<Vec<u8>>),
    List(Vec<Value>),
    // BTreeMap to keep a stable ordering (makes output deterministic).
    Map(BTreeMap<String, Value>),
    // User function (closure): parameters, body, captured environment
    // (lexical scope).
    Fn(Arc<FnValue>),
    // Built-in function written in Rust (builtin).
    Native(Arc<NativeFn>),
    // Request-scoped context store: `req.ctx` lives here (issue #68). A Map is
    // immutable + cloned, so for middleware and handler to see the same ctx we
    // need SHARED mutable state — `Arc<Mutex>` gives exactly that (on clone the
    // Arc is shared, the cell stays the same). The Send+Sync invariant is
    // preserved. To the user it looks like a plain map (type_name="map", and
    // reading it returns a snapshot Map — interp::get_field).
    Ctx(Arc<Mutex<BTreeMap<String, Value>>>),
}

pub struct FnValue {
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    // Closure parent link. `apply` opens a child scope from it — for top-level
    // fn's this is `Parent::Root`, so a recursive call does not clone the root
    // Arc / take a lock (no atomic contention). A nested closure captures the
    // current runtime scope (`Parent::Scope`). It used to be a full
    // `closure: Env` — every `apply` cloned the root Arc.
    pub parent: crate::interp::Parent,
    pub name: String,
}

pub struct NativeFn {
    pub name: String,
    pub func: Box<dyn Fn(Vec<Value>) -> Result<Value, crate::interp::Flow> + Send + Sync>,
}

// Prints a map as `{k:v ...}` (shared by Map and Ctx Display).
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

// Compares two maps with Fluxon `==` semantics (shared by Map and Ctx).
fn maps_equal(a: &BTreeMap<String, Value>, b: &BTreeMap<String, Value>) -> bool {
    a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|w| v.equals(w)))
}

impl Value {
    // Fluxon truthiness: only nil and false are falsy; everything else is true.
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
            // ctx looks like a plain map to the user — we do not expose the inner type.
            Value::Ctx(_) => "map",
            Value::Fn(_) | Value::Native(_) => "fn",
        }
    }

    // Equality — Fluxon `==` semantics.
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
            // ctx is compared like a plain map (via a snapshot). IMPORTANT: we do
            // NOT hold two locks at once — in `req == req` (or a clone) a and b may
            // be the same Arc<Mutex>; taking the second lock on that non-reentrant
            // mutex would deadlock. We first short-circuit identical Arc's via
            // ptr_eq, otherwise snapshot each one SEPARATELY and compare.
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

// User-visible format (for logging).
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{}", n),
            Value::Flt(x) => {
                // show a decimal point even for whole floats (1.0), otherwise plain
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
            // Dumping raw bytes as text is dangerous (corrupts terminal/log) —
            // we emit a short marker with the size. For text use: bytes.str b.
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
            // ctx is printed like a plain map (snapshot).
            Value::Ctx(c) => write_map(f, &c.lock().unwrap()),
            Value::Fn(fv) => write!(f, "<fn {}>", fv.name),
            Value::Native(nf) => write!(f, "<native {}>", nf.name),
        }
    }
}

impl Value {
    // Text form: used when a value is converted to a STRING (interpolation,
    // str.str, `+` concatenation, log). A symbol here gives its name without the
    // `:` prefix — `:` is a syntax marker, not part of the value's text form
    // (issue #57). Other types match Display.
    pub fn to_text(&self) -> String {
        match self {
            Value::Sym(s) => s.clone(),
            other => format!("{}", other),
        }
    }

    // Form inside a list/map: strings quoted, the rest as Display. Here a symbol
    // KEEPS its `:` prefix — inside a list/map a symbol must be distinguishable
    // from a string (or other type).
    pub fn repr(&self) -> String {
        match self {
            Value::Str(s) => format!("\"{}\"", s),
            other => format!("{}", other),
        }
    }
}
