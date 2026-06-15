// Expression evaluation: the `eval` dispatch, name lookup (with the lock-free
// frozen-global fast path), try/catch, and if/match selection.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::ast::*;
use crate::value::{FnValue, Value};

use super::Interp;
use super::scope::{Env, EvalResult, Flow, Parent, Scope};

impl Interp {
    // ---------------- evaluating expressions ----------------
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
                // `log` as a value (callback `xs.each log`, `f log`) — an
                // info-level shim for compatibility with the old global `log`
                // (issue #139). A direct `log "..."` call is caught earlier in
                // apply_callee (it does not reach this path). If `log` is declared
                // as a variable, lookup returns Ok — it takes precedence.
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
                        // i64::MIN cannot be negated — the same error as int_arith.
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
                            // If end = i64::MAX, i += 1 would overflow — we stop
                            // after pushing the last element.
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
            // inf is only meaningful in `each i in inf` — it cannot be used as a value.
            Expr::Inf => Err(Flow::err(
                "inf is only used in `each i in inf` (not a value)",
            )),
            Expr::Field { target, name } => {
                // `env.PORT` — an environment variable. `env` is a built-in ident;
                // if NOT declared as a variable, we read from std::env. If the user
                // creates a variable named `env`, it takes precedence.
                if let Expr::Ident(id) = target.as_ref() {
                    if id == "env" && self.lookup(id, env).is_err() {
                        // OS env > .env file (read lazily, only from here).
                        return Ok(self.env_lookup(name));
                    }
                    // An argument-less module function: `time.now` arrives as a
                    // Field, not a Call. If the module name is not declared as a
                    // variable, we call it as an argument-less module function.
                    // (str/math/rand require arguments; time.now is the only
                    // argument-less one, but we handle it generically.)
                    if crate::builtins::is_module(id) && self.lookup(id, env).is_err() {
                        return crate::builtins::call_module(id, name, vec![]);
                    }
                    // `log.info` with no message -> arrives as a Field. If `log` is
                    // not a variable we route it to the empty-message level
                    // (issue #139). An unknown level — an explicit error.
                    if id == "log" && self.lookup(id, env).is_err() {
                        return match name.as_str() {
                            "debug" | "info" | "warn" | "err" => self.log_dispatch(name, vec![]),
                            _ => Err(Flow::err(format!(
                                "log.{} does not exist (debug/info/warn/err)",
                                name
                            ))),
                        };
                    }
                    // `reg.names` with no args -> arrives as a Field, not a Call
                    // (like time.now). If `reg` is not declared as a variable, we
                    // call it as an argument-less reg function.
                    if id == "reg" && self.lookup(id, env).is_err() {
                        return self.reg_dispatch(name, vec![]);
                    }
                    // `crypto.uuid` with no args -> arrives as a Field, not a Call
                    // (like time.now). If `crypto` is not declared, we call it as a
                    // battery function.
                    if id == "crypto" && self.lookup(id, env).is_err() {
                        return crate::crypto_mod::crypto_module(name, vec![]);
                    }
                    // `cron.run` with no args -> arrives as a Field, not a Call. If
                    // cron is not declared as a variable, we call it as the
                    // argument-less cron function (run). (Otherwise `cron` would be
                    // looked up as an ident variable and give "unknown name".)
                    if id == "cron" && self.lookup(id, env).is_err() {
                        return self.arc_self().cron_dispatch(name, vec![]);
                    }
                    // queue is also a stateful module — an argument-less call (in
                    // the future) is caught here; otherwise `queue` would be looked
                    // up as an ident variable and give "unknown name".
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
                // expr! — if inner returns fail/err, we propagate it upward; on
                // success we return the value. In the core Fail/Error are raised
                // as Err anyway, so this is a pass-through.
                self.eval(inner, env)
            }
            // Reached via `eval` — an expression position (assignment RHS,
            // operand, argument), so the value IS used: `tail_used = true`.
            Expr::TryCatch {
                body,
                catch_var,
                catch_body,
            } => self.eval_try(body, catch_var.as_deref(), catch_body, env, true),
            Expr::If(ifx) => self.eval_if(ifx, env, true),
            Expr::Match(mx) => self.eval_match(mx, env, true),
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

    pub(crate) fn lookup(&self, name: &str, env: &Env) -> EvalResult {
        // We take the frozen global snapshot once, lock-free (an OnceLock read is
        // an atomic load — not a lock).
        let frozen = self.globals_frozen.get();
        let mut cur = env.clone();
        loop {
            // We view each level's scope under a SINGLE read lock: we both search
            // for the variable and get the next parent. (Previously there were two
            // separate `cur.read()` calls — each a parking_lot RwLock atomic
            // operation; parallel requests collided on the global root.)
            let parent = {
                let s = cur.read();
                // If the root scope ITSELF is frozen — lock-free snapshot.
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
                    // The parent is the root (marker). When frozen we read from
                    // the frozen snapshot WITHOUT TOUCHING the root Arc — parallel
                    // requests do not collide here (no atomic contention).
                    // Otherwise (top-level, not frozen) we move to the
                    // `Interp.global` Arc — no clone needed, it comes via `&self`.
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

    // True if `e` is a bare `rep ...` call whose callee resolves to the `rep`
    // BUILTIN. A bare builtin `rep` statement short-circuits the enclosing
    // function like `ret` (issue #173) — see the `Stmt::Expr` arm. A user
    // binding that shadows `rep` (a local/param/module export/user fn) must keep
    // normal call semantics, so we resolve the name first: users cannot create
    // `Value::Native`, so a `Native` named "rep" is unambiguously the builtin.
    pub(crate) fn is_builtin_rep_call(&self, e: &Expr, env: &Env) -> bool {
        let Expr::Call { callee, .. } = e else {
            return false;
        };
        let Expr::Ident(n) = callee.as_ref() else {
            return false;
        };
        if n != "rep" {
            return false;
        }
        matches!(self.lookup(n, env), Ok(Value::Native(nf)) if nf.name == "rep")
    }

    // try/catch (issue #125). The body runs in its own scope; if `fail`
    // (Flow::Fail) or a runtime error (Flow::Error) is raised — we catch it and
    // run the catch body. ret/skip/stop flow signals are not caught: they pass
    // through try to control the function/loop (flow, not an error). If there is a
    // catch variable, a {message, status} map is bound to it (status — int or nil).
    // `tail_used` flows into the body/catch blocks so a `rep` in a try used as a
    // value (`r = try ...`) stays a value, while a guard `rep` short-circuits.
    pub(crate) fn eval_try(
        &self,
        body: &[Stmt],
        catch_var: Option<&str>,
        catch_body: &[Stmt],
        env: &Env,
        tail_used: bool,
    ) -> EvalResult {
        let inner = Scope::child_of(env);
        match self.exec_block(body, &inner, tail_used) {
            Ok(v) => Ok(v),
            Err(Flow::Fail { status, message }) => {
                self.run_catch(catch_var, status, message, catch_body, env, tail_used)
            }
            Err(Flow::Error(message)) => {
                self.run_catch(catch_var, None, message, catch_body, env, tail_used)
            }
            // ret/skip/stop — flow signals, not caught.
            Err(other) => Err(other),
        }
    }

    // Runs the catch body with the error map.
    fn run_catch(
        &self,
        catch_var: Option<&str>,
        status: Option<i64>,
        message: String,
        catch_body: &[Stmt],
        env: &Env,
        tail_used: bool,
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
        self.exec_block(catch_body, &inner, tail_used)
    }

    // `tail_used` flows into the selected branch: `r = if ... \n rep ...` keeps
    // `rep` as a value, but a guard `if cond \n rep ...` (value discarded) lets
    // the branch's `rep` short-circuit the enclosing function (issue #173).
    pub(crate) fn eval_if(&self, ifx: &IfExpr, env: &Env, tail_used: bool) -> EvalResult {
        for (cond, block) in &ifx.arms {
            if self.eval(cond, env)?.truthy() {
                let inner = Scope::child_of(env);
                return self.exec_block(block, &inner, tail_used);
            }
        }
        if let Some(eb) = &ifx.else_block {
            let inner = Scope::child_of(env);
            return self.exec_block(eb, &inner, tail_used);
        }
        Ok(Value::Nil)
    }

    pub(crate) fn eval_match(&self, mx: &MatchExpr, env: &Env, tail_used: bool) -> EvalResult {
        let subj = self.eval(&mx.subject, env)?;
        for arm in &mx.arms {
            let matched = match &arm.pattern {
                MatchPat::Wildcard => true,
                MatchPat::Sym(s) => matches!(&subj, Value::Sym(v) if v == s),
                MatchPat::Int(n) => matches!(&subj, Value::Int(v) if v == n),
            };
            if matched {
                let inner = Scope::child_of(env);
                return self.exec_block(&arm.body, &inner, tail_used);
            }
        }
        Ok(Value::Nil)
    }
}
