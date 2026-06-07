// Flux WS battery — WebSocket server (realtime).
//
// `http` so'rov-javob bo'lsa, `ws` doimiy ikki tomonlama ulanish. Server
// tokio + tokio-tungstenite ustida quriladi. Flux handler'lari (`:connect`,
// `:message`, `:disconnect`) sinxron tree-walking bo'lgani uchun har biri
// `spawn_blocking` ichida chaqiriladi — http battery bilan bir xil model
// (Value: Send+Sync, haqiqiy parallel).
//
// Til API (docs 9.10):
//   ws.on :connect \conn ->            # yangi ulanish; conn.id — barqaror id
//   ws.on :message \conn msg ->        # msg — kelgan matn (str)
//   ws.on :disconnect \conn ->
//   ws.send conn text                  # SHU ulanishga yuborish
//   ws.room.join/leave conn room       # xona a'zoligi (broadcast guruhi)
//   ws.room.send room text             # xonadagi HAMMAGA yuborish
//   ws.room.members room               # xonadagi conn id'lar ro'yxati
//   ws.data.set conn :key val          # per-ulanish sessiya holati (yozish)
//   ws.data.get conn :key              # sessiya holatini o'qish
//   ws.serve port                      # bloklovchi server
//
// conn — Value::Map{id:str}. Haqiqiy socket Rust tomonida `id` orqali topiladi
// (WsState.conns). `ws.send`/`room.send` bevosita yozma kanalga (mpsc) yozadi,
// shuning uchun istalgan thread'dan (handler ichidan ham) xavfsiz yuborish
// mumkin — yozish bitta writer-task'da ketma-ket bajariladi.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::interp::{Flow, Interp};
use crate::value::Value;

// Hodisa turi — `ws.on :connect/:message/:disconnect` ro'yxatga oladi.
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

// Bitta ulanishning yozma uchi: writer-task'ga matn uzatadi. None(close) emas —
// yopish kanal drop bo'lishidan kelib chiqadi.
type ConnTx = mpsc::UnboundedSender<Message>;

// WS battery holati — jarayonga bitta (Interp ichida Arc). http `routes` kabi
// top-level kod to'ldiradi (`ws.on`), server thread'lari o'qiydi/yozadi.
pub struct WsState {
    // event -> handler (Value::Fn). `ws.on` to'ldiradi, serve loop o'qiydi.
    handlers: Mutex<HashMap<Event, Value>>,
    // conn id -> yozma kanal. Ulanish ochilganda qo'shiladi, yopilganda o'chadi.
    conns: Mutex<HashMap<String, ConnTx>>,
    // xona nomi -> a'zo conn id'lar. `ws.room.*` boshqaradi.
    rooms: Mutex<HashMap<String, HashSet<String>>>,
    // conn id -> per-ulanish sessiya map'i (`ws.data.set/get`).
    data: Mutex<HashMap<String, BTreeMap<String, Value>>>,
    // monoton id hisoblagichi (barqaror, takrorlanmas conn.id).
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

    // Ulanish ro'yxatdan o'chiriladi: kanal, sessiya, barcha xona a'zoligi.
    fn remove_conn(&self, id: &str) {
        self.conns.lock().remove(id);
        self.data.lock().remove(id);
        let mut rooms = self.rooms.lock();
        rooms.retain(|_, members| {
            members.remove(id);
            !members.is_empty()
        });
    }

    // Bitta conn id'ga matn yuborish. Kanal yopiq/topilmasa — jimgina e'tiborsiz
    // (ulanish allaqachon uzilgan bo'lishi mumkin).
    fn send_to(&self, id: &str, text: String) {
        if let Some(tx) = self.conns.lock().get(id) {
            let _ = tx.send(Message::text(text));
        }
    }

    // PR-7b: tag room'iga (":tag") matn broadcast — `ui.push :tag` ishlatadi.
    // ws.room.send bilan bir mantiq (snapshot + send_to), lekin UI tag room'i
    // uchun (a'zolar handle_ui_message subscribe orqali qo'shilgan). Istalgan
    // thread'dan xavfsiz (HTTP handler ichidan `ui.push` chaqirilsa ham).
    pub(crate) fn push_tag(&self, room: &str, text: &str) {
        let members: Vec<String> = match self.rooms.lock().get(room) {
            Some(set) => set.iter().cloned().collect(),
            None => return,
        };
        for id in members {
            self.send_to(&id, text.to_string());
        }
    }
}

