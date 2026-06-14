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
#[derive(Clone, Copy, PartialEq)]
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
}

// Detected provider + key + model (the auto-detect result).
struct AiConfig {
    provider: Provider,
    key: String,
    model: String,
}

impl Interp {
    // ai.ask / ai.json / ai.run dispatch.
    pub fn ai_dispatch(&self, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            "ask" => self.ai_ask(args),
            "json" => self.ai_json(args),
            "run" => self.ai_run(args),
            _ => Err(Flow::err(format!("ai.{} not found (ask/json/run)", func))),
        }
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

    // AUTO-detects provider + key + model. Nothing is mandatory — the order:
    //   1) if $AI_PROVIDER (anthropic|openai) overrides, that provider's key.
    //   2) otherwise the provider is detected from the available standard key:
    //        ANTHROPIC_API_KEY -> Anthropic,  OPENAI_API_KEY -> OpenAI.
    //   3) $AI_KEY — a generic override key regardless of provider.
    // Model: $AI_MODEL ?? provider default.
    fn ai_config(&self) -> Result<AiConfig, Flow> {
        let anthropic = self.ai_env("ANTHROPIC_API_KEY");
        let openai = self.ai_env("OPENAI_API_KEY");
        let generic = self.ai_env("AI_KEY");
        let forced = self.ai_env("AI_PROVIDER").map(|p| p.to_lowercase());

        // Determine the provider.
        let provider = match forced.as_deref() {
            Some("anthropic") | Some("claude") => Provider::Anthropic,
            Some("openai") | Some("gpt") => Provider::OpenAI,
            Some(other) => {
                return Err(Flow::err(format!(
                    "ai: unknown $AI_PROVIDER '{}' (anthropic|openai)",
                    other
                )));
            }
            // No override -> detect from the available standard key. Anthropic
            // wins (the project is oriented toward Claude), then OpenAI.
            None => {
                if anthropic.is_some() {
                    Provider::Anthropic
                } else if openai.is_some() {
                    Provider::OpenAI
                } else if generic.is_some() {
                    // If only $AI_KEY is set and no provider is given, we assume
                    // Anthropic (the project default).
                    Provider::Anthropic
                } else {
                    return Err(Flow::err(
                        "ai: API key not found — set ANTHROPIC_API_KEY or \
                         OPENAI_API_KEY in .env or the environment"
                            .to_string(),
                    ));
                }
            }
        };

        // Pick the key matching the provider. $AI_KEY is always the top override.
        let provider_key = match provider {
            Provider::Anthropic => anthropic,
            Provider::OpenAI => openai,
        };
        let key = generic.or(provider_key).ok_or_else(|| {
            Flow::err(format!(
                "ai: {} key not found (set ${} or $AI_KEY)",
                provider_name(provider),
                provider_key_name(provider),
            ))
        })?;

        let model = self
            .ai_env("AI_MODEL")
            .unwrap_or_else(|| provider.default_model().to_string());

        Ok(AiConfig {
            provider,
            key,
            model,
        })
    }

    // ai.ask "savol" -> response text (str).
    // Sends a single user message, returns the first text block.
    fn ai_ask(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let prompt = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => return Err(Flow::err("ai.ask: question (str) required".to_string())),
        };
        let messages = vec![user_msg(&prompt)];
        let resp = self.call_api(&messages, None, None)?;
        Ok(Value::Str(resp.text))
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

        // Explicit instruction to the model: return only JSON MATCHING the given
        // shape. The system prompt forces pure JSON (prefill gives a 400 on 4.6+).
        let system = format!(
            "Return the response STRICTLY matching this JSON shape. JSON only, NO comments/text.\nShape: {}",
            json_encode(&schema)
        );
        let messages = vec![user_msg(&prompt)];
        let resp = self.call_api(&messages, Some(&system), None)?;

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

        // msgs from the Fluxon shape to the Anthropic shape: {role content} -> {role, content}.
        // role may be a sym (:user) or str ("user"). The tool-result message
        // ({role::tool name content}) is also converted.
        let api_msgs: Vec<Value> = msgs.iter().map(normalize_msg).collect();

        let api_tools = tools
            .as_ref()
            .map(|t| t.iter().map(normalize_tool).collect());
        let resp = self.call_api(&api_msgs, None, api_tools.as_ref())?;

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
    ) -> Result<AiResp, Flow> {
        let cfg = self.ai_config()?;
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
        let body_str = json_encode(&Value::Map(body));
        let key = cfg.key.clone();

        let (text, ms) = post_json(ANTHROPIC_URL, body_str, self.ai_timeout(), move |b| {
            b.header("x-api-key", key.as_str())
                .header("anthropic-version", ANTHROPIC_VERSION)
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
        let body_str = json_encode(&Value::Map(body));
        let key = cfg.key.clone();

        let (text, ms) = post_json(OPENAI_URL, body_str, self.ai_timeout(), move |b| {
            b.header("authorization", format!("Bearer {}", key))
        })?;
        parse_openai(&text, &cfg.model, ms)
    }
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
                let builder = Request::builder()
                    .method("POST")
                    .uri(url.clone())
                    .header("content-type", "application/json");
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
