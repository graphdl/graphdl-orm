// crates/arest/src/platform/search.rs
//
// `search_files` — search File entities in a tenant's D by name,
// tag, or content (per AREST whitepaper §7.4 Platform-fn surface).
//
// ## Why this exists
//
// The file-browser UI (#405) needs an "all places that contain X"
// lookup over the filesystem nouns declared in `readings/os/filesystem.md`
// (#398-#399). The CLI (`arest search foo`) hits the same surface.
// The single read path is:
//
//   - name      → substring match on `File has Name`
//   - tag       → exact match on `Tag has Name` joined via
//                 `File has-tag Tag`
//   - content   → byte-substring scan over the inline-blob bytes
//                 sitting inside `File has ContentRef`
//
// Multiple criteria are AND-combined: a query that sets both `name`
// and `tag` returns only files that match *both*, with the union of
// matched-fields recorded in the returned `FileMatch`. (The "union"
// is bounded — a query asking only for `name` will never report a
// `tag` hit even if the file happens to be tagged; the report
// reflects the criteria the caller asked for, not the underlying
// file's full set of properties.)
//
// ## Why pure-Rust, std-only
//
// The lookup surface is entirely D-local — every input is already
// inside the Object passed in by the host. There is no I/O on the
// hot path: tag / name reads are O(facts) over the relevant cells,
// content scan is O(N·M) over inline blob bytes. The whole `platform`
// module is `#[cfg(not(no_std))]`-gated by `mod.rs`, so the kernel
// build never reaches this file (`String` / `Vec` / `format!` are
// safe).
//
// ## Region-backed blobs (#401)
//
// Files whose `ContentRef` is a region-backed blob (>64 KiB, stored
// out-of-line via `arest_kernel::block_storage::alloc_region`) are
// **silently skipped** for the content-substring scan in this commit.
// The read would need a `block_storage` region pull which is not
// wired into the engine surface here; the consumer that lands the
// region read (`crates/arest/src/blob.rs` follow-up) flips this
// branch from `continue` to a `read_region(...)` call. Name and tag
// matching are independent of ContentRef so they remain correct for
// region-backed files.
//
// ## Why no `apply_platform` adapter (yet)
//
// Like `mime.rs`, `search_files` has a non-trivially-typed input
// (`SearchQuery<'a>`). The adapter that maps a Platform-fn operand
// `<<name, ...>, <tag, ...>, ...>` Object to a SearchQuery belongs
// in the consumer (the file-browser HTTP handler / CLI), not here.
// Until that lands, callers reach this fn directly via
// `crate::platform::search::search_files`.

#![cfg(not(feature = "no_std"))]

use crate::ast::{self, Object};

// ── Public surface ──────────────────────────────────────────────────

/// One search criterion, or any combination thereof. All fields are
/// optional and AND-combined: a None field is "don't filter on this".
/// An entirely-None query returns every File in `state` (with no
/// match-flags set). The lifetime parameter lets callers pass
/// `&str` slices borrowed from a CLI arg buffer or HTTP query string
/// without allocating.
#[derive(Debug, Clone, Copy, Default)]
pub struct SearchQuery<'a> {
    /// Substring (case-sensitive) on `File.Name`. Empty string
    /// matches every file.
    pub name: Option<&'a str>,
    /// Exact match on `Tag.Name`. Joined to File via the
    /// `File has-tag Tag` cell. Tags whose Name does not match are
    /// ignored; files with no tag at all are non-matches when this
    /// is `Some`.
    pub tag: Option<&'a str>,
    /// Substring (byte-level) on the decoded `File.ContentRef` bytes.
    /// Region-backed blobs (#401) are silently skipped in this
    /// commit — see module-level docs.
    pub content: Option<&'a str>,
    /// Cap on returned matches. `None` = no limit. The cap is
    /// applied after AND-combining; the iteration order across
    /// File entities follows their insertion order in the
    /// `File_has_Name` cell, so `limit` is deterministic given a
    /// fixed D.
    pub limit: Option<usize>,
}

/// Which criteria contributed to a hit. Set bits correspond to the
/// `Some(_)` fields of the SearchQuery — a name-only query yields
/// `MatchKind { name: true, .. }` for every hit. Used by the
/// file-browser UI to render per-row "matched on" badges.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MatchKind {
    pub name: bool,
    pub tag: bool,
    pub content: bool,
}

