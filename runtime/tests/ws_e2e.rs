// WS battery end-to-end testi — haqiqiy WebSocket klient bilan.
//
// `flux` binary'ni subprocess sifatida ishga tushiramiz (ws_chat.fx serverini),
// keyin tokio-tungstenite klient bilan ulanib connect/message/room broadcast
// hayot tsiklini tekshiramiz. Bu unit test emas, to'liq oqim integratsiyasi:
// handshake -> :connect hello -> join -> say broadcast -> ikkinchi klientga yetib
// borishi.
//
// Port to'qnashuvidan qochish uchun har test alohida ws skriptini vaqtinchalik
// faylga yozadi va boshqa-boshqa portda ishlaydi.

use std::io::Write;
use std::process::{Child, Command};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

// Berilgan port uchun ws skriptini vaqtinchalik faylga yozib, flux serverini
// ishga tushiradi. Serverni o'chirish uchun `Child` qaytaradi.
fn spawn_server(port: u16, script: &str) -> (Child, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("flux_ws_test_{}.fx", port));
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

// Keyingi matn xabarini kutadi (ping/pong/binary'ni o'tkazib yuboradi). Timeout.
async fn next_text<S>(ws: &mut S) -> String
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let fut = async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(t))) => return t.to_string(),
                Some(Ok(_)) => continue,
                Some(Err(e)) => panic!("ws o'qish xatosi: {}", e),
                None => panic!("ulanish kutilmaganda yopildi"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(3), fut)
        .await
        .expect("xabar kutishda timeout")
}

const CHAT_SCRIPT: &str = r#"
use ws

ws.on :connect \conn ->
  ws.data.set conn :name "anon"
  ws.send conn (json.enc {t:"hello" id:conn.id})

ws.on :message \conn raw ->
  m = json.dec raw
  if m.t == "join"
    ws.data.set conn :name m.name
    ws.room.join conn m.room
    who = ws.room.members m.room
    ws.room.send m.room (json.enc {t:"joined" name:m.name online:who.len})
  elif m.t == "say"
    name = ws.data.get conn :name
    ws.room.send m.room (json.enc {t:"msg" from:name body:m.body})

ws.serve PORT
"#;

// Child drop bo'lganda jarayonni o'ldirish uchun guard.
struct Killer(Child);
impl Drop for Killer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn connect_hello_and_session() {
    let port = 9311;
    let script = CHAT_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    let url = format!("ws://127.0.0.1:{}", port);
    let (mut ws, _) = connect_async(&url).await.expect("ulanish");

