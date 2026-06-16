use super::*;

// tui colors are str -> str. Under `cargo test` stdout is NOT a tty, so the escape
// codes are dropped — the text comes back unchanged. (The escape path is covered by
// the unit tests in tui_mod.rs.) Here we check the value semantics from Fluxon: a
// color func returns a plain str and composes with `+`.
#[test]
fn tui_color_returns_str() {
    run(r#"
s = tui.green "ok"
(s == "ok") | (fail "color should pass text through off a tty: ${s}")
"#);
}

// tui.strip removes ANSI codes from a string built by hand.
#[test]
fn tui_strip_removes_escapes() {
    run(r#"
clean = tui.strip "[31mred[0m tail"
(clean == "red tail") | (fail "strip wrong: ${clean}")
"#);
}

// tui.table renders an aligned grid; we assert the cells survive into the output and
// the header rule (─) is present.
#[test]
fn tui_table_renders_cells() {
    run(r#"
t = tui.table [["alice" "admin"] ["bob" "viewer"]] ["user" "role"]
(str.has t "alice") | (fail "missing cell: ${t}")
(str.has t "viewer") | (fail "missing cell: ${t}")
(str.has t "─") | (fail "missing header rule: ${t}")
"#);
}

// tui.box frames the body; the rounded corners appear in the output.
#[test]
fn tui_box_frames_body() {
    run(r#"
b = tui.box "hi"
(str.has b "╭") | (fail "no top corner: ${b}")
(str.has b "hi") | (fail "no body: ${b}")
(str.has b "╯") | (fail "no bottom corner: ${b}")
"#);
}

// tui.badge without a tty falls back to [LABEL].
#[test]
fn tui_badge_plain_off_tty() {
    run(r#"
b = tui.badge "OK" :green
(b == "[OK]") | (fail "badge should be plain off a tty: ${b}")
"#);
}

// An unknown tui function is an explicit error that names the module.
#[test]
fn tui_unknown_func_errors() {
    let err = run_source(r#"tui.nope "x""#).expect_err("an unknown tui function should error");
    assert!(
        err.contains("tui module"),
        "expected a tui module error, got: {}",
        err
    );
}
