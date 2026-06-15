// Value/Flow -> hyper::Response: status validation, body formatting, custom
// headers, and the read-side HeaderMap -> Fluxon header map.

use std::collections::BTreeMap;

use bytes::Bytes;
use http_body_util::Full;
use hyper::{Response, StatusCode};

use crate::builtins::json_encode;
use crate::interp::Flow;
use crate::value::Value;

// --- Value/Flow -> hyper::Response ---

// Converts a Fluxon `Int` status (rep/fail) to a valid HTTP status u16. The
// check MUST be on the ORIGINAL i64: an `as u16` cast wraps first — `rep 65736`
// would wrap to 200 in u16, and some negative values would land in 3xx/4xx,
// faking success silently (issue #108). An out-of-range or non-HTTP code -> 500
// + a log, so the client does not read a handler error as success.
pub(crate) fn checked_status(n: i64) -> u16 {
    match u16::try_from(n) {
        Ok(s) if StatusCode::from_u16(s).is_ok() => s,
        _ => {
            eprintln!("Fluxon HTTP: invalid status code {} → 500", n);
            500
        }
    }
}

// u16 status -> StatusCode. A builder-level safety net: callers already pass a
// valid code (a literal or `checked_status`); this only returns 500 instead of
// panicking in an unexpected case.
fn status_or_500(status: u16) -> StatusCode {
    StatusCode::from_u16(status).unwrap_or_else(|_| {
        eprintln!("Fluxon HTTP: invalid status code {} → 500", status);
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

pub(crate) fn json_response(status: u16, body: String) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status_or_500(status))
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}

fn text_response(status: u16, body: String) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status_or_500(status))
        .header("content-type", "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}

// 413 Payload Too Large — the request body exceeded the size limit (#91).
pub(crate) fn payload_too_large(limit: usize) -> Response<Full<Bytes>> {
    let mut m = BTreeMap::new();
    m.insert(
        "error".to_string(),
        Value::Str(format!("request body too large (limit: {} bytes)", limit)),
    );
    json_response(413, json_encode(&Value::Map(m)))
}

// 400 Bad Request — error reading the request body (e.g. a dropped connection) (#91).
pub(crate) fn bad_request(msg: &str) -> Response<Full<Bytes>> {
    let mut m = BTreeMap::new();
    m.insert("error".to_string(), Value::Str(msg.to_string()));
    json_response(400, json_encode(&Value::Map(m)))
}

// Is the value a `rep` response? `rep status body` -> {__resp:true ...} map
// (builtins.rs). If middleware returns this response, the chain stops (P1: rep
// auth rejection).
pub(crate) fn is_resp(v: &Value) -> bool {
    matches!(v, Value::Map(m) if matches!(m.get("__resp"), Some(Value::Bool(true))))
}

// Converts a value the handler returned successfully into a response.
// `rep` -> {__resp:true status body}. Otherwise 200 + the value.
pub(crate) fn value_to_response(v: Value) -> Response<Full<Bytes>> {
    if is_resp(&v)
        && let Value::Map(m) = &v
    {
        let status = match m.get("status") {
            Some(Value::Int(n)) => checked_status(*n),
            _ => 200,
        };
        let body = m.get("body").cloned().unwrap_or(Value::Nil);
        // 3rd-argument custom headers (issue #16): `rep status body {hdr:val}`.
        let custom = m.get("headers");
        // Redirect: `rep 30x {location:url}` -> emit the location from the body
        // map into the Location header (spec: "Redirect: rep 302 {location:url}").
        // A legacy convenience behavior — works even without custom headers, and
        // the body is returned empty.
        if (300..400).contains(&status)
            && let Value::Map(bm) = &body
            && let Some(Value::Str(loc)) = bm.get("location")
        {
            let mut b = Response::builder()
                .status(StatusCode::from_u16(status).unwrap_or(StatusCode::FOUND))
                .header("location", loc.clone());
            b = apply_headers(b, custom);
            return b.body(Full::new(Bytes::new())).unwrap();
        }
        let mut resp = body_value_to_response(status, body);
        apply_headers_mut(resp.headers_mut(), custom);
        return resp;
    }
    // rep was not used — return the value itself with 200.
    body_value_to_response(200, v)
}

// Adds the custom header map to a Response::Builder (for the redirect path —
// it is still at the builder stage). A malformed header name/value is silently
// skipped: a single broken header must not turn the whole response into a 500.
fn apply_headers(
    mut b: hyper::http::response::Builder,
    headers: Option<&Value>,
) -> hyper::http::response::Builder {
    if let Some(Value::Map(hm)) = headers
        && let Some(hmap) = b.headers_mut()
    {
        apply_headers_mut(hmap, Some(&Value::Map(hm.clone())));
    }
    b
}

// Adds the custom header map to a ready Response's HeaderMap.
//
// If the value is a str — a single header. If a List — each element is a
// separate header line (a repeated header, e.g. several Set-Cookie; per RFC 7230
// Set-Cookie does not merge into a comma list). Headers like content-type
// override the body's default header (insert, not append): the developer's
// intent wins over the canonical body header.
//
// In the key `_` -> `-`: a Fluxon map key cannot contain a hyphen
// (`content-type` would parse as three tokens), so you write
// `{content_type:"..."}`. This is symmetric with reading — the server also does
// `-` -> `_` in req.headers (build_req), and the AI learns one pattern. A hyphenated
// string key (`{"set-cookie":...}`) also works: a hyphen has no `_`.
pub(crate) fn apply_headers_mut(hmap: &mut hyper::HeaderMap, headers: Option<&Value>) {
    use hyper::header::{HeaderName, HeaderValue};
    let Some(Value::Map(hm)) = headers else {
        return;
    };
    for (k, v) in hm {
        // Header name is case-insensitive (RFC 7230) — we store the lowercase
        // canonical form. Skip a malformed name silently.
        let canon = k.to_lowercase().replace('_', "-");
        let Ok(name) = HeaderName::from_bytes(canon.as_bytes()) else {
            continue;
        };
        match v {
            // List — a repeated header: the first is insert (overwrites the old
            // one), the rest are append.
            Value::List(items) => {
                let mut first = true;
                for item in items.iter() {
                    if let Ok(hv) = HeaderValue::from_str(&item.to_text()) {
                        if first {
                            hmap.insert(name.clone(), hv);
                            first = false;
                        } else {
                            hmap.append(name.clone(), hv);
                        }
                    }
                }
            }
            // Any other value as text — a single header.
            other => {
                if let Ok(hv) = HeaderValue::from_str(&other.to_text()) {
                    hmap.insert(name, hv);
                }
            }
        }
    }
}

// HeaderMap -> Fluxon header map (lowercase keys). The single read-side path —
// both the server's req.headers and the client's res.headers are built through it.
//
// So that repeated same-name headers are not lost (issue #101), values are
// joined with ", " per RFC 9110 §5.3. Two exceptions:
//   - `cookie` with "; " (RFC 6265 — the cookie-pair separator is not a comma);
//   - `set-cookie` is not merged at all (the Expires date contains a comma) —
//     if repeated it returns a List, symmetric with the write-side List.
// Non-UTF-8 bytes are read lossy (it used to silently become an empty string).
pub(crate) fn headers_to_map(hmap: &hyper::HeaderMap) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for key in hmap.keys() {
        let name = key.as_str().to_lowercase();
        let vals: Vec<String> = hmap
            .get_all(key)
            .iter()
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .collect();
        let value = if name == "set-cookie" && vals.len() > 1 {
            Value::List(vals.into_iter().map(Value::Str).collect())
        } else if name == "cookie" {
            Value::Str(vals.join("; "))
        } else {
            Value::Str(vals.join(", "))
        };
        out.insert(name, value);
    }
    out
}

