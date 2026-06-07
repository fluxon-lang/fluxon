// Flux frontend (UI qatlami) — 1-BOSQICH (MVP): statik element daraxti -> HTML.
//
// Falsafa (docs/flux-frontend.md): UI backend bilan BIR `.fx` faylda. `view` =
// `fn`ning UI varianti, element daraxti qaytaradi. Element YANGI Value variant
// TALAB QILMAYDI — `http_mod`ning `{__resp:true ...}` idiomasi takrorlanadi:
// element = maxsus kalitli map `{__node:true tag:"div" text:.. props:{..} children:[..]}`.
// Bu Send+Sync invariantini avtomatik saqlaydi (value.rs tegilmaydi).
//
// MVP doirasi: element konstruktorlari (`div`/`p`/`h1`/...), `node_to_html` (SSR),
// `ui.html node -> str`. Reaktivlik/server/source — keyingi bosqichlar.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

use crate::interp::{Flow, Interp};
use crate::value::Value;

// MVP'da qo'llab-quvvatlanadigan core HTML teglari. Semantik proplar bularning
// ustida ishlaydi; ro'yxat keyingi bosqichlarda kengayadi.
//
// Teglar GLOBAL O'ZGARUVCHI EMAS: `a`, `p`, `form`, `input` kabi nomlar keng
// tarqalgan o'zgaruvchi nomlari (`a = 5`). Shuning uchun ular faqat CHAQIRUV
// pozitsiyasida (callee) va nom band bo'lmaganda element sifatida hal qilinadi
// (interp::eval_call). Bu oddiy bind bilan to'qnashuvni butunlay yo'qotadi.
const CORE_TAGS: &[&str] = &[
    "div", "p", "h1", "h2", "h3", "span", "btn", "img", "input", "a", "ul", "li", "form", "badge",
];

// Nom element teg ekanmi (interp eval_call fallback uchun).
pub fn is_element_tag(name: &str) -> bool {
    CORE_TAGS.contains(&name)
}

// Element teg chaqirig'ini ({__node} map) quradi — interp eval_call'dan.
pub fn build_element(tag: &str, args: Vec<Value>) -> Result<Value, Flow> {
    build_node(tag, args)
}

// Element argumentlarini o'qib `{__node:true tag:.. text:.. props:.. children:..}`
// map quradi. Argumentlar tartibi erkin (spec: `tag content {props}`):
//   - str/int/sym/bool/flt qiymat -> text (matn bola)
//   - map -> props
//   - list -> children (boshqa elementlar; parser oxirgi argument sifatida beradi)
fn build_node(tag: &str, args: Vec<Value>) -> Result<Value, Flow> {
    let mut text: Option<String> = None;
    let mut props: BTreeMap<String, Value> = BTreeMap::new();
    let mut children: Vec<Value> = Vec::new();

    for a in args {
        match a {
            Value::Map(m) => {
                // {__node} bo'lsa — bu bola element (qavs ichida yozilgan bo'lsa).
                if is_node(&Value::Map(m.clone())) {
                    children.push(Value::Map(m));
                } else {
                    props = m;
                }
            }
            Value::List(items) => {
                // Bolalar ro'yxati (parser indentatsiyadan beradi yoki qo'lda list).
                for it in items {
                    children.push(it);
                }
            }
            // Matn bo'lishi mumkin bo'lgan skalyar qiymatlar.
            other @ (Value::Str(_)
            | Value::Int(_)
            | Value::Flt(_)
            | Value::Sym(_)
            | Value::Bool(_)) => {
                text = Some(other.to_text());
            }
            Value::Nil => {}
            other => {
                return Err(Flow::err(format!(
                    "{} elementi qo'llab-quvvatlamaydigan argument: {}",
                    tag,
                    other.type_name()
                )));
            }
        }
    }

    let mut node: BTreeMap<String, Value> = BTreeMap::new();
    node.insert("__node".to_string(), Value::Bool(true));
    node.insert("tag".to_string(), Value::Str(tag.to_string()));
    if let Some(t) = text {
        node.insert("text".to_string(), Value::Str(t));
    }
    if !props.is_empty() {
        node.insert("props".to_string(), Value::Map(props));
    }
    if !children.is_empty() {
        node.insert("children".to_string(), Value::List(children));
    }
    Ok(Value::Map(node))
}

// Qiymat element ({__node:true}) ekanmi.
fn is_node(v: &Value) -> bool {
    matches!(v, Value::Map(m) if matches!(m.get("__node"), Some(Value::Bool(true))))
}

// Public: interp view tanasidagi element qiymatlarini aniqlash uchun.
pub fn is_node_value(v: &Value) -> bool {
    is_node(v)
}

// Bir nechta top-level elementni ko'rinmas o'rovga (fragment) yig'adi. Fragment
// HTML'da yopuvchi tegsiz — faqat bolalari render qilinadi (React fragment kabi).
pub fn fragment(children: Vec<Value>) -> Value {
    let mut node: BTreeMap<String, Value> = BTreeMap::new();
    node.insert("__node".to_string(), Value::Bool(true));
    node.insert("tag".to_string(), Value::Str("__fragment".to_string()));
    node.insert("children".to_string(), Value::List(children));
    Value::Map(node)
}

// --- PR-4b: island markerlash (node-daraxt walk) ---
//
// Render {__node} daraxtni qurgach, uni BIR MARTA walk qilib "island"larni
// belgilaymiz. Falsafa (FRONTEND-PROD-ARCHITECTURE 1.2): interaktivlik izi
// (`on:`/`bind:` props) bo'lgan eng kichik o'rovchi element = ISLAND ILDIZI
// (client JS kerak), qolgani sof statik (SSR, 0 JS).
//
// Bu yondashuv AST-indeks emas — to'g'ridan render natijasida ishlaydi, shuning
// uchun analyzer/render indeks-moslik muammosi YO'Q. `on:`/`bind:` node props'da
// (build_node saqlaydi). Sof `<-` reaktiv-o'qish izi (event'siz) bu walk'da
// ko'rinmaydi — uni analyzer view-flag qoplaydi (interp render'da view interaktiv
// bo'lsa-yu walk hech island topmasa, butun view island bo'ladi — keyingi PR).
//
// Natija: island ildiz {__node}ga `__island:N`, on:/bind: elementga `__on`/`__bind`.

// Butun node daraxtni walk qilib island markerlar qo'shadi. `next_id` — keyingi
// island raqami (har sahifada 1, 2, ...). `react_state` — island ildiziga
// yoziladigan reaktiv (`<-`) state (PR-6, STATELESS: client keyingi event'da
// `data-fx-state`ni qaytaradi). Oddiy GET render'da bo'sh map (data-fx-state yo'q,
// initial qiymat literal seed'dan). Qaytaradi: topilgan island soni.
pub fn mark_islands(
    node: &mut Value,
    next_id: &mut u32,
    react_state: &BTreeMap<String, Value>,
) -> u32 {
    mark_walk(node, next_id, false, react_state)
}

