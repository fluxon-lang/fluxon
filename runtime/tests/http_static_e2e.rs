// http.static end-to-end test (issue #134).
//
// We run the `fluxon` binary as a subprocess and check static file serving with
// real HTTP requests:
//   - a file under the prefix comes back with correct content + Content-Type
//   - when a directory is requested, index.html is served
//   - path traversal (`../`, percent-encoded too) is blocked (404)
//   - an explicit route wins over static (route priority)
//   - in SPA mode an unfound path falls back to index.html
//   - middleware (http.before) also protects a static path
//
// Pattern from http_cors_e2e.rs: temporary .fx file + subprocess + raw HTTP/1.1.

use std::io::Write;
use std::process::{Child, Command};
use std::time::Duration;

// Creates a static folder for the test: index.html, app.css, a file inside sub/,
// and secret.txt OUTSIDE the folder (the traversal target).
fn make_static_dir(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("fluxon_static_e2e_{}", tag));
    let public = root.join("public");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(public.join("sub")).expect("test papka yaratish");
    std::fs::write(public.join("index.html"), "<h1>home page</h1>").expect("index.html");
    std::fs::write(public.join("app.css"), "body{color:red}").expect("app.css");
    std::fs::write(public.join("sub").join("inner.txt"), "inner text").expect("inner.txt");
    std::fs::write(root.join("secret.txt"), "SECRET").expect("secret.txt");
    root
}

// Writes the script to a temporary file and starts the fluxon server.
fn spawn_server(port: u16, script: &str) -> (Child, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("fluxon_static_test_{}.fx", port));
    let mut f = std::fs::File::create(&path).expect("temp fx yaratish");
    f.write_all(script.as_bytes()).expect("write temp fx");
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

// Raw HTTP/1.1 GET -- the path is sent RAW (so the hyper client does not
// normalize `..`): the traversal tests require exactly the raw bytes.
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
    stream.read_to_end(&mut resp).await.expect("http read");
    String::from_utf8_lossy(&resp).to_string()
}

