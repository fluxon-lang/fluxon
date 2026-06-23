// Binary operators, function application, the call dispatch (module/method/
// HOF routing), and field/index access.

use crate::ast::*;
use crate::value::Value;

use super::Interp;
use super::scope::{CallDepthGuard, Env, EvalResult, Flow, STACK_GROW_SIZE, STACK_RED_ZONE, Scope};
use super::util::{flt_arith, int_arith, is_num, to_f64};

impl Interp {
    pub(crate) fn eval_binary(&self, op: BinOp, lhs: &Expr, rhs: &Expr, env: &Env) -> EvalResult {
        // Short-circuit operators
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
                // x |> f      ==  f x       (f — a function value or lambda)
                // x |> f a b  ==  f a b x   (if rhs is a call, x is the LAST argument)
                //
                // The second form turns pipe into a partial call: in `db.from "t"
                // |> db.eq {...}` the `db.eq {...}` arrives as the rhs Call; we do
                // not evaluate it immediately but `eval_call` it with lhs appended
                // to the args. That is why module dispatches like db.*/str.* also
                // work naturally (eval_call routes them specially). The existing
                // `x |> str.up` now works — previously it called rhs with no
                // arguments and errored.
                let l = self.eval(lhs, env)?;
                match rhs {
                    // `x |> f a b` => `f a b x`: lhs appended to the args.
                    Expr::Call { callee, args } => {
                        let mut argv = self.eval_args(args, env)?;
                        argv.push(l);
                        // Same arg-position binding as a direct call (issue #222):
                        // `5 |> log fac` must behave like `log (fac 5)`.
                        let argv = self.fold_nested_calls(callee, argv, env)?;
                        return self.apply_callee(callee, argv, env);
                    }
                    // `x |> str.up` / `x |> db.all` => an argument-less
                    // module/method call, lhs the single argument. A Field cannot
                    // be evaluated as a value (a module function is not a value), so
                    // we go directly to apply_callee.
                    Expr::Field { .. } => {
                        return self.apply_callee(rhs, vec![l], env);
                    }
                    // rhs is a plain function value/lambda/ident: f x.
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
        // Comparison and arithmetic
        match (op, l, r) {
            // + string concatenation
            (BinOp::Add, Str(a), Str(b)) => Ok(Str(a + &b)),
            (BinOp::Add, Str(a), b) => Ok(Str(a + &b.to_text())),
            (BinOp::Add, a, Str(b)) => Ok(Str(a.to_text() + &b)),

            // int-int arithmetic
            (op, Int(a), Int(b)) => int_arith(op, a, b),
            // mixed/float arithmetic
            (op, a, b) if is_num(&a) && is_num(&b) => flt_arith(op, to_f64(&a), to_f64(&b)),

            // str ordering — lexicographic by Unicode scalar value (byte order
            // for UTF-8 is the same as code point order). Gives `<`/`>`/`<=`/`>=`
            // a canonical meaning for strings: sort names/ids/timestamps without
            // falling back to time.diff. See issue #174.
            (BinOp::Lt, Str(a), Str(b)) => Ok(Bool(a < b)),
            (BinOp::Le, Str(a), Str(b)) => Ok(Bool(a <= b)),
            (BinOp::Gt, Str(a), Str(b)) => Ok(Bool(a > b)),
            (BinOp::Ge, Str(a), Str(b)) => Ok(Bool(a >= b)),

            (op, a, b) => Err(Flow::err(format!(
                "{:?} operator cannot be applied to {} and {}",
                op,
                a.type_name(),
                b.type_name()
            ))),
        }
    }

    // ---------------- call ----------------
    pub(crate) fn eval_call(&self, callee: &Expr, args: &[Expr], env: &Env) -> EvalResult {
        let argv = self.eval_args(args, env)?;
        // Paren-free nested calls bind tighter in argument position (issue #219):
        // `log fac 4` means `log (fac 4)`, not `log(fac, 4)`. Before this, a bare
        // function value sitting in argument position was passed UNCALLED — the
        // program ran and printed the wrong thing (`<fn fac> 4`) with no error.
        // Now an argument that is a function value consumes the arguments that
        // follow it (innermost-first) and is applied, so `f g x` => `f (g x)`.
        let argv = self.fold_nested_calls(callee, argv, env)?;
        self.apply_callee(callee, argv, env)
    }

    // Is this callee a builtin that takes plain VALUES (so a function in
    // argument position is a mistake to be folded), as opposed to one that takes
    // a callback (which must arrive uncalled)? This is an ALLOWLIST of the
    // value-taking builtins, so any callback API is excluded by default:
    //   - bare `log` / `rep` / `assert` (the value-taking globals);
    //   - the leveled logger `log.debug/info/warn/err`;
    //   - the pure value modules (`is_module`: str/math/json/time/...).
    // Excluded (NOT folded): user `fn`s (may take a callback before an options
    // map, `run_with cb {opts}`), the callback dispatches (ai/http/ws/db/reg/
    // cron/queue/par), and list HOF methods (`xs.map f`). Names defer to a user
    // binding of the same name — the precedence `apply_callee` itself uses.
    fn callee_takes_values(&self, callee: &Expr, env: &Env) -> bool {
        match callee {
            // `rep`/`assert` are installed globals — `lookup` returns the native
            // (a user re-binding would shadow it and is left alone). `log` is not
            // installed, so it is the builtin only when no `log` var exists.
            Expr::Ident(id) => match id.as_str() {
                "log" => self.lookup(id, env).is_err(),
                "rep" | "assert" => {
                    matches!(self.lookup(id, env), Ok(Value::Native(_)))
                }
                _ => false,
            },
            Expr::Field { target, name } => match target.as_ref() {
                // `log.debug/info/warn/err` — the leveled logger (value-taking).
                Expr::Ident(m) if m == "log" => {
                    matches!(name.as_str(), "debug" | "info" | "warn" | "err")
                        && self.lookup(m, env).is_err()
                }
                // `str.*`/`math.*`/... — pure value modules. `apply_callee`
                // routes `is_module` names to `call_module` UNCONDITIONALLY (a
                // module name is never a value, so a `math = nil` binding does
                // not shadow `math.max`); match that here, with no lookup check,
                // so folding stays consistent with dispatch (Codex review #222).
                Expr::Ident(m) => crate::builtins::is_module(m),
                _ => false,
            },
            _ => false,
        }
    }

    // Collapses bare function values in argument position into nested calls
    // (issue #219). Scans RIGHT-to-LEFT so inner calls resolve before the
    // outer ones consume their result: in `log math.max fac 3 fac 4`, the inner
    // `fac` calls resolve first, then `math.max ..`, leaving `log` one argument.
    //
    // Only applies when the CALLEE is a builtin that takes VALUES, never a
    // callback: bare `log`, or a pure value module (`str`/`math`/`json`/... —
    // exactly `is_module`). Everything else is left alone — in particular a user
    // `fn` (which may legitimately take a callback followed by an options map,
    // `run_with cb {opts}`) and the callback-taking dispatches (`ai.stream`,
    // `http.on`, `xs.map`, `db.tx`, `reg.add`, ...), where folding would call the
    // callback during argument evaluation and break the API (Codex review #222).
    //
    // A function value is also only folded when it is NOT the last argument. A
    // user `fn` consumes exactly its own arity; a native fn's arity is unknown,
    // so it greedily consumes the rest (an arity mismatch then fails loudly
    // inside the native, never silently).
    fn fold_nested_calls(
        &self,
        callee: &Expr,
        argv: Vec<Value>,
        env: &Env,
    ) -> Result<Vec<Value>, Flow> {
        if !self.callee_takes_values(callee, env) {
            return Ok(argv);
        }
        // Fast path: nothing to fold unless a non-last argument is callable.
        let needs_fold = argv
            .iter()
            .take(argv.len().saturating_sub(1))
            .any(|v| matches!(v, Value::Fn(_) | Value::Native(_)));
        if !needs_fold {
            return Ok(argv);
        }
        let mut out: Vec<Value> = Vec::with_capacity(argv.len());
        // Build the result right-to-left, then reverse. `out` holds the
        // already-folded tail (in reverse); a function pops its arguments off it.
        for v in argv.into_iter().rev() {
            // A nullary fn consumes nothing — folding it would auto-call it
            // (`log new_id "tag"` must NOT run `new_id`; Fluxon requires the
            // explicit `new_id()`). So `take == 0` falls through to pass-through.
            let take = match &v {
                Value::Fn(fv) => fv.params.len().min(out.len()),
                Value::Native(_) => out.len(), // native: arity unknown, take the rest
                _ => 0,
            };
            match &v {
                Value::Fn(_) | Value::Native(_) if take > 0 => {
                    let mut call_args = Vec::with_capacity(take);
                    for _ in 0..take {
                        // `out` holds the tail in reverse, so its back is the
                        // argument nearest this function — popping yields them in
                        // left-to-right source order.
                        call_args.push(out.pop().unwrap());
                    }
                    let result = self.apply(v, call_args)?;
                    out.push(result);
                }
                _ => out.push(v),
            }
        }
        out.reverse();
        Ok(out)
    }

    // Calls the callee with the arguments ALREADY evaluated. eval_call and pipe
    // (`x |> f a` => `f a x`) both reach this single point — the dispatch logic is
    // in one place. `argv` are the call arguments (in the pipe case, lhs appended).
    pub(crate) fn apply_callee(&self, callee: &Expr, argv: Vec<Value>, env: &Env) -> EvalResult {
        // Method call: target.method arg...  -> arrives as a Field.
        if let Expr::Field { target, name } = callee {
            // A two-level module namespace: ws.room.* / ws.data.* — the target
            // itself is Field{Ident("ws"), "room"/"data"}. It does not reach the
            // `Ident` arm, so we catch it here separately (ws is stateful, needs
            // the Interp). For now only the `ws` namespace has inner groups.
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
                    _ => Err(Flow::err(format!("ws.{} group does not exist", sub))),
                };
            }
            // module.func (str.up, math.floor, ...) — `str` is not a variable, so
            // we check the module BEFORE evaluating the target.
            if let Expr::Ident(modname) = target.as_ref() {
                // http — stateful and needs the Interp (to apply handlers), so we
                // route to http_dispatch, not call_module.
                if modname == "http" {
                    return self.arc_self().http_dispatch(name, argv);
                }
                // db — stateful like http (connection + tx context); needs the
                // Interp. The db.tx argument arrives as a lambda (Value::Fn).
                if modname == "db" {
                    return self.arc_self().db_dispatch(name, argv);
                }
                // ws — stateful like http (live connections, needs the Interp to
                // apply handlers). ws.room.*/ws.data.* arrive as a two-level Field
                // — caught below (inside the Field target).
                if modname == "ws" {
                    return self.arc_self().ws_dispatch(name, argv);
                }
                // reg — stateful (the function registry); `reg.add`/`reg.call`
                // take functions/arguments as arguments. `reg.names` with no args
                // is caught in the Field arm (below).
                if modname == "reg" {
                    return self.reg_dispatch(name, argv);
                }
                // cron — stateful (scheduled tasks). `cron.on` takes an expression
                // + handler, `cron.run` blocks with no args. The expression arrives
                // from the parser as an unquoted 5-field str (the parser catches it
                // specially below).
                if modname == "cron" {
                    return self.arc_self().cron_dispatch(name, argv);
                }
                // queue — stateful (a background queue). `queue.push` takes
                // name+payload, `queue.on` takes name+handler. The worker applies
                // the handler — so it needs the Interp (not call_module).
                if modname == "queue" {
                    return self.arc_self().queue_dispatch(name, argv);
                }
                // ai — an LLM primitive (Anthropic). Needs the Interp to read
                // `$AI_KEY` via env_lookup (not call_module). Stateless — each call
                // is an independent https POST. If `ai` is declared as a variable,
                // it is not the module — it is seen as a variable.
                if modname == "ai" && self.lookup(modname, env).is_err() {
                    return self.ai_dispatch(name, argv);
                }
                // auth — authentication primitives (JWT + password hash).
                // Stateless like `ai`; needs the Interp to read `$AUTH_SECRET` via
                // env_lookup (not call_module). If `auth` is declared as a
                // variable, it is not the module — the variable takes precedence.
                if modname == "auth" && self.lookup(modname, env).is_err() {
                    return self.auth_dispatch(name, argv);
                }
                // crypto — cryptographic primitives (issue #131). Stateless and
                // does not need the Interp, but a battery like auth/ai: if the
                // `crypto` name is declared (e.g. `use ./crypto`), it takes
                // precedence — so it is not in the unconditional is_module list.
                if modname == "crypto" && self.lookup(modname, env).is_err() {
                    return crate::crypto_mod::crypto_module(name, argv);
                }
                // log — a leveled logger (issue #139). `log.debug/info/warn/err`.
                // `log` is not a global; if the user has not declared a `log`
                // variable it is caught here. An unknown level — an explicit error.
                if modname == "log" && self.lookup(modname, env).is_err() {
                    return match name.as_str() {
                        "debug" | "info" | "warn" | "err" => self.log_dispatch(name, argv),
                        _ => Err(Flow::err(format!(
                            "log.{} does not exist (debug/info/warn/err)",
                            name
                        ))),
                    };
                }
                if crate::builtins::is_module(modname) {
                    return crate::builtins::call_module(modname, name, argv);
                }
            }
            let recv = self.eval(target, env)?;
            // First, if a real map field is a function (e.g. a lambda inside the
            // map) — we call it; otherwise a builtin method.
            if let Value::Map(m) = &recv
                && let Some(v @ (Value::Fn(_) | Value::Native(_))) = m.get(name)
            {
                let f = v.clone();
                return self.apply(f, argv);
            }
            // Higher-order list methods (they call a lambda) — here, because
            // builtins cannot reach the Interp.
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
        // Bare `log "..."` — a level-less call = info (issue #139). `log` is not a
        // global (a pure dispatch battery); if the user has not declared a `log`
        // variable it is caught here, otherwise the variable takes precedence.
        if let Expr::Ident(id) = callee
            && id == "log"
            && self.lookup(id, env).is_err()
        {
            return self.log_dispatch("info", argv);
        }
        // par [\-> ... \-> ...] — a language-level parallel fan-out (issue #137).
        // Calls each lambda in the list on a SEPARATE thread, waits for all of
        // them, and returns the list of results (in input order). `par` is not a
        // global (a pure primitive); if the user has not declared a `par` variable
        // it is caught here, otherwise the variable takes precedence.
        if let Expr::Ident(id) = callee
            && id == "par"
            && self.lookup(id, env).is_err()
        {
            return self.arc_self().par_run(argv);
        }
        let f = self.eval(callee, env)?;
        self.apply(f, argv)
    }