/// One result row. `file_id` is the entity id (the value of `File`
/// in a `File_has_Name` fact). `kind` reports which of the requested
/// criteria contributed to this hit — useful for highlighting in
/// the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMatch {
    pub file_id: String,
    pub kind: MatchKind,
}

/// Search File entities in `state` against `query`. See SearchQuery
/// docs for criteria semantics. Returns matches in `File_has_Name`
/// insertion order, capped at `query.limit`.
pub fn search_files(state: &Object, query: &SearchQuery) -> Vec<FileMatch> {
    // Iterate the `File_has_Name` cell as the spine — it's the
    // mandatory cell for every File (the readings declare "Each
    // File has exactly one Name."), so every File is reachable from
    // it. This avoids needing a separate "all files" cell.
    let names_cell = ast::fetch_or_phi("File_has_Name", state);
    let names = match names_cell.as_seq() {
        Some(s) => s,
        None => return Vec::new(),
    };

    // Pre-compute the tag → matching file-ids set when the query
    // requests tag filtering. Doing the join once up-front is the
    // O(1)-per-File requirement called out in the task brief.
    let tag_filter: Option<TagFilter> = query.tag.map(|wanted| {
        TagFilter::build(wanted, state)
    });

    let mut out: Vec<FileMatch> = Vec::new();

    for fact in names.iter() {
        let file_id = match ast::binding(fact, "File") {
            Some(s) => s,
            None => continue,
        };
        let name = match ast::binding(fact, "Name") {
            Some(s) => s,
            None => continue,
        };

        // ── Name criterion ──
        let name_hit: Option<bool> = query.name.map(|needle| name.contains(needle));
        if let Some(false) = name_hit { continue; }

        // ── Tag criterion ──
        let tag_hit: Option<bool> = tag_filter.as_ref().map(|f| f.matches(file_id));
        if let Some(false) = tag_hit { continue; }

        // ── Content criterion ──
        // Defer the (potentially expensive) hex-decode + substring
        // scan until the cheaper criteria have already accepted
        // this file.
        let content_hit: Option<bool> = match query.content {
            None => None,
            Some(needle) => Some(content_contains(file_id, needle, state)),
        };
        if let Some(false) = content_hit { continue; }

        out.push(FileMatch {
            file_id: file_id.to_string(),
            kind: MatchKind {
                name: name_hit.unwrap_or(false),
                tag: tag_hit.unwrap_or(false),
                content: content_hit.unwrap_or(false),
            },
        });

        if let Some(limit) = query.limit {
            if out.len() >= limit { break; }
        }
    }

    out
}

// ── Tag join helper ─────────────────────────────────────────────────

/// Pre-computed File-id set keyed off "files with at least one Tag
/// whose Name == wanted". Building this up-front is O(tags + joins);
/// the `matches` lookup is O(joined_count) per file. For the
/// file-browser scale (~1k tags, ~10k files, ~30k join facts), a
/// linear scan beats a HashSet for cache locality; promote to
/// HashSet if profiling shows otherwise.
struct TagFilter {
    file_ids: Vec<String>,
}

impl TagFilter {
    fn build(wanted: &str, state: &Object) -> TagFilter {
        // Step 1: collect Tag ids whose Name == wanted.
        let mut tag_ids: Vec<String> = Vec::new();
        let tag_names = ast::fetch_or_phi("Tag_has_Name", state);
        if let Some(facts) = tag_names.as_seq() {
            for fact in facts {
                if ast::binding(fact, "Name") == Some(wanted) {
                    if let Some(tid) = ast::binding(fact, "Tag") {
                        tag_ids.push(tid.to_string());
                    }
                }
            }
        }

        // Step 2: collect File ids that are joined to any of those
        // Tag ids via `File has-tag Tag`. The cell name follows the
        // existing zip.rs convention of joining the reading words
        // with underscores (preserving the hyphen in `has-tag` so
        // the cell-id remains a faithful echo of the reading text).
        let mut file_ids: Vec<String> = Vec::new();
        if !tag_ids.is_empty() {
            let joins = ast::fetch_or_phi("File_has-tag_Tag", state);
            if let Some(facts) = joins.as_seq() {
                for fact in facts {
                    let tag = match ast::binding(fact, "Tag") { Some(t) => t, None => continue };
                    if !tag_ids.iter().any(|t| t == tag) { continue; }
                    if let Some(fid) = ast::binding(fact, "File") {
                        if !file_ids.iter().any(|f| f == fid) {
                            file_ids.push(fid.to_string());
                        }
                    }
                }
            }
        }

        TagFilter { file_ids }
    }

