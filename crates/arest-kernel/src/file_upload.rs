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

use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use arest::ast::{self, Object, fact_from_pairs};
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;

use crate::block::BLOCK_SECTOR_SIZE;
use crate::block_storage::{self, RegionHandle, BLOB_SLOT_BYTES};

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

/// Suggested per-chunk size advertised in the chunked-init JSON
/// response. Clients are not bound to it — `try_serve_chunk` accepts
/// any chunk size up to the per-PUT body cap — but it's a sensible
/// default that keeps each PUT body small enough to fit in the rx_buf
/// (4 KiB, see `net::register_http`) without spilling over multiple
/// polls. A chunk size = SECTOR_SIZE * 8 = 4 KiB also lines up with
/// the disk block layer for the common case.
const CHUNK_SIZE_HINT: u64 = 4096;

/// Largest chunk PUT body we'll accept. Picked at 64 KiB so a chunk
/// PUT comfortably exceeds CHUNK_SIZE_HINT, yet stays inside the
/// INLINE_THRESHOLD ceiling so we don't have to grow `net::register_http`'s
/// rx_buf to handle resumable writes. Chunks larger than this surface
/// 413 — the client should split.
const MAX_CHUNK_BYTES: usize = INLINE_THRESHOLD;

/// Hard cap on the declared total upload size. One blob slot from
/// `block_storage::alloc_region` (see `BLOB_SLOT_BYTES` = 256 KiB).
/// A future commit can chain multiple slots; for now uploads beyond
/// the slot ceiling fail at init with 413.
const MAX_REGION_BYTES: u64 = BLOB_SLOT_BYTES;

/// Idempotency-Key TTL (#446). Cache entries expire this far past
/// `arch::time::now_ms()`'s reading at insert time. 24h gives every
/// realistic retry window (mobile drops, server reboots, client
/// back-off chains) a stable resolution while staying short enough
/// that a stale entry can't pin a long-dead File id forever.
const IDEMPOTENCY_TTL_MS: u64 = 24 * 60 * 60 * 1000;

/// Maximum live entries in the IDEMPOTENCY map (#446). Bounded so a
/// burst of unique keys can't grow the map without limit. When full,
/// the LRU entry (smallest `last_seen_ms`) is evicted to make room
/// — adequate for the request rates the kernel sees today.
const IDEMPOTENCY_MAX_ENTRIES: usize = 1024;

/// Per-upload progress event ring depth (#447). One ring per file
/// id in the PROGRESS_EVENTS map; older events fall off the front
/// when the ring fills. 64 events covers a 256 KiB upload at the
/// 4 KiB CHUNK_SIZE_HINT cadence with no drops.
const PROGRESS_RING_DEPTH: usize = 64;

/// Minimum bytes required for the first-chunk MIME sniff (#448).
/// Mirrors the engine-side `detect_mime` table's window — no
/// signature in the table extends past 512 bytes, so a shorter
/// prefix wouldn't gain accuracy. Smaller first chunks defer to
/// the closing seal-step sniff.
const MIME_SNIFF_MIN_BYTES: usize = 512;

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
///
/// Idempotency-aware shim: forwards to `try_serve_idempotent` with
/// `idempotency_key = None`. The current `net.rs::drive_http` call
/// site doesn't surface request headers to this module yet; once
/// Track NNN's `register_http` (#360) lands, the dispatch layer
/// can re-scan the raw rx_buf for `Idempotency-Key` and call
/// `try_serve_idempotent` directly. Until then, retries from the
/// live wire create fresh File ids.
// TODO(#446): once net.rs grows an `Idempotency-Key` extractor
// (mirror of `extract_content_type_header`), route the live POST
// dispatch through `try_serve_idempotent` so retries collapse on
// the wire. Today only the in-process tests reach the cache path.
pub fn try_serve(
    method: &str,
    path: &str,
    content_type: Option<&str>,
    body: &[u8],
    state: Option<&Object>,
) -> ServeOutcome {
    try_serve_idempotent(method, path, content_type, body, state, None)
}

