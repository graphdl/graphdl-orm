// crates/arest-kernel/src/file_serve.rs
//
// HTTP `GET|HEAD /file/{id}/content` route (#403). Reads a File noun
// out of the kernel's baked SYSTEM state, decodes its `ContentRef`,
// and streams the bytes back over HTTP/1.1 with the right
// `Content-Type` (sourced from `File_has_MimeType`) and optional
// `Content-Range` for resumable downloads.
//
// Why a separate module: the route returns *raw* file bytes, so the
// canonical `http::Response` (whose `Content-Type` is
// `&'static str` and whose `to_wire()` always appends a fixed header
// set) doesn't fit. Instead, this module produces fully-serialised
// HTTP/1.1 wire bytes and `net::drive_http` writes them straight to
// the TCP send ring, bypassing the normal `Handler` chain.
//
// Two ContentRef shapes are supported (per readings/os/filesystem.md
// + #401):
//
//   * Inline path — bare lowercase hex atom OR the tagged form
//     `<INLINE, "hex-bytes">`. Decoded straight to bytes in memory.
//   * Region path — tagged form `<REGION, "base-sector", "byte-len">`.
//     Reads sector-by-sector via `block_storage::reserve_region` +
//     `RegionHandle::read`, then trims to `byte_len`.
//
// Range support is single-range only — `Range: bytes=N-M` and
// `Range: bytes=N-`. Multi-range requests, suffix-byte ranges
// (`bytes=-N`), and ranges that fall outside `[0, total_len)`
// return `416 Range Not Satisfiable` with a `Content-Range: bytes
// */{total}` header per RFC 7233 §4.4.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use arest::ast::{self, Object};

use crate::block_storage::{self, RegionHandle};
use crate::block::BLOCK_SECTOR_SIZE;

/// HTTP method that may produce a body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    Get,
    Head,
}

/// Decoded `ContentRef` value. Mirrors the two-variant shape declared
/// in `readings/os/filesystem.md`.
#[derive(Debug, Clone)]
enum ContentRef {
    /// Inline blob. Owns the raw byte buffer.
    Inline(Vec<u8>),
    /// Region-backed blob — base sector on the persistence disk plus
    /// a byte length (not necessarily a multiple of `SECTOR_SIZE`).
    Region { base_sector: u64, byte_len: u64 },
}

/// Outcome of `try_serve` — either a response was produced (caller
/// writes its `wire` bytes directly to the socket) or the path/method
/// did not match this module's responsibility (caller falls through
/// to its normal handler dispatch).
pub enum ServeOutcome {
    /// Wire-formatted HTTP/1.1 response, ready to push into the TCP
    /// send ring as-is. Includes status line, headers, and body.
    Response(Vec<u8>),
    /// Path is not `/file/{id}/content` or method is neither GET nor
    /// HEAD — defer to the existing handler.
    NotApplicable,
}

