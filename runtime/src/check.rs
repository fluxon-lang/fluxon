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
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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
            // A relative-path `use ./mod` defines its alias/basename in the
            // current scope as an immutable binding (interp::exec_stmt), so
            // reassigning it (`use ./mod`; `mod <- {}`) is an error. Batteries
            // (`use http`) bind nothing, so they are left untracked.
            Stmt::Use { items } => {
                for it in items {
                    if crate::interp::is_user_module_path(&it.path) {
                        let name = it
                            .alias
                            .clone()
                            .unwrap_or_else(|| crate::interp::module_basename(&it.path));
                        self.declare(&name, Mutability::Imm);
                    }
                }
            }
            // No bindings / nothing to walk into.
            Stmt::Ret(None) | Stmt::Skip | Stmt::Stop | Stmt::Tbl { .. } => {}
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

/// Analyze an imported module body. At runtime a module executes in a
/// transparent child of the global scope (`Scope::child_of(&self.global)`), so a
/// module-level `=`/`<-` resolves *outward* into the names already visible at the
/// `use` site — and reassigning an existing **mutable** global there is accepted
/// by `interp::bind`/`assign`. `globals` seeds that outer scope as
/// `(name, is_mutable)` so the pass does not mistake such a reassignment for a
/// fresh immutable module-local and reject a program the runtime runs fine.
pub fn analyze_module(prog: &Program, globals: &[(String, bool)]) -> Result<(), String> {
    let mut a = Analyzer::new();
    for (name, mutable) in globals {
        let m = if *mutable {
            Mutability::Mut
        } else {
            Mutability::Imm
        };
        a.declare(name, m);
    }
    // The module body is a transparent (non-boundary) child of the seeded global
    // scope: a module-local bind shadows rather than collides with a same-named
    // global, exactly like the runtime's `child_of(global)`.
    a.push(false);
    a.stmts(prog)
}

/// Static-check an entry program together with every relative-path (`use ./...`)
/// module it imports — the coverage `fluxon check` needs for the common
/// "entry imports handlers/routes" layout, where the reassignment bug lives in a
/// module and would otherwise only surface at `run` time. `base` is the entry
/// file's directory (where its `use ./...` paths resolve from).
pub fn check_program_with_imports(prog: &Program, base: &Path) -> Result<(), String> {
    // The entry runs at the global scope — analyze it fresh.
    analyze(prog)?;
    // Names a module-level `=`/`<-` can resolve outward to (the entry's top-level
    // globals). Modules are children of the SAME global regardless of nesting, so
    // one seed serves them all. We mark every name *mutable* (permissive): we
    // cannot know the exact runtime mutability/ordering statically, and a
    // permissive seed can only ever miss an error, never invent one — the real
    // #178 target (module-/handler-local immutable reassignment) is unaffected.
    let seed = entry_global_seed(prog);
    let mut visited = HashSet::new();
    check_imports(prog, base, &seed, &mut visited)
}

// Collects the names the entry binds at the top level (so an imported module's
// outward-resolving `=`/`<-` is not mistaken for a fresh immutable local). Only
// direct top-level statements define globals — binds inside top-level blocks
// stay block-local — so we do not recurse here.
fn entry_global_seed(prog: &Program) -> Vec<(String, bool)> {
    let mut names = Vec::new();
    for stmt in prog {
        match stmt {
            Stmt::Bind { name, .. } | Stmt::ExpBind { name, .. } | Stmt::FnDecl { name, .. } => {
                names.push((name.clone(), true))
            }
            Stmt::Assign { target, .. } => {
                if let Expr::Ident(name) = target.as_ref() {
                    names.push((name.clone(), true));
                }
            }
            Stmt::Use { items } => {
                for it in items {
                    if crate::interp::is_user_module_path(&it.path) {
                        let name = it
                            .alias
                            .clone()
                            .unwrap_or_else(|| crate::interp::module_basename(&it.path));
                        names.push((name, true));
                    }
                }
            }
            _ => {}
        }
    }
    names
}

// Walks a program's `use ./...` imports, resolving + analyzing each user module
// (and its own imports, recursively). `visited` dedupes shared imports and
// breaks cycles. Resolution mirrors `interp::load_module`: base + path, `.fx`
// default extension, canonicalize. A path that cannot be resolved or read is
// skipped (not a check failure) — `run` reports a genuinely missing module.
fn check_imports(
    prog: &Program,
    base: &Path,
    seed: &[(String, bool)],
    visited: &mut HashSet<PathBuf>,
) -> Result<(), String> {
    for stmt in prog {
        let Stmt::Use { items } = stmt else { continue };
        for it in items {
            if !crate::interp::is_user_module_path(&it.path) {
                continue;
            }
            let mut full = base.join(&it.path);
            if full.extension().is_none() {
                full.set_extension("fx");
            }
            let Ok(canon) = full.canonicalize() else {
                continue;
            };
            if !visited.insert(canon.clone()) {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&canon) else {
                continue;
            };
            let toks = crate::lexer::lex(&src)?;
            let modprog = crate::parser::parse(toks)?;
            analyze_module(&modprog, seed)?;
            // Nested imports resolve relative to THIS module's directory.
            let mod_base = canon.parent().unwrap_or(base);
            check_imports(&modprog, mod_base, seed, visited)?;
        }
    }
    Ok(())
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

    fn check_module(src: &str, globals: &[(&str, bool)]) -> Result<(), String> {
        let toks = lexer::lex(src).expect("lex");
        let prog = parser::parse(toks).expect("parse");
        let seed: Vec<(String, bool)> = globals.iter().map(|(n, m)| (n.to_string(), *m)).collect();
        analyze_module(&prog, &seed)
    }

    #[test]
    fn module_reassigns_existing_mutable_global_is_ok() {
        // The reviewer's case: the entry file declared `x <- 0` (mutable global)
        // before `use`; a module-level `x = 1` / `x = 2` resolves outward to that
        // mutable global, which the runtime accepts — so it must not be flagged.
        check_module("x = 1\nx = 2\n", &[("x", true)])
            .expect("reassigning an existing mutable global from a module is valid");
    }

    #[test]
    fn module_reassigns_immutable_global_errors() {
        // An immutable global of the same name is not updatable from the module.
        let err = check_module("x = 1\n", &[("x", false)])
            .expect_err("binding over an immutable global from a module should error");
        assert!(err.contains("is immutable"), "unexpected: {err}");
    }

    #[test]
    fn module_local_immutable_reassign_in_block_errors() {
        // A fresh module-local (not a global) still gets the issue #178 check.
        let err = check_module(
            r#"
result = {}
if true
  result = result.set "a" 1
"#,
            &[("log", false), ("rep", false)],
        )
        .expect_err("reassigning a module-local immutable inside a block should error");
        assert!(err.contains("is immutable"), "unexpected: {err}");
    }

    #[test]
    fn reassign_user_module_alias_errors() {
        // `use ./mod` binds `mod` as immutable; reassigning it must be flagged.
        let err = check("use ./mod\nmod <- {}\n")
            .expect_err("reassigning an imported user-module name should error");
        assert!(
            err.contains("cannot be changed with '<-'"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn reassign_aliased_user_module_errors() {
        let err = check("use ./tools as t\nt = 1\n")
            .expect_err("reassigning an aliased import should error");
        assert!(err.contains("is immutable"), "unexpected: {err}");
    }

    #[test]
    fn reassign_battery_name_is_ok() {
        // Batteries bind nothing, so `http` is a free name — `<-` just makes a
        // mutable local (matches the runtime), never an error.
        check("use http\nhttp <- 1\n").expect("a battery name is not an immutable binding");
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
