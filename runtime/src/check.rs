//! Static analysis pass — runs after parsing, before interpretation.
//!
//! Today it catches one whole class of bug: reassigning a `=`-bound (immutable)
//! variable, including from inside a nested block (`if`/`each`/`match`/`try`).
//! The interpreter already rejects this, but only when the offending line is
//! actually executed — so a handler compiles, `fluxon check` passes, the server
//! boots, and the bug only surfaces as a 500 on a specific request path (see
//! issue #178). For a language whose promise is "the AI writes it correctly the
//! first time", that silent trap has to be reported up front.
//!
//! The analysis mirrors `interp::bind`/`interp::assign` scoping exactly so it
//! never reports a reassignment the runtime would actually accept:
//!   * `if`/`each`/`match`/`try` blocks are lexically transparent — an `=` inside
//!     one resolves to an outer variable in the *same* fn;
//!   * fn/lambda bodies are isolation boundaries — an `=` there creates a new
//!     local and never touches an outer (so it is never flagged);
//!   * params, loop vars and catch vars are mutable / fresh, so they shadow
//!     rather than collide.
//!
//! It is intentionally conservative: when in doubt it stays silent (e.g. a `<-`
//! that would reach across a fn boundary into an outer immutable) so it can only
//! ever miss an error, never invent one against valid code.

use crate::ast::{Expr, IfExpr, MapEntry, MatchExpr, Program, Stmt, StrPiece};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq)]
enum Mutability {
    /// `=` / `exp` / fn name — cannot be reassigned.
    Imm,
    /// `<-` / param / loop var / catch var — can be reassigned.
    Mut,
}

struct ScopeFrame {
    vars: HashMap<String, Mutability>,
    /// A fn/lambda boundary. Name resolution stops once it crosses one — this is
    /// the `is_fn_boundary` walk from `interp::bind`.
    boundary: bool,
}

struct Analyzer {
    scopes: Vec<ScopeFrame>,
}

impl Analyzer {
    fn new() -> Self {
        // The top level is itself a boundary (interp's `Parent::Root`): a
        // top-level `=` does not escape into a different fn.
        Analyzer {
            scopes: vec![ScopeFrame {
                vars: HashMap::new(),
                boundary: true,
            }],
        }
    }

    fn push(&mut self, boundary: bool) {
        self.scopes.push(ScopeFrame {
            vars: HashMap::new(),
            boundary,
        });
    }

    fn pop(&mut self) {
        self.scopes.pop();
    }

    /// Resolve a name the way `interp::bind` does: innermost scope outward, but
    /// stop once we cross a fn/lambda boundary.
    fn lookup(&self, name: &str) -> Option<Mutability> {
        for frame in self.scopes.iter().rev() {
            if let Some(m) = frame.vars.get(name) {
                return Some(*m);
            }
            if frame.boundary {
                break;
            }
        }
        None
    }

    /// A fresh declaration into the innermost scope (params, loop vars, catch
    /// vars, fn names). Shadows any outer binding — never an error.
    fn declare(&mut self, name: &str, m: Mutability) {
        self.scopes
            .last_mut()
            .unwrap()
            .vars
            .insert(name.to_string(), m);
    }

    /// `=` (Bind/ExpBind). Mirrors `interp::bind`: reassigning a visible
    /// immutable is the error; a visible mutable is updated; a brand-new name
    /// becomes an immutable local in the current scope.
    fn bind(&mut self, name: &str) -> Result<(), String> {
        match self.lookup(name) {
            Some(Mutability::Imm) => Err(format!(
                "'{}' is immutable (declared with =); cannot be reassigned even \
                 from inside a block (declare it with `<-`)",
                name
            )),
            Some(Mutability::Mut) => Ok(()),
            None => {
                self.declare(name, Mutability::Imm);
                Ok(())
            }
        }
    }

    /// `<-` to a plain identifier. Mirrors `interp::assign`: an immutable target
    /// is the error; otherwise it updates / introduces a mutable local.
    fn assign(&mut self, name: &str) -> Result<(), String> {
        match self.lookup(name) {
            Some(Mutability::Imm) => Err(format!(
                "'{}' is immutable (declared with =), cannot be changed with '<-'",
                name
            )),
            Some(Mutability::Mut) => Ok(()),
            None => {
                self.declare(name, Mutability::Mut);
                Ok(())
            }
        }
    }

    fn stmts(&mut self, stmts: &[Stmt]) -> Result<(), String> {
        for s in stmts {
            self.stmt(s)?;
        }
        Ok(())
    }

