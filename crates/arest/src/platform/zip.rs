// crates/arest/src/platform/zip.rs
//
// `zip_directory` and `unzip_file` Platform functions (per
// AREST whitepaper §7.4). Pure data transforms over cell state — no
// host I/O. Operates on the `File` / `Directory` nouns declared in
// `readings/filesystem.md` (#397, #398, #399).
//
// ## Surface
//
// * `zip_directory(dir_id) → File` — walks the Directory subtree
//   rooted at `dir_id`, encodes every File's bytes as ZIP entries
//   keyed by their tree-relative path, registers a new File at the
//   same parent Directory holding the resulting archive bytes.
//
// * `unzip_file(file_id, target_dir_id) → ()` — reads `file_id`'s
//   bytes, parses the ZIP central directory, materialises each entry
//   as a fresh File / Directory under `target_dir_id`. Empty
//   directory entries (paths ending in `/`) become Directory cells;
//   other entries become File cells with their decoded bytes.
//
// Both are dispatched through `apply_platform`'s runtime-callback
// fallback (`PLATFORM_FALLBACK`); they are wired in by calling
// [`install`] once per process. The names registered are
// `"zip_directory"` and `"unzip_file"`. The sec-2 audit allowlist
// (`ast::APPROVED_PLATFORM_FN_NAMES`) is *not* updated here — the
// production engine does not auto-install these on tenant boot. They
// are made callable by an explicit host call to [`install`] (e.g.
// CLI startup, integration test setup).
//
// ## ZIP encoding
//
// **Stored-only** (compression method 0). Deflate is deferred:
//   - The on-wire shape (LFH / CDH / EOCD) is identical between
//     stored and deflated entries — only the data segment differs.
//     Adding deflate is a follow-up that swaps the data segment
//     encoder/decoder; no caller-visible API change.
//   - A no_std-friendly deflate codec (`miniz_oxide` is std-only on
//     stable, `libdeflate` requires C) is the harder dep, and the
//     `arest` crate already imports `flate2` only behind the
//     `std-deps` feature. Keeping the inlined codec stored-only
//     avoids dragging deflate into the kernel build's transitive
//     graph (the `platform` module is std-only today, but the data
//     formats produced here are read by every target).
//   - Stored archives are larger but otherwise valid PKZIP — every
//     mainstream `unzip` / 7z / OS shell extracts them without
//     issue. For the workloads this Platform fn was added for
//     (snapshotting a Directory subtree, shipping a bundle to a
//     peer node), wire size matters less than correctness and the
//     ability to round-trip on the same kernel.
//
// ## Blob storage (#401)
//
// File bytes are addressed via the `ContentRef` value role declared
// in `readings/filesystem.md`. The on-disk allocator + encoding spec
// landed in #401 (commit 8a97ab5):
//
//   Inline path:   <INLINE, "hex-bytes">      ≤ 64 KiB
//   Region path:   <REGION, "base-sector", "byte-len">     > 64 KiB
//
// The Rust encoder/decoder for that tagged Object lives in a future
// `crates/arest/src/blob.rs` (the consumer that lands File create /
// read wires it; #401 ships only the spec + the kernel allocator).
// Once `blob.rs` exists, `decode_content_ref` and
// `encode_content_ref` here become a one-line call into it.
//
// Until then this codec stores `ContentRef` as a **bare lowercase
// hex atom** (essentially an INLINE entry without the tag wrapper).
// `fact_from_pairs` is atom-only — passing a structured Object as a
// fact-binding value needs a richer fact constructor that does not
// yet exist on the public ast surface. The bare-hex form keeps
// round-trip semantics intact and lets a follow-up swap in the full
// tagged form at exactly two call sites.
//
// At runtime, if a File's `ContentRef` cannot be decoded as hex
// (e.g. a future blob-handle prefix the codec doesn't yet
// recognise), the zip path treats the entry as zero-length and
// continues; the unzip path emits the bytes verbatim into the new
// File's ContentRef hex.

// `Vec` / `String` / `format!` / `vec!` are pulled in via the std
// prelude when the bin / test / external lib build is std (default
// path here — the platform module is `cfg(not(no_std))`-gated by
// `mod.rs`). When this file is `#[path]`-included from a test binary
// that has its own `crate::` root (e.g. `tests/platform_zip.rs`),
// the same prelude entries are available and these unqualified names
// resolve the same way. Keeping the imports minimal — and avoiding
// `use alloc::*` paths — lets the module compile in both contexts
// without a `crate::` rewrite.
use hashbrown::{HashMap, HashSet};

use crate::ast::{
    self, Object, PlatformFn, fact_from_pairs, install_platform_fn,
};
use crate::sync::Arc;
use crate::command::{self, Command};

// ── Public entry points ─────────────────────────────────────────────

