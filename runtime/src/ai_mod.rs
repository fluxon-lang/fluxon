// Fluxon ai battery — the LLM primitive (Anthropic Claude + OpenAI GPT).
//
// Language API (docs/fluxon-agent.md):
//   txt = ai.ask "savol ${x}"                         # -> text (str)
//   r   = ai.json "extract: ${t}" {intent::a items:[...]}   # -> map + r._ metadata
//   r   = ai.run msgs tools                           # ONE step of the tool-loop
//
// Metadata (the `_` field in the `ai.json` result):
//   r._.conf   (0..1)   — the model's confidence (estimated from stop/finish_reason)
//   r._.tokens (int)    — sum of input+output tokens
//   r._.cost   (flt)    — estimated cost (USD), from the model's price table
//   r._.ms     (int)    — request duration in milliseconds
//
// Philosophy: "the language adapts to AI". `ai.run` returns EXACTLY one step
// (it does NOT run the tool itself) — the loop stays in the user's hands
// (log/cost/approval control). You call the tool on the Fluxon side via
// `reg.call` and append the result to msgs.
//
// PROVIDER AUTO-DETECT (the Fluxon user configures nothing):
//   - if ANTHROPIC_API_KEY is in `.env`/the environment -> Claude (default claude-opus-4-8)
//   - if OPENAI_API_KEY -> GPT (default gpt-4o)
//   - if both, Anthropic wins. Override: $AI_PROVIDER (anthropic|openai).
//   - $AI_KEY — a generic override key regardless of provider.
//   - Model: $AI_MODEL ?? provider default.
// The internal shape is built in the Anthropic Messages form; for OpenAI it is
// automatically converted to the Chat Completions form before the call
// (msgs/tools/response).
//
// There is no official Rust SDK -> raw https POST (reuses the `http` battery's
// client/pool). Stateless battery: reads env + sends a POST. But it needs
// Interp to fetch the key via `env_lookup` -> the `ai_dispatch` `&self` method.

use std::collections::BTreeMap;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Request;

use crate::builtins::{json_decode, json_encode};
use crate::http_mod::{client_runtime, pooled_http_client};
use crate::interp::{Flow, Interp};
use crate::value::Value;

const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const ANTHROPIC_DEFAULT_MODEL: &str = "claude-opus-4-8";

const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";
const OPENAI_DEFAULT_MODEL: &str = "gpt-4o";

// Response length limit. Enough for token-budget safety, but not unbounded.
// To keep `ai.ask`/`ai.json` semantics simple it is not configurable for now
// (may be exposed via opts in the future).
const MAX_TOKENS: i64 = 4096;

// Default timeout for an LLM request (issue #92). Without a timeout, a stuck
// LLM endpoint would block the whole script FOREVER (or, when called inside an
// HTTP handler, that request thread). LLM responses can be slow, so it is
// larger than the client's (30s): default 120s. Configured with `$AI_TIMEOUT`
// (seconds); 0 or negative — no timeout.
const DEFAULT_AI_TIMEOUT_SECS: u64 = 120;

// Supported LLM providers. The battery detects it ITSELF (auto) — the Fluxon
// user configures nothing: a standard provider key in `.env`
// (ANTHROPIC_API_KEY / OPENAI_API_KEY) is enough.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Provider {
    Anthropic,
    OpenAI,
}

impl Provider {
    fn default_model(self) -> &'static str {
        match self {
            Provider::Anthropic => ANTHROPIC_DEFAULT_MODEL,
            Provider::OpenAI => OPENAI_DEFAULT_MODEL,
        }
    }

    // Wire-style name (`:anthropic` / `:openai`) -> Provider. Used by `ai.config`
    // / per-call opts `style` and `$AI_STYLE`. This selects the REQUEST/RESPONSE
    // FORMAT, independent of which key/model is used: a GLM endpoint speaks the
    // OpenAI wire format, so `style::openai` + a custom `url` is enough.
    fn from_style(s: &str) -> Option<Provider> {
        match s.to_lowercase().as_str() {
            "anthropic" | "claude" => Some(Provider::Anthropic),
            "openai" | "gpt" => Some(Provider::OpenAI),
            _ => None,
        }
    }
}

// Overrides for the request shape (issue #199). Every field is optional; the
// default (all None / empty) leaves the battery byte-for-byte unchanged. Set
// globally by `ai.config {...}` (stored on Interp) and/or per-call via the
// trailing opts map. A per-call opts value wins over the global one, which in
// turn wins over the env/auto-detect default.
//
// `headers` / `extra` MERGE key-by-key onto the defaults (so you can add the
// single header a gateway needs without restating the auth header); `url` /
// `style` / `model` / `key` REPLACE the corresponding default.
#[derive(Clone, Default)]
pub struct AiOverride {
    // Full endpoint URL (replaces ANTHROPIC_URL / OPENAI_URL). For GLM/Z.AI,
    // OpenRouter, Ollama, vLLM, Azure, ...
    url: Option<String>,
    // Wire format / provider style. Decouples "which format" from "which key".
    style: Option<Provider>,
    // API key override (same role as $AI_KEY, but inline).
    key: Option<String>,
    // Model override (same role as $AI_MODEL, but inline).
    model: Option<String>,
    // Extra HTTP headers, merged onto the provider's fixed set (a value here
    // overrides a default header of the same name).
    headers: BTreeMap<String, String>,
    // Extra request-body fields, merged into the request JSON (e.g. OpenRouter's
    // `provider`/`route`/`transforms`). A key here overrides a default body field.
    extra: BTreeMap<String, Value>,
}

impl AiOverride {
    // Layers `other` ON TOP of `self`: scalar fields from `other` (when set)
    // replace; `headers`/`extra` merge key-by-key with `other` winning. Used to
    // fold the per-call opts over the global `ai.config`, AND (issue #200) to
    // fold a partial `ai.config {..}` over the stored config.
    fn merge(&self, other: &AiOverride) -> AiOverride {
        let mut out = self.clone();
        // SECURITY (issue #200, Codex P1 + review): an inherited auth header is
        // only valid for the host/credential it was set for. If THIS merge
        // CHANGES the target — a different `key`, `url`, or `style` — any auth
        // header carried over from `self` is STALE and must be dropped, or it
        // would be sent to the new host (a cross-host credential leak) and, via
        // `header_overridden`, suppress the auth header the new target should
        // generate. The leak is reachable by a url- or style-only switch too, not
        // just `key`.
        //
        // We compare the NEW value against `self` (not mere presence): restating
        // the SAME url/style/key while changing only e.g. `model` is NOT a
        // retarget, so it must keep the inherited auth header — otherwise a
        // reusable profile map `{url style key model}` would lose its
        // `headers.authorization` on every `/model` switch (Codex P2). Explicit
        // auth headers RESTATED by this switch survive — the re-insert loop below
        // runs AFTER the drop.
        let changed = |new: &Option<String>, cur: &Option<String>| new.is_some() && new != cur;
        let retargets = changed(&other.key, &self.key)
            || changed(&other.url, &self.url)
            || (other.style.is_some() && other.style != self.style);
        if retargets {
            out.headers.retain(|k, _| !is_auth_header(k));
            // Also invalidate the inherited `key` field. SECURITY (Codex P1
            // round 3): a switch like `ai.config {url:new key:env.NEW_KEY}` where
            // NEW_KEY is unset parses `key` to nil -> `other.key` is None (nil is
            // skipped for ergonomics). Without this, the partial merge would keep
            // the PREVIOUS host's key and `ai_config` would send it to the new
            // host. On a retarget the credential is host-scoped, so a missing new
            // key must NOT silently reuse the old one — it falls through to the
            // env / errors (same as the old replace semantics). If `other.key` IS
            // set, the line below restores it (the explicit new key wins).
            out.key = None;
        }
        if other.url.is_some() {
            out.url = other.url.clone();
        }
        if other.style.is_some() {
            out.style = other.style;
        }
        if other.key.is_some() {
            out.key = other.key.clone();
        }
        if other.model.is_some() {
            out.model = other.model.clone();
        }
        for (k, v) in &other.headers {
            out.headers.insert(k.clone(), v.clone());
        }
        for (k, v) in &other.extra {
            out.extra.insert(k.clone(), v.clone());
        }
        out
    }
}

// Parses an opts map ({url style key model headers extra}) into an AiOverride.
// Unknown keys are an explicit error — a typo like `{ur:...}` should fail loudly
// rather than be silently ignored. Shared by `ai.config` and per-call opts.
fn parse_override(opts: &BTreeMap<String, Value>) -> Result<AiOverride, Flow> {
    let mut ov = AiOverride::default();
    for (k, v) in opts {
        // A `nil` value means "not set" — skip it, so `key: env.MAYBE_UNSET`
        // falls through to the env/default instead of erroring. This keeps the
        // common `{key: env.X}` pattern ergonomic when X is absent.
        if matches!(v, Value::Nil) {
            continue;
        }
        match k.as_str() {
            "url" => ov.url = Some(require_str(v, "url")?),
            "style" => {
                let s = match v {
                    Value::Str(s) | Value::Sym(s) => s.clone(),
                    _ => return Err(Flow::err("ai: opts.style must be :anthropic|:openai")),
                };
                ov.style = Some(Provider::from_style(&s).ok_or_else(|| {
                    Flow::err(format!("ai: unknown style '{}' (anthropic|openai)", s))
                })?);
            }
            "key" => ov.key = Some(require_str(v, "key")?),
            "model" => ov.model = Some(require_str(v, "model")?),
            "headers" => match v {
                Value::Map(m) => {
                    for (hk, hv) in m {
                        // Header NAMES are normalized to lowercase: HTTP header
                        // names are case-insensitive, so without this a global
                        // `Authorization` and a per-call `authorization` would be
                        // two distinct BTreeMap keys — the merge would keep both
                        // and `add_extra_headers` would send DUPLICATE headers
                        // (the override would not actually win). Lowercase on the
                        // wire is always valid (and required by HTTP/2).
                        // Header VALUES are strings; other scalars are coerced to
                        // their text form (`to_text`, so a symbol value does NOT
                        // keep its `:` prefix — `X-Title::app` sends `app`).
                        ov.headers.insert(hk.to_lowercase(), hv.to_text());
                    }
                }
                _ => return Err(Flow::err("ai: opts.headers must be a map")),
            },
            "extra" => match v {
                Value::Map(m) => {
                    for (ek, ev) in m {
                        ov.extra.insert(ek.clone(), ev.clone());
                    }
                }
                _ => return Err(Flow::err("ai: opts.extra must be a map")),
            },
            other => {
                return Err(Flow::err(format!(
                    "ai: unknown opts key '{}' (url/style/key/model/headers/extra)",
                    other
                )));
            }
        }
    }
    Ok(ov)
}

// Reads a Value expected to be a string (for url/key/model opts).
fn require_str(v: &Value, field: &str) -> Result<String, Flow> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        _ => Err(Flow::err(format!("ai: opts.{} must be a string", field))),
    }
}

// Merges the `extra` body fields into the request body. A key present in both
// is OVERRIDDEN by the user's value — this is the escape hatch for providers
// that need vendor-specific fields (and lets a user tune even `max_tokens`).
fn merge_extra(body: &mut BTreeMap<String, Value>, extra: &BTreeMap<String, Value>) {
    for (k, v) in extra {
        body.insert(k.clone(), v.clone());
    }
}

// Finalizes the request headers: the default `content-type: application/json`
// (unless the user overrode it) followed by the extra/override headers. The
// caller has already emitted the provider's auth/version headers (guarded the
// same way). hyper APPENDS duplicate headers rather than replacing, so every
// default we emit must first be checked against `headers` — otherwise an
// override would send two values. Skips empty names defensively.
fn add_extra_headers(
    mut b: hyper::http::request::Builder,
    headers: &BTreeMap<String, String>,
) -> hyper::http::request::Builder {
    if !header_overridden(headers, "content-type") {
        b = b.header("content-type", "application/json");
    }
    for (k, v) in headers {
        if !k.is_empty() {
            b = b.header(k.as_str(), v.as_str());
        }
    }
    b
}

// True if `name` (a header key, stored lowercased) is an authentication-bearing
// header — one whose value is a credential scoped to a specific host/key. Used by
// `merge` to drop STALE inherited auth headers when a config switch retargets the
// request (a non-auth header like `x-title`/`http-referer` is host-agnostic and
// carries over). Covers the standard provider names plus the common gateway/cloud
// schemes (Azure `api-key`, Google `x-goog-api-key`) — not every conceivable
// custom name, but the realistic ones, so a `/model`/`/host` switch does not
// silently forward the previous deployment's credential.
fn is_auth_header(name: &str) -> bool {
    matches!(
        name,
        "authorization" | "x-api-key" | "api-key" | "x-goog-api-key"
    )
}

// True if `headers` contains `name`. Override header names are stored lowercased
// (see `parse_override`), and every `name` passed here is already lowercase, so a
// case-insensitive compare is belt-and-suspenders — it keeps the guard correct
// even if a header reaches the map by another path.
fn header_overridden(headers: &BTreeMap<String, String>, name: &str) -> bool {
    headers.keys().any(|k| k.eq_ignore_ascii_case(name))
}

// Detected provider + key + model (the auto-detect result), already folded with
// the overrides — `url`/`headers`/`extra` are the EFFECTIVE request pieces.
struct AiConfig {
    provider: Provider,
    key: String,
    model: String,
    // Effective endpoint URL (override ?? env ?? provider default).
    url: String,
    // Extra headers to merge onto the provider's fixed set.
    headers: BTreeMap<String, String>,
    // Extra body fields to merge into the request JSON.
    extra: BTreeMap<String, Value>,
}

