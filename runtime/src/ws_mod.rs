// Fluxon WS battery — WebSocket server (realtime).
//
// Where `http` is request-response, `ws` is a persistent bidirectional
// connection. The server is built on tokio + tokio-tungstenite. Since Fluxon
// handlers (`:connect`, `:message`, `:disconnect`) are synchronous tree-walking,
// each is invoked inside `spawn_blocking` — the same model as the http battery
// (Value: Send+Sync, true parallelism).
//
// Language API (docs 9.10):
//   ws.on :connect \conn ->            # new connection; conn.id — stable id
//   ws.on :message \conn msg ->        # msg — incoming text (str)
//   ws.on :disconnect \conn ->
//   ws.send conn text                  # send to THIS connection
//   ws.room.join/leave conn room       # room membership (broadcast group)
//   ws.room.send room text             # send to EVERYONE in the room
//   ws.room.members room               # list of conn ids in the room
//   ws.data.set conn :key val          # per-connection session state (write)
//   ws.data.get conn :key              # read session state
//   ws.serve port                      # blocking server
//
// conn — Value::Map{id:str}. The real socket is found on the Rust side via `id`
// (WsState.conns). `ws.send`/`room.send` write directly to the write channel
// (mpsc), so sending is safe from any thread (including inside a handler) — writes
// run sequentially in a single writer-task.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tokio_tungstenite::tungstenite::Message;

use crate::interp::{Flow, Interp};
use crate::value::Value;

// Write channel buffer size. Bounded: don't let a slow/malicious client (one that
// doesn't read) grow memory unbounded — if the buffer fills, the connection is
// closed (issue #107).
const SEND_BUFFER: usize = 256;
// Periodic ping interval — used to detect dead (half-open TCP) connections.
// If no pong arrives before the next tick after a ping is sent, the connection is
// closed as dead (i.e. the silence threshold is ~PING_INTERVAL). Configurable via
// the `FLUXON_WS_PING_SECS` env var (ops tuning + integration test) — see
// `ping_interval_dur`.
const PING_INTERVAL: Duration = Duration::from_secs(30);
// How long to wait for the writer-task after the reader loop ends. So the writer
// doesn't wait forever on a client whose TCP write buffer is full — abort after
// the timeout.
const WRITER_SHUTDOWN: Duration = Duration::from_secs(5);

// Event type — registered by `ws.on :connect/:message/:disconnect`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Event {
    Connect,
    Message,
    Disconnect,
}

impl Event {
    fn parse(s: &str) -> Option<Event> {
        match s {
            "connect" => Some(Event::Connect),
            "message" => Some(Event::Message),
            "disconnect" => Some(Event::Disconnect),
            _ => None,
        }
    }
}

// The server-side end of a single connection. `tx` — the bounded write channel
// (the writer-task reads from it and writes to the socket); `close` — a signal to
// forcibly close the connection from another thread (wakes the reader loop when
// the buffer fills). Closing usually results from the channel being dropped;
// `close` does it immediately.
struct Conn {
    tx: mpsc::Sender<Message>,
    close: Arc<Notify>,
}

// WS battery state — one per process (Arc inside Interp). Like http `routes`, it
// is filled by top-level code (`ws.on`) and read/written by server threads.
pub struct WsState {
    // event -> handler (Value::Fn). `ws.on` fills it, the serve loop reads it.
    handlers: Mutex<HashMap<Event, Value>>,
    // conn id -> connection end. Added when a connection opens, removed when it closes.
    conns: Mutex<HashMap<String, Conn>>,
    // room name -> member conn ids. Managed by `ws.room.*`.
    rooms: Mutex<HashMap<String, HashSet<String>>>,
    // conn id -> per-connection session map (`ws.data.set/get`).
    data: Mutex<HashMap<String, BTreeMap<String, Value>>>,
    // monotonic id counter (stable, non-repeating conn.id).
    next_id: AtomicU64,
}