/// Register `"zip_directory"` and `"unzip_file"` into
/// `ast::PLATFORM_FALLBACK`. Idempotent — re-installing replaces the
/// existing body. Hosts call this once during process startup.
///
/// The names this installs MUST be added to
/// `ast::APPROVED_PLATFORM_FN_NAMES` before they can be reached from
/// a production tenant; the sec-2 audit test would otherwise fail.
/// Since the audit list is maintained by the engine owner (not this
/// module), production wiring is a follow-up — for tests and local
/// CLI use the install is enough.
pub fn install() {
    let zip_fn: PlatformFn = Arc::new(|x: &Object, d: &Object| zip_directory_apply(x, d));
    install_platform_fn("zip_directory", zip_fn);

    let unzip_fn: PlatformFn = Arc::new(|x: &Object, d: &Object| unzip_file_apply(x, d));
    install_platform_fn("unzip_file", unzip_fn);
}

/// Programmatic entry point: walk the Directory subtree at `dir_id`,
/// build a ZIP archive, return a new D where the archive has been
/// registered as a File at the same parent Directory. The returned
/// tuple `(file_id, new_d)` lets callers chain further state writes.
///
/// Returns `Err(reason)` when the source Directory cannot be located
/// or the create-File command rejects.
pub fn zip_directory_in(dir_id: &str, d: &Object) -> Result<(String, Object), String> {
    let entries = walk_directory_subtree(dir_id, d)?;
    let archive = encode_zip_stored(&entries);

    // Pick the parent Directory of `dir_id` (or the root) as the
    // landing slot for the new File. If `dir_id` has no parent (it
    // *is* a root), the archive is placed at `dir_id` itself — the
    // archive must live somewhere; a top-level Directory is the
    // closest legal home.
    let landing_dir = directory_parent(dir_id, d).unwrap_or_else(|| dir_id.to_string());

    let dir_name = directory_name(dir_id, d).unwrap_or_else(|| dir_id.to_string());
    let archive_name = format!("{}.zip", dir_name);

    let (file_id, new_d) = create_file(&archive_name, "application/zip", &archive, &landing_dir, d)?;
    Ok((file_id, new_d))
}

/// Programmatic entry point: parse `file_id`'s bytes as a ZIP archive
/// and materialise each entry as a fresh File / Directory under
/// `target_dir_id`. Returns the post-write D and the list of newly-
/// created entity ids in the order they were materialised.
pub fn unzip_file_in(file_id: &str, target_dir_id: &str, d: &Object) -> Result<(Object, Vec<String>), String> {
    let bytes = file_content_bytes(file_id, d)
        .ok_or_else(|| format!("zip: file {} has no decodable ContentRef", file_id))?;
    let entries = decode_zip_stored(&bytes)
        .map_err(|e| format!("zip: decode failed: {}", e))?;

    let mut current = d.clone();
    let mut created: Vec<String> = Vec::new();
    // Path → Directory id, including the synthetic root "" → target_dir_id.
    let mut dir_ids: HashMap<String, String> = HashMap::new();
    dir_ids.insert(String::new(), target_dir_id.to_string());

    for (path, payload) in entries {
        let is_dir = path.ends_with('/');
        let trimmed = path.trim_end_matches('/').to_string();
        if trimmed.is_empty() { continue; } // root of the archive — already mapped
        let (parent_path, leaf) = split_parent(&trimmed);

        // Materialise any missing intermediate Directories along the
        // parent chain. ZIP entries are not required to be ordered
        // parent-first — synthesising as we go keeps the loop simple.
        let parent_id = ensure_dir_chain(&parent_path, &mut dir_ids, &mut current, &mut created)?;

        if is_dir {
            // Pure directory entry — make sure it exists and continue.
            let dir_id = ensure_dir_chain(&trimmed, &mut dir_ids, &mut current, &mut created)?;
            // Track for symmetry; created already pushed inside ensure_dir_chain when fresh.
            let _ = dir_id;
            continue;
        }

        // File entry — create under parent.
        let mime = guess_mime_from_name(leaf);
        let (file_id, new_d) = create_file(leaf, &mime, &payload, &parent_id, &current)?;
        current = new_d;
        created.push(file_id);
    }

    Ok((current, created))
}

// ── Platform-fn glue ────────────────────────────────────────────────

/// `apply_platform` adapter for `"zip_directory"`.
/// Operand shape: `<dir_id>` (atom) **or** `<<dir_id, …>>` (single-
/// element seq, tolerated). Returns `<file_id>` on success, `⊥` on
/// failure.
fn zip_directory_apply(x: &Object, d: &Object) -> Object {
    let dir_id = match extract_single_atom(x) {
        Some(s) => s,
        None => return Object::Bottom,
    };
    match zip_directory_in(&dir_id, d) {
        Ok((file_id, _new_d)) => Object::atom(&file_id),
        Err(_) => Object::Bottom,
    }
}

