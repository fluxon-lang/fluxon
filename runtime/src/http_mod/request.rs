// Request parsing: percent-decoding, query string, multipart/form-data, and
// building the `req` Value::Map (with the shared ctx cell).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use bytes::Bytes;

use crate::builtins::json_decode;
use crate::value::Value;

// Decodes percent-encoded UTF-8 bytes (`%D0%9A`): turns `%XX` pairs into bytes
// and accumulates them, leaving other bytes unchanged. The accumulated bytes are
// interpreted as UTF-8 — `from_utf8_lossy` replaces an invalid sequence with
// U+FFFD (no panic). An invalid `%` (e.g. `%zz` or a `%` at the end of the
// string) stays as a literal `%`. The browser always percent-encodes non-ASCII
// (Cyrillic/Uzbek) values in the query and path — without this function
// `req.query.q` would stay raw as `%D1%81...` (issue #100).
//
// `keep_path_seps` — when `true`, `%2F` (`/`) and `%5C` (`\`) are NOT decoded
// and stay raw as `%2F`/`%5C` (for path params). Reason: the invariant that a
// `:param` value comes from a single segment — if an encoded slash were decoded,
// `/` would enter the value and could not be told apart from a real path
// separator, and a handler using the param as an ID or safe path component would
// unexpectedly get an inner slash (codex review). Query values have no such risk
// — there it is `false`.
pub(crate) fn percent_decode(s: &str, keep_path_seps: bool) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                let byte = (hi * 16 + lo) as u8;
                // In a path param keep slash/backslash raw (to not break the
                // segment invariant) — pass the three bytes through unchanged.
                if keep_path_seps && (byte == b'/' || byte == b'\\') {
                    out.extend_from_slice(&bytes[i..i + 3]);
                    i += 3;
                    continue;
                }
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// "a=1&b=2" -> {a:"1" b:"2"}. Both key and value get `+` -> space
// (form-encoding) and percent-decode (issue #100) — keys can also contain
// non-ASCII, so both are decoded.
pub(crate) fn parse_query(q: &str) -> Value {
    let mut m = BTreeMap::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        let key = percent_decode(&k.replace('+', " "), false);
        let val = percent_decode(&v.replace('+', " "), false);
        m.insert(key, Value::Str(val));
    }
    Value::Map(m)
}

// --- multipart/form-data (issue #133) ---

// Extracts the multipart boundary from Content-Type. If it is not
// multipart/form-data, or no boundary is found, None — the caller falls back to
// the plain body path. The boundary may be quoted (RFC 2046 allows it).
pub(crate) fn multipart_boundary(ct: &str) -> Option<String> {
    let lower = ct.to_ascii_lowercase();
    if !lower.contains("multipart/form-data") {
        return None;
    }
    let i = lower.find("boundary=")?;
    let rest = &ct[i + "boundary=".len()..];
    let val = if let Some(r) = rest.strip_prefix('"') {
        r.split('"').next().unwrap_or("")
    } else {
        rest.split(';').next().unwrap_or("").trim()
    };
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

// Searches for a sub-sequence within bytes (memmem). A multipart body may be
// binary — str methods are invalid, so we work at the byte level.
fn find_sub(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < from + needle.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| i + from)
}

// Reads a parameter value from a Content-Disposition line (`name="x"`,
// `filename="a.png"`). When searching for `name`, to avoid mistakenly matching
// the "name=" inside `filename=` we check that the char BEFORE the match is a
// separator (`;`/space). The value may be unquoted too (old clients) — in that
// case it is read up to `;`.
pub(crate) fn cd_param(line: &str, key: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let pat = format!("{}=", key);
    let mut search = 0;
    while let Some(i) = lower[search..].find(&pat).map(|i| i + search) {
        let at_boundary = i == 0 || matches!(lower.as_bytes()[i - 1], b';' | b' ' | b'\t');
        if at_boundary {
            let rest = &line[i + pat.len()..];
            return Some(if let Some(r) = rest.strip_prefix('"') {
                match r.find('"') {
                    Some(e) => r[..e].to_string(),
                    None => r.to_string(), // unclosed quote — read to the end
                }
            } else {
                let e = rest.find(';').unwrap_or(rest.len());
                rest[..e].trim().to_string()
            });
        }
        search = i + pat.len();
    }
    None
}