    fn matches(&self, file_id: &str) -> bool {
        self.file_ids.iter().any(|f| f == file_id)
    }
}

// ── Content-scan helper ─────────────────────────────────────────────

/// True iff `file_id`'s `ContentRef` decodes to bytes containing
/// `needle` (interpreted as a byte sequence — the input `&str` is
/// implicitly UTF-8). Returns false (not an error) for files with
/// no ContentRef, undecodable ContentRef, or region-backed blobs;
/// this is intentional — search must not throw, and silent-skip
/// is the documented #401 behaviour.
fn content_contains(file_id: &str, needle: &str, state: &Object) -> bool {
    if needle.is_empty() {
        // Empty needle matches every byte slice including empty —
        // standard string-search semantics.
        return true;
    }
    let cell = ast::fetch_or_phi("File_has_ContentRef", state);
    let cref = match cell.as_seq().and_then(|facts| {
        facts.iter().find_map(|f| {
            if ast::binding(f, "File") == Some(file_id) {
                ast::binding(f, "ContentRef").map(|s| s.to_string())
            } else {
                None
            }
        })
    }) {
        Some(s) => s,
        None => return false,
    };

    // Region-backed blob? Skip silently per #401 deferral. The
    // region encoding starts with a `<REGION,...>` tagged object,
    // serialised via the bare-hex-or-tagged path described in
    // `zip.rs`. Today the encoder writes bare hex; once the tagged
    // form lands the prefix `<REGION` (or the tagged equivalent)
    // is the discriminator. Match both shapes pre-emptively so this
    // branch keeps working when blob.rs swaps encodings.
    if cref.starts_with("<REGION") || cref.starts_with("REGION:") {
        // TODO(#401): wire `block_storage::read_region(...)` once
        // exposed at the engine layer; until then a content search
        // does not match large blobs.
        return false;
    }

    let bytes = match decode_content_ref(&cref) {
        Some(b) => b,
        None => return false,
    };
    byte_contains(&bytes, needle.as_bytes())
}

/// Decode a `ContentRef` atom into raw bytes. Mirrors the bare-hex
/// path in `platform::zip::decode_content_ref` so a File created via
/// the zip codec round-trips through the search content scan
/// unchanged. Returns `None` for malformed hex (odd length, non-hex
/// char) — the search treats those as zero-byte inputs (no match)
/// rather than propagating the decode error to the caller.
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

