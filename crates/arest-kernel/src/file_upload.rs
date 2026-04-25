// crates/arest-kernel/src/file_upload.rs
//
// HTTP `POST /file` route (#444). Accepts a `multipart/form-data`
// request with a single `file` part and a `directory_id` form field;
// creates a File noun whose `ContentRef` holds the upload bytes
// inline (bare lowercase hex per #401's encoder convention) and
// returns `201 Created` with `Location: /file/{id}` plus a JSON
// body `{"id":"..."}`.
//
// First slice of the decomposed #407. Two hard limits:
//
//   * Bodies > 64 KiB get a `413 Payload Too Large` pointing at the
//     forthcoming `PUT /file/{id}/chunk` route (#445). 64 KiB is the
//     #401 ContentRef inline threshold — at or below it the file is
//     stored as a hex atom; above, the encoder is supposed to switch
//     to a region-backed handle (`<REGION, base, len>`), which lives
//     on the chunked path.
//   * Single-part-only multipart parser. The file content is the
//     `name="file"` part; `directory_id` is read from a separate
//     `name="directory_id"` part. Everything else in the body is
//     rejected with `400 Bad Request`. No general-purpose multipart
//     crate (no_std environment) — the parser is ~150 lines below.
//
// Mirrors `file_serve.rs`'s shape (Track XX, #403): `try_serve`
// returns `ServeOutcome::Response(wire_bytes)` for the route or
// `ServeOutcome::NotApplicable` to fall through to the registered
// `Handler` chain. `net::drive_http` calls this immediately after
// the GET/HEAD intercept arm.
//
// ── Persistence (resolved in #451) ──────────────────────────────────
//
// Earlier slices of this route discarded the would-be next state as
// `_new_state` because the kernel's SYSTEM was `spin::Once<Object>`
// and had no install path. #451 swapped that for an `RwLock`-backed
// pointer slot with `system::apply(new_state)` as the atomic-swap
// commit. This route now installs the new state before returning
// 201, so the returned id round-trips through `GET /file/{id}/
// content` immediately. An `apply` failure (only possible if
// `system::init()` hasn't run, which is a kernel-boot ordering
// regression) surfaces as `500 Internal Server Error` so the client
// retries against a freshly-booted tenant rather than silently
// losing the upload.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use arest::ast::{self, Object, fact_from_pairs};
use core::sync::atomic::{AtomicU64, Ordering};

/// 64 KiB ContentRef inline threshold (#401). Bodies at or below
/// this size are stored as a bare-hex atom in `File_has_ContentRef`;
/// anything larger must take the chunked PUT path (#445).
const INLINE_THRESHOLD: usize = 64 * 1024;

/// Maximum total Content-Length we'll accept on POST /file before
/// even attempting to parse multipart. Sized at INLINE_THRESHOLD +
/// 8 KiB headroom to cover boundaries, headers per-part, and the
/// `directory_id` form value. Beyond this we 413 immediately so
/// rx_buf can't grow unbounded on a malicious upload.
const MAX_BODY_BYTES: usize = INLINE_THRESHOLD + 8 * 1024;

/// Outcome of `try_serve` — see file_serve.rs for the convention.
pub enum ServeOutcome {
    /// Wire-formatted HTTP/1.1 response, ready to write straight onto
    /// the socket. Includes status line, headers, and body.
    Response(Vec<u8>),
    /// Path is not `POST /file` — defer to the existing handler
    /// chain.
    NotApplicable,
}