// Rekursiv walk. `inside_island` — biz allaqachon island ichidamizmi (shunda
// ichki elementga YANGI island bermaymiz — bitta island, ko'p emas).
fn mark_walk(
    node: &mut Value,
    next_id: &mut u32,
    inside_island: bool,
    react_state: &BTreeMap<String, Value>,
) -> u32 {
    let Value::Map(m) = node else {
        return 0;
    };
    if !matches!(m.get("__node"), Some(Value::Bool(true))) {
        return 0;
    }

    // Fragment (ko'rinmas o'rov) — HTML'da tegi yo'q, shuning uchun island ildizi
    // BO'LA OLMAYDI (marker qo'yadigan element yo'q). Faqat bolalariga o'tamiz.
    let is_fragment = matches!(m.get("tag"), Some(Value::Str(t)) if t == "__fragment");

    // Bu element o'zida interaktivlik izi (on:/bind:) bormi.
    let (on_marker, bind_marker) = extract_event_bind(m);
    let self_interactive = on_marker.is_some() || bind_marker.is_some();

    // Subtree interaktivmi (o'zi yoki biror bolasi). Island ildizini aniqlash
    // uchun: agar biz island ichida EMASMIZ va subtree interaktiv bo'lsa, bu
    // element island ildizi (eng kichik o'rovchi — chunki yuqoridan tushganimizda
    // eng birinchi interaktiv element shu).
    let subtree_interactive = self_interactive || children_interactive(m);

    let mut count = 0;
    let mut now_inside = inside_island;

    if !inside_island && !is_fragment && subtree_interactive {
        // Island ildizi shu element.
        let id = *next_id;
        *next_id += 1;
        m.insert("__island".to_string(), Value::Int(id as i64));
        // PR-6: reaktiv state'ni island ildiziga JSON sifatida yozamiz (STATELESS —
        // client keyingi event'da qaytaradi). Bo'sh bo'lsa yozmaymiz (GET render).
        if !react_state.is_empty() {
            let json = crate::builtins::json_encode(&Value::Map(react_state.clone()));
            m.insert("__state".to_string(), Value::Str(json));
        }
        count += 1;
        now_inside = true;
    }

    // on:/bind: markerlarini shu elementga qo'shamiz (island ichida bo'lsa ham).
    if let Some(on) = on_marker {
        m.insert("__on".to_string(), Value::Str(on));
    }
    if let Some(b) = bind_marker {
        m.insert("__bind".to_string(), Value::Str(b));
    }

    // Bolalarga rekursiv (island ichidamizmi holatini uzatib).
    if let Some(Value::List(children)) = m.get_mut("children") {
        for c in children.iter_mut() {
            count += mark_walk(c, next_id, now_inside, react_state);
        }
    }
    count
}

// Element props'idan on:/bind: izini ajratadi (marker string sifatida).
// on -> "event:handler" (event default "click"); bind -> "state_nomi".
fn extract_event_bind(node: &BTreeMap<String, Value>) -> (Option<String>, Option<String>) {
    let Some(Value::Map(props)) = node.get("props") else {
        return (None, None);
    };
    // on: qiymati (eval_element_props bergan): Str=handler nomi, Sym=event/belgi.
    // Marker formati "event:handler". PR-4b'da event default "click" (aniq event
    // sintaksisi keyingi PR); handler nomi bo'lsa o'shani, lambda bo'lsa "_".
    let on = props.get("on").map(|v| match v {
        Value::Str(handler) => format!("click:{}", handler),
        Value::Sym(_) => "click:_".to_string(),
        _ => "click:_".to_string(),
    });
    // bind: qiymati — state nomi (Str). eval_element_props ident'ni nom qilib saqlaydi.
    let bind = props.get("bind").map(|v| v.to_text());
    (on, bind)
}

// Node bolalaridan birortasi interaktivmi (rekursiv, o'zini hisobga olmasdan).
fn children_interactive(node: &BTreeMap<String, Value>) -> bool {
    let Some(Value::List(children)) = node.get("children") else {
        return false;
    };
    children.iter().any(node_interactive)
}

// `__island == target` bo'lgan node'ni daraxtdan topadi (re-render uchun).
fn find_island(node: &Value, target: i64) -> Option<&Value> {
    let Value::Map(m) = node else {
        return None;
    };
    if let Some(Value::Int(id)) = m.get("__island")
        && *id == target
    {
        return Some(node);
    }
    if let Some(Value::List(children)) = m.get("children") {
        for c in children {
            if let Some(found) = find_island(c, target) {
                return Some(found);
            }
        }
    }
    None
}

// Client runtime JS (PR-5a) — /_fx/client.js da beriladi. include_str! bilan
// crate ichida (ai_mod $AI_KEY env-resurs naqshi). Faqat island bor sahifaga
// yuklanadi (window.__fx mavjud bo'lsa client o'zini ishga tushiradi).
pub const CLIENT_JS: &str = include_str!("ui_client.js");

// Node (yoki uning subtree'si) interaktivmi (on:/bind: izi bor).
fn node_interactive(v: &Value) -> bool {
    let Value::Map(m) = v else {
        return false;
    };
    if !matches!(m.get("__node"), Some(Value::Bool(true))) {
        return false;
    }
    let (on, bind) = extract_event_bind(m);
    on.is_some() || bind.is_some() || children_interactive(m)
}

// `ui.*` dispatch — Interp'ga ulanadi (kelajakda `ui.serve` state kerak).
// MVP'da faqat `ui.html`. eval_call shu yerga yo'naltiradi.
impl Interp {
    pub fn ui_dispatch(self: &Arc<Self>, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            // ui.html node -> str (element/komponent server-side render). To'liq
            // hujjat EMAS — faqat element daraxti HTML'i (3-bosqich ui.serve to'liq
            // sahifani theme + body bilan birlashtiradi). Argument {__node} yoki nil.
            "html" => {
                let mut node = args.first().cloned().unwrap_or(Value::Nil);
                let mut id = 1;
                // GET render — reaktiv state bo'sh (data-fx-state yo'q, initial seed).
                mark_islands(&mut node, &mut id, &BTreeMap::new());
                Ok(Value::Str(node_to_html(&node)))
            }
            // ui.css -> str: theme tokenlaridan CSS custom properties + base CSS.
            // `<style>` ichiga qo'yiladi (ui.serve yoki qo'lda).
            "css" => {
                let theme = self.theme.read();
                Ok(Value::Str(theme_to_css(&theme)))
            }
            // ui.page node -> str: to'liq HTML hujjat (doctype + head[theme css] +
            // body[node] + island markerlar + window.__fx). render_page bilan bir xil.
            "page" => {
                let node = args.first().cloned().unwrap_or(Value::Nil);
                Ok(Value::Str(self.render_page(&node)))
            }
            // ui.serve [app] port — frontend serverini DARHOL bloklamaydi, deferred
            // ro'yxatga qo'shadi (http.serve naqshi). Top-level tugagach bitta umumiy
            // event-loopda ishga tushadi. `app` argument ixtiyoriy (3-bosqichda
            // `page` marshrutlari to'g'ridan ishlatiladi); port = oxirgi int argument.
            "serve" => {
                let port = args.iter().rev().find_map(|a| match a {
                    Value::Int(n) => Some(*n as u16),
                    _ => None,
                });
                let port = match port {
                    Some(p) => p,
                    None => return Err(Flow::err("ui.serve: port (int) bo'lishi kerak")),
                };
                self.pending_servers
                    .lock()
                    .unwrap()
                    .push(crate::serve_mod::PendingServer::Ui { port });
                Ok(Value::Nil)
            }
            // ui.invalidate :tag — LOKAL source qayta yuklash signali. Stateless,
            // joriy klient o'zini event re-render orqali yangilaydi (/_fx/event'da
            // view QAYTA eval -> source qayta bajariladi). No-op (Nil) — `ui.invalidate
            // :items` yozsa parse/eval buzilmasin. Broadcast egizagi = ui.push.
            "invalidate" => Ok(Value::Nil),
            // ui.push :tag — BROADCAST source reload (PR-7b). Tag room'iga (":tag")
            // ulangan BARCHA live klientlarga "reload" yuboradi (WS). Klient o'sha
            // tag source'ini qayta yuklaydi. Server mutation'idan keyin chaqiriladi
            // (`fn save_order d ... ui.push :orders`). Istalgan thread'dan xavfsiz.
            "push" => {
                let tag = match args.first() {
                    Some(Value::Sym(s)) | Some(Value::Str(s)) => s.clone(),
                    _ => return Err(Flow::err("ui.push: tag (:sym yoki str) kerak")),
                };
                let room = format!(":{}", tag);
                // json_encode tag'ni to'g'ri quote/escape qiladi (Value::Str -> "...").
                let tag_json = crate::builtins::json_encode(&Value::Str(tag));
                let msg = format!("{{\"fx\":\"reload\",\"tag\":{}}}", tag_json);
                self.ws.push_tag(&room, &msg);
                Ok(Value::Nil)
            }
            _ => Err(Flow::err(format!("ui.{} funksiyasi yo'q", func))),
        }
    }
}

