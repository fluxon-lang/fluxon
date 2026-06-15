// Middleware chain (http.use/before/limit) and CORS (issue #135): the kinds,
// the config, header application, and the synchronous chain runner.

use crate::interp::{Flow, Interp};
use crate::value::Value;

use super::limits::{LimitState, check_and_count, client_fallback_key, rate_limited_response};
use super::response::is_resp;

// Middleware kind: a plain fn (use/before) or a rate-limiter (http.limit). Limit
// is added to THIS SAME list (not separately) — so it runs in DECLARATION ORDER
// relative to other middleware: if an auth declared before it writes tenant_id
// into `req.ctx`, the key function `\req -> req.ctx.tenant_id` sees it (#79).
#[derive(Clone)]
pub enum MwKind {
    // http.use / http.before — calls the handler; `fail`/`rep` stops the chain.
    Fn,
    // http.limit — the handler is the KEY function (req -> key). On exceeding the limit, 429.
    Limit {
        limit: u32,
        window_secs: u64,
        state: LimitState,
    },
}

// Middleware (issue #67). `scope` = None — global (`http.use`, applies to every
// path); Some(pattern) — by prefix (`http.before "/api/*"`). Stored in the list
// in declaration order (the order is well-defined even when use/before/limit are mixed).
#[derive(Clone)]
pub struct Middleware {
    pub scope: Option<String>,
    pub handler: Value,
    pub kind: MwKind,
}

// CORS config (issue #135). Filled in by `http.cors`; when enabled, OPTIONS
// preflight is answered automatically and every response gets
// `Access-Control-Allow-*` headers.
//
//   http.cors "*"                                   # open to all (dev)
//   http.cors ["https://app.example.com"]           # allowed origins
//   http.cors ["https://app.example.com"] {creds: true}   # cookie/Authorization
//
// `origins`: None — any origin ("*"). Some(set) — only the listed ones.
// Wildcard "*" and `creds: true` cannot be combined (the browser rejects it),
// so when creds is on, the response reflects the request's exact Origin.
#[derive(Clone)]
pub struct CorsConfig {
    // Allowed origins. None — "*" (any). Some — an explicit list.
    pub(crate) origins: Option<Vec<String>>,
    // Allowed methods (Access-Control-Allow-Methods). A wide default set.
    pub(crate) methods: String,
    // Allowed request headers (Access-Control-Allow-Headers).
    pub(crate) headers: String,
    // Allow sharing cookies/Authorization (credentials) (Allow-Credentials).
    pub(crate) creds: bool,
    // How many seconds the browser caches the preflight response (Max-Age).
    pub(crate) max_age: u64,
}

// Is the request's origin allowed? If so, the response's
// `Access-Control-Allow-Origin` value is returned (the exact origin or "*").
impl CorsConfig {
    // Computes the Allow-Origin value based on the request's Origin header.
    // None means this origin is not allowed (no CORS header is added).
    fn allow_origin_for(&self, req_origin: Option<&str>) -> Option<String> {
        match &self.origins {
            // Any origin allowed. With creds=true "*" cannot be used —
            // we reflect the request origin (falling back to "*").
            None => {
                if self.creds {
                    req_origin.map(|o| o.to_string())
                } else {
                    Some("*".to_string())
                }
            }
            // Explicit list — if the request origin is in it, reflect that.
            Some(list) => match req_origin {
                Some(o) if list.iter().any(|a| a == o) => Some(o.to_string()),
                _ => None,
            },
        }
    }

    // Adds CORS headers to the response's HeaderMap (including for plain
    // non-preflight responses). If the origin is not allowed, nothing is added.
    pub(crate) fn apply_to(&self, hmap: &mut hyper::HeaderMap, req_origin: Option<&str>) {
        let Some(allow) = self.allow_origin_for(req_origin) else {
            return;
        };
        set_header(hmap, "access-control-allow-origin", &allow);
        // Allow-Origin varies by request origin — to keep caches correct we add
        // Origin to Vary (otherwise a proxy would serve one origin's response to
        // another). NOT insert — we preserve a Vary the handler set with
        // `rep ... {vary:"Accept-Encoding"}` and MERGE Origin into it (codex P2:
        // insert clobbered the existing cache key). "*" — the response does not
        // depend on origin, so Vary is unnecessary.
        if allow != "*" {
            add_vary_origin(hmap);
        }
        if self.creds {
            set_header(hmap, "access-control-allow-credentials", "true");
        }
    }
}

