// HTTP CORS end-to-end testi (issue #135).
//
// `fluxon` binary'ni subprocess sifatida ishga tushirib, real HTTP so'rovlar bilan
// `http.cors` ni tekshiramiz:
//   - OPTIONS preflight avtomatik 204 + Access-Control-Allow-* qaytaradi
//   - oddiy GET javobiga ham CORS header qo'shiladi
//   - ruxsat etilmagan origin CORS header olmaydi
//   - 404 javob ham CORS header oladi
//
// Pattern http_middleware_e2e.rs dan: vaqtinchalik .fx fayl + subprocess + raw HTTP/1.1.

use std::io::Write;
use std::process::{Child, Command};
use std::time::Duration;

// Skriptni vaqtinchalik faylga yozib, fluxon serverini ishga tushiradi.
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

// Port LISTEN bo'lguncha kutadi (server boot). Maks ~3s.
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

// Child drop bo'lganda jarayonni o'ldirish guard'i.
struct Killer(Child);
impl Drop for Killer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// Raw HTTP/1.1 so'rov, ixtiyoriy Origin header bilan. To'liq javob matnini
// qaytaradi (status + header + body) — test header'larni undan qidiradi.
async fn http_request(port: u16, method: &str, path: &str, origin: Option<&str>) -> String {
    http_request_full(port, method, path, origin, false).await
}

// `preflight: true` bo'lsa Access-Control-Request-Method header qo'shadi —
// brauzer CORS preflight'i shu bilan belgilanadi (Fetch standarti).
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
    stream.read_to_end(&mut resp).await.expect("http o'qish");
    String::from_utf8_lossy(&resp).to_string()
}

// Wildcard CORS (dev) — hammaga ochiq. Oddiy OPTIONS handler'i ham bor:
// CORS yoqilgan bo'lsa ham preflight EMAS so'rov shu handler'ga tushishi kerak.
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

    // Oddiy GET — javob tanasi + Access-Control-Allow-Origin: * bo'lishi kerak.
    let resp = http_request(port, "GET", "/data", Some("https://app.example.com")).await;
    assert!(resp.contains("200"), "200 kutilgan: {}", resp);
    assert!(
        resp.to_lowercase()
            .contains("access-control-allow-origin: *"),
        "Allow-Origin: * kutilgan: {}",
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

    // OPTIONS /data + Access-Control-Request-Method (haqiqiy preflight) — route
    // handler yo'q, lekin CORS preflight 204 + Allow-* qaytaradi.
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
        "preflight 204 No Content kutilgan: {}",
        resp
    );
    assert!(
        low.contains("access-control-allow-origin: *"),
        "preflight Allow-Origin kutilgan: {}",
        resp
    );
    assert!(
        low.contains("access-control-allow-methods:"),
        "preflight Allow-Methods kutilgan: {}",
        resp
    );
    assert!(
        low.contains("access-control-max-age:"),
        "preflight Max-Age kutilgan: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn cors_oddiy_options_handlerga_tushadi() {
    // CORS yoqilgan, lekin preflight EMAS OPTIONS (Access-Control-Request-Method
    // yo'q) — foydalanuvchining `http.on :options "/data"` handler'iga tushishi
    // kerak, bo'sh 204 EMAS (codex P2).
    let port = 8436;
    let script = STAR_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // Origin bor, lekin Access-Control-Request-Method YO'Q — bu preflight emas.
    let resp = http_request(port, "OPTIONS", "/data", Some("https://app.example.com")).await;
    assert!(resp.contains("200"), "handler 200 kutilgan: {}", resp);
    assert!(
        resp.contains("custom_options"),
        "oddiy OPTIONS foydalanuvchi handler'iga tushishi kerak: {}",
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

    // Mavjud bo'lmagan yo'l — 404, lekin CORS header bo'lishi kerak (brauzer
    // xato tanasini o'qiy olsin).
    let resp = http_request(port, "GET", "/yoq", Some("https://app.example.com")).await;
    assert!(resp.contains("404"), "404 kutilgan: {}", resp);
    assert!(
        resp.to_lowercase()
            .contains("access-control-allow-origin: *"),
        "404 javob ham CORS header olishi kerak: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

// Aniq origin ro'yxati + credentials.
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

    // Ro'yxatdagi origin — aks ettiriladi + Allow-Credentials: true.
    let resp = http_request(port, "GET", "/me", Some("https://app.example.com")).await;
    let low = resp.to_lowercase();
    assert!(
        low.contains("access-control-allow-origin: https://app.example.com"),
        "ruxsat etilgan origin aks ettirilishi kerak: {}",
        resp
    );
    assert!(
        low.contains("access-control-allow-credentials: true"),
        "Allow-Credentials kutilgan: {}",
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

    // Ro'yxatda yo'q origin — javob baribir 200 (server tomonidan), lekin
    // CORS header YO'Q (brauzer so'rovni bloklaydi).
    let resp = http_request(port, "GET", "/me", Some("https://evil.example.com")).await;
    assert!(
        !resp.to_lowercase().contains("access-control-allow-origin"),
        "ruxsat etilmagan origin CORS header OLMASLIGI kerak: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

// Past max_body bilan POST handler — katta tana 413 qaytaradi.
const LIMIT_SCRIPT: &str = r#"
http.cors "*"

http.on :post "/upload" \req ->
  rep 201 {ok: true}

http.serve PORT {max_body: 10}
"#;

#[tokio::test]
async fn cors_413_xato_javob_ham_header_oladi() {
    // Body-read xato javobi (413 Payload Too Large) handler'gacha yetmaydi, lekin
    // CORS yoqilgan bo'lsa baribir Access-Control-Allow-Origin olishi kerak —
    // hujjat "har javobga qo'shiladi" deydi (codex P2).
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let port = 8437;
    let script = LIMIT_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // Content-Length max_body (10) dan katta — tez yo'l tananing o'qimasdan 413.
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("ulanish");
    let req = "POST /upload HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://app.example.com\r\n\
               Content-Type: application/json\r\nContent-Length: 9999\r\nConnection: close\r\n\r\n";
    stream.write_all(req.as_bytes()).await.expect("yozish");
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.expect("o'qish");
    let resp = String::from_utf8_lossy(&buf).to_string();

    assert!(resp.contains("413"), "413 kutilgan: {}", resp);
    assert!(
        resp.to_lowercase()
            .contains("access-control-allow-origin: *"),
        "413 javob ham CORS header olishi kerak: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}