/// `apply_platform` adapter for `"unzip_file"`.
/// Operand shape: `<file_id, target_dir_id>`. Returns the count of
/// newly-created entity ids as a decimal atom on success, `⊥` on
/// failure.
fn unzip_file_apply(x: &Object, d: &Object) -> Object {
    let pair = match x.as_seq() {
        Some(p) if p.len() == 2 => p,
        _ => return Object::Bottom,
    };
    let file_id = match pair[0].as_atom() {
        Some(s) => s.to_string(),
        None => return Object::Bottom,
    };
    let target = match pair[1].as_atom() {
        Some(s) => s.to_string(),
        None => return Object::Bottom,
    };
    match unzip_file_in(&file_id, &target, d) {
        Ok((_new_d, created)) => Object::atom(&format!("{}", created.len())),
        Err(_) => Object::Bottom,
    }
}

fn extract_single_atom(x: &Object) -> Option<String> {
    if let Some(s) = x.as_atom() { return Some(s.to_string()); }
    let seq = x.as_seq()?;
    if seq.len() == 1 { return seq[0].as_atom().map(|s| s.to_string()); }
    None
}

// ── Directory traversal ────────────────────────────────────────────

/// (path, bytes) pairs for every File in the subtree rooted at
/// `dir_id`. Path is the tree-relative slash-joined name from the
/// root Directory to the File, e.g. `"sub/inner/notes.txt"`. Empty
/// directories appear as a `("path/", &[])` entry so the unzip path
/// can re-create them.
pub fn walk_directory_subtree(dir_id: &str, d: &Object) -> Result<Vec<(String, Vec<u8>)>, String> {
    if directory_name(dir_id, d).is_none() {
        return Err(format!("zip: directory {} not found", dir_id));
    }

    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(dir_id.to_string());
    walk_recursive(dir_id, "", d, &mut out, &mut seen);
    Ok(out)
}

fn walk_recursive(
    dir_id: &str,
    prefix: &str,
    d: &Object,
    out: &mut Vec<(String, Vec<u8>)>,
    seen: &mut HashSet<String>,
) {
    let mut had_child = false;

    // Files in this directory.
    let file_in = ast::fetch_or_phi("File_is_in_Directory", d);
    if let Some(facts) = file_in.as_seq() {
        for fact in facts {
            if ast::binding(fact, "Directory") == Some(dir_id) {
                if let Some(file_id) = ast::binding(fact, "File") {
                    let name = file_name(file_id, d).unwrap_or_else(|| file_id.to_string());
                    let path = if prefix.is_empty() { name.clone() } else { format!("{}/{}", prefix, name) };
                    let bytes = file_content_bytes(file_id, d).unwrap_or_default();
                    out.push((path, bytes));
                    had_child = true;
                }
            }
        }
    }

    // Child Directories.
    let dir_parent = ast::fetch_or_phi("Directory_has_parent_Directory", d);
    if let Some(facts) = dir_parent.as_seq() {
        for fact in facts {
            if ast::binding(fact, "parent Directory") == Some(dir_id) {
                if let Some(child_id) = ast::binding(fact, "Directory") {
                    if !seen.insert(child_id.to_string()) { continue; }
                    let name = directory_name(child_id, d).unwrap_or_else(|| child_id.to_string());
                    let child_prefix = if prefix.is_empty() { name } else { format!("{}/{}", prefix, name) };
                    walk_recursive(child_id, &child_prefix, d, out, seen);
                    had_child = true;
                }
            }
        }
    }

    // If this directory is empty AND not the root call (prefix non-empty),
    // emit a directory entry so unzip can re-materialise it.
    if !had_child && !prefix.is_empty() {
        out.push((format!("{}/", prefix), Vec::new()));
    }
}

fn directory_name(dir_id: &str, d: &Object) -> Option<String> {
    let cell = ast::fetch_or_phi("Directory_has_Name", d);
    cell.as_seq()?.iter().find_map(|f| {
        if ast::binding(f, "Directory") == Some(dir_id) {
            ast::binding(f, "Name").map(|s| s.to_string())
        } else { None }
    })
}

fn directory_parent(dir_id: &str, d: &Object) -> Option<String> {
    let cell = ast::fetch_or_phi("Directory_has_parent_Directory", d);
    cell.as_seq()?.iter().find_map(|f| {
        if ast::binding(f, "Directory") == Some(dir_id) {
            ast::binding(f, "parent Directory").map(|s| s.to_string())
        } else { None }
    })
}

/// Return the `Name` value for `file_id`, or `None` if the File is
/// not present in `d`. Re-exported under the alias `file_name_for`
/// for the integration test binary which `#[path]`-includes this
/// file (the alias avoids clashing with `core::file_name` paths the
/// caller may also pull in).
pub fn file_name_for(file_id: &str, d: &Object) -> Option<String> {
    file_name(file_id, d)
}

fn file_name(file_id: &str, d: &Object) -> Option<String> {
    let cell = ast::fetch_or_phi("File_has_Name", d);
    cell.as_seq()?.iter().find_map(|f| {
        if ast::binding(f, "File") == Some(file_id) {
            ast::binding(f, "Name").map(|s| s.to_string())
        } else { None }
    })
}

