// Flux yadro kutubxonasi (batteries'siz qism).
//
// Uch xil chaqiruv:
//   - global funksiyalar (log) — Env'ga o'rnatiladi (`install`)
//   - modul funksiyalari (str.up, math.floor, rand.int, json) — `call_module`
//   - qiymat metodlari (l.push, m.set, s.up emas...) — `call_method`
//
// list metodlari qiymat ustida (.push/.filter), str/math/rand modul orqali —
// bu spec'dagi farqni aniq aks ettiradi: `l.len` (a'zo) vs `str.len s` (modul).

use std::collections::BTreeMap;
use std::rc::Rc;

use crate::interp::{Env, Flow};
use crate::value::{NativeFn, Value};

type R = Result<Value, Flow>;

// --- global funksiyalarni o'rnatish ---
pub fn install(env: &Env) {
    let mut s = env.borrow_mut();
    let mut add = |name: &str, f: Box<dyn Fn(Vec<Value>) -> R>| {
        s.set_global(name, Value::Native(Rc::new(NativeFn { name: name.into(), func: f })));
    };
    add(
        "log",
        Box::new(|args: Vec<Value>| {
            let parts: Vec<String> = args.iter().map(|v| format!("{}", v)).collect();
            eprintln!("{}", parts.join(" "));
            Ok(Value::Nil)
        }),
    );
}

// --- modul nomimi? ---
pub fn is_module(name: &str) -> bool {
    matches!(name, "str" | "math" | "rand" | "json")
}

// --- modul funksiyasi chaqiruvi ---
pub fn call_module(module: &str, func: &str, args: Vec<Value>) -> R {
    match module {
        "str" => str_module(func, args),
        "math" => math_module(func, args),
        "rand" => rand_module(func, args),
        "json" => json_module(func, args),
        _ => Err(Flow::err(format!("noma'lum modul: {}", module))),
    }
}

// ---------------- str ----------------
fn str_module(func: &str, args: Vec<Value>) -> R {
    match func {
        "len" => {
            let s = arg_str(&args, 0, "str.len")?;
            Ok(Value::Int(s.chars().count() as i64))
        }
        "up" => Ok(Value::Str(arg_str(&args, 0, "str.up")?.to_uppercase())),
        "low" => Ok(Value::Str(arg_str(&args, 0, "str.low")?.to_lowercase())),
        "slice" => {
            let s = arg_str(&args, 0, "str.slice")?;
            let a = arg_int(&args, 1, "str.slice")? as usize;
            let b = arg_int(&args, 2, "str.slice")? as usize;
            let chars: Vec<char> = s.chars().collect();
            let a = a.min(chars.len());
            let b = b.min(chars.len());
            if a >= b {
                return Ok(Value::Str(String::new()));
            }
            Ok(Value::Str(chars[a..b].iter().collect()))
        }
        "split" => {
            let s = arg_str(&args, 0, "str.split")?;
            let sep = arg_str(&args, 1, "str.split")?;
            let parts: Vec<Value> = if sep.is_empty() {
                s.chars().map(|c| Value::Str(c.to_string())).collect()
            } else {
                s.split(&sep).map(|p| Value::Str(p.to_string())).collect()
            };
            Ok(Value::List(parts))
        }
        "has" => {
            let s = arg_str(&args, 0, "str.has")?;
            let sub = arg_str(&args, 1, "str.has")?;
            Ok(Value::Bool(s.contains(&sub)))
        }
        "int" => {
            let s = arg_str(&args, 0, "str.int")?;
            match s.trim().parse::<i64>() {
                Ok(n) => Ok(Value::Int(n)),
                Err(_) => Ok(Value::Nil),
            }
        }
        "str" => Ok(Value::Str(format!("{}", arg(&args, 0, "str.str")?))),
        _ => Err(Flow::err(format!("str modulida '{}' funksiyasi yo'q", func))),
    }
}