impl Interp {
    // ai.ask / ai.json / ai.run dispatch.
    pub fn ai_dispatch(&self, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            "ask" => self.ai_ask(args),
            "json" => self.ai_json(args),
            "run" => self.ai_run(args),
            "stream" => self.ai_stream(args),
            "config" => self.ai_config_set(args),
            _ => Err(Flow::err(format!(
                "ai.{} not found (ask/json/run/stream/config)",
                func
            ))),
        }
    }

    // ai.config {url:.. style:.. key:.. model:.. headers:{..} extra:{..}}
    // Sets the GLOBAL request overrides for every following `ai.*` call (issues
    // #199, #200). A top-level setup call (like `http.cors`), but ALSO the
    // runtime primitive a `/model` command is built on (issue #200): it can be
    // called again at any point to switch provider/model/key on the fly — the
    // next `ai.ask`/`ai.json`/`ai.run` uses the new configuration.
    //
    // SEMANTICS (issue #200): a non-empty `ai.config {..}` is a PARTIAL update —
    // the given fields are merged ON TOP of the stored config, so
    // `ai.config {model: pick}` switches only the model and keeps the key/url/
    // style/headers/extra already set. (`headers`/`extra` merge key-by-key, like
    // the per-call merge.) This is exactly what `/model` needs: list choices,
    // the user picks one, apply just that field. To REPLACE/clear everything and
    // fall back to the env defaults, call `ai.config {}` (or `ai.config nil`)
    // with no fields. Returns nil.
    fn ai_config_set(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let mut cur = self.ai_override.lock().unwrap();
        match args.first() {
            // A map with at least one field -> PARTIAL merge over the current
            // config (model/key/... switch). An EMPTY map -> reset (clear).
            Some(Value::Map(m)) if !m.is_empty() => {
                let ov = parse_override(m)?;
                *cur = cur.merge(&ov);
            }
            // No opts / nil / empty map -> clear back to the env/auto defaults.
            None | Some(Value::Nil) | Some(Value::Map(_)) => {
                *cur = AiOverride::default();
            }
            _ => return Err(Flow::err("ai.config: opts (map) required".to_string())),
        }
        Ok(Value::Nil)
    }

    // Fetches a value from env (OS env > .env), Some if non-empty.
    fn ai_env(&self, name: &str) -> Option<String> {
        match self.env_lookup(name) {
            Value::Str(s) if !s.is_empty() => Some(s),
            _ => None,
        }
    }

    // LLM request timeout: $AI_TIMEOUT (seconds) ?? default 120s. Issue #92:
    // a stuck endpoint must not block the thread forever.
    fn ai_timeout(&self) -> Option<Duration> {
        resolve_ai_timeout(self.ai_env("AI_TIMEOUT").as_deref())
    }

    // Folds the global `ai.config` override with the per-call opts: per-call
    // wins. Either may be empty (the default), in which case this is a no-op and
    // the request shape stays byte-for-byte unchanged.
    fn effective_override(&self, per_call: &AiOverride) -> AiOverride {
        let global = self.ai_override.lock().unwrap().clone();
        global.merge(per_call)
    }

    // AUTO-detects provider + key + model, then folds in the overrides (env +
    // global `ai.config` + per-call opts). Nothing is mandatory — the order:
    //   1) the wire style: opts/config `style` ?? $AI_STYLE ?? $AI_PROVIDER ??
    //      auto-detect from the available standard key.
    //   2) the URL: opts/config `url` ?? $AI_BASE_URL ?? provider default.
    //   3) the key: opts/config `key` ?? $AI_KEY ?? (ONLY on the default URL)
    //      the provider's standard key.
    //   4) the model: opts/config `model` ?? $AI_MODEL ?? provider default.
    // `style` only selects the request/response FORMAT (so a GLM endpoint can use
    // the OpenAI format with a custom URL) — it does NOT force a particular key.
    //
    // SECURITY: the standard provider keys ($OPENAI_API_KEY / $ANTHROPIC_API_KEY)
    // that we read from the environment are sent ONLY to that provider's OWN
    // default endpoint. A CUSTOM url (override or $AI_BASE_URL) requires an
    // EXPLICIT key (inline `key` / $AI_KEY) — we never fall back to a provider
    // key for a custom host, so the auto-detected env key can't leak to e.g.
    // Z.AI/OpenRouter. Custom url + no explicit key => clear error.
    // NOTE: this guards the AUTO-DETECTED key only. `headers`/`extra` are an
    // explicit, unguarded escape hatch — whatever a user puts there (including an
    // auth header) is sent as-is to whatever url they chose; that's on them.
    fn ai_config(&self, ov: &AiOverride) -> Result<AiConfig, Flow> {
        let anthropic = self.ai_env("ANTHROPIC_API_KEY");
        let openai = self.ai_env("OPENAI_API_KEY");
        // Explicit, provider-agnostic key (the GLM/OpenRouter case: only `key` +
        // `url`). The INLINE override wins over $AI_KEY — consistent with
        // url/style/model, and required so a per-call/config `{key:...}` can
        // actually retarget a deployment that already has $AI_KEY set (otherwise
        // it would keep authenticating against the env key / wrong provider).
        let explicit_key = ov.key.clone().or_else(|| self.ai_env("AI_KEY"));
        // Wire style: the inline `style` override wins, then $AI_STYLE, then the
        // legacy $AI_PROVIDER (kept for backward compatibility).
        let forced_style = self
            .ai_env("AI_STYLE")
            .or_else(|| self.ai_env("AI_PROVIDER"));

        // Determine the provider (= wire style).
        let provider = if let Some(p) = ov.style {
            p
        } else {
            match forced_style.as_deref() {
                Some(s) => Provider::from_style(s).ok_or_else(|| {
                    Flow::err(format!("ai: unknown style '{}' (anthropic|openai)", s))
                })?,
                // No style given -> detect from the available standard key.
                // Anthropic wins (the project is oriented toward Claude), then
                // OpenAI. If only $AI_KEY/inline key is set, assume Anthropic.
                None => {
                    if anthropic.is_some() {
                        Provider::Anthropic
                    } else if openai.is_some() {
                        Provider::OpenAI
                    } else if explicit_key.is_some() {
                        Provider::Anthropic
                    } else {
                        return Err(Flow::err(
                            "ai: API key not found — set ANTHROPIC_API_KEY or \
                             OPENAI_API_KEY in .env or the environment"
                                .to_string(),
                        ));
                    }
                }
            }
        };

        // URL: inline override ?? $AI_BASE_URL ?? provider default. Resolved
        // BEFORE the key, because a custom URL changes how the key is resolved.
        let default_url = match provider {
            Provider::Anthropic => ANTHROPIC_URL,
            Provider::OpenAI => OPENAI_URL,
        };
        let custom_url = ov.url.clone().or_else(|| self.ai_env("AI_BASE_URL"));
        let is_custom = custom_url.is_some();
        let url = custom_url.unwrap_or_else(|| default_url.to_string());

        // Key resolution (security-critical) is a pure function so it can be
        // tested without touching the environment.
        let provider_key = match provider {
            Provider::Anthropic => anthropic,
            Provider::OpenAI => openai,
        };
        let key = resolve_key(is_custom, explicit_key, provider_key, provider)?;

        // Model: inline override ?? $AI_MODEL ?? provider default.
        let model = ov
            .model
            .clone()
            .or_else(|| self.ai_env("AI_MODEL"))
            .unwrap_or_else(|| provider.default_model().to_string());

        Ok(AiConfig {
            provider,
            key,
            model,
            url,
            headers: ov.headers.clone(),
            extra: ov.extra.clone(),
        })
    }

    // ai.ask "savol" [opts] -> response text (str).
    // Sends a single user message, returns the first text block. The optional
    // trailing opts map ({url style key model headers extra}) overrides the
    // global `ai.config` for this one call (issue #199).
    fn ai_ask(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let prompt = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => return Err(Flow::err("ai.ask: question (str) required".to_string())),
        };
        let ov = self.per_call_override(args.get(1))?;
        let messages = vec![user_msg(&prompt)];
        let resp = self.call_api(&messages, None, None, &ov)?;
        Ok(Value::Str(resp.text))
    }

    // ai.stream "prompt" \chunk -> ... [opts] -> the full response text (str).
    // The token-by-token variant of `ai.ask` (issue #201): the callback is
    // invoked with each text chunk as it streams in (print it, `ws.send` it,
    // accumulate it), and the accumulated full text is returned at the end — so a
    // caller can append it to the conversation history just like `ai.ask`.
    //
    // The callback runs on the CALLING (Fluxon) thread, NOT inside the async
    // task: the SSE reader streams chunks over a std mpsc channel and this thread
    // drains it, calling `apply` per chunk. That keeps `Value: Send + Sync`
    // intact (no Fluxon value crosses into the async runtime) and means a
    // callback that does `ws.send`/`io.print` runs in the normal sync context.
    fn ai_stream(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let prompt = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => return Err(Flow::err("ai.stream: question (str) required".to_string())),
        };
        // The 2nd arg is the per-chunk callback (a fn/lambda). It is required —
        // without it `ai.stream` would be just a slower `ai.ask`.
        let cb = match args.get(1) {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => {
                return Err(Flow::err(
                    "ai.stream: a callback `\\chunk -> ...` is required (arg 2)".to_string(),
                ));
            }
        };
        let ov = self.per_call_override(args.get(2))?;
        let messages = vec![user_msg(&prompt)];
        let resp = self.call_api_stream(&messages, None, &ov, &cb)?;
        Ok(Value::Str(resp.text))
    }

    // Like `call_api`, but streams the response. Resolves the provider/config the
    // same way, then dispatches to the provider's SSE reader. `on_chunk` is
    // called (on this thread) for each text delta.
    fn call_api_stream(
        &self,
        messages: &[Value],
        system: Option<&str>,
        per_call: &AiOverride,
        on_chunk: &Value,
    ) -> Result<AiResp, Flow> {
        let cfg = self.ai_config(&self.effective_override(per_call))?;
        let (body, headers) = match cfg.provider {
            Provider::Anthropic => self.build_anthropic_stream_req(&cfg, messages, system),
            Provider::OpenAI => self.build_openai_stream_req(&cfg, messages, system),
        };
        self.run_stream(&cfg, body, headers, on_chunk)
    }

    // Builds the Anthropic streaming request body + the auth/version headers.
    // Same shape as `call_anthropic`, plus `stream: true`.
    fn build_anthropic_stream_req(
        &self,
        cfg: &AiConfig,
        messages: &[Value],
        system: Option<&str>,
    ) -> (String, BTreeMap<String, String>) {
        let mut body = BTreeMap::new();
        body.insert("model".to_string(), Value::Str(cfg.model.clone()));
        body.insert("max_tokens".to_string(), Value::Int(MAX_TOKENS));
        body.insert("messages".to_string(), Value::List(messages.to_vec()));
        body.insert("stream".to_string(), Value::Bool(true));
        if let Some(sys) = system {
            body.insert("system".to_string(), Value::Str(sys.to_string()));
        }
        merge_extra(&mut body, &cfg.extra);
        // Resolve the default auth/version headers UNLESS the user overrode them,
        // so the streaming path uses the same precedence as `call_anthropic`. We
        // fold them into one header map here (the reader is provider-agnostic).
        let mut headers = cfg.headers.clone();
        if !header_overridden(&headers, "x-api-key") {
            headers.insert("x-api-key".to_string(), cfg.key.clone());
        }
        if !header_overridden(&headers, "anthropic-version") {
            headers.insert(
                "anthropic-version".to_string(),
                ANTHROPIC_VERSION.to_string(),
            );
        }
        (json_encode(&Value::Map(body)), headers)
    }

    // Builds the OpenAI streaming request body + the Bearer auth header.
    // `stream_options.include_usage` asks OpenAI to emit a final usage chunk
    // (otherwise streamed responses carry no token counts).
    fn build_openai_stream_req(
        &self,
        cfg: &AiConfig,
        messages: &[Value],
        system: Option<&str>,
    ) -> (String, BTreeMap<String, String>) {
        let mut oa_msgs: Vec<Value> = Vec::new();
        if let Some(sys) = system {
            let mut m = BTreeMap::new();
            m.insert("role".to_string(), Value::Str("system".to_string()));
            m.insert("content".to_string(), Value::Str(sys.to_string()));
            oa_msgs.push(Value::Map(m));
        }
        oa_msgs.extend(messages.iter().map(anthropic_msg_to_openai));

        let mut body = BTreeMap::new();
        body.insert("model".to_string(), Value::Str(cfg.model.clone()));
        body.insert("max_tokens".to_string(), Value::Int(MAX_TOKENS));
        body.insert("messages".to_string(), Value::List(oa_msgs));
        body.insert("stream".to_string(), Value::Bool(true));
        let mut so = BTreeMap::new();
        so.insert("include_usage".to_string(), Value::Bool(true));
        body.insert("stream_options".to_string(), Value::Map(so));
        merge_extra(&mut body, &cfg.extra);

        let mut headers = cfg.headers.clone();
        if !header_overridden(&headers, "authorization") {
            headers.insert("authorization".to_string(), format!("Bearer {}", cfg.key));
        }
        (json_encode(&Value::Map(body)), headers)
    }

    // Drives the actual streaming request: spawns the SSE reader on the shared
    // client runtime, drains text chunks over a channel (calling `on_chunk` on
    // this thread per chunk), then joins the task for the final AiResp.
    fn run_stream(
        &self,
        cfg: &AiConfig,
        body: String,
        headers: BTreeMap<String, String>,
        on_chunk: &Value,
    ) -> Result<AiResp, Flow> {
        use std::sync::mpsc::channel;
        let (tx, rx) = channel::<String>();
        let url = cfg.url.clone();
        let model = cfg.model.clone();
        let provider = cfg.provider;
        let timeout = self.ai_timeout();

        // Spawn the reader on the shared client runtime. It owns the channel
        // sender; when it finishes (or errors) the sender drops and `rx` closes.
        let handle = client_runtime().spawn(async move {
            read_sse_stream(&url, body, headers, timeout, provider, &model, tx).await
        });

        // Drain on THIS thread, calling the Fluxon callback per chunk. If the
        // callback raises (fail/error), we stop draining and propagate — the task
        // is aborted so a half-read body does not linger.
        for chunk in &rx {
            if let Err(flow) = self.apply(on_chunk.clone(), vec![Value::Str(chunk)]) {
                handle.abort();
                return Err(flow);
            }
        }

        // The channel closed — the reader is done. Join it for the final result
        // (full text + usage). block_on is safe here: we are on the sync thread,
        // and the task has already finished (or is finishing).
        match client_runtime().block_on(handle) {
            Ok(r) => r,
            Err(e) => Err(Flow::err(format!("ai: stream task failed: {}", e))),
        }
    }

    // Parses an OPTIONAL trailing opts argument into a per-call override. Absent
    // / nil -> the default (no override), so the request shape is unchanged for
    // the common case where no opts are passed.
    fn per_call_override(&self, opts: Option<&Value>) -> Result<AiOverride, Flow> {
        match opts {
            None | Some(Value::Nil) => Ok(AiOverride::default()),
            Some(Value::Map(m)) => parse_override(m),
            Some(_) => Err(Flow::err("ai: opts must be a map".to_string())),
        }
    }

    // ai.json "prompt" {schema} -> map (+ `_` metadata).
    // We add the schema map to the prompt and ask the model for ONLY JSON; we
    // parse the response into a map and add the `_` (metadata) field.
    fn ai_json(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let prompt = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => return Err(Flow::err("ai.json: prompt (str) required".to_string())),
        };
        let schema = match args.get(1) {
            Some(v @ (Value::Map(_) | Value::List(_))) => v.clone(),
            _ => return Err(Flow::err("ai.json: schema (map/list) required".to_string())),
        };
        // Optional trailing opts (after the schema) — per-call override.
        let ov = self.per_call_override(args.get(2))?;

        // Explicit instruction to the model: return only JSON MATCHING the given
        // shape. The system prompt forces pure JSON (prefill gives a 400 on 4.6+).
        let system = format!(
            "Return the response STRICTLY matching this JSON shape. JSON only, NO comments/text.\nShape: {}",
            json_encode(&schema)
        );
        let messages = vec![user_msg(&prompt)];
        let resp = self.call_api(&messages, Some(&system), None, &ov)?;

        // Parse the response text into a map. The model sometimes wraps it in
        // ```json ... ``` — we strip the code fence.
        let cleaned = strip_code_fence(&resp.text);
        let mut parsed = match json_decode(&cleaned) {
            Ok(Value::Map(m)) => m,
            Ok(other) => {
                // If it is JSON but not a map (for example a list) — place it under `value`.
                let mut m = BTreeMap::new();
                m.insert("value".to_string(), other);
                m
            }
            Err(_) => {
                return Err(Flow::err(format!(
                    "ai.json: model did not return JSON: {}",
                    truncate(&resp.text, 200)
                )));
            }
        };
        parsed.insert("_".to_string(), Value::Map(resp.meta()));
        Ok(Value::Map(parsed))
    }

    // ai.run msgs tools -> ONE step of the tool-loop.
    //   msgs:  [{role::user content:str} ...]  (role sym or str)
    //   tools: [{name desc params} ...]        (params — a JSON-schema map)
    // Result (the kind names are from the spec — docs/fluxon-human.md):
    //   :final -> {kind::final text:str}
    //   :call  -> {kind::call tool:str args:map id:str calls:[{tool args id} ...]}
    // If the model calls several tools in parallel, all are in the `calls` list
    // (each needs a tool_result returned). `tool`/`args`/`id` are the first call,
    // for backward compatibility — identical to the [0] element of `calls`.
    // (it does NOT run the tool itself — the loop is in the user's hands.)
    fn ai_run(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let msgs = match args.first() {
            Some(Value::List(l)) => l.clone(),
            _ => return Err(Flow::err("ai.run: msgs (list) required".to_string())),
        };
        let tools = match args.get(1) {
            Some(Value::List(l)) => Some(l.clone()),
            None | Some(Value::Nil) => None,
            _ => return Err(Flow::err("ai.run: tools (list) must be a list".to_string())),
        };
        // Optional trailing opts (after tools) — per-call override. To pass opts
        // without tools, use `ai.run msgs nil opts`.
        let ov = self.per_call_override(args.get(2))?;

        // msgs from the Fluxon shape to the Anthropic shape: {role content} -> {role, content}.
        // role may be a sym (:user) or str ("user"). The tool-result message
        // ({role::tool name content}) is also converted.
        let api_msgs: Vec<Value> = msgs.iter().map(normalize_msg).collect();

        let api_tools = tools
            .as_ref()
            .map(|t| t.iter().map(normalize_tool).collect());
        let resp = self.call_api(&api_msgs, None, api_tools.as_ref(), &ov)?;

        let mut out = BTreeMap::new();
        if !resp.tool_calls.is_empty() {
            out.insert("kind".to_string(), Value::Sym("call".to_string()));
            // Convert each call to a {tool args id} map.
            let calls: Vec<Value> = resp
                .tool_calls
                .iter()
                .map(|(tool, input, id)| {
                    let mut c = BTreeMap::new();
                    c.insert("tool".to_string(), Value::Str(tool.clone()));
                    c.insert("args".to_string(), input.clone());
                    c.insert("id".to_string(), Value::Str(id.clone()));
                    Value::Map(c)
                })
                .collect();
            // Backward compatibility: the first call as top-level `tool`/`args`/`id`.
            let (tool, input, id) = &resp.tool_calls[0];
            out.insert("tool".to_string(), Value::Str(tool.clone()));
            out.insert("args".to_string(), input.clone());
            out.insert("id".to_string(), Value::Str(id.clone()));
            out.insert("calls".to_string(), Value::List(calls));
        } else {
            out.insert("kind".to_string(), Value::Sym("final".to_string()));
            out.insert("text".to_string(), Value::Str(resp.text));
        }
        Ok(Value::Map(out))
    }

    // A POST request matching the provider. messages — a Fluxon-normalized
    // list; system/tools optional. The provider is auto-detected, and the
    // request/response format is chosen by provider too.
    fn call_api(
        &self,
        messages: &[Value],
        system: Option<&str>,
        tools: Option<&Vec<Value>>,
        per_call: &AiOverride,
    ) -> Result<AiResp, Flow> {
        let cfg = self.ai_config(&self.effective_override(per_call))?;
        match cfg.provider {
            Provider::Anthropic => self.call_anthropic(&cfg, messages, system, tools),
            Provider::OpenAI => self.call_openai(&cfg, messages, system, tools),
        }
    }

    // Anthropic /v1/messages: system at top-level, x-api-key header, tools in
    // the {name description input_schema} shape, content an array of blocks.
    fn call_anthropic(
        &self,
        cfg: &AiConfig,
        messages: &[Value],
        system: Option<&str>,
        tools: Option<&Vec<Value>>,
    ) -> Result<AiResp, Flow> {
        let mut body = BTreeMap::new();
        body.insert("model".to_string(), Value::Str(cfg.model.clone()));
        body.insert("max_tokens".to_string(), Value::Int(MAX_TOKENS));
        body.insert("messages".to_string(), Value::List(messages.to_vec()));
        if let Some(sys) = system {
            body.insert("system".to_string(), Value::Str(sys.to_string()));
        }
        if let Some(t) = tools {
            body.insert("tools".to_string(), Value::List(t.clone()));
        }
        // Merge extra body params LAST so a user-supplied field overrides ours.
        merge_extra(&mut body, &cfg.extra);
        let body_str = json_encode(&Value::Map(body));
        let key = cfg.key.clone();
        let extra_headers = cfg.headers.clone();

        let (text, ms) = post_json(&cfg.url, body_str, self.ai_timeout(), move |b| {
            // Default auth/version headers, unless the user overrode them by
            // name (hyper appends duplicates, so we must not emit both).
            let mut b = b;
            if !header_overridden(&extra_headers, "x-api-key") {
                b = b.header("x-api-key", key.as_str());
            }
            if !header_overridden(&extra_headers, "anthropic-version") {
                b = b.header("anthropic-version", ANTHROPIC_VERSION);
            }
            add_extra_headers(b, &extra_headers)
        })?;
        parse_anthropic(&text, &cfg.model, ms)
    }

    // OpenAI /v1/chat/completions: system as {role:system} within messages,
    // Authorization: Bearer, tools in the {type:function function:{...}} shape,
    // response choices[0].message.{content|tool_calls}.
    fn call_openai(
        &self,
        cfg: &AiConfig,
        messages: &[Value],
        system: Option<&str>,
        tools: Option<&Vec<Value>>,
    ) -> Result<AiResp, Flow> {
        // In OpenAI system is not separate — we prepend {role:system} to messages.
        let mut oa_msgs: Vec<Value> = Vec::new();
        if let Some(sys) = system {
            let mut m = BTreeMap::new();
            m.insert("role".to_string(), Value::Str("system".to_string()));
            m.insert("content".to_string(), Value::Str(sys.to_string()));
            oa_msgs.push(Value::Map(m));
        }
        oa_msgs.extend(messages.iter().map(anthropic_msg_to_openai));

        let mut body = BTreeMap::new();
        body.insert("model".to_string(), Value::Str(cfg.model.clone()));
        body.insert("max_tokens".to_string(), Value::Int(MAX_TOKENS));
        body.insert("messages".to_string(), Value::List(oa_msgs));
        if let Some(t) = tools {
            // Convert the Anthropic tool shape ({name description input_schema})
            // to the OpenAI function shape.
            let oa_tools: Vec<Value> = t.iter().map(anthropic_tool_to_openai).collect();
            body.insert("tools".to_string(), Value::List(oa_tools));
        }
        // Merge extra body params LAST so a user-supplied field overrides ours
        // (e.g. OpenRouter's `provider`/`route`/`transforms`).
        merge_extra(&mut body, &cfg.extra);
        let body_str = json_encode(&Value::Map(body));
        let key = cfg.key.clone();
        let extra_headers = cfg.headers.clone();

        let (text, ms) = post_json(&cfg.url, body_str, self.ai_timeout(), move |b| {
            // Default Bearer auth, unless the user overrode `authorization` (some
            // gateways want a different scheme / a custom auth header).
            let mut b = b;
            if !header_overridden(&extra_headers, "authorization") {
                b = b.header("authorization", format!("Bearer {}", key));
            }
            add_extra_headers(b, &extra_headers)
        })?;
        parse_openai(&text, &cfg.model, ms)
    }
}