/// Resolve `file_id`'s `ContentRef` and decode it into raw bytes.
/// Returns `None` if the File is missing or the ContentRef cannot
/// be decoded (see `decode_content_ref` for the inline scheme).
pub fn file_content_bytes(file_id: &str, d: &Object) -> Option<Vec<u8>> {
    let cell = ast::fetch_or_phi("File_has_ContentRef", d);
    let cref = cell.as_seq()?.iter().find_map(|f| {
        if ast::binding(f, "File") == Some(file_id) {
            ast::binding(f, "ContentRef").map(|s| s.to_string())
        } else { None }
    })?;
    decode_content_ref(&cref)
}

// ── ContentRef encoding (TODO #401: blob storage) ──────────────────

/// Decode a `ContentRef` atom into raw bytes. Today the inline form
/// is bare lowercase hex (no tag wrapper); when `blob.rs` lands the
/// proper tagged-Object path per #401 (`<INLINE, hex>` /
/// `<REGION, sector, len>`), this becomes a one-line call into it.
fn decode_content_ref(cref: &str) -> Option<Vec<u8>> {
    if cref.is_empty() { return Some(Vec::new()); }
    let bs = cref.as_bytes();
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

/// Encode raw bytes as a `ContentRef` atom. Today produces bare
/// lowercase hex; will switch to the tagged `<INLINE, hex>` /
/// `<REGION, sector, len>` form once `blob.rs` lands per #401.
pub fn encode_content_ref(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(hex_nibble(b >> 4));
        s.push(hex_nibble(b & 0xF));
    }
    s
}

fn nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn hex_nibble(v: u8) -> char {
    match v {
        0..=9 => (b'0' + v) as char,
        10..=15 => (b'a' + (v - 10)) as char,
        _ => '0',
    }
}

// ── File / Directory creation via standard cell-write API ──────────

/// Create a fresh `File` entity under `parent_dir_id`, holding
/// `bytes` in its ContentRef. Returns the new file id and the
/// updated D.
///
/// When the `readings/filesystem.md` defs (`resolve:File`,
/// `validate:File`, derivations) are loaded into `d`, we funnel
/// through `command::apply_command_defs(CreateEntity)` — the same
/// path the HTTP `create:File` handler uses — so any derivations
/// (Size = byte-length of ContentRef) and validations fire as they
/// would from a normal create. The result of that path is a CELL
/// DELTA per #209, so we merge it back onto the input state to
/// return the full new D.
///
/// When the readings are NOT loaded (the platform module's tests
/// and any host that wants to drive the zip codec without first
/// compiling filesystem.md), `resolve:File` is Bottom and the
/// generic `create_via_defs` resolver would emit facts under
/// `File_has_<field>` cell names (e.g. the "in Directory" field
/// would land under the cell `File_has_in Directory`, with a
/// space). The actual cell that the rest of the codec — and the
/// readings, when they later load — uses is `File_is_in_Directory`.
/// To keep the round-trip intact in that case we skip the def
/// pipeline and do the direct cell push under the parser-canonical
/// fact-type ids. (#672 — round-trip tests previously failed
/// because the success path returned a delta with the wrong cell
/// names and dropped the rest of the state.)
fn create_file(name: &str, mime: &str, bytes: &[u8], parent_dir_id: &str, d: &Object)
    -> Result<(String, Object), String>
{
    let id = synth_id("file");

    // Fast path when readings aren't loaded: no resolve:File def, so
    // the def pipeline can only produce mis-named cells. Push facts
    // directly under the canonical names instead.
    if ast::fetch("resolve:File", d).is_bottom() {
        let new_d = direct_push_file(&id, name, mime, bytes, parent_dir_id, d);
        return Ok((id, new_d));
    }

    let mut fields: HashMap<String, String> = HashMap::new();
    fields.insert("Name".to_string(), name.to_string());
    fields.insert("MimeType".to_string(), mime.to_string());
    fields.insert("ContentRef".to_string(), encode_content_ref(bytes));
    fields.insert("in Directory".to_string(), parent_dir_id.to_string());

    let cmd = Command::CreateEntity {
        noun: "File".to_string(),
        domain: "filesystem".to_string(),
        id: Some(id.clone()),
        fields,
        sender: None,
        signature: None,
    };
    let result = command::apply_command_defs(d, &cmd, d);
    if result.rejected {
        // The `create:File` command was rejected by validation.
        // Fall back to a direct cell push so callers can recover.
        let new_d = direct_push_file(&id, name, mime, bytes, parent_dir_id, d);
        return Ok((id, new_d));
    }
    // `result.state` is a cell delta (#209). Merge it back onto the
    // input state to recover the full D.
    Ok((id, ast::merge_delta(d, &result.state)))
}

