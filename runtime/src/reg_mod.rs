// Fluxon reg battery — function registry (dynamic dispatch).
//
// Lets you store/call a function by its STRING name. The main use is AI agent
// tool-loops: the model picks "which tool" by string name, and the code runs it via
// `reg.call name args` (NOT a `match`-switch — tools are added at runtime).
//
// Language API (docs/fluxon-agent.md):
//   reg.add "calc" \args -> args.a + args.b   # register by name
//   out = reg.call "calc" {a:2 b:3}           # call by name -> 5
//   reg.has "calc"                            # bool
//   reg.names                                 # the registered names (list)
//
// The stored value is a Value::Fn (closure) or a Value::Native. Since the closure is
// declared at top-level it holds `Parent::Root` -> even after `http.serve`/`ws.serve`
// has frozen (on another thread) it reads the frozen globals correctly. Since Value is
// Send+Sync, the registry is safe across threads.

use std::collections::HashMap;

use parking_lot::Mutex;

use crate::interp::{Flow, Interp};
use crate::value::Value;

// reg battery state — one per process (an Arc inside Interp). Top-level code fills it
// with `reg.add`; `reg.call` reads it from any thread (including inside an http/ws
// handler). Same model as http `routes`/ws `WsState`.
pub struct RegState {
    // name -> function (Value::Fn or Value::Native).
    fns: Mutex<HashMap<String, Value>>,
}

impl RegState {
    pub fn new() -> Self {
        RegState {
            fns: Mutex::new(HashMap::new()),
        }
    }
}

impl Interp {
    // reg.* dispatch — `eval_call` routes Field{Ident("reg"), name} here. `reg.names`
    // also arrives without arguments (a Field, not a Call) — it too lands in this
    // function (with an empty argv).
    pub fn reg_dispatch(&self, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            // reg.add "name" fn — stores the function under the name. Returns: nil.
            // Overwrites an existing name (re-registration — updating a tool).
            "add" => {
                let mut it = args.into_iter();
                let name = match it.next() {
                    Some(Value::Str(s)) => s,
                    Some(other) => {
                        return Err(Flow::err(format!(
                            "reg.add: first argument must be a name (str), got {}",
                            other.type_name()
                        )));
                    }
                    None => return Err(Flow::err("reg.add: name and function arguments required")),
                };
                let f = match it.next() {
                    Some(f @ (Value::Fn(_) | Value::Native(_))) => f,
                    Some(other) => {
                        return Err(Flow::err(format!(
                            "reg.add: second argument must be a function, got {}",
                            other.type_name()
                        )));
                    }
                    None => return Err(Flow::err("reg.add: function argument required")),
                };
                self.reg.fns.lock().insert(name, f);
                Ok(Value::Nil)
            }
            // reg.call "name" args -> calls the function by name and returns its result.
            // Fails if the name is not found. We call the function OUTSIDE the lock (apply
            // can run long and may re-enter reg -> so no deadlock).
            "call" => {
                let mut it = args.into_iter();
                let name = match it.next() {
                    Some(Value::Str(s)) => s,
                    Some(other) => {
                        return Err(Flow::err(format!(
                            "reg.call: first argument must be a name (str), got {}",
                            other.type_name()
                        )));
                    }
                    None => return Err(Flow::err("reg.call: name argument required")),
                };
                let f = match self.reg.fns.lock().get(&name) {
                    Some(f) => f.clone(),
                    None => return Err(Flow::err(format!("reg.call: '{}' not registered", name))),
                };
                let rest: Vec<Value> = it.collect();
                self.apply(f, rest)
            }
            // reg.has "name" -> bool. Whether the name is registered.
            "has" => {
                let name = match args.into_iter().next() {
                    Some(Value::Str(s)) => s,
                    Some(other) => {
                        return Err(Flow::err(format!(
                            "reg.has: argument must be a name (str), got {}",
                            other.type_name()
                        )));
                    }
                    None => return Err(Flow::err("reg.has: name argument required")),
                };
                Ok(Value::Bool(self.reg.fns.lock().contains_key(&name)))
            }
            // reg.names -> the registered names (list, alphabetical — stable output).
            "names" => {
                let mut names: Vec<String> = self.reg.fns.lock().keys().cloned().collect();
                names.sort();
                Ok(Value::List(names.into_iter().map(Value::Str).collect()))
            }
            other => Err(Flow::err(format!("reg.{} function does not exist", other))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::NativeFn;
    use std::sync::Arc;

    // Value/Flow do not derive Debug (unwrap/{:?} cannot be used) — we match the result
    // by hand. The helpers below make that concise.

    // Test helper: a native function that returns the given result.
    fn native_const(name: &str, v: i64) -> Value {
        let name = name.to_string();
        Value::Native(Arc::new(NativeFn {
            name,
            func: Box::new(move |_args| Ok(Value::Int(v))),
        }))
    }

    // Extracts the Flow error text (panics if Ok).
    fn err_msg(r: Result<Value, Flow>) -> String {
        match r {
            Err(Flow::Error(m)) | Err(Flow::Fail { message: m, .. }) => m,
            Err(_) => panic!("unexpected flow kind"),
            Ok(_) => panic!("expected an error, got Ok"),
        }
    }

    #[test]
    fn add_then_has_and_call() {
        let it = Interp::new();
        assert!(
            it.reg_dispatch("add", vec![Value::Str("x".into()), native_const("x", 42)])
                .is_ok()
        );
        // has -> true
        assert!(matches!(
            it.reg_dispatch("has", vec![Value::Str("x".into())]),
            Ok(Value::Bool(true))
        ));
        // call -> 42
        assert!(matches!(
            it.reg_dispatch("call", vec![Value::Str("x".into())]),
            Ok(Value::Int(42))
        ));
    }

    #[test]
    fn has_unknown_is_false() {
        let it = Interp::new();
        assert!(matches!(
            it.reg_dispatch("has", vec![Value::Str("yoq".into())]),
            Ok(Value::Bool(false))
        ));
    }

    #[test]
    fn call_unknown_errors() {
        let it = Interp::new();
        let m = err_msg(it.reg_dispatch("call", vec![Value::Str("yoq".into())]));
        assert!(m.contains("not registered"), "text: {m}");
    }

    #[test]
    fn names_sorted() {
        let it = Interp::new();
        assert!(
            it.reg_dispatch("add", vec![Value::Str("b".into()), native_const("b", 1)])
                .is_ok()
        );
        assert!(
            it.reg_dispatch("add", vec![Value::Str("a".into()), native_const("a", 2)])
                .is_ok()
        );
        match it.reg_dispatch("names", vec![]) {
            Ok(Value::List(xs)) => {
                let got: Vec<String> = xs
                    .iter()
                    .map(|v| match v {
                        Value::Str(s) => s.clone(),
                        _ => panic!("name is not a str"),
                    })
                    .collect();
                assert_eq!(got, vec!["a".to_string(), "b".to_string()]);
            }
            _ => panic!("names did not return a list"),
        }
    }

    #[test]
    fn add_rejects_non_fn() {
        let it = Interp::new();
        let m = err_msg(it.reg_dispatch("add", vec![Value::Str("x".into()), Value::Int(5)]));
        assert!(m.contains("function"), "text: {m}");
    }
}