/// Top-level entry. Inspect the parsed request and, if it targets
/// `/file/{id}/content`, return a fully-serialised HTTP/1.1 response
/// (success, 404, 405, 416, or 500). Otherwise return `NotApplicable`
/// so the caller can route through the normal handler path.
///
/// `range_header` is the raw value of the `Range` request header
/// (e.g. `Some("bytes=0-1023")`) — `None` when absent.
pub fn try_serve(
    method: &str,
    path: &str,
    range_header: Option<&str>,
    state: Option<&Object>,
) -> ServeOutcome {
    // Strip a `?query` suffix so a future link generator that adds
    // cache-busters still routes here.
    let path = path.split('?').next().unwrap_or(path);

    let file_id = match parse_file_content_path(path) {
        Some(id) => id,
        None => return ServeOutcome::NotApplicable,
    };

    let m = match method {
        "GET" => Method::Get,
        "HEAD" => Method::Head,
        // Path matched but method is wrong — that's a 405, not a
        // pass-through. Surface it here so the assets / dispatch
        // chain doesn't re-claim a /file/.../content URL.
        _ => return ServeOutcome::Response(method_not_allowed()),
    };

    let state = match state {
        Some(s) => s,
        // No baked SYSTEM yet — treat it as an empty file table so
        // the route surfaces a real 404 rather than a 500.
        None => return ServeOutcome::Response(not_found(file_id)),
    };

    let mime = match lookup_mime(file_id, state) {
        Some(s) => s,
        // No File found at all — 404. We could differentiate
        // missing-mime vs missing-file but the caller can't tell
        // them apart from outside, so collapse both into 404.
        None => return ServeOutcome::Response(not_found(file_id)),
    };
    let cref_atom = match lookup_content_ref(file_id, state) {
        Some(s) => s,
        None => return ServeOutcome::Response(not_found(file_id)),
    };
    let content = match decode_content_ref(&cref_atom) {
        Some(c) => c,
        None => return ServeOutcome::Response(internal_error(
            "malformed ContentRef",
        )),
    };

    let total_len = content.byte_len();

    // Parse Range. None header → full body. Bad header that doesn't
    // parse → ignore it per RFC 7233 §3.1 ("a recipient MUST ignore
    // a Range header that is unsatisfiable on its own"). Out-of-
    // bounds range → 416. Multi-range → 416.
    let range = match range_header {
        Some(h) => match parse_range_header(h, total_len) {
            ParsedRange::Single(start, end) => Some((start, end)),
            ParsedRange::Unsatisfiable => {
                return ServeOutcome::Response(range_not_satisfiable(
                    &mime, total_len,
                ));
            }
            ParsedRange::Ignore => None,
        },
        None => None,
    };

    let (start, end) = range.unwrap_or((0, total_len.saturating_sub(1)));
    // Edge case: zero-length file with no Range header — return an
    // empty 200, not a 206 (start=0, end=u64::MAX after saturating
    // would overflow into the chunk). Detect explicitly.
    let is_partial = range.is_some();

    match read_range(&content, start, end) {
        Ok(body_bytes) => {
            let body = match m {
                Method::Get => body_bytes,
                Method::Head => Vec::new(),
            };
            ServeOutcome::Response(success_response(
                &mime, total_len, start, end, is_partial, body,
            ))
        }
        Err(()) => ServeOutcome::Response(internal_error(
            "block storage read failed",
        )),
    }
}

// ── Path parsing ────────────────────────────────────────────────────

/// Extract `{id}` from `/file/{id}/content`. Returns `None` for any
/// other path shape.
fn parse_file_content_path(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/file/")?;
    let id = rest.strip_suffix("/content")?;
    if id.is_empty() || id.contains('/') {
        return None;
    }
    Some(id)
}

// ── ContentRef lookup ──────────────────────────────────────────────

/// Look up `File_has_MimeType` for `file_id`. Mirrors the
/// `file_name` / `file_content_bytes` shape in `crates/arest/src/
/// platform/zip.rs` so engine-side and kernel-side code agree on how
/// File facts are addressed.
fn lookup_mime(file_id: &str, state: &Object) -> Option<String> {
    let cell = ast::fetch_or_phi("File_has_MimeType", state);
    cell.as_seq()?.iter().find_map(|fact| {
        if ast::binding(fact, "File") == Some(file_id) {
            ast::binding(fact, "MimeType").map(|s| s.to_string())
        } else {
            None
        }
    })
}

fn lookup_content_ref(file_id: &str, state: &Object) -> Option<String> {
    let cell = ast::fetch_or_phi("File_has_ContentRef", state);
    cell.as_seq()?.iter().find_map(|fact| {
        if ast::binding(fact, "File") == Some(file_id) {
            ast::binding(fact, "ContentRef").map(|s| s.to_string())
        } else {
            None
        }
    })
}

