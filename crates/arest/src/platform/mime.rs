// crates/arest/src/platform/mime.rs
//
// `detect_mime` — content-bytes → IANA-style MIME type, by magic-number
// sniffing (per AREST whitepaper §7.4 Platform-fn surface).
//
// ## Why this exists (and why it's pure-Rust)
//
// The File create / update flow in the filesystem epic (#397e) needs to
// auto-populate `MimeType` from the first chunk of a freshly-uploaded
// blob, *before* the user has a chance to mistype it. The Platform-fn
// surface is the right home for this:
//
//   - It runs at engine level (not in the kernel hot path), so we have
//     std + alloc available.
//   - Pulling in a real `tree_magic`-style dep is undesirable: it would
//     drag a multi-MB rules database, glob handling, and a transitive
//     graph that no other Platform fn currently needs. The failure modes
//     for *automatic* MIME detection on user-uploaded content are
//     bounded — guess wrong, the user overrides via the `MimeType`
//     field on update — so the cost/benefit favours an inlined
//     magic-number table over a library.
//   - The kernel build path never reaches this file: the whole
//     `platform` module is `cfg(not(feature = "no_std"))`-gated by
//     `mod.rs`, identical to `zip.rs` (#404). When `arest-kernel`
//     pulls `arest` as a dep with `default-features = false` and
//     `no_std`, this file is excluded from the build entirely, so
//     `String` / `Vec` / `format!` paths are safe.
//
// ## Coverage
//
// The cases below hit the "common-uploads" spread the filesystem epic
// will see in production: image attachments (PNG/JPEG/GIF), PDFs, ZIPs
// (incl. OOXML / docx / xlsx / pptx), gzip, executables (PE / ELF),
// WebAssembly, and structured text (JSON / XML / HTML / plain). When
// nothing matches, `application/octet-stream` is the IANA fallback —
// File update flows treat that as "user must specify" rather than
// silent corruption of MimeType.
//
// ## Why no `apply_platform` adapter (yet)
//
// `zip.rs` registers itself into `PLATFORM_FALLBACK` via an `install()`
// fn; that's the right shape when the operand is a structured value
// (a directory id, a (file_id, target) pair). For `detect_mime`, the
// natural operand is the raw byte-slice that lives inside a `File`'s
// `ContentRef` — and that decode/dispatch glue belongs in the File
// create/update command handler (the consumer in #397e), not here. The
// adapter can be added once the consumer exists; until then, callers
// in arest-cli, the HTTP handler, and the test harness reach this fn
// directly via `crate::platform::mime::detect_mime`.

#![cfg(not(feature = "no_std"))]