// --- Streaming (SSE) — issue #201 ---

// Reads a streamed (SSE) LLM response. Sends each text delta over `tx` as it
// arrives, accumulates the full text + usage, and returns the final AiResp.
//
// Runs on the client tokio runtime (NOT the Fluxon thread): it does no Fluxon
// `apply` itself — `tx` carries plain Strings, so no Fluxon value crosses into
// async land and `Value: Send + Sync` is preserved.
//
// Unlike `post_json`, this does NOT retry: a stream is a long-lived connection,
// and once bytes have been delivered to the callback a retry would duplicate
// them. A non-2xx status is read as a (small) error body and returned as an error.
#[allow(clippy::too_many_arguments)]
async fn read_sse_stream(
    url: &str,
    body: String,
    headers: BTreeMap<String, String>,
    timeout: Option<Duration>,
    provider: Provider,
    model: &str,
    tx: std::sync::mpsc::Sender<String>,
) -> Result<AiResp, Flow> {
    let started = std::time::Instant::now();
    let work = async {
        let mut builder = Request::builder().method("POST").uri(url);
        // content-type default unless overridden, then every header (auth/version
        // for Anthropic, Bearer for OpenAI, plus user headers) — same dedup rule
        // as the non-stream path. SSE responses prefer `accept: text/event-stream`.
        if !header_overridden(&headers, "content-type") {
            builder = builder.header("content-type", "application/json");
        }
        if !header_overridden(&headers, "accept") {
            builder = builder.header("accept", "text/event-stream");
        }
        for (k, v) in &headers {
            if !k.is_empty() {
                builder = builder.header(k.as_str(), v.as_str());
            }
        }
        let req = builder
            .body(Full::new(Bytes::from(body)))
            .map_err(|e| Flow::err(format!("ai: building request: {}", e)))?;

        let resp = pooled_http_client()
            .request(req)
            .await
            .map_err(|e| Flow::err(format!("ai: network error: {}", e)))?;
        let status = resp.status().as_u16();
        let mut bodyr = resp.into_body();

        // Non-2xx: read the (usually small) error body and surface it — a stream
        // is not started, so there is nothing partial to undo.
        if !(200..300).contains(&status) {
            let bytes = bodyr
                .collect()
                .await
                .map(|c| c.to_bytes())
                .unwrap_or_default();
            return Err(Flow::err(format!(
                "ai: API error (status {}): {}",
                status,
                truncate(&String::from_utf8_lossy(&bytes), 300)
            )));
        }

        // Frame-by-frame read. Accumulate raw BYTES (not text) into a line buffer
        // and split on `\n` at the byte level — only decode a COMPLETE line as
        // UTF-8. Decoding each frame separately (`from_utf8_lossy` per frame)
        // would corrupt a multibyte char (emoji / CJK / Cyrillic) split across a
        // frame boundary into U+FFFD (Codex P2 #210, same class as the json
        // decoder fix). `\n` (0x0A) is ASCII and never appears inside a multibyte
        // UTF-8 sequence, so splitting on the byte is safe, and each full line is
        // a complete UTF-8 string (a char never crosses a line boundary in SSE).
        // Cap a single un-terminated line. A real SSE line is well under this; an
        // endless stream with NO `\n` would otherwise grow `buf` without bound
        // (OOM/DoS on the request thread — the timeout does not fire while bytes
        // keep arriving). 8 MiB is generous for any legitimate event.
        const MAX_SSE_LINE: usize = 8 * 1024 * 1024;
        let mut acc = SseAccumulator::new(provider);
        let mut buf: Vec<u8> = Vec::new();
        while let Some(frame) = bodyr.frame().await {
            let frame = frame.map_err(|e| Flow::err(format!("ai: reading stream: {}", e)))?;
            let Ok(data) = frame.into_data() else {
                continue; // trailers etc. — no body data
            };
            buf.extend_from_slice(&data);
            if buf.len() > MAX_SSE_LINE && !buf.contains(&b'\n') {
                return Err(Flow::err(format!(
                    "ai: stream line exceeded {} bytes without a newline",
                    MAX_SSE_LINE
                )));
            }
            // Emit every complete line; a trailing partial line stays in `buf`
            // (it may end mid-char — we wait for the rest in the next frame).
            while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
                let line_bytes: Vec<u8> = buf.drain(..=nl).collect();
                let line = String::from_utf8_lossy(&line_bytes);
                let line = line.trim_end_matches(['\r', '\n']);
                match acc.feed_line(line) {
                    Ok(Some(delta)) => {
                        // A best-effort send: if the receiver hung up (callback
                        // raised, the drain loop returned), stop reading.
                        if tx.send(delta).is_err() {
                            return acc.finish(model, 0);
                        }
                    }
                    Ok(None) => {}
                    // A provider error event (e.g. Anthropic `type:"error"`,
                    // overload mid-stream) — surface it instead of finishing with
                    // a partial/empty success.
                    Err(flow) => return Err(flow),
                }
            }
        }
        // Flush any final buffered line (some servers omit a trailing newline).
        if !buf.is_empty() {
            let line = String::from_utf8_lossy(&buf);
            match acc.feed_line(line.trim_end_matches(['\r', '\n'])) {
                Ok(Some(delta)) => {
                    let _ = tx.send(delta);
                }
                Ok(None) => {}
                Err(flow) => return Err(flow),
            }
        }
        acc.finish(model, started.elapsed().as_millis() as i64)
    };

    match timeout {
        Some(dur) => match tokio::time::timeout(dur, work).await {
            Ok(r) => r,
            Err(_) => Err(Flow::err(format!(
                "ai: stream timeout (no completion within {} sec)",
                dur.as_secs()
            ))),
        },
        None => work.await,
    }
}

// Incremental SSE parser. Accumulates the full text and the usage/stop info as
// events arrive; `feed_line` returns Some(delta) when a line yields new text.
//
// Both providers send `data: {json}` lines (Anthropic also sends `event:` lines,
// which we ignore — the JSON `type` field is enough to dispatch). OpenAI ends
// with `data: [DONE]`. We read text deltas and the usage chunk; everything else
// (ping, message_start, ...) is skipped.
struct SseAccumulator {
    provider: Provider,
    text: String,
    in_tokens: i64,
    out_tokens: i64,
    stop: String,
}

impl SseAccumulator {
    fn new(provider: Provider) -> Self {
        SseAccumulator {
            provider,
            text: String::new(),
            in_tokens: 0,
            out_tokens: 0,
            stop: String::new(),
        }
    }

    // Feeds one SSE line. Returns Ok(Some(delta)) for new text, Ok(None) for a
    // non-text event, or Err for a provider error event (must abort the stream).
    fn feed_line(&mut self, line: &str) -> Result<Option<String>, Flow> {
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            return Ok(None);
        };
        if data.is_empty() || data == "[DONE]" {
            return Ok(None);
        }
        let Ok(Value::Map(ev)) = json_decode(data) else {
            return Ok(None);
        };
        // A provider error event mid-stream (a 200 response can still fail for
        // overload/rate-limit). Surface it as an error so a failed generation does
        // NOT look like a partial success. Detection is provider-agnostic:
        //   - Anthropic: a top-level `{"type":"error", "error":{...}}`.
        //   - OpenAI family (OpenAI / OpenRouter / GLM / Ollama / gateways): a
        //     top-level `{"error":{...}}` with NO `type:"error"` — so checking the
        //     Anthropic shape alone would silently drop it (the original fix
        //     covered only Anthropic; this also covers the OpenAI-style providers
        //     `ai.config` targets).
        let is_error = ev.get("type").and_then(as_str).as_deref() == Some("error")
            || matches!(ev.get("error"), Some(Value::Map(_)));
        if is_error {
            return Err(Flow::err(format!(
                "ai: stream error event: {}",
                truncate(&sse_error_message(&ev), 300)
            )));
        }
        match self.provider {
            Provider::Anthropic => Ok(self.feed_anthropic(&ev)),
            Provider::OpenAI => Ok(self.feed_openai(&ev)),
        }
    }

    // Anthropic SSE: content_block_delta -> delta.text_delta; message_delta ->
    // stop_reason + usage.output_tokens; message_start -> usage.input_tokens.
    fn feed_anthropic(&mut self, ev: &BTreeMap<String, Value>) -> Option<String> {
        match ev.get("type").and_then(as_str).as_deref() {
            Some("content_block_delta") => {
                let delta = match ev.get("delta") {
                    Some(Value::Map(d)) => d,
                    _ => return None,
                };
                // text_delta -> text. (input_json_delta for tool args is ignored:
                // text streaming is the priority; tool-use streaming is a follow-up.)
                let t = delta.get("text").and_then(as_str)?;
                if t.is_empty() {
                    return None;
                }
                self.text.push_str(&t);
                Some(t)
            }
            Some("message_start") => {
                if let Some(Value::Map(m)) = ev.get("message")
                    && let Some(Value::Map(u)) = m.get("usage")
                {
                    self.in_tokens = u.get("input_tokens").and_then(as_int).unwrap_or(0);
                }
                None
            }
            Some("message_delta") => {
                if let Some(Value::Map(d)) = ev.get("delta")
                    && let Some(s) = d.get("stop_reason").and_then(as_str)
                {
                    self.stop = s;
                }
                if let Some(Value::Map(u)) = ev.get("usage") {
                    self.out_tokens = u.get("output_tokens").and_then(as_int).unwrap_or(0);
                }
                None
            }
            _ => None,
        }
    }

    // OpenAI SSE: choices[0].delta.content -> text; finish_reason on the last
    // content chunk; usage on the final chunk (requires include_usage).
    fn feed_openai(&mut self, ev: &BTreeMap<String, Value>) -> Option<String> {
        // The usage-only final chunk has an empty choices list.
        if let Some(Value::Map(u)) = ev.get("usage") {
            self.in_tokens = u
                .get("prompt_tokens")
                .and_then(as_int)
                .unwrap_or(self.in_tokens);
            self.out_tokens = u
                .get("completion_tokens")
                .and_then(as_int)
                .unwrap_or(self.out_tokens);
        }
        let choice = match ev.get("choices") {
            Some(Value::List(cs)) if !cs.is_empty() => match &cs[0] {
                Value::Map(c) => c,
                _ => return None,
            },
            _ => return None,
        };
        if let Some(s) = choice.get("finish_reason").and_then(as_str) {
            self.stop = s;
        }
        let delta = match choice.get("delta") {
            Some(Value::Map(d)) => d,
            _ => return None,
        };
        let t = delta.get("content").and_then(as_str)?;
        if t.is_empty() {
            return None;
        }
        self.text.push_str(&t);
        Some(t)
    }

    // Builds the final AiResp from the accumulated state — same confidence
    // heuristic as the non-stream parsers.
    fn finish(self, model: &str, ms: i64) -> Result<AiResp, Flow> {
        let conf = match self.stop.as_str() {
            "end_turn" | "tool_use" | "stop_sequence" | "stop" | "tool_calls" => 0.9,
            "max_tokens" | "length" => 0.5,
            "refusal" | "content_filter" => 0.0,
            "" => 0.8, // stream ended without an explicit stop reason
            _ => 0.7,
        };
        Ok(AiResp {
            text: self.text,
            tool_calls: Vec::new(),
            in_tokens: self.in_tokens,
            out_tokens: self.out_tokens,
            model: model.to_string(),
            ms,
            conf,
        })
    }
}

// Extracts a human-readable message from an SSE error event. Anthropic shape:
// `{"type":"error","error":{"type":"overloaded_error","message":"..."}}`. Falls
// back to the whole event JSON if no nested message is present.
fn sse_error_message(ev: &BTreeMap<String, Value>) -> String {
    if let Some(Value::Map(err)) = ev.get("error")
        && let Some(msg) = err.get("message").and_then(as_str)
    {
        return msg;
    }
    json_encode(&Value::Map(ev.clone()))
}

// Generic https POST (content-type: json). `add_headers` adds the
// provider-specific authentication/version headers. Returns the response text
// + duration (ms); non-2xx -> explicit error.
//
// On transient errors (429 rate-limit / 529 overloaded) it retries ONCE (issue
// #92 bonus): LLM APIs return these statuses during short load spikes — a
// single retry with backoff improves stability noticeably, with no risk of an
// infinite loop. The timeout is applied to each attempt separately (worst
// case: 2 attempts + backoff). That is why `add_headers` is an Fn (not FnOnce)
// — the request is rebuilt on retry.
fn post_json<F>(
    url: &str,
    body: String,
    timeout: Option<Duration>,
    add_headers: F,
) -> Result<(String, i64), Flow>
where
    F: Fn(hyper::http::request::Builder) -> hyper::http::request::Builder + Send + 'static,
{
    let url = url.to_string();
    let started = std::time::Instant::now();
    let text = client_runtime().block_on(async move {
        let mut retried = false;
        loop {
            // A single attempt (send + read response) — the timeout covers this block.
            let work = async {
                // The default content-type/auth/version headers are emitted by
                // `add_headers` itself (each provider closure decides), so that a
                // user header of the same name can REPLACE rather than duplicate
                // (hyper appends duplicates). post_json stays header-agnostic.
                let builder = Request::builder().method("POST").uri(url.clone());
                let builder = add_headers(builder);
                let req = builder
                    .body(Full::new(Bytes::from(body.clone())))
                    .map_err(|e| Flow::err(format!("ai: building request: {}", e)))?;

                let resp = pooled_http_client()
                    .request(req)
                    .await
                    .map_err(|e| Flow::err(format!("ai: network error: {}", e)))?;
                let status = resp.status().as_u16();
                // Read Retry-After BEFORE consuming the body (into_body consumes
                // resp) — the server's wait time is used in backoff.
                let retry_after = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.trim().parse::<u64>().ok());
                let bytes = resp
                    .into_body()
                    .collect()
                    .await
                    .map_err(|e| Flow::err(format!("ai: reading response: {}", e)))?
                    .to_bytes();
                let text = String::from_utf8_lossy(&bytes).to_string();
                Ok::<_, Flow>((status, retry_after, text))
            };

            // If a timeout is set, wrap the attempt in it; if it does not finish,
            // an explicit error (a stuck LLM endpoint must not block the thread
            // forever — issue #92).
            let (status, retry_after, text) = match timeout {
                Some(dur) => match tokio::time::timeout(dur, work).await {
                    Ok(r) => r?,
                    Err(_) => {
                        return Err(Flow::err(format!(
                            "ai: request timeout (no response within {} sec)",
                            dur.as_secs()
                        )));
                    }
                },
                None => work.await?,
            };

            if (200..300).contains(&status) {
                return Ok(text);
            }
            if !retried && should_retry_status(status) {
                retried = true;
                tokio::time::sleep(retry_backoff(retry_after)).await;
                continue;
            }
            return Err(Flow::err(format!(
                "ai: API error (status {}): {}",
                status,
                truncate(&text, 300)
            )));
        }
    })?;
    let ms = started.elapsed().as_millis() as i64;
    Ok((text, ms))
}

