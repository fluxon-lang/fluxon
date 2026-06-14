// WS battery end-to-end test -- with a real WebSocket client.
//
// We run the `fluxon` binary as a subprocess (the ws_chat.fx server), then
// connect with a tokio-tungstenite client and check the connect/message/room
// broadcast life cycle. This is not a unit test, it's a full-flow integration:
// handshake -> :connect hello -> join -> say broadcast -> reaching the second
// client.
//
// To avoid port collisions, each test writes its own ws script to a temporary
// file and runs on a different port.

use std::io::Write;
use std::process::{Child, Command};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

// Writes the ws script for the given port to a temporary file and starts the
// fluxon server. Returns the `Child` so the server can be shut down.
fn spawn_server(port: u16, script: &str) -> (Child, std::path::PathBuf) {
    spawn_server_env(port, script, &[])
}

// Variant of `spawn_server` that supplies env variables. The env affects only
// this subprocess (no race with other tests).
fn spawn_server_env(port: u16, script: &str, env: &[(&str, &str)]) -> (Child, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("fluxon_ws_test_{}.fx", port));
    let mut f = std::fs::File::create(&path).expect("temp fx yaratish");
    f.write_all(script.as_bytes()).expect("temp fx yozish");
    drop(f);

    let bin = env!("CARGO_BIN_EXE_fluxon");
    let mut cmd = Command::new(bin);
    cmd.arg("run").arg(&path);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let child = cmd.spawn().expect("fluxon serverini ishga tushirish");
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

// Waits for the next text message (skips ping/pong/binary). With timeout.
async fn next_text<S>(ws: &mut S) -> String
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    next_text_within(ws, Duration::from_secs(3)).await
}

// Variant of `next_text` that supplies the wait duration (slow handler test).
async fn next_text_within<S>(ws: &mut S, dur: Duration) -> String
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let fut = async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(t))) => return t.to_string(),
                Some(Ok(_)) => continue,
                Some(Err(e)) => panic!("ws read error: {}", e),
                None => panic!("connection closed unexpectedly"),
            }
        }
    };
    tokio::time::timeout(dur, fut)
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

// Guard to kill the process when Child is dropped.
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

    // The :connect handler sends hello (with conn.id).
    let hello = next_text(&mut ws).await;
    assert!(hello.contains("\"hello\""), "expected hello: {}", hello);
    assert!(hello.contains("\"id\""), "expected conn.id: {}", hello);

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

    // Both receive hello.
    let _ = next_text(&mut a).await;
    let _ = next_text(&mut b).await;

    // Both join the "general" room.
    a.send(Message::text(
        r#"{"t":"join","room":"general","name":"alfa"}"#,
    ))
    .await
    .unwrap();
    // A receives its own join broadcast.
    let _ = next_text(&mut a).await;

    b.send(Message::text(
        r#"{"t":"join","room":"general","name":"beta"}"#,
    ))
    .await
    .unwrap();
    // After B joins, the joined broadcast goes to BOTH clients (online:2).
    let a_join2 = next_text(&mut a).await;
    let b_join2 = next_text(&mut b).await;
    assert!(
        a_join2.contains("\"online\":2"),
        "A expected online=2: {}",
        a_join2
    );
    assert!(b_join2.contains("beta"), "B saw its own join: {}", b_join2);

    // A sends "say" -> both B and A receive it (to everyone in the room).
    a.send(Message::text(
        r#"{"t":"say","room":"general","body":"hello"}"#,
    ))
    .await
    .unwrap();
    let b_msg = next_text(&mut b).await;
    assert!(b_msg.contains("\"msg\""), "B expected msg: {}", b_msg);
    assert!(
        b_msg.contains("hello"),
        "B expected body 'hello': {}",
        b_msg
    );
    assert!(b_msg.contains("alfa"), "B expected from='alfa': {}", b_msg);

    let _ = std::fs::remove_file(&path);
}

// HTTP + WS in one process: a server with http.serve and ws.serve declared
// together. HTTP POST /vote -> from inside the handler, a ws.room.send "live"
// broadcast. The WS client receives this broadcast. This is the cross-protocol
// flow at the center of issue #18.
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