impl WsState {
    pub fn new() -> Self {
        WsState {
            handlers: Mutex::new(HashMap::new()),
            conns: Mutex::new(HashMap::new()),
            rooms: Mutex::new(HashMap::new()),
            data: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    fn alloc_id(&self) -> String {
        let n = self.next_id.fetch_add(1, Ordering::Relaxed);
        format!("c{}", n)
    }

    // Removes the connection from the registry: channel, session, all room memberships.
    fn remove_conn(&self, id: &str) {
        self.conns.lock().remove(id);
        self.data.lock().remove(id);
        let mut rooms = self.rooms.lock();
        rooms.retain(|_, members| {
            members.remove(id);
            !members.is_empty()
        });
    }

    // Send text to a single conn id. If the channel is closed/not found — silently
    // ignored (the connection may already be gone). If the buffer fills (client
    // isn't reading) we signal to close the connection — so memory doesn't grow
    // unbounded.
    fn send_to(&self, id: &str, text: String) {
        if let Some(conn) = self.conns.lock().get(id) {
            match conn.tx.try_send(Message::text(text)) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => conn.close.notify_one(),
                Err(TrySendError::Closed(_)) => {}
            }
        }
    }
}

// --- conn helper ---

// Rust id -> Fluxon conn map: {id:"c7"}. The handler receives this map.
fn conn_value(id: &str) -> Value {
    let mut m = BTreeMap::new();
    m.insert("id".to_string(), Value::Str(id.to_string()));
    Value::Map(m)
}

// Extracts the id from a Fluxon conn map (the `ws.send`/`room.join` argument).
fn conn_id(v: &Value, ctx: &str) -> Result<String, Flow> {
    match v {
        Value::Map(m) => match m.get("id") {
            Some(Value::Str(s)) => Ok(s.clone()),
            _ => Err(Flow::err(format!("{}: conn.id (str) not found", ctx))),
        },
        _ => Err(Flow::err(format!(
            "{}: argument 1 must be a conn (map), got {}",
            ctx,
            v.type_name()
        ))),
    }
}

// :key or "key" -> String (the ws.data key).
fn key_str(v: Option<&Value>, ctx: &str) -> Result<String, Flow> {
    match v {
        Some(Value::Sym(s)) | Some(Value::Str(s)) => Ok(s.clone()),
        _ => Err(Flow::err(format!("{}: key (:sym or str) required", ctx))),
    }
}

// `ws.send conn text` / `ws.room.send room text` — the text to send.
// If a Map/List comes in it is not JSON; the user calls json.enc. Here we expect
// a str, but for convenience we Display other values into a string.
fn text_arg(v: Option<&Value>, ctx: &str) -> Result<String, Flow> {
    match v {
        Some(Value::Str(s)) => Ok(s.clone()),
        Some(other) => Ok(format!("{}", other)),
        None => Err(Flow::err(format!("{}: text to send (str) required", ctx))),
    }
}

// --- Interp WS dispatch ---

impl Interp {
    // ws.<func> calls (ws.room.* and ws.data.* are separate below).
    pub fn ws_dispatch(self: &Arc<Self>, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            "on" => self.ws_on(args),
            "send" => self.ws_send(args),
            "serve" => self.ws_serve(args),
            _ => Err(Flow::err(format!("ws module has no '{}' function", func))),
        }
    }

