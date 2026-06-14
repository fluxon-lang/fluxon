// Fluxon HTTP battery — server (http.on/http.serve/rep) and client (http.get/post).
//
// The server is built on tokio + hyper. Because Fluxon handlers are synchronous
// tree-walking, each request runs inside `spawn_blocking` — this makes the CPU
// work TRULY PARALLEL without blocking tokio workers (Value: Send+Sync, the
// thread-safety refactor guarantees this).
//
// `rep status body` -> {__resp:true status body} map (builtins.rs::install).
// `fail status "msg"` -> Flow::Fail -> JSON error response.

use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use tokio::net::TcpListener;

use crate::builtins::{json_decode, json_encode};
use crate::interp::{Flow, Interp};
use crate::value::Value;

// --- route structure ---

// Path segment: literal (`notes`) or parameter (`:id`).
#[derive(Clone)]
pub enum Seg {
    Lit(String),
    Param(String),
}

#[derive(Clone)]
pub struct Route {
    pub method: String, // lowercase: "get", "post", ...
    pub pattern: Vec<Seg>,
    pub handler: Value, // Value::Fn (closure)
}

// Rate-limit state: key -> (window_id, count). Fixed-window — `window_id =
// now_sec / window_sec`. Arc<Mutex> so the limiter is created once at
// REGISTRATION time, and every request shares this SINGLE state (cloning a
// Middleware copies the Arc — same pointer), which is why parallel requests count
// atomically (issue #79: thread-safe). State is in-memory — for a single instance (docs).
//
// Memory bound: if the key function is based on a user-controlled value
// (`req.headers.x_api_key`), every new value lands in the HashMap. On a public
// endpoint a client can grow the state without bound by sending a new key on
// every request. To prevent this, `LimitBucket` sweeps OLD-WINDOW keys once every
// `SWEEP_EVERY` operations (amortized O(1): the cleanup loop runs rarely). An
// old-window key would restart from count=0 on the next request anyway — so
// removing it is safe.
//
// pub: `pub enum MwKind` (via Middleware) exposes the LimitState type.
pub struct LimitBucket {
    counts: HashMap<String, (u64, u32)>,
    // Number of operations since the last cleanup (amortizes the sweep).
    ops: u32,
}

impl LimitBucket {
    fn new() -> Self {
        LimitBucket {
            counts: HashMap::new(),
            ops: 0,
        }
    }
}

// How often (in operations) we sweep old-window keys.
const SWEEP_EVERY: u32 = 1024;

// Default request body size limit for the HTTP server (issue #91). An unbounded
// `collect()` gathers the entire body into memory — a client can fill server
// memory by sending a huge body (DoS). Default 10 MiB; configured via `http.serve
// PORT {max_body: N}`. `max_body: 0` disables the limit (unbounded — use only
// behind a trusted, internal network).
const DEFAULT_MAX_BODY: usize = 10 * 1024 * 1024;

// Default timeout for the HTTP client (http.get/post/...) (issue #92). Without a
// timeout, a stuck upstream blocks the whole script FOREVER (or, if called inside
// a handler, that request thread). Default 30s; configured via `http.get url
// {timeout: N}` (seconds); `timeout: 0` — no timeout (only for trusted upstreams).
// The timeout covers the whole request: connect + send + response (including
// redirects) — even if it hangs at some stage, an error is returned once time runs out.
const DEFAULT_CLIENT_TIMEOUT_SECS: u64 = 30;

// HTTP server header-read timeout (issue #92). A slowloris-style connection may
// send headers very slowly (or not at all), holding the socket/task indefinitely.
// If hyper does not receive the headers fully within this period it closes the
// connection. (header_read_timeout only takes effect when Builder::timer is set.)
const DEFAULT_HEADER_READ_TIMEOUT_SECS: u64 = 30;