// Adds `Origin` to the response's `Vary` header, preserving existing values.
// If `Vary` already contains `Origin` (or `*`) it is left unchanged (avoids
// duplication). Otherwise it joins with a comma.
fn add_vary_origin(hmap: &mut hyper::HeaderMap) {
    use hyper::header::{HeaderValue, VARY};
    // Read the existing Vary values (there may be several Vary lines).
    let existing: Vec<String> = hmap
        .get_all(VARY)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .collect();
    // Already has Origin or * — nothing to add, return.
    let already = existing.iter().any(|line| {
        line.split(',').any(|tok| {
            let t = tok.trim();
            t.eq_ignore_ascii_case("origin") || t == "*"
        })
    });
    if already {
        return;
    }
    if existing.is_empty() {
        hmap.insert(VARY, HeaderValue::from_static("Origin"));
    } else {
        // Merge the existing value(s) into one line and append Origin.
        let merged = format!("{}, Origin", existing.join(", "));
        if let Ok(hv) = HeaderValue::from_str(&merged) {
            hmap.insert(VARY, hv);
        }
    }
}

// Adds CORS headers to the response (when enabled) and returns it. Body-read
// error responses (400/413), 404, and the handler response all finalize through
// here, so when CORS is enabled EVERY response gets `Access-Control-Allow-*`
// (codex P2: early-return errors used to return without headers).
pub(crate) fn cors_finalize(
    mut resp: hyper::Response<http_body_util::Full<bytes::Bytes>>,
    cors: &Option<CorsConfig>,
    req_origin: Option<&str>,
) -> hyper::Response<http_body_util::Full<bytes::Bytes>> {
    if let Some(cfg) = cors {
        cfg.apply_to(resp.headers_mut(), req_origin);
    }
    resp
}

// Inserts a single header into the HeaderMap (overwriting the old one). A
// malformed name/value is silently skipped. Helper for CORS headers (sidesteps
// the closure borrow problem).
fn set_header(hmap: &mut hyper::HeaderMap, name: &str, val: &str) {
    use hyper::header::{HeaderName, HeaderValue};
    if let (Ok(n), Ok(v)) = (
        HeaderName::from_bytes(name.as_bytes()),
        HeaderValue::from_str(val),
    ) {
        hmap.insert(n, v);
    }
}

// Runs the middleware chain (synchronous — called inside spawn_blocking).
// Each middleware gets a req clone (the ctx Arc is shared). Result:
//   - Ok(Some(v)) — one returned a response (`rep` or limit 429), the chain
//     stopped, the handler is NOT called; otherwise an auth `rep 401` would be
//     ignored.
//   - Ok(None)   — all passed (ctx writes, logging), the handler continues.
//   - Err(flow)  — `fail`/error, the chain stopped.
// Both route handlers and static files (issue #134) go through THIS chain — so
// http.before auth protects the static folder too.
pub(crate) fn run_middleware_chain(
    interp: &Interp,
    chain: Vec<Middleware>,
    request_value: &Value,
) -> Result<Option<Value>, Flow> {
    for mw in chain {
        match mw.kind {
            // Plain middleware (use/before): call the handler.
            MwKind::Fn => match interp.apply(mw.handler, vec![request_value.clone()]) {
                Ok(v) if is_resp(&v) => return Ok(Some(v)), // rep -> response, chain stops
                Ok(_) => {}                                 // continue (ctx/log)
                Err(flow) => return Err(flow),              // fail/error -> stop
            },
            // Rate-limit (http.limit): call the key function to identify the
            // client, then check the counter. If exceeded, 429 -> chain stops.
            MwKind::Limit {
                limit,
                window_secs,
                state,
            } => {
                let key = match interp.apply(mw.handler, vec![request_value.clone()]) {
                    // nil -> fall back to the client IP (limit keyless requests too).
                    Ok(Value::Nil) => client_fallback_key(request_value),
                    Ok(v) => {
                        let t = v.to_text();
                        if t.is_empty() {
                            client_fallback_key(request_value)
                        } else {
                            t
                        }
                    }
                    Err(flow) => return Err(flow), // the key fn errored -> stop
                };
                if let Some(retry) = check_and_count(&state, &key, limit, window_secs) {
                    return Ok(Some(rate_limited_response(retry)));
                }
            }
        }
    }
    Ok(None)
}