// ---------------- math ----------------
fn math_module(func: &str, args: Vec<Value>) -> R {
    let x = arg_num(&args, 0, &format!("math.{}", func))?;
    match func {
        "floor" => Ok(Value::Int(x.floor() as i64)),
        "ceil" => Ok(Value::Int(x.ceil() as i64)),
        "abs" => {
            // int kirsa int, flt kirsa flt qaytaramiz
            match &args[0] {
                Value::Int(n) => Ok(Value::Int(n.abs())),
                _ => Ok(Value::Flt(x.abs())),
            }
        }
        "round" => Ok(Value::Int(x.round() as i64)),
        _ => Err(Flow::err(format!("math modulida '{}' funksiyasi yo'q", func))),
    }
}

// ---------------- rand (dependency'siz LCG) ----------------
fn rand_module(func: &str, args: Vec<Value>) -> R {
    match func {
        "int" => {
            let a = arg_int(&args, 0, "rand.int")?;
            let b = arg_int(&args, 1, "rand.int")?;
            if b < a {
                return Err(Flow::err("rand.int: yuqori chegara pastdan kichik"));
            }
            let span = (b - a + 1) as u64;
            let r = next_rand() % span;
            Ok(Value::Int(a + r as i64))
        }
        "str" => {
            let n = arg_int(&args, 0, "rand.str")? as usize;
            const ALPHA: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
            let mut out = String::with_capacity(n);
            for _ in 0..n {
                let idx = (next_rand() % ALPHA.len() as u64) as usize;
                out.push(ALPHA[idx] as char);
            }
            Ok(Value::Str(out))
        }
        _ => Err(Flow::err(format!("rand modulida '{}' funksiyasi yo'q", func))),
    }
}

// Oddiy xorshift RNG. Seed system time'dan bir marta olinadi.
fn next_rand() -> u64 {
    use std::cell::Cell;
    thread_local! {
        static STATE: Cell<u64> = Cell::new(seed());
    }
    STATE.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        x
    })
}

fn seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E3779B97F4A7C15);
    nanos | 1 // nol bo'lmasligi uchun
}

// ---------------- json ----------------
fn json_module(func: &str, args: Vec<Value>) -> R {
    match func {
        "enc" => Ok(Value::Str(json_encode(arg(&args, 0, "json.enc")?))),
        "dec" => {
            let s = arg_str(&args, 0, "json.dec")?;
            json_decode(&s)
        }
        _ => Err(Flow::err(format!("json modulida '{}' funksiyasi yo'q", func))),
    }
}

