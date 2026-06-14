// par — a language-level parallel fan-out primitive (issue #137).
//
// `par [\-> ai.ask p1  \-> http.get u2  \-> db.one "..." [id]]` takes a list of
// lambdas, calls EACH on a separate thread, and waits for all of them to finish. The
// result is a list in input order; each element is `{ok: value}` (lambda succeeded) or
// `{err: message}` (lambda raised `fail`/an error). If one lambda fails the rest do not
// stop (partial success: 2 of 3 APIs worked — the caller inspects each result).
//
// Why threads (not tokio): `par` is called in a blocking context (top-level code or
// inside an HTTP/WS handler, which are already on their own thread). `std::thread` +
// `join` is the simplest and correct model — it fits the synchronous path like a
// handler. `Value` and `Flow` are Send (invariant), and `Arc<Interp>` is shared across
// threads (like cron/queue). The lambda closures hold `Parent::Scope(env)` — an
// `Arc<RwLock<Scope>>`, so parallel reads (lookup) are safe; `<-` writes are serialized
// by the RwLock write.

use std::sync::Arc;

use crate::interp::{Flow, Interp};
use crate::value::Value;

impl Interp {
    // par [\-> ... \-> ...] — a single argument: a list of lambdas.
    pub fn par_run(self: &Arc<Self>, args: Vec<Value>) -> Result<Value, Flow> {
        let mut it = args.into_iter();
        let list = match it.next() {
            Some(Value::List(xs)) => xs,
            Some(other) => {
                return Err(Flow::err(format!(
                    "par: argument must be a list of lambdas, got {}",
                    other.type_name()
                )));
            }
            None => return Err(Flow::err("par: a list of lambdas argument is required")),
        };
        if it.next().is_some() {
            return Err(Flow::err(
                "par: only one argument (a list of lambdas) is expected",
            ));
        }
        // Check UP FRONT that each element is a callable function (a 0-arity lambda or an
        // fn value) — so we give a clear error before any thread is spawned.
        for (i, v) in list.iter().enumerate() {
            if !matches!(v, Value::Fn(_) | Value::Native(_)) {
                return Err(Flow::err(format!(
                    "par: element {} must be a function (\\-> ...), got {}",
                    i,
                    v.type_name()
                )));
            }
        }

        // Empty list — empty result (no threads spawned).
        if list.is_empty() {
            return Ok(Value::List(Vec::new()));
        }

        // par CANNOT be called inside db.tx: the transaction lives in the current
        // thread's `CURRENT_TX` TLS and new threads do not inherit it — par
        // lambdas would silently run OUTSIDE the transaction (against the global
        // DB, not seeing uncommitted changes), breaking db.tx atomicity /
        // read-your-writes semantics. SQLite connections are not thread-safe, so
        // the tx cannot be shared either. Instead of working silently wrong we
        // give an explicit error (issue #137 PR review, P1).
        if crate::db_mod::tx_active() {
            return Err(Flow::err(
                "par: cannot be used inside db.tx (the transaction is not shared across threads); call it outside the tx",
            ));
        }

        // We call each lambda on a separate thread. The thread holds a clone of
        // `Arc<Interp>` (not weak — guaranteed alive for the duration of the
        // call). `current_base` is thread-local and a new thread starts from the
        // default CWD — so we SNAPSHOT the current base and pass it to the lambda
        // thread, otherwise a `use ./...` inside the lambda would resolve against
        // the wrong directory (when par is called from inside a module). The
        // cycle-detection stack, by contrast, starts empty on the new thread —
        // which is correct (each lambda is an independent chain).
        let base = self.base_dir();
        let handles: Vec<_> = list
            .into_iter()
            .map(|f| {
                let interp = self.clone();
                let base = base.clone();
                std::thread::spawn(move || {
                    interp.set_base(&base);
                    interp.apply(f, Vec::new())
                })
            })
            .collect();

        // We join in input order — the result list matches the lambda order.
        let mut out = Vec::with_capacity(handles.len());
        for h in handles {
            let cell = match h.join() {
                // The lambda returned successfully or with a Flow error.
                Ok(res) => flow_to_cell(res),
                // The thread panicked (an unexpected panic inside apply) — error cell.
                Err(_) => err_cell("par: an inner thread panicked"),
            };
            out.push(cell);
        }
        Ok(Value::List(out))
    }
}

// Turns the `apply` result into an `{ok:...}` or `{err:...}` map. `apply` has
// already normalized `Flow::Return` to `Ok`; only a real result or
// `Fail`/`Error`/`Skip`/`Stop` reaches here. `skip`/`stop` are meaningless
// inside a lambda (no loop) — we mark them as errors too.
fn flow_to_cell(res: Result<Value, Flow>) -> Value {
    match res {
        Ok(v) => ok_cell(v),
        Err(Flow::Fail { message, .. }) | Err(Flow::Error(message)) => err_cell(message),
        Err(Flow::Skip) => err_cell("par: `skip` inside a lambda (no loop)"),
        Err(Flow::Stop) => err_cell("par: `stop` inside a lambda (no loop)"),
        // Return is converted to Ok by apply — this arm is unreachable, but for
        // completeness we wrap its value in an ok cell.
        Err(Flow::Return(v)) => ok_cell(v),
    }
}

fn ok_cell(v: Value) -> Value {
    let mut m = std::collections::BTreeMap::new();
    m.insert("ok".to_string(), v);
    Value::Map(m)
}

fn err_cell(msg: impl Into<String>) -> Value {
    let mut m = std::collections::BTreeMap::new();
    m.insert("err".to_string(), Value::Str(msg.into()));
    Value::Map(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    // flow_to_cell: Ok -> {ok:v}, Fail/Error -> {err:msg}, skip/stop -> {err:...}.
    fn cell_kind(v: &Value) -> &'static str {
        match v {
            Value::Map(m) if m.contains_key("ok") => "ok",
            Value::Map(m) if m.contains_key("err") => "err",
            _ => "other",
        }
    }

    #[test]
    fn ok_natija_ok_cell() {
        assert_eq!(cell_kind(&flow_to_cell(Ok(Value::Int(5)))), "ok");
    }

    #[test]
    fn fail_xato_err_cell() {
        let c = flow_to_cell(Err(Flow::Fail {
            status: Some(422),
            message: "nope".into(),
        }));
        assert_eq!(cell_kind(&c), "err");
        if let Value::Map(m) = &c {
            assert!(matches!(m.get("err"), Some(Value::Str(s)) if s == "nope"));
        }
    }

    #[test]
    fn runtime_xato_err_cell() {
        assert_eq!(cell_kind(&flow_to_cell(Err(Flow::err("broke")))), "err");
    }

    // skip/stop inside a lambda — no loop, so error cell.
    #[test]
    fn skip_stop_err_cell() {
        assert_eq!(cell_kind(&flow_to_cell(Err(Flow::Skip))), "err");
        assert_eq!(cell_kind(&flow_to_cell(Err(Flow::Stop))), "err");
    }
}
