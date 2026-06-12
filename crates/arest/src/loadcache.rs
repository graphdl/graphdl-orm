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