    // Higher-order list methods (filter/map/reduce/find/any/all/sort) — call the
    // function argument for the element(s).
    fn list_hof(&self, xs: &[Value], method: &str, args: Vec<Value>) -> EvalResult {
        match method {
            "filter" => {
                let f = args
                    .into_iter()
                    .next()
                    .ok_or_else(|| Flow::err("list.filter: function argument required"))?;
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
                    .ok_or_else(|| Flow::err("list.map: function argument required"))?;
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
                    .ok_or_else(|| Flow::err("list.reduce: initial value required"))?;
                let f = it
                    .next()
                    .ok_or_else(|| Flow::err("list.reduce: function argument required"))?;
                for x in xs {
                    acc = self.apply(f.clone(), vec![acc, x.clone()])?;
                }
                Ok(acc)
            }
            "find" => {
                // Returns the first element matching the predicate; nil if none.
                // (list.index gives the position via -1; find gives the value.)
                let f = args
                    .into_iter()
                    .next()
                    .ok_or_else(|| Flow::err("list.find: function argument required"))?;
                for x in xs {
                    if self.apply(f.clone(), vec![x.clone()])?.truthy() {
                        return Ok(x.clone());
                    }
                }
                Ok(Value::Nil)
            }
            "any" => {
                // Stops at the first match (short-circuit) — unlike the
                // filter+len detour, the predicate is not called for the rest.
                let f = args
                    .into_iter()
                    .next()
                    .ok_or_else(|| Flow::err("list.any: function argument required"))?;
                for x in xs {
                    if self.apply(f.clone(), vec![x.clone()])?.truthy() {
                        return Ok(Value::Bool(true));
                    }
                }
                Ok(Value::Bool(false))
            }
            "all" => {
                // Stops at the first non-match; true for an empty list (vacuous).
                let f = args
                    .into_iter()
                    .next()
                    .ok_or_else(|| Flow::err("list.all: function argument required"))?;
                for x in xs {
                    if !self.apply(f.clone(), vec![x.clone()])?.truthy() {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(true))
            }
            "sort" => {
                // Sort with a comparator: \a b -> number (negative: a first,
                // positive: b first, 0: equal) — JS style. An argument-less
                // `l.sort` arrives as a Field and falls into the natural ordering
                // in builtins; only a Call (with arguments) reaches here, but an
                // empty argv is also handled defensively.
                let Some(f) = args.into_iter().next() else {
                    return crate::builtins::sort_default(xs);
                };
                let sorted = crate::builtins::sort_values(xs.to_vec(), &mut |a, b| match self
                    .apply(f.clone(), vec![a.clone(), b.clone()])?
                {
                    Value::Int(n) => Ok(n.cmp(&0)),
                    Value::Flt(x) => Ok(x.partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal)),
                    other => Err(Flow::err(format!(
                        "list.sort: comparator must return a number (negative/0/positive), got {}",
                        other.type_name()
                    ))),
                })?;
                Ok(Value::List(sorted))
            }
            _ => unreachable!(),
        }
    }

