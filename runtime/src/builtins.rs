// Fluxon core library (the part without batteries).
//
// Three kinds of call:
//   - global functions (log) — installed into Env (`install`)
//   - module functions (str.up, math.floor, rand.int, json) — `call_module`
//   - value methods (l.push, m.set, not s.up...) — `call_method`
//
// list methods act on the value (.push/.filter), str/math/rand go through the
// module — this mirrors the spec distinction exactly: `l.len` (member) vs
// `str.len s` (module).

mod args;
mod bytes_mod;
mod fs_mod;
mod io_mod;
mod json_mod;
mod math_mod;
mod methods;
mod rand_mod;
mod sh_mod;
mod str_mod;
mod time_mod;
mod tui_mod;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::interp::{Env, Flow};
use crate::value::{NativeFn, Value};

use bytes_mod::bytes_module;
use fs_mod::fs_module;
use io_mod::io_module;
use json_mod::{json_module, json_str};
use math_mod::math_module;
use rand_mod::rand_module;
use sh_mod::sh_module;
use str_mod::str_module;
use time_mod::{fmt_unix, now_unix, time_module};
use tui_mod::tui_module;

// json_encode/json_decode are part of the public surface used by other modules
// (http/db/ai call them as `crate::builtins::json_encode`) — re-exported here so
// those external call sites keep working after the split.
pub use json_mod::{json_decode, json_encode};
// call_method/sort_default/sort_values are called from interp as
// `crate::builtins::X` — re-exported from the methods submodule.
pub use methods::{call_method, sort_default, sort_values};
// arg_str/arg_bytes are used by the crypto battery (`crate::builtins::arg_str`) —
// re-exported from the args submodule so that call site keeps working.
pub(crate) use args::{arg_bytes, arg_str};

pub(crate) type R = Result<Value, Flow>;

// The "N assert" count in the `fluxon test` report. Atomic — http.serve handlers
// may call assert from parallel threads. The test runner resets it before each
// file (files run sequentially, in a single process).
static ASSERT_PASSED: AtomicU64 = AtomicU64::new(0);

pub fn assert_passed_reset() {
    ASSERT_PASSED.store(0, Ordering::Relaxed);
}

pub fn assert_passed() -> u64 {
    ASSERT_PASSED.load(Ordering::Relaxed)
}

// --- install global functions ---
pub fn install(env: &Env) {
    let mut s = env.write();
    let mut add = |name: &str, f: Box<dyn Fn(Vec<Value>) -> R + Send + Sync>| {
        s.set_global(
            name,
            Value::Native(Arc::new(NativeFn {
                name: name.into(),
                func: f,
            })),
        );
    };
    // `log` is NOT a global function — it is a leveled dispatch battery (issue #139):
    // `log.debug`/`log.info`/`log.warn`/`log.err`, bare `log` = info. Because it
    // needs to read $LOG_LEVEL and $LOG_FORMAT it is handled in Interp (log_dispatch),
    // not installed as a global here (same pattern as ai/crypto).
    // assert cond ["message"] — test primitive (issue #136). If the condition is
    // truthy it continues silently (counter +1), otherwise a runtime error —
    // execution stops and `fluxon test` marks the file as FAIL. A condition with
    // operators is written in parens (the bare-call rule): `assert (x == 2) "x is not two"`.
    add(
        "assert",
        Box::new(|args: Vec<Value>| {
            let cond = match args.first() {
                Some(v) => v,
                None => return Err(Flow::err("assert: condition argument required")),
            };
            if cond.truthy() {
                ASSERT_PASSED.fetch_add(1, Ordering::Relaxed);
                return Ok(Value::Nil);
            }
            Err(Flow::err(match args.get(1) {
                Some(msg) => format!("assert failed: {}", msg.to_text()),
                None => "assert failed".to_string(),
            }))
        }),
    );
    // rep status body [headers] — an HTTP response. To avoid adding a new Value
    // variant it is represented as a map with special keys:
    // {__resp:true status:N body:V headers:{...}}. http_mod::value_to_response
    // recognizes these keys.
    //
    // Optional 3rd argument — a map of custom headers (issue #16). It is a
    // separate 3rd arg rather than living in the body so it does not collide with
    // the body: in `rep 200 {ok}` the whole map is the body, so a header could not
    // be read out of the body. A header value can be a str (single header) or a
    // list (repeated header, e.g. multiple Set-Cookie).
    add(
        "rep",
        Box::new(|args: Vec<Value>| {
            let status = match args.first() {
                Some(Value::Int(n)) => *n,
                Some(other) => {
                    return Err(Flow::err(format!(
                        "rep: 1st argument must be status (int), got {}",
                        other.type_name()
                    )));
                }
                None => return Err(Flow::err("rep: status argument required")),
            };
            let body = args.get(1).cloned().unwrap_or(Value::Nil);
            let mut m = BTreeMap::new();
            m.insert("__resp".to_string(), Value::Bool(true));
            m.insert("status".to_string(), Value::Int(status));
            m.insert("body".to_string(), body);
            // If a 3rd argument is present — the headers map. If it is not a map
            // we return an explicit error, because silently ignoring it is
            // misleading for the AI.
            if let Some(h) = args.get(2) {
                match h {
                    Value::Map(_) => {
                        m.insert("headers".to_string(), h.clone());
                    }
                    other => {
                        return Err(Flow::err(format!(
                            "rep: 3rd argument must be headers (map), got {}",
                            other.type_name()
                        )));
                    }
                }
            }
            Ok(Value::Map(m))
        }),
    );
}