// theme tokenlarini CSS custom properties'ga aylantiradi + base semantik CSS.
//   theme {primary "#e84d8a" radius :lg}  ->  :root{--primary:#e84d8a;--radius:lg}
fn theme_to_css(theme: &BTreeMap<String, Value>) -> String {
    let mut out = String::new();
    out.push_str(":root{");
    for (k, v) in theme {
        out.push_str("--");
        out.push_str(k);
        out.push(':');
        out.push_str(&v.to_text());
        out.push(';');
    }
    out.push('}');
    // Semantik proplar uchun minimal base CSS (kind/pad/gap). To'liq dizayn
    // tizimi keyingi bosqich; hozir tokenlarni ishlatadigan asos.
    out.push_str(BASE_CSS);
    out
}

// Semantik prop class'lari uchun minimal base CSS. theme tokenlariga (`--primary`
// va h.k.) bog'lanadi, shunda `{kind::primary}` -> `.flux-primary` rang oladi.
const BASE_CSS: &str = "\
.flux-primary{background:var(--primary,#333);color:#fff}\
.flux-muted{color:var(--muted,#888)}\
.flux-panel{padding:1rem;border-radius:var(--radius,8px);background:var(--surface,#fff)}\
.flux-badge{display:inline-block;padding:.2em .5em;border-radius:.4em;font-size:.85em}";

// To'liq HTML hujjat: doctype + head (theme CSS) + body (element HTML).
// island_count > 0 bo'lsa body oxiriga `window.__fx` bootstrap script qo'shiladi
// (PR-4b minimal: island ro'yxati + mode; PR-5 to'ldiradi). 0 island = 0 JS
// (sof statik sahifa CDN-cacheable).
fn full_document(
    css: &str,
    body_html: &str,
    island_count: u32,
    path: &str,
    live: &[String],
) -> String {
    let script = fx_bootstrap_script(island_count, path, live);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<style>{}</style></head><body>{}{}</body></html>",
        css, body_html, script
    )
}

// PR-5a bootstrap: window.__fx (page + island ro'yxati + mode) + client.js yuklash.
// `page` — client /_fx/event POST'da qaytaradi (server qaysi view ekanini biladi,
// stateless). 0 island = script YO'Q (sof statik, 0 JS — CDN-cacheable invariant).
fn fx_bootstrap_script(island_count: u32, path: &str, live: &[String]) -> String {
    // 0 island VA 0 live source = script YO'Q (sof statik, 0 JS — CDN-cacheable).
    // PR-7b: live source bo'lsa (island bo'lmasa ham) client.js WS uchun kerak.
    if island_count == 0 && live.is_empty() {
        return String::new();
    }
    let mut islands = String::new();
    for i in 1..=island_count {
        if i > 1 {
            islands.push(',');
        }
        islands.push_str(&format!("\"{}\":{{\"mode\":\"server\"}}", i));
    }
    // PR-7b: live source tag'lari -> window.__fx.live (client WS subscribe qiladi).
    let live_json = crate::builtins::json_encode(&Value::List(
        live.iter().map(|t| Value::Str(t.clone())).collect(),
    ));
    format!(
        "<script>window.__fx={{\"page\":\"{}\",\"islands\":{{{}}},\"live\":{}}}</script>\
<script src=\"/_fx/client.js\"></script>",
        escape_attr(path),
        islands,
        live_json
    )
}

// --- ui.serve: SSR sahifa + /api/* http routes bitta portda ---

// Bitta UI server uchun accept loop (http_mod::serve_loop naqshi). Umumiy
// event-loopda spawn qilinadi (serve_mod). Bind'ni shu yerda bajaradi (deferred).
pub async fn serve_loop(interp: Arc<Interp>, port: u16) {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Flux UI port {} bind xatosi: {}", port, e);
            return;
        }
    };
    eprintln!("Flux UI server: http://localhost:{}", port);

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("ui accept xatosi: {}", e);
                continue;
            }
        };
        let io = TokioIo::new(stream);
        let interp = interp.clone();
        tokio::spawn(async move {
            let service = service_fn(move |req: Request<Incoming>| {
                let interp = interp.clone();
                async move { ui_handle_request(interp, req).await }
            });
            // .with_upgrades() — bir portda WS upgrade (/_fx/ws) uchun SHART
            // (PR-7b). Aks holda hyper::upgrade::on hech qachon hal bo'lmaydi.
            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .with_upgrades()
                .await
            {
                eprintln!("ui ulanish xatosi: {}", e);
            }
        });
    }
}