// 429 (rate limit) and 529 (Anthropic overloaded) — transient states: a single
// retry is worthwhile (issue #92 bonus). Other 4xx/5xx (401 wrong key, 400
// invalid request...) will not recover from a retry — error immediately.
fn should_retry_status(status: u16) -> bool {
    matches!(status, 429 | 529)
}

// Wait before retry: if the server gave `Retry-After` (seconds) we honor it —
// but clamped to 1..=30 (so as not to hold the handler thread too long); if
// there is no header, default 2s.
fn retry_backoff(retry_after: Option<u64>) -> Duration {
    Duration::from_secs(retry_after.map(|s| s.clamp(1, 30)).unwrap_or(2))
}

// --- Response parsing ---

// The parts we need from the Anthropic response.
struct AiResp {
    text: String,
    // (tool_name, input_map, tool_use_id) — when the model wants to call a tool.
    // Vec: the model may call SEVERAL tools in parallel in one response — we
    // collect all, otherwise a missing tool_use_id gives a 400 on the next request.
    tool_calls: Vec<(String, Value, String)>,
    in_tokens: i64,
    out_tokens: i64,
    model: String,
    ms: i64,
    // confidence estimated from stop_reason (end_turn -> high, max_tokens -> low).
    conf: f64,
}

impl AiResp {
    // The `r._` metadata map: conf/tokens/cost/ms.
    fn meta(&self) -> BTreeMap<String, Value> {
        let mut m = BTreeMap::new();
        m.insert("conf".to_string(), Value::Flt(self.conf));
        m.insert(
            "tokens".to_string(),
            Value::Int(self.in_tokens + self.out_tokens),
        );
        m.insert(
            "cost".to_string(),
            Value::Flt(estimate_cost(&self.model, self.in_tokens, self.out_tokens)),
        );
        m.insert("ms".to_string(), Value::Int(self.ms));
        m
    }
}