// Raw HTTP/1.1 POST (without reqwest) -- over a tokio TcpStream. Sends a JSON
// body and returns the status line (search for "200" to assert).
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
    stream.read_to_end(&mut resp).await.expect("http read");
    String::from_utf8_lossy(&resp).to_string()
}

#[tokio::test]
async fn http_and_ws_serve_together_cross_protocol() {
    let ws_port = 9314;
    let http_port = 8314;
    let script = POLL_SCRIPT
        .replace("HTTP_PORT", &http_port.to_string())
        .replace("WS_PORT", &ws_port.to_string());
    // spawn_server names the script path by the WS port -- no collision.
    let (child, path) = spawn_server(ws_port, &script);
    let _killer = Killer(child);
    // Wait for both servers to come up (together in one process).
    wait_port(ws_port).await;
    wait_port(http_port).await;

    // The WS client connects and joins the "live" room (:connect handler).
    let url = format!("ws://127.0.0.1:{}", ws_port);
    let (mut ws, _) = connect_async(&url).await.expect("ws ulanish");
    // room.join happens immediately; no message is sent to the client -- small pause.
    tokio::time::sleep(Duration::from_millis(150)).await;

    // We send an HTTP POST -- from inside the handler a broadcast to the WS room is triggered.
    let resp = http_post_json(http_port, "/vote", r#"{"msg":"alfa"}"#).await;
    assert!(resp.contains("200"), "expected HTTP 200: {}", resp);

    // The WS client receives the broadcast triggered by the HTTP handler -- proof they work together.
    let got = next_text(&mut ws).await;
    assert!(got.contains("\"vote\""), "expected vote broadcast: {}", got);
    assert!(
        got.contains("alfa"),
        "expected broadcast body 'alfa': {}",
        got
    );

    let _ = std::fs::remove_file(&path);
}

// cron.run must not block the server even if it comes BEFORE http.serve (issue #42).
// Previously cron.run with `loop { sleep }` killed the http.serve that followed it --
// the port never opened. Now cron.run is deferred: the scheduler runs in the
// background, http.serve comes up. Since cron is minute-granular, here we check
// the DEFERRED semantics, not that the handler FIRES: if the server responds,
// that proves cron.run did not block.
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
    // spawn_server names the script path by the port -- here we pass the http port
    // as the identifier (no WS).
    let (child, path) = spawn_server(http_port, &script);
    let _killer = Killer(child);
    // If cron.run blocks, http.serve never starts -> wait_port panics.
    wait_port(http_port).await;

    // The server really responds to the request (even if cron.run comes after it).
    let resp = http_post_json(http_port, "/ping", "").await;
    assert!(
        resp.contains("200") || resp.contains("ok"),
        "server javobi: {}",
        resp
    );

    let _ = std::fs::remove_file(&path);
}

// The server sends a periodic ping itself -- to detect half-open (dead)
// connections (issue #107). We speed up the ping interval with
// `FLUXON_WS_PING_SECS=1` and check that the client receives a Ping frame within ~1s.
const PING_SCRIPT: &str = r#"
use ws

ws.on :connect \conn ->
  log "ulandi"

ws.serve PORT
"#;

#[tokio::test]
async fn server_sends_periodic_ping() {
    let port = 9315;
    let script = PING_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server_env(port, &script, &[("FLUXON_WS_PING_SECS", "1")]);
    let _killer = Killer(child);
    wait_port(port).await;

    let url = format!("ws://127.0.0.1:{}", port);
    let (mut ws, _) = connect_async(&url).await.expect("ulanish");

    // Within ~1s the server must send a ping. The Ping frame reaches the client too
    // (tokio-tungstenite replies pong automatically, but still surfaces the frame).
    let fut = async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Ping(_))) => return,
                Some(Ok(_)) => continue,
                Some(Err(e)) => panic!("ws read error: {}", e),
                None => panic!("connection closed unexpectedly"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .expect("server davriy ping yubormadi");

    let _ = std::fs::remove_file(&path);
}

// When a long handler (exceeding the ping interval) blocks the select loop, the
// missed ping ticks must not burst and wrongly close the connection
// (MissedTickBehavior::Delay). The handler blocks via `time.sleep` -- it runs in
// spawn_blocking, i.e. it holds up the reader loop's await.
const SLOW_HANDLER_SCRIPT: &str = r#"
use ws

