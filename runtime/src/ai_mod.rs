// Flux ai battery — LLM birlamchi primitiv (Anthropic Claude + OpenAI GPT).
//
// Til API (docs/flux-agent.md):
//   txt = ai.ask "savol ${x}"                         # -> matn (str)
//   r   = ai.json "extract: ${t}" {intent::a items:[...]}   # -> map + r._ metadata
//   r   = ai.run msgs tools                           # tool-loop'ning BIR qadami
//
// Metadata (`ai.json` natijasidagi `_` maydoni):
//   r._.conf   (0..1)   — modelning ishonchi (stop/finish_reason'dan baholangan)
//   r._.tokens (int)    — input+output token yig'indisi
//   r._.cost   (flt)    — taxminiy narx (USD), modelning narx jadvalidan
//   r._.ms     (int)    — so'rov davomiyligi millisekundda
//
// Falsafa: "til AI'ga moslashadi". `ai.run` AYNAN bitta qadam qaytaradi (tool'ni
// O'ZI bajarmaydi) — loop foydalanuvchi qo'lida bo'lsin (log/narx/tasdiq nazorati).
// Tool'ni `reg.call` orqali Flux tomonda chaqirasiz, natijani msgs'ga qo'shasiz.
//
// PROVAYDER AUTO-DETECT (Flux foydalanuvchisi hech narsa sozlamaydi):
//   - `.env`/muhitda ANTHROPIC_API_KEY bo'lsa -> Claude (default claude-opus-4-8)
//   - OPENAI_API_KEY bo'lsa -> GPT (default gpt-4o)
//   - Ikkalasi bo'lsa Anthropic ustun. Override: $AI_PROVIDER (anthropic|openai).
//   - $AI_KEY — provayderdan qat'i nazar umumiy override kalit.
//   - Model: $AI_MODEL ?? provayder default.
// Ichki shakl Anthropic Messages shaklida quriladi; OpenAI uchun chaqiruvdan
// oldin Chat Completions shakliga avtomatik aylantiriladi (msgs/tools/javob).
//
// Rasmiy Rust SDK yo'q -> raw https POST (`http` battery klient/pool'ini qayta
// ishlatadi). Holatsiz battery: env o'qiydi + POST yuboradi. Lekin kalitni
// `env_lookup` orqali olish uchun Interp'ga muhtoj -> `ai_dispatch` `&self` metodi.

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

// Javob uzunligi cheki. Token-hisoblash xavfsizligi uchun yetarli, lekin
// cheksiz emas. Foydalanuvchi `ai.ask`/`ai.json` semantikasini sodda saqlash
// uchun hozircha sozlanmaydigan (kelajakda opts orqali ochilishi mumkin).
const MAX_TOKENS: i64 = 4096;

// LLM so'rovi standart timeout'i (issue #92). Timeout'siz qotgan LLM endpoint
// butun skriptni (yoki HTTP handler ichida chaqirilsa, o'sha request thread'ini)
// ABADIY bloklaydi. LLM javoblari sekin bo'lishi mumkin, shuning uchun klientdan
// (30s) kattaroq: default 120s. `$AI_TIMEOUT` (soniya) bilan sozlanadi; 0 yoki
// manfiy — timeout'siz.
const DEFAULT_AI_TIMEOUT_SECS: u64 = 120;

// Qo'llab-quvvatlanadigan LLM provayderlari. Battery O'ZI aniqlaydi (auto) —
// Flux foydalanuvchisi hech narsa sozlamaydi: `.env`da standart provayder kaliti
// (ANTHROPIC_API_KEY / OPENAI_API_KEY) bo'lsa kifoya.
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