// Bitta UI so'rovini boshqaradi. Dispatch tartibi:
//   1. http `routes` (http.on bilan ro'yxatga olingan, masalan /api/*) — REST javob.
//   2. `pages` (page bilan ro'yxatga olingan, GET) — SSR HTML sahifa.
//   3. topilmasa — 404.
// REST oldin: API endpoint'lar UI sahifalardan ustun (aniqroq, /api prefiksli).
async fn ui_handle_request(
    interp: Arc<Interp>,
    mut req: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method().as_str().to_lowercase();
    let uri = req.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or("").to_string();

    // 0) Maxsus /_fx/* yo'llari (frontend runtime) — boshqa hammasidan oldin.
    // /_fx/client.js — universal client JS (statik, keshlanadigan).
    if method == "get" && path == "/_fx/client.js" {
        return Ok(js_response(crate::ui_mod::CLIENT_JS));
    }
    // /_fx/ws — WebSocket upgrade (PR-7b, live source realtime, BIR PORTDA).
    // Body o'qishdan OLDIN bo'lishi shart (upgrade body consume qilmaydi).
    if method == "get" && path == "/_fx/ws" {
        return Ok(fx_ws_upgrade(interp, &mut req));
    }
    // /_fx/event — island re-render (PR-5a, server-driven, stateless POST).
    if method == "post" && path == "/_fx/event" {
        let body_bytes = match req.into_body().collect().await {
            Ok(c) => c.to_bytes(),
            Err(_) => Bytes::new(),
        };
        return Ok(handle_fx_event(interp, &body_bytes).await);
    }

    // Sarlavhalar (http_mod naqshi: lowercase, '-' -> '_').
    let mut headers = BTreeMap::new();
    let mut is_json = false;
    for (k, v) in req.headers() {
        let key = k.as_str().to_lowercase().replace('-', "_");
        let val = v.to_str().unwrap_or("").to_string();
        if key == "content_type" && val.contains("application/json") {
            is_json = true;
        }
        headers.insert(key, Value::Str(val));
    }

    // 1) http routes (REST/API) — bor bo'lsa o'sha javob.
    let api_match = {
        let routes = interp.routes.lock().unwrap();
        crate::http_mod::match_route(&routes, &method, &path)
    };
    // 2) page routes (SSR) — faqat GET.
    let page_match = if method == "get" {
        let pages = interp.pages.lock().unwrap();
        crate::http_mod::match_route(&pages, &method, &path)
    } else {
        None
    };

    let (route, params, is_page) = match (api_match, page_match) {
        (Some((r, p)), _) => (r, p, false),
        (None, Some((r, p))) => (r, p, true),
        (None, None) => {
            return Ok(crate::http_mod::json_response(
                404,
                format!("{{\"error\":\"topilmadi: {} {}\"}}", method, path),
            ));
        }
    };

    // Tanani yig'amiz (page GET'da odatda bo'sh).
    let body_bytes = match req.into_body().collect().await {
        Ok(c) => c.to_bytes(),
        Err(_) => Bytes::new(),
    };
    // Page render uchun path'ni saqlaymiz (build_req uni move qiladi) — client
    // /_fx/event POST'da shu path'ni qaytaradi (qaysi view re-render bo'lishi).
    let page_path = path.clone();
    let request_value =
        crate::http_mod::build_req(method, path, query, headers, params, body_bytes, is_json);
    let handler = route.handler;

    // Handler'ni blocking thread'da chaqiramiz (sinxron interp) — Value qaytaradi.
    // page/REST ajratishni TASHQARIDA qilamiz (REST -> Response, page -> HTML).
    let interp2 = interp.clone();
    let result = tokio::task::spawn_blocking(move || {
        // page view'lar req argument OLISHI SHART EMAS (`page "/" -> dashboard`).
        // Handler arity 0 bo'lsa argumentsiz, aks holda req bilan chaqiramiz —
        // shunda ham `view home` (0 param) ham `\req -> ...` (1 param) ishlaydi.
        let args = if is_page && interp2.fn_arity(&handler) == Some(0) {
            vec![]
        } else {
            vec![request_value]
        };
        // page (UI) render'ida FX kontekst: on:click lambda'lar barqaror `#N` marker
        // oladi (client birinchi click'da to'g'ri indeksni biladi). REST'da kerak emas.
        let _hguard = is_page.then(crate::interp::FxHandlerGuard::set);
        // PR-7b: `source live` tag'lari handler eval davomida yig'iladi (render_page_at
        // FxLiveGuard::take() bilan oladi -> window.__fx.live -> client WS subscribe).
        let _lguard = is_page.then(crate::interp::FxLiveGuard::set);
        let v = interp2.apply(handler, args)?;
        // page bo'lsa shu thread'da HTML render qilamiz (theme o'qish ham bu yerda),
        // REST bo'lsa xom Value qaytaramiz (tashqarida value_to_response).
        if is_page {
            Ok(PageOrRest::Page(interp2.render_page_at(&v, &page_path)))
        } else {
            Ok(PageOrRest::Rest(v))
        }
    })
    .await;

    match result {
        Ok(Ok(PageOrRest::Page(html))) => Ok(html_response(&html)),
        Ok(Ok(PageOrRest::Rest(v))) => Ok(crate::http_mod::value_to_response(v)),
        Ok(Err(flow)) => Ok(crate::http_mod::json_response(
            500,
            format!("{{\"error\":\"{}\"}}", flow_message(&flow)),
        )),
        Err(join_err) => Ok(crate::http_mod::json_response(
            500,
            format!("{{\"error\":\"handler panic: {}\"}}", join_err),
        )),
    }
}

// Handler natijasi: page (render qilingan HTML) yoki REST (xom Value).
enum PageOrRest {
    Page(String),
    Rest(Value),
}

// /_fx/ws — WebSocket upgrade (PR-7b, BIR PORTDA HTTP+WS). hyper 1.x upgrade:
// 101 Switching Protocols qaytaramiz, fon task upgraded socketni egallab WS qiladi.
// `serve_loop` `.with_upgrades()` bo'lishi SHART (aks holda on_upgrade osiladi).
fn fx_ws_upgrade(interp: Arc<Interp>, req: &mut Request<Incoming>) -> Response<Full<Bytes>> {
    // WebSocket handshake header'lari: upgrade:websocket + sec-websocket-key.
    let key = req
        .headers()
        .get("sec-websocket-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let Some(key) = key else {
        return crate::http_mod::json_response(
            400,
            "{\"error\":\"WS handshake: kalit yo'q\"}".to_string(),
        );
    };
    // Sec-WebSocket-Accept (tungstenite helper — SHA1+base64 magic GUID).
    let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes());

    // Upgrade'ni so'raymiz (req'dan, body consume QILINMAYDI). Fon task'da hal bo'ladi.
    let on_upgrade = hyper::upgrade::on(req);
    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => {
                // Upgraded hyper rt traitlarini beradi -> TokioIo wrapper SHART
                // (from_raw_socket tokio traitlarini kutadi).
                let io = TokioIo::new(upgraded);
                let ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
                    io,
                    tokio_tungstenite::tungstenite::protocol::Role::Server,
                    None,
                )
                .await;
                crate::ws_mod::handle_ui_conn(interp, ws).await;
            }
            Err(e) => eprintln!("ui ws upgrade xatosi: {}", e),
        }
    });

    // 101 Switching Protocols — hyper buni yuboradi, keyin spawn'langan task socketni oladi.
    Response::builder()
        .status(101)
        .header("upgrade", "websocket")
        .header("connection", "Upgrade")
        .header("sec-websocket-accept", accept)
        .body(Full::new(Bytes::new()))
        .unwrap()
}

// Flow xato xabarini oladi (json xato uchun).
fn flow_message(flow: &Flow) -> String {
    match flow {
        Flow::Error(e) => e.clone(),
        Flow::Fail { message, .. } => message.clone(),
        _ => "noma'lum xato".to_string(),
    }
}

// HTML javob (text/html).
fn html_response(html: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(200)
        .header("content-type", "text/html; charset=utf-8")
        .body(Full::new(Bytes::from(html.to_string())))
        .unwrap()
}

