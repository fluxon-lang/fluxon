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

// Yozma kanal bufer hajmi. Chegaralangan: sekin/zararli mijoz (o'qimaydigan)
// xotirani cheksiz o'stirmasin — bufer to'lsa ulanish uziladi (issue #107).
const SEND_BUFFER: usize = 256;
// Davriy ping oralig'i — o'lik (half-open TCP) ulanishlarni aniqlash uchun.
// Ping yuborilgach keyingi tick'gacha pong kelmasa ulanish o'lik deb yopiladi
// (ya'ni javobsiz qolish chegarasi ~PING_INTERVAL). `FLUX_WS_PING_SECS` env
// bilan sozlanadi (ops tuning + integratsion test) — `ping_interval_dur`.
const PING_INTERVAL: Duration = Duration::from_secs(30);
// reader loop tugagach writer-task'ni kutish muddati. TCP yozma buferi to'lib
// qotgan mijozda writer cheksiz kutmasligi uchun — muddatdan keyin abort.
const WRITER_SHUTDOWN: Duration = Duration::from_secs(5);

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

// Bitta ulanishning server tomonidagi uchi. `tx` — chegaralangan yozma kanal
// (writer-task shundan o'qib socketga yozadi); `close` — boshqa thread'dan
// ulanishni majburan uzish signali (bufer to'lganda reader loop'ni uyg'otadi).
// Yopish odatda kanal drop bo'lishidan kelib chiqadi; `close` esa zudlik bilan.
struct Conn {
    tx: mpsc::Sender<Message>,
    close: Arc<Notify>,
}

// WS battery holati — jarayonga bitta (Interp ichida Arc). http `routes` kabi
// top-level kod to'ldiradi (`ws.on`), server thread'lari o'qiydi/yozadi.
pub struct WsState {
    // event -> handler (Value::Fn). `ws.on` to'ldiradi, serve loop o'qiydi.
    handlers: Mutex<HashMap<Event, Value>>,
    // conn id -> ulanish uchi. Ulanish ochilganda qo'shiladi, yopilganda o'chadi.
    conns: Mutex<HashMap<String, Conn>>,
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
    // (ulanish allaqachon uzilgan bo'lishi mumkin). Bufer to'lib qolsa (mijoz
    // o'qimayapti) ulanishni uzishga signal beramiz — xotira cheksiz o'smasin.
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

// Ping oralig'ini aniqlaydi: standart PING_INTERVAL, `FLUX_WS_PING_SECS` env
// (musbat butun son, sekund) bilan override qilinadi. Noto'g'ri/0 qiymat —
// standart. Test va deploy sozlash uchun.
fn ping_interval_dur() -> Duration {
    match std::env::var("FLUX_WS_PING_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        Some(n) if n > 0 => Duration::from_secs(n),
        _ => PING_INTERVAL,
    }
}

// WS port'ni bind qiladi. Bind xatosini `Flow::Error` sifatida qaytaradi —
// http_mod::bind bilan bir xil (issue #108: bind muvaffaqiyatsizligi exit code
// ≠ 0 bilan tugashi kerak).
pub async fn bind(port: u16) -> Result<TcpListener, Flow> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    TcpListener::bind(addr)
        .await
        .map_err(|e| Flow::err(format!("Flux WS port {} bind xatosi: {}", port, e)))
}