// --- conn helper ---

// Rust id -> Flux conn map: {id:"c7"}. Handler shu map'ni oladi.
fn conn_value(id: &str) -> Value {
    let mut m = BTreeMap::new();
    m.insert("id".to_string(), Value::Str(id.to_string()));
    Value::Map(m)
}

// Flux conn map'idan id'ni ajratib oladi (`ws.send`/`room.join` argumenti).
fn conn_id(v: &Value, ctx: &str) -> Result<String, Flow> {
    match v {
        Value::Map(m) => match m.get("id") {
            Some(Value::Str(s)) => Ok(s.clone()),
            _ => Err(Flow::err(format!("{}: conn.id (str) topilmadi", ctx))),
        },
        _ => Err(Flow::err(format!(
            "{}: 1-argument conn (map) bo'lishi kerak, {} berildi",
            ctx,
            v.type_name()
        ))),
    }
}

// :key yoki "key" -> String (ws.data kaliti).
fn key_str(v: Option<&Value>, ctx: &str) -> Result<String, Flow> {
    match v {
        Some(Value::Sym(s)) | Some(Value::Str(s)) => Ok(s.clone()),
        _ => Err(Flow::err(format!("{}: kalit (:sym yoki str) kerak", ctx))),
    }
}

// `ws.send conn text` / `ws.room.send room text` — yuboriladigan matn.
// Map/List -> kelgan bo'lsa JSON emas; foydalanuvchi json.enc qiladi. Bu yerda
// str kutamiz, lekin qulaylik uchun boshqa qiymatni Display bilan stringga.
fn text_arg(v: Option<&Value>, ctx: &str) -> Result<String, Flow> {
    match v {
        Some(Value::Str(s)) => Ok(s.clone()),
        Some(other) => Ok(format!("{}", other)),
        None => Err(Flow::err(format!(
            "{}: yuboriladigan matn (str) kerak",
            ctx
        ))),
    }
}

// --- Interp WS dispatch ---

impl Interp {
    // ws.<func> chaqiruvlari (ws.room.* va ws.data.* alohida quyida).
    pub fn ws_dispatch(self: &Arc<Self>, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            "on" => self.ws_on(args),
            "send" => self.ws_send(args),
            "serve" => self.ws_serve(args),
            _ => Err(Flow::err(format!("ws modulida '{}' funksiyasi yo'q", func))),
        }
    }

    // ws.room.<func> — broadcast guruhlari.
    pub fn ws_room_dispatch(self: &Arc<Self>, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            "join" => self.ws_room_join(args),
            "leave" => self.ws_room_leave(args),
            "send" => self.ws_room_send(args),
            "members" => self.ws_room_members(args),
            _ => Err(Flow::err(format!(
                "ws.room modulida '{}' funksiyasi yo'q",
                func
            ))),
        }
    }

    // ws.data.<func> — per-ulanish sessiya holati.
    pub fn ws_data_dispatch(self: &Arc<Self>, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            "set" => self.ws_data_set(args),
            "get" => self.ws_data_get(args),
            _ => Err(Flow::err(format!(
                "ws.data modulida '{}' funksiyasi yo'q",
                func
            ))),
        }
    }

    // ws.on :event handler
    fn ws_on(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let ev = match args.first() {
            Some(Value::Sym(s)) | Some(Value::Str(s)) => Event::parse(s).ok_or_else(|| {
                Flow::err(format!(
                    "ws.on: noma'lum hodisa ':{}' (:connect/:message/:disconnect)",
                    s
                ))
            })?,
            _ => {
                return Err(Flow::err(
                    "ws.on: 1-argument hodisa (:connect/:message/:disconnect) bo'lishi kerak",
                ));
            }
        };
        let handler = match args.get(1) {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => return Err(Flow::err("ws.on: 2-argument handler (fn) bo'lishi kerak")),
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

    // ws.room.send room text — xonadagi har bir a'zoga yuboradi.
    fn ws_room_send(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let room = text_arg(args.first(), "ws.room.send")?;
        let text = text_arg(args.get(1), "ws.room.send")?;
        // Lock'ni qisqa ushlash uchun a'zo ro'yxatini nusxalaymiz, keyin yuboramiz.
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

    // ws.data.get conn :key -> val (yo'q bo'lsa nil)
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

    // ws.serve port — bloklovchi tokio multi-thread WebSocket server.
    // `ws.serve PORT` — http.serve kabi DARHOL bloklamaydi; kutilayotgan
    // serverlar ro'yxatiga qo'shadi. Top-level tugagach umumiy event-loopda
    // (`serve_mod`) HTTP bilan birga spawn qilinadi.
    fn ws_serve(self: &Arc<Self>, args: Vec<Value>) -> Result<Value, Flow> {
        let port = match args.first() {
            Some(Value::Int(n)) => *n as u16,
            _ => return Err(Flow::err("ws.serve: port (int) bo'lishi kerak")),
        };
        self.pending_servers
            .lock()
            .unwrap()
            .push(crate::serve_mod::PendingServer::Ws { port });
        Ok(Value::Nil)
    }
}

// Bitta WS server uchun accept loop — umumiy event-loop ichida spawn qilinadi.
pub async fn serve_loop(interp: Arc<Interp>, port: u16) {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Flux WS port {} bind xatosi: {}", port, e);
            return;
        }
    };
    eprintln!("Flux WS server: ws://localhost:{}", port);

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("ws accept xatosi: {}", e);
                continue;
            }
        };
        let interp = interp.clone();
        tokio::spawn(async move {
            handle_conn(interp, stream).await;
        });
    }
}

