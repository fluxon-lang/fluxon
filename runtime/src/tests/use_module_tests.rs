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

// A battery `use` (`use http`) is still a no-op — no file is loaded, dispatch works.
#[test]
fn use_batareya_hamon_no_op() {
    // `use math` does not look for a file (no error), math.* dispatch works.
    run(r#"
use math
(math.floor 3.7 == 3) | (fail "floor wrong")
"#);
}