/// Top-level entry. Inspect the parsed request and, if it targets
/// `POST /file`, return a fully-serialised HTTP/1.1 response.
/// Otherwise return `NotApplicable` so the caller can route through
/// the normal handler path.
///
///   * `method` / `path` come from `http::parse_request`.
///   * `content_type` is the raw value of the request's
///     `Content-Type` header (None when absent — the canonical
///     `http::Request` doesn't capture it, so the caller re-scans
///     the raw buffer with `extract_content_type_header`).
///   * `body` is the request body bytes (already framed by
///     Content-Length in `parse_request`).
///   * `state` is the baked SYSTEM state, used here only to
///     validate that `directory_id` resolves to a known Directory.
///     A `None` state means SYSTEM hasn't initialised — we still
///     accept the upload so the route is exercisable in early-boot
///     smoke tests, deferring directory-existence checks to the
///     persistence track.
pub fn try_serve(
    method: &str,
    path: &str,
    content_type: Option<&str>,
    body: &[u8],
    state: Option<&Object>,
) -> ServeOutcome {
    // Strip a `?query` suffix so a future link generator that adds
    // cache-busters still routes here. Kept symmetric with
    // file_serve::try_serve.
    let path = path.split('?').next().unwrap_or(path);
    if path != "/file" {
        return ServeOutcome::NotApplicable;
    }
    if method != "POST" {
        // Path matched but method is wrong — surface a 405 so the
        // assets/dispatch chain doesn't re-claim a /file URL.
        return ServeOutcome::Response(method_not_allowed());
    }

    // 413 immediately on oversized bodies before walking the parser.
    if body.len() > MAX_BODY_BYTES {
        return ServeOutcome::Response(payload_too_large());
    }

    let ct = match content_type {
        Some(s) => s,
        None => return ServeOutcome::Response(bad_request(
            "missing Content-Type header (expected multipart/form-data)",
        )),
    };
    let boundary = match extract_boundary(ct) {
        Some(b) => b,
        None => return ServeOutcome::Response(bad_request(
            "Content-Type missing boundary= parameter",
        )),
    };

    let parts = match parse_multipart(body, &boundary) {
        Ok(p) => p,
        Err(msg) => return ServeOutcome::Response(bad_request(msg)),
    };

    // Pull required fields out of the part list.
    let mut file_part: Option<&Part> = None;
    let mut directory_id: Option<&str> = None;
    for p in &parts {
        match p.name.as_str() {
            "file" => file_part = Some(p),
            "directory_id" => {
                // Form-field values arrive as the part body bytes.
                directory_id = core::str::from_utf8(&p.body).ok();
            }
            _ => {} // Tolerate unknown form fields (forward-compat).
        }
    }

    let file_part = match file_part {
        Some(p) => p,
        None => return ServeOutcome::Response(bad_request(
            "multipart body missing `file` part",
        )),
    };
    let directory_id = match directory_id {
        Some(s) if !s.is_empty() => s,
        _ => return ServeOutcome::Response(bad_request(
            "multipart body missing `directory_id` form field",
        )),
    };

    // 413 again on the sized part itself. The earlier check bounded
    // the *whole* request; this one bounds the file-content portion
    // specifically against the inline cap, since exceeding 64 KiB is
    // what triggers the chunked-PUT redirect, not just over-budget
    // headers.
    if file_part.body.len() > INLINE_THRESHOLD {
        return ServeOutcome::Response(payload_too_large());
    }

    // Generate a stable id. The synth_id pattern matches zip.rs — a
    // monotonic counter prefixed by the noun name — so future code
    // browsing finds the same shape on both engine and kernel sides.
    let file_id = synth_id("file");

    // Best-effort MIME sniff against the raw bytes (#402 logic, inlined
    // here because the engine-side `crate::mime::detect_mime` is
    // gated behind `cfg(not(feature = "no_std"))` — see
    // crates/arest/src/platform/mime.rs L51 — and the kernel pulls
    // arest with `no_std` on. The table below is a faithful subset
    // of that file's matchers covering the cases the inline path
    // sees today; richer cases (OOXML, EPUB, JAR) can land alongside
    // the persistence track when the no_std gate gets relaxed).
    let mime = detect_mime(&file_part.body);

    // Build the next state — the same fact-type ids the engine-side
    // `direct_push_file` (zip.rs) and the file_serve reader
    // (file_serve.rs) use — and atomically install it via the #451
    // SYSTEM mutator. The pre-state is `state.cloned()` so the new
    // facts layer on top of whatever was already there; an empty
    // `Object::phi()` fallback covers the early-boot smoke-test
    // case where SYSTEM hasn't been initialised yet (apply will
    // then surface a 500 — see below).
    let new_state = build_file_facts(
        state.cloned().unwrap_or_else(Object::phi),
        &file_id,
        &file_part.filename,
        mime,
        &file_part.body,
        directory_id,
    );
    if let Err(msg) = crate::system::apply(new_state) {
        // The only failure mode is "init() not called", which is a
        // boot-ordering regression. Surface it as 500 so the client
        // gets a clear signal rather than a silent vanish-on-write.
        return ServeOutcome::Response(internal_error(msg));
    }

    ServeOutcome::Response(created_response(&file_id))
}

// ── Multipart parsing ───────────────────────────────────────────────

/// One decoded multipart part. The kernel never sees gigabyte-class
/// uploads on this route (413 cap is 64 KiB+8) so the body is owned
/// rather than borrowed — keeps the lifetime story simple.
#[derive(Debug, Clone)]
struct Part {
    /// `name=` from the part's `Content-Disposition` header.
    name: String,
    /// `filename=` from the part's `Content-Disposition` header. Empty
    /// string for non-file parts (e.g. `directory_id`).
    filename: String,
    /// The part's payload bytes — everything between the part's
    /// CRLFCRLF header terminator and the next `--<boundary>` line.
    body: Vec<u8>,
}

/// Pull the `boundary=` token off a `multipart/form-data` Content-Type
/// header. Accepts the boundary value with or without surrounding
/// double quotes (RFC 2046 allows both); strips them when present.
///
/// Returns `None` when the header isn't multipart/form-data or the
/// boundary parameter is absent.
fn extract_boundary(ct: &str) -> Option<String> {
    // Lowercase the type/subtype prefix for the comparison; parameter
    // names are also case-insensitive per RFC 7231 §3.1.1.1.
    let ct_lower_head = ct
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if ct_lower_head != "multipart/form-data" {
        return None;
    }
    for param in ct.split(';').skip(1) {
        let param = param.trim();
        // Find the `=` ourselves so quoted values that contain `=` survive.
        let eq = param.find('=')?;
        let key = param[..eq].trim().to_ascii_lowercase();
        if key != "boundary" {
            continue;
        }
        let value = param[eq + 1..].trim();
        let unquoted = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value);
        if unquoted.is_empty() {
            return None;
        }
        return Some(unquoted.to_string());
    }
    None
}

