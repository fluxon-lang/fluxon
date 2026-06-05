// Flux interpreter — AST'ni to'g'ridan-to'g'ri bajaruvchi (tree-walking).
//
// Boshqaruv oqimi (ret/skip/stop/fail) Rust `Result`'ining `Err` tarmog'i
// orqali tarqatiladi: oddiy qiymatlar `Ok`, oqim-uzilishlari esa `Flow`.
// Bu `?` operatori bilan tabiiy yuqoriga ko'tariladi.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::*;
use crate::value::{FnValue, Value};

// Lexical scope: ota-muhitga havola bilan zanjir. Rc<RefCell<>> — closure'lar
// va mutatsiya uchun.
pub type Env = Rc<RefCell<Scope>>;

pub struct Scope {
    vars: HashMap<String, Value>,
    // mutable (`<-`) sifatida e'lon qilingan nomlar — qayta tayinlashga ruxsat.
    mutable: HashMap<String, bool>,
    parent: Option<Env>,
}

impl Scope {
    pub fn root() -> Env {
        Rc::new(RefCell::new(Scope {
            vars: HashMap::new(),
            mutable: HashMap::new(),
            parent: None,
        }))
    }
    fn child(parent: &Env) -> Env {
        Rc::new(RefCell::new(Scope {
            vars: HashMap::new(),
            mutable: HashMap::new(),
            parent: Some(parent.clone()),
        }))
    }
    // Builtins o'rnatish uchun: global nomga immutable qiymat qo'yadi.
    pub fn set_global(&mut self, name: &str, v: Value) {
        self.vars.insert(name.to_string(), v);
        self.mutable.insert(name.to_string(), false);
    }
}

// Oqim-uzilish signallari va xatolar. Hammasi `Err` tomonida sayohat qiladi.
pub enum Flow {
    Return(Value),
    Skip,
    Stop,
    // fail [status] message — biznes yoki ichki xato.
    Fail { status: Option<i64>, message: String },
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
}

impl Interp {
    pub fn new() -> Self {
        let global = Scope::root();
        crate::builtins::install(&global);
        Interp { global }
    }

