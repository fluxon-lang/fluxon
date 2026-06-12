// http.static end-to-end testi (issue #134).
//
// `fluxon` binary'ni subprocess sifatida ishga tushirib, real HTTP so'rovlar
// bilan static fayl tarqatishni tekshiramiz:
//   - prefiks ostidagi fayl to'g'ri mazmun + Content-Type bilan keladi
//   - katalog so'ralganda index.html beriladi
//   - path traversal (`../`, percent-encoded ham) bloklanadi (404)
//   - aniq route static'dan ustun (route prioriteti)
//   - SPA rejimida topilmagan yo'l index.html'ga tushadi
//   - middleware (http.before) static yo'lni ham himoya qiladi
//
// Pattern http_cors_e2e.rs dan: vaqtinchalik .fx fayl + subprocess + raw HTTP/1.1.

use std::io::Write;
use std::process::{Child, Command};
use std::time::Duration;

// Test uchun static papka yasaydi: index.html, app.css, sub/ ichida fayl,
// va papkadan TASHQARIDA secret.txt (traversal nishoni).
fn make_static_dir(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("fluxon_static_e2e_{}", tag));
    let public = root.join("public");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(public.join("sub")).expect("test papka yaratish");
    std::fs::write(public.join("index.html"), "<h1>bosh sahifa</h1>").expect("index.html");
    std::fs::write(public.join("app.css"), "body{color:red}").expect("app.css");
    std::fs::write(public.join("sub").join("ichki.txt"), "ichki matn").expect("ichki.txt");
    std::fs::write(root.join("secret.txt"), "MAXFIY").expect("secret.txt");
    root
}

// Skriptni vaqtinchalik faylga yozib, fluxon serverini ishga tushiradi.
fn spawn_server(port: u16, script: &str) -> (Child, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("fluxon_static_test_{}.fx", port));
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

// Raw HTTP/1.1 GET — yo'l XOM yuboriladi (hyper klienti `..`ni normalize
// qilmasin deb): traversal testlari aynan xom baytlarni talab qiladi.
async fn http_get(port: u16, path: &str) -> String {
    http_get_with_header(port, path, None).await
}

async fn http_get_with_header(port: u16, path: &str, header: Option<&str>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("http ulanish");
    let extra = match header {
        Some(h) => format!("{}\r\n", h),
        None => String::new(),
    };
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\n{}Connection: close\r\n\r\n",
        path, extra
    );
    stream.write_all(req.as_bytes()).await.expect("http yozish");
    let mut resp = Vec::new();
    stream.read_to_end(&mut resp).await.expect("http o'qish");
    String::from_utf8_lossy(&resp).to_string()
}

// Oddiy mount + aniq route (prioritet testi uchun /assets/api.json route ham bor).
const BASIC_SCRIPT: &str = r#"
http.static "/assets" "DIR/public"

http.on :get "/assets/api.json" \req ->
  rep 200 {source: "route"}

http.serve PORT
"#;