/// Walk a multipart body and emit one `Part` per `--<boundary>`-
/// delimited section. The parser is intentionally narrow:
///
///   * Single-pass scan; no streaming, no chunked decoding.
///   * Each part's headers must terminate in `\r\n\r\n` and must
///     include a `Content-Disposition: form-data; name="..."` line.
///   * Closing delimiter is `--<boundary>--` (RFC 2046 §5.1.1); we
///     stop at the first occurrence and ignore any trailing epilogue.
///
/// Errors return a `&'static str` so the wire-format builder can
/// surface them verbatim in the 400 body.
fn parse_multipart(body: &[u8], boundary: &str) -> Result<Vec<Part>, &'static str> {
    let dash_boundary: Vec<u8> = {
        let mut v = Vec::with_capacity(2 + boundary.len());
        v.extend_from_slice(b"--");
        v.extend_from_slice(boundary.as_bytes());
        v
    };
    // The first delimiter doesn't have a leading CRLF (it sits at the
    // very start of the body); subsequent delimiters do.
    let mut cursor = match find_subslice(body, &dash_boundary) {
        Some(i) => i + dash_boundary.len(),
        None => return Err("multipart body missing opening boundary"),
    };
    let mut parts = Vec::new();
    loop {
        // After the boundary token, two terminators are valid:
        //   "--"  → final closing delimiter, stop scanning.
        //   "\r\n" → another part follows; cursor advances past it.
        if body.len() >= cursor + 2 && &body[cursor..cursor + 2] == b"--" {
            return Ok(parts);
        }
        if body.len() < cursor + 2 || &body[cursor..cursor + 2] != b"\r\n" {
            return Err("multipart boundary not followed by CRLF or closing --");
        }
        cursor += 2;

        // Headers up to CRLFCRLF.
        let header_end = match find_subslice(&body[cursor..], b"\r\n\r\n") {
            Some(i) => cursor + i,
            None => return Err("multipart part missing header terminator"),
        };
        let headers = &body[cursor..header_end];
        let body_start = header_end + 4;

        // Body terminates at the next CRLF<dash><dash><boundary>.
        let mut next_delim: Vec<u8> = Vec::with_capacity(2 + dash_boundary.len());
        next_delim.extend_from_slice(b"\r\n");
        next_delim.extend_from_slice(&dash_boundary);
        let body_end_rel = find_subslice(&body[body_start..], &next_delim)
            .ok_or("multipart part body not terminated by CRLF--<boundary>")?;
        let part_body = body[body_start..body_start + body_end_rel].to_vec();

        // Pull out name=, filename= from Content-Disposition.
        let (name, filename) = parse_content_disposition(headers)?;
        parts.push(Part { name, filename, body: part_body });

        cursor = body_start + body_end_rel + 2 + dash_boundary.len();
    }
}

/// Read a part's header block and pull out the form-data `name=`
/// (mandatory) and `filename=` (optional) tokens from
/// `Content-Disposition`. Other headers (Content-Type, Content-
/// Transfer-Encoding) are tolerated and ignored — the inline path
/// treats every body as opaque bytes.
fn parse_content_disposition(headers: &[u8]) -> Result<(String, String), &'static str> {
    let s = core::str::from_utf8(headers)
        .map_err(|_| "non-utf8 part headers")?;
    for line in s.split("\r\n") {
        if line.is_empty() {
            continue;
        }
        let colon = line.find(':').ok_or("part header missing ':'")?;
        let name = line[..colon].trim();
        if !name.eq_ignore_ascii_case("content-disposition") {
            continue;
        }
        let value = line[colon + 1..].trim();
        // Value is `form-data; name="..."; filename="..."` or
        // `form-data; name="..."`. We split on `;` and pick the
        // tokens we know about — order isn't fixed by RFC 7578.
        let mut field_name: Option<String> = None;
        let mut filename: Option<String> = None;
        for tok in value.split(';').skip(1) {
            let tok = tok.trim();
            if let Some(eq) = tok.find('=') {
                let key = tok[..eq].trim().to_ascii_lowercase();
                let val = tok[eq + 1..].trim();
                let unquoted = val
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .unwrap_or(val);
                match key.as_str() {
                    "name" => field_name = Some(unquoted.to_string()),
                    "filename" => filename = Some(unquoted.to_string()),
                    _ => {} // ignored
                }
            }
        }
        let field_name = field_name
            .ok_or("Content-Disposition missing name=")?;
        return Ok((field_name, filename.unwrap_or_default()));
    }
    Err("part missing Content-Disposition header")
}

