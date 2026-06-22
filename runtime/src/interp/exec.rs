// Statement execution: the top-level `run`/`run_repl_chunk` drivers, the block
// and statement executor, the `=`/`<-` binding rules, and the `each` loop.

use std::sync::Arc;

use crate::ast::*;
use crate::value::{FnValue, Value};

use super::Interp;
use super::scope::{Env, ExecResult, Flow, Parent, Scope};

impl Interp {
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
            // Top level — no enclosing function to return from, so a bare `rep`
            // is just a value (`tail_used = true`); a Return would be ignored anyway.
            match self.exec_stmt(stmt, &self.global.clone(), true) {
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
            // REPL top level — keep the last expression's value (a bare `rep`
            // yields its response map for display); `tail_used = true`.
            match self.exec_stmt(stmt, &self.global.clone(), true) {
                Ok(v) => last = v,
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
        Ok(last)
    }

    // Executes the block in sequence; its value is the last expression (in Fluxon
    // a block is an expression). `tail_used` — whether this block's VALUE is
    // consumed by the caller (an `if` as an assignment RHS, a function body whose
    // last expression is the return value, ...). It flows to the LAST statement
    // only; the values of earlier statements are always discarded. It governs
    // whether a bare `rep` short-circuits (issue #173): a `rep` whose value is
    // discarded returns from the enclosing function like `ret`, but a `rep` in
    // value position stays a value so responses can still be built and inspected.
    pub(crate) fn exec_block(&self, stmts: &[Stmt], env: &Env, tail_used: bool) -> ExecResult {
        let mut last = Value::Nil;
        let n = stmts.len();
        for (i, s) in stmts.iter().enumerate() {
            last = self.exec_stmt(s, env, tail_used && i + 1 == n)?;
        }
        Ok(last)
    }

    pub(crate) fn exec_stmt(&self, stmt: &Stmt, env: &Env, tail_used: bool) -> ExecResult {
        match stmt {
            Stmt::Bind { name, value } => {
                let v = self.eval(value, env)?;
                self.bind(name, v, env)?;
                Ok(Value::Nil)
            }
            Stmt::Assign { target, value } => {
                let v = self.eval(value, env)?;
                match target.as_ref() {
                    // `x <- v` — plain variable reassignment (the old path).
                    Expr::Ident(name) => self.assign(name, v, env)?,
                    // `req.ctx <- v` — write to the shared ctx cell (issue #68).
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
            Stmt::IndexAssign { target, value } => {
                let v = self.eval(value, env)?;
                self.index_assign(target, v, env)?;
                Ok(Value::Nil)
            }
            Stmt::ExpBind { name, value } => {
                let v = self.eval(value, env)?;
                // exp bind — an exportable global. (The `false` is the legacy
                // mutability flag, no longer consulted — see `Scope::vars`.)
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
            Stmt::Expr(e) => {
                // `if`/`match`/`try` in statement position propagate `tail_used`
                // into their branch bodies, so a guard like `if cond \n rep ...`
                // (the `if` is NOT the consumed tail) short-circuits inside the
                // branch, while `r = if cond \n rep ...` (consumed) keeps `rep` as
                // a value. (A normal `self.eval` would treat them as value position.)
                match e {
                    Expr::If(ifx) => return self.eval_if(ifx, env, tail_used),
                    Expr::Match(mx) => return self.eval_match(mx, env, tail_used),
                    Expr::TryCatch {
                        body,
                        catch_var,
                        catch_body,
                    } => {
                        return self.eval_try(
                            body,
                            catch_var.as_deref(),
                            catch_body,
                            env,
                            tail_used,
                        );
                    }
                    _ => {}
                }
                let v = self.eval(e, env)?;
                // A bare `rep ...` whose value is DISCARDED short-circuits the
                // enclosing function like `ret` (issue #173): a guard clause
                // `if cond \n rep ...` must stop the handler instead of falling
                // through to a later `rep` (the last `rep` used to silently win).
                // In value position (`r = rep ...`, an argument, a consumed block
                // tail) `rep` stays a value. Only the BUILTIN `rep` triggers this —
                // a user binding that shadows the name keeps normal call semantics
                // (the shadowing invariant — see CLAUDE.md).
                if !tail_used && self.is_builtin_rep_call(e, env) {
                    return Err(Flow::Return(v));
                }
                Ok(v)
            }
            // use — module import. Two kinds:
            //  • Battery (`use http`, `use db`) — dispatched by name, NO
            //    registration NEEDED, so a no-op.
            //  • User file (`use ./tools`, `use ../lib/x as y`) — reads the file,
            //    executes it in a separate module scope, and binds the `exp`-ed
            //    names under `tools.name` (or the alias) into the current scope.
            Stmt::Use { items } => {
                self.exec_use(items, env)?;
                Ok(Value::Nil)
            }
            // tbl — written into the schema registry (sym/json conversion + migration).
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

    // `<-` reassignment: finds the variable in the scope chain and updates it.
    // If not found — creates it in the current scope as mutable.
    fn assign(&self, name: &str, v: Value, env: &Env) -> Result<(), Flow> {
        let mut cur = env.clone();
        loop {
            // Under a single write lock: find and update the name OR get the next
            // parent (previously a write + a separate read — two locks per level).
            let parent = {
                let mut s = cur.write();
                if let Some(slot) = s.get_mut_value(name) {
                    *slot = v;
                    return Ok(());
                }
                s.parent.clone()
            };
            match parent {
                Parent::Scope(p) => cur = p,
                // The parent is the root (marker). After freezing the global is
                // FROZEN (an immutable snapshot) — we DO NOT TOUCH the root. If
                // the name exists as a global, it cannot be changed from inside a
                // handler with `<-`: we give an EXPLICIT error (NOT a silent
                // shadow — the developer must not hit a silent failure). This is a
                // thread-safety guard (handlers run in parallel; a shared mutable
                // global would race), NOT immutability. If the name is new we
                // create a local in the current scope. If not frozen (top-level)
                // we look up/mutate `Interp.global` as usual.
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
        // a new mutable variable
        env.write().define(name, v, true);
        Ok(())
    }

    // `obj.field <- v` — member assignment. For now ONLY writing to the shared
    // ctx cell is supported (`req.ctx <- {...}`, issue #68). `obj` = `req` (Map),
    // `field` = "ctx" → the req map's "ctx" key holds `Value::Ctx(Arc<Mutex>)`.
    // `obj` (Map) is cloned, but the inner `Value::Ctx` Arc is shared, so even
    // through the clone we write to the original Mutex cell — the handler sees the
    // ctx the middleware wrote in the same cell. A plain Map stays immutable:
    // writing to a non-`Value::Ctx` field is rejected.
    pub(crate) fn assign_field(&self, obj: &Value, field: &str, v: Value) -> Result<(), Flow> {
        if let Value::Map(m) = obj
            && let Some(Value::Ctx(cell)) = m.get(field)
        {
            // ctx is fully replaced (a new map is written). The value being
            // written must be a map (or another ctx snapshot).
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

    // `m[k] = v` / `l[i] = v` / `m.field = v` — in-place element mutation (issue
    // #220). `target` is an Index/Field chain whose innermost base must be a plain
    // variable (`cnt[w]`, `grid[r][c]`, `cfg.db.port`). We:
    //   1. flatten the chain into the root variable name + a path of accessors
    //      (outermost→innermost), evaluating any computed keys up front, then
    //   2. locate the variable's slot with BIND lookup (within the current fn,
    //      if/each/match transparent), and mutate the nested element in place.
    // If the root variable doesn't exist yet, we create it (matching `=` for a
    // single key: `cnt = {}` is the normal precursor, but `cnt[w] = 1` on a fresh
    // name should still build the map). Maps grow on a missing key; lists require
    // an in-range integer index (a list is fixed-length — extend with `l.push`).
    fn index_assign(&self, target: &Expr, v: Value, env: &Env) -> Result<(), Flow> {
        // Flatten the accessor chain. We walk from the outer expression inward,
        // pushing each accessor, until we hit the root Ident.
        let mut path: Vec<Accessor> = Vec::new();
        let mut node = target;
        let root_name = loop {
            match node {
                Expr::Index { target, key } => {
                    let k = self.eval(key, env)?;
                    let acc = match k {
                        Value::Str(s) | Value::Sym(s) => Accessor::Key(s),
                        Value::Int(i) => Accessor::Idx(i),
                        other => {
                            return Err(Flow::err(format!(
                                "index assignment key must be a string or int, got {}",
                                other.type_name()
                            )));
                        }
                    };
                    path.push(acc);
                    node = target;
                }
                Expr::Field { target, name } => {
                    path.push(Accessor::Key(name.clone()));
                    node = target;
                }
                Expr::Ident(name) => break name.clone(),
                _ => {
                    return Err(Flow::err(
                        "index assignment target must start at a variable (e.g. `m[k] = v`)",
                    ));
                }
            }
        };
        // We collected outer→inner; reverse so the path reads root→leaf.
        path.reverse();

        // Locate the variable slot with bind lookup and mutate in place under one
        // write lock. We mirror `bind`'s traversal (function-local, transparent
        // blocks) so `cnt[w]=` inside an `each` updates the outer `cnt`.
        let mut cur = env.clone();
        loop {
            let (parent, at_boundary) = {
                let mut s = cur.write();
                if let Some(slot) = s.get_mut_value(&root_name) {
                    return apply_path(slot, &path, v, &root_name);
                }
                (s.parent.clone(), s.is_fn_boundary)
            };
            if at_boundary {
                break;
            }
            match parent {
                Parent::Scope(p) => cur = p,
                Parent::Root => {
                    if self.globals_frozen.get().is_some() {
                        break;
                    }
                    cur = self.global.clone();
                }
                Parent::None => break,
            }
        }
        // The variable does not exist yet — create it in the current scope as an
        // empty map and write into it (so `cnt[w] = 1` works without `cnt = {}`).
        let mut fresh = Value::Map(std::collections::BTreeMap::new());
        apply_path(&mut fresh, &path, v, &root_name)?;
        env.write().define(&root_name, fresh, true);
        Ok(())
    }

    // `=` bind: searches for the variable WITHIN THE CURRENT FUNCTION. if/each/
    // match blocks are lexically transparent — an `=` inside them updates an
    // outer variable in the same fn (Python: if/for open no scope), so an
    // accumulator (`total = 0` then `each .. total = total + x`) works. The
    // search STOPS at the fn/lambda boundary (`is_fn_boundary`): inside a fn an
    // `=` does not reach an outer global, it creates a new LOCAL — the Python
    // rule (assignment in a function makes a local unless you reach out
    // explicitly, which here is `<-`). If found within the frame, the variable
    // is updated in place; if not, a fresh local is created in the current scope.
    // There is NO immutability: re-binding a name is always allowed, like Python.
    fn bind(&self, name: &str, v: Value, env: &Env) -> Result<(), Flow> {
        let mut cur = env.clone();
        loop {
            let (parent, at_boundary) = {
                let mut s = cur.write();
                if let Some(slot) = s.get_mut_value(name) {
                    *slot = v;
                    return Ok(());
                }
                // Reached the fn/lambda boundary — we do not go outside this fn.
                (s.parent.clone(), s.is_fn_boundary)
            };
            if at_boundary {
                break;
            }
            match parent {
                Parent::Scope(p) => cur = p,
                // Root — the top-level global. When not frozen we could search the
                // global, but `=` semantics: create a new local in the current
                // scope (at top-level `cur` is already the global). The chain
                // continues to search the outer global.
                Parent::Root => {
                    if self.globals_frozen.get().is_some() {
                        break; // frozen global — we create a new local
                    }
                    cur = self.global.clone();
                }
                Parent::None => break,
            }
        }
        // a new variable in the current scope
        env.write().define(name, v, true);
        Ok(())
    }

    fn exec_each(&self, vars: &[String], iter: &Expr, body: &[Stmt], env: &Env) -> ExecResult {
        // `each i in inf` — an infinite loop (for REPL/event-loop). i = 0,1,2,...
        // controlled with `stop`/`skip`. Does not build an eager Vec (it would be
        // infinite).
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
                // Loop variables are mutable (`<-` allowed in the body; reset on
                // each iteration).
                if vars.len() == 2 {
                    // each k, v in map
                    let k = key.unwrap_or(Value::Nil);
                    s.define(&vars[0], k, true);
                    s.define(&vars[1], val, true);
                } else {
                    // each x in list  — over a map, this is the value
                    s.define(&vars[0], val, true);
                }
            }
            // The loop body's value is discarded (each returns nil), so
            // `tail_used = false`: a `rep` inside the loop short-circuits.
            match self.exec_block(body, &loop_env, false) {
                Ok(_) => {}
                Err(Flow::Skip) => continue,
                Err(Flow::Stop) => break,
                Err(other) => return Err(other),
            }
        }
        Ok(Value::Nil)
    }

    // `each i in inf` — infinite repetition. The counter i starts at 0 and
    // increments by 1 each iteration (stops on i64 overflow — unreachable in
    // practice). `stop` exits, `skip` moves to the next.
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
                // The loop variable is mutable (`<-` allowed in the body).
                s.define(&vars[0], Value::Int(i), true);
            }
            match self.exec_block(body, &loop_env, false) {
                Ok(_) => {}
                Err(Flow::Skip) => {}
                Err(Flow::Stop) => break,
                Err(other) => return Err(other),
            }
            match i.checked_add(1) {
                Some(n) => i = n,
                None => break, // i64 limit — unreachable in practice
            }
        }
        Ok(Value::Nil)
    }
}

// One step in an index-assignment path: a map key or a list index. Computed keys
// are evaluated before the path is walked (see `index_assign`), so the path holds
// already-resolved accessors and the slot mutation needs no `&self`/`env`.
enum Accessor {
    Key(String),
    Idx(i64),
}

// Walks `path` into `slot` (a Map/List value held in a scope) and writes `v` at
// the leaf, mutating in place. Intermediate map keys are auto-created (an empty
// map) so a deep write like `cfg["db"]["port"] = 8080` works on a fresh `cfg`;
// list indices must already be in range (lists are fixed-length — grow with
// `l.push`). `root` is only used for error messages.
fn apply_path(slot: &mut Value, path: &[Accessor], v: Value, root: &str) -> Result<(), Flow> {
    let (last, parents) = match path.split_last() {
        Some(p) => p,
        // No accessors — this would be a plain `=`, never produced by the parser.
        None => return Err(Flow::err("index assignment has no key")),
    };
    // Descend through the parent accessors, auto-creating missing map levels.
    let mut cur = slot;
    for acc in parents {
        cur = step_into(cur, acc, root)?;
    }
    // Write the leaf.
    match (cur, last) {
        (Value::Map(m), Accessor::Key(k)) => {
            m.insert(k.clone(), v);
            Ok(())
        }
        (Value::List(xs), Accessor::Idx(i)) => {
            let idx = *i;
            if idx < 0 || idx as usize >= xs.len() {
                return Err(Flow::err(format!(
                    "list index {} out of range (len {}) — lists are fixed-length, use l.push to grow",
                    idx,
                    xs.len()
                )));
            }
            xs[idx as usize] = v;
            Ok(())
        }
        (other, Accessor::Key(_)) => Err(Flow::err(format!(
            "cannot assign a string key into {} (only maps have string keys)",
            other.type_name()
        ))),
        (other, Accessor::Idx(_)) => Err(Flow::err(format!(
            "cannot assign an int index into {} (only lists are int-indexed)",
            other.type_name()
        ))),
    }
}

// Returns a mutable reference to the child at `acc`, auto-creating a missing map
// key as an empty map (so deep writes build the path). List indices must exist.
fn step_into<'a>(cur: &'a mut Value, acc: &Accessor, root: &str) -> Result<&'a mut Value, Flow> {
    match (cur, acc) {
        (Value::Map(m), Accessor::Key(k)) => Ok(m
            .entry(k.clone())
            .or_insert_with(|| Value::Map(std::collections::BTreeMap::new()))),
        (Value::List(xs), Accessor::Idx(i)) => {
            let idx = *i;
            if idx < 0 || idx as usize >= xs.len() {
                return Err(Flow::err(format!(
                    "list index {} out of range (len {})",
                    idx,
                    xs.len()
                )));
            }
            Ok(&mut xs[idx as usize])
        }
        (other, Accessor::Key(_)) => Err(Flow::err(format!(
            "cannot index into {} with a string key (in `{}[...]`)",
            other.type_name(),
            root
        ))),
        (other, Accessor::Idx(_)) => Err(Flow::err(format!(
            "cannot index into {} with an int (in `{}[...]`)",
            other.type_name(),
            root
        ))),
    }
}
