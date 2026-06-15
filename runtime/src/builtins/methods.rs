// ---------------- value methods (list/map) ----------------
use std::collections::BTreeMap;

use crate::builtins::R;
use crate::builtins::args::*;
use crate::interp::Flow;
use crate::value::Value;

pub fn call_method(recv: &Value, method: &str, args: Vec<Value>) -> R {
    match recv {
        Value::List(xs) => list_method(xs, method, args),
        Value::Map(m) => map_method(m, method, args),
        Value::Str(_) => Err(Flow::err(format!(
            "str methods are called via the module: str.{} s (not a value method)",
            method
        ))),
        other => Err(Flow::err(format!(
            "{} type has no '.{}' method",
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
        "index" => {
            // Index of the first matching element; -1 if not found (unlike a bool,
            // index gives a position — paired with list.has).
            let needle = arg(&args, 0, "list.index")?;
            let i = xs
                .iter()
                .position(|v| v.equals(needle))
                .map(|i| i as i64)
                .unwrap_or(-1);
            Ok(Value::Int(i))
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
        // Argument-less sort — natural order (number/string). The comparator
        // variant takes a function argument, so it comes through list_hof in interp.
        "sort" => sort_default(xs),
        "reverse" => {
            let mut new = xs.to_vec();
            new.reverse();
            Ok(Value::List(new))
        }
        "uniq" => {
            // The first occurrence is kept (order is preserved). Value has no hash,
            // so a linear search with equals — lists are small.
            let mut out: Vec<Value> = Vec::new();
            for x in xs {
                if !out.iter().any(|v| v.equals(x)) {
                    out.push(x.clone());
                }
            }
            Ok(Value::List(out))
        }
        "flat" => {
            // Flattens one level: inner list elements are unwrapped, the rest stay
            // as-is — chain flat if deep recursion is needed.
            let mut out = Vec::new();
            for x in xs {
                match x {
                    Value::List(inner) => out.extend(inner.iter().cloned()),
                    other => out.push(other.clone()),
                }
            }
            Ok(Value::List(out))
        }
        "zip" => {
            let other = arg(&args, 0, "list.zip")?;
            let Value::List(ys) = other else {
                return Err(Flow::err(format!(
                    "list.zip: argument must be a list, got {}",
                    other.type_name()
                )));
            };
            // Stops when the shorter one ends — extra elements are dropped.
            Ok(Value::List(
                xs.iter()
                    .zip(ys)
                    .map(|(a, b)| Value::List(vec![a.clone(), b.clone()]))
                    .collect(),
            ))
        }
        // filter/map/reduce/find/any/all — take a function argument; interp cannot
        // call it here (apply lives in Interp). So these methods need special
        // handling — see the note below.
        "filter" | "map" | "reduce" | "find" | "any" | "all" => Err(Flow::err(format!(
            "internal: list.{} is handled via a separate path",
            method
        ))),
        _ => Err(Flow::err(format!(
            "list method '{}' does not exist",
            method
        ))),
    }
}

// Sort in natural order: works when numbers (mixed int/flt) and strings/syms are
// homogeneous; mixed types require providing a comparator.
pub fn sort_default(xs: &[Value]) -> R {
    let sorted = sort_values(xs.to_vec(), &mut |a, b| {
        use std::cmp::Ordering;
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => Ok(x.cmp(y)),
            // NaN is unordered — treat as Equal (so the sort does not break).
            (Value::Flt(x), Value::Flt(y)) => Ok(x.partial_cmp(y).unwrap_or(Ordering::Equal)),
            (Value::Int(x), Value::Flt(y)) => {
                Ok((*x as f64).partial_cmp(y).unwrap_or(Ordering::Equal))
            }
            (Value::Flt(x), Value::Int(y)) => {
                Ok(x.partial_cmp(&(*y as f64)).unwrap_or(Ordering::Equal))
            }
            (Value::Str(x), Value::Str(y)) => Ok(x.cmp(y)),
            (Value::Sym(x), Value::Sym(y)) => Ok(x.cmp(y)),
            (a, b) => Err(Flow::err(format!(
                "list.sort: cannot compare {} and {} — provide a comparator: l.sort \\a b -> ...",
                a.type_name(),
                b.type_name()
            ))),
        }
    })?;
    Ok(Value::List(sorted))
}

// Stable merge sort — instead of std sort_by, because when the comparator is a
// Fluxon function it may return an error (Flow): if we returned Equal on the error
// path, std sort might panic with "total order broken". This path propagates the
// error cleanly.
pub fn sort_values<F>(mut xs: Vec<Value>, cmp: &mut F) -> Result<Vec<Value>, Flow>
where
    F: FnMut(&Value, &Value) -> Result<std::cmp::Ordering, Flow>,
{
    let len = xs.len();
    if len <= 1 {
        return Ok(xs);
    }
    let right = xs.split_off(len / 2);
    let left = sort_values(xs, cmp)?;
    let right = sort_values(right, cmp)?;
    let mut out = Vec::with_capacity(len);
    let mut li = left.into_iter().peekable();
    let mut ri = right.into_iter().peekable();
    loop {
        match (li.peek(), ri.peek()) {
            // On a tie the left (earlier in the original order) goes first — stable.
            (Some(a), Some(b)) => {
                if cmp(a, b)? == std::cmp::Ordering::Greater {
                    out.push(ri.next().unwrap());
                } else {
                    out.push(li.next().unwrap());
                }
            }
            (Some(_), None) => out.push(li.next().unwrap()),
            (None, Some(_)) => out.push(ri.next().unwrap()),
            (None, None) => break,
        }
    }
    Ok(out)
}

fn map_method(m: &BTreeMap<String, Value>, method: &str, args: Vec<Value>) -> R {
    match method {
        "len" => Ok(Value::Int(m.len() as i64)),
        "has" => {
            let k = key_of(arg(&args, 0, "map.has")?);
            Ok(Value::Bool(m.contains_key(&k)))
        }
        "keys" => Ok(Value::List(
            m.keys().map(|k| Value::Str(k.clone())).collect(),
        )),
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
        "merge" => {
            // Keys in `other` take precedence (consistent with set semantics: the
            // later write wins) — for the default config + override pattern.
            let other = match arg(&args, 0, "map.merge")? {
                Value::Map(o) => o.clone(),
                other => {
                    return Err(Flow::err(format!(
                        "map.merge: argument must be a map, got {}",
                        other.type_name()
                    )));
                }
            };
            let mut new = m.clone();
            new.extend(other);
            Ok(Value::Map(new))
        }
        _ => Err(Flow::err(format!("map method '{}' does not exist", method))),
    }
}

fn key_of(v: &Value) -> String {
    match v {
        Value::Str(s) | Value::Sym(s) => s.clone(),
        other => format!("{}", other),
    }
}