ws.on :message \conn msg ->
  if msg == "slow"
    time.sleep 2.5
  ws.send conn "ok:${msg}"

ws.serve PORT
"#;

#[tokio::test]
async fn long_handler_does_not_kill_connection() {
    let port = 9316;
    let script = SLOW_HANDLER_SCRIPT.replace("PORT", &port.to_string());
    // Ping interval 1s: the 2.5s handler exceeds 2 intervals -> burst risk.
    let (child, path) = spawn_server_env(port, &script, &[("FLUXON_WS_PING_SECS", "1")]);
    let _killer = Killer(child);
    wait_port(port).await;

    let url = format!("ws://127.0.0.1:{}", port);
    let (mut ws, _) = connect_async(&url).await.expect("ulanish");

    // "slow" -- the handler blocks 2.5s, then returns a response.
    ws.send(Message::text("slow")).await.unwrap();
    let first = next_text_within(&mut ws, Duration::from_secs(6)).await;
    assert_eq!(first, "ok:slow", "expected slow handler response");

    // Most importantly: the connection is still alive -- the burst tick did not
    // close it. A new message must get a quick response (if closed, next_text
    // times out / panics).
    ws.send(Message::text("again")).await.unwrap();
    let second = next_text_within(&mut ws, Duration::from_secs(3)).await;
    assert_eq!(
        second, "ok:again",
        "connection wrongly closed after a long handler"
    );

    let _ = std::fs::remove_file(&path);
}

// review P2 post-ping state: after the server sends a ping (awaiting_pong=true),
// the client sends a slow message before it has read the ping and replied pong.
// Because the handler blocks in `fire_handler().await`, the pong (queued behind
// the message) is not read -- in the old logic the next tick closed a healthy
// connection. Now the message itself is proof of liveness, the connection is kept.
#[tokio::test]
async fn slow_handler_with_outstanding_ping_keeps_connection() {
    let port = 9317;
    let script = SLOW_HANDLER_SCRIPT.replace("PORT", &port.to_string());
    let (child, path) = spawn_server_env(port, &script, &[("FLUXON_WS_PING_SECS", "1")]);
    let _killer = Killer(child);
    wait_port(port).await;

    let url = format!("ws://127.0.0.1:{}", port);
    let (mut ws, _) = connect_async(&url).await.expect("ulanish");

    // We don't read for ~1.3s: the server sends a ping at 1s (awaiting_pong=true),
    // but since the client has not read the ping, no pong is returned yet.
    tokio::time::sleep(Duration::from_millis(1300)).await;

    // A slow message while a ping is pending (the handler blocks 2.5s).
    ws.send(Message::text("slow")).await.unwrap();

    // We DON'T READ until the handler finishes and the server makes its decision
    // (~end of the handler) -- otherwise the client would immediately read the ping
    // and reply pong, hiding the bug (to keep it deterministic). In the old logic
    // the server closed the connection at this point with awaiting_pong=true.
    tokio::time::sleep(Duration::from_millis(3000)).await;

    let first = next_text_within(&mut ws, Duration::from_secs(3)).await;
    assert_eq!(first, "ok:slow", "expected slow handler response");

    // The connection must stay alive -- not closed even if the pong is queued.
    ws.send(Message::text("again")).await.unwrap();
    let second = next_text_within(&mut ws, Duration::from_secs(3)).await;
    assert_eq!(
        second, "ok:again",
        "slow handler wrongly closed the connection while a ping was pending"
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
    let _ = next_text(&mut a).await; // b's join (online:2)
    let _ = next_text(&mut b).await;

    // A disconnects -> the server must clean up the room membership.
    drop(a);
    tokio::time::sleep(Duration::from_millis(200)).await;

    // B says again -> only B receives it (A is gone), the broadcast goes through without error.
    b.send(Message::text(r#"{"t":"say","room":"r1","body":"again"}"#))
        .await
        .unwrap();
    let b_msg = next_text(&mut b).await;
    assert!(
        b_msg.contains("again"),
        "B received its own message: {}",
        b_msg
    );

    let _ = std::fs::remove_file(&path);
}
