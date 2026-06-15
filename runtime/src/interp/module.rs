// `use ./file` user-module loading + the tbl schema registry.
//
// Module loading: read the file, run it in its own module scope, build the
// namespace `Value::Map` from the `exp`-ed names. Caching and circular-import
// protection live here (the loading stack is thread-local — see `scope.rs` — so
// `par` parallel imports do not see each other as a cycle). `register_tbl`
// writes a `tbl` declaration into the schema registry used by auto-migration.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::ast::*;
use crate::value::{FnValue, Value};

use super::scope::{Env, EvalResult, Flow, MODULE_LOADING, Scope};
use super::util::{collect_exported, is_user_module_path, module_basename};
use super::{ColMeta, Interp, TableMeta};

impl Interp {
    // Writes the tbl declaration into the schema registry (columns + order + indexes).
    pub(crate) fn register_tbl(&self, name: &str, columns: &[TblColumn], indexes: &[TblIndex]) {
        let mut cols = BTreeMap::new();
        let mut col_order = Vec::with_capacity(columns.len());
        for c in columns {
            cols.insert(
                c.name.clone(),
                ColMeta {
                    type_name: c.type_name.clone(),
                    modifiers: c.modifiers.clone(),
                },
            );
            col_order.push(c.name.clone());
        }
        let idx_defs = indexes
            .iter()
            .map(|i| crate::db_mod::IndexDef {
                table: name.to_string(),
                columns: i.columns.clone(),
                unique: i.unique,
            })
            .collect();
        self.schema.write().insert(
            name.to_string(),
            TableMeta {
                columns: cols,
                col_order,
                indexes: idx_defs,
            },
        );
    }

    // `use ./file` — loads a user module and returns the namespace `Value::Map`.
    // The path is resolved relative to the current file's directory
    // (`current_base`). Cache and circular-import protection live here. Only
    // `exp`-ed names enter the namespace (the rest are module-private).
    pub(crate) fn load_module(&self, rel_path: &str) -> EvalResult {
        // 1. Build the full path: base + relative path, adding the .fx extension.
        let base = self.base_dir();
        let mut full = base.join(rel_path);
        if full.extension().is_none() {
            full.set_extension("fx");
        }
        // canonicalize: so the cache/cycle key is stable (symlink/`..` are
        // normalized). If the file does not exist it errors here.
        let canon = full
            .canonicalize()
            // In the error message we show the path the user wrote (`./greet`),
            // not the normalized full path — easier to read.
            .map_err(|e| Flow::err(format!("module not found '{}': {}", rel_path, e)))?;

        // 2. Cache hit — we do not re-execute (idempotent import).
        if let Some(v) = self.module_cache.lock().unwrap().get(&canon) {
            return Ok(v.clone());
        }

        // 3. Circular import: if this module is currently in the loading process
        //    ON THIS THREAD (A -> B -> A), we stop — otherwise infinite
        //    recursion. The stack is thread-local: `par` parallel imports do not
        //    see each other as a cycle (each lambda is an independent chain).
        let cycle = MODULE_LOADING.with(|l| {
            let loading = l.borrow();
            loading.contains(&canon).then(|| {
                loading
                    .iter()
                    .chain(std::iter::once(&canon))
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            })
        });
        if let Some(chain) = cycle {
            return Err(Flow::err(format!("circular import: {}", chain)));
        }
        MODULE_LOADING.with(|l| l.borrow_mut().push(canon.clone()));

        // 4. Execute the file. Regardless of the result, pop it off the stack.
        let result = self.run_module_file(&canon);
        MODULE_LOADING.with(|l| {
            l.borrow_mut().pop();
        });
        let ns = result?;

        // 5. Write to the cache (closure Arcs are shared — a second import takes a clone).
        self.module_cache.lock().unwrap().insert(canon, ns.clone());
        Ok(ns)
    }

