// HTTP client: http.get/post/put/del — pooled hyper client, redirect following
// with cross-origin credential dropping, timeouts, and option parsing.

use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;

use crate::builtins::{json_decode, json_encode};
use crate::interp::Flow;
use crate::value::Value;

use super::response::headers_to_map;

// Default timeout for the HTTP client (http.get/post/...) (issue #92). Without a
// timeout, a stuck upstream blocks the whole script FOREVER (or, if called inside
// a handler, that request thread). Default 30s; configured via `http.get url
// {timeout: N}` (seconds); `timeout: 0` — no timeout (only for trusted upstreams).
// The timeout covers the whole request: connect + send + response (including
// redirects) — even if it hangs at some stage, an error is returned once time runs out.
const DEFAULT_CLIENT_TIMEOUT_SECS: u64 = 30;

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
pub(crate) fn http_client(method: &str, args: Vec<Value>, has_body: bool) -> Result<Value, Flow> {
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
                    Err(e) => Value::Bytes(std::sync::Arc::new(e.into_bytes())),
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
}