    // ws.room.<func> — broadcast groups.
    pub fn ws_room_dispatch(self: &Arc<Self>, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            "join" => self.ws_room_join(args),
            "leave" => self.ws_room_leave(args),
            "send" => self.ws_room_send(args),
            "members" => self.ws_room_members(args),
            _ => Err(Flow::err(format!(
                "ws.room module has no '{}' function",
                func
            ))),
        }
    }

    // ws.data.<func> — per-connection session state.
    pub fn ws_data_dispatch(self: &Arc<Self>, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            "set" => self.ws_data_set(args),
            "get" => self.ws_data_get(args),
            _ => Err(Flow::err(format!(
                "ws.data module has no '{}' function",
                func
            ))),
        }
    }

    // ws.on :event handler
    fn ws_on(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let ev = match args.first() {
            Some(Value::Sym(s)) | Some(Value::Str(s)) => Event::parse(s).ok_or_else(|| {
                Flow::err(format!(
                    "ws.on: unknown event ':{}' (:connect/:message/:disconnect)",
                    s
                ))
            })?,
            _ => {
                return Err(Flow::err(
                    "ws.on: argument 1 must be an event (:connect/:message/:disconnect)",
                ));
            }
        };
        let handler = match args.get(1) {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => return Err(Flow::err("ws.on: argument 2 must be a handler (fn)")),
        };
        self.ws.handlers.lock().insert(ev, handler);
        Ok(Value::Nil)
    }

    // ws.send conn text
    fn ws_send(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let id = conn_id(args.first().unwrap_or(&Value::Nil), "ws.send")?;
        let text = text_arg(args.get(1), "ws.send")?;
        self.ws.send_to(&id, text);
        Ok(Value::Nil)
    }

    // ws.room.join conn room
    fn ws_room_join(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let id = conn_id(args.first().unwrap_or(&Value::Nil), "ws.room.join")?;
        let room = text_arg(args.get(1), "ws.room.join")?;
        self.ws.rooms.lock().entry(room).or_default().insert(id);
        Ok(Value::Nil)
    }

    // ws.room.leave conn room
    fn ws_room_leave(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let id = conn_id(args.first().unwrap_or(&Value::Nil), "ws.room.leave")?;
        let room = text_arg(args.get(1), "ws.room.leave")?;
        let mut rooms = self.ws.rooms.lock();
        if let Some(members) = rooms.get_mut(&room) {
            members.remove(&id);
            if members.is_empty() {
                rooms.remove(&room);
            }
        }
        Ok(Value::Nil)
    }

    // ws.room.send room text — sends to every member in the room.
    fn ws_room_send(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let room = text_arg(args.first(), "ws.room.send")?;
        let text = text_arg(args.get(1), "ws.room.send")?;
        // To hold the lock only briefly we copy the member list, then send.
        let members: Vec<String> = match self.ws.rooms.lock().get(&room) {
            Some(set) => set.iter().cloned().collect(),
            None => return Ok(Value::Nil),
        };
        for id in members {
            self.ws.send_to(&id, text.clone());
        }
        Ok(Value::Nil)
    }

    // ws.room.members room -> [conn_id, ...]
    fn ws_room_members(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let room = text_arg(args.first(), "ws.room.members")?;
        let list = match self.ws.rooms.lock().get(&room) {
            Some(set) => set.iter().cloned().map(Value::Str).collect(),
            None => Vec::new(),
        };
        Ok(Value::List(list))
    }

    // ws.data.set conn :key val
    fn ws_data_set(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let id = conn_id(args.first().unwrap_or(&Value::Nil), "ws.data.set")?;
        let key = key_str(args.get(1), "ws.data.set")?;
        let val = args.get(2).cloned().unwrap_or(Value::Nil);
        self.ws.data.lock().entry(id).or_default().insert(key, val);
        Ok(Value::Nil)
    }

    // ws.data.get conn :key -> val (nil if absent)
    fn ws_data_get(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let id = conn_id(args.first().unwrap_or(&Value::Nil), "ws.data.get")?;
        let key = key_str(args.get(1), "ws.data.get")?;
        let val = self
            .ws
            .data
            .lock()
            .get(&id)
            .and_then(|m| m.get(&key))
            .cloned()
            .unwrap_or(Value::Nil);
        Ok(val)
    }

    // ws.serve port — blocking tokio multi-thread WebSocket server.
    // `ws.serve PORT` — like http.serve, does NOT block immediately; it appends to
    // the pending servers list. Once top-level finishes it is spawned alongside
    // HTTP in the shared event loop (`serve_mod`).
    fn ws_serve(self: &Arc<Self>, args: Vec<Value>) -> Result<Value, Flow> {
        let port = match args.first() {
            Some(Value::Int(n)) => *n as u16,
            _ => return Err(Flow::err("ws.serve: port (int) required")),
        };
        self.pending_servers
            .lock()
            .unwrap()
            .push(crate::serve_mod::PendingServer::Ws { port });
        Ok(Value::Nil)
    }
}

// Determines the ping interval: default PING_INTERVAL, overridden by the
// `FLUXON_WS_PING_SECS` env var (positive integer, seconds). An invalid/0 value
// falls back to the default. For test and deploy tuning.
fn ping_interval_dur() -> Duration {
    match std::env::var("FLUXON_WS_PING_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        Some(n) if n > 0 => Duration::from_secs(n),
        _ => PING_INTERVAL,
    }
}

// Binds the WS port. Returns a bind error as `Flow::Error` — same as
// http_mod::bind (issue #108: a bind failure must end with exit code != 0).
pub async fn bind(port: u16) -> Result<TcpListener, Flow> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    TcpListener::bind(addr)
        .await
        .map_err(|e| Flow::err(format!("Fluxon WS port {} bind error: {}", port, e)))
}