    // Reads and parses the module file, executes it in a separate module scope,
    // and builds the namespace `Value::Map` from the `exp`-ed names. Temporarily
    // sets `current_base` to the module's directory (for nested imports) and
    // restores it when done.
    fn run_module_file(&self, canon: &std::path::Path) -> EvalResult {
        let src = std::fs::read_to_string(canon).map_err(|e| {
            Flow::err(format!(
                "could not read module '{}': {}",
                canon.display(),
                e
            ))
        })?;
        let toks = crate::lexer::lex(&src).map_err(Flow::err)?;
        let prog = crate::parser::parse(toks).map_err(Flow::err)?;

        // Module scope — a child of global: builtins (`log`/`rep`) and top-level
        // fns are visible through the lookup chain, but the module's own
        // `exp`/`=` names are searched first (shadowing — isolation is enough).
        let mod_scope = Scope::child_of(&self.global);

        // We set base to the module's directory — so a `use ./...` inside the
        // module resolves relative to this module. Save/restore: when a nested
        // import returns, the parent module's base is restored (on the error path too).
        let prev_base = self.base_dir();
        if let Some(dir) = canon.parent() {
            self.set_base(dir);
        }
        let exec = self.exec_module_body(&prog, &mod_scope);
        self.set_base(&prev_base);
        exec?;

        // We collect only the exported names: `exp NAME =` and `exp fn`.
        let exported = collect_exported(&prog);
        let mut ns = BTreeMap::new();
        for (name, v, _) in mod_scope.read().vars.iter() {
            if exported.contains(&**name) {
                ns.insert(name.to_string(), v.clone());
            }
        }
        Ok(Value::Map(ns))
    }

    // Executes the module body in the given scope. Differences from `run`:
    //  • fns are stored with a REAL `Parent::Scope(mod_scope)` Arc (NOT a
    //    Parent::Root marker) — so when a module fn is applied it reaches its own
    //    module scope (`exp greeting`), not the importer's global. This is
    //    REQUIRED for closure capture to work correctly.
    //  • `run_pending` is not called — a `http.serve`/`ws.serve` inside the
    //    module appends to the SAME Interp's `pending_servers` (because
    //    `arc_self` is that Interp), and they all start once at the end of top-level.
    //
    // Note (an intentionally accepted leak): the module scope holds fns in its
    // `vars`, and the fns hold the module scope via `Parent::Scope(mod_scope)` —
    // an Arc cycle. Modules must stay alive for the lifetime of the process (HTTP
    // handlers use them), so not dropping them is deliberate.
    fn exec_module_body(&self, prog: &Program, scope: &Env) -> Result<(), Flow> {
        // Hoisting — fn/tbl pre-registered (they can call each other regardless
        // of order). The difference from `run`: the parent is the module scope (Arc).
        for stmt in prog {
            match stmt {
                Stmt::FnDecl {
                    name, params, body, ..
                } => {
                    let f = Value::Fn(Arc::new(FnValue {
                        params: params.clone(),
                        body: body.clone(),
                        parent: Scope::parent_link(scope),
                        name: name.clone(),
                    }));
                    scope.write().define(name, f, false);
                }
                Stmt::Tbl {
                    name,
                    columns,
                    indexes,
                } => self.register_tbl(name, columns, indexes),
                _ => {}
            }
        }
        for stmt in prog {
            if matches!(stmt, Stmt::FnDecl { .. } | Stmt::Tbl { .. }) {
                continue;
            }
            // Module top level — like the script top level, no function to return
            // from, so a bare `rep` stays a value (`tail_used = true`).
            match self.exec_stmt(stmt, scope, true) {
                Ok(_) => {}
                Err(Flow::Error(e)) => return Err(Flow::Error(e)),
                Err(Flow::Fail { status, message }) => {
                    let pfx = status.map(|s| format!("[{}] ", s)).unwrap_or_default();
                    return Err(Flow::err(format!("fail: {}{}", pfx, message)));
                }
                Err(Flow::Return(_)) => {} // module top-level ret — ignored
                Err(Flow::Skip) | Err(Flow::Stop) => {
                    return Err(Flow::err("skip/stop used outside a loop"));
                }
            }
        }
        Ok(())
    }

    // `use` statement handling: load each user-file item and bind its namespace.
    // Battery names (`use http`) are a no-op (dispatched by name elsewhere).
    pub(crate) fn exec_use(&self, items: &[UseItem], env: &Env) -> Result<(), Flow> {
        for item in items {
            // A relative path (starts with `.`/`..`) — a user file. Otherwise a
            // battery name (no-op, the old behavior).
            if !is_user_module_path(&item.path) {
                continue;
            }
            let ns = self.load_module(&item.path)?;
            // The binding name: the alias if present, otherwise the path
            // "basename" (`./lib/greet` -> `greet`).
            let name = item
                .alias
                .clone()
                .unwrap_or_else(|| module_basename(&item.path));
            env.write().define(&name, ns, false);
        }
        Ok(())
    }
}
