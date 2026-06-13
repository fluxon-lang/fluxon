// Fluxon runtime — buyruq qatori interfeysi.
//
// Foydalanish:
//   fluxon run <fayl.fx>     — Fluxon faylini bajaradi
//   fluxon <fayl.fx>         — xuddi shu (qisqartma)
//   fluxon check <fayl.fx>   — faqat lex+parse (bajarmaydi); parse xato -> exit 2
//   fluxon test [yo'l]       — test fayllarini ishga tushiradi (standart: tests/);
//                              yo'l fayl yoki katalog bo'lishi mumkin
//   fluxon repl              — interaktiv REPL (read-eval-print); argumentsiz
//                              `fluxon` ham xuddi shu REPL'ni ochadi
//   fluxon --version         — build qilingan package versiyasini chiqaradi
//   fluxon --help            — foydalanish yo'riqnomasini chiqaradi

// mimalloc — parallel'da system malloc'dan ancha kam contention beradi.
// Interpreter qisqa umrli scope allokatsiyalarini ko'p qiladi (tree-walking).
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

// Buyruq turi: `run` kodni bajaradi, `check` faqat sintaksisni tekshiradi.
// Exit kodlari ataylab farqli: faylni o'qib bo'lmasa/runtime xato -> 1,
// foydalanish/parse xato -> 2 (chaqiruvchi qaysi bosqichda yiqilganini biladi).
enum Command {
    Run(String),
    Check(String),
    // test: yo'l ixtiyoriy — berilmasa standart `tests/` katalogi ishlatiladi.
    Test(Option<String>),
    // repl: interaktiv read-eval-print sessiyasi (argument yo'q).
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
        // run: LEX -> PARSE -> BAJAR. Xato (parse yoki runtime) -> exit 1.
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
        // check: faqat LEX + PARSE (interp YO'Q -> side-effect yo'q). Forge
        // eval-gate QATLAM 1: AI yozgan blok sintaktik to'g'rimi, bajarmasdan.
        // Parse/lex xato -> exit 2 (runtime exit 1 dan farqli).
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
        // test: bitta fayl o'qimaydi — o'zi fayllarni topib ishga tushiradi.
        Command::Test(path) => run_tests(path.as_deref()),
        // repl: stdin'dan o'qib, har blokni bajaradi (interaktiv sessiya).
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

// Faylni o'qiydi; xatoda xabarni chiqarib, chiqish kodini (1) qaytaradi.
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
        // Argumentsiz `fluxon` — interaktiv REPL (odam tilni tez sinab ko'rsin).
        None => Some(Command::Repl),
        _ => None,
    }
}

// `fluxon test [yo'l]` — issue #136. Yo'l berilmasa joriy katalogdagi `tests/`
// ishlatiladi; fayl berilsa faqat o'sha fayl, katalog berilsa ichidagi barcha
// .fx fayllar (rekursiv, nom bo'yicha tartibda). Har fayl alohida interp bilan
// bajariladi: xatosiz tugasa PASS, xato (assert yiqilishi ham) bo'lsa FAIL —
// keyingi fayllar baribir ishlaydi. Exit: hammasi o'tsa 0, FAIL bor -> 1,
// yo'l/fayl topilmasa 2 (run/check exit konvensiyasiga mos).
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