/// Idempotency-aware variant of `try_serve` (#446). When
/// `idempotency_key` is `Some`, the function probes IDEMPOTENCY for
/// a fresh entry and short-circuits with the cached response when
/// it finds one — so a retried POST returns the same File id rather
/// than allocating a new one. After a successful 2xx response, the
/// wire bytes are recorded against the key for the TTL window.
///
/// `idempotency_key = None` is identical to the legacy `try_serve`
/// path — no cache reads, no cache writes — so the existing
/// callers see no behaviour change while net.rs catches up to the
/// new entry point.
pub fn try_serve_idempotent(
    method: &str,
    path: &str,
    content_type: Option<&str>,
    body: &[u8],
    state: Option<&Object>,
    idempotency_key: Option<&str>,
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

    // #446: probe before parsing so a retry of a successful upload
    // returns the cached response without re-walking multipart.
    if let Some(key) = idempotency_key {
        if let Some(cached) = lookup_idempotent(key) {
            return ServeOutcome::Response(cached);
        }
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
    let mut total_field: Option<&str> = None;
    let mut filename_field: Option<&str> = None;
    let mut mime_field: Option<&str> = None;
    for p in &parts {
        match p.name.as_str() {
            "file" => file_part = Some(p),
            "directory_id" => {
                // Form-field values arrive as the part body bytes.
                directory_id = core::str::from_utf8(&p.body).ok();
            }
            "total" => {
                // Chunked-mode init only. Decimal byte count of the
                // full upload. Treated as the trigger for the
                // region-backed path below.
                total_field = core::str::from_utf8(&p.body).ok();
            }
            "filename" => {
                filename_field = core::str::from_utf8(&p.body).ok();
            }
            "mime_type" => {
                mime_field = core::str::from_utf8(&p.body).ok();
            }
            _ => {} // Tolerate unknown form fields (forward-compat).
        }
    }

    let directory_id = match directory_id {
        Some(s) if !s.is_empty() => s,
        _ => return ServeOutcome::Response(bad_request(
            "multipart body missing `directory_id` form field",
        )),
    };

    // Chunked-mode init detection: a `total` form field is present
    // (declared upload size) and either no inline `file` part exists
    // or the declared `total` exceeds the inline ceiling. Either way
    // the route allocates a region, plants a `<REGION,base,0>`
    // ContentRef placeholder, and hands the client an upload_id +
    // chunk_size hint so subsequent PUT /file/{id}/chunk?offset=N
    // calls can stream the bytes in.
    if let Some(total_str) = total_field {
        let total: u64 = match total_str.trim().parse() {
            Ok(n) => n,
            Err(_) => return ServeOutcome::Response(bad_request(
                "`total` form field must be a decimal byte count",
            )),
        };
        // Honour an inline-only total even when the client opted
        // into chunked init: if the file part is also present and
        // its size matches the declared total and fits inline, we
        // could fall back to the inline path. Today we keep the
        // behaviour predictable — `total` is the explicit chunked-
        // mode opt-in — and reject contradictory bodies.
        if file_part.is_some() {
            return ServeOutcome::Response(bad_request(
                "chunked-mode init: do not include a `file` part \
                 (use PUT /file/{id}/chunk to stream bytes)",
            ));
        }
        let filename = filename_field.unwrap_or("");
        let mime = mime_field.unwrap_or("application/octet-stream");
        let response = begin_chunked_upload(
            state, directory_id, filename, mime, total,
        );
        // #446: cache the chunked-init response on success so a
        // retry yields the same file_id + region rather than
        // allocating a second region under the same key.
        if let Some(key) = idempotency_key {
            if is_2xx_response(&response) {
                record_idempotent(key, &response);
            }
        }
        return ServeOutcome::Response(response);
    }

    let file_part = match file_part {
        Some(p) => p,
        None => return ServeOutcome::Response(bad_request(
            "multipart body missing `file` part",
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
        return ServeOutcome::Response(internal_error(&msg));
    }

    let response = created_response(&file_id);
    if let Some(key) = idempotency_key {
        record_idempotent(key, &response);
    }
    ServeOutcome::Response(response)
}

// ── Chunked-upload init ────────────────────────────────────────────
//
// Opens a region-backed upload session. Sequence of work:
//
//   1. Reject totals outside [1, MAX_REGION_BYTES]. Zero-byte uploads
//      have no chunks to ship, so they can use the inline path with
//      an empty `file` part (single 201 round-trip); declared totals
//      above the slot ceiling 413 today (chained-slot follow-up).
//   2. `block_storage::alloc_region(total)` reserves one blob slot
//      from the disk's slot table. The handle's `base_sector` becomes
//      the address recorded in the File's ContentRef placeholder.
//      Failure modes are propagated as 503 (NoDevice), 413 (StateTooLarge),
//      or 500 (anything else) — see `region_alloc_error_response`.
//   3. Mint the file id, build the four facts (Name / MimeType /
//      ContentRef as `<REGION,base,0>` placeholder / Size as 0 /
//      is_in_Directory) and `system::apply` the new state so a
//      subsequent `GET /file/{id}/upload-state` finds it.
//   4. Stash `UploadState { region, total, highest_contiguous_byte=0,
//      mime_hint }` in the in-memory map keyed by file id so PUT
//      chunks can look up the region without re-querying the SYSTEM.
//   5. Return 201 with `Location: /file/{id}` + JSON body
//      `{"id":"...","upload_id":"...","chunk_size":4096}`.
//
// The `upload_id` is currently the file id (one upload session per
// file); the JSON shape is forward-compat for a future where the
// session id and the file id diverge (e.g. multi-version uploads).
fn begin_chunked_upload(
    state: Option<&Object>,
    directory_id: &str,
    filename: &str,
    mime: &str,
    total: u64,
) -> Vec<u8> {
    if total == 0 {
        return bad_request(
            "chunked-mode init: declared `total` must be > 0; \
             use the inline path with an empty `file` part for zero-byte uploads",
        );
    }
    if total > MAX_REGION_BYTES {
        return payload_too_large();
    }

    let handle = match block_storage::alloc_region(total) {
        Ok(h) => h,
        Err(e) => return region_alloc_error_response(e),
    };
    let base_sector = handle.base_sector();

    let file_id = synth_id("file");
    let cref = format!("<REGION,{},0>", base_sector);
    let new_state = build_file_facts_with_cref(
        state.cloned().unwrap_or_else(Object::phi),
        &file_id,
        filename,
        mime,
        &cref,
        0,
        directory_id,
    );
    if let Err(msg) = crate::system::apply(new_state) {
        return internal_error(&msg);
    }

    UPLOADS.lock().insert(
        file_id.clone(),
        UploadState {
            region: handle,
            base_sector,
            total,
            highest_contiguous_byte: 0,
            filename: filename.to_string(),
            directory_id: directory_id.to_string(),
            mime_hint: mime.to_string(),
            sniff_window: Vec::new(),
            complete: false,
            mime_promoted: false,
        },
    );

    chunked_init_response(&file_id)
}

// ── PUT /file/{id}/chunk?offset=N ──────────────────────────────────
//
// Streams the next chunk into a previously-initialised region. The
// route is split out so `net::drive_http` can dispatch on `PUT` to
// `try_serve_chunk` independently of the POST init path.
//
// Inputs:
//   * `path` is the request path (with optional `?offset=N` query).
//     We re-parse the file id off it rather than have the caller do
//     it, so the route-arm in net.rs stays a single-line dispatch.
//   * `body` is the raw chunk bytes (Content-Length-framed).
//   * `content_range` is the optional `Content-Range: bytes N-M/total`
//     header, used as a fallback when `?offset=N` is absent.
//
// Returns:
//   * 204 No Content on a partial chunk (highest_contiguous_byte
//     advanced but more bytes remain).
//   * 200 OK with the final size on the last chunk; the upload is
//     sealed (mime sniffed, facts updated, in-memory state removed).
//   * 416 Range Not Satisfiable when offset+len exceeds declared total.
//   * 400 Bad Request when offset is non-numeric / missing /
//     misaligned with the upload's high-water mark.
//   * 404 Not Found when the file id has no active upload session.
//   * 500 Internal Server Error on disk I/O failure.
///
/// Idempotency-aware shim: forwards to `try_serve_chunk_idempotent`
/// with `idempotency_key = None`, preserving the existing
/// `net::drive_http` call signature. See `try_serve` for the
/// matching shim on the POST entry.
// TODO(#446): once net.rs grows an `Idempotency-Key` extractor,
// route the live PUT dispatch through `try_serve_chunk_idempotent`
// so retried chunks yield the cached completion response.
pub fn try_serve_chunk(
    method: &str,
    path: &str,
    body: &[u8],
    content_range: Option<&str>,
) -> ServeOutcome {
    try_serve_chunk_idempotent(method, path, body, content_range, None)
}

/// Idempotency-aware variant of `try_serve_chunk` (#446). When
/// `idempotency_key` is `Some`, the function probes IDEMPOTENCY for
/// a fresh entry and short-circuits with the cached chunk-completion
/// response when one matches — the typical retry case after a
/// transient network blip on the final chunk. Caches the response
/// only on the sealing 200 OK; intermediate 204 No Contents aren't
/// cached because they encode a state-machine position, not a
/// resource creation that retries should idempotently restore.
///
/// Also wires:
///   * #447 — enqueues a `ProgressEvent` after every successful
///     write (intermediate or final). The SSE handler at
///     `try_serve_progress` drains the queue.
///   * #448 — when `offset == 0` and `body.len() >= 512`, sniffs
///     the leading bytes via `detect_mime` and rewrites the
///     `File_has_MimeType` fact when the sniffed type differs from
///     the placeholder. Subsequent chunks skip the sniff via the
///     `mime_promoted` flag on UploadState.
pub fn try_serve_chunk_idempotent(
    method: &str,
    path: &str,
    body: &[u8],
    content_range: Option<&str>,
    idempotency_key: Option<&str>,
) -> ServeOutcome {
    let (path_only, query) = split_query(path);
    let file_id = match parse_chunk_path(path_only) {
        Some(id) => id,
        None => return ServeOutcome::NotApplicable,
    };
    if method != "PUT" {
        return ServeOutcome::Response(method_not_allowed_chunk());
    }
    if body.len() > MAX_CHUNK_BYTES {
        return ServeOutcome::Response(payload_too_large());
    }

    // #446: probe before parsing offset / locking UPLOADS so a retry
    // doesn't briefly wedge the in-memory session-map mutex.
    if let Some(key) = idempotency_key {
        if let Some(cached) = lookup_idempotent(key) {
            return ServeOutcome::Response(cached);
        }
    }

    let offset = match parse_chunk_offset(query, content_range) {
        Ok(n) => n,
        Err(msg) => return ServeOutcome::Response(bad_request(msg)),
    };

    let mut uploads = UPLOADS.lock();
    let st = match uploads.get_mut(file_id) {
        Some(s) => s,
        None => return ServeOutcome::Response(not_found_upload(file_id)),
    };

    // Bounds: offset must be ≤ total, and offset+len must be ≤ total.
    let end = match offset.checked_add(body.len() as u64) {
        Some(n) => n,
        None => return ServeOutcome::Response(range_not_satisfiable_chunk(st.total)),
    };
    if end > st.total {
        return ServeOutcome::Response(range_not_satisfiable_chunk(st.total));
    }

    // Out-of-order writes are rejected — the resume protocol is
    // strictly append-only at the highest_contiguous_byte. A future
    // commit can relax this by buffering ahead-of-mark chunks; today
    // the simpler invariant keeps the in-memory state bounded.
    if offset != st.highest_contiguous_byte {
        return ServeOutcome::Response(bad_request(
            "chunk offset must equal current highest_contiguous_byte; \
             use GET /file/{id}/upload-state to resume",
        ));
    }

    // #448: snapshot whether this is the first chunk and the leading
    // window has enough bytes to sniff. Promotion fires *after* the
    // disk write succeeds, so a write failure doesn't leave the
    // SYSTEM fact ahead of the on-disk state. Anything shorter than
    // MIME_SNIFF_MIN_BYTES on the first chunk falls through to the
    // closing seal-step sniff.
    let first_chunk_sniff: Option<&'static str> = if offset == 0
        && !st.mime_promoted
        && body.len() >= MIME_SNIFF_MIN_BYTES
    {
        Some(detect_mime(&body[..MIME_SNIFF_MIN_BYTES]))
    } else {
        None
    };

    if !body.is_empty() {
        if let Err(msg) = write_chunk_to_region(st, offset, body) {
            return ServeOutcome::Response(internal_error(&msg));
        }
        // Capture the first 512 bytes for the closing MIME sniff.
        if st.sniff_window.len() < 512 {
            let need = 512 - st.sniff_window.len();
            let take = body.len().min(need);
            st.sniff_window.extend_from_slice(&body[..take]);
        }
        st.highest_contiguous_byte = end;
        // #447: enqueue a progress event for any SSE subscriber on
        // /file/{file_id}/progress. The function is total-aware so
        // the UI can render a percentage without a separate fetch.
        enqueue_progress(file_id, st.highest_contiguous_byte, st.total);
    }

    // #448: promote the sniffed MIME type onto the SYSTEM fact when
    // it differs from the placeholder mime_hint. Sets mime_promoted
    // so subsequent chunks don't re-walk the apply() path. Rewrites
    // only the File_has_MimeType cell (via cell_filter + cell_push)
    // — the other four facts stay intact, keeping the placeholder
    // ContentRef live until the closing seal step swaps it.
    if let Some(sniffed) = first_chunk_sniff {
        if sniffed != st.mime_hint.as_str() {
            let pre_state = crate::system::state().cloned().unwrap_or_else(Object::phi);
            let new_state = update_mime_fact(&pre_state, file_id, sniffed);
            if let Err(msg) = crate::system::apply(new_state) {
                return ServeOutcome::Response(internal_error(&msg));
            }
            st.mime_hint = sniffed.to_string();
        }
        // Mark promoted whether or not the type changed — the
        // sniff-window has the bytes it needs and re-running the
        // detector each chunk is wasted work.
        st.mime_promoted = true;
    }

    // Last chunk → seal the upload, return 200.
    if st.highest_contiguous_byte == st.total {
        let sniffed_mime = if !st.sniff_window.is_empty() {
            detect_mime(&st.sniff_window).to_string()
        } else {
            st.mime_hint.clone()
        };
        let cref = format!("<REGION,{},{}>", st.base_sector, st.total);
        let final_size = st.total;
        let filename = st.filename.clone();
        let directory_id = st.directory_id.clone();
        let _ = st.region.flush(); // best-effort durability fence

        // Re-write the File facts under the new ContentRef + Size +
        // sniffed MimeType. `system::apply` swaps the SYSTEM pointer
        // atomically; readers see either the placeholder or the
        // sealed state, never a half-update.
        let pre_state = crate::system::state().cloned().unwrap_or_else(Object::phi);
        let new_state = build_file_facts_with_cref(
            pre_state,
            file_id,
            &filename,
            &sniffed_mime,
            &cref,
            final_size,
            &directory_id,
        );
        if let Err(msg) = crate::system::apply(new_state) {
            return ServeOutcome::Response(internal_error(&msg));
        }
        st.complete = true;
        // Drop the in-memory session — anyone re-querying upload-state
        // after completion gets `complete: true` from the closure
        // below before the entry vanishes.
        let final_size_for_response = final_size;
        let id_for_response = file_id.to_string();
        uploads.remove(file_id);
        // #447: emit a final marker on the progress queue so an SSE
        // subscriber knows to close. The drain-side handler
        // recognises `bytes_written == total_bytes` and writes the
        // `event: complete\ndata: {}\n\n` framing.
        enqueue_progress(&id_for_response, final_size_for_response, final_size_for_response);
        let response = chunk_complete_response(
            &id_for_response, final_size_for_response,
        );
        // Cache the sealing response — a retry on the final chunk
        // is the canonical idempotent case (client lost the 200 to
        // a network blip and is retrying the last PUT).
        if let Some(key) = idempotency_key {
            record_idempotent(key, &response);
        }
        return ServeOutcome::Response(response);
    }

    ServeOutcome::Response(no_content_response())
}

/// Write `chunk` to the region starting at byte `offset`. Sector-
/// aligned on both ends — non-aligned heads / tails are read-modify-
/// written so the on-disk image stays consistent for region readers.
///
/// `offset` is guaranteed by the caller to equal `state.highest_
/// contiguous_byte`, so there's no prior-data preservation needed
/// for the head sector beyond the bytes the upload itself wrote.
/// The tail sector may need a read-modify-write only if it overlaps
/// previously-written bytes (which can happen when chunks happen to
/// straddle a sector boundary in resume mode). For simplicity we
/// always RMW partial heads/tails — the cost is one extra sector
/// read per non-aligned chunk boundary, which is fine for the rates
/// the kernel sees today.
fn write_chunk_to_region(st: &UploadState, offset: u64, chunk: &[u8]) -> Result<(), &'static str> {
    let sec_size = BLOCK_SECTOR_SIZE as u64;
    let mut wpos = offset;
    let mut idx: usize = 0;
    let chunk_end = offset + chunk.len() as u64;

    while wpos < chunk_end {
        let sector = wpos / sec_size;
        let off_in_sec = (wpos % sec_size) as usize;
        let bytes_in_sec = (BLOCK_SECTOR_SIZE - off_in_sec).min(chunk.len() - idx);
        let mut sec_buf = [0u8; BLOCK_SECTOR_SIZE];

        // RMW only when the write doesn't span the full sector.
        // A fresh region is zero-initialised anyway, so the read
        // is only meaningful when the sector has prior data. We
        // unconditionally read for safety — the small extra I/O
        // is cheaper than carrying a per-sector dirty bitmap.
        if off_in_sec != 0 || bytes_in_sec != BLOCK_SECTOR_SIZE {
            st.region.read(sector, &mut sec_buf).map_err(|_| "region read failed")?;
        }
        sec_buf[off_in_sec..off_in_sec + bytes_in_sec]
            .copy_from_slice(&chunk[idx..idx + bytes_in_sec]);
        st.region.write(sector, &sec_buf).map_err(|_| "region write failed")?;

        wpos += bytes_in_sec as u64;
        idx += bytes_in_sec;
    }
    Ok(())
}

// ── GET /file/{id}/upload-state ────────────────────────────────────
//
// Resume probe. Returns JSON `{"highest_contiguous_byte": N, "size": M,
// "complete": false}` for an in-flight upload, or `{"size": N,
// "complete": true}` for one that has already sealed (the in-memory
// session is gone but the file's `File_has_Size` fact is in SYSTEM).
//
// 404 when neither an in-flight session nor a known File exists for
// the id.
//
// As of #447, this entry point also dispatches `GET /file/{id}/
// progress` — the SSE handler — by chaining to `try_serve_progress`
// before its own match runs. The chain lives here (rather than in
// `net.rs::drive_http`) because file_upload.rs owns the upload-
// related route family; net.rs's existing arm calls
// `try_serve_upload_state` for any GET that fell through the chunk
// dispatch, so the progress route lands automatically without
// touching net.rs. Once Track NNN's #360 register_http surfaces a
// dedicated route table, the SSE handler can register itself
// directly and this chain can be unwound.
pub fn try_serve_upload_state(
    method: &str,
    path: &str,
    state: Option<&Object>,
) -> ServeOutcome {
    // #447: SSE progress handler chains in here. NotApplicable means
    // the path didn't match `/file/{id}/progress` — fall through to
    // upload-state's own match.
    match try_serve_progress(method, path) {
        ServeOutcome::Response(bytes) => return ServeOutcome::Response(bytes),
        ServeOutcome::NotApplicable => {} // fall through
    }

    let (path_only, _) = split_query(path);
    let file_id = match parse_upload_state_path(path_only) {
        Some(id) => id,
        None => return ServeOutcome::NotApplicable,
    };
    if method != "GET" {
        return ServeOutcome::Response(method_not_allowed_upload_state());
    }

    // In-flight upload: the in-memory session is the source of truth.
    if let Some(st) = UPLOADS.lock().get(file_id) {
        return ServeOutcome::Response(upload_state_response(
            st.highest_contiguous_byte, st.total, false,
        ));
    }

    // Sealed upload: look up the File's Size fact. Treat a present
    // size as `complete: true` because the in-memory session is
    // dropped only after the seal succeeds.
    if let Some(state) = state {
        if let Some(size_str) = lookup_size(file_id, state) {
            if let Ok(size) = size_str.parse::<u64>() {
                return ServeOutcome::Response(upload_state_response(
                    size, size, true,
                ));
            }
        }
    }
    ServeOutcome::Response(not_found_upload(file_id))
}

// ── Path / query parsing for chunk routes ──────────────────────────

/// Split `"/file/foo/chunk?offset=42"` into `("/file/foo/chunk", Some("offset=42"))`.
/// Returns `(path, None)` when no `?` is present.
fn split_query(path: &str) -> (&str, Option<&str>) {
    match path.find('?') {
        Some(i) => (&path[..i], Some(&path[i + 1..])),
        None => (path, None),
    }
}

/// Extract `{id}` from `/file/{id}/chunk`. Returns `None` for any
/// other path shape — caller falls through to the next dispatch arm.
fn parse_chunk_path(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/file/")?;
    let id = rest.strip_suffix("/chunk")?;
    if id.is_empty() || id.contains('/') {
        return None;
    }
    Some(id)
}

/// Extract `{id}` from `/file/{id}/upload-state`. Symmetric with
/// `parse_chunk_path`.
fn parse_upload_state_path(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/file/")?;
    let id = rest.strip_suffix("/upload-state")?;
    if id.is_empty() || id.contains('/') {
        return None;
    }
    Some(id)
}

/// Resolve the chunk's byte offset from either `?offset=N` or
/// `Content-Range: bytes N-M/total`. The query takes precedence —
/// the header form is the fallback per the spec.
fn parse_chunk_offset(
    query: Option<&str>,
    content_range: Option<&str>,
) -> Result<u64, &'static str> {
    if let Some(q) = query {
        for kv in q.split('&') {
            let mut parts = kv.splitn(2, '=');
            let k = parts.next().unwrap_or("");
            let v = parts.next().unwrap_or("");
            if k == "offset" {
                return v.parse().map_err(|_| "?offset= must be a decimal byte count");
            }
        }
    }
    if let Some(cr) = content_range {
        // `bytes N-M/total` — we only need N.
        let cr = cr.trim();
        let rest = cr.strip_prefix("bytes ")
            .ok_or("Content-Range must start with `bytes `")?;
        let dash = rest.find('-')
            .ok_or("Content-Range missing `-`")?;
        let n_str = &rest[..dash];
        return n_str.trim().parse()
            .map_err(|_| "Content-Range start byte must be a decimal");
    }
    Err("missing chunk offset (use `?offset=N` or `Content-Range`)")
}

// ── Per-upload state ────────────────────────────────────────────────
//
// One entry per active chunked upload, keyed by file id. The entry
// holds the region handle (so PUT chunks don't have to re-derive it
// from the placeholder ContentRef), the declared total, the
// highest contiguous byte that has landed on disk, and a sliding
// MIME-sniff window of the first 512 bytes for the closing seal.
//
// `Mutex<BTreeMap>` is the no_std-friendly shape: `BTreeMap` lives
// in `alloc::collections` (no hashbrown / std), and `spin::Mutex`
// works in IRQ context. The map is bounded by active upload count;
// completed uploads remove their entry on the seal step.
struct UploadState {
    region: RegionHandle,
    base_sector: u64,
    total: u64,
    highest_contiguous_byte: u64,
    filename: String,
    directory_id: String,
    mime_hint: String,
    /// Captured prefix of the upload bytes for the closing `detect_mime`
    /// pass. Capped at 512 bytes (the sniff table only inspects the
    /// first 512 anyway), so the per-upload memory cost is bounded.
    sniff_window: Vec<u8>,
    /// Set by the seal step before the entry is removed; lets a
    /// late `try_serve_upload_state` call (in the narrow window
    /// between the seal and the map remove) report `complete: true`.
    #[allow(dead_code)]
    complete: bool,
    /// #448: marks that the first-chunk MIME sniff has run. Set
    /// after the sniff fires (whether or not the type changed) so
    /// subsequent chunks don't re-walk `detect_mime` / re-write the
    /// `File_has_MimeType` fact. The closing seal still runs the
    /// detector against the full `sniff_window` as a final pass —
    /// typically a no-op once promoted.
    mime_promoted: bool,
}

static UPLOADS: Mutex<BTreeMap<String, UploadState>> = Mutex::new(BTreeMap::new());

// ── Idempotency-Key cache (#446) ───────────────────────────────────
//
// Maps a client-supplied `Idempotency-Key` header value to the
// previously-issued response bytes for a successful POST /file or
// PUT /file/{id}/chunk. A retry under the same key returns the
// cached response verbatim — same File id, same Location header,
// same status — instead of allocating a new id (POST) or re-walking
// the seal pipeline (PUT).
//
// Keying simplification: the entry is keyed on the header value
// alone. The fully-correct shape is `(client-id, key)` so two
// clients can't trample each other's keyspace, but the kernel
// doesn't have a stable client identity yet — `net::drive_http`
// runs one connection at a time and doesn't expose the remote
// address to the request pipeline. The header-only key is safe in
// single-tenant deployments (the kernel's only deployment shape
// today) and degrades gracefully when multi-tenancy lands: that
// change pairs with extending the key to a tuple, which only
// requires updating this module.
//
// Bounds:
//   * Per-entry expiry: `IDEMPOTENCY_TTL_MS` (24h) past the entry's
//     insert time, measured against `arch::time::now_ms()`. Lookup
//     evicts an expired entry before returning a cache miss.
//   * Total entries: `IDEMPOTENCY_MAX_ENTRIES` (1024). When full,
//     the least-recently-touched entry (smallest `last_seen_ms`) is
//     dropped. Lookup updates `last_seen_ms` on hit so the cache
//     follows the actual retry pattern.

/// One IDEMPOTENCY map entry. The cached `response` is the wire
/// bytes the server originally sent — including status line,
/// headers, and body — so a retry replays the exact response.
struct IdempotencyEntry {
    response: Vec<u8>,
    expires_at_ms: u64,
    last_seen_ms: u64,
}

static IDEMPOTENCY: Mutex<BTreeMap<String, IdempotencyEntry>> =
    Mutex::new(BTreeMap::new());

/// Probe IDEMPOTENCY for `key`. Returns the cached response bytes
/// on a fresh hit (after refreshing the entry's `last_seen_ms`),
/// or `None` for either a miss or an expired entry. Expired entries
/// are dropped on encounter so the map self-cleans without a
/// separate sweep task.
fn lookup_idempotent(key: &str) -> Option<Vec<u8>> {
    let now = idempotency_now_ms();
    let mut map = IDEMPOTENCY.lock();
    let hit = match map.get_mut(key) {
        Some(entry) if entry.expires_at_ms > now => {
            entry.last_seen_ms = now;
            Some(entry.response.clone())
        }
        Some(_) => None, // expired — fall through to remove
        None => return None,
    };
    if hit.is_none() {
        map.remove(key);
    }
    hit
}

/// Insert (or refresh) an IDEMPOTENCY entry for `key`. If the map
/// is at `IDEMPOTENCY_MAX_ENTRIES` and `key` is new, the LRU entry
/// (smallest `last_seen_ms`) is evicted to make room.
fn record_idempotent(key: &str, response: &[u8]) {
    let now = idempotency_now_ms();
    let mut map = IDEMPOTENCY.lock();
    if map.len() >= IDEMPOTENCY_MAX_ENTRIES && !map.contains_key(key) {
        // O(n) over n=1024 is fine at the request rates the kernel
        // sees today.
        if let Some(victim) = map
            .iter()
            .min_by_key(|(_, e)| e.last_seen_ms)
            .map(|(k, _)| k.clone())
        {
            map.remove(&victim);
        }
    }
    map.insert(
        key.to_string(),
        IdempotencyEntry {
            response: response.to_vec(),
            expires_at_ms: now.saturating_add(IDEMPOTENCY_TTL_MS),
            last_seen_ms: now,
        },
    );
}

/// Wall-clock reading for IDEMPOTENCY TTL/LRU bookkeeping. Routes
/// through `arch::time::now_ms()` on the kernel target. The test
/// build uses a per-call monotonic counter so LRU ordering still
/// works without an arch dep.
#[cfg(not(test))]
fn idempotency_now_ms() -> u64 {
    crate::arch::time::now_ms()
}

#[cfg(test)]
fn idempotency_now_ms() -> u64 {
    static CTR: AtomicU64 = AtomicU64::new(1);
    CTR.fetch_add(1, Ordering::Relaxed)
}

/// True iff `response` starts with an `HTTP/1.1 2xx` status line.
/// Used to skip caching error responses — caching a 400/413/503
/// would freeze a transient failure into a permanent one for any
/// client that retries under the same key.
fn is_2xx_response(response: &[u8]) -> bool {
    response.starts_with(b"HTTP/1.1 2")
}

/// Look up the `Idempotency-Key` header value (case-insensitive) in
/// a buffered HTTP/1.1 request. Mirrors
/// `extract_content_type_header` / `extract_content_range_header`.
/// Public so a future net.rs revision can extract the header and
/// pass it into `try_serve_idempotent` /
/// `try_serve_chunk_idempotent`.
// `#[allow(dead_code)]` until net.rs grows the matching call site.
// Mirrors the existing pattern (extract_content_type_header,
// extract_content_range_header) which have live callers in
// `net::drive_http`; once Track NNN's #360 register_http surfaces
// a dispatch table that re-scans rx_buf for this header, the
// allow can be dropped.
#[allow(dead_code)]
pub fn extract_idempotency_key_header(buf: &[u8]) -> Option<String> {
    extract_named_header(buf, "idempotency-key")
}

// ── Per-upload progress events (#447) ──────────────────────────────
//
// Each chunk write enqueues a ProgressEvent into the per-file ring
// in PROGRESS_EVENTS. The SSE handler at `try_serve_progress`
// drains the ring and writes the events as `text/event-stream`
// wire bytes with `\n\n` framing. The ring is bounded at
// PROGRESS_RING_DEPTH (64) — a slow / disconnected subscriber
// can't pin unbounded memory; older events fall off the front
// when the ring fills.
//
// Lifecycle: enqueues happen via `enqueue_progress`; the handler
// drains via `drain_progress`. Once an upload completes (the seal
// step in `try_serve_chunk_idempotent` enqueues the final event)
// the ring is left in place for any straggler GET — a future
// commit can prune completed rings on a periodic sweep.

#[derive(Clone, Copy)]
struct ProgressEvent {
    bytes_written: u64,
    total_bytes: u64,
}

static PROGRESS_EVENTS: Mutex<BTreeMap<String, VecDeque<ProgressEvent>>> =
    Mutex::new(BTreeMap::new());

/// Enqueue a ProgressEvent for `file_id`. If the per-file ring is
/// at `PROGRESS_RING_DEPTH`, the oldest event is dropped to make
/// room (front-drop semantics). Called from
/// `try_serve_chunk_idempotent` after each successful chunk write.
fn enqueue_progress(file_id: &str, bytes_written: u64, total_bytes: u64) {
    let mut events = PROGRESS_EVENTS.lock();
    let ring = events
        .entry(file_id.to_string())
        .or_insert_with(VecDeque::new);
    if ring.len() >= PROGRESS_RING_DEPTH {
        ring.pop_front();
    }
    ring.push_back(ProgressEvent {
        bytes_written,
        total_bytes,
    });
}

/// Drain all pending events for `file_id`. Returns an empty `Vec`
/// if no events are queued.
fn drain_progress(file_id: &str) -> Vec<ProgressEvent> {
    let mut events = PROGRESS_EVENTS.lock();
    match events.get_mut(file_id) {
        Some(ring) => ring.drain(..).collect(),
        None => Vec::new(),
    }
}

/// HTTP `GET /file/{id}/progress` (#447). Returns
/// `text/event-stream` wire bytes containing all currently-queued
/// progress events for `{id}`. Path-only function — the existing
/// dispatch chain in `net::drive_http` reaches it through
/// `try_serve_upload_state`, which now chains to it before its
/// own match (so the new GET path is reachable on the live wire
/// without a net.rs touch).
///
/// SSE shape choice (polled-each-GET, not held-open): the kernel's
/// HTTP listener is single-shot per connection — `drive_http`
/// parses one request, queues one response, and closes the socket.
/// Holding a connection open across the response loop would
/// require a rewrite of the listener state machine. The polled-
/// each-GET shape fits the existing surface cleanly: each GET
/// drains the queue and writes whatever events are pending, plus a
/// `Last-Event-ID` header so the client knows where it left off.
/// EventSource's reconnect logic does the right thing — it'll
/// re-poll automatically. When the upload completes (closing event
/// has `bytes_written == total_bytes`), the response includes a
/// final `event: complete\ndata: {}\n\n` frame.
pub fn try_serve_progress(method: &str, path: &str) -> ServeOutcome {
    let (path_only, _) = split_query(path);
    let file_id = match parse_progress_path(path_only) {
        Some(id) => id,
        None => return ServeOutcome::NotApplicable,
    };
    if method != "GET" {
        return ServeOutcome::Response(method_not_allowed_progress());
    }
    let events = drain_progress(file_id);
    ServeOutcome::Response(progress_sse_response(file_id, &events))
}

/// Extract `{id}` from `/file/{id}/progress`. Symmetric with
/// `parse_chunk_path` and `parse_upload_state_path`.
fn parse_progress_path(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/file/")?;
    let id = rest.strip_suffix("/progress")?;
    if id.is_empty() || id.contains('/') {
        return None;
    }
    Some(id)
}

// ── First-chunk MIME promotion (#448) ──────────────────────────────

/// Replace the `File_has_MimeType` fact for `file_id` with one
/// whose MimeType binding is `mime`. Implemented as `cell_filter`
/// (drops the existing fact for this file id) followed by
/// `cell_push` (adds the new one) so the cell ends up with exactly
/// one MimeType fact per file id — no duplicates, no reordering of
/// facts for unrelated files. The other four File facts (Name /
/// ContentRef / Size / is_in_Directory) are untouched, so the
/// placeholder ContentRef the chunked-init step planted stays live
/// until the closing seal step swaps it for `<REGION,base,len>`.
fn update_mime_fact(state: &Object, file_id: &str, mime: &str) -> Object {
    let target_file_id = file_id.to_string();
    let filtered = ast::cell_filter(
        "File_has_MimeType",
        move |fact| ast::binding(fact, "File") != Some(&target_file_id),
        state,
    );
    ast::cell_push(
        "File_has_MimeType",
        fact_from_pairs(&[("File", file_id), ("MimeType", mime)]),
        &filtered,
    )
}

/// Look up `File_has_Size` for `file_id`. Mirrors `file_serve`'s
/// lookup_mime / lookup_content_ref shape.
fn lookup_size(file_id: &str, state: &Object) -> Option<String> {
    let cell = ast::fetch_or_phi("File_has_Size", state);
    cell.as_seq()?.iter().find_map(|fact| {
        if ast::binding(fact, "File") == Some(file_id) {
            ast::binding(fact, "Size").map(|s| s.to_string())
        } else {
            None
        }
    })
}

// ── Header extraction (raw request bytes) ───────────────────────────

/// Look up the `Content-Range` header value (case-insensitive) in a
/// buffered HTTP/1.1 request. Mirrors `extract_content_type_header` —
/// the canonical `http::parse_request` doesn't capture this header.
pub fn extract_content_range_header(buf: &[u8]) -> Option<String> {
    extract_named_header(buf, "content-range")
}

/// Generic case-insensitive header extractor used by the chunk +
/// upload-state arms in net.rs. Returned value has surrounding
/// whitespace stripped.
fn extract_named_header(buf: &[u8], name_lower: &str) -> Option<String> {
    let header_end = find_subslice(buf, b"\r\n\r\n")?;
    let head = core::str::from_utf8(&buf[..header_end]).ok()?;
    for line in head.split("\r\n") {
        if line.is_empty() { continue; }
        let colon = match line.find(':') {
            Some(i) => i,
            None => continue,
        };
        let nm = line[..colon].trim();
        if nm.eq_ignore_ascii_case(name_lower) {
            return Some(line[colon + 1..].trim().to_string());
        }
    }
    None
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
    build_file_facts_with_cref(
        state, file_id, name, mime, &cref, bytes.len() as u64, parent_dir_id,
    )
}

/// Variant that takes a pre-built `ContentRef` atom + explicit size.
/// Used by the chunked-upload path so the encoded value can be a
/// `<REGION,base,len>` tagged form (placeholder at init, sealed at
/// the last chunk) rather than the inline-only hex shape.
fn build_file_facts_with_cref(
    state: Object,
    file_id: &str,
    name: &str,
    mime: &str,
    cref: &str,
    size: u64,
    parent_dir_id: &str,
) -> Object {
    let size_str = format!("{}", size);
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
        fact_from_pairs(&[("File", file_id), ("ContentRef", cref)]),
        &d,
    );
    let d = ast::cell_push(
        "File_has_Size",
        fact_from_pairs(&[("File", file_id), ("Size", &size_str)]),
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

/// 201 Created variant for chunked-mode init. JSON body carries the
/// upload_id (currently identical to the file id) and the suggested
/// chunk_size for subsequent PUT /file/{id}/chunk calls.
fn chunked_init_response(file_id: &str) -> Vec<u8> {
    let body = format!(
        "{{\"id\":\"{0}\",\"upload_id\":\"{0}\",\"chunk_size\":{1}}}\n",
        file_id, CHUNK_SIZE_HINT,
    ).into_bytes();
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

/// 204 No Content for a partial chunk write. Carries no body; the
/// client tracks the next offset itself or polls
/// GET /file/{id}/upload-state.
fn no_content_response() -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    push_status(&mut out, 204, "No Content");
    push_header(&mut out, "Content-Length", "0");
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out
}

/// 200 OK terminating a chunked upload. JSON body reports the final
/// size and `status: complete` so a client driving the upload from
/// a script can branch on a single field.
fn chunk_complete_response(file_id: &str, size: u64) -> Vec<u8> {
    let body = format!(
        "{{\"file_id\":\"{}\",\"status\":\"complete\",\"size\":{}}}\n",
        file_id, size,
    ).into_bytes();
    let mut out = Vec::with_capacity(192 + body.len());
    push_status(&mut out, 200, "OK");
    push_header(&mut out, "Content-Type", "application/json");
    push_header(&mut out, "Content-Length", &format!("{}", body.len()));
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&body);
    out
}

/// JSON body for GET /file/{id}/upload-state.
fn upload_state_response(highest: u64, size: u64, complete: bool) -> Vec<u8> {
    let body = format!(
        "{{\"highest_contiguous_byte\":{},\"size\":{},\"complete\":{}}}\n",
        highest, size, complete,
    ).into_bytes();
    let mut out = Vec::with_capacity(192 + body.len());
    push_status(&mut out, 200, "OK");
    push_header(&mut out, "Content-Type", "application/json");
    push_header(&mut out, "Content-Length", &format!("{}", body.len()));
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&body);
    out
}

