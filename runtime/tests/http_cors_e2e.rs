// HTTP CORS end-to-end test (issue #135).
//
// We run the `fluxon` binary as a subprocess and check `http.cors` with real
// HTTP requests:
//   - OPTIONS preflight automatically returns 204 + Access-Control-Allow-*
//   - a plain GET response also gets CORS headers added
//   - a disallowed origin does not get CORS headers
//   - a 404 response also gets CORS headers
//
// Pattern from http_middleware_e2e.rs: temporary .fx file + subprocess + raw HTTP/1.1.

use std::io::Write;
use std::process::{Child, Command};
use std::time::Duration;

// Writes the script to a temporary file and starts the fluxon server.
fn spawn_server(port: u16, script: &str) -> (Child, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("fluxon_cors_test_{}.fx", port));
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

// Raw HTTP/1.1 request, with an optional Origin header. Returns the full
// response text (status + headers + body) -- the test searches it for headers.
async fn http_request(port: u16, method: &str, path: &str, origin: Option<&str>) -> String {
    http_request_full(port, method, path, origin, false).await
}

// If `preflight: true`, adds an Access-Control-Request-Method header -- this is
// how a browser CORS preflight is identified (Fetch standard).
async fn http_request_full(
    port: u16,
    method: &str,
    path: &str,
    origin: Option<&str>,
    preflight: bool,
) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("http ulanish");
    let origin_line = match origin {
        Some(o) => format!("Origin: {}\r\n", o),
        None => String::new(),
    };
    let preflight_line = if preflight {
        "Access-Control-Request-Method: GET\r\n"
    } else {
        ""
    };
    let req = format!(
        "{} {} HTTP/1.1\r\nHost: 127.0.0.1\r\n{}{}Content-Length: 0\r\nConnection: close\r\n\r\n",
        method, path, origin_line, preflight_line
    );
    stream.write_all(req.as_bytes()).await.expect("http yozish");
    let mut resp = Vec::new();
    stream.read_to_end(&mut resp).await.expect("http read");
    String::from_utf8_lossy(&resp).to_string()
}

// Wildcard CORS (dev) -- open to everyone. There is also a plain OPTIONS handler:
// even with CORS enabled, a NON-preflight request must reach this handler.
const STAR_SCRIPT: &str = r#"
http.cors "*"

http.on :get "/data" \req ->
  rep 200 {items: [1 2 3]}

http.on :options "/data" \req ->
  rep 200 {custom_options: true}

http.serve PORT
"#;

#[tokio::test]
async fn cors_oddiy_javobga_header_qoshiladi() {
    let port = 8431;
    let script = STAR_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // Plain GET -- response body + Access-Control-Allow-Origin: * expected.
    let resp = http_request(port, "GET", "/data", Some("https://app.example.com")).await;
    assert!(resp.contains("200"), "expected 200: {}", resp);
    assert!(
        resp.to_lowercase()
            .contains("access-control-allow-origin: *"),
        "expected Allow-Origin: *: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn cors_preflight_options_avtomatik_javob() {
    let port = 8432;
    let script = STAR_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // OPTIONS /data + Access-Control-Request-Method (real preflight) -- no route
    // handler, but CORS preflight returns 204 + Allow-*.
    let resp = http_request_full(
        port,
        "OPTIONS",
        "/data",
        Some("https://app.example.com"),
        true,
    )
    .await;
    let low = resp.to_lowercase();
    assert!(
        resp.contains("204"),
        "preflight expected 204 No Content: {}",
        resp
    );
    assert!(
        low.contains("access-control-allow-origin: *"),
        "preflight expected Allow-Origin: {}",
        resp
    );
    assert!(
        low.contains("access-control-allow-methods:"),
        "preflight expected Allow-Methods: {}",
        resp
    );
    assert!(
        low.contains("access-control-max-age:"),
        "preflight expected Max-Age: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn cors_oddiy_options_handlerga_tushadi() {
    // CORS enabled, but a NON-preflight OPTIONS (no Access-Control-Request-Method)
    // -- must reach the user's `http.on :options "/data"` handler, NOT an empty
    // 204 (codex P2).
    let port = 8436;
    let script = STAR_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // Origin is present, but NO Access-Control-Request-Method -- this is not a preflight.
    let resp = http_request(port, "OPTIONS", "/data", Some("https://app.example.com")).await;
    assert!(resp.contains("200"), "handler expected 200: {}", resp);
    assert!(
        resp.contains("custom_options"),
        "a plain OPTIONS should reach the user handler: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn cors_404_ham_header_oladi() {
    let port = 8433;
    let script = STAR_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // A nonexistent path -- 404, but CORS headers must be present (so the browser
    // can read the error body).
    let resp = http_request(port, "GET", "/missing", Some("https://app.example.com")).await;
    assert!(resp.contains("404"), "expected 404: {}", resp);
    assert!(
        resp.to_lowercase()
            .contains("access-control-allow-origin: *"),
        "a 404 response should also get CORS headers: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

// Explicit origin list + credentials.
const LIST_SCRIPT: &str = r#"
http.cors ["https://app.example.com"] {creds: true}

http.on :get "/me" \req ->
  rep 200 {name: "Ali"}

http.serve PORT
"#;

#[tokio::test]
async fn cors_ruxsat_etilgan_origin_aks_ettiriladi() {
    let port = 8434;
    let script = LIST_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // An origin from the list -- reflected + Allow-Credentials: true.
    let resp = http_request(port, "GET", "/me", Some("https://app.example.com")).await;
    let low = resp.to_lowercase();
    assert!(
        low.contains("access-control-allow-origin: https://app.example.com"),
        "an allowed origin should be reflected: {}",
        resp
    );
    assert!(
        low.contains("access-control-allow-credentials: true"),
        "expected Allow-Credentials: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn cors_ruxsat_etilmagan_origin_header_olmaydi() {
    let port = 8435;
    let script = LIST_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // An origin not in the list -- the response is still 200 (from the server),
    // but NO CORS headers (the browser blocks the request).
    let resp = http_request(port, "GET", "/me", Some("https://evil.example.com")).await;
    assert!(
        !resp.to_lowercase().contains("access-control-allow-origin"),
        "a disallowed origin should NOT get CORS headers: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

// POST handler with a low max_body -- a large body returns 413.
const LIMIT_SCRIPT: &str = r#"
http.cors "*"

http.on :post "/upload" \req ->
  rep 201 {ok: true}

http.serve PORT {max_body: 10}
"#;

#[tokio::test]
async fn cors_413_xato_javob_ham_header_oladi() {
    // A body-read error response (413 Payload Too Large) never reaches the handler,
    // but with CORS enabled it must still get Access-Control-Allow-Origin -- the
    // docs say "added to every response" (codex P2).
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let port = 8437;
    let script = LIMIT_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // Content-Length larger than max_body (10) -- fast path returns 413 without reading the body.
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("ulanish");
    let req = "POST /upload HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://app.example.com\r\n\
               Content-Type: application/json\r\nContent-Length: 9999\r\nConnection: close\r\n\r\n";
    stream.write_all(req.as_bytes()).await.expect("yozish");
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.expect("read");
    let resp = String::from_utf8_lossy(&buf).to_string();

    assert!(resp.contains("413"), "expected 413: {}", resp);
    assert!(
        resp.to_lowercase()
            .contains("access-control-allow-origin: *"),
        "a 413 response should also get CORS headers: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}