// Plain mount + explicit route (there is also an /assets/api.json route for the priority test).
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

    // CSS file -- content + Content-Type from the extension.
    let resp = http_get(port, "/assets/app.css").await;
    assert!(resp.contains("200"), "expected 200: {}", resp);
    assert!(
        resp.contains("body{color:red}"),
        "expected css content: {}",
        resp
    );
    assert!(
        resp.to_lowercase().contains("content-type: text/css"),
        "expected text/css: {}",
        resp
    );

    // A file in a subfolder also works.
    let resp = http_get(port, "/assets/sub/inner.txt").await;
    assert!(resp.contains("inner text"), "expected inner file: {}", resp);

    // When a directory is requested, index.html.
    let resp = http_get(port, "/assets").await;
    assert!(resp.contains("home page"), "expected index.html: {}", resp);
    assert!(
        resp.to_lowercase().contains("content-type: text/html"),
        "expected text/html: {}",
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

    // Raw `../`, percent-encoded `%2e%2e` and `..%2f` -- all 404, SECRET content
    // is never exposed.
    for p in [
        "/assets/../secret.txt",
        "/assets/%2e%2e/secret.txt",
        "/assets/..%2fsecret.txt",
        "/assets/sub/../../secret.txt",
    ] {
        let resp = http_get(port, p).await;
        assert!(
            !resp.contains("SECRET"),
            "traversal should not expose content ({}): {}",
            p,
            resp
        );
        assert!(resp.contains("404"), "expected 404 ({}): {}", p, resp);
    }

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn static_aniq_route_ustun() {
    let port = 8443;
    let root = make_static_dir("prio");
    // The static folder also has api.json -- but the route should win.
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
        "an explicit route should win over a static file: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&root);
}

// SPA mounted at root: an unfound path falls back to index.html, the API route works.
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

    // An existing file -- comes back as itself.
    let resp = http_get(port, "/app.css").await;
    assert!(resp.contains("body{color:red}"), "expected css: {}", resp);

    // Frontend route (no file) -- index.html fallback.
    let resp = http_get(port, "/profil/42").await;
    assert!(resp.contains("200"), "spa fallback expected 200: {}", resp);
    assert!(resp.contains("home page"), "expected index.html: {}", resp);

    // The API route wins over static.
    let resp = http_get(port, "/api/health").await;
    assert!(resp.contains("\"ok\":true"), "expected api route: {}", resp);

    // POST does not reach static (only GET/HEAD) -- 404.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("ulanish");
    let req = "POST /app.css HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    stream.write_all(req.as_bytes()).await.expect("yozish");
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.expect("read");
    let resp = String::from_utf8_lossy(&buf).to_string();
    assert!(
        resp.contains("404"),
        "POST to static expected 404: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&root);
}

// Middleware also protects a static path: auth required under /protected/*.
const MW_SCRIPT: &str = r#"
http.before "/protected/*" \req ->
  if !req.headers.x_token
    fail 401 "auth required"

http.static "/protected" "DIR/public"

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

    // Without a token -- middleware fail 401, file content is not exposed.
    let resp = http_get(port, "/protected/app.css").await;
    assert!(resp.contains("401"), "expected 401: {}", resp);
    assert!(
        !resp.contains("body{color:red}"),
        "file content should not be exposed without auth: {}",
        resp
    );

    // Without a token a MISSING file is also 401 -- not 404 (codex P2): otherwise
    // the status difference (401=exists, 404=missing) would leak protected file names.
    let resp = http_get(port, "/protected/missing-file.css").await;
    assert!(
        resp.contains("401"),
        "a missing file should also be 401 without auth (no name leak): {}",
        resp
    );

    // With a token -- the file comes back.
    let resp = http_get_with_header(port, "/protected/app.css", Some("X-Token: secret")).await;
    assert!(resp.contains("200"), "with token expected 200: {}", resp);
    assert!(resp.contains("body{color:red}"), "expected css: {}", resp);

    // With a token, a missing file -- now a real 404.
    let resp =
        http_get_with_header(port, "/protected/missing-file.css", Some("X-Token: secret")).await;
    assert!(
        resp.contains("404"),
        "a missing file with a token must 404: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn static_yoq_katalog_xato_beradi() {
    // A nonexistent directory -- the server exits with an error right at start
    // (fail fast), not a silent 404.
    let port = 8446;
    let script = format!("http.static \"/x\" \"/no/such/dir\"\nhttp.serve {}\n", port);
    let path = std::env::temp_dir().join(format!("fluxon_static_test_{}.fx", port));
    std::fs::write(&path, &script).expect("write temp fx");

    let bin = env!("CARGO_BIN_EXE_fluxon");
    let out = Command::new(bin)
        .arg("run")
        .arg(&path)
        .output()
        .expect("fluxon ishga tushirish");
    assert!(
        !out.status.success(),
        "start with a missing directory should fail"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("http.static"),
        "the error message should contain http.static: {}",
        err
    );

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn static_head_faylni_oqimasdan_javob_beradi() {
    // HEAD -- no body, but Content-Length is the real file size (from metadata,
    // without reading the file -- codex P2). Same Content-Type as GET.
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
    stream.read_to_end(&mut buf).await.expect("read");
    let resp = String::from_utf8_lossy(&buf).to_string();
    let low = resp.to_lowercase();

    assert!(resp.contains("200"), "HEAD expected 200: {}", resp);
    // "body{color:red}" -- 15 bytes: the size is in the header, but the content is absent.
    assert!(
        low.contains("content-length: 15"),
        "expected real Content-Length: {}",
        resp
    );
    assert!(
        low.contains("content-type: text/css"),
        "expected text/css: {}",
        resp
    );
    assert!(
        !resp.contains("body{color:red}"),
        "a HEAD response should have no body: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&root);
}