pub type LimitState = Arc<Mutex<LimitBucket>>;

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
    origins: Option<Vec<String>>,
    // Allowed methods (Access-Control-Allow-Methods). A wide default set.
    methods: String,
    // Allowed request headers (Access-Control-Allow-Headers).
    headers: String,
    // Allow sharing cookies/Authorization (credentials) (Allow-Credentials).
    creds: bool,
    // How many seconds the browser caches the preflight response (Max-Age).
    max_age: u64,
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
    fn apply_to(&self, hmap: &mut hyper::HeaderMap, req_origin: Option<&str>) {
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
fn cors_finalize(
    mut resp: Response<Full<Bytes>>,
    cors: &Option<CorsConfig>,
    req_origin: Option<&str>,
) -> Response<Full<Bytes>> {
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

// "/notes/:id" -> [Lit("notes"), Param("id")]. Empty segments are dropped.
fn parse_pattern(path: &str) -> Vec<Seg> {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(|s| {
            if let Some(name) = s.strip_prefix(':') {
                Seg::Param(name.to_string())
            } else {
                Seg::Lit(s.to_string())
            }
        })
        .collect()
}

// Splits the request path into segments (without the query).
fn path_segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

// Finds the first route matching method+path; on a match returns a params map.
fn match_route(
    routes: &[Route],
    method: &str,
    path: &str,
) -> Option<(Route, BTreeMap<String, Value>)> {
    let segs = path_segments(path);
    for r in routes {
        if r.method != method {
            continue;
        }
        if r.pattern.len() != segs.len() {
            continue;
        }
        let mut params = BTreeMap::new();
        let mut ok = true;
        for (pat, seg) in r.pattern.iter().zip(&segs) {
            match pat {
                Seg::Lit(lit) => {
                    if lit != seg {
                        ok = false;
                        break;
                    }
                }
                Seg::Param(name) => {
                    // Path segments also percent-encode non-ASCII (e.g.
                    // `/users/:name` -> `%D0%9A...`) — we decode it (issue #100).
                    // In a path `+` is literal, so it is not turned into a space
                    // (the form-encoding rule applies only in the query).
                    // `keep_path_seps=true`: `%2F`/`%5C` stay raw (segment
                    // invariant — no `/` inside the value, codex review).
                    params.insert(name.clone(), Value::Str(percent_decode(seg, true)));
                }
            }
        }
        if ok {
            return Some((r.clone(), params));
        }
    }
    None
}

// Does an http.before pattern match the request path? (issue #67)
// "/api/*" -> paths that are "/api" or start with "/api/..." (segment boundary).
// A pattern without "*" -> exact match. "/apix" does NOT match "/api/*"
// (the prefix is split on a segment boundary).
fn prefix_matches(pat: &str, path: &str) -> bool {
    if let Some(prefix) = pat.strip_suffix("/*") {
        // "/api/*" → "/api" itself or anything starting with "/api/".
        path == prefix || path.starts_with(&format!("{}/", prefix))
    } else {
        // No pattern — exact path match.
        pat == path
    }
}

// --- static file mount (issue #134) ---

// An `http.static prefix dir` mount. The prefix is stored split into segments
// ("/assets" -> ["assets"], "/" -> []) — matching is checked on a segment
// boundary (so "/assetsx" does not fall under the "/assets" mount). `dir` is an
// absolute path canonicalized at registration time (resolved relative to the
// script directory).
#[derive(Clone)]
pub struct StaticMount {
    pub prefix: Vec<String>,
    pub dir: PathBuf,
    // SPA fallback: if a path under the prefix matches no file, `dir/index.html`
    // is returned (the frontend router handles it itself).
    pub spa: bool,
}

// "/assets/img" -> ["assets", "img"]; "/" -> []. Empty segments are dropped.
fn parse_static_prefix(prefix: &str) -> Vec<String> {
    prefix
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

// Does the mount prefix match the start of the request segments? If so, returns
// the part after the prefix (the file path).
fn strip_mount_prefix<'a>(prefix: &[String], segs: &'a [String]) -> Option<&'a [String]> {
    if segs.len() < prefix.len() {
        return None;
    }
    if prefix.iter().zip(segs).all(|(a, b)| a == b) {
        Some(&segs[prefix.len()..])
    } else {
        None
    }
}

// Joins segments onto the mount directory with MANDATORY traversal protection.
// Each segment is checked AFTER percent-decoding (so `%2e%2e` is caught too):
// it must be a plain name (Component::Normal) — `..`, `.`, empty, absolute, or
// Windows-prefix (`C:`) segments are rejected. An extra `\`/NUL check: such a
// name is unexpected on the filesystem anyway, so a silent 404.
fn safe_join(dir: &Path, rest: &[String]) -> Option<PathBuf> {
    let mut p = dir.to_path_buf();
    for seg in rest {
        if seg.contains('\\') || seg.contains('\0') {
            return None;
        }
        let mut comps = Path::new(seg).components();
        match (comps.next(), comps.next()) {
            (Some(Component::Normal(_)), None) => {}
            _ => return None,
        }
        p.push(seg);
    }
    Some(p)
}

// Content-Type from the extension (issue requirement: automatic). An extension
// not in the list -> octet-stream (the browser downloads it, but the content is
// not corrupted).
fn mime_for(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("json") | Some("map") => "application/json",
        Some("txt") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml",
        Some("csv") => "text/csv; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("wasm") => "application/wasm",
        Some("pdf") => "application/pdf",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mp3") => "audio/mpeg",
        Some("gz") => "application/gzip",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

// Canonicalizes the candidate and confirms it is a plain file UNDER the mount
// root (codex P2): safe_join only checks lexical segments, while metadata
// follows symlinks — if a symlink inside the folder points to a file outside
// the root (e.g. /etc/passwd), the lexical protection would be bypassed. `root`
// is canonicalized at registration, so the prefix comparison works correctly.
// A symlink inside the root (whose canonical target is also under the root) is
// served as before. The returned path is canonical — the subsequent read also
// gets exactly the file that was checked.
async fn confined_file(p: &Path, root: &Path) -> Option<(PathBuf, u64)> {
    let canon = tokio::fs::canonicalize(p).await.ok()?;
    if !canon.starts_with(root) {
        return None;
    }
    let md = tokio::fs::metadata(&canon).await.ok()?;
    if md.is_file() {
        let len = md.len();
        Some((canon, len))
    } else {
        None
    }
}

// Resolves the request segments (already percent-decoded — the caller prepares
// them) to a file across the mounts. A longer prefix wins (the "/assets" mount
// is checked before the "/" mount) — the most specific mount takes it. Two
// stages: (1) the exact file (if a directory is requested, its index.html);
// (2) if not found — the `index.html` fallback of SPA mounts whose prefix
// matches. The size (bytes) is returned together from metadata — so a HEAD
// response can give Content-Length without reading the file (codex P2). Each
// candidate is confined to the root via confined_file.
async fn resolve_static(
    mounts: &[StaticMount],
    segs: &[String],
) -> Option<(PathBuf, &'static str, u64)> {
    let mut order: Vec<&StaticMount> = mounts.iter().collect();
    order.sort_by_key(|m| std::cmp::Reverse(m.prefix.len()));

    for m in &order {
        let Some(rest) = strip_mount_prefix(&m.prefix, segs) else {
            continue;
        };
        let Some(p) = safe_join(&m.dir, rest) else {
            continue;
        };
        // Exact file. Mime from the canonical path — the real file extension,
        // not the symlink name, determines the response type.
        if let Some((canon, len)) = confined_file(&p, &m.dir).await {
            let mime = mime_for(&canon);
            return Some((canon, mime, len));
        }
        // A directory (or the prefix itself) may have been requested — try its
        // index.html. If p is a file, this silently fails.
        if let Some((canon, len)) = confined_file(&p.join("index.html"), &m.dir).await {
            let mime = mime_for(&canon);
            return Some((canon, mime, len));
        }
    }

    for m in &order {
        if !m.spa || strip_mount_prefix(&m.prefix, segs).is_none() {
            continue;
        }
        if let Some((canon, len)) = confined_file(&m.dir.join("index.html"), &m.dir).await {
            return Some((canon, "text/html; charset=utf-8", len));
        }
    }
    None
}

// Static file response: 200 + Content-Type determined from the extension.
fn static_response(data: Vec<u8>, mime: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", mime)
        .body(Full::new(Bytes::from(data)))
        .unwrap()
}

// Static response for HEAD: the file is NOT read (wasted disk I/O and memory on
// a large asset — codex P2), only the size from metadata is set manually as
// Content-Length (an empty body would auto-give 0). hyper writes no body on HEAD.
fn static_head_response(len: u64, mime: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", mime)
        .header("content-length", len.to_string())
        .body(Full::new(Bytes::new()))
        .unwrap()
}

// --- rate-limit (issue #79) ---

// Converts a window-unit symbol to seconds. Only :sec/:min/:hr — few tokens, a
// canonical set the AI remembers (add a new unit here if needed).
fn window_to_secs(unit: &str) -> Option<u64> {
    match unit {
        "sec" => Some(1),
        "min" => Some(60),
        "hr" => Some(3600),
        _ => None,
    }
}

// Fixed-window counter: counts the request for a key in the current window and
// checks after incrementing. If the limit is exceeded, Some(retry_after_secs)
// (until the window ends), otherwise None. The Mutex does read-modify-write under
// one lock — so parallel requests count a key atomically (no race).
fn check_and_count(state: &LimitState, key: &str, limit: u32, window_secs: u64) -> Option<u64> {
    // Wall-clock time (not Instant): the window boundary is tied to the epoch, so
    // Retry-After also comes out exactly as (window_id+1)*window_secs - now.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let window_id = now / window_secs;
    let mut bucket = state.lock().unwrap();
    // Periodic cleanup: remove keys from old windows (window_id smaller than the
    // current one) so user-controlled keys do not grow memory without bound. Only
    // once every SWEEP_EVERY operations — O(1) amortized.
    bucket.ops = bucket.ops.saturating_add(1);
    if bucket.ops >= SWEEP_EVERY {
        bucket.ops = 0;
        bucket.counts.retain(|_, (wid, _)| *wid >= window_id);
    }
    let entry = bucket
        .counts
        .entry(key.to_string())
        .or_insert((window_id, 0));
    // Moved to a new window — reset the count to zero.
    if entry.0 != window_id {
        *entry = (window_id, 0);
    }
    entry.1 = entry.1.saturating_add(1);
    if entry.1 > limit {
        // The window resets at epoch (window_id+1)*window_secs; now is smaller,
        // so the difference is always >= 1.
        Some((window_id + 1) * window_secs - now)
    } else {
        None
    }
}

// If the key function returns nil/empty — fall back to the client IP (so even a
// keyless request is limited). The "ip:" prefix avoids accidental collisions
// with a tenant_id/api-key value (in one limiter's state both live in the same
// HashMap).
fn client_fallback_key(req: &Value) -> String {
    let ip = match req {
        Value::Map(m) => match m.get("ip") {
            Some(Value::Str(s)) if !s.is_empty() => s.clone(),
            _ => "unknown".to_string(),
        },
        _ => "unknown".to_string(),
    };
    format!("ip:{}", ip)
}

// Response returned when the limit is exceeded: `429` + a `Retry-After` header
// (PRD format). As a __resp map — handle_request sends it like other rep responses.
fn rate_limited_response(retry_after: u64) -> Value {
    let mut body = BTreeMap::new();
    body.insert(
        "error".to_string(),
        Value::Str("rate limit exceeded".to_string()),
    );
    let mut headers = BTreeMap::new();
    headers.insert(
        "retry-after".to_string(),
        Value::Str(retry_after.to_string()),
    );
    let mut m = BTreeMap::new();
    m.insert("__resp".to_string(), Value::Bool(true));
    m.insert("status".to_string(), Value::Int(429));
    m.insert("body".to_string(), Value::Map(body));
    m.insert("headers".to_string(), Value::Map(headers));
    Value::Map(m)
}

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
fn percent_decode(s: &str, keep_path_seps: bool) -> String {
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
fn parse_query(q: &str) -> Value {
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
fn multipart_boundary(ct: &str) -> Option<String> {
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
fn cd_param(line: &str, key: &str) -> Option<String> {
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
fn parse_multipart(body: &[u8], boundary: &str) -> Option<(BTreeMap<String, Value>, Vec<Value>)> {
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
fn build_req(
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
fn with_ctx(req: Value, ctx: Arc<Mutex<BTreeMap<String, Value>>>) -> Value {
    if let Value::Map(mut m) = req {
        m.insert("ctx".to_string(), Value::Ctx(ctx));
        Value::Map(m)
    } else {
        req
    }
}

// --- Value/Flow -> hyper::Response ---

// Converts a Fluxon `Int` status (rep/fail) to a valid HTTP status u16. The
// check MUST be on the ORIGINAL i64: an `as u16` cast wraps first — `rep 65736`
// would wrap to 200 in u16, and some negative values would land in 3xx/4xx,
// faking success silently (issue #108). An out-of-range or non-HTTP code -> 500
// + a log, so the client does not read a handler error as success.
fn checked_status(n: i64) -> u16 {
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

fn json_response(status: u16, body: String) -> Response<Full<Bytes>> {
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
fn payload_too_large(limit: usize) -> Response<Full<Bytes>> {
    let mut m = BTreeMap::new();
    m.insert(
        "error".to_string(),
        Value::Str(format!("request body too large (limit: {} bytes)", limit)),
    );
    json_response(413, json_encode(&Value::Map(m)))
}

// 400 Bad Request — error reading the request body (e.g. a dropped connection) (#91).
fn bad_request(msg: &str) -> Response<Full<Bytes>> {
    let mut m = BTreeMap::new();
    m.insert("error".to_string(), Value::Str(msg.to_string()));
    json_response(400, json_encode(&Value::Map(m)))
}

// Is the value a `rep` response? `rep status body` -> {__resp:true ...} map
// (builtins.rs). If middleware returns this response, the chain stops (P1: rep
// auth rejection).
fn is_resp(v: &Value) -> bool {
    matches!(v, Value::Map(m) if matches!(m.get("__resp"), Some(Value::Bool(true))))
}

// Converts a value the handler returned successfully into a response.
// `rep` -> {__resp:true status body}. Otherwise 200 + the value.
fn value_to_response(v: Value) -> Response<Full<Bytes>> {
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
fn apply_headers_mut(hmap: &mut hyper::HeaderMap, headers: Option<&Value>) {
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
fn headers_to_map(hmap: &hyper::HeaderMap) -> BTreeMap<String, Value> {
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
fn flow_to_response(flow: Flow) -> Response<Full<Bytes>> {
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

// --- Interp HTTP dispatch ---

impl Interp {
    // http.<func> calls. eval_call routes here.
    pub fn http_dispatch(self: &Arc<Self>, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            "on" => self.http_on(args),
            "use" => self.http_use(args),
            "before" => self.http_before(args),
            "cors" => self.http_cors(args),
            "static" => self.http_static(args),
            "limit" => self.http_limit(args),
            "serve" => self.http_serve(args),
            "get" => http_client("GET", args, false),
            "post" => http_client("POST", args, true),
            "put" => http_client("PUT", args, true),
            "del" => http_client("DELETE", args, false),
            _ => Err(Flow::err(format!("http module has no '{}' function", func))),
        }
    }

    // http.on :method "/path" handler
    fn http_on(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let method = match args.first() {
            Some(Value::Sym(s)) | Some(Value::Str(s)) => s.to_lowercase(),
            _ => {
                return Err(Flow::err(
                    "http.on: argument 1 must be a method (:get/:post...)",
                ));
            }
        };
        let path = match args.get(1) {
            Some(Value::Str(s)) => s.clone(),
            _ => return Err(Flow::err("http.on: argument 2 must be a path (str)")),
        };
        let handler = match args.get(2) {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => return Err(Flow::err("http.on: argument 3 must be a handler (fn)")),
        };
        self.routes.lock().unwrap().push(Route {
            method,
            pattern: parse_pattern(&path),
            handler,
        });
        Ok(Value::Nil)
    }

    // http.use \req -> ...  — global middleware for all routes (issue #67).
    // Multiple calls form a chain (running in declaration order).
    fn http_use(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let handler = match args.first() {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => return Err(Flow::err("http.use: argument must be a handler (fn)")),
        };
        self.middlewares.lock().unwrap().push(Middleware {
            scope: None,
            handler,
            kind: MwKind::Fn,
        });
        Ok(Value::Nil)
    }

    // http.before "/api/*" \req -> ...  — middleware by path prefix (#67).
    // Pattern "/api/*" -> paths starting with /api; without "*" -> exact match.
    fn http_before(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let pat = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => {
                return Err(Flow::err("http.before: argument 1 must be a path (str)"));
            }
        };
        let handler = match args.get(1) {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => {
                return Err(Flow::err("http.before: argument 2 must be a handler (fn)"));
            }
        };
        self.middlewares.lock().unwrap().push(Middleware {
            scope: Some(pat),
            handler,
            kind: MwKind::Fn,
        });
        Ok(Value::Nil)
    }

    // http.cors origins [opts]  — declarative CORS (issue #135).
    //
    //   http.cors "*"                                # open to all (dev)
    //   http.cors ["https://app.example.com"]        # allowed origins
    //   http.cors ["https://app.example.com"] {creds: true}
    //
    // 1st arg: "*" (str) — any origin, or a list of origins.
    // 2nd arg (optional): an options map:
    //   creds:   true -> Allow-Credentials (cookie/Authorization). When combined
    //            with "*" the response reflects the request origin (browser rule).
    //   methods: allowed methods (str). A wide default set.
    //   headers: allowed request headers (str). A wide default set.
    //   max_age: preflight cache duration in seconds (int). Default 86400 (1 day).
    fn http_cors(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let origins = match args.first() {
            // "*" — any origin (None internally).
            Some(Value::Str(s)) if s == "*" => None,
            // Accept a single origin as a str too (convenience).
            Some(Value::Str(s)) => Some(vec![s.clone()]),
            // A list of origins.
            Some(Value::List(items)) => {
                let mut list = Vec::with_capacity(items.len());
                for it in items.iter() {
                    match it {
                        Value::Str(s) => list.push(s.clone()),
                        _ => {
                            return Err(Flow::err(
                                "http.cors: origin list must consist of str elements",
                            ));
                        }
                    }
                }
                Some(list)
            }
            _ => {
                return Err(Flow::err(
                    "http.cors: argument 1 must be \"*\" or a list of origins",
                ));
            }
        };

        let mut cfg = CorsConfig {
            origins,
            // A wide default set — works without the agent configuring it.
            methods: "GET, POST, PUT, PATCH, DELETE, OPTIONS".to_string(),
            headers: "Content-Type, Authorization".to_string(),
            creds: false,
            max_age: 86400,
        };

        if let Some(Value::Map(opts)) = args.get(1) {
            if let Some(v) = opts.get("creds") {
                cfg.creds = !matches!(v, Value::Nil | Value::Bool(false));
            }
            if let Some(Value::Str(s)) = opts.get("methods") {
                cfg.methods = s.clone();
            }
            if let Some(Value::Str(s)) = opts.get("headers") {
                cfg.headers = s.clone();
            }
            if let Some(Value::Int(n)) = opts.get("max_age")
                && *n >= 0
            {
                cfg.max_age = *n as u64;
            }
        } else if args.len() > 1 && !matches!(args.get(1), Some(Value::Nil)) {
            return Err(Flow::err(
                "http.cors: argument 2 must be an options map ({creds: true})",
            ));
        }

        *self.cors.lock().unwrap() = Some(cfg);
        Ok(Value::Nil)
    }

    // http.static prefix dir [opts]  — serve static files from a folder (#134).
    //
    //   http.static "/assets" "./public"        # /assets/app.css -> ./public/app.css
    //   http.static "/" "./dist" {spa: true}    # if not found -> ./dist/index.html
    //
    // The directory is resolved relative to the script file's directory (the same
    // rule as `use ./file`) and canonicalized at registration — a missing
    // directory errors at startup (fail fast instead of a silent 404 at deploy
    // time). Content-Type is automatic from the extension; `../` traversal (also
    // percent-encoded) is mandatorily blocked; route priority: exact route > static.
    fn http_static(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let prefix = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => {
                return Err(Flow::err(
                    "http.static: argument 1 must be a prefix (str), for example \"/assets\"",
                ));
            }
        };
        let dir = match args.get(1) {
            Some(Value::Str(s)) => s.clone(),
            _ => {
                return Err(Flow::err(
                    "http.static: argument 2 must be a directory (str), for example \"./public\"",
                ));
            }
        };
        let spa = match args.get(2) {
            None | Some(Value::Nil) => false,
            Some(Value::Map(m)) => !matches!(
                m.get("spa"),
                None | Some(Value::Nil) | Some(Value::Bool(false))
            ),
            _ => {
                return Err(Flow::err(
                    "http.static: argument 3 must be an options map ({spa: true})",
                ));
            }
        };
        let p = PathBuf::from(&dir);
        let resolved = if p.is_absolute() {
            p
        } else {
            self.base_dir().join(p)
        };
        let canon = std::fs::canonicalize(&resolved).map_err(|e| {
            Flow::err(format!(
                "http.static: could not open directory '{}': {}",
                dir, e
            ))
        })?;
        if !canon.is_dir() {
            return Err(Flow::err(format!(
                "http.static: '{}' is not a directory (a file was given)",
                dir
            )));
        }
        self.statics.lock().unwrap().push(StaticMount {
            prefix: parse_static_prefix(&prefix),
            dir: canon,
            spa,
        });
        Ok(Value::Nil)
    }

    // http.limit [path] N :sec|:min|:hr \req -> key  — declarative rate-limit (#79).
    //
    //   http.limit 100 :min \req -> req.ctx.tenant_id          # per-tenant, all paths
    //   http.limit "/api/*" 100 :min \req -> req.headers.x_api_key  # per-key, prefix
    //
    // Path (str) is an optional 1st arg — if present it attaches by prefix like
    // http.before, otherwise it is global like http.use. The key function is
    // called per request to identify the client; if it returns nil/empty we fall
    // back to req.ip. On exceeding the limit, an automatic `429` + `Retry-After`
    // (seconds until the window ends).
    fn http_limit(&self, args: Vec<Value>) -> Result<Value, Flow> {
        // If the 1st arg is a str — path scope (like http.before). Otherwise global.
        let (scope, i) = match args.first() {
            Some(Value::Str(s)) => (Some(s.clone()), 1),
            _ => (None, 0),
        };
        let limit = match args.get(i) {
            Some(Value::Int(n)) if *n > 0 => *n as u32,
            _ => {
                return Err(Flow::err(
                    "http.limit: limit must be a positive int (for example 100)",
                ));
            }
        };
        let window_secs = match args.get(i + 1) {
            Some(Value::Sym(s)) | Some(Value::Str(s)) => match window_to_secs(s) {
                Some(secs) => secs,
                None => {
                    return Err(Flow::err("http.limit: window must be :sec, :min or :hr"));
                }
            },
            _ => {
                return Err(Flow::err(
                    "http.limit: window unit (:sec/:min/:hr) is required",
                ));
            }
        };
        let keyfn = match args.get(i + 2) {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => {
                return Err(Flow::err(
                    "http.limit: key function (\\req -> ...) is required",
                ));
            }
        };
        self.middlewares.lock().unwrap().push(Middleware {
            scope,
            handler: keyfn,
            kind: MwKind::Limit {
                limit,
                window_secs,
                state: Arc::new(Mutex::new(LimitBucket::new())),
            },
        });
        Ok(Value::Nil)
    }

    // http.serve port — a blocking tokio multi-thread server.
    // `http.serve PORT` does NOT block immediately; instead it adds to the list
    // of pending servers (deferred). After top-level code finishes
    // (`serve_mod::run_pending`) they are all spawned on ONE shared tokio
    // runtime — so HTTP + WS run together in one process.
    fn http_serve(self: &Arc<Self>, args: Vec<Value>) -> Result<Value, Flow> {
        let port = match args.first() {
            Some(Value::Int(n)) => *n as u16,
            _ => return Err(Flow::err("http.serve: port (int) is required")),
        };
        // Optional second argument — an options map: `{max_body: BYTES}`.
        // If omitted, default DEFAULT_MAX_BODY; `max_body: 0` disables the limit.
        let max_body = match args.get(1) {
            None => DEFAULT_MAX_BODY,
            Some(Value::Map(m)) => match m.get("max_body") {
                None => DEFAULT_MAX_BODY,
                Some(Value::Int(n)) if *n >= 0 => *n as usize,
                _ => {
                    return Err(Flow::err("http.serve: max_body must be a non-negative int"));
                }
            },
            _ => {
                return Err(Flow::err(
                    "http.serve: second argument must be an options map ({max_body: N})",
                ));
            }
        };
        self.pending_servers
            .lock()
            .unwrap()
            .push(crate::serve_mod::PendingServer::Http { port, max_body });
        Ok(Value::Nil)
    }
}

// Binds the port (deferred: after top-level finishes, in `serve_mod`). Returns a
// bind error as `Flow::Error` — `run_pending` propagates it up, so if the port
// is busy the process exits with code != 0 (issue #108: let deploy/supervisor
// notice the error). Called before the accept loop, so a bind failure surfaces
// before the spawn.
pub async fn bind(port: u16) -> Result<TcpListener, Flow> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    TcpListener::bind(addr)
        .await
        .map_err(|e| Flow::err(format!("Fluxon HTTP port {} bind error: {}", port, e)))
}

// The accept loop for a single HTTP server — spawned inside the shared
// event-loop (`serve_mod`). The listener was opened beforehand with `bind`
// (a bind error is raised before the spawn).
pub async fn serve_loop(interp: Arc<Interp>, listener: TcpListener, max_body: usize) {
    let port = listener.local_addr().map(|a| a.port()).unwrap_or_default();
    eprintln!("Fluxon HTTP server: http://localhost:{}", port);

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("http accept error: {}", e);
                continue;
            }
        };
        let io = TokioIo::new(stream);
        let interp = interp.clone();
        // Client IP (rate-limit fallback + req.ip). From the peer SocketAddr —
        // we read the IP once per connection (every request on this connection
        // shares one IP).
        let client_ip = peer.ip().to_string();
        tokio::spawn(async move {
            let service = service_fn(move |req: Request<Incoming>| {
                let interp = interp.clone();
                let client_ip = client_ip.clone();
                async move { handle_request(interp, req, client_ip, max_body).await }
            });
            // header_read_timeout limits slowloris connections (those sending
            // headers very slowly) (issue #92). Without a timer set, the option
            // is silently ignored (or panics) — so we provide a TokioTimer.
            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(Duration::from_secs(DEFAULT_HEADER_READ_TIMEOUT_SECS))
                .serve_connection(io, service)
                .await
            {
                eprintln!("connection error: {}", e);
            }
        });
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
fn run_middleware_chain(
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

// Static file attempt (issue #134) — called only when no exact route is found
// (route priority). None — static did not match, the caller returns 404.
// Only GET/HEAD (reading a file is idempotent; other methods are API semantics).
// The middleware chain runs here too — so `http.before "/admin/*"` auth protects
// the static folder as well (if the chain returns a response, no file is read).
async fn try_serve_static(
    interp: &Arc<Interp>,
    method: &str,
    path: &str,
    query: String,
    headers: BTreeMap<String, Value>,
    client_ip: String,
) -> Option<Response<Full<Bytes>>> {
    if method != "get" && method != "head" {
        return None;
    }
    let mounts: Vec<StaticMount> = interp.statics.lock().unwrap().clone();
    if mounts.is_empty() {
        return None;
    }
    // Segments are percent-decoded (the browser encodes non-ASCII names);
    // `keep_path_seps=true` — `%2F` stays raw, decoding spawns no new `/`.
    let segs: Vec<String> = path_segments(path)
        .iter()
        .map(|s| percent_decode(s, true))
        .collect();
    // If no mount prefix matches — not the static area, a plain 404
    // (no middleware, same behavior as the route-404).
    if !mounts
        .iter()
        .any(|m| strip_mount_prefix(&m.prefix, &segs).is_some())
    {
        return None;
    }

    // The middleware chain runs BEFORE checking whether the file EXISTS (codex
    // P2): otherwise, under a protected mount an existing file would give 401 and
    // a missing one 404, letting an unauthenticated client discover file names
    // from the status difference. Every request whose prefix matches passes
    // through the chain — regardless of whether the file exists.
    let chain: Vec<Middleware> = interp
        .middlewares
        .lock()
        .unwrap()
        .iter()
        .filter(|mw| match &mw.scope {
            None => true,
            Some(pat) => prefix_matches(pat, path),
        })
        .cloned()
        .collect();
    if !chain.is_empty() {
        // GET/HEAD has an empty body — the body is not read (unlike the route path).
        let ctx = Arc::new(Mutex::new(BTreeMap::new()));
        let request_value = with_ctx(
            build_req(
                method.to_string(),
                path.to_string(),
                query,
                headers,
                BTreeMap::new(),
                client_ip,
                Bytes::new(),
                false,
                None,
            ),
            ctx,
        );
        let interp2 = interp.clone();
        let mw_result = tokio::task::spawn_blocking(move || {
            run_middleware_chain(&interp2, chain, &request_value)
        })
        .await;
        match mw_result {
            Ok(Ok(None)) => {} // chain passed — move on to the file
            Ok(Ok(Some(v))) => return Some(value_to_response(v)),
            Ok(Err(flow)) => return Some(flow_to_response(flow)),
            Err(join_err) => {
                return Some(flow_to_response(Flow::Error(format!(
                    "middleware panic: {}",
                    join_err
                ))));
            }
        }
    }
    let (file, mime, len) = resolve_static(&mounts, &segs).await?;
    // HEAD — no content needed: respond with the metadata size WITHOUT reading
    // the file (avoid wasted disk I/O / memory on a large asset — codex P2).
    if method == "head" {
        return Some(static_head_response(len, mime));
    }
    match tokio::fs::read(&file).await {
        Ok(data) => Some(static_response(data, mime)),
        // metadata reported a file, but the read still errors (race/permission) —
        // fall to a silent 404 (do not leak whether the file exists).
        Err(_) => None,
    }
}

// Handles a single request: find the route -> build req -> call the handler in
// spawn_blocking (synchronous interp) -> response.
async fn handle_request(
    interp: Arc<Interp>,
    req: Request<Incoming>,
    client_ip: String,
    max_body: usize,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method().as_str().to_lowercase();
    let uri = req.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or("").to_string();

    // Gather headers into a map (lowercase keys, '-' -> '_' so they read as
    // req.headers.x_user_id in Fluxon). Repeated names are merged inside
    // headers_to_map (issue #101).
    let headers: BTreeMap<String, Value> = headers_to_map(req.headers())
        .into_iter()
        .map(|(k, v)| (k.replace('-', "_"), v))
        .collect();
    // CORS config (issue #135) — when enabled we answer preflight based on Origin
    // and add `Access-Control-Allow-*` to every response. The request's Origin
    // header (with '-' -> '_' in headers) is used for the allow check.
    let cors = interp.cors.lock().unwrap().clone();
    let req_origin = match headers.get("origin") {
        Some(Value::Str(o)) => Some(o.clone()),
        _ => None,
    };

    // CORS preflight — if CORS is enabled we answer directly without looking up a
    // route (the browser sends OPTIONS before the real request). This also solves
    // the route-not-found (404) problem: no handler is needed for preflight.
    //
    // Not EVERY OPTIONS — we catch only a REAL preflight: per the Fetch standard a
    // browser preflight ALWAYS sends the Access-Control-Request-Method header. If
    // it is absent (a plain OPTIONS — querying resource capabilities, or the
    // user's own `http.on :options "/..."` handler), the request falls through to
    // routing as usual (codex P2). When CORS is off, OPTIONS also goes to plain
    // routing.
    let is_preflight = method == "options" && headers.contains_key("access_control_request_method");
    if is_preflight && let Some(cfg) = &cors {
        return Ok(cors_preflight_response(cfg, req_origin.as_deref()));
    }

    let is_json = matches!(
        headers.get("content_type"),
        Some(Value::Str(ct)) if ct.contains("application/json")
    );
    // multipart/form-data boundary (issue #133) — if present, the body is split
    // into parts (req.body fields, req.files files).
    let multipart = match headers.get("content_type") {
        Some(Value::Str(ct)) => multipart_boundary(ct),
        _ => None,
    };

    // Find the route (the handler before the bytes, to return 404 quickly).
    let matched = {
        let routes = interp.routes.lock().unwrap();
        match_route(&routes, &method, &path)
    };

    let (route, params) = match matched {
        Some(x) => x,
        None => {
            // No exact route found — try the static mounts (issue #134).
            // Route priority: exact route > static (static only here).
            if let Some(resp) = try_serve_static(
                &interp,
                &method,
                &path,
                query.clone(),
                headers.clone(),
                client_ip.clone(),
            )
            .await
            {
                return Ok(cors_finalize(resp, &cors, req_origin.as_deref()));
            }
            let mut m = BTreeMap::new();
            m.insert(
                "error".to_string(),
                Value::Str(format!("not found: {} {}", method, path)),
            );
            // 404 also gets CORS headers — otherwise the browser cannot read the
            // error response body because of the CORS barrier (makes debugging hard).
            let resp = json_response(404, json_encode(&Value::Map(m)));
            return Ok(cors_finalize(resp, &cors, req_origin.as_deref()));
        }
    };

    // Gather the body — with a size limit (issue #91). Without a limit, collect()
    // gathers the whole body into memory: a client can fill server memory by
    // sending a huge body (DoS).
    let body_bytes = if max_body == 0 {
        // Limit disabled (http.serve PORT {max_body: 0}) — unbounded read.
        match req.into_body().collect().await {
            Ok(c) => c.to_bytes(),
            // This used to silently fall to Bytes::new() (a dropped POST reached
            // the handler with body:nil); now we return 400 (issue #91). Finalized
            // with CORS headers (codex P2: every response gets CORS).
            Err(_) => {
                return Ok(cors_finalize(
                    bad_request("could not read request body"),
                    &cors,
                    req_origin.as_deref(),
                ));
            }
        }
    } else {
        // Fast path: if the declared Content-Length exceeds the limit, return 413
        // without reading the body at all (we do not wait for the client to finish
        // uploading GBs). A false/missing Content-Length is caught by Limited below.
        let declared = req
            .headers()
            .get(hyper::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());
        if matches!(declared, Some(len) if len > max_body as u64) {
            return Ok(cors_finalize(
                payload_too_large(max_body),
                &cors,
                req_origin.as_deref(),
            ));
        }
        // Limited enforces the real limit during streaming too — if the limit is
        // exceeded it stops reading (protects even when Content-Length is false).
        match Limited::new(req.into_body(), max_body).collect().await {
            Ok(c) => c.to_bytes(),
            Err(e) => {
                // Limit exceeded -> 413; another read error (e.g. a dropped
                // connection) -> 400.
                if e.downcast_ref::<http_body_util::LengthLimitError>()
                    .is_some()
                {
                    return Ok(cors_finalize(
                        payload_too_large(max_body),
                        &cors,
                        req_origin.as_deref(),
                    ));
                }
                return Ok(cors_finalize(
                    bad_request("could not read request body"),
                    &cors,
                    req_origin.as_deref(),
                ));
            }
        }
    };

    // Gather the middleware chain matching this request (issue #67): first global
    // (http.use), then by path prefix (http.before). The list order is preserved.
    // We take the lock early here (handlers are Value clones — Arc, cheap).
    let chain: Vec<Middleware> = {
        interp
            .middlewares
            .lock()
            .unwrap()
            .iter()
            .filter(|mw| match &mw.scope {
                None => true,                            // http.use/limit — all paths
                Some(pat) => prefix_matches(pat, &path), // http.before/limit — prefix match
            })
            .cloned() // in a Middleware clone the Limit state is an Arc — the same pointer is shared
            .collect()
    };

    // Request-scoped ctx cell: fresh per request. req clones share the Arc, so the
    // handler sees the ctx middleware wrote in the same cell (#68).
    let ctx = Arc::new(Mutex::new(BTreeMap::new()));
    let request_value = with_ctx(
        build_req(
            method, path, query, headers, params, client_ip, body_bytes, is_json, multipart,
        ),
        ctx,
    );
    let handler = route.handler;

    // Run the synchronous interp work on a blocking thread — it does not block a
    // tokio worker, and each request runs on its own thread -> truly parallel.
    let result = tokio::task::spawn_blocking(move || {
        match run_middleware_chain(&interp, chain, &request_value) {
            Ok(Some(v)) => Ok(v), // middleware returned a response (rep/429) -> handler not called
            Ok(None) => interp.apply(handler, vec![request_value]),
            Err(flow) => Err(flow),
        }
    })
    .await;

    let resp = match result {
        Ok(Ok(v)) => value_to_response(v),
        Ok(Err(flow)) => flow_to_response(flow),
        Err(join_err) => flow_to_response(Flow::Error(format!("handler panic: {}", join_err))),
    };
    // When CORS is enabled we add `Access-Control-Allow-*` to every response
    // (issue #135). Even if the handler wrote it manually with
    // `rep ... {access_control_allow_origin: ...}`, insert overrides it — the
    // canonical config wins.
    Ok(cors_finalize(resp, &cors, req_origin.as_deref()))
}

// OPTIONS preflight response (issue #135). The browser sends OPTIONS before the
// real request and expects `Access-Control-Allow-*` headers. No body
// (204 No Content). If the origin is not allowed, 204 is returned without CORS
// headers (the browser blocks the request — correct behavior).
fn cors_preflight_response(cfg: &CorsConfig, req_origin: Option<&str>) -> Response<Full<Bytes>> {
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

// --- HTTP client: http.get/post/put/del ---

// The request body is now a plain bytes buffer: the alias makes the client type
// easier to read.
type ClientBody = Full<Bytes>;
// HttpsConnector<HttpConnector> handles both http:// and https:// — TLS
// activates only on the https scheme, plaintext requests work as before.
type PooledHttpClient = Client<HttpsConnector<HttpConnector>, ClientBody>;

// A one-time global runtime for client requests (the Fluxon script is sync).
// pub(crate): the `ai` battery reuses this runtime/pool too (the LLM API is also
// a plain https POST), to avoid building a duplicate tokio runtime/pool.
pub(crate) fn client_runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("klient tokio runtime")
    })
}

// The hyper client has a connection pool inside; we keep it global and reuse one
// pool across requests via clone().
// pub(crate): the `ai` battery reuses this pool too.
pub(crate) fn pooled_http_client() -> PooledHttpClient {
    static CLIENT: OnceLock<PooledHttpClient> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            // Build an https connector with webpki-roots roots. enable_http1 fits
            // the hyper 1.x http1 client. https_or_http() lets http URLs through too
            // (not https-only) — which is why plaintext requests are preserved.
            let https = hyper_rustls::HttpsConnectorBuilder::new()
                .with_webpki_roots()
                .https_or_http()
                .enable_http1()
                .build();
            Client::builder(TokioExecutor::new()).build(https)
        })
        .clone()
}

// Client request options (read from the last map argument).
// follow=true -> automatically follows a 3xx redirect by Location (default off).
// max -> redirect hop limit (default 10); exceeding it is an error.
// headers -> custom request headers to add (x-api-key, Authorization,
// anthropic-version...). Symmetric with req.headers/res.headers.
struct ClientOpts {
    follow: bool,
    max: i64,
    headers: BTreeMap<String, String>,
    // Request timeout: Some(dur) — error if not finished within it; None — no
    // timeout (`timeout: 0`). Default Some(30s) (issue #92).
    timeout: Option<Duration>,
}

impl Default for ClientOpts {
    fn default() -> Self {
        ClientOpts {
            follow: false,
            max: 10,
            headers: BTreeMap::new(),
            timeout: Some(Duration::from_secs(DEFAULT_CLIENT_TIMEOUT_SECS)),
        }
    }
}

// Reads the options map. If follow is truthy, following is enabled.
fn parse_client_opts(opts: Option<&Value>) -> ClientOpts {
    let mut o = ClientOpts::default();
    if let Some(Value::Map(m)) = opts {
        if let Some(v) = m.get("follow") {
            o.follow = !matches!(v, Value::Nil | Value::Bool(false));
        }
        if let Some(Value::Int(n)) = m.get("max") {
            o.max = *n;
        }
        // timeout: N (seconds) — error if the request does not finish within it.
        // 0 or negative — no timeout (None). Other value types are ignored
        // (default 30s).
        if let Some(Value::Int(n)) = m.get("timeout") {
            o.timeout = if *n > 0 {
                Some(Duration::from_secs(*n as u64))
            } else {
                None
            };
        }
        // headers: {key: value} — convert each pair to a str. The key is kept as
        // given (an HTTP header name is case-insensitive, but we do not mangle
        // what the user wrote). A non-str value (e.g. int) is converted to its
        // text form.
        if let Some(Value::Map(hm)) = m.get("headers") {
            for (k, v) in hm {
                let val = match v {
                    Value::Str(s) => s.clone(),
                    Value::Nil => continue, // nil header — skip it
                    other => format!("{}", other),
                };
                o.headers.insert(k.clone(), val);
            }
        }
    }
    o
}

// http.get url [opts]  /  http.post url body [opts]
// If has_body=true then args[1]=body, opts=args[2]; otherwise opts=args[1].
fn http_client(method: &str, args: Vec<Value>, has_body: bool) -> Result<Value, Flow> {
    let url = match args.first() {
        Some(Value::Str(s)) => s.clone(),
        _ => {
            return Err(Flow::err(format!(
                "http.{}: url (str) is required",
                method.to_lowercase()
            )));
        }
    };
    let (body, opts_arg) = if has_body {
        (args.get(1).cloned(), args.get(2))
    } else {
        (None, args.get(1))
    };
    let opts = parse_client_opts(opts_arg);

    // Prepare the request body once (reused across redirects too). A bytes body
    // goes raw (issue #132) — so Bytes, not String.
    let (body_payload, is_json) = match &body {
        Some(Value::Map(_)) | Some(Value::List(_)) => {
            (Bytes::from(json_encode(body.as_ref().unwrap())), true)
        }
        Some(Value::Str(s)) => (Bytes::from(s.clone()), false),
        Some(Value::Bytes(b)) => (Bytes::from(b.as_ref().clone()), false),
        Some(other) => (Bytes::from(format!("{}", other)), false),
        None => (Bytes::new(), false),
    };

    // Take the timeout out of opts separately (opts is moved into the async block below).
    let timeout = opts.timeout;
    client_runtime().block_on(async move {
        // The whole request logic (including redirects) — the timeout wraps it.
        let work = async move {
            let mut current = url;
            // The method can change on a redirect (303 and GET-converting 301/302).
            let mut cur_method = method.to_string();
            let mut hops: i64 = 0;
            // The original request origin (scheme, host, port). If a redirect
            // leads to a foreign origin, credential headers are not sent (issue #96).
            let mut first_origin: Option<(String, String, u16)> = None;
            // The flag is sticky: even if a foreign origin redirects back to the
            // original host, credentials are not restored (same caution as
            // reqwest/curl).
            let mut cross_origin = false;

            loop {
                let uri: hyper::Uri = current
                    .parse()
                    .map_err(|e| Flow::err(format!("invalid url: {}", e)))?;

                let this_origin = uri_origin(&uri);
                match &first_origin {
                    None => first_origin = Some(this_origin),
                    Some(o) if *o != this_origin => cross_origin = true,
                    _ => {}
                }

                // Once turned into GET, no body is sent.
                let send_body = cur_method != "GET" && cur_method != "DELETE";
                let mut builder = Request::builder().method(cur_method.as_str()).uri(uri);
                // Add the user's custom headers first. If the user supplied
                // content-type themselves, do not overwrite it with the auto value.
                let mut has_user_ct = false;
                for (k, v) in &opts.headers {
                    // Cross-origin redirect: do not leak Authorization/x-api-key/
                    // Cookie to a foreign host (issue #96).
                    if cross_origin && is_sensitive_header(k) {
                        continue;
                    }
                    if k.eq_ignore_ascii_case("content-type") {
                        has_user_ct = true;
                    }
                    builder = builder.header(k.as_str(), v.as_str());
                }
                if is_json && send_body && !has_user_ct {
                    builder = builder.header("content-type", "application/json");
                }
                let payload = if send_body {
                    body_payload.clone()
                } else {
                    Bytes::new()
                };
                let req = builder
                    .body(Full::new(payload))
                    .map_err(|e| Flow::err(format!("building request: {}", e)))?;

                let resp = pooled_http_client()
                    .request(req)
                    .await
                    .map_err(|e| Flow::err(format!("http request: {}", e)))?;

                let status = resp.status().as_u16();

                // Redirect following (opt-in). On 3xx + Location, move to the next hop.
                if opts.follow
                    && (300..400).contains(&status)
                    && let Some(loc) = resp
                        .headers()
                        .get("location")
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string())
                {
                    hops += 1;
                    if hops > opts.max {
                        return Err(Flow::err(format!(
                            "redirect limit exceeded ({} hops)",
                            opts.max
                        )));
                    }
                    // Turn a relative Location into a full URL based on the current URL.
                    current = resolve_location(&current, &loc);
                    // 303 is always GET; 301/302 in practice become GET (POST->GET).
                    // 307/308 preserve the method and body.
                    if status == 303 || ((status == 301 || status == 302) && cur_method == "POST") {
                        cur_method = "GET".to_string();
                    }
                    // If the 3xx body is drained, the hyper pool can reuse the
                    // connection (issue #96). But draining must not stall the
                    // redirect (PR #144 review): only if the size is known and
                    // small do we read frame-by-frame (unbuffered) within a short
                    // timeout. If the size is unknown (chunked/stream) or large,
                    // drop immediately — the connection closes and the next hop
                    // opens a new one.
                    const REDIRECT_DRAIN_MAX: u64 = 64 * 1024;
                    let known_len = resp
                        .headers()
                        .get("content-length")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok());
                    if let Some(len) = known_len
                        && len <= REDIRECT_DRAIN_MAX
                    {
                        let drain = async {
                            let mut body = resp.into_body();
                            while let Some(frame) = body.frame().await {
                                if frame.is_err() {
                                    break;
                                }
                            }
                        };
                        // A slow upstream may trickle even a small declared size —
                        // if it does not finish, we abandon the connection.
                        let _ = tokio::time::timeout(Duration::from_millis(500), drain).await;
                    }
                    continue;
                }

                // Final response — gather headers, status, body.
                let resp_is_json = resp
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.contains("application/json"))
                    .unwrap_or(false);

                // Headers: lowercase keys (hyphen preserved — read with m[k]),
                // repeated names are merged (issue #101).
                let headers = headers_to_map(resp.headers());

                let bytes = resp
                    .into_body()
                    .collect()
                    .await
                    .map_err(|e| Flow::err(format!("reading response: {}", e)))?
                    .to_bytes();
                let resp_body = match String::from_utf8(bytes.to_vec()) {
                    Ok(text) if resp_is_json => json_decode(&text).unwrap_or(Value::Str(text)),
                    Ok(text) => Value::Str(text),
                    // A non-UTF-8 response (image, archive) — bytes (issue #132).
                    // Lossy reading used to silently corrupt binary data.
                    Err(e) => Value::Bytes(Arc::new(e.into_bytes())),
                };

                let mut m = BTreeMap::new();
                m.insert("status".to_string(), Value::Int(status as i64));
                m.insert("body".to_string(), resp_body);
                m.insert("headers".to_string(), Value::Map(headers));
                // If follow is enabled, also return how many redirects happened.
                if opts.follow {
                    m.insert("hops".to_string(), Value::Int(hops));
                }
                return Ok(Value::Map(m));
            }
        };

        // If a timeout is set, wrap the request in it; on expiry a clear error
        // (a stuck upstream must not block the whole thread forever — issue #92).
        match timeout {
            Some(dur) => match tokio::time::timeout(dur, work).await {
                Ok(r) => r,
                Err(_) => Err(Flow::err(format!(
                    "http request timeout (no response within {} sec)",
                    dur.as_secs()
                ))),
            },
            None => work.await,
        }
    })
}

// Resolves a redirect Location based on the current URL. If Location is a full
// URL (`http://...`) that is returned; otherwise it attaches to the current
// URL's scheme+host (an absolute path `/x` or a relative path).
fn resolve_location(base: &str, loc: &str) -> String {
    if loc.starts_with("http://") || loc.starts_with("https://") {
        return loc.to_string();
    }
    // Extract the scheme://host part from base. Cut the query/fragment first —
    // a `/` in them is not a path segment (e.g. `?q=/z`, issue #96).
    let scheme_end = base.find("://").map(|i| i + 3).unwrap_or(0);
    let base_end = base[scheme_end..]
        .find(['?', '#'])
        .map(|i| scheme_end + i)
        .unwrap_or(base.len());
    let base = &base[..base_end];
    // Scheme-relative `//host/path` — the base scheme is kept, the rest from Location.
    if let Some(rest) = loc.strip_prefix("//") {
        let scheme = if scheme_end >= 3 {
            &base[..scheme_end - 2]
        } else {
            "http:"
        };
        return format!("{}//{}", scheme, rest);
    }
    let host_end = base[scheme_end..]
        .find('/')
        .map(|i| scheme_end + i)
        .unwrap_or(base.len());
    let origin = &base[..host_end];
    if loc.starts_with('/') {
        format!("{}{}", origin, loc)
    } else {
        // Relative path: replace the last segment of the current path. If there
        // is no path at all it is treated as root — `/` is added (issue #96:
        // it used to produce "http://a.com" + "page" -> "http://a.compage").
        let path_part = &base[host_end..];
        match path_part.rfind('/') {
            Some(i) => format!("{}{}", &base[..host_end + i + 1], loc),
            None => format!("{}/{}", origin, loc),
        }
    }
}

// The origin (scheme, host, port) triple — used to detect whether a redirect
// changed host/port/scheme. If no port is given, the scheme default is used
// (http=80, https=443): `http://a.com` and `http://a.com:80` are one origin.
fn uri_origin(uri: &hyper::Uri) -> (String, String, u16) {
    let scheme = uri.scheme_str().unwrap_or("http").to_ascii_lowercase();
    let host = uri.host().unwrap_or("").to_ascii_lowercase();
    let port = uri
        .port_u16()
        .unwrap_or(if scheme == "https" { 443 } else { 80 });
    (scheme, host, port)
}

// Credential headers dropped on a cross-origin redirect — same behavior as
// curl/reqwest: a foreign host must not see the API key/session.
fn is_sensitive_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("authorization")
        || name.eq_ignore_ascii_case("proxy-authorization")
        || name.eq_ignore_ascii_case("cookie")
        || name.eq_ignore_ascii_case("x-api-key")
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn location_absolute_url() {
        // If it is a full URL it is returned as-is (base is ignored).
        let got = resolve_location("http://a.com/x", "http://b.com/y");
        assert_eq!(got, "http://b.com/y");
    }

    #[test]
    fn location_root_relative() {
        // A `/...` absolute path — attaches to base's origin, the path is replaced.
        let got = resolve_location("http://a.com/old/path", "/new");
        assert_eq!(got, "http://a.com/new");
    }

    #[test]
    fn location_relative_path() {
        // A relative path — placed in place of the current path's last segment.
        let got = resolve_location("http://a.com/dir/file", "other");
        assert_eq!(got, "http://a.com/dir/other");
    }

    #[test]
    fn location_relative_at_root() {
        // If there is no path after the host it is treated as root — `/` is added
        // (issue #96: it used to produce the broken URL "http://a.compage").
        let got = resolve_location("http://a.com", "page");
        assert_eq!(got, "http://a.com/page");
    }

    #[test]
    fn location_base_query_kesiladi() {
        // A `/` in the base query is not a path segment (issue #96) — a relative
        // path is resolved against the real path before the query.
        let got = resolve_location("http://a.com/search?q=/z", "next");
        assert_eq!(got, "http://a.com/next");
        // For an absolute path too, the query does not corrupt the origin.
        let got2 = resolve_location("http://a.com/a/b?x=1", "/new");
        assert_eq!(got2, "http://a.com/new");
    }

    #[test]
    fn location_scheme_relative() {
        // `//host/path` — the scheme is taken from base (https is preserved).
        let got = resolve_location("https://a.com/x", "//b.com/y");
        assert_eq!(got, "https://b.com/y");
    }

    #[test]
    fn origin_default_port_va_case() {
        // Whether the default port is written and letter case do not matter.
        let a: hyper::Uri = "http://A.com/x".parse().unwrap();
        let b: hyper::Uri = "http://a.com:80/y".parse().unwrap();
        assert_eq!(uri_origin(&a), uri_origin(&b));
        // A scheme or port difference — a different origin (credentials must not go).
        let c: hyper::Uri = "https://a.com/x".parse().unwrap();
        let d: hyper::Uri = "http://a.com:8080/x".parse().unwrap();
        assert_ne!(uri_origin(&a), uri_origin(&c));
        assert_ne!(uri_origin(&a), uri_origin(&d));
    }

    #[test]
    fn sensitive_header_royxati() {
        // Credential headers are recognized case-insensitively; a plain header is not.
        assert!(is_sensitive_header("Authorization"));
        assert!(is_sensitive_header("X-API-Key"));
        assert!(is_sensitive_header("cookie"));
        assert!(is_sensitive_header("Proxy-Authorization"));
        assert!(!is_sensitive_header("content-type"));
        assert!(!is_sensitive_header("x-request-id"));
    }

    // Mini HTTP server: accepts one connection for each response in `responses`
    // and records the incoming request text. Responses must use `Connection:
    // close` — so each hop arrives on a new connection and the records are
    // deterministic.
    fn spawn_test_server(responses: Vec<String>) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let mut captured = Vec::new();
            for resp in responses {
                let (mut sock, _) = listener.accept().unwrap();
                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                loop {
                    let n = sock.read(&mut tmp).unwrap();
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                captured.push(String::from_utf8_lossy(&buf).to_string());
                sock.write_all(resp.as_bytes()).unwrap();
            }
            captured
        });
        (port, handle)
    }

    // Builds a GET request with follow:true + credential headers.
    fn follow_get_with_credentials(url: String) -> Result<Value, Flow> {
        let mut headers = BTreeMap::new();
        headers.insert(
            "authorization".to_string(),
            Value::Str("Bearer secret".into()),
        );
        headers.insert("x-api-key".to_string(), Value::Str("key".into()));
        headers.insert("x-custom".to_string(), Value::Str("stays".into()));
        let mut opts = BTreeMap::new();
        opts.insert("follow".to_string(), Value::Bool(true));
        opts.insert("headers".to_string(), Value::Map(headers));
        http_client("GET", vec![Value::Str(url), Value::Map(opts)], false)
    }

    #[test]
    fn cross_origin_redirect_credential_tushiriladi() {
        // issue #96: a redirect to a foreign origin (different port) — Authorization
        // and x-api-key must not reach it, while a plain header stays.
        let (port_b, hb) = spawn_test_server(vec![
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok".to_string(),
        ]);
        let (port_a, ha) = spawn_test_server(vec![format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/dest\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            port_b
        )]);

        let Ok(Value::Map(res)) =
            follow_get_with_credentials(format!("http://127.0.0.1:{}/start", port_a))
        else {
            panic!("request must have succeeded");
        };
        assert!(matches!(res.get("status"), Some(Value::Int(200))));

        // The first host (the original origin) gets the credentials in full.
        let req_a = ha.join().unwrap().remove(0).to_lowercase();
        assert!(req_a.contains("authorization: bearer secret"));
        assert!(req_a.contains("x-api-key: key"));
        // Credentials do not go to the foreign host, but a plain header does.
        let req_b = hb.join().unwrap().remove(0).to_lowercase();
        assert!(!req_b.contains("authorization"), "Authorization leaked");
        assert!(!req_b.contains("x-api-key"), "x-api-key leaked");
        assert!(req_b.contains("x-custom: stays"));
    }

    #[test]
    fn same_origin_redirect_credential_saqlanadi() {
        // On a same-origin redirect, credentials are not dropped — the fix only
        // affects a foreign host.
        let (port, h) = spawn_test_server(vec![
            "HTTP/1.1 302 Found\r\nLocation: /dest\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
                .to_string(),
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok".to_string(),
        ]);

        let Ok(Value::Map(res)) =
            follow_get_with_credentials(format!("http://127.0.0.1:{}/start", port))
        else {
            panic!("request must have succeeded");
        };
        assert!(matches!(res.get("status"), Some(Value::Int(200))));

        let captured = h.join().unwrap();
        let req2 = captured[1].to_lowercase();
        assert!(req2.contains("authorization: bearer secret"));
        assert!(req2.contains("x-api-key: key"));
    }

    #[test]
    fn https_connector_quriladi() {
        // pooled_http_client builds the https connector without panicking (the
        // rustls ring crypto provider is present, webpki-roots loads). A
        // deterministic check that works without a network — guards that the
        // HTTPS path builds (issue #14: not just http://, https:// too).
        let _client = pooled_http_client();
        // clone() reuses one pool — there must be no panic again.
        let _client2 = pooled_http_client();
    }

    #[test]
    fn opts_default_no_follow() {
        // If no options are given, redirects are not followed, limit 10.
        let o = parse_client_opts(None);
        assert!(!o.follow);
        assert_eq!(o.max, 10);
    }

    #[test]
    fn opts_follow_true() {
        let mut m = BTreeMap::new();
        m.insert("follow".to_string(), Value::Bool(true));
        m.insert("max".to_string(), Value::Int(3));
        let o = parse_client_opts(Some(&Value::Map(m)));
        assert!(o.follow);
        assert_eq!(o.max, 3);
    }

    #[test]
    fn opts_follow_falsey() {
        // follow:false and follow:nil — neither enables following.
        let mut m = BTreeMap::new();
        m.insert("follow".to_string(), Value::Bool(false));
        assert!(!parse_client_opts(Some(&Value::Map(m))).follow);
    }

    #[test]
    fn opts_headers_parse_qiladi() {
        // The headers map is read with str values (issue #34).
        let mut hm = BTreeMap::new();
        hm.insert(
            "x-api-key".to_string(),
            Value::Str("secret-val".to_string()),
        );
        hm.insert(
            "anthropic-version".to_string(),
            Value::Str("2023-06-01".to_string()),
        );
        let mut m = BTreeMap::new();
        m.insert("headers".to_string(), Value::Map(hm));
        let o = parse_client_opts(Some(&Value::Map(m)));
        assert_eq!(
            o.headers.get("x-api-key").map(|s| s.as_str()),
            Some("secret-val")
        );
        assert_eq!(
            o.headers.get("anthropic-version").map(|s| s.as_str()),
            Some("2023-06-01")
        );
    }

    #[test]
    fn opts_headers_str_bolmagan_qiymat_matnga_aylanadi() {
        // A non-str value (int) is converted to its text form; nil is dropped.
        let mut hm = BTreeMap::new();
        hm.insert("x-count".to_string(), Value::Int(42));
        hm.insert("x-skip".to_string(), Value::Nil);
        let mut m = BTreeMap::new();
        m.insert("headers".to_string(), Value::Map(hm));
        let o = parse_client_opts(Some(&Value::Map(m)));
        assert_eq!(o.headers.get("x-count").map(|s| s.as_str()), Some("42"));
        assert!(!o.headers.contains_key("x-skip"));
    }

    #[test]
    fn opts_default_headers_bosh() {
        // If no options are given, headers are empty.
        assert!(parse_client_opts(None).headers.is_empty());
    }

    // --- client timeout (issue #92) ---

    #[test]
    fn opts_default_timeout_30s() {
        // If no options are given, a default 30s timeout (protection against a stuck upstream).
        let o = parse_client_opts(None);
        assert_eq!(
            o.timeout,
            Some(Duration::from_secs(DEFAULT_CLIENT_TIMEOUT_SECS))
        );
    }

    #[test]
    fn opts_timeout_sozlanadi() {
        // `{timeout: N}` — N seconds.
        let mut m = BTreeMap::new();
        m.insert("timeout".to_string(), Value::Int(5));
        let o = parse_client_opts(Some(&Value::Map(m)));
        assert_eq!(o.timeout, Some(Duration::from_secs(5)));
    }

    #[test]
    fn opts_timeout_nol_ochiradi() {
        // `timeout: 0` (and negative) — no timeout (None). Only for a trusted upstream.
        let mut m = BTreeMap::new();
        m.insert("timeout".to_string(), Value::Int(0));
        assert_eq!(parse_client_opts(Some(&Value::Map(m))).timeout, None);
        let mut m2 = BTreeMap::new();
        m2.insert("timeout".to_string(), Value::Int(-1));
        assert_eq!(parse_client_opts(Some(&Value::Map(m2))).timeout, None);
    }

    #[test]
    fn http_get_qotgan_upstream_timeout_qaytaradi() {
        // Acceptance (issue #92): an upstream that accepts the connection but
        // DOES NOT RESPOND must not block the whole thread forever — with a short
        // timeout it must return an error. We open a listener, accept the
        // connection, but write nothing (simulating a slow/stuck server).
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            // Hold the connection, send no response.
            for stream in listener.incoming() {
                let _held = stream;
                std::thread::sleep(Duration::from_secs(10));
            }
        });
        let mut opts = BTreeMap::new();
        opts.insert("timeout".to_string(), Value::Int(1));
        let url = format!("http://{}/", addr);
        let res = http_client("GET", vec![Value::Str(url), Value::Map(opts)], false);
        match res {
            Err(Flow::Error(msg)) => {
                assert!(
                    msg.contains("timeout"),
                    "timeout error expected, got: {}",
                    msg
                )
            }
            Ok(_) => panic!("Ok not expected from a stuck upstream — must be a timeout"),
            Err(_) => panic!("Flow::Error(timeout) expected"),
        }
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

    // --- middleware prefix matching (issue #67) ---

    #[test]
    fn prefix_yulduz_aniq_prefiks_mos() {
        // "/api/*" → both "/api" itself and anything under it match.
        assert!(prefix_matches("/api/*", "/api"));
        assert!(prefix_matches("/api/*", "/api/users"));
        assert!(prefix_matches("/api/*", "/api/v1/bookings"));
    }

    #[test]
    fn prefix_yulduz_segment_chegarasi() {
        // "/apix" does NOT match "/api/*" — the prefix splits on a segment
        // boundary (otherwise "/api" would leak to other resources).
        assert!(!prefix_matches("/api/*", "/apix"));
        assert!(!prefix_matches("/api/*", "/ap"));
        assert!(!prefix_matches("/api/*", "/"));
    }

    #[test]
    fn prefix_yulduzsiz_aniq_mos() {
        // A pattern without "*" — only exact path matching.
        assert!(prefix_matches("/api/v1/users", "/api/v1/users"));
        assert!(!prefix_matches("/api/v1/users", "/api/v1"));
        assert!(!prefix_matches("/api/v1/users", "/api/v1/users/5"));
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

    #[tokio::test]
    async fn band_port_bind_xato_qaytaradi() {
        // If the port is busy, bind returns `Err` (issue #108) — not a silent
        // `return`. First we occupy a port (0 -> the OS picks a free one), then
        // try to bind the same port again: the exact same addr -> EADDRINUSE.
        let Ok(occupied) = bind(0).await else {
            panic!("bind to a free port must succeed");
        };
        let port = occupied.local_addr().unwrap().port();
        let res = bind(port).await;
        assert!(
            matches!(res, Err(Flow::Error(_))),
            "Err expected for a busy port"
        );
    }

    // --- rate-limit (issue #79) ---

    #[test]
    fn window_birligi_soniyaga_aylanadi() {
        // Canonical set: :sec/:min/:hr. An unknown unit is None.
        assert_eq!(window_to_secs("sec"), Some(1));
        assert_eq!(window_to_secs("min"), Some(60));
        assert_eq!(window_to_secs("hr"), Some(3600));
        assert_eq!(window_to_secs("day"), None);
    }

    #[test]
    fn limit_oyna_ichida_sanaydi_va_429_beradi() {
        // limit=3 — the first 3 requests pass (None), the 4th is blocked (Some).
        let state: LimitState = Arc::new(Mutex::new(LimitBucket::new()));
        assert!(check_and_count(&state, "t1", 3, 3600).is_none());
        assert!(check_and_count(&state, "t1", 3, 3600).is_none());
        assert!(check_and_count(&state, "t1", 3, 3600).is_none());
        let retry = check_and_count(&state, "t1", 3, 3600);
        assert!(retry.is_some(), "the 4th request must be blocked");
        // Retry-After is until the window ends — in the range [1, window_secs].
        let r = retry.unwrap();
        assert!((1..=3600).contains(&r), "Retry-After is sensible: {}", r);
    }

    #[test]
    fn limit_kalitlar_alohida_sanaladi() {
        // Each key (tenant/key) has its own counter — exhausting one does not affect another.
        let state: LimitState = Arc::new(Mutex::new(LimitBucket::new()));
        assert!(check_and_count(&state, "a", 1, 3600).is_none()); // a: 1st passes
        assert!(check_and_count(&state, "a", 1, 3600).is_some()); // a: 2nd blocked
        assert!(check_and_count(&state, "b", 1, 3600).is_none()); // b: separate bucket, passes
    }

    #[test]
    fn limit_yangi_oynada_tiklanadi() {
        // window_secs=1 — after one second a new window, the count resets to zero.
        let state: LimitState = Arc::new(Mutex::new(LimitBucket::new()));
        assert!(check_and_count(&state, "k", 1, 1).is_none());
        assert!(check_and_count(&state, "k", 1, 1).is_some()); // exhausted in this window
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(
            check_and_count(&state, "k", 1, 1).is_none(),
            "count must reset in a new window"
        );
    }

    #[test]
    fn limit_eski_oyna_kalitlari_tozalanadi() {
        // So that memory does not grow without bound (Codex review P2):
        // user-controlled keys must not pile up — old-window keys are removed in
        // the sweep. window_secs=1: write "old", let the window pass, then trigger
        // the sweep with SWEEP_EVERY operations.
        let state: LimitState = Arc::new(Mutex::new(LimitBucket::new()));
        check_and_count(&state, "old", 1000, 1);
        std::thread::sleep(std::time::Duration::from_millis(1100)); // next window
        for _ in 0..SWEEP_EVERY {
            check_and_count(&state, "new", 1_000_000, 1);
        }
        let bucket = state.lock().unwrap();
        assert!(
            !bucket.counts.contains_key("old"),
            "old window key must be swept"
        );
        assert!(
            bucket.counts.contains_key("new"),
            "current window key must remain"
        );
    }

    #[test]
    fn limit_parallel_atomik_sanaydi() {
        // Acceptance: counts correctly under parallel requests (no race).
        // 16 threads x 50 attempts = 800; exactly limit=100 of them MUST pass.
        use std::sync::atomic::{AtomicU32, Ordering};
        let state: LimitState = Arc::new(Mutex::new(LimitBucket::new()));
        let allowed = Arc::new(AtomicU32::new(0));
        let mut handles = vec![];
        for _ in 0..16 {
            let st = state.clone();
            let al = allowed.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..50 {
                    if check_and_count(&st, "k", 100, 3600).is_none() {
                        al.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(
            allowed.load(Ordering::SeqCst),
            100,
            "exactly limit=100 requests must pass (atomic counting)"
        );
    }

    #[test]
    fn fallback_kalit_ip_prefiksli() {
        // When the key is nil, req.ip is used, with the "ip:" prefix.
        let req = with_ctx(
            build_req(
                "GET".into(),
                "/".into(),
                String::new(),
                BTreeMap::new(),
                BTreeMap::new(),
                "203.0.113.7".into(),
                Bytes::new(),
                false,
                None,
            ),
            Arc::new(Mutex::new(BTreeMap::new())),
        );
        assert_eq!(client_fallback_key(&req), "ip:203.0.113.7");
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

    // --- http.static (issue #134) ---

    fn segv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn static_prefix_parse_va_moslik() {
        // "/" -> empty prefix (matches all paths); "/assets" is checked on a
        // segment boundary — "/assetsx" does NOT match.
        assert!(parse_static_prefix("/").is_empty());
        assert_eq!(parse_static_prefix("/assets"), segv(&["assets"]));
        assert_eq!(parse_static_prefix("/a/b/"), segv(&["a", "b"]));

        let pref = parse_static_prefix("/assets");
        assert!(strip_mount_prefix(&pref, &segv(&["assets", "app.css"])).is_some());
        assert!(strip_mount_prefix(&pref, &segv(&["assets"])).is_some());
        assert!(strip_mount_prefix(&pref, &segv(&["assetsx", "a.css"])).is_none());
        assert!(strip_mount_prefix(&pref, &segv(&["other"])).is_none());
        // The remainder — the file path after the prefix.
        assert_eq!(
            strip_mount_prefix(&pref, &segv(&["assets", "img", "a.png"])).unwrap(),
            segv(&["img", "a.png"])
        );
    }

    #[test]
    fn static_safe_join_traversalni_bloklaydi() {
        // Traversal protection is MANDATORY (issue #134): "..", ".", empty,
        // absolute, and backslash/NUL segments are rejected — you cannot escape
        // the directory. Percent-decode happens in the caller, so `%2e%2e` already
        // arrives here as ".." and is caught by this check.
        let dir = Path::new("/srv/public");
        assert!(safe_join(dir, &segv(&["..", "secret"])).is_none());
        assert!(safe_join(dir, &segv(&["a", "..", "b"])).is_none());
        assert!(safe_join(dir, &segv(&["."])).is_none());
        assert!(safe_join(dir, &segv(&[""])).is_none());
        assert!(safe_join(dir, &segv(&["a\\b"])).is_none());
        assert!(safe_join(dir, &segv(&["a\0b"])).is_none());
        assert!(safe_join(dir, &segv(&["/etc", "passwd"])).is_none());
        // Plain names — joined.
        let p = safe_join(dir, &segv(&["img", "a.png"])).unwrap();
        assert_eq!(p, PathBuf::from("/srv/public/img/a.png"));
        // Empty rest (the prefix itself was requested) — the directory itself.
        assert_eq!(safe_join(dir, &[]).unwrap(), PathBuf::from("/srv/public"));
    }

    #[test]
    fn static_mime_kengaytmadan() {
        // Content-Type is automatic from the extension; unknown -> octet-stream.
        assert_eq!(
            mime_for(Path::new("a/index.html")),
            "text/html; charset=utf-8"
        );
        assert_eq!(mime_for(Path::new("app.CSS")), "text/css; charset=utf-8");
        assert_eq!(
            mime_for(Path::new("app.js")),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(mime_for(Path::new("logo.svg")), "image/svg+xml");
        assert_eq!(mime_for(Path::new("a.png")), "image/png");
        assert_eq!(mime_for(Path::new("font.woff2")), "font/woff2");
        assert_eq!(mime_for(Path::new("data.bin")), "application/octet-stream");
        assert_eq!(
            mime_for(Path::new("noextension")),
            "application/octet-stream"
        );
    }

    // Splits the request path into segments by the same rule as try_serve_static
    // (percent-decode, %2F stays raw) — resolve_static now takes ready segments
    // (decoding in the caller, in one place with the prefix check).
    fn decode_segs(path: &str) -> Vec<String> {
        path_segments(path)
            .iter()
            .map(|s| percent_decode(s, true))
            .collect()
    }

    #[tokio::test]
    async fn static_resolve_uzun_prefiks_yutadi() {
        // "/" and "/assets" mounts together: /assets/a.css is served from the
        // folder of the longer prefix (the most specific mount wins).
        let root = std::env::temp_dir().join("fluxon_static_unit_1");
        std::fs::create_dir_all(root.join("dist")).unwrap();
        std::fs::create_dir_all(root.join("public")).unwrap();
        // The mount directory is canonicalized at registration (http_static) —
        // same in the test, otherwise the /tmp symlink on macOS breaks the comparison.
        let dist = std::fs::canonicalize(root.join("dist")).unwrap();
        let public = std::fs::canonicalize(root.join("public")).unwrap();
        std::fs::write(dist.join("a.css"), "dist css").unwrap();
        std::fs::write(public.join("a.css"), "public css").unwrap();
        std::fs::write(dist.join("index.html"), "<h1>spa</h1>").unwrap();

        let mounts = vec![
            StaticMount {
                prefix: vec![],
                dir: dist.clone(),
                spa: true,
            },
            StaticMount {
                prefix: vec!["assets".to_string()],
                dir: public.clone(),
                spa: false,
            },
        ];
        // /assets/a.css -> public (long prefix), /a.css -> dist (root mount).
        // len — the byte count from metadata (HEAD Content-Length is given with it).
        let (p, mime, len) = resolve_static(&mounts, &decode_segs("/assets/a.css"))
            .await
            .unwrap();
        assert_eq!(p, public.join("a.css"));
        assert_eq!(mime, "text/css; charset=utf-8");
        assert_eq!(len, "public css".len() as u64);
        let (p, _, len) = resolve_static(&mounts, &decode_segs("/a.css"))
            .await
            .unwrap();
        assert_eq!(p, dist.join("a.css"));
        assert_eq!(len, "dist css".len() as u64);
        // When a directory is requested, index.html.
        let (p, mime, _) = resolve_static(&mounts, &decode_segs("/")).await.unwrap();
        assert_eq!(p, dist.join("index.html"));
        assert_eq!(mime, "text/html; charset=utf-8");
        // A path not found — SPA fallback (root mount spa:true).
        let (p, _, _) = resolve_static(&mounts, &decode_segs("/no/such/page"))
            .await
            .unwrap();
        assert_eq!(p, dist.join("index.html"));
        // A file not found under /assets: the assets mount is not spa, but the
        // root SPA mount's prefix still matches — the fallback goes to it.
        let (p, _, _) = resolve_static(&mounts, &decode_segs("/assets/none.css"))
            .await
            .unwrap();
        assert_eq!(p, dist.join("index.html"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn static_resolve_traversal_404() {
        // `..` (xom yoki percent-encoded) mount katalogidan tashqariga olib
        // chiqmaydi — None (404), sirli fayl o'qilmaydi.
        let root = std::env::temp_dir().join("fluxon_static_unit_2");
        std::fs::create_dir_all(root.join("public")).unwrap();
        let public = std::fs::canonicalize(root.join("public")).unwrap();
        std::fs::write(public.join("ok.txt"), "ok").unwrap();
        std::fs::write(root.join("secret.txt"), "secret").unwrap();

        let mounts = vec![StaticMount {
            prefix: vec!["assets".to_string()],
            dir: public.clone(),
            spa: false,
        }];
        assert!(
            resolve_static(&mounts, &decode_segs("/assets/../secret.txt"))
                .await
                .is_none()
        );
        assert!(
            resolve_static(&mounts, &decode_segs("/assets/%2e%2e/secret.txt"))
                .await
                .is_none()
        );
        assert!(
            resolve_static(&mounts, &decode_segs("/assets/..%2Fsecret.txt"))
                .await
                .is_none()
        );
        assert!(
            resolve_static(&mounts, &decode_segs("/assets/ok.txt"))
                .await
                .is_some()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn static_symlink_ildizdan_chiqsa_404() {
        // The lexical guard (safe_join) does not see through symlinks: a
        // symlink inside the dir pointing OUTSIDE the root must not be served
        // (codex P2 — canonicalize + root check). A symlink pointing to a
        // target INSIDE the root is still served as before.
        let root = std::env::temp_dir().join("fluxon_static_unit_3");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("public")).unwrap();
        let public = std::fs::canonicalize(root.join("public")).unwrap();
        std::fs::write(root.join("secret.txt"), "SECRET").unwrap();
        std::fs::write(public.join("inner.txt"), "inner").unwrap();
        // Points outside: public/evil.txt -> ../secret.txt
        std::os::unix::fs::symlink(root.join("secret.txt"), public.join("evil.txt")).unwrap();
        // Points inside: public/alias.txt -> public/inner.txt
        std::os::unix::fs::symlink(public.join("inner.txt"), public.join("alias.txt")).unwrap();

        let mounts = vec![StaticMount {
            prefix: vec!["assets".to_string()],
            dir: public.clone(),
            spa: false,
        }];
        assert!(
            resolve_static(&mounts, &decode_segs("/assets/evil.txt"))
                .await
                .is_none(),
            "a symlink pointing outside the root must not be served"
        );
        let (p, _, _) = resolve_static(&mounts, &decode_segs("/assets/alias.txt"))
            .await
            .expect("a symlink inside the root must work");
        // Canonical path — the symlink target (the real file).
        assert_eq!(p, public.join("inner.txt"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
