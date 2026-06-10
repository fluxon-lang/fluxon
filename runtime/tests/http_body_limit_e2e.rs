// HTTP so'rov tanasi (body) o'lcham chegarasi end-to-end testi (issue #91).
//
// Chegarasiz `req.into_body().collect()` butun tanani xotiraga yig'ardi — mijoz
// ulkan body yuborib server xotirasini to'ldira olardi (DoS). Endi `http.serve`
// default 10 MiB chegara qo'yadi va `{max_body: N}` bilan sozlanadi; oshsa 413.
//
// `flux` binary'ni subprocess sifatida ishga tushirib, real HTTP POST bilan
// tekshiramiz:
//   - chegaradan kichik body  -> 200
//   - chegaradan katta body (to'g'ri Content-Length)  -> 413 (tez yo'l)
//   - Content-Length'siz (chunked) katta body  -> 413 (Limited oqim davomida)
//   - max_body: 0 -> chegara o'chiriladi (katta body o'tadi)
//
// Pattern http_ratelimit_e2e.rs dan: vaqtinchalik .fx fayl + subprocess + raw HTTP.

use std::io::Write;
use std::process::{Child, Command};
use std::time::Duration;

fn spawn_server(port: u16, script: &str) -> (Child, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("flux_body_test_{}.fx", port));
    let mut f = std::fs::File::create(&path).expect("temp fx yaratish");
    f.write_all(script.as_bytes()).expect("temp fx yozish");
    drop(f);

    let bin = env!("CARGO_BIN_EXE_flux");
    let child = Command::new(bin)
        .arg("run")
        .arg(&path)
        .spawn()
        .expect("flux serverini ishga tushirish");
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

// Raw HTTP/1.1 POST — Content-Length bilan. To'liq javob matnini qaytaradi.
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
    stream.read_to_end(&mut resp).await.expect("http o'qish");
    String::from_utf8_lossy(&resp).to_string()
}

// Raw HTTP/1.1 POST — Content-Length'siz, chunked transfer encoding bilan. Bu
// Content-Length tez yo'lini chetlab o'tadi, shuning uchun chegarani Limited
// oqim davomida majburlashi tekshiriladi. `body` bitta chunk sifatida yuboriladi.
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
    stream.read_to_end(&mut resp).await.expect("http o'qish");
    String::from_utf8_lossy(&resp).to_string()
}

// max_body: 64 bayt.
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

    // 20 bayt — 64 chegaradan kichik, 200 bo'lishi kerak.
    let resp = http_post(port, "/echo", &"x".repeat(20)).await;
    assert!(resp.contains("200"), "kichik body 200 kutilgan: {}", resp);

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn body_chegaradan_katta_413() {
    let port = 8532;
    let script = LIMIT_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // 200 bayt — 64 chegaradan katta, to'g'ri Content-Length bilan -> 413 (tez yo'l).
    let resp = http_post(port, "/echo", &"x".repeat(200)).await;
    assert!(
        resp.contains("413"),
        "katta body 413 (Payload Too Large) kutilgan: {}",
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

    // Content-Length'siz (chunked) 200 bayt — tez yo'l yo'q, Limited oqim
    // davomida chegarani majburlab 413 qaytarishi kerak.
    let resp = http_post_chunked(port, "/echo", &"x".repeat(200)).await;
    assert!(
        resp.contains("413"),
        "chunked katta body 413 kutilgan: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

// max_body: 0 -> chegara o'chiriladi; katta body ham o'tishi kerak.
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

    // 5000 bayt — chegara o'chirilgan, 200 bo'lishi kerak.
    let resp = http_post(port, "/echo", &"x".repeat(5000)).await;
    assert!(
        resp.contains("200"),
        "max_body:0 da katta body 200 kutilgan: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}
