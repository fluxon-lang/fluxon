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
mod check;
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

// Checks the syntax: lex + parse, plus a static immutability pass — but skips
// interp, so the code is NOT executed (no side effects). The immutability pass
// (issue #178) catches reassignment of a `=`-bound var, including from inside a
// block, statically — a trap that previously only surfaced as a runtime 500 on
// the specific request that hit it. On success Ok(()), otherwise the error text.
fn check_source(src: &str) -> Result<(), String> {
    let toks = lexer::lex(src)?;
    let prog = parser::parse(toks)?;
    check::check_immutability(&prog)?;
    Ok(())
}

// Runs the source. `path` is the file's path; `use ./file` modules are resolved
// relative to this file's directory.
fn run_source_at(src: &str, path: &std::path::Path) -> Result<(), String> {
    let toks = lexer::lex(src)?;
    let prog = parser::parse(toks)?;
    // Static immutability check (issue #178) BEFORE running: catch a reassigned
    // `=`-bound var now, so the server fails fast at load instead of 500-ing on
    // the one request that reaches the offending block.
    check::check_immutability(&prog)?;
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
mod tests;
