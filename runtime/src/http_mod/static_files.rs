// Static file serving (issue #134): mount config, traversal-safe path joining,
// MIME detection, and the file/HEAD responses.

use std::path::{Component, Path, PathBuf};

use bytes::Bytes;
use http_body_util::Full;
use hyper::{Response, StatusCode};

// --- static file mount (issue #134) ---

// An `http.static prefix dir` mount. The prefix is stored split into segments
// ("/assets" -> ["assets"], "/" -> []) — matching is checked on a segment
// boundary (so "/assetsx" does not fall under the "/assets" mount). `dir` is an
// absolute path canonicalized at registration time (resolved relative to the
// script directory).
#[derive(Clone)]
pub struct StaticMount {
    pub prefix: Vec<String>,
    pub dir: PathBuf,
    // SPA fallback: if a path under the prefix matches no file, `dir/index.html`
    // is returned (the frontend router handles it itself).
    pub spa: bool,
}

// "/assets/img" -> ["assets", "img"]; "/" -> []. Empty segments are dropped.
pub(crate) fn parse_static_prefix(prefix: &str) -> Vec<String> {
    prefix
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

// Does the mount prefix match the start of the request segments? If so, returns
// the part after the prefix (the file path).
pub(crate) fn strip_mount_prefix<'a>(
    prefix: &[String],
    segs: &'a [String],
) -> Option<&'a [String]> {
    if segs.len() < prefix.len() {
        return None;
    }
    if prefix.iter().zip(segs).all(|(a, b)| a == b) {
        Some(&segs[prefix.len()..])
    } else {
        None
    }
}

