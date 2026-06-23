//! load-state-cache (task `load-state-cache-or-warm-engine`, lever A):
//! a binary SIDECAR cache of the deserialized cell graph, keyed on
//! `(binary self-hash, db len, db mtime)`.
//!
//! Why: every CLI spawn pays `db::load_state` — `Object::parse` over
//! every cells/defs row — before ANY verb runs. At arc-agi-3's 113 MB
//! that is ~2 minutes per call, linear in db size (their datum #4:
//! a single-row `query` measured 1m55s round-trip). The metamodel
//! parse cache does not help here: its storage IS a SQLite db read
//! back through `load_state`, so it caches the fold, not the
//! deserialize. This cache attacks the deserialize itself: a
//! length-prefixed binary tree that decodes with no tokenization.
//!
//! Invalidation is structural: the key embeds the db file's length and
//! mtime (any SQLite commit moves them) and the binary self-hash (a
//! rebuilt engine must not serve a stale tree across format or
//! semantics changes). A mismatched or malformed sidecar is ignored
//! and the caller falls back to `Object::parse`, then rewrites the
//! sidecar — self-healing, never authoritative. `AREST_LOAD_CACHE=0`
//! disables reads and writes entirely.
//!
//! Format (little-endian):
//!   magic  b"ARESTLC1"
//!   key    u64
//!   tree   node := tag u8
//!     0 = Bottom
//!     1 = Atom   : u32 len + utf8 bytes
//!     2 = Seq    : u32 count + count nodes
//!     3 = Map    : u32 count + count * (u32 keylen + key utf8 + node)
//! Map keys are written SORTED so racing writers emit byte-identical
//! files (same atomic temp+rename discipline as the metamodel cache).

use crate::ast::Object;
use std::io::Write as _;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"ARESTLC1";

/// Sidecar path: `<db>.loadcache` beside the db file.
pub(crate) fn cache_path(db_path: &Path) -> PathBuf {
    let mut s = db_path.as_os_str().to_os_string();
    s.push(".loadcache");
    PathBuf::from(s)
}

/// Read the sidecar; `Some(state)` only when the magic and key match
/// and the whole tree decodes cleanly with no trailing garbage.
pub(crate) fn load(db_path: &Path, key: u64) -> Option<Object> {
    let bytes = std::fs::read(cache_path(db_path)).ok()?;
    if bytes.len() < 16 || &bytes[..8] != MAGIC {
        return None;
    }
    let stored_key = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
    if stored_key != key {
        return None;
    }
    let mut pos = 16usize;
    let tree = decode(&bytes, &mut pos)?;
    (pos == bytes.len()).then_some(tree)
}

/// Write the sidecar atomically (per-process temp + rename). Failures
/// are silent — the cache is an accelerator, never load-bearing.
pub(crate) fn store(db_path: &Path, key: u64, state: &Object) {
    let path = cache_path(db_path);
    let tmp = path.with_extension(format!("loadcache.tmp{}", std::process::id()));
    let mut buf: Vec<u8> = Vec::with_capacity(1 << 20);
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&key.to_le_bytes());
    encode(state, &mut buf);
    let ok = std::fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(&buf).and_then(|_| f.sync_all()))
        .is_ok();
    if ok {
        let _ = std::fs::rename(&tmp, &path);
    }
    let _ = std::fs::remove_file(&tmp);
}