/// Decode a `ContentRef` atom into a `ContentRef`. Two shapes are
/// accepted:
///
///   * Tagged form (#401): `<INLINE,deadbeef>` /
///     `<REGION,8192,131072>` — the spec form that the encoder
///     switches to once `crates/arest/src/blob.rs` lands.
///   * Bare lowercase hex (today's encoder output, see zip.rs +
///     search.rs) — interpreted as inline bytes.
///
/// Returns `None` only for hex-decode errors (odd-length hex or
/// non-hex characters in the bytes payload). An empty atom decodes
/// to `Inline(Vec::new())` so a zero-byte File round-trips cleanly.
fn decode_content_ref(cref: &str) -> Option<ContentRef> {
    if let Some(inner) = strip_tagged(cref, "INLINE") {
        return decode_hex(inner).map(ContentRef::Inline);
    }
    if let Some(inner) = strip_tagged(cref, "REGION") {
        // Two comma-separated decimal atoms after the discriminant.
        let mut parts = inner.split(',');
        let base = parts.next()?.trim().parse::<u64>().ok()?;
        let len  = parts.next()?.trim().parse::<u64>().ok()?;
        if parts.next().is_some() {
            return None;
        }
        return Some(ContentRef::Region { base_sector: base, byte_len: len });
    }
    // Fallback: bare lowercase hex (today's zip.rs encoder output).
    decode_hex(cref).map(ContentRef::Inline)
}

/// Match `<TAG,...>` and return the substring between the comma after
/// `TAG` and the closing `>`. Returns `None` when the input does not
/// start with `<TAG,` or doesn't end with `>`.
fn strip_tagged<'a>(s: &'a str, tag: &str) -> Option<&'a str> {
    let s = s.strip_prefix('<')?;
    let s = s.strip_suffix('>')?;
    let rest = s.strip_prefix(tag)?;
    rest.strip_prefix(',')
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() { return Some(Vec::new()); }
    let bs = s.as_bytes();
    if bs.len() % 2 != 0 { return None; }
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

impl ContentRef {
    fn byte_len(&self) -> u64 {
        match self {
            ContentRef::Inline(b) => b.len() as u64,
            ContentRef::Region { byte_len, .. } => *byte_len,
        }
    }
}

// ── Range parsing ───────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
enum ParsedRange {
    /// Inclusive byte range — `(start, end_inclusive)`.
    Single(u64, u64),
    /// Header parsed but the requested range is outside `[0, total)`
    /// or otherwise unrepresentable. Caller emits 416.
    Unsatisfiable,
    /// Header didn't match a recognised form. Per RFC 7233 §3.1 we
    /// silently fall back to a full-body response.
    Ignore,
}

/// Parse a single-range `Range` header. Accepted shapes:
///
///   `bytes=N-M`  →  Single(N, M) when 0 ≤ N ≤ M < total
///   `bytes=N-`   →  Single(N, total - 1) when N < total
///
/// Multi-range (`bytes=0-499,1000-1499`) is rejected with
/// `Unsatisfiable` (we don't ship a multipart/byteranges body
/// builder). Suffix ranges (`bytes=-500`) are not implemented yet
/// and also yield `Unsatisfiable`. Anything that doesn't start
/// with `bytes=` returns `Ignore` so the caller falls back to the
/// full body.
fn parse_range_header(h: &str, total_len: u64) -> ParsedRange {
    let spec = match h.trim().strip_prefix("bytes=") {
        Some(s) => s.trim(),
        None => return ParsedRange::Ignore,
    };
    if spec.contains(',') {
        // Multi-range — explicit reject per the route's contract.
        return ParsedRange::Unsatisfiable;
    }
    let dash = match spec.find('-') {
        Some(i) => i,
        None => return ParsedRange::Ignore,
    };
    let head = &spec[..dash];
    let tail = &spec[dash + 1..];
    if head.is_empty() {
        // Suffix form `bytes=-N` — not supported yet.
        return ParsedRange::Unsatisfiable;
    }
    let start: u64 = match head.parse() {
        Ok(n) => n,
        Err(_) => return ParsedRange::Ignore,
    };
    if total_len == 0 {
        // Range against an empty file is always unsatisfiable per
        // RFC 7233 §4.4.
        return ParsedRange::Unsatisfiable;
    }
    if start >= total_len {
        return ParsedRange::Unsatisfiable;
    }
    let end: u64 = if tail.is_empty() {
        total_len - 1
    } else {
        match tail.parse() {
            Ok(n) => n,
            Err(_) => return ParsedRange::Ignore,
        }
    };
    if end >= total_len || end < start {
        return ParsedRange::Unsatisfiable;
    }
    ParsedRange::Single(start, end)
}

