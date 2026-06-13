// HTTP request body size limit end-to-end test (issue #91).
//
// Without a limit, `req.into_body().collect()` collected the whole body into
// memory -- a client could send a huge body and fill the server's memory (DoS).
// Now `http.serve` sets a default 10 MiB limit, configurable via `{max_body: N}`;
// if exceeded, 413.
//
// We run the `fluxon` binary as a subprocess and check with a real HTTP POST:
//   - body smaller than the limit  -> 200
//   - body larger than the limit (correct Content-Length)  -> 413 (fast path)
//   - large body without Content-Length (chunked)  -> 413 (during the Limited stream)
//   - max_body: 0 -> limit is disabled (large body passes through)
//
// Pattern from http_ratelimit_e2e.rs: temporary .fx file + subprocess + raw HTTP.

use std::io::Write;
use std::process::{Child, Command};
use std::time::Duration;

fn spawn_server(port: u16, script: &str) -> (Child, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("fluxon_body_test_{}.fx", port));
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

struct Killer(Child);
impl Drop for Killer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// Raw HTTP/1.1 POST -- with Content-Length. Returns the full response text.
async fn http_post(port: u16, path: &str, body: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("http ulanish");
    let req = format!(
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        path,
        body.len(),
        body
    );
    stream.write_all(req.as_bytes()).await.expect("http yozish");
    let mut resp = Vec::new();
    stream.read_to_end(&mut resp).await.expect("http read");
    String::from_utf8_lossy(&resp).to_string()
}

// Raw HTTP/1.1 POST -- without Content-Length, with chunked transfer encoding.
// This bypasses the Content-Length fast path, so it checks that the limit is
// enforced during the Limited stream. `body` is sent as a single chunk.
async fn http_post_chunked(port: u16, path: &str, body: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("http ulanish");
    let req = format!(
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\n\
         Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
        path,
        body.len(),
        body
    );
    stream.write_all(req.as_bytes()).await.expect("http yozish");
    let mut resp = Vec::new();
    stream.read_to_end(&mut resp).await.expect("http read");
    String::from_utf8_lossy(&resp).to_string()
}

// max_body: 64 bytes.
const LIMIT_SCRIPT: &str = r#"
http.on :post "/echo" \req ->
  rep 200 {ok: true}

http.serve PORT {max_body: 64}
"#;

#[tokio::test]
async fn body_chegaradan_kichik_otadi() {
    let port = 8531;
    let script = LIMIT_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // 20 bytes -- smaller than the 64 limit, should be 200.
    let resp = http_post(port, "/echo", &"x".repeat(20)).await;
    assert!(resp.contains("200"), "small body expected 200: {}", resp);

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn body_chegaradan_katta_413() {
    let port = 8532;
    let script = LIMIT_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // 200 bytes -- larger than the 64 limit, with correct Content-Length -> 413 (fast path).
    let resp = http_post(port, "/echo", &"x".repeat(200)).await;
    assert!(
        resp.contains("413"),
        "large body expected 413 (Payload Too Large): {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn chunked_katta_body_413() {
    let port = 8533;
    let script = LIMIT_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // 200 bytes without Content-Length (chunked) -- no fast path, must enforce
    // the limit during the Limited stream and return 413.
    let resp = http_post_chunked(port, "/echo", &"x".repeat(200)).await;
    assert!(
        resp.contains("413"),
        "chunked large body expected 413: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

// max_body: 0 -> limit is disabled; even a large body should pass through.
const UNLIMITED_SCRIPT: &str = r#"
http.on :post "/echo" \req ->
  rep 200 {ok: true}

http.serve PORT {max_body: 0}
"#;

#[tokio::test]
async fn max_body_nol_chegarani_ochiradi() {
    let port = 8534;
    let script = UNLIMITED_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // 5000 bytes -- limit is disabled, should be 200.
    let resp = http_post(port, "/echo", &"x".repeat(5000)).await;
    assert!(
        resp.contains("200"),
        "with max_body:0 large body expected 200: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}