// Test fayllar ro'yxatini tuzadi. Fayl -> o'zi; katalog -> ichidagi .fx'lar.
// Bo'sh ro'yxat ham xato: "test o'tdi" deb jim chiqish chalg'ituvchi bo'lardi.
fn collect_test_files(target: &std::path::Path) -> Result<Vec<std::path::PathBuf>, String> {
    let files = if target.is_file() {
        // .fx bo'lmagan fayl (codex P2): Fluxon sifatida parse qilib FAIL/exit 1
        // berish chalg'ituvchi — bu discovery xatosi (exit 2), test natijasi emas.
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

// IO xatolari yutilmaydi (codex P2): o'qib bo'lmaydigan ichki katalog jim
// o'tkazilsa, "hammasi o'tdi" hisoboti aslida topilmagan testlarni yashirardi.
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
        // file_type() symlink'ni KUZATMAYDI — halqali symlink (tests/x -> tests/)
        // cheksiz rekursiyaga olib bormasin. Symlink'langan .fx fayl baribir
        // kiradi (pastdagi is_file symlink'ni kuzatadi), symlink-katalog esa yo'q.
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

// Fayllarni ketma-ket bajaradi, (PASS, FAIL) sonini qaytaradi. Assert
// hisoblagichi har fayldan oldin reset qilinadi — "N assert" fayl o'zinikini
// ko'rsatadi (fayllar bitta protsessda ketma-ket ishlaydi).
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

// Sintaksisni tekshiradi: lex + parse, lekin interp'ni o't kazib yuboradi —
// kod BAJARILMAYDI (side-effect yo'q). Muvaffaqiyatda Ok(()), aks holda xato matni.
fn check_source(src: &str) -> Result<(), String> {
    let toks = lexer::lex(src)?;
    parser::parse(toks)?;
    Ok(())
}

// Manbani bajaradi. `path` — faylning yo'li; `use ./fayl` modullari shu faylning
// katalogiga nisbatan hal qilinadi.
fn run_source_at(src: &str, path: &std::path::Path) -> Result<(), String> {
    let toks = lexer::lex(src)?;
    let prog = parser::parse(toks)?;
    // Arc<Interp>: http.serve handler'larni server thread'larida apply qiladi,
    // shuning uchun interp thread'lar orasida ulashiladigan bo'lishi kerak.
    let interp = interp::Interp::new_arc();
    // `use ./fayl` uchun base — top-level faylning katalogi.
    if let Some(dir) = path.parent() {
        // parent() bo'sh ("") bo'lsa joriy katalog (default) qoladi.
        if !dir.as_os_str().is_empty() {
            interp.set_base(dir);
        }
    }
    interp.run(&prog)
}

// Path'siz qulay wrapper — testlar uchun (modul yo'llari joriy katalogga nisbatan).
#[cfg(test)]
fn run_source(src: &str) -> Result<(), String> {
    run_source_at(src, std::path::Path::new("."))
}

// `fluxon repl` — interaktiv read-eval-print (issue #138). Odam tilni o'rganishi
// va bir-ikki qator kodni faylsiz sinab ko'rishi uchun. Bitta interp obyekti
// butun sessiya davomida yashaydi: `x = 1` keyingi qatorda ko'rinadi.
//
// Ko'p qatorli blok: indentatsiyali konstruksiya (if/each/fn/...) bir necha
// qatorga cho'ziladi. Qaysi qatorda blok tugashini xato-matnini tahlil qilib
// emas, balki yig'ilgan buferni qayta parse qilib aniqlaymiz — parse o'tsa
// blok to'liq (eval); parse xato bo'lsa ko'proq qator kutamiz, lekin foydalanuvchi
// BO'SH qator kiritsa (blokni majburan yopsa) xatoni ko'rsatib buferni tozalaymiz.
// Bu lex/parse xato xabarlarining ichki matniga bog'lanmaslik uchun (mo'rt bo'lardi).
fn run_repl() -> ExitCode {
    use std::io::Write;

    let interp = interp::Interp::new_arc();
    // REPL'da `use ./fayl` joriy ishchi katalogga nisbatan hal qilinsin.
    interp.set_base(std::path::Path::new("."));

    println!(
        "Fluxon {} REPL — exit: :q or Ctrl-D, help: :help",
        env!("CARGO_PKG_VERSION")
    );

    let stdin = std::io::stdin();
    // Yig'iladigan bufer (ko'p qatorli blok uchun) — qatorlar `\n` bilan ulanadi.
    let mut buf = String::new();

    loop {
        // Bo'sh buferda asosiy prompt, davom etayotgan blokda `...` prompt.
        let prompt = if buf.is_empty() { "fx> " } else { "... " };
        print!("{}", prompt);
        // print! satr oxirisiz — prompt ko'rinishi uchun majburan flush qilamiz.
        let _ = std::io::stdout().flush();

        let mut line = String::new();
        match stdin.read_line(&mut line) {
            Ok(0) => {
                // EOF (Ctrl-D). Yarim yig'ilgan blok bo'lsa oxirgi marta urinib
                // ko'ramiz, keyin yangi qatorda chiqamiz.
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

        // read_line oxirgi `\n` ni saqlaydi — trailing'ni olib, bo'shlikni tekshiramiz.
        let trimmed = line.trim_end_matches(['\n', '\r']);

        // REPL buyruqlari faqat bufer bo'sh bo'lganda (blok o'rtasida emas).
        if buf.is_empty() {
            match trimmed.trim() {
                ":q" | ":quit" | ":exit" => return ExitCode::SUCCESS,
                ":help" | ":h" => {
                    print_repl_help();
                    continue;
                }
                "" => continue, // bo'sh prompt — hech narsa qilmaymiz
                _ => {}
            }
        }

        // Bo'sh qator + yig'ilayotgan blok: foydalanuvchi blokni majburan yopdi —
        // hozir bor narsani eval qilamiz (parse hali xato bo'lsa xato ko'rinadi).
        let force_eval = !buf.is_empty() && trimmed.trim().is_empty();

        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(trimmed);

        // Indentatsiyali (ko'p qatorli) blokni darhol eval QILMAYMIZ — `else`/`catch`
        // yoki davomi keyingi qatorda kelishi mumkin (masalan `if`+body parse bo'ladi,
        // ammo `else` hali yo'q). Bunday blokni faqat bo'sh qator (force_eval) yopadi.
        // Indentatsiyasiz bir-qatorli ifoda esa parse o'tishi bilan darhol eval bo'ladi
        // (oddiy `1 + 2` uchun bo'sh qator talab qilmaymiz).
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

// Bufer indentatsiyali blokmi (biror qator bo'shliq bilan boshlanadimi)? Shunday
// bo'lsa REPL davomini kutadi (else/catch/qo'shimcha body kelishi mumkin) va faqat
// bo'sh qator blokni yopadi. Birinchi qator hech qachon indentatsiyali bo'lmaydi —
// shuning uchun keyingi qatorlardan birortasi indentatsiyali bo'lsa yetadi.
fn is_multiline_block(buf: &str) -> bool {
    buf.lines()
        .any(|l| l.starts_with(' ') || l.starts_with('\t'))
}

// Bufer mazmunini bajaradi va natijani chop etadi. Xato bo'lsa stderr'ga
// "Fluxon xato: ..." ko'rinishida — sessiya tugamaydi (keyingi promptga o'tadi).
fn repl_eval(interp: &interp::Interp, src: &str) {
    match interp.run_repl_chunk(&match lex_parse(src) {
        Ok(prog) => prog,
        Err(e) => {
            eprintln!("Fluxon error: {}", e);
            return;
        }
    }) {
        // Nil natijani (e'lon, log, assign) chop etmaymiz — shovqin bo'lardi.
        // Boshqa qiymatni `repr` bilan (string'lar tirnoqli) ko'rsatamiz.
        Ok(value::Value::Nil) => {}
        Ok(v) => println!("{}", v.repr()),
        Err(e) => eprintln!("Fluxon error: {}", e),
    }
}

// REPL uchun lex+parse — `run_source_at` ichidagi bilan bir xil, lekin AST'ni
// qaytaradi (REPL chunk'iga uzatish uchun).
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

    // Kichik yordamchi: manbani bajaradi, xato bo'lsa panic.
    fn run(src: &str) {
        run_source(src).unwrap_or_else(|e| panic!("error: {}", e));
    }

    // Issue #139: darajali log — `log.debug/info/warn/err` va bare `log` (=info)
    // dispatch'ga ulangan va xatosiz ishlaydi (stderr'ga chiqaradi, nil qaytaradi).
    // Filtr/format $LOG_LEVEL/$LOG_FORMAT bilan; format mantig'i builtins::log_tests
    // da unit-test qilingan. Bu yerda faqat sintaksis/dispatch tekshiriladi.
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

    // Issue #139: `log` qiymat sifatida (callback/saqlash) ishlashda davom etadi —
    // eski global `log` Native bilan moslik (info-darajali shim). PR #163 review.
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

    // Issue #137: par — til-darajasidagi parallel fan-out. Lambdalar ro'yxatini
    // har birini alohida thread'da chaqiradi, hammasini kutadi, natijalar
    // (kirish tartibida) har biri {ok:...} yoki {err:...}.
    // Eslatma: list ichidagi lambda elementlar QAVS bilan ajraladi — `(\-> ...)`.
    // Lexer list/map ichida Newline token chiqarmaydi (`paren_depth>0`), shuning
    // uchun qavssiz `[\-> a  \-> b]` da birinchi body ikkinchisini argument deb
    // yutardi; qavs body chegarasini aniqlaydi va nested HOF (`\-> xs.map \x ->`)
    // ham buzilmaydi (issue #137 PR review, P2).
    #[test]
    fn par_asosiy_fan_out() {
        run(r#"
r = par [
  (\-> 1 + 1)
  (\-> str.up "hi")
  (\-> [1 2 3].len)
]
((r.len) == 3) | (fail "par 3 results should be returned")
((r.0.ok) == 2) | (fail "1-natija {ok:2} should be")
((r.1.ok) == "HI") | (fail "2-natija {ok:HI} should be")
((r.2.ok) == 3) | (fail "3-natija {ok:3} should be")
"#);
    }

    // Issue #137: qisman muvaffaqiyat — bitta lambda fail qilsa qolganlari
    // to'xtamaydi; xato {err:xabar} bo'lib qaytadi, tartib saqlanadi.
    #[test]
    fn par_qisman_muvaffaqiyat() {
        run(r#"
r = par [
  (\-> 42)
  (\-> fail "on purpose")
  (\-> "uchinchi")
]
((r.0.ok) == 42) | (fail "1-natija ok should be")
((r.1.err) == "on purpose") | (fail "2nd result should be err")
((r.2.ok) == "uchinchi") | (fail "3-natija ok should be")
"#);
    }

    // Issue #137: closure tashqi (sikl/scope) o'zgaruvchini parallel o'qiy oladi.
    #[test]
    fn par_closure_capture() {
        run(r#"
base = 100
r = par [(\-> base + 1) (\-> base + 2)]
((r.0.ok) == 101) | (fail "closure capture 1 broke")
((r.1.ok) == 102) | (fail "closure capture 2 broke")
"#);
    }

    // Issue #137: lambda body ichida nested paren-free HOF (`xs.map \x -> ...`)
    // qavs ichida to'liq o'qiladi — P2 regressiyasi yo'q.
    #[test]
    fn par_nested_hof() {
        run(r#"
r = par [(\-> [1 2 3].map \x -> x + 1)]
((r.0.ok.0) == 2) | (fail "nested HOF 1-element broke")
((r.0.ok.2) == 4) | (fail "nested HOF 3-element broke")
"#);
    }

    // Issue #137: bo'sh ro'yxat -> bo'sh natija (thread ochilmaydi).
    #[test]
    fn par_bosh_royxat() {
        run(r#"
r = par []
((r.len) == 0) | (fail "par [] should return an empty list")
"#);
    }

    // Issue #137: lambda bo'lmagan element aniq xato beradi (thread ochilmasdan).
    #[test]
    fn par_lambda_bolmagan_element_xato() {
        let e = run_source("par [42]").unwrap_err();
        assert!(
            e.contains("must be a function"),
            "a clear error is expected for a non-lambda par element, got: {}",
            e
        );
    }

    // Issue #137 (PR review P2): ikki par lambda bir xil CACHE'LANMAGAN modulni
    // parallel `use ./m` qilsa, ikkalasi ham {ok:...} qaytishi kerak — soxta
    // "sikllik import" emas. module_loading/current_base thread-local bo'lgani
    // uchun parallel import bir-birini sikl deb ko'rmaydi va base buzilmaydi.
    #[test]
    fn par_parallel_modul_import_soxta_sikl_yoq() {
        let dir = std::env::temp_dir().join(format!("fluxon_par_mod_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("m.fx"), "exp fn greet n -> \"hello ${n}\"\n").unwrap();
        let main = dir.join("main.fx");
        // Har lambda alohida thread'da MODULNI BIRINCHI MARTA import qiladi
        // (cache bo'sh) — Codex reproduksiyasi.
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

    // Issue #137: foydalanuvchi `par` nomli o'zgaruvchi e'lon qilsa u ustun
    // (boshqa dispatch-battery'lar bilan izchil shadowing).
    #[test]
    fn par_ozgaruvchi_sifatida_shadow() {
        run(r#"
fn id v -> v
par = (id 7)
(par == 7) | (fail "par did not shadow as a variable")
"#);
    }

    // Issue #137 (PR review P1): par'ni db.tx ichidan chaqirish aniq xato beradi —
    // yangi thread'lar CURRENT_TX TLS'ni meros qilmaydi, jim ravishda tx
    // tashqarisida ishlash o'rniga rad etiladi. (DB test — DB_TEST_LOCK.)
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

    // Issue #139: foydalanuvchi `log` nomli o'zgaruvchi e'lon qilsa, u ustun
    // (battery'ni soyalaydi) — eski shadowing invarianti buzilmaydi.
    #[test]
    fn log_ozgaruvchi_sifatida_shadow() {
        run(r#"
fn log_id v -> v
log = (log_id 42)
(log == 42) | (fail "log did not shadow as a variable")
"#);
    }

    // Issue #93: `log !x` da `!` callee'ga postfix Try bo'lib yopishar edi —
    // `Call(Try(log), [x])` — inkor jim yo'qolardi. Endi bo'shliqdan keyingi
    // `!` prefiks not sifatida argument boshlaydi.
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

    // Issue #93 (regressiya himoyasi): tutash `!` avvalgidek postfix Try —
    // qiymatga yopishadi va muvaffaqiyatda o'tkazgich bo'lib qoladi.
    #[test]
    fn tutash_bang_postfix_try_qoladi() {
        run(r#"
fn safe v -> v
a = (safe 5)!
(a == 5) | (fail "postfix try passthrough broke")
"#);
    }

    // Issue #125: try/catch — `fail` ko'tarilgan xatoni ushlab, qiymat sifatida
    // davom ettiradi. catch o'zgaruvchisi {message, status} map'iga bog'lanadi.
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

    // Statussiz fail va runtime xato — ikkalasi ham ushlanadi; statussizda
    // e.status nil bo'ladi.
    #[test]
    fn try_catch_runtime_xato_va_statussiz() {
        run(r#"
r = try
  fail "boom"
catch e
  (e.status == nil) | (fail "status should be nil for fail without status should be")
  e.message
(r == "boom") | (fail "fail message without status was not caught")

# runtime xato (nolga bo'lish) ham ushlanadi
r2 = try
  1 / 0
catch e
  (e.status == nil) | (fail "runtime error status should be nil should be")
  "ushlandi"
(r2 == "ushlandi") | (fail "runtime error was not caught")
"#);
    }

    // Muvaffaqiyatda body oxirgi ifodasi qaytadi; catch ishlamaydi.
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

    // ret/skip/stop oqim-signallari try'dan o'tib ketadi — catch ularni ushlamaydi.
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

    // Ichma-ich try va catch ichidan qayta fail (re-raise) tashqi try'ga boradi.
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

    // Issue #90: cheksiz rekursiya stack overflow ABORT o'rniga graceful
    // runtime xato qaytarishi kerak (HTTP handler'da butun server o'lmasin).
    #[test]
    fn cheksiz_rekursiya_graceful_xato() {
        let e = run_source("fn f n -> f (n + 1)\nf 0").unwrap_err();
        assert!(e.contains("recursion too deep"), "unexpected error: {}", e);
    }

    // Issue #90: limit xatosidan keyin chuqurlik hisoblagichi to'liq qaytadi —
    // xuddi shu thread'da keyingi bajarish toza boshlanadi (RAII guard).
    #[test]
    fn rekursiya_limitdan_keyin_tiklanish() {
        assert!(run_source("fn f n -> f (n + 1)\nf 0").is_err());
        run(r#"
fn g x -> x + 1
((g 1) == 2) | (fail "call after the limit broke")
"#);
    }

    // Issue #90: ~2000 ichma-ich qavs parser'da stack overflow abort qilardi.
    // Endi limit (256) dan oshganda aniq parse xatosi; 200 daraja esa ishlaydi.
    #[test]
    fn chuqur_qavs_parse_limiti() {
        let deep = format!("x = {}1{}", "(".repeat(300), ")".repeat(300));
        let e = check_source(&deep).unwrap_err();
        assert!(e.contains("too deep"), "unexpected error: {}", e);

        let ok = format!("x = {}1{}", "(".repeat(200), ")".repeat(200));
        check_source(&ok).unwrap_or_else(|e| panic!("200 levels should pass: {}", e));
    }

    // Issue #89: int arifmetika overflow'da panic (debug) / jim wrap (release)
    // o'rniga ikkala rejimda ham bir xil Fluxon xatosi qaytadi.
    #[test]
    fn int_overflow_xato_panic_emas() {
        // + overflow (debug'da panic berardi)
        let e = run_source("log (9223372036854775806 + 2)").unwrap_err();
        assert!(e.contains("number out of range"), "unexpected error: {}", e);
        // i64::MIN / -1 — Rust'da release'da ham panic berardi
        let e = run_source(
            r#"
a = 0 - 9223372036854775807 - 1
log (a / (0 - 1))
"#,
        )
        .unwrap_err();
        assert!(e.contains("number out of range"), "unexpected error: {}", e);
        // i64::MIN % -1 — xuddi shu oila
        let e = run_source(
            r#"
a = 0 - 9223372036854775807 - 1
log (a % (0 - 1))
"#,
        )
        .unwrap_err();
        assert!(e.contains("number out of range"), "unexpected error: {}", e);
        // unar minus ham: -(i64::MIN) sig'maydi
        let e = run_source(
            r#"
a = 0 - 9223372036854775807 - 1
log (-a)
"#,
        )
        .unwrap_err();
        assert!(e.contains("number out of range"), "unexpected error: {}", e);
        // * va - ham checked
        assert!(run_source("log (4611686018427387904 * 2)").is_err());
        assert!(run_source("log (0 - 9223372036854775807 - 2)").is_err());
        // Oddiy arifmetika avvalgidek ishlaydi
        run(r#"
((2 + 3) == 5) | (fail "sum broke")
((7 / 2) == 3) | (fail "division broke")
((7 % 2) == 1) | (fail "mod broke")
((-(5)) == (0 - 5)) | (fail "unary minus broke")
"#);
    }

    // Issue #89: range oxiri i64::MAX bo'lganda `i += 1` toshib ketardi —
    // endi oxirgi elementdan keyin to'xtaydi.
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

    // Issue #99: `..` arifmetikadan PAST, lekin pipe/taqqoslashdan YUQORI
    // bog'lanadi. `1..n+1` = `1..(n+1)` (AI uchun tabiiy), avval `(1..n)+1`
    // bo'lib runtime xato berardi. Pipe esa butun range'ni o'raydi.
    #[test]
    fn range_ustuvorligi() {
        run(r#"
n = 3
# end tomon: +1 butun range'ga emas, faqat n'ga qo'llanadi
(1..n+1 == [1 2 3 4]) | (fail "1..n+1 wrong")
# end tomon: -1
(0..n-1 == [0 1 2]) | (fail "0..n-1 wrong")
# har ikki tomon arifmetika bilan
(2*1..2+1 == [2 3]) | (fail "2*1..2+1 wrong")
# each loop ichida ham xatosiz ishlaydi
sum <- 0
each i in 1..n+1
  sum <- sum + i
(sum == 10) | (fail "each 1..n+1 sum wrong: ${sum}")
"#);
    }

    // Issue #99 (review): pipe range'dan PASTROQ bog'lanadi, shuning uchun
    // `1..3 |> f` = `(1..3) |> f` — qurilgan range f'ga uzatiladi, qavssiz.
    #[test]
    fn range_pipe_butun_diapazonni_uzatadi() {
        run(r#"
fn total xs
  xs.reduce 0 \acc x -> acc + x
# pipe butun range'ga (1..3 = [1 2 3]) qo'llanadi, end tomonga emas
(1..3 |> total == 6) | (fail "pipe range wrong")
"#);
    }

    // Inline if (ternary ekvivalenti): `if shart a else b` bir qiymat qaytaradi.
    // Issue #66 — ixcham shartli ifoda (leading-zero formatlash kabi joylar uchun).
    #[test]
    fn inline_if_ifoda() {
        run(r#"
# Issue'dagi asosiy misol: leading-zero formatlash
h = 5
pad = if h < 10 ("0" + str.str h) else (str.str h)
(pad == "05") | (fail "inline if value did not give: ${pad}")

# shart yolg'on bo'lganda else tarmog'i
x = 20
pad2 = if x < 10 ("0" + str.str x) else (str.str x)
(pad2 == "20") | (fail "else branch did not work: ${pad2}")

# qavssiz oddiy tarmoqlar
y = if h > 3 "big" else "small"
(y == "big") | (fail "branch without parens did not work: ${y}")

# else-if zanjiri (ichma-ich inline if)
g = if h == 0 "zero" else if h < 0 "negative" else "positive"
(g == "positive") | (fail "else-if chain did not work: ${g}")

# chaqiruvli shart qavs ichida
s = "hi"
r = if (str.len s) > 0 "full" else "empty"
(r == "full") | (fail "parenthesized condition did not work: ${r}")

# katta ifoda ichida ishlatish
n = 7
msg = "son " + (if n % 2 == 0 "juft" else "toq")
(msg == "son toq") | (fail "inner inline if did not work: ${msg}")
"#);
    }

    // rep'ning ixtiyoriy 3-argument headers map'i (issue #16). rep shunchaki
    // {__resp:true status body headers} map qaytaradi — Fluxon'da kalitlarini
    // o'qib tekshiramiz (haqiqiy header yozish http_mod testlarida).
    #[test]
    fn rep_headers_argumenti() {
        run(r#"
# 2-argument (eski shakl) — headers kaliti yo'q
r = rep 200 {ok:true}
(r.status == 200) | (fail "rep status broke: ${r.status}")
(r.headers == nil) | (fail "headers key appeared in rep without headers")

# 3-argument — headers map qo'shiladi. Defis o'rniga `_` (map kalitida defis
# bo'lolmaydi; runtime yozishda `_` → `-` qiladi). O'qish ham `_` bilan.
r2 = rep 200 "<h1>Salom</h1>" {content_type:"text/html"}
(r2.headers.content_type == "text/html") | (fail "headers could not be read")

# body map + alohida headers — to'qnashmaydi
r3 = rep 200 {data:1} {set_cookie:"s=abc"}
(r3.body.data == 1) | (fail "body map broke")
(r3.headers.set_cookie == "s=abc") | (fail "set-cookie could not be read")
"#);
    }

    // 3-argument map bo'lmasa rep aniq xato beradi (jim e'tiborsizlik emas).
    #[test]
    fn rep_headers_nomap_xato() {
        let e = run_source(r#"x = rep 200 "body" "notmap""#).unwrap_err();
        assert!(
            e.contains("3rd argument must be headers"),
            "unexpected error: {}",
            e
        );
    }

    // Inline shakl qo'shilgach ham blok shakli (chaqiruvli shart bilan) ishlashi
    // kerak — regressiya tekshiruvi.
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

    // Argumentsiz (nullary) chaqiruv: `f()`. Qavssiz chaqirish argument bilan
    // aniqlangani uchun 0-arity funksiyani chaqirishning yagona yo'li shu.
    // `f` (qavssiz) funksiya QIYMATI, `f()` esa CHAQIRUV.
    #[test]
    fn nullary_call() {
        run(r#"
fn new_id
  ret rand.str 8

a = new_id()
b = new_id()
(str.len a == 8) | (fail "new_id() was not called: ${a}")
(a != b) | (fail "each call did not give a new value")

# qavssiz: funksiya qiymati (chaqirilmaydi) — boolean truthy
f = new_id
(f != nil) | (fail "bare name should be a function value")

# lambda nullary
g = \->
  ret 42
(g() == 42) | (fail "lambda nullary call did not work: ${g()}")
"#);
    }

    // Argumentsiz rekursiya: `tick()` o'zini chaqiradi. Ilgari dummy argument
    // (`tick n`) kiritishga majbur edik — endi shart emas.
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

    // `f(x)` (argument bilan qavsli chaqiruv) RAD ETILADI — canonical shakl `f x`.
    // Bo'sh `()` faqat nullary uchun; bir ish = bir yo'l.
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

    // list.index pozitsiya beradi (topilmasa -1), list.find predikatga mos
    // birinchi elementni (topilmasa nil). has bool, index pozitsiya — juftlik.
    #[test]
    fn list_index_and_find() {
        run(r#"
names = ["catalog_manager" "order_extractor" "billing"]
(names.index "order_extractor" == 1) | (fail "index did not find: ${names.index "order_extractor"}")
(names.index "yoq" == -1) | (fail "none element -1 did not give")

nums = [3 1 4 1 5 9]
(nums.index 4 == 2) | (fail "int index: ${nums.index 4}")

# find: predikatga mos birinchi element
big = nums.find \x -> x > 4
(big == 5) | (fail "find did not return the matching element: ${big}")
none = nums.find \x -> x > 99
(none == nil) | (fail "find topmaganda nil did not give: ${none}")

# index'ni solishtirish uchun ishlatish (issue manbasi: blok tartibi)
a = names.index "catalog_manager"
b = names.index "billing"
(a < b) | (fail "index comparison did not work: ${a} ${b}")
"#);
    }

    // Issue #127: list.sort — argumentsiz tabiiy tartib (son/matn), komparator
    // bilan ixtiyoriy tartib. Asl list o'zgarmaydi (immutable qiymatlar).
    #[test]
    fn list_sort() {
        run(r#"
nums = [3 1 4 1 5]
s = nums.sort
(s == [1 1 3 4 5]) | (fail "natural sort: ${s}")
(nums == [3 1 4 1 5]) | (fail "sort modified the original list: ${nums}")

# komparator: son qaytaradi (manfiy: a oldin) — kamayish tartibi
d = nums.sort \a b -> b - a
(d == [5 4 3 1 1]) | (fail "comparator sort: ${d}")

# matnlar leksikografik
names = ["banan" "olma" "anor"].sort
(names == ["anor" "banan" "olma"]) | (fail "str sort: ${names}")

# int/flt aralash son tartibi
mixed = [2 1.5 1].sort
(mixed == [1 1.5 2]) | (fail "mixed number sort: ${mixed}")

# chekka holatlar
([].sort == []) | (fail "empty list sort")
([7].sort == [7]) | (fail "single element sort")
"#);
    }

    // Issue #127: komparatorli sort stable — teng elementlar asl tartibda qoladi
    // (bir nechta manbadan yig'ilgan map-yozuvlarni maydon bo'yicha saralash).
    #[test]
    fn list_sort_stable_va_maplar() {
        run(r#"
items = [{n:"b" p:2} {n:"a" p:1} {n:"c" p:1}]
sorted = items.sort \a b -> a.p - b.p
ns = sorted.map \x -> x.n
(ns == ["a" "c" "b"]) | (fail "stable map sort: ${ns}")
"#);
    }

    // Issue #127: sort xato yo'llari — aralash tiplar komparatorsiz, komparator
    // son qaytarmasa, zip argumenti list bo'lmasa.
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

    // Issue #127: reverse/uniq/flat/zip — sof list metodlari.
    #[test]
    fn list_reverse_uniq_flat_zip() {
        run(r#"
([1 2 3].reverse == [3 2 1]) | (fail "reverse did not work")
([1 2 1 3 2].uniq == [1 2 3]) | (fail "uniq did not work")

# flat bir daraja tekislaydi; list bo'lmagan element o'z holicha qoladi
([[1 2] [3] 4].flat == [1 2 3 4]) | (fail "flat did not work")

# zip qisqasi tugaganda to'xtaydi
z = [1 2 3].zip ["a" "b"]
(z == [[1 "a"] [2 "b"]]) | (fail "zip did not work: ${z}")
"#);
    }

    // Issue #127: any/all predikat metodlari — filter+len aylanma yo'l o'rniga.
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

# bo'sh list: any false, all true (vacuous)
e1 = [].any \x -> x
(e1 == false) | (fail "empty any false not")
e2 = [].all \x -> x
e2 | (fail "empty all true not")
"#);
    }

    // Hisoblangan indeks: `xs.(ifoda)` va `xs[ifoda]` ikkalasi ham ishlashi kerak.
    // Issue #64 — pagination/oxirgi element olish uchun literal emas, ifoda-indeks.
    #[test]
    fn hisoblangan_indeks() {
        run(r#"
xs = ["a" "b" "c"]
i = xs.len - 1

# .(ifoda) shakli — oxirgi elementni hisoblangan indeks bilan ol
last = xs.(i)
(last == "c") | (fail ".(i) oxirgi elementni did not give: ${last}")

# ichida to'liq ifoda
(xs.(xs.len - 1) == "c") | (fail "xs.(xs.len - 1) did not work")

# bracket shakli ham bir xil natija beradi
(xs[i] == "c") | (fail "xs[i] did not work")

# map'ni hisoblangan kalit (str) bilan indekslash
m = {name: "Ali" age: 30}
k = "name"
(m.(k) == "Ali") | (fail "m.(k) did not work: ${m.(k)}")

# chegaradan tashqari -> nil (mavjud get_index xulqi)
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

    // Issue #129: m.merge other — ikki map'ni birlashtirish (other ustun keladi).
    #[test]
    fn map_merge() {
        run(r#"
# asosiy naqsh: default config + foydalanuvchi override'i
defaults = {host:"localhost" port:8080 debug:false}
user = {port:3000 debug:true}
cfg = defaults.merge user

# other'dagi kalitlar ustun keladi
(cfg.port == 3000) | (fail "merge: other key did not win: ${cfg.port}")
(cfg.debug == true) | (fail "merge: debug override did not happen")
# other'da yo'q kalit asl qiymatda qoladi
(cfg.host == "localhost") | (fail "merge: host lost: ${cfg.host}")
(cfg.len == 3) | (fail "merge: key count wrong: ${cfg.len}")

# asl map'lar o'zgarmaydi (set/del bilan izchil — yangi map qaytadi)
(defaults.port == 8080) | (fail "merge: original map changed")
((user.has "host") == false) | (fail "merge: other map changed")

# bo'sh map bilan merge — o'zini qaytaradi
((defaults.merge {}).len == 3) | (fail "merge: with empty map broke")
(({}.merge defaults).port == 8080) | (fail "merge: merge from empty map broke")
"#);
    }

    // map.merge map bo'lmagan argument bilan tushunarli xato qaytaradi.
    #[test]
    fn map_merge_notogri_argument() {
        let e = run_source(r#"({a:1}).merge 42"#).unwrap_err();
        assert!(e.contains("map.merge"), "unexpected error text: {}", e);
    }

    // Schema map qiymat pozitsiyasidagi bare tip nomi (`{a:str b:int}`) sym'ga
    // aylanadi — docs (`ai.json {product:str qty:int}`) va'da qilgani. `str` ham
    // modul nomi bo'lgani uchun ilgari "noma'lum nom: str" xatosini berardi.
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

# nested list ichidagi map ham ishlasin (`{items:[{product:str qty:int}]}`)
nested = {items:[{product:str qty:int}]}
row = nested.items.0
(row.product == :str) | (fail "nested product :str not")
(row.qty == :int) | (fail "nested qty :int not")

# regressiya: tip nomi BO'LMAGAN ident hamon o'zgaruvchi sifatida qidiriladi
x = 5
m = {n:x}
(m.n == 5) | (fail "oddiy variable value broke: ${m.n}")

# regressiya: str modul-chaqiruvi qiymat sifatida buzilmadi
up = str.up "hello"
(up == "HELLO") | (fail "str.up broke: ${up}")
"#);
    }

    // Issue #98 — ichma-ich raqamli indeks `m.0.1`. Lexer ilgari `.1` ni
    // ochko'zlik bilan `Flt(0.1)` deb yutardi (oldin `.` member konteksti
    // borligini bilmasdan). Endi member-indeksdan keyingi son float
    // boshlamaydi: `m.0.1` ≡ `(m.0).1`.
    #[test]
    fn nested_numeric_index() {
        run(r#"
m = [[1 2] [3 4]]
(m.0.1 == 2) | (fail "m.0.1 != 2: ${m.0.1}")
(m.1.0 == 3) | (fail "m.1.0 != 3: ${m.1.0}")

# uch darajali ichma-ich indeks ham
deep = [[[7 8]]]
(deep.0.0.1 == 8) | (fail "deep.0.0.1 != 8: ${deep.0.0.1}")

# regressiya: oddiy float literallar buzilmadi
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

    // if/each/match bloklari leksik jihatdan SHAFFOF: ichidagi `=` tashqi (bir xil
    // fn'dagi) o'zgaruvchini yangilaydi — boshqa tillar kabi, klon olinmaydi. Bu
    // accumulator pattern'ni tabiiy qiladi (avval blok ichida `=` jim yangi local
    // yaratardi → tashqi nil qolardi).
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

    // Immutability saqlanadi: tashqi `=` (immutable) o'zgaruvchini blok ichidan
    // `=` bilan ham qayta tayinlab bo'lmaydi (aniq xato — jim shadow EMAS).
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

    // fn/lambda CHEGARA: ichidagi `=` tashqi o'zgaruvchini emas, yangi LOCAL
    // yaratadi (shadowing/izolyatsiya). Tashqi qiymat o'zgarmaydi.
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

    // `<-` (assign) esa fn chegarasidan O'TADI — closure capture saqlanadi
    // (`=` chegarada to'xtaydi, `<-` to'xtamaydi: ikkalasining aniq farqi).
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

    // Issue #126: str.trim/replace/starts/ends/pad/repeat — har real loyihada
    // birinchi kunda kerak bo'ladigan str funksiyalari.
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

    // str.repeat manfiy son va str.pad bo'sh to'ldiruvchi — aniq xato (jim
    // noto'g'ri natija emas).
    #[test]
    fn str_repeat_negative_and_pad_empty_fail() {
        assert!(run_source(r#"str.repeat "a" (0 - 1)"#).is_err());
        assert!(run_source(r#"str.pad "a" 3 """#).is_err());
        // Baytlar usize'ga sig'sa ham isize::MAX (allokatsiya chegarasi) dan
        // oshsa — panic emas, Fluxon xatosi (PR #151 review).
        assert!(run_source(r#"str.repeat "aa" 4611686018427387904"#).is_err());
        assert!(run_source(r#"str.pad "x" 4611686018427387904 "🙂""#).is_err());
    }

    #[test]
    fn time_module_fmt_and_roundtrip() {
        // time.fmt unix int bilan deterministik: 1700000000 = 2023-11-14 22:13:20 UTC.
        // time.now/time.ago matn formatini ("YYYY-MM-DD HH:MM:SS") tekshiramiz va
        // fmt orqali round-trip qilamiz.
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
        // time.ago hozirdan oldin: ISO matn format leksikografik = xronologik,
        // shuning uchun DB filtri (`created > $1`) SQL'da to'g'ri ishlaydi. Bu
        // yerda yil/oy/kun bo'laklarini taqqoslab xronologik tartibni isbotlaymiz.
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
        // time.in hozirdan keyin (TTL/expiry uchun). time.ago ning ko'zgusi:
        // ISO matn leksikografik = xronologik, shuning uchun `expires > $now`
        // SQL filtri to'g'ri ishlaydi. Yil/oy/...sek bo'laklarini taqqoslaymiz.
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
        // Issue #65: mijoz ISO `start_at` va `duration_minutes` beradi ->
        // server `end_at` ni hisoblaydi. Booking yadrosining e2e ssenariysi.
        run(r#"
start_at = time.parse "2026-06-10T10:00:00Z"
(start_at == "2026-06-10 10:00:00") | (fail "parse wrong: ${start_at}")
end_at = time.add start_at 30 :min
(end_at == "2026-06-10 10:30:00") | (fail "add wrong: ${end_at}")
mins = (time.diff end_at start_at) / 60
(mins == 30) | (fail "diff wrong: ${mins}")
# buffer-inclusive interval: start - 5min (time.sub — add ning ko'zgusi)
buf_start = time.sub start_at 5 :min
(buf_start == "2026-06-10 09:55:00") | (fail "time.sub wrong: ${buf_start}")
"#);
    }

    #[test]
    fn time_parse_handles_iso_offset() {
        // ISO mintaqali matn UTC ga keltiriladi (+05:00 -> vaqt 5 soat oldin).
        run(r#"
t = time.parse "2026-06-10T15:00:00+05:00"
(t == "2026-06-10 10:00:00") | (fail "mintaqa UTC ga kelmadi: ${t}")
"#);
    }

    #[test]
    fn time_parse_fmt_iana_zone_dst() {
        // Issue #80: IANA zona nomi bilan DST-aware konversiya. "09:00 local"
        // qishda va yozda turli UTC ga tushadi — fiksrlangan offset emas.
        run(r#"
# Qish (EST = UTC-5): 09:00 local -> 14:00 UTC
w = time.parse "2026-01-15 09:00:00" "America/New_York"
(w == "2026-01-15 14:00:00") | (fail "winter DST wrong: ${w}")
# Yoz (EDT = UTC-4): aynan shu wall-clock -> 13:00 UTC
s = time.parse "2026-07-15 09:00:00" "America/New_York"
(s == "2026-07-15 13:00:00") | (fail "summer DST wrong: ${s}")
# Teskari yo'l: UTC instant -> zona wall-clock'i (ko'rsatish uchun)
back = time.fmt s "HH:mm" "America/New_York"
(back == "09:00") | (fail "fmt zone wrong: ${back}")
"#);
    }

    #[test]
    fn keyword_as_field_name() {
        // `.` dan keyin kalit so'z field nomi bo'la oladi (time.in shu tufayli ishlaydi).
        // Map kaliti kalit so'z bo'lsa ham `.in`/`.match` bilan o'qiladi — bu Fluxon
        // falsafasi: member pozitsiyasida kalit so'zning grammatik ma'nosi yo'q.
        run(r#"
m = {in: 1 match: 2 each: 3}
(m.in == 1) | (fail "m.in: ${m.in}")
(m.match == 2) | (fail "m.match: ${m.match}")
(m.each == 3) | (fail "m.each: ${m.each}")
"#);
    }

    #[test]
    fn env_member_access() {
        // env.NOM -> std::env. Yo'q bo'lsa nil -> `??` default. Bor bo'lsa qiymat.
        // FLUXON_TEST_VAR'ni o'rnatib o'qiymiz (DB_TEST_LOCK kerak emas — boshqa env).
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
        // Foydalanuvchi `env` nomli o'zgaruvchi yaratsa, u built-in env'ni ustun
        // bosadi (member access map'ga ishlaydi, std::env'ga emas).
        run(r#"
env = {PORT:"9999"}
p = env.PORT
(p == "9999") | (fail "local env shadow did not work: ${p}")
"#);
    }

    #[test]
    fn json_unicode_roundtrip() {
        // json.dec ko'p baytli UTF-8 (emoji, o'zbekcha) va \u escape (surrogate
        // juftligi) ni TO'G'RI dekodlasin — avval bayt-bayt `as char` mojibake
        // berardi (🙂 -> ð...). Bu yadro tuzatishi http/db/ai hammasiga taalluqli.
        run(r#"
# Xom UTF-8 baytlar (escape'siz): emoji + o'zbekcha — bayt-bayt as char BUZARDI
r = json.dec "{\"s\":\"o'zbek 🙂 g'ayrat\"}"
(r.s == "o'zbek 🙂 g'ayrat") | (fail "raw UTF-8 broke: ${r.s}")
# \u escape: BMP belgisi (ü = ü). \\u -> manbada literal \u bo'ladi.
u = json.dec "{\"c\":\"\\u00fc\"}"
(u.c == "ü") | (fail "\\u00fc dekod broke: ${u.c}")
# \u surrogate juftligi (🙂 = 🙂)
e = json.dec "{\"c\":\"\\ud83d\\ude42\"}"
(e.c == "🙂") | (fail "\\u surrogate evenligi broke: ${e.c}")
# enc -> dec round-trip
back = json.dec (json.enc {x:"hello 🙂 dünyo"})
(back.x == "hello 🙂 dünyo") | (fail "round-trip broke: ${back.x}")
"#);
    }

    #[test]
    fn json_enc_valid_output() {
        // issue #102: control belgilar escape bo'lsin, non-finite float -> null.
        run(r#"
# 1/0 = Infinity -> JSON'da "inf" emas, null bo'lishi kerak
enc = json.enc (1.0 / 0.0)
(enc == "null") | (fail "Infinity was not null: ${enc}")
# tab (control belgi) \t qisqa shaklda escape bo'lib round-trip qilinsin
back = json.dec (json.enc "a\tb")
(back == "a\tb") | (fail "control char round-trip broke: ${back}")
"#);
        // "1 garbage" -> dekoder xato berishi kerak (avval jim 1 qaytarardi)
        assert!(run_source(r#"log (json.dec "1 garbage")"#).is_err());
        // noto'g'ri null-o'xshash matn xato beradi
        assert!(run_source(r#"log (json.dec "nqqq")"#).is_err());
        // boshida '+' bo'lgan son xato beradi
        assert!(run_source(r#"log (json.dec "+5")"#).is_err());
    }

    #[test]
    fn reg_add_call_has_names() {
        // reg battery: funksiyani nom bilan saqlash/chaqirish (dinamik dispatch).
        // closure args map oladi (agent tool naqshi); reg.has bool, reg.names list.
        run(r#"
reg.add "calc" \args -> args.a + args.b
reg.add "greet" \args -> "hello ${args.name}"

out = reg.call "calc" {a:2 b:3}
(out == 5) | (fail "reg.call calc wrong: ${out}")

g = reg.call "greet" {name:"Aziza"}
(g == "hello Aziza") | (fail "reg.call greet wrong: ${g}")

(reg.has "calc") | (fail "reg.has calc should not be false")
((reg.has "none") == false) | (fail "reg.has none should not be true")

# reg.names argumentsiz (Field) — alifbo tartibida barqaror chiqish
ns = reg.names
(ns.len == 2) | (fail "reg.names uzunligi 2 not: ${ns}")
(ns.0 == "calc") | (fail "reg.names[0] calc not: ${ns}")
"#);
    }

    #[test]
    fn reg_call_unknown_fails() {
        // Ro'yxatda yo'q nomni chaqirish fail bo'lishi kerak (jim nil emas).
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
        // Bir nomga qayta reg.add — ustiga yozadi (tool yangilash holati).
        run(r#"
reg.add "f" \args -> 1
reg.add "f" \args -> 2
out = reg.call "f" {}
(out == 2) | (fail "reg.add ustiga yozmadi: ${out}")
"#);
    }

    #[test]
    fn fail_as_expr_and_guard() {
        // fail ifoda kontekstida (guard) — oqimni uzadi, yuqoriga ko'tariladi.
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

    // Ko'p-qatorli pipe: qator `|>` bilan boshlansa, oldingi ifoda davomi
    // (builder zanjiri o'qiluvchanligi, issue #78). Faqat `|>` — `|` (Or) emas.
    #[test]
    fn multiline_pipe_continuation() {
        run(r#"
fn inc x -> x + 1
fn dbl x -> x * 2
# bosqichlar yangi qatorda, leading |>
r = 5
  |> inc
  |> dbl
  |> inc
(r == 13) | (fail "multi-line pipe wrong: ${r}")
# izoh va bo'sh qator orasida ham davom etadi
r2 = 10
  |> inc

  # bu yerda izoh
  |> dbl
(r2 == 22) | (fail "pipe continuation through comment/empty line broke: ${r2}")
"#);
    }

    // Pipe qisman chaqiruv: `x |> f a b` => `f a b x` (lhs OXIRGI argument).
    // Builder/chain naqshini ishlatadi. Argumentsiz funksiya qiymati va
    // argumentsiz modul chaqiruvi (`|> str.up`) eski xulqni saqlaydi.
    #[test]
    fn pipe_partial_application() {
        run(r#"
fn addto base n -> base + n
# argumentli chaqiruv: lhs oxirgi argument bo'lib qo'shiladi
(5 |> addto 100) == 105 | (fail "pipe argumentli chaqiruv did not work")
# zanjir
(3 |> addto 10 |> addto 100) == 113 | (fail "pipe zanjir did not work")
# argumentsiz modul chaqiruvi (eski xulq saqlanishi kerak)
("hello" |> str.up) == "HELLO" | (fail "pipe argumentsiz modul chaqiruvi broke")
# lambda (eski xulq)
(5 |> \n -> n * 2) == 10 | (fail "pipe lambda broke")
"#);
    }

    // --- db battery testlari (in-memory SQLite, har Interp alohida DB) ---

    // DATABASE_URL global env — uni o'rnatib darhol run qilish race bo'lmasligi
    // uchun db testlarini global mutex bilan SERIALIZATSIYA qilamiz. Guard
    // ushlangan paytda boshqa db testi env'ni o'zgartirmaydi. Har test ALOHIDA
    // nomlangan shared-cache memory DB ishlatadi (pool bir nechta connection
    // ochadi → shared-cache kerak; unikal nom → testlar bir-birini ko'rmaydi).
    static DB_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_db_test(name: &str, body: impl FnOnce()) {
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let url = format!("sqlite:file:{name}?mode=memory&cache=shared");
        // SAFETY: guard ushlanган — bir vaqtda faqat bitta db testi env qo'yadi.
        unsafe { std::env::set_var("DATABASE_URL", &url) };
        body();
    }

    #[test]
    fn db_ins_sym_json_roundtrip() {
        // ins generatsiya qilingan id qaytaradi; sym Str<->Sym; json map round-trip.
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
        // param'siz q + $1 placeholder SQLite'da rewrite'siz bog'lanadi + sym param.
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

    // Deklarativ o'qish builder'i (issue #78): db.from |> db.eq/cmp/order/limit
    // |> db.all/first. List qiymat → IN. Xom SQL'siz filtr+range+order+paging.
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

# IN-filtr (list qiymat) + order
in_rows = db.from "bookings" |> db.eq {tenant_id:1 status:[:pending :confirmed]} |> db.order :start_at |> db.all
(in_rows.len == 2) | (fail "IN-filter 2 row expected, ${in_rows.len}")
match in_rows.0.status
  :confirmed -> log "ok IN order"
  _ -> fail "order start_at wrong"

# cmp range + limit
rng = db.from "bookings" |> db.eq {tenant_id:1} |> db.cmp :start_at :ge "2026-06-02" |> db.limit 10 |> db.all
(rng.len == 2) | (fail "cmp >= 2 row expected, ${rng.len}")

# first — bitta yoki nil
one = db.from "bookings" |> db.eq {tenant_id:1 resource_id:7} |> db.first
(one != nil) | (fail "first returned nil")
match one.status
  :pending -> log "ok first"
  _ -> fail "first wrong row"

# first — mos qator yo'q → nil
none = db.from "bookings" |> db.eq {tenant_id:99} |> db.first
(none == nil) | (fail "first with no match expected nil")

# bo'sh IN list → hech narsa
empty = db.from "bookings" |> db.eq {status:[]} |> db.all
(empty.len == 0) | (fail "empty IN 0 row expected")

# nil qiymat → IS NULL ( = NULL hech qachon mos kelmaydi). resource_id null qator.
db.ins "bookings" {tenant_id:1 resource_id:nil status::pending start_at:"2026-06-09"}
nulls = db.from "bookings" |> db.eq {tenant_id:1 resource_id:nil} |> db.all
(nulls.len == 1) | (fail "nil → IS NULL 1 row expected, ${nulls.len}")
"#);
        });
    }

    // Issue #104: db.up bo'sh shart map'i bilan chaqirilsa, build_update ustunsiz
    // "WHERE" quradi (malformed SQL) va butun jadval yangilanardi. db.del'dagi
    // kabi guard endi aniq o'zbekcha xato beradi (SQLite'ning xom "incomplete
    // input" o'rniga).
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

    // Issue #104: db.offset LIMIT'siz avval jim e'tiborsiz qolardi (SQLite OFFSET
    // uchun LIMIT talab qiladi). Endi LIMIT -1 OFFSET m bilan to'g'ri qo'llanadi.
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
# offset 1, limit yo'q → birinchisini o'tkazib, qolgan 2 ta qaytadi.
rows = db.from "t" |> db.order :n |> db.offset 1 |> db.all
(rows.len == 2) | (fail "offset without LIMIT 2 row expected, ${rows.len}")
(rows.0.n == 2) | (fail "offset should skip the first needed, ${rows.0.n}")
"#);
        });
    }

    // Issue #104: manfiy limit/offset SQLite'da kutilmagan xulq beradi (manfiy
    // LIMIT = cheksiz). Endi user sathida aniq rad etiladi.
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

    // Aggregatsiya builder'i: group + count/sum + conditional agg (count_if/sum_if).
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

# conditional agg (overview, group'siz) → bitta qator
ov = db.from "bookings" |> db.eq {tenant_id:1} |> db.count_if {status::confirmed} :confirmed |> db.count_if {status::pending} :pending |> db.sum_if :total_cents {status::done} :revenue |> db.agg_row
(ov.confirmed == 1) | (fail "count_if confirmed 1, ${ov.confirmed}")
(ov.pending == 1) | (fail "count_if pending 1, ${ov.pending}")
(ov.revenue == 5000) | (fail "sum_if revenue 5000, ${ov.revenue}")

# bo'sh tenant: count_if 0 qaytarishi kerak (nil emas — COUNT semantikasi)
empty_ov = db.from "bookings" |> db.eq {tenant_id:99} |> db.count_if {status::done} :done |> db.agg_row
(empty_ov.done == 0) | (fail "empty count_if 0 expected (nil not), ${empty_ov.done}")
"#);
        });
    }

    // str.sym: string→symbol (query-string statuslarini sym filtrga aylantirish).
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

    // --- Issue #82: tbl deklarativ schema migration + index/uniq ---

    // Migration testlari uchun yordamchi: fayl-backed temp DB tayyorlaydi (ikki
    // ALOHIDA Interp = ikki deploy sikli; memory DB birinchi drop'da o'chadi).
    // Yangilangan path qaytaradi; oxirida `cleanup_db` chaqirilsin.
    #[cfg(test)]
    fn setup_db(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        // SAFETY: chaqiruvchi DB_TEST_LOCK ushlaydi.
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
        // tbl'ga yangi ustun qo'shilsa -> ADD COLUMN; eski qatorlar saqlanadi;
        // qayta-deploy idempotent (yiqilmaydi).
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = setup_db("fluxon_mig_addcol.db");

        // Deploy 1: ikki ustunli jadval + bitta qator.
        run_source("use db\ntbl t\n  id serial pk\n  a int\ndb.ins \"t\" {a:1}\n")
            .unwrap_or_else(|e| panic!("deploy1: {}", e));

        // Deploy 2: yangi ustun `b` qo'shilgan. ADD COLUMN bo'lishi, eski qator
        // saqlanib (b NULL) qolishi kerak.
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

        // Deploy 3: aynan o'sha schema — idempotent, yiqilmaydi.
        run_source("use db\ntbl t\n  id serial pk\n  a int\n  b str\n")
            .unwrap_or_else(|e| panic!("deploy3 idempotent: {}", e));

        cleanup_db(&path);
    }

    #[test]
    fn migrate_drop_column_with_backup() {
        // tbl'dan ustun olib tashlansa -> DROP COLUMN + _fluxon_bak_* backup jadval
        // eski ma'lumot bilan qoladi.
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = setup_db("fluxon_mig_dropcol.db");

        run_source(
            "use db\ntbl t\n  id serial pk\n  a int\n  b str\ndb.ins \"t\" {a:1 b:\"keep\"}\n",
        )
        .unwrap_or_else(|e| panic!("deploy1: {}", e));

        // Deploy 2: `b` ustuni olib tashlangan -> DROP COLUMN. `b` so'rovi xato
        // beradi (ustun yo'q), lekin backup jadvalda `b="keep"` saqlanadi.
        run_source(
            r#"
use db
tbl t
  id serial pk
  a  int
# b ustuni endi yo'q -> DROP COLUMN
baks = db.q "select name from sqlite_master where type='table' and name like '_fluxon_bak_t_%'"
(baks.len >= 1) | (fail "backup table should be created needed")
"#,
        )
        .unwrap_or_else(|e| panic!("deploy2 drop column: {}", e));

        // Deploy 3: aynan o'sha (b'siz) schema — `b` allaqachon yo'q, DROP COLUMN
        // yo'q ustunga uriniladi, lekin idempotent: jim pass, yiqilmaydi.
        run_source("use db\ntbl t\n  id serial pk\n  a int\n")
            .unwrap_or_else(|e| panic!("deploy3 drop idempotent: {}", e));

        cleanup_db(&path);
    }

    #[test]
    fn migrate_drop_table_only_fluxon_managed() {
        // tbl source'dan butunlay olib tashlansa -> DROP TABLE + backup, lekin
        // FAQAT Fluxon yaratgan jadval (_fluxon_schema'da). Fluxon yaratmagan jadval saqlanadi.
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = setup_db("fluxon_mig_droptbl.db");

        // Deploy 1: Fluxon `a` jadvalini yaratadi + qo'lda Fluxon-bo'lmagan `manual` jadval.
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

        // Deploy 2: `a` tbl olib tashlangan (lekin boshqa tbl bor — registry bo'sh
        // EMAS). `a` DROP bo'lishi, `manual` saqlanishi kerak.
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
        // Index e'loni -> CREATE INDEX; olib tashlansa -> DROP INDEX. uniq(a b) ->
        // duplicate insert xato. sqlite_autoindex_* tegilmaydi.
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = setup_db("fluxon_mig_index.db");

        // Deploy 1: single index + multi unique.
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

        // uniq buzilishi: bir xil (resource_id start_at) -> xato.
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

        // Deploy 2: status index olib tashlangan -> DROP INDEX. uniq qoladi.
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
        // REGRESSIYA (code review): index'lanган ustun olib tashlanganda eskirgan
        // index ustun DROP'idan OLDIN tashlanishi kerak — aks holda ba'zi SQLite
        // holatlarida DROP COLUMN "error in index ... no such column" bilan rad
        // etiladi va deploy migrate qila olmaydi. Single va kompozit index ikkalasi.
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = setup_db("fluxon_mig_dropidxcol.db");

        // Deploy 1: index'li `status` ustun + kompozit index(a status).
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

        // Deploy 2: `status` ustuni olib tashlangan. Eski idx_t_status va
        // idx_t_a_status hali DB'da — migration yiqilmasligi (eskirgan index avval
        // tashlanadi), keyin DROP COLUMN ishlashi kerak.
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
# status ustuni haqiqatan yo'qolgan
cols = db.q "select name from pragma_table_info('t') where name='status'"
(cols.len == 0) | (fail "status columni DROP should be")
"#,
        )
        .unwrap_or_else(|e| panic!("deploy2 drop indexed column: {}", e));

        cleanup_db(&path);
    }

    #[test]
    fn migrate_pipe_modifier_creates_unique_index() {
        // `email str index|uniq` -> bitta UNIQUE index yaratiladi (uniq subsume
        // qiladi), duplicate insert xato beradi.
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
        // Issue #94: `uniq(a, b)` (vergulli) ko'p-ustunli UNIQUE cheklov yaratadi —
        // soxta "uniq" ustun EMAS. Dublikat (a,b) juftligi xato beradi.
        with_db_test("multi_uniq", || {
            // 1. Soxta `uniq` ustun yo'qligi: jadvalda faqat a, b bo'lishi kerak.
            run(r#"
use db
tbl t
  a str
  b str
  uniq(a, b)
n = (db.q "select count(*) c from pragma_table_info('t')").0.c
(n == 2) | (fail "tableda faqat 2 column (a, b) should be — soxta uniq none")
ui = db.q "select name from sqlite_master where type='index' and name='uniq_t_a_b'"
(ui.len == 1) | (fail "uniq_t_a_b unique index should be created needed")
db.ins "t" {a:"x" b:"y"}
"#);

            // 2. Dublikat (a, b) juftligi UNIQUE cheklovni buzadi. Ikkala insert
            //    bir manbada — shared-memory db run'lar orasida yo'qolmasin.
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
        // Issue #94 (bog'liq): `ref:tbl.col` FK modifikatori endi enforce qilinadi —
        // mavjud bo'lmagan ota qatorga ishora qilgan insert xato beradi.
        with_db_test("fk_ref", || {
            // Yaroqli FK: ota qator mavjud — insert o'tadi.
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

            // Yetim FK: owner=999 mavjud emas -> FOREIGN KEY constraint failed.
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
        // Issue #94 (codex revyu): FK faqat YANGI jadvalga emas — MAVJUD jadvaldagi
        // mavjud ustunga ham qo'llanishi kerak. Eski holatni (DB introspeksiyasi)
        // declaration bilan solishtirib, farqda jadval rebuild qilinadi. Ma'lumot
        // saqlanadi, autoincrement davom etadi, FK enforce qilinadi.
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = setup_db("fluxon_fk_rebuild.db");

        // Deploy 1: posts FK'siz, ma'lumot bilan.
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

        // Deploy 2: mavjud `owner` ustuniga ref:users.id qo'shildi -> rebuild.
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

        // Endi yetim insert rad etiladi (FK enforce).
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
        // Codex revyu: bitta migration ham ustun DROP qilsa, ham mavjud ustunga
        // ref qo'shsa — DROP COLUMN backup'i (`_fluxon_bak_<t>_<ts>`) bilan rebuild
        // backup'i NOM TO'QNASHMASLIGI kerak (rebuild `_fk` suffiks ishlatadi).
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = setup_db("fluxon_drop_and_fk.db");

        // Deploy 1: `old` ustuni bor, ref yo'q.
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

        // Deploy 2: `old` DROP + `owner` ga ref qo'shish (bitta migration).
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
        // Mavjud ma'lumotda yetim qator bo'lsa, FK qo'shish rebuild'i JIM yo'qotmaydi
        // — aniq xato beradi va ROLLBACK orqali ma'lumot butun qoladi.
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

        // ref qo'shish -> yetim qator FK ni buzadi -> migrate xato (rebuild abort).
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

        // Ma'lumot va eski (FK'siz) sxema saqlangan bo'lishi kerak.
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
        // tx ichida fail -> butun blok rollback; xato yuqoriga ko'tariladi va
        // birinchi (tx'siz) ins saqlanib, tx ichidagi ins rollback bo'ladi.
        // FAYL-backed temp DB: ikki run_source orasida saqlanadi (memory DB esa
        // birinchi Interp drop bo'lganda o'chadi). Tekshiruvchi run ALOHIDA Interp.
        let path = std::env::temp_dir().join("fluxon_tx_rollback_test.db");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guard ushlangan.
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

        // Alohida (yangi) Interp/pool — fayl DB saqlangan. Rollback ishlagan bo'lsa
        // faqat tx'siz ins (n:1) qoladi, tx ichidagi (n:2) yo'q.
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
        // Issue #63: json ustun `tbl` e'lon QILINMAGAN process'da ham map qaytarsin.
        // Ikki ALOHIDA Interp (= ikki process) bir FAYL DB ustida: birinchi yozadi
        // (tbl bilan), ikkinchi tbl'siz o'qiydi — DB introspeksiyasi ustun json
        // ekanini tiklab map beradi (ilgari xom string qaytib row.body.x xato berardi).
        let path = std::env::temp_dir().join("fluxon_json_xproc_test.db");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guard ushlangan.
        unsafe {
            std::env::set_var("DATABASE_URL", format!("sqlite:{}", path.display()));
        }

        // Yozuvchi process: tbl e'lon qiladi + json map (ichida list ham) yozadi.
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

        // O'qiydigan process: tbl YO'Q — faqat o'qiydi. json map bo'lib kelishi shart.
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
        // Regression: tbl e'lon QILINMAGAN process TEXT ustuniga map/list yoza olsin.
        // Ilgari DB introspeksiyasi TEXT ustunni Some("text") qaytarardi va write path
        // "json ustun emas" xatosi berardi — endi yozish tomoni faqat tbl registry'dan
        // foydalanadi, shuning uchun tbl yo'q process uchun schema-less yozish ishlaydi.
        //
        // Scenario: birinchi process `str` (TEXT) ustun yaratadi; ikkinchi process
        // tbl YO'Q holda map yozadi — bu ilgari "json ustun emas" xatosi berardi.
        let path = std::env::temp_dir().join("fluxon_schemaless_write_test.db");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::set_var("DATABASE_URL", format!("sqlite:{}", path.display()));
        }

        // Birinchi process: str (TEXT) ustun bilan jadval yaratadi va bir qator yozadi
        // (db.ins lazy DB open + migrate qiladi — jadval aynan shu yerda yaratiladi).
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

        // Ikkinchi process: tbl YO'Q — TEXT ustuniga map yozishi kerak (schema-less).
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
        // Ichki tx (SAVEPOINT). Ichki blok ret qiymat qaytaradi, tashqi commit.
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
        // uniq buzilishi tx ichida -> rollback (idempotency naqshi).
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
            // uniq buzilishi db xato sifatida ko'tariladi.
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
        // Tirnoqsiz 5-maydon (nomli funksiya). cron.on bloklamaydi, dastur tugaydi.
        run(r#"
fn check
  log "check"
cron.on 0 * * * * check
"#);
    }

    #[test]
    fn cron_on_lambda_va_murakkab_ifoda() {
        // Inline lambda + step/range/list aralash ifoda.
        run(r#"
cron.on */15 9 1,15 * 1-5 \->
  log "har 15 daqiqa, 9-soat, 1 va 15-kun, ish kunlari"
"#);
    }

    #[test]
    fn cron_on_tirnoqli_variant() {
        // Tirnoqli str ham ishlaydi (inson uchun; AI docs'da yo'q).
        run(r#"
fn report
  log "report"
cron.on "30 9 * * *" report
"#);
    }

    #[test]
    fn cron_on_notogri_ifoda_xato() {
        // 99-daqiqa yo'q — cron.on xato qaytarishi kerak.
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
        // queue.on handler ro'yxatga oladi, queue.push ish qo'shadi — ikkalasi ham
        // bloklamaydi, dastur tugaydi (worker fonda ishlayveradi). Handler bittagina
        // `job` map argumenti oladi.
        run(r#"
queue.on "send" \job ->
  log "sending: ${job.ph}"
queue.push "send" {ph:"+99890" body:"hello"}
"#);
    }

    #[test]
    fn queue_push_payloadsiz() {
        // Payload ixtiyoriy — berilmasa job Nil bo'ladi.
        run(r#"
queue.on "tozala" \job ->
  log "cleaned"
queue.push "tozala"
"#);
    }

    #[test]
    fn queue_handlersiz_push_dastur_tugaydi() {
        // Issue #105: handler'i hech qachon ro'yxatga olinmagan ish dastur
        // chiqishini bloklamasligi kerak — run() ogohlantirish bilan normal
        // tugaydi (eski busy-loop'da ish abadiy aylanardi).
        run(r#"queue.push "orphan" {x:1}"#);
    }

    #[test]
    fn queue_drain_handler_haqiqatan_ishlaydi() {
        // Issue #105: run() qaytishidan oldin navbat drain bo'ladi — handler
        // haqiqatan ishlagani DB orqali RACE'siz tekshiriladi (ilgari worker
        // fon thread'i tugashini kafolatlab bo'lmasdi).
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

        // Birinchi run() drain bilan tugadi — ikkala ish DB'da bo'lishi SHART.
        run(r#"
use db
((db.q "select * from jobs").len == 2) | (fail "queue jobs were not executed")
"#);

        cleanup_db(&path);
    }

    #[test]
    fn queue_push_nom_str_bolmasa_xato() {
        // 1-argument ish nomi str bo'lishi shart.
        let err = run_source(r#"queue.push 5"#).expect_err("a non-str name should error");
        assert!(
            err.contains("queue.push"),
            "expected queue.push error, got: {}",
            err
        );
    }

    #[test]
    fn queue_argumentsiz_dispatch_ga_yetadi() {
        // Argumentsiz `queue.X` (Call emas, Field bo'lib keladi) modul dispatch'iga
        // yetishi kerak — `queue` ident o'zgaruvchi deb qidirilib "noma'lum nom"
        // bermasin. Noma'lum funksiya bilan sinaymiz: dispatch'ga yetsa "queue
        // modulida ... yo'q" xatosi keladi (noma'lum nom EMAS). [cron.run regressiyasi]
        let err = run_source(r#"queue.yoq"#).expect_err("argument-less queue.yoq should error");
        assert!(
            err.contains("queue module") && !err.contains("unknown name"),
            "argument-less queue should reach dispatch, got: {}",
            err
        );
    }

    #[test]
    fn cron_argumentsiz_dispatch_ga_yetadi() {
        // `cron.run` argumentsiz — Field bo'lib keladi va dispatch'ga yetishi kerak
        // (aks holda "noma'lum nom: cron"). cron.run bloklaydi, shuning uchun mavjud
        // funksiya o'rniga noma'lum funksiya bilan dispatch'ga yetganini tekshiramiz.
        let err = run_source(r#"cron.yoq"#).expect_err("argument-less cron.yoq should error");
        assert!(
            err.contains("cron module") && !err.contains("unknown name"),
            "argument-less cron should reach dispatch, got: {}",
            err
        );
    }

    #[test]
    fn queue_on_handler_fn_bolmasa_xato() {
        // 2-argument handler fn bo'lishi shart.
        let err = run_source(r#"queue.on "send" 5"#).expect_err("a non-fn handler should error");
        assert!(
            err.contains("queue.on"),
            "expected queue.on error, got: {}",
            err
        );
    }

    // `ai` testlari env'ga (kalitlarga) bog'liq — global mutex bilan serializatsiya
    // qilamiz (boshqa testlar parallel env'ni o'zgartirmasin). Bu testlar
    // TARMOQQA CHIQMAYDI: kalit yo'qligida API chaqiruvidan OLDIN xato berilishini
    // tekshiramiz.
    static AI_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn ai_kalit_yoq_bolsa_aniq_xato() {
        let _guard = AI_ENV_LOCK.lock().unwrap();
        // Hamma kalit env'larini vaqtincha o'chiramiz (auto-detect hech qaysisini
        // topmasligi kerak). runtime/ da .env yo'q -> aniq "kaliti topilmadi" xatosi,
        // tarmoqqa chiqilmaydi. Oldingi qiymatlarni saqlab, testdan keyin tiklaymiz.
        let saved: Vec<(&str, Option<String>)> = ["AI_KEY", "ANTHROPIC_API_KEY", "OPENAI_API_KEY"]
            .iter()
            .map(|k| (*k, std::env::var(k).ok()))
            .collect();
        for (k, _) in &saved {
            unsafe { std::env::remove_var(k) };
        }
        let err = run_source(r#"x = ai.ask "hello""#).expect_err("a missing key should error");
        // env'ni tiklaymiz (boshqa testlarga ta'sir qilmasin).
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
        // ai.foo -> dispatch'ga yetib "ai.foo yo'q" beradi (noma'lum nom EMAS).
        // Kalit bo'lsa ham bo'lmasa ham bu funksiya nomini tekshirishdan oldin keladi.
        let err = run_source(r#"ai.foo "x""#).expect_err("an unknown ai function should error");
        assert!(
            err.contains("ai.foo") && !err.contains("unknown name"),
            "ai should reach dispatch and give a function error, got: {}",
            err
        );
    }

    #[test]
    fn ai_ozgaruvchi_modulni_yopadi() {
        // `ai` o'zgaruvchi sifatida e'lon qilinsa, u modul emas — oddiy map maydoni
        // sifatida o'qiladi (http/db kabi emas, lekin ai dispatch lookup tekshiradi).
        run(r#"
ai = {ask:"shadowed"}
log "ai.ask = ${ai.ask}"
"#);
    }

    // sh.run -> {stdout stderr code}: echo natijasi va muvaffaqiyat kodi to'g'ri.
    // (Unix-mos echo, CI ubuntu+macOS da ishlaydi.)
    #[test]
    fn sh_run_echo_natija_va_kod() {
        run(r#"
r = sh.run "printf hello"
(r.code == 0) | (fail "code should be 0: ${r.code}")
(r.stdout == "hello") | (fail "stdout wrong: ${r.stdout}")
(r.stderr == "") | (fail "stderr empty should be: ${r.stderr}")
"#);
    }

    // Non-zero exit -> Flow::err EMAS, `code` orqali tekshiriladi (kutilgan natija).
    #[test]
    fn sh_run_nolik_bolmagan_kod_xato_emas() {
        run(r#"
r = sh.run "exit 7"
(r.code == 7) | (fail "code 7 should be: ${r.code}")
"#);
    }

    // --- `use ./fayl` foydalanuvchi modullari (issue #45) ---

    use std::sync::atomic::{AtomicU64, Ordering};

    // Unikal vaqtinchalik katalog — parallel testlar to'qnashmasligi uchun
    // (process id + atomik hisoblagich). Test fayllari shu yerga yoziladi.
    fn temp_module_dir() -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("fluxon_mod_test_{}_{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // `files` ([(nom, manba), ...]) ni `dir`ga yozadi, birinchisini run qiladi,
    // natijani qaytaradi. Tugagach katalogni tozalaydi.
    fn run_modules(files: &[(&str, &str)]) -> Result<(), String> {
        let dir = temp_module_dir();
        for (name, src) in files {
            // Fayl nomi subkatalogli bo'lishi mumkin ("sub/test.fx") — `../`
            // (yuqori papka) modul yo'llarini sinash uchun papka ierarxiyasi kerak.
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

    // Asosiy holat (issue #45 reproduction): `exp` qilingan qiymat va funksiya
    // `modul.nom` ostida ko'rinadi; modul funksiyasi modul-darajadagi `exp`ga
    // (closure) kira oladi.
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

    // `as alias` — bog'lash nomi alias bo'ladi (batareya nomi bilan to'qnashmaslik).
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

    // Modul-private nomlar (oddiy `=`/`fn`) namespace'ga KIRMAYDI — faqat `exp`.
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

    // Nested import (main -> a -> b): modul boshqa modulni import qila oladi,
    // yo'l import qiluvchi modulning katalogiga nisbatan hal qilinadi.
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

    // `../` (yuqori papka) modul yo'li (issue #47): subkatalogdagi fayl
    // ota-katalogdagi modulni import qila oladi. parse_use `Tok::DotDot`'ni
    // tan olishi va runtime yo'lni `..` bilan hal qila olishi shu yerda sinaladi.
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

    // Cache: bir modul ikki marta `use` qilinsa bir marta bajariladi (idempotent).
    // Modul top-level `<-` hisoblagichni oshiradi; ikki import'da ham 1 bo'lib qoladi.
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
            // `exp n` bir martagina hisoblanadi — cache bo'lsa shunday.
            ("c.fx", "exp n = 1\n"),
        ])
        .unwrap();
    }

    // Sikllik import (x -> y -> x) aniq xato beradi (cheksiz rekursiya emas).
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

    // Mavjud bo'lmagan modul — aniq "topilmadi" xatosi.
    #[test]
    fn use_module_topilmadi_xato() {
        let err = run_modules(&[("main.fx", "use ./yoq\n")]).unwrap_err();
        assert!(
            err.contains("module not found"),
            "not-found error expected, got: {}",
            err
        );
    }

    // `.fx` kengaytmasi avtomatik qo'shiladi: `use ./greet` -> `greet.fx`.
    // (Yuqoridagi testlar ham shunga tayanadi; bu aniq tekshiruv.)
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

    // Batareya `use` (`use http`) hamon no-op — fayl yuklanmaydi, dispatch ishlaydi.
    #[test]
    fn use_batareya_hamon_no_op() {
        // `use math` fayl izlamaydi (xato bermaydi), math.* dispatch ishlaydi.
        run(r#"
use math
(math.floor 3.7 == 3) | (fail "floor wrong")
"#);
    }

    // Issue #128: math.min/max/pow/sqrt — .fx yuzasidan tekshiruv.
    #[test]
    fn math_min_max_pow_sqrt() {
        run(r#"
(math.min 3 7 == 3) | (fail "min wrong")
(math.max 3 7 == 7) | (fail "max wrong")
(math.min 3 2.5 == 2.5) | (fail "aralash min wrong")
(math.pow 2 10 == 1024) | (fail "pow wrong")
(math.sqrt 9 == 3.0) | (fail "sqrt wrong")
"#);
    }

    // `each i in inf` — cheksiz loop. `stop` chiqaradi, `i` 0 dan ortadi.
    // REPL/event-loop uchun (issue #27): oldin model 1..1000 hiylasiga murojaat
    // qilardi; endi tabiiy cheksiz takror bor.
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

    // `skip` cheksiz loop'da keyingi iteratsiyaga o'tadi (i baribir ortadi).
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

    // inf qiymat sifatida ishlatib bo'lmaydi — faqat `each i in inf` da.
    #[test]
    fn inf_qiymat_sifatida_xato() {
        let err = run_source("x = inf\n").expect_err("inf as a value should error");
        assert!(err.contains("inf"), "unexpected error: {}", err);
    }

    // `each k, v in inf` — ikki o'zgaruvchi ma'nosiz (cheksiz oddiy hisoblagich).
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

    // --- `fluxon check` (faqat parse, issue #55) ---

    // To'g'ri kod -> check muvaffaqiyatli (Ok).
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

    // Parse/lex xato -> check Err qaytaradi (main bu Err'ni exit 2 ga aylantiradi).
    #[test]
    fn check_parse_xato_err() {
        let err = check_source("fn g x\n  ret (\n").expect_err("a parse error should return Err");
        assert!(!err.is_empty(), "error text should not be empty");
    }

    // ENG MUHIM: check kodni BAJARMAYDI — runtime side-effect/xato bo'lmaydi.
    // Quyidagi kod runtime'da fail qiladi (noma'lum nom), lekin sintaksis to'g'ri,
    // shuning uchun check Ok beradi. Bu check'ning interp'ni o't kazib yuborishini
    // isbotlaydi (Forge eval-gate QATLAM 1: bajarish XAVFLI).
    #[test]
    fn check_kodni_bajarmaydi() {
        // `nomalum_funksiya` runtime'da "noma'lum nom" beradi, lekin sintaksis joyida.
        check_source("x = nomalum_funksiya 5\n")
            .expect("syntactically valid code should pass check (not executed)");
        // Tasdiq: xuddi shu kod run'da xato beradi (bajariladi).
        assert!(
            run_source("x = nomalum_funksiya 5\n").is_err(),
            "run should execute this code and error (unlike check)"
        );
    }

    // parse_args: `check` buyrug'ini tanib, faylni Command::Check ga joylaydi.
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

    // parse_args: `test` yo'lsiz (standart tests/) va yo'l bilan ishlaydi.
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

    // parse_args: version flag build qilingan package versiyasini chiqaradigan
    // buyruqqa map bo'ladi.
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

    // parse_args: help flaglari foydalanish matnini chiqaradigan buyruqqa map bo'ladi.
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

    // issue #136: assert primitivi — truthy shart jim o'tadi, falsy shart
    // xabar bilan runtime xato beradi (fayl FAIL bo'ladi).
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
        // nil ham falsy
        assert!(run_source("assert nil").is_err());
    }

    // issue #136: `fluxon test` fayl topish — katalogdan .fx'lar rekursiv,
    // tartiblangan; bitta fayl o'z holicha; yo'q yo'l/bo'sh katalog -> xato.
    #[test]
    fn test_fayllarini_topish() {
        let dir = std::env::temp_dir().join(format!("fluxon_test_disc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // oldingi muvaffaqiyatsiz run qoldig'i
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

        // bitta fayl — ro'yxat faqat o'sha fayldan iborat
        let one = collect_test_files(&dir.join("a.fx")).unwrap();
        assert_eq!(one.len(), 1);

        // .fx bo'lmagan aniq fayl — discovery xatosi (Fluxon sifatida bajarilmaydi)
        let err = collect_test_files(&dir.join("eslatma.txt")).unwrap_err();
        assert!(err.contains("is not a .fx file"), "message: {}", err);

        // mavjud bo'lmagan yo'l — xato
        assert!(collect_test_files(&dir.join("yoq")).is_err());

        // .fx'siz katalog — xato (jim "0 fayl o'tdi" chalg'ituvchi bo'lardi)
        let empty = dir.join("bosh");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(collect_test_files(&empty).is_err());

        // halqali symlink (katalog o'ziga ishora) cheksiz rekursiya bermasin —
        // file_type() symlink'ni kuzatmaydi, halqa shunchaki o'tkazib yuboriladi.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&dir, dir.join("halqa")).unwrap();
            let with_loop = collect_test_files(&dir).unwrap();
            assert_eq!(with_loop.len(), 3, "a loop should not change the file list");
        }

        // o'qib bo'lmaydigan ichki katalog jim o'tkazilmasin — xato ko'tarilsin
        // (codex P2). root ruxsat cheklovini chetlab o'tadi, shuning uchun faqat
        // cheklov haqiqatan ishlagan muhitda tekshiramiz (CI runner non-root).
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
            // cleanup uchun ruxsatni qaytaramiz
            std::fs::set_permissions(&yopiq, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    // issue #136: yiqilgan fayl keyingilarni to'xtatmaydi — har fayl alohida
    // hisoblanadi va yakunda (PASS, FAIL) soni to'g'ri chiqadi.
    #[test]
    fn test_runner_fail_keyingisini_toxtatmaydi() {
        let dir = std::env::temp_dir().join(format!("fluxon_test_run_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // oldingi muvaffaqiyatsiz run qoldig'i
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("01_yiqiladi.fx"), r#"assert false "on purpose""#).unwrap();
        std::fs::write(dir.join("02_otadi.fx"), "assert (2 > 1)").unwrap();

        let files = collect_test_files(&dir).unwrap();
        let (passed, failed) = run_test_files(&files);
        assert_eq!((passed, failed), (1, 1));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    // issue #57: symbol MATNGA aylanganda `:` prefiks tashlanadi
    // (interpolatsiya, str.str, `+` birlashtirish). Symbol literal sintaksisi
    // (`:florist`) o'zgarmaydi — faqat matn ko'rinishi `:` siz.
    #[test]
    fn sym_to_text_colon_tashlanadi() {
        run(r#"
s = :florist
# interpolatsiya
(("v/${s}") == "v/florist") | (fail "interpolation: ${"v/${s}"}")
# str.str
((str.str s) == "florist") | (fail "str.str: ${str.str s}")
# `+` birlashtirish (ikkala tomon)
(("p/" + s) == "p/florist") | (fail "left + : ${"p/" + s}")
((s + "/q") == "florist/q") | (fail "right + : ${s + "/q"}")
# symbol literal va taqqoslash O'ZGARMAYDI
(s == :florist) | (fail "symbol comparison broke")
"#);
    }

    // Symbol list/map ICHIDA `:` prefiksini SAQLAYDI — u yerda symbol
    // string'dan ajralib turishi kerak (repr matn ko'rinishidan farq qiladi).
    #[test]
    fn sym_repr_listda_colon_saqlaydi() {
        run(r#"
xs = [:a "b"]
((str.str xs) == "[:a \"b\"]") | (fail "list repr: ${str.str xs}")
"#);
    }

    // --- auth battery (issue #69) ---
    //
    // $AUTH_SECRET env'ga muhtoj testlar uchun lock — parallel testlar env'ga
    // race qilmasin (AI_ENV_LOCK naqshi).
    static AUTH_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn auth_jwt_verify_roundtrip() {
        let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("AUTH_SECRET", "sirli-kalit-123") };
        run(r#"
use auth
token = auth.jwt {sub:"u1" tenant:"t1" role:"admin"}
# imzolangan JWT — 3 segment (header.payload.imzo)
parts = str.split token "."
(parts.len == 3) | (fail "JWT 3 segment not: ${parts.len}")
# verify -> payload map qaytaradi, da'volar saqlanadi
claims = auth.verify token
(claims.sub == "u1") | (fail "sub wrong: ${claims.sub}")
(claims.tenant == "t1") | (fail "tenant wrong: ${claims.tenant}")
(claims.role == "admin") | (fail "role wrong: ${claims.role}")
# iat/exp avtomatik qo'shilgan
(claims.exp > claims.iat) | (fail "exp should be greater than iat")
"#);
    }

    #[test]
    fn auth_verify_buzilgan_token_xato() {
        let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("AUTH_SECRET", "sirli-kalit-123") };
        // Imzo buzilgan token -> auth.verify err (Fluxon'da `try` o'tkazgich, xato
        // run'ni to'xtatadi — shuning uchun Rust tomonda expect_err bilan
        // tekshiramiz). token'ga belgi qo'shsak imzo mos kelmaydi.
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
        // 3 segmentdan kam — JWT shakli noto'g'ri -> err.
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
        // `exp:nil` payload -> auth.jwt `or_insert` nil'ni override qilmaydi,
        // ya'ni token sonli `exp`siz imzolanadi. To'g'ri imzolangan bo'lsa ham,
        // auth.verify uni RAD ETISHI kerak (aks holda abadiy amal qilardi —
        // Codex P2). Kalit to'g'ri, shuning uchun bu imzo emas, exp xatosi.
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
        // hash/check env'ga muhtoj emas (lock kerak emas).
        run(r#"
use auth
h = auth.hash "user-parol"
# argon2id PHC string
(str.has h "argon2id") | (fail "argon2id hash not: ${h}")
# to'g'ri parol -> true
(auth.check "user-parol" h) | (fail "check returned false for correct password")
# noto'g'ri parol -> false
((auth.check "wrong-password" h) == false) | (fail "check returned true for wrong password")
"#);
    }

    #[test]
    fn auth_noma_lum_funksiya_xato() {
        // auth.foo -> dispatch'ga yetib "auth.foo yo'q" beradi (noma'lum nom EMAS).
        let err = run_source(r#"auth.foo "x""#).expect_err("an unknown auth function should error");
        assert!(
            err.contains("auth.foo") && !err.contains("unknown name"),
            "auth should reach dispatch and give a function error, got: {}",
            err
        );
    }

    #[test]
    fn auth_ozgaruvchi_modulni_yopadi() {
        // `auth` o'zgaruvchi sifatida e'lon qilinsa, u modul emas — oddiy map.
        run(r#"
auth = {jwt:"shadowed"}
log "auth.jwt = ${auth.jwt}"
"#);
    }

    // Issue #106: string interpolatsiya ichidagi parse xatosi asl qatorni
    // ko'rsatishi kerak ("1-qatorda" ga yiqilib qolmasligi) va "interpolatsiya
    // ichida:" prefiksi bilan kelishi kerak.
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

    // Issue #106: lex xatosi ham asl qatorni saqlaydi. Ko'p qatorli ifoda
    // ham qator hisobini buzmaydi — inner string 3-qatorda ochiladi.
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

    // Issue #106: ${...} chegarasi ichki string literallarni hisobga oladi —
    // string ichidagi `}` interpolatsiyani erta yopmaydi.
    #[test]
    fn interp_ichki_string_qavsni_yopmaydi() {
        run(r#"
x = "v: ${"inner } brace"}"
(x == "v: inner } brace") | (fail "inner string brace wrong ishlandi: ${x}")
"#);
    }

    // Issue #106: ichki string ichidagi escape qilingan tirnoq (\") string'ni
    // yopmaydi, undan keyingi `}` ham interpolatsiyani yopmaydi.
    #[test]
    fn interp_ichki_string_escape_tirnoq() {
        run(r#"
x = "x=${"a\"}b"}"
(x == "x=a\"}b") | (fail "escaped quote wrong ishlandi: ${x}")
"#);
    }

    // Issue #130: """ blok satr — umumiy chekinish kesiladi, yopuvchi """
    // o'z qatorida bo'lsa oxirida \n qolmaydi.
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

    // Issue #130: blok satr ichida ${expr} va $ident interpolatsiya ishlaydi.
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

    // Issue #130: bo'sh qator \n bo'ladi, `"` va `""` escape'siz erkin —
    // JSON/HTML parchalari to'g'ridan-to'g'ri yoziladi.
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

    // Issue #130: minimal chekinishdan chuqurroq qatorlar nisbiy joyini saqlaydi
    // (SQL/prompt ichki strukturasi buzilmasin).
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

    // Issue #130: yopuvchi """ kontent qatorining oxirida ham kelishi mumkin.
    #[test]
    fn blok_satr_kontent_qatorida_yopilish() {
        run(r#"
s = """
  one line"""
(s == "one line") | (fail "closing on a content line error: ${s}")
"#);
    }

    // Issue #130: blok satr indentatsiyali blok (fn tanasi) ichida ham ishlaydi —
    // satr ichidagi qatorlar INDENT/DEDENT chiqarmaydi.
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

    // Issue #130: uchta ketma-ket tirnoq kerak bo'lsa \""" yoziladi.
    #[test]
    fn blok_satr_escape_uchta_tirnoq() {
        run(r#"
s = """
  three: \"""
  """
(s == "three: \"\"\"") | (fail "escape quote error: ${s}")
"#);
    }

    // Issue #130: ochuvchi """ dan keyin shu qatorda matn — aniq xato
    // (kanonik bitta yo'l: kontent yangi qatordan).
    #[test]
    fn blok_satr_ochilishda_matn_xato() {
        let err = run_source("s = \"\"\"matn\nx\"\"\"\n")
            .expect_err("text on the opening line should error");
        assert!(err.contains("a new line"), "unexpected error: {}", err);
    }

    // Issue #130: yopilmagan blok satr aniq xato beradi (ochilgan qator bilan).
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

    // Issue #131: crypto battery Fluxon kodidan ochiq — argumentli chaqiruv
    // (Call) ham, argument'siz `crypto.uuid` (Field) ham ishlaydi.
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

    // Issue #131: crypto.b64d noto'g'ri kirishda aniq xato (panic emas).
    #[test]
    fn crypto_b64d_xato_beradi() {
        let err = run_source("crypto.b64d \"this is not base64!!!\"")
            .expect_err("invalid base64 should error");
        assert!(err.contains("base64"), "unexpected error: {}", err);
    }

    // Issue #131 (review): foydalanuvchi `crypto` nomini e'lon qilgan bo'lsa
    // (masalan `use ./crypto` moduli), battery emas — uniki ustun. auth/ai
    // bilan bir xil shadowing xatti-harakati, Call ham Field yo'li ham.
    #[test]
    fn crypto_lokal_nom_battery_dan_ustun() {
        run(r#"
crypto = {sha256: \s -> "meniki ${s}" uuid: 7}
((crypto.sha256 "x") == "meniki x") | (fail "lokal crypto.sha256 column did not happen")
((crypto.uuid) == 7) | (fail "lokal crypto.uuid column did not happen")
"#);
    }

    // Issue #132: bytes turi asoslari — of/str/len/slice, tenglik, Display.
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

    // Issue #132: bytes.len BAYT o'lchaydi, str.len BELGI — diakritikali matnda
    // farq ko'rinadi (’ U+2019 = 3 bayt, 1 belgi).
    #[test]
    fn bytes_len_bayt_str_len_belgi() {
        run(r#"
s = "o’zbek"
((str.len s) == 6) | (fail "str.len belgi sanashi needed")
((bytes.len (bytes.of s)) == 8) | (fail "bytes.len bayt sanashi needed")
"#);
    }

    // Issue #132: crypto bilan integratsiya — b64db ikkilik dekodlash, bytes
    // kirishlar str bilan bir xil natija.
    #[test]
    fn bytes_crypto_integratsiya() {
        run(r#"
data = crypto.b64db "AP/+iA=="
((bytes.len data) == 4) | (fail "b64db uzunlik broke")
((crypto.b64 data) == "AP/+iA==") | (fail "bytes b64 aylanasi broke")
((crypto.sha256 (bytes.of "abc")) == (crypto.sha256 "abc")) | (fail "sha256 bytes/str farq qildi")
"#);
    }

    // Issue #132: bytes.str UTF-8 bo'lmagan baytlarda aniq xato (jim buzilmaydi).
    #[test]
    fn bytes_str_yaroqsiz_utf8_xato() {
        let err = run_source("bytes.str (crypto.b64db \"//4=\")")
            .expect_err("invalid UTF-8 should error");
        assert!(err.contains("UTF-8"), "unexpected error: {}", err);
    }

    // Issue #132: fs bilan ikkilik aylana — bytes yoziladi, fs.readb aynan
    // o'sha baytlarni qaytaradi (rasm/PDF stsenariysi).
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

    // Issue #132: json.enc bytes'ni base64 matn sifatida kodlaydi (yo'qotishsiz).
    #[test]
    fn bytes_json_enc_base64() {
        run(r#"
b = crypto.b64db "AP/+iA=="
((json.enc {fayl:b}) == "{\"fayl\":\"AP/+iA==\"}") | (fail "json.enc bytes broke")
"#);
    }

    // Issue #138: REPL bitta blokni bajarib oxirgi ifoda QIYMATINI qaytaradi
    // (chop etish uchun). `run` () qaytaradi — bu farq REPL natijasini ko'rsatishga
    // imkon beradi. lex_parse + run_repl_chunk REPL'da aynan shu yo'l bilan ishlaydi.
    fn repl_chunk(interp: &interp::Interp, src: &str) -> Result<value::Value, String> {
        interp.run_repl_chunk(&lex_parse(src)?)
    }

    // Value Debug/PartialEq derive QILMAYDI (closure'lar) — qiymatni `repr()` matni
    // bilan solishtiramiz (REPL ham aynan repr'ni chop etadi).
    #[test]
    fn repl_oxirgi_ifoda_qiymatini_qaytaradi() {
        let interp = interp::Interp::new_arc();
        // Ifoda qiymati qaytadi
        assert_eq!(repl_chunk(&interp, "1 + 2").unwrap().repr(), "3");
        // Bind (e'lon) nil qaytaradi — REPL bunday natijani chop ETMAYDI
        assert!(matches!(
            repl_chunk(&interp, "x = 10").unwrap(),
            value::Value::Nil
        ));
        // Oxirgi stmt qiymati: oldingi chunk'dagi `x` ko'rinadi (state saqlanadi)
        assert_eq!(repl_chunk(&interp, "x * 3").unwrap().repr(), "30");
        // String qiymat repr'da tirnoq bilan ko'rsatiladi
        assert_eq!(
            repl_chunk(&interp, r#""hello""#).unwrap().repr(),
            "\"hello\""
        );
    }

    #[test]
    fn repl_state_chunklar_orasida_saqlanadi() {
        let interp = interp::Interp::new_arc();
        // fn ta'rifi bir chunk'da, chaqiruvi keyingisida — bitta interp'da yashaydi.
        repl_chunk(&interp, "fn sq n\n  ret n * n").unwrap();
        assert_eq!(repl_chunk(&interp, "sq 9").unwrap().repr(), "81");
        // <- bilan o'zgaruvchi va keyin uni o'qish
        repl_chunk(&interp, "c <- 0").unwrap();
        repl_chunk(&interp, "c <- c + 5").unwrap();
        assert_eq!(repl_chunk(&interp, "c").unwrap().repr(), "5");
    }

    #[test]
    fn repl_xato_qaytadi_sessiya_oldinmas() {
        let interp = interp::Interp::new_arc();
        // Noma'lum nom xato qaytaradi (panic emas) — REPL buni stderr'ga chiqarib
        // davom etadi. Keyingi chunk normal ishlaydi (interp buzilmaydi).
        assert!(repl_chunk(&interp, "nosuchvar + 1").is_err());
        assert_eq!(repl_chunk(&interp, "1 + 1").unwrap().repr(), "2");
    }

    #[test]
    fn repl_multiline_block_heuristikasi() {
        // Bir qatorli ifoda — blok emas (parse o'tishi bilan darhol eval bo'ladi).
        assert!(!is_multiline_block("1 + 2"));
        // if + indentatsiyali body — blok (else/davom kelishi mumkin, kutiladi).
        assert!(is_multiline_block("if x > 5\n  \"big\""));
        // tab bilan chekinish ham blok hisoblanadi.
        assert!(is_multiline_block("fn f\n\tret 1"));
        // Bo'sh bufer — blok emas.
        assert!(!is_multiline_block(""));
    }
}