    pub(crate) fn eval_args(&self, args: &[Expr], env: &Env) -> Result<Vec<Value>, Flow> {
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
                        "{}: expected {} arguments, got {}",
                        fv.name,
                        fv.params.len(),
                        args.len()
                    )));
                }
                // Depth limit: infinite recursion ABORTS the whole process on a
                // stack overflow (not a panic — spawn_blocking does not save it
                // either). The limit returns a graceful Flow::err well before that
                // abort. The guard is RAII — the counter decrements correctly even
                // on the error/panic path (issue #90).
                let _depth = CallDepthGuard::enter(&fv.name)?;
                // If the native stack is running low we allocate a new segment (the
                // rustc approach): deep (but within-limit) recursion does not
                // overflow even on a 2MB spawn_blocking/test thread — the real
                // bound stays only MAX_CALL_DEPTH.
                stacker::maybe_grow(STACK_RED_ZONE, STACK_GROW_SIZE, || {
                    // A child pre-sized by the number of params — the Vec is not
                    // re-allocated during bind. Params are mutable: they can be
                    // changed with `<-` in the body (this was already allowed).
                    let call_env = Scope::child_with_capacity(fv.parent.clone(), fv.params.len());
                    {
                        let mut s = call_env.write();
                        for (p, a) in fv.params.iter().zip(args) {
                            // We use `define` (not a raw push): the parser rejects a
                            // duplicate param, but define is defensive — if a name
                            // does repeat, write/read stay on a single slot (no
                            // define-front / get-back inconsistency). Params are
                            // small (0-4), so O(n²) is cheap. Mutable: the body can `<-`.
                            s.define(p, a, true);
                        }
                    }
                    // A function body's last expression IS the return value, so
                    // `tail_used = true`: a trailing `rep` becomes the value (same
                    // observable result as returning it), while a guard `rep`
                    // earlier in the body short-circuits.
                    match self.exec_block(&fv.body, &call_env, true) {
                        Ok(v) => Ok(v),                // last expression — returns
                        Err(Flow::Return(v)) => Ok(v), // early ret
                        Err(other) => Err(other),      // fail/err/skip/stop
                    }
                })
            }
            other => Err(Flow::err(format!(
                "{} is not callable (not a function)",
                other.type_name()
            ))),
        }
    }

    // ---------------- field / index ----------------
    pub(crate) fn get_field(&self, t: &Value, name: &str, _env: &Env) -> EvalResult {
        match t {
            Value::Map(m) => {
                // First a real key; if absent, an argument-less method (keys/vals/len).
                if let Some(v) = m.get(name) {
                    // Reading a ctx cell — we return a snapshot Map (the handler
                    // should see a plain map, not the inner Ctx type).
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
            // Argument-less methods like .len also work as a field.
            Value::List(_) | Value::Str(_) => crate::builtins::call_method(t, name, vec![]),
            Value::Nil => Ok(Value::Nil), // nil.x -> nil (safe navigation)
            other => Err(Flow::err(format!(
                "{} type has no field '.{}'",
                other.type_name(),
                name
            ))),
        }
    }

    pub(crate) fn get_index(&self, t: &Value, k: &Value) -> EvalResult {
        match (t, k) {
            (Value::List(xs), Value::Int(i)) => {
                let idx = *i;
                if idx < 0 || idx as usize >= xs.len() {
                    Ok(Value::Nil)
                } else {
                    Ok(xs[idx as usize].clone())
                }
            }
            // Reading a ctx key, consistent with get_field — we return a snapshot Map.
            (Value::Map(m), Value::Str(key)) | (Value::Map(m), Value::Sym(key)) => {
                match m.get(key) {
                    Some(Value::Ctx(cell)) => Ok(Value::Map(cell.lock().unwrap().clone())),
                    other => Ok(other.cloned().unwrap_or(Value::Nil)),
                }
            }
            (Value::Nil, _) => Ok(Value::Nil),
            (t, k) => Err(Flow::err(format!(
                "{}[{}] indexing is not supported",
                t.type_name(),
                k.type_name()
            ))),
        }
    }
}