// =========================================================================
// SP1 build-once libraries: derived-LFP cache.
//
// The metamodel parse-cache (cli/entry.rs) caches the PARSE of the
// metamodel readings — but every app compile still re-derives the
// metamodel's self-derived cells from scratch (the `Function`
// supertype-union reconstitution storm: ~595 re-derives per 25s window,
// a trivial 1-entity app failing to converge in 90s — root cause
// `supertype-union-reconstitution`). SP1 pays each library's derivation
// LFP ONCE per (content, binary) and warm-loads it on app compile, so app
// compiles delta-derive only their own additions.
//
// This is a CONTENT-ADDRESSED sidecar (same discipline as the metamodel
// parse cache and the per-db loadcache above): the key is FNV(readings
// content + dep keys + binary self-hash); the filename encodes the key, so
// a present, populated file IS by construction the derived LFP of the
// current (content, binary). Any readings or binary change moves the key →
// a miss → rebuild. Stored as the same length-prefixed binary tree the
// loadcache codec emits (no SQLite round-trip — this caches the derived
// cell graph directly, decoded with no tokenization). Reuses the atomic
// temp+rename write so concurrent compilers (different apps, same binary)
// never observe a torn file; the content is a deterministic function of
// (readings, binary), so racing writers emit byte-identical files.
//
// `AREST_NO_LIB_CACHE=1` bypasses this cache entirely (the "cold"
// reference for the cold==warm equivalence gate) — honoured by the caller
// in `crate::sp1`, not here.
// =========================================================================

const DERIVED_MAGIC: &[u8; 8] = b"ARESTLD1";

/// Sidecar path for a derived-LFP cache keyed by `sig`: a file in the
/// system temp dir named `arest-lib-derived-{sig:016x}.bin`.
#[cfg(feature = "local")]
pub(crate) fn derived_cache_path(sig: u64) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("arest-lib-derived-{:016x}.bin", sig));
    dir
}

/// Read the derived-LFP sidecar for `sig`; `Some(state)` only when the
/// magic and key match and the whole tree decodes cleanly with no trailing
/// garbage. The signature is encoded in the filename AND stored inline, so
/// a present, well-formed file is by construction the derived LFP of the
/// signature's (content, binary). Returns `None` on any read/decode failure
/// → caller rebuilds.
#[cfg(feature = "local")]
pub(crate) fn load_derived(sig: u64) -> Option<Object> {
    let bytes = std::fs::read(derived_cache_path(sig)).ok()?;
    if bytes.len() < 16 || &bytes[..8] != DERIVED_MAGIC {
        return None;
    }
    let stored_key = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
    if stored_key != sig {
        return None;
    }
    let mut pos = 16usize;
    let tree = decode(&bytes, &mut pos)?;
    (pos == bytes.len()).then_some(tree)
}

/// Write the derived-LFP sidecar for `sig` atomically (per-process temp +
/// rename). Failures are silent — the cache is an accelerator, never
/// load-bearing.
#[cfg(feature = "local")]
pub(crate) fn store_derived(sig: u64, state: &Object) {
    let path = derived_cache_path(sig);
    let tmp = path.with_extension(format!("bin.tmp{}", std::process::id()));
    let mut buf: Vec<u8> = Vec::with_capacity(1 << 20);
    buf.extend_from_slice(DERIVED_MAGIC);
    buf.extend_from_slice(&sig.to_le_bytes());
    encode(state, &mut buf);
    let ok = std::fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(&buf).and_then(|_| f.sync_all()))
        .is_ok();
    if ok {
        let _ = std::fs::rename(&tmp, &path);
    }
    let _ = std::fs::remove_file(&tmp);
}

