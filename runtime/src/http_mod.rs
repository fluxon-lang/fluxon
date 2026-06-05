// Flux HTTP battery — server (http.on/http.serve/rep) va klient (http.get/post).
//
// Server tokio + hyper ustida quriladi. Flux handler'lari sinxron tree-walking
// bo'lgani uchun har request `spawn_blocking` ichida bajariladi — bu CPU ishini
// tokio worker'larini bloklamasdan HAQIQIY PARALLEL qiladi (Value: Send+Sync,
// thread-safety refactor shuni ta'minlaydi).
//
// `rep status body` -> {__resp:true status body} map (builtins.rs::install).
// `fail status "msg"` -> Flow::Fail -> JSON xato javob.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
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
                    params.insert(name.clone(), Value::Str((*seg).to_string()));
                }
            }
        }
        if ok {
            return Some((r.clone(), params));
        }
    }
    None
}

// "a=1&b=2" -> {a:"1" b:"2"}. URL-dekod minimal (faqat '+' -> bo'shliq).
fn parse_query(q: &str) -> Value {
    let mut m = BTreeMap::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k.to_string(), v.replace('+', " ")),
            None => (pair.to_string(), String::new()),
        };
        m.insert(k, Value::Str(v));
    }
    Value::Map(m)
}

// --- request -> Value::Map ---

// req = {method, path, query:{}, headers:{}, params:{}, body:(JSON map/str)}
fn build_req(
    method: String,
    path: String,
    query: String,
    headers: BTreeMap<String, Value>,
    params: BTreeMap<String, Value>,
    body_bytes: Bytes,
    is_json: bool,
) -> Value {
    let body = if body_bytes.is_empty() {
        Value::Nil
    } else if is_json {
        let s = String::from_utf8_lossy(&body_bytes);
        // JSON dekod xato bo'lsa — xom matn sifatida qoldiramiz.
        json_decode(&s).unwrap_or_else(|_| Value::Str(s.to_string()))
    } else {
        Value::Str(String::from_utf8_lossy(&body_bytes).to_string())
    };

    let mut m = BTreeMap::new();
    m.insert("method".to_string(), Value::Str(method));
    m.insert("path".to_string(), Value::Str(path));
    m.insert("query".to_string(), parse_query(&query));
    m.insert("headers".to_string(), Value::Map(headers));
    m.insert("params".to_string(), Value::Map(params));
    m.insert("body".to_string(), body);
    Value::Map(m)
}

// --- Value/Flow -> hyper::Response ---

fn json_response(status: u16, body: String) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK))
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}

fn text_response(status: u16, body: String) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK))
        .header("content-type", "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}

// Handler muvaffaqiyatli qaytargan qiymatni javobga aylantiradi.
// `rep` -> {__resp:true status body}. Aks holda 200 + qiymat.
fn value_to_response(v: Value) -> Response<Full<Bytes>> {
    if let Value::Map(m) = &v
        && matches!(m.get("__resp"), Some(Value::Bool(true)))
    {
        let status = match m.get("status") {
            Some(Value::Int(n)) => *n as u16,
            _ => 200,
        };
        let body = m.get("body").cloned().unwrap_or(Value::Nil);
        return body_value_to_response(status, body);
    }
    // rep ishlatilmagan — qiymatning o'zini 200 bilan qaytaramiz.
    body_value_to_response(200, v)
}

// Javob tanasini tipiga qarab formatlash: map/list -> JSON, str -> matn,
// nil -> bo'sh, qolgani -> JSON.
fn body_value_to_response(status: u16, body: Value) -> Response<Full<Bytes>> {
    match body {
        Value::Nil => Response::builder()
            .status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK))
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
        Flow::Fail { status, message } => (status.unwrap_or(400) as u16, message),
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
            "serve" => self.http_serve(args),
            "get" => http_client("GET", args),
            "post" => http_client("POST", args),
            "put" => http_client("PUT", args),
            "del" => http_client("DELETE", args),
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

    // http.serve port — bloklovchi tokio multi-thread server.
    fn http_serve(self: &Arc<Self>, args: Vec<Value>) -> Result<Value, Flow> {
        let port = match args.first() {
            Some(Value::Int(n)) => *n as u16,
            _ => return Err(Flow::err("http.serve: port (int) bo'lishi kerak")),
        };
        // Top-level kod tugadi — global'ni lock-free snapshot'ga muzlatamiz,
        // shunda parallel handler'lar global qidiruvda RwLock'ga urilmaydi.
        self.freeze_globals();
        let interp = self.clone();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| Flow::err(format!("tokio runtime: {}", e)))?;

        rt.block_on(async move {
            let addr = SocketAddr::from(([0, 0, 0, 0], port));
            let listener = TcpListener::bind(addr)
                .await
                .map_err(|e| Flow::err(format!("port {} bind: {}", port, e)))?;
            eprintln!("Flux HTTP server: http://localhost:{}", port);

            loop {
                let (stream, _) = listener
                    .accept()
                    .await
                    .map_err(|e| Flow::err(format!("accept: {}", e)))?;
                let io = TokioIo::new(stream);
                let interp = interp.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |req: Request<Incoming>| {
                        let interp = interp.clone();
                        async move { handle_request(interp, req).await }
                    });
                    if let Err(e) =
                        hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                            .serve_connection(io, service)
                            .await
                    {
                        eprintln!("ulanish xatosi: {}", e);
                    }
                });
            }
        })
    }
}

