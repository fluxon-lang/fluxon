use super::*;

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