// ── Body materialisation ───────────────────────────────────────────

/// Read `[start..=end]` out of a `ContentRef`, in-memory or off-disk.
/// `start` and `end` are guaranteed by the caller to satisfy
/// `start <= end < total_len` (the parser already checks this).
///
/// For region-backed blobs we read whole sectors and then trim to the
/// requested window. Reading sector-by-sector keeps the temporary
/// buffer at `BLOCK_SECTOR_SIZE` bytes regardless of how big the
/// requested range is — important on the kernel's tiny static heap.
fn read_range(content: &ContentRef, start: u64, end: u64) -> Result<Vec<u8>, ()> {
    if start > end {
        // Defensive — the parser guarantees start<=end, but a future
        // caller could regress. Empty body is still wire-valid.
        return Ok(Vec::new());
    }
    if content.byte_len() == 0 {
        // Zero-byte file: any read returns an empty body. Bail before
        // trying to construct a `RegionHandle` on a zero-sector range
        // (which `reserve_region` rejects with `Error::OutOfRange`).
        return Ok(Vec::new());
    }
    let want = (end - start + 1) as usize;
    match content {
        ContentRef::Inline(bytes) => {
            let s = start as usize;
            let e = (end as usize).saturating_add(1).min(bytes.len());
            if s >= bytes.len() {
                return Ok(Vec::new());
            }
            Ok(bytes[s..e].to_vec())
        }
        ContentRef::Region { base_sector, byte_len } => {
            // Build a `RegionHandle` covering all sectors that hold
            // the file's bytes. `reserve_region` checks that the
            // range is inside the disk capacity and that the device
            // is mounted.
            let total_sectors = sector_span(*byte_len);
            let handle: RegionHandle =
                block_storage::reserve_region(*base_sector, total_sectors)
                    .map_err(|_| ())?;

            let first_sector = start / (BLOCK_SECTOR_SIZE as u64);
            let last_sector  = end / (BLOCK_SECTOR_SIZE as u64);
            // `want` here is the requested byte count; `Vec::with_capacity`
            // sizes the output buffer to skip realloc when the loop
            // copies sector slices in.
            let mut out: Vec<u8> = Vec::with_capacity(want);
            let mut sec_buf = [0u8; BLOCK_SECTOR_SIZE];

            for s in first_sector..=last_sector {
                handle.read(s, &mut sec_buf).map_err(|_| ())?;
                let sec_byte_start = s * (BLOCK_SECTOR_SIZE as u64);
                let off_lo = if s == first_sector {
                    (start - sec_byte_start) as usize
                } else {
                    0
                };
                let off_hi = if s == last_sector {
                    (end - sec_byte_start + 1) as usize
                } else {
                    BLOCK_SECTOR_SIZE
                };
                out.extend_from_slice(&sec_buf[off_lo..off_hi]);
            }
            Ok(out)
        }
    }
}

/// Number of 512-byte sectors needed to hold `byte_len` bytes,
/// rounding up. Mirrors the encoder's slot-sizing math; pulled out
/// so the test harness can pin it.
fn sector_span(byte_len: u64) -> u64 {
    let sec = BLOCK_SECTOR_SIZE as u64;
    (byte_len + sec - 1) / sec
}

// ── Wire-format builders ───────────────────────────────────────────

/// 200/206 success. `is_partial` selects the status line. `start`/`end`
/// are inclusive byte positions used to populate `Content-Range` on a
/// 206 response. `body` is empty for HEAD requests.
fn success_response(
    mime: &str,
    total_len: u64,
    start: u64,
    end: u64,
    is_partial: bool,
    body: Vec<u8>,
) -> Vec<u8> {
    let (code, reason) = if is_partial {
        (206u16, "Partial Content")
    } else {
        (200u16, "OK")
    };
    let content_length = if is_partial {
        end - start + 1
    } else {
        total_len
    };
    let mut out = Vec::with_capacity(192 + body.len());
    push_status(&mut out, code, reason);
    push_header(&mut out, "Content-Type", mime);
    push_header(&mut out, "Content-Length", &format!("{}", content_length));
    push_header(&mut out, "Accept-Ranges", "bytes");
    if is_partial {
        push_header(
            &mut out,
            "Content-Range",
            &format!("bytes {}-{}/{}", start, end, total_len),
        );
    }
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&body);
    out
}