// --- bitta ulanishni boshqarish ---

// `ws.serve` (alohida port) ulanishi: TcpStream handshake -> run_conn (Flux rejimi).
async fn handle_conn(interp: Arc<Interp>, stream: tokio::net::TcpStream) {
    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ws handshake xatosi: {}", e);
            return;
        }
    };
    run_conn(interp, ws_stream, false).await;
}

// `ui.serve` bir portda WS upgrade ulanishi (PR-7b): handshake ALLAQACHON hyper
// upgrade'da bajarilgan, tayyor WebSocketStream keladi -> run_conn (UI rejimi:
// live source subscription, Flux ws.on handlerlari OTILMAYDI).
pub async fn handle_ui_conn<S>(
    interp: Arc<Interp>,
    ws_stream: tokio_tungstenite::WebSocketStream<S>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    run_conn(interp, ws_stream, true).await;
}

// Ulanish yadrosi (ws.serve VA ui.serve birga ishlatadi). `ui_mode`:
//   false -> Flux handlerlari (:connect/:message/:disconnect) — ws.serve kanali.
//   true  -> UI live subscription: kelgan {"sub":[tag...]} -> room ":tag" ga qo'shish;
//            Flux handler OTILMAYDI (bu UI realtime kanali, ws.serve emas).
// Yozish alohida task'da: mpsc kanal -> writer-task socketga ketma-ket yozadi
// (ws.send/room.send/ui.push istalgan thread'dan shu kanalga itaradi).
async fn run_conn<S>(
    interp: Arc<Interp>,
    ws_stream: tokio_tungstenite::WebSocketStream<S>,
    ui_mode: bool,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let id = interp.ws.alloc_id();
    let (mut writer, mut reader) = ws_stream.split();

    // Yozma kanal: ws.send/room.send/ui.push shu yerga itaradi.
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    interp.ws.conns.lock().insert(id.clone(), tx);

    let writer_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if writer.send(msg).await.is_err() {
                break; // socket yopildi
            }
        }
        let _ = writer.close().await;
    });

    // Flux rejimida :connect handler (UI rejimida yo'q).
    if !ui_mode {
        fire_handler(&interp, Event::Connect, &id, None).await;
    }

    while let Some(item) = reader.next().await {
        let text = match item {
            Ok(Message::Text(t)) => Some(t.to_string()),
            // Binary'ni lossy matn sifatida (Flux str-markazli).
            Ok(Message::Binary(b)) => Some(String::from_utf8_lossy(&b).to_string()),
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => None,
            Err(_) => break, // ulanish uzildi
        };
        let Some(t) = text else { continue };
        if ui_mode {
            // UI subscribe: {"sub":["orders",...]} -> har tag uchun room ":tag" ga.
            handle_ui_message(&interp, &id, &t);
        } else {
            fire_handler(&interp, Event::Message, &id, Some(t)).await;
        }
    }

    if !ui_mode {
        fire_handler(&interp, Event::Disconnect, &id, None).await;
    }
    interp.ws.remove_conn(&id);
    let _ = writer_task.await;
}

