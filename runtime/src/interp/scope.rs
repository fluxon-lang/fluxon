// Lexical scope, flow signals, and the call-depth guard.
//
// This is the foundation the rest of the interpreter is built on:
//   - `Env`/`Scope`/`Parent` — the scope chain (with the root-marker
//     optimization that eliminates Arc contention; see ARCHITECTURE.md).
//   - `Flow` — control-flow signals (ret/skip/stop/fail) + errors, carried on
//     the `Err` side of `Result`.
//   - `CallDepthGuard` + the thread-local depth/module-loading state.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::value::Value;

// Lexical scope: a chain linked to the parent environment. Arc<RwLock<>> — for
// closures, mutation AND sharing across threads (true parallel HTTP). RwLock
// (not Mutex): lookup/read allows many readers in parallel, so parallel
// requests read functions in the global scope (e.g. recursive `fib`) without
// blocking each other. Writes (`<-`, bind) are exclusive.
pub type Env = Arc<RwLock<Scope>>;

// Parent link of the scope chain. Important: the ROOT (global) scope is SHARED
// across all threads — cloning/locking it on every lookup is the main source of
// atomic contention (cache-line bouncing on 8 cores). So the chain reaching the
// root uses `Parent::Root(env)`: the root Arc is preserved (intermediate scopes
// NEVER clone it), and once the global is frozen, lookup reads from a lock-free
// frozen snapshot WITHOUT TOUCHING the root Arc.
#[derive(Clone)]
pub enum Parent {
    // The root scope itself — no parent above.
    None,
    // Parent is the root (global) scope. A MARKER (not an Arc!) — the root Arc
    // is not held, so a fn call / scope opening does not ATOMICALLY bump the
    // root refcount (no cache-line bouncing). Once frozen, lookup reads from the
    // frozen snapshot; when not frozen (top-level) it reads from the
    // `Interp.global` Arc — both via `&self`, no clone needed.
    Root,
    // Parent is a plain (non-root) scope.
    Scope(Env),
}

pub struct Scope {
    // Names — a small VECTOR (not a HashMap). Fn-call / block scopes usually
    // hold 0-4 names; for such a small set a linear scan beats computing a hash
    // + a HashMap allocation, and the per-call allocation is cheap (one Vec
    // buffer instead of two empty HashMaps). Element: (name, value, is-mutable).
    // mutable = whether it can be re-bound with `<-` (`=`/`exp`/param are
    // immutable; `<-` and loop vars are mutable).
    pub(crate) vars: Vec<(Box<str>, Value, bool)>,
    pub(crate) parent: Parent,
    // Is this scope the root (global)? When lookup reaches the root, if the
    // Interp has frozen the global it reads from a lock-free snapshot (no
    // parallel contention).
    pub(crate) is_root: bool,
    // Is this scope an fn/lambda call boundary? An `=` bind looking up an outer
    // variable stops here (function isolation/shadowing). if/each/match blocks
    // are `false` — they are lexically TRANSPARENT: inside them an `=` can
    // update an outer variable (within the same fn).
    pub(crate) is_fn_boundary: bool,
}

impl Scope {
    pub fn root() -> Env {
        Arc::new(RwLock::new(Scope {
            vars: Vec::new(),
            parent: Parent::None,
            is_root: true,
            is_fn_boundary: false,
        }))
    }
    // A new (empty) child scope under the given `Parent` link. `apply`/`if`/
    // `each`/`match` open scopes through this. IMPORTANT: it does NOT LOCK the
    // parent — the link type (Root/Scope) comes from the caller, so a recursive
    // fn call never touches the root Arc at all (no contention).
    pub(crate) fn child(parent: Parent) -> Env {
        Arc::new(RwLock::new(Scope {
            vars: Vec::new(),
            parent,
            is_root: false,
            is_fn_boundary: false, // if/each/match — transparent block
        }))
    }
    // A child pre-sized by the number of params (fn call — no re-allocation
    // during bind).
    pub(crate) fn child_with_capacity(parent: Parent, cap: usize) -> Env {
        Arc::new(RwLock::new(Scope {
            vars: Vec::with_capacity(cap),
            parent,
            is_root: false,
            is_fn_boundary: true, // fn/lambda call — isolation boundary
        }))
    }
    // Turns the `env` Arc into a parent link for a child (a single lock, only to
    // learn `is_root`). Top-level code (if/each/match in the global env) goes
    // through this — single-threaded, contention-free. A fn call instead uses
    // `FnValue.parent` (Parent) directly and does not enter this path.
    pub(crate) fn parent_link(env: &Env) -> Parent {
        if env.read().is_root {
            Parent::Root
        } else {
            Parent::Scope(env.clone())
        }
    }
    // A child under the given env (combines the two above).
    pub(crate) fn child_of(env: &Env) -> Env {
        Scope::child(Scope::parent_link(env))
    }
    // Declares a name. If it already exists, updates value + mutable
    // (shadow/re-bind — the old HashMap insert semantics: last one wins).
    pub(crate) fn define(&mut self, name: &str, v: Value, mutable: bool) {
        for slot in self.vars.iter_mut() {
            if &*slot.0 == name {
                slot.1 = v;
                slot.2 = mutable;
                return;
            }
        }
        self.vars.push((name.into(), v, mutable));
    }
    // Reads a name's value (from the last declaration — scanning back to front).
    pub(crate) fn get(&self, name: &str) -> Option<&Value> {
        self.vars
            .iter()
            .rev()
            .find(|(n, _, _)| &**n == name)
            .map(|(_, v, _)| v)
    }
    // For `<-`: finds the mutable slot. Returns (slot, is-mutable).
    pub(crate) fn get_mut_entry(&mut self, name: &str) -> Option<(&mut Value, bool)> {
        self.vars
            .iter_mut()
            .rev()
            .find(|(n, _, _)| &**n == name)
            .map(|(_, v, m)| (v, *m))
    }
    // For installing builtins: sets an immutable value on a global name.
    pub fn set_global(&mut self, name: &str, v: Value) {
        self.define(name, v, false);
    }
}