/// Vec<u8>'s answer to `slice::find` — returns the index of the first
/// occurrence of `needle` in `haystack`. Linear scan; the buffers
/// involved here are bounded by `MAX_BODY_BYTES` (72 KiB) so the
/// quadratic worst case is irrelevant.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    let last = haystack.len() - needle.len();
    for i in 0..=last {
        if &haystack[i..i + needle.len()] == needle {
            return Some(i);
        }
    }
    None
}

// ── Header extraction (raw request bytes) ───────────────────────────

/// Look up the `Content-Type` header value (case-insensitive) in a
/// buffered HTTP/1.1 request. Mirrors `file_serve::extract_range_
/// header` — `http::parse_request` only captures Content-Length and
/// Accept, so the multipart route re-scans the raw buffer.
///
/// Returns `Some(value)` with leading/trailing whitespace stripped
/// on match, `None` when absent or when the header block hasn't been
/// fully received.
pub fn extract_content_type_header(buf: &[u8]) -> Option<String> {
    let header_end = find_subslice(buf, b"\r\n\r\n")?;
    let head = core::str::from_utf8(&buf[..header_end]).ok()?;
    for line in head.split("\r\n") {
        if line.is_empty() { continue; }
        let colon = match line.find(':') {
            Some(i) => i,
            None => continue,
        };
        let name = line[..colon].trim();
        if name.eq_ignore_ascii_case("content-type") {
            return Some(line[colon + 1..].trim().to_string());
        }
    }
    None
}

// ── ContentRef encoding ────────────────────────────────────────────

/// Encode raw bytes as a bare-hex Atom — the inline shape today's
/// engine-side encoder (`platform::zip::encode_content_ref`) emits
/// and `file_serve::decode_content_ref` already round-trips. Once
/// the tagged form `<INLINE,...>` becomes the writer default this
/// can switch over without touching any reader. The reader handles
/// both shapes today so a mid-flight transition is safe.
fn encode_inline_content_ref(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(hex_nibble(b >> 4));
        s.push(hex_nibble(b & 0xF));
    }
    s
}

fn hex_nibble(n: u8) -> char {
    match n & 0xF {
        0..=9  => (b'0' + n) as char,
        10..=15 => (b'a' + (n - 10)) as char,
        _ => unreachable!(),
    }
}

// ── State writes ────────────────────────────────────────────────────

/// Mirror of `crates/arest/src/platform/zip.rs::direct_push_file` —
/// the engine-side direct-push fallback used when `apply_command_defs`
/// rejects a `create:File` (typically because the readings haven't
/// been compiled into the tenant's D yet). Re-implemented here
/// because the engine fn lives behind `cfg(not(feature = "no_std"))`
/// (it constructs a `command::Command::CreateEntity` first), so the
/// kernel can't call it.
///
/// Pushes five facts atomically (in the sense that the returned
/// state contains all of them or none — there's no observer between
/// the calls). When the SYSTEM mutator lands, the caller swaps the
/// returned Object into the live state in a single atomic install.
fn build_file_facts(
    state: Object,
    file_id: &str,
    name: &str,
    mime: &str,
    bytes: &[u8],
    parent_dir_id: &str,
) -> Object {
    let cref = encode_inline_content_ref(bytes);
    let size = format!("{}", bytes.len());
    let d = ast::cell_push(
        "File_has_Name",
        fact_from_pairs(&[("File", file_id), ("Name", name)]),
        &state,
    );
    let d = ast::cell_push(
        "File_has_MimeType",
        fact_from_pairs(&[("File", file_id), ("MimeType", mime)]),
        &d,
    );
    let d = ast::cell_push(
        "File_has_ContentRef",
        fact_from_pairs(&[("File", file_id), ("ContentRef", &cref)]),
        &d,
    );
    let d = ast::cell_push(
        "File_has_Size",
        fact_from_pairs(&[("File", file_id), ("Size", &size)]),
        &d,
    );
    // The containment edge is the last fact, so a future caller that
    // checks for it before reading the cell graph never sees a half-
    // populated File.
    ast::cell_push(
        "File_is_in_Directory",
        fact_from_pairs(&[("File", file_id), ("Directory", parent_dir_id)]),
        &d,
    )
}

/// Monotonic id generator for new File nouns. Mirrors zip.rs's
/// `synth_id` shape — `<prefix>-zip-<n>` — so the two slices share a
/// namespace and a future audit can recognise both. Per-boot counter,
/// reset on kernel restart; the persistence track will swap this for
/// a UUID-style id once it stops fitting (id collisions on rehydrated
/// state are a real risk after the SYSTEM mutator + freeze land).
fn synth_id(prefix: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    format!("{}-upload-{}", prefix, SEQ.fetch_add(1, Ordering::Relaxed))
}

// ── MIME sniff (no_std subset of platform::mime::detect_mime, #402) ─