/// 416 Range Not Satisfiable per RFC 7233 §4.4. Body is the standard
/// `bytes */{total}` `Content-Range` header.
fn range_not_satisfiable(mime: &str, total_len: u64) -> Vec<u8> {
    let body = b"requested range not satisfiable\n";
    let mut out = Vec::with_capacity(192 + body.len());
    push_status(&mut out, 416, "Range Not Satisfiable");
    push_header(&mut out, "Content-Type", "text/plain; charset=utf-8");
    push_header(&mut out, "Content-Length", &format!("{}", body.len()));
    push_header(
        &mut out,
        "Content-Range",
        &format!("bytes */{}", total_len),
    );
    // Echo the resource MIME so a client that retries with a valid
    // range can re-validate cache headers without an extra HEAD.
    push_header(&mut out, "X-File-Content-Type", mime);
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(body);
    out
}

fn not_found(file_id: &str) -> Vec<u8> {
    let body = format!("file {} not found\n", file_id).into_bytes();
    let mut out = Vec::with_capacity(96 + body.len());
    push_status(&mut out, 404, "Not Found");
    push_header(&mut out, "Content-Type", "text/plain; charset=utf-8");
    push_header(&mut out, "Content-Length", &format!("{}", body.len()));
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&body);
    out
}

fn method_not_allowed() -> Vec<u8> {
    let body = b"only GET and HEAD are supported on /file/{id}/content\n";
    let mut out = Vec::with_capacity(128 + body.len());
    push_status(&mut out, 405, "Method Not Allowed");
    push_header(&mut out, "Allow", "GET, HEAD");
    push_header(&mut out, "Content-Type", "text/plain; charset=utf-8");
    push_header(&mut out, "Content-Length", &format!("{}", body.len()));
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(body);
    out
}

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

// ── Range-header extraction (raw request bytes) ─────────────────────