// Joins segments onto the mount directory with MANDATORY traversal protection.
// Each segment is checked AFTER percent-decoding (so `%2e%2e` is caught too):
// it must be a plain name (Component::Normal) — `..`, `.`, empty, absolute, or
// Windows-prefix (`C:`) segments are rejected. An extra `\`/NUL check: such a
// name is unexpected on the filesystem anyway, so a silent 404.
pub(crate) fn safe_join(dir: &Path, rest: &[String]) -> Option<PathBuf> {
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

// Content-Type from the extension (issue requirement: automatic). An extension
// not in the list -> octet-stream (the browser downloads it, but the content is
// not corrupted).
pub(crate) fn mime_for(path: &Path) -> &'static str {
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

// Canonicalizes the candidate and confirms it is a plain file UNDER the mount
// root (codex P2): safe_join only checks lexical segments, while metadata
// follows symlinks — if a symlink inside the folder points to a file outside
// the root (e.g. /etc/passwd), the lexical protection would be bypassed. `root`
// is canonicalized at registration, so the prefix comparison works correctly.
// A symlink inside the root (whose canonical target is also under the root) is
// served as before. The returned path is canonical — the subsequent read also
// gets exactly the file that was checked.
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

// Resolves the request segments (already percent-decoded — the caller prepares
// them) to a file across the mounts. A longer prefix wins (the "/assets" mount
// is checked before the "/" mount) — the most specific mount takes it. Two
// stages: (1) the exact file (if a directory is requested, its index.html);
// (2) if not found — the `index.html` fallback of SPA mounts whose prefix
// matches. The size (bytes) is returned together from metadata — so a HEAD
// response can give Content-Length without reading the file (codex P2). Each
// candidate is confined to the root via confined_file.
pub(crate) async fn resolve_static(
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
        // Exact file. Mime from the canonical path — the real file extension,
        // not the symlink name, determines the response type.
        if let Some((canon, len)) = confined_file(&p, &m.dir).await {
            let mime = mime_for(&canon);
            return Some((canon, mime, len));
        }
        // A directory (or the prefix itself) may have been requested — try its
        // index.html. If p is a file, this silently fails.
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

// Static file response: 200 + Content-Type determined from the extension.
pub(crate) fn static_response(data: Vec<u8>, mime: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", mime)
        .body(Full::new(Bytes::from(data)))
        .unwrap()
}

// Static response for HEAD: the file is NOT read (wasted disk I/O and memory on
// a large asset — codex P2), only the size from metadata is set manually as
// Content-Length (an empty body would auto-give 0). hyper writes no body on HEAD.
pub(crate) fn static_head_response(len: u64, mime: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", mime)
        .header("content-length", len.to_string())
        .body(Full::new(Bytes::new()))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http_mod::request::percent_decode;
    use crate::http_mod::routing::path_segments;

    // --- http.static (issue #134) ---

    fn segv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn static_prefix_parse_va_moslik() {
        // "/" -> empty prefix (matches all paths); "/assets" is checked on a
        // segment boundary — "/assetsx" does NOT match.
        assert!(parse_static_prefix("/").is_empty());
        assert_eq!(parse_static_prefix("/assets"), segv(&["assets"]));
        assert_eq!(parse_static_prefix("/a/b/"), segv(&["a", "b"]));

        let pref = parse_static_prefix("/assets");
        assert!(strip_mount_prefix(&pref, &segv(&["assets", "app.css"])).is_some());
        assert!(strip_mount_prefix(&pref, &segv(&["assets"])).is_some());
        assert!(strip_mount_prefix(&pref, &segv(&["assetsx", "a.css"])).is_none());
        assert!(strip_mount_prefix(&pref, &segv(&["other"])).is_none());
        // The remainder — the file path after the prefix.
        assert_eq!(
            strip_mount_prefix(&pref, &segv(&["assets", "img", "a.png"])).unwrap(),
            segv(&["img", "a.png"])
        );
    }

    #[test]
    fn static_safe_join_traversalni_bloklaydi() {
        // Traversal protection is MANDATORY (issue #134): "..", ".", empty,
        // absolute, and backslash/NUL segments are rejected — you cannot escape
        // the directory. Percent-decode happens in the caller, so `%2e%2e` already
        // arrives here as ".." and is caught by this check.
        let dir = Path::new("/srv/public");
        assert!(safe_join(dir, &segv(&["..", "secret"])).is_none());
        assert!(safe_join(dir, &segv(&["a", "..", "b"])).is_none());
        assert!(safe_join(dir, &segv(&["."])).is_none());
        assert!(safe_join(dir, &segv(&[""])).is_none());
        assert!(safe_join(dir, &segv(&["a\\b"])).is_none());
        assert!(safe_join(dir, &segv(&["a\0b"])).is_none());
        assert!(safe_join(dir, &segv(&["/etc", "passwd"])).is_none());
        // Plain names — joined.
        let p = safe_join(dir, &segv(&["img", "a.png"])).unwrap();
        assert_eq!(p, PathBuf::from("/srv/public/img/a.png"));
        // Empty rest (the prefix itself was requested) — the directory itself.
        assert_eq!(safe_join(dir, &[]).unwrap(), PathBuf::from("/srv/public"));
    }

    #[test]
    fn static_mime_kengaytmadan() {
        // Content-Type is automatic from the extension; unknown -> octet-stream.
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

    // Splits the request path into segments by the same rule as try_serve_static
    // (percent-decode, %2F stays raw) — resolve_static now takes ready segments
    // (decoding in the caller, in one place with the prefix check).
    fn decode_segs(path: &str) -> Vec<String> {
        path_segments(path)
            .iter()
            .map(|s| percent_decode(s, true))
            .collect()
    }

    #[tokio::test]
    async fn static_resolve_uzun_prefiks_yutadi() {
        // "/" and "/assets" mounts together: /assets/a.css is served from the
        // folder of the longer prefix (the most specific mount wins).
        let root = std::env::temp_dir().join("fluxon_static_unit_1");
        std::fs::create_dir_all(root.join("dist")).unwrap();
        std::fs::create_dir_all(root.join("public")).unwrap();
        // The mount directory is canonicalized at registration (http_static) —
        // same in the test, otherwise the /tmp symlink on macOS breaks the comparison.
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
        // /assets/a.css -> public (long prefix), /a.css -> dist (root mount).
        // len — the byte count from metadata (HEAD Content-Length is given with it).
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
        // When a directory is requested, index.html.
        let (p, mime, _) = resolve_static(&mounts, &decode_segs("/")).await.unwrap();
        assert_eq!(p, dist.join("index.html"));
        assert_eq!(mime, "text/html; charset=utf-8");
        // A path not found — SPA fallback (root mount spa:true).
        let (p, _, _) = resolve_static(&mounts, &decode_segs("/no/such/page"))
            .await
            .unwrap();
        assert_eq!(p, dist.join("index.html"));
        // A file not found under /assets: the assets mount is not spa, but the
        // root SPA mount's prefix still matches — the fallback goes to it.
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
        // The lexical guard (safe_join) does not see through symlinks: a
        // symlink inside the dir pointing OUTSIDE the root must not be served
        // (codex P2 — canonicalize + root check). A symlink pointing to a
        // target INSIDE the root is still served as before.
        let root = std::env::temp_dir().join("fluxon_static_unit_3");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("public")).unwrap();
        let public = std::fs::canonicalize(root.join("public")).unwrap();
        std::fs::write(root.join("secret.txt"), "SECRET").unwrap();
        std::fs::write(public.join("inner.txt"), "inner").unwrap();
        // Points outside: public/evil.txt -> ../secret.txt
        std::os::unix::fs::symlink(root.join("secret.txt"), public.join("evil.txt")).unwrap();
        // Points inside: public/alias.txt -> public/inner.txt
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
        // Canonical path — the symlink target (the real file).
        assert_eq!(p, public.join("inner.txt"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
