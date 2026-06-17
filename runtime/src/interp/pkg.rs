// `.pkg` manifest parsing — the "battery-shaped module" AI-doc sidecar (#202).
//
// A reusable user module (`use ./lib/s3`) may ship an OPTIONAL sibling manifest
// `lib/s3.pkg` whose mandatory `doc` block is the micro-equivalent of a
// battery's entry in `fluxon-agent.md`: a short canonical doc the agent reads
// instead of the implementation. The format is deliberately NOT Fluxon syntax —
// it is a tiny line-oriented format with exactly two keys (`name`, `doc`), so
// this parser is intentionally separate from the lexer/parser.
//
// This file is pure (no IO, no `&self`): `module.rs::validate_pkg` reads the
// file and applies the load-time policy. Mirrors the `parse_dotenv`/`load_dotenv`
// split in `util.rs`.

use std::collections::HashSet;

// A parsed `.pkg` manifest. `doc` is the dedented block-string body, verbatim —
// its internal sections (WHAT/CANONICAL/GOTCHAS/DEPENDS) are free-text
// conventions, not parsed structure.
#[derive(Debug)]
pub(crate) struct PkgManifest {
    #[allow(dead_code)] // read by the phase-3 skill via the file, not in-process yet
    pub name: String,
    pub doc: String,
}

// Parses `.pkg` text. Line-oriented; `#` comments and blank lines are skipped.
// Recognised keys: `name <value>` and a `doc """ ... """` block. Unknown keys
// are an error (so typos like `nam`/`doc:` surface rather than being ignored).
// The empty-doc policy is NOT enforced here — the parser only requires the keys
// to be syntactically present; the load hook decides whether an empty doc fails.
pub(crate) fn parse_pkg(src: &str) -> Result<PkgManifest, String> {
    let lines: Vec<&str> = src.lines().collect();
    let mut name: Option<String> = None;
    let mut doc: Option<String> = None;

    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        // Split into the key and the rest of the line on the first whitespace run.
        let (key, rest) = match trimmed.split_once(char::is_whitespace) {
            Some((k, r)) => (k, r.trim()),
            None => (trimmed, ""),
        };
        match key {
            "name" => {
                if name.is_some() {
                    return Err("pkg: duplicate 'name'".into());
                }
                if rest.is_empty() {
                    return Err("pkg: 'name' value is empty".into());
                }
                name = Some(rest.to_string());
                i += 1;
            }
            "doc" => {
                if doc.is_some() {
                    return Err("pkg: duplicate 'doc'".into());
                }
                if rest != "\"\"\"" {
                    return Err("pkg: text after \"\"\" — doc content starts on a new line".into());
                }
                // Consume the body until a line that trims to the closing fence.
                let mut body: Vec<String> = Vec::new();
                i += 1;
                let mut closed = false;
                while i < lines.len() {
                    if lines[i].trim() == "\"\"\"" {
                        closed = true;
                        i += 1;
                        break;
                    }
                    body.push(lines[i].to_string());
                    i += 1;
                }
                if !closed {
                    return Err("pkg: unterminated doc block (missing closing \"\"\")".into());
                }
                doc = Some(dedent(&body));
            }
            other => return Err(format!("pkg: unknown key '{}'", other)),
        }
    }

    let name = name.ok_or("pkg: missing required key 'name'")?;
    let doc = doc.ok_or("pkg: missing required key 'doc'")?;
    Ok(PkgManifest { name, doc })
}

