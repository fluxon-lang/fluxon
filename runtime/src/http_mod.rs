// Fluxon HTTP battery — server (http.on/http.serve/rep) and client (http.get/post).
//
// The server is built on tokio + hyper. Because Fluxon handlers are synchronous
// tree-walking, each request runs inside `spawn_blocking` — this makes the CPU
// work TRULY PARALLEL without blocking tokio workers (Value: Send+Sync, the
// thread-safety refactor guarantees this).
//
// `rep status body` -> {__resp:true status body} map (builtins.rs::install).
// `fail status "msg"` -> Flow::Fail -> JSON error response.

use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use tokio::net::TcpListener;

use crate::builtins::{json_decode, json_encode};
use crate::interp::{Flow, Interp};
use crate::value::Value;

// --- route structure ---

// Path segment: literal (`notes`) or parameter (`:id`).
#[derive(Clone)]
pub enum Seg {
    Lit(String),
    Param(String),
}

#[derive(Clone)]
pub struct Route {
    pub method: String, // lowercase: "get", "post", ...
    pub pattern: Vec<Seg>,
    pub handler: Value, // Value::Fn (closure)
}

// Rate-limit state: key -> (window_id, count). Fixed-window — `window_id =
// now_sec / window_sec`. Arc<Mutex> so the limiter is created once at
// REGISTRATION time, and every request shares this SINGLE state (cloning a
// Middleware copies the Arc — same pointer), which is why parallel requests count
// atomically (issue #79: thread-safe). State is in-memory — for a single instance (docs).
//
// Memory bound: if the key function is based on a user-controlled value
// (`req.headers.x_api_key`), every new value lands in the HashMap. On a public
// endpoint a client can grow the state without bound by sending a new key on
// every request. To prevent this, `LimitBucket` sweeps OLD-WINDOW keys once every
// `SWEEP_EVERY` operations (amortized O(1): the cleanup loop runs rarely). An
// old-window key would restart from count=0 on the next request anyway — so
// removing it is safe.
//
// pub: `pub enum MwKind` (via Middleware) exposes the LimitState type.
pub struct LimitBucket {
    counts: HashMap<String, (u64, u32)>,
    // Number of operations since the last cleanup (amortizes the sweep).
    ops: u32,
}

impl LimitBucket {
    fn new() -> Self {
        LimitBucket {
            counts: HashMap::new(),
            ops: 0,
        }
    }
}

// How often (in operations) we sweep old-window keys.
const SWEEP_EVERY: u32 = 1024;

// Default request body size limit for the HTTP server (issue #91). An unbounded
// `collect()` gathers the entire body into memory — a client can fill server
// memory by sending a huge body (DoS). Default 10 MiB; configured via `http.serve
// PORT {max_body: N}`. `max_body: 0` disables the limit (unbounded — use only
// behind a trusted, internal network).
const DEFAULT_MAX_BODY: usize = 10 * 1024 * 1024;

// Default timeout for the HTTP client (http.get/post/...) (issue #92). Without a
// timeout, a stuck upstream blocks the whole script FOREVER (or, if called inside
// a handler, that request thread). Default 30s; configured via `http.get url
// {timeout: N}` (seconds); `timeout: 0` — no timeout (only for trusted upstreams).
// The timeout covers the whole request: connect + send + response (including
// redirects) — even if it hangs at some stage, an error is returned once time runs out.
const DEFAULT_CLIENT_TIMEOUT_SECS: u64 = 30;

// HTTP server header-read timeout (issue #92). A slowloris-style connection may
// send headers very slowly (or not at all), holding the socket/task indefinitely.
// If hyper does not receive the headers fully within this period it closes the
// connection. (header_read_timeout only takes effect when Builder::timer is set.)
const DEFAULT_HEADER_READ_TIMEOUT_SECS: u64 = 30;

pub type LimitState = Arc<Mutex<LimitBucket>>;

// Middleware kind: a plain fn (use/before) or a rate-limiter (http.limit). Limit
// is added to THIS SAME list (not separately) — so it runs in DECLARATION ORDER
// relative to other middleware: if an auth declared before it writes tenant_id
// into `req.ctx`, the key function `\req -> req.ctx.tenant_id` sees it (#79).
#[derive(Clone)]
pub enum MwKind {
    // http.use / http.before — calls the handler; `fail`/`rep` stops the chain.
    Fn,
    // http.limit — the handler is the KEY function (req -> key). On exceeding the limit, 429.
    Limit {
        limit: u32,
        window_secs: u64,
        state: LimitState,
    },
}

// Middleware (issue #67). `scope` = None — global (`http.use`, applies to every
// path); Some(pattern) — by prefix (`http.before "/api/*"`). Stored in the list
// in declaration order (the order is well-defined even when use/before/limit are mixed).
#[derive(Clone)]
pub struct Middleware {
    pub scope: Option<String>,
    pub handler: Value,
    pub kind: MwKind,
}

// CORS sozlamasi (issue #135). `http.cors` to'ldiradi; yoqilgan bo'lsa OPTIONS
// preflight avtomatik javob oladi va har javobga `Access-Control-Allow-*`
// header'lar qo'shiladi.
//
//   http.cors "*"                                   # hammaga ochiq (dev)
//   http.cors ["https://app.example.com"]           # ruxsat etilgan origin'lar
//   http.cors ["https://app.example.com"] {creds: true}   # cookie/Authorization
//
// `origins`: None — har qanday origin ("*"). Some(set) — faqat ro'yxatdagilar.
// Wildcard "*" va `creds: true` birga ishlatib bo'lmaydi (brauzer rad etadi),
// shuning uchun creds yoqilsa javob so'rovning aniq Origin'ini aks ettiradi.
#[derive(Clone)]
pub struct CorsConfig {
    // Ruxsat etilgan origin'lar. None — "*" (har qanday). Some — aniq ro'yxat.
    origins: Option<Vec<String>>,
    // Ruxsat etilgan metodlar (Access-Control-Allow-Methods). Default keng to'plam.
    methods: String,
    // Ruxsat etilgan so'rov header'lari (Access-Control-Allow-Headers).
    headers: String,
    // Cookie/Authorization (credentials) ulashishga ruxsat (Allow-Credentials).
    creds: bool,
    // Preflight javobini brauzer necha soniya kesh qiladi (Max-Age).
    max_age: u64,
}

// Origin so'rovga ruxsat berilganmi? Ruxsat berilsa, javobning
// `Access-Control-Allow-Origin` qiymati qaytadi (aniq origin yoki "*").
impl CorsConfig {
    // So'rovning Origin header'iga qarab Allow-Origin qiymatini hisoblaydi.
    // None qaytsa — bu origin ruxsat etilmagan (CORS header qo'shilmaydi).
    fn allow_origin_for(&self, req_origin: Option<&str>) -> Option<String> {
        match &self.origins {
            // Har qanday origin ruxsat. creds=true bo'lsa "*" ishlatib bo'lmaydi —
            // so'rov origin'ini aks ettiramiz (bo'lmasa "*").
            None => {
                if self.creds {
                    req_origin.map(|o| o.to_string())
                } else {
                    Some("*".to_string())
                }
            }
            // Aniq ro'yxat — so'rov origin'i ichida bo'lsa o'shani aks ettiramiz.
            Some(list) => match req_origin {
                Some(o) if list.iter().any(|a| a == o) => Some(o.to_string()),
                _ => None,
            },
        }
    }

    // Javobning HeaderMap'iga CORS header'larini qo'shadi (preflight'siz oddiy
    // javoblar uchun ham). Origin ruxsat etilmagan bo'lsa hech narsa qo'shilmaydi.
    fn apply_to(&self, hmap: &mut hyper::HeaderMap, req_origin: Option<&str>) {
        let Some(allow) = self.allow_origin_for(req_origin) else {
            return;
        };
        set_header(hmap, "access-control-allow-origin", &allow);
        // Allow-Origin so'rov origin'iga qarab o'zgaradi — kesh to'g'ri bo'lishi
        // uchun Vary'ga Origin qo'shamiz (aks holda proksi bir origin javobini
        // boshqasiga beradi). insert EMAS — handler `rep ... {vary:"Accept-Encoding"}`
        // bilan qo'ygan Vary'ni saqlab, Origin'ni BIRLASHTIRAMIZ (codex P2: insert
        // mavjud kesh kalitini buzardi). "*" — javob origin'ga bog'liq emas, Vary shart emas.
        if allow != "*" {
            add_vary_origin(hmap);
        }
        if self.creds {
            set_header(hmap, "access-control-allow-credentials", "true");
        }
    }
}

// Javobning `Vary` header'iga `Origin` qo'shadi, mavjud qiymatlarni saqlab.
// `Vary` allaqachon `Origin` (yoki `*`) ni o'z ichiga olsa — o'zgartirmaydi
// (takror oldini olish). Aks holda vergul bilan birlashtiradi.
fn add_vary_origin(hmap: &mut hyper::HeaderMap) {
    use hyper::header::{HeaderValue, VARY};
    // Mavjud Vary qiymatlarini o'qiymiz (bir nechta Vary qatori bo'lishi mumkin).
    let existing: Vec<String> = hmap
        .get_all(VARY)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .collect();
    // Allaqachon Origin yoki * bor bo'lsa — qo'shilgan, qaytamiz.
    let already = existing.iter().any(|line| {
        line.split(',').any(|tok| {
            let t = tok.trim();
            t.eq_ignore_ascii_case("origin") || t == "*"
        })
    });
    if already {
        return;
    }
    if existing.is_empty() {
        hmap.insert(VARY, HeaderValue::from_static("Origin"));
    } else {
        // Mavjud qiymat(lar)ni bitta qatorga birlashtirib Origin qo'shamiz.
        let merged = format!("{}, Origin", existing.join(", "));
        if let Ok(hv) = HeaderValue::from_str(&merged) {
            hmap.insert(VARY, hv);
        }
    }
}

// Javobga CORS header'larini qo'shadi (yoqilgan bo'lsa) va javobni qaytaradi.
// Body-read xato javoblari (400/413), 404 va handler javobi — hammasi shu
// orqali yakunlanadi, shunda CORS yoqilganda HAR javob `Access-Control-Allow-*`
// oladi (codex P2: oldin erta-return xatolari header'siz qaytardi).
fn cors_finalize(
    mut resp: Response<Full<Bytes>>,
    cors: &Option<CorsConfig>,
    req_origin: Option<&str>,
) -> Response<Full<Bytes>> {
    if let Some(cfg) = cors {
        cfg.apply_to(resp.headers_mut(), req_origin);
    }
    resp
}

// HeaderMap'ga bitta header'ni insert qiladi (eskisini bosadi). Buzuq nom/qiymat
// jim o'tkazib yuboriladi. CORS header'lari uchun yordamchi (closure borrow
// muammosini chetlab o'tadi).
fn set_header(hmap: &mut hyper::HeaderMap, name: &str, val: &str) {
    use hyper::header::{HeaderName, HeaderValue};
    if let (Ok(n), Ok(v)) = (
        HeaderName::from_bytes(name.as_bytes()),
        HeaderValue::from_str(val),
    ) {
        hmap.insert(n, v);
    }
}

// "/notes/:id" -> [Lit("notes"), Param("id")]. Bo'sh segmentlar tashlanadi.
fn parse_pattern(path: &str) -> Vec<Seg> {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(|s| {
            if let Some(name) = s.strip_prefix(':') {
                Seg::Param(name.to_string())
            } else {
                Seg::Lit(s.to_string())
            }
        })
        .collect()
}

// So'rov yo'lini segmentlarga bo'ladi (query'siz).
fn path_segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

// method+path bo'yicha birinchi mos marshrutni topadi; topilsa params map qaytadi.
fn match_route(
    routes: &[Route],
    method: &str,
    path: &str,
) -> Option<(Route, BTreeMap<String, Value>)> {
    let segs = path_segments(path);
    for r in routes {
        if r.method != method {
            continue;
        }
        if r.pattern.len() != segs.len() {
            continue;
        }
        let mut params = BTreeMap::new();
        let mut ok = true;
        for (pat, seg) in r.pattern.iter().zip(&segs) {
            match pat {
                Seg::Lit(lit) => {
                    if lit != seg {
                        ok = false;
                        break;
                    }
                }
                Seg::Param(name) => {
                    // Path segmentlarida ham non-ASCII percent-encode qilinadi
                    // (masalan `/users/:name` -> `%D0%9A...`) — dekod qilamiz
                    // (issue #100). Path'da `+` literal, shuning uchun bo'shliqqa
                    // almashtirilmaydi (faqat query'da form-encoding qoidasi).
                    // `keep_path_seps=true`: `%2F`/`%5C` xom qoladi (segment
                    // invarianti — qiymatga `/` kirmasin, codex revyu).
                    params.insert(name.clone(), Value::Str(percent_decode(seg, true)));
                }
            }
        }
        if ok {
            return Some((r.clone(), params));
        }
    }
    None
}

// http.before shabloni so'rov yo'liga mosmi? (issue #67)
// "/api/*" -> "/api" yoki "/api/..." bilan boshlanuvchi yo'llar (segment chegarasi).
// "*"siz shablon -> aniq mos. "/apix" "/api/*" ga MOS EMAS (segment ajratiladi).
fn prefix_matches(pat: &str, path: &str) -> bool {
    if let Some(prefix) = pat.strip_suffix("/*") {
        // "/api/*" → "/api" ning o'zi yoki "/api/" bilan boshlanuvchilar.
        path == prefix || path.starts_with(&format!("{}/", prefix))
    } else {
        // Shablonsiz — aniq yo'l mosligi.
        pat == path
    }
}

// --- static fayl mount (issue #134) ---

// `http.static prefiks katalog` mount'i. Prefiks segmentlarga bo'lib saqlanadi
// ("/assets" -> ["assets"], "/" -> []) — moslik segment chegarasida tekshiriladi
// ("/assetsx" "/assets" mount'iga tushmasin). `dir` registratsiya paytida
// canonicalize qilingan mutlaq yo'l (skript katalogiga nisbatan hal qilinadi).
#[derive(Clone)]
pub struct StaticMount {
    pub prefix: Vec<String>,
    pub dir: PathBuf,
    // SPA fallback: prefiks ostidagi yo'l faylga mos kelmasa `dir/index.html`
    // qaytadi (frontend router o'zi hal qiladi).
    pub spa: bool,
}

// "/assets/img" -> ["assets", "img"]; "/" -> []. Bo'sh segmentlar tashlanadi.
fn parse_static_prefix(prefix: &str) -> Vec<String> {
    prefix
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

// Mount prefiksi so'rov segmentlarining boshiga mosmi? Mos bo'lsa prefiksdan
// keyingi qism (fayl yo'li) qaytadi.
fn strip_mount_prefix<'a>(prefix: &[String], segs: &'a [String]) -> Option<&'a [String]> {
    if segs.len() < prefix.len() {
        return None;
    }
    if prefix.iter().zip(segs).all(|(a, b)| a == b) {
        Some(&segs[prefix.len()..])
    } else {
        None
    }
}

// Segmentlarni mount katalogiga MAJBURIY traversal himoyasi bilan ulaydi.
// Har segment percent-dekoddan KEYIN tekshiriladi (shu sabab `%2e%2e` ham
// ushlanadi): faqat oddiy nom (Component::Normal) bo'lishi shart — `..`, `.`,
// bo'sh, mutlaq yoki Windows prefiks (`C:`) segmentlari rad etiladi. Qo'shimcha
// `\`/NUL tekshiruvi: bunday nom fayl tizimida baribir kutilmagan, jim 404.
fn safe_join(dir: &Path, rest: &[String]) -> Option<PathBuf> {
    let mut p = dir.to_path_buf();
    for seg in rest {
        if seg.contains('\\') || seg.contains('\0') {
            return None;
        }
        let mut comps = Path::new(seg).components();
        match (comps.next(), comps.next()) {
            (Some(Component::Normal(_)), None) => {}
            _ => return None,
        }
        p.push(seg);
    }
    Some(p)
}