// OPTIONS preflight response (issue #135). The browser sends OPTIONS before the
// real request and expects `Access-Control-Allow-*` headers. No body
// (204 No Content). If the origin is not allowed, 204 is returned without CORS
// headers (the browser blocks the request — correct behavior).
pub(crate) fn cors_preflight_response(
    cfg: &CorsConfig,
    req_origin: Option<&str>,
) -> hyper::Response<http_body_util::Full<bytes::Bytes>> {
    use bytes::Bytes;
    use http_body_util::Full;
    use hyper::{Response, StatusCode};
    let mut b = Response::builder().status(StatusCode::NO_CONTENT);
    if let Some(hmap) = b.headers_mut() {
        cfg.apply_to(hmap, req_origin);
        // Preflight-specific headers (not needed in a plain response): allowed
        // methods, headers, and cache duration. We add them only when the origin
        // is allowed — if apply_to set no Allow-Origin, we leave the preflight
        // empty too (the browser rejects it).
        if hmap.contains_key("access-control-allow-origin") {
            set_header(hmap, "access-control-allow-methods", &cfg.methods);
            set_header(hmap, "access-control-allow-headers", &cfg.headers);
            set_header(hmap, "access-control-max-age", &cfg.max_age.to_string());
        }
    }
    b.body(Full::new(Bytes::new())).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::StatusCode;

    // --- CORS (issue #135) ---

    // A config with default settings — tests change only the field they need.
    fn cors_cfg(origins: Option<Vec<String>>, creds: bool) -> CorsConfig {
        CorsConfig {
            origins,
            methods: "GET, POST, OPTIONS".into(),
            headers: "Content-Type".into(),
            creds,
            max_age: 600,
        }
    }

    // Gets a str value from a HeaderMap (None if absent).
    fn hv(h: &hyper::HeaderMap, name: &str) -> Option<String> {
        h.get(name).map(|v| v.to_str().unwrap().to_string())
    }

    #[test]
    fn cors_wildcard_har_origin_uchun_star() {
        // `http.cors "*"` — any origin gets "*" (no creds).
        let cfg = cors_cfg(None, false);
        let mut h = hyper::HeaderMap::new();
        cfg.apply_to(&mut h, Some("https://a.example.com"));
        assert_eq!(hv(&h, "access-control-allow-origin").as_deref(), Some("*"));
        // With "*" no Vary: Origin is added (the response does not depend on origin).
        assert_eq!(hv(&h, "vary"), None);
    }

    #[test]
    fn cors_wildcard_creds_origin_aks_ettiradi() {
        // `http.cors "*" {creds: true}` — the browser rejects "*" + credentials,
        // so we reflect the request origin + Allow-Credentials.
        let cfg = cors_cfg(None, true);
        let mut h = hyper::HeaderMap::new();
        cfg.apply_to(&mut h, Some("https://a.example.com"));
        assert_eq!(
            hv(&h, "access-control-allow-origin").as_deref(),
            Some("https://a.example.com")
        );
        assert_eq!(
            hv(&h, "access-control-allow-credentials").as_deref(),
            Some("true")
        );
        // When a specific origin is reflected, Vary: Origin is required (cache correctness).
        assert_eq!(hv(&h, "vary").as_deref(), Some("Origin"));
    }

    #[test]
    fn cors_royxat_faqat_ruxsat_etilgan_origin() {
        // An explicit list — an allowed origin is reflected.
        let cfg = cors_cfg(Some(vec!["https://app.example.com".into()]), false);
        let mut h = hyper::HeaderMap::new();
        cfg.apply_to(&mut h, Some("https://app.example.com"));
        assert_eq!(
            hv(&h, "access-control-allow-origin").as_deref(),
            Some("https://app.example.com")
        );
        assert_eq!(hv(&h, "vary").as_deref(), Some("Origin"));
    }

    #[test]
    fn cors_vary_mavjud_qiymatni_saqlaydi() {
        // Without erasing a Vary the handler set with `rep ... {vary:"Accept-Encoding"}`,
        // we merge in Origin (codex P2: insert clobbered the cache key).
        let cfg = cors_cfg(Some(vec!["https://app.example.com".into()]), false);
        let mut h = hyper::HeaderMap::new();
        h.insert(hyper::header::VARY, "Accept-Encoding".parse().unwrap());
        cfg.apply_to(&mut h, Some("https://app.example.com"));
        assert_eq!(hv(&h, "vary").as_deref(), Some("Accept-Encoding, Origin"));
    }

    #[test]
    fn cors_vary_takror_origin_qoshmaydi() {
        // If Vary already has Origin — it is not added again (case-insensitive).
        let cfg = cors_cfg(Some(vec!["https://app.example.com".into()]), false);
        let mut h = hyper::HeaderMap::new();
        h.insert(hyper::header::VARY, "origin".parse().unwrap());
        cfg.apply_to(&mut h, Some("https://app.example.com"));
        // The existing "origin" is preserved, not added a second time.
        assert_eq!(hv(&h, "vary").as_deref(), Some("origin"));
    }

    #[test]
    fn cors_royxat_tashqi_origin_rad() {
        // An origin not in the list — no CORS header is added
        // (the browser blocks the request).
        let cfg = cors_cfg(Some(vec!["https://app.example.com".into()]), false);
        let mut h = hyper::HeaderMap::new();
        cfg.apply_to(&mut h, Some("https://evil.example.com"));
        assert_eq!(hv(&h, "access-control-allow-origin"), None);
    }

    #[test]
    fn cors_origin_yoq_royxat_bilan_header_qoshilmaydi() {
        // A request without an Origin header (e.g. curl) — no match in the
        // explicit list, no CORS header is added.
        let cfg = cors_cfg(Some(vec!["https://app.example.com".into()]), false);
        let mut h = hyper::HeaderMap::new();
        cfg.apply_to(&mut h, None);
        assert_eq!(hv(&h, "access-control-allow-origin"), None);
    }

    #[test]
    fn cors_preflight_metod_va_header_qaytaradi() {
        // OPTIONS preflight returns 204 + Allow-Methods/Headers/Max-Age.
        let cfg = cors_cfg(None, false);
        let resp = cors_preflight_response(&cfg, Some("https://a.example.com"));
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let h = resp.headers();
        assert_eq!(hv(h, "access-control-allow-origin").as_deref(), Some("*"));
        assert_eq!(
            hv(h, "access-control-allow-methods").as_deref(),
            Some("GET, POST, OPTIONS")
        );
        assert_eq!(
            hv(h, "access-control-allow-headers").as_deref(),
            Some("Content-Type")
        );
        assert_eq!(hv(h, "access-control-max-age").as_deref(), Some("600"));
    }

    #[test]
    fn cors_preflight_rad_etilgan_origin_header_qoshmaydi() {
        // For a disallowed origin, preflight returns 204 but without CORS
        // headers — the browser blocks the request (correct behavior).
        let cfg = cors_cfg(Some(vec!["https://app.example.com".into()]), false);
        let resp = cors_preflight_response(&cfg, Some("https://evil.example.com"));
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let h = resp.headers();
        assert_eq!(hv(h, "access-control-allow-origin"), None);
        assert_eq!(hv(h, "access-control-allow-methods"), None);
    }
}