// UI live subscription xabari: {"sub":["orders","metrics"]} -> conn'ni har tag'ning
// room'iga (":tag") qo'shadi. ui.push :tag o'sha room'ga reload yuboradi.
fn handle_ui_message(interp: &Arc<Interp>, id: &str, text: &str) {
    let Ok(Value::Map(m)) = crate::builtins::json_decode(text) else {
        return; // noto'g'ri JSON — jim e'tiborsiz
    };
    let Some(Value::List(tags)) = m.get("sub") else {
        return;
    };
    let mut rooms = interp.ws.rooms.lock();
    for tag in tags {
        if let Value::Str(t) = tag {
            rooms
                .entry(format!(":{}", t))
                .or_default()
                .insert(id.to_string());
        }
    }
}

// Hodisa handler'ini chaqiradi (ro'yxatga olingan bo'lsa). Sinxron interp
// ishini spawn_blocking'da bajaramiz — tokio worker'ini bloklamaydi.
// :message uchun conn + msg, qolganlari uchun faqat conn argument.
async fn fire_handler(interp: &Arc<Interp>, ev: Event, id: &str, msg: Option<String>) {
    let handler = match interp.ws.handlers.lock().get(&ev) {
        Some(h) => h.clone(),
        None => return, // bu hodisa uchun handler yo'q — jim
    };
    let conn = conn_value(id);
    let mut argv = vec![conn];
    if let Some(m) = msg {
        argv.push(Value::Str(m));
    }
    let interp = interp.clone();
    let result = tokio::task::spawn_blocking(move || interp.apply(handler, argv)).await;
    // Handler xatosi — serverni o'ldirmaydi; stderr'ga diagnostika.
    match result {
        Ok(Err(flow)) => eprintln!("ws handler xatosi: {}", flow_msg(&flow)),
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

    // PR-7b: push_tag tag room'iga ulangan conn'larga reload xabarini yuboradi.
    #[test]
    fn push_tag_room_azolariga_yuboradi() {
        let ws = WsState::new();
        // Ikki conn: mock yozma kanal (rx ushlab turamiz, send_to itarganini ko'ramiz).
        let (tx1, mut rx1) = mpsc::unbounded_channel::<Message>();
        let (tx2, mut rx2) = mpsc::unbounded_channel::<Message>();
        ws.conns.lock().insert("c1".to_string(), tx1);
        ws.conns.lock().insert("c2".to_string(), tx2);
        // c1 :orders ga subscribe, c2 boshqa room'da (push olmasligi kerak).
        ws.rooms
            .lock()
            .entry(":orders".to_string())
            .or_default()
            .insert("c1".to_string());
        ws.rooms
            .lock()
            .entry(":boshqa".to_string())
            .or_default()
            .insert("c2".to_string());

        ws.push_tag(":orders", "{\"fx\":\"reload\"}");

        // c1 xabar oladi, c2 olmaydi.
        let got = rx1.try_recv().expect("c1 reload olishi kerak");
        assert!(matches!(got, Message::Text(t) if t.as_str().contains("reload")));
        assert!(rx2.try_recv().is_err(), "c2 (boshqa room) olmasligi kerak");
    }

    // Yo'q room'ga push — jim (panic yo'q).
    #[test]
    fn push_tag_yoq_room() {
        let ws = WsState::new();
        ws.push_tag(":yoq", "x"); // panic bermasligi kerak
    }

    // handle_ui_message {"sub":[...]} -> conn room'larga qo'shiladi.
    #[test]
    fn ui_subscribe_room_qoshadi() {
        let interp = Interp::new_arc();
        let (tx, _rx) = mpsc::unbounded_channel::<Message>();
        interp.ws.conns.lock().insert("c1".to_string(), tx);
        handle_ui_message(&interp, "c1", "{\"sub\":[\"orders\",\"metrics\"]}");
        let rooms = interp.ws.rooms.lock();
        assert!(
            rooms.get(":orders").is_some_and(|s| s.contains("c1")),
            ":orders room'ga qo'shilishi kerak"
        );
        assert!(
            rooms.get(":metrics").is_some_and(|s| s.contains("c1")),
            ":metrics room'ga qo'shilishi kerak"
        );
    }
}