// JS javob (statik client.js — keshlanadigan).
fn js_response(js: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(200)
        .header("content-type", "application/javascript; charset=utf-8")
        .header("cache-control", "public, max-age=3600")
        .body(Full::new(Bytes::from(js.to_string())))
        .unwrap()
}

// /_fx/event — island re-render (PR-5a, server-driven, stateless).
// POST tanasi: {page, island, event, handler, state}. Oqim: page handler'ni
// client state seed bilan re-render -> island N node'ini topib HTML qaytarish.
// Faqat STATE-DRIVEN (bind:) — handler-effekt (on:) PR-6 (handler tanasi kerak).
async fn handle_fx_event(interp: Arc<Interp>, body: &[u8]) -> Response<Full<Bytes>> {
    let body = body.to_vec();
    let result = tokio::task::spawn_blocking(move || fx_event_render(&interp, &body)).await;
    match result {
        Ok(Ok(html)) => html_response(&html),
        Ok(Err(flow)) => crate::http_mod::json_response(
            500,
            format!("{{\"error\":\"{}\"}}", flow_message(&flow)),
        ),
        Err(e) => {
            crate::http_mod::json_response(500, format!("{{\"error\":\"event panic: {}\"}}", e))
        }
    }
}

// Event JSON'ni parse qilib island'ni client state ostida re-render qiladi.
// Sinxron (spawn_blocking ichida chaqiriladi). pub(crate): integratsiya testi
// async serverni ochmasdan to'g'ridan chaqiradi.
pub(crate) fn fx_event_render(interp: &Arc<Interp>, body: &[u8]) -> Result<String, Flow> {
    let s = String::from_utf8_lossy(body);
    let payload = crate::builtins::json_decode(&s)
        .map_err(|e| Flow::err(format!("/_fx/event JSON parse: {}", flow_message(&e))))?;
    let Value::Map(m) = payload else {
        return Err(Flow::err("/_fx/event: JSON obyekt kutilgan"));
    };
    // page (qaysi view), island (qaysi qism), state (client React state'i).
    let page = match m.get("page") {
        Some(Value::Str(p)) => p.clone(),
        _ => "/".to_string(),
    };
    let island_id = match m.get("island") {
        Some(Value::Str(s)) => s.parse::<i64>().unwrap_or(0),
        Some(Value::Int(n)) => *n,
        _ => 0,
    };
    let client_state = match m.get("state") {
        Some(Value::Map(st)) => st.clone(),
        _ => BTreeMap::new(),
    };
    // event turi ("click" -> handler-effekt, PR-6; "input" -> bind-driven re-render,
    // PR-5a). handler — click uchun `#N` (registr indeksi) yoki nom.
    let event = match m.get("event") {
        Some(Value::Str(e)) => e.clone(),
        _ => "input".to_string(),
    };
    let handler = match m.get("handler") {
        Some(Value::Str(h)) => h.clone(),
        _ => String::new(),
    };

    // page bo'yicha handler topish (pages route'lari, GET).
    let matched = {
        let pages = interp.pages.lock().unwrap();
        crate::http_mod::match_route(&pages, "get", &page)
    };
    let (route, _params) =
        matched.ok_or_else(|| Flow::err(format!("/_fx/event: page topilmadi: {}", page)))?;

    let page_args = |arity: Option<usize>| -> Vec<Value> {
        if arity == Some(0) {
            vec![]
        } else {
            // page handler req kutsa — minimal bo'sh req (state seed orqali ishlaydi).
            vec![Value::Map(BTreeMap::new())]
        }
    };

    // PR-6: on:click handler-effekt. Handler `#N` bo'lsa registr indeksi -> view'ni
    // FX kontekstda render (registr to'ladi) -> handler'ni view scope'da apply
    // (`count <- count+1` scope'dagi count'ni yangilaydi) -> yangi reaktiv state'ni
    // scope'dan o'qib SEED qilib toza re-render. STATELESS: yangi state HTML'ga
    // data-fx-state bo'lib yoziladi, client keyingi event'da qaytaradi.
    if event == "click" && handler.starts_with('#') {
        let idx: usize = handler[1..]
            .parse()
            .map_err(|_| Flow::err(format!("/_fx/event: yaroqsiz handler indeksi: {}", handler)))?;

        // exec1: FX kontekst (handler registri 0'dan to'planadi) + client state seed.
        // MUHIM: handler indeksi BARQAROR bo'lishi uchun har render guard'ni QAYTA
        // o'rnatadi (registr 0'dan) — GET render, exec1, exec2 bir xil tartib beradi.
        let _hguard = crate::interp::FxHandlerGuard::set();
        let scope1 = {
            let _sguard = crate::interp::FxRenderGuard::set(client_state.clone());
            let args = page_args(interp.fn_arity(&route.handler));
            let (_tree1, scope1) = interp.render_view_with_scope(route.handler.clone(), args)?;
            scope1
            // _sguard shu yerda drop — SEED faqat dastlabki render uchun. Handler
            // apply paytida seed AKTIV bo'lsa, `count <- count+1` ichidagi `count`
            // Assign'ni seed client qiymatiga override qilib qo'yardi (bug).
        };

        // Handler'ni view scope'da apply — registr (_hguard) hali aktiv, seed YO'Q.
        // `count <- count+1` scope1'dagi count'ni haqiqatan oshiradi.
        let lambda = crate::interp::fx_handler_get(idx)
            .ok_or_else(|| Flow::err(format!("/_fx/event: handler #{} topilmadi", idx)))?;
        interp.apply(lambda, vec![])?;

        // Yangi reaktiv state: handler body'dagi `<-` nomlarini scope1'dan o'qiymiz,
        // ustiga client bind input'larini saqlaymiz (ular DOM'da, scope'da emas).
        let mut new_state = client_state.clone();
        for name in react_bind_names(&route.handler) {
            if let Some(v) = interp.read_var(&scope1, &name) {
                new_state.insert(name, v);
            }
        }

        // exec2: toza re-render, yangi state SEED + FX kontekst (handler markerlari
        // YANA #0'dan — client javobdagi markerni keyingi click'da to'g'ri yuboradi).
        let tree2 = {
            let _hguard = crate::interp::FxHandlerGuard::set();
            let _sguard = crate::interp::FxRenderGuard::set(new_state.clone());
            let args = page_args(interp.fn_arity(&route.handler));
            interp.apply(route.handler.clone(), args)?
        };
        return render_island_html(tree2, island_id, &new_state);
    }

    // PR-5a: bind-driven (input) — client state seed qilib view re-render (effekt yo'q).
    let _guard = crate::interp::FxRenderGuard::set(client_state.clone());
    let args = page_args(interp.fn_arity(&route.handler));
    let tree = interp.apply(route.handler, args)?;
    // bind-driven javobda ham reaktiv state'ni saqlaymiz (client state'ni qaytaramiz,
    // shunda non-input state, masalan count, yo'qolmaydi).
    render_island_html(tree, island_id, &client_state)
}

