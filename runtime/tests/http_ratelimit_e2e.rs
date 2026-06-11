// HTTP rate-limit (http.limit) end-to-end testi (issue #79).
//
// `fluxon` binary'ni subprocess sifatida ishga tushirib, real HTTP so'rovlar bilan
// to'liq oqimni tekshiramiz:
//   - http.limit N :min \req -> kalit  — N so'rovdan keyin 429 + Retry-After
//   - har kalit (tenant/api-key) alohida sanaydi
//   - path-scoped variant ("/api/*") faqat o'sha prefiksga ta'sir qiladi
//   - kalit nil bo'lsa req.ip'ga fallback
//
// Pattern http_middleware_e2e.rs dan: vaqtinchalik .fx fayl + subprocess + raw HTTP.

use std::io::Write;
use std::process::{Child, Command};
use std::time::Duration;

// Skriptni vaqtinchalik faylga yozib, fluxon serverini ishga tushiradi.
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

// Raw HTTP/1.1 GET. Ixtiyoriy bitta custom header (nom, qiymat) — kalit funksiyasi
// shuni o'qiydi. To'liq javob matnini qaytaradi (status qatori + header + body).
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
    stream.read_to_end(&mut resp).await.expect("http o'qish");
    String::from_utf8_lossy(&resp).to_string()
}

// Per-key limit: x-client header bo'yicha 3 so'rov/min. 4-si 429 + Retry-After.
// Boshqa x-client alohida bucket — uning so'rovi o'tadi.
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

    // "a" kaliti bilan 3 ta so'rov — hammasi 200.
    for i in 1..=3 {
        let resp = http_get(port, "/ping", Some(("x-client", "a"))).await;
        assert!(resp.contains("200"), "{}-so'rov 200 kutilgan: {}", i, resp);
    }
    // 4-so'rov — limit oshdi, 429 + Retry-After header.
    let resp = http_get(port, "/ping", Some(("x-client", "a"))).await;
    assert!(resp.contains("429"), "4-so'rov 429 kutilgan: {}", resp);
    assert!(
        resp.to_lowercase().contains("retry-after"),
        "Retry-After header kutilgan: {}",
        resp
    );

    // Boshqa kalit "b" — alohida hisob, hali o'tishi kerak.
    let resp = http_get(port, "/ping", Some(("x-client", "b"))).await;
    assert!(
        resp.contains("200"),
        "boshqa kalit alohida sanalishi kerak: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

// Path-scoped limit: faqat "/api/*" cheklanadi; "/health" cheksiz.
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

    // /api/data — limit=2; 1,2 o'tadi, 3-si 429.
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
        "/api/data 3-so'rov 429 kutilgan: {}",
        resp
    );

    // /health "/api/*" ga mos EMAS — limitsiz, ko'p so'rov ham 200.
    for _ in 0..5 {
        let resp = http_get(port, "/health", Some(("x-client", "z"))).await;
        assert!(
            resp.contains("200"),
            "/health limitsiz bo'lishi kerak: {}",
            resp
        );
    }

    let _ = std::fs::remove_file(&path);
}

// Kalit nil (header yo'q) -> mijoz IP'siga fallback. Bir IP'dan (127.0.0.1) 2
// so'rov/min; 3-si 429.
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

    // x-client header YO'Q -> kalit nil -> req.ip (127.0.0.1) bo'yicha sanaydi.
    assert!(http_get(port, "/ping", None).await.contains("200"));
    assert!(http_get(port, "/ping", None).await.contains("200"));
    let resp = http_get(port, "/ping", None).await;
    assert!(
        resp.contains("429"),
        "kalitsiz so'rov IP bo'yicha cheklanishi kerak: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}