// Bitta WS server uchun accept loop — umumiy event-loop ichida spawn qilinadi.
// Listener oldindan `bind` bilan ochilgan.
pub async fn serve_loop(interp: Arc<Interp>, listener: TcpListener) {
    let port = listener.local_addr().map(|a| a.port()).unwrap_or_default();
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

// Handshake -> :connect -> xabar loop (:message har xabar) -> :disconnect.
// Yozish alohida task'da: mpsc kanal orqali kelgan matnlar ketma-ket socketga
// yoziladi (ws.send/room.send istalgan thread'dan shu kanalga itaradi).
async fn handle_conn(interp: Arc<Interp>, stream: tokio::net::TcpStream) {
    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ws handshake xatosi: {}", e);
            return;
        }
    };

    let id = interp.ws.alloc_id();
    let (mut writer, mut reader) = ws_stream.split();

    // Yozma kanal (chegaralangan): ws.send/room.send shu yerga itaradi,
    // writer-task socketga yozadi. ping_tx — davriy ping uchun nusxa.
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

    // Writer-task: kanaldan kelgan har xabarni socketga ketma-ket yozadi.
    let writer_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if writer.send(msg).await.is_err() {
                break; // socket yopildi
            }
        }
        let _ = writer.close().await;
    });

    // :connect handler.
    fire_handler(&interp, Event::Connect, &id, None).await;

    // Davriy ping: birinchi tick oraliqdan keyin (darhol emas).
    let ping_dur = ping_interval_dur();
    let mut ping_interval =
        tokio::time::interval_at(tokio::time::Instant::now() + ping_dur, ping_dur);
    // Uzoq handler (select loop'ni bloklab turadigan) ping oralig'idan oshsa,
    // standart Burst xulqi o'tkazib yuborilgan tick'larni ketma-ket beradi:
    // birinchisi ping yuborib awaiting_pong=true qiladi, keyingisi darhol
    // "javobsiz" deb yopib yuborardi. Delay — har tick orasida to'liq oraliqni
    // kafolatlaydi, ya'ni ping va o'lik-tekshiruv orasida mijozga javob vaqti.
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Oldingi ping'ga pong kutilyaptimi: yana tick kelganda hali true bo'lsa,
    // mijoz javob bermadi — ulanish o'lik (half-open) deb yopiladi.
    let mut awaiting_pong = false;

    // Xabar loop: socket o'qish, davriy ping va uzish signalini birga kutamiz.
    loop {
        tokio::select! {
            item = reader.next() => {
                match item {
                    Some(Ok(msg)) => {
                        // Har qanday kelgan kadr ulanish tirikligini isbotlaydi —
                        // pong shart emas. Uzoq handler `fire_handler().await` da
                        // bloklab, mijoz pong'ini vaqtida o'qitmasligi mumkin
                        // (pong xabar orqasida navbatda turadi); shu sabab kutish
                        // bayrog'ini har kadrda tozalaymiz (review P2).
                        awaiting_pong = false;
                        match msg {
                            Message::Text(t) => {
                                fire_handler(&interp, Event::Message, &id, Some(t.to_string()))
                                    .await;
                            }
                            Message::Binary(b) => {
                                // Binary'ni lossy matn sifatida uzatamiz (Flux str-markazli).
                                let t = String::from_utf8_lossy(&b).to_string();
                                fire_handler(&interp, Event::Message, &id, Some(t)).await;
                            }
                            Message::Close(_) => break,
                            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                        }
                    }
                    Some(Err(_)) | None => break, // ulanish uzildi
                }
            }
            _ = ping_interval.tick() => {
                if awaiting_pong {
                    break; // oldingi ping'ga javob (pong yoki har qanday kadr) kelmadi
                }
                awaiting_pong = true;
                // Kanal to'la/yopiq bo'lsa yozolmaymiz — uzamiz.
                if ping_tx.try_send(Message::Ping(Vec::new())).is_err() {
                    break;
                }
            }
            _ = close.notified() => break, // sekin mijoz: bufer to'ldi — uzamiz
        }
    }

    // :disconnect handler — keyin ro'yxatdan o'chiramiz.
    fire_handler(&interp, Event::Disconnect, &id, None).await;
    interp.ws.remove_conn(&id);
    drop(ping_tx); // qolgan sender — kanal yopilsin (writer-task tugaydi)
    // Sekin mijozda (TCP yozma buferi to'la) writer cheksiz qotmasligi uchun
    // qisqa kutamiz, so'ng majburan to'xtatamiz.
    let abort = writer_task.abort_handle();
    if tokio::time::timeout(WRITER_SHUTDOWN, writer_task)
        .await
        .is_err()
    {
        abort.abort();
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

    // issue #107: bounded kanal to'lganda send_to ulanishni uzishga signal
    // beradi (sekin/zararli mijoz xotirani cheksiz o'stirmasin).
    #[tokio::test]
    async fn send_to_bufer_tolsa_uzadi() {
        let state = WsState::new();
        // Sig'imi 1: 1-xabar buferga sig'adi, 2-chisi to'ldiradi → close signal.
        let (tx, _rx) = mpsc::channel::<Message>(1);
        let close = Arc::new(Notify::new());
        state.conns.lock().insert(
            "c1".to_string(),
            Conn {
                tx,
                close: close.clone(),
            },
        );

        state.send_to("c1", "a".to_string()); // buferga sig'adi
        state.send_to("c1", "b".to_string()); // bufer to'la → notify

        // close zudlik bilan signal berilgan bo'lishi kerak.
        tokio::time::timeout(Duration::from_millis(500), close.notified())
            .await
            .expect("bufer to'lganda close signali kutilgan edi");
    }

    // remove_conn kanal, sessiya holati va xona a'zoligini tozalaydi
    // (o'lik ulanish yozuvlari abadiy qolmasin).
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

        assert!(state.conns.lock().is_empty(), "conns tozalanmadi");
        assert!(state.rooms.lock().is_empty(), "bo'sh xona o'chmadi");
        assert!(state.data.lock().is_empty(), "sessiya tozalanmadi");
    }

    // To'liq oqim: haqiqiy server + mijoz. Echo handler xabarni qaytaradi
    // (refaktor qilingan select-loop xabarni boshqaradi), so'ng mijoz uzilganda
    // server conn'ni tozalaydi.
    #[tokio::test]
    async fn echo_va_uzilishda_tozalash() {
        use futures_util::{SinkExt as _, StreamExt as _};

        // :message echo handler'ini ro'yxatga olamiz.
        let src = "ws.on :message \\conn msg -> ws.send conn msg\n";
        let toks = crate::lexer::lex(src).unwrap();
        let prog = crate::parser::parse(toks).unwrap();
        let interp = Interp::new_arc();
        interp.run(&prog).unwrap();

        // Ephemeral portda server ko'taramiz.
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let srv = interp.clone();
        tokio::spawn(async move { serve_loop(srv, listener).await });

        // Haqiqiy mijoz ulanadi va xabar yuboradi.
        let url = format!("ws://127.0.0.1:{}/", port);
        let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        ws.send(Message::text("salom")).await.unwrap();
        let reply = ws.next().await.unwrap().unwrap();
        assert_eq!(reply.to_text().unwrap(), "salom");
        assert_eq!(interp.ws.conns.lock().len(), 1, "ulanish ro'yxatda yo'q");

        // Mijoz uzadi → server :disconnect + remove_conn (async) bajaradi.
        ws.close(None).await.unwrap();
        for _ in 0..100 {
            if interp.ws.conns.lock().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            interp.ws.conns.lock().is_empty(),
            "uzilgandan keyin conn tozalanmadi"
        );
    }
}
