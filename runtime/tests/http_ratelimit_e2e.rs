// HTTP rate-limit (http.limit) end-to-end test (issue #79).
//
// We run the `fluxon` binary as a subprocess and check the full flow with real
// HTTP requests:
//   - http.limit N :min \req -> key  -- after N requests, 429 + Retry-After
//   - each key (tenant/api-key) is counted separately
//   - the path-scoped variant ("/api/*") only affects that prefix
//   - if the key is nil, falls back to req.ip
//
// Pattern from http_middleware_e2e.rs: temporary .fx file + subprocess + raw HTTP.

use std::io::Write;
use std::process::{Child, Command};
use std::time::Duration;

// Writes the script to a temporary file and starts the fluxon server.
fn spawn_server(port: u16, script: &str) -> (Child, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("fluxon_rl_test_{}.fx", port));
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

// Raw HTTP/1.1 GET. An optional single custom header (name, value) -- the key
// function reads it. Returns the full response text (status line + headers + body).
async fn http_get(port: u16, path: &str, header: Option<(&str, &str)>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("http ulanish");
    let header_line = match header {
        Some((k, v)) => format!("{}: {}\r\n", k, v),
        None => String::new(),
    };
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\n{}Connection: close\r\n\r\n",
        path, header_line
    );
    stream.write_all(req.as_bytes()).await.expect("http yozish");
    let mut resp = Vec::new();
    stream.read_to_end(&mut resp).await.expect("http read");
    String::from_utf8_lossy(&resp).to_string()
}

// Per-key limit: 3 requests/min by the x-client header. The 4th is 429 + Retry-After.
// A different x-client is a separate bucket -- its request passes.
const PERKEY_SCRIPT: &str = r#"
http.limit 3 :min \req -> req.headers.x_client

http.on :get "/ping" \req ->
  rep 200 {ok: true}

http.serve PORT
"#;

#[tokio::test]
async fn limit_perkey_429_va_retry_after() {
    let port = 8501;
    let script = PERKEY_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // 3 requests with key "a" -- all 200.
    for i in 1..=3 {
        let resp = http_get(port, "/ping", Some(("x-client", "a"))).await;
        assert!(
            resp.contains("200"),
            "{}-th request expected 200: {}",
            i,
            resp
        );
    }
    // 4th request -- limit exceeded, 429 + Retry-After header.
    let resp = http_get(port, "/ping", Some(("x-client", "a"))).await;
    assert!(resp.contains("429"), "4th request expected 429: {}", resp);
    assert!(
        resp.to_lowercase().contains("retry-after"),
        "expected Retry-After header: {}",
        resp
    );

    // A different key "b" -- separate count, should still pass.
    let resp = http_get(port, "/ping", Some(("x-client", "b"))).await;
    assert!(
        resp.contains("200"),
        "another key should be counted separately: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

// Path-scoped limit: only "/api/*" is limited; "/health" is unlimited.
const PATH_SCRIPT: &str = r#"
http.limit "/api/*" 2 :min \req -> req.headers.x_client

http.on :get "/api/data" \req ->
  rep 200 {ok: true}

http.on :get "/health" \req ->
  rep 200 {ok: true}

http.serve PORT
"#;

#[tokio::test]
async fn limit_path_scoped_faqat_prefiksga_tasir() {
    let port = 8502;
    let script = PATH_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // /api/data -- limit=2; 1,2 pass, the 3rd is 429.
    assert!(
        http_get(port, "/api/data", Some(("x-client", "z")))
            .await
            .contains("200")
    );
    assert!(
        http_get(port, "/api/data", Some(("x-client", "z")))
            .await
            .contains("200")
    );
    let resp = http_get(port, "/api/data", Some(("x-client", "z"))).await;
    assert!(
        resp.contains("429"),
        "/api/data 3rd request expected 429: {}",
        resp
    );

    // /health does NOT match "/api/*" -- unlimited, even many requests are 200.
    for _ in 0..5 {
        let resp = http_get(port, "/health", Some(("x-client", "z"))).await;
        assert!(
            resp.contains("200"),
            "/health should be rate-limit-free: {}",
            resp
        );
    }

    let _ = std::fs::remove_file(&path);
}

// Key nil (no header) -> falls back to the client IP. From one IP (127.0.0.1),
// 2 requests/min; the 3rd is 429.
const IP_FALLBACK_SCRIPT: &str = r#"
http.limit 2 :min \req -> req.headers.x_client

http.on :get "/ping" \req ->
  rep 200 {ok: true}

http.serve PORT
"#;

#[tokio::test]
async fn limit_nil_kalit_ip_fallback() {
    let port = 8503;
    let script = IP_FALLBACK_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // NO x-client header -> key nil -> counts by req.ip (127.0.0.1).
    assert!(http_get(port, "/ping", None).await.contains("200"));
    assert!(http_get(port, "/ping", None).await.contains("200"));
    let resp = http_get(port, "/ping", None).await;
    assert!(
        resp.contains("429"),
        "a request without a key should be limited by IP: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}
