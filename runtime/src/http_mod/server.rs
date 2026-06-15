// HTTP server: bind, the accept loop, the static-file path, and per-request
// handling (route match -> middleware chain -> handler in spawn_blocking).

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::TcpListener;

use crate::builtins::json_encode;
use crate::interp::{Flow, Interp};
use crate::value::Value;

use super::middleware::{Middleware, cors_finalize, cors_preflight_response, run_middleware_chain};
use super::request::{build_req, multipart_boundary, percent_decode, with_ctx};
use super::response::{
    bad_request, flow_to_response, headers_to_map, json_response, payload_too_large,
    value_to_response,
};
use super::routing::{match_route, path_segments, prefix_matches};
use super::static_files::{StaticMount, resolve_static, static_head_response, static_response};

// Default request body size limit for the HTTP server (issue #91). An unbounded
// `collect()` gathers the entire body into memory — a client can fill server
// memory by sending a huge body (DoS). Default 10 MiB; configured via `http.serve
// PORT {max_body: N}`. `max_body: 0` disables the limit (unbounded — use only
// behind a trusted, internal network).
pub(crate) const DEFAULT_MAX_BODY: usize = 10 * 1024 * 1024;

// HTTP server header-read timeout (issue #92). A slowloris-style connection may
// send headers very slowly (or not at all), holding the socket/task indefinitely.
// If hyper does not receive the headers fully within this period it closes the
// connection. (header_read_timeout only takes effect when Builder::timer is set.)
const DEFAULT_HEADER_READ_TIMEOUT_SECS: u64 = 30;

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
        .any(|m| super::static_files::strip_mount_prefix(&m.prefix, &segs).is_some())
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
