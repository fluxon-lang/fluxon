// HTTP route structure and matching: path patterns, method normalization, and
// prefix matching for middleware scopes.

use std::collections::BTreeMap;

use crate::value::Value;

use super::request::percent_decode;

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

// "/notes/:id" -> [Lit("notes"), Param("id")]. Empty segments are dropped.
pub(crate) fn parse_pattern(path: &str) -> Vec<Seg> {
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

// Splits the request path into segments (without the query).
pub(crate) fn path_segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

// Normalizes a route method symbol to the lowercase form used by incoming
// requests (`req.method().as_str().to_lowercase()`). The docs prescribe `:del`
// as the canonical DELETE form, so we map the documented short spelling onto
// the actual HTTP method name — otherwise `:del` routes never match (issue #177).
pub(crate) fn normalize_method(s: &str) -> String {
    let m = s.to_lowercase();
    match m.as_str() {
        "del" => "delete".to_string(),
        _ => m,
    }
}

// Finds the first route matching method+path; on a match returns a params map.
pub(crate) fn match_route(
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
                    // Path segments also percent-encode non-ASCII (e.g.
                    // `/users/:name` -> `%D0%9A...`) — we decode it (issue #100).
                    // In a path `+` is literal, so it is not turned into a space
                    // (the form-encoding rule applies only in the query).
                    // `keep_path_seps=true`: `%2F`/`%5C` stay raw (segment
                    // invariant — no `/` inside the value, codex review).
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

// Does an http.before pattern match the request path? (issue #67)
// "/api/*" -> paths that are "/api" or start with "/api/..." (segment boundary).
// A pattern without "*" -> exact match. "/apix" does NOT match "/api/*"
// (the prefix is split on a segment boundary).
pub(crate) fn prefix_matches(pat: &str, path: &str) -> bool {
    if let Some(prefix) = pat.strip_suffix("/*") {
        // "/api/*" → "/api" itself or anything starting with "/api/".
        path == prefix || path.starts_with(&format!("{}/", prefix))
    } else {
        // No pattern — exact path match.
        pat == path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- http.before prefix matching (issue #67) ---

    #[test]
    fn prefix_yulduz_aniq_prefiks_mos() {
        assert!(prefix_matches("/api/*", "/api"));
        assert!(prefix_matches("/api/*", "/api/users"));
        assert!(prefix_matches("/api/*", "/api/users/1"));
    }

    #[test]
    fn prefix_yulduz_segment_chegarasi() {
        // "/apix" must NOT fall under "/api/*" — the boundary is a segment.
        assert!(!prefix_matches("/api/*", "/apix"));
        assert!(!prefix_matches("/api/*", "/apixyz/foo"));
    }

    #[test]
    fn prefix_yulduzsiz_aniq_mos() {
        assert!(prefix_matches("/health", "/health"));
        assert!(!prefix_matches("/health", "/health/check"));
        assert!(!prefix_matches("/health", "/healthz"));
    }

    // --- :del canonical method maps to DELETE (issue #177) ---

    #[test]
    fn del_canonical_delete_ga_map_qilinadi() {
        // `:del` is the documented canonical DELETE form — it must normalize to
        // "delete" so the route matches incoming DELETE requests (issue #177).
        assert_eq!(normalize_method("del"), "delete");
        assert_eq!(normalize_method("DEL"), "delete");
        // The explicit/standard spellings stay as-is (lowercased).
        assert_eq!(normalize_method("delete"), "delete");
        assert_eq!(normalize_method("DELETE"), "delete");
        assert_eq!(normalize_method("get"), "get");

        // A `:del` route matches a "delete" request, mirroring runtime dispatch.
        let routes = vec![Route {
            method: normalize_method("del"),
            pattern: parse_pattern("/x"),
            handler: Value::Nil,
        }];
        assert!(match_route(&routes, "delete", "/x").is_some());
    }
}
