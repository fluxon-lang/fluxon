// Fluxon runtime — command-line interface.
//
// Usage:
//   fluxon run <file.fx>     — runs a Fluxon file
//   fluxon <file.fx>         — same (shorthand)
//   fluxon check <file.fx>   — lex+parse only (does not run); parse error -> exit 2
//   fluxon test [path]       — runs test files (default: tests/);
//                              path may be a file or a directory
//   fluxon repl              — interactive REPL (read-eval-print); with no args
//                              `fluxon` also opens this REPL
//   fluxon --version         — prints the built package version
//   fluxon --help            — prints the usage guide

// mimalloc — gives much less contention than the system malloc under parallelism.
// The interpreter makes many short-lived scope allocations (tree-walking).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod ai_mod;
mod ast;
mod auth_mod;
mod builtins;
mod cron_mod;
mod crypto_mod;
mod db_mod;
mod http_mod;
mod interp;
mod lexer;
mod par_mod;
mod parser;
mod queue_mod;
mod reg_mod;
mod serve_mod;
mod token;
mod value;
mod ws_mod;

use std::process::ExitCode;

// Command type: `run` executes code, `check` only validates the syntax.
// Exit codes are intentionally distinct: file unreadable/runtime error -> 1,
// usage/parse error -> 2 (the caller knows at which stage it failed).
enum Command {
    Run(String),
    Check(String),
    // test: path is optional — if omitted, the default `tests/` directory is used.
    Test(Option<String>),
    // repl: interactive read-eval-print session (no argument).
    Repl,
    Version,
    Help,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let cmd = match parse_args(&args) {
        Some(c) => c,
        None => {
            eprintln!("{}", usage());
            return ExitCode::from(2);
        }
    };

    match cmd {
        // run: LEX -> PARSE -> EXECUTE. Error (parse or runtime) -> exit 1.
        Command::Run(path) => {
            let src = match read_source(&path) {
                Ok(s) => s,
                Err(code) => return code,
            };
            match run_source_at(&src, std::path::Path::new(&path)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("Fluxon error: {}", e);
                    ExitCode::from(1)
                }
            }
        }
        // check: LEX + PARSE only (NO interp -> no side effects). Forge
        // eval-gate LAYER 1: is the AI-written block syntactically valid, without running it.
        // Parse/lex error -> exit 2 (distinct from runtime exit 1).
        Command::Check(path) => {
            let src = match read_source(&path) {
                Ok(s) => s,
                Err(code) => return code,
            };
            match check_source(&src) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("Fluxon error: {}", e);
                    ExitCode::from(2)
                }
            }
        }
        // test: does not read a single file — it discovers files and runs them.
        Command::Test(path) => run_tests(path.as_deref()),
        // repl: reads from stdin and runs each block (interactive session).
        Command::Repl => run_repl(),
        Command::Version => {
            println!("fluxon {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Help => {
            println!("{}", usage());
            ExitCode::SUCCESS
        }
    }
}

fn usage() -> &'static str {
    "Usage: fluxon run <file.fx>  |  fluxon check <file.fx>  |  fluxon test [path]  |  fluxon repl  |  fluxon --version  |  fluxon --help"
}

// Reads a file; on error prints the message and returns the exit code (1).
fn read_source(path: &str) -> Result<String, ExitCode> {
    std::fs::read_to_string(path).map_err(|e| {
        eprintln!("Could not read file '{}': {}", path, e);
        ExitCode::from(1)
    })
}

fn parse_args(args: &[String]) -> Option<Command> {
    match args.get(1).map(|s| s.as_str()) {
        Some("run") => args.get(2).cloned().map(Command::Run),
        Some("check") => args.get(2).cloned().map(Command::Check),
        Some("test") => Some(Command::Test(args.get(2).cloned())),
        Some("repl") => Some(Command::Repl),
        Some("--version" | "-V") => Some(Command::Version),
        Some("--help" | "-h") => Some(Command::Help),
        Some(p) if !p.starts_with('-') => Some(Command::Run(p.to_string())),
        // `fluxon` with no args — interactive REPL (so a human can try the language quickly).
        None => Some(Command::Repl),
        _ => None,
    }
}

// `fluxon test [path]` — issue #136. If no path is given, the `tests/` directory
// in the current directory is used; a file given runs only that file, a directory
// runs all .fx files inside (recursive, ordered by name). Each file runs in its own
// interp: finishing without error is PASS, an error (including an assert failure) is
// FAIL — the remaining files run regardless. Exit: 0 if all pass, 1 if any FAIL,
// 2 if the path/file is not found (matches the run/check exit convention).
fn run_tests(path: Option<&str>) -> ExitCode {
    let target = std::path::Path::new(path.unwrap_or("tests"));
    let files = match collect_test_files(target) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(2);
        }
    };

    let (passed, failed) = run_test_files(&files);
    println!(
        "SUMMARY: {} PASS, {} FAIL ({} files)",
        passed,
        failed,
        files.len()
    );
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

// Builds the list of test files. File -> itself; directory -> the .fx files inside.
// An empty list is also an error: silently reporting "tests passed" would mislead.
fn collect_test_files(target: &std::path::Path) -> Result<Vec<std::path::PathBuf>, String> {
    let files = if target.is_file() {
        // A non-.fx file (codex P2): parsing it as Fluxon and giving FAIL/exit 1
        // would mislead — this is a discovery error (exit 2), not a test result.
        if target.extension().is_none_or(|e| e != "fx") {
            return Err(format!("'{}' is not a .fx file", target.display()));
        }
        vec![target.to_path_buf()]
    } else if target.is_dir() {
        let mut v = Vec::new();
        collect_fx_files(target, &mut v)?;
        v.sort();
        v
    } else {
        return Err(format!("Test path not found: '{}'", target.display()));
    };
    if files.is_empty() {
        return Err(format!("no .fx test file found in '{}'", target.display()));
    }
    Ok(files)
}

// IO errors are not swallowed (codex P2): if an unreadable subdirectory were
// silently skipped, an "all passed" report would actually hide undiscovered tests.
fn collect_fx_files(
    dir: &std::path::Path,
    out: &mut Vec<std::path::PathBuf>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("could not read directory '{}': {}", dir.display(), e))?;
    for entry in entries {
        let entry =
            entry.map_err(|e| format!("could not read inside '{}': {}", dir.display(), e))?;
        let p = entry.path();
        // file_type() does NOT follow the symlink — so a looping symlink (tests/x -> tests/)
        // cannot cause infinite recursion. A symlinked .fx file still gets included
        // (the is_file below follows the symlink), but a symlinked directory does not.
        let ft = entry
            .file_type()
            .map_err(|e| format!("could not determine type of '{}': {}", p.display(), e))?;
        if ft.is_dir() {
            collect_fx_files(&p, out)?;
        } else if p.extension().is_some_and(|e| e == "fx") && p.is_file() {
            out.push(p);
        }
    }
    Ok(())
}

// Runs the files sequentially, returns the (PASS, FAIL) counts. The assert
// counter is reset before each file — so "N asserts" reflects that file's own
// count (the files run sequentially in a single process).
fn run_test_files(files: &[std::path::PathBuf]) -> (usize, usize) {
    let mut passed = 0usize;
    let mut failed = 0usize;
    for f in files {
        builtins::assert_passed_reset();
        let result = std::fs::read_to_string(f)
            .map_err(|e| format!("could not read file: {}", e))
            .and_then(|src| run_source_at(&src, f));
        match result {
            Ok(()) => {
                println!(
                    "PASS {} ({} assert)",
                    f.display(),
                    builtins::assert_passed()
                );
                passed += 1;
            }
            Err(e) => {
                println!("FAIL {} — {}", f.display(), e);
                failed += 1;
            }
        }
    }
    (passed, failed)
}

// Checks the syntax: lex + parse, but skips interp — the code is NOT executed
// (no side effects). On success Ok(()), otherwise the error text.
fn check_source(src: &str) -> Result<(), String> {
    let toks = lexer::lex(src)?;
    parser::parse(toks)?;
    Ok(())
}

// Runs the source. `path` is the file's path; `use ./file` modules are resolved
// relative to this file's directory.
fn run_source_at(src: &str, path: &std::path::Path) -> Result<(), String> {
    let toks = lexer::lex(src)?;
    let prog = parser::parse(toks)?;
    // Arc<Interp>: http.serve applies handlers on server threads, so the interp
    // must be shareable across threads.
    let interp = interp::Interp::new_arc();
    // base for `use ./file` — the directory of the top-level file.
    if let Some(dir) = path.parent() {
        // If parent() is empty (""), the current directory (default) stays.
        if !dir.as_os_str().is_empty() {
            interp.set_base(dir);
        }
    }
    interp.run(&prog)
}

// Convenience wrapper without a path — for tests (module paths relative to the current dir).
#[cfg(test)]
fn run_source(src: &str) -> Result<(), String> {
    run_source_at(src, std::path::Path::new("."))
}

// `fluxon repl` — interactive read-eval-print (issue #138). So a human can learn
// the language and try a line or two of code without a file. A single interp object
// lives for the whole session: `x = 1` is visible on the next line.
//
// Multi-line block: an indented construct (if/each/fn/...) spans several lines.
// We detect where the block ends not by analyzing the error text, but by re-parsing
// the accumulated buffer — if parse passes the block is complete (eval); if parse
// fails we wait for more lines, but if the user enters an EMPTY line (forcing the
// block closed) we show the error and clear the buffer. This avoids coupling to the
// inner text of lex/parse error messages (which would be brittle).
fn run_repl() -> ExitCode {
    use std::io::Write;

    let interp = interp::Interp::new_arc();
    // In the REPL, resolve `use ./file` relative to the current working directory.
    interp.set_base(std::path::Path::new("."));

    println!(
        "Fluxon {} REPL — exit: :q or Ctrl-D, help: :help",
        env!("CARGO_PKG_VERSION")
    );

    let stdin = std::io::stdin();
    // Accumulating buffer (for a multi-line block) — lines are joined with `\n`.
    let mut buf = String::new();

    loop {
        // Main prompt on an empty buffer, `...` prompt while a block continues.
        let prompt = if buf.is_empty() { "fx> " } else { "... " };
        print!("{}", prompt);
        // print! has no trailing newline — force a flush so the prompt is visible.
        let _ = std::io::stdout().flush();

        let mut line = String::new();
        match stdin.read_line(&mut line) {
            Ok(0) => {
                // EOF (Ctrl-D). If a partially accumulated block exists, try it one
                // last time, then exit on a new line.
                println!();
                if !buf.trim().is_empty() {
                    repl_eval(&interp, &buf);
                }
                return ExitCode::SUCCESS;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("REPL read error: {}", e);
                return ExitCode::from(1);
            }
        }

        // read_line keeps the trailing `\n` — strip the trailing newline, then check for emptiness.
        let trimmed = line.trim_end_matches(['\n', '\r']);

        // REPL commands only when the buffer is empty (not in the middle of a block).
        if buf.is_empty() {
            match trimmed.trim() {
                ":q" | ":quit" | ":exit" => return ExitCode::SUCCESS,
                ":help" | ":h" => {
                    print_repl_help();
                    continue;
                }
                "" => continue, // empty prompt — do nothing
                _ => {}
            }
        }

        // Empty line + an accumulating block: the user forced the block closed —
        // we eval what we have now (if parse still fails, the error is shown).
        let force_eval = !buf.is_empty() && trimmed.trim().is_empty();

        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(trimmed);

        // We do NOT immediately eval an indented (multi-line) block — `else`/`catch`
        // or a continuation may come on the next line (e.g. `if`+body parses, but
        // `else` is not there yet). Such a block is closed only by an empty line (force_eval).
        // An unindented single-line expression, however, is evaluated as soon as it parses
        // (we do not require an empty line for a simple `1 + 2`).
        let ready = if force_eval {
            true
        } else if is_multiline_block(&buf) {
            false
        } else {
            check_source(&buf).is_ok()
        };

        if ready {
            repl_eval(&interp, &buf);
            buf.clear();
        }
    }
}

// Is the buffer an indented block (does any line start with whitespace)? If so,
// the REPL waits for a continuation (else/catch/extra body may come) and only an
// empty line closes the block. The first line is never indented — so it is enough
// if any of the following lines is indented.
fn is_multiline_block(buf: &str) -> bool {
    buf.lines()
        .any(|l| l.starts_with(' ') || l.starts_with('\t'))
}

// Runs the buffer contents and prints the result. On error, "Fluxon error: ..."
// to stderr — the session does not end (it moves on to the next prompt).
fn repl_eval(interp: &interp::Interp, src: &str) {
    match interp.run_repl_chunk(&match lex_parse(src) {
        Ok(prog) => prog,
        Err(e) => {
            eprintln!("Fluxon error: {}", e);
            return;
        }
    }) {
        // We do not print a Nil result (declaration, log, assign) — it would be noise.
        // Any other value we show via `repr` (strings quoted).
        Ok(value::Value::Nil) => {}
        Ok(v) => println!("{}", v.repr()),
        Err(e) => eprintln!("Fluxon error: {}", e),
    }
}

// lex+parse for the REPL — same as inside `run_source_at`, but returns the AST
// (to pass to the REPL chunk).
fn lex_parse(src: &str) -> Result<ast::Program, String> {
    let toks = lexer::lex(src)?;
    parser::parse(toks)
}