// Bitta so'rovni boshqaradi: marshrut topish -> req qurish -> handler'ni
// spawn_blocking'da (sinxron interp) chaqirish -> javob.
async fn handle_request(
    interp: Arc<Interp>,
    req: Request<Incoming>,
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

    // Tanani yig'amiz.
    let body_bytes = match req.into_body().collect().await {
        Ok(c) => c.to_bytes(),
        Err(_) => Bytes::new(),
    };

    let request_value = build_req(method, path, query, headers, params, body_bytes, is_json);
    let handler = route.handler;

    // Sinxron interp ishini blocking thread'da bajaramiz — tokio worker'ini
    // bloklamaydi, har request alohida thread'da -> haqiqiy parallel.
    let result =
        tokio::task::spawn_blocking(move || interp.apply(handler, vec![request_value])).await;

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
type PooledHttpClient = Client<HttpConnector, ClientBody>;

// Klient so'rovlari uchun bir martalik global runtime (Flux skripti sinxron).
fn client_runtime() -> &'static tokio::runtime::Runtime {
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
fn pooled_http_client() -> PooledHttpClient {
    static CLIENT: OnceLock<PooledHttpClient> = OnceLock::new();
    CLIENT
        .get_or_init(|| Client::builder(TokioExecutor::new()).build_http())
        .clone()
}

// http.get url  /  http.post url body
fn http_client(method: &str, args: Vec<Value>) -> Result<Value, Flow> {
    let url = match args.first() {
        Some(Value::Str(s)) => s.clone(),
        _ => {
            return Err(Flow::err(format!(
                "http.{}: url (str) kerak",
                method.to_lowercase()
            )));
        }
    };
    let body = args.get(1).cloned();

    client_runtime().block_on(async move {
        let uri: hyper::Uri = url
            .parse()
            .map_err(|e| Flow::err(format!("noto'g'ri url: {}", e)))?;

        let (body_str, is_json) = match &body {
            Some(Value::Map(_)) | Some(Value::List(_)) => {
                (json_encode(body.as_ref().unwrap()), true)
            }
            Some(Value::Str(s)) => (s.clone(), false),
            Some(other) => (format!("{}", other), false),
            None => (String::new(), false),
        };

        let mut builder = Request::builder().method(method).uri(uri);
        if is_json {
            builder = builder.header("content-type", "application/json");
        }
        let req = builder
            .body(Full::new(Bytes::from(body_str)))
            .map_err(|e| Flow::err(format!("so'rov qurish: {}", e)))?;

        let resp = pooled_http_client()
            .request(req)
            .await
            .map_err(|e| Flow::err(format!("http so'rov: {}", e)))?;

        let status = resp.status().as_u16() as i64;
        let resp_is_json = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.contains("application/json"))
            .unwrap_or(false);

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
        m.insert("status".to_string(), Value::Int(status));
        m.insert("body".to_string(), resp_body);
        Ok(Value::Map(m))
    })
}