// Aniqlangan provayder + kalit + model (auto-detect natijasi).
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
            _ => Err(Flow::err(format!("ai.{} yo'q (ask/json/run)", func))),
        }
    }

    // env'dan qiymat oladi (OS env > .env), bo'sh bo'lmasa Some.
    fn ai_env(&self, name: &str) -> Option<String> {
        match self.env_lookup(name) {
            Value::Str(s) if !s.is_empty() => Some(s),
            _ => None,
        }
    }

    // LLM so'rovi timeout'i: $AI_TIMEOUT (soniya) ?? default 120s. Issue #92:
    // qotgan endpoint thread'ni abadiy bloklamasin.
    fn ai_timeout(&self) -> Option<Duration> {
        resolve_ai_timeout(self.ai_env("AI_TIMEOUT").as_deref())
    }

    // Provayder + kalit + modelni AVTOMATIK aniqlaydi. Hech narsa majburiy emas —
    // tartib:
    //   1) $AI_PROVIDER (anthropic|openai) override bo'lsa, o'sha provayder kaliti.
    //   2) Aks holda mavjud standart kalitdan provayder aniqlanadi:
    //        ANTHROPIC_API_KEY -> Anthropic,  OPENAI_API_KEY -> OpenAI.
    //   3) $AI_KEY — provayderdan qat'i nazar umumiy override kalit.
    // Model: $AI_MODEL ?? provayder default.
    fn ai_config(&self) -> Result<AiConfig, Flow> {
        let anthropic = self.ai_env("ANTHROPIC_API_KEY");
        let openai = self.ai_env("OPENAI_API_KEY");
        let generic = self.ai_env("AI_KEY");
        let forced = self.ai_env("AI_PROVIDER").map(|p| p.to_lowercase());

        // Provayderni aniqlaymiz.
        let provider = match forced.as_deref() {
            Some("anthropic") | Some("claude") => Provider::Anthropic,
            Some("openai") | Some("gpt") => Provider::OpenAI,
            Some(other) => {
                return Err(Flow::err(format!(
                    "ai: noma'lum $AI_PROVIDER '{}' (anthropic|openai)",
                    other
                )));
            }
            // Override yo'q -> mavjud standart kalitdan aniqlaymiz. Anthropic ustun
            // (loyiha Claude'ga yo'naltirilgan), keyin OpenAI.
            None => {
                if anthropic.is_some() {
                    Provider::Anthropic
                } else if openai.is_some() {
                    Provider::OpenAI
                } else if generic.is_some() {
                    // Faqat $AI_KEY bo'lsa va provayder ko'rsatilmagan bo'lsa,
                    // Anthropic deb faraz qilamiz (loyiha default'i).
                    Provider::Anthropic
                } else {
                    return Err(Flow::err(
                        "ai: API kaliti topilmadi — .env yoki muhitda \
                         ANTHROPIC_API_KEY yoki OPENAI_API_KEY belgilang"
                            .to_string(),
                    ));
                }
            }
        };

        // Kalitni provayderga mos tanlaymiz. $AI_KEY har doim eng ustun override.
        let provider_key = match provider {
            Provider::Anthropic => anthropic,
            Provider::OpenAI => openai,
        };
        let key = generic.or(provider_key).ok_or_else(|| {
            Flow::err(format!(
                "ai: {} kaliti topilmadi (${} yoki $AI_KEY belgilang)",
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

    // ai.ask "savol" -> javob matni (str).
    // Bitta user xabar yuboradi, birinchi text blokini qaytaradi.
    fn ai_ask(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let prompt = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => return Err(Flow::err("ai.ask: savol (str) kerak".to_string())),
        };
        let messages = vec![user_msg(&prompt)];
        let resp = self.call_api(&messages, None, None)?;
        Ok(Value::Str(resp.text))
    }

    // ai.json "prompt" {schema} -> map (+ `_` metadata).
    // Schema map'ini prompt'ga qo'shib, modeldan FAQAT JSON so'raymiz; javobni
    // map'ga parse qilamiz va `_` (metadata) maydonini qo'shamiz.
    fn ai_json(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let prompt = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => return Err(Flow::err("ai.json: prompt (str) kerak".to_string())),
        };
        let schema = match args.get(1) {
            Some(v @ (Value::Map(_) | Value::List(_))) => v.clone(),
            _ => return Err(Flow::err("ai.json: schema (map/list) kerak".to_string())),
        };

        // Modelga aniq ko'rsatma: berilgan shaklga MOS faqat JSON qaytar.
        // System prompt sof JSON majburlaydi (prefill 4.6+ da 400 beradi).
        let system = format!(
            "Javobni QAT'IY shu JSON shakliga mos qaytar. Faqat JSON, izoh/matn YO'Q.\nShakl: {}",
            json_encode(&schema)
        );
        let messages = vec![user_msg(&prompt)];
        let resp = self.call_api(&messages, Some(&system), None)?;

        // Javob matnini map'ga parse qilamiz. Model ba'zan ```json ... ``` o'rab
        // qaytaradi — kod blokini tozalaymiz.
        let cleaned = strip_code_fence(&resp.text);
        let mut parsed = match json_decode(&cleaned) {
            Ok(Value::Map(m)) => m,
            Ok(other) => {
                // JSON bo'lsa-yu map bo'lmasa (masalan list) — `value` ostiga joylaymiz.
                let mut m = BTreeMap::new();
                m.insert("value".to_string(), other);
                m
            }
            Err(_) => {
                return Err(Flow::err(format!(
                    "ai.json: model JSON qaytarmadi: {}",
                    truncate(&resp.text, 200)
                )));
            }
        };
        parsed.insert("_".to_string(), Value::Map(resp.meta()));
        Ok(Value::Map(parsed))
    }

    // ai.run msgs tools -> tool-loop'ning BIR qadami.
    //   msgs:  [{role::user content:str} ...]  (role sym yoki str)
    //   tools: [{name desc params} ...]        (params — JSON-schema map)
    // Natija (kind nomi spec'dan — docs/flux-human.md):
    //   :final -> {kind::final text:str}
    //   :call  -> {kind::call tool:str args:map id:str calls:[{tool args id} ...]}
    // Model parallel bir nechta tool chaqirsa, hammasi `calls` ro'yxatida bo'ladi
    // (har biriga tool_result qaytarish kerak). `tool`/`args`/`id` esa orqaga
    // moslik uchun birinchi chaqiruv — `calls`'ning [0] elementi bilan bir xil.
    // (tool'ni O'ZI bajarmaydi — loop foydalanuvchi qo'lida.)
    fn ai_run(&self, args: Vec<Value>) -> Result<Value, Flow> {
        let msgs = match args.first() {
            Some(Value::List(l)) => l.clone(),
            _ => return Err(Flow::err("ai.run: msgs (list) kerak".to_string())),
        };
        let tools = match args.get(1) {
            Some(Value::List(l)) => Some(l.clone()),
            None | Some(Value::Nil) => None,
            _ => return Err(Flow::err("ai.run: tools (list) bo'lishi kerak".to_string())),
        };

        // msgs Flux shaklidan Anthropic shakliga: {role content} -> {role, content}.
        // role sym (:user) yoki str ("user") bo'lishi mumkin. tool natijasi
        // xabari ({role::tool name content}) ham o'tkaziladi.
        let api_msgs: Vec<Value> = msgs.iter().map(normalize_msg).collect();

        let api_tools = tools
            .as_ref()
            .map(|t| t.iter().map(normalize_tool).collect());
        let resp = self.call_api(&api_msgs, None, api_tools.as_ref())?;

        let mut out = BTreeMap::new();
        if !resp.tool_calls.is_empty() {
            out.insert("kind".to_string(), Value::Sym("call".to_string()));
            // Har chaqiruvni {tool args id} map'ga aylantiramiz.
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
            // Orqaga moslik: birinchi chaqiruv top-level `tool`/`args`/`id`.
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

    // Provayderga mos POST so'rov. messages — Flux normalize qilingan list;
    // system/tools opsional. Provayder auto-detect qilinadi, request/response
    // formati ham provayderga qarab tanlanadi.
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

    // Anthropic /v1/messages: system top-level, x-api-key header, tools shakli
    // {name description input_schema}, content bloklar massivi.
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

    // OpenAI /v1/chat/completions: system — messages ichida {role:system},
    // Authorization: Bearer, tools shakli {type:function function:{...}},
    // javob choices[0].message.{content|tool_calls}.
    fn call_openai(
        &self,
        cfg: &AiConfig,
        messages: &[Value],
        system: Option<&str>,
        tools: Option<&Vec<Value>>,
    ) -> Result<AiResp, Flow> {
        // OpenAI'da system alohida emas — messages boshiga {role:system} qo'shamiz.
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
            // Anthropic tool shaklini ({name description input_schema}) OpenAI
            // function shakliga aylantiramiz.
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

// Umumiy https POST (content-type: json). `add_headers` provayderga xos
// autentifikatsiya/versiya header'larini qo'shadi. Javob matni + davomiyligini
// (ms) qaytaradi; non-2xx -> aniq xato.
//
// Vaqtinchalik xatolarda (429 rate-limit / 529 overloaded) BIR marta qayta
// urinish (issue #92 bonus): LLM API'lari qisqa yuk cho'qqilarida shu
// statuslarni qaytaradi — bitta backoff'li retry barqarorlikni sezilarli
// oshiradi, cheksiz loop xavfisiz. Timeout har urinishga alohida qo'llanadi
// (eng yomon holat: 2 urinish + backoff). Shu sababli `add_headers` Fn
// (FnOnce emas) — retry'da so'rov qaytadan quriladi.
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
            // Bitta urinish (yuborish + javob o'qish) — timeout shu blokni qamraydi.
            let work = async {
                let builder = Request::builder()
                    .method("POST")
                    .uri(url.clone())
                    .header("content-type", "application/json");
                let builder = add_headers(builder);
                let req = builder
                    .body(Full::new(Bytes::from(body.clone())))
                    .map_err(|e| Flow::err(format!("ai: so'rov qurish: {}", e)))?;

                let resp = pooled_http_client()
                    .request(req)
                    .await
                    .map_err(|e| Flow::err(format!("ai: tarmoq xatosi: {}", e)))?;
                let status = resp.status().as_u16();
                // Retry-After'ni tana o'qilishidan OLDIN olamiz (into_body resp'ni
                // yutadi) — server bergan kutish vaqti backoff'da ishlatiladi.
                let retry_after = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.trim().parse::<u64>().ok());
                let bytes = resp
                    .into_body()
                    .collect()
                    .await
                    .map_err(|e| Flow::err(format!("ai: javob o'qish: {}", e)))?
                    .to_bytes();
                let text = String::from_utf8_lossy(&bytes).to_string();
                Ok::<_, Flow>((status, retry_after, text))
            };

            // Timeout o'rnatilgan bo'lsa urinishni unga o'raymiz; tugamasa aniq
            // xato (qotgan LLM endpoint thread'ni abadiy bloklamasin — issue #92).
            let (status, retry_after, text) = match timeout {
                Some(dur) => match tokio::time::timeout(dur, work).await {
                    Ok(r) => r?,
                    Err(_) => {
                        return Err(Flow::err(format!(
                            "ai: so'rov timeout ({} sek ichida javob yo'q)",
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
                "ai: API xatosi (status {}): {}",
                status,
                truncate(&text, 300)
            )));
        }
    })?;
    let ms = started.elapsed().as_millis() as i64;
    Ok((text, ms))
}

// 429 (rate limit) va 529 (Anthropic overloaded) — vaqtinchalik holatlar: bir
// marta qayta urinish o'rinli (issue #92 bonus). Boshqa 4xx/5xx (401 noto'g'ri
// kalit, 400 yaroqsiz so'rov...) qayta urinishdan tuzalmaydi — darhol xato.
fn should_retry_status(status: u16) -> bool {
    matches!(status, 429 | 529)
}

// Retry oldidan kutish: server `Retry-After` (soniya) bergan bo'lsa unga amal
// qilamiz — lekin 1..=30 ga qisqartirilgan (handler thread'ini juda uzoq
// ushlamaslik uchun); header bo'lmasa default 2s.
fn retry_backoff(retry_after: Option<u64>) -> Duration {
    Duration::from_secs(retry_after.map(|s| s.clamp(1, 30)).unwrap_or(2))
}

// --- Javob parse ---

// Anthropic javobidan kerakli qismlar.
struct AiResp {
    text: String,
    // (tool_name, input_map, tool_use_id) — model tool chaqirmoqchi bo'lsa.
    // Vec: model bitta javobda BIR NECHTA tool'ni parallel chaqirishi mumkin —
    // hammasini yig'amiz, aks holda yo'qolgan tool_use_id keyingi so'rovda 400 beradi.
    tool_calls: Vec<(String, Value, String)>,
    in_tokens: i64,
    out_tokens: i64,
    model: String,
    ms: i64,
    // stop_reason'dan baholangan ishonch (end_turn -> yuqori, max_tokens -> past).
    conf: f64,
}

impl AiResp {
    // `r._` metadata map'i: conf/tokens/cost/ms.
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

// Anthropic javob matnini AiResp'ga parse qiladi. content massivdan text va
// tool_use bloklarini ajratadi; usage'dan token'larni oladi.
fn parse_anthropic(text: &str, model: &str, ms: i64) -> Result<AiResp, Flow> {
    let map = decode_obj(text)?;

    // stop_reason -> ishonch bahosi (heuristik).
    let stop = map.get("stop_reason").and_then(as_str).unwrap_or_default();
    let conf = match stop.as_str() {
        "end_turn" | "tool_use" | "stop_sequence" => 0.9,
        "max_tokens" => 0.5, // kesilgan -> ishonch past
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
    // Model parallel ravishda bir nechta tool_use blok qaytarishi mumkin —
    // hammasini yig'amiz (faqat oxirgisini saqlasak qolganlari uchun keyingi
    // so'rovda tool_result bo'lmaydi va Anthropic API 400 qaytaradi).
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

// OpenAI Chat Completions javobini AiResp'ga parse qiladi:
//   choices[0].message.{content, tool_calls[0].function.{name, arguments}}
//   choices[0].finish_reason, usage.{prompt_tokens, completion_tokens}
// tool_calls[].function.arguments — JSON-kodlangan STRING (Anthropic'da map).
fn parse_openai(text: &str, model: &str, ms: i64) -> Result<AiResp, Flow> {
    let map = decode_obj(text)?;

    let choice = match map.get("choices") {
        Some(Value::List(cs)) if !cs.is_empty() => match &cs[0] {
            Value::Map(c) => c.clone(),
            _ => {
                return Err(Flow::err(
                    "ai: OpenAI choices[0] shakli noto'g'ri".to_string(),
                ));
            }
        },
        _ => {
            return Err(Flow::err(format!(
                "ai: OpenAI javobida choices yo'q: {}",
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
        "length" => 0.5, // token cheki -> kesilgan
        "content_filter" => 0.0,
        _ => 0.7,
    };

    // usage: OpenAI prompt_tokens/completion_tokens deydi.
    let (in_tokens, out_tokens) = match map.get("usage") {
        Some(Value::Map(u)) => (
            u.get("prompt_tokens").and_then(as_int).unwrap_or(0),
            u.get("completion_tokens").and_then(as_int).unwrap_or(0),
        ),
        _ => (0, 0),
    };

    let message = match choice.get("message") {
        Some(Value::Map(m)) => m.clone(),
        _ => return Err(Flow::err("ai: OpenAI message yo'q".to_string())),
    };

    // content (null bo'lishi mumkin tool_calls bo'lganda).
    let out_text = message.get("content").and_then(as_str).unwrap_or_default();

    // tool_calls[] -> [(name, args_map, id)]. arguments JSON-string -> map.
    // Model bir javobda bir nechta tool chaqirishi mumkin — hammasini yig'amiz
    // (faqat tc[0] ni olsak qolganlari uchun tool natijasi qaytmaydi va keyingi
    // so'rov 400 oladi).
    let mut tool_calls = Vec::new();
    if let Some(Value::List(tc)) = message.get("tool_calls") {
        for call in tc {
            let Value::Map(call) = call else { continue };
            let id = call.get("id").and_then(as_str).unwrap_or_default();
            let func = match call.get("function") {
                Some(Value::Map(f)) => f,
                _ => return Err(Flow::err("ai: OpenAI tool_call.function yo'q".to_string())),
            };
            let name = func.get("name").and_then(as_str).unwrap_or_default();
            // arguments — JSON-kodlangan string; map'ga parse qilamiz.
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

// JSON matnni map'ga dekod qiladi (ikki parser uchun umumiy).
fn decode_obj(text: &str) -> Result<BTreeMap<String, Value>, Flow> {
    let v = json_decode(text).map_err(|_| {
        Flow::err(format!(
            "ai: javobni parse qilib bo'lmadi: {}",
            truncate(text, 200)
        ))
    })?;
    match v {
        Value::Map(m) => Ok(m),
        _ => Err(Flow::err("ai: kutilmagan javob shakli".to_string())),
    }
}

// --- Yordamchilar ---

// {role::user content:"..."} — bitta user xabar.
fn user_msg(content: &str) -> Value {
    let mut m = BTreeMap::new();
    m.insert("role".to_string(), Value::Str("user".to_string()));
    m.insert("content".to_string(), Value::Str(content.to_string()));
    Value::Map(m)
}

// Flux xabarini Anthropic shakliga keltiradi. role sym (:user) yoki str bo'lishi
// mumkin -> har doim str. tool natijasi xabari ({role::tool name content}) esa
// Anthropic'da user roli + tool_result blok bo'lib ketadi.
fn normalize_msg(msg: &Value) -> Value {
    let m = match msg {
        Value::Map(m) => m,
        other => return other.clone(),
    };
    // role sym (:user) yoki str ("user") bo'lishi mumkin — ikkalasini ham oqaymiz.
    let role = m
        .get("role")
        .and_then(sym_or_str)
        .unwrap_or_else(|| "user".to_string());

    // tool natijasi: {role::tool name content} -> {role:"user" content:[{type:tool_result ...}]}
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

    // Oddiy xabar: role'ni str'ga, content'ni o'z holicha (str yoki blok list).
    let mut out = BTreeMap::new();
    out.insert("role".to_string(), Value::Str(role));
    if let Some(c) = m.get("content") {
        out.insert("content".to_string(), c.clone());
    } else {
        out.insert("content".to_string(), Value::Str(String::new()));
    }
    Value::Map(out)
}

// Flux tool ta'rifini ({name desc params}) Anthropic shakliga
// ({name description input_schema}) keltiradi.
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
    // params — JSON-schema (object). Foydalanuvchi {a:str b:int} kabi sodda berishi
    // mumkin; biz uni JSON-schema object'iga o'raymiz, agar allaqachon type:object
    // bo'lmasa.
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

// params map'ini to'liq JSON-schema object'iga aylantiradi. Agar map allaqachon
// {type:"object" ...} bo'lsa, o'z holicha qoldiramiz. Aks holda har maydonni
// {type:"..."} ga aylantirib `properties` ostiga joylaymiz.
fn wrap_schema(schema: Value) -> Value {
    let fields = match schema {
        Value::Map(m) => m,
        // map bo'lmasa — bo'sh object schema.
        _ => return empty_object_schema(),
    };
    // Allaqachon to'liq schema bo'lsa (type:object), tegmaymiz.
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

// Bitta maydon qiymatini JSON-schema bo'lagiga aylantiradi. Tip nomi (sym/str)
// -> {type:"..."}; ro'yxat (`[T]`) -> {type:"array", items:<T schema>}; map ->
// rekursiv object schema (allaqachon to'liq schema bo'lsa o'z holicha).
fn field_schema(v: &Value) -> Value {
    match v {
        // {a:str} — qiymat tip nomi (sym yoki str).
        Value::Sym(s) | Value::Str(s) => {
            let mut field = BTreeMap::new();
            field.insert("type".to_string(), Value::Str(json_type(s)));
            Value::Map(field)
        }
        // [T] — array; element tipi rekursiv. `[]` (bo'sh) -> items'siz array.
        Value::List(items) => {
            let mut field = BTreeMap::new();
            field.insert("type".to_string(), Value::Str("array".to_string()));
            if let Some(first) = items.first() {
                field.insert("items".to_string(), field_schema(first));
            }
            Value::Map(field)
        }
        // {a:{...}} — ichki object; agar allaqachon to'liq schema bo'lmasa,
        // rekursiv wrap_schema bilan object'ga aylantiramiz.
        Value::Map(m) => {
            if m.get("type").and_then(as_str).is_some() {
                // foydalanuvchi allaqachon {type:"..."} bergan — tegmaymiz.
                v.clone()
            } else {
                wrap_schema(v.clone())
            }
        }
        // boshqa qiymatlar uchun zaxira sifatida string.
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

// Flux tip nomini JSON-schema tipiga: str->string, int->integer, flt->number,
// bool->boolean. Boshqasi o'z holicha (list/object foydalanuvchi bersa).
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

// tool_result content'ini matnga keltiradi: map/list -> JSON, str -> o'zi.
fn content_to_str(v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(),
        Value::Map(_) | Value::List(_) => json_encode(v),
        other => format!("{}", other),
    }
}

// Model ```json ... ``` yoki ``` ... ``` bilan o'rab qaytarsa, blokni tozalaydi.
fn strip_code_fence(s: &str) -> String {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```") {
        // birinchi qatorni (```json) tashlab, oxirgi ``` gacha olamiz.
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

// Taxminiy narx (USD) modelning $/1M token jadvalidan. Noma'lum model -> 0.
// Anthropic: Opus 4.x / Sonnet 4.6 / Haiku 4.5. OpenAI: gpt-4o / gpt-4o-mini.
// ($input, $output per 1M token).
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

// --- Provayder yordamchilari ---

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

// Anthropic shaklidagi xabarni OpenAI shakliga aylantiradi.
//   {role:"user" content:"..."}                       -> o'zgarmaydi
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

    // content blok list bo'lsa — tool_result yoki tool_use ni aylantiramiz.
    if let Some(Value::List(blocks)) = m.get("content") {
        // tool_result (Anthropic user roli) -> OpenAI {role:"tool" tool_call_id content}
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
        // tool_use (Anthropic assistant roli) -> OpenAI tool_calls.
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
            // OpenAI arguments — JSON-kodlangan STRING.
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

    // Oddiy xabar: role + content (str) o'z holicha.
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

// Anthropic tool shaklini ({name description input_schema}) OpenAI function
// shakliga ({type:function function:{name description parameters}}) aylantiradi.
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

// Value'dan str/int o'qish yordamchilari (json_decode natijasi uchun).
fn as_str(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.clone()),
        _ => None,
    }
}

// Sym yoki Str — ikkalasidan ham matn (role :user va "user" bir xil ko'rinsin).
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

// $AI_TIMEOUT (soniya str) -> Duration. Berilmagan/yaroqsiz -> default 120s; 0
// yoki manfiy -> None (timeout'siz). Sof funksiya — env'siz test qilinadi (#92).
fn resolve_ai_timeout(env: Option<&str>) -> Option<Duration> {
    match env.and_then(|s| s.trim().parse::<i64>().ok()) {
        Some(n) if n > 0 => Some(Duration::from_secs(n as u64)),
        Some(_) => None, // 0 yoki manfiy — timeout'siz
        None => Some(Duration::from_secs(DEFAULT_AI_TIMEOUT_SECS)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_timeout_resolve() {
        // issue #92: default 120s, $AI_TIMEOUT bilan sozlanadi, 0/manfiy — timeout'siz.
        assert_eq!(resolve_ai_timeout(None), Some(Duration::from_secs(120)));
        assert_eq!(
            resolve_ai_timeout(Some("30")),
            Some(Duration::from_secs(30))
        );
        assert_eq!(resolve_ai_timeout(Some("0")), None);
        assert_eq!(resolve_ai_timeout(Some("-5")), None);
        // Yaroqsiz qiymat — default'ga qaytadi (parse muvaffaqiyatsiz).
        assert_eq!(
            resolve_ai_timeout(Some("abc")),
            Some(Duration::from_secs(120))
        );
    }

    #[test]
    fn retry_status_faqat_vaqtinchalik() {
        // 429/529 — retry; autentifikatsiya/validatsiya xatolari — yo'q.
        assert!(should_retry_status(429));
        assert!(should_retry_status(529));
        assert!(!should_retry_status(400));
        assert!(!should_retry_status(401));
        assert!(!should_retry_status(500));
        assert!(!should_retry_status(503));
    }

    #[test]
    fn retry_backoff_retry_after_va_default() {
        // Retry-After bo'lsa unga amal (1..=30 clamp), bo'lmasa default 2s.
        assert_eq!(retry_backoff(Some(5)), Duration::from_secs(5));
        assert_eq!(retry_backoff(Some(0)), Duration::from_secs(1));
        assert_eq!(retry_backoff(Some(300)), Duration::from_secs(30));
        assert_eq!(retry_backoff(None), Duration::from_secs(2));
    }

    // Lokal test serveri: berilgan javoblarni navbat bilan qaytaradi (har ulanish
    // bitta javob, connection: close). Nechta so'rov kelganini qaytaradi.
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
                // So'rovni to'liq o'qiymiz (header'lar + content-length tana) —
                // aks holda javobdan keyin yopilgan socket RST berishi mumkin.
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
        // issue #92 bonus: birinchi javob 429 bo'lsa BIR marta qayta uriniladi —
        // ikkinchi urinish 200 qaytarsa natija muvaffaqiyatli.
        let r429 = "HTTP/1.1 429 Too Many Requests\r\nretry-after: 1\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_string();
        let r200 = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 8\r\nconnection: close\r\n\r\n{\"ok\":1}".to_string();
        let (addr, handle) = serve_responses(vec![r429, r200]);
        let url = format!("http://{}/v1/x", addr);
        let res = post_json(&url, "{}".to_string(), Some(Duration::from_secs(10)), |b| b);
        match res {
            Ok((text, _ms)) => assert_eq!(text, "{\"ok\":1}"),
            Err(Flow::Error(e)) => panic!("retry'dan keyin Ok kutilgan: {}", e),
            Err(_) => panic!("kutilmagan Flow"),
        }
        assert_eq!(
            handle.join().unwrap(),
            2,
            "ikki so'rov (asl + retry) kelishi kerak"
        );
    }

    #[test]
    fn post_json_ikkinchi_429_xato_qaytaradi() {
        // Faqat BIR marta retry — ikkinchi urinish ham 429 bo'lsa aniq xato
        // qaytadi (cheksiz retry loop yo'q).
        let r429 = "HTTP/1.1 429 Too Many Requests\r\nretry-after: 1\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_string();
        let (addr, handle) = serve_responses(vec![r429.clone(), r429]);
        let url = format!("http://{}/v1/x", addr);
        let res = post_json(&url, "{}".to_string(), Some(Duration::from_secs(10)), |b| b);
        match res {
            Err(Flow::Error(e)) => assert!(e.contains("429"), "429 xatosi kutilgan: {}", e),
            Ok(_) => panic!("ikkinchi 429 dan keyin xato kutilgan"),
            Err(_) => panic!("Flow::Error kutilgan"),
        }
        assert_eq!(
            handle.join().unwrap(),
            2,
            "aniq ikki urinish bo'lishi kerak"
        );
    }

    #[test]
    fn post_json_401_retry_qilmaydi() {
        // Doimiy xato (401 noto'g'ri kalit) — retry YO'Q, bitta so'rov bilan xato.
        let r401 = "HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            .to_string();
        let (addr, handle) = serve_responses(vec![r401]);
        let url = format!("http://{}/v1/x", addr);
        let res = post_json(&url, "{}".to_string(), Some(Duration::from_secs(10)), |b| b);
        match res {
            Err(Flow::Error(e)) => assert!(e.contains("401"), "401 xatosi kutilgan: {}", e),
            Ok(_) => panic!("401 dan keyin xato kutilgan"),
            Err(_) => panic!("Flow::Error kutilgan"),
        }
        assert_eq!(
            handle.join().unwrap(),
            1,
            "faqat bitta so'rov bo'lishi kerak"
        );
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
        // {name:str age:int} -> JSON-schema object.
        let mut s = BTreeMap::new();
        s.insert("name".to_string(), Value::Sym("str".to_string()));
        s.insert("age".to_string(), Value::Sym("int".to_string()));
        let wrapped = wrap_schema(Value::Map(s));
        let m = match wrapped {
            Value::Map(m) => m,
            _ => panic!("map kutilgan"),
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
        // {tags:[str] items:[{name:str}]} -> array tiplari to'g'ri quriladi.
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
            _ => panic!("map kutilgan"),
        };
        // tags -> {type:array, items:{type:string}}
        let tags = match props.get("tags").unwrap() {
            Value::Map(f) => f.clone(),
            _ => panic!(),
        };
        assert_eq!(as_str(tags.get("type").unwrap()).as_deref(), Some("array"));
        let tags_items = match tags.get("items").unwrap() {
            Value::Map(i) => i.clone(),
            _ => panic!("tags items map bo'lishi kerak"),
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
            _ => panic!("items items map bo'lishi kerak"),
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
        // Allaqachon type:object bo'lsa, tegilmaydi.
        // (Value Debug/PartialEq derive qilmaydi -> .equals bilan solishtiramiz.)
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
        t.insert("desc".to_string(), Value::Str("ob-havo".to_string()));
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
        // {role::tool id content} -> user + tool_result blok.
        let mut t = BTreeMap::new();
        t.insert("role".to_string(), Value::Sym("tool".to_string()));
        t.insert("id".to_string(), Value::Str("toolu_1".to_string()));
        t.insert("content".to_string(), Value::Str("25 daraja".to_string()));
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
            _ => panic!("tool_result list kutilgan"),
        }
    }

    #[test]
    fn normalize_sym_role() {
        // {role::user content} -> role str'ga aylanadi.
        let mut t = BTreeMap::new();
        t.insert("role".to_string(), Value::Sym("user".to_string()));
        t.insert("content".to_string(), Value::Str("salom".to_string()));
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
            "content": [{"type": "text", "text": "javob"}]
        }"#;
        // Flow Debug derive qilmaydi -> .unwrap() o'rniga match.
        let r = match parse_anthropic(json, "claude-opus-4-8", 100) {
            Ok(r) => r,
            Err(_) => panic!("parse muvaffaqiyatsiz"),
        };
        assert_eq!(r.text, "javob");
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
                {"type": "text", "text": "tekshiraman"},
                {"type": "tool_use", "id": "toolu_9", "name": "weather", "input": {"city": "Toshkent"}}
            ]
        }"#;
        let r = match parse_anthropic(json, "claude-opus-4-8", 50) {
            Ok(r) => r,
            Err(_) => panic!("parse muvaffaqiyatsiz"),
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
        // Model bir javobda IKKI tool chaqirsa, ikkalasi ham yig'iladi
        // (issue #95 — ilgari faqat oxirgisi qolardi).
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
            Err(_) => panic!("parse muvaffaqiyatsiz"),
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
        // narx: (1000*5 + 500*25)/1e6 = (5000+12500)/1e6 = 0.0175
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
        assert_eq!(estimate_cost("noma'lum", 1_000_000, 0), 0.0);
    }

    #[test]
    fn parse_openai_text() {
        // OpenAI Chat Completions text javobi.
        let json = r#"{
            "choices": [{
                "finish_reason": "stop",
                "message": {"role": "assistant", "content": "salom"}
            }],
            "usage": {"prompt_tokens": 12, "completion_tokens": 4}
        }"#;
        let r = match parse_openai(json, "gpt-4o", 80) {
            Ok(r) => r,
            Err(_) => panic!("openai parse muvaffaqiyatsiz"),
        };
        assert_eq!(r.text, "salom");
        assert!(r.tool_calls.is_empty());
        assert_eq!(r.in_tokens, 12);
        assert_eq!(r.out_tokens, 4);
    }

    #[test]
    fn parse_openai_tool_call() {
        // OpenAI tool_calls: arguments JSON-STRING -> map'ga parse bo'ladi.
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
            Err(_) => panic!("openai tool parse muvaffaqiyatsiz"),
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
            _ => panic!("args map kutilgan"),
        }
    }

    #[test]
    fn parse_openai_parallel_tool_calls() {
        // OpenAI ham bir javobda bir nechta tool_call qaytarishi mumkin —
        // hammasi yig'iladi (issue #95).
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
            Err(_) => panic!("openai parallel parse muvaffaqiyatsiz"),
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
        t.insert("description".to_string(), Value::Str("ob-havo".to_string()));
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
            _ => panic!("function map kutilgan"),
        }
    }

    #[test]
    fn anthropic_msg_to_openai_tool_result() {
        // Anthropic tool_result (user roli) -> OpenAI {role:tool tool_call_id content}.
        let mut blk = BTreeMap::new();
        blk.insert("type".to_string(), Value::Str("tool_result".to_string()));
        blk.insert("tool_use_id".to_string(), Value::Str("toolu_3".to_string()));
        blk.insert("content".to_string(), Value::Str("25 daraja".to_string()));
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