// Accept loop for a single WS server — spawned inside the shared event loop.
// The listener was already opened via `bind`.
pub async fn serve_loop(interp: Arc<Interp>, listener: TcpListener) {
    let port = listener.local_addr().map(|a| a.port()).unwrap_or_default();
    eprintln!("Fluxon WS server: ws://localhost:{}", port);

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("ws accept error: {}", e);
                continue;
            }
        };
        let interp = interp.clone();
        tokio::spawn(async move {
            handle_conn(interp, stream).await;
        });
    }
}

// --- handling a single connection ---

// Handshake -> :connect -> message loop (:message per message) -> :disconnect.
// Writing happens in a separate task: texts arriving over the mpsc channel are
// written to the socket sequentially (ws.send/room.send push into this channel
// from any thread).
async fn handle_conn(interp: Arc<Interp>, stream: tokio::net::TcpStream) {
    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ws handshake error: {}", e);
            return;
        }
    };

    let id = interp.ws.alloc_id();
    let (mut writer, mut reader) = ws_stream.split();

    // Write channel (bounded): ws.send/room.send push here, the writer-task writes
    // to the socket. ping_tx — a clone for the periodic ping.
    let (tx, mut rx) = mpsc::channel::<Message>(SEND_BUFFER);
    let close = Arc::new(Notify::new());
    let ping_tx = tx.clone();
    interp.ws.conns.lock().insert(
        id.clone(),
        Conn {
            tx,
            close: close.clone(),
        },
    );

    // Writer-task: writes each message from the channel to the socket in order.
    let writer_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if writer.send(msg).await.is_err() {
                break; // socket closed
            }
        }
        let _ = writer.close().await;
    });

    // :connect handler.
    fire_handler(&interp, Event::Connect, &id, None).await;

    // Periodic ping: first tick after one interval (not immediately).
    let ping_dur = ping_interval_dur();
    let mut ping_interval =
        tokio::time::interval_at(tokio::time::Instant::now() + ping_dur, ping_dur);
    // If a long handler (one that blocks the select loop) exceeds the ping
    // interval, the default Burst behavior would deliver the missed ticks back to
    // back: the first sends a ping and sets awaiting_pong=true, the next would
    // immediately close as "unresponsive". Delay guarantees a full interval
    // between ticks, i.e. it gives the client response time between the ping and
    // the dead-check.
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Whether a pong is awaited from the previous ping: if it's still true when
    // the next tick arrives, the client didn't respond — the connection is closed
    // as dead (half-open).
    let mut awaiting_pong = false;

    // Message loop: read the socket, periodic ping and the close signal together.
    loop {
        tokio::select! {
            item = reader.next() => {
                match item {
                    Some(Ok(msg)) => {
                        // Any incoming frame proves the connection is alive — a
                        // pong is not required. A long handler may block in
                        // `fire_handler().await` and prevent the client's pong
                        // from being read in time (the pong queues behind the
                        // message); that's why we clear the awaiting flag on every
                        // frame (review P2).
                        awaiting_pong = false;
                        match msg {
                            Message::Text(t) => {
                                fire_handler(&interp, Event::Message, &id, Some(t.to_string()))
                                    .await;
                            }
                            Message::Binary(b) => {
                                // Pass Binary through as lossy text (Fluxon is str-centric).
                                let t = String::from_utf8_lossy(&b).to_string();
                                fire_handler(&interp, Event::Message, &id, Some(t)).await;
                            }
                            Message::Close(_) => break,
                            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                        }
                    }
                    Some(Err(_)) | None => break, // connection dropped
                }
            }
            _ = ping_interval.tick() => {
                if awaiting_pong {
                    break; // no response (pong or any frame) to the previous ping
                }
                awaiting_pong = true;
                // If the channel is full/closed we can't write — close.
                if ping_tx.try_send(Message::Ping(Vec::new())).is_err() {
                    break;
                }
            }
            _ = close.notified() => break, // slow client: buffer filled — close
        }
    }

    // :disconnect handler — then remove from the registry.
    fire_handler(&interp, Event::Disconnect, &id, None).await;
    interp.ws.remove_conn(&id);
    drop(ping_tx); // the remaining sender — let the channel close (writer-task ends)
    // So the writer doesn't hang forever on a slow client (TCP write buffer full)
    // we wait briefly, then forcibly abort it.
    let abort = writer_task.abort_handle();
    if tokio::time::timeout(WRITER_SHUTDOWN, writer_task)
        .await
        .is_err()
    {
        abort.abort();
    }
}