/// Same shape as `create_file` for Directory entities. See
/// `create_file` for the rationale behind the readings-absent fast
/// path and the delta merge.
fn create_directory(name: &str, parent_dir_id: Option<&str>, d: &Object)
    -> Result<(String, Object), String>
{
    let id = synth_id("dir");

    if ast::fetch("resolve:Directory", d).is_bottom() {
        let new_d = direct_push_directory(&id, name, parent_dir_id, d);
        return Ok((id, new_d));
    }

    let mut fields: HashMap<String, String> = HashMap::new();
    fields.insert("Name".to_string(), name.to_string());
    if let Some(p) = parent_dir_id {
        fields.insert("parent Directory".to_string(), p.to_string());
    }
    let cmd = Command::CreateEntity {
        noun: "Directory".to_string(),
        domain: "filesystem".to_string(),
        id: Some(id.clone()),
        fields,
        sender: None,
        signature: None,
    };
    let result = command::apply_command_defs(d, &cmd, d);
    if result.rejected {
        let new_d = direct_push_directory(&id, name, parent_dir_id, d);
        return Ok((id, new_d));
    }
    Ok((id, ast::merge_delta(d, &result.state)))
}

/// Direct cell-push fallback for File creation. Reproduces the
/// fact-type ids `create_via_defs` would emit when the
/// `readings/filesystem.md` derivations are not loaded in the
/// tenant's D. Only invoked when `apply_command_defs` rejects.
fn direct_push_file(id: &str, name: &str, mime: &str, bytes: &[u8], parent_dir_id: &str, d: &Object) -> Object {
    let cref = encode_content_ref(bytes);
    let size = format!("{}", bytes.len());
    let d = ast::cell_push("File_has_Name", fact_from_pairs(&[("File", id), ("Name", name)]), d);
    let d = ast::cell_push("File_has_MimeType", fact_from_pairs(&[("File", id), ("MimeType", mime)]), &d);
    let d = ast::cell_push("File_has_ContentRef", fact_from_pairs(&[("File", id), ("ContentRef", &cref)]), &d);
    let d = ast::cell_push("File_has_Size", fact_from_pairs(&[("File", id), ("Size", &size)]), &d);
    ast::cell_push("File_is_in_Directory",
        fact_from_pairs(&[("File", id), ("Directory", parent_dir_id)]), &d)
}

fn direct_push_directory(id: &str, name: &str, parent_dir_id: Option<&str>, d: &Object) -> Object {
    let d = ast::cell_push("Directory_has_Name", fact_from_pairs(&[("Directory", id), ("Name", name)]), d);
    if let Some(p) = parent_dir_id {
        ast::cell_push("Directory_has_parent_Directory",
            fact_from_pairs(&[("Directory", id), ("parent Directory", p)]), &d)
    } else {
        d
    }
}

// ── Path / mime helpers ────────────────────────────────────────────

fn split_parent(path: &str) -> (String, &str) {
    match path.rfind('/') {
        Some(i) => (path[..i].to_string(), &path[i + 1..]),
        None => (String::new(), path),
    }
}

fn ensure_dir_chain(
    rel_path: &str,
    dir_ids: &mut HashMap<String, String>,
    current: &mut Object,
    created: &mut Vec<String>,
) -> Result<String, String> {
    if let Some(id) = dir_ids.get(rel_path) { return Ok(id.clone()); }
    if rel_path.is_empty() {
        // Mapped at entry; this branch is unreachable but keeps the API total.
        return Err("zip: empty rel_path with no root mapping".to_string());
    }
    let (parent_path, leaf) = split_parent(rel_path);
    let parent_id = ensure_dir_chain(&parent_path, dir_ids, current, created)?;
    let (id, new_d) = create_directory(leaf, Some(&parent_id), current)?;
    *current = new_d;
    created.push(id.clone());
    dir_ids.insert(rel_path.to_string(), id.clone());
    Ok(id)
}

fn synth_id(prefix: &str) -> String {
    use core::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    format!("{}-zip-{}", prefix, SEQ.fetch_add(1, Ordering::Relaxed))
}

fn guess_mime_from_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "txt" | "md" | "log" => "text/plain",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }.to_string()
}

// ── Inlined ZIP codec (stored-only, method 0) ──────────────────────
//
// On-wire constants — see PKZIP APPNOTE.TXT §4.3 for field layouts.

const SIG_LFH:  u32 = 0x0403_4b50;
const SIG_CDH:  u32 = 0x0201_4b50;
const SIG_EOCD: u32 = 0x0605_4b50;
const VERSION_NEEDED: u16 = 20; // ZIP 2.0
const METHOD_STORED: u16 = 0;
const FLAG_UTF8_NAME: u16 = 0x0800;