// Kengaytmadan Content-Type (issue talabi: avtomatik). Ro'yxatda yo'q kengaytma
// -> octet-stream (brauzer yuklab oladi, lekin mazmun buzilmaydi).
fn mime_for(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("json") | Some("map") => "application/json",
        Some("txt") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml",
        Some("csv") => "text/csv; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("wasm") => "application/wasm",
        Some("pdf") => "application/pdf",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mp3") => "audio/mpeg",
        Some("gz") => "application/gzip",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

// Kandidatni canonicalize qilib mount ildizi OSTIDA oddiy fayl ekanini
// tasdiqlaydi (codex P2): safe_join faqat leksik segmentlarni tekshiradi,
// metadata esa symlink'ni kuzatadi — papka ichidagi symlink ildizdan tashqari
// faylga (masalan /etc/passwd) ishora qilsa, leksik himoya chetlab o'tilardi.
// `root` registratsiyada canonicalize qilingan, shuning uchun prefiks
// taqqoslash to'g'ri ishlaydi. Ildiz ichidagi symlink (canonical manzili ham
// ildiz ostida) avvalgidek xizmat qilinadi. Qaytgan yo'l canonical — keyingi
// o'qish ham aynan tekshirilgan faylni oladi.
async fn confined_file(p: &Path, root: &Path) -> Option<(PathBuf, u64)> {
    let canon = tokio::fs::canonicalize(p).await.ok()?;
    if !canon.starts_with(root) {
        return None;
    }
    let md = tokio::fs::metadata(&canon).await.ok()?;
    if md.is_file() {
        let len = md.len();
        Some((canon, len))
    } else {
        None
    }
}

// So'rov segmentlarini (percent-dekod qilingan — chaqiruvchi tayyorlaydi)
// mount'lar bo'yicha faylga aylantiradi. Uzun prefiks ustun ("/assets" mount'i
// "/" mount'idan oldin tekshiriladi) — eng aniq mount yutadi. Ikki bosqich:
// (1) aniq fayl (katalog so'ralsa ichidagi index.html); (2) topilmasa —
// prefiksi mos SPA mount'larning `index.html` fallback'i. Hajm (bayt)
// metadata'dan birga qaytadi — HEAD javobi faylni o'qimasdan Content-Length
// bera olsin (codex P2). Har kandidat confined_file orqali ildizga qamaladi.
async fn resolve_static(
    mounts: &[StaticMount],
    segs: &[String],
) -> Option<(PathBuf, &'static str, u64)> {
    let mut order: Vec<&StaticMount> = mounts.iter().collect();
    order.sort_by_key(|m| std::cmp::Reverse(m.prefix.len()));

    for m in &order {
        let Some(rest) = strip_mount_prefix(&m.prefix, segs) else {
            continue;
        };
        let Some(p) = safe_join(&m.dir, rest) else {
            continue;
        };
        // Aniq fayl. Mime canonical yo'ldan — symlink nomi emas, haqiqiy fayl
        // kengaytmasi javob turini belgilaydi.
        if let Some((canon, len)) = confined_file(&p, &m.dir).await {
            let mime = mime_for(&canon);
            return Some((canon, mime, len));
        }
        // Katalog so'ralgan bo'lishi mumkin (yoki prefiksning o'zi) —
        // ichidagi index.html'ga urinish. p fayl bo'lsa bu jim muvaffaqiyatsiz.
        if let Some((canon, len)) = confined_file(&p.join("index.html"), &m.dir).await {
            let mime = mime_for(&canon);
            return Some((canon, mime, len));
        }
    }

    for m in &order {
        if !m.spa || strip_mount_prefix(&m.prefix, segs).is_none() {
            continue;
        }
        if let Some((canon, len)) = confined_file(&m.dir.join("index.html"), &m.dir).await {
            return Some((canon, "text/html; charset=utf-8", len));
        }
    }
    None
}

// Static fayl javobi: 200 + kengaytmadan aniqlangan Content-Type.
fn static_response(data: Vec<u8>, mime: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", mime)
        .body(Full::new(Bytes::from(data)))
        .unwrap()
}

// HEAD uchun static javob: fayl O'QILMAYDI (katta asset'da behuda disk I/O va
// xotira — codex P2), faqat metadata'dagi hajm Content-Length sifatida qo'lda
// qo'yiladi (bo'sh body avtomatik 0 berardi). hyper HEAD'ga tana yozmaydi.
fn static_head_response(len: u64, mime: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", mime)
        .header("content-length", len.to_string())
        .body(Full::new(Bytes::new()))
        .unwrap()
}

// --- rate-limit (issue #79) ---

// Oyna birligi symbol'ini soniyaga aylantiradi. Faqat :sec/:min/:hr — kam token,
// AI eslab qoladigan canonical to'plam (yangi birlik kerak bo'lsa shu yerga).
fn window_to_secs(unit: &str) -> Option<u64> {
    match unit {
        "sec" => Some(1),
        "min" => Some(60),
        "hr" => Some(3600),
        _ => None,
    }
}

// Fixed-window hisobgich: kalit uchun joriy oynadagi so'rovni sanaydi va oshirib
// bo'lgach tekshiradi. Limit oshsa Some(retry_after_sek) (oyna tugashigacha),
// aks holda None. Mutex bitta lock ostida read-modify-write qiladi — shuning
// uchun parallel request'lar bir kalitni atomik sanaydi (race yo'q).
fn check_and_count(state: &LimitState, key: &str, limit: u32, window_secs: u64) -> Option<u64> {
    // Devor-soat vaqti (Instant emas): oyna chegarasi epoch'ga bog'langan, shunda
    // Retry-After ham (window_id+1)*window_secs - now sifatida aniq chiqadi.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let window_id = now / window_secs;
    let mut bucket = state.lock().unwrap();
    // Davriy tozalash: eski oyna (window_id'i joriyidan kichik) kalitlarini
    // olib tashlaymiz, shunda foydalanuvchi nazoratidagi kalitlar xotirani
    // cheksiz o'stirmaydi. Faqat SWEEP_EVERY operatsiyada bir marta — O(1) amortized.
    bucket.ops = bucket.ops.saturating_add(1);
    if bucket.ops >= SWEEP_EVERY {
        bucket.ops = 0;
        bucket.counts.retain(|_, (wid, _)| *wid >= window_id);
    }
    let entry = bucket
        .counts
        .entry(key.to_string())
        .or_insert((window_id, 0));
    // Yangi oynaga o'tdik — hisobni nolga tushiramiz.
    if entry.0 != window_id {
        *entry = (window_id, 0);
    }
    entry.1 = entry.1.saturating_add(1);
    if entry.1 > limit {
        // Oyna (window_id+1)*window_secs epoch'da yangilanadi; now undan kichik,
        // shuning uchun farq doim >= 1.
        Some((window_id + 1) * window_secs - now)
    } else {
        None
    }
}

// Kalit funksiyasi nil/bo'sh qaytarsa — mijoz IP'siga qaytamiz (kalitsiz so'rovni
// ham cheklash uchun). "ip:" prefiksi tenant_id/api-key qiymati bilan tasodifan
// to'qnashmaslik uchun (bitta limiter holatida ikkalasi bir HashMap'da yashaydi).
fn client_fallback_key(req: &Value) -> String {
    let ip = match req {
        Value::Map(m) => match m.get("ip") {
            Some(Value::Str(s)) if !s.is_empty() => s.clone(),
            _ => "unknown".to_string(),
        },
        _ => "unknown".to_string(),
    };
    format!("ip:{}", ip)
}

// Limit oshganda qaytariladigan javob: `429` + `Retry-After` header (PRD format).
// __resp map sifatida — handle_request uni boshqa rep javoblari kabi yuboradi.
fn rate_limited_response(retry_after: u64) -> Value {
    let mut body = BTreeMap::new();
    body.insert(
        "error".to_string(),
        Value::Str("rate limit exceeded".to_string()),
    );
    let mut headers = BTreeMap::new();
    headers.insert(
        "retry-after".to_string(),
        Value::Str(retry_after.to_string()),
    );
    let mut m = BTreeMap::new();
    m.insert("__resp".to_string(), Value::Bool(true));
    m.insert("status".to_string(), Value::Int(429));
    m.insert("body".to_string(), Value::Map(body));
    m.insert("headers".to_string(), Value::Map(headers));
    Value::Map(m)
}

// Percent-encoded UTF-8 baytlarni (`%D0%9A`) dekod qiladi: `%XX` juftliklarni
// baytga aylantirib yig'adi, qolgan baytlarni o'zgarmas qoldiradi. Yig'ilgan
// baytlar UTF-8 deb talqin qilinadi — `from_utf8_lossy` yaroqsiz ketma-ketlikni
// U+FFFD bilan almashtiradi (panic yo'q). Yaroqsiz `%` (masalan, `%zz` yoki
// satr oxiridagi `%`) literal `%` sifatida qoladi. Brauzer query va path'dagi
// non-ASCII (kirill/o'zbekcha) qiymatlarni doim percent-encode qiladi — bu
// funksiyasiz `req.query.q` xom `%D1%81...` holicha qolardi (issue #100).
//
// `keep_path_seps` — `true` bo'lsa `%2F` (`/`) va `%5C` (`\`) DEKOD QILINMAYDI,
// xom `%2F`/`%5C` holicha qoladi (path param uchun). Sabab: `:param` qiymati
// bitta segmentdan keladi degan invariant — encoded slash dekod qilinsa qiymatga
// `/` kirib, uni haqiqiy yo'l ajratuvchisidan farqlab bo'lmaydi, va param'ni ID
// yoki xavfsiz yo'l komponenti deb ishlatadigan handler kutilmaganda ichki slash
// oladi (codex revyu). Query qiymatlarida bu xavf yo'q — u yerda `false`.
fn percent_decode(s: &str, keep_path_seps: bool) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                let byte = (hi * 16 + lo) as u8;
                // Path param'da slash/backslash'ni xom qoldiramiz (segment
                // invariantini buzmaslik uchun) — uch baytni o'zgarmas o'tkazamiz.
                if keep_path_seps && (byte == b'/' || byte == b'\\') {
                    out.extend_from_slice(&bytes[i..i + 3]);
                    i += 3;
                    continue;
                }
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// "a=1&b=2" -> {a:"1" b:"2"}. Kalit va qiymatga `+` -> bo'shliq (form-encoding)
// va percent-dekod qo'llanadi (issue #100) — kalitlarda ham non-ASCII bo'lishi
// mumkin, shuning uchun ikkalasi ham dekod qilinadi.
fn parse_query(q: &str) -> Value {
    let mut m = BTreeMap::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        let key = percent_decode(&k.replace('+', " "), false);
        let val = percent_decode(&v.replace('+', " "), false);
        m.insert(key, Value::Str(val));
    }
    Value::Map(m)
}

// --- multipart/form-data (issue #133) ---

// Content-Type'dan multipart boundary'ni ajratadi. multipart/form-data
// bo'lmasa yoki boundary topilmasa None — chaqiruvchi oddiy body yo'liga
// qaytadi. Boundary qo'shtirnoqli bo'lishi mumkin (RFC 2046 ruxsat beradi).
fn multipart_boundary(ct: &str) -> Option<String> {
    let lower = ct.to_ascii_lowercase();
    if !lower.contains("multipart/form-data") {
        return None;
    }
    let i = lower.find("boundary=")?;
    let rest = &ct[i + "boundary=".len()..];
    let val = if let Some(r) = rest.strip_prefix('"') {
        r.split('"').next().unwrap_or("")
    } else {
        rest.split(';').next().unwrap_or("").trim()
    };
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

// Baytlar ichidan pastki ketma-ketlikni qidiradi (memmem). Multipart tana
// ikkilik bo'lishi mumkin — str metodlari yaroqsiz, shuning uchun bayt darajasida.
fn find_sub(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < from + needle.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| i + from)
}

// Content-Disposition qatoridan parametr qiymatini oladi (`name="x"`,
// `filename="a.png"`). `name` qidirilganda `filename=` ichidagi "name=" ga
// adashib tushmaslik uchun mos kelgan joydan OLDINGI belgi ajratuvchi
// (`;`/bo'shliq) ekani tekshiriladi. Qiymat qo'shtirnoqsiz ham bo'lishi mumkin
// (eski klientlar) — u holda `;` gacha o'qiladi.
fn cd_param(line: &str, key: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let pat = format!("{}=", key);
    let mut search = 0;
    while let Some(i) = lower[search..].find(&pat).map(|i| i + search) {
        let at_boundary = i == 0 || matches!(lower.as_bytes()[i - 1], b';' | b' ' | b'\t');
        if at_boundary {
            let rest = &line[i + pat.len()..];
            return Some(if let Some(r) = rest.strip_prefix('"') {
                match r.find('"') {
                    Some(e) => r[..e].to_string(),
                    None => r.to_string(), // yopilmagan qo'shtirnoq — oxirigacha
                }
            } else {
                let e = rest.find(';').unwrap_or(rest.len());
                rest[..e].trim().to_string()
            });
        }
        search = i + pat.len();
    }
    None
}

// Boundary qatori shu yerda haqiqatan tugayaptimi? RFC 2046: `--boundary` dan
// keyin yo yopuvchi `--`, yo ixtiyoriy transport padding (bo'shliq/tab) + CRLF
// keladi. Tekshiruvsiz fayl mazmunidagi tasodifiy `\r\n--abcXYZ` (boundary
// `abc` ning prefiksi) chegara deb olinib qism noto'g'ri kesilardi (codex P2
// revyu) — bunday holatda bu valid mazmun, chegara emas.
fn boundary_line_ends(rest: &[u8]) -> bool {
    if rest.starts_with(b"--") {
        return true;
    }
    let mut i = 0;
    while i < rest.len() && (rest[i] == b' ' || rest[i] == b'\t') {
        i += 1;
    }
    rest[i..].starts_with(b"\r\n")
}

// To'liq boundary qatorini qidiradi: `marker` dan keyingi baytlar ham chegara
// qatorini tasdiqlashi shart (boundary_line_ends). Mos kelmagan prefiks
// uchrashlar (fayl mazmunidagi `--boundaryX...`) o'tkazib yuboriladi.
fn find_boundary(body: &[u8], marker: &[u8], from: usize) -> Option<usize> {
    let mut search = from;
    loop {
        let i = find_sub(body, marker, search)?;
        if boundary_line_ends(&body[i + marker.len()..]) {
            return Some(i);
        }
        search = i + 1;
    }
}

// multipart/form-data tanani qismlarga ajratadi: oddiy form maydonlari ->
// fields map (req.body — JSON bilan simmetrik), fayl qismlari (filename bor) ->
// files ro'yxati ({name filename content size}). Tana formatga mos kelmasa
// (boundary topilmadi, buzuq struktura) None — chaqiruvchi xom body'ga qaytadi,
// shunda buzuq so'rov ma'lumotni yo'qotmaydi.
//
// Fayl mazmuni req.body bilan bir xil qoidaga amal qiladi: UTF-8 matn -> str,
// ikkilik -> bytes (issue #132) — AI bitta naqshni o'rganadi. `size` doim BAYT
// soni (str.len belgi sanaydi — fayl o'lchami uchun noto'g'ri bo'lardi).
#[allow(clippy::type_complexity)]
fn parse_multipart(body: &[u8], boundary: &str) -> Option<(BTreeMap<String, Value>, Vec<Value>)> {
    let delim = format!("--{}", boundary).into_bytes();
    // Qism oxiri belgisi: CRLF + boundary (CRLF qism mazmuniga kirmaydi).
    let mut end_marker = b"\r\n".to_vec();
    end_marker.extend_from_slice(&delim);

    let mut fields = BTreeMap::new();
    let mut files = Vec::new();

    // Birinchi boundary (RFC 2046 undan oldin preamble'ga ruxsat beradi).
    // Tana to'g'ridan-to'g'ri `--boundary` bilan boshlansa CRLF prefiksi yo'q —
    // alohida tekshiramiz; aks holda qator boshidagi (CRLF'dan keyingi)
    // to'liq chegarani qidiramiz.
    let mut pos = if body.starts_with(&delim) && boundary_line_ends(&body[delim.len()..]) {
        delim.len()
    } else {
        find_boundary(body, &end_marker, 0)? + end_marker.len()
    };
    loop {
        // Boundary'dan keyin `--` — yakuniy chegara, tugadik.
        if body[pos..].starts_with(b"--") {
            break;
        }
        // Boundary qatori CRLF bilan tugaydi (orada transport padding mumkin).
        let nl = find_sub(body, b"\r\n", pos)?;
        let part_start = nl + 2;
        let part_end = find_boundary(body, &end_marker, part_start)?;
        let part = &body[part_start..part_end];

        // Qism: header'lar + bo'sh qator + mazmun. Header'lar matn (ASCII) —
        // lossy o'qish xavfsiz; mazmun esa xom baytlar bo'lib qoladi.
        if let Some(hdr_end) = find_sub(part, b"\r\n\r\n", 0) {
            let headers_raw = String::from_utf8_lossy(&part[..hdr_end]);
            let content = &part[hdr_end + 4..];
            let cd_line = headers_raw
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("content-disposition:"));
            if let Some(cd) = cd_line
                && let Some(name) = cd_param(cd, "name")
            {
                let content_value = match std::str::from_utf8(content) {
                    Ok(s) => Value::Str(s.to_string()),
                    Err(_) => Value::Bytes(Arc::new(content.to_vec())),
                };
                match cd_param(cd, "filename") {
                    // filename bor — fayl qismi (bo'sh filename ham fayl:
                    // brauzer bo'sh file input'ni shunday yuboradi).
                    Some(filename) => {
                        let mut fm = BTreeMap::new();
                        fm.insert("name".to_string(), Value::Str(name));
                        fm.insert("filename".to_string(), Value::Str(filename));
                        fm.insert("content".to_string(), content_value);
                        fm.insert("size".to_string(), Value::Int(content.len() as i64));
                        files.push(Value::Map(fm));
                    }
                    // Oddiy form maydoni — req.body'ga (matn deb qaraladi).
                    None => {
                        fields.insert(
                            name,
                            Value::Str(String::from_utf8_lossy(content).into_owned()),
                        );
                    }
                }
            }
        }
        pos = part_end + end_marker.len();
    }
    Some((fields, files))
}

// --- request -> Value::Map ---

// req = {method, path, query:{}, headers:{}, params:{}, body:(JSON map/str), files:[], ctx}
// ctx — shared request-scoped store (issue #68): middleware `req.ctx <- {...}`
// yozadi, handler `req.ctx` o'qiydi. ctx'ni caller (`handle_request`) qo'shadi
// (`with_ctx`), chunki u har so'rovga yangi Arc<Mutex> yaratadi — middleware va
// handler bir xil cell'ni ko'rishi uchun.
// req maydonlari ko'p (method/path/query/headers/params/ip/body) — bularni alohida
// struct'ga yig'ish faqat bitta chaqiruv joyi uchun ortiqcha bo'lardi, shuning uchun
// pozitsion argument qoldiramiz (too_many_arguments lintini bu yerda o'chiramiz).
#[allow(clippy::too_many_arguments)]
fn build_req(
    method: String,
    path: String,
    query: String,
    headers: BTreeMap<String, Value>,
    params: BTreeMap<String, Value>,
    ip: String,
    body_bytes: Bytes,
    is_json: bool,
    multipart: Option<String>,
) -> Value {
    // multipart/form-data (issue #133): oddiy maydonlar req.body'ga (JSON bilan
    // simmetrik), fayllar req.files'ga. Parse muvaffaqiyatsiz bo'lsa (buzuq
    // tana) quyidagi oddiy yo'lga tushamiz — xom body yo'qolmaydi.
    let parsed_multipart = multipart
        .as_deref()
        .and_then(|b| parse_multipart(&body_bytes, b));
    let mut files = Vec::new();
    let body = if let Some((fields, fs)) = parsed_multipart {
        files = fs;
        Value::Map(fields)
    } else if body_bytes.is_empty() {
        Value::Nil
    } else {
        match std::str::from_utf8(&body_bytes) {
            // Content-Type JSON bo'lsa, YOKI tana `{`/`[` bilan boshlansa — JSON
            // parse'ga urinamiz. Sabab: `curl -d` standart holda
            // x-www-form-urlencoded yuboradi, lekin tana ko'rinishidan JSON; agar
            // Content-Type'ga qat'i bog'lansak, dasturchi sababsiz string oladi va
            // `body.field` access chalg'ituvchi "str.field metodi" xatosi beradi.
            Ok(s) => {
                let looks_like_json =
                    matches!(s.trim_start().as_bytes().first(), Some(b'{') | Some(b'['));
                if is_json || looks_like_json {
                    // JSON dekod xato bo'lsa — xom matn sifatida qoldiramiz.
                    json_decode(s).unwrap_or_else(|_| Value::Str(s.to_string()))
                } else {
                    Value::Str(s.to_string())
                }
            }
            // UTF-8 bo'lmagan tana — ikkilik yuklama (rasm, gzip): bytes
            // (issue #132). Avval lossy o'qish ma'lumotni jim buzardi.
            Err(_) => Value::Bytes(Arc::new(body_bytes.to_vec())),
        }
    };

    let mut m = BTreeMap::new();
    m.insert("method".to_string(), Value::Str(method));
    m.insert("path".to_string(), Value::Str(path));
    m.insert("query".to_string(), parse_query(&query));
    m.insert("headers".to_string(), Value::Map(headers));
    m.insert("params".to_string(), Value::Map(params));
    // req.ip — mijoz IP (TCP peer). rate-limit kalit funksiyasi nil qaytarsa
    // shunga qaytamiz; foydalanuvchi ham `req.ip` o'qishi mumkin. Proksi orqasida
    // bu proksi IP'si bo'ladi (X-Forwarded-For v1'da boshqarilmaydi — docs).
    m.insert("ip".to_string(), Value::Str(ip));
    m.insert("body".to_string(), body);
    // req.files doim list (multipart bo'lmasa bo'sh) — `each f in req.files`
    // nil tekshiruvisiz ishlaydi (issue #133).
    m.insert("files".to_string(), Value::List(files));
    Value::Map(m)
}

// req map'iga shared ctx cell'ni (`req.ctx`) qo'shadi (issue #68). build_req'dan
// alohida — har so'rovga yangi cell yaratiladi (caller'da), bu funksiya uni
// req'ning "ctx" kalitiga joylaydi.
fn with_ctx(req: Value, ctx: Arc<Mutex<BTreeMap<String, Value>>>) -> Value {
    if let Value::Map(mut m) = req {
        m.insert("ctx".to_string(), Value::Ctx(ctx));
        Value::Map(m)
    } else {
        req
    }
}

// --- Value/Flow -> hyper::Response ---

// Fluxon `Int` statusni (rep/fail) yaroqli HTTP status u16'ga aylantiradi.
// Tekshiruv ASL i64 ustida bo'lishi shart: `as u16` cast oldin wrap qiladi —
// `rep 65736` u16'da 200 ga, ba'zi manfiy qiymatlar 3xx/4xx ga tushib jim
// muvaffaqiyatga aldardi (issue #108). Diapazondan tashqari yoki HTTP bo'lmagan
// kod → 500 + log, shunda mijoz handler xatosini muvaffaqiyat deb o'qimaydi.
fn checked_status(n: i64) -> u16 {
    match u16::try_from(n) {
        Ok(s) if StatusCode::from_u16(s).is_ok() => s,
        _ => {
            eprintln!("Fluxon HTTP: invalid status code {} → 500", n);
            500
        }
    }
}