fn nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// O(N·M) byte-substring search. For the workloads this Platform
/// fn was added for — file-browser ad-hoc query, ≤ a few MB inline
/// blobs — this is fast enough and avoids pulling in a substring-
/// search dep (`memchr` is std-only on stable today). Promote to
/// Boyer-Moore / KMP if profiling shows hot inner-loop time on a
/// content-heavy tenant.
fn byte_contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() { return true; }
    if haystack.len() < needle.len() { return false; }
    let last = haystack.len() - needle.len();
    for i in 0..=last {
        if haystack[i..i + needle.len()] == *needle {
            return true;
        }
    }
    false
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{self, fact_from_pairs};

    /// Build a D containing 3 Files with distinct names, tags, and
    /// inline-content payloads. Returns the populated D for the
    /// per-criterion tests below.
    fn build_test_d() -> Object {
        let d = Object::phi();

        // Three Files. f1 holds the word "alpha" in its content;
        // f2 has none of the search needles in its bytes; f3
        // holds the substring "match-me-here".
        let files: [(&str, &str, &[u8]); 3] = [
            ("f1", "alpha-notes.txt",  b"alpha bravo charlie"),
            ("f2", "beta-data.bin",    b"\x00\x01\x02\x03\x04\x05"),
            ("f3", "gamma-readme.md",  b"# heading\n\nsome match-me-here body"),
        ];
        let mut d = d;
        for (id, name, bytes) in files.iter() {
            d = ast::cell_push("File_has_Name",
                fact_from_pairs(&[("File", id), ("Name", name)]), &d);
            d = ast::cell_push("File_has_ContentRef",
                fact_from_pairs(&[
                    ("File", id),
                    ("ContentRef", &encode_hex(bytes)),
                ]), &d);
        }

        // Two Tags. "important" tags f1; "draft" tags f2 and f3.
        // Tag entity ids are arbitrary atoms — the search joins
        // through Name, not id.
        let tags = [
            ("t1", "important"),
            ("t2", "draft"),
        ];
        for (id, name) in tags.iter() {
            d = ast::cell_push("Tag_has_Name",
                fact_from_pairs(&[("Tag", id), ("Name", name)]), &d);
        }
        let joins = [("f1", "t1"), ("f2", "t2"), ("f3", "t2")];
        for (file, tag) in joins.iter() {
            d = ast::cell_push("File_has-tag_Tag",
                fact_from_pairs(&[("File", file), ("Tag", tag)]), &d);
        }

        d
    }

    /// Hex-encode bytes. Local copy of zip.rs's `encode_content_ref`
    /// so the test module is self-contained (the `crate::platform::zip`
    /// path is also valid but pulling in a sibling-module test dep is
    /// unnecessary friction).
    fn encode_hex(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            s.push(nibble_to_char(b >> 4));
            s.push(nibble_to_char(b & 0xF));
        }
        s
    }

    fn nibble_to_char(v: u8) -> char {
        match v {
            0..=9 => (b'0' + v) as char,
            10..=15 => (b'a' + (v - 10)) as char,
            _ => '0',
        }
    }

    #[test]
    fn name_substring_matches_one_file() {
        let d = build_test_d();
        let q = SearchQuery { name: Some("gamma"), ..Default::default() };
        let hits = search_files(&d, &q);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file_id, "f3");
        assert!(hits[0].kind.name);
        assert!(!hits[0].kind.tag);
        assert!(!hits[0].kind.content);
    }

    #[test]
    fn name_substring_matches_multiple_files() {
        let d = build_test_d();
        // All three filenames contain "-" — should match every File.
        let q = SearchQuery { name: Some("-"), ..Default::default() };
        let hits = search_files(&d, &q);
        assert_eq!(hits.len(), 3);
        let ids: Vec<&str> = hits.iter().map(|m| m.file_id.as_str()).collect();
        assert!(ids.contains(&"f1"));
        assert!(ids.contains(&"f2"));
        assert!(ids.contains(&"f3"));
    }

    #[test]
    fn tag_exact_matches_single_tagged_file() {
        let d = build_test_d();
        let q = SearchQuery { tag: Some("important"), ..Default::default() };
        let hits = search_files(&d, &q);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file_id, "f1");
        assert!(hits[0].kind.tag);
        assert!(!hits[0].kind.name);
        assert!(!hits[0].kind.content);
    }

    #[test]
    fn tag_exact_matches_multiple_tagged_files() {
        let d = build_test_d();
        let q = SearchQuery { tag: Some("draft"), ..Default::default() };
        let hits = search_files(&d, &q);
        assert_eq!(hits.len(), 2);
        let ids: Vec<&str> = hits.iter().map(|m| m.file_id.as_str()).collect();
        assert!(ids.contains(&"f2"));
        assert!(ids.contains(&"f3"));
        for hit in &hits {
            assert!(hit.kind.tag);
        }
    }

    #[test]
    fn tag_unknown_returns_no_hits() {
        let d = build_test_d();
        let q = SearchQuery { tag: Some("does-not-exist"), ..Default::default() };
        let hits = search_files(&d, &q);
        assert!(hits.is_empty());
    }

    #[test]
    fn content_substring_matches_one_file() {
        let d = build_test_d();
        let q = SearchQuery { content: Some("match-me-here"), ..Default::default() };
        let hits = search_files(&d, &q);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file_id, "f3");
        assert!(hits[0].kind.content);
        assert!(!hits[0].kind.name);
        assert!(!hits[0].kind.tag);
    }

    #[test]
    fn content_substring_finds_text_in_alpha_file() {
        let d = build_test_d();
        let q = SearchQuery { content: Some("bravo"), ..Default::default() };
        let hits = search_files(&d, &q);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file_id, "f1");
        assert!(hits[0].kind.content);
    }

    #[test]
    fn content_substring_no_match_when_absent() {
        let d = build_test_d();
        let q = SearchQuery { content: Some("nonsense-xyzzy"), ..Default::default() };
        let hits = search_files(&d, &q);
        assert!(hits.is_empty());
    }

    #[test]
    fn and_combination_name_and_tag() {
        let d = build_test_d();
        // f3's name contains "gamma" AND it has tag "draft" — the
        // intersection is exactly {f3}.
        let q = SearchQuery {
            name: Some("gamma"),
            tag: Some("draft"),
            ..Default::default()
        };
        let hits = search_files(&d, &q);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file_id, "f3");
        assert!(hits[0].kind.name);
        assert!(hits[0].kind.tag);
        assert!(!hits[0].kind.content);
    }

    #[test]
    fn and_combination_disjoint_returns_empty() {
        let d = build_test_d();
        // f1 has tag "important" but its name does NOT contain "gamma".
        // No file matches both → empty.
        let q = SearchQuery {
            name: Some("gamma"),
            tag: Some("important"),
            ..Default::default()
        };
        let hits = search_files(&d, &q);
        assert!(hits.is_empty());
    }

    #[test]
    fn and_combination_all_three_criteria() {
        let d = build_test_d();
        // f1's name contains "alpha", it has tag "important", and
        // its content contains "alpha". The intersection of all
        // three is {f1}.
        let q = SearchQuery {
            name: Some("alpha"),
            tag: Some("important"),
            content: Some("alpha"),
            ..Default::default()
        };
        let hits = search_files(&d, &q);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file_id, "f1");
        assert!(hits[0].kind.name);
        assert!(hits[0].kind.tag);
        assert!(hits[0].kind.content);
    }

    #[test]
    fn empty_query_returns_every_file() {
        let d = build_test_d();
        let q = SearchQuery::default();
        let hits = search_files(&d, &q);
        assert_eq!(hits.len(), 3);
        // No criterion was supplied, so no match-flag is set.
        for hit in &hits {
            assert!(!hit.kind.name);
            assert!(!hit.kind.tag);
            assert!(!hit.kind.content);
        }
    }

    #[test]
    fn limit_caps_result_count() {
        let d = build_test_d();
        let q = SearchQuery {
            name: Some("-"), // matches all three
            limit: Some(2),
            ..Default::default()
        };
        let hits = search_files(&d, &q);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn empty_state_returns_no_hits() {
        let d = Object::phi();
        let q = SearchQuery { name: Some("foo"), ..Default::default() };
        let hits = search_files(&d, &q);
        assert!(hits.is_empty());
    }

    #[test]
    fn region_backed_blob_is_silently_skipped_for_content() {
        // Synthesise a File whose ContentRef is the region-form
        // marker (the future tagged-Object encoding from #401).
        // The content-search must NOT match — region reads are
        // deferred to a follow-up.
        let d = Object::phi();
        let d = ast::cell_push("File_has_Name",
            fact_from_pairs(&[("File", "fr"), ("Name", "huge.bin")]), &d);
        let d = ast::cell_push("File_has_ContentRef",
            fact_from_pairs(&[
                ("File", "fr"),
                ("ContentRef", "<REGION,8192,131072>"),
            ]), &d);

        // Name search still works.
        let q = SearchQuery { name: Some("huge"), ..Default::default() };
        assert_eq!(search_files(&d, &q).len(), 1);

        // Content search returns nothing — file is silently skipped.
        let q = SearchQuery { content: Some("anything"), ..Default::default() };
        assert!(search_files(&d, &q).is_empty());
    }

    #[test]
    fn malformed_content_ref_does_not_match() {
        let d = Object::phi();
        let d = ast::cell_push("File_has_Name",
            fact_from_pairs(&[("File", "fb"), ("Name", "broken.txt")]), &d);
        // Odd-length hex → decode returns None → content match returns false.
        let d = ast::cell_push("File_has_ContentRef",
            fact_from_pairs(&[("File", "fb"), ("ContentRef", "abc")]), &d);

        let q = SearchQuery { content: Some("a"), ..Default::default() };
        assert!(search_files(&d, &q).is_empty());

        // But the file IS still reachable for name / tag matching.
        let q = SearchQuery { name: Some("broken"), ..Default::default() };
        assert_eq!(search_files(&d, &q).len(), 1);
    }
}