// ---------------- log levels (issue #139) ----------------
//
// `log` is a leveled logger: `log.debug`/`log.info`/`log.warn`/`log.err`, bare
// `log` = info. Levels are ordered (debug < info < warn < err) — $LOG_LEVEL sets
// the minimum level, messages BELOW it are silently dropped. With $LOG_FORMAT
// =json each line is a JSON object (time/level/msg) — for log aggregators.
//
// Interp reads the envs (OS env + .env, the db/ai convention) and passes them
// here; this function is stateless and pure formatting/filter logic.

// Level order (smaller = lower). An unknown name is treated as info (safe default).
fn log_level_rank(name: &str) -> u8 {
    match name {
        "debug" => 0,
        "info" => 1,
        "warn" => 2,
        // `err` is canonical, `error` is also accepted (a human may write it in LOG_LEVEL).
        "err" | "error" => 3,
        _ => 1,
    }
}

// Formats a log line. If `min_level` (=$LOG_LEVEL) is given and the message level
// is below it, returns None (filtered — nothing is emitted). With `json`
// (=$LOG_FORMAT=json) a structured line, otherwise `[LEVEL] message`.
pub fn format_log(
    level: &str,
    args: &[Value],
    min_level: Option<&str>,
    json: bool,
) -> Option<String> {
    if let Some(min) = min_level
        && log_level_rank(level) < log_level_rank(min)
    {
        return None;
    }
    let msg: String = args
        .iter()
        .map(|v| v.to_text())
        .collect::<Vec<_>>()
        .join(" ");
    if json {
        // Time is UTC, same format as time.now. json_str escapes correctly —
        // quotes/newlines inside the message do not break the JSON.
        Some(format!(
            "{{\"time\":{},\"level\":{},\"msg\":{}}}",
            json_str(&fmt_unix(now_unix())),
            json_str(level),
            json_str(&msg)
        ))
    } else {
        Some(format!("[{}] {}", level.to_uppercase(), msg))
    }
}

// Emits the format_log result to stderr (silent if filtered). Like the old `log`
// it appends `\n` to stderr — so it does not mix with stdout (io.print/prompt).
pub fn emit_log(level: &str, args: &[Value], min_level: Option<&str>, json: bool) {
    if let Some(line) = format_log(level, args, min_level, json) {
        eprintln!("{}", line);
    }
}

// The info-level Native shim returned when `log` is used as a value (callback
// `xs.each log`, storing `f log`) — for compatibility with the old global `log`
// (issue #139). A direct `log "..."` call goes through the env-aware log_dispatch
// in apply_callee; this shim is only for the value position and runs without
// Interp, so it reads $LOG_LEVEL/$LOG_FORMAT from the OS env (.env is not seen on
// this path — using log as a value is a rare case).
pub fn log_value_shim() -> Value {
    Value::Native(Arc::new(NativeFn {
        name: "log".into(),
        func: Box::new(|args: Vec<Value>| {
            let min = std::env::var("LOG_LEVEL").ok();
            let json = std::env::var("LOG_FORMAT")
                .map(|s| s.eq_ignore_ascii_case("json"))
                .unwrap_or(false);
            emit_log("info", &args, min.as_deref(), json);
            Ok(Value::Nil)
        }),
    }))
}