    // :connect handler hello yuboradi (conn.id bilan).
    let hello = next_text(&mut ws).await;
    assert!(hello.contains("\"hello\""), "hello kutilgan: {}", hello);
    assert!(hello.contains("\"id\""), "conn.id kutilgan: {}", hello);

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn room_broadcast_reaches_other_client() {
    let port = 9312;
    let script = CHAT_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    let url = format!("ws://127.0.0.1:{}", port);
    let (mut a, _) = connect_async(&url).await.expect("klient A ulanish");
    let (mut b, _) = connect_async(&url).await.expect("klient B ulanish");

    // Ikkalasi ham hello oladi.
    let _ = next_text(&mut a).await;
    let _ = next_text(&mut b).await;

    // Ikkalasi ham "general" xonasiga qo'shiladi.
    a.send(Message::text(
        r#"{"t":"join","room":"general","name":"alfa"}"#,
    ))
    .await
    .unwrap();
    // A o'zining join-broadcast'ini oladi.
    let _ = next_text(&mut a).await;

    b.send(Message::text(
        r#"{"t":"join","room":"general","name":"beta"}"#,
    ))
    .await
    .unwrap();
    // B qo'shilgach, joined broadcast IKKALA klientga ketadi (online:2).
    let a_join2 = next_text(&mut a).await;
    let b_join2 = next_text(&mut b).await;
    assert!(
        a_join2.contains("\"online\":2"),
        "A online=2 kutdi: {}",
        a_join2
    );
    assert!(
        b_join2.contains("beta"),
        "B o'z joinini ko'rdi: {}",
        b_join2
    );

    // A "say" yuboradi -> B ham, A ham oladi (xonadagi hammaga).
    a.send(Message::text(
        r#"{"t":"say","room":"general","body":"salom"}"#,
    ))
    .await
    .unwrap();
    let b_msg = next_text(&mut b).await;
    assert!(b_msg.contains("\"msg\""), "B msg kutdi: {}", b_msg);
    assert!(b_msg.contains("salom"), "B body 'salom' kutdi: {}", b_msg);
    assert!(b_msg.contains("alfa"), "B from='alfa' kutdi: {}", b_msg);

    let _ = std::fs::remove_file(&path);
}

// HTTP + WS bir jarayonda: http.serve va ws.serve birga e'lon qilingan server.
// HTTP POST /vote -> handler ichidan ws.room.send "live" broadcast. WS klient
// shu broadcast'ni oladi. Bu issue #18 markazidagi cross-protocol oqim.
const POLL_SCRIPT: &str = r#"
ws.on :connect \conn ->
  ws.room.join conn "live"

http.on :post "/vote" \req ->
  msg = req.body.msg ?? "ovoz"
  ws.room.send "live" (json.enc {vote: msg})
  rep 200 {ok: true}

http.serve HTTP_PORT
ws.serve WS_PORT
"#;

// Raw HTTP/1.1 POST (reqwest'siz) — tokio TcpStream ustida. JSON body yuborib,
// status qatorini qaytaradi (tasdiq uchun "200" qidiriladi).
async fn http_post_json(port: u16, path: &str, body: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("http ulanish");
    let req = format!(
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
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

#[tokio::test]
async fn http_and_ws_serve_together_cross_protocol() {
    let ws_port = 9314;
    let http_port = 8314;
    let script = POLL_SCRIPT
        .replace("HTTP_PORT", &http_port.to_string())
        .replace("WS_PORT", &ws_port.to_string());
    // spawn_server skript yo'lini WS portga bog'lab nomlaydi — to'qnashuv yo'q.
    let (child, path) = spawn_server(ws_port, &script);
    let _killer = Killer(child);
    // Ikkala server ham ko'tarilishini kutamiz (bir jarayonda birga).
    wait_port(ws_port).await;
    wait_port(http_port).await;

    // WS klient ulanadi va "live" xonasiga qo'shiladi (:connect handler).
    let url = format!("ws://127.0.0.1:{}", ws_port);
    let (mut ws, _) = connect_async(&url).await.expect("ws ulanish");
    // room.join darhol bo'ladi; klientga xabar yuborilmaydi — kichik pauza.
    tokio::time::sleep(Duration::from_millis(150)).await;

    // HTTP POST yuboramiz — handler ichidan WS room'ga broadcast qo'zg'aladi.
    let resp = http_post_json(http_port, "/vote", r#"{"msg":"alfa"}"#).await;
    assert!(resp.contains("200"), "HTTP 200 kutilgan: {}", resp);

    // WS klient HTTP handler qo'zg'agan broadcast'ni oladi — birga ishlash isboti.
    let got = next_text(&mut ws).await;
    assert!(got.contains("\"vote\""), "vote broadcast kutilgan: {}", got);
    assert!(
        got.contains("alfa"),
        "broadcast body 'alfa' kutilgan: {}",
        got
    );

    let _ = std::fs::remove_file(&path);
}

// cron.run http.serve'dan OLDIN kelsa ham serverni bloklamasligi kerak (issue #42).
// Ilgari cron.run `loop { sleep }` bilan o'zidan keyingi http.serve'ni o'ldirardi —
// port hech qachon ochilmasdi. Endi cron.run deferred: scheduler fonda, http.serve
// ko'tariladi. Cron daqiqalik bo'lgani uchun bu yerda handler OTILISHINI emas,
// DEFERRED semantikani tekshiramiz: server javob bersa, cron.run bloklamagani isbot.
const CRON_HTTP_SCRIPT: &str = r#"
cron.on "* * * * *" \->
  log "tick"

http.on :post "/ping" \req ->
  rep 200 {ok:true}

cron.run
http.serve HTTP_PORT
"#;

#[tokio::test]
async fn cron_run_does_not_block_http_serve() {
    let http_port = 8316;
    let script = CRON_HTTP_SCRIPT.replace("HTTP_PORT", &http_port.to_string());
    // spawn_server skript yo'lini portga bog'lab nomlaydi — bu yerda http portni
    // identifikator sifatida beramiz (WS yo'q).
    let (child, path) = spawn_server(http_port, &script);
    let _killer = Killer(child);
    // cron.run bloklasa, http.serve hech qachon ishga tushmaydi -> wait_port panic.
    wait_port(http_port).await;

    // Server haqiqatan so'rovga javob beradi (cron.run undan keyin kelsa ham).
    let resp = http_post_json(http_port, "/ping", "").await;
    assert!(
        resp.contains("200") || resp.contains("ok"),
        "server javobi: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn disconnect_cleans_room_membership() {
    let port = 9313;
    let script = CHAT_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server(port, &script);
    let _killer = Killer(child);
    wait_port(port).await;

    let url = format!("ws://127.0.0.1:{}", port);
    let (mut a, _) = connect_async(&url).await.expect("A ulanish");
    let (mut b, _) = connect_async(&url).await.expect("B ulanish");
    let _ = next_text(&mut a).await;
    let _ = next_text(&mut b).await;

    a.send(Message::text(r#"{"t":"join","room":"r1","name":"a"}"#))
        .await
        .unwrap();
    let _ = next_text(&mut a).await;
    b.send(Message::text(r#"{"t":"join","room":"r1","name":"b"}"#))
        .await
        .unwrap();
    let _ = next_text(&mut a).await; // b ning joini (online:2)
    let _ = next_text(&mut b).await;

    // A uziladi -> server room a'zoligini tozalashi kerak.
    drop(a);
    tokio::time::sleep(Duration::from_millis(200)).await;

    // B yana say qiladi -> faqat B oladi (A yo'q), broadcast xatosiz ketadi.
    b.send(Message::text(r#"{"t":"say","room":"r1","body":"yana"}"#))
        .await
        .unwrap();
    let b_msg = next_text(&mut b).await;
    assert!(b_msg.contains("yana"), "B o'z xabarini oldi: {}", b_msg);

    let _ = std::fs::remove_file(&path);
}