#[tokio::test]
async fn static_fayl_mazmun_va_content_type() {
    let port = 8441;
    let root = make_static_dir("basic1");
    let script = BASIC_SCRIPT
        .replace("PORT", &port.to_string())
        .replace("DIR", &root.to_string_lossy());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // CSS fayl — mazmun + kengaytmadan Content-Type.
    let resp = http_get(port, "/assets/app.css").await;
    assert!(resp.contains("200"), "200 kutilgan: {}", resp);
    assert!(
        resp.contains("body{color:red}"),
        "css mazmuni kutilgan: {}",
        resp
    );
    assert!(
        resp.to_lowercase().contains("content-type: text/css"),
        "text/css kutilgan: {}",
        resp
    );

    // Ichki papkadagi fayl ham ishlaydi.
    let resp = http_get(port, "/assets/sub/ichki.txt").await;
    assert!(resp.contains("ichki matn"), "ichki fayl kutilgan: {}", resp);

    // Katalog so'ralganda index.html.
    let resp = http_get(port, "/assets").await;
    assert!(
        resp.contains("bosh sahifa"),
        "index.html kutilgan: {}",
        resp
    );
    assert!(
        resp.to_lowercase().contains("content-type: text/html"),
        "text/html kutilgan: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn static_traversal_bloklanadi() {
    let port = 8442;
    let root = make_static_dir("trav");
    let script = BASIC_SCRIPT
        .replace("PORT", &port.to_string())
        .replace("DIR", &root.to_string_lossy());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // Xom `../`, percent-encoded `%2e%2e` va `..%2f` — hammasi 404, MAXFIY
    // mazmun hech qachon chiqmaydi.
    for p in [
        "/assets/../secret.txt",
        "/assets/%2e%2e/secret.txt",
        "/assets/..%2fsecret.txt",
        "/assets/sub/../../secret.txt",
    ] {
        let resp = http_get(port, p).await;
        assert!(
            !resp.contains("MAXFIY"),
            "traversal mazmun chiqarmasligi kerak ({}): {}",
            p,
            resp
        );
        assert!(resp.contains("404"), "404 kutilgan ({}): {}", p, resp);
    }

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn static_aniq_route_ustun() {
    let port = 8443;
    let root = make_static_dir("prio");
    // Static papkada ham api.json bor — lekin route yutishi kerak.
    std::fs::write(
        root.join("public").join("api.json"),
        "{\"source\":\"file\"}",
    )
    .expect("api.json");
    let script = BASIC_SCRIPT
        .replace("PORT", &port.to_string())
        .replace("DIR", &root.to_string_lossy());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    let resp = http_get(port, "/assets/api.json").await;
    assert!(
        resp.contains("\"source\":\"route\""),
        "aniq route static fayldan ustun bo'lishi kerak: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&root);
}

// SPA mount root'da: topilmagan yo'l index.html'ga tushadi, API route ishlaydi.
const SPA_SCRIPT: &str = r#"
http.static "/" "DIR/public" {spa: true}

http.on :get "/api/health" \req ->
  rep 200 {ok: true}

http.serve PORT
"#;

#[tokio::test]
async fn static_spa_fallback() {
    let port = 8444;
    let root = make_static_dir("spa");
    let script = SPA_SCRIPT
        .replace("PORT", &port.to_string())
        .replace("DIR", &root.to_string_lossy());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // Mavjud fayl — o'zi keladi.
    let resp = http_get(port, "/app.css").await;
    assert!(resp.contains("body{color:red}"), "css kutilgan: {}", resp);

    // Frontend route (fayl yo'q) — index.html fallback.
    let resp = http_get(port, "/profil/42").await;
    assert!(resp.contains("200"), "spa fallback 200 kutilgan: {}", resp);
    assert!(
        resp.contains("bosh sahifa"),
        "index.html kutilgan: {}",
        resp
    );

    // API route static'dan ustun.
    let resp = http_get(port, "/api/health").await;
    assert!(resp.contains("\"ok\":true"), "api route kutilgan: {}", resp);

    // POST static'ka tushmaydi (faqat GET/HEAD) — 404.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("ulanish");
    let req = "POST /app.css HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    stream.write_all(req.as_bytes()).await.expect("yozish");
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.expect("o'qish");
    let resp = String::from_utf8_lossy(&buf).to_string();
    assert!(
        resp.contains("404"),
        "POST static'ka 404 kutilgan: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&root);
}

// Middleware static yo'lni ham himoya qiladi: /himoya/* ostida auth talab.
const MW_SCRIPT: &str = r#"
http.before "/himoya/*" \req ->
  if !req.headers.x_token
    fail 401 "auth kerak"

http.static "/himoya" "DIR/public"

http.serve PORT
"#;

#[tokio::test]
async fn static_middleware_himoya_qiladi() {
    let port = 8445;
    let root = make_static_dir("mw");
    let script = MW_SCRIPT
        .replace("PORT", &port.to_string())
        .replace("DIR", &root.to_string_lossy());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    // Token'siz — middleware fail 401, fayl mazmuni chiqmaydi.
    let resp = http_get(port, "/himoya/app.css").await;
    assert!(resp.contains("401"), "401 kutilgan: {}", resp);
    assert!(
        !resp.contains("body{color:red}"),
        "auth'siz fayl mazmuni chiqmasligi kerak: {}",
        resp
    );

    // Token bilan — fayl keladi.
    let resp = http_get_with_header(port, "/himoya/app.css", Some("X-Token: sir")).await;
    assert!(resp.contains("200"), "token bilan 200 kutilgan: {}", resp);
    assert!(resp.contains("body{color:red}"), "css kutilgan: {}", resp);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn static_yoq_katalog_xato_beradi() {
    // Mavjud bo'lmagan katalog — server start'dayoq xato bilan tugaydi
    // (fail fast), jim 404 emas.
    let port = 8446;
    let script = format!(
        "http.static \"/x\" \"/yoq/katalog/aslo\"\nhttp.serve {}\n",
        port
    );
    let path = std::env::temp_dir().join(format!("fluxon_static_test_{}.fx", port));
    std::fs::write(&path, &script).expect("temp fx yozish");

    let bin = env!("CARGO_BIN_EXE_fluxon");
    let out = Command::new(bin)
        .arg("run")
        .arg(&path)
        .output()
        .expect("fluxon ishga tushirish");
    assert!(
        !out.status.success(),
        "yo'q katalog bilan start muvaffaqiyatsiz bo'lishi kerak"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("http.static"),
        "xato xabarida http.static bo'lishi kerak: {}",
        err
    );

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn static_head_faylni_oqimasdan_javob_beradi() {
    // HEAD — tana yo'q, lekin Content-Length haqiqiy fayl hajmi (metadata'dan,
    // fayl o'qilmasdan — codex P2). GET bilan bir xil Content-Type.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let port = 8447;
    let root = make_static_dir("head");
    let script = BASIC_SCRIPT
        .replace("PORT", &port.to_string())
        .replace("DIR", &root.to_string_lossy());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("ulanish");
    let req = "HEAD /assets/app.css HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    stream.write_all(req.as_bytes()).await.expect("yozish");
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.expect("o'qish");
    let resp = String::from_utf8_lossy(&buf).to_string();
    let low = resp.to_lowercase();

    assert!(resp.contains("200"), "HEAD 200 kutilgan: {}", resp);
    // "body{color:red}" — 15 bayt: hajm header'da, mazmun esa yo'q.
    assert!(
        low.contains("content-length: 15"),
        "haqiqiy Content-Length kutilgan: {}",
        resp
    );
    assert!(
        low.contains("content-type: text/css"),
        "text/css kutilgan: {}",
        resp
    );
    assert!(
        !resp.contains("body{color:red}"),
        "HEAD javobida tana bo'lmasligi kerak: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&root);
}