// Strips the smallest common leading-space indentation across non-blank lines
// and joins with `\n`. A plain-text re-implementation of the lexer's
// `dedent_block_lines` (it operates on interpolation `StrPart`s, so it cannot be
// reused here). Only ASCII spaces are stripped (matching the lexer).
fn dedent(lines: &[String]) -> String {
    let min_indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start_matches(' ').len())
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| {
            if l.trim().is_empty() {
                ""
            } else {
                &l[min_indent..]
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// Scans the doc for `<modname>.<ident>` references (the CANONICAL form, e.g.
// `s3.upload`) and returns the set of `<ident>`s. Used for the soft check that
// the doc only advertises names the module actually `exp`-orts. Scanning the
// whole doc (not just a CANONICAL: sub-section) is robust — the false-positive
// risk is negligible since the doc is about this one module.
pub(crate) fn referenced_names(doc: &str, modname: &str) -> HashSet<String> {
    let bytes = doc.as_bytes();
    let prefix = format!("{}.", modname);
    let pre = prefix.as_bytes();
    let mut out = HashSet::new();
    let mut i = 0;
    while i + pre.len() < bytes.len() {
        if &bytes[i..i + pre.len()] == pre {
            // The char before the prefix must not be an identifier char, so
            // `foos3.x` does not match `s3.x`.
            let boundary = i == 0 || !is_ident_cont(bytes[i - 1]);
            if boundary {
                let mut j = i + pre.len();
                let start = j;
                if j < bytes.len() && is_ident_start(bytes[j]) {
                    j += 1;
                    while j < bytes.len() && is_ident_cont(bytes[j]) {
                        j += 1;
                    }
                    out.insert(doc[start..j].to_string());
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}
fn is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod pkg_tests {
    use super::*;

    #[test]
    fn parses_name_and_doc() {
        let m = parse_pkg("name s3\ndoc \"\"\"\n  hello\n  world\n\"\"\"\n").unwrap();
        assert_eq!(m.name, "s3");
        assert_eq!(m.doc, "hello\nworld");
    }

    #[test]
    fn dedents_nested_indentation() {
        let src = "name s3\ndoc \"\"\"\n  WHAT: x\n  CANONICAL:\n    use ./lib/s3\n\"\"\"\n";
        let m = parse_pkg(src).unwrap();
        // The 2-space common indent is stripped; the deeper line keeps its extra.
        assert_eq!(m.doc, "WHAT: x\nCANONICAL:\n  use ./lib/s3");
    }

    #[test]
    fn skips_comments_and_blanks() {
        let m = parse_pkg("# a comment\n\nname s3\n\ndoc \"\"\"\nbody\n\"\"\"\n").unwrap();
        assert_eq!(m.name, "s3");
        assert_eq!(m.doc, "body");
    }

    #[test]
    fn rejects_text_after_fence() {
        let err = parse_pkg("name s3\ndoc \"\"\" oops\nbody\n\"\"\"\n").unwrap_err();
        assert!(err.contains("text after"), "{}", err);
    }

    #[test]
    fn rejects_unterminated_doc() {
        let err = parse_pkg("name s3\ndoc \"\"\"\nbody never closed\n").unwrap_err();
        assert!(err.contains("unterminated doc block"), "{}", err);
    }

    #[test]
    fn rejects_unknown_key() {
        let err = parse_pkg("name s3\nnam typo\n").unwrap_err();
        assert!(err.contains("unknown key 'nam'"), "{}", err);
    }

    #[test]
    fn rejects_missing_name() {
        let err = parse_pkg("doc \"\"\"\nbody\n\"\"\"\n").unwrap_err();
        assert!(err.contains("missing required key 'name'"), "{}", err);
    }

    #[test]
    fn rejects_missing_doc() {
        let err = parse_pkg("name s3\n").unwrap_err();
        assert!(err.contains("missing required key 'doc'"), "{}", err);
    }

    #[test]
    fn extracts_referenced_names() {
        let doc = "url = s3.upload \"bucket\" bytes!\nlink = s3.presign \"k\"\nuse ./lib/s3";
        let refs = referenced_names(doc, "s3");
        assert!(refs.contains("upload"));
        assert!(refs.contains("presign"));
        // bare `s3` (no dot) is not a reference.
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn reference_scan_respects_word_boundary() {
        // `foos3.x` must not match the `s3.` prefix.
        let refs = referenced_names("foos3.x and s3.real", "s3");
        assert!(refs.contains("real"));
        assert!(!refs.contains("x"));
        assert_eq!(refs.len(), 1);
    }
}