// Flow-interruption signals and errors. All travel on the `Err` side.
pub enum Flow {
    Return(Value),
    Skip,
    Stop,
    // fail [status] message — a business or internal error.
    Fail {
        status: Option<i64>,
        message: String,
    },
    // A plain runtime error (type mismatch, unknown variable, ...).
    Error(String),
}

impl Flow {
    pub fn err(msg: impl Into<String>) -> Flow {
        Flow::Error(msg.into())
    }

    // The single error for i64 arithmetic going out of range (issue #89). Used
    // together with checked_*: instead of a debug panic and a silent wrap in
    // release, it gives the same explicit runtime error in both modes.
    pub fn overflow(who: &str) -> Flow {
        Flow::Error(format!("{}: number out of range (i64)", who))
    }
}

pub type EvalResult = Result<Value, Flow>;
pub(crate) type ExecResult = Result<Value, Flow>; // a block returns its last expression's value

// Maximum depth for Fluxon-level fn calls. The native stack grows in segments
// via `stacker::maybe_grow`, so the real limit is this counter: on reaching the
// limit it's a graceful Flow::err, not an abort. 1000 is in the same ballpark
// as Python's default recursion limit; real backend code does not recurse
// deeper than this, while infinite recursion is caught quickly.
pub(crate) const MAX_CALL_DEPTH: usize = 1000;

// stacker parameters: the red zone must be larger than the native stack that
// can be used within one Fluxon call (until the next check) — measured at
// ~15KB/level in a debug build. The segment size — each allocation fits ~130
// levels, so a few segments suffice for 1000 levels.
pub(crate) const STACK_RED_ZONE: usize = 128 * 1024;
pub(crate) const STACK_GROW_SIZE: usize = 2 * 1024 * 1024;

thread_local! {
    // Fluxon call depth on the current thread. Thread-local: each HTTP request
    // runs in its own spawn_blocking thread — one request's recursion does not
    // count toward another's. A field cannot be added to Interp (&self, Sync — a
    // Cell is not possible).
    pub(crate) static CALL_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };

    // For `use ./file`: the current file's directory and the cycle-detection
    // stack — THREAD-LOCAL (not an Interp field). `par` calls each lambda in a
    // separate thread, so parallel module loading must not corrupt each other's
    // base / in-flight stack. The base defaults to the current working directory
    // (`set_base` pins it to the top-level file); `par` snapshots the parent
    // thread's base into the new thread. The loading stack starts empty on each
    // thread (each par lambda is an independent import chain). module_cache, by
    // contrast, is shared in Interp — a loaded module is shared.
    pub(crate) static CURRENT_BASE: std::cell::RefCell<PathBuf> =
        std::cell::RefCell::new(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    pub(crate) static MODULE_LOADING: std::cell::RefCell<Vec<PathBuf>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

// RAII guard: bumps the counter on enter, decrements on Drop. Without Drop, on
// an error (`?`) or panic path the counter would stay elevated and poison
// subsequent requests once the spawn_blocking thread is reused.
pub(crate) struct CallDepthGuard;

impl CallDepthGuard {
    pub(crate) fn enter(fname: &str) -> Result<CallDepthGuard, Flow> {
        CALL_DEPTH.with(|d| {
            let depth = d.get();
            if depth >= MAX_CALL_DEPTH {
                return Err(Flow::err(format!(
                    "recursion too deep: '{}' call reached the {} level limit",
                    fname, MAX_CALL_DEPTH
                )));
            }
            d.set(depth + 1);
            Ok(CallDepthGuard)
        })
    }
}

impl Drop for CallDepthGuard {
    fn drop(&mut self) {
        CALL_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}