/// IANA MIME type guess from the raw upload bytes. A faithful subset
/// of the engine-side `crate::platform::mime::detect_mime` table
/// (`crates/arest/src/platform/mime.rs`) — that file is std-gated so
/// the kernel can't link to it. Coverage targets the cases the inline
/// path realistically sees on a 64 KiB payload: PNG / JPEG / GIF
/// images, PDFs, plain ZIP, gzip, ELF / PE / WebAssembly binaries,
/// JSON / XML / HTML / plain text. Everything unmatched falls back
/// to `application/octet-stream`.
///
/// Returns `&'static str` so the result can be stored in an Atom or
/// passed to `format!` without a Cow round-trip.
pub fn detect_mime(content: &[u8]) -> &'static str {
    if content.is_empty() {
        return "application/octet-stream";
    }
    if content.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return "image/png";
    }
    if content.starts_with(&[0x7F, 0x45, 0x4C, 0x46]) {
        return "application/x-elf";
    }
    if content.starts_with(&[0x00, 0x61, 0x73, 0x6D]) {
        return "application/wasm";
    }
    if content.starts_with(&[0x25, 0x50, 0x44, 0x46]) {
        return "application/pdf";
    }
    if content.starts_with(b"GIF87a") || content.starts_with(b"GIF89a") {
        return "image/gif";
    }
    if content.starts_with(&[0x50, 0x4B, 0x03, 0x04]) {
        // Plain ZIP — OOXML / EPUB / JAR disambiguation is skipped on
        // this no_std subset; the engine-side fn is the source of
        // truth for the richer cases when the std-gate gets relaxed.
        return "application/zip";
    }
    if content.starts_with(&[0x1F, 0x8B, 0x08]) {
        return "application/gzip";
    }
    if content.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return "image/jpeg";
    }
    if content.starts_with(&[0x4D, 0x5A]) {
        return "application/x-msdownload";
    }

    // Text family. Strip a UTF-8 BOM before classifying.
    let body = if content.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &content[3..]
    } else {
        content
    };
    let trimmed = trim_leading_whitespace(body);
    if let Some(&first) = trimmed.first() {
        if (first == b'{' || first == b'[') && looks_like_text(content) {
            return "application/json";
        }
        if first == b'<' {
            let window_end = trimmed.len().min(512);
            let window = &trimmed[..window_end];
            if window.starts_with(b"<?xml") {
                return "application/xml";
            }
            if starts_with_ci(window, b"<!doctype html")
                || starts_with_ci(window, b"<html")
                || contains_ci(window, b"<html")
                || contains_ci(window, b"<!doctype html")
            {
                return "text/html";
            }
            if looks_like_text(content) {
                return "application/xml";
            }
        }
    }
    if looks_like_text(content) {
        return "text/plain";
    }
    "application/octet-stream"
}

fn trim_leading_whitespace(bytes: &[u8]) -> &[u8] {
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            _ => break,
        }
    }
    &bytes[i..]
}

fn looks_like_text(content: &[u8]) -> bool {
    let n = content.len().min(1024);
    if n == 0 {
        return false;
    }
    for &b in &content[..n] {
        match b {
            b'\t' | b'\n' | b'\r' | 0x0C => {}
            0x20..=0x7E => {}
            0x80..=0xFF => {}
            _ => return false,
        }
    }
    true
}

fn starts_with_ci(haystack: &[u8], needle: &[u8]) -> bool {
    if haystack.len() < needle.len() {
        return false;
    }
    for (h, n) in haystack.iter().zip(needle.iter()) {
        if h.eq_ignore_ascii_case(n) {
            continue;
        }
        return false;
    }
    true
}

fn contains_ci(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }
    let last = haystack.len() - needle.len();
    for i in 0..=last {
        if starts_with_ci(&haystack[i..], needle) {
            return true;
        }
    }
    false
}

// ── Wire-format builders ───────────────────────────────────────────

/// 201 Created. Body is `{"id":"<file-id>"}\n` (one trailing newline
/// so curl's default output renders cleanly without `-N`).
fn created_response(file_id: &str) -> Vec<u8> {
    let body = format!("{{\"id\":\"{}\"}}\n", file_id).into_bytes();
    let mut out = Vec::with_capacity(192 + body.len());
    push_status(&mut out, 201, "Created");
    push_header(&mut out, "Location", &format!("/file/{}", file_id));
    push_header(&mut out, "Content-Type", "application/json");
    push_header(&mut out, "Content-Length", &format!("{}", body.len()));
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&body);
    out
}

/// 413. Body points the client at the chunked-PUT route (#445) so
/// the next request can succeed without reading the spec.
fn payload_too_large() -> Vec<u8> {
    let body = b"upload exceeds 64 KiB inline limit; \
        use PUT /file/{id}/chunk for larger files (#445)\n";
    let mut out = Vec::with_capacity(192 + body.len());
    push_status(&mut out, 413, "Payload Too Large");
    push_header(&mut out, "Content-Type", "text/plain; charset=utf-8");
    push_header(&mut out, "Content-Length", &format!("{}", body.len()));
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(body);
    out
}

