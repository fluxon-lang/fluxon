// Static immutability check (issue #178).
//
// Fluxon's `=` binds an IMMUTABLE variable; reassigning it (with `=` or `<-`),
// even from inside a transparent block (`if`/`each`/`match`/`try`), is a runtime
// error. The trouble: the error only fires when that code path is actually
// executed, so a handler can pass `fluxon check`, boot fine, and only 500 on the
// specific request that hits the reassignment. For an "AI writes it correctly the
// first time" language that silent trap is the worst failure mode.
//
// This pass detects the violation statically — before any code runs — by
// replaying the interpreter's lexical scoping rules over the AST:
//
//   • `=` (`bind`) is transparent across `if`/`each`/`match` blocks but STOPS at
//     the fn/lambda boundary (a `=` inside a fn shadows, it does not touch an
//     outer var). So a re-`=` is resolved only within the current function frame.
//   • `<-` (`assign`) CROSSES fn boundaries (closure capture), so it is resolved
//     against the whole lexical chain.
//   • params and `each`/`<-` bindings are mutable; `=`-bound names, fn names and
//     `catch` vars are immutable — mirroring `interp::scope::Scope::define`.
//
// If a name that is bound immutable is ever the target of a later `=`/`<-` that
// resolves to it, that is the same error the interpreter would raise at runtime —
// we just raise it now, with the same message.

use crate::ast::{Expr, IfExpr, MatchExpr, Program, Stmt, StrPiece};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq)]
enum Mutability {
    Imm,
    Mut,
}

// Entry point: walk the program as the top-level function frame.
pub fn check_immutability(prog: &Program) -> Result<(), String> {
    let mut c = Checker {
        scopes: Vec::new(),
        fn_base: Vec::new(),
    };
    c.enter_fn();
    // Mirror the interpreter's top-level/module hoisting: `Interp::run` (and
    // `exec_module_body`) register every DIRECT top-level `FnDecl` — immutably —
    // before executing any statement. So `x = 1` followed by `fn x ...` is a
    // rebind error at run time; pre-seed the fn names so `check` agrees. (Fn
    // BODIES are not hoisted — they run in order — so this only applies to the
    // program/module root, not nested frames.)
    for stmt in prog {
        if let Stmt::FnDecl { name, .. } = stmt {
            c.define(name, Mutability::Imm);
        }
    }
    c.check_block(prog)?;
    Ok(())
}

// Is the `use` path a user file (relative) or a battery (a plain name)? Mirrors
// `interp::util::is_user_module_path` — only user modules bind an env name.
fn is_user_module_path(path: &str) -> bool {
    path.starts_with("./") || path.starts_with("../") || path == "." || path == ".."
}

// The binding name a module path resolves to: last segment without `.fx`.
// Mirrors `interp::util::module_basename` (`./lib/greet` -> `greet`).
fn module_basename(path: &str) -> String {
    let last = path.rsplit('/').next().unwrap_or(path);
    last.strip_suffix(".fx").unwrap_or(last).to_string()
}

struct Checker {
    // A stack of lexical scopes. Each block (`if`/`each`/...) pushes one; each
    // function/lambda pushes its base scope and records its index in `fn_base`.
    scopes: Vec<HashMap<String, Mutability>>,
    // Index (into `scopes`) of the current function's base scope. `=` resolution
    // never looks below the last entry — that is the fn boundary.
    fn_base: Vec<usize>,
}

impl Checker {
    fn enter_fn(&mut self) {
        self.scopes.push(HashMap::new());
        self.fn_base.push(self.scopes.len() - 1);
    }

    fn exit_fn(&mut self) {
        let base = self.fn_base.pop().expect("fn_base underflow");
        self.scopes.truncate(base);
    }