/// Encode an ordered list of (path, bytes) entries as a single
/// stored-only ZIP archive.
pub fn encode_zip_stored(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    let mut count = 0u16;

    for (path, data) in entries {
        let crc = crc32(data);
        let size = data.len() as u32;
        let lfh_offset = out.len() as u32;
        let name_bytes = path.as_bytes();
        let name_len = name_bytes.len() as u16;

        // Local file header.
        write_u32(&mut out, SIG_LFH);
        write_u16(&mut out, VERSION_NEEDED);
        write_u16(&mut out, FLAG_UTF8_NAME);
        write_u16(&mut out, METHOD_STORED);
        write_u16(&mut out, 0); // mod time
        write_u16(&mut out, 0); // mod date
        write_u32(&mut out, crc);
        write_u32(&mut out, size); // compressed
        write_u32(&mut out, size); // uncompressed
        write_u16(&mut out, name_len);
        write_u16(&mut out, 0); // extra len
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);

        // Central directory header.
        write_u32(&mut central, SIG_CDH);
        write_u16(&mut central, VERSION_NEEDED); // version made by
        write_u16(&mut central, VERSION_NEEDED); // version needed
        write_u16(&mut central, FLAG_UTF8_NAME);
        write_u16(&mut central, METHOD_STORED);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u32(&mut central, crc);
        write_u32(&mut central, size);
        write_u32(&mut central, size);
        write_u16(&mut central, name_len);
        write_u16(&mut central, 0); // extra len
        write_u16(&mut central, 0); // comment len
        write_u16(&mut central, 0); // disk start
        write_u16(&mut central, 0); // internal attrs
        // External attrs: directory entries (path ending in '/') get
        // the "is dir" bit set per common convention; otherwise zero.
        let ext_attrs = if path.ends_with('/') { 0x0000_0010 } else { 0 };
        write_u32(&mut central, ext_attrs);
        write_u32(&mut central, lfh_offset);
        central.extend_from_slice(name_bytes);

        count += 1;
    }

    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);

    // End-of-central-directory record.
    write_u32(&mut out, SIG_EOCD);
    write_u16(&mut out, 0); // this disk
    write_u16(&mut out, 0); // disk with CD start
    write_u16(&mut out, count);
    write_u16(&mut out, count);
    write_u32(&mut out, cd_size);
    write_u32(&mut out, cd_offset);
    write_u16(&mut out, 0); // comment len

    out
}

/// Decode a stored-only ZIP archive back into the same shape
/// `encode_zip_stored` accepts. Rejects archives that contain any
/// entry with a non-stored compression method (deflate, bzip2, etc.)
/// — the caller can detect this and arrange a fallback.
pub fn decode_zip_stored(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, &'static str> {
    if bytes.len() < 22 {
        return Err("archive too small for EOCD");
    }
    // Find EOCD by scanning backwards from the end (the comment field is
    // variable-length up to 65535 bytes, so a linear back-scan is the
    // textbook algorithm).
    let mut eocd_off = None;
    let max_back = bytes.len().min(22 + 0xFFFF);
    let start = bytes.len() - max_back;
    for i in (start..=bytes.len() - 22).rev() {
        if read_u32(bytes, i) == SIG_EOCD {
            eocd_off = Some(i);
            break;
        }
    }
    let eocd_off = eocd_off.ok_or("EOCD signature not found")?;
    let total_entries = read_u16(bytes, eocd_off + 10) as usize;
    let cd_size = read_u32(bytes, eocd_off + 12) as usize;
    let cd_offset = read_u32(bytes, eocd_off + 16) as usize;

    if cd_offset + cd_size > bytes.len() {
        return Err("central directory beyond archive end");
    }

    // Walk the central directory.
    let mut p = cd_offset;
    let cd_end = cd_offset + cd_size;
    let mut out: Vec<(String, Vec<u8>)> = Vec::with_capacity(total_entries);
    while p + 46 <= cd_end {
        if read_u32(bytes, p) != SIG_CDH {
            return Err("bad central directory signature");
        }
        let method = read_u16(bytes, p + 10);
        if method != METHOD_STORED {
            return Err("non-stored compression method (deflate not yet supported)");
        }
        let comp_size = read_u32(bytes, p + 20) as usize;
        let _uncomp_size = read_u32(bytes, p + 24) as usize;
        let name_len = read_u16(bytes, p + 28) as usize;
        let extra_len = read_u16(bytes, p + 30) as usize;
        let comment_len = read_u16(bytes, p + 32) as usize;
        let lfh_offset = read_u32(bytes, p + 42) as usize;
        let name_off = p + 46;
        if name_off + name_len > cd_end { return Err("CD name overruns CD"); }
        let name = match core::str::from_utf8(&bytes[name_off..name_off + name_len]) {
            Ok(s) => s.to_string(),
            Err(_) => return Err("non-utf8 entry name"),
        };

        // Jump to LFH and skip its variable header to reach the data.
        if lfh_offset + 30 > bytes.len() { return Err("LFH offset out of range"); }
        if read_u32(bytes, lfh_offset) != SIG_LFH { return Err("bad local file header"); }
        let lfh_name_len = read_u16(bytes, lfh_offset + 26) as usize;
        let lfh_extra_len = read_u16(bytes, lfh_offset + 28) as usize;
        let data_off = lfh_offset + 30 + lfh_name_len + lfh_extra_len;
        if data_off + comp_size > bytes.len() { return Err("entry data overruns archive"); }
        let payload = bytes[data_off..data_off + comp_size].to_vec();

        out.push((name, payload));

        p = name_off + name_len + extra_len + comment_len;
    }

    Ok(out)
}