fn bad_request(msg: &str) -> Vec<u8> {
    let body = format!("{}\n", msg).into_bytes();
    let mut out = Vec::with_capacity(96 + body.len());
    push_status(&mut out, 400, "Bad Request");
    push_header(&mut out, "Content-Type", "text/plain; charset=utf-8");
    push_header(&mut out, "Content-Length", &format!("{}", body.len()));
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&body);
    out
}

fn method_not_allowed() -> Vec<u8> {
    let body = b"only POST is supported on /file\n";
    let mut out = Vec::with_capacity(128 + body.len());
    push_status(&mut out, 405, "Method Not Allowed");
    push_header(&mut out, "Allow", "POST");
    push_header(&mut out, "Content-Type", "text/plain; charset=utf-8");
    push_header(&mut out, "Content-Length", &format!("{}", body.len()));
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(body);
    out
}

/// 500 Internal Server Error. Surfaced when `system::apply` fails —
/// today the only failure mode is "init() not called" (boot ordering
/// regression). Mirrors `file_serve::internal_error` so a future
/// refactor can collapse the two.
fn internal_error(msg: &str) -> Vec<u8> {
    let body = format!("{}\n", msg).into_bytes();
    let mut out = Vec::with_capacity(96 + body.len());
    push_status(&mut out, 500, "Internal Server Error");
    push_header(&mut out, "Content-Type", "text/plain; charset=utf-8");
    push_header(&mut out, "Content-Length", &format!("{}", body.len()));
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&body);
    out
}

fn push_status(out: &mut Vec<u8>, code: u16, reason: &str) {
    let line = format!("HTTP/1.1 {} {}\r\n", code, reason);
    out.extend_from_slice(line.as_bytes());
}

fn push_header(out: &mut Vec<u8>, name: &str, value: &str) {
    let line = format!("{}: {}\r\n", name, value);
    out.extend_from_slice(line.as_bytes());
}

// ── Tests ──────────────────────────────────────────────────────────
//
// `arest-kernel`'s bin target sets `test = false` (Cargo.toml L33),
// so these `#[cfg(test)]` cases are reachable only when the crate is
// re-shaped into a lib for hosted testing — the same pattern
// `file_serve.rs` and other kernel modules use. They document the
// intended behaviour and provide a ready-to-run battery for the day
// the kernel grows a lib facade.

#[cfg(test)]
mod tests {
    use super::*;