    fn stmt(&mut self, s: &Stmt) -> Result<(), String> {
        match s {
            // The value is evaluated before the name is bound (interp order), so
            // analyze it first.
            Stmt::Bind { name, value } => {
                self.expr(value)?;
                self.bind(name)?;
            }
            // `exp NAME = expr` does NOT go through the bind/immutability check at
            // runtime: interp defines it directly (overwriting), so exporting a
            // value under a name already bound with `=` (`handler = ...` then
            // `exp handler = handler`) is a valid module-export pattern. Treat it
            // as a fresh (re)declaration, never a reassignment error.
            Stmt::ExpBind { name, value } => {
                self.expr(value)?;
                self.declare(name, Mutability::Imm);
            }
            Stmt::Assign { target, value } => {
                self.expr(value)?;
                match target.as_ref() {
                    // A plain identifier is a variable reassignment.
                    Expr::Ident(name) => self.assign(name)?,
                    // A field target (`req.ctx <- v`) writes into a cell, not a
                    // binding — just analyze the sub-expressions.
                    other => self.expr(other)?,
                }
            }
            Stmt::FnDecl {
                name, params, body, ..
            } => {
                // The fn name is an immutable binding in the current scope.
                // Declared before the body so the pass tolerates self/mutual
                // recursion exactly like the runtime.
                self.declare(name, Mutability::Imm);
                self.push(true); // fn boundary
                for p in params {
                    self.declare(p, Mutability::Mut); // params are mutable (`<-` ok)
                }
                self.stmts(body)?;
                self.pop();
            }
            Stmt::Each { vars, iter, body } => {
                self.expr(iter)?;
                // The body is a transparent block, but the loop vars live in it
                // (fresh & mutable each iteration), shadowing any outer name.
                self.push(false);
                for v in vars {
                    self.declare(v, Mutability::Mut);
                }
                self.stmts(body)?;
                self.pop();
            }
            Stmt::Ret(Some(e)) => self.expr(e)?,
            Stmt::Fail { status, message } => {
                if let Some(st) = status {
                    self.expr(st)?;
                }
                self.expr(message)?;
            }
            Stmt::Expr(e) => self.expr(e)?,
            // No bindings / nothing to walk into.
            Stmt::Ret(None) | Stmt::Skip | Stmt::Stop | Stmt::Use { .. } | Stmt::Tbl { .. } => {}
        }
        Ok(())
    }

    fn expr(&mut self, e: &Expr) -> Result<(), String> {
        match e {
            Expr::Int(_)
            | Expr::Flt(_)
            | Expr::Bool(_)
            | Expr::Nil
            | Expr::Sym(_)
            | Expr::Ident(_)
            | Expr::Inf => {}
            Expr::Str(pieces) => {
                for p in pieces {
                    if let StrPiece::Expr(ex) = p {
                        self.expr(ex)?;
                    }
                }
            }
            Expr::List(items) => {
                for it in items {
                    self.expr(it)?;
                }
            }
            Expr::Map(entries) => {
                for ent in entries {
                    match ent {
                        MapEntry::Pair { value, .. } => self.expr(value)?,
                        MapEntry::Spread(ex) => self.expr(ex)?,
                        MapEntry::Dynamic { key, value } => {
                            self.expr(key)?;
                            self.expr(value)?;
                        }
                    }
                }
            }
            Expr::Unary { expr, .. } => self.expr(expr)?,
            Expr::Binary { lhs, rhs, .. } => {
                self.expr(lhs)?;
                self.expr(rhs)?;
            }
            Expr::Field { target, .. } => self.expr(target)?,
            Expr::Index { target, key } => {
                self.expr(target)?;
                self.expr(key)?;
            }
            Expr::Call { callee, args } => {
                self.expr(callee)?;
                for a in args {
                    self.expr(a)?;
                }
            }
            Expr::Lambda { params, body } => {
                self.push(true); // a lambda is a fn boundary
                for p in params {
                    self.declare(p, Mutability::Mut);
                }
                self.stmts(body)?;
                self.pop();
            }
            Expr::Try(inner) => self.expr(inner)?,
            Expr::TryCatch {
                body,
                catch_var,
                catch_body,
            } => {
                self.push(false);
                self.stmts(body)?;
                self.pop();
                self.push(false);
                if let Some(v) = catch_var {
                    // The catch var is an immutable binding (interp defines it
                    // with `false`) scoped to the catch block.
                    self.declare(v, Mutability::Imm);
                }
                self.stmts(catch_body)?;
                self.pop();
            }
            Expr::Fail { status, message } => {
                if let Some(st) = status {
                    self.expr(st)?;
                }
                self.expr(message)?;
            }
            Expr::If(ifx) => self.if_expr(ifx)?,
            Expr::Match(mx) => self.match_expr(mx)?,
            Expr::Range { start, end } => {
                self.expr(start)?;
                self.expr(end)?;
            }
        }
        Ok(())
    }