fn json_encode(v: &Value) -> String {
    match v {
        Value::Int(n) => n.to_string(),
        Value::Flt(x) => x.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Nil => "null".into(),
        Value::Str(s) | Value::Sym(s) => json_str(s),
        Value::List(items) => {
            let parts: Vec<String> = items.iter().map(json_encode).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Map(m) => {
            let parts: Vec<String> = m
                .iter()
                .map(|(k, val)| format!("{}:{}", json_str(k), json_encode(val)))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Fn(_) | Value::Native(_) => "null".into(),
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

// Minimal JSON dekoder (yadro versiyasi uchun yetarli).
fn json_decode(s: &str) -> R {
    let mut p = JsonParser { b: s.as_bytes(), i: 0 };
    p.skip_ws();
    let v = p.value()?;
    p.skip_ws();
    Ok(v)
}

struct JsonParser<'a> {
    b: &'a [u8],
    i: usize,
}
impl<'a> JsonParser<'a> {
    fn skip_ws(&mut self) {
        while self.i < self.b.len() && (self.b[self.i] as char).is_whitespace() {
            self.i += 1;
        }
    }
    fn value(&mut self) -> R {
        self.skip_ws();
        if self.i >= self.b.len() {
            return Err(Flow::err("json: kutilmagan oxir"));
        }
        match self.b[self.i] {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => Ok(Value::Str(self.string()?)),
            b't' | b'f' => self.boolean(),
            b'n' => {
                self.i += 4;
                Ok(Value::Nil)
            }
            _ => self.number(),
        }
    }
    fn object(&mut self) -> R {
        self.i += 1; // {
        let mut m = BTreeMap::new();
        self.skip_ws();
        if self.i < self.b.len() && self.b[self.i] == b'}' {
            self.i += 1;
            return Ok(Value::Map(m));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            if self.i >= self.b.len() || self.b[self.i] != b':' {
                return Err(Flow::err("json: ':' kutilgan"));
            }
            self.i += 1;
            let val = self.value()?;
            m.insert(key, val);
            self.skip_ws();
            match self.b.get(self.i) {
                Some(b',') => {
                    self.i += 1;
                }
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                _ => return Err(Flow::err("json: ',' yoki '}' kutilgan")),
            }
        }
        Ok(Value::Map(m))
    }
    fn array(&mut self) -> R {
        self.i += 1; // [
        let mut out = Vec::new();
        self.skip_ws();
        if self.i < self.b.len() && self.b[self.i] == b']' {
            self.i += 1;
            return Ok(Value::List(out));
        }
        loop {
            let v = self.value()?;
            out.push(v);
            self.skip_ws();
            match self.b.get(self.i) {
                Some(b',') => {
                    self.i += 1;
                }
                Some(b']') => {
                    self.i += 1;
                    break;
                }
                _ => return Err(Flow::err("json: ',' yoki ']' kutilgan")),
            }
        }
        Ok(Value::List(out))
    }
    fn string(&mut self) -> Result<String, Flow> {
        if self.b[self.i] != b'"' {
            return Err(Flow::err("json: satr kutilgan"));
        }
        self.i += 1;
        let mut out = String::new();
        while self.i < self.b.len() {
            let c = self.b[self.i];
            self.i += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let e = self.b[self.i];
                    self.i += 1;
                    match e {
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        _ => out.push(e as char),
                    }
                }
                _ => out.push(c as char),
            }
        }
        Err(Flow::err("json: yopilmagan satr"))
    }
    fn boolean(&mut self) -> R {
        if self.b[self.i..].starts_with(b"true") {
            self.i += 4;
            Ok(Value::Bool(true))
        } else if self.b[self.i..].starts_with(b"false") {
            self.i += 5;
            Ok(Value::Bool(false))
        } else {
            Err(Flow::err("json: noto'g'ri bool"))
        }
    }
    fn number(&mut self) -> R {
        let start = self.i;
        let mut is_float = false;
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if c.is_ascii_digit() || c == b'-' || c == b'+' {
                self.i += 1;
            } else if c == b'.' || c == b'e' || c == b'E' {
                is_float = true;
                self.i += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.b[start..self.i]).unwrap_or("");
        if is_float {
            text.parse::<f64>()
                .map(Value::Flt)
                .map_err(|_| Flow::err("json: noto'g'ri son"))
        } else {
            text.parse::<i64>()
                .map(Value::Int)
                .map_err(|_| Flow::err("json: noto'g'ri son"))
        }
    }
}

// ---------------- qiymat metodlari (list/map) ----------------
pub fn call_method(recv: &Value, method: &str, args: Vec<Value>) -> R {
    match recv {
        Value::List(xs) => list_method(xs, method, args),
        Value::Map(m) => map_method(m, method, args),
        Value::Str(_) => Err(Flow::err(format!(
            "str metodlari modul orqali chaqiriladi: str.{} s (qiymat metodi emas)",
            method
        ))),
        other => Err(Flow::err(format!(
            "{} tipida '.{}' metodi yo'q",
            other.type_name(),
            method
        ))),
    }
}

fn list_method(xs: &[Value], method: &str, args: Vec<Value>) -> R {
    match method {
        "len" => Ok(Value::Int(xs.len() as i64)),
        "push" => {
            let mut new = xs.to_vec();
            new.push(arg(&args, 0, "list.push")?.clone());
            Ok(Value::List(new))
        }
        "has" => {
            let needle = arg(&args, 0, "list.has")?;
            Ok(Value::Bool(xs.iter().any(|v| v.equals(needle))))
        }
        "join" => {
            let sep = arg_str(&args, 0, "list.join")?;
            let parts: Vec<String> = xs.iter().map(|v| format!("{}", v)).collect();
            Ok(Value::Str(parts.join(&sep)))
        }
        "slice" => {
            let a = arg_int(&args, 0, "list.slice")? as usize;
            let b = arg_int(&args, 1, "list.slice")? as usize;
            let a = a.min(xs.len());
            let b = b.min(xs.len());
            if a >= b {
                return Ok(Value::List(vec![]));
            }
            Ok(Value::List(xs[a..b].to_vec()))
        }
        // filter/map/reduce — funksiya argument oladi; interp uni shu yerda
        // chaqira olmaydi (apply Interp'da). Shuning uchun bu metodlar maxsus
        // ishlov talab qiladi — pastdagi izohga qarang.
        "filter" | "map" | "reduce" => Err(Flow::err(format!(
            "ichki: list.{} alohida yo'l bilan ishlov beriladi",
            method
        ))),
        _ => Err(Flow::err(format!("list metodi '{}' mavjud emas", method))),
    }
}

fn map_method(m: &BTreeMap<String, Value>, method: &str, args: Vec<Value>) -> R {
    match method {
        "len" => Ok(Value::Int(m.len() as i64)),
        "has" => {
            let k = key_of(arg(&args, 0, "map.has")?);
            Ok(Value::Bool(m.contains_key(&k)))
        }
        "keys" => Ok(Value::List(m.keys().map(|k| Value::Str(k.clone())).collect())),
        "vals" => Ok(Value::List(m.values().cloned().collect())),
        "set" => {
            let k = key_of(arg(&args, 0, "map.set")?);
            let v = arg(&args, 1, "map.set")?.clone();
            let mut new = m.clone();
            new.insert(k, v);
            Ok(Value::Map(new))
        }
        "del" => {
            let k = key_of(arg(&args, 0, "map.del")?);
            let mut new = m.clone();
            new.remove(&k);
            Ok(Value::Map(new))
        }
        _ => Err(Flow::err(format!("map metodi '{}' mavjud emas", method))),
    }
}

fn key_of(v: &Value) -> String {
    match v {
        Value::Str(s) | Value::Sym(s) => s.clone(),
        other => format!("{}", other),
    }
}

// ---------------- argument yordamchilari ----------------
fn arg<'a>(args: &'a [Value], i: usize, who: &str) -> Result<&'a Value, Flow> {
    args.get(i)
        .ok_or_else(|| Flow::err(format!("{}: {}-argument yetishmadi", who, i + 1)))
}
fn arg_str(args: &[Value], i: usize, who: &str) -> Result<String, Flow> {
    match arg(args, i, who)? {
        Value::Str(s) => Ok(s.clone()),
        Value::Sym(s) => Ok(s.clone()),
        other => Err(Flow::err(format!(
            "{}: {}-argument str bo'lishi kerak, {} berildi",
            who,
            i + 1,
            other.type_name()
        ))),
    }
}
fn arg_int(args: &[Value], i: usize, who: &str) -> Result<i64, Flow> {
    match arg(args, i, who)? {
        Value::Int(n) => Ok(*n),
        other => Err(Flow::err(format!(
            "{}: {}-argument int bo'lishi kerak, {} berildi",
            who,
            i + 1,
            other.type_name()
        ))),
    }
}
fn arg_num(args: &[Value], i: usize, who: &str) -> Result<f64, Flow> {
    match arg(args, i, who)? {
        Value::Int(n) => Ok(*n as f64),
        Value::Flt(x) => Ok(*x),
        other => Err(Flow::err(format!(
            "{}: {}-argument son bo'lishi kerak, {} berildi",
            who,
            i + 1,
            other.type_name()
        ))),
    }
}