// Render qilingan daraxtdan island N node'ini topib HTML qaytaradi. `react_state`
// island ildiziga data-fx-state bo'lib yoziladi (STATELESS).
fn render_island_html(
    mut tree: Value,
    island_id: i64,
    react_state: &BTreeMap<String, Value>,
) -> Result<String, Flow> {
    let mut next_id = 1u32;
    mark_islands(&mut tree, &mut next_id, react_state);
    match find_island(&tree, island_id) {
        Some(node) => Ok(node_to_html(node)),
        None => Err(Flow::err(format!(
            "/_fx/event: island {} topilmadi",
            island_id
        ))),
    }
}

// page handler (view fn yoki lambda) tanasidagi reaktiv (`<-`) bind nomlarini
// yig'adi (PR-6: handler bajarilgandan keyin scope'dan shu nomlarni o'qiymiz).
// View body'dagi top-level `Stmt::Assign` nomlari = React state.
fn react_bind_names(handler: &Value) -> Vec<String> {
    let Value::Fn(fv) = handler else {
        return vec![];
    };
    collect_assign_names(&fv.body)
}

// Stmt ro'yxatidagi `<-` (Assign) nomlarini yig'adi (nested each/if/match ham).
fn collect_assign_names(stmts: &[crate::ast::Stmt]) -> Vec<String> {
    let mut out = Vec::new();
    fn walk_stmt(s: &crate::ast::Stmt, out: &mut Vec<String>) {
        use crate::ast::Stmt;
        match s {
            Stmt::Assign { name, .. } => out.push(name.clone()),
            Stmt::Each { body, .. } => {
                for s in body {
                    walk_stmt(s, out);
                }
            }
            Stmt::Expr(e) => walk_expr(e, out),
            _ => {}
        }
    }
    fn walk_expr(e: &crate::ast::Expr, out: &mut Vec<String>) {
        use crate::ast::Expr;
        match e {
            Expr::If(ifx) => {
                for (_, block) in &ifx.arms {
                    for s in block {
                        walk_stmt(s, out);
                    }
                }
                if let Some(eb) = &ifx.else_block {
                    for s in eb {
                        walk_stmt(s, out);
                    }
                }
            }
            Expr::Match(mx) => {
                for arm in &mx.arms {
                    for s in &arm.body {
                        walk_stmt(s, out);
                    }
                }
            }
            Expr::Children(stmts) => {
                for s in stmts {
                    walk_stmt(s, out);
                }
            }
            _ => {}
        }
    }
    for s in stmts {
        walk_stmt(s, &mut out);
    }
    out
}

impl Interp {
    // Funksiya qiymatining parametr sonini qaytaradi (Value::Fn). Native yoki
    // boshqa qiymat uchun None (arity noma'lum -> req beriladi).
    pub fn fn_arity(&self, f: &Value) -> Option<usize> {
        match f {
            Value::Fn(fv) => Some(fv.params.len()),
            _ => None,
        }
    }

    // page handler natijasini (element daraxti) to'liq HTML hujjatga aylantiradi
    // (theme CSS + body + island markerlar + window.__fx). `path` — joriy URL
    // (client /_fx/event POST'ida qaytaradi, server qaysi view ekanini biladi).
    pub fn render_page_at(&self, node: &Value, path: &str) -> String {
        // Island markerlar (PR-4b) — node clone'iga qo'shamiz (kiruvchi o'zgarmaydi).
        let mut node = node.clone();
        let mut next_id = 1u32;
        // GET render — reaktiv state bo'sh (initial qiymat literal seed'dan keladi,
        // birinchi click to'g'ri; keyingilar data-fx-state orqali).
        let island_count = mark_islands(&mut node, &mut next_id, &BTreeMap::new());
        let css = {
            let theme = self.theme.read();
            theme_to_css(&theme)
        };
        // PR-7b: render davomida yig'ilgan `source live` tag'larini olamiz (page
        // handler FxLiveGuard ostida eval qilingan — render_page_with_live qarang).
        let live = crate::interp::FxLiveGuard::take();
        full_document(&css, &node_to_html(&node), island_count, path, &live)
    }

    // ui.page (qo'lda render) — path noma'lum, "/" default.
    pub fn render_page(&self, node: &Value) -> String {
        self.render_page_at(node, "/")
    }
}

// --- SSR: element daraxti -> HTML string (sof funksiya) ---

// `{__node}` map'ni HTML stringga aylantiradi. Element bo'lmagan qiymat (matn)
// to'g'ridan-to'g'ri escape qilinib chiqadi. nil -> bo'sh string.
pub fn node_to_html(v: &Value) -> String {
    match v {
        Value::Nil => String::new(),
        Value::Map(m) if is_node(v) => {
            let tag = match m.get("tag") {
                Some(Value::Str(t)) => t.as_str(),
                _ => "div",
            };
            // Fragment — ko'rinmas o'rov: faqat bolalarni render qiladi (teg yo'q).
            if tag == "__fragment" {
                let mut out = String::new();
                if let Some(Value::List(items)) = m.get("children") {
                    for c in items {
                        out.push_str(&node_to_html(c));
                    }
                }
                return out;
            }
            let html_tag = html_tag_name(tag);
            let mut out = String::new();
            out.push('<');
            out.push_str(html_tag);
            out.push_str(&attrs_html(tag, m.get("props")));
            // PR-4b island markerlari (mark_islands qo'ygan): data-fx-*.
            out.push_str(&fx_markers_html(m));
            if is_void_tag(html_tag) {
                out.push_str(" />");
                return out;
            }
            out.push('>');
            // text bola (escape qilinadi).
            if let Some(Value::Str(t)) = m.get("text") {
                out.push_str(&escape_html(t));
            }
            // children (rekursiv render).
            if let Some(Value::List(items)) = m.get("children") {
                for c in items {
                    out.push_str(&node_to_html(c));
                }
            }
            out.push_str("</");
            out.push_str(html_tag);
            out.push('>');
            out
        }
        // Element bo'lmagan qiymat (matn/son) — escape qilingan matn.
        other => escape_html(&other.to_text()),
    }
}

// Flux teg nomini HTML teg nomiga moslaydi (semantik nomlar -> HTML).
fn html_tag_name(tag: &str) -> &str {
    match tag {
        "btn" => "button",
        "badge" => "span",
        other => other,
    }
}

// Yopilmaydigan (void) HTML teglari — bola/yopuvchi teg olmaydi.
fn is_void_tag(html_tag: &str) -> bool {
    matches!(html_tag, "img" | "input" | "br" | "hr")
}