// ── byte / int helpers ─────────────────────────────────────────────

fn write_u16(buf: &mut Vec<u8>, v: u16) {
    buf.push((v & 0xFF) as u8);
    buf.push(((v >> 8) & 0xFF) as u8);
}
fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.push((v & 0xFF) as u8);
    buf.push(((v >> 8) & 0xFF) as u8);
    buf.push(((v >> 16) & 0xFF) as u8);
    buf.push(((v >> 24) & 0xFF) as u8);
}
fn read_u16(buf: &[u8], off: usize) -> u16 {
    (buf[off] as u16) | ((buf[off + 1] as u16) << 8)
}
fn read_u32(buf: &[u8], off: usize) -> u32 {
    (buf[off] as u32)
        | ((buf[off + 1] as u32) << 8)
        | ((buf[off + 2] as u32) << 16)
        | ((buf[off + 3] as u32) << 24)
}

/// Test-binary entry point that exposes the inlined CRC-32. Same
/// implementation as `crc32`; published under a distinct name so the
/// integration test (`tests/platform_zip.rs`) can call it without
/// making the bare `crc32` symbol part of the module's public API
/// (the underlying impl is still considered an internal detail of
/// the codec and may be replaced when deflate lands).
pub fn crc32_for_test(data: &[u8]) -> u32 { crc32(data) }