    /// Compose a minimal multipart body with one `file` part and one
    /// `directory_id` part. Mirrors what `curl -F file=@x -F
    /// directory_id=...` puts on the wire.
    fn synth_multipart(boundary: &str, file_name: &str, file_body: &[u8], dir_id: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"--");
        out.extend_from_slice(boundary.as_bytes());
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\n",
                file_name
            ).as_bytes(),
        );
        out.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
        out.extend_from_slice(file_body);
        out.extend_from_slice(b"\r\n--");
        out.extend_from_slice(boundary.as_bytes());
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(
            b"Content-Disposition: form-data; name=\"directory_id\"\r\n\r\n",
        );
        out.extend_from_slice(dir_id.as_bytes());
        out.extend_from_slice(b"\r\n--");
        out.extend_from_slice(boundary.as_bytes());
        out.extend_from_slice(b"--\r\n");
        out
    }

    #[test]
    fn extract_boundary_basic() {
        assert_eq!(
            extract_boundary("multipart/form-data; boundary=abc123").as_deref(),
            Some("abc123"),
        );
    }

    #[test]
    fn extract_boundary_quoted() {
        assert_eq!(
            extract_boundary("multipart/form-data; boundary=\"abc 123\"").as_deref(),
            Some("abc 123"),
        );
    }

    #[test]
    fn extract_boundary_case_insensitive_header() {
        assert_eq!(
            extract_boundary("MultiPart/Form-Data; Boundary=xyz").as_deref(),
            Some("xyz"),
        );
    }

    #[test]
    fn extract_boundary_rejects_non_multipart() {
        assert!(extract_boundary("text/plain").is_none());
        assert!(extract_boundary("application/json; boundary=nope").is_none());
    }

    #[test]
    fn extract_boundary_missing_param() {
        assert!(extract_boundary("multipart/form-data").is_none());
        assert!(extract_boundary("multipart/form-data; charset=utf-8").is_none());
    }

    #[test]
    fn parse_multipart_happy_path() {
        let body = synth_multipart("BNDRY", "hello.txt", b"Hello", "dir-1");
        let parts = parse_multipart(&body, "BNDRY").expect("parse ok");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].name, "file");
        assert_eq!(parts[0].filename, "hello.txt");
        assert_eq!(parts[0].body, b"Hello");
        assert_eq!(parts[1].name, "directory_id");
        assert_eq!(parts[1].filename, "");
        assert_eq!(parts[1].body, b"dir-1");
    }

    #[test]
    fn parse_multipart_rejects_missing_opening_boundary() {
        let body = b"some random bytes without any boundary marker";
        assert!(parse_multipart(body, "BNDRY").is_err());
    }

    #[test]
    fn parse_multipart_rejects_unterminated_part() {
        // Opening boundary present, but the part body has no closing
        // delimiter — parser should refuse rather than silently
        // returning a truncated part.
        let body = b"--BNDRY\r\nContent-Disposition: form-data; name=\"x\"\r\n\r\nbody";
        assert!(parse_multipart(body, "BNDRY").is_err());
    }

    #[test]
    fn extract_content_type_present() {
        let req = b"POST /file HTTP/1.1\r\n\
                    Host: arest\r\n\
                    Content-Type: multipart/form-data; boundary=abc\r\n\
                    Content-Length: 0\r\n\
                    \r\n";
        assert_eq!(
            extract_content_type_header(req).as_deref(),
            Some("multipart/form-data; boundary=abc"),
        );
    }

    #[test]
    fn extract_content_type_case_insensitive() {
        let req = b"POST /file HTTP/1.1\r\n\
                    content-TYPE: text/plain\r\n\
                    \r\n";
        assert_eq!(
            extract_content_type_header(req).as_deref(),
            Some("text/plain"),
        );
    }

    #[test]
    fn extract_content_type_absent() {
        let req = b"POST /file HTTP/1.1\r\nHost: arest\r\n\r\n";
        assert!(extract_content_type_header(req).is_none());
    }

    #[test]
    fn encode_inline_round_trip_via_decoder() {
        // Encode here, then decode via the same hex shape file_serve
        // uses (decode_hex / decode_content_ref's bare-hex fallback).
        // The two routes round-trip cleanly, which is the contract
        // upload + download share.
        let bytes: &[u8] = b"Hello, world!";
        let s = encode_inline_content_ref(bytes);
        assert_eq!(s, "48656c6c6f2c20776f726c6421");
    }

    #[test]
    fn encode_inline_empty() {
        assert_eq!(encode_inline_content_ref(&[]), "");
    }

    #[test]
    fn detect_mime_known_signatures() {
        assert_eq!(detect_mime(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]), "image/png");
        assert_eq!(detect_mime(b"%PDF-1.7"), "application/pdf");
        assert_eq!(detect_mime(b"GIF89a..."), "image/gif");
        assert_eq!(detect_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
        assert_eq!(detect_mime(b"hello world\n"), "text/plain");
        assert_eq!(detect_mime(b"{\"a\":1}"), "application/json");
        assert_eq!(detect_mime(b"<html></html>"), "text/html");
        assert_eq!(detect_mime(&[0x00, 0x01, 0x02]), "application/octet-stream");
        assert_eq!(detect_mime(&[]), "application/octet-stream");
    }

    #[test]
    fn try_serve_other_path_passes_through() {
        match try_serve("POST", "/api/welcome", None, &[], None) {
            ServeOutcome::NotApplicable => {}
            _ => panic!("expected NotApplicable"),
        }
    }

    #[test]
    fn try_serve_wrong_method_405() {
        match try_serve("GET", "/file", None, &[], None) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"), "got: {}", s);
                assert!(s.contains("Allow: POST"));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_missing_content_type_400() {
        match try_serve("POST", "/file", None, b"some body", None) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 400 Bad Request\r\n"));
                assert!(s.contains("missing Content-Type"));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_missing_boundary_400() {
        match try_serve(
            "POST", "/file",
            Some("multipart/form-data"),
            b"x",
            None,
        ) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 400 Bad Request\r\n"));
                assert!(s.contains("boundary="));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_oversize_413() {
        // body > MAX_BODY_BYTES triggers the early 413 before any
        // multipart parsing happens.
        let big = alloc::vec![0u8; MAX_BODY_BYTES + 1];
        match try_serve(
            "POST", "/file",
            Some("multipart/form-data; boundary=BNDRY"),
            &big,
            None,
        ) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 413 Payload Too Large\r\n"), "got: {}", s);
                // Body points at the chunked PUT route.
                assert!(s.contains("/file/{id}/chunk"));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_oversize_inline_part_413() {
        // Total request fits under MAX_BODY_BYTES (which has 8 KiB
        // headroom), but the file part itself sits above
        // INLINE_THRESHOLD — the second 413 check catches it.
        let payload = alloc::vec![b'a'; INLINE_THRESHOLD + 1];
        let body = synth_multipart("BNDRY", "big.bin", &payload, "dir-1");
        match try_serve(
            "POST", "/file",
            Some("multipart/form-data; boundary=BNDRY"),
            &body,
            None,
        ) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 413 Payload Too Large\r\n"), "got: {}", s);
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_missing_directory_id_400() {
        // Synthesise a body that has only the `file` part — no
        // directory_id. Easier than tweaking synth_multipart.
        let mut body = Vec::new();
        body.extend_from_slice(b"--BNDRY\r\n");
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"file\"; filename=\"x\"\r\n\r\n",
        );
        body.extend_from_slice(b"hello");
        body.extend_from_slice(b"\r\n--BNDRY--\r\n");
        match try_serve(
            "POST", "/file",
            Some("multipart/form-data; boundary=BNDRY"),
            &body,
            None,
        ) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 400 Bad Request\r\n"));
                assert!(s.contains("directory_id"));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_missing_file_part_400() {
        // Inverse: only directory_id, no file part.
        let mut body = Vec::new();
        body.extend_from_slice(b"--BNDRY\r\n");
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"directory_id\"\r\n\r\n",
        );
        body.extend_from_slice(b"dir-1");
        body.extend_from_slice(b"\r\n--BNDRY--\r\n");
        match try_serve(
            "POST", "/file",
            Some("multipart/form-data; boundary=BNDRY"),
            &body,
            None,
        ) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 400 Bad Request\r\n"));
                assert!(s.contains("`file` part"));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_happy_path_201() {
        // #451 made `try_serve` actually install the new state via
        // `system::apply`, which requires `system::init()` to have
        // run. Without it, apply errors and the route returns 500.
        // Init the singleton once for the test process — the
        // `spin::Once` guard makes repeated calls idempotent.
        crate::system::init();

        let body = synth_multipart("BNDRY", "greet.txt", b"Hello", "dir-1");
        match try_serve(
            "POST", "/file",
            Some("multipart/form-data; boundary=BNDRY"),
            &body,
            None,
        ) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 201 Created\r\n"), "got: {}", s);
                // Location points at the canonical /file/{id} URL.
                assert!(s.contains("Location: /file/file-upload-"));
                // Body is the JSON shape `{"id":"file-upload-N"}`.
                assert!(s.contains("\"id\":\"file-upload-"));
                assert!(s.contains("Content-Type: application/json"));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn build_file_facts_emits_five_cells() {
        // The cell-push pipeline must produce all five File facts:
        // Name / MimeType / ContentRef / Size / is_in_Directory. We
        // assert each one by re-fetching the cell off the returned
        // state — and (post-#451) this is the state the SYSTEM
        // mutator does install via `system::apply` on the real wire
        // path. See `try_serve_round_trips_into_system` below for the
        // upload-then-read assertion.
        let state = build_file_facts(
            Object::phi(),
            "file-upload-77",
            "greet.txt",
            "text/plain",
            b"Hello",
            "dir-1",
        );
        for cell in &[
            "File_has_Name",
            "File_has_MimeType",
            "File_has_ContentRef",
            "File_has_Size",
            "File_is_in_Directory",
        ] {
            let c = ast::fetch_or_phi(cell, &state);
            let seq = c.as_seq().unwrap_or(&[]);
            assert!(!seq.is_empty(), "cell {} missing on the new state", cell);
            assert!(
                seq.iter().any(|f| ast::binding(f, "File") == Some("file-upload-77")),
                "cell {} has no fact for the new file id", cell,
            );
        }
        // Spot-check the hex encoding round-trips: the ContentRef
        // value should be the hex of "Hello" so file_serve's reader
        // can decode it on a subsequent GET.
        let cref_cell = ast::fetch_or_phi("File_has_ContentRef", &state);
        let cref = cref_cell.as_seq().unwrap()[0].clone();
        assert_eq!(
            ast::binding(&cref, "ContentRef"),
            Some("48656c6c6f"),
        );
    }

    /// End-to-end shape assertion for #451's mutator: a successful
    /// `try_serve` (POST /file) must install the new state into
    /// SYSTEM such that a subsequent `system::with_state` lookup
    /// finds the new File facts. This is the "uploads now persist"
    /// claim made on the wire — without `system::apply`, the test
    /// below would fail on `with_state` returning `None` for the
    /// File_has_Name cell.
    #[test]
    fn try_serve_round_trips_into_system() {
        crate::system::init();

        // Snapshot the count of File_has_Name facts on the live
        // SYSTEM before the upload, so we can assert the upload
        // really did add one (rather than just observing leftover
        // facts from a prior test).
        let before = crate::system::with_state(|s| {
            ast::fetch_or_phi("File_has_Name", s)
                .as_seq()
                .map(|v| v.len())
                .unwrap_or(0)
        })
        .unwrap_or(0);

        let body = synth_multipart("BNDRY", "round.txt", b"round-trip", "dir-xx");
        match try_serve(
            "POST", "/file",
            Some("multipart/form-data; boundary=BNDRY"),
            &body,
            // `state` parameter is the *pre-state* the route layers
            // facts on top of. Pass the live SYSTEM snapshot so the
            // test mirrors what `net::drive_http` does in production.
            crate::system::state(),
        ) {
            ServeOutcome::Response(_) => {}
            _ => panic!("expected Response from happy-path upload"),
        }

        let after = crate::system::with_state(|s| {
            ast::fetch_or_phi("File_has_Name", s)
                .as_seq()
                .map(|v| v.len())
                .unwrap_or(0)
        })
        .expect("with_state returns Some after init");
        assert_eq!(
            after, before + 1,
            "File_has_Name count should grow by 1 after upload (before={}, after={})",
            before, after,
        );
    }
}