// u16 status → StatusCode. Builder darajasidagi himoya to'ri: chaqiruvchilar
// allaqachon yaroqli kod beradi (literal yoki `checked_status`), bu faqat
// kutilmagan holatda panic o'rniga 500 qaytaradi.
fn status_or_500(status: u16) -> StatusCode {
    StatusCode::from_u16(status).unwrap_or_else(|_| {
        eprintln!("Fluxon HTTP: invalid status code {} → 500", status);
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

fn json_response(status: u16, body: String) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status_or_500(status))
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}

fn text_response(status: u16, body: String) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status_or_500(status))
        .header("content-type", "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}

// 413 Payload Too Large — so'rov tanasi o'lcham chegarasidan oshib ketdi (#91).
fn payload_too_large(limit: usize) -> Response<Full<Bytes>> {
    let mut m = BTreeMap::new();
    m.insert(
        "error".to_string(),
        Value::Str(format!("request body too large (limit: {} bytes)", limit)),
    );
    json_response(413, json_encode(&Value::Map(m)))
}

// 400 Bad Request — so'rov tanasini o'qishda xato (masalan uzilgan ulanish) (#91).
fn bad_request(msg: &str) -> Response<Full<Bytes>> {
    let mut m = BTreeMap::new();
    m.insert("error".to_string(), Value::Str(msg.to_string()));
    json_response(400, json_encode(&Value::Map(m)))
}

// Qiymat `rep`-javobmi? `rep status body` -> {__resp:true ...} map (builtins.rs).
// Middleware shu javobni qaytarsa zanjir to'xtaydi (P1: rep auth rad etishi).
fn is_resp(v: &Value) -> bool {
    matches!(v, Value::Map(m) if matches!(m.get("__resp"), Some(Value::Bool(true))))
}

// Handler muvaffaqiyatli qaytargan qiymatni javobga aylantiradi.
// `rep` -> {__resp:true status body}. Aks holda 200 + qiymat.
fn value_to_response(v: Value) -> Response<Full<Bytes>> {
    if is_resp(&v)
        && let Value::Map(m) = &v
    {
        let status = match m.get("status") {
            Some(Value::Int(n)) => checked_status(*n),
            _ => 200,
        };
        let body = m.get("body").cloned().unwrap_or(Value::Nil);
        // 3-argument custom header'lar (issue #16): `rep status body {hdr:val}`.
        let custom = m.get("headers");
        // Redirect: `rep 30x {location:url}` → body map'idagi location'ni Location
        // header'ga chiqaramiz (spec: "Redirect: rep 302 {location:url}"). Eski
        // qulaylik xulqi — custom headers'siz ham ishlaydi, body bo'sh qaytadi.
        if (300..400).contains(&status)
            && let Value::Map(bm) = &body
            && let Some(Value::Str(loc)) = bm.get("location")
        {
            let mut b = Response::builder()
                .status(StatusCode::from_u16(status).unwrap_or(StatusCode::FOUND))
                .header("location", loc.clone());
            b = apply_headers(b, custom);
            return b.body(Full::new(Bytes::new())).unwrap();
        }
        let mut resp = body_value_to_response(status, body);
        apply_headers_mut(resp.headers_mut(), custom);
        return resp;
    }
    // rep ishlatilmagan — qiymatning o'zini 200 bilan qaytaramiz.
    body_value_to_response(200, v)
}

// Custom header map'ini Response::Builder'ga qo'shadi (redirect yo'li uchun —
// u hali builder bosqichida). Noto'g'ri header nomi/qiymati jim o'tkazib
// yuboriladi: yagona buzuq header butun javobni 500 qilmasligi kerak.
fn apply_headers(
    mut b: hyper::http::response::Builder,
    headers: Option<&Value>,
) -> hyper::http::response::Builder {
    if let Some(Value::Map(hm)) = headers
        && let Some(hmap) = b.headers_mut()
    {
        apply_headers_mut(hmap, Some(&Value::Map(hm.clone())));
    }
    b
}

// Custom header map'ini tayyor Response'ning HeaderMap'iga qo'shadi.
//
// Qiymat str bo'lsa — bitta sarlavha. List bo'lsa — har element alohida
// sarlavha qatori (takror header, masalan bir nechta Set-Cookie; RFC 7230 ga
// ko'ra Set-Cookie vergulli ro'yxat bilan birlashmaydi). content-type kabi
// turdagi sarlavhalar body'ning standart sarlavhasini bosib o'tadi (append
// emas, insert): canonical body sarlavhasi ustidan dasturchi niyati ustun.
//
// Kalitda `_` → `-` ga aylanadi: Fluxon map kalitida defis bo'lolmaydi
// (`content-type` uchta token sifatida parse bo'ladi), shuning uchun
// `{content_type:"..."}` yoziladi. Bu o'qish bilan simmetrik — server
// req.headers'da ham `-` → `_` qiladi (build_req), AI bitta naqshni o'rganadi.
// Defisli string kalit (`{"set-cookie":...}`) ham ishlaydi: defisda `_` yo'q.
fn apply_headers_mut(hmap: &mut hyper::HeaderMap, headers: Option<&Value>) {
    use hyper::header::{HeaderName, HeaderValue};
    let Some(Value::Map(hm)) = headers else {
        return;
    };
    for (k, v) in hm {
        // Header nomi case-insensitive (RFC 7230) — lowercase kanonik shaklda
        // saqlaymiz. Buzuq nomni jim o'tkazib yuboramiz.
        let canon = k.to_lowercase().replace('_', "-");
        let Ok(name) = HeaderName::from_bytes(canon.as_bytes()) else {
            continue;
        };
        match v {
            // List — takror sarlavha: birinchisi insert (eskisini bosadi),
            // qolganlari append.
            Value::List(items) => {
                let mut first = true;
                for item in items.iter() {
                    if let Ok(hv) = HeaderValue::from_str(&item.to_text()) {
                        if first {
                            hmap.insert(name.clone(), hv);
                            first = false;
                        } else {
                            hmap.append(name.clone(), hv);
                        }
                    }
                }
            }
            // Boshqa har qanday qiymat matn sifatida — bitta sarlavha.
            other => {
                if let Ok(hv) = HeaderValue::from_str(&other.to_text()) {
                    hmap.insert(name, hv);
                }
            }
        }
    }
}

// HeaderMap -> Fluxon header map (kalitlar lowercase). O'qish tomonidagi yagona
// yo'l — server req.headers ham, klient res.headers ham shu orqali quriladi.
//
// Bir nomli takror header'lar yo'qolmasligi uchun (issue #101) qiymatlar
// RFC 9110 §5.3 bo'yicha ", " bilan birlashtiriladi. Ikki istisno:
//   - `cookie` "; " bilan (RFC 6265 — cookie-pair ajratkichi vergul emas);
//   - `set-cookie` umuman birlashtirilmaydi (Expires sanasida vergul bor) —
//     takror bo'lsa List qaytadi, yozish tomonidagi List bilan simmetrik.
// UTF-8 bo'lmagan baytlar lossy o'qiladi (oldin jim bo'sh string bo'lardi).
fn headers_to_map(hmap: &hyper::HeaderMap) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for key in hmap.keys() {
        let name = key.as_str().to_lowercase();
        let vals: Vec<String> = hmap
            .get_all(key)
            .iter()
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .collect();
        let value = if name == "set-cookie" && vals.len() > 1 {
            Value::List(vals.into_iter().map(Value::Str).collect())
        } else if name == "cookie" {
            Value::Str(vals.join("; "))
        } else {
            Value::Str(vals.join(", "))
        };
        out.insert(name, value);
    }
    out
}

// Javob tanasini tipiga qarab formatlash: map/list -> JSON, str -> matn,
// nil -> bo'sh, qolgani -> JSON.
fn body_value_to_response(status: u16, body: Value) -> Response<Full<Bytes>> {
    match body {
        Value::Nil => Response::builder()
            .status(status_or_500(status))
            .body(Full::new(Bytes::new()))
            .unwrap(),
        Value::Str(s) => text_response(status, s),
        // Ikkilik javob (rasm, PDF, arxiv — issue #132). Standart tur
        // octet-stream; aniq tur 3-arg bilan: rep 200 b {content_type:"image/png"}.
        Value::Bytes(b) => Response::builder()
            .status(status_or_500(status))
            .header("content-type", "application/octet-stream")
            .body(Full::new(Bytes::from(b.as_ref().clone())))
            .unwrap(),
        Value::Map(_) | Value::List(_) => json_response(status, json_encode(&body)),
        other => text_response(status, format!("{}", other)),
    }
}

// fail/error -> JSON xato javob.
fn flow_to_response(flow: Flow) -> Response<Full<Bytes>> {
    let (status, message) = match flow {
        Flow::Fail { status, message } => (checked_status(status.unwrap_or(400)), message),
        Flow::Error(e) => (500, e),
        Flow::Return(v) => return value_to_response(v), // handler ichida `ret`
        Flow::Skip | Flow::Stop => (500, "handler used skip/stop".to_string()),
    };
    let mut m = BTreeMap::new();
    m.insert("error".to_string(), Value::Str(message));
    json_response(status, json_encode(&Value::Map(m)))
}

// --- Interp HTTP dispatch ---

impl Interp {
    // http.<func> chaqiruvlari. eval_call shu yerga yo'naltiradi.
    pub fn http_dispatch(self: &Arc<Self>, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            "on" => self.http_on(args),
            "use" => self.http_use(args),
            "before" => self.http_before(args),
            "cors" => self.http_cors(args),
            "static" => self.http_static(args),
            "limit" => self.http_limit(args),
            "serve" => self.http_serve(args),
            "get" => http_client("GET", args, false),
            "post" => http_client("POST", args, true),
            "put" => http_client("PUT", args, true),
            "del" => http_client("DELETE", args, false),
            _ => Err(Flow::err(format!("http module has no '{}' function", func))),
        }
    }

    // http.on :method "/path" handler
    fn http_on(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let method = match args.first() {
            Some(Value::Sym(s)) | Some(Value::Str(s)) => s.to_lowercase(),
            _ => {
                return Err(Flow::err(
                    "http.on: argument 1 must be a method (:get/:post...)",
                ));
            }
        };
        let path = match args.get(1) {
            Some(Value::Str(s)) => s.clone(),
            _ => return Err(Flow::err("http.on: argument 2 must be a path (str)")),
        };
        let handler = match args.get(2) {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => return Err(Flow::err("http.on: argument 3 must be a handler (fn)")),
        };
        self.routes.lock().unwrap().push(Route {
            method,
            pattern: parse_pattern(&path),
            handler,
        });
        Ok(Value::Nil)
    }

    // http.use \req -> ...  — barcha route'larga global middleware (issue #67).
    // Bir nechta chaqiruv zanjir hosil qiladi (deklaratsiya tartibida ishlaydi).
    fn http_use(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let handler = match args.first() {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => return Err(Flow::err("http.use: argument must be a handler (fn)")),
        };
        self.middlewares.lock().unwrap().push(Middleware {
            scope: None,
            handler,
            kind: MwKind::Fn,
        });
        Ok(Value::Nil)
    }

    // http.before "/api/*" \req -> ...  — yo'l prefiks bo'yicha middleware (#67).
    // Shablon "/api/*" → /api bilan boshlanuvchi yo'llar; "*"siz → aniq mos.
    fn http_before(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let pat = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => {
                return Err(Flow::err("http.before: argument 1 must be a path (str)"));
            }
        };
        let handler = match args.get(1) {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => {
                return Err(Flow::err("http.before: argument 2 must be a handler (fn)"));
            }
        };
        self.middlewares.lock().unwrap().push(Middleware {
            scope: Some(pat),
            handler,
            kind: MwKind::Fn,
        });
        Ok(Value::Nil)
    }

    // http.cors origins [opts]  — deklarativ CORS (issue #135).
    //
    //   http.cors "*"                                # hammaga ochiq (dev)
    //   http.cors ["https://app.example.com"]        # ruxsat etilgan origin'lar
    //   http.cors ["https://app.example.com"] {creds: true}
    //
    // 1-argument: "*" (str) — har qanday origin, yoki origin'lar ro'yxati (list).
    // 2-argument (ixtiyoriy): opsiyalar map'i:
    //   creds:   true → Allow-Credentials (cookie/Authorization). "*" bilan birga
    //            ishlatilsa javob so'rov origin'ini aks ettiradi (brauzer talabi).
    //   methods: ruxsat etilgan metodlar (str). Default keng to'plam.
    //   headers: ruxsat etilgan so'rov header'lari (str). Default keng to'plam.
    //   max_age: preflight kesh muddati soniyada (int). Default 86400 (1 kun).
    fn http_cors(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let origins = match args.first() {
            // "*" — har qanday origin (None ichki ifoda).
            Some(Value::Str(s)) if s == "*" => None,
            // Bitta origin str sifatida ham qabul qilamiz (qulaylik).
            Some(Value::Str(s)) => Some(vec![s.clone()]),
            // Origin'lar ro'yxati.
            Some(Value::List(items)) => {
                let mut list = Vec::with_capacity(items.len());
                for it in items.iter() {
                    match it {
                        Value::Str(s) => list.push(s.clone()),
                        _ => {
                            return Err(Flow::err(
                                "http.cors: origin list must consist of str elements",
                            ));
                        }
                    }
                }
                Some(list)
            }
            _ => {
                return Err(Flow::err(
                    "http.cors: argument 1 must be \"*\" or a list of origins",
                ));
            }
        };

        let mut cfg = CorsConfig {
            origins,
            // Keng standart to'plam — agent alohida sozlamasdan ishlaydi.
            methods: "GET, POST, PUT, PATCH, DELETE, OPTIONS".to_string(),
            headers: "Content-Type, Authorization".to_string(),
            creds: false,
            max_age: 86400,
        };

        if let Some(Value::Map(opts)) = args.get(1) {
            if let Some(v) = opts.get("creds") {
                cfg.creds = !matches!(v, Value::Nil | Value::Bool(false));
            }
            if let Some(Value::Str(s)) = opts.get("methods") {
                cfg.methods = s.clone();
            }
            if let Some(Value::Str(s)) = opts.get("headers") {
                cfg.headers = s.clone();
            }
            if let Some(Value::Int(n)) = opts.get("max_age")
                && *n >= 0
            {
                cfg.max_age = *n as u64;
            }
        } else if args.len() > 1 && !matches!(args.get(1), Some(Value::Nil)) {
            return Err(Flow::err(
                "http.cors: argument 2 must be an options map ({creds: true})",
            ));
        }

        *self.cors.lock().unwrap() = Some(cfg);
        Ok(Value::Nil)
    }

    // http.static prefiks katalog [opts]  — papkadan static fayl tarqatish (#134).
    //
    //   http.static "/assets" "./public"        # /assets/app.css -> ./public/app.css
    //   http.static "/" "./dist" {spa: true}    # topilmasa -> ./dist/index.html
    //
    // Katalog skript fayli katalogiga nisbatan hal qilinadi (`use ./fayl` bilan
    // bir xil qoida) va registratsiyada canonicalize qilinadi — yo'q katalog
    // start'dayoq xato beradi (deploy paytida jim 404 o'rniga fail fast).
    // Content-Type kengaytmadan avtomatik; `../` traversal (percent-encoded ham)
    // majburiy bloklanadi; route prioriteti: aniq route > static.
    fn http_static(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let prefix = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => {
                return Err(Flow::err(
                    "http.static: argument 1 must be a prefix (str), for example \"/assets\"",
                ));
            }
        };
        let dir = match args.get(1) {
            Some(Value::Str(s)) => s.clone(),
            _ => {
                return Err(Flow::err(
                    "http.static: argument 2 must be a directory (str), for example \"./public\"",
                ));
            }
        };
        let spa = match args.get(2) {
            None | Some(Value::Nil) => false,
            Some(Value::Map(m)) => !matches!(
                m.get("spa"),
                None | Some(Value::Nil) | Some(Value::Bool(false))
            ),
            _ => {
                return Err(Flow::err(
                    "http.static: argument 3 must be an options map ({spa: true})",
                ));
            }
        };
        let p = PathBuf::from(&dir);
        let resolved = if p.is_absolute() {
            p
        } else {
            self.base_dir().join(p)
        };
        let canon = std::fs::canonicalize(&resolved).map_err(|e| {
            Flow::err(format!(
                "http.static: could not open directory '{}': {}",
                dir, e
            ))
        })?;
        if !canon.is_dir() {
            return Err(Flow::err(format!(
                "http.static: '{}' is not a directory (a file was given)",
                dir
            )));
        }
        self.statics.lock().unwrap().push(StaticMount {
            prefix: parse_static_prefix(&prefix),
            dir: canon,
            spa,
        });
        Ok(Value::Nil)
    }

    // http.limit [path] N :sec|:min|:hr \req -> kalit  — deklarativ rate-limit (#79).
    //
    //   http.limit 100 :min \req -> req.ctx.tenant_id          # per-tenant, barcha yo'l
    //   http.limit "/api/*" 100 :min \req -> req.headers.x_api_key  # per-key, prefiks
    //
    // Path (str) ixtiyoriy 1-argument — bo'lsa http.before kabi prefiks bo'yicha
    // ulanadi, bo'lmasa http.use kabi global. Kalit funksiyasi har request uchun
    // chaqirilib mijozni aniqlaydi; nil/bo'sh qaytarsa req.ip'ga qaytamiz. Limit
    // oshsa avtomatik `429` + `Retry-After` (oyna tugashigacha soniya).
    fn http_limit(&self, args: Vec<Value>) -> Result<Value, Flow> {
        // 1-argument str bo'lsa — path scope (http.before kabi). Aks holda global.
        let (scope, i) = match args.first() {
            Some(Value::Str(s)) => (Some(s.clone()), 1),
            _ => (None, 0),
        };
        let limit = match args.get(i) {
            Some(Value::Int(n)) if *n > 0 => *n as u32,
            _ => {
                return Err(Flow::err(
                    "http.limit: limit must be a positive int (for example 100)",
                ));
            }
        };
        let window_secs = match args.get(i + 1) {
            Some(Value::Sym(s)) | Some(Value::Str(s)) => match window_to_secs(s) {
                Some(secs) => secs,
                None => {
                    return Err(Flow::err("http.limit: window must be :sec, :min or :hr"));
                }
            },
            _ => {
                return Err(Flow::err(
                    "http.limit: window unit (:sec/:min/:hr) is required",
                ));
            }
        };
        let keyfn = match args.get(i + 2) {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => {
                return Err(Flow::err(
                    "http.limit: key function (\\req -> ...) is required",
                ));
            }
        };
        self.middlewares.lock().unwrap().push(Middleware {
            scope,
            handler: keyfn,
            kind: MwKind::Limit {
                limit,
                window_secs,
                state: Arc::new(Mutex::new(LimitBucket::new())),
            },
        });
        Ok(Value::Nil)
    }

    // http.serve port — bloklovchi tokio multi-thread server.
    // `http.serve PORT` — serverni DARHOL bloklamaydi, balki kutilayotgan
    // serverlar ro'yxatiga qo'shadi (deferred). Top-level kod tugagach
    // (`serve_mod::run_pending`) hammasi BITTA umumiy tokio runtime'da
    // spawn qilinadi — shunda HTTP + WS bir jarayonda birga ishlaydi.
    fn http_serve(self: &Arc<Self>, args: Vec<Value>) -> Result<Value, Flow> {
        let port = match args.first() {
            Some(Value::Int(n)) => *n as u16,
            _ => return Err(Flow::err("http.serve: port (int) is required")),
        };
        // Ixtiyoriy ikkinchi argument — opsiyalar map'i: `{max_body: BAYT}`.
        // Berilmasa default DEFAULT_MAX_BODY; `max_body: 0` chegarani o'chiradi.
        let max_body = match args.get(1) {
            None => DEFAULT_MAX_BODY,
            Some(Value::Map(m)) => match m.get("max_body") {
                None => DEFAULT_MAX_BODY,
                Some(Value::Int(n)) if *n >= 0 => *n as usize,
                _ => {
                    return Err(Flow::err("http.serve: max_body must be a non-negative int"));
                }
            },
            _ => {
                return Err(Flow::err(
                    "http.serve: second argument must be an options map ({max_body: N})",
                ));
            }
        };
        self.pending_servers
            .lock()
            .unwrap()
            .push(crate::serve_mod::PendingServer::Http { port, max_body });
        Ok(Value::Nil)
    }
}