// Props map'ni HTML atributlariga aylantiradi. MVP'da semantik proplar CSS
// class'ga aylanadi (`kind::primary pad:4` -> class="flux-primary flux-pad-4`),
// `id`/`href`/`src`/`placeholder`/`type`/`value`/`alt` esa to'g'ridan-to'g'ri
// HTML atributi bo'ladi. `on:`/`bind:` (event/binding) keyingi bosqich — MVP'da
// e'tiborsiz qoldiriladi (statik render).
fn attrs_html(tag: &str, props: Option<&Value>) -> String {
    let Some(Value::Map(p)) = props else {
        // `badge` semantik teg — base class beriladi.
        return base_class_attr(tag, &[]);
    };
    let mut classes: Vec<String> = Vec::new();
    let mut attrs: Vec<(String, String)> = Vec::new();
    for (k, v) in p {
        // event/binding proplari MVP'da statik renderda chiqarilmaydi.
        if k == "on" || k == "bind" {
            continue;
        }
        // To'g'ridan-to'g'ri HTML atributlari.
        if matches!(
            k.as_str(),
            "id" | "href" | "src" | "placeholder" | "type" | "value" | "alt" | "name" | "title"
        ) {
            attrs.push((k.clone(), v.to_text()));
            continue;
        }
        // Qolgani semantik prop -> CSS class `flux-<k>-<v>` yoki `flux-<v>`.
        match v {
            Value::Sym(s) => classes.push(format!("flux-{}", s)),
            Value::Bool(true) => classes.push(format!("flux-{}", k)),
            Value::Bool(false) | Value::Nil => {}
            other => classes.push(format!("flux-{}-{}", k, other.to_text())),
        }
    }
    let mut out = base_class_attr(tag, &classes);
    for (k, val) in attrs {
        out.push(' ');
        out.push_str(&escape_attr(&k));
        out.push_str("=\"");
        out.push_str(&escape_attr(&val));
        out.push('"');
    }
    out
}

// `badge` kabi semantik teglar uchun base class + qo'shimcha class'lar.
fn base_class_attr(tag: &str, extra: &[String]) -> String {
    let mut classes: Vec<String> = Vec::new();
    if tag == "badge" {
        classes.push("flux-badge".to_string());
    }
    classes.extend(extra.iter().cloned());
    if classes.is_empty() {
        return String::new();
    }
    format!(" class=\"{}\"", escape_attr(&classes.join(" ")))
}

// PR-4b: island markerlarini (mark_islands qo'ygan `__island`/`__on`/`__bind`)
// data-fx-* atributlariga aylantiradi. PR-5 bu markerlarni client'da ishlatadi.
fn fx_markers_html(node: &BTreeMap<String, Value>) -> String {
    let mut out = String::new();
    if let Some(Value::Int(id)) = node.get("__island") {
        out.push_str(&format!(" data-fx-island=\"{}\"", id));
    }
    if let Some(Value::Str(on)) = node.get("__on") {
        out.push_str(&format!(" data-fx-on=\"{}\"", escape_attr(on)));
    }
    if let Some(Value::Str(b)) = node.get("__bind") {
        out.push_str(&format!(" data-fx-bind=\"{}\"", escape_attr(b)));
    }
    // PR-6: reaktiv state JSON (STATELESS — client keyingi event'da qaytaradi).
    if let Some(Value::Str(state)) = node.get("__state") {
        out.push_str(&format!(" data-fx-state=\"{}\"", escape_attr(state)));
    }
    out
}

// HTML matn kontekstida xavfli belgilarni escape qiladi.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

