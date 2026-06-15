use super::*;

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