/// Inspect `content` (typically the first ~4 KiB of a file upload) and
/// return the most plausible IANA MIME type as a `&'static str`.
///
/// The table is ordered most-specific first: signatures with longer or
/// rarer magic numbers (PNG's 8-byte header, gzip's 3-byte header)
/// take precedence over short ASCII heuristics (`{` for JSON, `<` for
/// XML/HTML). When nothing matches, `application/octet-stream` is the
/// IANA-blessed catch-all.
///
/// Returning `&'static str` — not `String` — lets callers stash the
/// result in a `Cow<'static, str>` or copy into a heap atom without
/// the codec needing an `alloc::string::String` round-trip on the hot
/// path.
pub fn detect_mime(content: &[u8]) -> &'static str {
    // Empty content is its own case: don't claim any structured type.
    if content.is_empty() {
        return "application/octet-stream";
    }

    // ── Binary signatures (longest / rarest first) ──────────────────

    // PNG: ‰PNG\r\n\x1a\n
    if content.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return "image/png";
    }
    // ELF: \x7fELF
    if content.starts_with(&[0x7F, 0x45, 0x4C, 0x46]) {
        return "application/x-elf";
    }
    // WebAssembly: \0asm
    if content.starts_with(&[0x00, 0x61, 0x73, 0x6D]) {
        return "application/wasm";
    }
    // PDF: %PDF
    if content.starts_with(&[0x25, 0x50, 0x44, 0x46]) {
        return "application/pdf";
    }
    // GIF: GIF87a / GIF89a
    if content.starts_with(b"GIF87a") || content.starts_with(b"GIF89a") {
        return "image/gif";
    }
    // ZIP family — also covers OOXML (.docx / .xlsx / .pptx), .jar,
    // .odt, .epub. Look one layer deeper to disambiguate the most
    // common OOXML cases by inspecting the first stored entry's name
    // (it always sits at LFH offset +30; encode_zip_stored in zip.rs
    // produces the same shape on the wire).
    if content.starts_with(&[0x50, 0x4B, 0x03, 0x04]) {
        return classify_zip(content);
    }
    // gzip: 1F 8B 08 (08 = deflate, the only method gzip ever uses
    // in practice). The 4th byte (flags) is variable so don't match it.
    if content.starts_with(&[0x1F, 0x8B, 0x08]) {
        return "application/gzip";
    }
    // JPEG: FF D8 FF — the 4th byte is one of {E0, E1, E2, E3, E8, DB, EE}
    // depending on the segment marker; ignoring it keeps the matcher
    // tolerant of non-JFIF / non-Exif variants (e.g. SPIFF).
    if content.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return "image/jpeg";
    }
    // PE/COFF (.exe / .dll / .efi): 'MZ' DOS-stub. We don't currently
    // distinguish between PE32 / PE32+ / .efi here — the consumer can
    // crack the COFF header for a sharper type if it cares.
    if content.starts_with(&[0x4D, 0x5A]) {
        return "application/x-msdownload";
    }

    // ── Text family ────────────────────────────────────────────────
    //
    // Strip a UTF-8 BOM (EF BB BF) before classifying — a BOM-prefixed
    // JSON or HTML file is still JSON / HTML.
    let body = if content.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &content[3..]
    } else {
        content
    };

    // Find the first non-whitespace byte to look at; everything we
    // care about (`<`, `{`, `[`) is ASCII, so a byte-level skip is
    // enough. We bound the skip to keep the scan O(N) regardless of
    // pathological leading whitespace.
    let trimmed = trim_leading_whitespace(body);

    if let Some(&first) = trimmed.first() {
        // JSON: leading `{` (object) or `[` (array). The full grammar
        // also allows a bare top-level scalar (RFC 8259), but those are
        // ambiguous with arbitrary text and not worth the false-positive
        // risk on auto-detect.
        if (first == b'{' || first == b'[') && looks_like_text(content) {
            return "application/json";
        }

        // XML / HTML: leading `<`. Inspect a small window to disambiguate.
        if first == b'<' {
            let window_end = (trimmed.len()).min(512);
            let window = &trimmed[..window_end];
            // Case-insensitive needles for HTML; XML `<?xml` is case-
            // sensitive per the spec but we accept either form.
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
            // Some other angle-bracket-led document — assume XML
            // (covers SVG, RSS, Atom, plain XML payloads).
            if looks_like_text(content) {
                return "application/xml";
            }
        }
    }

    // Fully ASCII / valid UTF-8 with no binary control chars → text/plain.
    if looks_like_text(content) {
        return "text/plain";
    }

    "application/octet-stream"
}

// ── helpers ─────────────────────────────────────────────────────────

/// Skip ASCII whitespace (space, tab, CR, LF) at the front of `bytes`.
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

/// True iff every byte in `content` is consistent with text — printable
/// ASCII plus the common whitespace controls. Forbids NUL, BEL, BS,
/// SO/SI, and the rest of the C0 control range (except \t \n \r \f).
/// The check looks only at the first 1024 bytes, which is enough for
/// the auto-detect use case (callers pass in a sniff window already).
fn looks_like_text(content: &[u8]) -> bool {
    let n = content.len().min(1024);
    if n == 0 {
        return false;
    }
    for &b in &content[..n] {
        match b {
            // Common text whitespace.
            b'\t' | b'\n' | b'\r' | 0x0C => {}
            // Printable ASCII.
            0x20..=0x7E => {}
            // High-bit bytes — accept them; valid UTF-8 lives here, and
            // mis-classifying valid UTF-8 as octet-stream is the worse
            // failure mode for the File create flow.
            0x80..=0xFF => {}
            // C0 controls and DEL → not text.
            _ => return false,
        }
    }
    true
}

/// True iff `haystack` starts with `needle`, comparing the ASCII
/// letters case-insensitively. Non-letter bytes must match exactly.
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

/// True iff `haystack` contains `needle` (case-insensitive ASCII). The
/// scan is O(n*m) which is fine for the small windows we feed in
/// (≤512 bytes, needles ≤14 bytes); this avoids pulling in a
/// search-algorithm crate for a single-call site.
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

