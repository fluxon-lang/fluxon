// ---------------- argument helpers ----------------
use std::sync::Arc;

use crate::interp::Flow;
use crate::value::Value;

pub(crate) fn arg<'a>(args: &'a [Value], i: usize, who: &str) -> Result<&'a Value, Flow> {
    args.get(i)
        .ok_or_else(|| Flow::err(format!("{}: argument {} is missing", who, i + 1)))
}
pub(crate) fn arg_str(args: &[Value], i: usize, who: &str) -> Result<String, Flow> {
    match arg(args, i, who)? {
        Value::Str(s) => Ok(s.clone()),
        Value::Sym(s) => Ok(s.clone()),
        other => Err(Flow::err(format!(
            "{}: argument {} must be str, got {}",
            who,
            i + 1,
            other.type_name()
        ))),
    }
}
// Reads a binary argument. str/sym are also accepted (their UTF-8 bytes) — so
// consumers like crypto accept text and bytes through a single path (the AI does
// not have to learn two separate function names).
pub(crate) fn arg_bytes(args: &[Value], i: usize, who: &str) -> Result<Arc<Vec<u8>>, Flow> {
    match arg(args, i, who)? {
        Value::Bytes(b) => Ok(b.clone()),
        Value::Str(s) | Value::Sym(s) => Ok(Arc::new(s.clone().into_bytes())),
        other => Err(Flow::err(format!(
            "{}: argument {} must be bytes or str, got {}",
            who,
            i + 1,
            other.type_name()
        ))),
    }
}
pub(crate) fn arg_int(args: &[Value], i: usize, who: &str) -> Result<i64, Flow> {
    match arg(args, i, who)? {
        Value::Int(n) => Ok(*n),
        other => Err(Flow::err(format!(
            "{}: argument {} must be int, got {}",
            who,
            i + 1,
            other.type_name()
        ))),
    }
}
// Reads a timestamp argument into unix seconds: text (ISO/canonical, zone too) or
// a direct unix int. So time.fmt/add/diff accept the same input.
pub(crate) fn arg_ts(args: &[Value], i: usize, who: &str) -> Result<i64, Flow> {
    match arg(args, i, who)? {
        Value::Str(s) => crate::builtins::time_mod::parse_iso(s)
            .ok_or_else(|| Flow::err(format!("{}: could not parse timestamp text: {}", who, s))),
        Value::Int(n) => Ok(*n),
        other => Err(Flow::err(format!(
            "{}: argument {} must be timestamp (str/int), got {}",
            who,
            i + 1,
            other.type_name()
        ))),
    }
}
pub(crate) fn arg_num(args: &[Value], i: usize, who: &str) -> Result<f64, Flow> {
    match arg(args, i, who)? {
        Value::Int(n) => Ok(*n as f64),
        Value::Flt(x) => Ok(*x),
        other => Err(Flow::err(format!(
            "{}: argument {} must be a number, got {}",
            who,
            i + 1,
            other.type_name()
        ))),
    }
}