// Invokes the event handler (if one is registered). The synchronous interp work
// runs in spawn_blocking — so it doesn't block a tokio worker.
// For :message the args are conn + msg, for the rest just conn.
async fn fire_handler(interp: &Arc<Interp>, ev: Event, id: &str, msg: Option<String>) {
    let handler = match interp.ws.handlers.lock().get(&ev) {
        Some(h) => h.clone(),
        None => return, // no handler for this event — silent
    };
    let conn = conn_value(id);
    let mut argv = vec![conn];
    if let Some(m) = msg {
        argv.push(Value::Str(m));
    }
    let interp = interp.clone();
    let result = tokio::task::spawn_blocking(move || interp.apply(handler, argv)).await;
    // A handler error doesn't kill the server; diagnostics go to stderr.
    match result {
        Ok(Err(flow)) => eprintln!("ws handler error: {}", flow_msg(&flow)),
        Err(join) => eprintln!("ws handler panic: {}", join),
        Ok(Ok(_)) => {}
    }
}

fn flow_msg(flow: &Flow) -> String {
    match flow {
        Flow::Fail { message, .. } => message.clone(),
        Flow::Error(e) => e.clone(),
        _ => "skip/stop/return".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // issue #107: when the bounded channel fills, send_to signals to close the
    // connection (so a slow/malicious client doesn't grow memory unbounded).
    #[tokio::test]
    async fn send_to_bufer_tolsa_uzadi() {
        let state = WsState::new();
        // Capacity 1: the 1st message fits the buffer, the 2nd fills it → close signal.
        let (tx, _rx) = mpsc::channel::<Message>(1);
        let close = Arc::new(Notify::new());
        state.conns.lock().insert(
            "c1".to_string(),
            Conn {
                tx,
                close: close.clone(),
            },
        );

        state.send_to("c1", "a".to_string()); // fits the buffer
        state.send_to("c1", "b".to_string()); // buffer full → notify

        // close must have been signaled immediately.
        tokio::time::timeout(Duration::from_millis(500), close.notified())
            .await
            .expect("expected close signal when buffer is full");
    }

    // remove_conn clears the channel, session state and room membership
    // (so dead-connection records don't stick around forever).
    #[test]
    fn remove_conn_hammasini_tozalaydi() {
        let state = WsState::new();
        let (tx, _rx) = mpsc::channel::<Message>(SEND_BUFFER);
        let close = Arc::new(Notify::new());
        state
            .conns
            .lock()
            .insert("c1".to_string(), Conn { tx, close });
        state
            .rooms
            .lock()
            .entry("xona".to_string())
            .or_default()
            .insert("c1".to_string());
        state
            .data
            .lock()
            .entry("c1".to_string())
            .or_default()
            .insert("k".to_string(), Value::Int(1));

        state.remove_conn("c1");

        assert!(state.conns.lock().is_empty(), "conns not cleared");
        assert!(state.rooms.lock().is_empty(), "empty room not removed");
        assert!(state.data.lock().is_empty(), "session not cleared");
    }

    // Full flow: real server + client. The echo handler replies with the message
    // (the refactored select-loop handles the message), then when the client
    // disconnects the server cleans up the conn.
    #[tokio::test]
    async fn echo_va_uzilishda_tozalash() {
        use futures_util::{SinkExt as _, StreamExt as _};

        // Register the :message echo handler.
        let src = "ws.on :message \\conn msg -> ws.send conn msg\n";
        let toks = crate::lexer::lex(src).unwrap();
        let prog = crate::parser::parse(toks).unwrap();
        let interp = Interp::new_arc();
        interp.run(&prog).unwrap();

        // Bring up the server on an ephemeral port.
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let srv = interp.clone();
        tokio::spawn(async move { serve_loop(srv, listener).await });

        // A real client connects and sends a message.
        let url = format!("ws://127.0.0.1:{}/", port);
        let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        ws.send(Message::text("hello")).await.unwrap();
        let reply = ws.next().await.unwrap().unwrap();
        assert_eq!(reply.to_text().unwrap(), "hello");
        assert_eq!(interp.ws.conns.lock().len(), 1, "connection not registered");

        // Client disconnects → server runs :disconnect + remove_conn (async).
        ws.close(None).await.unwrap();
        for _ in 0..100 {
            if interp.ws.conns.lock().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            interp.ws.conns.lock().is_empty(),
            "conn not cleared after disconnect"
        );
    }
}
