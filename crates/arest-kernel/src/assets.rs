// crates/arest-kernel/src/assets.rs
//
// Static-asset lookup for the ui.do bundle (#266 → #580).
//
// ── #580 — cell-graph runtime serving ───────────────────────────────
//
// Pre-#580 the kernel served the ui.do React bundle straight out of
// a build-time `include_bytes!` table emitted by `build.rs` into
// `$OUT_DIR/ui_assets.rs`. That made the static-asset path a pure
// compile-time table walk but coupled the kernel binary's footprint
// to whatever was sitting under `apps/ui.do/dist/` at build time.
// #581 wants to lift the ui.do source out of this repo — and it
// can't do that until the kernel stops `include_bytes!`-ing the
// bundle.
//
// The split lands in two layers:
//
//   * Layer A (this file). The runtime asset handler queries the
//     File cell graph held in the live SYSTEM state. `lookup_from_state`
//     walks `File_has_*` cells looking for a `path` binding that
//     matches the requested URL and returns the asset's bytes via
//     `File_has_ContentRef` (which `file_serve.rs` already populates
//     and decodes for `POST /file` uploads). Profile-agnostic: when
//     no cells are seeded, every asset path 404s; when cells are
//     seeded (e.g. by `system::seed_ui_bundle_cells` under the
//     `ui-bundle` feature) the same handler serves them.
//
//   * Layer B (system.rs). At boot, gated on `feature = "ui-bundle"`,
//     `system::seed_ui_bundle_cells` walks the build-time `UI_ASSETS`
//     table and inserts one File cell per entry. This is the bridge
//     for #581: today the cells get filled from `include_bytes!`;
//     once #581 lifts ui.do out, the same cells will be filled from
//     a runtime fetch (HTTP / freeze blob / disk image) without any
//     other code in the kernel needing to change.
//
// ── Routing rules (unchanged from #266) ─────────────────────────────
//
//   * `GET /`              → index.html
//   * `GET /assets/<hash>` → exact asset match (404 on miss)
//   * `GET /api/*`         → None, handler falls back to system::dispatch
//   * `GET /arest/*`       → None, handler falls back to HATEOAS / dispatch
//   * `GET /<anything>`    → index.html (SPA fallback for HTML5 router)
//
// Cache policy is filename-aware: Vite hashes asset names so the
// `/assets/*` tree is safe to mark immutable; `index.html` and any
// SPA-fallback response must be `no-cache` so a freshly-deployed
// bundle picks up its new asset URLs.

#![allow(dead_code)]

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use arest::ast::{self, Object};

include!(concat!(env!("OUT_DIR"), "/ui_assets.rs"));

/// Response descriptor for a matched static asset. The handler
/// copies `body` into the response buffer and emits `content_type`
/// and `cache_control` as HTTP headers.
///
/// `body` is owned (`Vec<u8>`) rather than a `&'static [u8]`
/// reference because `lookup_from_state` may decode the bytes out
/// of a `File_has_ContentRef` atom (hex-encoded inline content) at
/// request time. The pre-#580 `&'static [u8]` shape only worked
/// when bytes lived in the build-time `UI_ASSETS` table; the
/// cell-graph path needs to materialise them on each lookup.
#[derive(Debug, Clone)]
pub struct Asset {
    pub content_type: &'static str,
    pub cache_control: &'static str,
    pub body: Vec<u8>,
}

/// Path-level routing — common to both the static-table and
/// cell-graph lookup paths. Returns `Some(canonical_path)` when the
/// request should be resolved as a static asset (`/` is rewritten
/// to `/index.html`); returns `None` when the handler should fall
/// through to a non-asset tier (`/api/*` and `/arest/*` are owned
/// by `system::dispatch` / the HATEOAS read fallback).
fn route_for(path: &str) -> Option<&str> {
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
    Some(if path == "/" { "/index.html" } else { path })
}

/// Locate the asset for `path` in the build-time `UI_ASSETS` table.
/// Pre-#580 entry point — kept for the host-side cell-graph–free
/// tests (the lookup_from_state path is the wire path post-#580).
/// Returns `None` for API paths and for misses under `/assets/`
/// (real 404s). Returns `Some(asset)` for `/`, `/index.html`, any
/// exact match, and — when a bundle is baked in — any other path
/// (SPA router fallback).
pub fn lookup(path: &str) -> Option<Asset> {
    let lookup_path = route_for(path)?;
    let original = path.split('?').next().unwrap_or(path);

    if let Some((p, body)) = UI_ASSETS.iter().find(|(p, _)| *p == lookup_path) {
        return Some(Asset {
            content_type: content_type_for(p),
            cache_control: cache_control_for(p),
            body: body.to_vec(),
        });
    }

    // A miss under `/assets/` is a real 404 — we never SPA-fallback
    // into the React shell for a versioned asset.
    if original.starts_with("/assets/") {
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
            body: body.to_vec(),
        })
}