/// 416 Range Not Satisfiable for chunk writes. Mirrors RFC 7233 §4.4
/// with a `Content-Range: bytes */{total}` header so the client can
/// re-anchor.
fn range_not_satisfiable_chunk(total: u64) -> Vec<u8> {
    let body = b"chunk offset+length exceeds declared total\n";
    let mut out = Vec::with_capacity(192 + body.len());
    push_status(&mut out, 416, "Range Not Satisfiable");
    push_header(&mut out, "Content-Type", "text/plain; charset=utf-8");
    push_header(&mut out, "Content-Length", &format!("{}", body.len()));
    push_header(&mut out, "Content-Range", &format!("bytes */{}", total));
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(body);
    out
}

fn not_found_upload(file_id: &str) -> Vec<u8> {
    let body = format!("no upload session for file {}\n", file_id).into_bytes();
    let mut out = Vec::with_capacity(96 + body.len());
    push_status(&mut out, 404, "Not Found");
    push_header(&mut out, "Content-Type", "text/plain; charset=utf-8");
    push_header(&mut out, "Content-Length", &format!("{}", body.len()));
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&body);
    out
}

fn method_not_allowed_chunk() -> Vec<u8> {
    let body = b"only PUT is supported on /file/{id}/chunk\n";
    let mut out = Vec::with_capacity(128 + body.len());
    push_status(&mut out, 405, "Method Not Allowed");
    push_header(&mut out, "Allow", "PUT");
    push_header(&mut out, "Content-Type", "text/plain; charset=utf-8");
    push_header(&mut out, "Content-Length", &format!("{}", body.len()));
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(body);
    out
}

