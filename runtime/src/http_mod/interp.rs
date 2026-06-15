// Interp HTTP dispatch: http.<func> registration calls (on/use/before/cors/
// static/limit/serve) and the client calls (get/post/put/del).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::interp::{Flow, Interp};
use crate::value::Value;

use super::client::http_client;
use super::limits::{LimitBucket, window_to_secs};
use super::middleware::{CorsConfig, Middleware, MwKind};
use super::routing::{Route, normalize_method, parse_pattern};
use super::server::DEFAULT_MAX_BODY;
use super::static_files::{StaticMount, parse_static_prefix};

impl Interp {
    // http.<func> calls. eval_call routes here.
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
            Some(Value::Sym(s)) | Some(Value::Str(s)) => normalize_method(s),
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

    // http.use \req -> ...  — global middleware for all routes (issue #67).
    // Multiple calls form a chain (running in declaration order).
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

    // http.before "/api/*" \req -> ...  — middleware by path prefix (#67).
    // Pattern "/api/*" -> paths starting with /api; without "*" -> exact match.
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

    // http.cors origins [opts]  — declarative CORS (issue #135).
    //
    //   http.cors "*"                                # open to all (dev)
    //   http.cors ["https://app.example.com"]        # allowed origins
    //   http.cors ["https://app.example.com"] {creds: true}
    //
    // 1st arg: "*" (str) — any origin, or a list of origins.
    // 2nd arg (optional): an options map:
    //   creds:   true -> Allow-Credentials (cookie/Authorization). When combined
    //            with "*" the response reflects the request origin (browser rule).
    //   methods: allowed methods (str). A wide default set.
    //   headers: allowed request headers (str). A wide default set.
    //   max_age: preflight cache duration in seconds (int). Default 86400 (1 day).
    fn http_cors(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let origins = match args.first() {
            // "*" — any origin (None internally).
            Some(Value::Str(s)) if s == "*" => None,
            // Accept a single origin as a str too (convenience).
            Some(Value::Str(s)) => Some(vec![s.clone()]),
            // A list of origins.
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
            // A wide default set — works without the agent configuring it.
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

    // http.static prefix dir [opts]  — serve static files from a folder (#134).
    //
    //   http.static "/assets" "./public"        # /assets/app.css -> ./public/app.css
    //   http.static "/" "./dist" {spa: true}    # if not found -> ./dist/index.html
    //
    // The directory is resolved relative to the script file's directory (the same
    // rule as `use ./file`) and canonicalized at registration — a missing
    // directory errors at startup (fail fast instead of a silent 404 at deploy
    // time). Content-Type is automatic from the extension; `../` traversal (also
    // percent-encoded) is mandatorily blocked; route priority: exact route > static.
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

    // http.limit [path] N :sec|:min|:hr \req -> key  — declarative rate-limit (#79).
    //
    //   http.limit 100 :min \req -> req.ctx.tenant_id          # per-tenant, all paths
    //   http.limit "/api/*" 100 :min \req -> req.headers.x_api_key  # per-key, prefix
    //
    // Path (str) is an optional 1st arg — if present it attaches by prefix like
    // http.before, otherwise it is global like http.use. The key function is
    // called per request to identify the client; if it returns nil/empty we fall
    // back to req.ip. On exceeding the limit, an automatic `429` + `Retry-After`
    // (seconds until the window ends).
    fn http_limit(&self, args: Vec<Value>) -> Result<Value, Flow> {
        // If the 1st arg is a str — path scope (like http.before). Otherwise global.
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

    // http.serve port — a blocking tokio multi-thread server.
    // `http.serve PORT` does NOT block immediately; instead it adds to the list
    // of pending servers (deferred). After top-level code finishes
    // (`serve_mod::run_pending`) they are all spawned on ONE shared tokio
    // runtime — so HTTP + WS run together in one process.
    fn http_serve(self: &Arc<Self>, args: Vec<Value>) -> Result<Value, Flow> {
        let port = match args.first() {
            Some(Value::Int(n)) => *n as u16,
            _ => return Err(Flow::err("http.serve: port (int) is required")),
        };
        // Optional second argument — an options map: `{max_body: BYTES}`.
        // If omitted, default DEFAULT_MAX_BODY; `max_body: 0` disables the limit.
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