/// Locate the asset for `path` in the live SYSTEM state's File cell
/// graph (#580). Walks `File_has_Path` for a fact whose `Path`
/// binding matches `lookup_path`, then materialises the bytes via
/// the same `File_has_ContentRef` decoder shape that
/// `file_serve.rs::lookup_content_ref` + `decode_content_ref`
/// implement.
///
/// Profile-agnostic: returns `None` for every asset path when no
/// cells are seeded, so the handler falls through to
/// `system::dispatch` exactly the way the empty-bundle build does
/// today. When `system::seed_ui_bundle_cells` (or any future loader)
/// has populated the cell graph, this is the path the wire handler
/// uses.
///
/// SPA fallback semantics mirror the static-table path: a non-`/api`,
/// non-`/arest`, non-`/assets/` miss returns `index.html` from the
/// cell graph (when present) so React-router can claim the URL
/// client-side.
pub fn lookup_from_state(state: &Object, path: &str) -> Option<Asset> {
    let lookup_path = route_for(path)?;
    let original = path.split('?').next().unwrap_or(path);

    if let Some(asset) = lookup_in_cells(state, lookup_path) {
        return Some(asset);
    }

    // A miss under `/assets/` is a real 404 — never SPA-fallback for
    // versioned assets.
    if original.starts_with("/assets/") {
        return None;
    }

    // SPA fallback: pull `/index.html` out of the cell graph (if
    // present) and serve it under the original path. When the cell
    // graph has no bundle seeded, this returns `None` and the
    // handler drops to `system::dispatch`.
    lookup_in_cells(state, "/index.html").map(|mut a| {
        a.content_type = "text/html; charset=utf-8";
        a.cache_control = CACHE_HTML;
        a
    })
}

/// Walk `File_has_Path` to find a fact whose `Path` binding matches
/// `path`, then resolve its `File` id and look up the corresponding
/// `File_has_ContentRef` entry. Returns the decoded inline bytes as
/// a fresh `Asset` with cache + content-type derived from the path.
///
/// Mirror of `file_serve.rs::lookup_content_ref` + `decode_content_ref`,
/// duplicated rather than re-exported because `file_serve` is gated
/// on `target_arch = "x86_64"` (it reaches `block_storage` for the
/// `<REGION,...>` content-ref shape) and `assets` is target-agnostic.
/// The duplication accepts only the inline-hex shape — the only one
/// the boot-time bundle seeder produces — and hands back `None` for
/// any tagged form so a future REGION-backed entry can't accidentally
/// drop bytes on the floor here.
fn lookup_in_cells(state: &Object, path: &str) -> Option<Asset> {
    // 1. Look up the File id for `path` via `File_has_Path`.
    let path_cell = ast::fetch_or_phi("File_has_Path", state);
    let file_id: String = path_cell.as_seq()?.iter().find_map(|fact| {
        if ast::binding(fact, "Path") == Some(path) {
            ast::binding(fact, "File").map(|s| s.to_string())
        } else {
            None
        }
    })?;

    // 2. Pull the bytes out of `File_has_ContentRef`.
    let cref_cell = ast::fetch_or_phi("File_has_ContentRef", state);
    let cref: String = cref_cell.as_seq()?.iter().find_map(|fact| {
        if ast::binding(fact, "File") == Some(&file_id) {
            ast::binding(fact, "ContentRef").map(|s| s.to_string())
        } else {
            None
        }
    })?;
    let body = decode_inline_hex(&cref)?;

    Some(Asset {
        content_type: content_type_for(path),
        cache_control: cache_control_for(path),
        body,
    })
}