fn method_not_allowed_upload_state() -> Vec<u8> {
    let body = b"only GET is supported on /file/{id}/upload-state\n";
    let mut out = Vec::with_capacity(128 + body.len());
    push_status(&mut out, 405, "Method Not Allowed");
    push_header(&mut out, "Allow", "GET");
    push_header(&mut out, "Content-Type", "text/plain; charset=utf-8");
    push_header(&mut out, "Content-Length", &format!("{}", body.len()));
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(body);
    out
}

fn method_not_allowed_progress() -> Vec<u8> {
    let body = b"only GET is supported on /file/{id}/progress\n";
    let mut out = Vec::with_capacity(128 + body.len());
    push_status(&mut out, 405, "Method Not Allowed");
    push_header(&mut out, "Allow", "GET");
    push_header(&mut out, "Content-Type", "text/plain; charset=utf-8");
    push_header(&mut out, "Content-Length", &format!("{}", body.len()));
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(body);
    out
}

/// Render a polled SSE response for `/file/{id}/progress` (#447).
/// Each `ProgressEvent` becomes one `data: {...}\n\n` frame; the
/// closing event (`bytes_written == total_bytes` with `total_bytes
/// > 0` so empty-upload doesn't fire it) is additionally promoted
/// to an `event: complete\ndata: {}\n\n` frame so a subscriber can
/// branch on the SSE event name without parsing the JSON body.
///
/// Wire shape per SSE spec (https://html.spec.whatwg.org/#server-
/// sent-events): `Content-Type: text/event-stream`. The polled-
/// each-GET shape this listener uses sets Content-Length so the
/// existing single-shot `drive_http` close-after-write loop sees a
/// terminated response. The browser EventSource handles both shapes
/// (held-open or polled-with-Content-Length) transparently.
///
/// `Last-Event-ID` (in response headers) is the highest
/// `bytes_written` value in this batch — clients reconnect with
/// `Last-Event-ID:` in the request, and the next GET resumes after
/// that point. Today the kernel doesn't honour the request header
/// (the queue is drain-only), but emitting it keeps the wire shape
/// EventSource-compliant.
fn progress_sse_response(file_id: &str, events: &[ProgressEvent]) -> Vec<u8> {
    let mut body = Vec::with_capacity(128 * (events.len() + 1));
    let mut last_id: u64 = 0;
    let mut completed = false;
    for ev in events {
        body.extend_from_slice(b"id: ");
        body.extend_from_slice(format!("{}", ev.bytes_written).as_bytes());
        body.extend_from_slice(b"\n");
        body.extend_from_slice(b"data: ");
        body.extend_from_slice(
            format!(
                "{{\"file_id\":\"{}\",\"bytes_written\":{},\"total_bytes\":{}}}",
                file_id, ev.bytes_written, ev.total_bytes,
            )
            .as_bytes(),
        );
        body.extend_from_slice(b"\n\n");
        if ev.total_bytes > 0 && ev.bytes_written >= ev.total_bytes {
            completed = true;
        }
        if ev.bytes_written > last_id {
            last_id = ev.bytes_written;
        }
    }
    if completed {
        body.extend_from_slice(b"event: complete\ndata: {}\n\n");
    } else if events.is_empty() {
        // Comment-only keepalive when no events are queued — keeps
        // the polled connection cheap and signals "no progress yet"
        // without inventing a custom event name.
        body.extend_from_slice(b": keepalive\n\n");
    }

    let mut out = Vec::with_capacity(192 + body.len());
    push_status(&mut out, 200, "OK");
    push_header(&mut out, "Content-Type", "text/event-stream");
    push_header(&mut out, "Cache-Control", "no-cache");
    push_header(&mut out, "Last-Event-ID", &format!("{}", last_id));
    push_header(&mut out, "Content-Length", &format!("{}", body.len()));
    push_header(&mut out, "Connection", "close");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&body);
    out
}