// Does a boundary line really end here? RFC 2046: after `--boundary` comes
// either a closing `--`, or optional transport padding (space/tab) + CRLF.
// Without this check, a chance `\r\n--abcXYZ` in file content (a prefix of
// boundary `abc`) would be taken as a boundary and the part wrongly cut (codex
// P2 review) — in that case it is valid content, not a boundary.
fn boundary_line_ends(rest: &[u8]) -> bool {
    if rest.starts_with(b"--") {
        return true;
    }
    let mut i = 0;
    while i < rest.len() && (rest[i] == b' ' || rest[i] == b'\t') {
        i += 1;
    }
    rest[i..].starts_with(b"\r\n")
}

// Searches for a complete boundary line: the bytes after `marker` must also
// confirm a boundary line (boundary_line_ends). Non-matching prefix occurrences
// (`--boundaryX...` in file content) are skipped.
fn find_boundary(body: &[u8], marker: &[u8], from: usize) -> Option<usize> {
    let mut search = from;
    loop {
        let i = find_sub(body, marker, search)?;
        if boundary_line_ends(&body[i + marker.len()..]) {
            return Some(i);
        }
        search = i + 1;
    }
}

// Splits a multipart/form-data body into parts: plain form fields -> a fields
// map (req.body — symmetric with JSON), file parts (with a filename) -> a files
// list ({name filename content size}). If the body does not match the format
// (no boundary found, broken structure) None — the caller falls back to the raw
// body, so a malformed request does not lose data.
//
// File content follows the same rule as req.body: UTF-8 text -> str, binary ->
// bytes (issue #132) — the AI learns one pattern. `size` is always the BYTE
// count (str.len counts characters — which would be wrong for a file size).
#[allow(clippy::type_complexity)]
pub(crate) fn parse_multipart(
    body: &[u8],
    boundary: &str,
) -> Option<(BTreeMap<String, Value>, Vec<Value>)> {
    let delim = format!("--{}", boundary).into_bytes();
    // Part-end marker: CRLF + boundary (the CRLF is not part of the content).
    let mut end_marker = b"\r\n".to_vec();
    end_marker.extend_from_slice(&delim);

    let mut fields = BTreeMap::new();
    let mut files = Vec::new();

    // The first boundary (RFC 2046 allows a preamble before it). If the body
    // starts directly with `--boundary` there is no CRLF prefix — check that
    // separately; otherwise search for the full boundary at a line start
    // (after a CRLF).
    let mut pos = if body.starts_with(&delim) && boundary_line_ends(&body[delim.len()..]) {
        delim.len()
    } else {
        find_boundary(body, &end_marker, 0)? + end_marker.len()
    };
    loop {
        // `--` after the boundary — the final boundary, we are done.
        if body[pos..].starts_with(b"--") {
            break;
        }
        // The boundary line ends with CRLF (transport padding may be in between).
        let nl = find_sub(body, b"\r\n", pos)?;
        let part_start = nl + 2;
        let part_end = find_boundary(body, &end_marker, part_start)?;
        let part = &body[part_start..part_end];

        // A part: headers + a blank line + content. The headers are text (ASCII)
        // — lossy reading is safe; the content stays as raw bytes.
        if let Some(hdr_end) = find_sub(part, b"\r\n\r\n", 0) {
            let headers_raw = String::from_utf8_lossy(&part[..hdr_end]);
            let content = &part[hdr_end + 4..];
            let cd_line = headers_raw
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("content-disposition:"));
            if let Some(cd) = cd_line
                && let Some(name) = cd_param(cd, "name")
            {
                let content_value = match std::str::from_utf8(content) {
                    Ok(s) => Value::Str(s.to_string()),
                    Err(_) => Value::Bytes(Arc::new(content.to_vec())),
                };
                match cd_param(cd, "filename") {
                    // Has filename — a file part (an empty filename is a file
                    // too: that is how the browser sends an empty file input).
                    Some(filename) => {
                        let mut fm = BTreeMap::new();
                        fm.insert("name".to_string(), Value::Str(name));
                        fm.insert("filename".to_string(), Value::Str(filename));
                        fm.insert("content".to_string(), content_value);
                        fm.insert("size".to_string(), Value::Int(content.len() as i64));
                        files.push(Value::Map(fm));
                    }
                    // A plain form field — goes to req.body (treated as text).
                    None => {
                        fields.insert(
                            name,
                            Value::Str(String::from_utf8_lossy(content).into_owned()),
                        );
                    }
                }
            }
        }
        pos = part_end + end_marker.len();
    }
    Some((fields, files))
}