/// Look up the `Range` header value (case-insensitive) in a buffered
/// HTTP/1.1 request. Used by `net::drive_http` because the canonical
/// `http::parse_request` only captures `Content-Length` and `Accept`.
///
/// Returns `Some(value)` with leading/trailing whitespace stripped on
/// match, `None` when absent or when the header block hasn't been
/// fully received (we never reach this path with a partial buffer
/// because dispatch only fires after `parse_request` returns
/// `Ok(Some(_))`, but the function is defensive against a torn-line
/// regression).
pub fn extract_range_header(buf: &[u8]) -> Option<String> {
    let header_end = find_double_crlf(buf)?;
    let head = core::str::from_utf8(&buf[..header_end]).ok()?;
    for line in head.split("\r\n") {
        if line.is_empty() { continue; }
        let colon = match line.find(':') {
            Some(i) => i,
            None => continue,
        };
        let name = line[..colon].trim();
        if name.eq_ignore_ascii_case("range") {
            return Some(line[colon + 1..].trim().to_string());
        }
    }
    None
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use arest::ast::fact_from_pairs;

    fn build_state_with_file(id: &str, mime: &str, cref: &str) -> Object {
        let phi = Object::phi();
        let d = ast::cell_push(
            "File_has_MimeType",
            fact_from_pairs(&[("File", id), ("MimeType", mime)]),
            &phi,
        );
        ast::cell_push(
            "File_has_ContentRef",
            fact_from_pairs(&[("File", id), ("ContentRef", cref)]),
            &d,
        )
    }

    #[test]
    fn parse_path_extracts_id() {
        assert_eq!(parse_file_content_path("/file/abc/content"), Some("abc"));
        assert_eq!(
            parse_file_content_path("/file/file-zip-1/content"),
            Some("file-zip-1"),
        );
    }

    #[test]
    fn parse_path_rejects_other_shapes() {
        assert_eq!(parse_file_content_path("/file/abc"), None);
        assert_eq!(parse_file_content_path("/file//content"), None);
        assert_eq!(parse_file_content_path("/file/abc/content/extra"), None);
        assert_eq!(parse_file_content_path("/files/abc/content"), None);
        assert_eq!(parse_file_content_path("/api/file/abc/content"), None);
    }

    #[test]
    fn decode_inline_tagged() {
        let r = decode_content_ref("<INLINE,deadbeef>").unwrap();
        match r {
            ContentRef::Inline(b) => assert_eq!(b, vec![0xde, 0xad, 0xbe, 0xef]),
            _ => panic!("expected Inline"),
        }
    }

    #[test]
    fn decode_inline_bare_hex() {
        let r = decode_content_ref("48656c6c6f").unwrap();
        match r {
            ContentRef::Inline(b) => assert_eq!(b, b"Hello"),
            _ => panic!("expected Inline"),
        }
    }

    #[test]
    fn decode_region_tagged() {
        let r = decode_content_ref("<REGION,8192,131072>").unwrap();
        match r {
            ContentRef::Region { base_sector, byte_len } => {
                assert_eq!(base_sector, 8192);
                assert_eq!(byte_len, 131072);
            }
            _ => panic!("expected Region"),
        }
    }

    #[test]
    fn decode_rejects_bad_hex() {
        assert!(decode_content_ref("xyz").is_none());
        // Odd length.
        assert!(decode_content_ref("abc").is_none());
    }

    #[test]
    fn decode_empty_is_zero_byte_inline() {
        let r = decode_content_ref("").unwrap();
        match r {
            ContentRef::Inline(b) => assert!(b.is_empty()),
            _ => panic!("expected Inline"),
        }
    }

    #[test]
    fn range_full_form() {
        assert_eq!(
            parse_range_header("bytes=0-499", 1000),
            ParsedRange::Single(0, 499),
        );
        assert_eq!(
            parse_range_header("bytes=500-999", 1000),
            ParsedRange::Single(500, 999),
        );
    }

    #[test]
    fn range_open_ended() {
        assert_eq!(
            parse_range_header("bytes=500-", 1000),
            ParsedRange::Single(500, 999),
        );
    }

    #[test]
    fn range_unsatisfiable_out_of_bounds() {
        assert_eq!(
            parse_range_header("bytes=1000-1500", 1000),
            ParsedRange::Unsatisfiable,
        );
        assert_eq!(
            parse_range_header("bytes=500-1500", 1000),
            ParsedRange::Unsatisfiable,
        );
        assert_eq!(
            parse_range_header("bytes=600-500", 1000),
            ParsedRange::Unsatisfiable,
        );
    }

    #[test]
    fn range_multi_rejected() {
        assert_eq!(
            parse_range_header("bytes=0-499,600-999", 1000),
            ParsedRange::Unsatisfiable,
        );
    }

    #[test]
    fn range_suffix_unsupported() {
        assert_eq!(
            parse_range_header("bytes=-500", 1000),
            ParsedRange::Unsatisfiable,
        );
    }

    #[test]
    fn range_unrecognised_ignored() {
        assert_eq!(parse_range_header("seconds=0-10", 1000), ParsedRange::Ignore);
    }

    #[test]
    fn range_against_empty_file_unsatisfiable() {
        assert_eq!(parse_range_header("bytes=0-0", 0), ParsedRange::Unsatisfiable);
    }

    #[test]
    fn read_inline_full() {
        let c = ContentRef::Inline(b"Hello, world!".to_vec());
        let out = read_range(&c, 0, 12).unwrap();
        assert_eq!(out, b"Hello, world!");
    }

    #[test]
    fn read_inline_window() {
        let c = ContentRef::Inline(b"Hello, world!".to_vec());
        // "world"
        let out = read_range(&c, 7, 11).unwrap();
        assert_eq!(out, b"world");
    }

    #[test]
    fn extract_range_present() {
        let req = b"GET /file/abc/content HTTP/1.1\r\n\
                    Host: arest\r\n\
                    Range: bytes=0-99\r\n\
                    \r\n";
        assert_eq!(extract_range_header(req).as_deref(), Some("bytes=0-99"));
    }

    #[test]
    fn extract_range_case_insensitive() {
        let req = b"GET /file/abc/content HTTP/1.1\r\n\
                    Host: arest\r\n\
                    range: bytes=10-20\r\n\
                    \r\n";
        assert_eq!(extract_range_header(req).as_deref(), Some("bytes=10-20"));
    }

    #[test]
    fn extract_range_absent() {
        let req = b"GET /file/abc/content HTTP/1.1\r\nHost: arest\r\n\r\n";
        assert!(extract_range_header(req).is_none());
    }

    #[test]
    fn try_serve_get_inline() {
        let state = build_state_with_file("a", "text/plain", "48656c6c6f");
        let out = try_serve("GET", "/file/a/content", None, Some(&state));
        match out {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 200 OK\r\n"), "status: {}", s);
                assert!(s.contains("Content-Type: text/plain"));
                assert!(s.contains("Content-Length: 5"));
                assert!(s.contains("Accept-Ranges: bytes"));
                assert!(bytes.ends_with(b"Hello"));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_head_omits_body() {
        let state = build_state_with_file("a", "text/plain", "48656c6c6f");
        let out = try_serve("HEAD", "/file/a/content", None, Some(&state));
        match out {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
                // HEAD reports the same Content-Length as GET would.
                assert!(s.contains("Content-Length: 5"));
                // …but no body bytes after the header terminator.
                let body_start = s.find("\r\n\r\n").unwrap() + 4;
                assert_eq!(&bytes[body_start..], &[] as &[u8]);
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_range_returns_206() {
        // 13-byte payload "Hello, world!"
        let cref = "48656c6c6f2c20776f726c6421";
        let state = build_state_with_file("a", "text/plain", cref);
        let out = try_serve("GET", "/file/a/content", Some("bytes=7-11"), Some(&state));
        match out {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(
                    s.starts_with("HTTP/1.1 206 Partial Content\r\n"),
                    "status: {}", s,
                );
                assert!(s.contains("Content-Length: 5"));
                assert!(s.contains("Content-Range: bytes 7-11/13"));
                assert!(bytes.ends_with(b"world"));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_range_unsatisfiable_returns_416() {
        let cref = "48656c6c6f2c20776f726c6421";
        let state = build_state_with_file("a", "text/plain", cref);
        let out = try_serve(
            "GET", "/file/a/content", Some("bytes=20-30"), Some(&state),
        );
        match out {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(
                    s.starts_with("HTTP/1.1 416 Range Not Satisfiable\r\n"),
                    "status: {}", s,
                );
                assert!(s.contains("Content-Range: bytes */13"));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_unknown_file_returns_404() {
        let state = build_state_with_file("a", "text/plain", "00");
        let out = try_serve("GET", "/file/missing/content", None, Some(&state));
        match out {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 404 Not Found\r\n"));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_wrong_method_405() {
        let state = build_state_with_file("a", "text/plain", "00");
        let out = try_serve("POST", "/file/a/content", None, Some(&state));
        match out {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"));
                assert!(s.contains("Allow: GET, HEAD"));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_other_path_passes_through() {
        let state = build_state_with_file("a", "text/plain", "00");
        match try_serve("GET", "/api/welcome", None, Some(&state)) {
            ServeOutcome::NotApplicable => {}
            _ => panic!("expected NotApplicable"),
        }
    }

    #[test]
    fn sector_span_rounds_up() {
        assert_eq!(sector_span(0), 0);
        assert_eq!(sector_span(1), 1);
        assert_eq!(sector_span(BLOCK_SECTOR_SIZE as u64), 1);
        assert_eq!(sector_span((BLOCK_SECTOR_SIZE as u64) + 1), 2);
        assert_eq!(sector_span(131072), 256);
    }
}