// Formats the response body by type: map/list -> JSON, str -> text,
// nil -> empty, otherwise -> JSON.
fn body_value_to_response(status: u16, body: Value) -> Response<Full<Bytes>> {
    match body {
        Value::Nil => Response::builder()
            .status(status_or_500(status))
            .body(Full::new(Bytes::new()))
            .unwrap(),
        Value::Str(s) => text_response(status, s),
        // Binary response (image, PDF, archive — issue #132). Default type is
        // octet-stream; an explicit type via the 3rd arg: rep 200 b {content_type:"image/png"}.
        Value::Bytes(b) => Response::builder()
            .status(status_or_500(status))
            .header("content-type", "application/octet-stream")
            .body(Full::new(Bytes::from(b.as_ref().clone())))
            .unwrap(),
        Value::Map(_) | Value::List(_) => json_response(status, json_encode(&body)),
        other => text_response(status, format!("{}", other)),
    }
}

// fail/error -> JSON error response.
pub(crate) fn flow_to_response(flow: Flow) -> Response<Full<Bytes>> {
    let (status, message) = match flow {
        Flow::Fail { status, message } => (checked_status(status.unwrap_or(400)), message),
        Flow::Error(e) => (500, e),
        Flow::Return(v) => return value_to_response(v), // `ret` inside the handler
        Flow::Skip | Flow::Stop => (500, "handler used skip/stop".to_string()),
    };
    let mut m = BTreeMap::new();
    m.insert("error".to_string(), Value::Str(message));
    json_response(status, json_encode(&Value::Map(m)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http_mod::request::build_req;
    use std::sync::Arc;

    // --- headers_to_map: repeated headers on the read side (issue #101) ---

    // Gets a str value from a map (Value is not Debug/PartialEq — match by pattern).
    fn hstr(m: &BTreeMap<String, Value>, k: &str) -> String {
        match m.get(k) {
            Some(Value::Str(s)) => s.clone(),
            _ => panic!("{k}: Str value expected"),
        }
    }

    #[test]
    fn headers_takror_nom_vergul_bilan_birlashadi() {
        // Two headers with the same name (e.g. an X-Forwarded-For chain) must not
        // be lost — per RFC 9110 §5.3 they merge into one value with ", ".
        let mut h = hyper::HeaderMap::new();
        h.append("x-forwarded-for", "1.1.1.1".parse().unwrap());
        h.append("x-forwarded-for", "2.2.2.2".parse().unwrap());
        let m = headers_to_map(&h);
        assert_eq!(hstr(&m, "x-forwarded-for"), "1.1.1.1, 2.2.2.2");
    }

    // bytes response — raw bytes + the application/octet-stream default type.
    #[test]
    fn bytes_javob_octet_stream() {
        let resp = body_value_to_response(200, Value::Bytes(Arc::new(vec![1, 2, 3])));
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/octet-stream"
        );
    }

    #[test]
    fn headers_bitta_qiymat_oddiy_str() {
        let mut h = hyper::HeaderMap::new();
        h.insert("content-type", "application/json".parse().unwrap());
        let m = headers_to_map(&h);
        assert_eq!(hstr(&m, "content-type"), "application/json");
    }

    #[test]
    fn headers_cookie_nuqta_vergul_bilan_birlashadi() {
        // The cookie-pair separator is "; " (RFC 6265) — merging with a comma
        // would corrupt the cookie value.
        let mut h = hyper::HeaderMap::new();
        h.append("cookie", "a=1".parse().unwrap());
        h.append("cookie", "b=2".parse().unwrap());
        let m = headers_to_map(&h);
        assert_eq!(hstr(&m, "cookie"), "a=1; b=2");
    }

    #[test]
    fn headers_takror_set_cookie_list_qaytadi() {
        // Set-Cookie cannot be merged (the Expires date has a comma) — if
        // repeated it is a List, symmetric with the write-side List.
        let mut h = hyper::HeaderMap::new();
        h.append("set-cookie", "a=1".parse().unwrap());
        h.append("set-cookie", "b=2".parse().unwrap());
        let m = headers_to_map(&h);
        match m.get("set-cookie") {
            Some(Value::List(items)) => {
                let got: Vec<String> = items.iter().map(|v| v.to_text()).collect();
                assert_eq!(got, vec!["a=1", "b=2"]);
            }
            _ => panic!("set-cookie: List expected"),
        }
    }

    #[test]
    fn headers_bitta_set_cookie_str_qoladi() {
        // The simple case (a single cookie) must not change — old code expects str.
        let mut h = hyper::HeaderMap::new();
        h.insert("set-cookie", "s=xyz".parse().unwrap());
        let m = headers_to_map(&h);
        assert_eq!(hstr(&m, "set-cookie"), "s=xyz");
    }

    #[test]
    fn headers_utf8_bolmagan_qiymat_lossy_oqiladi() {
        // unwrap_or("") used to silently return an empty string — now lossy: a
        // broken byte becomes U+FFFD, the rest is preserved.
        let mut h = hyper::HeaderMap::new();
        h.insert(
            "x-raw",
            hyper::header::HeaderValue::from_bytes(b"ok\xffend").unwrap(),
        );
        let m = headers_to_map(&h);
        assert_eq!(hstr(&m, "x-raw"), "ok\u{fffd}end");
    }

    // Helper that extracts the body Value from build_req.
    fn body_of(bytes: &str, is_json: bool) -> Value {
        let v = build_req(
            "POST".into(),
            "/".into(),
            String::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            "127.0.0.1".into(),
            Bytes::from(bytes.to_string()),
            is_json,
            None,
        );
        match v {
            Value::Map(m) => m.get("body").cloned().unwrap(),
            _ => panic!("build_req must return a Map"),
        }
    }

    #[test]
    fn bosh_tana_nil() {
        assert!(matches!(body_of("", false), Value::Nil));
    }

    #[test]
    fn content_type_json_parse_qiladi() {
        // When Content-Type is JSON (old behavior preserved).
        assert!(matches!(body_of(r#"{"a":1}"#, true), Value::Map(_)));
    }

    #[test]
    fn content_type_yoq_lekin_obyekt_korinishida_parse_qiladi() {
        // The main fix: even when Content-Type is not JSON, parse if it starts with `{`.
        assert!(matches!(body_of(r#"{"a":1}"#, false), Value::Map(_)));
    }

    #[test]
    fn content_type_yoq_lekin_royxat_korinishida_parse_qiladi() {
        // A body starting with `[` is also tried as JSON.
        assert!(matches!(body_of("[1,2,3]", false), Value::List(_)));
    }

    #[test]
    fn boshidagi_boshliq_belgi_eotiborga_olinadi() {
        // `{` is detected even with leading whitespace.
        assert!(matches!(body_of("  \n {\"a\":1}", false), Value::Map(_)));
    }

    #[test]
    fn oddiy_matn_string_boladi() {
        // A body that does not look like JSON stays a string.
        assert!(matches!(body_of("hello=world", false), Value::Str(_)));
    }

    #[test]
    fn buzilgan_json_xom_matn_qoladi() {
        // Starts with `{` but invalid JSON — stays as a string.
        assert!(matches!(body_of("{buzuq", false), Value::Str(_)));
    }

    #[test]
    fn is_resp_rep_javobni_taniydi() {
        // rep -> {__resp:true ...}. If middleware returns this response, the chain stops.
        let mut m = BTreeMap::new();
        m.insert("__resp".to_string(), Value::Bool(true));
        m.insert("status".to_string(), Value::Int(401));
        assert!(is_resp(&Value::Map(m)));
        // A plain map or nil — not a response (middleware continues).
        assert!(!is_resp(&Value::Map(BTreeMap::new())));
        assert!(!is_resp(&Value::Nil));
    }

    // --- custom headers (issue #16) ---

    // A __resp map mimicking the result of `rep status body {headers}`.
    fn resp_map(status: i64, body: Value, headers: Option<Value>) -> Value {
        let mut m = BTreeMap::new();
        m.insert("__resp".to_string(), Value::Bool(true));
        m.insert("status".to_string(), Value::Int(status));
        m.insert("body".to_string(), body);
        if let Some(h) = headers {
            m.insert("headers".to_string(), h);
        }
        Value::Map(m)
    }

    fn hmap(pairs: &[(&str, Value)]) -> Value {
        let mut m = BTreeMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        Value::Map(m)
    }

    #[test]
    fn custom_content_type_body_standartini_bosadi() {
        // A str body gives the default "text/plain"; a custom content-type overrides it.
        let r = value_to_response(resp_map(
            200,
            Value::Str("<h1>Hello</h1>".into()),
            Some(hmap(&[("content-type", Value::Str("text/html".into()))])),
        ));
        assert_eq!(r.headers().get("content-type").unwrap(), "text/html");
    }

    #[test]
    fn custom_header_nomi_lowercase_kanonik() {
        // Even if Content-Type (uppercase) is given, it is stored lowercase
        // (RFC 7230 — header name is case-insensitive).
        let r = value_to_response(resp_map(
            200,
            Value::Nil,
            Some(hmap(&[("X-Request-Id", Value::Str("abc".into()))])),
        ));
        assert_eq!(r.headers().get("x-request-id").unwrap(), "abc");
    }

    #[test]
    fn set_cookie_list_takror_sarlavha() {
        // A List value -> each element is a separate Set-Cookie line (RFC 7230:
        // Set-Cookie does not merge into a comma list).
        let cookies = Value::List(vec![Value::Str("a=1".into()), Value::Str("b=2".into())]);
        let r = value_to_response(resp_map(
            200,
            Value::Nil,
            Some(hmap(&[("set-cookie", cookies)])),
        ));
        let got: Vec<_> = r.headers().get_all("set-cookie").iter().collect();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], "a=1");
        assert_eq!(got[1], "b=2");
    }

    #[test]
    fn redirect_location_plus_custom_header() {
        // The legacy `rep 302 {location:url}` behavior + a custom header work
        // together (e.g. setting Set-Cookie alongside a redirect).
        let r = value_to_response(resp_map(
            302,
            hmap(&[("location", Value::Str("/dest".into()))]),
            Some(hmap(&[("set-cookie", Value::Str("s=xyz".into()))])),
        ));
        assert_eq!(r.status().as_u16(), 302);
        assert_eq!(r.headers().get("location").unwrap(), "/dest");
        assert_eq!(r.headers().get("set-cookie").unwrap(), "s=xyz");
    }

    #[test]
    fn notogri_status_500_ga_tushadi() {
        // `rep 1000 ...` — an invalid HTTP status. Must not silently fall to 200
        // (issue #108): when the handler returns a bad status the client must not
        // see success. 1000 is out of the HTTP range -> 500.
        let r = value_to_response(resp_map(1000, Value::Str("error".into()), None));
        assert_eq!(r.status().as_u16(), 500);
    }

    #[test]
    fn manfiy_status_500_ga_tushadi() {
        // A negative status — invalid, falls to 500 (issue #108), not 200.
        let r = value_to_response(resp_map(-1, Value::Nil, None));
        assert_eq!(r.status().as_u16(), 500);
    }

    #[test]
    fn u16_wrap_status_500_ga_tushadi() {
        // Code-review (PR #110): if the check is not on the ORIGINAL i64,
        // `65736 as u16` wraps to 200 and fakes success silently. Now
        // `checked_status` checks the range before the u16 cast -> 500.
        let r = value_to_response(resp_map(65736, Value::Str("ok".into()), None));
        assert_eq!(r.status().as_u16(), 500);
        // A negative value that wraps into the 3xx range too (-65234 -> 302) -> 500.
        let r2 = value_to_response(resp_map(-65234, Value::Nil, None));
        assert_eq!(r2.status().as_u16(), 500);
    }

    #[test]
    fn yaroqli_status_saqlanadi() {
        // A valid status (404) is not changed — the fix only touches a broken status.
        let r = value_to_response(resp_map(404, Value::Str("not found".into()), None));
        assert_eq!(r.status().as_u16(), 404);
    }

    #[test]
    fn buzuq_header_jim_otkaziladi() {
        // An invalid header value (a newline) does not break the whole response —
        // it is silently skipped, the rest of the headers are set.
        let r = value_to_response(resp_map(
            200,
            Value::Nil,
            Some(hmap(&[
                ("x-bad", Value::Str("bad\nvalue".into())),
                ("x-good", Value::Str("good".into())),
            ])),
        ));
        assert!(r.headers().get("x-bad").is_none());
        assert_eq!(r.headers().get("x-good").unwrap(), "good");
    }
}