// --- request -> Value::Map ---

// req = {method, path, query:{}, headers:{}, params:{}, body:(JSON map/str), files:[], ctx}
// ctx — a shared request-scoped store (issue #68): middleware writes
// `req.ctx <- {...}`, the handler reads `req.ctx`. The caller (`handle_request`)
// adds ctx (`with_ctx`), because it creates a fresh Arc<Mutex> per request — so
// middleware and handler see the same cell.
// req has many fields (method/path/query/headers/params/ip/body) — gathering
// them into a separate struct would be overkill for a single call site, so we
// keep positional arguments (disabling the too_many_arguments lint here).
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_req(
    method: String,
    path: String,
    query: String,
    headers: BTreeMap<String, Value>,
    params: BTreeMap<String, Value>,
    ip: String,
    body_bytes: Bytes,
    is_json: bool,
    multipart: Option<String>,
) -> Value {
    // multipart/form-data (issue #133): plain fields go to req.body (symmetric
    // with JSON), files to req.files. If the parse fails (a malformed body) we
    // fall through to the plain path below — the raw body is not lost.
    let parsed_multipart = multipart
        .as_deref()
        .and_then(|b| parse_multipart(&body_bytes, b));
    let mut files = Vec::new();
    let body = if let Some((fields, fs)) = parsed_multipart {
        files = fs;
        Value::Map(fields)
    } else if body_bytes.is_empty() {
        Value::Nil
    } else {
        match std::str::from_utf8(&body_bytes) {
            // If Content-Type is JSON, OR the body starts with `{`/`[` — try a
            // JSON parse. Reason: `curl -d` sends x-www-form-urlencoded by
            // default, yet the body looks like JSON; if we bound strictly to
            // Content-Type, the developer would needlessly get a string and a
            // `body.field` access would give the misleading "str.field method" error.
            Ok(s) => {
                let looks_like_json =
                    matches!(s.trim_start().as_bytes().first(), Some(b'{') | Some(b'['));
                if is_json || looks_like_json {
                    // If JSON decoding fails — keep it as raw text.
                    json_decode(s).unwrap_or_else(|_| Value::Str(s.to_string()))
                } else {
                    Value::Str(s.to_string())
                }
            }
            // A non-UTF-8 body — binary payload (image, gzip): bytes
            // (issue #132). Lossy reading used to silently corrupt the data.
            Err(_) => Value::Bytes(Arc::new(body_bytes.to_vec())),
        }
    };

    let mut m = BTreeMap::new();
    m.insert("method".to_string(), Value::Str(method));
    m.insert("path".to_string(), Value::Str(path));
    m.insert("query".to_string(), parse_query(&query));
    m.insert("headers".to_string(), Value::Map(headers));
    m.insert("params".to_string(), Value::Map(params));
    // req.ip — the client IP (TCP peer). If the rate-limit key function returns
    // nil we fall back to this; the user can also read `req.ip`. Behind a proxy
    // this is the proxy's IP (X-Forwarded-For is not handled in v1 — docs).
    m.insert("ip".to_string(), Value::Str(ip));
    m.insert("body".to_string(), body);
    // req.files is always a list (empty when not multipart) — `each f in
    // req.files` works without a nil check (issue #133).
    m.insert("files".to_string(), Value::List(files));
    Value::Map(m)
}