// Port'ni bind qiladi (deferred: top-level tugagandan keyin, `serve_mod`).
// Bind xatosini `Flow::Error` sifatida qaytaradi — `run_pending` uni yuqoriga
// ko'taradi, shunda port band bo'lsa jarayon exit code ≠ 0 bilan tugaydi
// (issue #108: deploy/supervisor xatoni sezsin). Accept loop'dan oldin
// chaqiriladi, shuning uchun bind muvaffaqiyatsizligi spawn'dan oldin chiqadi.
pub async fn bind(port: u16) -> Result<TcpListener, Flow> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    TcpListener::bind(addr)
        .await
        .map_err(|e| Flow::err(format!("Fluxon HTTP port {} bind error: {}", port, e)))
}

// Bitta HTTP server uchun accept loop — umumiy event-loop ichida spawn qilinadi
// (`serve_mod`). Listener oldindan `bind` bilan ochilgan (bind xatosi spawn'dan
// oldin ko'tariladi).
pub async fn serve_loop(interp: Arc<Interp>, listener: TcpListener, max_body: usize) {
    let port = listener.local_addr().map(|a| a.port()).unwrap_or_default();
    eprintln!("Fluxon HTTP server: http://localhost:{}", port);

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("http accept error: {}", e);
                continue;
            }
        };
        let io = TokioIo::new(stream);
        let interp = interp.clone();
        // Mijoz IP (rate-limit fallback + req.ip). peer SocketAddr — IP'sini
        // ulanish bo'yi bir marta olamiz (har request shu connection'da bir IP).
        let client_ip = peer.ip().to_string();
        tokio::spawn(async move {
            let service = service_fn(move |req: Request<Incoming>| {
                let interp = interp.clone();
                let client_ip = client_ip.clone();
                async move { handle_request(interp, req, client_ip, max_body).await }
            });
            // header_read_timeout slowloris ulanishlarini (header'larni juda sekin
            // yuboradigan) cheklaydi (issue #92). Timer o'rnatilmasa sozlama jim
            // e'tiborsiz qoladi (yoki panic) — TokioTimer beramiz.
            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(Duration::from_secs(DEFAULT_HEADER_READ_TIMEOUT_SECS))
                .serve_connection(io, service)
                .await
            {
                eprintln!("connection error: {}", e);
            }
        });
    }
}

// Middleware zanjirini ishlatadi (sinxron — spawn_blocking ichida chaqiriladi).
// Har middleware req klonini oladi (ctx Arc ulashilgan). Natija:
//   - Ok(Some(v)) — biri javob qaytardi (`rep` yoki limit 429), zanjir to'xtadi,
//     handler CHAQIRILMAYDI; aks holda auth `rep 401` e'tiborsiz qolardi.
//   - Ok(None)   — hammasi o'tdi (ctx yozish, log), handler davom etadi.
//   - Err(flow)  — `fail`/xato, zanjir to'xtadi.
// Route handler'lari ham, static fayllar ham (issue #134) SHU zanjir orqali
// o'tadi — http.before auth static papkani ham himoya qiladi.
fn run_middleware_chain(
    interp: &Interp,
    chain: Vec<Middleware>,
    request_value: &Value,
) -> Result<Option<Value>, Flow> {
    for mw in chain {
        match mw.kind {
            // Oddiy middleware (use/before): handler'ni chaqiramiz.
            MwKind::Fn => match interp.apply(mw.handler, vec![request_value.clone()]) {
                Ok(v) if is_resp(&v) => return Ok(Some(v)), // rep -> javob, zanjir to'xta
                Ok(_) => {}                                 // davom (ctx/log)
                Err(flow) => return Err(flow),              // fail/xato -> to'xta
            },
            // Rate-limit (http.limit): kalit funksiyasini chaqirib mijozni
            // aniqlaymiz, keyin hisobgichni tekshiramiz. Oshsa 429 -> zanjir to'xta.
            MwKind::Limit {
                limit,
                window_secs,
                state,
            } => {
                let key = match interp.apply(mw.handler, vec![request_value.clone()]) {
                    // nil -> mijoz IP'siga qaytamiz (kalitsiz so'rovni ham cheklash).
                    Ok(Value::Nil) => client_fallback_key(request_value),
                    Ok(v) => {
                        let t = v.to_text();
                        if t.is_empty() {
                            client_fallback_key(request_value)
                        } else {
                            t
                        }
                    }
                    Err(flow) => return Err(flow), // kalit fn xato berdi -> to'xta
                };
                if let Some(retry) = check_and_count(&state, &key, limit, window_secs) {
                    return Ok(Some(rate_limited_response(retry)));
                }
            }
        }
    }
    Ok(None)
}

// Static fayl urinishi (issue #134) — faqat aniq route topilmaganda chaqiriladi
// (route prioriteti). None — static mos kelmadi, chaqiruvchi 404 qaytaradi.
// Faqat GET/HEAD (fayl o'qish idempotent; boshqa metodlar API semantikasi).
// Middleware zanjiri bu yerda ham ishlaydi — `http.before "/admin/*"` auth
// static papkani ham himoya qilsin (zanjir javob qaytarsa fayl o'qilmaydi).
async fn try_serve_static(
    interp: &Arc<Interp>,
    method: &str,
    path: &str,
    query: String,
    headers: BTreeMap<String, Value>,
    client_ip: String,
) -> Option<Response<Full<Bytes>>> {
    if method != "get" && method != "head" {
        return None;
    }
    let mounts: Vec<StaticMount> = interp.statics.lock().unwrap().clone();
    if mounts.is_empty() {
        return None;
    }
    // Segmentlar percent-dekod qilinadi (brauzer non-ASCII nomni encode qiladi);
    // `keep_path_seps=true` — `%2F` xom qoladi, dekoddan yangi `/` tug'ilmaydi.
    let segs: Vec<String> = path_segments(path)
        .iter()
        .map(|s| percent_decode(s, true))
        .collect();
    // Hech bir mount prefiksi mos kelmasa — static hududi emas, oddiy 404
    // (middleware'siz, route-404 bilan bir xil xulq).
    if !mounts
        .iter()
        .any(|m| strip_mount_prefix(&m.prefix, &segs).is_some())
    {
        return None;
    }

    // Middleware zanjiri fayl MAVJUDLIGIDAN OLDIN ishlaydi (codex P2): aks
    // holda himoyalangan mount ostida bor fayl 401, yo'q fayl 404 qaytarib,
    // auth'siz mijoz fayl nomlarini status farqidan topa olardi. Prefiks mos
    // kelgan har so'rov — fayl bor-yo'qligidan qat'i nazar — zanjirdan o'tadi.
    let chain: Vec<Middleware> = interp
        .middlewares
        .lock()
        .unwrap()
        .iter()
        .filter(|mw| match &mw.scope {
            None => true,
            Some(pat) => prefix_matches(pat, path),
        })
        .cloned()
        .collect();
    if !chain.is_empty() {
        // GET/HEAD tanasi bo'sh — body o'qilmaydi (route yo'lidan farqli).
        let ctx = Arc::new(Mutex::new(BTreeMap::new()));
        let request_value = with_ctx(
            build_req(
                method.to_string(),
                path.to_string(),
                query,
                headers,
                BTreeMap::new(),
                client_ip,
                Bytes::new(),
                false,
                None,
            ),
            ctx,
        );
        let interp2 = interp.clone();
        let mw_result = tokio::task::spawn_blocking(move || {
            run_middleware_chain(&interp2, chain, &request_value)
        })
        .await;
        match mw_result {
            Ok(Ok(None)) => {} // zanjir o'tdi — faylga o'tamiz
            Ok(Ok(Some(v))) => return Some(value_to_response(v)),
            Ok(Err(flow)) => return Some(flow_to_response(flow)),
            Err(join_err) => {
                return Some(flow_to_response(Flow::Error(format!(
                    "middleware panic: {}",
                    join_err
                ))));
            }
        }
    }
    let (file, mime, len) = resolve_static(&mounts, &segs).await?;
    // HEAD — mazmun kerak emas: faylni O'QIMASDAN metadata hajmi bilan javob
    // (katta asset'da behuda disk I/O / xotira bo'lmasin — codex P2).
    if method == "head" {
        return Some(static_head_response(len, mime));
    }
    match tokio::fs::read(&file).await {
        Ok(data) => Some(static_response(data, mime)),
        // metadata fayl deb ko'rsatdi, lekin o'qish baribir xato (race/ruxsat) —
        // jim 404 ga tushamiz (fayl mavjudligi haqida ma'lumot sizdirmaymiz).
        Err(_) => None,
    }
}

// Bitta so'rovni boshqaradi: marshrut topish -> req qurish -> handler'ni
// spawn_blocking'da (sinxron interp) chaqirish -> javob.
async fn handle_request(
    interp: Arc<Interp>,
    req: Request<Incoming>,
    client_ip: String,
    max_body: usize,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method().as_str().to_lowercase();
    let uri = req.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or("").to_string();

    // Sarlavhalarni map'ga yig'amiz (kalitlar lowercase, '-' -> '_' shunda
    // Fluxon'da req.headers.x_user_id sifatida o'qiladi). Takror nomlar
    // headers_to_map ichida birlashtiriladi (issue #101).
    let headers: BTreeMap<String, Value> = headers_to_map(req.headers())
        .into_iter()
        .map(|(k, v)| (k.replace('-', "_"), v))
        .collect();
    // CORS sozlamasi (issue #135) — yoqilgan bo'lsa Origin'ga qarab preflight'ga
    // javob beramiz va har javobga `Access-Control-Allow-*` qo'shamiz. So'rovning
    // Origin header'i (headers'da '-' -> '_' qilingan) ruxsat tekshiruvi uchun.
    let cors = interp.cors.lock().unwrap().clone();
    let req_origin = match headers.get("origin") {
        Some(Value::Str(o)) => Some(o.clone()),
        _ => None,
    };

    // CORS preflight — CORS yoqilgan bo'lsa route izlamasdan to'g'ridan-to'g'ri
    // javob beramiz (brauzer haqiqiy so'rovdan oldin OPTIONS yuboradi). Bu route
    // topilmasligi (404) muammosini ham hal qiladi: preflight uchun handler shart
    // emas.
    //
    // HAR OPTIONS emas — faqat HAQIQIY preflight'ni ushlaymiz: Fetch standartiga
    // ko'ra brauzer preflight'i HAR DOIM Access-Control-Request-Method header
    // yuboradi. Bu shart bo'lmasa (oddiy OPTIONS — resurs imkoniyatini so'rash
    // yoki foydalanuvchining `http.on :options "/..."` handler'i), so'rov
    // odatdagidek marshrutga tushadi (codex P2). CORS o'chiq bo'lsa ham OPTIONS
    // oddiy marshrutga tushadi.
    let is_preflight = method == "options" && headers.contains_key("access_control_request_method");
    if is_preflight && let Some(cfg) = &cors {
        return Ok(cors_preflight_response(cfg, req_origin.as_deref()));
    }

    let is_json = matches!(
        headers.get("content_type"),
        Some(Value::Str(ct)) if ct.contains("application/json")
    );
    // multipart/form-data boundary (issue #133) — bo'lsa tana qismlarga
    // ajratiladi (req.body maydonlar, req.files fayllar).
    let multipart = match headers.get("content_type") {
        Some(Value::Str(ct)) => multipart_boundary(ct),
        _ => None,
    };

    // Marshrutni topamiz (handler'ni baytlardan oldin, 404 ni tez qaytarish uchun).
    let matched = {
        let routes = interp.routes.lock().unwrap();
        match_route(&routes, &method, &path)
    };

    let (route, params) = match matched {
        Some(x) => x,
        None => {
            // Aniq route topilmadi — static mount'lardan urinamiz (issue #134).
            // Route prioriteti: aniq route > static (static faqat shu yerda).
            if let Some(resp) = try_serve_static(
                &interp,
                &method,
                &path,
                query.clone(),
                headers.clone(),
                client_ip.clone(),
            )
            .await
            {
                return Ok(cors_finalize(resp, &cors, req_origin.as_deref()));
            }
            let mut m = BTreeMap::new();
            m.insert(
                "error".to_string(),
                Value::Str(format!("not found: {} {}", method, path)),
            );
            // 404 ham CORS header oladi — aks holda brauzer xato javob tanasini
            // CORS to'sig'i sabab o'qiy olmaydi (debugni qiyinlashtiradi).
            let resp = json_response(404, json_encode(&Value::Map(m)));
            return Ok(cors_finalize(resp, &cors, req_origin.as_deref()));
        }
    };

    // Tanani yig'amiz — o'lcham chegarasi bilan (issue #91). Chegarasiz collect()
    // butun tanani xotiraga yig'adi: mijoz ulkan body yuborib server xotirasini
    // to'ldira oladi (DoS).
    let body_bytes = if max_body == 0 {
        // Chegara o'chirilgan (http.serve PORT {max_body: 0}) — cheklovsiz o'qish.
        match req.into_body().collect().await {
            Ok(c) => c.to_bytes(),
            // Oldin bu jim Bytes::new() ga tushardi (uzilgan POST handler'ga
            // body:nil bilan yetardi); endi 400 qaytaramiz (issue #91). CORS
            // header bilan yakunlanadi (codex P2: har javob CORS oladi).
            Err(_) => {
                return Ok(cors_finalize(
                    bad_request("could not read request body"),
                    &cors,
                    req_origin.as_deref(),
                ));
            }
        }
    } else {
        // Tez yo'l: Content-Length e'lon qilingan o'lcham chegaradan oshsa, tanani
        // umuman o'qimasdan 413 qaytaramiz (mijoz GB'lab yuklab tugatishini
        // kutmaymiz). Yolg'on/yo'q Content-Length'ni quyidagi Limited ushlaydi.
        let declared = req
            .headers()
            .get(hyper::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());
        if matches!(declared, Some(len) if len > max_body as u64) {
            return Ok(cors_finalize(
                payload_too_large(max_body),
                &cors,
                req_origin.as_deref(),
            ));
        }
        // Limited oqim davomida ham haqiqiy chegarani majburlaydi — chegara oshsa
        // o'qishni to'xtatadi (Content-Length yolg'on bo'lsa ham himoyalaydi).
        match Limited::new(req.into_body(), max_body).collect().await {
            Ok(c) => c.to_bytes(),
            Err(e) => {
                // Chegara oshib ketdi -> 413; boshqa o'qish xatosi (masalan uzilgan
                // ulanish) -> 400.
                if e.downcast_ref::<http_body_util::LengthLimitError>()
                    .is_some()
                {
                    return Ok(cors_finalize(
                        payload_too_large(max_body),
                        &cors,
                        req_origin.as_deref(),
                    ));
                }
                return Ok(cors_finalize(
                    bad_request("could not read request body"),
                    &cors,
                    req_origin.as_deref(),
                ));
            }
        }
    };

    // Bu so'rovga mos middleware zanjirini yig'amiz (issue #67): avval global
    // (http.use), keyin yo'l prefiks bo'yicha (http.before). Ro'yxat tartibi
    // saqlanadi. Lock'larni shu yerda erta olib qo'yamiz (handler'lar Value
    // klonlari — Arc, arzon).
    let chain: Vec<Middleware> = {
        interp
            .middlewares
            .lock()
            .unwrap()
            .iter()
            .filter(|mw| match &mw.scope {
                None => true,                            // http.use/limit — barcha yo'lga
                Some(pat) => prefix_matches(pat, &path), // http.before/limit — prefiks mos
            })
            .cloned() // Middleware klonida Limit holati Arc — bir xil pointer ulashiladi
            .collect()
    };

    // Request-scoped ctx cell: har so'rovga yangi. req klonlari Arc'ni ulashadi,
    // shuning uchun middleware yozgan ctx'ni handler bir xil cell'da ko'radi (#68).
    let ctx = Arc::new(Mutex::new(BTreeMap::new()));
    let request_value = with_ctx(
        build_req(
            method, path, query, headers, params, client_ip, body_bytes, is_json, multipart,
        ),
        ctx,
    );
    let handler = route.handler;

    // Sinxron interp ishini blocking thread'da bajaramiz — tokio worker'ini
    // bloklamaydi, har request alohida thread'da -> haqiqiy parallel.
    let result = tokio::task::spawn_blocking(move || {
        match run_middleware_chain(&interp, chain, &request_value) {
            Ok(Some(v)) => Ok(v), // middleware javob qaytardi (rep/429) -> handler chaqirilmaydi
            Ok(None) => interp.apply(handler, vec![request_value]),
            Err(flow) => Err(flow),
        }
    })
    .await;

    let resp = match result {
        Ok(Ok(v)) => value_to_response(v),
        Ok(Err(flow)) => flow_to_response(flow),
        Err(join_err) => flow_to_response(Flow::Error(format!("handler panic: {}", join_err))),
    };
    // CORS yoqilgan bo'lsa har javobga `Access-Control-Allow-*` qo'shamiz
    // (issue #135). Handler `rep ... {access_control_allow_origin: ...}` bilan
    // qo'lda yozgan bo'lsa ham insert ustidan yozadi — kanonik sozlama ustun.
    Ok(cors_finalize(resp, &cors, req_origin.as_deref()))
}

// OPTIONS preflight javobi (issue #135). Brauzer haqiqiy so'rovdan oldin
// OPTIONS yuboradi va `Access-Control-Allow-*` header'larni kutadi. Tana yo'q
// (204 No Content). Origin ruxsat etilmagan bo'lsa CORS header'larsiz 204
// qaytadi (brauzer so'rovni bloklaydi — to'g'ri xulq).
fn cors_preflight_response(cfg: &CorsConfig, req_origin: Option<&str>) -> Response<Full<Bytes>> {
    let mut b = Response::builder().status(StatusCode::NO_CONTENT);
    if let Some(hmap) = b.headers_mut() {
        cfg.apply_to(hmap, req_origin);
        // Preflight'ga xos header'lar (oddiy javobda kerak emas): ruxsat etilgan
        // metodlar, header'lar va kesh muddati. Faqat origin ruxsat etilganda
        // qo'shamiz — apply_to Allow-Origin qo'ymagan bo'lsa preflight'ni ham
        // bo'sh qoldiramiz (brauzer rad etadi).
        if hmap.contains_key("access-control-allow-origin") {
            set_header(hmap, "access-control-allow-methods", &cfg.methods);
            set_header(hmap, "access-control-allow-headers", &cfg.headers);
            set_header(hmap, "access-control-max-age", &cfg.max_age.to_string());
        }
    }
    b.body(Full::new(Bytes::new())).unwrap()
}

// --- HTTP klient: http.get/post/put/del ---

// Request body hozir sodda bytes buffer: alias client tipini o'qilishi oson qiladi.
type ClientBody = Full<Bytes>;
// HttpsConnector<HttpConnector> ham http:// ham https:// ni boshqaradi — TLS
// faqat https sxemada faollashadi, plaintext so'rovlar avvalgidek ishlaydi.
type PooledHttpClient = Client<HttpsConnector<HttpConnector>, ClientBody>;

