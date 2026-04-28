// crates/arest-kernel/src/assets.rs
//
// Static-asset lookup for the baked ui.do bundle (#266).
//
// `build.rs` emits `$OUT_DIR/ui_assets.rs` with a static
// `UI_ASSETS: &[(&str, &[u8])]` table — one entry per file under
// `apps/ui.do/dist/` keyed on the HTTP path. This module wraps that
// table with the routing the kernel HTTP handler needs:
//
//   * `GET /`              → index.html
//   * `GET /assets/<hash>` → exact asset match (404 on miss)
//   * `GET /api/*`         → None, handler falls back to system::dispatch
//   * `GET /<anything>`    → index.html (SPA fallback for HTML5 router)
//
// Cache policy is filename-aware: Vite hashes asset names so the
// `/assets/*` tree is safe to mark immutable; `index.html` and any
// SPA-fallback response must be `no-cache` so a freshly-deployed
// bundle picks up its new asset URLs.

#![allow(dead_code)]

include!(concat!(env!("OUT_DIR"), "/ui_assets.rs"));

/// Response descriptor for a matched static asset. The handler
/// copies `body` into the response buffer and emits `content_type`
/// and `cache_control` as HTTP headers.
#[derive(Debug, Clone, Copy)]
pub struct Asset {
    pub content_type: &'static str,
    pub cache_control: &'static str,
    pub body: &'static [u8],
}

/// Locate the asset for `path`. Returns `None` for API paths and
/// for misses under `/assets/` (real 404s). Returns `Some(asset)`
/// for `/`, `/index.html`, any exact match, and — when a bundle is
/// baked in — any other path (SPA router fallback).
pub fn lookup(path: &str) -> Option<Asset> {
    // Strip a trailing `?query` so `/assets/foo.js?v=1` still matches.
    let path = path.split('?').next().unwrap_or(path);

    // API paths are owned by `system::dispatch`.
    if path == "/api" || path.starts_with("/api/") {
        return None;
    }

    // `/arest/*` is the HATEOAS namespace — same `system::dispatch`
    // ownership as `/api/*`. Without this exclusion the SPA fallback
    // below rewrites `/arest/parse`, `/arest/organizations/{id}`, etc.
    // to `index.html`, masking real 404s and breaking the host-driven
    // e2e suite (#608/#609/#610). Mirrors the worker's behaviour where
    // `/arest/*` is a dynamic catch-all, never a static asset.
    if path == "/arest" || path.starts_with("/arest/") {
        return None;
    }

    // `/` resolves to `/index.html`.
    let lookup_path = if path == "/" { "/index.html" } else { path };

    if let Some((p, body)) = UI_ASSETS.iter().find(|(p, _)| *p == lookup_path) {
        return Some(Asset {
            content_type: content_type_for(p),
            cache_control: cache_control_for(p),
            body,
        });
    }

    // A miss under `/assets/` is a real 404 — we never SPA-fallback
    // into the React shell for a versioned asset.
    if path.starts_with("/assets/") {
        return None;
    }

    // SPA fallback: any non-asset, non-API path that didn't match
    // gets served `index.html` so the React router can claim it.
    // When no bundle is baked in, UI_ASSETS is empty and this lookup
    // returns None — the handler falls through to `system::dispatch`.
    UI_ASSETS
        .iter()
        .find(|(p, _)| *p == "/index.html")
        .map(|(_, body)| Asset {
            content_type: "text/html; charset=utf-8",
            cache_control: CACHE_HTML,
            body,
        })
}

/// True when a bundle is baked into the binary. The handler uses
/// this to decide whether the HTML path is served at all.
pub fn has_bundle() -> bool {
    !UI_ASSETS.is_empty()
}

/// MIME type keyed on file extension. Covers the filetypes Vite
/// emits by default — `.html`, `.js`, `.css`, fonts, images — and
/// a few safe extras. Unknown extensions fall back to
/// `application/octet-stream` so the browser treats them as opaque
/// rather than mis-sniffing.
pub fn content_type_for(path: &str) -> &'static str {
    let ext = extension(path).unwrap_or("");
    match ext {
        "html" | "htm"  => "text/html; charset=utf-8",
        "js" | "mjs"    => "application/javascript; charset=utf-8",
        "css"           => "text/css; charset=utf-8",
        "json"          => "application/json",
        "map"           => "application/json",
        "svg"           => "image/svg+xml",
        "png"           => "image/png",
        "jpg" | "jpeg"  => "image/jpeg",
        "gif"           => "image/gif",
        "webp"          => "image/webp",
        "ico"           => "image/x-icon",
        "woff"          => "font/woff",
        "woff2"         => "font/woff2",
        "ttf"           => "font/ttf",
        "otf"           => "font/otf",
        "wasm"          => "application/wasm",
        "txt"           => "text/plain; charset=utf-8",
        _               => "application/octet-stream",
    }
}

/// Cache-Control value keyed on the asset path. Hashed paths under
/// `/assets/` are immutable for a year; everything else (index.html,
/// SPA fallbacks) is `no-cache` so a redeploy is picked up on the
/// next request.
pub fn cache_control_for(path: &str) -> &'static str {
    if path.starts_with("/assets/") {
        CACHE_IMMUTABLE
    } else {
        CACHE_HTML
    }
}