/// Distinguish between a vanilla ZIP and an OOXML / EPUB / JAR archive
/// by peeking at the first stored entry's filename. Word, Excel, and
/// PowerPoint files always have `[Content_Types].xml` as the first
/// entry, with a body that names the part type; EPUB has `mimetype`
/// (uncompressed, exactly the IANA string); JAR has `META-INF/`.
///
/// This is a heuristic — false negatives just fall through to
/// `application/zip`, which is exactly what the file genuinely is at
/// that byte level. False positives are unlikely because the names we
/// match are conventional + deeply established.
fn classify_zip(content: &[u8]) -> &'static str {
    // LFH layout: signature (4) + 22 bytes + name_len (2) + extra_len (2) + name.
    if content.len() < 30 {
        return "application/zip";
    }
    let name_len = u16::from_le_bytes([content[26], content[27]]) as usize;
    let extra_len = u16::from_le_bytes([content[28], content[29]]) as usize;
    if 30 + name_len > content.len() {
        return "application/zip";
    }
    let name = &content[30..30 + name_len];

    // EPUB: first entry is literally "mimetype" (stored, uncompressed)
    // and its body is the IANA string.
    if name == b"mimetype" {
        let data_off = 30 + name_len + extra_len;
        let body_end = (data_off + 64).min(content.len());
        if data_off < content.len() {
            let body = &content[data_off..body_end];
            if body.starts_with(b"application/epub+zip") {
                return "application/epub+zip";
            }
        }
        return "application/zip";
    }

    // OOXML: distinguish docx / xlsx / pptx by sniffing for the
    // canonical part-type strings inside the first 2 KiB of the
    // archive. The `[Content_Types].xml` blob lists the part types
    // verbatim; this is the lowest-cost way to pick the right MIME
    // without unzipping.
    if name == b"[Content_Types].xml" {
        let window_end = content.len().min(2048);
        let window = &content[..window_end];
        if contains_ci(window, b"wordprocessingml") {
            return "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
        }
        if contains_ci(window, b"spreadsheetml") {
            return "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
        }
        if contains_ci(window, b"presentationml") {
            return "application/vnd.openxmlformats-officedocument.presentationml.presentation";
        }
        return "application/zip";
    }

    // JAR: first entry is `META-INF/MANIFEST.MF` (or the META-INF
    // directory marker). Most-specific case first.
    if name.starts_with(b"META-INF/") {
        return "application/java-archive";
    }

    "application/zip"
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal stored-only ZIP with a single entry. Mirrors the
    /// on-wire shape produced by `platform::zip::encode_zip_stored` —
    /// kept inline to avoid a cross-module test dep and to make this
    /// table self-contained.
    fn synth_zip(entry_name: &[u8], body: &[u8]) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        // Local file header.
        out.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]); // signature
        out.extend_from_slice(&[20, 0]); // version
        out.extend_from_slice(&[0, 0]);  // flags
        out.extend_from_slice(&[0, 0]);  // method (stored)
        out.extend_from_slice(&[0, 0]);  // mod time
        out.extend_from_slice(&[0, 0]);  // mod date
        out.extend_from_slice(&[0, 0, 0, 0]); // crc
        out.extend_from_slice(&(body.len() as u32).to_le_bytes()); // comp size
        out.extend_from_slice(&(body.len() as u32).to_le_bytes()); // uncomp size
        out.extend_from_slice(&(entry_name.len() as u16).to_le_bytes());
        out.extend_from_slice(&[0, 0]); // extra len
        out.extend_from_slice(entry_name);
        out.extend_from_slice(body);
        out
    }

    /// Spec table: for every common upload type, assert detect_mime
    /// returns the expected IANA string. Documents the surface in one
    /// place.
    #[test]
    fn known_magic_numbers_classify_correctly() {
        // (description, content, expected)
        let cases: Vec<(&str, Vec<u8>, &'static str)> = vec![
            ("png", vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13], "image/png"),
            ("jpeg-jfif", vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F'], "image/jpeg"),
            ("jpeg-exif", vec![0xFF, 0xD8, 0xFF, 0xE1, 0, 0x10, b'E', b'x', b'i', b'f'], "image/jpeg"),
            ("gif87a", b"GIF87a\x10\x00\x10\x00".to_vec(), "image/gif"),
            ("gif89a", b"GIF89a\x10\x00\x10\x00".to_vec(), "image/gif"),
            ("pdf", b"%PDF-1.7\n".to_vec(), "application/pdf"),
            ("elf", vec![0x7F, b'E', b'L', b'F', 2, 1, 1, 0], "application/x-elf"),
            ("pe-mz", vec![0x4D, 0x5A, 0x90, 0x00], "application/x-msdownload"),
            ("wasm", vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00], "application/wasm"),
            ("gzip", vec![0x1F, 0x8B, 0x08, 0x00], "application/gzip"),
            ("text-ascii", b"hello, world!\n".to_vec(), "text/plain"),
            ("text-utf8-bom", {
                let mut v = vec![0xEF, 0xBB, 0xBF];
                v.extend_from_slice(b"hello\n");
                v
            }, "text/plain"),
            ("json-object", b"{\"key\": 42}\n".to_vec(), "application/json"),
            ("json-array", b"[1, 2, 3]".to_vec(), "application/json"),
            ("json-with-leading-ws", b"   \n  {\"k\": 1}".to_vec(), "application/json"),
            ("html-doctype", b"<!DOCTYPE html><html><body>x</body></html>".to_vec(), "text/html"),
            ("html-doctype-mixed-case", b"<!doctype HTML>\n<HTML></HTML>".to_vec(), "text/html"),
            ("html-no-doctype", b"<html><head></head></html>".to_vec(), "text/html"),
            ("xml-decl", b"<?xml version=\"1.0\"?><root/>".to_vec(), "application/xml"),
            ("svg-no-decl", b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec(), "application/xml"),
            ("octet-stream-empty", vec![], "application/octet-stream"),
            ("octet-stream-binary-noise", vec![0x00, 0x01, 0x02, 0x03, 0x05, 0x07, 0x08], "application/octet-stream"),
        ];

        for (desc, bytes, expected) in cases {
            let got = detect_mime(&bytes);
            assert_eq!(got, expected, "case {} expected {} got {}", desc, expected, got);
        }
    }

    #[test]
    fn zip_plain_archive() {
        let archive = synth_zip(b"hello.txt", b"hello\n");
        assert_eq!(detect_mime(&archive), "application/zip");
    }

    #[test]
    fn zip_ooxml_docx() {
        // [Content_Types].xml with a wordprocessingml part-type mention.
        let body = br#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;
        let archive = synth_zip(b"[Content_Types].xml", body);
        assert_eq!(
            detect_mime(&archive),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
    }

    #[test]
    fn zip_ooxml_xlsx() {
        let body = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Override ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
</Types>"#;
        let archive = synth_zip(b"[Content_Types].xml", body);
        assert_eq!(
            detect_mime(&archive),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        );
    }

    #[test]
    fn zip_ooxml_pptx() {
        let body = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Override ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
</Types>"#;
        let archive = synth_zip(b"[Content_Types].xml", body);
        assert_eq!(
            detect_mime(&archive),
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        );
    }

    #[test]
    fn zip_epub() {
        let archive = synth_zip(b"mimetype", b"application/epub+zip");
        assert_eq!(detect_mime(&archive), "application/epub+zip");
    }

    #[test]
    fn zip_jar() {
        let archive = synth_zip(b"META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\n");
        assert_eq!(detect_mime(&archive), "application/java-archive");
    }

    #[test]
    fn html_with_leading_whitespace() {
        // HTML files commonly have leading newlines or BOMs.
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"\n  <html><body></body></html>");
        assert_eq!(detect_mime(&bytes), "text/html");
    }

    #[test]
    fn json_with_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"{\"x\": 1}");
        assert_eq!(detect_mime(&bytes), "application/json");
    }

    #[test]
    fn binary_with_text_prefix_is_text() {
        // High-bit bytes are accepted as UTF-8 continuation; pure
        // printable ASCII + UTF-8 should classify as text.
        let bytes = "héllo wörld".as_bytes().to_vec();
        assert_eq!(detect_mime(&bytes), "text/plain");
    }

    #[test]
    fn nul_byte_in_first_kib_blocks_text() {
        // A NUL byte in a "would-be-text" stream pushes it to octet-stream.
        let mut bytes = b"hello".to_vec();
        bytes.push(0x00);
        bytes.extend_from_slice(b" world");
        assert_eq!(detect_mime(&bytes), "application/octet-stream");
    }

    #[test]
    fn returned_str_is_static() {
        // Compile-time guarantee that the lifetime is 'static — useful
        // for callers that want to embed the return value in a
        // long-lived enum or Cow.
        let s: &'static str = detect_mime(b"%PDF-1.4");
        assert_eq!(s, "application/pdf");
    }
}
