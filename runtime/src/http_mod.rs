// Flux HTTP battery — server (http.on/http.serve/rep) va klient (http.get/post).
//
// Server tokio + hyper ustida quriladi. Flux handler'lari sinxron tree-walking
// bo'lgani uchun har request `spawn_blocking` ichida bajariladi — bu CPU ishini
// tokio worker'larini bloklamasdan HAQIQIY PARALLEL qiladi (Value: Send+Sync,
// thread-safety refactor shuni ta'minlaydi).
//
// `rep status body` -> {__resp:true status body} map (builtins.rs::install).
// `fail status "msg"` -> Flow::Fail -> JSON xato javob.

use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::TcpListener;

use crate::builtins::{json_decode, json_encode};
use crate::interp::{Flow, Interp};
use crate::value::Value;

// --- marshrut tuzilmasi ---

// Yo'l segmenti: literal (`notes`) yoki parametr (`:id`).
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

// Rate-limit holati: kalit -> (window_id, count). Fixed-window — `window_id =
// now_sek / window_sek`. Arc<Mutex> shuning uchun limiter REGISTRATSIYA paytida
// bir marta yaratiladi, har request shu BITTA holatni ulashadi (Middleware klonida
// Arc nusxalanadi — pointer bir xil), shu sababli parallel request'lar atomik
// sanaydi (issue #79: thread-safe). Holat in-memory — bitta instance uchun (docs).
//
// Xotira chegarasi: kalit funksiyasi foydalanuvchi nazoratidagi qiymatga
// (`req.headers.x_api_key`) asoslansa, har yangi qiymat HashMap'ga kirib qoladi.
// Public endpoint'da mijoz har so'rovda yangi kalit yuborib holatni cheksiz
// o'stira oladi. Buni oldini olish uchun `LimitBucket` har `SWEEP_EVERY`
// operatsiyada bir marta ESKI OYNADAGI kalitlarni tozalaydi (amortizatsiyalangan
// O(1): tozalash sikli kamdan-kam ishlaydi). Eski oyna kaliti baribir keyingi
// so'rovda count=0 dan qayta boshlanardi — shuning uchun o'chirish xavfsiz.
//
// pub: `pub enum MwKind` (Middleware orqali) LimitState turini oshkor qiladi.
pub struct LimitBucket {
    counts: HashMap<String, (u64, u32)>,
    // Oxirgi tozalashdan beri operatsiyalar soni (sweep'ni amortizatsiya qiladi).
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

// Necha operatsiyada bir marta eski oyna kalitlarini tozalaymiz.
const SWEEP_EVERY: u32 = 1024;

// HTTP server uchun standart so'rov tanasi (body) o'lcham chegarasi (issue #91).
// Chegarasiz `collect()` butun tanani xotiraga yig'adi — mijoz ulkan body yuborib
// server xotirasini to'ldira oladi (DoS). Default 10 MiB; `http.serve PORT
// {max_body: N}` bilan sozlanadi. `max_body: 0` chegarani o'chiradi (cheklovsiz —
// faqat ishonchli, ichki tarmoq orqasida ishlating).
const DEFAULT_MAX_BODY: usize = 10 * 1024 * 1024;

pub type LimitState = Arc<Mutex<LimitBucket>>;

// Middleware turi: oddiy fn (use/before) yoki rate-limiter (http.limit). Limit
// ham SHU ro'yxatga qo'shiladi (alohida emas) — shunda u boshqa middleware bilan
// DEKLARATSIYA TARTIBIDA ishlaydi: undan oldin e'lon qilingan auth `req.ctx`'ga
// tenant_id yozsa, kalit funksiyasi `\req -> req.ctx.tenant_id` uni ko'radi (#79).
#[derive(Clone)]
pub enum MwKind {
    // http.use / http.before — handler'ni chaqiradi; `fail`/`rep` zanjirni to'xtatadi.
    Fn,
    // http.limit — handler KALIT funksiyasi (req -> kalit). Limit oshsa 429.
    Limit {
        limit: u32,
        window_secs: u64,
        state: LimitState,
    },
}

// Middleware (issue #67). `scope` = None — global (`http.use`, barcha yo'lga);
// Some(shablon) — prefiks bo'yicha (`http.before "/api/*"`). Ro'yxatda
// deklaratsiya tartibida saqlanadi (use/before/limit aralashsa ham tartib aniq).
#[derive(Clone)]
pub struct Middleware {
    pub scope: Option<String>,
    pub handler: Value,
    pub kind: MwKind,
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
        Value::Str("rate limit oshib ketdi".to_string()),
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

// --- request -> Value::Map ---

// req = {method, path, query:{}, headers:{}, params:{}, body:(JSON map/str), ctx}
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
) -> Value {
    let body = if body_bytes.is_empty() {
        Value::Nil
    } else {
        let s = String::from_utf8_lossy(&body_bytes);
        // Content-Type JSON bo'lsa, YOKI tana `{`/`[` bilan boshlansa — JSON
        // parse'ga urinamiz. Sabab: `curl -d` standart holda
        // x-www-form-urlencoded yuboradi, lekin tana ko'rinishidan JSON; agar
        // Content-Type'ga qat'i bog'lansak, dasturchi sababsiz string oladi va
        // `body.field` access chalg'ituvchi "str.field metodi" xatosi beradi.
        let looks_like_json = matches!(s.trim_start().as_bytes().first(), Some(b'{') | Some(b'['));
        if is_json || looks_like_json {
            // JSON dekod xato bo'lsa — xom matn sifatida qoldiramiz.
            json_decode(&s).unwrap_or_else(|_| Value::Str(s.to_string()))
        } else {
            Value::Str(s.to_string())
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

// Flux `Int` statusni (rep/fail) yaroqli HTTP status u16'ga aylantiradi.
// Tekshiruv ASL i64 ustida bo'lishi shart: `as u16` cast oldin wrap qiladi —
// `rep 65736` u16'da 200 ga, ba'zi manfiy qiymatlar 3xx/4xx ga tushib jim
// muvaffaqiyatga aldardi (issue #108). Diapazondan tashqari yoki HTTP bo'lmagan
// kod → 500 + log, shunda mijoz handler xatosini muvaffaqiyat deb o'qimaydi.
fn checked_status(n: i64) -> u16 {
    match u16::try_from(n) {
        Ok(s) if StatusCode::from_u16(s).is_ok() => s,
        _ => {
            eprintln!("Flux HTTP: noto'g'ri status kodi {} → 500", n);
            500
        }
    }
}

// u16 status → StatusCode. Builder darajasidagi himoya to'ri: chaqiruvchilar
// allaqachon yaroqli kod beradi (literal yoki `checked_status`), bu faqat
// kutilmagan holatda panic o'rniga 500 qaytaradi.
fn status_or_500(status: u16) -> StatusCode {
    StatusCode::from_u16(status).unwrap_or_else(|_| {
        eprintln!("Flux HTTP: noto'g'ri status kodi {} → 500", status);
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
        Value::Str(format!(
            "so'rov tanasi juda katta (chegara: {} bayt)",
            limit
        )),
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
// Kalitda `_` → `-` ga aylanadi: Flux map kalitida defis bo'lolmaydi
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

// Javob tanasini tipiga qarab formatlash: map/list -> JSON, str -> matn,
// nil -> bo'sh, qolgani -> JSON.
fn body_value_to_response(status: u16, body: Value) -> Response<Full<Bytes>> {
    match body {
        Value::Nil => Response::builder()
            .status(status_or_500(status))
            .body(Full::new(Bytes::new()))
            .unwrap(),
        Value::Str(s) => text_response(status, s),
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
        Flow::Skip | Flow::Stop => (500, "handler skip/stop ishlatdi".to_string()),
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
            "limit" => self.http_limit(args),
            "serve" => self.http_serve(args),
            "get" => http_client("GET", args, false),
            "post" => http_client("POST", args, true),
            "put" => http_client("PUT", args, true),
            "del" => http_client("DELETE", args, false),
            _ => Err(Flow::err(format!(
                "http modulida '{}' funksiyasi yo'q",
                func
            ))),
        }
    }

    // http.on :method "/path" handler
    fn http_on(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let method = match args.first() {
            Some(Value::Sym(s)) | Some(Value::Str(s)) => s.to_lowercase(),
            _ => {
                return Err(Flow::err(
                    "http.on: 1-argument metod (:get/:post...) bo'lishi kerak",
                ));
            }
        };
        let path = match args.get(1) {
            Some(Value::Str(s)) => s.clone(),
            _ => return Err(Flow::err("http.on: 2-argument yo'l (str) bo'lishi kerak")),
        };
        let handler = match args.get(2) {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => return Err(Flow::err("http.on: 3-argument handler (fn) bo'lishi kerak")),
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
            _ => return Err(Flow::err("http.use: argument handler (fn) bo'lishi kerak")),
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
                return Err(Flow::err(
                    "http.before: 1-argument yo'l (str) bo'lishi kerak",
                ));
            }
        };
        let handler = match args.get(1) {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => {
                return Err(Flow::err(
                    "http.before: 2-argument handler (fn) bo'lishi kerak",
                ));
            }
        };
        self.middlewares.lock().unwrap().push(Middleware {
            scope: Some(pat),
            handler,
            kind: MwKind::Fn,
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
                    "http.limit: limit musbat int bo'lishi kerak (masalan 100)",
                ));
            }
        };
        let window_secs = match args.get(i + 1) {
            Some(Value::Sym(s)) | Some(Value::Str(s)) => match window_to_secs(s) {
                Some(secs) => secs,
                None => {
                    return Err(Flow::err(
                        "http.limit: oyna :sec, :min yoki :hr bo'lishi kerak",
                    ));
                }
            },
            _ => {
                return Err(Flow::err(
                    "http.limit: oyna birligi (:sec/:min/:hr) bo'lishi kerak",
                ));
            }
        };
        let keyfn = match args.get(i + 2) {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => {
                return Err(Flow::err(
                    "http.limit: kalit funksiyasi (\\req -> ...) bo'lishi kerak",
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
            _ => return Err(Flow::err("http.serve: port (int) bo'lishi kerak")),
        };
        // Ixtiyoriy ikkinchi argument — opsiyalar map'i: `{max_body: BAYT}`.
        // Berilmasa default DEFAULT_MAX_BODY; `max_body: 0` chegarani o'chiradi.
        let max_body = match args.get(1) {
            None => DEFAULT_MAX_BODY,
            Some(Value::Map(m)) => match m.get("max_body") {
                None => DEFAULT_MAX_BODY,
                Some(Value::Int(n)) if *n >= 0 => *n as usize,
                _ => {
                    return Err(Flow::err(
                        "http.serve: max_body manfiy bo'lmagan int bo'lishi kerak",
                    ));
                }
            },
            _ => {
                return Err(Flow::err(
                    "http.serve: ikkinchi argument opsiyalar map'i bo'lishi kerak ({max_body: N})",
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
        .map_err(|e| Flow::err(format!("Flux HTTP port {} bind xatosi: {}", port, e)))
}

// Bitta HTTP server uchun accept loop — umumiy event-loop ichida spawn qilinadi
// (`serve_mod`). Listener oldindan `bind` bilan ochilgan (bind xatosi spawn'dan
// oldin ko'tariladi).
pub async fn serve_loop(interp: Arc<Interp>, listener: TcpListener, max_body: usize) {
    let port = listener.local_addr().map(|a| a.port()).unwrap_or_default();
    eprintln!("Flux HTTP server: http://localhost:{}", port);

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("http accept xatosi: {}", e);
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
            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .await
            {
                eprintln!("ulanish xatosi: {}", e);
            }
        });
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
    // Flux'da req.headers.x_user_id sifatida o'qiladi).
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

    // Marshrutni topamiz (handler'ni baytlardan oldin, 404 ni tez qaytarish uchun).
    let matched = {
        let routes = interp.routes.lock().unwrap();
        match_route(&routes, &method, &path)
    };

    let (route, params) = match matched {
        Some(x) => x,
        None => {
            let mut m = BTreeMap::new();
            m.insert(
                "error".to_string(),
                Value::Str(format!("topilmadi: {} {}", method, path)),
            );
            return Ok(json_response(404, json_encode(&Value::Map(m))));
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
            // body:nil bilan yetardi); endi 400 qaytaramiz (issue #91).
            Err(_) => return Ok(bad_request("so'rov tanasini o'qib bo'lmadi")),
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
            return Ok(payload_too_large(max_body));
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
                    return Ok(payload_too_large(max_body));
                }
                return Ok(bad_request("so'rov tanasini o'qib bo'lmadi"));
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
            method, path, query, headers, params, client_ip, body_bytes, is_json,
        ),
        ctx,
    );
    let handler = route.handler;

    // Sinxron interp ishini blocking thread'da bajaramiz — tokio worker'ini
    // bloklamaydi, har request alohida thread'da -> haqiqiy parallel.
    let result = tokio::task::spawn_blocking(move || {
        // Middleware zanjiri handler'dan OLDIN ishlaydi. Har biri req klonini
        // oladi (ctx Arc ulashilgan). To'xtatish shartlari:
        //   - `fail`/xato -> Err(flow), zanjir to'xtaydi.
        //   - `rep` -> Ok({__resp:true ...}) MUVAFFAQIYATLI qaytadi (Flow emas),
        //     shuning uchun bu javobni ALOHIDA aniqlab to'xtatamiz; aks holda
        //     auth `rep 401` e'tiborsiz qolib, handler baribir ishlardi.
        //   - boshqa Ok(_) (ctx yozish, log) -> zanjir davom etadi.
        for mw in chain {
            match mw.kind {
                // Oddiy middleware (use/before): handler'ni chaqiramiz.
                MwKind::Fn => match interp.apply(mw.handler, vec![request_value.clone()]) {
                    Ok(v) if is_resp(&v) => return Ok(v), // rep -> javob, zanjir to'xta
                    Ok(_) => {}                           // davom (ctx/log)
                    Err(flow) => return Err(flow),        // fail/xato -> to'xta
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
                        Ok(Value::Nil) => client_fallback_key(&request_value),
                        Ok(v) => {
                            let t = v.to_text();
                            if t.is_empty() {
                                client_fallback_key(&request_value)
                            } else {
                                t
                            }
                        }
                        Err(flow) => return Err(flow), // kalit fn xato berdi -> to'xta
                    };
                    if let Some(retry) = check_and_count(&state, &key, limit, window_secs) {
                        return Ok(rate_limited_response(retry));
                    }
                }
            }
        }
        interp.apply(handler, vec![request_value])
    })
    .await;

    let resp = match result {
        Ok(Ok(v)) => value_to_response(v),
        Ok(Err(flow)) => flow_to_response(flow),
        Err(join_err) => flow_to_response(Flow::Error(format!("handler panic: {}", join_err))),
    };
    Ok(resp)
}

// --- HTTP klient: http.get/post/put/del ---

// Request body hozir sodda bytes buffer: alias client tipini o'qilishi oson qiladi.
type ClientBody = Full<Bytes>;
// HttpsConnector<HttpConnector> ham http:// ham https:// ni boshqaradi — TLS
// faqat https sxemada faollashadi, plaintext so'rovlar avvalgidek ishlaydi.
type PooledHttpClient = Client<HttpsConnector<HttpConnector>, ClientBody>;

// Klient so'rovlari uchun bir martalik global runtime (Flux skripti sinxron).
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
}

impl Default for ClientOpts {
    fn default() -> Self {
        ClientOpts {
            follow: false,
            max: 10,
            headers: BTreeMap::new(),
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
                "http.{}: url (str) kerak",
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
    let (body_str, is_json) = match &body {
        Some(Value::Map(_)) | Some(Value::List(_)) => (json_encode(body.as_ref().unwrap()), true),
        Some(Value::Str(s)) => (s.clone(), false),
        Some(other) => (format!("{}", other), false),
        None => (String::new(), false),
    };

    client_runtime().block_on(async move {
        let mut current = url;
        // method redirect'da o'zgarishi mumkin (303 va GET-aylantiruvchi 301/302).
        let mut cur_method = method.to_string();
        let mut hops: i64 = 0;

        loop {
            let uri: hyper::Uri = current
                .parse()
                .map_err(|e| Flow::err(format!("noto'g'ri url: {}", e)))?;

            // GET'ga aylangach tana yuborilmaydi.
            let send_body = cur_method != "GET" && cur_method != "DELETE";
            let mut builder = Request::builder().method(cur_method.as_str()).uri(uri);
            // Foydalanuvchi custom header'larini avval qo'shamiz. content-type'ni
            // foydalanuvchi o'zi bergan bo'lsa, avtomatik qiymat ustiga yozmaymiz.
            let mut has_user_ct = false;
            for (k, v) in &opts.headers {
                if k.eq_ignore_ascii_case("content-type") {
                    has_user_ct = true;
                }
                builder = builder.header(k.as_str(), v.as_str());
            }
            if is_json && send_body && !has_user_ct {
                builder = builder.header("content-type", "application/json");
            }
            let payload = if send_body {
                Bytes::from(body_str.clone())
            } else {
                Bytes::new()
            };
            let req = builder
                .body(Full::new(payload))
                .map_err(|e| Flow::err(format!("so'rov qurish: {}", e)))?;

            let resp = pooled_http_client()
                .request(req)
                .await
                .map_err(|e| Flow::err(format!("http so'rov: {}", e)))?;

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
                        "redirect limiti oshib ketdi ({} hop)",
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
                continue;
            }

            // Yakuniy javob — header, status, body'ni yig'amiz.
            let resp_is_json = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.contains("application/json"))
                .unwrap_or(false);

            // Header'lar: kalit kichik harf, qiymat str (req.headers bilan simmetrik).
            let mut headers = BTreeMap::new();
            for (k, v) in resp.headers() {
                if let Ok(val) = v.to_str() {
                    headers.insert(k.as_str().to_lowercase(), Value::Str(val.to_string()));
                }
            }

            let bytes = resp
                .into_body()
                .collect()
                .await
                .map_err(|e| Flow::err(format!("javob o'qish: {}", e)))?
                .to_bytes();
            let text = String::from_utf8_lossy(&bytes).to_string();
            let resp_body = if resp_is_json {
                json_decode(&text).unwrap_or(Value::Str(text))
            } else {
                Value::Str(text)
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
    })
}

// Redirect Location'ini joriy URL asosida hal qiladi. Location to'liq URL bo'lsa
// (`http://...`) o'sha qaytadi; aks holda joriy URL'ning sxema+host'iga ulanadi
// (mutlaq yo'l `/x` yoki nisbiy yo'l).
fn resolve_location(base: &str, loc: &str) -> String {
    if loc.starts_with("http://") || loc.starts_with("https://") {
        return loc.to_string();
    }
    // base'dan sxema://host qismini ajratamiz.
    let scheme_end = base.find("://").map(|i| i + 3).unwrap_or(0);
    let host_end = base[scheme_end..]
        .find('/')
        .map(|i| scheme_end + i)
        .unwrap_or(base.len());
    let origin = &base[..host_end];
    if loc.starts_with('/') {
        format!("{}{}", origin, loc)
    } else {
        // nisbiy yo'l: joriy yo'lning oxirgi segmentini almashtiramiz.
        let path_part = &base[host_end..];
        let dir_end = path_part
            .rfind('/')
            .map(|i| host_end + i + 1)
            .unwrap_or(base.len());
        format!("{}{}", &base[..dir_end], loc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // host'dan keyin yo'l yo'q bo'lsa, nisbiy yo'l to'g'ridan-to'g'ri ulanadi.
        let got = resolve_location("http://a.com", "page");
        assert_eq!(got, "http://a.compage");
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
        hm.insert("x-api-key".to_string(), Value::Str("sirli".to_string()));
        hm.insert(
            "anthropic-version".to_string(),
            Value::Str("2023-06-01".to_string()),
        );
        let mut m = BTreeMap::new();
        m.insert("headers".to_string(), Value::Map(hm));
        let o = parse_client_opts(Some(&Value::Map(m)));
        assert_eq!(
            o.headers.get("x-api-key").map(|s| s.as_str()),
            Some("sirli")
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
        );
        match v {
            Value::Map(m) => m.get("body").cloned().unwrap(),
            _ => panic!("build_req Map qaytarishi kerak"),
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
        assert!(matches!(body_of("salom=dunyo", false), Value::Str(_)));
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
        );
        let req = with_ctx(req, cell.clone());
        let Value::Map(m) = &req else {
            panic!("req Map bo'lishi kerak");
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
        assert!(got.equals(&Value::Int(7)), "ctx klon orqali yangilandi");
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
            ),
            cell,
        );
        let req_clone = req.clone();
        // Map equality ctx kalitiga yetadi -> (Ctx,Ctx) bir xil Arc -> ptr_eq.
        assert!(
            req.equals(&req_clone),
            "req o'z kloniga teng, deadlock yo'q"
        );
        assert!(req.equals(&req), "req o'ziga teng, deadlock yo'q");
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
            Value::Str("<h1>Salom</h1>".into()),
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
        let r = value_to_response(resp_map(1000, Value::Str("xato".into()), None));
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
        let r = value_to_response(resp_map(404, Value::Str("topilmadi".into()), None));
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
                ("x-bad", Value::Str("yomon\nqiymat".into())),
                ("x-good", Value::Str("yaxshi".into())),
            ])),
        ));
        assert!(r.headers().get("x-bad").is_none());
        assert_eq!(r.headers().get("x-good").unwrap(), "yaxshi");
    }

    #[tokio::test]
    async fn band_port_bind_xato_qaytaradi() {
        // Port band bo'lsa bind `Err` qaytaradi (issue #108) — jim `return` emas.
        // Avval portni egallaymiz (0 → OS bo'sh port tanlaydi), so'ng o'sha
        // portga qayta bind urinamiz: aynan bir xil addr → EADDRINUSE.
        let Ok(occupied) = bind(0).await else {
            panic!("bo'sh portga bind bo'lishi kerak");
        };
        let port = occupied.local_addr().unwrap().port();
        let res = bind(port).await;
        assert!(
            matches!(res, Err(Flow::Error(_))),
            "band port → Err kutilgan"
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
        assert!(retry.is_some(), "4-so'rov bloklanishi kerak");
        // Retry-After oyna tugashigacha — [1, window_secs] oralig'ida.
        let r = retry.unwrap();
        assert!((1..=3600).contains(&r), "Retry-After mantiqiy: {}", r);
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
            "yangi oynada hisob tiklanishi kerak"
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
            "eski oyna kaliti tozalanishi kerak"
        );
        assert!(
            bucket.counts.contains_key("new"),
            "joriy oyna kaliti qolishi kerak"
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
            "aniq limit=100 so'rov o'tishi kerak (atomik sanash)"
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
        );
        let Value::Map(m) = &req else {
            panic!("Map");
        };
        assert!(
            matches!(m.get("ip"), Some(Value::Str(s)) if s == "10.0.0.1"),
            "req.ip o'rnatilishi kerak"
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
        let (_r, params) = match_route(&routes, "get", "/users/%D0%90%D0%BB%D0%B8")
            .expect("marshrut mos kelishi kerak");
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
        let (_r, params) = match_route(&routes, "get", "/users/a%2Fb").expect("bir segment — mos");
        assert!(matches!(params.get("name"), Some(Value::Str(s)) if s == "a%2Fb"));
    }
}