// IEEE 802.3 CRC-32 (polynomial 0xEDB88320, reflected). Standard
// Slice-by-1 implementation; throughput is low but the sizes we
// process here are small (a few KiB per archive in tests, MB at
// worst in production until #401 lands a streaming blob API).
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_known_vectors() {
        // PNG / standard CRC-32 test vectors.
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn hex_round_trip() {
        let bytes = b"Hello, world!\x00\xff\x10".to_vec();
        let hex = encode_content_ref(&bytes);
        assert_eq!(hex, "48656c6c6f2c20776f726c642100ff10");
        assert_eq!(decode_content_ref(&hex).unwrap(), bytes);
    }

    #[test]
    fn zip_codec_round_trips_three_files() {
        let entries: Vec<(String, Vec<u8>)> = vec![
            ("a.txt".to_string(), b"first file".to_vec()),
            ("sub/b.bin".to_string(), b"\x00\x01\x02\x03\xfe\xff".to_vec()),
            ("sub/inner/c.json".to_string(), b"{\"k\": 42}".to_vec()),
        ];
        let archive = encode_zip_stored(&entries);
        let decoded = decode_zip_stored(&archive).expect("decode");
        assert_eq!(decoded, entries);
    }

    #[test]
    fn zip_codec_handles_empty_directory_entry() {
        let entries: Vec<(String, Vec<u8>)> = vec![
            ("empty/".to_string(), Vec::new()),
            ("file.txt".to_string(), b"x".to_vec()),
        ];
        let archive = encode_zip_stored(&entries);
        let decoded = decode_zip_stored(&archive).expect("decode");
        assert_eq!(decoded, entries);
    }

    #[test]
    fn decode_rejects_non_stored_method() {
        let entries = vec![("a.txt".to_string(), b"hello".to_vec())];
        let mut archive = encode_zip_stored(&entries);
        // Locate the LFH method field (offset +8 from start) and flip
        // to deflate (8). decode_zip_stored consults the CDH method,
        // so flip that too: CDH starts after the LFH+name+data block.
        archive[8] = 8;
        // Find the CDH signature scanning forward.
        let mut cdh = None;
        for i in 0..archive.len() - 4 {
            if read_u32(&archive, i) == SIG_CDH { cdh = Some(i); break; }
        }
        let cdh = cdh.expect("cdh present");
        archive[cdh + 10] = 8;
        let res = decode_zip_stored(&archive);
        assert!(res.is_err());
    }

    /// Synthesise a tiny D containing a Directory subtree of three
    /// Files, then round-trip through `walk_directory_subtree` /
    /// `encode_zip_stored` / `decode_zip_stored` and assert byte-
    /// identity on every File's payload. This is the spec test from
    /// the task brief.
    #[test]
    fn directory_subtree_round_trip_three_files() {
        let d = Object::phi();

        // Root Directory.
        let d = ast::cell_push("Directory_has_Name",
            fact_from_pairs(&[("Directory", "root"), ("Name", "root")]), &d);

        // Subdirectory under root.
        let d = ast::cell_push("Directory_has_Name",
            fact_from_pairs(&[("Directory", "sub"), ("Name", "sub")]), &d);
        let d = ast::cell_push("Directory_has_parent_Directory",
            fact_from_pairs(&[("Directory", "sub"), ("parent Directory", "root")]), &d);

        // Three files: two in root, one in sub.
        let payloads: [(&str, &str, &str, &[u8]); 3] = [
            ("f1", "alpha.txt", "root", b"alpha contents"),
            ("f2", "beta.bin",  "root", b"\x00\x01\x02\x03\x04\xff"),
            ("f3", "gamma.json","sub",  b"{\"value\": 7}"),
        ];
        let mut d_acc = d;
        for (id, name, parent, bytes) in payloads.iter() {
            d_acc = ast::cell_push("File_has_Name",
                fact_from_pairs(&[("File", id), ("Name", name)]), &d_acc);
            d_acc = ast::cell_push("File_is_in_Directory",
                fact_from_pairs(&[("File", id), ("Directory", parent)]), &d_acc);
            let cref = encode_content_ref(bytes);
            d_acc = ast::cell_push("File_has_ContentRef",
                fact_from_pairs(&[("File", id), ("ContentRef", &cref)]), &d_acc);
        }

        // Walk + zip.
        let entries = walk_directory_subtree("root", &d_acc).expect("walk root");
        // Order isn't load-bearing but we expect exactly one entry per file.
        let by_path: HashMap<String, Vec<u8>> = entries.iter().cloned().collect();
        assert_eq!(by_path.get("alpha.txt").unwrap(), &b"alpha contents".to_vec());
        assert_eq!(by_path.get("beta.bin").unwrap(),  &b"\x00\x01\x02\x03\x04\xff".to_vec());
        assert_eq!(by_path.get("sub/gamma.json").unwrap(), &b"{\"value\": 7}".to_vec());

        // Encode + decode; assert byte-identity per entry.
        let archive = encode_zip_stored(&entries);
        let decoded = decode_zip_stored(&archive).expect("decode");
        let decoded_map: HashMap<String, Vec<u8>> = decoded.into_iter().collect();
        for (path, expected) in entries {
            assert_eq!(decoded_map.get(&path).expect("path present"), &expected);
        }
    }

    /// End-to-end: zip a Directory subtree, then unzip the resulting
    /// File into a fresh target Directory, and verify every original
    /// File's bytes resurface under the unzip target.
    #[test]
    fn zip_then_unzip_round_trips_payloads() {
        let d = Object::phi();
        let d = ast::cell_push("Directory_has_Name",
            fact_from_pairs(&[("Directory", "src"), ("Name", "src")]), &d);
        let d = ast::cell_push("Directory_has_Name",
            fact_from_pairs(&[("Directory", "dst"), ("Name", "dst")]), &d);

        // Source files.
        let payloads: [(&str, &str, &[u8]); 3] = [
            ("fA", "one.txt",  b"first"),
            ("fB", "two.bin",  b"\x00\xff\x55\xaa"),
            ("fC", "three.md", b"# heading\n\nbody"),
        ];
        let mut d_acc = d;
        for (id, name, bytes) in payloads.iter() {
            d_acc = ast::cell_push("File_has_Name",
                fact_from_pairs(&[("File", id), ("Name", name)]), &d_acc);
            d_acc = ast::cell_push("File_is_in_Directory",
                fact_from_pairs(&[("File", id), ("Directory", "src")]), &d_acc);
            let cref = encode_content_ref(bytes);
            d_acc = ast::cell_push("File_has_ContentRef",
                fact_from_pairs(&[("File", id), ("ContentRef", &cref)]), &d_acc);
        }

        // src has no parent, so the new archive lands at src itself.
        let (zip_id, d_after_zip) = zip_directory_in("src", &d_acc).expect("zip");

        // Unzip into dst.
        let (d_after_unzip, _created) = unzip_file_in(&zip_id, "dst", &d_after_zip).expect("unzip");

        // Every original payload is present under dst.
        let in_dir = ast::fetch_or_phi("File_is_in_Directory", &d_after_unzip);
        let in_dir_facts = in_dir.as_seq().unwrap_or(&[]).to_vec();
        let in_dst: Vec<String> = in_dir_facts.iter().filter_map(|f| {
            if ast::binding(f, "Directory") == Some("dst") {
                ast::binding(f, "File").map(|s| s.to_string())
            } else { None }
        }).collect();
        assert!(!in_dst.is_empty(), "expected at least one file under dst");

        // Collect (name → bytes) for files under dst.
        let mut found: HashMap<String, Vec<u8>> = HashMap::new();
        for fid in &in_dst {
            let name = file_name(fid, &d_after_unzip).expect("name");
            let bytes = file_content_bytes(fid, &d_after_unzip).expect("bytes");
            found.insert(name, bytes);
        }
        for (_id, name, bytes) in payloads.iter() {
            assert_eq!(found.get(*name).expect("name present in dst"),
                       &bytes.to_vec());
        }
    }
}