// HTML atribut qiymati kontekstida escape (qo'shtirnoq ham).
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(tag: &str, args: Vec<Value>) -> Value {
        // Flow Debug implement qilmaydi — .unwrap() o'rniga match.
        match build_node(tag, args) {
            Ok(v) => v,
            Err(_) => panic!("build_node xato qaytardi"),
        }
    }

    #[test]
    fn oddiy_matnli_element() {
        let n = node("h1", vec![Value::Str("Salom".into())]);
        assert_eq!(node_to_html(&n), "<h1>Salom</h1>");
    }

    #[test]
    fn matn_escape_qilinadi() {
        let n = node("p", vec![Value::Str("a < b & c".into())]);
        assert_eq!(node_to_html(&n), "<p>a &lt; b &amp; c</p>");
    }

    #[test]
    fn nested_children() {
        let inner = node("h1", vec![Value::Str("Sarlavha".into())]);
        let p = node("p", vec![Value::Str("matn".into())]);
        let outer = node("div", vec![Value::List(vec![inner, p])]);
        assert_eq!(
            node_to_html(&outer),
            "<div><h1>Sarlavha</h1><p>matn</p></div>"
        );
    }

    #[test]
    fn semantik_prop_class_boladi() {
        let mut props = BTreeMap::new();
        props.insert("kind".to_string(), Value::Sym("primary".into()));
        props.insert("pad".to_string(), Value::Int(4));
        let n = node("btn", vec![Value::Str("Saqlash".into()), Value::Map(props)]);
        let html = node_to_html(&n);
        // btn -> button, kind::primary -> flux-primary, pad:4 -> flux-pad-4
        assert!(html.starts_with("<button class=\""), "html: {}", html);
        assert!(html.contains("flux-primary"), "html: {}", html);
        assert!(html.contains("flux-pad-4"), "html: {}", html);
        assert!(html.contains(">Saqlash</button>"), "html: {}", html);
    }

    #[test]
    fn html_atribut_togridan() {
        let mut props = BTreeMap::new();
        props.insert("src".to_string(), Value::Str("/rasm.png".into()));
        props.insert("alt".to_string(), Value::Str("rasm".into()));
        let n = node("img", vec![Value::Map(props)]);
        let html = node_to_html(&n);
        // img — void teg
        assert!(html.starts_with("<img"), "html: {}", html);
        assert!(html.contains("src=\"/rasm.png\""), "html: {}", html);
        assert!(html.ends_with("/>"), "html: {}", html);
    }

    #[test]
    fn nil_bosh_string() {
        assert_eq!(node_to_html(&Value::Nil), "");
    }

    #[test]
    fn badge_base_class() {
        let n = node("badge", vec![Value::Str("Yangi".into())]);
        let html = node_to_html(&n);
        // badge -> span.flux-badge
        assert_eq!(html, "<span class=\"flux-badge\">Yangi</span>");
    }

    #[test]
    fn fragment_yopuvchi_tegsiz() {
        let a = node("h1", vec![Value::Str("A".into())]);
        let b = node("p", vec![Value::Str("B".into())]);
        let frag = fragment(vec![a, b]);
        // fragment teg chiqarmaydi — faqat bolalar.
        assert_eq!(node_to_html(&frag), "<h1>A</h1><p>B</p>");
    }

    #[test]
    fn theme_css_custom_properties() {
        let mut theme = BTreeMap::new();
        theme.insert("primary".to_string(), Value::Str("#e84d8a".into()));
        theme.insert("radius".to_string(), Value::Sym("lg".into()));
        let css = theme_to_css(&theme);
        // sym `:` prefiksisiz (to_text), str o'z holicha.
        assert!(css.contains("--primary:#e84d8a;"), "css: {}", css);
        assert!(css.contains("--radius:lg;"), "css: {}", css);
        assert!(css.contains(".flux-primary{"), "base css yo'q: {}", css);
    }

    // --- PR-4b: island markerlash ---

    // on: bo'lgan element -> island ildizi, marker.
    fn props_node(tag: &str, props: Vec<(&str, Value)>, text: Option<&str>) -> Value {
        let mut p = BTreeMap::new();
        for (k, v) in props {
            p.insert(k.to_string(), v);
        }
        let mut args = vec![];
        if let Some(t) = text {
            args.push(Value::Str(t.into()));
        }
        args.push(Value::Map(p));
        node(tag, args)
    }

    #[test]
    fn statik_element_island_emas() {
        let mut n = node("h1", vec![Value::Str("Salom".into())]);
        let mut id = 1;
        let cnt = mark_islands(&mut n, &mut id, &BTreeMap::new());
        assert_eq!(cnt, 0, "statik element island bermasligi kerak");
        assert!(!node_to_html(&n).contains("data-fx"));
    }

    #[test]
    fn on_element_island_boladi() {
        let mut n = props_node("btn", vec![("on", Value::Str("add".into()))], Some("Qo'sh"));
        let mut id = 1;
        let cnt = mark_islands(&mut n, &mut id, &BTreeMap::new());
        assert_eq!(cnt, 1, "on: bo'lgan element island ildizi");
        let html = node_to_html(&n);
        assert!(html.contains("data-fx-island=\"1\""), "html: {}", html);
        assert!(html.contains("data-fx-on=\"click:add\""), "html: {}", html);
    }

    #[test]
    fn eng_kichik_orovchi_island() {
        // Tashqi statik div ichida interaktiv btn -> island ildizi DIV (eng kichik
        // o'rovchi interaktiv), ichidagi btn YANGI island OLMAYDI (bitta island).
        let btn = props_node("btn", vec![("on", Value::Str("go".into()))], Some("Bos"));
        let div = node("div", vec![Value::List(vec![btn])]);
        let mut n = div;
        let mut id = 1;
        let cnt = mark_islands(&mut n, &mut id, &BTreeMap::new());
        assert_eq!(cnt, 1, "faqat bitta island (div), btn alohida emas");
        let html = node_to_html(&n);
        // div island, btn faqat data-fx-on (island emas).
        assert!(html.contains("<div data-fx-island=\"1\""), "html: {}", html);
        let island_count = html.matches("data-fx-island").count();
        assert_eq!(island_count, 1, "bitta island bo'lishi kerak: {}", html);
    }

    #[test]
    fn bind_marker() {
        let mut n = props_node("input", vec![("bind", Value::Str("q".into()))], None);
        let mut id = 1;
        mark_islands(&mut n, &mut id, &BTreeMap::new());
        let html = node_to_html(&n);
        assert!(html.contains("data-fx-bind=\"q\""), "html: {}", html);
    }

    #[test]
    fn fragment_island_olmaydi() {
        // Fragment (ko'rinmas o'rov) island ildizi bo'la olmaydi; bolasi (btn) bo'ladi.
        let btn = props_node("btn", vec![("on", Value::Str("x".into()))], Some("B"));
        let mut frag = fragment(vec![node("h1", vec![Value::Str("S".into())]), btn]);
        let mut id = 1;
        let cnt = mark_islands(&mut frag, &mut id, &BTreeMap::new());
        assert_eq!(cnt, 1, "fragment emas, btn island bo'ladi");
    }

    #[test]
    fn find_island_topadi() {
        // div(island 1) ichida btn — find_island(1) div'ni qaytaradi.
        let btn = props_node("btn", vec![("on", Value::Str("go".into()))], Some("B"));
        let mut div = node("div", vec![Value::List(vec![btn])]);
        let mut id = 1;
        mark_islands(&mut div, &mut id, &BTreeMap::new());
        let found = find_island(&div, 1).expect("island 1 topilishi kerak");
        let html = node_to_html(found);
        assert!(html.contains("data-fx-island=\"1\""), "html: {}", html);
        assert!(find_island(&div, 99).is_none(), "yo'q island None");
    }

    #[test]
    fn bootstrap_script_island_bilan() {
        assert_eq!(
            fx_bootstrap_script(0, "/", &[]),
            "",
            "0 island + 0 live -> script yo'q"
        );
        let s = fx_bootstrap_script(2, "/shop", &[]);
        assert!(s.contains("window.__fx"), "s: {}", s);
        assert!(s.contains("\"page\":\"/shop\""), "page yo'q: {}", s);
        assert!(s.contains("\"1\":{\"mode\":\"server\"}"), "s: {}", s);
        assert!(s.contains("\"2\":{\"mode\":\"server\"}"), "s: {}", s);
        assert!(s.contains("/_fx/client.js"), "client.js yo'q: {}", s);
    }

    // PR-7b: live source (island bo'lmasa ham) -> script + window.__fx.live.
    #[test]
    fn bootstrap_script_live_bilan() {
        // 0 island lekin live tag bor -> script CHIQADI (WS uchun kerak).
        let s = fx_bootstrap_script(0, "/", &["orders".to_string()]);
        assert!(
            s.contains("window.__fx"),
            "live'da script bo'lishi kerak: {}",
            s
        );
        assert!(s.contains("\"live\":[\"orders\"]"), "live tag yo'q: {}", s);
        assert!(s.contains("/_fx/client.js"), "client.js yo'q: {}", s);
        // 0 island + 0 live -> script YO'Q (sof statik invariant).
        assert_eq!(fx_bootstrap_script(0, "/", &[]), "");
    }

    // --- PR-6: handler-effekt (data-fx-state + react_bind_names) ---

    #[test]
    fn data_fx_state_island_ildizida() {
        // mark_islands react_state bilan -> island ildiziga data-fx-state (JSON,
        // escape qilingan). on: marker bo'lgan element ildiz bo'ladi.
        let mut n = props_node("btn", vec![("on", Value::Str("#0".into()))], Some("+1"));
        let mut id = 1;
        let mut state = BTreeMap::new();
        state.insert("count".to_string(), Value::Int(3));
        let cnt = mark_islands(&mut n, &mut id, &state);
        assert_eq!(cnt, 1);
        let html = node_to_html(&n);
        // JSON `{count:3}` -> atribut escape (qo'shtirnoq -> &quot;).
        assert!(
            html.contains("data-fx-state=\"{&quot;count&quot;:3}\""),
            "html: {}",
            html
        );
        // handler marker `#0` -> data-fx-on="click:#0".
        assert!(html.contains("data-fx-on=\"click:#0\""), "html: {}", html);
    }

    #[test]
    fn bosh_react_state_data_fx_state_yozmaydi() {
        // GET render (bo'sh react_state) -> data-fx-state YO'Q (initial seed'dan).
        let mut n = props_node("btn", vec![("on", Value::Str("#0".into()))], Some("+1"));
        let mut id = 1;
        mark_islands(&mut n, &mut id, &BTreeMap::new());
        let html = node_to_html(&n);
        assert!(
            !html.contains("data-fx-state"),
            "bo'sh state yozmasligi: {}",
            html
        );
    }

    #[test]
    fn react_bind_names_yigadi() {
        // page handler (view) body'dagi `<-` nomlari = React state.
        use crate::lexer::lex;
        use crate::parser::parse;
        let src = "view c\n  count <- 0\n  flag <- false\n  btn \"x\" {on:add}\n";
        let prog = parse(lex(src).unwrap()).unwrap();
        // ViewDecl -> Value::Fn yasab react_bind_names'ni sinash.
        let crate::ast::Stmt::ViewDecl { params, body, name } = &prog[0] else {
            panic!("view kutilgan");
        };
        let f = Value::Fn(std::sync::Arc::new(crate::value::FnValue {
            params: params.clone(),
            body: body.clone(),
            parent: crate::interp::Parent::None,
            name: name.clone(),
            is_view: true,
        }));
        let mut names = react_bind_names(&f);
        names.sort();
        assert_eq!(names, vec!["count".to_string(), "flag".to_string()]);
    }
}