    fn enter_block(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn exit_block(&mut self) {
        self.scopes.pop();
    }

    fn define(&mut self, name: &str, m: Mutability) {
        self.scopes.last_mut().unwrap().insert(name.to_string(), m);
    }

    // `=` resolution: innermost scope down to (and including) the current fn base.
    fn resolve_bind(&self, name: &str) -> Option<Mutability> {
        let base = *self.fn_base.last().unwrap();
        self.scopes[base..]
            .iter()
            .rev()
            .find_map(|s| s.get(name).copied())
    }

    // `<-` resolution: the whole lexical chain (crosses fn boundaries — closures).
    fn resolve_assign(&self, name: &str) -> Option<Mutability> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn check_block(&mut self, body: &[Stmt]) -> Result<(), String> {
        for stmt in body {
            self.check_stmt(stmt)?;
        }
        Ok(())
    }

    fn check_stmt(&mut self, stmt: &Stmt) -> Result<(), String> {
        match stmt {
            // `x = expr` — immutable bind. The value is evaluated BEFORE the
            // binding takes effect, so check it first.
            Stmt::Bind { name, value } => {
                self.check_expr(value)?;
                match self.resolve_bind(name) {
                    Some(Mutability::Imm) => {
                        return Err(format!(
                            "'{}' is immutable (declared with =); cannot be \
                             reassigned even from inside a block (declare it with `<-`)",
                            name
                        ));
                    }
                    // Resolves to an existing mutable var — `=` just updates it.
                    Some(Mutability::Mut) => {}
                    // New name — a fresh immutable binding in the current scope.
                    None => self.define(name, Mutability::Imm),
                }
                Ok(())
            }
            // `exp x = expr` — an exported binding. The interpreter writes it with a
            // direct `define(.., false)`, which OVERWRITES any prior mutability and
            // never errors (unlike `=`, which goes through `bind`). Mirror that: force
            // the name immutable without a rebind error here, so a later `=`/`<-`
            // against it is correctly rejected, while `exp` itself never is.
            Stmt::ExpBind { name, value } => {
                self.check_expr(value)?;
                self.define(name, Mutability::Imm);
                Ok(())
            }
            // `target <- expr`. A plain-ident target is a (re)assignment; a field
            // target (`req.ctx <- ...`) is a context write, never a var rebind.
            Stmt::Assign { target, value } => {
                self.check_expr(value)?;
                if let Expr::Ident(name) = target.as_ref() {
                    match self.resolve_assign(name) {
                        Some(Mutability::Imm) => {
                            return Err(format!(
                                "'{}' is immutable (declared with =), cannot be changed with '<-'",
                                name
                            ));
                        }
                        Some(Mutability::Mut) => {}
                        // New mutable variable in the current scope.
                        None => self.define(name, Mutability::Mut),
                    }
                } else {
                    self.check_expr(target)?;
                }
                Ok(())
            }
            // A fn name is an immutable binding in the enclosing scope; its body is
            // a new function frame with mutable params.
            Stmt::FnDecl {
                name, params, body, ..
            } => {
                self.define(name, Mutability::Imm);
                self.enter_fn();
                for p in params {
                    self.define(p, Mutability::Mut);
                }
                self.check_block(body)?;
                self.exit_fn();
                Ok(())
            }
            // `each [k,] v in iter` — loop vars are mutable, scoped to the body.
            Stmt::Each { vars, iter, body } => {
                self.check_expr(iter)?;
                self.enter_block();
                for v in vars {
                    self.define(v, Mutability::Mut);
                }
                self.check_block(body)?;
                self.exit_block();
                Ok(())
            }
            Stmt::Ret(Some(e)) => self.check_expr(e),
            Stmt::Fail { status, message } => {
                if let Some(s) = status {
                    self.check_expr(s)?;
                }
                self.check_expr(message)
            }
            Stmt::Expr(e) => self.check_expr(e),
            // `use ./mod [as alias]` binds the module namespace immutably
            // (`exec_use` → `define(.., false)`), so a later `<-`/`=` against the
            // alias is rejected. Battery imports (`use http`) bind nothing — they
            // dispatch by name — so they introduce no binding here.
            Stmt::Use { items } => {
                for item in items {
                    if is_user_module_path(&item.path) {
                        let name = item
                            .alias
                            .clone()
                            .unwrap_or_else(|| module_basename(&item.path));
                        self.define(&name, Mutability::Imm);
                    }
                }
                Ok(())
            }
            // No bindings / nothing to recurse into.
            Stmt::Ret(None) | Stmt::Skip | Stmt::Stop | Stmt::Tbl { .. } => Ok(()),
        }
    }

    fn check_expr(&mut self, expr: &Expr) -> Result<(), String> {
        match expr {
            // Blocks living inside expressions (`r = if ... \n z = 1 \n z`) are
            // transparent, just like statement-position blocks.
            Expr::If(ifx) => self.check_if(ifx),
            Expr::Match(mx) => self.check_match(mx),
            Expr::TryCatch {
                body,
                catch_var,
                catch_body,
            } => {
                self.enter_block();
                self.check_block(body)?;
                self.exit_block();
                self.enter_block();
                // The caught error is bound immutable (Scope::define(.., false)).
                if let Some(name) = catch_var {
                    self.define(name, Mutability::Imm);
                }
                self.check_block(catch_body)?;
                self.exit_block();
                Ok(())
            }
            // A lambda body is a fresh function frame with mutable params.
            Expr::Lambda { params, body } => {
                self.enter_fn();
                for p in params {
                    self.define(p, Mutability::Mut);
                }
                self.check_block(body)?;
                self.exit_fn();
                Ok(())
            }
            Expr::Unary { expr, .. } => self.check_expr(expr),
            Expr::Binary { lhs, rhs, .. } => {
                self.check_expr(lhs)?;
                self.check_expr(rhs)
            }
            Expr::Field { target, .. } => self.check_expr(target),
            Expr::Index { target, key } => {
                self.check_expr(target)?;
                self.check_expr(key)
            }
            Expr::Call { callee, args } => {
                self.check_expr(callee)?;
                for a in args {
                    self.check_expr(a)?;
                }
                Ok(())
            }
            Expr::Try(e) => self.check_expr(e),
            Expr::Fail { status, message } => {
                if let Some(s) = status {
                    self.check_expr(s)?;
                }
                self.check_expr(message)
            }
            Expr::List(items) => {
                for it in items {
                    self.check_expr(it)?;
                }
                Ok(())
            }
            Expr::Map(entries) => {
                use crate::ast::MapEntry;
                for e in entries {
                    match e {
                        MapEntry::Pair { value, .. } => self.check_expr(value)?,
                        MapEntry::Spread(v) => self.check_expr(v)?,
                        MapEntry::Dynamic { key, value } => {
                            self.check_expr(key)?;
                            self.check_expr(value)?;
                        }
                    }
                }
                Ok(())
            }
            Expr::Str(pieces) => {
                for p in pieces {
                    if let StrPiece::Expr(e) = p {
                        self.check_expr(e)?;
                    }
                }
                Ok(())
            }
            Expr::Range { start, end } => {
                self.check_expr(start)?;
                self.check_expr(end)
            }
            // Leaves — no nested expressions, no bindings.
            Expr::Int(_)
            | Expr::Flt(_)
            | Expr::Bool(_)
            | Expr::Nil
            | Expr::Sym(_)
            | Expr::Ident(_)
            | Expr::Inf => Ok(()),
        }
    }

    fn check_if(&mut self, ifx: &IfExpr) -> Result<(), String> {
        for (cond, block) in &ifx.arms {
            self.check_expr(cond)?;
            self.enter_block();
            self.check_block(block)?;
            self.exit_block();
        }
        if let Some(block) = &ifx.else_block {
            self.enter_block();
            self.check_block(block)?;
            self.exit_block();
        }
        Ok(())
    }

    fn check_match(&mut self, mx: &MatchExpr) -> Result<(), String> {
        self.check_expr(&mx.subject)?;
        for arm in &mx.arms {
            // Patterns are symbol/int literals or `_` — they bind no variables.
            self.enter_block();
            self.check_block(&arm.body)?;
            self.exit_block();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lexer, parser};

    fn check(src: &str) -> Result<(), String> {
        let toks = lexer::lex(src).unwrap();
        let prog = parser::parse(toks).unwrap();
        check_immutability(&prog)
    }

    // The headline case from issue #178: a `=`-bound var reassigned inside a block.
    #[test]
    fn rebind_in_block_errors() {
        let err = check("result = {}\nif true\n  result = result.set \"a\" 1\n").unwrap_err();
        assert!(err.contains("is immutable"), "got: {}", err);
    }

    #[test]
    fn rebind_in_each_errors() {
        let err = check("acc = {}\neach k in [1 2]\n  acc = acc.set k k\n").unwrap_err();
        assert!(err.contains("is immutable"), "got: {}", err);
    }

    // `<-` against an immutable is just as wrong as a second `=`.
    #[test]
    fn assign_to_immutable_errors() {
        let err = check("x = 1\nx <- 2\n").unwrap_err();
        assert!(err.contains("is immutable"), "got: {}", err);
    }

    // A re-`=` at the same (top) level is also a violation.
    #[test]
    fn rebind_same_level_errors() {
        assert!(check("x = 1\nx = 2\n").is_err());
    }

    // Mutable accumulator (`<-`) is the correct form — must pass clean.
    #[test]
    fn mutable_accumulator_ok() {
        check("result <- {}\nif true\n  result <- result.set \"a\" 1\n").unwrap();
    }

    // `=` updating an OUTER mutable var inside a block is fine.
    #[test]
    fn bind_updates_outer_mutable_ok() {
        check("top <- 0\neach e in [3 7 2]\n  if e > top\n    top = e\n").unwrap();
    }

    // fn/lambda boundary: an inner `=` is a NEW local — shadowing, not a rebind.
    #[test]
    fn shadow_across_fn_boundary_ok() {
        check("x = 100\nf = \\n ->\n  x = 5\n  x + n\n").unwrap();
    }

    // `<-` legitimately crosses the fn boundary to a captured MUTABLE var.
    #[test]
    fn closure_captures_mutable_ok() {
        check("counter <- 0\ninc = \\n ->\n  counter <- counter + n\ninc 5\n").unwrap();
    }

    // The same name bound in two SIBLING blocks is independent — not a rebind.
    #[test]
    fn sibling_blocks_independent_ok() {
        check("if true\n  x = 1\nif true\n  x = 2\n").unwrap();
    }

    // A param is mutable: reassigning it is allowed.
    #[test]
    fn param_reassign_ok() {
        check("fn f n\n  n = n + 1\n  n\n").unwrap();
    }

    // A `<-`-first var can later be `=`-updated.
    #[test]
    fn assign_then_bind_ok() {
        check("x <- 1\nx = 2\n").unwrap();
    }

    // `exp x = ..` freezes the name immutable, so a later `<-` is rejected.
    #[test]
    fn exp_bind_then_assign_errors() {
        let err = check("exp x = 1\nx <- 2\n").unwrap_err();
        assert!(err.contains("is immutable"), "got: {}", err);
    }

    // `exp` writes with a direct overwrite (interp `define(.., false)`), so it
    // never errors itself — even over a prior mutable binding.
    #[test]
    fn assign_then_exp_bind_ok() {
        check("x <- 1\nexp x = 2\n").unwrap();
    }

    // ...but it DOES freeze the name, so a `<-` after the `exp` is rejected.
    #[test]
    fn exp_bind_freezes_prior_mutable() {
        let err = check("x <- 1\nexp x = 2\nx <- 3\n").unwrap_err();
        assert!(err.contains("is immutable"), "got: {}", err);
    }

    // Top-level fns are hoisted immutably, so `x = 1` then `fn x ...` is the same
    // rebind error at check time as at run time — regardless of textual order.
    #[test]
    fn bind_then_fn_decl_same_name_errors() {
        let err = check("x = 1\nfn x n\n  n\n").unwrap_err();
        assert!(err.contains("is immutable"), "got: {}", err);
    }

    // Hoisting also means a fn can be referenced before its declaration — reading
    // it (not rebinding) is fine.
    #[test]
    fn call_fn_before_decl_ok() {
        check("log (f 1)\nfn f n\n  n + 1\n").unwrap();
    }

    // A user-module import alias is immutable: assigning it is rejected.
    #[test]
    fn assign_to_module_alias_errors() {
        let err = check("use ./tools as t\nt <- {}\n").unwrap_err();
        assert!(err.contains("is immutable"), "got: {}", err);
        // The default basename binding is immutable too.
        let err2 = check("use ./tools\ntools = 1\n").unwrap_err();
        assert!(err2.contains("is immutable"), "got: {}", err2);
    }

    // Battery imports (`use http`) bind no env name — `http <- x` is just a new
    // local, never an error.
    #[test]
    fn battery_import_not_a_binding_ok() {
        check("use http\nhttp <- 1\n").unwrap();
    }
}
