// Rate-limiting (issue #79): a fixed-window in-memory counter shared across
// requests, plus the helpers that compute and format the 429 response.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::value::Value;

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
    counts: std::collections::HashMap<String, (u64, u32)>,
    // Number of operations since the last cleanup (amortizes the sweep).
    ops: u32,
}

impl LimitBucket {
    pub(crate) fn new() -> Self {
        LimitBucket {
            counts: std::collections::HashMap::new(),
            ops: 0,
        }
    }
}

// How often (in operations) we sweep old-window keys.
const SWEEP_EVERY: u32 = 1024;

pub type LimitState = Arc<Mutex<LimitBucket>>;

// Converts a window-unit symbol to seconds. Only :sec/:min/:hr — few tokens, a
// canonical set the AI remembers (add a new unit here if needed).
pub(crate) fn window_to_secs(unit: &str) -> Option<u64> {
    match unit {
        "sec" => Some(1),
        "min" => Some(60),
        "hr" => Some(3600),
        _ => None,
    }
}

// Fixed-window counter: counts the request for a key in the current window and
// checks after incrementing. If the limit is exceeded, Some(retry_after_secs)
// (until the window ends), otherwise None. The Mutex does read-modify-write under
// one lock — so parallel requests count a key atomically (no race).
pub(crate) fn check_and_count(
    state: &LimitState,
    key: &str,
    limit: u32,
    window_secs: u64,
) -> Option<u64> {
    // Wall-clock time (not Instant): the window boundary is tied to the epoch, so
    // Retry-After also comes out exactly as (window_id+1)*window_secs - now.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let window_id = now / window_secs;
    let mut bucket = state.lock().unwrap();
    // Periodic cleanup: remove keys from old windows (window_id smaller than the
    // current one) so user-controlled keys do not grow memory without bound. Only
    // once every SWEEP_EVERY operations — O(1) amortized.
    bucket.ops = bucket.ops.saturating_add(1);
    if bucket.ops >= SWEEP_EVERY {
        bucket.ops = 0;
        bucket.counts.retain(|_, (wid, _)| *wid >= window_id);
    }
    let entry = bucket
        .counts
        .entry(key.to_string())
        .or_insert((window_id, 0));
    // Moved to a new window — reset the count to zero.
    if entry.0 != window_id {
        *entry = (window_id, 0);
    }
    entry.1 = entry.1.saturating_add(1);
    if entry.1 > limit {
        // The window resets at epoch (window_id+1)*window_secs; now is smaller,
        // so the difference is always >= 1.
        Some((window_id + 1) * window_secs - now)
    } else {
        None
    }
}

// If the key function returns nil/empty — fall back to the client IP (so even a
// keyless request is limited). The "ip:" prefix avoids accidental collisions
// with a tenant_id/api-key value (in one limiter's state both live in the same
// HashMap).
pub(crate) fn client_fallback_key(req: &Value) -> String {
    let ip = match req {
        Value::Map(m) => match m.get("ip") {
            Some(Value::Str(s)) if !s.is_empty() => s.clone(),
            _ => "unknown".to_string(),
        },
        _ => "unknown".to_string(),
    };
    format!("ip:{}", ip)
}

// Response returned when the limit is exceeded: `429` + a `Retry-After` header
// (PRD format). As a __resp map — handle_request sends it like other rep responses.
pub(crate) fn rate_limited_response(retry_after: u64) -> Value {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http_mod::request::{build_req, with_ctx};
    use bytes::Bytes;

    #[test]
    fn window_birligi_soniyaga_aylanadi() {
        // Canonical set: :sec/:min/:hr. An unknown unit is None.
        assert_eq!(window_to_secs("sec"), Some(1));
        assert_eq!(window_to_secs("min"), Some(60));
        assert_eq!(window_to_secs("hr"), Some(3600));
        assert_eq!(window_to_secs("day"), None);
    }

    #[test]
    fn limit_oyna_ichida_sanaydi_va_429_beradi() {
        // limit=3 — the first 3 requests pass (None), the 4th is blocked (Some).
        let state: LimitState = Arc::new(Mutex::new(LimitBucket::new()));
        assert!(check_and_count(&state, "t1", 3, 3600).is_none());
        assert!(check_and_count(&state, "t1", 3, 3600).is_none());
        assert!(check_and_count(&state, "t1", 3, 3600).is_none());
        let retry = check_and_count(&state, "t1", 3, 3600);
        assert!(retry.is_some(), "the 4th request must be blocked");
        // Retry-After is until the window ends — in the range [1, window_secs].
        let r = retry.unwrap();
        assert!((1..=3600).contains(&r), "Retry-After is sensible: {}", r);
    }

    #[test]
    fn limit_kalitlar_alohida_sanaladi() {
        // Each key (tenant/key) has its own counter — exhausting one does not affect another.
        let state: LimitState = Arc::new(Mutex::new(LimitBucket::new()));
        assert!(check_and_count(&state, "a", 1, 3600).is_none()); // a: 1st passes
        assert!(check_and_count(&state, "a", 1, 3600).is_some()); // a: 2nd blocked
        assert!(check_and_count(&state, "b", 1, 3600).is_none()); // b: separate bucket, passes
    }

    #[test]
    fn limit_yangi_oynada_tiklanadi() {
        // window_secs=1 — after one second a new window, the count resets to zero.
        let state: LimitState = Arc::new(Mutex::new(LimitBucket::new()));
        assert!(check_and_count(&state, "k", 1, 1).is_none());
        assert!(check_and_count(&state, "k", 1, 1).is_some()); // exhausted in this window
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(
            check_and_count(&state, "k", 1, 1).is_none(),
            "count must reset in a new window"
        );
    }

    #[test]
    fn limit_eski_oyna_kalitlari_tozalanadi() {
        // So that memory does not grow without bound (Codex review P2):
        // user-controlled keys must not pile up — old-window keys are removed in
        // the sweep. window_secs=1: write "old", let the window pass, then trigger
        // the sweep with SWEEP_EVERY operations.
        let state: LimitState = Arc::new(Mutex::new(LimitBucket::new()));
        check_and_count(&state, "old", 1000, 1);
        std::thread::sleep(std::time::Duration::from_millis(1100)); // next window
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
        // Acceptance: counts correctly under parallel requests (no race).
        // 16 threads x 50 attempts = 800; exactly limit=100 of them MUST pass.
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
        // When the key is nil, req.ip is used, with the "ip:" prefix.
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
}
