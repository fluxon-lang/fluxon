use super::*;

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

// A clean import passes check, and nested imports (main -> a -> b) parse fine.
#[test]
fn check_clean_nested_modules_ok() {
    check_modules(&[
        ("main.fx", "use ./a\nlog (a.get())\n"),
        ("a.fx", "use ./b\nexp fn get -> b.val + 1\n"),
        ("b.fx", "exp val = 42\n"),
    ])
    .unwrap();
}

// A circular import must not hang the recursive check (the visited set breaks
// the cycle); a clean pair like this passes.
#[test]
fn check_circular_import_terminates() {
    check_modules(&[
        ("x.fx", "use ./y\nexp a = 1\n"),
        ("y.fx", "use ./x\nexp b = 2\n"),
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

// ---- battery-shaped modules: optional `.pkg` manifest (#202) ----

// A valid `.pkg` sibling (doc references a real exported name) loads cleanly.
#[test]
fn use_module_pkg_valid_loads() {
    run_modules(&[
        (
            "main.fx",
            "use ./s3\n(s3.upload \"b\" \"k\" == \"b/k\") | (fail \"upload wrong\")\n",
        ),
        ("s3.fx", "exp fn upload bucket key -> \"${bucket}/${key}\"\n"),
        (
            "s3.pkg",
            "name s3\ndoc \"\"\"\n  WHAT: upload to S3.\n  CANONICAL:\n    url = s3.upload \"b\" \"k\"\n\"\"\"\n",
        ),
    ])
    .unwrap();
}

// An empty doc is a hard load error — the AI-doc block is mandatory.
#[test]
fn use_module_pkg_empty_doc_fails() {
    let err = run_modules(&[
        ("main.fx", "use ./s3\nlog s3.upload\n"),
        ("s3.fx", "exp fn upload b k -> b\n"),
        ("s3.pkg", "name s3\ndoc \"\"\"\n   \n\"\"\"\n"),
    ])
    .unwrap_err();
    assert!(err.contains("doc is empty"), "{}", err);
}

// A CANONICAL reference to a name the module does NOT export is a soft warning
// (stderr), not an error — the module still loads.
#[test]
fn use_module_pkg_missing_exp_warns() {
    run_modules(&[
        ("main.fx", "use ./s3\nlog s3.upload\n"),
        ("s3.fx", "exp fn upload b k -> b\n"),
        (
            "s3.pkg",
            "name s3\ndoc \"\"\"\n  CANONICAL: s3.presign \"k\"\n\"\"\"\n",
        ),
    ])
    .unwrap();
}

// No `.pkg` sibling -> backward compatible: the module loads as before.
#[test]
fn use_module_no_pkg_backward_compatible() {
    run_modules(&[
        ("main.fx", "use ./s3\nlog s3.upload\n"),
        ("s3.fx", "exp fn upload b k -> b\n"),
    ])
    .unwrap();
}

// A malformed `.pkg` (unterminated doc block) is a hard load error.
#[test]
fn use_module_pkg_malformed_fails() {
    let err = run_modules(&[
        ("main.fx", "use ./s3\nlog s3.upload\n"),
        ("s3.fx", "exp fn upload b k -> b\n"),
        ("s3.pkg", "name s3\ndoc \"\"\"\nnever closed\n"),
    ])
    .unwrap_err();
    assert!(err.contains("unterminated doc block"), "{}", err);
}

// A malformed `.pkg` must be rejected before the module body runs, so
// top-level effects are not allowed to leak from a failed import.
#[test]
fn use_module_pkg_invalid_does_not_execute_module_body() {
    let dir = temp_module_dir();
    let marker = dir.join("marker.txt");
    std::fs::write(dir.join("main.fx"), "use ./s3\n").unwrap();
    std::fs::write(
        dir.join("s3.fx"),
        format!(
            "fs.write {:?} \"ran\"\nexp fn upload b k -> b\n",
            marker.display().to_string()
        ),
    )
    .unwrap();
    std::fs::write(dir.join("s3.pkg"), "name s3\ndoc \"\"\"\nnever closed\n").unwrap();

    let main_path = dir.join("main.fx");
    let src = std::fs::read_to_string(&main_path).unwrap();
    let err = run_source_at(&src, &main_path).unwrap_err();
    assert!(err.contains("unterminated doc block"), "{}", err);
    assert!(
        !marker.exists(),
        "module body executed before manifest validation"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