fn print_repl_help() {
    println!(
        "REPL commands:\n  \
         :help, :h    — this help\n  \
         :q, :quit    — exit (also Ctrl-D)\n\
         Type one line of code and press Enter. If blocks like if/each/fn span\n\
         multiple lines, keep typing until the block is complete; an empty line\n\
         closes the block. The result (unless nil) is printed automatically."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // Small helper: runs the source, panics on error.
    fn run(src: &str) {
        run_source(src).unwrap_or_else(|e| panic!("error: {}", e));
    }

    // Issue #139: leveled log — `log.debug/info/warn/err` and bare `log` (=info)
    // are wired into dispatch and work without error (writes to stderr, returns nil).
    // Filter/format via $LOG_LEVEL/$LOG_FORMAT; the format logic is unit-tested in
    // builtins::log_tests. Here only syntax/dispatch is checked.
    #[test]
    fn log_darajalari() {
        run(r#"
log "bare = info"
log.debug "tafsilot"
log.info "info"
log.warn "ogohlantirish"
log.err "error"
log.info "interpolatsiya ${1 + 1}"
"#);
    }

    // Issue #139: `log` continues to work as a value (callback/storage) —
    // compatible with the old global `log` Native (an info-level shim). PR #163 review.
    #[test]
    fn log_qiymat_sifatida_callback() {
        run(r#"
fn call f -> f "by value"
call log
[1, 2, 3].map log
g = log
g "saved function"
"#);
    }

    // Issue #137: par — language-level parallel fan-out. Takes a list of lambdas,
    // calls each one on its own thread, waits for all, results (in input order) are
    // each {ok:...} or {err:...}.
    // Note: lambda elements inside a list are separated by PARENS — `(\-> ...)`.
    // The lexer does not emit a Newline token inside a list/map (`paren_depth>0`), so
    // without parens `[\-> a  \-> b]` the first body would swallow the second as an
    // argument; the parens delimit the body and a nested HOF (`\-> xs.map \x ->`)
    // is not broken either (issue #137 PR review, P2).
    #[test]
    fn par_asosiy_fan_out() {
        run(r#"
r = par [
  (\-> 1 + 1)
  (\-> str.up "hi")
  (\-> [1 2 3].len)
]
((r.len) == 3) | (fail "par 3 results should be returned")
((r.0.ok) == 2) | (fail "1st result should be {ok:2}")
((r.1.ok) == "HI") | (fail "2nd result should be {ok:HI}")
((r.2.ok) == 3) | (fail "3rd result should be {ok:3}")
"#);
    }

    // Issue #137: partial success — if one lambda fails the others do not stop;
    // the error comes back as {err:message}, order is preserved.
    #[test]
    fn par_qisman_muvaffaqiyat() {
        run(r#"
r = par [
  (\-> 42)
  (\-> fail "on purpose")
  (\-> "third")
]
((r.0.ok) == 42) | (fail "1st result ok should be set")
((r.1.err) == "on purpose") | (fail "2nd result should be err")
((r.2.ok) == "third") | (fail "3rd result ok should be set")
"#);
    }

    // Issue #137: a closure can read an outer (loop/scope) variable in parallel.
    #[test]
    fn par_closure_capture() {
        run(r#"
base = 100
r = par [(\-> base + 1) (\-> base + 2)]
((r.0.ok) == 101) | (fail "closure capture 1 broke")
((r.1.ok) == 102) | (fail "closure capture 2 broke")
"#);
    }

    // Issue #137: a nested paren-free HOF inside a lambda body (`xs.map \x -> ...`)
    // is read in full inside the parens — no P2 regression.
    #[test]
    fn par_nested_hof() {
        run(r#"
r = par [(\-> [1 2 3].map \x -> x + 1)]
((r.0.ok.0) == 2) | (fail "nested HOF 1-element broke")
((r.0.ok.2) == 4) | (fail "nested HOF 3-element broke")
"#);
    }

    // Issue #137: empty list -> empty result (no thread is spawned).
    #[test]
    fn par_bosh_royxat() {
        run(r#"
r = par []
((r.len) == 0) | (fail "par [] should return an empty list")
"#);
    }

    // Issue #137: a non-lambda element gives a clear error (without spawning a thread).
    #[test]
    fn par_lambda_bolmagan_element_xato() {
        let e = run_source("par [42]").unwrap_err();
        assert!(
            e.contains("must be a function"),
            "a clear error is expected for a non-lambda par element, got: {}",
            e
        );
    }

    // Issue #137 (PR review P2): if two par lambdas `use ./m` the same UNCACHED
    // module in parallel, both must return {ok:...} — not a false "circular import".
    // Because module_loading/current_base are thread-local, parallel imports do not
    // see each other as a cycle and the base is not corrupted.
    #[test]
    fn par_parallel_modul_import_soxta_sikl_yoq() {
        let dir = std::env::temp_dir().join(format!("fluxon_par_mod_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("m.fx"), "exp fn greet n -> \"hello ${n}\"\n").unwrap();
        let main = dir.join("main.fx");
        // Each lambda imports the MODULE FOR THE FIRST TIME on its own thread
        // (cache empty) — Codex reproduction.
        std::fs::write(
            &main,
            r#"
fn load n
  use ./m
  ret m.greet n
r = par [
  (\-> load 1)
  (\-> load 2)
]
((r.0.ok) == "hello 1") | (fail "par module import 1 broke: ${r.0}")
((r.1.ok) == "hello 2") | (fail "par module import 2 broke: ${r.1}")
"#,
        )
        .unwrap();
        let src = std::fs::read_to_string(&main).unwrap();
        let res = run_source_at(&src, &main);
        let _ = std::fs::remove_dir_all(&dir);
        res.unwrap_or_else(|e| panic!("par parallel module import error: {}", e));
    }

    // Issue #137: if the user declares a variable named `par`, it wins
    // (shadowing consistent with the other dispatch batteries).
    #[test]
    fn par_ozgaruvchi_sifatida_shadow() {
        run(r#"
fn id v -> v
par = (id 7)
(par == 7) | (fail "par did not shadow as a variable")
"#);
    }

    // Issue #137 (PR review P1): calling par from inside db.tx gives a clear error —
    // the new threads do not inherit the CURRENT_TX TLS, so instead of silently
    // running outside the tx, it is rejected. (DB test — DB_TEST_LOCK.)
    #[test]
    fn par_db_tx_ichida_rad_etiladi() {
        with_db_test("par_in_tx", || {
            let e = run_source(
                r#"
use db
tbl t
  id serial pk
db.tx \->
  par [(\-> 1)]
"#,
            )
            .unwrap_err();
            assert!(
                e.contains("cannot be used inside db.tx"),
                "a clear error is expected for par inside db.tx, got: {}",
                e
            );
        });
    }

    // Issue #139: if the user declares a variable named `log`, it wins
    // (shadows the battery) — the old shadowing invariant is not broken.
    #[test]
    fn log_ozgaruvchi_sifatida_shadow() {
        run(r#"
fn log_id v -> v
log = (log_id 42)
(log == 42) | (fail "log did not shadow as a variable")
"#);
    }

    // Issue #93: in `log !x` the `!` used to stick to the callee as a postfix Try —
    // `Call(Try(log), [x])` — the negation silently disappeared. Now a `!` after
    // whitespace starts an argument as a prefix not.
    #[test]
    fn chaqiruv_argumentida_prefiks_not() {
        run(r#"
x = false
(!x) | (fail "parenthesized prefix not broke")
fn id v -> v
((id !x) == true) | (fail "in call argument !x was not negated")
y = true
((id !y) == false) | (fail "in call argument !y was not negated")
fn second a b -> b
((second x !y) == false) | (fail "prefix not in the second argument broke")
"#);
    }

    // Issue #93 (regression guard): an attached `!` is still a postfix Try as before —
    // it sticks to the value and stays a passthrough on success.
    #[test]
    fn tutash_bang_postfix_try_qoladi() {
        run(r#"
fn safe v -> v
a = (safe 5)!
(a == 5) | (fail "postfix try passthrough broke")
"#);
    }

    // Issue #125: try/catch — catches an error raised by `fail` and continues
    // as a value. The catch variable binds to a {message, status} map.
    #[test]
    fn try_catch_fail_statusli_ushlaydi() {
        run(r#"
r = try
  fail 422 "invalid data"
catch e
  (e.message == "invalid data") | (fail "catch message broke")
  (e.status == 422) | (fail "catch status broke")
  "fallback"
(r == "fallback") | (fail "catch body value did not return")
"#);
    }

    // Status-less fail and a runtime error — both are caught; without a status
    // e.status is nil.
    #[test]
    fn try_catch_runtime_xato_va_statussiz() {
        run(r#"
r = try
  fail "boom"
catch e
  (e.status == nil) | (fail "status should be nil for fail without status should be")
  e.message
(r == "boom") | (fail "fail message without status was not caught")

# runtime errors (divide by zero) are caught too
r2 = try
  1 / 0
catch e
  (e.status == nil) | (fail "runtime error status should be nil should be")
  "ushlandi"
(r2 == "ushlandi") | (fail "runtime error was not caught")
"#);
    }

    // On success the body's last expression returns; catch does not run.
    #[test]
    fn try_catch_muvaffaqiyatda_body_qiymati() {
        run(r#"
r = try
  40 + 2
catch
  0
(r == 42) | (fail "body value on success did not return")
"#);
    }

    // ret/skip/stop flow signals pass through try — catch does not catch them.
    #[test]
    fn try_catch_oqim_signallarini_ushlamaydi() {
        run(r#"
fn f
  try
    ret "early"
  catch
    ret "caught"
((f()) == "early") | (fail "ret from inside try was caught (wrong)")

total <- 0
each i in 1..5
  try
    if i == 3
      skip
    if i == 5
      stop
    total <- total + i
  catch
    fail "skip/stop should not be caught"
(total == 7) | (fail "skip/stop try ichida broke: ${total}")
"#);
    }

    // Nested try, and a re-fail (re-raise) from inside catch goes to the outer try.
    #[test]
    fn try_catch_ichmaich_va_qayta_fail() {
        run(r#"
r = try
  try
    fail "inner"
  catch e
    fail "outer: ${e.message}"
catch e
  e.message
(r == "outer: inner") | (fail "nested try or re-fail broke")
"#);
    }

    // Issue #90: infinite recursion must return a graceful runtime error instead
    // of a stack overflow ABORT (so an HTTP handler does not kill the whole server).
    #[test]
    fn cheksiz_rekursiya_graceful_xato() {
        let e = run_source("fn f n -> f (n + 1)\nf 0").unwrap_err();
        assert!(e.contains("recursion too deep"), "unexpected error: {}", e);
    }

    // Issue #90: after a limit error the depth counter fully resets —
    // the next execution on the same thread starts clean (RAII guard).
    #[test]
    fn rekursiya_limitdan_keyin_tiklanish() {
        assert!(run_source("fn f n -> f (n + 1)\nf 0").is_err());
        run(r#"
fn g x -> x + 1
((g 1) == 2) | (fail "call after the limit broke")
"#);
    }

    // Issue #90: ~2000 nested parens used to abort the parser with a stack overflow.
    // Now exceeding the limit (256) is a clear parse error; 200 levels still work.
    #[test]
    fn chuqur_qavs_parse_limiti() {
        let deep = format!("x = {}1{}", "(".repeat(300), ")".repeat(300));
        let e = check_source(&deep).unwrap_err();
        assert!(e.contains("too deep"), "unexpected error: {}", e);

        let ok = format!("x = {}1{}", "(".repeat(200), ")".repeat(200));
        check_source(&ok).unwrap_or_else(|e| panic!("200 levels should pass: {}", e));
    }

    // Issue #89: on int arithmetic overflow, instead of a panic (debug) / silent
    // wrap (release), both modes return the same Fluxon error.
    #[test]
    fn int_overflow_xato_panic_emas() {
        // + overflow (used to panic in debug)
        let e = run_source("log (9223372036854775806 + 2)").unwrap_err();
        assert!(e.contains("number out of range"), "unexpected error: {}", e);
        // i64::MIN / -1 — used to panic even in release in Rust
        let e = run_source(
            r#"
a = 0 - 9223372036854775807 - 1
log (a / (0 - 1))
"#,
        )
        .unwrap_err();
        assert!(e.contains("number out of range"), "unexpected error: {}", e);
        // i64::MIN % -1 — same family
        let e = run_source(
            r#"
a = 0 - 9223372036854775807 - 1
log (a % (0 - 1))
"#,
        )
        .unwrap_err();
        assert!(e.contains("number out of range"), "unexpected error: {}", e);
        // unary minus too: -(i64::MIN) does not fit
        let e = run_source(
            r#"
a = 0 - 9223372036854775807 - 1
log (-a)
"#,
        )
        .unwrap_err();
        assert!(e.contains("number out of range"), "unexpected error: {}", e);
        // * and - are checked too
        assert!(run_source("log (4611686018427387904 * 2)").is_err());
        assert!(run_source("log (0 - 9223372036854775807 - 2)").is_err());
        // Ordinary arithmetic works as before
        run(r#"
((2 + 3) == 5) | (fail "sum broke")
((7 / 2) == 3) | (fail "division broke")
((7 % 2) == 1) | (fail "mod broke")
((-(5)) == (0 - 5)) | (fail "unary minus broke")
"#);
    }

    // Issue #89: when the range end was i64::MAX, `i += 1` used to overflow —
    // now it stops after the last element.
    #[test]
    fn range_i64_max_chegarasida_toxtaydi() {
        run(r#"
m = 9223372036854775806
r = m..(m + 1)
(r.len == 2) | (fail "range length wrong: ${r.len}")
"#);
    }

    #[test]
    fn fib_recursion() {
        run(r#"
fn fib n
  if n < 2
    ret n
  (fib (n - 1)) + (fib (n - 2))

each i in 0..10
  log "fib ${i} = ${fib i}"
"#);
    }

    // Issue #99: `..` binds LOWER than arithmetic, but HIGHER than pipe/comparison.
    // `1..n+1` = `1..(n+1)` (natural for AI), it used to be `(1..n)+1` and gave a
    // runtime error. Pipe, on the other hand, wraps the whole range.
    #[test]
    fn range_ustuvorligi() {
        run(r#"
n = 3
# end side: +1 applies only to n, not the whole range
(1..n+1 == [1 2 3 4]) | (fail "1..n+1 wrong")
# end side: -1
(0..n-1 == [0 1 2]) | (fail "0..n-1 wrong")
# arithmetic on both sides
(2*1..2+1 == [2 3]) | (fail "2*1..2+1 wrong")
# works inside an each loop too, without error
sum <- 0
each i in 1..n+1
  sum <- sum + i
(sum == 10) | (fail "each 1..n+1 sum wrong: ${sum}")
"#);
    }

    // Issue #99 (review): pipe binds LOWER than range, so
    // `1..3 |> f` = `(1..3) |> f` — the built range is passed to f, without parens.
    #[test]
    fn range_pipe_butun_diapazonni_uzatadi() {
        run(r#"
fn total xs
  xs.reduce 0 \acc x -> acc + x
# pipe applies to the whole range (1..3 = [1 2 3]), not the end side
(1..3 |> total == 6) | (fail "pipe range wrong")
"#);
    }

    // Inline if (ternary equivalent): `if cond a else b` returns a single value.
    // Issue #66 — a compact conditional expression (for places like leading-zero formatting).
    #[test]
    fn inline_if_ifoda() {
        run(r#"
# the main example from the issue: leading-zero formatting
h = 5
pad = if h < 10 ("0" + str.str h) else (str.str h)
(pad == "05") | (fail "inline if value did not give: ${pad}")

# else branch when the condition is false
x = 20
pad2 = if x < 10 ("0" + str.str x) else (str.str x)
(pad2 == "20") | (fail "else branch did not work: ${pad2}")

# simple branches without parens
y = if h > 3 "big" else "small"
(y == "big") | (fail "branch without parens did not work: ${y}")

# else-if chain (nested inline if)
g = if h == 0 "zero" else if h < 0 "negative" else "positive"
(g == "positive") | (fail "else-if chain did not work: ${g}")

# a call as the condition, inside parens
s = "hi"
r = if (str.len s) > 0 "full" else "empty"
(r == "full") | (fail "parenthesized condition did not work: ${r}")

# using it inside a larger expression
n = 7
msg = "son " + (if n % 2 == 0 "juft" else "toq")
(msg == "son toq") | (fail "inner inline if did not work: ${msg}")
"#);
    }

    // rep's optional 3rd-argument headers map (issue #16). rep simply returns a
    // {__resp:true status body headers} map — we read and check its keys in Fluxon
    // (actual header writing is in the http_mod tests).
    #[test]
    fn rep_headers_argumenti() {
        run(r#"
# 2-argument (old form) — no headers key
r = rep 200 {ok:true}
(r.status == 200) | (fail "rep status broke: ${r.status}")
(r.headers == nil) | (fail "headers key appeared in rep without headers")

# 3-argument — a headers map is added. Use `_` instead of a dash (a map key
# cannot contain a dash; on write the runtime turns `_` into `-`). Read with `_` too.
r2 = rep 200 "<h1>Salom</h1>" {content_type:"text/html"}
(r2.headers.content_type == "text/html") | (fail "headers could not be read")

# body map + separate headers — they do not collide
r3 = rep 200 {data:1} {set_cookie:"s=abc"}
(r3.body.data == 1) | (fail "body map broke")
(r3.headers.set_cookie == "s=abc") | (fail "set-cookie could not be read")
"#);
    }

    // If the 3rd argument is not a map, rep gives a clear error (not silent disregard).
    #[test]
    fn rep_headers_nomap_xato() {
        let e = run_source(r#"x = rep 200 "body" "notmap""#).unwrap_err();
        assert!(
            e.contains("3rd argument must be headers"),
            "unexpected error: {}",
            e
        );
    }

    // Even after the inline form was added, the block form (with a call condition)
    // must still work — regression check.
    #[test]
    fn blok_if_inline_qoshilgach_ishlaydi() {
        run(r#"
s = "hi"
out <- "none"
if str.len s > 0
  out <- "full"
else
  out <- "empty"
(out == "full") | (fail "block if broke: ${out}")
"#);
    }

    // Argument-less (nullary) call: `f()`. Since a paren-free call is defined by its
    // argument, this is the only way to call a 0-arity function.
    // `f` (paren-free) is the function VALUE, `f()` is a CALL.
    #[test]
    fn nullary_call() {
        run(r#"
fn new_id
  ret rand.str 8

a = new_id()
b = new_id()
(str.len a == 8) | (fail "new_id() was not called: ${a}")
(a != b) | (fail "each call did not give a new value")

# paren-free: the function value (not called) — boolean truthy
f = new_id
(f != nil) | (fail "bare name should be a function value")

# nullary lambda
g = \->
  ret 42
(g() == 42) | (fail "lambda nullary call did not work: ${g()}")
"#);
    }

    // Argument-less recursion: `tick()` calls itself. We used to be forced to add a
    // dummy argument (`tick n`) — now it is not required.
    #[test]
    fn nullary_recursion() {
        run(r#"
n <- 0
fn tick
  n <- n + 1
  if n < 3
    tick()
  ret n
(tick() == 3) | (fail "nullary recursion did not work: ${n}")
"#);
    }

    // `f(x)` (a parenthesized call with an argument) is REJECTED — the canonical form is `f x`.
    // Empty `()` is only for nullary; one task = one way.
    #[test]
    fn paren_call_with_arg_errors() {
        let err = run_source(
            r#"
fn g x
  ret x
g(5)
"#,
        )
        .expect_err("f(x) with parenthesized argument should error");
        assert!(err.contains("argument-less"), "unexpected error: {}", err);
    }

    #[test]
    fn list_methods() {
        run(r#"
nums = [1 2 3 4 5]
evens = nums.filter \x -> x % 2 == 0
doubled = evens.map \x -> x * 2
total = doubled.reduce 0 \acc x -> acc + x
log "evens=${evens} doubled=${doubled} total=${total}"
"#);
    }

    // list.index gives the position (-1 if not found), list.find gives the first
    // element matching the predicate (nil if not found). has is bool, index is the
    // position — a pair.
    #[test]
    fn list_index_and_find() {
        run(r#"
names = ["catalog_manager" "order_extractor" "billing"]
(names.index "order_extractor" == 1) | (fail "index did not find: ${names.index "order_extractor"}")
(names.index "yoq" == -1) | (fail "none element -1 did not give")

nums = [3 1 4 1 5 9]
(nums.index 4 == 2) | (fail "int index: ${nums.index 4}")

# find: the first element matching the predicate
big = nums.find \x -> x > 4
(big == 5) | (fail "find did not return the matching element: ${big}")
none = nums.find \x -> x > 99
(none == nil) | (fail "find should return nil when nothing matches: ${none}")

# using index for comparison (issue source: block order)
a = names.index "catalog_manager"
b = names.index "billing"
(a < b) | (fail "index comparison did not work: ${a} ${b}")
"#);
    }

    // Issue #127: list.sort — natural order without an argument (number/string),
    // arbitrary order with a comparator. The original list is unchanged (immutable values).
    #[test]
    fn list_sort() {
        run(r#"
nums = [3 1 4 1 5]
s = nums.sort
(s == [1 1 3 4 5]) | (fail "natural sort: ${s}")
(nums == [3 1 4 1 5]) | (fail "sort modified the original list: ${nums}")

# comparator: returns a number (negative: a first) — descending order
d = nums.sort \a b -> b - a
(d == [5 4 3 1 1]) | (fail "comparator sort: ${d}")

# strings sort lexicographically
names = ["banan" "olma" "anor"].sort
(names == ["anor" "banan" "olma"]) | (fail "str sort: ${names}")

# mixed int/flt numeric order
mixed = [2 1.5 1].sort
(mixed == [1 1.5 2]) | (fail "mixed number sort: ${mixed}")

# edge cases
([].sort == []) | (fail "empty list sort")
([7].sort == [7]) | (fail "single element sort")
"#);
    }

    // Issue #127: sort with a comparator is stable — equal elements keep their
    // original order (sorting map records gathered from several sources by a field).
    #[test]
    fn list_sort_stable_va_maplar() {
        run(r#"
items = [{n:"b" p:2} {n:"a" p:1} {n:"c" p:1}]
sorted = items.sort \a b -> a.p - b.p
ns = sorted.map \x -> x.n
(ns == ["a" "c" "b"]) | (fail "stable map sort: ${ns}")
"#);
    }

    // Issue #127: sort error paths — mixed types without a comparator, a comparator
    // that does not return a number, a zip argument that is not a list.
    #[test]
    fn list_sort_zip_xatolari() {
        let e = run_source(r#"x = [1 "a"].sort"#).unwrap_err();
        assert!(e.contains("cannot compare"), "unexpected error: {}", e);

        let e = run_source(r#"x = [1 2].sort \a b -> "x""#).unwrap_err();
        assert!(
            e.contains("must return a number"),
            "unexpected error: {}",
            e
        );

        let e = run_source("x = [1 2].zip 5").unwrap_err();
        assert!(e.contains("must be a list"), "unexpected error: {}", e);
    }

    // Issue #127: reverse/uniq/flat/zip — pure list methods.
    #[test]
    fn list_reverse_uniq_flat_zip() {
        run(r#"
([1 2 3].reverse == [3 2 1]) | (fail "reverse did not work")
([1 2 1 3 2].uniq == [1 2 3]) | (fail "uniq did not work")

# flat flattens one level; a non-list element stays as-is
([[1 2] [3] 4].flat == [1 2 3 4]) | (fail "flat did not work")

# zip stops when the shorter one runs out
z = [1 2 3].zip ["a" "b"]
(z == [[1 "a"] [2 "b"]]) | (fail "zip did not work: ${z}")
"#);
    }

    // Issue #127: any/all predicate methods — instead of the filter+len workaround.
    #[test]
    fn list_any_all() {
        run(r#"
nums = [1 2 3]
a1 = nums.any \x -> x > 2
a1 | (fail "any did not return true on a match")
a2 = nums.any \x -> x > 9
(a2 == false) | (fail "any did not return false without a match")

b1 = nums.all \x -> x > 0
b1 | (fail "all did not return true when all match")
b2 = nums.all \x -> x > 1
(b2 == false) | (fail "all did not return false on a mismatch")

# empty list: any false, all true (vacuous)
e1 = [].any \x -> x
(e1 == false) | (fail "empty any false not")
e2 = [].all \x -> x
e2 | (fail "empty all true not")
"#);
    }

    // Computed index: both `xs.(expr)` and `xs[expr]` must work.
    // Issue #64 — an expression index, not a literal, for pagination/getting the last element.
    #[test]
    fn hisoblangan_indeks() {
        run(r#"
xs = ["a" "b" "c"]
i = xs.len - 1

# .(expr) form — get the last element with a computed index
last = xs.(i)
(last == "c") | (fail ".(i) oxirgi elementni did not give: ${last}")

# a full expression inside
(xs.(xs.len - 1) == "c") | (fail "xs.(xs.len - 1) did not work")

# the bracket form gives the same result
(xs[i] == "c") | (fail "xs[i] did not work")

# indexing a map with a computed key (str)
m = {name: "Ali" age: 30}
k = "name"
(m.(k) == "Ali") | (fail "m.(k) did not work: ${m.(k)}")

# out of bounds -> nil (existing get_index behavior)
(xs.(99) == nil) | (fail "chegaradan tashqari indeks nil did not give")
"#);
    }

    #[test]
    fn map_operations() {
        run(r#"
u = {name:"Aziza" age:30}
u2 = u.set "age" 31
log "keys=${u.keys} hasName=${u.has "name"} age=${u2.age}"
"#);
    }

    // Issue #129: m.merge other — merges two maps (other wins).
    #[test]
    fn map_merge() {
        run(r#"
# the main pattern: default config + user override
defaults = {host:"localhost" port:8080 debug:false}
user = {port:3000 debug:true}
cfg = defaults.merge user

# keys from other win
(cfg.port == 3000) | (fail "merge: other key did not win: ${cfg.port}")
(cfg.debug == true) | (fail "merge: debug override did not happen")
# a key not present in other keeps its original value
(cfg.host == "localhost") | (fail "merge: host lost: ${cfg.host}")
(cfg.len == 3) | (fail "merge: key count wrong: ${cfg.len}")

# the original maps are unchanged (consistent with set/del — a new map is returned)
(defaults.port == 8080) | (fail "merge: original map changed")
((user.has "host") == false) | (fail "merge: other map changed")

# merge with an empty map — returns itself
((defaults.merge {}).len == 3) | (fail "merge: with empty map broke")
(({}.merge defaults).port == 8080) | (fail "merge: merge from empty map broke")
"#);
    }

    // map.merge with a non-map argument returns an understandable error.
    #[test]
    fn map_merge_notogri_argument() {
        let e = run_source(r#"({a:1}).merge 42"#).unwrap_err();
        assert!(e.contains("map.merge"), "unexpected error text: {}", e);
    }

    // A bare type name in a schema map's value position (`{a:str b:int}`) turns into
    // a sym — as the docs promise (`ai.json {product:str qty:int}`). Because `str` is
    // also a module name, it used to give an "unknown name: str" error.
    #[test]
    fn schema_bare_type_names() {
        run(r#"
schema = {product:str qty:int price:flt active:bool data:json tag:sym}
(schema.product == :str) | (fail "product :str not: ${schema.product}")
(schema.qty == :int) | (fail "qty :int not: ${schema.qty}")
(schema.price == :flt) | (fail "price :flt not")
(schema.active == :bool) | (fail "active :bool not")
(schema.data == :json) | (fail "data :json not")
(schema.tag == :sym) | (fail "tag :sym not")

# a map inside a nested list should work too (`{items:[{product:str qty:int}]}`)
nested = {items:[{product:str qty:int}]}
row = nested.items.0
(row.product == :str) | (fail "nested product :str not")
(row.qty == :int) | (fail "nested qty :int not")

# regression: an ident that is NOT a type name is still resolved as a variable
x = 5
m = {n:x}
(m.n == 5) | (fail "oddiy variable value broke: ${m.n}")

# regression: a str module call as a value is not broken
up = str.up "hello"
(up == "HELLO") | (fail "str.up broke: ${up}")
"#);
    }

    // Issue #98 — nested numeric index `m.0.1`. The lexer used to greedily swallow
    // `.1` as `Flt(0.1)` (not knowing it was in a `.` member context). Now a number
    // after a member index does not start a float: `m.0.1` ≡ `(m.0).1`.
    #[test]
    fn nested_numeric_index() {
        run(r#"
m = [[1 2] [3 4]]
(m.0.1 == 2) | (fail "m.0.1 != 2: ${m.0.1}")
(m.1.0 == 3) | (fail "m.1.0 != 3: ${m.1.0}")

# three-level nested index too
deep = [[[7 8]]]
(deep.0.0.1 == 8) | (fail "deep.0.0.1 != 8: ${deep.0.0.1}")

# regression: ordinary float literals are not broken
(0.5 + 0.5 == 1.0) | (fail "float literal broke")
fs = [0.5 1.5]
(fs.1 == 1.5) | (fail "float element broke: ${fs.1}")
"#);
    }

    #[test]
    fn mutable_and_each() {
        run(r#"
total <- 0
each n in [10 20 30]
  total <- total + n
log "total=${total}"
"#);
    }

    // if/each/match blocks are lexically TRANSPARENT: an inner `=` updates the outer
    // (same-fn) variable — like other languages, no clone is taken. This makes the
    // accumulator pattern natural (before, an `=` inside a block silently created a
    // new local -> the outer stayed nil).
    #[test]
    fn bind_in_block_updates_outer() {
        run(r#"
best <- nil
top <- 0
each e in [{n:"a" v:3} {n:"b" v:7} {n:"c" v:2}]
  if e.v > top
    top = e.v
    best = e
(top == 7) | (fail "top wrong: ${top}")
(best.n == "b") | (fail "best wrong: ${best.n}")
"#);
    }

    // Immutability is preserved: an outer `=` (immutable) variable cannot be
    // reassigned with `=` from inside a block either (a clear error — NOT a silent shadow).
    #[test]
    fn bind_in_block_immutable_errors() {
        let err = run_source(
            r#"
x = 10
if true
  x = 20
"#,
        )
        .expect_err("updating an immutable with = inside a block should error");
        assert!(err.contains("is immutable"), "unexpected error: {}", err);
    }

    // fn/lambda BOUNDARY: an inner `=` creates a new LOCAL, not the outer variable
    // (shadowing/isolation). The outer value is unchanged.
    #[test]
    fn bind_in_fn_shadows_not_mutates() {
        run(r#"
x = 100
f = \n ->
  x = 5
  x + n
(f 1 == 6) | (fail "fn local x did not work")
(x == 100) | (fail "= inside fn changed outer x: ${x}")
"#);
    }

    // `<-` (assign), however, CROSSES the fn boundary — closure capture is preserved
    // (`=` stops at the boundary, `<-` does not: the clear difference between them).
    #[test]
    fn assign_crosses_fn_boundary_capture() {
        run(r#"
counter <- 0
inc = \n ->
  counter <- counter + n
inc 5
inc 3
(counter == 8) | (fail "closure capture did not work: ${counter}")
"#);
    }

    #[test]
    fn match_symbols() {
        run(r#"
fn label s
  match s
    :new -> "new"
    :done -> "done"
    _ -> "other"

log (label :new)
log (label :x)
"#);
    }

    #[test]
    fn string_and_modules() {
        run(r#"
s = "Salom Dunyo"
log (str.up s)
log "len=${str.len s} floor=${math.floor 3.7}"
parts = str.split "a,b,c" ","
log "parts=${parts} joined=${parts.join "-"}"
"#);
    }

    // Issue #126: str.trim/replace/starts/ends/pad/repeat — the str functions every
    // real project needs on day one.
    #[test]
    fn str_trim_replace_starts_ends_pad_repeat() {
        run(r#"
(str.trim "  hello  " == "hello") | (fail "str.trim")
(str.trim "hello" == "hello") | (fail "str.trim unchanged")
(str.replace "a-b-c" "-" "+" == "a+b+c") | (fail "str.replace")
(str.replace "abc" "x" "y" == "abc") | (fail "str.replace not found")
(str.replace "abc" "" "y" == "abc") | (fail "str.replace empty pattern")
(str.starts "/api/users" "/api") | (fail "str.starts true")
((str.starts "/api" "/web") == false) | (fail "str.starts false")
(str.ends "file.fx" ".fx") | (fail "str.ends true")
((str.ends "file.fx" ".rs") == false) | (fail "str.ends false")
(str.pad "7" 3 "0" == "007") | (fail "str.pad")
(str.pad "1234" 3 "0" == "1234") | (fail "str.pad long unchanged")
(str.pad "ab" 4 " " == "  ab") | (fail "str.pad whitespace")
(str.repeat "ab" 3 == "ababab") | (fail "str.repeat")
(str.repeat "ab" 0 == "") | (fail "str.repeat zero")
"#);
    }

    // str.repeat with a negative number and str.pad with an empty filler — a clear
    // error (not a silent wrong result).
    #[test]
    fn str_repeat_negative_and_pad_empty_fail() {
        assert!(run_source(r#"str.repeat "a" (0 - 1)"#).is_err());
        assert!(run_source(r#"str.pad "a" 3 """#).is_err());
        // Even if the bytes fit in usize, exceeding isize::MAX (the allocation limit)
        // gives a Fluxon error, not a panic (PR #151 review).
        assert!(run_source(r#"str.repeat "aa" 4611686018427387904"#).is_err());
        assert!(run_source(r#"str.pad "x" 4611686018427387904 "🙂""#).is_err());
    }

    #[test]
    fn time_module_fmt_and_roundtrip() {
        // time.fmt is deterministic with a unix int: 1700000000 = 2023-11-14 22:13:20 UTC.
        // We check the time.now/time.ago text format ("YYYY-MM-DD HH:MM:SS") and
        // round-trip it through fmt.
        run(r#"
d = time.fmt 1700000000 "YYYY-MM-DD"
(d == "2023-11-14") | (fail "fmt sana wrong: ${d}")
t = time.fmt 1700000000 "HH:mm:ss"
(t == "22:13:20") | (fail "fmt vaqt wrong: ${t}")
n = time.now
(str.len n == 19) | (fail "time.now uzunligi 19 not: ${n}")
back = time.fmt n "YYYY"
(str.len back == 4) | (fail "time.now -> fmt yil 4 raqam not")
"#);
    }

    #[test]
    fn time_ago_is_earlier() {
        // time.ago is before now: the ISO text format is lexicographic = chronological,
        // so a DB filter (`created > $1`) works correctly in SQL. Here we prove the
        // chronological order by comparing the year/month/day parts.
        run(r#"
now = time.now
past = time.ago 1 :day
ny = str.int (time.fmt now "YYYYMMDDHHmmss")
py = str.int (time.fmt past "YYYYMMDDHHmmss")
(py < ny) | (fail "time.ago kelajakda: past=${past} now=${now}")
"#);
    }

    #[test]
    fn time_in_is_later() {
        // time.in is after now (for TTL/expiry). The mirror of time.ago:
        // ISO text is lexicographic = chronological, so the `expires > $now`
        // SQL filter works correctly. We compare the year/month/.../sec parts.
        run(r#"
now = time.now
soon = time.in 1 :hr
ny = str.int (time.fmt now "YYYYMMDDHHmmss")
sy = str.int (time.fmt soon "YYYYMMDDHHmmss")
(sy > ny) | (fail "time.in in the past: soon=${soon} now=${now}")
"#);
    }

    #[test]
    fn time_parse_add_diff_booking_flow() {
        // Issue #65: the client gives an ISO `start_at` and `duration_minutes` ->
        // the server computes `end_at`. The e2e scenario of the booking core.
        run(r#"
start_at = time.parse "2026-06-10T10:00:00Z"
(start_at == "2026-06-10 10:00:00") | (fail "parse wrong: ${start_at}")
end_at = time.add start_at 30 :min
(end_at == "2026-06-10 10:30:00") | (fail "add wrong: ${end_at}")
mins = (time.diff end_at start_at) / 60
(mins == 30) | (fail "diff wrong: ${mins}")
# buffer-inclusive interval: start - 5min (time.sub — the mirror of add)
buf_start = time.sub start_at 5 :min
(buf_start == "2026-06-10 09:55:00") | (fail "time.sub wrong: ${buf_start}")
"#);
    }

    #[test]
    fn time_parse_handles_iso_offset() {
        // ISO text with an offset is brought to UTC (+05:00 -> the time is 5 hours earlier).
        run(r#"
t = time.parse "2026-06-10T15:00:00+05:00"
(t == "2026-06-10 10:00:00") | (fail "mintaqa UTC ga kelmadi: ${t}")
"#);
    }

    #[test]
    fn time_parse_fmt_iana_zone_dst() {
        // Issue #80: DST-aware conversion with an IANA zone name. "09:00 local"
        // maps to different UTC in winter and summer — not a fixed offset.
        run(r#"
# winter (EST = UTC-5): 09:00 local -> 14:00 UTC
w = time.parse "2026-01-15 09:00:00" "America/New_York"
(w == "2026-01-15 14:00:00") | (fail "winter DST wrong: ${w}")
# summer (EDT = UTC-4): the exact same wall-clock -> 13:00 UTC
s = time.parse "2026-07-15 09:00:00" "America/New_York"
(s == "2026-07-15 13:00:00") | (fail "summer DST wrong: ${s}")
# reverse path: UTC instant -> the zone's wall-clock (for display)
back = time.fmt s "HH:mm" "America/New_York"
(back == "09:00") | (fail "fmt zone wrong: ${back}")
"#);
    }

    #[test]
    fn keyword_as_field_name() {
        // After `.` a keyword can be a field name (this is why time.in works).
        // Even if a map key is a keyword, it is read with `.in`/`.match` — this is the
        // Fluxon philosophy: in member position a keyword has no grammatical meaning.
        run(r#"
m = {in: 1 match: 2 each: 3}
(m.in == 1) | (fail "m.in: ${m.in}")
(m.match == 2) | (fail "m.match: ${m.match}")
(m.each == 3) | (fail "m.each: ${m.each}")
"#);
    }

    #[test]
    fn env_member_access() {
        // env.NAME -> std::env. Missing -> nil -> `??` default. Present -> the value.
        // We set and read FLUXON_TEST_VAR (no DB_TEST_LOCK needed — a different env var).
        unsafe { std::env::set_var("FLUXON_TEST_VAR", "hello") };
        run(r#"
v = env.FLUXON_TEST_VAR
(v == "hello") | (fail "env read: ${v}")
miss = env.FLUXON_NONEXISTENT_XYZ ?? "default"
(miss == "default") | (fail "missing env nil -> default not: ${miss}")
"#);
        unsafe { std::env::remove_var("FLUXON_TEST_VAR") };
    }

    #[test]
    fn env_shadowed_by_local() {
        // If the user creates a variable named `env`, it overrides the built-in env
        // (member access goes to the map, not to std::env).
        run(r#"
env = {PORT:"9999"}
p = env.PORT
(p == "9999") | (fail "local env shadow did not work: ${p}")
"#);
    }

    #[test]
    fn json_unicode_roundtrip() {
        // json.dec must decode multi-byte UTF-8 (emoji, Uzbek) and \u escapes (surrogate
        // pairs) CORRECTLY — before, byte-by-byte `as char` gave mojibake
        // (🙂 -> ð...). This core fix applies to http/db/ai alike.
        run(r#"
# raw UTF-8 bytes (no escapes): emoji + Uzbek — byte-by-byte as char USED TO BREAK
r = json.dec "{\"s\":\"o'zbek 🙂 g'ayrat\"}"
(r.s == "o'zbek 🙂 g'ayrat") | (fail "raw UTF-8 broke: ${r.s}")
# \u escape: a BMP character (ü = ü). \\u -> a literal \u in the source.
u = json.dec "{\"c\":\"\\u00fc\"}"
(u.c == "ü") | (fail "\\u00fc dekod broke: ${u.c}")
# \u surrogate pair (🙂 = 🙂)
e = json.dec "{\"c\":\"\\ud83d\\ude42\"}"
(e.c == "🙂") | (fail "\\u surrogate evenligi broke: ${e.c}")
# enc -> dec round-trip
back = json.dec (json.enc {x:"hello 🙂 dünyo"})
(back.x == "hello 🙂 dünyo") | (fail "round-trip broke: ${back.x}")
"#);
    }

    #[test]
    fn json_enc_valid_output() {
        // issue #102: control characters must be escaped, non-finite float -> null.
        run(r#"
# 1/0 = Infinity -> in JSON it must be null, not "inf"
enc = json.enc (1.0 / 0.0)
(enc == "null") | (fail "Infinity was not null: ${enc}")
# tab (control char) \t should escape in short form and round-trip
back = json.dec (json.enc "a\tb")
(back == "a\tb") | (fail "control char round-trip broke: ${back}")
"#);
        // "1 garbage" -> the decoder must error (it used to silently return 1)
        assert!(run_source(r#"log (json.dec "1 garbage")"#).is_err());
        // an invalid null-like string errors
        assert!(run_source(r#"log (json.dec "nqqq")"#).is_err());
        // a number with a leading '+' errors
        assert!(run_source(r#"log (json.dec "+5")"#).is_err());
    }

    #[test]
    fn reg_add_call_has_names() {
        // reg battery: store/call a function by name (dynamic dispatch).
        // the closure takes an args map (the agent tool pattern); reg.has bool, reg.names list.
        run(r#"
reg.add "calc" \args -> args.a + args.b
reg.add "greet" \args -> "hello ${args.name}"

out = reg.call "calc" {a:2 b:3}
(out == 5) | (fail "reg.call calc wrong: ${out}")

g = reg.call "greet" {name:"Aziza"}
(g == "hello Aziza") | (fail "reg.call greet wrong: ${g}")

(reg.has "calc") | (fail "reg.has calc should not be false")
((reg.has "none") == false) | (fail "reg.has none should not be true")

# reg.names with no argument (Field) — stable output in alphabetical order
ns = reg.names
(ns.len == 2) | (fail "reg.names uzunligi 2 not: ${ns}")
(ns.0 == "calc") | (fail "reg.names[0] calc not: ${ns}")
"#);
    }

    #[test]
    fn reg_call_unknown_fails() {
        // Calling a name that is not registered must fail (not silently nil).
        let err = run_source(
            r#"
out = reg.call "yoq" {a:1}
log out
"#,
        )
        .unwrap_err();
        assert!(
            err.contains("not registered"),
            "expected 'not registered', got: {}",
            err
        );
    }

    #[test]
    fn reg_add_overwrites() {
        // A repeat reg.add to the same name — overwrites (the tool-update case).
        run(r#"
reg.add "f" \args -> 1
reg.add "f" \args -> 2
out = reg.call "f" {}
(out == 2) | (fail "reg.add ustiga yozmadi: ${out}")
"#);
    }

    #[test]
    fn fail_as_expr_and_guard() {
        // fail in an expression context (guard) — breaks the flow, propagates upward.
        let err = run_source(
            r#"
fn check x
  x > 0 | (fail 422 "must be positive")
  "ok"
log (check 5)
log (check 0)
"#,
        )
        .unwrap_err();
        assert!(err.contains("422"), "expected 422, got: {}", err);
    }

    #[test]
    fn pipe_and_coalesce() {
        run(r#"
fn inc x -> x + 1
fn sq x -> x * x
r = 3 |> inc |> sq
log "r=${r}"
m = {a:1}
log "missing=${m.b ?? "none"}"
"#);
    }

    // Multi-line pipe: if a line starts with `|>`, it continues the previous
    // expression (builder-chain readability, issue #78). Only `|>` — not `|` (Or).
    #[test]
    fn multiline_pipe_continuation() {
        run(r#"
fn inc x -> x + 1
fn dbl x -> x * 2
# stages on new lines, leading |>
r = 5
  |> inc
  |> dbl
  |> inc
(r == 13) | (fail "multi-line pipe wrong: ${r}")
# continues across a comment and a blank line too
r2 = 10
  |> inc

  # a comment here
  |> dbl
(r2 == 22) | (fail "pipe continuation through comment/empty line broke: ${r2}")
"#);
    }

    // Pipe partial application: `x |> f a b` => `f a b x` (lhs is the LAST argument).
    // Drives the builder/chain pattern. An argument-less function value and an
    // argument-less module call (`|> str.up`) keep the old behavior.
    #[test]
    fn pipe_partial_application() {
        run(r#"
fn addto base n -> base + n
# call with arguments: lhs is appended as the last argument
(5 |> addto 100) == 105 | (fail "pipe call with arguments did not work")
# chain
(3 |> addto 10 |> addto 100) == 113 | (fail "pipe zanjir did not work")
# argument-less module call (old behavior must be preserved)
("hello" |> str.up) == "HELLO" | (fail "pipe argumentsiz modul chaqiruvi broke")
# lambda (old behavior)
(5 |> \n -> n * 2) == 10 | (fail "pipe lambda broke")
"#);
    }

    // --- db battery tests (in-memory SQLite, a separate DB per Interp) ---

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

    #[test]
    fn db_ins_sym_json_roundtrip() {
        // ins returns the generated id; sym Str<->Sym; json map round-trip.
        with_db_test("ins_sym_json", || {
            run(r#"
use db
tbl tickets
  id       serial pk
  category sym
  meta     json
t = db.ins "tickets" {category::billing meta:{tries:3}}
(t.id == 1) | (fail "id 1 should be")
match t.category
  :billing -> log "ok sym"
  _ -> fail "sym :billing should be"
(t.meta.tries == 3) | (fail "json meta.tries 3 should be")
"#);
        });
    }

    #[test]
    fn db_param_and_placeholder() {
        // q without a param + the $1 placeholder binds in SQLite without a rewrite + sym param.
        with_db_test("param_placeholder", || {
            run(r#"
use db
tbl items
  id   serial pk
  kind sym
db.ins "items" {kind::a}
db.ins "items" {kind::b}
all = db.q "select * from items"
(all.len == 2) | (fail "q without param 2 row")
only = db.q "select * from items where kind=$1" [:a]
(only.len == 1) | (fail "$1 sym param 1 row")
"#);
        });
    }

    // Declarative read builder (issue #78): db.from |> db.eq/cmp/order/limit
    // |> db.all/first. A list value -> IN. Filter+range+order+paging without raw SQL.
    #[test]
    fn db_query_builder_reads() {
        with_db_test("query_builder", || {
            run(r#"
use db
tbl bookings
  id          serial pk
  tenant_id   int
  resource_id int
  status      sym
  start_at    str
db.ins "bookings" {tenant_id:1 resource_id:5 status::done start_at:"2026-06-01"}
db.ins "bookings" {tenant_id:1 resource_id:5 status::confirmed start_at:"2026-06-02"}
db.ins "bookings" {tenant_id:1 resource_id:7 status::pending start_at:"2026-06-03"}
db.ins "bookings" {tenant_id:2 resource_id:9 status::done start_at:"2026-06-04"}

# IN filter (list value) + order
in_rows = db.from "bookings" |> db.eq {tenant_id:1 status:[:pending :confirmed]} |> db.order :start_at |> db.all
(in_rows.len == 2) | (fail "IN-filter 2 row expected, ${in_rows.len}")
match in_rows.0.status
  :confirmed -> log "ok IN order"
  _ -> fail "order start_at wrong"

# cmp range + limit
rng = db.from "bookings" |> db.eq {tenant_id:1} |> db.cmp :start_at :ge "2026-06-02" |> db.limit 10 |> db.all
(rng.len == 2) | (fail "cmp >= 2 row expected, ${rng.len}")

# first — one or nil
one = db.from "bookings" |> db.eq {tenant_id:1 resource_id:7} |> db.first
(one != nil) | (fail "first returned nil")
match one.status
  :pending -> log "ok first"
  _ -> fail "first wrong row"

# first — no matching row → nil
none = db.from "bookings" |> db.eq {tenant_id:99} |> db.first
(none == nil) | (fail "first with no match expected nil")

# empty IN list → nothing
empty = db.from "bookings" |> db.eq {status:[]} |> db.all
(empty.len == 0) | (fail "empty IN 0 row expected")

# nil value → IS NULL ( = NULL never matches). a row with resource_id null.
db.ins "bookings" {tenant_id:1 resource_id:nil status::pending start_at:"2026-06-09"}
nulls = db.from "bookings" |> db.eq {tenant_id:1 resource_id:nil} |> db.all
(nulls.len == 1) | (fail "nil → IS NULL 1 row expected, ${nulls.len}")
"#);
        });
    }

    // Issue #104: when db.up was called with an empty condition map, build_update
    // built a column-less "WHERE" (malformed SQL) and the whole table got updated.
    // Like the guard in db.del, it now gives a clear error (instead of SQLite's raw
    // "incomplete input").
    #[test]
    fn db_up_bosh_shart_rad_etiladi() {
        with_db_test("up_empty_where", || {
            let setup = "use db\ntbl t\n  id serial pk\n  n int\ndb.ins \"t\" {n:1}\n";
            let e = run_source(&format!("{setup}db.up \"t\" {{n:5}} {{}}\n")).unwrap_err();
            assert!(
                e.contains("db.up: condition map is empty"),
                "unexpected error: {e}"
            );
        });
    }

    // Issue #104: db.offset without LIMIT used to be silently ignored (SQLite requires
    // LIMIT for OFFSET). Now it is applied correctly with LIMIT -1 OFFSET m.
    #[test]
    fn db_offset_limitsiz_qollanadi() {
        with_db_test("offset_no_limit", || {
            run(r#"
use db
tbl t
  id serial pk
  n  int
db.ins "t" {n:1}
db.ins "t" {n:2}
db.ins "t" {n:3}
# offset 1, no limit → skip the first, return the remaining 2.
rows = db.from "t" |> db.order :n |> db.offset 1 |> db.all
(rows.len == 2) | (fail "offset without LIMIT 2 row expected, ${rows.len}")
(rows.0.n == 2) | (fail "offset should skip the first needed, ${rows.0.n}")
"#);
        });
    }

    // Issue #104: a negative limit/offset gives unexpected behavior in SQLite (a
    // negative LIMIT = unlimited). Now it is clearly rejected at the user level.
    #[test]
    fn db_manfiy_limit_offset_rad_etiladi() {
        with_db_test("neg_limit_offset", || {
            let setup = "use db\ntbl t\n  id serial pk\n  n int\n";
            let e1 = run_source(&format!(
                "{setup}db.from \"t\" |> db.limit (0 - 1) |> db.all\n"
            ))
            .unwrap_err();
            assert!(e1.contains("db.limit: negative"), "limit error: {e1}");
            let e2 = run_source(&format!(
                "{setup}db.from \"t\" |> db.offset (0 - 3) |> db.all\n"
            ))
            .unwrap_err();
            assert!(e2.contains("db.offset: negative"), "offset error: {e2}");
        });
    }

    // Aggregation builder: group + count/sum + conditional agg (count_if/sum_if).
    #[test]
    fn db_query_builder_agg() {
        with_db_test("query_builder_agg", || {
            run(r#"
use db
tbl bookings
  id          serial pk
  tenant_id   int
  resource_id int
  status      sym
  total_cents money
db.ins "bookings" {tenant_id:1 resource_id:5 status::done total_cents:5000}
db.ins "bookings" {tenant_id:1 resource_id:5 status::confirmed total_cents:3000}
db.ins "bookings" {tenant_id:1 resource_id:7 status::pending total_cents:1000}

# group + count + sum, order desc
ag = db.from "bookings" |> db.eq {tenant_id:1 status:[:done :confirmed]} |> db.group :resource_id |> db.count :n |> db.sum :total_cents :rev |> db.order :rev :desc |> db.agg
(ag.len == 1) | (fail "agg 1 guruh expected, ${ag.len}")
(ag.0.resource_id == 5) | (fail "agg resource_id 5")
(ag.0.n == 2) | (fail "agg count 2, ${ag.0.n}")
(ag.0.rev == 8000) | (fail "agg sum 8000, ${ag.0.rev}")

# conditional agg (overview, no group) → a single row
ov = db.from "bookings" |> db.eq {tenant_id:1} |> db.count_if {status::confirmed} :confirmed |> db.count_if {status::pending} :pending |> db.sum_if :total_cents {status::done} :revenue |> db.agg_row
(ov.confirmed == 1) | (fail "count_if confirmed 1, ${ov.confirmed}")
(ov.pending == 1) | (fail "count_if pending 1, ${ov.pending}")
(ov.revenue == 5000) | (fail "sum_if revenue 5000, ${ov.revenue}")

# empty tenant: count_if must return 0 (not nil — COUNT semantics)
empty_ov = db.from "bookings" |> db.eq {tenant_id:99} |> db.count_if {status::done} :done |> db.agg_row
(empty_ov.done == 0) | (fail "empty count_if 0 expected (nil not), ${empty_ov.done}")
"#);
        });
    }

    // str.sym: string -> symbol (turning query-string statuses into a sym filter).
    #[test]
    fn str_sym_conversion() {
        run(r#"
(str.sym "done" == :done) | (fail "str.sym done")
syms = (str.split "pending,confirmed" ",").map \s -> str.sym s
(syms.0 == :pending) | (fail "str.sym split 0")
(syms.1 == :confirmed) | (fail "str.sym split 1")
(str.sym " done " == :done) | (fail "str.sym trim")
"#);
    }

    // --- Issue #82: tbl declarative schema migration + index/uniq ---

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

    #[test]
    fn migrate_add_column_idempotent() {
        // Adding a new column to tbl -> ADD COLUMN; old rows are preserved;
        // re-deploy is idempotent (does not fail).
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = setup_db("fluxon_mig_addcol.db");

        // Deploy 1: a two-column table + one row.
        run_source("use db\ntbl t\n  id serial pk\n  a int\ndb.ins \"t\" {a:1}\n")
            .unwrap_or_else(|e| panic!("deploy1: {}", e));

        // Deploy 2: new column `b` added. It must be an ADD COLUMN, and the old row
        // must be preserved (b NULL).
        run_source(
            r#"
use db
tbl t
  id serial pk
  a  int
  b  str
old = db.one "select * from t where a=1"
(old != nil) | (fail "old row should be preserved needed")
(old.b == nil) | (fail "new column b NULL should be")
db.ins "t" {a:2 b:"hi"}
(db.one "select b from t where a=2").b == "hi" | (fail "write to new column")
"#,
        )
        .unwrap_or_else(|e| panic!("deploy2 add column: {}", e));

        // Deploy 3: the exact same schema — idempotent, does not fail.
        run_source("use db\ntbl t\n  id serial pk\n  a int\n  b str\n")
            .unwrap_or_else(|e| panic!("deploy3 idempotent: {}", e));

        cleanup_db(&path);
    }

    #[test]
    fn migrate_drop_column_with_backup() {
        // Removing a column from tbl -> DROP COLUMN + a _fluxon_bak_* backup table
        // remains with the old data.
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = setup_db("fluxon_mig_dropcol.db");

        run_source(
            "use db\ntbl t\n  id serial pk\n  a int\n  b str\ndb.ins \"t\" {a:1 b:\"keep\"}\n",
        )
        .unwrap_or_else(|e| panic!("deploy1: {}", e));

        // Deploy 2: column `b` removed -> DROP COLUMN. A query for `b` errors
        // (column gone), but the backup table keeps `b="keep"`.
        run_source(
            r#"
use db
tbl t
  id serial pk
  a  int
# column b is gone now -> DROP COLUMN
baks = db.q "select name from sqlite_master where type='table' and name like '_fluxon_bak_t_%'"
(baks.len >= 1) | (fail "backup table should be created needed")
"#,
        )
        .unwrap_or_else(|e| panic!("deploy2 drop column: {}", e));

        // Deploy 3: the exact same (b-less) schema — `b` is already gone, DROP COLUMN
        // is attempted on a missing column, but idempotent: silent pass, no failure.
        run_source("use db\ntbl t\n  id serial pk\n  a int\n")
            .unwrap_or_else(|e| panic!("deploy3 drop idempotent: {}", e));

        cleanup_db(&path);
    }

    #[test]
    fn migrate_drop_table_only_fluxon_managed() {
        // If a tbl is removed from the source entirely -> DROP TABLE + backup, but
        // ONLY a Fluxon-created table (in _fluxon_schema). A non-Fluxon table is preserved.
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = setup_db("fluxon_mig_droptbl.db");

        // Deploy 1: Fluxon creates table `a` + a manual, non-Fluxon `manual` table.
        run_source(
            r#"
use db
tbl a
  id serial pk
  n  int
db.ins "a" {n:1}
db.q "CREATE TABLE manual (x int)"
db.q "INSERT INTO manual VALUES (42)"
"#,
        )
        .unwrap_or_else(|e| panic!("deploy1: {}", e));

        // Deploy 2: tbl `a` removed (but another tbl exists — the registry is NOT
        // empty). `a` must be DROPped, `manual` must be preserved.
        run_source(
            r#"
use db
tbl b
  id serial pk
gone = db.q "select name from sqlite_master where type='table' and name='a'"
(gone.len == 0) | (fail "a tablei DROP should be")
kept = db.q "select name from sqlite_master where type='table' and name='manual'"
(kept.len == 1) | (fail "manual table should be preserved needed (not created by Fluxon)")
(db.one "select x from manual").x == 42 | (fail "manual data should be preserved needed")
baks = db.q "select name from sqlite_master where type='table' and name like '_fluxon_bak_a_%'"
(baks.len >= 1) | (fail "backup for a should be created needed")
"#,
        )
        .unwrap_or_else(|e| panic!("deploy2 drop table: {}", e));

        cleanup_db(&path);
    }

    #[test]
    fn migrate_index_create_and_drop() {
        // An index declaration -> CREATE INDEX; removing it -> DROP INDEX. uniq(a b) ->
        // a duplicate insert errors. sqlite_autoindex_* is left untouched.
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = setup_db("fluxon_mig_index.db");

        // Deploy 1: a single index + a multi-column unique.
        run_source(
            r#"
use db
tbl bookings
  id          serial pk
  resource_id int
  status      sym index
  start_at    str
  uniq(resource_id start_at)
idx = db.q "select name from sqlite_master where type='index' and name='idx_bookings_status'"
(idx.len == 1) | (fail "idx_bookings_status yaratilishi needed")
uniq = db.q "select name from sqlite_master where type='index' and name='uniq_bookings_resource_id_start_at'"
(uniq.len == 1) | (fail "uniq index should be created needed")
db.ins "bookings" {resource_id:5 status::done start_at:"2026-06-01"}
"#,
        )
        .unwrap_or_else(|e| panic!("deploy1 index: {}", e));

        // uniq violation: the same (resource_id start_at) -> error.
        let dup = run_source(
            r#"
use db
tbl bookings
  id          serial pk
  resource_id int
  status      sym index
  start_at    str
  uniq(resource_id start_at)
db.ins "bookings" {resource_id:5 status::pending start_at:"2026-06-01"}
"#,
        );
        assert!(
            dup.is_err(),
            "uniq(resource_id start_at) duplicate insert should error"
        );

        // Deploy 2: the status index removed -> DROP INDEX. uniq stays.
        run_source(
            r#"
use db
tbl bookings
  id          serial pk
  resource_id int
  status      sym
  start_at    str
  uniq(resource_id start_at)
dropped = db.q "select name from sqlite_master where type='index' and name='idx_bookings_status'"
(dropped.len == 0) | (fail "idx_bookings_status DROP should be")
kept = db.q "select name from sqlite_master where type='index' and name='uniq_bookings_resource_id_start_at'"
(kept.len == 1) | (fail "uniq index should be preserved needed")
"#,
        )
        .unwrap_or_else(|e| panic!("deploy2 drop index: {}", e));

        cleanup_db(&path);
    }

    #[test]
    fn migrate_drop_indexed_column() {
        // REGRESSION (code review): when an indexed column is removed, the stale
        // index must be dropped BEFORE the column DROP — otherwise in some SQLite
        // states DROP COLUMN is rejected with "error in index ... no such column"
        // and the deploy cannot migrate. Both single and composite index.
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = setup_db("fluxon_mig_dropidxcol.db");

        // Deploy 1: an indexed `status` column + a composite index(a status).
        run_source(
            r#"
use db
tbl t
  id     serial pk
  a      int
  status sym index
  index(a status)
db.ins "t" {a:1 status::x}
"#,
        )
        .unwrap_or_else(|e| panic!("deploy1: {}", e));

        // Deploy 2: column `status` removed. The old idx_t_status and idx_t_a_status
        // are still in the DB — the migration must not fail (the stale index is
        // dropped first), then DROP COLUMN must work.
        run_source(
            r#"
use db
tbl t
  id serial pk
  a  int
gone = db.q "select name from sqlite_master where type='index' and name='idx_t_status'"
(gone.len == 0) | (fail "idx_t_status DROP should be")
comp = db.q "select name from sqlite_master where type='index' and name='idx_t_a_status'"
(comp.len == 0) | (fail "idx_t_a_status (depending on status) DROP should be")
# the status column is really gone
cols = db.q "select name from pragma_table_info('t') where name='status'"
(cols.len == 0) | (fail "status columni DROP should be")
"#,
        )
        .unwrap_or_else(|e| panic!("deploy2 drop indexed column: {}", e));

        cleanup_db(&path);
    }

    #[test]
    fn migrate_pipe_modifier_creates_unique_index() {
        // `email str index|uniq` -> a single UNIQUE index is created (uniq subsumes
        // it), a duplicate insert errors.
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = setup_db("fluxon_mig_pipe.db");

        run_source(
            r#"
use db
tbl users
  id    serial pk
  email str index|uniq
ui = db.q "select name from sqlite_master where type='index' and name='uniq_users_email'"
(ui.len == 1) | (fail "uniq_users_email should be created needed")
db.ins "users" {email:"a@x.uz"}
"#,
        )
        .unwrap_or_else(|e| panic!("deploy1 pipe: {}", e));

        let dup = run_source(
            r#"
use db
tbl users
  id    serial pk
  email str index|uniq
db.ins "users" {email:"a@x.uz"}
"#,
        );
        assert!(dup.is_err(), "index|uniq duplicate email should error");

        cleanup_db(&path);
    }

    #[test]
    fn migrate_multi_column_uniq_constraint() {
        // Issue #94: `uniq(a, b)` (comma-separated) creates a multi-column UNIQUE
        // constraint — NOT a fake "uniq" column. A duplicate (a,b) pair errors.
        with_db_test("multi_uniq", || {
            // 1. No fake `uniq` column: the table must contain only a, b.
            run(r#"
use db
tbl t
  a str
  b str
  uniq(a, b)
n = (db.q "select count(*) c from pragma_table_info('t')").0.c
(n == 2) | (fail "table should have only 2 columns (a, b) — no phantom uniq column")
ui = db.q "select name from sqlite_master where type='index' and name='uniq_t_a_b'"
(ui.len == 1) | (fail "uniq_t_a_b unique index should be created")
db.ins "t" {a:"x" b:"y"}
"#);

            // 2. A duplicate (a, b) pair violates the UNIQUE constraint. Both inserts
            //    are in one source — so the shared-memory db is not lost between runs.
            let dup = run_source(
                r#"
use db
tbl t
  a str
  b str
  uniq(a, b)
db.ins "t" {a:"x" b:"y"}
db.ins "t" {a:"x" b:"y"}
"#,
            );
            assert!(dup.is_err(), "duplicate (a, b) should violate uniq");
        });
    }

    #[test]
    fn fk_ref_modifier_enforced() {
        // Issue #94 (related): the `ref:tbl.col` FK modifier is now enforced —
        // an insert referencing a non-existent parent row errors.
        with_db_test("fk_ref", || {
            // Valid FK: the parent row exists — the insert passes.
            run(r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int ref:users.id
  title str
db.ins "users" {name:"ali"}
p = db.ins "posts" {owner:1 title:"hello"}
(p.id == 1) | (fail "valid FK insert should pass needed")
"#);

            // Orphan FK: owner=999 does not exist -> FOREIGN KEY constraint failed.
            let orphan = run_source(
                r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int ref:users.id
  title str
db.ins "posts" {owner:999 title:"orphan"}
"#,
            );
            assert!(orphan.is_err(), "orphan FK insert should error");
        });
    }

    #[test]
    fn migrate_adds_fk_to_existing_column_via_rebuild() {
        // Issue #94 (codex review): FK must apply not only to a NEW table — also to
        // an existing column in an EXISTING table. The old state (DB introspection) is
        // compared with the declaration, and on a difference the table is rebuilt. Data
        // is preserved, autoincrement continues, FK is enforced.
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = setup_db("fluxon_fk_rebuild.db");

        // Deploy 1: posts without an FK, with data.
        run_source(
            r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int
  title str
db.ins "users" {name:"ali"}
db.ins "posts" {owner:1 title:"a"}
db.ins "posts" {owner:1 title:"b"}
"#,
        )
        .unwrap_or_else(|e| panic!("deploy1: {}", e));

        // Deploy 2: ref:users.id added to the existing `owner` column -> rebuild.
        run_source(
            r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int ref:users.id
  title str
rows = db.q "select count(*) c from posts"
(rows.0.c == 2) | (fail "rebuild should preserve data needed (2 row)")
fk = db.q "select count(*) c from pragma_foreign_key_list('posts')"
(fk.0.c == 1) | (fail "posts should have FK after rebuild")
n = db.ins "posts" {owner:1 title:"c"}
(n.id == 3) | (fail "autoincrement should continue needed (id=3)")
"#,
        )
        .unwrap_or_else(|e| panic!("deploy2 rebuild: {}", e));

        // Now an orphan insert is rejected (FK enforced).
        let orphan = run_source(
            r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int ref:users.id
  title str
db.ins "posts" {owner:404 title:"orphan"}
"#,
        );
        assert!(
            orphan.is_err(),
            "orphan FK insert should error after rebuild"
        );

        cleanup_db(&path);
    }

    #[test]
    fn migrate_drop_column_and_add_fk_same_deploy() {
        // Codex review: if one migration both DROPs a column and adds a ref to an
        // existing column — the DROP COLUMN backup (`_fluxon_bak_<t>_<ts>`) and the
        // rebuild backup must NOT COLLIDE in name (rebuild uses a `_fk` suffix).
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = setup_db("fluxon_drop_and_fk.db");

        // Deploy 1: an `old` column exists, no ref.
        run_source(
            r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int
  title str
  old   str
db.ins "users" {name:"a"}
db.ins "posts" {owner:1 title:"x" old:"old"}
"#,
        )
        .unwrap_or_else(|e| panic!("deploy1 drop+fk: {}", e));

        // Deploy 2: DROP `old` + add a ref to `owner` (one migration).
        run_source(
            r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int ref:users.id
  title str
n = db.q "select count(*) c from posts"
(n.0.c == 1) | (fail "data should be preserved needed (1 row)")
fk = db.q "select count(*) c from pragma_foreign_key_list('posts')"
(fk.0.c == 1) | (fail "FK should be added needed")
cols = db.q "select count(*) c from pragma_table_info('posts')"
(cols.0.c == 3) | (fail "old column DROPped, 3 columns should remain needed")
"#,
        )
        .unwrap_or_else(|e| panic!("deploy2 drop+fk (backup collision?): {}", e));

        cleanup_db(&path);
    }

    #[test]
    fn migrate_fk_rebuild_aborts_on_orphan_data() {
        // If existing data has an orphan row, the FK-adding rebuild does NOT silently
        // lose it — it gives a clear error and the data stays intact via ROLLBACK.
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = setup_db("fluxon_fk_orphan.db");

        run_source(
            r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int
  title str
db.ins "users" {name:"a"}
db.ins "posts" {owner:1 title:"ok"}
db.ins "posts" {owner:777 title:"orphan"}
"#,
        )
        .unwrap_or_else(|e| panic!("deploy1 orphan: {}", e));

        // adding a ref -> the orphan row violates the FK -> migrate errors (rebuild aborts).
        let res = run_source(
            r#"
use db
tbl users
  id   serial pk
  name str
tbl posts
  id    serial pk
  owner int ref:users.id
  title str
db.q "select 1 x"
"#,
        );
        assert!(res.is_err(), "FK rebuild should abort on orphan data");

        // The data and the old (FK-less) schema must be preserved.
        run_source(
            r#"
use db
n = db.q "select count(*) c from posts"
(n.0.c == 2) | (fail "rollback should preserve data needed (2 row)")
fk = db.q "select count(*) c from pragma_foreign_key_list('posts')"
(fk.0.c == 0) | (fail "FK should not be added after abort needed")
"#,
        )
        .unwrap_or_else(|e| panic!("verify orphan: {}", e));

        cleanup_db(&path);
    }

    #[test]
    fn db_tx_commit_returns_value() {
        with_db_test("tx_commit", || {
            run(r#"
use db
tbl t
  id serial pk
  n  int
r = db.tx \->
  x = db.ins "t" {n:7}
  ret x
(r.n == 7) | (fail "tx ret valuei n=7")
(db.one "select count(*) c from t").c == 1 | (fail "1 row commit should be")
"#);
        });
    }

    #[test]
    fn db_tx_rollback_on_fail() {
        // fail inside tx -> the whole block rolls back; the error propagates upward
        // and the first (tx-less) ins is preserved, while the ins inside the tx is
        // rolled back. FILE-backed temp DB: persists between two run_source calls (a
        // memory DB is gone when the first Interp drops). The verifying run is a SEPARATE Interp.
        let path = std::env::temp_dir().join("fluxon_tx_rollback_test.db");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: the guard is held.
        unsafe {
            std::env::set_var("DATABASE_URL", format!("sqlite:{}", path.display()));
        }

        let err = run_source(
            r#"
use db
tbl t
  id serial pk
  n  int
db.ins "t" {n:1}
db.tx \->
  db.ins "t" {n:2}
  fail "on purpose"
"#,
        )
        .unwrap_err();
        assert!(
            err.contains("on purpose"),
            "expected fail message, got: {}",
            err
        );

        // A separate (new) Interp/pool — the file DB is preserved. If rollback worked,
        // only the tx-less ins (n:1) remains, the one inside the tx (n:2) does not.
        run_source(
            r#"
use db
tbl t
  id serial pk
  n  int
(db.one "select count(*) c from t").c == 1 | (fail "1 row should remain after rollback needed")
"#,
        )
        .unwrap_or_else(|e| panic!("rollback tekshiruvi: {}", e));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn db_json_col_cross_process_decode() {
        // Issue #63: a json column must return a map even in a process where `tbl` is
        // NOT declared. Two SEPARATE Interps (= two processes) over one FILE DB: the
        // first writes (with tbl), the second reads without tbl — DB introspection
        // recovers that the column is json and gives a map (before, a raw string came
        // back and row.body.x errored).
        let path = std::env::temp_dir().join("fluxon_json_xproc_test.db");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: the guard is held.
        unsafe {
            std::env::set_var("DATABASE_URL", format!("sqlite:{}", path.display()));
        }

        // Writer process: declares tbl + writes a json map (which also contains a list).
        run_source(
            r#"
use db
tbl t
  k    sym
  body json
db.ins "t" {k::a body:{x:1 y:[1 2 3]}}
"#,
        )
        .unwrap_or_else(|e| panic!("yozish: {}", e));

        // Reader process: NO tbl — only reads. The json must come back as a map.
        run_source(
            r#"
use db
row = db.one "select * from t where k=$1" [:a]
(row.body.x == 1) | (fail "json column should decode as a map (x)")
(row.body.y.len == 3) | (fail "inner json list should also be restored needed (y)")
"#,
        )
        .unwrap_or_else(|e| panic!("read: {}", e));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn db_json_schema_less_write_to_text_col() {
        // Regression: a process where tbl is NOT declared must be able to write a
        // map/list to a TEXT column. Before, DB introspection returned the TEXT column
        // as Some("text") and the write path errored with "not a json column" — now the
        // write side uses only the tbl registry, so schema-less writes work for a process without tbl.
        //
        // Scenario: the first process creates a `str` (TEXT) column; the second process
        // writes a map with NO tbl — this used to error with "not a json column".
        let path = std::env::temp_dir().join("fluxon_schemaless_write_test.db");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::set_var("DATABASE_URL", format!("sqlite:{}", path.display()));
        }

        // First process: creates a table with a str (TEXT) column and writes one row
        // (db.ins does a lazy DB open + migrate — the table is created right here).
        run_source(
            r#"
use db
tbl t3
  id   serial pk
  body str
db.ins "t3" {body:"init"}
"#,
        )
        .unwrap_or_else(|e| panic!("jadval yaratish: {}", e));

        // Second process: NO tbl — must write a map to the TEXT column (schema-less).
        run_source(
            r#"
use db
db.ins "t3" {body:{x:42 y:[1 2]}}
row = db.one "select body from t3 limit 1"
row.body | (fail "body should not be empty needed")
"#,
        )
        .unwrap_or_else(|e| panic!("schema-less yozish: {}", e));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn db_tx_nested_savepoint() {
        // Inner tx (SAVEPOINT). The inner block returns a ret value, the outer commits.
        with_db_test("tx_nested", || {
            run(r#"
use db
tbl t
  id serial pk
  n  int
r = db.tx \->
  db.ins "t" {n:1}
  inner = db.tx \->
    x = db.ins "t" {n:2}
    ret x
  ret inner
(r.n == 2) | (fail "nested tx ret valuei n=2")
(db.one "select count(*) c from t").c == 2 | (fail "ikkala ins commit should be")
"#);
        });
    }

    #[test]
    fn db_put_upsert() {
        with_db_test("put_upsert", || {
            run(r#"
use db
tbl counters
  name str pk
  hits int
db.put "counters" {hits:1} {name:"x"}
db.put "counters" {hits:9} {name:"x"}
c = db.one "select * from counters where name=$1" ["x"]
(c.hits == 9) | (fail "upsert hits=9 should be")
n = (db.q "select * from counters").len
(n == 1) | (fail "upsert should not create a duplicate needed")
"#);
        });
    }

    #[test]
    fn db_uniq_violation_rolls_back_tx() {
        // A uniq violation inside tx -> rollback (the idempotency pattern).
        with_db_test("uniq_violation", || {
            let err = run_source(
                r#"
use db
tbl txns
  id   serial pk
  ikey str uniq
db.ins "txns" {ikey:"k1"}
db.tx \->
  db.ins "txns" {ikey:"k1"}
"#,
            )
            .unwrap_err();
            // The uniq violation is raised as a db error.
            assert!(
                err.to_lowercase().contains("unique") || err.contains("db error"),
                "expected uniq violation error, got: {}",
                err
            );
        });
    }

    // --- cron battery ---

    #[test]
    fn cron_on_registratsiya_xatosiz() {
        // Unquoted 5-field form (a named function). cron.on does not block, the program ends.
        run(r#"
fn check
  log "check"
cron.on 0 * * * * check
"#);
    }

    #[test]
    fn cron_on_lambda_va_murakkab_ifoda() {
        // Inline lambda + a mixed step/range/list expression.
        run(r#"
cron.on */15 9 1,15 * 1-5 \->
  log "har 15 daqiqa, 9-soat, 1 va 15-kun, ish kunlari"
"#);
    }

    #[test]
    fn cron_on_tirnoqli_variant() {
        // A quoted str also works (for humans; not in the AI docs).
        run(r#"
fn report
  log "report"
cron.on "30 9 * * *" report
"#);
    }

    #[test]
    fn cron_on_notogri_ifoda_xato() {
        // There is no minute 99 — cron.on must return an error.
        let err = run_source(
            r#"
fn f
  log "x"
cron.on 99 * * * * f
"#,
        )
        .expect_err("an invalid cron expression should error");
        assert!(
            err.contains("cron") && err.to_lowercase().contains("expression"),
            "expected cron expression error, got: {}",
            err
        );
    }

    // --- queue battery ---

    #[test]
    fn queue_on_push_registratsiya_xatosiz() {
        // queue.on registers a handler, queue.push adds a job — neither blocks, the
        // program ends (the worker keeps running in the background). The handler takes
        // a single `job` map argument.
        run(r#"
queue.on "send" \job ->
  log "sending: ${job.ph}"
queue.push "send" {ph:"+99890" body:"hello"}
"#);
    }

    #[test]
    fn queue_push_payloadsiz() {
        // Payload is optional — if omitted, job is Nil.
        run(r#"
queue.on "tozala" \job ->
  log "cleaned"
queue.push "tozala"
"#);
    }

    #[test]
    fn queue_handlersiz_push_dastur_tugaydi() {
        // Issue #105: a job whose handler is never registered must not block the
        // program from exiting — run() ends normally with a warning (in the old
        // busy-loop the job spun forever).
        run(r#"queue.push "orphan" {x:1}"#);
    }

    #[test]
    fn queue_drain_handler_haqiqatan_ishlaydi() {
        // Issue #105: the queue is drained before run() returns — that the handler
        // actually ran is checked via the DB without a RACE (before, you could not
        // guarantee the worker background thread had finished).
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = setup_db("fluxon_queue_drain.db");

        run(r#"
use db
tbl jobs
  id  serial pk
  nom str
queue.on "yoz" \job ->
  db.ins "jobs" {nom:job.nom}
queue.push "yoz" {nom:"a"}
queue.push "yoz" {nom:"b"}
"#);

        // The first run() ended with a drain — both jobs MUST be in the DB.
        run(r#"
use db
((db.q "select * from jobs").len == 2) | (fail "queue jobs were not executed")
"#);

        cleanup_db(&path);
    }

    #[test]
    fn queue_push_nom_str_bolmasa_xato() {
        // The 1st argument, the job name, must be a str.
        let err = run_source(r#"queue.push 5"#).expect_err("a non-str name should error");
        assert!(
            err.contains("queue.push"),
            "expected queue.push error, got: {}",
            err
        );
    }

    #[test]
    fn queue_argumentsiz_dispatch_ga_yetadi() {
        // An argument-less `queue.X` (it arrives as a Field, not a Call) must reach
        // module dispatch — so the `queue` ident is not looked up as a variable and
        // does not give "unknown name". We test with an unknown function: if it reaches
        // dispatch, a "no ... in queue module" error comes (NOT unknown name). [cron.run regression]
        let err = run_source(r#"queue.yoq"#).expect_err("argument-less queue.yoq should error");
        assert!(
            err.contains("queue module") && !err.contains("unknown name"),
            "argument-less queue should reach dispatch, got: {}",
            err
        );
    }

    #[test]
    fn cron_argumentsiz_dispatch_ga_yetadi() {
        // `cron.run` argument-less — arrives as a Field and must reach dispatch
        // (otherwise "unknown name: cron"). cron.run blocks, so instead of an existing
        // function we test, with an unknown function, that it reaches dispatch.
        let err = run_source(r#"cron.yoq"#).expect_err("argument-less cron.yoq should error");
        assert!(
            err.contains("cron module") && !err.contains("unknown name"),
            "argument-less cron should reach dispatch, got: {}",
            err
        );
    }

    #[test]
    fn queue_on_handler_fn_bolmasa_xato() {
        // The 2nd argument, the handler, must be an fn.
        let err = run_source(r#"queue.on "send" 5"#).expect_err("a non-fn handler should error");
        assert!(
            err.contains("queue.on"),
            "expected queue.on error, got: {}",
            err
        );
    }

    // The `ai` tests depend on the env (keys) — we serialize them with a global mutex
    // (so other tests do not change the env in parallel). These tests do NOT GO TO THE
    // NETWORK: we check that an error is raised BEFORE the API call when the key is missing.
    static AI_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn ai_kalit_yoq_bolsa_aniq_xato() {
        let _guard = AI_ENV_LOCK.lock().unwrap();
        // We temporarily remove all key envs (auto-detect must find none). There is no
        // .env in runtime/ -> a clear "key not found" error, no network call. We save
        // the previous values and restore them after the test.
        let saved: Vec<(&str, Option<String>)> = ["AI_KEY", "ANTHROPIC_API_KEY", "OPENAI_API_KEY"]
            .iter()
            .map(|k| (*k, std::env::var(k).ok()))
            .collect();
        for (k, _) in &saved {
            unsafe { std::env::remove_var(k) };
        }
        let err = run_source(r#"x = ai.ask "hello""#).expect_err("a missing key should error");
        // restore the env (so it does not affect other tests).
        for (k, v) in &saved {
            if let Some(val) = v {
                unsafe { std::env::set_var(k, val) };
            }
        }
        assert!(
            err.contains("key not found") || err.contains("key"),
            "expected key-not-found error, got: {}",
            err
        );
    }

    #[test]
    fn ai_noma_lum_funksiya_xato() {
        let _guard = AI_ENV_LOCK.lock().unwrap();
        // ai.foo -> reaches dispatch and gives "no ai.foo" (NOT unknown name).
        // Whether or not a key exists, this comes before checking the function name.
        let err = run_source(r#"ai.foo "x""#).expect_err("an unknown ai function should error");
        assert!(
            err.contains("ai.foo") && !err.contains("unknown name"),
            "ai should reach dispatch and give a function error, got: {}",
            err
        );
    }

    #[test]
    fn ai_ozgaruvchi_modulni_yopadi() {
        // If `ai` is declared as a variable, it is not a module — it is read as a plain
        // map field (unlike http/db, but the ai dispatch lookup checks for it).
        run(r#"
ai = {ask:"shadowed"}
log "ai.ask = ${ai.ask}"
"#);
    }

    // sh.run -> {stdout stderr code}: the echo output and the success code are correct.
    // (Unix-compatible echo, works on CI ubuntu+macOS.)
    #[test]
    fn sh_run_echo_natija_va_kod() {
        run(r#"
r = sh.run "printf hello"
(r.code == 0) | (fail "code should be 0: ${r.code}")
(r.stdout == "hello") | (fail "stdout wrong: ${r.stdout}")
(r.stderr == "") | (fail "stderr empty should be: ${r.stderr}")
"#);
    }

    // Non-zero exit -> NOT a Flow::err, it is checked via `code` (the expected result).
    #[test]
    fn sh_run_nolik_bolmagan_kod_xato_emas() {
        run(r#"
r = sh.run "exit 7"
(r.code == 7) | (fail "code 7 should be: ${r.code}")
"#);
    }

    // --- `use ./file` user modules (issue #45) ---

    use std::sync::atomic::{AtomicU64, Ordering};

    // A unique temporary directory — so parallel tests do not collide
    // (process id + an atomic counter). Test files are written here.
    fn temp_module_dir() -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("fluxon_mod_test_{}_{}", std::process::id(), n));
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

    // Main case (issue #45 reproduction): an `exp`-ed value and function appear
    // under `module.name`; a module function can access a module-level `exp`
    // (closure).
    #[test]
    fn use_module_exp_va_closure() {
        run_modules(&[
            (
                "main.fx",
                r#"
use ./greet
(greet.greeting == "hello") | (fail "greeting: ${greet.greeting}")
(greet.hello "Aziza" == "hello, Aziza") | (fail "hello: ${greet.hello "Aziza"}")
"#,
            ),
            (
                "greet.fx",
                "exp greeting = \"hello\"\nexp fn hello name -> \"${greeting}, ${name}\"\n",
            ),
        ])
        .unwrap();
    }

    // `as alias` — the binding name becomes the alias (to avoid clashing with a battery name).
    #[test]
    fn use_module_alias() {
        run_modules(&[
            (
                "main.fx",
                r#"
use ./tools as t
(t.classify "x" == "type: x") | (fail "classify: ${t.classify "x"}")
"#,
            ),
            ("tools.fx", "exp fn classify v -> \"type: ${v}\"\n"),
        ])
        .unwrap();
    }

    // Module-private names (plain `=`/`fn`) do NOT enter the namespace — only `exp`.
    #[test]
    fn use_module_private_nom_eksport_qilinmaydi() {
        run_modules(&[
            (
                "main.fx",
                r#"
use ./m
(m.pub_v == 1) | (fail "pub_v: ${m.pub_v}")
(m.priv_v == nil) | (fail "priv_v should not be exported needed: ${m.priv_v}")
"#,
            ),
            ("m.fx", "exp pub_v = 1\npriv_v = 2\n"),
        ])
        .unwrap();
    }

    // Nested import (main -> a -> b): a module can import another module, the
    // path is resolved relative to the importing module's directory.
    #[test]
    fn use_module_nested() {
        run_modules(&[
            (
                "main.fx",
                r#"
use ./a
(a.get() == 43) | (fail "get: ${a.get()}")
"#,
            ),
            ("a.fx", "use ./b\nexp fn get -> b.val + 1\n"),
            ("b.fx", "exp val = 42\n"),
        ])
        .unwrap();
    }

    // `../` (parent directory) module path (issue #47): a file in a subdirectory
    // can import a module in the parent directory. This tests that parse_use
    // recognizes `Tok::DotDot` and the runtime can resolve a path with `..`.
    #[test]
    fn use_module_yuqori_papka() {
        run_modules(&[
            (
                "sub/test.fx",
                r#"
use ../greet
(greet.greeting == "hello") | (fail "greeting: ${greet.greeting}")
"#,
            ),
            ("greet.fx", "exp greeting = \"hello\"\n"),
        ])
        .unwrap();
    }

    // Cache: if one module is `use`d twice it runs once (idempotent).
    // The module's top-level `<-` increments a counter; even with two imports it stays 1.
    #[test]
    fn use_module_cache_bir_marta_bajariladi() {
        run_modules(&[
            (
                "main.fx",
                r#"
use ./c
use ./c as c2
(c.n == 1) | (fail "n: ${c.n}")
(c2.n == 1) | (fail "c2.n: ${c2.n}")
"#,
            ),
            // `exp n` is computed only once — that is what caching means.
            ("c.fx", "exp n = 1\n"),
        ])
        .unwrap();
    }

    // A circular import (x -> y -> x) gives a clear error (not infinite recursion).
    #[test]
    fn use_module_sikllik_import_xato() {
        let err = run_modules(&[
            ("x.fx", "use ./y\nexp a = 1\n"),
            ("y.fx", "use ./x\nexp b = 2\n"),
        ])
        .unwrap_err();
        assert!(
            err.contains("circular import"),
            "circular import error expected, got: {}",
            err
        );
    }

    // A non-existent module — a clear "not found" error.
    #[test]
    fn use_module_topilmadi_xato() {
        let err = run_modules(&[("main.fx", "use ./yoq\n")]).unwrap_err();
        assert!(
            err.contains("module not found"),
            "not-found error expected, got: {}",
            err
        );
    }

    // The `.fx` extension is added automatically: `use ./greet` -> `greet.fx`.
    // (The tests above rely on this too; this is the explicit check.)
    #[test]
    fn use_module_fx_kengaytma_avto() {
        run_modules(&[
            (
                "main.fx",
                "use ./util\n(util.x == 7) | (fail \"x: ${util.x}\")\n",
            ),
            ("util.fx", "exp x = 7\n"),
        ])
        .unwrap();
    }

    // A battery `use` (`use http`) is still a no-op — no file is loaded, dispatch works.
    #[test]
    fn use_batareya_hamon_no_op() {
        // `use math` does not look for a file (no error), math.* dispatch works.
        run(r#"
use math
(math.floor 3.7 == 3) | (fail "floor wrong")
"#);
    }

    // Issue #128: math.min/max/pow/sqrt — a check through the .fx surface.
    #[test]
    fn math_min_max_pow_sqrt() {
        run(r#"
(math.min 3 7 == 3) | (fail "min wrong")
(math.max 3 7 == 7) | (fail "max wrong")
(math.min 3 2.5 == 2.5) | (fail "mixed int/float min wrong")
(math.pow 2 10 == 1024) | (fail "pow wrong")
(math.sqrt 9 == 3.0) | (fail "sqrt wrong")
"#);
    }

    // `each i in inf` — an infinite loop. `stop` exits it, `i` increases from 0.
    // For the REPL/event-loop (issue #27): the model used to resort to the 1..1000
    // trick; now there is a natural infinite repeat.
    #[test]
    fn each_inf_stop_va_hisoblagich() {
        run(r#"
sum <- 0
each i in inf
  if i == 5
    stop
  sum <- sum + i
(sum == 10) | (fail "0+1+2+3+4 = 10 should be: ${sum}")
"#);
    }

    // `skip` in an infinite loop moves to the next iteration (i still increases).
    #[test]
    fn each_inf_skip() {
        run(r#"
cnt <- 0
each i in inf
  if i >= 10
    stop
  if i % 2 == 0
    skip
  cnt <- cnt + 1
(cnt == 5) | (fail "odd sonlar 1,3,5,7,9 = 5 ta: ${cnt}")
"#);
    }

    // inf cannot be used as a value — only in `each i in inf`.
    #[test]
    fn inf_qiymat_sifatida_xato() {
        let err = run_source("x = inf\n").expect_err("inf as a value should error");
        assert!(err.contains("inf"), "unexpected error: {}", err);
    }

    // `each k, v in inf` — two variables are meaningless (a plain infinite counter).
    #[test]
    fn each_inf_ikki_ozgaruvchi_xato() {
        let err = run_source("each k, v in inf\n  stop\n")
            .expect_err("two variables with inf should error");
        assert!(
            err.contains("a single variable"),
            "unexpected error: {}",
            err
        );
    }

    // --- `fluxon check` (parse only, issue #55) ---

    // Valid code -> check succeeds (Ok).
    #[test]
    fn check_togri_kod_ok() {
        check_source(
            r#"
fn fib n
  if n < 2
    ret n
  (fib (n - 1)) + (fib (n - 2))
log "${fib 10}"
"#,
        )
        .expect("valid code should pass check");
    }

    // Parse/lex error -> check returns Err (main turns this Err into exit 2).
    #[test]
    fn check_parse_xato_err() {
        let err = check_source("fn g x\n  ret (\n").expect_err("a parse error should return Err");
        assert!(!err.is_empty(), "error text should not be empty");
    }

    // MOST IMPORTANT: check does NOT execute code — no runtime side effect/error.
    // The code below fails at runtime (unknown name), but the syntax is valid, so
    // check returns Ok. This proves that check skips the interp (Forge eval-gate
    // LAYER 1: executing is DANGEROUS).
    #[test]
    fn check_kodni_bajarmaydi() {
        // `nomalum_funksiya` gives "unknown name" at runtime, but the syntax is fine.
        check_source("x = nomalum_funksiya 5\n")
            .expect("syntactically valid code should pass check (not executed)");
        // Confirm: the same code errors under run (it is executed).
        assert!(
            run_source("x = nomalum_funksiya 5\n").is_err(),
            "run should execute this code and error (unlike check)"
        );
    }

    // parse_args: recognizes the `check` command and puts the file into Command::Check.
    #[test]
    fn parse_args_check_buyrugi() {
        let args: Vec<String> = ["fluxon", "check", "test.fx"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        match parse_args(&args) {
            Some(Command::Check(p)) => assert_eq!(p, "test.fx"),
            _ => panic!("expected Command::Check, found another variant"),
        }
    }

    // parse_args: `test` works without a path (default tests/) and with a path.
    #[test]
    fn parse_args_test_buyrugi() {
        let to_args = |a: &[&str]| -> Vec<String> { a.iter().map(|s| s.to_string()).collect() };
        match parse_args(&to_args(&["fluxon", "test"])) {
            Some(Command::Test(None)) => {}
            _ => panic!("expected Command::Test(None)"),
        }
        match parse_args(&to_args(&["fluxon", "test", "smoke.fx"])) {
            Some(Command::Test(Some(p))) => assert_eq!(p, "smoke.fx"),
            _ => panic!("expected Command::Test(Some)"),
        }
    }

    // parse_args: a version flag maps to the command that prints the built package
    // version.
    #[test]
    fn parse_args_version_flaglari() {
        let to_args = |a: &[&str]| -> Vec<String> { a.iter().map(|s| s.to_string()).collect() };
        match parse_args(&to_args(&["fluxon", "--version"])) {
            Some(Command::Version) => {}
            _ => panic!("expected Command::Version"),
        }
        match parse_args(&to_args(&["fluxon", "-V"])) {
            Some(Command::Version) => {}
            _ => panic!("expected Command::Version"),
        }
    }

    // parse_args: help flags map to the command that prints the usage text.
    #[test]
    fn parse_args_help_flaglari() {
        let to_args = |a: &[&str]| -> Vec<String> { a.iter().map(|s| s.to_string()).collect() };
        match parse_args(&to_args(&["fluxon", "--help"])) {
            Some(Command::Help) => {}
            _ => panic!("expected Command::Help"),
        }
        match parse_args(&to_args(&["fluxon", "-h"])) {
            Some(Command::Help) => {}
            _ => panic!("expected Command::Help"),
        }
    }

    // issue #136: the assert primitive — a truthy condition passes silently, a falsy
    // condition gives a runtime error with the message (the file becomes FAIL).
    #[test]
    fn assert_primitivi() {
        run(r#"
assert true
assert (1 + 1 == 2) "math works"
assert "a non-empty str is also truthy"
"#);
        let err = run_source(r#"assert (1 == 2) "one is not two""#).unwrap_err();
        assert!(
            err.contains("assert failed: one is not two"),
            "message not as expected: {}",
            err
        );
        // the variant without a message fails too
        let err = run_source("assert false").unwrap_err();
        assert!(err.contains("assert failed"), "message: {}", err);
        // nil is falsy too
        assert!(run_source("assert nil").is_err());
    }

    // issue #136: `fluxon test` file discovery — .fx files from a directory,
    // recursive, ordered; a single file as-is; a missing path/empty directory -> error.
    #[test]
    fn test_fayllarini_topish() {
        let dir = std::env::temp_dir().join(format!("fluxon_test_disc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // leftover from a previous failed run
        let sub = dir.join("ichki");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.join("b.fx"), "assert true").unwrap();
        std::fs::write(dir.join("a.fx"), "assert true").unwrap();
        std::fs::write(dir.join("eslatma.txt"), "not fx").unwrap();
        std::fs::write(sub.join("c.fx"), "assert true").unwrap();

        let files = collect_test_files(&dir).unwrap();
        let names: Vec<String> = files
            .iter()
            .map(|p| p.strip_prefix(&dir).unwrap().display().to_string())
            .collect();
        assert_eq!(names, ["a.fx", "b.fx", "ichki/c.fx"]);

        // a single file — the list consists of only that file
        let one = collect_test_files(&dir.join("a.fx")).unwrap();
        assert_eq!(one.len(), 1);

        // an explicit non-.fx file — a discovery error (not executed as Fluxon)
        let err = collect_test_files(&dir.join("eslatma.txt")).unwrap_err();
        assert!(err.contains("is not a .fx file"), "message: {}", err);

        // a non-existent path — error
        assert!(collect_test_files(&dir.join("yoq")).is_err());

        // a directory with no .fx — error (a silent "0 files passed" would mislead)
        let empty = dir.join("bosh");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(collect_test_files(&empty).is_err());

        // a looping symlink (a directory pointing to itself) must not cause infinite
        // recursion — file_type() does not follow the symlink, the loop is simply skipped.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&dir, dir.join("halqa")).unwrap();
            let with_loop = collect_test_files(&dir).unwrap();
            assert_eq!(with_loop.len(), 3, "a loop should not change the file list");
        }

        // an unreadable subdirectory must not be silently skipped — an error must be
        // raised (codex P2). root bypasses permission restrictions, so we only check
        // in an environment where the restriction actually applies (the CI runner is non-root).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let yopiq = dir.join("yopiq");
            std::fs::create_dir_all(&yopiq).unwrap();
            std::fs::write(yopiq.join("d.fx"), "assert true").unwrap();
            std::fs::set_permissions(&yopiq, std::fs::Permissions::from_mode(0o000)).unwrap();
            if std::fs::read_dir(&yopiq).is_err() {
                let err = collect_test_files(&dir).unwrap_err();
                assert!(err.contains("could not read"), "message: {}", err);
            }
            // restore the permission for cleanup
            std::fs::set_permissions(&yopiq, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    // issue #136: a failed file does not stop the rest — each file is counted
    // separately and the final (PASS, FAIL) count comes out correct.
    #[test]
    fn test_runner_fail_keyingisini_toxtatmaydi() {
        let dir = std::env::temp_dir().join(format!("fluxon_test_run_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // leftover from a previous failed run
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("01_yiqiladi.fx"), r#"assert false "on purpose""#).unwrap();
        std::fs::write(dir.join("02_otadi.fx"), "assert (2 > 1)").unwrap();

        let files = collect_test_files(&dir).unwrap();
        let (passed, failed) = run_test_files(&files);
        assert_eq!((passed, failed), (1, 1));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    // issue #57: when a symbol turns into TEXT the `:` prefix is dropped
    // (interpolation, str.str, `+` concatenation). The symbol literal syntax
    // (`:florist`) is unchanged — only the text representation is without `:`.
    #[test]
    fn sym_to_text_colon_tashlanadi() {
        run(r#"
s = :florist
# interpolation
(("v/${s}") == "v/florist") | (fail "interpolation: ${"v/${s}"}")
# str.str
((str.str s) == "florist") | (fail "str.str: ${str.str s}")
# `+` concatenation (both sides)
(("p/" + s) == "p/florist") | (fail "left + : ${"p/" + s}")
((s + "/q") == "florist/q") | (fail "right + : ${s + "/q"}")
# symbol literal and comparison are UNCHANGED
(s == :florist) | (fail "symbol comparison broke")
"#);
    }

    // INSIDE a list/map a symbol KEEPS the `:` prefix — there a symbol must be
    // distinguishable from a string (repr differs from the text representation).
    #[test]
    fn sym_repr_listda_colon_saqlaydi() {
        run(r#"
xs = [:a "b"]
((str.str xs) == "[:a \"b\"]") | (fail "list repr: ${str.str xs}")
"#);
    }

    // --- auth battery (issue #69) ---
    //
    // A lock for tests that need the $AUTH_SECRET env — so parallel tests do not
    // race on the env (the AI_ENV_LOCK pattern).
    static AUTH_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn auth_jwt_verify_roundtrip() {
        let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("AUTH_SECRET", "sirli-kalit-123") };
        run(r#"
use auth
token = auth.jwt {sub:"u1" tenant:"t1" role:"admin"}
# a signed JWT — 3 segments (header.payload.signature)
parts = str.split token "."
(parts.len == 3) | (fail "JWT 3 segment not: ${parts.len}")
# verify -> returns the payload map, claims are preserved
claims = auth.verify token
(claims.sub == "u1") | (fail "sub wrong: ${claims.sub}")
(claims.tenant == "t1") | (fail "tenant wrong: ${claims.tenant}")
(claims.role == "admin") | (fail "role wrong: ${claims.role}")
# iat/exp added automatically
(claims.exp > claims.iat) | (fail "exp should be greater than iat")
"#);
    }

    #[test]
    fn auth_verify_buzilgan_token_xato() {
        let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("AUTH_SECRET", "sirli-kalit-123") };
        // A token with a tampered signature -> auth.verify err (in Fluxon `try` is a
        // passthrough, the error stops the run — so we check with expect_err on the
        // Rust side). Adding a character to the token makes the signature mismatch.
        let err = run_source(
            r#"use auth
token = auth.jwt {sub:"u1"}
auth.verify (token + "x")"#,
        )
        .expect_err("a tampered token should error");
        assert!(
            err.contains("signature"),
            "expected signature error, got: {}",
            err
        );
    }

    #[test]
    fn auth_verify_yaroqsiz_shakl_xato() {
        let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("AUTH_SECRET", "sirli-kalit-123") };
        // Fewer than 3 segments — the JWT format is invalid -> err.
        let err = run_source(
            r#"use auth
auth.verify "faqat.ikki""#,
        )
        .expect_err("an invalid format should error");
        assert!(
            err.contains("format") || err.contains("segment"),
            "expected format error, got: {}",
            err
        );
    }

    #[test]
    fn auth_verify_exp_siz_token_rad_etiladi() {
        let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("AUTH_SECRET", "sirli-kalit-123") };
        // An `exp:nil` payload -> auth.jwt `or_insert` does not override nil,
        // i.e. the token is signed without a numeric `exp`. Even if correctly signed,
        // auth.verify must REJECT it (otherwise it would be valid forever —
        // Codex P2). The key is correct, so this is an exp error, not a signature one.
        let err = run_source(
            r#"use auth
token = auth.jwt {sub:"u1" exp:nil}
auth.verify token"#,
        )
        .expect_err("a token without exp should be rejected");
        assert!(
            err.contains("exp") || err.contains("expir"),
            "expected exp-missing error, got: {}",
            err
        );
    }

    #[test]
    fn auth_secret_yoq_bolsa_aniq_xato() {
        let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::var("AUTH_SECRET").ok();
        unsafe { std::env::remove_var("AUTH_SECRET") };
        let err = run_source(
            r#"use auth
token = auth.jwt {sub:"u1"}"#,
        )
        .expect_err("a missing $AUTH_SECRET should error");
        if let Some(v) = saved {
            unsafe { std::env::set_var("AUTH_SECRET", v) };
        }
        assert!(
            err.contains("AUTH_SECRET"),
            "expected AUTH_SECRET error, got: {}",
            err
        );
    }

    #[test]
    fn auth_hash_check_roundtrip() {
        // hash/check do not need the env (no lock required).
        run(r#"
use auth
h = auth.hash "user-parol"
# argon2id PHC string
(str.has h "argon2id") | (fail "argon2id hash not: ${h}")
# correct password -> true
(auth.check "user-parol" h) | (fail "check returned false for correct password")
# wrong password -> false
((auth.check "wrong-password" h) == false) | (fail "check returned true for wrong password")
"#);
    }

    #[test]
    fn auth_noma_lum_funksiya_xato() {
        // auth.foo -> reaches dispatch and gives "no auth.foo" (NOT unknown name).
        let err = run_source(r#"auth.foo "x""#).expect_err("an unknown auth function should error");
        assert!(
            err.contains("auth.foo") && !err.contains("unknown name"),
            "auth should reach dispatch and give a function error, got: {}",
            err
        );
    }

    #[test]
    fn auth_ozgaruvchi_modulni_yopadi() {
        // If `auth` is declared as a variable, it is not a module — a plain map.
        run(r#"
auth = {jwt:"shadowed"}
log "auth.jwt = ${auth.jwt}"
"#);
    }

    // Issue #106: a parse error inside string interpolation must point to the
    // original line (not collapse to "on line 1") and must arrive with the
    // "inside interpolation:" prefix.
    #[test]
    fn interp_parse_xatosi_asl_qatorni_korsatadi() {
        let err = run_source("log \"a\"\nlog \"b\"\nlog \"c\"\nlog \"d\"\nlog \"${x +}\"\n")
            .expect_err("a broken interpolation expression should error");
        assert!(
            err.contains("on line 5"),
            "error should point to the original line (5), got: {}",
            err
        );
        assert!(
            err.contains("inside interpolation"),
            "the parse error should also carry the 'inside interpolation' prefix, got: {}",
            err
        );
    }

    // Issue #106: a lex error also preserves the original line. A multi-line
    // expression does not break the line count either — the inner string opens on line 3.
    #[test]
    fn interp_lex_xatosi_asl_qatorni_korsatadi() {
        let err = run_source("log \"a\"\nlog \"b\"\nlog \"v=${\"x\ny\"}\"\n")
            .expect_err("a multi-line inner string should error");
        assert!(
            err.contains("inside interpolation") && err.contains("on line 3"),
            "the lex error should point to the original line (3), got: {}",
            err
        );
    }

    // Issue #106: the ${...} boundary accounts for inner string literals —
    // a `}` inside a string does not close the interpolation early.
    #[test]
    fn interp_ichki_string_qavsni_yopmaydi() {
        run(r#"
x = "v: ${"inner } brace"}"
(x == "v: inner } brace") | (fail "inner string brace wrong ishlandi: ${x}")
"#);
    }

    // Issue #106: an escaped quote (\") inside an inner string does not close the
    // string, and the `}` after it does not close the interpolation either.
    #[test]
    fn interp_ichki_string_escape_tirnoq() {
        run(r#"
x = "x=${"a\"}b"}"
(x == "x=a\"}b") | (fail "escaped quote wrong ishlandi: ${x}")
"#);
    }

    // Issue #130: """ block string — the common indentation is stripped, if the
    // closing """ is on its own line there is no trailing \n.
    #[test]
    fn blok_satr_dedent_va_trailing_yoq() {
        run(r#"
s = """
  hello
  world
  """
(s == "hello\nworld") | (fail "block string dedent error: ${s}")
"#);
    }

    // Issue #130: ${expr} and $ident interpolation work inside a block string.
    #[test]
    fn blok_satr_interpolatsiya() {
        run(r#"
name = "fluxon"
n = 2
s = """
  hello ${name}!
  n+1 = ${n + 1}
  short: $name
  """
(s == "hello fluxon!\nn+1 = 3\nshort: fluxon") | (fail "block string interpolation error: ${s}")
"#);
    }

    // Issue #130: an empty line becomes \n, `"` and `""` are free without escaping —
    // JSON/HTML snippets are written directly.
    #[test]
    fn blok_satr_bosh_qator_va_tirnoq() {
        run(r#"
s = """
  a "quoted"

  {"json": true}
  """
(s == "a \"quoted\"\n\n{\"json\": true}") | (fail "block string quote/empty line error: ${s}")
"#);
    }

    // Issue #130: lines deeper than the minimal indentation keep their relative
    // position (so the inner structure of SQL/a prompt is not broken).
    #[test]
    fn blok_satr_nisbiy_chekinish() {
        run(r#"
s = """
  SELECT *
    FROM t
  """
(s == "SELECT *\n  FROM t") | (fail "relative indentation should be preserved needed: ${s}")
"#);
    }

    // Issue #130: the closing """ may also come at the end of a content line.
    #[test]
    fn blok_satr_kontent_qatorida_yopilish() {
        run(r#"
s = """
  one line"""
(s == "one line") | (fail "closing on a content line error: ${s}")
"#);
    }

    // Issue #130: a block string also works inside an indented block (an fn body) —
    // the lines within the string do not emit INDENT/DEDENT.
    #[test]
    fn blok_satr_fn_ichida() {
        run(r#"
fn f x ->
  s = """
    inner ${x}
    """
  ret s
((f "a") == "inner a") | (fail "block string inside fn error")
"#);
    }

    // Issue #130: if three consecutive quotes are needed, write \""".
    #[test]
    fn blok_satr_escape_uchta_tirnoq() {
        run(r#"
s = """
  three: \"""
  """
(s == "three: \"\"\"") | (fail "escape quote error: ${s}")
"#);
    }

    // Issue #130: text on the same line after the opening """ — a clear error
    // (the one canonical way: content starts on a new line).
    #[test]
    fn blok_satr_ochilishda_matn_xato() {
        let err = run_source("s = \"\"\"matn\nx\"\"\"\n")
            .expect_err("text on the opening line should error");
        assert!(err.contains("a new line"), "unexpected error: {}", err);
    }

    // Issue #130: an unterminated block string gives a clear error (with the opening line).
    #[test]
    fn blok_satr_yopilmagan_xato() {
        let err = run_source("s = \"\"\"\n  abc\n")
            .expect_err("an unterminated block string should error");
        assert!(
            err.contains("unterminated block string") && err.contains("on line 1"),
            "unexpected error: {}",
            err
        );
    }

    // Issue #131: the crypto battery is accessible from Fluxon code — both a call
    // with arguments (Call) and the argument-less `crypto.uuid` (Field) work.
    #[test]
    fn crypto_battery_fluxon_kodidan() {
        run(r#"
h = crypto.sha256 "abc"
(h == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad") | (fail "sha256 broke: ${h}")
sig = crypto.hmac "Jefe" "what do ya want for nothing?"
(sig == "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843") | (fail "hmac broke: ${sig}")
((crypto.b64d (crypto.b64 "hello world")) == "hello world") | (fail "b64 roundtrip broke")
((crypto.hex "abz") == "61627a") | (fail "hex broke")
u = crypto.uuid
((str.len u) == 36) | (fail "uuid uzunligi broke: ${u}")
(u != crypto.uuid) | (fail "uuid takrorlandi")
"#);
    }

    // Issue #131: crypto.b64d gives a clear error on invalid input (not a panic).
    #[test]
    fn crypto_b64d_xato_beradi() {
        let err = run_source("crypto.b64d \"this is not base64!!!\"")
            .expect_err("invalid base64 should error");
        assert!(err.contains("base64"), "unexpected error: {}", err);
    }

    // Issue #131 (review): if the user has declared the name `crypto`
    // (e.g. a `use ./crypto` module), it is not the battery — theirs wins. Same
    // shadowing behavior as auth/ai, on both the Call and the Field path.
    #[test]
    fn crypto_lokal_nom_battery_dan_ustun() {
        run(r#"
crypto = {sha256: \s -> "meniki ${s}" uuid: 7}
((crypto.sha256 "x") == "meniki x") | (fail "lokal crypto.sha256 column did not happen")
((crypto.uuid) == 7) | (fail "lokal crypto.uuid column did not happen")
"#);
    }

    // Issue #132: bytes type basics — of/str/len/slice, equality, Display.
    #[test]
    fn bytes_turi_asoslari() {
        run(r#"
b = bytes.of "hello"
((bytes.len b) == 5) | (fail "bytes.len broke")
((bytes.str b) == "hello") | (fail "bytes.str broke")
(b == (bytes.of "hello")) | (fail "bytes equality broke")
(b != (bytes.of "other")) | (fail "bytes inequality broke")
part = bytes.slice b 0 2
((bytes.str part) == "he") | (fail "bytes.slice broke")
("${b}" == "<bytes 5>") | (fail "bytes interpolation representation broke: ${b}")
"#);
    }

    // Issue #132: bytes.len measures BYTES, str.len measures CHARACTERS — the
    // difference shows in text with diacritics (’ U+2019 = 3 bytes, 1 character).
    #[test]
    fn bytes_len_bayt_str_len_belgi() {
        run(r#"
s = "o’zbek"
((str.len s) == 6) | (fail "str.len belgi sanashi needed")
((bytes.len (bytes.of s)) == 8) | (fail "bytes.len bayt sanashi needed")
"#);
    }

    // Issue #132: integration with crypto — b64db binary decoding, bytes inputs
    // give the same result as str.
    #[test]
    fn bytes_crypto_integratsiya() {
        run(r#"
data = crypto.b64db "AP/+iA=="
((bytes.len data) == 4) | (fail "b64db uzunlik broke")
((crypto.b64 data) == "AP/+iA==") | (fail "bytes b64 aylanasi broke")
((crypto.sha256 (bytes.of "abc")) == (crypto.sha256 "abc")) | (fail "sha256 bytes/str differ")
"#);
    }

    // Issue #132: bytes.str gives a clear error on non-UTF-8 bytes (not silent corruption).
    #[test]
    fn bytes_str_yaroqsiz_utf8_xato() {
        let err = run_source("bytes.str (crypto.b64db \"//4=\")")
            .expect_err("invalid UTF-8 should error");
        assert!(err.contains("UTF-8"), "unexpected error: {}", err);
    }

    // Issue #132: a binary round-trip with fs — bytes are written, fs.readb returns
    // exactly those bytes (the image/PDF scenario).
    #[test]
    fn bytes_fs_integratsiya() {
        run(r#"
yol = "/tmp/fluxon_bytes_it_" + (rand.str 10) + ".bin"
fs.write yol (crypto.b64db "AP/+iA==")
b = fs.readb yol
((bytes.len b) == 4) | (fail "fs.readb uzunlik broke")
((crypto.b64 b) == "AP/+iA==") | (fail "fs ikkilik aylanasi broke")
fs.del yol
((fs.readb yol) == nil) | (fail "deleted fayl nil should be")
"#);
    }

    // Issue #132: json.enc encodes bytes as base64 text (without loss).
    #[test]
    fn bytes_json_enc_base64() {
        run(r#"
b = crypto.b64db "AP/+iA=="
((json.enc {fayl:b}) == "{\"fayl\":\"AP/+iA==\"}") | (fail "json.enc bytes broke")
"#);
    }

    // Issue #138: the REPL runs one block and returns the last expression's VALUE
    // (to print). `run` returns () — this difference lets the REPL show the result.
    // lex_parse + run_repl_chunk is exactly how the REPL works.
    fn repl_chunk(interp: &interp::Interp, src: &str) -> Result<value::Value, String> {
        interp.run_repl_chunk(&lex_parse(src)?)
    }

    // Value does NOT derive Debug/PartialEq (closures) — we compare the value via its
    // `repr()` text (the REPL also prints exactly the repr).
    #[test]
    fn repl_oxirgi_ifoda_qiymatini_qaytaradi() {
        let interp = interp::Interp::new_arc();
        // An expression value returns
        assert_eq!(repl_chunk(&interp, "1 + 2").unwrap().repr(), "3");
        // A bind (declaration) returns nil — the REPL does NOT print such a result
        assert!(matches!(
            repl_chunk(&interp, "x = 10").unwrap(),
            value::Value::Nil
        ));
        // Last stmt value: `x` from the previous chunk is visible (state persists)
        assert_eq!(repl_chunk(&interp, "x * 3").unwrap().repr(), "30");
        // A string value is shown with quotes in repr
        assert_eq!(
            repl_chunk(&interp, r#""hello""#).unwrap().repr(),
            "\"hello\""
        );
    }

    #[test]
    fn repl_state_chunklar_orasida_saqlanadi() {
        let interp = interp::Interp::new_arc();
        // The fn definition in one chunk, the call in the next — they live in one interp.
        repl_chunk(&interp, "fn sq n\n  ret n * n").unwrap();
        assert_eq!(repl_chunk(&interp, "sq 9").unwrap().repr(), "81");
        // a variable with <- and then reading it
        repl_chunk(&interp, "c <- 0").unwrap();
        repl_chunk(&interp, "c <- c + 5").unwrap();
        assert_eq!(repl_chunk(&interp, "c").unwrap().repr(), "5");
    }

    #[test]
    fn repl_xato_qaytadi_sessiya_oldinmas() {
        let interp = interp::Interp::new_arc();
        // An unknown name returns an error (not a panic) — the REPL prints it to stderr
        // and continues. The next chunk works normally (the interp is not corrupted).
        assert!(repl_chunk(&interp, "nosuchvar + 1").is_err());
        assert_eq!(repl_chunk(&interp, "1 + 1").unwrap().repr(), "2");
    }

    #[test]
    fn repl_multiline_block_heuristikasi() {
        // A single-line expression — not a block (evaluated as soon as it parses).
        assert!(!is_multiline_block("1 + 2"));
        // if + an indented body — a block (else/continuation may come, awaited).
        assert!(is_multiline_block("if x > 5\n  \"big\""));
        // tab indentation also counts as a block.
        assert!(is_multiline_block("fn f\n\tret 1"));
        // An empty buffer — not a block.
        assert!(!is_multiline_block(""));
    }
}