/// Decode a `ContentRef` atom that the bundle seeder produced (#580):
/// either bare lowercase hex (the `encode_inline_content_ref` shape
/// `file_upload.rs` emits today) or the tagged `<INLINE,...>` form.
/// REGION-backed refs are intentionally rejected — the boot-time
/// seeder writes inline bytes only, and a region-backed asset would
/// need to round-trip through `block_storage` which this module
/// can't reach on every target.
fn decode_inline_hex(cref: &str) -> Option<Vec<u8>> {
    let body = if let Some(rest) = cref.strip_prefix("<INLINE,") {
        rest.strip_suffix('>')?
    } else if cref.starts_with("<REGION,") {
        // Region-backed refs aren't supported on the asset path —
        // the seeder writes inline bytes only.
        return None;
    } else {
        cref
    };
    decode_hex(body)
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() {
        return Some(Vec::new());
    }
    let bs = s.as_bytes();
    if bs.len() % 2 != 0 {
        return None;
    }
    let mut out: Vec<u8> = Vec::with_capacity(bs.len() / 2);
    let mut i = 0;
    while i + 1 < bs.len() {
        let hi = nibble(bs[i])?;
        let lo = nibble(bs[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Some(out)
}

fn nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Encode raw bytes as a bare lowercase-hex Atom, matching
/// `file_upload.rs::encode_inline_content_ref`. The seeder uses
/// this so the cell graph round-trips through the same decoder
/// shape that `file_serve` honours.
pub fn encode_inline_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(hex_nibble(b >> 4));
        s.push(hex_nibble(b & 0xF));
    }
    s
}

fn hex_nibble(n: u8) -> char {
    match n & 0xF {
        0..=9   => (b'0' + n) as char,
        10..=15 => (b'a' + (n - 10)) as char,
        _       => unreachable!(),
    }
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
        // should not land in the match key. Pre-#580 this test asserted
        // pointer-equality of the bundled body (the static slice was
        // shared); post-#580 `Asset.body` is `Vec<u8>` (so the lookup
        // returns a fresh allocation each call) — the contract is the
        // same body bytes either way.
        let a = lookup("/index.html?v=1");
        let b = lookup("/index.html");
        match (a, b) {
            (Some(x), Some(y)) => assert_eq!(x.body, y.body),
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

    // ── #580 cell-graph lookup tests ─────────────────────────────────
    //
    // These tests exercise `lookup_from_state` directly rather than
    // through `system::with_state` so they don't contend with the
    // module-level SYSTEM singleton (or with whatever `init()` baked
    // in for other tests in the same binary). Each test builds a
    // local `Object` containing the File cells it needs and calls
    // `lookup_from_state(&local, path)` for the assertion.

    use alloc::vec;
    use arest::ast::{cell_push, fact_from_pairs};

    /// Helper — push a single fake File cell into `state` carrying
    /// `path` and the inline-hex-encoded `body`.
    fn push_file_cell(state: Object, file_id: &str, path: &str, body: &[u8]) -> Object {
        let cref = encode_inline_hex(body);
        let s = cell_push(
            "File_has_Path",
            fact_from_pairs(&[("File", file_id), ("Path", path)]),
            &state,
        );
        cell_push(
            "File_has_ContentRef",
            fact_from_pairs(&[("File", file_id), ("ContentRef", &cref)]),
            &s,
        )
    }

    /// Asset served from cell graph — the basic round-trip the wire
    /// handler relies on.
    #[test]
    fn lookup_from_state_returns_seeded_bytes() {
        let state = push_file_cell(Object::phi(), "ui-1", "/foo.js", b"console.log('hi');");
        let asset = lookup_from_state(&state, "/foo.js")
            .expect("seeded /foo.js must be served from the cell graph");
        assert_eq!(asset.body, b"console.log('hi');".to_vec());
        assert_eq!(asset.content_type, "application/javascript; charset=utf-8");
    }

    /// 404 on missing path — no cell, no bytes.
    #[test]
    fn lookup_from_state_misses_unseeded_path() {
        // Empty cell graph — every asset path 404s.
        let state = Object::phi();
        assert!(lookup_from_state(&state, "/foo.js").is_none());
        assert!(lookup_from_state(&state, "/").is_none());
    }

    /// `/api/*` and `/arest/*` are never asset paths — even when a
    /// File cell happens to have a colliding `Path` binding (which
    /// would only happen via a weird seeding choice; the prefix
    /// guards still kick in first).
    #[test]
    fn lookup_from_state_excludes_api_and_arest() {
        let state = push_file_cell(Object::phi(), "rogue", "/api/welcome", b"oops");
        assert!(lookup_from_state(&state, "/api/welcome").is_none());
        assert!(lookup_from_state(&state, "/arest/organizations").is_none());
    }

    /// `/` resolves to `/index.html` against the cell graph.
    #[test]
    fn lookup_from_state_serves_root_as_index_html() {
        let state = push_file_cell(
            Object::phi(),
            "idx",
            "/index.html",
            b"<!doctype html>",
        );
        let asset =
            lookup_from_state(&state, "/").expect("/ must resolve to /index.html when seeded");
        assert_eq!(asset.body, b"<!doctype html>".to_vec());
        assert_eq!(asset.content_type, "text/html; charset=utf-8");
        assert_eq!(asset.cache_control, CACHE_HTML);
    }

    /// SPA fallback: arbitrary path with no matching File cell, but
    /// `/index.html` is seeded — falls back to index.html so the
    /// React router can claim the URL client-side.
    #[test]
    fn lookup_from_state_spa_falls_back_to_index_html() {
        let state = push_file_cell(
            Object::phi(),
            "idx",
            "/index.html",
            b"<!doctype html>",
        );
        let asset = lookup_from_state(&state, "/Organization/abc")
            .expect("SPA fallback must return index.html bytes when seeded");
        assert_eq!(asset.body, b"<!doctype html>".to_vec());
        assert_eq!(asset.content_type, "text/html; charset=utf-8");
        assert_eq!(asset.cache_control, CACHE_HTML);
    }

    /// `/assets/<hash>.<ext>` miss is a real 404 — never SPA-fall back.
    #[test]
    fn lookup_from_state_assets_miss_is_real_404() {
        let state = push_file_cell(
            Object::phi(),
            "idx",
            "/index.html",
            b"<!doctype html>",
        );
        // Even with index.html seeded, a `/assets/...` miss must not
        // fall back into the React shell.
        assert!(lookup_from_state(&state, "/assets/does-not-exist.js").is_none());
    }

    /// Profile-agnostic: the empty cell graph (the `--features server`
    /// shape with no `ui-bundle`) returns `None` for every asset path
    /// without panicking and without compile-time errors.
    #[test]
    fn lookup_from_state_empty_state_returns_404_for_all_asset_paths() {
        let state = Object::phi();
        // Sample of the path shapes the kernel HTTP handler sees.
        let paths = vec![
            "/",
            "/index.html",
            "/assets/index-abc123.js",
            "/assets/main.css",
            "/Organization/acme",
            "/Noun/SupportRequest",
        ];
        for p in paths {
            assert!(
                lookup_from_state(&state, p).is_none(),
                "empty cell graph must 404 every asset path; offender: {}",
                p,
            );
        }
    }

    /// Hex round-trip — the encoder + decoder agree on every byte
    /// value, so the bundle seeder's encoding survives a trip through
    /// the cell graph.
    #[test]
    fn encode_then_decode_round_trips_all_bytes() {
        let original: Vec<u8> = (0u8..=255u8).collect();
        let encoded = encode_inline_hex(&original);
        let decoded = decode_hex(&encoded).expect("encoded hex must decode");
        assert_eq!(decoded, original);
    }

    /// REGION-tagged content refs aren't supported on the asset path
    /// (the seeder writes inline bytes only). A REGION ref must
    /// resolve to `None` rather than silently returning empty bytes.
    #[test]
    fn lookup_from_state_rejects_region_content_ref() {
        let state = cell_push(
            "File_has_Path",
            fact_from_pairs(&[("File", "rg"), ("Path", "/big.bin")]),
            &Object::phi(),
        );
        let state = cell_push(
            "File_has_ContentRef",
            fact_from_pairs(&[("File", "rg"), ("ContentRef", "<REGION,4096,131072>")]),
            &state,
        );
        assert!(
            lookup_from_state(&state, "/big.bin").is_none(),
            "REGION content refs must not be served from the asset path",
        );
    }

    /// Tagged INLINE form decodes the same as bare hex — both shapes
    /// land in the same Asset bytes.
    #[test]
    fn lookup_from_state_accepts_tagged_inline_content_ref() {
        let cref_tagged = "<INLINE,48656c6c6f>"; // "Hello"
        let state = cell_push(
            "File_has_Path",
            fact_from_pairs(&[("File", "tg"), ("Path", "/hello.txt")]),
            &Object::phi(),
        );
        let state = cell_push(
            "File_has_ContentRef",
            fact_from_pairs(&[("File", "tg"), ("ContentRef", cref_tagged)]),
            &state,
        );
        let asset = lookup_from_state(&state, "/hello.txt")
            .expect("tagged INLINE content refs must round-trip");
        assert_eq!(asset.body, b"Hello".to_vec());
    }

}