// --- is it a module name? ---
pub fn is_module(name: &str) -> bool {
    matches!(
        name,
        "str" | "math" | "rand" | "json" | "time" | "io" | "fs" | "sh" | "bytes" | "tui"
    )
}

// --- module function call ---
pub fn call_module(module: &str, func: &str, args: Vec<Value>) -> R {
    match module {
        "str" => str_module(func, args),
        "math" => math_module(func, args),
        "rand" => rand_module(func, args),
        "json" => json_module(func, args),
        "time" => time_module(func, args),
        "io" => io_module(func, args),
        "fs" => fs_module(func, args),
        "sh" => sh_module(func, args),
        "bytes" => bytes_module(func, args),
        "tui" => tui_module(func, args),
        _ => Err(Flow::err(format!("unknown module: {}", module))),
    }
}

#[cfg(test)]
mod log_tests {
    use super::*;

    fn s(x: &str) -> Value {
        Value::Str(x.to_string())
    }

    // No level (no filter) — every message comes out as `[LEVEL] text`.
    #[test]
    fn text_format_prefiks() {
        assert_eq!(
            format_log("info", &[s("hello")], None, false),
            Some("[INFO] hello".to_string())
        );
        assert_eq!(
            format_log("err", &[s("failed")], None, false),
            Some("[ERR] failed".to_string())
        );
    }

    // Multiple arguments are joined with a space (the old `log` behavior).
    #[test]
    fn text_multi_arg() {
        assert_eq!(
            format_log("warn", &[s("a"), Value::Int(2), s("b")], None, false),
            Some("[WARN] a 2 b".to_string())
        );
    }

    // $LOG_LEVEL filter: messages BELOW the minimum level are None (silent).
    #[test]
    fn level_filter() {
        // min=warn -> debug/info silent, warn/err emitted.
        assert_eq!(format_log("debug", &[s("x")], Some("warn"), false), None);
        assert_eq!(format_log("info", &[s("x")], Some("warn"), false), None);
        assert!(format_log("warn", &[s("x")], Some("warn"), false).is_some());
        assert!(format_log("err", &[s("x")], Some("warn"), false).is_some());
    }

    // `error` is ordered like `err` (a human may write it in LOG_LEVEL).
    #[test]
    fn error_alias() {
        assert_eq!(format_log("warn", &[s("x")], Some("error"), false), None);
        assert!(format_log("err", &[s("x")], Some("error"), false).is_some());
    }

    // An unknown LOG_LEVEL is treated as info — debug is filtered, info passes.
    #[test]
    fn unknown_min_level_info() {
        assert_eq!(format_log("debug", &[s("x")], Some("qqq"), false), None);
        assert!(format_log("info", &[s("x")], Some("qqq"), false).is_some());
    }

    // JSON mode: a structured line, level and message correct, quotes escaped.
    #[test]
    fn json_format() {
        let line = format_log("warn", &[s("failed")], None, true).unwrap();
        assert!(
            line.starts_with("{\"time\":\""),
            "time field missing: {}",
            line
        );
        assert!(
            line.contains("\"level\":\"warn\""),
            "level missing: {}",
            line
        );
        assert!(
            line.contains("\"msg\":\"failed\""),
            "message missing: {}",
            line
        );
        // Read back via the decoder — confirms it is valid JSON.
        let Ok(Value::Map(m)) = json_decode(&line) else {
            panic!("invalid JSON: {}", line);
        };
        assert!(matches!(m.get("level"), Some(Value::Str(s)) if s == "warn"));
        assert!(matches!(m.get("msg"), Some(Value::Str(s)) if s == "failed"));
    }

    // A quote/newline inside the JSON message does not break the JSON (escape).
    #[test]
    fn json_escapes_message() {
        let line = format_log("info", &[s("a\"b\nc")], None, true).unwrap();
        assert!(json_decode(&line).is_ok(), "escape broken: {}", line);
    }

    // The filter works in JSON mode too.
    #[test]
    fn json_respects_filter() {
        assert_eq!(format_log("debug", &[s("x")], Some("info"), true), None);
    }
}