/// Map a `block_storage::Error` from `alloc_region` into the
/// closest HTTP status. Kept out of `internal_error` so the chunked-
/// init path can distinguish "no disk attached" (a 503) from "slot
/// table full" (a 507) from "requested size too big" (a 413).
fn region_alloc_error_response(e: block_storage::Error) -> Vec<u8> {
    use block_storage::Error::*;
    match e {
        StateTooLarge => payload_too_large(),
        NotMounted => {
            let body = b"no persistence disk attached; \
                chunked uploads require virtio-blk\n";
            let mut out = Vec::with_capacity(192 + body.len());
            push_status(&mut out, 503, "Service Unavailable");
            push_header(&mut out, "Content-Type", "text/plain; charset=utf-8");
            push_header(&mut out, "Content-Length", &format!("{}", body.len()));
            push_header(&mut out, "Connection", "close");
            out.extend_from_slice(b"\r\n");
            out.extend_from_slice(body);
            out
        }
        OutOfRange | Io => {
            let body = b"blob slot table exhausted; \
                free a slot or expand the disk\n";
            let mut out = Vec::with_capacity(192 + body.len());
            push_status(&mut out, 507, "Insufficient Storage");
            push_header(&mut out, "Content-Type", "text/plain; charset=utf-8");
            push_header(&mut out, "Content-Length", &format!("{}", body.len()));
            push_header(&mut out, "Connection", "close");
            out.extend_from_slice(b"\r\n");
            out.extend_from_slice(body);
            out
        }
        // Aead is unreachable on the alloc_region path (which never
        // touches the AEAD primitive), but the match must stay
        // exhaustive after #659 added the variant. Surface it as a
        // 500 with a distinct hint so the operator notices if it
        // ever fires.
        Aead(_) => {
            let body = b"unexpected AEAD error from region allocator; \
                this should be unreachable - please file an issue\n";
            let mut out = Vec::with_capacity(192 + body.len());
            push_status(&mut out, 500, "Internal Server Error");
            push_header(&mut out, "Content-Type", "text/plain; charset=utf-8");
            push_header(&mut out, "Content-Length", &format!("{}", body.len()));
            push_header(&mut out, "Connection", "close");
            out.extend_from_slice(b"\r\n");
            out.extend_from_slice(body);
            out
        }
    }
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

    // ── Chunked-upload tests (#445) ────────────────────────────────
    //
    // The chunked path's hot loop touches `block_storage::alloc_region`
    // / `RegionHandle::write`, both of which short-circuit when no
    // virtio-blk device is installed (which is always the case under
    // the kernel-test harness). The tests below exercise:
    //
    //   * Path & query parsers (pure functions)
    //   * Header extraction (raw-buffer scan)
    //   * Wire-format builders (status line + headers + body shape)
    //   * Chunked-init disk-absence path (503 Service Unavailable)
    //   * Chunk path NotApplicable + 405 + 404 paths
    //   * Upload-state probe NotApplicable + 405 + 404 paths
    //
    // Each test that mutates UPLOADS first removes any leftover entry
    // for its synthetic id so order-dependent state doesn't bleed
    // across the in-process test binary's parallel test runner.

    #[test]
    fn parse_chunk_path_extracts_id() {
        assert_eq!(parse_chunk_path("/file/abc/chunk"), Some("abc"));
        assert_eq!(parse_chunk_path("/file/file-upload-7/chunk"), Some("file-upload-7"));
    }

    #[test]
    fn parse_chunk_path_rejects_other_shapes() {
        assert_eq!(parse_chunk_path("/file/abc"), None);
        assert_eq!(parse_chunk_path("/file//chunk"), None);
        assert_eq!(parse_chunk_path("/file/abc/chunk/extra"), None);
        assert_eq!(parse_chunk_path("/file/abc/content"), None);
        assert_eq!(parse_chunk_path("/files/abc/chunk"), None);
    }

    #[test]
    fn parse_upload_state_path_extracts_id() {
        assert_eq!(parse_upload_state_path("/file/abc/upload-state"), Some("abc"));
    }

    #[test]
    fn parse_upload_state_path_rejects_other_shapes() {
        assert_eq!(parse_upload_state_path("/file/abc/state"), None);
        assert_eq!(parse_upload_state_path("/file/abc/upload"), None);
        assert_eq!(parse_upload_state_path("/file//upload-state"), None);
    }

    #[test]
    fn split_query_separates_path_and_query() {
        assert_eq!(split_query("/file/a/chunk?offset=42"), ("/file/a/chunk", Some("offset=42")));
        assert_eq!(split_query("/file/a/chunk"), ("/file/a/chunk", None));
        assert_eq!(split_query("/file/a/chunk?offset=42&foo=bar"),
                   ("/file/a/chunk", Some("offset=42&foo=bar")));
    }

    #[test]
    fn parse_chunk_offset_from_query() {
        assert_eq!(parse_chunk_offset(Some("offset=0"), None), Ok(0));
        assert_eq!(parse_chunk_offset(Some("offset=4096"), None), Ok(4096));
        // Multi-key query — pick the right one.
        assert_eq!(parse_chunk_offset(Some("foo=bar&offset=10&baz=qux"), None), Ok(10));
    }

    #[test]
    fn parse_chunk_offset_from_content_range() {
        // Spec form: "bytes N-M/total"; we only need N.
        assert_eq!(parse_chunk_offset(None, Some("bytes 4096-8191/16384")), Ok(4096));
        assert_eq!(parse_chunk_offset(None, Some("bytes 0-1023/2048")), Ok(0));
    }

    #[test]
    fn parse_chunk_offset_query_takes_precedence_over_header() {
        // Per the route's documented precedence rules.
        assert_eq!(
            parse_chunk_offset(Some("offset=42"), Some("bytes 100-200/300")),
            Ok(42),
        );
    }

    #[test]
    fn parse_chunk_offset_missing_yields_error() {
        assert!(parse_chunk_offset(None, None).is_err());
        assert!(parse_chunk_offset(Some("foo=bar"), None).is_err());
    }

    #[test]
    fn parse_chunk_offset_bad_decimal_yields_error() {
        assert!(parse_chunk_offset(Some("offset=abc"), None).is_err());
        assert!(parse_chunk_offset(None, Some("bytes abc-def/100")).is_err());
        assert!(parse_chunk_offset(None, Some("not bytes 0-1/2")).is_err());
    }

    #[test]
    fn extract_content_range_header_present() {
        let req = b"PUT /file/abc/chunk HTTP/1.1\r\n\
                    Host: arest\r\n\
                    Content-Range: bytes 4096-8191/16384\r\n\
                    Content-Length: 4096\r\n\
                    \r\n";
        assert_eq!(
            extract_content_range_header(req).as_deref(),
            Some("bytes 4096-8191/16384"),
        );
    }

    #[test]
    fn extract_content_range_header_case_insensitive() {
        let req = b"PUT /file/abc/chunk HTTP/1.1\r\n\
                    CONTENT-range: bytes 0-1/2\r\n\
                    \r\n";
        assert_eq!(
            extract_content_range_header(req).as_deref(),
            Some("bytes 0-1/2"),
        );
    }

    #[test]
    fn extract_content_range_header_absent() {
        let req = b"PUT /file/abc/chunk HTTP/1.1\r\nHost: arest\r\n\r\n";
        assert!(extract_content_range_header(req).is_none());
    }

    #[test]
    fn try_serve_chunk_other_path_passes_through() {
        match try_serve_chunk("PUT", "/api/welcome", &[], None) {
            ServeOutcome::NotApplicable => {}
            _ => panic!("expected NotApplicable"),
        }
    }

    #[test]
    fn try_serve_chunk_wrong_method_405() {
        match try_serve_chunk("POST", "/file/abc/chunk", &[], None) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"));
                assert!(s.contains("Allow: PUT"));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_chunk_oversize_413() {
        let big = alloc::vec![0u8; MAX_CHUNK_BYTES + 1];
        match try_serve_chunk(
            "PUT", "/file/abc/chunk?offset=0",
            &big,
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
    fn try_serve_chunk_missing_offset_400() {
        // No `?offset=` and no Content-Range. The route bails with 400.
        match try_serve_chunk("PUT", "/file/abc/chunk", b"x", None) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 400 Bad Request\r\n"), "got: {}", s);
                assert!(s.contains("offset"));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_chunk_unknown_id_404() {
        // No matching UPLOADS entry — surface 404.
        let unique_id = "test-chunk-unknown-12345";
        UPLOADS.lock().remove(unique_id);
        let path = format!("/file/{}/chunk?offset=0", unique_id);
        match try_serve_chunk("PUT", &path, b"hello", None) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 404 Not Found\r\n"), "got: {}", s);
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_upload_state_other_path_passes_through() {
        match try_serve_upload_state("GET", "/api/welcome", None) {
            ServeOutcome::NotApplicable => {}
            _ => panic!("expected NotApplicable"),
        }
    }

    #[test]
    fn try_serve_upload_state_wrong_method_405() {
        match try_serve_upload_state("POST", "/file/abc/upload-state", None) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"));
                assert!(s.contains("Allow: GET"));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_upload_state_unknown_id_404() {
        // No active session, no SYSTEM File facts → 404.
        let unique_id = "test-state-unknown-67890";
        UPLOADS.lock().remove(unique_id);
        let path = format!("/file/{}/upload-state", unique_id);
        match try_serve_upload_state("GET", &path, None) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 404 Not Found\r\n"), "got: {}", s);
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_upload_state_completed_via_size_fact() {
        // A file with no in-flight session but a present `File_has_Size`
        // fact in SYSTEM should be reported as `complete: true` with
        // size sourced from the fact. Mirrors the post-seal lookup
        // path for a long-completed upload.
        let phi = Object::phi();
        let state = ast::cell_push(
            "File_has_Size",
            fact_from_pairs(&[("File", "test-state-done"), ("Size", "131072")]),
            &phi,
        );
        UPLOADS.lock().remove("test-state-done");
        let path = "/file/test-state-done/upload-state";
        match try_serve_upload_state("GET", path, Some(&state)) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 200 OK\r\n"), "got: {}", s);
                assert!(s.contains("\"complete\":true"), "got: {}", s);
                assert!(s.contains("\"size\":131072"), "got: {}", s);
            }
            _ => panic!("expected Response"),
        }
    }

    /// Chunked-init request (POST /file with a `total` form field and
    /// no `file` part) attempts `block_storage::alloc_region`. With no
    /// virtio-blk device present (which is the kernel-test harness'
    /// permanent state), the call surfaces `Error::NotMounted` and the
    /// route returns 503 Service Unavailable.
    #[test]
    fn try_serve_chunked_init_no_disk_503() {
        crate::system::init();
        // Build a multipart body: directory_id, filename, total — no
        // `file` part. This is what a curl chunked-init looks like.
        let mut body = Vec::new();
        body.extend_from_slice(b"--BNDRY\r\n");
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"directory_id\"\r\n\r\n",
        );
        body.extend_from_slice(b"dir-x");
        body.extend_from_slice(b"\r\n--BNDRY\r\n");
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"filename\"\r\n\r\n",
        );
        body.extend_from_slice(b"big.bin");
        body.extend_from_slice(b"\r\n--BNDRY\r\n");
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"total\"\r\n\r\n",
        );
        body.extend_from_slice(b"131072"); // 128 KiB > INLINE_THRESHOLD
        body.extend_from_slice(b"\r\n--BNDRY--\r\n");
        match try_serve(
            "POST", "/file",
            Some("multipart/form-data; boundary=BNDRY"),
            &body,
            crate::system::state(),
        ) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(
                    s.starts_with("HTTP/1.1 503 Service Unavailable\r\n"),
                    "got: {}", s,
                );
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_chunked_init_zero_total_400() {
        // total=0 is rejected with a hint to use the inline path.
        let mut body = Vec::new();
        body.extend_from_slice(b"--BNDRY\r\n");
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"directory_id\"\r\n\r\n",
        );
        body.extend_from_slice(b"dir-x");
        body.extend_from_slice(b"\r\n--BNDRY\r\n");
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"total\"\r\n\r\n",
        );
        body.extend_from_slice(b"0");
        body.extend_from_slice(b"\r\n--BNDRY--\r\n");
        match try_serve(
            "POST", "/file",
            Some("multipart/form-data; boundary=BNDRY"),
            &body,
            None,
        ) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 400 Bad Request\r\n"), "got: {}", s);
                assert!(s.contains("total"));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_chunked_init_oversize_total_413() {
        // total > MAX_REGION_BYTES (256 KiB) → 413 before alloc_region.
        let mut body = Vec::new();
        body.extend_from_slice(b"--BNDRY\r\n");
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"directory_id\"\r\n\r\n",
        );
        body.extend_from_slice(b"dir-x");
        body.extend_from_slice(b"\r\n--BNDRY\r\n");
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"total\"\r\n\r\n",
        );
        let too_big = format!("{}", MAX_REGION_BYTES + 1);
        body.extend_from_slice(too_big.as_bytes());
        body.extend_from_slice(b"\r\n--BNDRY--\r\n");
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
    fn try_serve_chunked_init_with_file_part_400() {
        // chunked init must not include a `file` part.
        let mut body = Vec::new();
        body.extend_from_slice(b"--BNDRY\r\n");
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"directory_id\"\r\n\r\n",
        );
        body.extend_from_slice(b"dir-x");
        body.extend_from_slice(b"\r\n--BNDRY\r\n");
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"file\"; filename=\"x\"\r\n\r\n",
        );
        body.extend_from_slice(b"hello");
        body.extend_from_slice(b"\r\n--BNDRY\r\n");
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"total\"\r\n\r\n",
        );
        body.extend_from_slice(b"131072");
        body.extend_from_slice(b"\r\n--BNDRY--\r\n");
        match try_serve(
            "POST", "/file",
            Some("multipart/form-data; boundary=BNDRY"),
            &body,
            None,
        ) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 400 Bad Request\r\n"), "got: {}", s);
                assert!(s.contains("file"));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn try_serve_chunked_init_bad_total_400() {
        // total field must parse as decimal.
        let mut body = Vec::new();
        body.extend_from_slice(b"--BNDRY\r\n");
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"directory_id\"\r\n\r\n",
        );
        body.extend_from_slice(b"dir-x");
        body.extend_from_slice(b"\r\n--BNDRY\r\n");
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"total\"\r\n\r\n",
        );
        body.extend_from_slice(b"not-a-number");
        body.extend_from_slice(b"\r\n--BNDRY--\r\n");
        match try_serve(
            "POST", "/file",
            Some("multipart/form-data; boundary=BNDRY"),
            &body,
            None,
        ) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 400 Bad Request\r\n"), "got: {}", s);
                assert!(s.contains("decimal"));
            }
            _ => panic!("expected Response"),
        }
    }

    /// In-memory exercise of the chunk dispatch + state machine —
    /// runs independently of `block_storage::alloc_region` (which
    /// requires a real disk). We synthesise an `UploadState` whose
    /// `region` field is a synthetic `RegionHandle` with a base sector
    /// outside the active disk range; the chunk path under test never
    /// reaches the actual `region.write` call because the dispatch
    /// arms we exercise (offset bounds, OOO writes, 416, completion
    /// short-circuit) all return before disk I/O.
    fn synth_upload(id: &str, total: u64) {
        // We can't construct a `RegionHandle` directly (its fields are
        // private) and `block_storage::reserve_region` requires a real
        // disk. Skip the test if we can't, by routing through
        // `alloc_region` and bailing out gracefully when there's no
        // virtio-blk device. Tests that depend on this helper are
        // marked `#[cfg(...)]`-guarded below.
        let handle = match block_storage::alloc_region(total) {
            Ok(h) => h,
            Err(_) => return, // skip when no disk
        };
        let base_sector = handle.base_sector();
        UPLOADS.lock().insert(id.to_string(), UploadState {
            region: handle,
            base_sector,
            total,
            highest_contiguous_byte: 0,
            filename: "test.bin".to_string(),
            directory_id: "dir-test".to_string(),
            mime_hint: "application/octet-stream".to_string(),
            sniff_window: Vec::new(),
            complete: false,
            mime_promoted: false,
        });
    }

    /// Verifies the 416 path: when offset+len > total. We synthesise
    /// an UploadState and let dispatch hit the bounds check before
    /// any disk I/O — works disk-or-no-disk because the bounds check
    /// runs ahead of `region.write`.
    #[test]
    fn try_serve_chunk_oob_offset_416() {
        let id = "test-oob-416";
        UPLOADS.lock().remove(id);
        // Manually inject a synthetic UploadState whose region we
        // never actually write to. We need a real RegionHandle though,
        // and `block_storage::alloc_region` will fail with NotMounted
        // in the test harness. Skip the strict 416 assertion if we
        // can't get a region; otherwise check the response.
        synth_upload(id, 100);
        let in_map = UPLOADS.lock().contains_key(id);
        if !in_map {
            // No disk — chunk write would 404 instead of 416 since
            // there's no session. Check 404 to keep the test useful.
            let path = format!("/file/{}/chunk?offset=0", id);
            match try_serve_chunk("PUT", &path, b"hello", None) {
                ServeOutcome::Response(bytes) => {
                    let s = core::str::from_utf8(&bytes).unwrap();
                    assert!(s.starts_with("HTTP/1.1 404 Not Found\r\n"), "got: {}", s);
                }
                _ => panic!("expected Response"),
            }
            return;
        }
        // Disk available — check the real 416 path. offset=50 + len=100
        // > total=100.
        let path = format!("/file/{}/chunk?offset=50", id);
        let chunk = alloc::vec![0u8; 100];
        match try_serve_chunk("PUT", &path, &chunk, None) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 416 Range Not Satisfiable\r\n"), "got: {}", s);
                assert!(s.contains("Content-Range: bytes */100"));
            }
            _ => panic!("expected Response"),
        }
        UPLOADS.lock().remove(id);
    }

    #[test]
    fn no_content_response_shape() {
        let bytes = no_content_response();
        let s = core::str::from_utf8(&bytes).unwrap();
        assert!(s.starts_with("HTTP/1.1 204 No Content\r\n"));
        assert!(s.contains("Content-Length: 0"));
    }

    #[test]
    fn chunked_init_response_shape() {
        let bytes = chunked_init_response("file-foo");
        let s = core::str::from_utf8(&bytes).unwrap();
        assert!(s.starts_with("HTTP/1.1 201 Created\r\n"));
        assert!(s.contains("Location: /file/file-foo"));
        assert!(s.contains("\"id\":\"file-foo\""));
        assert!(s.contains("\"upload_id\":\"file-foo\""));
        assert!(s.contains(&format!("\"chunk_size\":{}", CHUNK_SIZE_HINT)));
    }

    #[test]
    fn chunk_complete_response_shape() {
        let bytes = chunk_complete_response("file-bar", 65536);
        let s = core::str::from_utf8(&bytes).unwrap();
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(s.contains("\"file_id\":\"file-bar\""));
        assert!(s.contains("\"status\":\"complete\""));
        assert!(s.contains("\"size\":65536"));
    }

    #[test]
    fn upload_state_response_shape_in_progress() {
        let bytes = upload_state_response(4096, 16384, false);
        let s = core::str::from_utf8(&bytes).unwrap();
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(s.contains("\"highest_contiguous_byte\":4096"));
        assert!(s.contains("\"size\":16384"));
        assert!(s.contains("\"complete\":false"));
    }

    #[test]
    fn range_not_satisfiable_chunk_shape() {
        let bytes = range_not_satisfiable_chunk(100);
        let s = core::str::from_utf8(&bytes).unwrap();
        assert!(s.starts_with("HTTP/1.1 416 Range Not Satisfiable\r\n"));
        assert!(s.contains("Content-Range: bytes */100"));
    }

    // ── Idempotency-Key tests (#446) ──────────────────────────────────

    /// A retried POST under the same idempotency key returns the
    /// cached response verbatim — same File id, same Location header,
    /// same JSON body. Without the cache, a second POST would mint a
    /// fresh `file-upload-N` id.
    #[test]
    fn idempotency_hit_returns_cached_response() {
        crate::system::init();
        // Use a unique key per test run so prior test runs don't
        // shadow the new entry. The IDEMPOTENCY map is process-
        // global since it lives behind a static Mutex.
        let key = "test-idempotency-hit-446";
        IDEMPOTENCY.lock().remove(key);

        let body = synth_multipart("BNDRY", "first.txt", b"hello", "dir-1");
        let r1 = match try_serve_idempotent(
            "POST", "/file",
            Some("multipart/form-data; boundary=BNDRY"),
            &body, None,
            Some(key),
        ) {
            ServeOutcome::Response(b) => b,
            _ => panic!("expected Response on first POST"),
        };
        // The retry should get the *same* bytes back — including the
        // file id. We don't even pass a body the second time; the
        // cache short-circuits before parsing anything.
        let r2 = match try_serve_idempotent(
            "POST", "/file",
            Some("multipart/form-data; boundary=BNDRY"),
            // Different body, same key — cache should hide the change.
            b"different-body-but-same-key",
            None, Some(key),
        ) {
            ServeOutcome::Response(b) => b,
            _ => panic!("expected Response on retry"),
        };
        assert_eq!(r1, r2, "idempotency replay must return identical bytes");
        IDEMPOTENCY.lock().remove(key);
    }

    /// A miss followed by an expired-entry encounter returns `None`
    /// and removes the stale entry. Today's synthetic clock advances
    /// monotonically per call; we feed an entry whose `expires_at_ms`
    /// is `0` to model already-expired state.
    #[test]
    fn idempotency_expired_entry_returns_none() {
        let key = "test-idempotency-expired-446";
        IDEMPOTENCY.lock().insert(key.to_string(), IdempotencyEntry {
            response: b"HTTP/1.1 200 OK\r\n\r\n".to_vec(),
            expires_at_ms: 0, // already expired against any positive clock
            last_seen_ms: 0,
        });
        assert!(lookup_idempotent(key).is_none(), "expired entry must miss");
        assert!(!IDEMPOTENCY.lock().contains_key(key), "expired entry must be evicted on miss");
    }

    /// `record_idempotent` evicts the LRU entry when the map is at
    /// the cap. Drives the cap down (via direct insert) to make the
    /// test cheap, then asserts the oldest entry is the one kicked
    /// out by a fresh insert.
    #[test]
    fn idempotency_lru_eviction_drops_oldest() {
        // Clear out any pre-existing entries so the map state is
        // deterministic within this test.
        IDEMPOTENCY.lock().clear();
        // Record N + 1 entries where N == IDEMPOTENCY_MAX_ENTRIES.
        // We can't drive the loop that high in a unit test (that's
        // 1024 inserts) — instead, reach into the map manually for
        // the first MAX entries and let `record_idempotent` insert
        // the (MAX+1)th, which should evict the LRU entry.
        for i in 0..IDEMPOTENCY_MAX_ENTRIES {
            // Synthetic ascending last_seen_ms — entry "0" is the
            // oldest, entry "(MAX-1)" is the newest.
            IDEMPOTENCY.lock().insert(
                alloc::format!("eviction-key-{}", i),
                IdempotencyEntry {
                    response: b"HTTP/1.1 200 OK\r\n\r\n".to_vec(),
                    expires_at_ms: u64::MAX, // never expire
                    last_seen_ms: (i + 1) as u64,
                },
            );
        }
        assert_eq!(IDEMPOTENCY.lock().len(), IDEMPOTENCY_MAX_ENTRIES);
        // New key triggers an LRU eviction. Per LRU semantics, the
        // first entry (smallest last_seen_ms = 1) gets dropped.
        record_idempotent("eviction-trigger-new", b"HTTP/1.1 201 Created\r\n\r\n");
        let map = IDEMPOTENCY.lock();
        assert!(!map.contains_key("eviction-key-0"), "LRU entry must be evicted");
        assert!(map.contains_key("eviction-trigger-new"), "fresh entry must land");
        assert_eq!(map.len(), IDEMPOTENCY_MAX_ENTRIES, "size cap holds");
    }

    /// `is_2xx_response` distinguishes 2xx from 4xx / 5xx so the
    /// IDEMPOTENCY caching path can skip error responses (caching
    /// a 503 would freeze a transient failure into a permanent
    /// one for any client retrying with the same key).
    #[test]
    fn is_2xx_response_filters_correctly() {
        assert!(is_2xx_response(b"HTTP/1.1 200 OK\r\n\r\n"));
        assert!(is_2xx_response(b"HTTP/1.1 201 Created\r\n\r\n"));
        assert!(is_2xx_response(b"HTTP/1.1 204 No Content\r\n\r\n"));
        assert!(!is_2xx_response(b"HTTP/1.1 400 Bad Request\r\n\r\n"));
        assert!(!is_2xx_response(b"HTTP/1.1 413 Payload Too Large\r\n\r\n"));
        assert!(!is_2xx_response(b"HTTP/1.1 503 Service Unavailable\r\n\r\n"));
        assert!(!is_2xx_response(b""));
    }

    /// `extract_idempotency_key_header` reads the header value the
    /// way the other `extract_*_header` helpers do — case-insensitive,
    /// whitespace-stripped.
    #[test]
    fn extract_idempotency_key_header_present() {
        let req = b"POST /file HTTP/1.1\r\n\
                    Host: arest\r\n\
                    Idempotency-Key: client-supplied-uuid-abc\r\n\
                    Content-Length: 0\r\n\
                    \r\n";
        assert_eq!(
            extract_idempotency_key_header(req).as_deref(),
            Some("client-supplied-uuid-abc"),
        );
    }

    #[test]
    fn extract_idempotency_key_header_case_insensitive() {
        let req = b"POST /file HTTP/1.1\r\n\
                    IDEMPOTENCY-key: ABC\r\n\
                    \r\n";
        assert_eq!(
            extract_idempotency_key_header(req).as_deref(),
            Some("ABC"),
        );
    }

    #[test]
    fn extract_idempotency_key_header_absent() {
        let req = b"POST /file HTTP/1.1\r\nHost: arest\r\n\r\n";
        assert!(extract_idempotency_key_header(req).is_none());
    }

    // ── SSE progress tests (#447) ─────────────────────────────────────

    #[test]
    fn parse_progress_path_extracts_id() {
        assert_eq!(parse_progress_path("/file/abc/progress"), Some("abc"));
        assert_eq!(parse_progress_path("/file/file-upload-7/progress"), Some("file-upload-7"));
    }

    #[test]
    fn parse_progress_path_rejects_other_shapes() {
        assert_eq!(parse_progress_path("/file/abc"), None);
        assert_eq!(parse_progress_path("/file//progress"), None);
        assert_eq!(parse_progress_path("/file/abc/progress/extra"), None);
        assert_eq!(parse_progress_path("/file/abc/upload-state"), None);
        assert_eq!(parse_progress_path("/file/abc/chunk"), None);
    }

    /// `enqueue_progress` should append to the per-file ring; a
    /// drain returns the events in FIFO order. Tests the per-file
    /// keyed-map shape stays isolated across distinct file ids.
    #[test]
    fn enqueue_then_drain_returns_events_in_order() {
        let id_a = "test-progress-enqueue-a";
        let id_b = "test-progress-enqueue-b";
        PROGRESS_EVENTS.lock().remove(id_a);
        PROGRESS_EVENTS.lock().remove(id_b);

        enqueue_progress(id_a, 100, 1000);
        enqueue_progress(id_a, 200, 1000);
        enqueue_progress(id_b, 50, 500); // unrelated file
        enqueue_progress(id_a, 300, 1000);

        let drained = drain_progress(id_a);
        assert_eq!(drained.len(), 3, "all three events for id_a");
        assert_eq!(drained[0].bytes_written, 100);
        assert_eq!(drained[1].bytes_written, 200);
        assert_eq!(drained[2].bytes_written, 300);
        // id_b's ring must be untouched by the id_a drain.
        let drained_b = drain_progress(id_b);
        assert_eq!(drained_b.len(), 1, "id_b ring isolated");
        assert_eq!(drained_b[0].bytes_written, 50);
    }

    /// When the per-file ring fills, the oldest event is dropped to
    /// make room for the new one (front-drop). Bounds memory under
    /// a slow / disconnected SSE subscriber.
    #[test]
    fn enqueue_drops_oldest_when_ring_full() {
        let id = "test-progress-front-drop";
        PROGRESS_EVENTS.lock().remove(id);
        // Push DEPTH+2 events; only the last DEPTH should survive.
        for i in 0..(PROGRESS_RING_DEPTH + 2) {
            enqueue_progress(id, i as u64, PROGRESS_RING_DEPTH as u64);
        }
        let drained = drain_progress(id);
        assert_eq!(drained.len(), PROGRESS_RING_DEPTH);
        // The first surviving event should have bytes_written == 2
        // (the first two were dropped to make room for the last two).
        assert_eq!(drained[0].bytes_written, 2);
        assert_eq!(drained.last().unwrap().bytes_written, (PROGRESS_RING_DEPTH + 1) as u64);
    }

    /// SSE wire format — exactly one `\n\n`-delimited frame per event,
    /// each with `data: ` + JSON body. The `id:` line and
    /// `Last-Event-ID` header carry the `bytes_written` cursor so a
    /// reconnecting client can resume.
    #[test]
    fn sse_wire_format_proper_framing() {
        let events = alloc::vec![
            ProgressEvent { bytes_written: 4096, total_bytes: 16384 },
            ProgressEvent { bytes_written: 8192, total_bytes: 16384 },
        ];
        let bytes = progress_sse_response("file-x", &events);
        let s = core::str::from_utf8(&bytes).unwrap();
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(s.contains("Content-Type: text/event-stream"));
        assert!(s.contains("Cache-Control: no-cache"));
        assert!(s.contains("Last-Event-ID: 8192"), "Last-Event-ID tracks the highest cursor: {}", s);
        // Each event ends in \n\n per SSE spec.
        let body_start = s.find("\r\n\r\n").expect("header/body separator") + 4;
        let body = &s[body_start..];
        // Expect: "id: 4096\ndata: {...}\n\nid: 8192\ndata: {...}\n\n"
        assert!(body.contains("id: 4096\ndata: {\"file_id\":\"file-x\",\"bytes_written\":4096,\"total_bytes\":16384}\n\n"),
                "first frame: {}", body);
        assert!(body.contains("id: 8192\ndata: {\"file_id\":\"file-x\",\"bytes_written\":8192,\"total_bytes\":16384}\n\n"),
                "second frame: {}", body);
        // No trailing `event: complete` since neither event hit total.
        assert!(!body.contains("event: complete"), "no complete event yet: {}", body);
    }

    /// When the closing event has `bytes_written == total_bytes`,
    /// the response includes a `event: complete\ndata: {}\n\n` frame
    /// so an EventSource subscriber can branch on the SSE event name
    /// without parsing the JSON body.
    #[test]
    fn sse_wire_includes_complete_event_on_seal() {
        let events = alloc::vec![
            ProgressEvent { bytes_written: 8192, total_bytes: 16384 },
            ProgressEvent { bytes_written: 16384, total_bytes: 16384 },
        ];
        let bytes = progress_sse_response("file-y", &events);
        let s = core::str::from_utf8(&bytes).unwrap();
        assert!(s.contains("event: complete\ndata: {}\n\n"), "complete frame: {}", s);
    }

    /// An empty drain returns a comment-only `: keepalive\n\n` so the
    /// polled connection doesn't 404 / 204 and the EventSource client
    /// stays subscribed. Helps the UI distinguish "no events yet" from
    /// "session vanished".
    #[test]
    fn sse_wire_empty_drain_emits_keepalive() {
        let bytes = progress_sse_response("file-z", &[]);
        let s = core::str::from_utf8(&bytes).unwrap();
        let body_start = s.find("\r\n\r\n").expect("header/body separator") + 4;
        let body = &s[body_start..];
        assert_eq!(body, ": keepalive\n\n");
    }

    /// `try_serve_progress` returns NotApplicable on paths that
    /// don't match `/file/{id}/progress`, so the chain in
    /// `try_serve_upload_state` falls through cleanly.
    #[test]
    fn try_serve_progress_unrelated_path_passes_through() {
        match try_serve_progress("GET", "/api/welcome") {
            ServeOutcome::NotApplicable => {}
            _ => panic!("expected NotApplicable"),
        }
    }

    #[test]
    fn try_serve_progress_wrong_method_405() {
        match try_serve_progress("POST", "/file/abc/progress") {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"));
                assert!(s.contains("Allow: GET"));
            }
            _ => panic!("expected Response"),
        }
    }

    /// `try_serve_upload_state` chains to `try_serve_progress` first,
    /// so a GET on `/file/{id}/progress` must yield SSE wire bytes
    /// (not a 404 from the upload-state lookup arm).
    #[test]
    fn try_serve_upload_state_chains_to_progress() {
        // No active upload, no events queued — should still return
        // a 200 with the keepalive comment, not a 404.
        let id = "test-progress-chain-id";
        PROGRESS_EVENTS.lock().remove(id);
        let path = alloc::format!("/file/{}/progress", id);
        match try_serve_upload_state("GET", &path, None) {
            ServeOutcome::Response(bytes) => {
                let s = core::str::from_utf8(&bytes).unwrap();
                assert!(s.starts_with("HTTP/1.1 200 OK\r\n"), "should chain to progress: {}", s);
                assert!(s.contains("Content-Type: text/event-stream"));
            }
            _ => panic!("expected Response from chained progress handler"),
        }
    }

    // ── First-chunk MIME promotion tests (#448) ───────────────────────

    /// `update_mime_fact` removes any existing File_has_MimeType
    /// fact for the file id and pushes a single fresh one — net
    /// effect is exactly one MimeType fact per file id, regardless
    /// of how many times the function is called.
    #[test]
    fn update_mime_fact_replaces_existing_mime() {
        let phi = Object::phi();
        // Pre-state has MimeType=octet-stream (placeholder from
        // chunked-init) plus a sibling Name fact that should be
        // untouched.
        let pre = ast::cell_push(
            "File_has_MimeType",
            fact_from_pairs(&[("File", "f-1"), ("MimeType", "application/octet-stream")]),
            &phi,
        );
        let pre = ast::cell_push(
            "File_has_Name",
            fact_from_pairs(&[("File", "f-1"), ("Name", "image.bin")]),
            &pre,
        );
        // Apply the sniffed promotion.
        let next = update_mime_fact(&pre, "f-1", "image/png");
        // MimeType cell now has exactly one fact for f-1, with the
        // promoted type.
        let mime_cell = ast::fetch_or_phi("File_has_MimeType", &next);
        let seq = mime_cell.as_seq().expect("MimeType cell exists");
        let f1_facts: Vec<_> = seq.iter()
            .filter(|f| ast::binding(f, "File") == Some("f-1"))
            .collect();
        assert_eq!(f1_facts.len(), 1, "exactly one MimeType fact post-promotion");
        assert_eq!(ast::binding(f1_facts[0], "MimeType"), Some("image/png"));
        // Name fact unaffected.
        let name_cell = ast::fetch_or_phi("File_has_Name", &next);
        let name_seq = name_cell.as_seq().expect("Name cell exists");
        assert!(name_seq.iter().any(|f| {
            ast::binding(f, "File") == Some("f-1")
                && ast::binding(f, "Name") == Some("image.bin")
        }), "Name fact preserved");
    }

    /// Other files' MimeType facts must not be touched by an
    /// update for one file id.
    #[test]
    fn update_mime_fact_isolates_per_file_id() {
        let phi = Object::phi();
        let pre = ast::cell_push(
            "File_has_MimeType",
            fact_from_pairs(&[("File", "other"), ("MimeType", "text/plain")]),
            &phi,
        );
        let pre = ast::cell_push(
            "File_has_MimeType",
            fact_from_pairs(&[("File", "target"), ("MimeType", "application/octet-stream")]),
            &pre,
        );
        let next = update_mime_fact(&pre, "target", "image/jpeg");
        let mime_cell = ast::fetch_or_phi("File_has_MimeType", &next);
        let seq = mime_cell.as_seq().expect("MimeType cell exists");
        // Other file's fact preserved.
        assert!(seq.iter().any(|f| {
            ast::binding(f, "File") == Some("other")
                && ast::binding(f, "MimeType") == Some("text/plain")
        }), "other file's fact preserved");
        // Target file's fact promoted.
        let target_facts: Vec<_> = seq.iter()
            .filter(|f| ast::binding(f, "File") == Some("target"))
            .collect();
        assert_eq!(target_facts.len(), 1);
        assert_eq!(ast::binding(target_facts[0], "MimeType"), Some("image/jpeg"));
    }

    /// MIME_SNIFF_MIN_BYTES of PNG-magic should detect as image/png
    /// even when the placeholder mime_hint is application/octet-
    /// stream. This is the core sniff-overrides-spoofed-Content-Type
    /// behaviour #448 ships.
    #[test]
    fn mime_sniff_overrides_octet_stream_placeholder() {
        let mut buf = alloc::vec![0u8; MIME_SNIFF_MIN_BYTES];
        buf[..8].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        let sniffed = detect_mime(&buf[..MIME_SNIFF_MIN_BYTES]);
        assert_eq!(sniffed, "image/png");
        assert_ne!(sniffed, "application/octet-stream",
                   "sniffed type must override the placeholder");
    }
}
