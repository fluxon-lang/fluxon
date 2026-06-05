// Flux runtime — buyruq qatori interfeysi.
//
// Foydalanish:
//   flux run <fayl.fx>     — Flux faylini bajaradi
//   flux <fayl.fx>         — xuddi shu (qisqartma)

mod token;
mod lexer;
mod ast;
mod parser;
mod value;
mod interp;
mod builtins;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let path = match parse_args(&args) {
        Some(p) => p,
        None => {
            eprintln!("Foydalanish: flux run <fayl.fx>");
            return ExitCode::from(2);
        }
    };

    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Faylni o'qib bo'lmadi '{}': {}", path, e);
            return ExitCode::from(1);
        }
    };

    match run_source(&src) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Flux xato: {}", e);
            ExitCode::from(1)
        }
    }
}

fn parse_args(args: &[String]) -> Option<String> {
    match args.get(1).map(|s| s.as_str()) {
        Some("run") => args.get(2).cloned(),
        Some(p) if !p.starts_with('-') => Some(p.to_string()),
        _ => None,
    }
}

fn run_source(src: &str) -> Result<(), String> {
    let toks = lexer::lex(src)?;
    let prog = parser::parse(toks)?;
    let mut interp = interp::Interp::new();
    interp.run(&prog)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Kichik yordamchi: manbani bajaradi, xato bo'lsa panic.
    fn run(src: &str) {
        run_source(src).unwrap_or_else(|e| panic!("xato: {}", e));
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

    #[test]
    fn map_operations() {
        run(r#"
u = {name:"Aziza" age:30}
u2 = u.set "age" 31
log "keys=${u.keys} hasName=${u.has "name"} age=${u2.age}"
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

    #[test]
    fn match_symbols() {
        run(r#"
fn label s
  match s
    :new -> "yangi"
    :done -> "tugadi"
    _ -> "boshqa"

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

    #[test]
    fn fail_as_expr_and_guard() {
        // fail ifoda kontekstida (guard) — oqimni uzadi, yuqoriga ko'tariladi.
        let err = run_source(
            r#"
fn check x
  x > 0 | (fail 422 "musbat bo'lishi kerak")
  "ok"
log (check 5)
log (check 0)
"#,
        )
        .unwrap_err();
        assert!(err.contains("422"), "kutilgan 422, topildi: {}", err);
    }

    #[test]
    fn pipe_and_coalesce() {
        run(r#"
fn inc x -> x + 1
fn sq x -> x * x
r = 3 |> inc |> sq
log "r=${r}"
m = {a:1}
log "missing=${m.b ?? "yo'q"}"
"#);
    }
}