// Klient so'rovlari uchun bir martalik global runtime (Fluxon skripti sinxron).
// pub(crate): `ai` battery ham shu runtime/poolni qayta ishlatadi (LLM API ham
// oddiy https POST), takror tokio runtime/pool qurmaslik uchun.
pub(crate) fn client_runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("klient tokio runtime")
    })
}

// Hyper client ichida connection pool bor; global saqlab, clone() orqali
// requestlar orasida bitta poolni qayta ishlatamiz.
// pub(crate): `ai` battery ham shu poolni qayta ishlatadi.
pub(crate) fn pooled_http_client() -> PooledHttpClient {
    static CLIENT: OnceLock<PooledHttpClient> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            // webpki-roots ildizlari bilan https connector quramiz. enable_http1
            // hyper 1.x http1 klientiga mos. enable_http() http URL'larni ham
            // o'tkazadi (https-only emas) — shu sabab plaintext so'rovlar saqlanadi.
            let https = hyper_rustls::HttpsConnectorBuilder::new()
                .with_webpki_roots()
                .https_or_http()
                .enable_http1()
                .build();
            Client::builder(TokioExecutor::new()).build(https)
        })
        .clone()
}

// Klient so'rovi opsiyalari (oxirgi map argumentdan o'qiladi).
// follow=true → 3xx redirectni Location bo'yicha avtomat kuzatadi (default off).
// max → redirect hop limiti (default 10), undan oshsa xato.
// headers → so'rovga qo'shiladigan custom request header'lar (x-api-key,
// Authorization, anthropic-version...). req.headers/res.headers bilan simmetrik.
struct ClientOpts {
    follow: bool,
    max: i64,
    headers: BTreeMap<String, String>,
    // So'rov timeout'i: Some(dur) — shu muddatda tugamasa xato; None — timeout'siz
    // (`timeout: 0`). Default Some(30s) (issue #92).
    timeout: Option<Duration>,
}

impl Default for ClientOpts {
    fn default() -> Self {
        ClientOpts {
            follow: false,
            max: 10,
            headers: BTreeMap::new(),
            timeout: Some(Duration::from_secs(DEFAULT_CLIENT_TIMEOUT_SECS)),
        }
    }
}

// Opsiya map'ini o'qiydi. follow truthy bo'lsa kuzatish yoqiladi.
fn parse_client_opts(opts: Option<&Value>) -> ClientOpts {
    let mut o = ClientOpts::default();
    if let Some(Value::Map(m)) = opts {
        if let Some(v) = m.get("follow") {
            o.follow = !matches!(v, Value::Nil | Value::Bool(false));
        }
        if let Some(Value::Int(n)) = m.get("max") {
            o.max = *n;
        }
        // timeout: N (soniya) — so'rov shu muddatda tugamasa xato. 0 yoki manfiy —
        // timeout'siz (None). Boshqa qiymat turlari e'tiborsiz qoladi (default 30s).
        if let Some(Value::Int(n)) = m.get("timeout") {
            o.timeout = if *n > 0 {
                Some(Duration::from_secs(*n as u64))
            } else {
                None
            };
        }
        // headers: {kalit: qiymat} — har bir juftlikni str'ga aylantirib olamiz.
        // Kalit asl holida saqlanadi (HTTP header nomi katta-kichik harfga
        // sezgir emas, lekin foydalanuvchi yozganini buzmaymiz). Qiymat str
        // bo'lmasa ham (masalan int) matn ko'rinishiga aylantiriladi.
        if let Some(Value::Map(hm)) = m.get("headers") {
            for (k, v) in hm {
                let val = match v {
                    Value::Str(s) => s.clone(),
                    Value::Nil => continue, // nil header — tashlab ketamiz
                    other => format!("{}", other),
                };
                o.headers.insert(k.clone(), val);
            }
        }
    }
    o
}

// http.get url [opts]  /  http.post url body [opts]
// has_body=true bo'lsa args[1]=body, opts=args[2]; aks holda opts=args[1].
fn http_client(method: &str, args: Vec<Value>, has_body: bool) -> Result<Value, Flow> {
    let url = match args.first() {
        Some(Value::Str(s)) => s.clone(),
        _ => {
            return Err(Flow::err(format!(
                "http.{}: url (str) is required",
                method.to_lowercase()
            )));
        }
    };
    let (body, opts_arg) = if has_body {
        (args.get(1).cloned(), args.get(2))
    } else {
        (None, args.get(1))
    };
    let opts = parse_client_opts(opts_arg);

    // So'rov tanasini bir marta tayyorlaymiz (redirect'larda ham qayta ishlatamiz).
    // bytes body xom holida ketadi (issue #132) — shuning uchun String emas, Bytes.
    let (body_payload, is_json) = match &body {
        Some(Value::Map(_)) | Some(Value::List(_)) => {
            (Bytes::from(json_encode(body.as_ref().unwrap())), true)
        }
        Some(Value::Str(s)) => (Bytes::from(s.clone()), false),
        Some(Value::Bytes(b)) => (Bytes::from(b.as_ref().clone()), false),
        Some(other) => (Bytes::from(format!("{}", other)), false),
        None => (Bytes::new(), false),
    };

    // Timeout opts'dan alohida olamiz (opts quyida async blokka ko'chiriladi).
    let timeout = opts.timeout;
    client_runtime().block_on(async move {
        // Butun so'rov mantig'i (redirect'lar bilan birga) — timeout uni qamraydi.
        let work = async move {
            let mut current = url;
            // method redirect'da o'zgarishi mumkin (303 va GET-aylantiruvchi 301/302).
            let mut cur_method = method.to_string();
            let mut hops: i64 = 0;
            // Asl so'rov origin'i (sxema, host, port). Redirect begona origin'ga
            // olib chiqsa credential header'lar yuborilmaydi (issue #96).
            let mut first_origin: Option<(String, String, u16)> = None;
            // Belgi yopishqoq: begona origin orqali asl host'ga qaytsa ham
            // credential tiklanmaydi (reqwest/curl bilan bir xil ehtiyotkorlik).
            let mut cross_origin = false;

            loop {
                let uri: hyper::Uri = current
                    .parse()
                    .map_err(|e| Flow::err(format!("invalid url: {}", e)))?;

                let this_origin = uri_origin(&uri);
                match &first_origin {
                    None => first_origin = Some(this_origin),
                    Some(o) if *o != this_origin => cross_origin = true,
                    _ => {}
                }

                // GET'ga aylangach tana yuborilmaydi.
                let send_body = cur_method != "GET" && cur_method != "DELETE";
                let mut builder = Request::builder().method(cur_method.as_str()).uri(uri);
                // Foydalanuvchi custom header'larini avval qo'shamiz. content-type'ni
                // foydalanuvchi o'zi bergan bo'lsa, avtomatik qiymat ustiga yozmaymiz.
                let mut has_user_ct = false;
                for (k, v) in &opts.headers {
                    // Cross-origin redirect: Authorization/x-api-key/Cookie begona
                    // host'ga sizib chiqmasin (issue #96).
                    if cross_origin && is_sensitive_header(k) {
                        continue;
                    }
                    if k.eq_ignore_ascii_case("content-type") {
                        has_user_ct = true;
                    }
                    builder = builder.header(k.as_str(), v.as_str());
                }
                if is_json && send_body && !has_user_ct {
                    builder = builder.header("content-type", "application/json");
                }
                let payload = if send_body {
                    body_payload.clone()
                } else {
                    Bytes::new()
                };
                let req = builder
                    .body(Full::new(payload))
                    .map_err(|e| Flow::err(format!("building request: {}", e)))?;

                let resp = pooled_http_client()
                    .request(req)
                    .await
                    .map_err(|e| Flow::err(format!("http request: {}", e)))?;

                let status = resp.status().as_u16();

                // Redirect kuzatuvi (opt-in). 3xx + Location bo'lsa keyingi hop'ga o'tamiz.
                if opts.follow
                    && (300..400).contains(&status)
                    && let Some(loc) = resp
                        .headers()
                        .get("location")
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string())
                {
                    hops += 1;
                    if hops > opts.max {
                        return Err(Flow::err(format!(
                            "redirect limit exceeded ({} hops)",
                            opts.max
                        )));
                    }
                    // Nisbiy Location'ni joriy URL asosida to'liq URL'ga aylantiramiz.
                    current = resolve_location(&current, &loc);
                    // 303 har doim GET; 301/302 amaliyotda GET'ga aylanadi (POST→GET).
                    // 307/308 metod va tanani saqlaydi.
                    if status == 303 || ((status == 301 || status == 302) && cur_method == "POST") {
                        cur_method = "GET".to_string();
                    }
                    // 3xx tanasi drain qilinsa hyper pool ulanishni qayta ishlata
                    // oladi (issue #96). Lekin drain redirect'ni qotirmasin (PR
                    // #144 revyu): faqat hajmi ma'lum va kichik bo'lsa, qisqa
                    // timeout ichida frame-ma-frame (bufersiz) o'qiymiz. Hajmi
                    // noma'lum (chunked/stream) yoki katta bo'lsa darhol drop —
                    // ulanish yopiladi, keyingi hop yangisini ochadi.
                    const REDIRECT_DRAIN_MAX: u64 = 64 * 1024;
                    let known_len = resp
                        .headers()
                        .get("content-length")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok());
                    if let Some(len) = known_len
                        && len <= REDIRECT_DRAIN_MAX
                    {
                        let drain = async {
                            let mut body = resp.into_body();
                            while let Some(frame) = body.frame().await {
                                if frame.is_err() {
                                    break;
                                }
                            }
                        };
                        // Sekin upstream e'lon qilingan kichik hajmni ham asta
                        // oqizishi mumkin — tugamasa ulanishni tashlab ketamiz.
                        let _ = tokio::time::timeout(Duration::from_millis(500), drain).await;
                    }
                    continue;
                }

                // Yakuniy javob — header, status, body'ni yig'amiz.
                let resp_is_json = resp
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.contains("application/json"))
                    .unwrap_or(false);

                // Header'lar: kalit kichik harf (defis saqlanadi — m[k] bilan
                // o'qiladi), takror nomlar birlashtiriladi (issue #101).
                let headers = headers_to_map(resp.headers());

                let bytes = resp
                    .into_body()
                    .collect()
                    .await
                    .map_err(|e| Flow::err(format!("reading response: {}", e)))?
                    .to_bytes();
                let resp_body = match String::from_utf8(bytes.to_vec()) {
                    Ok(text) if resp_is_json => json_decode(&text).unwrap_or(Value::Str(text)),
                    Ok(text) => Value::Str(text),
                    // UTF-8 bo'lmagan javob (rasm, arxiv) — bytes (issue #132).
                    // Avval lossy o'qish ikkilik ma'lumotni jim buzardi.
                    Err(e) => Value::Bytes(Arc::new(e.into_bytes())),
                };

                let mut m = BTreeMap::new();
                m.insert("status".to_string(), Value::Int(status as i64));
                m.insert("body".to_string(), resp_body);
                m.insert("headers".to_string(), Value::Map(headers));
                // follow yoqilgan bo'lsa nechta redirect bo'lganini ham qaytaramiz.
                if opts.follow {
                    m.insert("hops".to_string(), Value::Int(hops));
                }
                return Ok(Value::Map(m));
            }
        };

        // Timeout o'rnatilgan bo'lsa so'rovni unga o'raymiz; tugamasa aniq xato
        // (qotgan upstream butun thread'ni abadiy bloklamasin — issue #92).
        match timeout {
            Some(dur) => match tokio::time::timeout(dur, work).await {
                Ok(r) => r,
                Err(_) => Err(Flow::err(format!(
                    "http request timeout (no response within {} sec)",
                    dur.as_secs()
                ))),
            },
            None => work.await,
        }
    })
}

// Redirect Location'ini joriy URL asosida hal qiladi. Location to'liq URL bo'lsa
// (`http://...`) o'sha qaytadi; aks holda joriy URL'ning sxema+host'iga ulanadi
// (mutlaq yo'l `/x` yoki nisbiy yo'l).
fn resolve_location(base: &str, loc: &str) -> String {
    if loc.starts_with("http://") || loc.starts_with("https://") {
        return loc.to_string();
    }
    // base'dan sxema://host qismini ajratamiz. Query/fragment'ni avval kesamiz —
    // ulardagi `/` yo'l segmenti hisoblanmasin (masalan `?q=/z`, issue #96).
    let scheme_end = base.find("://").map(|i| i + 3).unwrap_or(0);
    let base_end = base[scheme_end..]
        .find(['?', '#'])
        .map(|i| scheme_end + i)
        .unwrap_or(base.len());
    let base = &base[..base_end];
    // Sxema-nisbiy `//host/yo'l` — base sxemasi saqlanadi, qolgani Location'dan.
    if let Some(rest) = loc.strip_prefix("//") {
        let scheme = if scheme_end >= 3 {
            &base[..scheme_end - 2]
        } else {
            "http:"
        };
        return format!("{}//{}", scheme, rest);
    }
    let host_end = base[scheme_end..]
        .find('/')
        .map(|i| scheme_end + i)
        .unwrap_or(base.len());
    let origin = &base[..host_end];
    if loc.starts_with('/') {
        format!("{}{}", origin, loc)
    } else {
        // nisbiy yo'l: joriy yo'lning oxirgi segmentini almashtiramiz. Yo'l
        // umuman bo'lmasa root deb qaraladi — `/` qo'shiladi (issue #96:
        // ilgari "http://a.com" + "page" → "http://a.compage" chiqardi).
        let path_part = &base[host_end..];
        match path_part.rfind('/') {
            Some(i) => format!("{}{}", &base[..host_end + i + 1], loc),
            None => format!("{}/{}", origin, loc),
        }
    }
}

// Origin (sxema, host, port) uchligi — redirect host/port/sxemani o'zgartirganini
// aniqlash uchun. Port berilmagan bo'lsa sxema standarti olinadi (http=80,
// https=443): `http://a.com` va `http://a.com:80` bir origin.
fn uri_origin(uri: &hyper::Uri) -> (String, String, u16) {
    let scheme = uri.scheme_str().unwrap_or("http").to_ascii_lowercase();
    let host = uri.host().unwrap_or("").to_ascii_lowercase();
    let port = uri
        .port_u16()
        .unwrap_or(if scheme == "https" { 443 } else { 80 });
    (scheme, host, port)
}