fn encode(obj: &Object, out: &mut Vec<u8>) {
    match obj {
        Object::Bottom => out.push(0),
        Object::Atom(s) => {
            out.push(1);
            out.extend_from_slice(&(s.len() as u32).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        Object::Seq(items) => {
            out.push(2);
            out.extend_from_slice(&(items.len() as u32).to_le_bytes());
            for item in items.iter() {
                encode(item, out);
            }
        }
        Object::Map(m) => {
            out.push(3);
            out.extend_from_slice(&(m.len() as u32).to_le_bytes());
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            for k in keys {
                out.extend_from_slice(&(k.len() as u32).to_le_bytes());
                out.extend_from_slice(k.as_bytes());
                encode(&m[k.as_str()], out);
            }
        }
    }
}

fn read_u32(bytes: &[u8], pos: &mut usize) -> Option<usize> {
    let end = pos.checked_add(4)?;
    if end > bytes.len() {
        return None;
    }
    let v = u32::from_le_bytes(bytes[*pos..end].try_into().ok()?) as usize;
    *pos = end;
    Some(v)
}

fn read_str(bytes: &[u8], pos: &mut usize) -> Option<String> {
    let len = read_u32(bytes, pos)?;
    let end = pos.checked_add(len)?;
    if end > bytes.len() {
        return None;
    }
    let s = core::str::from_utf8(&bytes[*pos..end]).ok()?.to_string();
    *pos = end;
    Some(s)
}

fn decode(bytes: &[u8], pos: &mut usize) -> Option<Object> {
    let tag = *bytes.get(*pos)?;
    *pos += 1;
    match tag {
        0 => Some(Object::Bottom),
        1 => Some(Object::Atom(read_str(bytes, pos)?)),
        2 => {
            let count = read_u32(bytes, pos)?;
            // Defensive cap: a count larger than the remaining bytes is
            // malformed (each child needs at least one tag byte).
            if count > bytes.len().saturating_sub(*pos) {
                return None;
            }
            let mut items: Vec<Object> = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(decode(bytes, pos)?);
            }
            Some(Object::Seq(items.into()))
        }
        3 => {
            let count = read_u32(bytes, pos)?;
            if count > bytes.len().saturating_sub(*pos) {
                return None;
            }
            let mut m: hashbrown::HashMap<String, Object> =
                hashbrown::HashMap::with_capacity(count);
            for _ in 0..count {
                let k = read_str(bytes, pos)?;
                let v = decode(bytes, pos)?;
                m.insert(k, v);
            }
            Some(Object::map(m))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast;

    fn sample_state() -> Object {
        let mut m: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        m.insert("Noun".to_string(), Object::seq(vec![
            ast::fact_from_pairs(&[("name", "Task"), ("objectType", "entity")]),
            ast::fact_from_pairs(&[("name", "Größe-µ"), ("objectType", "value")]),
        ]));
        m.insert("Empty".to_string(), Object::phi());
        m.insert("Bot".to_string(), Object::Bottom);
        m.insert("Nested".to_string(), Object::map({
            let mut inner: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
            inner.insert("k#1".to_string(), Object::atom("v|with,specials"));
            inner
        }));
        Object::map(m)
    }

    /// Round-trip identity over every variant, unicode atoms, empty
    /// seqs, nested maps.
    #[test]
    fn codec_round_trips_all_variants() {
        let state = sample_state();
        let mut buf = Vec::new();
        encode(&state, &mut buf);
        let mut pos = 0usize;
        let back = decode(&buf, &mut pos).expect("decodes");
        assert_eq!(pos, buf.len(), "no trailing bytes");
        assert_eq!(back, state, "round-trip identity");
    }

    /// File-level: store then load with the right key hits; a wrong
    /// key misses; a truncated file misses (falls back to parse).
    #[test]
    fn sidecar_hits_on_key_match_misses_otherwise() {
        let dir = std::env::temp_dir();
        let db = dir.join(format!("arest-loadcache-test-{}.db", std::process::id()));
        let state = sample_state();
        store(&db, 0xfeedbeef, &state);
        assert_eq!(load(&db, 0xfeedbeef), Some(state.clone()), "key match must hit");
        assert_eq!(load(&db, 0xdeadc0de), None, "key mismatch must miss");
        // Truncate the sidecar mid-tree: decode must fail closed.
        let sc = cache_path(&db);
        let bytes = std::fs::read(&sc).unwrap();
        std::fs::write(&sc, &bytes[..bytes.len() / 2]).unwrap();
        assert_eq!(load(&db, 0xfeedbeef), None, "torn file must miss");
        let _ = std::fs::remove_file(&sc);
    }
}