    pub fn run(&mut self, prog: &Program) -> Result<(), String> {
        // Birinchi o'tish: top-level fn/tbl e'lonlarini oldindan ro'yxatga olamiz
        // (hoisting), shunda tartibdan qat'i nazar bir-birini chaqira oladi.
        for stmt in prog {
            if let Stmt::FnDecl { name, params, body, .. } = stmt {
                let f = Value::Fn(Rc::new(FnValue {
                    params: params.clone(),
                    body: body.clone(),
                    closure: self.global.clone(),
                    name: name.clone(),
                }));
                self.global.borrow_mut().vars.insert(name.clone(), f);
            }
        }
        for stmt in prog {
            // fn'lar allaqachon ro'yxatda — qayta bajarmaymiz.
            if matches!(stmt, Stmt::FnDecl { .. }) {
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
                    return Err("skip/stop loop tashqarisida ishlatildi".into())
                }
            }
        }
        Ok(())
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
                let mut s = env.borrow_mut();
                s.vars.insert(name.clone(), v);
                s.mutable.insert(name.clone(), false);
                Ok(Value::Nil)
            }
            Stmt::Assign { name, value } => {
                let v = self.eval(value, env)?;
                self.assign(name, v, env)?;
                Ok(Value::Nil)
            }
            Stmt::ExpBind { name, value } => {
                let v = self.eval(value, env)?;
                env.borrow_mut().vars.insert(name.clone(), v);
                Ok(Value::Nil)
            }
            Stmt::FnDecl { name, params, body, .. } => {
                let f = Value::Fn(Rc::new(FnValue {
                    params: params.clone(),
                    body: body.clone(),
                    closure: env.clone(),
                    name: name.clone(),
                }));
                env.borrow_mut().vars.insert(name.clone(), f);
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
                            )))
                        }
                    },
                    None => None,
                };
                let msg = self.eval(message, env)?;
                Err(Flow::Fail { status: st, message: format!("{}", msg) })
            }
            Stmt::Each { vars, iter, body } => self.exec_each(vars, iter, body, env),
            Stmt::Expr(e) => self.eval(e, env),
            // Yadro versiyada use/tbl e'tiborsiz (batteries kelganda ishlaydi).
            Stmt::Use { .. } => Ok(Value::Nil),
            Stmt::Tbl { .. } => Ok(Value::Nil),
        }
    }

    // `<-` qayta tayinlash: o'zgaruvchini scope zanjirida topib yangilaydi.
    // Topilmasa — joriy scope'da mutable sifatida yaratadi.
    fn assign(&self, name: &str, v: Value, env: &Env) -> Result<(), Flow> {
        let mut cur = env.clone();
        loop {
            {
                let mut s = cur.borrow_mut();
                if s.vars.contains_key(name) {
                    if s.mutable.get(name) == Some(&false) {
                        return Err(Flow::err(format!(
                            "'{}' o'zgarmas (=) e'lon qilingan, '<-' bilan o'zgartirib bo'lmaydi",
                            name
                        )));
                    }
                    s.vars.insert(name.to_string(), v);
                    s.mutable.insert(name.to_string(), true);
                    return Ok(());
                }
            }
            let parent = cur.borrow().parent.clone();
            match parent {
                Some(p) => cur = p,
                None => break,
            }
        }
        // yangi mutable o'zgaruvchi
        let mut s = env.borrow_mut();
        s.vars.insert(name.to_string(), v);
        s.mutable.insert(name.to_string(), true);
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
                )))
            }
        };
        for (key, val) in items {
            let loop_env = Scope::child(env);
            {
                let mut s = loop_env.borrow_mut();
                if vars.len() == 2 {
                    // each k, v in map
                    let k = key.unwrap_or(Value::Nil);
                    s.vars.insert(vars[0].clone(), k);
                    s.vars.insert(vars[1].clone(), val);
                } else {
                    // each x in list  — map ustida bo'lsa, qiymat
                    s.vars.insert(vars[0].clone(), val);
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
                let t = self.eval(target, env)?;
                self.get_field(&t, name, env)
            }
            Expr::Index { target, key } => {
                let t = self.eval(target, env)?;
                let k = self.eval(key, env)?;
                self.get_index(&t, &k)
            }
            Expr::Lambda { params, body } => Ok(Value::Fn(Rc::new(FnValue {
                params: params.clone(),
                body: body.clone(),
                closure: env.clone(),
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
                            )))
                        }
                    },
                    None => None,
                };
                let msg = self.eval(message, env)?;
                Err(Flow::Fail { status: st, message: format!("{}", msg) })
            }
        }
    }

    fn lookup(&self, name: &str, env: &Env) -> EvalResult {
        let mut cur = env.clone();
        loop {
            if let Some(v) = cur.borrow().vars.get(name) {
                return Ok(v.clone());
            }
            let parent = cur.borrow().parent.clone();
            match parent {
                Some(p) => cur = p,
                None => return Err(Flow::err(format!("noma'lum nom: {}", name))),
            }
        }
    }

    fn eval_if(&self, ifx: &IfExpr, env: &Env) -> EvalResult {
        for (cond, block) in &ifx.arms {
            if self.eval(cond, env)?.truthy() {
                let inner = Scope::child(env);
                return self.exec_block(block, &inner);
            }
        }
        if let Some(eb) = &ifx.else_block {
            let inner = Scope::child(env);
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
                let inner = Scope::child(env);
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
            // module.func (str.up, math.floor, ...) — `str` o'zgaruvchi emas,
            // shuning uchun target'ni baholashdan OLDIN modulni tekshiramiz.
            if let Expr::Ident(modname) = target.as_ref() {
                if crate::builtins::is_module(modname) {
                    let argv = self.eval_args(args, env)?;
                    return crate::builtins::call_module(modname, name, argv);
                }
            }
            let recv = self.eval(target, env)?;
            // Avval haqiqiy map maydoni funksiya bo'lsa (masalan map ichidagi
            // lambda) — uni chaqiramiz; aks holda builtin metod.
            if let Value::Map(m) = &recv {
                if let Some(v @ (Value::Fn(_) | Value::Native(_))) = m.get(name) {
                    let f = v.clone();
                    let argv = self.eval_args(args, env)?;
                    return self.apply(f, argv);
                }
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
                let f = args.into_iter().next().ok_or_else(|| {
                    Flow::err("list.filter: funksiya argumenti kerak")
                })?;
                let mut out = Vec::new();
                for x in xs {
                    if self.apply(f.clone(), vec![x.clone()])?.truthy() {
                        out.push(x.clone());
                    }
                }
                Ok(Value::List(out))
            }
            "map" => {
                let f = args.into_iter().next().ok_or_else(|| {
                    Flow::err("list.map: funksiya argumenti kerak")
                })?;
                let mut out = Vec::with_capacity(xs.len());
                for x in xs {
                    out.push(self.apply(f.clone(), vec![x.clone()])?);
                }
                Ok(Value::List(out))
            }
            "reduce" => {
                let mut it = args.into_iter();
                let mut acc = it.next().ok_or_else(|| {
                    Flow::err("list.reduce: boshlang'ich qiymat kerak")
                })?;
                let f = it.next().ok_or_else(|| {
                    Flow::err("list.reduce: funksiya argumenti kerak")
                })?;
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
                let call_env = Scope::child(&fv.closure);
                {
                    let mut s = call_env.borrow_mut();
                    for (p, a) in fv.params.iter().zip(args) {
                        s.vars.insert(p.clone(), a);
                    }
                }
                match self.exec_block(&fv.body, &call_env) {
                    Ok(v) => Ok(v),                       // oxirgi ifoda — qaytadi
                    Err(Flow::Return(v)) => Ok(v),        // erta ret
                    Err(other) => Err(other),             // fail/err/skip/stop
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
            Value::List(_) | Value::Str(_) => {
                crate::builtins::call_method(t, name, vec![])
            }
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
            (Value::Map(m), Value::Str(key)) => {
                Ok(m.get(key).cloned().unwrap_or(Value::Nil))
            }
            (Value::Map(m), Value::Sym(key)) => {
                Ok(m.get(key).cloned().unwrap_or(Value::Nil))
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