    fn if_expr(&mut self, ifx: &IfExpr) -> Result<(), String> {
        for (cond, block) in &ifx.arms {
            self.expr(cond)?;
            self.push(false);
            self.stmts(block)?;
            self.pop();
        }
        if let Some(eb) = &ifx.else_block {
            self.push(false);
            self.stmts(eb)?;
            self.pop();
        }
        Ok(())
    }

    fn match_expr(&mut self, mx: &MatchExpr) -> Result<(), String> {
        self.expr(&mx.subject)?;
        // Match patterns (sym/int/`_`) are comparisons, not bindings.
        for arm in &mx.arms {
            self.push(false);
            self.stmts(&arm.body)?;
            self.pop();
        }
        Ok(())
    }
}

/// Static-analyze a parsed program. `Ok(())` means clean; `Err` carries a
/// human-readable, located-enough message in the same wording the runtime uses.
pub fn analyze(prog: &Program) -> Result<(), String> {
    let mut a = Analyzer::new();
    a.stmts(prog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lexer, parser};

    fn check(src: &str) -> Result<(), String> {
        let toks = lexer::lex(src).expect("lex");
        let prog = parser::parse(toks).expect("parse");
        analyze(&prog)
    }

    #[test]
    fn reassign_immutable_in_if_block_errors() {
        // The exact issue #178 reproduction.
        let err = check(
            r#"
result = {}
if true
  result = result.set "a" 1
"#,
        )
        .expect_err("reassigning a =-bound var inside an if should be a static error");
        assert!(err.contains("is immutable"), "unexpected: {err}");
    }

    #[test]
    fn reassign_immutable_in_each_block_errors() {
        let err = check(
            r#"
result = {}
each k in [1, 2, 3]
  result = k
"#,
        )
        .expect_err("reassigning a =-bound var inside each should be a static error");
        assert!(err.contains("is immutable"), "unexpected: {err}");
    }

    #[test]
    fn reassign_immutable_with_arrow_errors() {
        let err = check(
            r#"
x = 1
x <- 2
"#,
        )
        .expect_err("`<-` on a =-bound var should be a static error");
        assert!(
            err.contains("cannot be changed with '<-'"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn rebind_same_immutable_errors() {
        let err =
            check("x = 1\nx = 2\n").expect_err("re-binding the same immutable name should error");
        assert!(err.contains("is immutable"), "unexpected: {err}");
    }

    #[test]
    fn mutable_accumulator_is_ok() {
        // The correct version of the pattern: `<-` throughout.
        check(
            r#"
result <- {}
each k in [1, 2, 3]
  result <- result.set k k
"#,
        )
        .expect("a `<-` accumulator across a block is valid");
    }

    #[test]
    fn exp_export_of_existing_bind_is_ok() {
        // `exp` defines directly at runtime (no immutability check), so exporting
        // a value under a name already bound with `=` is a valid pattern.
        check("handler = 1\nexp handler = handler\n")
            .expect("exporting an existing `=` binding under the same name is valid");
    }

    #[test]
    fn mutable_then_bind_is_ok() {
        // A mutable variable may be updated with `=` (interp allows it).
        check("x <- 1\nx = 2\n").expect("= on a mutable var is allowed");
    }

    #[test]
    fn fn_boundary_shadows_not_reassigns() {
        // An `=` inside a fn/lambda creates a new local; it must not be flagged
        // against the outer immutable of the same name.
        check(
            r#"
x = 100
f = \n ->
  x = 5
  x + n
"#,
        )
        .expect("an inner `=` shadows across a fn boundary");
    }

    #[test]
    fn loop_var_shadows_outer_immutable() {
        check(
            r#"
x = 1
each x in [1, 2, 3]
  log x
"#,
        )
        .expect("a loop var shadows an outer immutable of the same name");
    }

    #[test]
    fn sibling_blocks_do_not_collide() {
        // Each block has its own scope: the same name bound in two arms is fine.
        check(
            r#"
if true
  y = 1
else
  y = 2
"#,
        )
        .expect("the same name in sibling blocks is independent");
    }

    #[test]
    fn bind_after_block_scoped_bind_is_ok() {
        // A name bound only inside a block does not leak to the outer scope, so a
        // later outer `=` of the same name is a fresh binding.
        check(
            r#"
if true
  z = 1
z = 2
"#,
        )
        .expect("a block-local bind does not leak to the outer scope");
    }
}
