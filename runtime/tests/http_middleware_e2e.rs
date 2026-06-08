// HTTP middleware + request-scoped context end-to-end testi (issue #67, #68).
//
// `flux` binary'ni subprocess sifatida ishga tushirib, real HTTP so'rovlar bilan
// to'liq oqimni tekshiramiz:
//   - http.use \req -> ...      global middleware (barcha route'larga)
//   - http.before "/api/*" ...  yo'l prefiks bo'yicha middleware
//   - middleware `fail` qaytarsa zanjir to'xtaydi, javob darrov qaytadi (401)
//   - middleware `req.ctx <- {...}` yozadi, handler `req.ctx` orqali o'qiydi
//
// Pattern ws_e2e.rs dan: vaqtinchalik .fx fayl + subprocess + raw HTTP/1.1.

use std::io::Write;
use std::process::{Child, Command};
use std::time::Duration;

// Skriptni vaqtinchalik faylga yozib, flux serverini ishga tushiradi.
fn spawn_server(port: u16, script: &str) -> (Child, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("flux_mw_test_{}.fx", port));
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

// Raw HTTP/1.1 so'rov. Auth header ixtiyoriy. To'liq javob matnini qaytaradi
// (status qatori + header + body) — test status va body'ni undan qidiradi.
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
    stream.read_to_end(&mut resp).await.expect("http o'qish");
    String::from_utf8_lossy(&resp).to_string()
}

// Auth middleware: Authorization header'ni tekshiradi. Yo'q bo'lsa `fail 401`
// (zanjir to'xtaydi). Bor bo'lsa `req.ctx <- {tenant_id role}` yozadi — handler
// shu kontekstni qayta hisoblamasdan o'qiydi (issue #68). http.before bilan
// faqat /api/* yo'llariga ulanadi (issue #67). /health himoyalanmagan.
const APP_SCRIPT: &str = r#"
http.before "/api/*" \req ->
  token = req.headers.authorization
  if !token
    fail 401 "auth kerak"
  # token "Bearer t5-admin" ko'rinishida -> tenant/role ajratamiz (soddalashtirilgan)
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

    // /api/me ga auth'siz so'rov — http.before middleware fail 401 qaytaradi,
    // handler umuman chaqirilmaydi.
    let resp = http_request(port, "GET", "/api/me", None, "").await;
    assert!(
        resp.contains("401"),
        "auth'siz /api/me 401 kutilgan: {}",
        resp
    );
    assert!(
        resp.contains("auth kerak"),
        "fail xabari kutilgan: {}",
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

    // Auth bilan so'rov — middleware req.ctx yozadi, handler o'qiydi.
    let resp = http_request(port, "GET", "/api/me", Some("Bearer t5"), "").await;
    assert!(resp.contains("200"), "auth bilan 200 kutilgan: {}", resp);
    // Handler middleware qo'ygan ctx'dan tenant/role qaytaradi.
    assert!(
        resp.contains("\"tenant\":5"),
        "ctx.tenant_id handler'ga yetishi kerak: {}",
        resp
    );
    assert!(
        resp.contains("\"role\":\"admin\""),
        "ctx.role handler'ga yetishi kerak: {}",
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

    // /health "/api/*" ga mos EMAS — auth middleware ishlamaydi, auth'siz ham 200.
    let resp = http_request(port, "GET", "/health", None, "").await;
    assert!(
        resp.contains("200"),
        "/health auth'siz ham 200 kutilgan: {}",
        resp
    );
    assert!(resp.contains("\"ok\":true"), "/health javobi: {}", resp);

    let _ = std::fs::remove_file(&path);
}

// Middleware `fail` o'rniga `rep` bilan rad etsa ham zanjir to'xtashi kerak
// (Codex P1). `rep` muvaffaqiyatli {__resp:...} map qaytaradi (Flow emas), shuning
// uchun handle_request uni alohida aniqlaydi. Bu ishlamasa, handler baribir
// ishlab o'z javobini yuborardi (auth chetlab o'tilardi).
const REP_GUARD_SCRIPT: &str = r#"
http.use \req ->
  if req.path == "/secret"
    rep 403 {error: "taqiqlangan"}

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

    // Middleware rep 403 qaytaradi — handler (rep 200 leaked) ISHLAMASLIGI kerak.
    let resp = http_request(port, "GET", "/secret", None, "").await;
    assert!(
        resp.contains("403"),
        "middleware rep 403 kutilgan: {}",
        resp
    );
    assert!(
        resp.contains("taqiqlangan"),
        "middleware javobi kutilgan: {}",
        resp
    );
    assert!(
        !resp.contains("leaked"),
        "handler ISHLAMASLIGI kerak edi (rep zanjirni to'xtatmadi): {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

// Middleware'lar DEKLARATSIYA TARTIBIDA ishlaydi — use va before aralashganda ham
// (Codex P2). Bu yerda before AVVAL req.ctx ga yozadi, keyin e'lon qilingan use
// uni o'qib /api/check javobiga qo'shadi. Agar barcha use'lar before'dan oldin
// ketsa (tartib buzilsa), use bo'sh ctx ko'rardi.
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

    // before (avval) ctx yozadi -> use (keyin) uni o'qiydi -> handler ikkalasini ko'radi.
    let resp = http_request(port, "GET", "/api/check", None, "").await;
    assert!(resp.contains("200"), "200 kutilgan: {}", resp);
    assert!(
        resp.contains("\"step\":\"before\""),
        "before ctx use'dan oldin yozishi kerak: {}",
        resp
    );
    assert!(
        resp.contains("\"seen\":true"),
        "use before'dan keyin ishlab ctx'ni ko'rishi kerak: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}
