// Integration tests for the Fluxon runtime, split by topic from the old
// monolithic `mod tests` in main.rs (refactor #184). Each submodule holds
// the tests for one area; shared helpers live here in the parent.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

// Small helper: runs the source, panics on error.
fn run(src: &str) {
    run_source(src).unwrap_or_else(|e| panic!("error: {}", e));
}

// DATABASE_URL is a global env var — to avoid a race between setting it and
// immediately running, we SERIALIZE the db tests with a global mutex. While the
// guard is held no other db test changes the env. Each test uses a SEPARATELY
// named shared-cache memory DB (the pool opens several connections -> shared-cache
// is required; a unique name -> tests do not see each other).
static DB_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn with_db_test(name: &str, body: impl FnOnce()) {
    let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let url = format!("sqlite:file:{name}?mode=memory&cache=shared");
    // SAFETY: the guard is held — only one db test sets the env at a time.
    unsafe { std::env::set_var("DATABASE_URL", &url) };
    body();
}

// Helper for migration tests: prepares a file-backed temp DB (two SEPARATE
// Interps = two deploy cycles; a memory DB is gone on the first drop).
// Returns the path; call `cleanup_db` at the end.
#[cfg(test)]
fn setup_db(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(name);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    // SAFETY: the caller holds DB_TEST_LOCK.
    unsafe {
        std::env::set_var("DATABASE_URL", format!("sqlite:{}", path.display()));
    }
    path
}

#[cfg(test)]
fn cleanup_db(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

// A unique temporary directory — so parallel tests do not collide
// (process id + an atomic counter). Test files are written here.
fn temp_module_dir() -> std::path::PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("fluxon_mod_test_{}_{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// Writes `files` ([(name, source), ...]) into `dir`, runs the first one, and
// returns the result. Cleans up the directory when done.
fn run_modules(files: &[(&str, &str)]) -> Result<(), String> {
    let dir = temp_module_dir();
    for (name, src) in files {
        // The file name may include a subdirectory ("sub/test.fx") — a directory
        // hierarchy is needed to test `../` (parent directory) module paths.
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, src).unwrap();
    }
    let main_path = dir.join(files[0].0);
    let src = std::fs::read_to_string(&main_path).unwrap();
    let r = run_source_at(&src, &main_path);
    let _ = std::fs::remove_dir_all(&dir);
    r
}

// Like `run_modules`, but `check`s the first file (without executing) — used to
// verify that `fluxon check` recursively validates imported user modules.
fn check_modules(files: &[(&str, &str)]) -> Result<(), String> {
    let dir = temp_module_dir();
    for (name, src) in files {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, src).unwrap();
    }
    let main_path = dir.join(files[0].0);
    let src = std::fs::read_to_string(&main_path).unwrap();
    let r = check_source_at(&src, &main_path);
    let _ = std::fs::remove_dir_all(&dir);
    r
}

// Issue #138: the REPL runs one block and returns the last expression's VALUE
// (to print). `run` returns () — this difference lets the REPL show the result.
// lex_parse + run_repl_chunk is exactly how the REPL works.
fn repl_chunk(interp: &interp::Interp, src: &str) -> Result<value::Value, String> {
    interp.run_repl_chunk(&lex_parse(src)?)
}

mod ai_sh_tests;
mod auth_tests;
mod bind_tests;
mod block_str_tests;
mod bytes_tests;
mod call_tests;
mod cron_tests;
mod crypto_tests;
mod db_tests;
mod db_tx_tests;
mod env_json_reg_tests;
mod framework_tests;
mod interp_err_tests;
mod list_tests;
mod log_tests;
mod map_tests;
mod math_each_tests;
mod migrate_tests;
mod par_tests;
mod pipe_fail_tests;
mod queue_tests;
mod range_tests;
mod rep_tests;
mod repl_tests;
mod str_order_tests;
mod str_time_tests;
mod sym_tests;
mod try_tests;
mod tui_tests;
mod use_module_tests;