// Parses the Anthropic response text into an AiResp. Extracts text and
// tool_use blocks from the content array; reads tokens from usage.
fn parse_anthropic(text: &str, model: &str, ms: i64) -> Result<AiResp, Flow> {
    let map = decode_obj(text)?;

    // stop_reason -> confidence estimate (heuristic).
    let stop = map.get("stop_reason").and_then(as_str).unwrap_or_default();
    let conf = match stop.as_str() {
        "end_turn" | "tool_use" | "stop_sequence" => 0.9,
        "max_tokens" => 0.5, // truncated -> low confidence
        "refusal" => 0.0,
        _ => 0.7,
    };

    // usage.input_tokens / output_tokens.
    let (in_tokens, out_tokens) = match map.get("usage") {
        Some(Value::Map(u)) => (
            u.get("input_tokens").and_then(as_int).unwrap_or(0),
            u.get("output_tokens").and_then(as_int).unwrap_or(0),
        ),
        _ => (0, 0),
    };

    // content: [{type:"text" text:...} | {type:"tool_use" name input id} ...]
    // The model may return several tool_use blocks in parallel — we collect all
    // of them (if we kept only the last, the rest would have no tool_result on
    // the next request and the Anthropic API would return 400).
    let mut out_text = String::new();
    let mut tool_calls = Vec::new();
    if let Some(Value::List(blocks)) = map.get("content") {
        for block in blocks {
            if let Value::Map(b) = block {
                match b.get("type").and_then(as_str).as_deref() {
                    Some("text") => {
                        if let Some(t) = b.get("text").and_then(as_str) {
                            out_text.push_str(&t);
                        }
                    }
                    Some("tool_use") => {
                        let name = b.get("name").and_then(as_str).unwrap_or_default();
                        let input = b
                            .get("input")
                            .cloned()
                            .unwrap_or(Value::Map(BTreeMap::new()));
                        let id = b.get("id").and_then(as_str).unwrap_or_default();
                        tool_calls.push((name, input, id));
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(AiResp {
        text: out_text,
        tool_calls,
        in_tokens,
        out_tokens,
        model: model.to_string(),
        ms,
        conf,
    })
}

// Parses the OpenAI Chat Completions response into an AiResp:
//   choices[0].message.{content, tool_calls[0].function.{name, arguments}}
//   choices[0].finish_reason, usage.{prompt_tokens, completion_tokens}
// tool_calls[].function.arguments — a JSON-encoded STRING (a map in Anthropic).
fn parse_openai(text: &str, model: &str, ms: i64) -> Result<AiResp, Flow> {
    let map = decode_obj(text)?;

    let choice = match map.get("choices") {
        Some(Value::List(cs)) if !cs.is_empty() => match &cs[0] {
            Value::Map(c) => c.clone(),
            _ => {
                return Err(Flow::err(
                    "ai: OpenAI choices[0] has wrong shape".to_string(),
                ));
            }
        },
        _ => {
            return Err(Flow::err(format!(
                "ai: OpenAI response has no choices: {}",
                truncate(text, 200)
            )));
        }
    };

    let finish = choice
        .get("finish_reason")
        .and_then(as_str)
        .unwrap_or_default();
    let conf = match finish.as_str() {
        "stop" | "tool_calls" => 0.9,
        "length" => 0.5, // token limit -> truncated
        "content_filter" => 0.0,
        _ => 0.7,
    };

    // usage: OpenAI calls them prompt_tokens/completion_tokens.
    let (in_tokens, out_tokens) = match map.get("usage") {
        Some(Value::Map(u)) => (
            u.get("prompt_tokens").and_then(as_int).unwrap_or(0),
            u.get("completion_tokens").and_then(as_int).unwrap_or(0),
        ),
        _ => (0, 0),
    };

    let message = match choice.get("message") {
        Some(Value::Map(m)) => m.clone(),
        _ => return Err(Flow::err("ai: OpenAI message missing".to_string())),
    };

    // content (may be null when there are tool_calls).
    let out_text = message.get("content").and_then(as_str).unwrap_or_default();

    // tool_calls[] -> [(name, args_map, id)]. arguments JSON-string -> map.
    // The model may call several tools in one response — we collect all of them
    // (if we took only tc[0], the rest would get no tool result and the next
    // request would get a 400).
    let mut tool_calls = Vec::new();
    if let Some(Value::List(tc)) = message.get("tool_calls") {
        for call in tc {
            let Value::Map(call) = call else { continue };
            let id = call.get("id").and_then(as_str).unwrap_or_default();
            let func = match call.get("function") {
                Some(Value::Map(f)) => f,
                _ => {
                    return Err(Flow::err(
                        "ai: OpenAI tool_call.function missing".to_string(),
                    ));
                }
            };
            let name = func.get("name").and_then(as_str).unwrap_or_default();
            // arguments — a JSON-encoded string; we parse it into a map.
            let args = match func.get("arguments").and_then(as_str) {
                Some(s) => json_decode(&s).unwrap_or(Value::Map(BTreeMap::new())),
                None => Value::Map(BTreeMap::new()),
            };
            tool_calls.push((name, args, id));
        }
    }

    Ok(AiResp {
        text: out_text,
        tool_calls,
        in_tokens,
        out_tokens,
        model: model.to_string(),
        ms,
        conf,
    })
}

// Decodes JSON text into a map (shared by both parsers).
fn decode_obj(text: &str) -> Result<BTreeMap<String, Value>, Flow> {
    let v = json_decode(text).map_err(|_| {
        Flow::err(format!(
            "ai: could not parse response: {}",
            truncate(text, 200)
        ))
    })?;
    match v {
        Value::Map(m) => Ok(m),
        _ => Err(Flow::err("ai: unexpected response shape".to_string())),
    }
}

// --- Helpers ---

// {role::user content:"..."} — a single user message.
fn user_msg(content: &str) -> Value {
    let mut m = BTreeMap::new();
    m.insert("role".to_string(), Value::Str("user".to_string()));
    m.insert("content".to_string(), Value::Str(content.to_string()));
    Value::Map(m)
}

// Brings a Fluxon message into the Anthropic shape. role may be a sym (:user)
// or str -> always str. The tool-result message ({role::tool name content})
// becomes a user role + tool_result block in Anthropic.
fn normalize_msg(msg: &Value) -> Value {
    let m = match msg {
        Value::Map(m) => m,
        other => return other.clone(),
    };
    // role may be a sym (:user) or str ("user") — we read both.
    let role = m
        .get("role")
        .and_then(sym_or_str)
        .unwrap_or_else(|| "user".to_string());

    // tool result: {role::tool name content} -> {role:"user" content:[{type:tool_result ...}]}
    if role == "tool" {
        let tool_use_id = m
            .get("id")
            .or_else(|| m.get("tool_use_id"))
            .and_then(as_str)
            .unwrap_or_default();
        let content = m.get("content").map(content_to_str).unwrap_or_default();
        let mut result = BTreeMap::new();
        result.insert("type".to_string(), Value::Str("tool_result".to_string()));
        result.insert("tool_use_id".to_string(), Value::Str(tool_use_id));
        result.insert("content".to_string(), Value::Str(content));
        let mut out = BTreeMap::new();
        out.insert("role".to_string(), Value::Str("user".to_string()));
        out.insert("content".to_string(), Value::List(vec![Value::Map(result)]));
        return Value::Map(out);
    }

    // Ordinary message: role to str, content as is (str or a block list).
    let mut out = BTreeMap::new();
    out.insert("role".to_string(), Value::Str(role));
    if let Some(c) = m.get("content") {
        out.insert("content".to_string(), c.clone());
    } else {
        out.insert("content".to_string(), Value::Str(String::new()));
    }
    Value::Map(out)
}

// Brings a Fluxon tool definition ({name desc params}) into the Anthropic
// shape ({name description input_schema}).
fn normalize_tool(tool: &Value) -> Value {
    let m = match tool {
        Value::Map(m) => m,
        other => return other.clone(),
    };
    let name = m.get("name").and_then(as_str).unwrap_or_default();
    let desc = m
        .get("desc")
        .or_else(|| m.get("description"))
        .and_then(as_str)
        .unwrap_or_default();
    // params — a JSON-schema (object). The user may give something simple like
    // {a:str b:int}; we wrap it into a JSON-schema object, unless it is already
    // type:object.
    let schema = m
        .get("params")
        .or_else(|| m.get("input_schema"))
        .cloned()
        .unwrap_or_else(|| Value::Map(BTreeMap::new()));
    let input_schema = wrap_schema(schema);

    let mut out = BTreeMap::new();
    out.insert("name".to_string(), Value::Str(name));
    out.insert("description".to_string(), Value::Str(desc));
    out.insert("input_schema".to_string(), input_schema);
    Value::Map(out)
}

// Converts the params map into a full JSON-schema object. If the map is already
// {type:"object" ...}, we leave it as is. Otherwise we turn each field into
// {type:"..."} and place it under `properties`.
fn wrap_schema(schema: Value) -> Value {
    let fields = match schema {
        Value::Map(m) => m,
        // not a map — an empty object schema.
        _ => return empty_object_schema(),
    };
    // If it is already a full schema (type:object), we do not touch it.
    if fields.get("type").and_then(as_str).as_deref() == Some("object") {
        return Value::Map(fields);
    }
    let mut props = BTreeMap::new();
    let mut required = Vec::new();
    for (k, v) in &fields {
        props.insert(k.clone(), field_schema(v));
        required.push(Value::Str(k.clone()));
    }
    let mut obj = BTreeMap::new();
    obj.insert("type".to_string(), Value::Str("object".to_string()));
    obj.insert("properties".to_string(), Value::Map(props));
    obj.insert("required".to_string(), Value::List(required));
    Value::Map(obj)
}

// Converts a single field value into a JSON-schema fragment. A type name
// (sym/str) -> {type:"..."}; a list (`[T]`) -> {type:"array", items:<T schema>};
// a map -> a recursive object schema (as is if already a full schema).
fn field_schema(v: &Value) -> Value {
    match v {
        // {a:str} — the value is a type name (sym or str).
        Value::Sym(s) | Value::Str(s) => {
            let mut field = BTreeMap::new();
            field.insert("type".to_string(), Value::Str(json_type(s)));
            Value::Map(field)
        }
        // [T] — array; element type recursive. `[]` (empty) -> array without items.
        Value::List(items) => {
            let mut field = BTreeMap::new();
            field.insert("type".to_string(), Value::Str("array".to_string()));
            if let Some(first) = items.first() {
                field.insert("items".to_string(), field_schema(first));
            }
            Value::Map(field)
        }
        // {a:{...}} — a nested object; if it is not already a full schema, we
        // turn it into an object with a recursive wrap_schema.
        Value::Map(m) => {
            if m.get("type").and_then(as_str).is_some() {
                // the user already gave {type:"..."} — we do not touch it.
                v.clone()
            } else {
                wrap_schema(v.clone())
            }
        }
        // string as a fallback for other values.
        _ => {
            let mut field = BTreeMap::new();
            field.insert("type".to_string(), Value::Str("string".to_string()));
            Value::Map(field)
        }
    }
}

fn empty_object_schema() -> Value {
    let mut obj = BTreeMap::new();
    obj.insert("type".to_string(), Value::Str("object".to_string()));
    obj.insert("properties".to_string(), Value::Map(BTreeMap::new()));
    Value::Map(obj)
}

// Fluxon type name to JSON-schema type: str->string, int->integer, flt->number,
// bool->boolean. Anything else as is (if the user gives list/object).
fn json_type(t: &str) -> String {
    match t {
        "str" => "string",
        "int" => "integer",
        "flt" => "number",
        "bool" => "boolean",
        other => other,
    }
    .to_string()
}

// Brings tool_result content to text: map/list -> JSON, str -> itself.
fn content_to_str(v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(),
        Value::Map(_) | Value::List(_) => json_encode(v),
        other => format!("{}", other),
    }
}

// If the model wraps the result in ```json ... ``` or ``` ... ```, strips the block.
fn strip_code_fence(s: &str) -> String {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```") {
        // drop the first line (```json) and take up to the last ```.
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        let rest = rest.trim_start_matches('\n');
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim().to_string();
        }
    }
    t.to_string()
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let cut: String = s.chars().take(n).collect();
        format!("{}…", cut)
    }
}

// Estimated cost (USD) from the model's $/1M token table. Unknown model -> 0.
// Anthropic: Opus 4.x / Sonnet 4.6 / Haiku 4.5. OpenAI: gpt-4o / gpt-4o-mini.
// ($input, $output per 1M tokens).
fn estimate_cost(model: &str, in_tokens: i64, out_tokens: i64) -> f64 {
    let (in_rate, out_rate) = if model.contains("opus") {
        (5.0, 25.0)
    } else if model.contains("sonnet") {
        (3.0, 15.0)
    } else if model.contains("haiku") {
        (1.0, 5.0)
    } else if model.contains("gpt-4o-mini") {
        (0.15, 0.60)
    } else if model.contains("gpt-4o") {
        (2.5, 10.0)
    } else {
        (0.0, 0.0)
    };
    (in_tokens as f64 * in_rate + out_tokens as f64 * out_rate) / 1_000_000.0
}

// --- Provider helpers ---

fn provider_name(p: Provider) -> &'static str {
    match p {
        Provider::Anthropic => "Anthropic",
        Provider::OpenAI => "OpenAI",
    }
}

fn provider_key_name(p: Provider) -> &'static str {
    match p {
        Provider::Anthropic => "ANTHROPIC_API_KEY",
        Provider::OpenAI => "OPENAI_API_KEY",
    }
}

// Resolves the API key — security-critical, so it is a pure function (tested
// without env). `explicit` is an inline `key` or $AI_KEY (provider-agnostic);
// `provider_key` is the standard $OPENAI_API_KEY/$ANTHROPIC_API_KEY.
//
// On a CUSTOM url, ONLY the explicit key is allowed: we MUST NOT fall back to a
// provider key, or we would send e.g. $OPENAI_API_KEY to a third-party host
// (Z.AI/OpenRouter) — a credential leak. On the DEFAULT (official) url, the
// provider key is the normal zero-config fallback.
fn resolve_key(
    is_custom: bool,
    explicit: Option<String>,
    provider_key: Option<String>,
    provider: Provider,
) -> Result<String, Flow> {
    if is_custom {
        explicit.ok_or_else(|| {
            Flow::err(
                "ai: a custom url needs an explicit key — set `key` in ai.config \
                 / the call opts, or $AI_KEY (a provider key like $OPENAI_API_KEY \
                 is never sent to a custom host)"
                    .to_string(),
            )
        })
    } else {
        explicit.or(provider_key).ok_or_else(|| {
            Flow::err(format!(
                "ai: {} key not found (set ${}, $AI_KEY, or ai.config key)",
                provider_name(provider),
                provider_key_name(provider),
            ))
        })
    }
}

// Converts an Anthropic-shaped message into the OpenAI shape.
//   {role:"user" content:"..."}                       -> unchanged
//   {role:"user" content:[{type:tool_result tool_use_id content}]}
//                                                      -> {role:"tool" tool_call_id content}
//   {role:"assistant" content:[{type:tool_use id name input}]}
//                                                      -> {role:"assistant" tool_calls:[...]}
fn anthropic_msg_to_openai(msg: &Value) -> Value {
    let m = match msg {
        Value::Map(m) => m,
        other => return other.clone(),
    };
    let role = m.get("role").and_then(as_str).unwrap_or_default();

    // if content is a block list — convert tool_result or tool_use.
    if let Some(Value::List(blocks)) = m.get("content") {
        // tool_result (Anthropic user role) -> OpenAI {role:"tool" tool_call_id content}
        if let Some(Value::Map(b)) = blocks.first()
            && b.get("type").and_then(as_str).as_deref() == Some("tool_result")
        {
            let mut out = BTreeMap::new();
            out.insert("role".to_string(), Value::Str("tool".to_string()));
            out.insert(
                "tool_call_id".to_string(),
                b.get("tool_use_id")
                    .cloned()
                    .unwrap_or(Value::Str(String::new())),
            );
            out.insert(
                "content".to_string(),
                b.get("content")
                    .cloned()
                    .unwrap_or(Value::Str(String::new())),
            );
            return Value::Map(out);
        }
        // tool_use (Anthropic assistant role) -> OpenAI tool_calls.
        if let Some(Value::Map(b)) = blocks.first()
            && b.get("type").and_then(as_str).as_deref() == Some("tool_use")
        {
            let id = b.get("id").and_then(as_str).unwrap_or_default();
            let name = b.get("name").and_then(as_str).unwrap_or_default();
            let args = b
                .get("input")
                .cloned()
                .unwrap_or(Value::Map(BTreeMap::new()));
            let mut func = BTreeMap::new();
            func.insert("name".to_string(), Value::Str(name));
            // OpenAI arguments — a JSON-encoded STRING.
            func.insert("arguments".to_string(), Value::Str(json_encode(&args)));
            let mut call = BTreeMap::new();
            call.insert("id".to_string(), Value::Str(id));
            call.insert("type".to_string(), Value::Str("function".to_string()));
            call.insert("function".to_string(), Value::Map(func));
            let mut out = BTreeMap::new();
            out.insert("role".to_string(), Value::Str("assistant".to_string()));
            out.insert("content".to_string(), Value::Nil);
            out.insert(
                "tool_calls".to_string(),
                Value::List(vec![Value::Map(call)]),
            );
            return Value::Map(out);
        }
    }

    // Ordinary message: role + content (str) as is.
    let mut out = BTreeMap::new();
    out.insert("role".to_string(), Value::Str(role));
    out.insert(
        "content".to_string(),
        m.get("content")
            .cloned()
            .unwrap_or(Value::Str(String::new())),
    );
    Value::Map(out)
}

// Converts the Anthropic tool shape ({name description input_schema}) into the
// OpenAI function shape ({type:function function:{name description parameters}}).
fn anthropic_tool_to_openai(tool: &Value) -> Value {
    let t = match tool {
        Value::Map(m) => m,
        other => return other.clone(),
    };
    let mut func = BTreeMap::new();
    func.insert(
        "name".to_string(),
        t.get("name").cloned().unwrap_or(Value::Str(String::new())),
    );
    func.insert(
        "description".to_string(),
        t.get("description")
            .cloned()
            .unwrap_or(Value::Str(String::new())),
    );
    func.insert(
        "parameters".to_string(),
        t.get("input_schema")
            .cloned()
            .unwrap_or_else(empty_object_schema),
    );
    let mut out = BTreeMap::new();
    out.insert("type".to_string(), Value::Str("function".to_string()));
    out.insert("function".to_string(), Value::Map(func));
    Value::Map(out)
}

// Helpers to read str/int from a Value (for the json_decode result).
fn as_str(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.clone()),
        _ => None,
    }
}

// Sym or Str — text from either (so role :user and "user" look the same).
fn sym_or_str(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) | Value::Sym(s) => Some(s.clone()),
        _ => None,
    }
}

fn as_int(v: &Value) -> Option<i64> {
    match v {
        Value::Int(n) => Some(*n),
        Value::Flt(x) => Some(*x as i64),
        _ => None,
    }
}

// $AI_TIMEOUT (seconds str) -> Duration. Absent/invalid -> default 120s; 0 or
// negative -> None (no timeout). A pure function — tested without env (#92).
fn resolve_ai_timeout(env: Option<&str>) -> Option<Duration> {
    match env.and_then(|s| s.trim().parse::<i64>().ok()) {
        Some(n) if n > 0 => Some(Duration::from_secs(n as u64)),
        Some(_) => None, // 0 or negative — no timeout
        None => Some(Duration::from_secs(DEFAULT_AI_TIMEOUT_SECS)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- issue #199: ai.config / per-call override ---

    // Builds an opts map (str/sym values) for parse_override tests.
    fn opts(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn parse_override_fields() {
        // url/style/key/model + headers + extra all parse into the override.
        let mut headers = BTreeMap::new();
        headers.insert("X-Title".to_string(), Value::Str("App".to_string()));
        // A symbol header value must NOT keep its `:` prefix (to_text coercion).
        headers.insert("X-Mode".to_string(), Value::Sym("fast".to_string()));
        let mut extra = BTreeMap::new();
        extra.insert("route".to_string(), Value::Str("fallback".to_string()));
        let m = opts(&[
            ("url", Value::Str("https://api.z.ai/v4/chat".to_string())),
            ("style", Value::Sym("openai".to_string())),
            ("key", Value::Str("sk-x".to_string())),
            ("model", Value::Str("glm-4.6".to_string())),
            ("headers", Value::Map(headers)),
            ("extra", Value::Map(extra)),
        ]);
        let ov = match parse_override(&m) {
            Ok(o) => o,
            Err(_) => panic!("parse failed"),
        };
        assert_eq!(ov.url.as_deref(), Some("https://api.z.ai/v4/chat"));
        assert_eq!(ov.style, Some(Provider::OpenAI));
        assert_eq!(ov.key.as_deref(), Some("sk-x"));
        assert_eq!(ov.model.as_deref(), Some("glm-4.6"));
        // Header names are normalized to lowercase (case-insensitive on the wire).
        assert_eq!(ov.headers.get("x-title").map(|s| s.as_str()), Some("App"));
        assert_eq!(ov.headers.get("x-mode").map(|s| s.as_str()), Some("fast"));
        assert!(!ov.headers.contains_key("X-Title"), "name not lowercased");
        // extra (body) keys keep their case — JSON fields are case-sensitive.
        assert!(ov.extra.contains_key("route"));
    }

    #[test]
    fn parse_override_unknown_key_errors() {
        // A typo must fail loudly, not be silently ignored.
        let m = opts(&[("ur", Value::Str("x".to_string()))]);
        match parse_override(&m) {
            Err(Flow::Error(e)) => assert!(e.contains("unknown opts key"), "{}", e),
            _ => panic!("expected error for unknown key"),
        }
    }

    #[test]
    fn parse_override_skips_nil() {
        // `{key: env.UNSET}` -> nil is treated as "not set", not an error, so the
        // value falls through to the env/default.
        let m = opts(&[
            ("key", Value::Nil),
            ("model", Value::Str("glm-4.6".to_string())),
        ]);
        let ov = match parse_override(&m) {
            Ok(o) => o,
            Err(_) => panic!("nil should be skipped, not error"),
        };
        assert!(ov.key.is_none());
        assert_eq!(ov.model.as_deref(), Some("glm-4.6"));
    }

    #[test]
    fn parse_override_bad_style_errors() {
        let m = opts(&[("style", Value::Sym("grok".to_string()))]);
        match parse_override(&m) {
            Err(Flow::Error(e)) => assert!(e.contains("unknown style"), "{}", e),
            _ => panic!("expected error for bad style"),
        }
    }

    #[test]
    fn merge_per_call_wins() {
        // Global config: openai style + url + a header. Per-call overrides the
        // url and adds another header; the global header survives (merge).
        let global = AiOverride {
            url: Some("https://global".to_string()),
            style: Some(Provider::OpenAI),
            headers: [("A".to_string(), "1".to_string())].into(),
            extra: [("x".to_string(), Value::Int(1))].into(),
            ..AiOverride::default()
        };
        let per_call = AiOverride {
            url: Some("https://percall".to_string()),
            headers: [("B".to_string(), "2".to_string())].into(),
            extra: [("y".to_string(), Value::Int(2))].into(),
            ..AiOverride::default()
        };

        let merged = global.merge(&per_call);
        // scalar: per-call replaces.
        assert_eq!(merged.url.as_deref(), Some("https://percall"));
        // style only set globally — survives.
        assert_eq!(merged.style, Some(Provider::OpenAI));
        // headers + extra: both keys present (key-by-key merge).
        assert_eq!(merged.headers.get("A").map(|s| s.as_str()), Some("1"));
        assert_eq!(merged.headers.get("B").map(|s| s.as_str()), Some("2"));
        assert!(merged.extra.contains_key("x") && merged.extra.contains_key("y"));
    }

    #[test]
    fn merge_same_key_per_call_overrides() {
        // Same header/extra key: per-call value wins.
        let global = AiOverride {
            headers: [("A".to_string(), "old".to_string())].into(),
            extra: [("k".to_string(), Value::Str("old".to_string()))].into(),
            ..AiOverride::default()
        };
        let per_call = AiOverride {
            headers: [("A".to_string(), "new".to_string())].into(),
            extra: [("k".to_string(), Value::Str("new".to_string()))].into(),
            ..AiOverride::default()
        };
        let merged = global.merge(&per_call);
        assert_eq!(merged.headers.get("A").map(|s| s.as_str()), Some("new"));
        match merged.extra.get("k") {
            Some(Value::Str(s)) => assert_eq!(s, "new"),
            _ => panic!(),
        }
    }

    #[test]
    fn merge_extra_into_body_overrides() {
        // merge_extra lets a user field override a default body field (max_tokens).
        let mut body = BTreeMap::new();
        body.insert("model".to_string(), Value::Str("m".to_string()));
        body.insert("max_tokens".to_string(), Value::Int(4096));
        let mut extra = BTreeMap::new();
        extra.insert("max_tokens".to_string(), Value::Int(100));
        extra.insert("provider".to_string(), Value::Str("zai".to_string()));
        merge_extra(&mut body, &extra);
        assert_eq!(as_int(body.get("max_tokens").unwrap()), Some(100));
        assert_eq!(
            as_str(body.get("provider").unwrap()).as_deref(),
            Some("zai")
        );
        // untouched default stays.
        assert_eq!(as_str(body.get("model").unwrap()).as_deref(), Some("m"));
    }

    #[test]
    fn header_overridden_is_case_insensitive() {
        let mut h = BTreeMap::new();
        h.insert("Authorization".to_string(), "Basic x".to_string());
        assert!(header_overridden(&h, "authorization"));
        assert!(!header_overridden(&h, "x-api-key"));
    }

    #[test]
    fn style_from_str() {
        assert_eq!(Provider::from_style("openai"), Some(Provider::OpenAI));
        assert_eq!(Provider::from_style("GPT"), Some(Provider::OpenAI));
        assert_eq!(Provider::from_style("anthropic"), Some(Provider::Anthropic));
        assert_eq!(Provider::from_style("claude"), Some(Provider::Anthropic));
        assert_eq!(Provider::from_style("grok"), None);
    }

    #[test]
    fn resolve_key_custom_url_requires_explicit() {
        // SECURITY (PR #205 P1): on a custom url, a provider key must NOT be used
        // as a fallback — only an explicit key. Otherwise $OPENAI_API_KEY would
        // leak to a third-party host (Z.AI/OpenRouter).
        let s = |x: &str| Some(x.to_string());

        // custom url + explicit key -> the explicit key is used.
        match resolve_key(true, s("sk-explicit"), s("sk-provider"), Provider::OpenAI) {
            Ok(k) => assert_eq!(k, "sk-explicit"),
            Err(_) => panic!("explicit key should be accepted"),
        }
        // custom url + NO explicit key, but a provider key IS present -> ERROR,
        // and the provider key is NOT leaked.
        match resolve_key(true, None, s("sk-provider-leak"), Provider::OpenAI) {
            Err(Flow::Error(e)) => {
                assert!(e.contains("custom url"), "{}", e);
                assert!(!e.contains("sk-provider-leak"), "key must not leak: {}", e);
            }
            Ok(k) => panic!(
                "must not fall back to provider key on custom url: got {}",
                k
            ),
            Err(_) => panic!("expected Flow::Error"),
        }
        // custom url + nothing at all -> error.
        match resolve_key(true, None, None, Provider::Anthropic) {
            Err(Flow::Error(_)) => {}
            _ => panic!("expected error"),
        }
    }

    #[test]
    fn resolve_key_default_url_falls_back_to_provider() {
        // On the DEFAULT url the provider key is the normal zero-config fallback.
        let s = |x: &str| Some(x.to_string());
        // explicit wins when present.
        match resolve_key(false, s("sk-explicit"), s("sk-provider"), Provider::OpenAI) {
            Ok(k) => assert_eq!(k, "sk-explicit"),
            Err(_) => panic!("explicit should win"),
        }
        // no explicit -> provider key.
        match resolve_key(false, None, s("sk-provider"), Provider::OpenAI) {
            Ok(k) => assert_eq!(k, "sk-provider"),
            Err(_) => panic!("provider key should be used on default url"),
        }
        // nothing -> error.
        match resolve_key(false, None, None, Provider::OpenAI) {
            Err(Flow::Error(e)) => assert!(e.contains("key not found"), "{}", e),
            _ => panic!("expected error"),
        }
    }

    #[test]
    fn ai_timeout_resolve() {
        // issue #92: default 120s, configured with $AI_TIMEOUT, 0/negative — no timeout.
        assert_eq!(resolve_ai_timeout(None), Some(Duration::from_secs(120)));
        assert_eq!(
            resolve_ai_timeout(Some("30")),
            Some(Duration::from_secs(30))
        );
        assert_eq!(resolve_ai_timeout(Some("0")), None);
        assert_eq!(resolve_ai_timeout(Some("-5")), None);
        // Invalid value — falls back to default (parse fails).
        assert_eq!(
            resolve_ai_timeout(Some("abc")),
            Some(Duration::from_secs(120))
        );
    }

    #[test]
    fn retry_status_faqat_vaqtinchalik() {
        // 429/529 — retry; authentication/validation errors — no.
        assert!(should_retry_status(429));
        assert!(should_retry_status(529));
        assert!(!should_retry_status(400));
        assert!(!should_retry_status(401));
        assert!(!should_retry_status(500));
        assert!(!should_retry_status(503));
    }

    #[test]
    fn retry_backoff_retry_after_va_default() {
        // If Retry-After is present, honor it (clamp 1..=30), otherwise default 2s.
        assert_eq!(retry_backoff(Some(5)), Duration::from_secs(5));
        assert_eq!(retry_backoff(Some(0)), Duration::from_secs(1));
        assert_eq!(retry_backoff(Some(300)), Duration::from_secs(30));
        assert_eq!(retry_backoff(None), Duration::from_secs(2));
    }

    // Local test server: returns the given responses in order (one response per
    // connection, connection: close). Returns how many requests arrived.
    fn serve_responses(
        responses: Vec<String>,
    ) -> (std::net::SocketAddr, std::thread::JoinHandle<usize>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut served = 0;
            for resp in responses {
                let (mut stream, _) = listener.accept().unwrap();
                // Read the request fully (headers + content-length body) —
                // otherwise a socket closed after the response may send RST.
                let mut data = Vec::new();
                let mut buf = [0u8; 4096];
                let total = loop {
                    let n = stream.read(&mut buf).unwrap();
                    if n == 0 {
                        break None;
                    }
                    data.extend_from_slice(&buf[..n]);
                    if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&data[..pos]).to_lowercase();
                        let cl = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break Some(pos + 4 + cl);
                    }
                };
                if let Some(total) = total {
                    while data.len() < total {
                        let n = stream.read(&mut buf).unwrap();
                        if n == 0 {
                            break;
                        }
                        data.extend_from_slice(&buf[..n]);
                    }
                }
                stream.write_all(resp.as_bytes()).unwrap();
                served += 1;
            }
            served
        });
        (addr, handle)
    }

    // Like serve_responses but for ONE request — returns the captured raw request
    // text (head + body) so a test can assert on the URL path, headers, and the
    // JSON body the battery sent. Used by the issue #199 override e2e tests.
    fn serve_capture(response: String) -> (std::net::SocketAddr, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut data = Vec::new();
            let mut buf = [0u8; 4096];
            let total = loop {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break None;
                }
                data.extend_from_slice(&buf[..n]);
                if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&data[..pos]).to_lowercase();
                    let cl = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    break Some(pos + 4 + cl);
                }
            };
            if let Some(total) = total {
                while data.len() < total {
                    let n = stream.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buf[..n]);
                }
            }
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&data).to_string()
        });
        (addr, handle)
    }

    // A minimal OpenAI-style 200 response with the given text.
    fn openai_200(text: &str) -> String {
        let body = format!(
            "{{\"choices\":[{{\"finish_reason\":\"stop\",\"message\":{{\"role\":\"assistant\",\"content\":\"{}\"}}}}],\"usage\":{{\"prompt_tokens\":1,\"completion_tokens\":1}}}}",
            text
        );
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        )
    }

    #[test]
    fn config_override_url_style_extra_headers() {
        // issue #199 e2e: ai.config selects the OpenAI wire format, swaps the URL
        // to a local server, adds a header + an extra body field. ai.ask must hit
        // THAT url and the request must carry the override. Key is given inline
        // (no env) so this is independent of the environment.
        let (addr, handle) = serve_capture(openai_200("hi from glm"));
        let url = format!("http://{}/api/paas/v4/chat/completions", addr);

        let interp = Interp::new();
        let mut headers = BTreeMap::new();
        headers.insert("X-Title".to_string(), Value::Str("Fluxon".to_string()));
        let mut extra = BTreeMap::new();
        extra.insert("provider".to_string(), Value::Str("zai".to_string()));
        let cfg = opts(&[
            ("url", Value::Str(url.clone())),
            ("style", Value::Sym("openai".to_string())),
            ("key", Value::Str("sk-test".to_string())),
            ("model", Value::Str("glm-4.6".to_string())),
            ("headers", Value::Map(headers)),
            ("extra", Value::Map(extra)),
        ]);
        // ai.config {...}  (Flow has no Debug -> match, not .expect)
        if interp.ai_dispatch("config", vec![Value::Map(cfg)]).is_err() {
            panic!("ai.config failed");
        }
        // ai.ask "hi"
        let out = match interp.ai_dispatch("ask", vec![Value::Str("hi".to_string())]) {
            Ok(v) => v,
            Err(_) => panic!("ai.ask failed"),
        };
        match out {
            Value::Str(s) => assert_eq!(s, "hi from glm"),
            _ => panic!("expected str"),
        }

        let req = handle.join().unwrap();
        let lower = req.to_lowercase();
        // hit the custom path.
        assert!(
            req.contains("POST /api/paas/v4/chat/completions"),
            "request line: {}",
            req.lines().next().unwrap_or("")
        );
        // OpenAI wire format: Bearer auth, model + the extra body field present.
        assert!(lower.contains("authorization: bearer sk-test"), "{}", req);
        assert!(lower.contains("x-title: fluxon"), "{}", req);
        assert!(req.contains("\"model\":\"glm-4.6\""), "{}", req);
        assert!(req.contains("\"provider\":\"zai\""), "{}", req);
        // content-type emitted exactly once (not duplicated by the extra-header
        // path) when the user did NOT override it.
        assert_eq!(
            lower.matches("content-type:").count(),
            1,
            "exactly one content-type header: {}",
            req
        );
    }

    #[test]
    fn content_type_override_not_duplicated() {
        // A user-supplied content-type REPLACES the default (no duplicate header).
        let (addr, handle) = serve_capture(openai_200("ok"));
        let url = format!("http://{}/v1/chat/completions", addr);
        let interp = Interp::new();
        let mut headers = BTreeMap::new();
        headers.insert(
            "Content-Type".to_string(),
            Value::Str("application/json; charset=utf-8".to_string()),
        );
        let cfg = opts(&[
            ("url", Value::Str(url)),
            ("style", Value::Sym("openai".to_string())),
            ("key", Value::Str("sk".to_string())),
            ("headers", Value::Map(headers)),
        ]);
        if interp.ai_dispatch("config", vec![Value::Map(cfg)]).is_err() {
            panic!("ai.config failed");
        }
        if interp
            .ai_dispatch("ask", vec![Value::Str("hi".to_string())])
            .is_err()
        {
            panic!("ai.ask failed");
        }
        let req = handle.join().unwrap().to_lowercase();
        // exactly one content-type, and it is the user's value.
        assert_eq!(
            req.matches("content-type:").count(),
            1,
            "no duplicate content-type: {}",
            req
        );
        assert!(req.contains("content-type: application/json; charset=utf-8"));
    }

    #[test]
    fn inline_key_wins_over_env() {
        // Regression for PR #205 review (P2): an inline `key` must win over a key
        // present in the environment, otherwise a per-call/config override on a
        // deployment that already has $AI_KEY/$OPENAI_API_KEY set would keep
        // sending the env key (wrong provider). We don't touch the env here: the
        // test machine may have a standard key set, but the inline `key` must be
        // the one on the wire regardless. (generic = ov.key.or(env), so a Some
        // inline key short-circuits before the env is read.)
        let (addr, handle) = serve_capture(openai_200("ok"));
        let url = format!("http://{}/v1/chat/completions", addr);
        let interp = Interp::new();
        let cfg = opts(&[
            ("url", Value::Str(url)),
            ("style", Value::Sym("openai".to_string())),
            ("key", Value::Str("sk-inline-wins".to_string())),
        ]);
        if interp.ai_dispatch("config", vec![Value::Map(cfg)]).is_err() {
            panic!("ai.config failed");
        }
        if interp
            .ai_dispatch("ask", vec![Value::Str("hi".to_string())])
            .is_err()
        {
            panic!("ai.ask failed");
        }
        let req = handle.join().unwrap().to_lowercase();
        assert!(
            req.contains("authorization: bearer sk-inline-wins"),
            "inline key must be on the wire: {}",
            req
        );
    }

    #[test]
    fn header_keys_case_insensitive_across_merge() {
        // Regression for PR #205 review (P2): a global `ai.config` header and a
        // per-call header that differ ONLY in case must NOT both reach the wire
        // (HTTP names are case-insensitive; duplicates confuse gateways). The
        // per-call value must win, as a single header.
        let (addr, handle) = serve_capture(openai_200("ok"));
        let url = format!("http://{}/v1/chat/completions", addr);
        let interp = Interp::new();
        // Global config sets `Authorization` (capitalized) to a stale value.
        let mut g_headers = BTreeMap::new();
        g_headers.insert(
            "Authorization".to_string(),
            Value::Str("Bearer stale".to_string()),
        );
        let cfg = opts(&[
            ("url", Value::Str(url)),
            ("style", Value::Sym("openai".to_string())),
            ("key", Value::Str("sk".to_string())),
            ("headers", Value::Map(g_headers)),
        ]);
        if interp.ai_dispatch("config", vec![Value::Map(cfg)]).is_err() {
            panic!("ai.config failed");
        }
        // Per-call overrides the SAME header with different casing.
        let mut p_headers = BTreeMap::new();
        p_headers.insert(
            "authorization".to_string(),
            Value::Str("Bearer fresh".to_string()),
        );
        let per = opts(&[("headers", Value::Map(p_headers))]);
        if interp
            .ai_dispatch("ask", vec![Value::Str("hi".to_string()), Value::Map(per)])
            .is_err()
        {
            panic!("ai.ask failed");
        }
        let req = handle.join().unwrap().to_lowercase();
        // Exactly one authorization header, and it is the per-call value. (The
        // default Bearer is also skipped because authorization is overridden.)
        assert_eq!(
            req.matches("authorization:").count(),
            1,
            "exactly one authorization header: {}",
            req
        );
        assert!(req.contains("authorization: bearer fresh"), "{}", req);
        assert!(!req.contains("bearer stale"), "{}", req);
    }

    #[test]
    fn per_call_opts_override_global_config() {
        // Per-call opts win over ai.config: config points at one model + the live
        // url; the call overrides ONLY the model (same host, so the global key is
        // correctly reused — a model change is not a retarget).
        let (addr, handle) = serve_capture(openai_200("ok"));
        let url = format!("http://{}/v1/chat/completions", addr);

        let interp = Interp::new();
        // Global config: openai style + key + the live url + a stale model.
        let global = opts(&[
            ("style", Value::Sym("openai".to_string())),
            ("key", Value::Str("sk-global".to_string())),
            ("model", Value::Str("stale".to_string())),
            ("url", Value::Str(url)),
        ]);
        if interp
            .ai_dispatch("config", vec![Value::Map(global)])
            .is_err()
        {
            panic!("ai.config failed");
        }
        // Per-call: override only the model.
        let per = opts(&[("model", Value::Str("glm-4.6".to_string()))]);
        if interp
            .ai_dispatch("ask", vec![Value::Str("hi".to_string()), Value::Map(per)])
            .is_err()
        {
            panic!("ai.ask failed");
        }

        let req = handle.join().unwrap();
        // The per-call model won; the global key survived (merge, same host).
        assert!(req.contains("\"model\":\"glm-4.6\""), "{}", req);
        assert!(
            req.to_lowercase()
                .contains("authorization: bearer sk-global"),
            "{}",
            req
        );
    }

    #[test]
    fn config_partial_merge_switches_one_field() {
        // issue #200: ai.config is a PARTIAL update — a `/model` switch changes
        // ONLY the model and KEEPS the key/url/style already set (the same host,
        // so the inherited key is correctly reused — not a retarget). Both calls
        // hit the SAME local server; only the model differs between them.
        let (addr, h) = serve_responses(vec![openai_200("first"), openai_200("second")]);
        let url = format!("http://{}/v1/chat", addr);

        let interp = Interp::new();
        // Initial config: openai style + key + url + model.
        let initial = opts(&[
            ("style", Value::Sym("openai".to_string())),
            ("key", Value::Str("sk-carry".to_string())),
            ("url", Value::Str(url)),
            ("model", Value::Str("model-a".to_string())),
        ]);
        if interp
            .ai_dispatch("config", vec![Value::Map(initial)])
            .is_err()
        {
            panic!("ai.config (initial) failed");
        }
        // ai.ask -> model-a on the carried key.
        match interp.ai_dispatch("ask", vec![Value::Str("hi".to_string())]) {
            Ok(Value::Str(s)) => assert_eq!(s, "first"),
            _ => panic!("ai.ask (1) failed"),
        }

        // /model command: switch ONLY the model — same host, so key/url/style
        // carry over (a model change is not a retarget).
        let switch = opts(&[("model", Value::Str("model-b".to_string()))]);
        if interp
            .ai_dispatch("config", vec![Value::Map(switch)])
            .is_err()
        {
            panic!("ai.config (switch) failed");
        }
        // The carried key/url still reach the server, now with model-b.
        match interp.ai_dispatch("ask", vec![Value::Str("hi again".to_string())]) {
            Ok(Value::Str(s)) => assert_eq!(s, "second"),
            _ => panic!("ai.ask (2) failed"),
        }
        // Two requests reached the same server (the merge kept the url + key).
        assert_eq!(h.join().unwrap(), 2, "both calls must hit the carried url");

        // And the stored override now carries model-b with the original key/url.
        let ov = interp.ai_override.lock().unwrap();
        assert_eq!(ov.model.as_deref(), Some("model-b"));
        assert_eq!(ov.key.as_deref(), Some("sk-carry"), "key carried over");
    }

    #[test]
    fn config_key_switch_drops_stale_auth_header() {
        // issue #200 (Codex P1): a previous config sets `authorization` via
        // `headers` (a gateway scheme). A later /model-style switch supplies a new
        // `key` (+url) WITHOUT restating headers. The merge must DROP the stale
        // authorization header, so the new key is sent as Bearer to the new host —
        // not the old token. (Pure merge test, no network.)
        let global = AiOverride {
            style: Some(Provider::OpenAI),
            headers: [("authorization".to_string(), "Bearer OLD".to_string())].into(),
            ..AiOverride::default()
        };
        let switch = AiOverride {
            key: Some("NEW".to_string()),
            url: Some("https://new-host".to_string()),
            ..AiOverride::default()
        };
        let merged = global.merge(&switch);
        // The stale header is gone — so the Bearer generated from `key` is what
        // call_openai will emit (header_overridden returns false now).
        assert!(
            !header_overridden(&merged.headers, "authorization"),
            "stale auth header must be dropped on a key switch"
        );
        assert_eq!(merged.key.as_deref(), Some("NEW"));
    }

    #[test]
    fn config_url_switch_drops_stale_auth_header() {
        // issue #200 (review P1): a URL-only switch (no key change) must ALSO drop
        // a stale inherited auth header — otherwise the previous host's token is
        // forwarded to the new host. This is the same leak as the key switch,
        // reached via the host-switch path.
        let global = AiOverride {
            style: Some(Provider::OpenAI),
            url: Some("https://host-a".to_string()),
            headers: [("authorization".to_string(), "Bearer TOKEN_A".to_string())].into(),
            ..AiOverride::default()
        };
        let switch = AiOverride {
            url: Some("https://host-b".to_string()),
            ..AiOverride::default()
        };
        let merged = global.merge(&switch);
        assert!(
            !header_overridden(&merged.headers, "authorization"),
            "a url switch must drop the stale auth header too"
        );
    }

    #[test]
    fn config_style_switch_drops_stale_auth_header() {
        // issue #200 (review P1/P2): a style switch retargets the wire format (and
        // the auth header NAME differs per style) — drop the inherited auth header.
        let global = AiOverride {
            style: Some(Provider::Anthropic),
            headers: [("x-api-key".to_string(), "KEY_A".to_string())].into(),
            ..AiOverride::default()
        };
        let switch = AiOverride {
            style: Some(Provider::OpenAI),
            ..AiOverride::default()
        };
        let merged = global.merge(&switch);
        assert!(
            !header_overridden(&merged.headers, "x-api-key"),
            "a style switch must drop the stale x-api-key"
        );
    }

    #[test]
    fn config_switch_keeps_non_auth_headers() {
        // The drop is SCOPED to auth headers — a host-agnostic header the user set
        // globally (X-Title, HTTP-Referer) must carry over across a switch.
        let global = AiOverride {
            url: Some("https://host-a".to_string()),
            headers: [
                ("authorization".to_string(), "Bearer A".to_string()),
                ("x-title".to_string(), "MyApp".to_string()),
            ]
            .into(),
            ..AiOverride::default()
        };
        let switch = AiOverride {
            url: Some("https://host-b".to_string()),
            ..AiOverride::default()
        };
        let merged = global.merge(&switch);
        assert!(!header_overridden(&merged.headers, "authorization"));
        assert_eq!(
            merged.headers.get("x-title").map(|s| s.as_str()),
            Some("MyApp"),
            "non-auth headers must survive a switch"
        );
    }

    #[test]
    fn config_switch_drops_gateway_auth_header() {
        // The drop covers common gateway/cloud auth schemes too (Azure api-key,
        // Google x-goog-api-key), not just authorization/x-api-key.
        let global = AiOverride {
            headers: [
                ("api-key".to_string(), "AZ_OLD".to_string()),
                ("x-goog-api-key".to_string(), "G_OLD".to_string()),
            ]
            .into(),
            ..AiOverride::default()
        };
        let switch = AiOverride {
            key: Some("NEW".to_string()),
            url: Some("https://host-b".to_string()),
            ..AiOverride::default()
        };
        let merged = global.merge(&switch);
        assert!(!merged.headers.contains_key("api-key"), "Azure api-key");
        assert!(
            !merged.headers.contains_key("x-goog-api-key"),
            "Google x-goog-api-key"
        );
    }

    #[test]
    fn config_no_retarget_keeps_auth_header() {
        // A switch that does NOT retarget (only `model` changes) must KEEP the
        // inherited auth header — the model is a property of the same host/key.
        let global = AiOverride {
            url: Some("https://host-a".to_string()),
            headers: [("authorization".to_string(), "Bearer A".to_string())].into(),
            ..AiOverride::default()
        };
        let switch = AiOverride {
            model: Some("gpt-4o".to_string()),
            ..AiOverride::default()
        };
        let merged = global.merge(&switch);
        assert_eq!(
            merged.headers.get("authorization").map(|s| s.as_str()),
            Some("Bearer A"),
            "a model-only switch must keep the auth header"
        );
    }

    #[test]
    fn config_url_switch_drops_inherited_key_when_new_key_nil() {
        // issue #200 (Codex P1 round 3): switching to a new url where the new key
        // resolves to nil (env unset -> parse_override skips it) must NOT keep the
        // PREVIOUS host's key — the merge would otherwise reuse the old credential
        // for the new host. On a retarget a missing new key drops the inherited
        // one (falls through to env / errors, like the old replace semantics).
        let global = AiOverride {
            url: Some("https://host-a".to_string()),
            key: Some("sk-old".to_string()),
            ..AiOverride::default()
        };
        // {url:new} with key nil -> parsed override has url set, key None.
        let switch = AiOverride {
            url: Some("https://host-b".to_string()),
            ..AiOverride::default()
        };
        let merged = global.merge(&switch);
        assert!(
            merged.key.is_none(),
            "the old key must not carry over to a new host"
        );
        assert_eq!(merged.url.as_deref(), Some("https://host-b"));
    }

    #[test]
    fn config_url_switch_with_explicit_new_key_uses_it() {
        // The drop must not lose a key the switch DOES provide: a retarget with a
        // fresh key keeps that fresh key.
        let global = AiOverride {
            url: Some("https://host-a".to_string()),
            key: Some("sk-old".to_string()),
            ..AiOverride::default()
        };
        let switch = AiOverride {
            url: Some("https://host-b".to_string()),
            key: Some("sk-new".to_string()),
            ..AiOverride::default()
        };
        let merged = global.merge(&switch);
        assert_eq!(merged.key.as_deref(), Some("sk-new"));
    }

    #[test]
    fn config_restate_same_target_keeps_key() {
        // The dual of the drop: restating the SAME url while changing only model
        // is not a retarget, so the inherited key survives.
        let global = AiOverride {
            url: Some("https://host-a".to_string()),
            key: Some("sk-keep".to_string()),
            model: Some("model-a".to_string()),
            ..AiOverride::default()
        };
        let switch = AiOverride {
            url: Some("https://host-a".to_string()),
            model: Some("model-b".to_string()),
            ..AiOverride::default()
        };
        let merged = global.merge(&switch);
        assert_eq!(
            merged.key.as_deref(),
            Some("sk-keep"),
            "restating the same target keeps the inherited key"
        );
    }

    #[test]
    fn config_restate_same_target_keeps_auth_header() {
        // issue #200 (Codex P2): retarget is decided by VALUE change, not mere
        // presence. A reusable profile map restates the SAME url/style/key while
        // changing only `model` — that is NOT a retarget, so the inherited auth
        // header must survive. (Mere-presence logic would wrongly strip it.)
        let global = AiOverride {
            style: Some(Provider::OpenAI),
            url: Some("https://host-a".to_string()),
            key: Some("sk-1".to_string()),
            model: Some("model-a".to_string()),
            headers: [("authorization".to_string(), "Bearer GATEWAY".to_string())].into(),
            ..AiOverride::default()
        };
        // Same url/style/key restated, only the model differs.
        let switch = AiOverride {
            style: Some(Provider::OpenAI),
            url: Some("https://host-a".to_string()),
            key: Some("sk-1".to_string()),
            model: Some("model-b".to_string()),
            ..AiOverride::default()
        };
        let merged = global.merge(&switch);
        assert_eq!(
            merged.headers.get("authorization").map(|s| s.as_str()),
            Some("Bearer GATEWAY"),
            "restating the same target must keep the auth header"
        );
        assert_eq!(merged.model.as_deref(), Some("model-b"));
    }

    #[test]
    fn config_key_switch_keeps_explicit_new_auth_header() {
        // The drop must NOT clobber an auth header the SWITCH itself restates: if
        // the new config gives both `key` and `headers.authorization`, the
        // explicit header wins (a gateway with a custom scheme + a key elsewhere).
        let global = AiOverride {
            headers: [("authorization".to_string(), "Bearer OLD".to_string())].into(),
            ..AiOverride::default()
        };
        let switch = AiOverride {
            key: Some("NEW".to_string()),
            headers: [("authorization".to_string(), "Custom FRESH".to_string())].into(),
            ..AiOverride::default()
        };
        let merged = global.merge(&switch);
        assert_eq!(
            merged.headers.get("authorization").map(|s| s.as_str()),
            Some("Custom FRESH"),
            "an explicit auth header in the switch must survive"
        );
    }

    #[test]
    fn config_key_switch_drops_stale_auth_e2e() {
        // e2e for the Codex P1: ai.config sets a stale authorization header, then
        // ai.config {key url} switches without restating headers. The wire must
        // carry the NEW key's Bearer, exactly once, and NOT the stale token.
        let (addr, handle) = serve_capture(openai_200("ok"));
        let url = format!("http://{}/v1/chat/completions", addr);
        let interp = Interp::new();
        // Initial config: a gateway authorization header (and a dummy url/key).
        let mut g_headers = BTreeMap::new();
        g_headers.insert(
            "Authorization".to_string(),
            Value::Str("Bearer STALE".to_string()),
        );
        let initial = opts(&[
            ("style", Value::Sym("openai".to_string())),
            ("key", Value::Str("sk-old".to_string())),
            ("url", Value::Str("http://127.0.0.1:1/dead".to_string())),
            ("headers", Value::Map(g_headers)),
        ]);
        if interp
            .ai_dispatch("config", vec![Value::Map(initial)])
            .is_err()
        {
            panic!("ai.config (initial) failed");
        }
        // /model-style switch: new key + url, no headers restated.
        let switch = opts(&[
            ("key", Value::Str("sk-new".to_string())),
            ("url", Value::Str(url)),
        ]);
        if interp
            .ai_dispatch("config", vec![Value::Map(switch)])
            .is_err()
        {
            panic!("ai.config (switch) failed");
        }
        if interp
            .ai_dispatch("ask", vec![Value::Str("hi".to_string())])
            .is_err()
        {
            panic!("ai.ask failed");
        }
        let req = handle.join().unwrap().to_lowercase();
        assert_eq!(
            req.matches("authorization:").count(),
            1,
            "exactly one authorization header: {}",
            req
        );
        assert!(
            req.contains("authorization: bearer sk-new"),
            "the new key must be sent: {}",
            req
        );
        assert!(
            !req.contains("bearer stale"),
            "the stale token must NOT be sent: {}",
            req
        );
    }

    #[test]
    fn config_empty_map_clears() {
        // issue #200: ai.config {} (empty) RESETS to the env/auto defaults — the
        // escape hatch from a partial-merge config. After a config with a custom
        // key, an empty call must leave a fully empty override.
        let interp = Interp::new();
        let cfg = opts(&[
            ("style", Value::Sym("openai".to_string())),
            ("key", Value::Str("sk-x".to_string())),
            ("model", Value::Str("m".to_string())),
        ]);
        if interp.ai_dispatch("config", vec![Value::Map(cfg)]).is_err() {
            panic!("ai.config failed");
        }
        // Sanity: the override is set.
        {
            let ov = interp.ai_override.lock().unwrap();
            assert!(ov.key.is_some(), "config should be set before clear");
        }
        // Empty map -> clear.
        if interp
            .ai_dispatch("config", vec![Value::Map(BTreeMap::new())])
            .is_err()
        {
            panic!("ai.config {{}} failed");
        }
        let ov = interp.ai_override.lock().unwrap();
        assert!(ov.key.is_none() && ov.model.is_none() && ov.style.is_none());
    }

    #[test]
    fn default_override_is_empty() {
        // issue #199 acceptance (byte-for-byte unchanged): with no ai.config and
        // no per-call opts, the effective override is fully empty — every field
        // None / no headers / no extra. So `ai_config` falls through to exactly
        // the same env+default path as before this feature: same URL, same
        // headers, no extra body. (Env-independent: we assert the override, not a
        // resolved URL, to avoid depending on the test machine's env.)
        let interp = Interp::new();
        let eff = interp.effective_override(&AiOverride::default());
        assert!(eff.url.is_none());
        assert!(eff.style.is_none());
        assert!(eff.key.is_none());
        assert!(eff.model.is_none());
        assert!(eff.headers.is_empty());
        assert!(eff.extra.is_empty());
    }

    #[test]
    fn post_json_429_dan_keyin_retry_qiladi() {
        // issue #92 bonus: if the first response is 429 it retries ONCE — if the
        // second attempt returns 200 the result is successful.
        let r429 = "HTTP/1.1 429 Too Many Requests\r\nretry-after: 1\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_string();
        let r200 = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 8\r\nconnection: close\r\n\r\n{\"ok\":1}".to_string();
        let (addr, handle) = serve_responses(vec![r429, r200]);
        let url = format!("http://{}/v1/x", addr);
        let res = post_json(&url, "{}".to_string(), Some(Duration::from_secs(10)), |b| b);
        match res {
            Ok((text, _ms)) => assert_eq!(text, "{\"ok\":1}"),
            Err(Flow::Error(e)) => panic!("expected Ok after retry: {}", e),
            Err(_) => panic!("unexpected Flow"),
        }
        assert_eq!(
            handle.join().unwrap(),
            2,
            "two requests (original + retry) must arrive"
        );
    }

    #[test]
    fn post_json_ikkinchi_429_xato_qaytaradi() {
        // Only ONE retry — if the second attempt is also 429, an explicit error
        // is returned (no infinite retry loop).
        let r429 = "HTTP/1.1 429 Too Many Requests\r\nretry-after: 1\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_string();
        let (addr, handle) = serve_responses(vec![r429.clone(), r429]);
        let url = format!("http://{}/v1/x", addr);
        let res = post_json(&url, "{}".to_string(), Some(Duration::from_secs(10)), |b| b);
        match res {
            Err(Flow::Error(e)) => assert!(e.contains("429"), "expected 429 error: {}", e),
            Ok(_) => panic!("expected error after second 429"),
            Err(_) => panic!("expected Flow::Error"),
        }
        assert_eq!(handle.join().unwrap(), 2, "exactly two attempts must occur");
    }

    #[test]
    fn post_json_401_retry_qilmaydi() {
        // A permanent error (401 wrong key) — NO retry, error with a single request.
        let r401 = "HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            .to_string();
        let (addr, handle) = serve_responses(vec![r401]);
        let url = format!("http://{}/v1/x", addr);
        let res = post_json(&url, "{}".to_string(), Some(Duration::from_secs(10)), |b| b);
        match res {
            Err(Flow::Error(e)) => assert!(e.contains("401"), "expected 401 error: {}", e),
            Ok(_) => panic!("expected error after 401"),
            Err(_) => panic!("expected Flow::Error"),
        }
        assert_eq!(handle.join().unwrap(), 1, "only one request must occur");
    }

    #[test]
    fn strip_fence_works() {
        assert_eq!(strip_code_fence("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(strip_code_fence("```\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(strip_code_fence("{\"a\":1}"), "{\"a\":1}");
        assert_eq!(strip_code_fence("  {\"a\":1}  "), "{\"a\":1}");
    }

    #[test]
    fn json_type_maps() {
        assert_eq!(json_type("str"), "string");
        assert_eq!(json_type("int"), "integer");
        assert_eq!(json_type("flt"), "number");
        assert_eq!(json_type("bool"), "boolean");
        assert_eq!(json_type("array"), "array");
    }

    #[test]
    fn wrap_simple_schema() {
        // {name:str age:int} -> a JSON-schema object.
        let mut s = BTreeMap::new();
        s.insert("name".to_string(), Value::Sym("str".to_string()));
        s.insert("age".to_string(), Value::Sym("int".to_string()));
        let wrapped = wrap_schema(Value::Map(s));
        let m = match wrapped {
            Value::Map(m) => m,
            _ => panic!("expected map"),
        };
        assert_eq!(as_str(m.get("type").unwrap()).as_deref(), Some("object"));
        let props = match m.get("properties").unwrap() {
            Value::Map(p) => p,
            _ => panic!(),
        };
        let name_ty = match props.get("name").unwrap() {
            Value::Map(f) => as_str(f.get("type").unwrap()),
            _ => None,
        };
        assert_eq!(name_ty.as_deref(), Some("string"));
    }

    #[test]
    fn wrap_array_schema() {
        // {tags:[str] items:[{name:str}]} -> array types are built correctly.
        let mut s = BTreeMap::new();
        s.insert(
            "tags".to_string(),
            Value::List(vec![Value::Sym("str".to_string())]),
        );
        let mut item_obj = BTreeMap::new();
        item_obj.insert("name".to_string(), Value::Sym("str".to_string()));
        s.insert("items".to_string(), Value::List(vec![Value::Map(item_obj)]));
        let wrapped = wrap_schema(Value::Map(s));
        let props = match &wrapped {
            Value::Map(m) => match m.get("properties").unwrap() {
                Value::Map(p) => p.clone(),
                _ => panic!(),
            },
            _ => panic!("expected map"),
        };
        // tags -> {type:array, items:{type:string}}
        let tags = match props.get("tags").unwrap() {
            Value::Map(f) => f.clone(),
            _ => panic!(),
        };
        assert_eq!(as_str(tags.get("type").unwrap()).as_deref(), Some("array"));
        let tags_items = match tags.get("items").unwrap() {
            Value::Map(i) => i.clone(),
            _ => panic!("tags items must be a map"),
        };
        assert_eq!(
            as_str(tags_items.get("type").unwrap()).as_deref(),
            Some("string")
        );
        // items -> {type:array, items:{type:object, properties:{name:...}}}
        let items = match props.get("items").unwrap() {
            Value::Map(f) => f.clone(),
            _ => panic!(),
        };
        assert_eq!(as_str(items.get("type").unwrap()).as_deref(), Some("array"));
        let elem = match items.get("items").unwrap() {
            Value::Map(e) => e.clone(),
            _ => panic!("items items must be a map"),
        };
        assert_eq!(as_str(elem.get("type").unwrap()).as_deref(), Some("object"));
        let elem_props = match elem.get("properties").unwrap() {
            Value::Map(p) => p.clone(),
            _ => panic!(),
        };
        assert!(elem_props.contains_key("name"));
    }

    #[test]
    fn wrap_passthrough_full_schema() {
        // Already a type:object schema -> left as-is.
        // (Value does not derive Debug/PartialEq -> we compare with .equals.)
        let mut s = BTreeMap::new();
        s.insert("type".to_string(), Value::Str("object".to_string()));
        s.insert("properties".to_string(), Value::Map(BTreeMap::new()));
        let wrapped = wrap_schema(Value::Map(s.clone()));
        assert!(wrapped.equals(&Value::Map(s)));
    }

    #[test]
    fn normalize_tool_shape() {
        // {name desc params} -> {name description input_schema}.
        let mut params = BTreeMap::new();
        params.insert("city".to_string(), Value::Sym("str".to_string()));
        let mut t = BTreeMap::new();
        t.insert("name".to_string(), Value::Str("weather".to_string()));
        t.insert("desc".to_string(), Value::Str("weather".to_string()));
        t.insert("params".to_string(), Value::Map(params));
        let out = match normalize_tool(&Value::Map(t)) {
            Value::Map(m) => m,
            _ => panic!(),
        };
        assert_eq!(as_str(out.get("name").unwrap()).as_deref(), Some("weather"));
        assert!(out.contains_key("description"));
        assert!(out.contains_key("input_schema"));
    }

    #[test]
    fn normalize_tool_msg() {
        // {role::tool id content} -> user + tool_result block.
        let mut t = BTreeMap::new();
        t.insert("role".to_string(), Value::Sym("tool".to_string()));
        t.insert("id".to_string(), Value::Str("toolu_1".to_string()));
        t.insert("content".to_string(), Value::Str("25 degrees".to_string()));
        let out = match normalize_msg(&Value::Map(t)) {
            Value::Map(m) => m,
            _ => panic!(),
        };
        assert_eq!(as_str(out.get("role").unwrap()).as_deref(), Some("user"));
        match out.get("content").unwrap() {
            Value::List(l) => {
                let blk = match &l[0] {
                    Value::Map(m) => m,
                    _ => panic!(),
                };
                assert_eq!(
                    as_str(blk.get("type").unwrap()).as_deref(),
                    Some("tool_result")
                );
                assert_eq!(
                    as_str(blk.get("tool_use_id").unwrap()).as_deref(),
                    Some("toolu_1")
                );
            }
            _ => panic!("expected tool_result list"),
        }
    }

    #[test]
    fn normalize_sym_role() {
        // {role::user content} -> role is turned into a str.
        let mut t = BTreeMap::new();
        t.insert("role".to_string(), Value::Sym("user".to_string()));
        t.insert("content".to_string(), Value::Str("hello".to_string()));
        let out = match normalize_msg(&Value::Map(t)) {
            Value::Map(m) => m,
            _ => panic!(),
        };
        assert_eq!(as_str(out.get("role").unwrap()).as_deref(), Some("user"));
    }

    // --- issue #201: streaming (SSE) ---

    // feed_line returns Result<Option<String>, Flow> (Flow has no Debug -> no
    // assert_eq on the Result). These helpers assert the Ok cases concisely.
    fn fed_delta(acc: &mut SseAccumulator, line: &str) -> String {
        match acc.feed_line(line) {
            Ok(Some(d)) => d,
            Ok(None) => panic!("expected a delta for: {}", line),
            Err(_) => panic!("unexpected stream error for: {}", line),
        }
    }
    fn fed_none(acc: &mut SseAccumulator, line: &str) {
        match acc.feed_line(line) {
            Ok(None) => {}
            Ok(Some(d)) => panic!("expected no delta, got {:?} for: {}", d, line),
            Err(_) => panic!("unexpected stream error for: {}", line),
        }
    }

    #[test]
    fn sse_anthropic_accumulates_text_and_usage() {
        // Anthropic SSE: message_start (input usage) -> content_block_delta(s) ->
        // message_delta (stop_reason + output usage). feed_line yields each text
        // delta; finish builds the full AiResp.
        let mut acc = SseAccumulator::new(Provider::Anthropic);
        fed_none(
            &mut acc,
            r#"data: {"type":"message_start","message":{"usage":{"input_tokens":12}}}"#,
        );
        assert_eq!(
            fed_delta(
                &mut acc,
                r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"Hel"}}"#
            ),
            "Hel"
        );
        assert_eq!(
            fed_delta(
                &mut acc,
                r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"lo"}}"#
            ),
            "lo"
        );
        // ping / unrelated events yield nothing.
        fed_none(&mut acc, r#"data: {"type":"ping"}"#);
        fed_none(
            &mut acc,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":7}}"#,
        );
        let r = match acc.finish("claude-opus-4-8", 5) {
            Ok(r) => r,
            Err(_) => panic!("finish failed"),
        };
        assert_eq!(r.text, "Hello");
        assert_eq!(r.in_tokens, 12);
        assert_eq!(r.out_tokens, 7);
        assert!((r.conf - 0.9).abs() < 1e-9);
    }

    #[test]
    fn sse_openai_accumulates_text_and_usage() {
        // OpenAI SSE: choices[0].delta.content chunks, then a final usage-only
        // chunk (empty choices) + [DONE].
        let mut acc = SseAccumulator::new(Provider::OpenAI);
        assert_eq!(
            fed_delta(
                &mut acc,
                r#"data: {"choices":[{"delta":{"content":"Hi"}}]}"#
            ),
            "Hi"
        );
        assert_eq!(
            fed_delta(
                &mut acc,
                r#"data: {"choices":[{"delta":{"content":" there"},"finish_reason":"stop"}]}"#
            ),
            " there"
        );
        // Final usage chunk — empty choices, carries token counts.
        fed_none(
            &mut acc,
            r#"data: {"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":4}}"#,
        );
        fed_none(&mut acc, "data: [DONE]");
        let r = match acc.finish("gpt-4o", 9) {
            Ok(r) => r,
            Err(_) => panic!("finish failed"),
        };
        assert_eq!(r.text, "Hi there");
        assert_eq!(r.in_tokens, 3);
        assert_eq!(r.out_tokens, 4);
    }

    #[test]
    fn sse_ignores_non_data_and_blank_lines() {
        // `event:` lines, comments, and blanks are not `data:` -> no delta.
        let mut acc = SseAccumulator::new(Provider::Anthropic);
        fed_none(&mut acc, "event: content_block_delta");
        fed_none(&mut acc, "");
        fed_none(&mut acc, ": this is a comment");
        fed_none(&mut acc, "data:"); // empty data
    }

    #[test]
    fn sse_error_event_is_surfaced() {
        // issue #201 (Codex P2): a 200 stream can still emit `type:"error"`
        // mid-stream (overload/rate-limit). It must abort with the provider's
        // message, not finish as a partial success.
        let mut acc = SseAccumulator::new(Provider::Anthropic);
        // some text arrived first...
        assert_eq!(
            fed_delta(
                &mut acc,
                r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"par"}}"#
            ),
            "par"
        );
        // ...then the provider errors.
        match acc.feed_line(
            r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        ) {
            Err(Flow::Error(e)) => {
                assert!(e.contains("stream error event"), "{}", e);
                assert!(e.contains("Overloaded"), "message must surface: {}", e);
            }
            _ => panic!("a type:error event must be surfaced as an error"),
        }
    }

    #[test]
    fn sse_openai_error_chunk_is_surfaced() {
        // issue #201 (review P1): OpenAI-family providers signal a mid-stream
        // error as {"error":{...}} with NO top-level type:"error" — it must STILL
        // be surfaced, not silently dropped into a partial success.
        let mut acc = SseAccumulator::new(Provider::OpenAI);
        assert_eq!(
            fed_delta(
                &mut acc,
                r#"data: {"choices":[{"delta":{"content":"par"}}]}"#
            ),
            "par"
        );
        match acc.feed_line(
            r#"data: {"error":{"message":"rate limit exceeded","type":"rate_limit_error"}}"#,
        ) {
            Err(Flow::Error(e)) => {
                assert!(e.contains("stream error event"), "{}", e);
                assert!(
                    e.contains("rate limit exceeded"),
                    "message must surface: {}",
                    e
                );
            }
            _ => panic!("an OpenAI error chunk must be surfaced as an error"),
        }
    }

    // Builds a raw SSE HTTP response (200, text/event-stream) from a list of
    // `data:` JSON event payloads.
    fn sse_response(events: &[&str]) -> String {
        let mut body = String::new();
        for e in events {
            body.push_str("data: ");
            body.push_str(e);
            body.push_str("\n\n");
        }
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        )
    }

    #[test]
    fn stream_e2e_callback_per_chunk_and_full_text() {
        // issue #201 e2e: ai.stream hits a local SSE server, the callback fires
        // per text chunk (we collect them via a reg-free Native fn), and the
        // returned value is the full accumulated text. OpenAI wire format + a
        // local url, key inline (env-independent).
        let events = [
            r#"{"choices":[{"delta":{"role":"assistant"}}]}"#,
            r#"{"choices":[{"delta":{"content":"Hello"}}]}"#,
            r#"{"choices":[{"delta":{"content":", "}}]}"#,
            r#"{"choices":[{"delta":{"content":"world"},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":3}}"#,
            "[DONE]",
        ];
        let (addr, handle) = serve_capture(sse_response(&events));
        let url = format!("http://{}/v1/chat/completions", addr);

        let interp = Interp::new();
        let cfg = opts(&[
            ("url", Value::Str(url)),
            ("style", Value::Sym("openai".to_string())),
            ("key", Value::Str("sk-test".to_string())),
            ("model", Value::Str("gpt-4o".to_string())),
        ]);
        if interp.ai_dispatch("config", vec![Value::Map(cfg)]).is_err() {
            panic!("ai.config failed");
        }

        // The callback appends each chunk to a shared buffer (Native fn captures
        // an Arc<Mutex<Vec>>). This mirrors what a Fluxon `\chunk -> io.print`
        // does, but lets the test assert the chunk sequence.
        let chunks: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let chunks_cb = chunks.clone();
        let cb = Value::Native(std::sync::Arc::new(crate::value::NativeFn {
            name: "on_chunk".to_string(),
            func: Box::new(move |args: Vec<Value>| {
                if let Some(Value::Str(s)) = args.first() {
                    chunks_cb.lock().unwrap().push(s.clone());
                }
                Ok(Value::Nil)
            }),
        }));

        let out = match interp.ai_dispatch("stream", vec![Value::Str("hi".to_string()), cb]) {
            Ok(v) => v,
            Err(_) => panic!("ai.stream failed"),
        };
        // The returned value is the full text.
        match out {
            Value::Str(s) => assert_eq!(s, "Hello, world"),
            _ => panic!("expected str"),
        }
        // The callback saw exactly the three text deltas, in order (no empty
        // role-only / usage / [DONE] chunks).
        let got = chunks.lock().unwrap().clone();
        assert_eq!(got, vec!["Hello", ", ", "world"]);

        // The request actually asked for a stream.
        let req = handle.join().unwrap();
        assert!(req.contains("\"stream\":true"), "{}", req);
        assert!(
            req.to_lowercase().contains("authorization: bearer sk-test"),
            "{}",
            req
        );
    }

    // An SSE server that writes the response in raw byte SLICES, flushing +
    // sleeping between them so the client receives them as SEPARATE frames. The
    // split offsets are chosen to cut a multibyte UTF-8 char in half across a
    // frame boundary (the Codex P2 regression scenario).
    fn serve_chunked_bytes(
        head_and_body: Vec<u8>,
        splits: Vec<usize>,
    ) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // Drain the request (we don't assert on it here).
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let mut prev = 0;
            for &at in &splits {
                stream.write_all(&head_and_body[prev..at]).unwrap();
                stream.flush().unwrap();
                std::thread::sleep(Duration::from_millis(15));
                prev = at;
            }
            stream.write_all(&head_and_body[prev..]).unwrap();
            stream.flush().unwrap();
        });
        (addr, handle)
    }

    #[test]
    fn stream_preserves_utf8_across_frame_boundary() {
        // issue #201 (Codex P2): a multibyte char split across two HTTP frames
        // must NOT be corrupted to U+FFFD. We stream "héllo — салом 🚀" as OpenAI
        // SSE and cut the body in the MIDDLE of multibyte chars at the byte level.
        let text = "héllo — салом 🚀"; // é=2B, —=3B, Cyrillic=2B each, 🚀=4B
        // Build the SSE body: one content delta with the text, then [DONE].
        let mut body = String::new();
        body.push_str("data: ");
        body.push_str(&format!(
            "{{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}},\"finish_reason\":\"stop\"}}]}}",
            text
        ));
        body.push_str("\n\n");
        body.push_str("data: [DONE]\n\n");
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        let full: Vec<u8> = head.bytes().chain(body.bytes()).collect();

        // Pick split offsets INSIDE the body that land mid-char. The 🚀 (4 bytes)
        // is near the end; split one byte into it, and also split inside the first
        // multibyte run. We find byte offsets of the rocket and a Cyrillic char.
        let head_len = head.len();
        let rocket_byte = head_len + body.find('🚀').unwrap() + 1; // 1 byte into 🚀
        let cyr_byte = head_len + body.find('с').unwrap() + 1; // 1 byte into с
        let mut splits = vec![cyr_byte, rocket_byte];
        splits.sort();

        let (addr, handle) = serve_chunked_bytes(full, splits);
        let url = format!("http://{}/v1/chat/completions", addr);
        let interp = Interp::new();
        let cfg = opts(&[
            ("url", Value::Str(url)),
            ("style", Value::Sym("openai".to_string())),
            ("key", Value::Str("sk-test".to_string())),
        ]);
        if interp.ai_dispatch("config", vec![Value::Map(cfg)]).is_err() {
            panic!("ai.config failed");
        }
        let cb = Value::Native(std::sync::Arc::new(crate::value::NativeFn {
            name: "noop".to_string(),
            func: Box::new(|_args: Vec<Value>| Ok(Value::Nil)),
        }));
        let out = match interp.ai_dispatch("stream", vec![Value::Str("hi".to_string()), cb]) {
            Ok(v) => v,
            Err(_) => panic!("ai.stream failed"),
        };
        handle.join().unwrap();
        // The full text round-trips intact — NO U+FFFD replacement characters.
        match out {
            Value::Str(s) => {
                assert_eq!(s, text, "multibyte text must survive frame splitting");
                assert!(!s.contains('\u{FFFD}'), "no replacement char: {:?}", s);
            }
            _ => panic!("expected str"),
        }
    }

    #[test]
    fn stream_requires_callback() {
        // Without a callback ai.stream is an error (it would be a slower ai.ask).
        let interp = Interp::new();
        match interp.ai_dispatch("stream", vec![Value::Str("hi".to_string())]) {
            Err(Flow::Error(e)) => assert!(e.contains("callback"), "{}", e),
            _ => panic!("expected a callback-required error"),
        }
    }

    #[test]
    fn parse_text_response() {
        let json = r#"{
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5},
            "content": [{"type": "text", "text": "answer"}]
        }"#;
        // Flow does not derive Debug -> match instead of .unwrap().
        let r = match parse_anthropic(json, "claude-opus-4-8", 100) {
            Ok(r) => r,
            Err(_) => panic!("parse failed"),
        };
        assert_eq!(r.text, "answer");
        assert!(r.tool_calls.is_empty());
        assert_eq!(r.in_tokens, 10);
        assert_eq!(r.out_tokens, 5);
        assert!((r.conf - 0.9).abs() < 1e-9);
    }

    #[test]
    fn parse_tool_use_response() {
        let json = r#"{
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 20, "output_tokens": 8},
            "content": [
                {"type": "text", "text": "let me check"},
                {"type": "tool_use", "id": "toolu_9", "name": "weather", "input": {"city": "Toshkent"}}
            ]
        }"#;
        let r = match parse_anthropic(json, "claude-opus-4-8", 50) {
            Ok(r) => r,
            Err(_) => panic!("parse failed"),
        };
        assert_eq!(r.tool_calls.len(), 1);
        let (name, input, id) = &r.tool_calls[0];
        assert_eq!(name, "weather");
        assert_eq!(id, "toolu_9");
        match input {
            Value::Map(m) => {
                assert_eq!(as_str(m.get("city").unwrap()).as_deref(), Some("Toshkent"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_parallel_tool_use() {
        // If the model calls TWO tools in one response, both are collected
        // (issue #95 — previously only the last one would remain).
        let json = r#"{
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 20, "output_tokens": 8},
            "content": [
                {"type": "tool_use", "id": "toolu_1", "name": "weather", "input": {"city": "Toshkent"}},
                {"type": "tool_use", "id": "toolu_2", "name": "time", "input": {"tz": "UTC"}}
            ]
        }"#;
        let r = match parse_anthropic(json, "claude-opus-4-8", 50) {
            Ok(r) => r,
            Err(_) => panic!("parse failed"),
        };
        assert_eq!(r.tool_calls.len(), 2);
        assert_eq!(r.tool_calls[0].0, "weather");
        assert_eq!(r.tool_calls[0].2, "toolu_1");
        assert_eq!(r.tool_calls[1].0, "time");
        assert_eq!(r.tool_calls[1].2, "toolu_2");
    }

    #[test]
    fn meta_fields() {
        let r = AiResp {
            text: "x".to_string(),
            tool_calls: Vec::new(),
            in_tokens: 1000,
            out_tokens: 500,
            model: "claude-opus-4-8".to_string(),
            ms: 1234,
            conf: 0.9,
        };
        let m = r.meta();
        assert!(as_int(m.get("tokens").unwrap()) == Some(1500));
        assert!(as_int(m.get("ms").unwrap()) == Some(1234));
        // cost: (1000*5 + 500*25)/1e6 = (5000+12500)/1e6 = 0.0175
        match m.get("cost").unwrap() {
            Value::Flt(c) => assert!((c - 0.0175).abs() < 1e-9),
            _ => panic!(),
        }
    }

    #[test]
    fn cost_by_model() {
        assert!((estimate_cost("claude-opus-4-8", 1_000_000, 0) - 5.0).abs() < 1e-9);
        assert!((estimate_cost("claude-sonnet-4-6", 1_000_000, 0) - 3.0).abs() < 1e-9);
        assert!((estimate_cost("claude-haiku-4-5", 1_000_000, 0) - 1.0).abs() < 1e-9);
        assert!((estimate_cost("gpt-4o", 1_000_000, 0) - 2.5).abs() < 1e-9);
        assert!((estimate_cost("gpt-4o-mini", 1_000_000, 0) - 0.15).abs() < 1e-9);
        assert_eq!(estimate_cost("unknown", 1_000_000, 0), 0.0);
    }

    #[test]
    fn parse_openai_text() {
        // OpenAI Chat Completions text response.
        let json = r#"{
            "choices": [{
                "finish_reason": "stop",
                "message": {"role": "assistant", "content": "hello"}
            }],
            "usage": {"prompt_tokens": 12, "completion_tokens": 4}
        }"#;
        let r = match parse_openai(json, "gpt-4o", 80) {
            Ok(r) => r,
            Err(_) => panic!("openai parse failed"),
        };
        assert_eq!(r.text, "hello");
        assert!(r.tool_calls.is_empty());
        assert_eq!(r.in_tokens, 12);
        assert_eq!(r.out_tokens, 4);
    }

    #[test]
    fn parse_openai_tool_call() {
        // OpenAI tool_calls: arguments JSON-STRING -> is parsed into a map.
        let json = r#"{
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_7",
                        "type": "function",
                        "function": {"name": "ob_havo", "arguments": "{\"shahar\":\"Toshkent\"}"}
                    }]
                }
            }],
            "usage": {"prompt_tokens": 30, "completion_tokens": 9}
        }"#;
        let r = match parse_openai(json, "gpt-4o", 60) {
            Ok(r) => r,
            Err(_) => panic!("openai tool parse failed"),
        };
        assert_eq!(r.tool_calls.len(), 1);
        let (name, input, id) = &r.tool_calls[0];
        assert_eq!(name, "ob_havo");
        assert_eq!(id, "call_7");
        match input {
            Value::Map(m) => {
                assert_eq!(
                    as_str(m.get("shahar").unwrap()).as_deref(),
                    Some("Toshkent")
                );
            }
            _ => panic!("expected args map"),
        }
    }

    #[test]
    fn parse_openai_parallel_tool_calls() {
        // OpenAI may also return several tool_call entries in one response —
        // all are collected (issue #95).
        let json = r#"{
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {"id": "call_1", "type": "function",
                         "function": {"name": "ob_havo", "arguments": "{\"shahar\":\"Toshkent\"}"}},
                        {"id": "call_2", "type": "function",
                         "function": {"name": "vaqt", "arguments": "{\"tz\":\"UTC\"}"}}
                    ]
                }
            }],
            "usage": {"prompt_tokens": 30, "completion_tokens": 9}
        }"#;
        let r = match parse_openai(json, "gpt-4o", 60) {
            Ok(r) => r,
            Err(_) => panic!("openai parallel parse failed"),
        };
        assert_eq!(r.tool_calls.len(), 2);
        assert_eq!(r.tool_calls[0].0, "ob_havo");
        assert_eq!(r.tool_calls[0].2, "call_1");
        assert_eq!(r.tool_calls[1].0, "vaqt");
        assert_eq!(r.tool_calls[1].2, "call_2");
    }

    #[test]
    fn anthropic_tool_to_openai_shape() {
        // {name description input_schema} -> {type:function function:{...}}.
        let mut t = BTreeMap::new();
        t.insert("name".to_string(), Value::Str("weather".to_string()));
        t.insert("description".to_string(), Value::Str("weather".to_string()));
        t.insert("input_schema".to_string(), empty_object_schema());
        let out = match anthropic_tool_to_openai(&Value::Map(t)) {
            Value::Map(m) => m,
            _ => panic!(),
        };
        assert_eq!(
            as_str(out.get("type").unwrap()).as_deref(),
            Some("function")
        );
        match out.get("function").unwrap() {
            Value::Map(f) => {
                assert_eq!(as_str(f.get("name").unwrap()).as_deref(), Some("weather"));
                assert!(f.contains_key("parameters"));
            }
            _ => panic!("expected function map"),
        }
    }

    #[test]
    fn anthropic_msg_to_openai_tool_result() {
        // Anthropic tool_result (user roli) -> OpenAI {role:tool tool_call_id content}.
        let mut blk = BTreeMap::new();
        blk.insert("type".to_string(), Value::Str("tool_result".to_string()));
        blk.insert("tool_use_id".to_string(), Value::Str("toolu_3".to_string()));
        blk.insert("content".to_string(), Value::Str("25 degrees".to_string()));
        let mut msg = BTreeMap::new();
        msg.insert("role".to_string(), Value::Str("user".to_string()));
        msg.insert("content".to_string(), Value::List(vec![Value::Map(blk)]));
        let out = match anthropic_msg_to_openai(&Value::Map(msg)) {
            Value::Map(m) => m,
            _ => panic!(),
        };
        assert_eq!(as_str(out.get("role").unwrap()).as_deref(), Some("tool"));
        assert_eq!(
            as_str(out.get("tool_call_id").unwrap()).as_deref(),
            Some("toolu_3")
        );
    }
}