// Adds the shared ctx cell (`req.ctx`) to the req map (issue #68). Separate from
// build_req — a fresh cell is created per request (in the caller), and this
// function places it in req's "ctx" key.
pub(crate) fn with_ctx(req: Value, ctx: Arc<Mutex<BTreeMap<String, Value>>>) -> Value {
    if let Value::Map(mut m) = req {
        m.insert("ctx".to_string(), Value::Ctx(ctx));
        Value::Map(m)
    } else {
        req
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http_mod::routing::Route;
    use crate::http_mod::routing::{match_route, parse_pattern};

    // --- bytes (issue #132): binary body on the request/response paths ---

    // A non-UTF-8 request body comes as bytes (it used to be corrupted by lossy).
    #[test]
    fn build_req_ikkilik_tana_bytes() {
        let req = build_req(
            "POST".into(),
            "/upload".into(),
            String::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            "1.1.1.1".into(),
            Bytes::from(vec![0xff, 0xfe, 0x00]),
            false,
            None,
        );
        let Value::Map(m) = req else {
            panic!("req map expected");
        };
        match m.get("body") {
            Some(Value::Bytes(b)) => assert_eq!(**b, vec![0xff, 0xfe, 0x00]),
            _ => panic!("binary body must be bytes"),
        }
    }

    // A text body stays str as before (regression guard).
    #[test]
    fn build_req_matn_tana_str_qoladi() {
        let req = build_req(
            "POST".into(),
            "/t".into(),
            String::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            "1.1.1.1".into(),
            Bytes::from("hello"),
            false,
            None,
        );
        let Value::Map(m) = req else {
            panic!("req map expected");
        };
        match m.get("body") {
            Some(Value::Str(s)) => assert_eq!(s, "hello"),
            _ => panic!("text body must be str"),
        }
    }

    // --- multipart/form-data (issue #133) ---

    // Builds a typical multipart body like a browser/curl sends.
    fn multipart_body(boundary: &str, parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, filename, content) in parts {
            out.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
            match filename {
                Some(f) => out.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n",
                        name, f
                    )
                    .as_bytes(),
                ),
                None => out.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{}\"\r\n", name).as_bytes(),
                ),
            }
            out.extend_from_slice(b"\r\n");
            out.extend_from_slice(content);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());
        out
    }

    #[test]
    fn multipart_boundary_oddiy_va_qoshtirnoqli() {
        // Both plain and quoted boundaries parse; another content-type (JSON)
        // returns None.
        assert_eq!(
            multipart_boundary("multipart/form-data; boundary=----WebKit123"),
            Some("----WebKit123".to_string())
        );
        assert_eq!(
            multipart_boundary("multipart/form-data; boundary=\"abc def\""),
            Some("abc def".to_string())
        );
        assert_eq!(multipart_boundary("application/json"), None);
        assert_eq!(multipart_boundary("multipart/form-data"), None);
    }

    #[test]
    fn cd_param_filename_ichidagi_name_adashtirmaydi() {
        // A "filename=" search must not match "name=" — the separator is checked.
        let line = "Content-Disposition: form-data; name=\"avatar\"; filename=\"a.png\"";
        assert_eq!(cd_param(line, "name").as_deref(), Some("avatar"));
        assert_eq!(cd_param(line, "filename").as_deref(), Some("a.png"));
        // A part without filename — a plain field.
        let field = "Content-Disposition: form-data; name=\"title\"";
        assert_eq!(cd_param(field, "name").as_deref(), Some("title"));
        assert_eq!(cd_param(field, "filename"), None);
    }

    #[test]
    fn parse_multipart_maydon_va_fayl() {
        // A plain field goes to req.body, a file (with filename) to the files list.
        let body = multipart_body(
            "BB",
            &[
                ("title", None, b"hello world"),
                ("doc", Some("a.txt"), b"text file"),
            ],
        );
        let (fields, files) = parse_multipart(&body, "BB").expect("parse must succeed");
        match fields.get("title") {
            Some(Value::Str(s)) => assert_eq!(s, "hello world"),
            _ => panic!("title must be str"),
        }
        assert_eq!(files.len(), 1);
        let Value::Map(f) = &files[0] else {
            panic!("file map expected");
        };
        assert!(matches!(f.get("name"), Some(Value::Str(s)) if s == "doc"));
        assert!(matches!(f.get("filename"), Some(Value::Str(s)) if s == "a.txt"));
        assert!(matches!(f.get("content"), Some(Value::Str(s)) if s == "text file"));
        assert!(matches!(f.get("size"), Some(Value::Int(9))));
    }

    #[test]
    fn parse_multipart_ikkilik_fayl_bytes() {
        // Binary content (not UTF-8, with CRLF inside) comes as bytes and the
        // bytes are preserved exactly; size is the byte count.
        let data: &[u8] = &[0xff, 0xd8, b'\r', b'\n', 0x00, 0xfe];
        let body = multipart_body("XX", &[("img", Some("a.jpg"), data)]);
        let (_, files) = parse_multipart(&body, "XX").expect("parse must succeed");
        let Value::Map(f) = &files[0] else {
            panic!("file map expected");
        };
        match f.get("content") {
            Some(Value::Bytes(b)) => assert_eq!(**b, data.to_vec()),
            _ => panic!("binary content must be bytes"),
        }
        assert!(matches!(f.get("size"), Some(Value::Int(6))));
    }

    #[test]
    fn parse_multipart_bir_nom_bir_nechta_fayl() {
        // Multiple files with the same name (`<input multiple>`) — all stay in
        // the list (not a map, so none is lost).
        let body = multipart_body(
            "MM",
            &[
                ("docs", Some("1.txt"), b"bir"),
                ("docs", Some("2.txt"), b"ikki"),
            ],
        );
        let (_, files) = parse_multipart(&body, "MM").expect("parse must succeed");
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn parse_multipart_mazmundagi_boundary_prefiksi_kesmaydi() {
        // The file content has `\r\n--abcXYZ` (a prefix of boundary `abc`, but
        // not a full boundary line) — the part must be kept WHOLE, not cut
        // (codex P2 review: searching only for `\r\n--boundary` corrupted content).
        let data: &[u8] = b"first\r\n--abcXYZ\r\nremaining part";
        let body = multipart_body("abc", &[("doc", Some("a.txt"), data)]);
        let (_, files) = parse_multipart(&body, "abc").expect("parse must succeed");
        assert_eq!(files.len(), 1);
        let Value::Map(f) = &files[0] else {
            panic!("file map expected");
        };
        match f.get("content") {
            Some(Value::Str(s)) => assert_eq!(s.as_bytes(), data),
            _ => panic!("content must be a whole str"),
        }
        assert!(matches!(f.get("size"), Some(Value::Int(n)) if *n == data.len() as i64));
    }

    #[test]
    fn parse_multipart_padding_bilan_boundary_qabul() {
        // RFC 2046: a boundary line may be followed by transport padding
        // (space/tab) — such a boundary is taken as valid.
        let mut body = Vec::new();
        body.extend_from_slice(b"--PP  \r\n");
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"a\"\r\n\r\n");
        body.extend_from_slice(b"value");
        body.extend_from_slice(b"\r\n--PP--\r\n");
        let (fields, _) = parse_multipart(&body, "PP").expect("parse must succeed");
        assert!(matches!(fields.get("a"), Some(Value::Str(s)) if s == "value"));
    }

    #[test]
    fn parse_multipart_buzuq_tana_none() {
        // The boundary is not in the body at all — None, the caller falls back to the raw body.
        assert!(parse_multipart(b"just text", "NONE").is_none());
    }

    #[test]
    fn build_req_multipart_body_va_files() {
        // Full path: when a boundary is given, req.body is a fields map and
        // req.files is the files list.
        let body = multipart_body(
            "ZZ",
            &[("title", None, b"my image"), ("pic", Some("p.png"), b"PNG")],
        );
        let req = build_req(
            "POST".into(),
            "/upload".into(),
            String::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            "1.1.1.1".into(),
            Bytes::from(body),
            false,
            Some("ZZ".to_string()),
        );
        let Value::Map(m) = req else {
            panic!("req map expected");
        };
        let Some(Value::Map(b)) = m.get("body") else {
            panic!("body map expected");
        };
        assert!(matches!(b.get("title"), Some(Value::Str(s)) if s == "my image"));
        match m.get("files") {
            Some(Value::List(fs)) => assert_eq!(fs.len(), 1),
            _ => panic!("files must be a list"),
        }
    }

    #[test]
    fn build_req_multipart_emas_files_bosh_list() {
        // req.files exists on a plain request too (empty list) — `each` works
        // without a nil check.
        let req = build_req(
            "POST".into(),
            "/t".into(),
            String::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            "1.1.1.1".into(),
            Bytes::from("{\"a\":1}"),
            true,
            None,
        );
        let Value::Map(m) = req else {
            panic!("req map expected");
        };
        match m.get("files") {
            Some(Value::List(fs)) => assert!(fs.is_empty()),
            _ => panic!("files must be an empty list"),
        }
    }

    #[test]
    fn build_req_multipart_buzuq_xom_qoladi() {
        // A boundary is present but the body does not match — parse None, the
        // body stays a raw str (data is not silently lost), files empty.
        let req = build_req(
            "POST".into(),
            "/u".into(),
            String::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            "1.1.1.1".into(),
            Bytes::from("plain text"),
            false,
            Some("QQ".to_string()),
        );
        let Value::Map(m) = req else {
            panic!("req map expected");
        };
        assert!(matches!(m.get("body"), Some(Value::Str(s)) if s == "plain text"));
        match m.get("files") {
            Some(Value::List(fs)) => assert!(fs.is_empty()),
            _ => panic!("files must be an empty list"),
        }
    }

    #[test]
    fn req_ip_maydoni_mavjud() {
        // build_req sets req.ip — the user can read `req.ip`.
        let req = build_req(
            "GET".into(),
            "/".into(),
            String::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            "10.0.0.1".into(),
            Bytes::new(),
            false,
            None,
        );
        let Value::Map(m) = &req else {
            panic!("Map");
        };
        assert!(
            matches!(m.get("ip"), Some(Value::Str(s)) if s == "10.0.0.1"),
            "req.ip must be set"
        );
    }

    // --- query/path percent-decode (issue #100) ---

    fn query_get(q: &str, key: &str) -> Option<String> {
        match parse_query(q) {
            Value::Map(m) => match m.get(key) {
                Some(Value::Str(s)) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    #[test]
    fn percent_dekod_utf8_kirill() {
        // `%D1%81...` -> "салом" (Cyrillic UTF-8 bytes are assembled correctly).
        assert_eq!(
            percent_decode("%D1%81%D0%B0%D0%BB%D0%BE%D0%BC", false),
            "салом"
        );
    }

    #[test]
    fn percent_dekod_oddiy_belgi() {
        // `%20` -> space, `%2B` -> literal `+` (does not become a space).
        assert_eq!(percent_decode("a%20b", false), "a b");
        assert_eq!(percent_decode("a%2Bb", false), "a+b");
    }

    #[test]
    fn percent_dekod_yaroqsiz_qoldiradi() {
        // An invalid `%` sequence (`%zz`) and a `%` at the end of the string stay
        // literal (no panic).
        assert_eq!(percent_decode("%zz", false), "%zz");
        assert_eq!(percent_decode("100%", false), "100%");
        assert_eq!(percent_decode("a%2", false), "a%2");
    }

    #[test]
    fn percent_dekod_slash_keep_path_seps() {
        // keep_path_seps=true: `%2F`/`%5C` (both cases) stay raw, but other bytes
        // (`%61` -> 'a') are decoded as usual. When false (query) they become `/`/`\`.
        assert_eq!(percent_decode("a%2Fb", true), "a%2Fb");
        assert_eq!(percent_decode("a%2fb", true), "a%2fb");
        assert_eq!(percent_decode("a%5Cb", true), "a%5Cb");
        assert_eq!(percent_decode("%61%2F%61", true), "a%2Fa");
        assert_eq!(percent_decode("a%2Fb", false), "a/b");
    }

    #[test]
    fn query_percent_dekod_qiymat() {
        // GET /search?q=%D1%81%D0%B0%D0%BB%D0%BE%D0%BC -> q = "салом".
        assert_eq!(
            query_get("q=%D1%81%D0%B0%D0%BB%D0%BE%D0%BC", "q").as_deref(),
            Some("салом")
        );
    }

    #[test]
    fn query_plus_boshliq_va_percent() {
        // `+` -> space (form-encoding), `%20` is also a space.
        assert_eq!(
            query_get("name=John+Doe", "name").as_deref(),
            Some("John Doe")
        );
        assert_eq!(
            query_get("name=John%20Doe", "name").as_deref(),
            Some("John Doe")
        );
    }

    #[test]
    fn query_kalit_ham_dekod() {
        // A key can contain non-ASCII too — it is decoded as well.
        assert_eq!(query_get("%D0%B0=1", "а").as_deref(), Some("1"));
    }

    #[test]
    fn path_param_percent_dekod() {
        // `/users/:name` -> "/users/%D0%90%D0%BB%D0%B8" gives param "name" = "Али".
        let routes = vec![Route {
            method: "get".into(),
            pattern: parse_pattern("/users/:name"),
            handler: Value::Nil,
        }];
        let (_r, params) =
            match_route(&routes, "get", "/users/%D0%90%D0%BB%D0%B8").expect("route must match");
        assert!(matches!(params.get("name"), Some(Value::Str(s)) if s == "Али"));
    }

    #[test]
    fn path_param_encoded_slash_xom_qoladi() {
        // "/users/a%2Fb" matches ":name" as a single segment, but `%2F` is NOT
        // decoded — no `/` enters the param value (segment invariant; a handler
        // using it as an ID/path component must not get an inner slash, codex
        // review). Non-ASCII in another segment is still decoded.
        let routes = vec![Route {
            method: "get".into(),
            pattern: parse_pattern("/users/:name"),
            handler: Value::Nil,
        }];
        let (_r, params) =
            match_route(&routes, "get", "/users/a%2Fb").expect("one segment — match");
        assert!(matches!(params.get("name"), Some(Value::Str(s)) if s == "a%2Fb"));
    }

    // --- req.ctx shared cell (issue #68) ---

    #[test]
    fn with_ctx_shared_cell_qoshadi() {
        // with_ctx puts the "ctx" key into the req map as a Value::Ctx.
        let cell = Arc::new(Mutex::new(BTreeMap::new()));
        let req = build_req(
            "GET".into(),
            "/".into(),
            String::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            "127.0.0.1".into(),
            Bytes::new(),
            false,
            None,
        );
        let req = with_ctx(req, cell.clone());
        let Value::Map(m) = &req else {
            panic!("req must be a Map");
        };
        assert!(matches!(m.get("ctx"), Some(Value::Ctx(_))));
    }

    #[test]
    fn ctx_cell_klon_orqali_ulashiladi() {
        // When req is cloned the ctx Arc is shared — if we write to the cell from
        // outside, it is visible through the clone too (the middleware->handler
        // flow relies on this mechanism).
        let cell = Arc::new(Mutex::new(BTreeMap::new()));
        let req = with_ctx(
            build_req(
                "GET".into(),
                "/".into(),
                String::new(),
                BTreeMap::new(),
                BTreeMap::new(),
                "127.0.0.1".into(),
                Bytes::new(),
                false,
                None,
            ),
            cell.clone(),
        );
        let req_clone = req.clone();
        // Write to the cell from outside (middleware `req.ctx <-` does this).
        cell.lock()
            .unwrap()
            .insert("tenant_id".to_string(), Value::Int(7));
        // Reading through the clone shows the new value (proof the Arc is shared).
        let Value::Map(m) = &req_clone else {
            panic!("Map");
        };
        let Some(Value::Ctx(c)) = m.get("ctx") else {
            panic!("ctx cell");
        };
        // Value does not derive Debug — check with equals (not assert_eq).
        let got = c.lock().unwrap().get("tenant_id").cloned().unwrap();
        assert!(got.equals(&Value::Int(7)), "ctx updated through the clone");
    }

    #[test]
    fn ctx_self_equals_deadlock_qilmaydi() {
        // `req == req` (or comparing a req clone) — sees the same ctx Arc<Mutex>
        // from two sides. equals must not hold both locks at once, otherwise a
        // non-reentrant mutex deadlocks (Codex P2). This test exercises that
        // path: if it blocks it hangs, otherwise it passes immediately.
        let cell = Arc::new(Mutex::new(BTreeMap::new()));
        let req = with_ctx(
            build_req(
                "GET".into(),
                "/".into(),
                String::new(),
                BTreeMap::new(),
                BTreeMap::new(),
                "127.0.0.1".into(),
                Bytes::new(),
                false,
                None,
            ),
            cell,
        );
        let req_clone = req.clone();
        // Map equality reaches the ctx key -> (Ctx,Ctx) same Arc -> ptr_eq.
        assert!(req.equals(&req_clone), "req equals its clone, no deadlock");
        assert!(req.equals(&req), "req equals itself, no deadlock");
    }
}
