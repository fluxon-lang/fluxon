// Free helper functions: migration error-swallowing, `.env` parsing, and the
// arithmetic primitives used by `binary_values`. These are pure (no `&self`),
// so they live here rather than in any `impl Interp` block.

use std::collections::HashMap;

use crate::ast::*;
use crate::value::Value;

use super::scope::{EvalResult, Flow};

// Swallows an ADD/DROP COLUMN error in the "already present/absent" case (SQLite
// does not support IF [NOT] EXISTS for these DDLs). For idempotency: if the
// column already exists (user-added / the new side of a rename) or is already
// absent (user-removed / the old side of a rename) — the migration does not
// fail. ALL other errors (e.g. syntax, type) are raised.
pub(crate) fn swallow_benign(res: Result<usize, String>) -> Result<(), Flow> {
    match res {
        Ok(_) => Ok(()),
        Err(msg) => {
            let m = msg.to_lowercase();
            if m.contains("duplicate column name") || m.contains("no such column") {
                Ok(()) // already in the desired state — pass quietly
            } else {
                Err(Flow::err(msg))
            }
        }
    }
}

// Is the `use` path a user file or a battery? User modules are given as a
// relative path (`./tools`, `../lib/x`). Batteries are a plain name (`http`,
// `db`) — they dispatch by name and no file is loaded.
pub(crate) fn is_user_module_path(path: &str) -> bool {
    path.starts_with("./") || path.starts_with("../") || path == "." || path == ".."
}

// Derives the binding name from a module path: the last segment, without `.fx`.
// `./lib/greet` -> `greet`, `./tools` -> `tools`.
pub(crate) fn module_basename(path: &str) -> String {
    let last = path.rsplit('/').next().unwrap_or(path);
    last.strip_suffix(".fx").unwrap_or(last).to_string()
}

// Collects the top-level names exported from a module program: `exp NAME = ...`
// and `exp fn NAME`. Only these enter the namespace — the remaining `=`/`fn` are
// module-private.
pub(crate) fn collect_exported(prog: &Program) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    for stmt in prog {
        match stmt {
            Stmt::ExpBind { name, .. } => {
                set.insert(name.clone());
            }
            Stmt::FnDecl {
                name,
                exported: true,
                ..
            } => {
                set.insert(name.clone());
            }
            _ => {}
        }
    }
    set
}

// Reads and parses the `.env` file in the current directory. If the file is
// absent or unreadable — an empty map (not an error; .env is optional). Format:
//   KEY=VALUE        # comment
//   export KEY=VALUE   (the export prefix is ignored)
//   KEY="value"  /  KEY='value'   (the surrounding quote/apostrophe is stripped)
// Empty lines and lines starting with `#` are dropped.
pub(crate) fn load_dotenv() -> HashMap<String, String> {
    match std::fs::read_to_string(".env") {
        Ok(c) => parse_dotenv(&c),
        Err(_) => HashMap::new(), // no .env -> empty (optional)
    }
}

// .env text -> map. Split out from load_dotenv (a pure, testable function).
fn parse_dotenv(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // `export KEY=VAL` -> `KEY=VAL`
        let line = line.strip_prefix("export ").map(str::trim).unwrap_or(line);
        let Some((key, val)) = line.split_once('=') else {
            continue; // no `=` -> a malformed line, dropped
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let val = val.trim();
        // Strip a surrounding double-quote or apostrophe.
        let val = if val.len() >= 2
            && ((val.starts_with('"') && val.ends_with('"'))
                || (val.starts_with('\'') && val.ends_with('\'')))
        {
            &val[1..val.len() - 1]
        } else {
            val
        };
        map.insert(key.to_string(), val.to_string());
    }
    map
}

// ---- arithmetic helpers ----
pub(crate) fn is_num(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Flt(_))
}
pub(crate) fn to_f64(v: &Value) -> f64 {
    match v {
        Value::Int(n) => *n as f64,
        Value::Flt(x) => *x,
        _ => 0.0,
    }
}

pub(crate) fn int_arith(op: BinOp, a: i64, b: i64) -> EvalResult {
    use Value::*;
    // checked_*: instead of a debug panic / silent release wrap on overflow, the
    // same Fluxon error in both modes. i64::MIN / -1 (and % -1) panicked even in
    // Rust release — checked_div/checked_rem catch that too.
    Ok(match op {
        BinOp::Add => Int(a.checked_add(b).ok_or_else(|| Flow::overflow("+"))?),
        BinOp::Sub => Int(a.checked_sub(b).ok_or_else(|| Flow::overflow("-"))?),
        BinOp::Mul => Int(a.checked_mul(b).ok_or_else(|| Flow::overflow("*"))?),
        BinOp::Div => {
            if b == 0 {
                return Err(Flow::err("division by zero"));
            }
            Int(a.checked_div(b).ok_or_else(|| Flow::overflow("/"))?)
        }
        BinOp::Mod => {
            if b == 0 {
                return Err(Flow::err("division by zero (mod)"));
            }
            Int(a.checked_rem(b).ok_or_else(|| Flow::overflow("%"))?)
        }
        BinOp::Lt => Bool(a < b),
        BinOp::Le => Bool(a <= b),
        BinOp::Gt => Bool(a > b),
        BinOp::Ge => Bool(a >= b),
        _ => return Err(Flow::err("internal: unexpected int operator")),
    })
}

pub(crate) fn flt_arith(op: BinOp, a: f64, b: f64) -> EvalResult {
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
        _ => return Err(Flow::err("internal: unexpected flt operator")),
    })
}

#[cfg(test)]
mod dotenv_tests {
    use super::parse_dotenv;

    #[test]
    fn parses_basic_and_comments() {
        let m = parse_dotenv("# izoh\nPORT=8080\n\nNAME=Aziza   \n  # yana izoh\nEMPTY=\n");
        assert_eq!(m.get("PORT").map(String::as_str), Some("8080"));
        assert_eq!(m.get("NAME").map(String::as_str), Some("Aziza"));
        assert_eq!(m.get("EMPTY").map(String::as_str), Some(""));
        assert_eq!(m.len(), 3); // comments/empty lines were dropped
    }

    #[test]
    fn strips_quotes_and_export() {
        let m = parse_dotenv("export KEY=\"qiymat\"\nTOKEN='abc123'\nURL=http://x?a=1&b=2\n");
        assert_eq!(m.get("KEY").map(String::as_str), Some("qiymat"));
        assert_eq!(m.get("TOKEN").map(String::as_str), Some("abc123"));
        // if a `=` appears inside the value, only the FIRST `=` splits
        assert_eq!(m.get("URL").map(String::as_str), Some("http://x?a=1&b=2"));
    }

    #[test]
    fn skips_malformed_lines() {
        let m = parse_dotenv("noequalsign\n=novalue\nGOOD=ok\n");
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("GOOD").map(String::as_str), Some("ok"));
    }
}