// Cross-origin redirect'da tushirib yuboriladigan credential header'lar —
// curl/reqwest xulqi bilan bir xil: begona host API kalit/sessiyani ko'rmasin.
fn is_sensitive_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("authorization")
        || name.eq_ignore_ascii_case("proxy-authorization")
        || name.eq_ignore_ascii_case("cookie")
        || name.eq_ignore_ascii_case("x-api-key")
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- headers_to_map: o'qish tomonida takror header'lar (issue #101) ---

    // Map'dan str qiymatni oladi (Value Debug/PartialEq emas — pattern bilan).
    fn hstr(m: &BTreeMap<String, Value>, k: &str) -> String {
        match m.get(k) {
            Some(Value::Str(s)) => s.clone(),
            _ => panic!("{k}: Str value expected"),
        }
    }

    #[test]
    fn headers_takror_nom_vergul_bilan_birlashadi() {
        // Bir nomli ikki header (masalan X-Forwarded-For zanjiri) yo'qolmasin —
        // RFC 9110 §5.3 bo'yicha ", " bilan bitta qiymatga birlashadi.
        let mut h = hyper::HeaderMap::new();
        h.append("x-forwarded-for", "1.1.1.1".parse().unwrap());
        h.append("x-forwarded-for", "2.2.2.2".parse().unwrap());
        let m = headers_to_map(&h);
        assert_eq!(hstr(&m, "x-forwarded-for"), "1.1.1.1, 2.2.2.2");
    }

    // --- bytes (issue #132): ikkilik tana so'rov/javob yo'llarida ---

    // UTF-8 bo'lmagan so'rov tanasi bytes bo'lib keladi (avval lossy buzilardi).
    #[test]
    fn build_req_ikkilik_tana_bytes() {
        let req = build_req(
            "POST".into(),
            "/upload".into(),
            String::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            "1.1.1.1".into(),
            Bytes::from(vec![0xff, 0xfe, 0x00]),
            false,
            None,
        );
        let Value::Map(m) = req else {
            panic!("req map expected");
        };
        match m.get("body") {
            Some(Value::Bytes(b)) => assert_eq!(**b, vec![0xff, 0xfe, 0x00]),
            _ => panic!("binary body must be bytes"),
        }
    }

    // Matnli tana avvalgidek str (regressiya himoyasi).
    #[test]
    fn build_req_matn_tana_str_qoladi() {
        let req = build_req(
            "POST".into(),
            "/t".into(),
            String::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            "1.1.1.1".into(),
            Bytes::from("hello"),
            false,
            None,
        );
        let Value::Map(m) = req else {
            panic!("req map expected");
        };
        match m.get("body") {
            Some(Value::Str(s)) => assert_eq!(s, "hello"),
            _ => panic!("text body must be str"),
        }
    }

    // --- multipart/form-data (issue #133) ---

    // Brauzer/curl yuboradigan tipik multipart tana yasaydi.
    fn multipart_body(boundary: &str, parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, filename, content) in parts {
            out.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
            match filename {
                Some(f) => out.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n",
                        name, f
                    )
                    .as_bytes(),
                ),
                None => out.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{}\"\r\n", name).as_bytes(),
                ),
            }
            out.extend_from_slice(b"\r\n");
            out.extend_from_slice(content);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());
        out
    }

    #[test]
    fn multipart_boundary_oddiy_va_qoshtirnoqli() {
        // Oddiy va qo'shtirnoqli boundary'lar ham parse bo'ladi; boshqa
        // content-type (JSON) None qaytaradi.
        assert_eq!(
            multipart_boundary("multipart/form-data; boundary=----WebKit123"),
            Some("----WebKit123".to_string())
        );
        assert_eq!(
            multipart_boundary("multipart/form-data; boundary=\"abc def\""),
            Some("abc def".to_string())
        );
        assert_eq!(multipart_boundary("application/json"), None);
        assert_eq!(multipart_boundary("multipart/form-data"), None);
    }

    #[test]
    fn cd_param_filename_ichidagi_name_adashtirmaydi() {
        // "filename=" qidiruvda "name=" ga mos kelmasin — ajratuvchi tekshiriladi.
        let line = "Content-Disposition: form-data; name=\"avatar\"; filename=\"a.png\"";
        assert_eq!(cd_param(line, "name").as_deref(), Some("avatar"));
        assert_eq!(cd_param(line, "filename").as_deref(), Some("a.png"));
        // filename yo'q qism — oddiy maydon.
        let field = "Content-Disposition: form-data; name=\"title\"";
        assert_eq!(cd_param(field, "name").as_deref(), Some("title"));
        assert_eq!(cd_param(field, "filename"), None);
    }

    #[test]
    fn parse_multipart_maydon_va_fayl() {
        // Oddiy maydon req.body'ga, fayl (filename bor) files ro'yxatiga tushadi.
        let body = multipart_body(
            "BB",
            &[
                ("title", None, b"hello world"),
                ("doc", Some("a.txt"), b"text file"),
            ],
        );
        let (fields, files) = parse_multipart(&body, "BB").expect("parse must succeed");
        match fields.get("title") {
            Some(Value::Str(s)) => assert_eq!(s, "hello world"),
            _ => panic!("title must be str"),
        }
        assert_eq!(files.len(), 1);
        let Value::Map(f) = &files[0] else {
            panic!("file map expected");
        };
        assert!(matches!(f.get("name"), Some(Value::Str(s)) if s == "doc"));
        assert!(matches!(f.get("filename"), Some(Value::Str(s)) if s == "a.txt"));
        assert!(matches!(f.get("content"), Some(Value::Str(s)) if s == "text file"));
        assert!(matches!(f.get("size"), Some(Value::Int(9))));
    }

    #[test]
    fn parse_multipart_ikkilik_fayl_bytes() {
        // Ikkilik mazmun (UTF-8 emas, ichida CRLF ham bor) bytes bo'lib keladi
        // va baytlar aynan saqlanadi; size — bayt soni.
        let data: &[u8] = &[0xff, 0xd8, b'\r', b'\n', 0x00, 0xfe];
        let body = multipart_body("XX", &[("img", Some("a.jpg"), data)]);
        let (_, files) = parse_multipart(&body, "XX").expect("parse must succeed");
        let Value::Map(f) = &files[0] else {
            panic!("file map expected");
        };
        match f.get("content") {
            Some(Value::Bytes(b)) => assert_eq!(**b, data.to_vec()),
            _ => panic!("binary content must be bytes"),
        }
        assert!(matches!(f.get("size"), Some(Value::Int(6))));
    }

    #[test]
    fn parse_multipart_bir_nom_bir_nechta_fayl() {
        // Bir xil name bilan bir nechta fayl (`<input multiple>`) — hammasi
        // ro'yxatda qoladi (map emas, list bo'lgani uchun yo'qolmaydi).
        let body = multipart_body(
            "MM",
            &[
                ("docs", Some("1.txt"), b"bir"),
                ("docs", Some("2.txt"), b"ikki"),
            ],
        );
        let (_, files) = parse_multipart(&body, "MM").expect("parse must succeed");
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn parse_multipart_mazmundagi_boundary_prefiksi_kesmaydi() {
        // Fayl mazmunida `\r\n--abcXYZ` bor (boundary `abc` ning prefiksi, lekin
        // to'liq chegara qatori emas) — qism KESILMAY butun saqlanishi kerak
        // (codex P2 revyu: faqat `\r\n--boundary` qidirish mazmunni buzardi).
        let data: &[u8] = b"first\r\n--abcXYZ\r\nremaining part";
        let body = multipart_body("abc", &[("doc", Some("a.txt"), data)]);
        let (_, files) = parse_multipart(&body, "abc").expect("parse must succeed");
        assert_eq!(files.len(), 1);
        let Value::Map(f) = &files[0] else {
            panic!("file map expected");
        };
        match f.get("content") {
            Some(Value::Str(s)) => assert_eq!(s.as_bytes(), data),
            _ => panic!("content must be a whole str"),
        }
        assert!(matches!(f.get("size"), Some(Value::Int(n)) if *n == data.len() as i64));
    }

    #[test]
    fn parse_multipart_padding_bilan_boundary_qabul() {
        // RFC 2046: boundary qatoridan keyin transport padding (bo'shliq/tab)
        // bo'lishi mumkin — bunday chegara haqiqiy deb olinadi.
        let mut body = Vec::new();
        body.extend_from_slice(b"--PP  \r\n");
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"a\"\r\n\r\n");
        body.extend_from_slice(b"value");
        body.extend_from_slice(b"\r\n--PP--\r\n");
        let (fields, _) = parse_multipart(&body, "PP").expect("parse must succeed");
        assert!(matches!(fields.get("a"), Some(Value::Str(s)) if s == "value"));
    }

    #[test]
    fn parse_multipart_buzuq_tana_none() {
        // Boundary tanada umuman yo'q — None, chaqiruvchi xom body'ga qaytadi.
        assert!(parse_multipart(b"just text", "NONE").is_none());
    }

    #[test]
    fn build_req_multipart_body_va_files() {
        // To'liq yo'l: boundary berilganda req.body maydonlar map'i, req.files
        // fayllar ro'yxati bo'ladi.
        let body = multipart_body(
            "ZZ",
            &[("title", None, b"my image"), ("pic", Some("p.png"), b"PNG")],
        );
        let req = build_req(
            "POST".into(),
            "/upload".into(),
            String::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            "1.1.1.1".into(),
            Bytes::from(body),
            false,
            Some("ZZ".to_string()),
        );
        let Value::Map(m) = req else {
            panic!("req map expected");
        };
        let Some(Value::Map(b)) = m.get("body") else {
            panic!("body map expected");
        };
        assert!(matches!(b.get("title"), Some(Value::Str(s)) if s == "my image"));
        match m.get("files") {
            Some(Value::List(fs)) => assert_eq!(fs.len(), 1),
            _ => panic!("files must be a list"),
        }
    }

    #[test]
    fn build_req_multipart_emas_files_bosh_list() {
        // Oddiy so'rovda ham req.files mavjud (bo'sh list) — `each` nil
        // tekshiruvisiz ishlaydi.
        let req = build_req(
            "POST".into(),
            "/t".into(),
            String::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            "1.1.1.1".into(),
            Bytes::from("{\"a\":1}"),
            true,
            None,
        );
        let Value::Map(m) = req else {
            panic!("req map expected");
        };
        match m.get("files") {
            Some(Value::List(fs)) => assert!(fs.is_empty()),
            _ => panic!("files must be an empty list"),
        }
    }

    #[test]
    fn build_req_multipart_buzuq_xom_qoladi() {
        // Boundary bor lekin tana mos emas — parse None, body xom str qoladi
        // (ma'lumot jim yo'qolmaydi), files bo'sh.
        let req = build_req(
            "POST".into(),
            "/u".into(),
            String::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            "1.1.1.1".into(),
            Bytes::from("plain text"),
            false,
            Some("QQ".to_string()),
        );
        let Value::Map(m) = req else {
            panic!("req map expected");
        };
        assert!(matches!(m.get("body"), Some(Value::Str(s)) if s == "plain text"));
        match m.get("files") {
            Some(Value::List(fs)) => assert!(fs.is_empty()),
            _ => panic!("files must be an empty list"),
        }
    }

    // bytes javob — xom baytlar + application/octet-stream standart turi.
    #[test]
    fn bytes_javob_octet_stream() {
        let resp = body_value_to_response(200, Value::Bytes(Arc::new(vec![1, 2, 3])));
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/octet-stream"
        );
    }

    #[test]
    fn headers_bitta_qiymat_oddiy_str() {
        let mut h = hyper::HeaderMap::new();
        h.insert("content-type", "application/json".parse().unwrap());
        let m = headers_to_map(&h);
        assert_eq!(hstr(&m, "content-type"), "application/json");
    }

    #[test]
    fn headers_cookie_nuqta_vergul_bilan_birlashadi() {
        // Cookie-pair ajratkichi "; " (RFC 6265) — vergul bilan birlashtirsak
        // cookie qiymati buziladi.
        let mut h = hyper::HeaderMap::new();
        h.append("cookie", "a=1".parse().unwrap());
        h.append("cookie", "b=2".parse().unwrap());
        let m = headers_to_map(&h);
        assert_eq!(hstr(&m, "cookie"), "a=1; b=2");
    }

    #[test]
    fn headers_takror_set_cookie_list_qaytadi() {
        // Set-Cookie birlashtirib bo'lmaydi (Expires sanasida vergul bor) —
        // takror bo'lsa List, yozish tomonidagi List bilan simmetrik.
        let mut h = hyper::HeaderMap::new();
        h.append("set-cookie", "a=1".parse().unwrap());
        h.append("set-cookie", "b=2".parse().unwrap());
        let m = headers_to_map(&h);
        match m.get("set-cookie") {
            Some(Value::List(items)) => {
                let got: Vec<String> = items.iter().map(|v| v.to_text()).collect();
                assert_eq!(got, vec!["a=1", "b=2"]);
            }
            _ => panic!("set-cookie: List expected"),
        }
    }

    #[test]
    fn headers_bitta_set_cookie_str_qoladi() {
        // Oddiy holat (bitta cookie) o'zgarmasin — eski kod str kutadi.
        let mut h = hyper::HeaderMap::new();
        h.insert("set-cookie", "s=xyz".parse().unwrap());
        let m = headers_to_map(&h);
        assert_eq!(hstr(&m, "set-cookie"), "s=xyz");
    }

    #[test]
    fn headers_utf8_bolmagan_qiymat_lossy_oqiladi() {
        // Oldin unwrap_or("") jim bo'sh string qaytarardi — endi lossy: buzuq
        // bayt U+FFFD bo'ladi, qolgan qism saqlanadi.
        let mut h = hyper::HeaderMap::new();
        h.insert(
            "x-raw",
            hyper::header::HeaderValue::from_bytes(b"ok\xffend").unwrap(),
        );
        let m = headers_to_map(&h);
        assert_eq!(hstr(&m, "x-raw"), "ok\u{fffd}end");
    }

    #[test]
    fn location_absolute_url() {
        // To'liq URL bo'lsa o'zi qaytadi (base e'tiborga olinmaydi).
        let got = resolve_location("http://a.com/x", "http://b.com/y");
        assert_eq!(got, "http://b.com/y");
    }

    #[test]
    fn location_root_relative() {
        // `/...` mutlaq yo'l — base'ning origin'iga ulanadi, yo'l almashtiriladi.
        let got = resolve_location("http://a.com/old/path", "/new");
        assert_eq!(got, "http://a.com/new");
    }

    #[test]
    fn location_relative_path() {
        // nisbiy yo'l — joriy yo'lning oxirgi segmenti o'rniga qo'yiladi.
        let got = resolve_location("http://a.com/dir/file", "other");
        assert_eq!(got, "http://a.com/dir/other");
    }

    #[test]
    fn location_relative_at_root() {
        // host'dan keyin yo'l yo'q bo'lsa root deb qaraladi — `/` qo'shiladi
        // (issue #96: ilgari "http://a.compage" degan buzuq URL chiqardi).
        let got = resolve_location("http://a.com", "page");
        assert_eq!(got, "http://a.com/page");
    }

    #[test]
    fn location_base_query_kesiladi() {
        // Base query'sidagi `/` yo'l segmenti emas (issue #96) — nisbiy yo'l
        // query'dan oldingi haqiqiy yo'lga nisbatan hal qilinadi.
        let got = resolve_location("http://a.com/search?q=/z", "next");
        assert_eq!(got, "http://a.com/next");
        // Mutlaq yo'lda ham query origin'ni buzmaydi.
        let got2 = resolve_location("http://a.com/a/b?x=1", "/new");
        assert_eq!(got2, "http://a.com/new");
    }

    #[test]
    fn location_scheme_relative() {
        // `//host/yo'l` — sxema base'dan olinadi (https saqlanadi).
        let got = resolve_location("https://a.com/x", "//b.com/y");
        assert_eq!(got, "https://b.com/y");
    }

    #[test]
    fn origin_default_port_va_case() {
        // Standart port yozilgan-yozilmagani va harf katta-kichikligi farq qilmaydi.
        let a: hyper::Uri = "http://A.com/x".parse().unwrap();
        let b: hyper::Uri = "http://a.com:80/y".parse().unwrap();
        assert_eq!(uri_origin(&a), uri_origin(&b));
        // Sxema yoki port farqi — boshqa origin (credential ketmasligi kerak).
        let c: hyper::Uri = "https://a.com/x".parse().unwrap();
        let d: hyper::Uri = "http://a.com:8080/x".parse().unwrap();
        assert_ne!(uri_origin(&a), uri_origin(&c));
        assert_ne!(uri_origin(&a), uri_origin(&d));
    }

    #[test]
    fn sensitive_header_royxati() {
        // Credential header'lar case-insensitive taniladi; oddiy header emas.
        assert!(is_sensitive_header("Authorization"));
        assert!(is_sensitive_header("X-API-Key"));
        assert!(is_sensitive_header("cookie"));
        assert!(is_sensitive_header("Proxy-Authorization"));
        assert!(!is_sensitive_header("content-type"));
        assert!(!is_sensitive_header("x-request-id"));
    }

    // Mini HTTP server: `responses` dagi har bir javob uchun bitta ulanish qabul
    // qiladi va kelgan so'rov matnini qayd etadi. `Connection: close` bilan javob
    // berilishi shart — har hop yangi ulanishda kelib, qaydlar deterministik bo'ladi.
    fn spawn_test_server(responses: Vec<String>) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let mut captured = Vec::new();
            for resp in responses {
                let (mut sock, _) = listener.accept().unwrap();
                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                loop {
                    let n = sock.read(&mut tmp).unwrap();
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                captured.push(String::from_utf8_lossy(&buf).to_string());
                sock.write_all(resp.as_bytes()).unwrap();
            }
            captured
        });
        (port, handle)
    }

    // follow:true + credential header'lar bilan GET so'rovini quradi.
    fn follow_get_with_credentials(url: String) -> Result<Value, Flow> {
        let mut headers = BTreeMap::new();
        headers.insert(
            "authorization".to_string(),
            Value::Str("Bearer secret".into()),
        );
        headers.insert("x-api-key".to_string(), Value::Str("key".into()));
        headers.insert("x-custom".to_string(), Value::Str("stays".into()));
        let mut opts = BTreeMap::new();
        opts.insert("follow".to_string(), Value::Bool(true));
        opts.insert("headers".to_string(), Value::Map(headers));
        http_client("GET", vec![Value::Str(url), Value::Map(opts)], false)
    }

    #[test]
    fn cross_origin_redirect_credential_tushiriladi() {
        // issue #96: begona origin'ga (boshqa port) redirect — Authorization va
        // x-api-key u yerga yetib bormasligi kerak, oddiy header esa qoladi.
        let (port_b, hb) = spawn_test_server(vec![
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok".to_string(),
        ]);
        let (port_a, ha) = spawn_test_server(vec![format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/dest\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            port_b
        )]);

        let Ok(Value::Map(res)) =
            follow_get_with_credentials(format!("http://127.0.0.1:{}/start", port_a))
        else {
            panic!("request must have succeeded");
        };
        assert!(matches!(res.get("status"), Some(Value::Int(200))));

        // Birinchi host (asl origin) credential'larni to'liq oladi.
        let req_a = ha.join().unwrap().remove(0).to_lowercase();
        assert!(req_a.contains("authorization: bearer secret"));
        assert!(req_a.contains("x-api-key: key"));
        // Begona host'ga credential'lar ketmaydi, oddiy header esa boradi.
        let req_b = hb.join().unwrap().remove(0).to_lowercase();
        assert!(!req_b.contains("authorization"), "Authorization leaked");
        assert!(!req_b.contains("x-api-key"), "x-api-key leaked");
        assert!(req_b.contains("x-custom: stays"));
    }

    #[test]
    fn same_origin_redirect_credential_saqlanadi() {
        // Bir xil origin ichidagi redirect'da credential'lar tushirilmaydi —
        // fix faqat begona host'ga ta'sir qiladi.
        let (port, h) = spawn_test_server(vec![
            "HTTP/1.1 302 Found\r\nLocation: /dest\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
                .to_string(),
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok".to_string(),
        ]);

        let Ok(Value::Map(res)) =
            follow_get_with_credentials(format!("http://127.0.0.1:{}/start", port))
        else {
            panic!("request must have succeeded");
        };
        assert!(matches!(res.get("status"), Some(Value::Int(200))));

        let captured = h.join().unwrap();
        let req2 = captured[1].to_lowercase();
        assert!(req2.contains("authorization: bearer secret"));
        assert!(req2.contains("x-api-key: key"));
    }

    #[test]
    fn https_connector_quriladi() {
        // pooled_http_client https connectorni panic'siz quradi (rustls ring
        // crypto provayder mavjud, webpki-roots yuklanadi). Bu tarmoqsiz ham
        // ishlaydigan deterministik tekshiruv — HTTPS yo'lining qurilishini
        // himoyalaydi (issue #14: faqat http:// emas, https:// ham).
        let _client = pooled_http_client();
        // clone() bir poolni qayta ishlatadi — yana panic bo'lmasin.
        let _client2 = pooled_http_client();
    }

    #[test]
    fn opts_default_no_follow() {
        // Opsiya berilmasa redirect kuzatilmaydi, limit 10.
        let o = parse_client_opts(None);
        assert!(!o.follow);
        assert_eq!(o.max, 10);
    }

    #[test]
    fn opts_follow_true() {
        let mut m = BTreeMap::new();
        m.insert("follow".to_string(), Value::Bool(true));
        m.insert("max".to_string(), Value::Int(3));
        let o = parse_client_opts(Some(&Value::Map(m)));
        assert!(o.follow);
        assert_eq!(o.max, 3);
    }

    #[test]
    fn opts_follow_falsey() {
        // follow:false va follow:nil — ikkalasi ham kuzatishni yoqmaydi.
        let mut m = BTreeMap::new();
        m.insert("follow".to_string(), Value::Bool(false));
        assert!(!parse_client_opts(Some(&Value::Map(m))).follow);
    }

    #[test]
    fn opts_headers_parse_qiladi() {
        // headers map'i str qiymatlar bilan o'qiladi (issue #34).
        let mut hm = BTreeMap::new();
        hm.insert(
            "x-api-key".to_string(),
            Value::Str("secret-val".to_string()),
        );
        hm.insert(
            "anthropic-version".to_string(),
            Value::Str("2023-06-01".to_string()),
        );
        let mut m = BTreeMap::new();
        m.insert("headers".to_string(), Value::Map(hm));
        let o = parse_client_opts(Some(&Value::Map(m)));
        assert_eq!(
            o.headers.get("x-api-key").map(|s| s.as_str()),
            Some("secret-val")
        );
        assert_eq!(
            o.headers.get("anthropic-version").map(|s| s.as_str()),
            Some("2023-06-01")
        );
    }

    #[test]
    fn opts_headers_str_bolmagan_qiymat_matnga_aylanadi() {
        // str bo'lmagan qiymat (int) matn ko'rinishiga aylantiriladi; nil tashlanadi.
        let mut hm = BTreeMap::new();
        hm.insert("x-count".to_string(), Value::Int(42));
        hm.insert("x-skip".to_string(), Value::Nil);
        let mut m = BTreeMap::new();
        m.insert("headers".to_string(), Value::Map(hm));
        let o = parse_client_opts(Some(&Value::Map(m)));
        assert_eq!(o.headers.get("x-count").map(|s| s.as_str()), Some("42"));
        assert!(!o.headers.contains_key("x-skip"));
    }

    #[test]
    fn opts_default_headers_bosh() {
        // Opsiya berilmasa header'lar bo'sh.
        assert!(parse_client_opts(None).headers.is_empty());
    }

    // --- klient timeout (issue #92) ---

    #[test]
    fn opts_default_timeout_30s() {
        // Opsiya berilmasa default 30s timeout (qotgan upstream'ga qarshi himoya).
        let o = parse_client_opts(None);
        assert_eq!(
            o.timeout,
            Some(Duration::from_secs(DEFAULT_CLIENT_TIMEOUT_SECS))
        );
    }

    #[test]
    fn opts_timeout_sozlanadi() {
        // `{timeout: N}` — N soniya.
        let mut m = BTreeMap::new();
        m.insert("timeout".to_string(), Value::Int(5));
        let o = parse_client_opts(Some(&Value::Map(m)));
        assert_eq!(o.timeout, Some(Duration::from_secs(5)));
    }

    #[test]
    fn opts_timeout_nol_ochiradi() {
        // `timeout: 0` (va manfiy) — timeout'siz (None). Faqat ishonchli upstream uchun.
        let mut m = BTreeMap::new();
        m.insert("timeout".to_string(), Value::Int(0));
        assert_eq!(parse_client_opts(Some(&Value::Map(m))).timeout, None);
        let mut m2 = BTreeMap::new();
        m2.insert("timeout".to_string(), Value::Int(-1));
        assert_eq!(parse_client_opts(Some(&Value::Map(m2))).timeout, None);
    }

    #[test]
    fn http_get_qotgan_upstream_timeout_qaytaradi() {
        // Acceptance (issue #92): ulanishni qabul qilib JAVOB BERMAYDIGAN upstream
        // butun thread'ni abadiy bloklamasligi kerak — qisqa timeout bilan xato
        // qaytishi shart. Listener'ni ochamiz, ulanishni qabul qilamiz, lekin hech
        // narsa yozmaymiz (slow/qotgan server taqlidi).
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            // Ulanishni ushlab turamiz, javob yubormaymiz.
            for stream in listener.incoming() {
                let _held = stream;
                std::thread::sleep(Duration::from_secs(10));
            }
        });
        let mut opts = BTreeMap::new();
        opts.insert("timeout".to_string(), Value::Int(1));
        let url = format!("http://{}/", addr);
        let res = http_client("GET", vec![Value::Str(url), Value::Map(opts)], false);
        match res {
            Err(Flow::Error(msg)) => {
                assert!(
                    msg.contains("timeout"),
                    "timeout error expected, got: {}",
                    msg
                )
            }
            Ok(_) => panic!("Ok not expected from a stuck upstream — must be a timeout"),
            Err(_) => panic!("Flow::Error(timeout) expected"),
        }
    }

    // build_req'dan body Value'ni ajratib oluvchi yordamchi.
    fn body_of(bytes: &str, is_json: bool) -> Value {
        let v = build_req(
            "POST".into(),
            "/".into(),
            String::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            "127.0.0.1".into(),
            Bytes::from(bytes.to_string()),
            is_json,
            None,
        );
        match v {
            Value::Map(m) => m.get("body").cloned().unwrap(),
            _ => panic!("build_req must return a Map"),
        }
    }

    #[test]
    fn bosh_tana_nil() {
        assert!(matches!(body_of("", false), Value::Nil));
    }

    #[test]
    fn content_type_json_parse_qiladi() {
        // Content-Type JSON bo'lsa (eski xulq saqlangan).
        assert!(matches!(body_of(r#"{"a":1}"#, true), Value::Map(_)));
    }

    #[test]
    fn content_type_yoq_lekin_obyekt_korinishida_parse_qiladi() {
        // Asosiy tuzatish: Content-Type JSON bo'lmasa ham `{` bilan boshlansa parse.
        assert!(matches!(body_of(r#"{"a":1}"#, false), Value::Map(_)));
    }

    #[test]
    fn content_type_yoq_lekin_royxat_korinishida_parse_qiladi() {
        // `[` bilan boshlangan tana ham JSON deb urinish.
        assert!(matches!(body_of("[1,2,3]", false), Value::List(_)));
    }

    #[test]
    fn boshidagi_boshliq_belgi_eotiborga_olinadi() {
        // Old whitespace bo'lsa ham `{` aniqlanadi.
        assert!(matches!(body_of("  \n {\"a\":1}", false), Value::Map(_)));
    }

    #[test]
    fn oddiy_matn_string_boladi() {
        // JSON ko'rinishida bo'lmagan tana string bo'lib qoladi.
        assert!(matches!(body_of("hello=world", false), Value::Str(_)));
    }

    #[test]
    fn buzilgan_json_xom_matn_qoladi() {
        // `{` bilan boshlanadi, lekin yaroqsiz JSON — string sifatida qoladi.
        assert!(matches!(body_of("{buzuq", false), Value::Str(_)));
    }

    // --- middleware prefiks mosligi (issue #67) ---

    #[test]
    fn prefix_yulduz_aniq_prefiks_mos() {
        // "/api/*" → "/api" ning o'zi ham, ostidagilar ham mos.
        assert!(prefix_matches("/api/*", "/api"));
        assert!(prefix_matches("/api/*", "/api/users"));
        assert!(prefix_matches("/api/*", "/api/v1/bookings"));
    }

    #[test]
    fn prefix_yulduz_segment_chegarasi() {
        // "/apix" "/api/*" ga MOS EMAS — prefiks segment chegarasida ajraladi
        // (aks holda "/api" boshqa resurslarga sizib ketardi).
        assert!(!prefix_matches("/api/*", "/apix"));
        assert!(!prefix_matches("/api/*", "/ap"));
        assert!(!prefix_matches("/api/*", "/"));
    }

    #[test]
    fn prefix_yulduzsiz_aniq_mos() {
        // "*"siz shablon — faqat aniq yo'l mosligi.
        assert!(prefix_matches("/api/v1/users", "/api/v1/users"));
        assert!(!prefix_matches("/api/v1/users", "/api/v1"));
        assert!(!prefix_matches("/api/v1/users", "/api/v1/users/5"));
    }

    // --- req.ctx shared cell (issue #68) ---

    #[test]
    fn with_ctx_shared_cell_qoshadi() {
        // with_ctx req map'iga "ctx" kalitini Value::Ctx sifatida qo'yadi.
        let cell = Arc::new(Mutex::new(BTreeMap::new()));
        let req = build_req(
            "GET".into(),
            "/".into(),
            String::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            "127.0.0.1".into(),
            Bytes::new(),
            false,
            None,
        );
        let req = with_ctx(req, cell.clone());
        let Value::Map(m) = &req else {
            panic!("req must be a Map");
        };
        assert!(matches!(m.get("ctx"), Some(Value::Ctx(_))));
    }

    #[test]
    fn ctx_cell_klon_orqali_ulashiladi() {
        // req klonlanganda ctx Arc ulashiladi — tashqaridan cell'ga yozsak,
        // klon orqali ham ko'rinadi (middleware->handler oqimi shu mexanizmga
        // tayanadi).
        let cell = Arc::new(Mutex::new(BTreeMap::new()));
        let req = with_ctx(
            build_req(
                "GET".into(),
                "/".into(),
                String::new(),
                BTreeMap::new(),
                BTreeMap::new(),
                "127.0.0.1".into(),
                Bytes::new(),
                false,
                None,
            ),
            cell.clone(),
        );
        let req_clone = req.clone();
        // Tashqaridan cell'ga yozamiz (middleware `req.ctx <-` shuni qiladi).
        cell.lock()
            .unwrap()
            .insert("tenant_id".to_string(), Value::Int(7));
        // Klon orqali o'qiganda yangi qiymat ko'rinadi (Arc ulashilgani isboti).
        let Value::Map(m) = &req_clone else {
            panic!("Map");
        };
        let Some(Value::Ctx(c)) = m.get("ctx") else {
            panic!("ctx cell");
        };
        // Value Debug derive qilmaydi — equals bilan tekshiramiz (assert_eq emas).
        let got = c.lock().unwrap().get("tenant_id").cloned().unwrap();
        assert!(got.equals(&Value::Int(7)), "ctx updated through the clone");
    }

    #[test]
    fn ctx_self_equals_deadlock_qilmaydi() {
        // `req == req` (yoki req klonini taqqoslash) — bir xil ctx Arc<Mutex>'ni
        // ikki tomondan ko'radi. equals ikki lock'ni birga ushlamasligi kerak,
        // aks holda non-reentrant mutex deadlock qiladi (Codex P2). Bu test
        // o'sha yo'lni kechiradi: bloklanса hang qiladi, aks holda darrov o'tadi.
        let cell = Arc::new(Mutex::new(BTreeMap::new()));
        let req = with_ctx(
            build_req(
                "GET".into(),
                "/".into(),
                String::new(),
                BTreeMap::new(),
                BTreeMap::new(),
                "127.0.0.1".into(),
                Bytes::new(),
                false,
                None,
            ),
            cell,
        );
        let req_clone = req.clone();
        // Map equality ctx kalitiga yetadi -> (Ctx,Ctx) bir xil Arc -> ptr_eq.
        assert!(req.equals(&req_clone), "req equals its clone, no deadlock");
        assert!(req.equals(&req), "req equals itself, no deadlock");
    }

    #[test]
    fn is_resp_rep_javobni_taniydi() {
        // rep -> {__resp:true ...}. Middleware shu javobni qaytarsa zanjir to'xtaydi.
        let mut m = BTreeMap::new();
        m.insert("__resp".to_string(), Value::Bool(true));
        m.insert("status".to_string(), Value::Int(401));
        assert!(is_resp(&Value::Map(m)));
        // Oddiy map yoki nil — javob emas (middleware davom etadi).
        assert!(!is_resp(&Value::Map(BTreeMap::new())));
        assert!(!is_resp(&Value::Nil));
    }

    // --- custom header'lar (issue #16) ---

    // `rep status body {headers}` natijasini taqlid qiluvchi __resp map.
    fn resp_map(status: i64, body: Value, headers: Option<Value>) -> Value {
        let mut m = BTreeMap::new();
        m.insert("__resp".to_string(), Value::Bool(true));
        m.insert("status".to_string(), Value::Int(status));
        m.insert("body".to_string(), body);
        if let Some(h) = headers {
            m.insert("headers".to_string(), h);
        }
        Value::Map(m)
    }

    fn hmap(pairs: &[(&str, Value)]) -> Value {
        let mut m = BTreeMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        Value::Map(m)
    }

    #[test]
    fn custom_content_type_body_standartini_bosadi() {
        // str body standart "text/plain" beradi; custom content-type uni bosadi.
        let r = value_to_response(resp_map(
            200,
            Value::Str("<h1>Hello</h1>".into()),
            Some(hmap(&[("content-type", Value::Str("text/html".into()))])),
        ));
        assert_eq!(r.headers().get("content-type").unwrap(), "text/html");
    }

    #[test]
    fn custom_header_nomi_lowercase_kanonik() {
        // Content-Type (katta harf) berilsa ham kichik harfda saqlanadi
        // (RFC 7230 — header nomi case-insensitive).
        let r = value_to_response(resp_map(
            200,
            Value::Nil,
            Some(hmap(&[("X-Request-Id", Value::Str("abc".into()))])),
        ));
        assert_eq!(r.headers().get("x-request-id").unwrap(), "abc");
    }

    #[test]
    fn set_cookie_list_takror_sarlavha() {
        // List qiymat → har element alohida Set-Cookie qatori (RFC 7230:
        // Set-Cookie vergulli ro'yxat bilan birlashmaydi).
        let cookies = Value::List(vec![Value::Str("a=1".into()), Value::Str("b=2".into())]);
        let r = value_to_response(resp_map(
            200,
            Value::Nil,
            Some(hmap(&[("set-cookie", cookies)])),
        ));
        let got: Vec<_> = r.headers().get_all("set-cookie").iter().collect();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], "a=1");
        assert_eq!(got[1], "b=2");
    }

    #[test]
    fn redirect_location_plus_custom_header() {
        // Eski `rep 302 {location:url}` xulqi + custom header birga ishlaydi
        // (masalan redirect bilan birga Set-Cookie o'rnatish).
        let r = value_to_response(resp_map(
            302,
            hmap(&[("location", Value::Str("/dest".into()))]),
            Some(hmap(&[("set-cookie", Value::Str("s=xyz".into()))])),
        ));
        assert_eq!(r.status().as_u16(), 302);
        assert_eq!(r.headers().get("location").unwrap(), "/dest");
        assert_eq!(r.headers().get("set-cookie").unwrap(), "s=xyz");
    }

    #[test]
    fn notogri_status_500_ga_tushadi() {
        // `rep 1000 ...` — yaroqsiz HTTP status. Jim 200 ga tushmasligi kerak
        // (issue #108): handler xato status qaytarganda mijoz muvaffaqiyat
        // ko'rmasin. 1000 HTTP diapazonidan tashqarida → 500.
        let r = value_to_response(resp_map(1000, Value::Str("error".into()), None));
        assert_eq!(r.status().as_u16(), 500);
    }

    #[test]
    fn manfiy_status_500_ga_tushadi() {
        // Manfiy status — yaroqsiz, 500 ga tushadi (issue #108), 200 ga emas.
        let r = value_to_response(resp_map(-1, Value::Nil, None));
        assert_eq!(r.status().as_u16(), 500);
    }

    #[test]
    fn u16_wrap_status_500_ga_tushadi() {
        // Code-review (PR #110): tekshiruv ASL i64 ustida bo'lmasa, `65736 as u16`
        // 200 ga wrap bo'lib jim muvaffaqiyatga aldardi. Endi `checked_status`
        // u16 cast'idan oldin diapazonni tekshiradi → 500.
        let r = value_to_response(resp_map(65736, Value::Str("ok".into()), None));
        assert_eq!(r.status().as_u16(), 500);
        // 3xx diapazoniga wrap bo'ladigan manfiy qiymat ham (-65234 → 302) 500 ga.
        let r2 = value_to_response(resp_map(-65234, Value::Nil, None));
        assert_eq!(r2.status().as_u16(), 500);
    }

    #[test]
    fn yaroqli_status_saqlanadi() {
        // Yaroqli status (404) o'zgartirilmaydi — fix faqat buzuq statusга tegadi.
        let r = value_to_response(resp_map(404, Value::Str("not found".into()), None));
        assert_eq!(r.status().as_u16(), 404);
    }

    #[test]
    fn buzuq_header_jim_otkaziladi() {
        // Yaroqsiz header qiymati (yangi qator) butun javobni buzmaydi —
        // jim o'tkazib yuboriladi, qolgan header'lar o'rnatiladi.
        let r = value_to_response(resp_map(
            200,
            Value::Nil,
            Some(hmap(&[
                ("x-bad", Value::Str("bad\nvalue".into())),
                ("x-good", Value::Str("good".into())),
            ])),
        ));
        assert!(r.headers().get("x-bad").is_none());
        assert_eq!(r.headers().get("x-good").unwrap(), "good");
    }

    #[tokio::test]
    async fn band_port_bind_xato_qaytaradi() {
        // Port band bo'lsa bind `Err` qaytaradi (issue #108) — jim `return` emas.
        // Avval portni egallaymiz (0 → OS bo'sh port tanlaydi), so'ng o'sha
        // portga qayta bind urinamiz: aynan bir xil addr → EADDRINUSE.
        let Ok(occupied) = bind(0).await else {
            panic!("bind to a free port must succeed");
        };
        let port = occupied.local_addr().unwrap().port();
        let res = bind(port).await;
        assert!(
            matches!(res, Err(Flow::Error(_))),
            "Err expected for a busy port"
        );
    }

    // --- rate-limit (issue #79) ---

    #[test]
    fn window_birligi_soniyaga_aylanadi() {
        // Canonical to'plam: :sec/:min/:hr. Noma'lum birlik None.
        assert_eq!(window_to_secs("sec"), Some(1));
        assert_eq!(window_to_secs("min"), Some(60));
        assert_eq!(window_to_secs("hr"), Some(3600));
        assert_eq!(window_to_secs("day"), None);
    }

    #[test]
    fn limit_oyna_ichida_sanaydi_va_429_beradi() {
        // limit=3 — dastlabki 3 so'rov o'tadi (None), 4-si bloklanadi (Some).
        let state: LimitState = Arc::new(Mutex::new(LimitBucket::new()));
        assert!(check_and_count(&state, "t1", 3, 3600).is_none());
        assert!(check_and_count(&state, "t1", 3, 3600).is_none());
        assert!(check_and_count(&state, "t1", 3, 3600).is_none());
        let retry = check_and_count(&state, "t1", 3, 3600);
        assert!(retry.is_some(), "the 4th request must be blocked");
        // Retry-After oyna tugashigacha — [1, window_secs] oralig'ida.
        let r = retry.unwrap();
        assert!((1..=3600).contains(&r), "Retry-After is sensible: {}", r);
    }

    #[test]
    fn limit_kalitlar_alohida_sanaladi() {
        // Har kalit (tenant/key) o'z hisobgichiga ega — biri tugasa boshqasiga ta'sir yo'q.
        let state: LimitState = Arc::new(Mutex::new(LimitBucket::new()));
        assert!(check_and_count(&state, "a", 1, 3600).is_none()); // a: 1-chi o'tadi
        assert!(check_and_count(&state, "a", 1, 3600).is_some()); // a: 2-chi bloklanadi
        assert!(check_and_count(&state, "b", 1, 3600).is_none()); // b: alohida bucket, o'tadi
    }

    #[test]
    fn limit_yangi_oynada_tiklanadi() {
        // window_secs=1 — bir soniya o'tgach yangi oyna, hisob nolga tushadi.
        let state: LimitState = Arc::new(Mutex::new(LimitBucket::new()));
        assert!(check_and_count(&state, "k", 1, 1).is_none());
        assert!(check_and_count(&state, "k", 1, 1).is_some()); // shu oynada tugadi
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(
            check_and_count(&state, "k", 1, 1).is_none(),
            "count must reset in a new window"
        );
    }

    #[test]
    fn limit_eski_oyna_kalitlari_tozalanadi() {
        // Xotira cheksiz o'smasligi uchun (Codex review P2): foydalanuvchi
        // nazoratidagi kalitlar yig'ilib qolmasin — eski oyna kalitlari sweep'da
        // o'chadi. window_secs=1: "old"ni yozamiz, oyna o'tkazamiz, keyin
        // SWEEP_EVERY operatsiya bilan sweep'ni ishga tushiramiz.
        let state: LimitState = Arc::new(Mutex::new(LimitBucket::new()));
        check_and_count(&state, "old", 1000, 1);
        std::thread::sleep(std::time::Duration::from_millis(1100)); // keyingi oyna
        for _ in 0..SWEEP_EVERY {
            check_and_count(&state, "new", 1_000_000, 1);
        }
        let bucket = state.lock().unwrap();
        assert!(
            !bucket.counts.contains_key("old"),
            "old window key must be swept"
        );
        assert!(
            bucket.counts.contains_key("new"),
            "current window key must remain"
        );
    }

    #[test]
    fn limit_parallel_atomik_sanaydi() {
        // Acceptance: parallel request'lar ostida to'g'ri sanaydi (race yo'q).
        // 16 thread x 50 urinish = 800; faqat limit=100 tasi o'tishi SHART.
        use std::sync::atomic::{AtomicU32, Ordering};
        let state: LimitState = Arc::new(Mutex::new(LimitBucket::new()));
        let allowed = Arc::new(AtomicU32::new(0));
        let mut handles = vec![];
        for _ in 0..16 {
            let st = state.clone();
            let al = allowed.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..50 {
                    if check_and_count(&st, "k", 100, 3600).is_none() {
                        al.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(
            allowed.load(Ordering::SeqCst),
            100,
            "exactly limit=100 requests must pass (atomic counting)"
        );
    }

    #[test]
    fn fallback_kalit_ip_prefiksli() {
        // Kalit nil bo'lganda req.ip ishlatiladi, "ip:" prefiksi bilan.
        let req = with_ctx(
            build_req(
                "GET".into(),
                "/".into(),
                String::new(),
                BTreeMap::new(),
                BTreeMap::new(),
                "203.0.113.7".into(),
                Bytes::new(),
                false,
                None,
            ),
            Arc::new(Mutex::new(BTreeMap::new())),
        );
        assert_eq!(client_fallback_key(&req), "ip:203.0.113.7");
    }

    #[test]
    fn req_ip_maydoni_mavjud() {
        // build_req req.ip ni qo'yadi — foydalanuvchi `req.ip` o'qiy oladi.
        let req = build_req(
            "GET".into(),
            "/".into(),
            String::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            "10.0.0.1".into(),
            Bytes::new(),
            false,
            None,
        );
        let Value::Map(m) = &req else {
            panic!("Map");
        };
        assert!(
            matches!(m.get("ip"), Some(Value::Str(s)) if s == "10.0.0.1"),
            "req.ip must be set"
        );
    }

    // --- query/path percent-dekod (issue #100) ---

    fn query_get(q: &str, key: &str) -> Option<String> {
        match parse_query(q) {
            Value::Map(m) => match m.get(key) {
                Some(Value::Str(s)) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    #[test]
    fn percent_dekod_utf8_kirill() {
        // `%D1%81...` -> "салом" (kirill UTF-8 baytlar to'g'ri yig'iladi).
        assert_eq!(
            percent_decode("%D1%81%D0%B0%D0%BB%D0%BE%D0%BC", false),
            "салом"
        );
    }

    #[test]
    fn percent_dekod_oddiy_belgi() {
        // `%20` -> bo'shliq, `%2B` -> literal `+` (bo'shliqqa aylanmaydi).
        assert_eq!(percent_decode("a%20b", false), "a b");
        assert_eq!(percent_decode("a%2Bb", false), "a+b");
    }

    #[test]
    fn percent_dekod_yaroqsiz_qoldiradi() {
        // Yaroqsiz `%` ketma-ketligi (`%zz`) va satr oxiridagi `%` literal qoladi
        // (panic yo'q).
        assert_eq!(percent_decode("%zz", false), "%zz");
        assert_eq!(percent_decode("100%", false), "100%");
        assert_eq!(percent_decode("a%2", false), "a%2");
    }

    #[test]
    fn percent_dekod_slash_keep_path_seps() {
        // keep_path_seps=true: `%2F`/`%5C` (har ikki registr) xom qoladi, lekin
        // boshqa baytlar (`%61` -> 'a') odatdagidek dekodlanadi. false bo'lsa
        // (query) ular `/`/`\` ga aylanadi.
        assert_eq!(percent_decode("a%2Fb", true), "a%2Fb");
        assert_eq!(percent_decode("a%2fb", true), "a%2fb");
        assert_eq!(percent_decode("a%5Cb", true), "a%5Cb");
        assert_eq!(percent_decode("%61%2F%61", true), "a%2Fa");
        assert_eq!(percent_decode("a%2Fb", false), "a/b");
    }

    #[test]
    fn query_percent_dekod_qiymat() {
        // GET /search?q=%D1%81%D0%B0%D0%BB%D0%BE%D0%BC -> q = "салом".
        assert_eq!(
            query_get("q=%D1%81%D0%B0%D0%BB%D0%BE%D0%BC", "q").as_deref(),
            Some("салом")
        );
    }

    #[test]
    fn query_plus_boshliq_va_percent() {
        // `+` -> bo'shliq (form-encoding), `%20` ham bo'shliq.
        assert_eq!(
            query_get("name=John+Doe", "name").as_deref(),
            Some("John Doe")
        );
        assert_eq!(
            query_get("name=John%20Doe", "name").as_deref(),
            Some("John Doe")
        );
    }

    #[test]
    fn query_kalit_ham_dekod() {
        // Kalitda ham non-ASCII bo'lishi mumkin — u ham dekod qilinadi.
        assert_eq!(query_get("%D0%B0=1", "а").as_deref(), Some("1"));
    }

    #[test]
    fn path_param_percent_dekod() {
        // `/users/:name` -> "/users/%D0%90%D0%BB%D0%B8" param "name" = "Али".
        let routes = vec![Route {
            method: "get".into(),
            pattern: parse_pattern("/users/:name"),
            handler: Value::Nil,
        }];
        let (_r, params) =
            match_route(&routes, "get", "/users/%D0%90%D0%BB%D0%B8").expect("route must match");
        assert!(matches!(params.get("name"), Some(Value::Str(s)) if s == "Али"));
    }

    #[test]
    fn path_param_encoded_slash_xom_qoladi() {
        // "/users/a%2Fb" bitta segment sifatida ":name"ga mos keladi, lekin
        // `%2F` dekod QILINMAYDI — param qiymatiga `/` kirmasin (segment
        // invarianti; ID/yo'l komponenti deb ishlatuvchi handler ichki slash
        // olmasin, codex revyu). Boshqa segmentdagi non-ASCII baribir dekodlanadi.
        let routes = vec![Route {
            method: "get".into(),
            pattern: parse_pattern("/users/:name"),
            handler: Value::Nil,
        }];
        let (_r, params) =
            match_route(&routes, "get", "/users/a%2Fb").expect("one segment — match");
        assert!(matches!(params.get("name"), Some(Value::Str(s)) if s == "a%2Fb"));
    }

    // --- CORS (issue #135) ---

    // Standart sozlamalar bilan config — testlar faqat kerakli maydonni o'zgartiradi.
    fn cors_cfg(origins: Option<Vec<String>>, creds: bool) -> CorsConfig {
        CorsConfig {
            origins,
            methods: "GET, POST, OPTIONS".into(),
            headers: "Content-Type".into(),
            creds,
            max_age: 600,
        }
    }

    // HeaderMap'dan str qiymat oladi (yo'q bo'lsa None).
    fn hv(h: &hyper::HeaderMap, name: &str) -> Option<String> {
        h.get(name).map(|v| v.to_str().unwrap().to_string())
    }

    #[test]
    fn cors_wildcard_har_origin_uchun_star() {
        // `http.cors "*"` — har qanday origin "*" oladi (creds yo'q).
        let cfg = cors_cfg(None, false);
        let mut h = hyper::HeaderMap::new();
        cfg.apply_to(&mut h, Some("https://a.example.com"));
        assert_eq!(hv(&h, "access-control-allow-origin").as_deref(), Some("*"));
        // "*" bilan Vary: Origin qo'shilmaydi (javob origin'ga bog'liq emas).
        assert_eq!(hv(&h, "vary"), None);
    }

    #[test]
    fn cors_wildcard_creds_origin_aks_ettiradi() {
        // `http.cors "*" {creds: true}` — brauzer "*" + credentials'ni rad etadi,
        // shuning uchun so'rov origin'ini aks ettiramiz + Allow-Credentials.
        let cfg = cors_cfg(None, true);
        let mut h = hyper::HeaderMap::new();
        cfg.apply_to(&mut h, Some("https://a.example.com"));
        assert_eq!(
            hv(&h, "access-control-allow-origin").as_deref(),
            Some("https://a.example.com")
        );
        assert_eq!(
            hv(&h, "access-control-allow-credentials").as_deref(),
            Some("true")
        );
        // Aniq origin aks ettirilganda Vary: Origin shart (kesh to'g'riligi).
        assert_eq!(hv(&h, "vary").as_deref(), Some("Origin"));
    }

    #[test]
    fn cors_royxat_faqat_ruxsat_etilgan_origin() {
        // Aniq ro'yxat — ruxsat etilgan origin aks ettiriladi.
        let cfg = cors_cfg(Some(vec!["https://app.example.com".into()]), false);
        let mut h = hyper::HeaderMap::new();
        cfg.apply_to(&mut h, Some("https://app.example.com"));
        assert_eq!(
            hv(&h, "access-control-allow-origin").as_deref(),
            Some("https://app.example.com")
        );
        assert_eq!(hv(&h, "vary").as_deref(), Some("Origin"));
    }

    #[test]
    fn cors_vary_mavjud_qiymatni_saqlaydi() {
        // Handler `rep ... {vary:"Accept-Encoding"}` qo'ygan Vary'ni o'chirmasdan
        // Origin'ni birlashtiramiz (codex P2: insert kesh kalitini buzardi).
        let cfg = cors_cfg(Some(vec!["https://app.example.com".into()]), false);
        let mut h = hyper::HeaderMap::new();
        h.insert(hyper::header::VARY, "Accept-Encoding".parse().unwrap());
        cfg.apply_to(&mut h, Some("https://app.example.com"));
        assert_eq!(hv(&h, "vary").as_deref(), Some("Accept-Encoding, Origin"));
    }

    #[test]
    fn cors_vary_takror_origin_qoshmaydi() {
        // Vary allaqachon Origin bo'lsa — takror qo'shilmaydi (case-insensitive).
        let cfg = cors_cfg(Some(vec!["https://app.example.com".into()]), false);
        let mut h = hyper::HeaderMap::new();
        h.insert(hyper::header::VARY, "origin".parse().unwrap());
        cfg.apply_to(&mut h, Some("https://app.example.com"));
        // Mavjud "origin" saqlanadi, ikkinchi marta qo'shilmaydi.
        assert_eq!(hv(&h, "vary").as_deref(), Some("origin"));
    }

    #[test]
    fn cors_royxat_tashqi_origin_rad() {
        // Ro'yxatda yo'q origin — hech qanday CORS header qo'shilmaydi
        // (brauzer so'rovni bloklaydi).
        let cfg = cors_cfg(Some(vec!["https://app.example.com".into()]), false);
        let mut h = hyper::HeaderMap::new();
        cfg.apply_to(&mut h, Some("https://evil.example.com"));
        assert_eq!(hv(&h, "access-control-allow-origin"), None);
    }

    #[test]
    fn cors_origin_yoq_royxat_bilan_header_qoshilmaydi() {
        // Origin header'siz so'rov (masalan curl) — aniq ro'yxatda mos yo'q,
        // CORS header qo'shilmaydi.
        let cfg = cors_cfg(Some(vec!["https://app.example.com".into()]), false);
        let mut h = hyper::HeaderMap::new();
        cfg.apply_to(&mut h, None);
        assert_eq!(hv(&h, "access-control-allow-origin"), None);
    }

    #[test]
    fn cors_preflight_metod_va_header_qaytaradi() {
        // OPTIONS preflight 204 + Allow-Methods/Headers/Max-Age qaytaradi.
        let cfg = cors_cfg(None, false);
        let resp = cors_preflight_response(&cfg, Some("https://a.example.com"));
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let h = resp.headers();
        assert_eq!(hv(h, "access-control-allow-origin").as_deref(), Some("*"));
        assert_eq!(
            hv(h, "access-control-allow-methods").as_deref(),
            Some("GET, POST, OPTIONS")
        );
        assert_eq!(
            hv(h, "access-control-allow-headers").as_deref(),
            Some("Content-Type")
        );
        assert_eq!(hv(h, "access-control-max-age").as_deref(), Some("600"));
    }

    #[test]
    fn cors_preflight_rad_etilgan_origin_header_qoshmaydi() {
        // Ruxsat etilmagan origin'ga preflight 204 qaytaradi, lekin CORS
        // header'larsiz — brauzer so'rovni bloklaydi (to'g'ri xulq).
        let cfg = cors_cfg(Some(vec!["https://app.example.com".into()]), false);
        let resp = cors_preflight_response(&cfg, Some("https://evil.example.com"));
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let h = resp.headers();
        assert_eq!(hv(h, "access-control-allow-origin"), None);
        assert_eq!(hv(h, "access-control-allow-methods"), None);
    }

    // --- http.static (issue #134) ---

    fn segv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn static_prefix_parse_va_moslik() {
        // "/" -> bo'sh prefiks (hamma yo'lga mos); "/assets" segment chegarasida
        // tekshiriladi — "/assetsx" mos EMAS.
        assert!(parse_static_prefix("/").is_empty());
        assert_eq!(parse_static_prefix("/assets"), segv(&["assets"]));
        assert_eq!(parse_static_prefix("/a/b/"), segv(&["a", "b"]));

        let pref = parse_static_prefix("/assets");
        assert!(strip_mount_prefix(&pref, &segv(&["assets", "app.css"])).is_some());
        assert!(strip_mount_prefix(&pref, &segv(&["assets"])).is_some());
        assert!(strip_mount_prefix(&pref, &segv(&["assetsx", "a.css"])).is_none());
        assert!(strip_mount_prefix(&pref, &segv(&["other"])).is_none());
        // Qolgan qism — prefiksdan keyingi fayl yo'li.
        assert_eq!(
            strip_mount_prefix(&pref, &segv(&["assets", "img", "a.png"])).unwrap(),
            segv(&["img", "a.png"])
        );
    }

    #[test]
    fn static_safe_join_traversalni_bloklaydi() {
        // Traversal himoyasi MAJBURIY (issue #134): "..", ".", bo'sh, mutlaq va
        // backslash/NUL segmentlari rad etiladi — katalogdan tashqariga chiqib
        // bo'lmaydi. Percent-dekod chaqiruvchida bo'ladi, shuning uchun bu yerga
        // `%2e%2e` allaqachon ".." bo'lib keladi va shu tekshiruvga ilinadi.
        let dir = Path::new("/srv/public");
        assert!(safe_join(dir, &segv(&["..", "secret"])).is_none());
        assert!(safe_join(dir, &segv(&["a", "..", "b"])).is_none());
        assert!(safe_join(dir, &segv(&["."])).is_none());
        assert!(safe_join(dir, &segv(&[""])).is_none());
        assert!(safe_join(dir, &segv(&["a\\b"])).is_none());
        assert!(safe_join(dir, &segv(&["a\0b"])).is_none());
        assert!(safe_join(dir, &segv(&["/etc", "passwd"])).is_none());
        // Oddiy nomlar — ulanadi.
        let p = safe_join(dir, &segv(&["img", "a.png"])).unwrap();
        assert_eq!(p, PathBuf::from("/srv/public/img/a.png"));
        // Bo'sh rest (prefiksning o'zi so'ralgan) — katalogning o'zi.
        assert_eq!(safe_join(dir, &[]).unwrap(), PathBuf::from("/srv/public"));
    }

    #[test]
    fn static_mime_kengaytmadan() {
        // Content-Type kengaytmadan avtomatik; noma'lum -> octet-stream.
        assert_eq!(
            mime_for(Path::new("a/index.html")),
            "text/html; charset=utf-8"
        );
        assert_eq!(mime_for(Path::new("app.CSS")), "text/css; charset=utf-8");
        assert_eq!(
            mime_for(Path::new("app.js")),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(mime_for(Path::new("logo.svg")), "image/svg+xml");
        assert_eq!(mime_for(Path::new("a.png")), "image/png");
        assert_eq!(mime_for(Path::new("font.woff2")), "font/woff2");
        assert_eq!(mime_for(Path::new("data.bin")), "application/octet-stream");
        assert_eq!(
            mime_for(Path::new("noextension")),
            "application/octet-stream"
        );
    }

    // So'rov yo'lini try_serve_static bilan bir xil qoidada segmentlarga
    // ajratadi (percent-dekod, %2F xom qoladi) — resolve_static endi tayyor
    // segmentlarni oladi (dekod chaqiruvchida, prefiks tekshiruvi bilan bitta).
    fn decode_segs(path: &str) -> Vec<String> {
        path_segments(path)
            .iter()
            .map(|s| percent_decode(s, true))
            .collect()
    }

    #[tokio::test]
    async fn static_resolve_uzun_prefiks_yutadi() {
        // "/" va "/assets" mount'lari birga: /assets/a.css uzunroq prefiksdagi
        // papkadan olinadi (eng aniq mount ustun).
        let root = std::env::temp_dir().join("fluxon_static_unit_1");
        std::fs::create_dir_all(root.join("dist")).unwrap();
        std::fs::create_dir_all(root.join("public")).unwrap();
        // Mount katalogi registratsiyada canonicalize qilinadi (http_static) —
        // testda ham shunday, aks holda macOS'da /tmp symlink'i taqqoslashni buzadi.
        let dist = std::fs::canonicalize(root.join("dist")).unwrap();
        let public = std::fs::canonicalize(root.join("public")).unwrap();
        std::fs::write(dist.join("a.css"), "dist css").unwrap();
        std::fs::write(public.join("a.css"), "public css").unwrap();
        std::fs::write(dist.join("index.html"), "<h1>spa</h1>").unwrap();

        let mounts = vec![
            StaticMount {
                prefix: vec![],
                dir: dist.clone(),
                spa: true,
            },
            StaticMount {
                prefix: vec!["assets".to_string()],
                dir: public.clone(),
                spa: false,
            },
        ];
        // /assets/a.css -> public (uzun prefiks), /a.css -> dist (root mount).
        // len — metadata'dagi bayt soni (HEAD Content-Length shu bilan beriladi).
        let (p, mime, len) = resolve_static(&mounts, &decode_segs("/assets/a.css"))
            .await
            .unwrap();
        assert_eq!(p, public.join("a.css"));
        assert_eq!(mime, "text/css; charset=utf-8");
        assert_eq!(len, "public css".len() as u64);
        let (p, _, len) = resolve_static(&mounts, &decode_segs("/a.css"))
            .await
            .unwrap();
        assert_eq!(p, dist.join("a.css"));
        assert_eq!(len, "dist css".len() as u64);
        // Katalog so'ralganda index.html.
        let (p, mime, _) = resolve_static(&mounts, &decode_segs("/")).await.unwrap();
        assert_eq!(p, dist.join("index.html"));
        assert_eq!(mime, "text/html; charset=utf-8");
        // Topilmagan yo'l — SPA fallback (root mount spa:true).
        let (p, _, _) = resolve_static(&mounts, &decode_segs("/no/such/page"))
            .await
            .unwrap();
        assert_eq!(p, dist.join("index.html"));
        // /assets ostida topilmagan fayl: assets mount spa emas, lekin root SPA
        // mount prefiksi baribir mos — fallback unga tushadi.
        let (p, _, _) = resolve_static(&mounts, &decode_segs("/assets/none.css"))
            .await
            .unwrap();
        assert_eq!(p, dist.join("index.html"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn static_resolve_traversal_404() {
        // `..` (xom yoki percent-encoded) mount katalogidan tashqariga olib
        // chiqmaydi — None (404), sirli fayl o'qilmaydi.
        let root = std::env::temp_dir().join("fluxon_static_unit_2");
        std::fs::create_dir_all(root.join("public")).unwrap();
        let public = std::fs::canonicalize(root.join("public")).unwrap();
        std::fs::write(public.join("ok.txt"), "ok").unwrap();
        std::fs::write(root.join("secret.txt"), "secret").unwrap();

        let mounts = vec![StaticMount {
            prefix: vec!["assets".to_string()],
            dir: public.clone(),
            spa: false,
        }];
        assert!(
            resolve_static(&mounts, &decode_segs("/assets/../secret.txt"))
                .await
                .is_none()
        );
        assert!(
            resolve_static(&mounts, &decode_segs("/assets/%2e%2e/secret.txt"))
                .await
                .is_none()
        );
        assert!(
            resolve_static(&mounts, &decode_segs("/assets/..%2Fsecret.txt"))
                .await
                .is_none()
        );
        assert!(
            resolve_static(&mounts, &decode_segs("/assets/ok.txt"))
                .await
                .is_some()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn static_symlink_ildizdan_chiqsa_404() {
        // Leksik himoya (safe_join) symlink'ni ko'rmaydi: papka ichidagi
        // symlink ildizdan TASHQARI faylga ishora qilsa xizmat qilinmasligi
        // kerak (codex P2 — canonicalize + ildiz tekshiruvi). Ildiz ICHIDAGI
        // nishonga ishora qiluvchi symlink esa avvalgidek beriladi.
        let root = std::env::temp_dir().join("fluxon_static_unit_3");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("public")).unwrap();
        let public = std::fs::canonicalize(root.join("public")).unwrap();
        std::fs::write(root.join("secret.txt"), "SECRET").unwrap();
        std::fs::write(public.join("inner.txt"), "inner").unwrap();
        // Tashqariga ishora: public/evil.txt -> ../secret.txt
        std::os::unix::fs::symlink(root.join("secret.txt"), public.join("evil.txt")).unwrap();
        // Ichkariga ishora: public/alias.txt -> public/inner.txt
        std::os::unix::fs::symlink(public.join("inner.txt"), public.join("alias.txt")).unwrap();

        let mounts = vec![StaticMount {
            prefix: vec!["assets".to_string()],
            dir: public.clone(),
            spa: false,
        }];
        assert!(
            resolve_static(&mounts, &decode_segs("/assets/evil.txt"))
                .await
                .is_none(),
            "a symlink pointing outside the root must not be served"
        );
        let (p, _, _) = resolve_static(&mounts, &decode_segs("/assets/alias.txt"))
            .await
            .expect("a symlink inside the root must work");
        // Canonical yo'l — symlink nishoni (haqiqiy fayl).
        assert_eq!(p, public.join("inner.txt"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
