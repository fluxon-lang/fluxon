use super::*;

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
