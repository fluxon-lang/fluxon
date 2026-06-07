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
// island raqami (har sahifada 1, 2, ...). Qaytaradi: topilgan island soni.
pub fn mark_islands(node: &mut Value, next_id: &mut u32) -> u32 {
    mark_walk(node, next_id, false)
}

// Rekursiv walk. `inside_island` — biz allaqachon island ichidamizmi (shunda
// ichki elementga YANGI island bermaymiz — bitta island, ko'p emas).
fn mark_walk(node: &mut Value, next_id: &mut u32, inside_island: bool) -> u32 {
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
            count += mark_walk(c, next_id, now_inside);
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
                mark_islands(&mut node, &mut id);
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
fn full_document(css: &str, body_html: &str, island_count: u32) -> String {
    let script = fx_bootstrap_script(island_count);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<style>{}</style></head><body>{}{}</body></html>",
        css, body_html, script
    )
}

// PR-4b minimal bootstrap: island ro'yxati + mode (HAL QILINGAN QARORLAR:
// hammasi server-driven). 0 island = script YO'Q (sof statik, 0 JS).
fn fx_bootstrap_script(island_count: u32) -> String {
    if island_count == 0 {
        return String::new();
    }
    let mut islands = String::new();
    for i in 1..=island_count {
        if i > 1 {
            islands.push(',');
        }
        islands.push_str(&format!("\"{}\":{{\"mode\":\"server\"}}", i));
    }
    format!(
        "<script>window.__fx={{\"islands\":{{{}}}}}</script>",
        islands
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
            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
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
    req: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method().as_str().to_lowercase();
    let uri = req.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or("").to_string();

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
        let v = interp2.apply(handler, args)?;
        // page bo'lsa shu thread'da HTML render qilamiz (theme o'qish ham bu yerda),
        // REST bo'lsa xom Value qaytaramiz (tashqarida value_to_response).
        if is_page {
            Ok(PageOrRest::Page(interp2.render_page(&v)))
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
    // (theme CSS + body). ui.page bilan bir xil, lekin serve_loop ichidan.
    pub fn render_page(&self, node: &Value) -> String {
        // Island markerlar (PR-4b) — node clone'iga qo'shamiz (kiruvchi o'zgarmaydi).
        let mut node = node.clone();
        let mut next_id = 1u32;
        let island_count = mark_islands(&mut node, &mut next_id);
        let css = {
            let theme = self.theme.read();
            theme_to_css(&theme)
        };
        full_document(&css, &node_to_html(&node), island_count)
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
        let cnt = mark_islands(&mut n, &mut id);
        assert_eq!(cnt, 0, "statik element island bermasligi kerak");
        assert!(!node_to_html(&n).contains("data-fx"));
    }

    #[test]
    fn on_element_island_boladi() {
        let mut n = props_node("btn", vec![("on", Value::Str("add".into()))], Some("Qo'sh"));
        let mut id = 1;
        let cnt = mark_islands(&mut n, &mut id);
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
        let cnt = mark_islands(&mut n, &mut id);
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
        mark_islands(&mut n, &mut id);
        let html = node_to_html(&n);
        assert!(html.contains("data-fx-bind=\"q\""), "html: {}", html);
    }

    #[test]
    fn fragment_island_olmaydi() {
        // Fragment (ko'rinmas o'rov) island ildizi bo'la olmaydi; bolasi (btn) bo'ladi.
        let btn = props_node("btn", vec![("on", Value::Str("x".into()))], Some("B"));
        let mut frag = fragment(vec![node("h1", vec![Value::Str("S".into())]), btn]);
        let mut id = 1;
        let cnt = mark_islands(&mut frag, &mut id);
        assert_eq!(cnt, 1, "fragment emas, btn island bo'ladi");
    }

    #[test]
    fn bootstrap_script_island_bilan() {
        assert_eq!(fx_bootstrap_script(0), "", "0 island -> script yo'q");
        let s = fx_bootstrap_script(2);
        assert!(s.contains("window.__fx"), "s: {}", s);
        assert!(s.contains("\"1\":{\"mode\":\"server\"}"), "s: {}", s);
        assert!(s.contains("\"2\":{\"mode\":\"server\"}"), "s: {}", s);
    }
}