/// Cache directive for hashed assets: cache aggressively, never
/// revalidate. Vite's file-hash suffix makes the URL a content hash,
/// so a changed file always has a new URL.
pub const CACHE_IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// Cache directive for HTML shells: force a round-trip so a newly-
/// deployed bundle (with new asset hashes) takes effect immediately.
pub const CACHE_HTML: &str = "no-cache";

fn extension(path: &str) -> Option<&str> {
    // Last segment after the final '/'. Avoids returning a directory
    // component that happens to contain a '.'.
    let last = path.rsplit('/').next()?;
    last.rsplit('.').next().filter(|ext| *ext != last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_for_html() {
        assert_eq!(content_type_for("/index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type_for("/nested/page.htm"), "text/html; charset=utf-8");
    }

    #[test]
    fn content_type_for_javascript() {
        assert_eq!(
            content_type_for("/assets/index-DjXaD7eJ.js"),
            "application/javascript; charset=utf-8",
        );
        assert_eq!(
            content_type_for("/assets/worker.mjs"),
            "application/javascript; charset=utf-8",
        );
    }

    #[test]
    fn content_type_for_css_and_fonts() {
        assert_eq!(content_type_for("/assets/main.css"), "text/css; charset=utf-8");
        assert_eq!(content_type_for("/assets/inter.woff2"), "font/woff2");
    }

    #[test]
    fn content_type_falls_back_to_octet_stream() {
        assert_eq!(content_type_for("/assets/blob.xyz"), "application/octet-stream");
        // Directory-like paths have no extension — don't mis-attribute
        // a dotted parent component.
        assert_eq!(content_type_for("/foo.bar/index"), "application/octet-stream");
    }

    #[test]
    fn extension_handles_dotted_directory() {
        assert_eq!(extension("/foo.bar/baz"), None);
        assert_eq!(extension("/foo.bar/baz.js"), Some("js"));
        assert_eq!(extension("/"), None);
        assert_eq!(extension("/index"), None);
    }

    #[test]
    fn cache_control_policy() {
        assert_eq!(cache_control_for("/assets/index-abc.js"), CACHE_IMMUTABLE);
        assert_eq!(cache_control_for("/index.html"), CACHE_HTML);
        assert_eq!(cache_control_for("/Noun/123"), CACHE_HTML);
    }

    // The UI_ASSETS table is generated at build time. When the repo
    // has a bundle, these tests verify the table-driven behavior;
    // when it doesn't, they verify the empty-table behavior.

    #[test]
    fn api_paths_are_never_served_by_assets() {
        assert!(lookup("/api/welcome").is_none());
        assert!(lookup("/api/entities/Noun").is_none());
        assert!(lookup("/api").is_none());
    }

    // `/arest/*` is the HATEOAS namespace owned by `system::dispatch`.
    // Without this exclusion the SPA fallback rewrites `/arest/parse`
    // (and every other dynamic path) to `index.html`, masking real
    // 404s during the kernel HATEOAS port.
    #[test]
    fn arest_paths_are_never_served_by_assets() {
        assert!(lookup("/arest/parse").is_none());
        assert!(lookup("/arest/organizations").is_none());
        assert!(lookup("/arest/organizations/abc-123").is_none());
        assert!(lookup("/arest/entity").is_none());
        assert!(lookup("/arest/entities/SupportRequest").is_none());
        assert!(lookup("/arest").is_none());
    }

    #[test]
    fn query_string_is_stripped_before_lookup() {
        // Regardless of whether a bundle is present, the query string
        // should not land in the match key.
        let a = lookup("/index.html?v=1");
        let b = lookup("/index.html");
        match (a, b) {
            (Some(x), Some(y)) => assert_eq!(x.body.as_ptr(), y.body.as_ptr()),
            (None, None) => { /* no bundle — still consistent */ }
            _ => panic!("query-string stripping is inconsistent"),
        }
    }

    #[test]
    fn assets_miss_is_a_real_404_not_a_fallback() {
        // When a bundle is present: /assets/does-not-exist.js → None.
        // When no bundle:           /assets/does-not-exist.js → None.
        // Either way, this path never SPA-falls-back.
        assert!(lookup("/assets/does-not-exist.js").is_none());
    }

    // Behaviour-split tests — each only exercises one branch of
    // `has_bundle()`.
    #[cfg(test)]
    #[test]
    fn root_serves_html_when_bundle_present() {
        if has_bundle() {
            let asset = lookup("/").expect("root must serve something when a bundle is present");
            assert_eq!(asset.content_type, "text/html; charset=utf-8");
            assert_eq!(asset.cache_control, CACHE_HTML);
        }
    }

    #[cfg(test)]
    #[test]
    fn spa_fallback_when_bundle_present() {
        if has_bundle() {
            // Arbitrary React-router path — no file at this URL,
            // must still return index.html so client-side routing runs.
            let asset = lookup("/Organization/abc").expect(
                "SPA fallback must return index.html when a bundle is present",
            );
            assert_eq!(asset.content_type, "text/html; charset=utf-8");
        }
    }

    #[cfg(test)]
    #[test]
    fn empty_table_passes_through() {
        if !has_bundle() {
            assert!(lookup("/").is_none());
            assert!(lookup("/Noun/x").is_none());
        }
    }
}
