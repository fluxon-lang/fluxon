// HTTP middleware + request-scoped context end-to-end test (issue #67, #68).
//
// We run the `fluxon` binary as a subprocess and check the full flow with real
// HTTP requests:
//   - http.use \req -> ...      global middleware (for all routes)
//   - http.before "/api/*" ...  middleware by path prefix
//   - if middleware returns `fail`, the chain stops and the response returns immediately (401)
//   - middleware writes `req.ctx <- {...}`, the handler reads it via `req.ctx`
//
// Pattern from ws_e2e.rs: temporary .fx file + subprocess + raw HTTP/1.1.

use std::io::Write;
use std::process::{Child, Command};
use std::time::Duration;

// Writes the script to a temporary file and starts the fluxon server.
fn spawn_server(port: u16, script: &str) -> (Child, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("fluxon_mw_test_{}.fx", port));
    let mut f = std::fs::File::create(&path).expect("temp fx yaratish");
    f.write_all(script.as_bytes()).expect("temp fx yozish");
    drop(f);

    let bin = env!("CARGO_BIN_EXE_fluxon");
    let child = Command::new(bin)
        .arg("run")
        .arg(&path)
        .spawn()
        .expect("fluxon serverini ishga tushirish");
    (child, path)
}

// Waits until the port is LISTENing (server boot). Max ~3s.
async fn wait_port(port: u16) {
    for _ in 0..60 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("port {} ochilmadi", port);
}

// Guard that kills the process when Child is dropped.
struct Killer(Child);
impl Drop for Killer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// Raw HTTP/1.1 request. The Auth header is optional. Returns the full response
// text (status line + headers + body) -- the test searches it for status and body.
async fn http_request(
    port: u16,
    method: &str,
    path: &str,
    auth: Option<&str>,
    body: &str,
) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("http ulanish");
    let auth_line = match auth {
        Some(token) => format!("Authorization: {}\r\n", token),
        None => String::new(),
    };
    let req = format!(
        "{} {} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         {}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        method,
        path,
        auth_line,
        body.len(),
        body
    );
    stream.write_all(req.as_bytes()).await.expect("http yozish");
    let mut resp = Vec::new();
    stream.read_to_end(&mut resp).await.expect("http read");
    String::from_utf8_lossy(&resp).to_string()
}

// Auth middleware: checks the Authorization header. If absent, `fail 401`
// (the chain stops). If present, writes `req.ctx <- {tenant_id role}` -- the
// handler reads this context without recomputing it (issue #68). With http.before
// it only attaches to /api/* paths (issue #67). /health is unprotected.
const APP_SCRIPT: &str = r#"
http.before "/api/*" \req ->
  token = req.headers.authorization
  if !token
    fail 401 "auth required"
  # token in the form "Bearer t5-admin" -> we extract tenant/role (simplified)
  req.ctx <- {tenant_id: 5 role: "admin" token: token}

http.on :get "/health" \req ->
  rep 200 {ok: true}

http.on :get "/api/me" \req ->
  ctx = req.ctx
  rep 200 {tenant: ctx.tenant_id role: ctx.role}

http.serve PORT
"#;

#[tokio::test]
async fn middleware_auth_rad_etadi_401() {
    let port = 8401;
    let script = APP_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // Request to /api/me without auth -- the http.before middleware returns fail 401,
    // the handler is not called at all.
    let resp = http_request(port, "GET", "/api/me", None, "").await;
    assert!(
        resp.contains("401"),
        "/api/me without auth expected 401: {}",
        resp
    );
    assert!(
        resp.contains("auth required"),
        "expected fail message: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn middleware_ctx_handlerga_yetadi() {
    let port = 8402;
    let script = APP_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // Request with auth -- middleware writes req.ctx, the handler reads it.
    let resp = http_request(port, "GET", "/api/me", Some("Bearer t5"), "").await;
    assert!(resp.contains("200"), "with auth expected 200: {}", resp);
    // The handler returns tenant/role from the ctx set by the middleware.
    assert!(
        resp.contains("\"tenant\":5"),
        "ctx.tenant_id should reach the handler: {}",
        resp
    );
    assert!(
        resp.contains("\"role\":\"admin\""),
        "ctx.role should reach the handler: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn before_prefiks_himoyalanmagan_yulni_otkazadi() {
    let port = 8403;
    let script = APP_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // /health does NOT match "/api/*" -- the auth middleware does not run, 200 even without auth.
    let resp = http_request(port, "GET", "/health", None, "").await;
    assert!(
        resp.contains("200"),
        "/health expected 200 even without auth: {}",
        resp
    );
    assert!(resp.contains("\"ok\":true"), "/health javobi: {}", resp);

    let _ = std::fs::remove_file(&path);
}

// Even if the middleware rejects with `rep` instead of `fail`, the chain must
// stop (Codex P1). `rep` returns a successful {__resp:...} map (not a Flow), so
// handle_request detects it specially. If this did not work, the handler would
// still run and send its own response (auth would be bypassed).
const REP_GUARD_SCRIPT: &str = r#"
http.use \req ->
  if req.path == "/secret"
    rep 403 {error: "forbidden"}

http.on :get "/secret" \req ->
  rep 200 {leaked: true}

http.serve PORT
"#;

#[tokio::test]
async fn middleware_rep_zanjirni_toxtatadi() {
    let port = 8404;
    let script = REP_GUARD_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // The middleware returns rep 403 -- the handler (rep 200 leaked) must NOT run.
    let resp = http_request(port, "GET", "/secret", None, "").await;
    assert!(
        resp.contains("403"),
        "middleware expected rep 403: {}",
        resp
    );
    assert!(
        resp.contains("forbidden"),
        "expected middleware response: {}",
        resp
    );
    assert!(
        !resp.contains("leaked"),
        "handler should NOT have run (rep did not stop the chain): {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

// Middleware runs in DECLARATION ORDER -- even when use and before are mixed
// (Codex P2). Here before writes req.ctx FIRST, then the use declared afterward
// reads it and adds it to the /api/check response. If all use's ran before the
// before's (order broken), use would see an empty ctx.
const ORDER_SCRIPT: &str = r#"
http.before "/api/*" \req ->
  req.ctx <- {step: "before"}

http.use \req ->
  c = req.ctx
  req.ctx <- {step: c.step seen_by_use: true}

http.on :get "/api/check" \req ->
  ctx = req.ctx
  rep 200 {step: ctx.step seen: ctx.seen_by_use}

http.serve PORT
"#;

#[tokio::test]
async fn middleware_deklaratsiya_tartibi_saqlanadi() {
    let port = 8405;
    let script = ORDER_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // before (first) writes ctx -> use (after) reads it -> the handler sees both.
    let resp = http_request(port, "GET", "/api/check", None, "").await;
    assert!(resp.contains("200"), "expected 200: {}", resp);
    assert!(
        resp.contains("\"step\":\"before\""),
        "before should write ctx before use: {}",
        resp
    );
    assert!(
        resp.contains("\"seen\":true"),
        "use should run after before and see ctx: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}
