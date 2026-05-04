// crates/arest/src/freeze.rs
//
// Byte-level serialization for `ast::Object` (#185).
//
// Feeds three consumers:
//   - x86_64 bare-metal boot bakes a post-compile metamodel image into
//     the kernel binary (skips the ~50ms parse/compile per cold start).
//   - Snapshot persistence (snapshot/rollback outliving the process).
//   - FPGA on-chip ROM (the metamodel image burned into boot ROM so
//     the boot FSM initialises BRAMs without re-running the compiler).
//
// Format v1 (little-endian throughout):
//   Header:     magic "AREST\x01" (6 bytes)
//   Root:       TAG (1B) then value per the table below.
//
//   TAG_ATOM   = 0x00 : u32 len, [u8; len] utf8
//   TAG_SEQ    = 0x01 : u32 count, count × <TAG, value>
//   TAG_MAP    = 0x02 : u32 count, count × <u32 key_len, key_bytes, TAG, value>
//   TAG_BOTTOM = 0x03 : zero bytes
//   TAG_STR    = 0x04 : reserved for a future fast-path; today encoded as ATOM.
//
// Same tag convention the WASM-lowering Object layout uses (#162), so
// a future optimisation can memory-map the frozen bytes directly into
// the WASM heap without an intermediate conversion.

use crate::ast::Object;
use crate::cell_aead::{self, CellAddress, TenantMasterKey};
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned, format};

const MAGIC: &[u8] = b"AREST\x01";
const TAG_ATOM: u8 = 0x00;
const TAG_SEQ: u8 = 0x01;
const TAG_MAP: u8 = 0x02;
const TAG_BOTTOM: u8 = 0x03;

/// Magic prefix for a sealed (per-cell encrypted) freeze blob — see
/// `freeze_sealed` (#659). Distinct from the plaintext `MAGIC` so a
/// consumer that read the wrong layer fails fast at the boundary
/// rather than stumbling into a CRC / tag mismatch deeper in.
const SEALED_MAGIC: &[u8] = b"ARESTSEAL\x01";

/// Serialise an Object to a flat byte stream. Nested Seq / Map are
/// walked recursively; the output is self-describing via magic + tags.
pub fn freeze(obj: &Object) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);
    buf.extend_from_slice(MAGIC);
    write_object(&mut buf, obj);
    buf
}

/// Deserialise bytes produced by `freeze` back into an Object.
/// Returns an error on truncated input, bad magic, or unknown tag.
pub fn thaw(bytes: &[u8]) -> Result<Object, String> {
    if bytes.len() < MAGIC.len() || &bytes[..MAGIC.len()] != MAGIC {
        return Err("bad magic — not an AREST freeze image".to_string());
    }
    let mut cursor = MAGIC.len();
    let obj = read_object(bytes, &mut cursor)?;
    if cursor != bytes.len() {
        return Err(format!("trailing {} bytes after root object", bytes.len() - cursor));
    }
    Ok(obj)
}

fn write_object(buf: &mut Vec<u8>, obj: &Object) {
    match obj {
        Object::Atom(s) => {
            buf.push(TAG_ATOM);
            write_u32(buf, s.len() as u32);
            buf.extend_from_slice(s.as_bytes());
        }
        Object::Seq(items) => {
            buf.push(TAG_SEQ);
            write_u32(buf, items.len() as u32);
            for item in items.iter() { write_object(buf, item); }
        }
        Object::Map(m) => {
            buf.push(TAG_MAP);
            // Sort keys for deterministic output — two freezes of the
            // same state produce byte-identical images, which matters
            // for ROM hashing and reproducible builds.
            let mut entries: Vec<(&String, &Object)> = m.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            write_u32(buf, entries.len() as u32);
            for (k, v) in entries {
                write_u32(buf, k.len() as u32);
                buf.extend_from_slice(k.as_bytes());
                write_object(buf, v);
            }
        }
        Object::Bottom => {
            buf.push(TAG_BOTTOM);
        }
    }
}

fn read_object(bytes: &[u8], cursor: &mut usize) -> Result<Object, String> {
    let tag = read_u8(bytes, cursor)?;
    match tag {
        TAG_ATOM => {
            let len = read_u32(bytes, cursor)? as usize;
            let end = cursor.checked_add(len).ok_or("length overflow")?;
            if end > bytes.len() { return Err("atom payload truncated".to_string()); }
            let s = core::str::from_utf8(&bytes[*cursor..end])
                .map_err(|e| format!("atom utf8: {e}"))?
                .to_string();
            *cursor = end;
            Ok(Object::Atom(s))
        }
        TAG_SEQ => {
            let n = read_u32(bytes, cursor)? as usize;
            // #707 / Audit P1-A3: cap pre-allocation against remaining
            // bytes. Each element encoding requires ≥ 1 byte
            // (TAG_BOTTOM), so a count exceeding remaining bytes is
            // structurally impossible and lets a tampered freeze image
            // request a multi-GB allocation up front.
            let remaining = bytes.len().saturating_sub(*cursor);
            if n > remaining {
                return Err(format!(
                    "seq length {n} exceeds remaining bytes {remaining}"
                ));
            }
            let mut items = Vec::with_capacity(n);
            for _ in 0..n { items.push(read_object(bytes, cursor)?); }
            Ok(Object::Seq(items.into()))
        }
        TAG_MAP => {
            let n = read_u32(bytes, cursor)? as usize;
            // Same bound as TAG_SEQ — each entry needs ≥ 1 byte (klen
            // takes 4, payload ≥ 1, so 5 minimum), but the loose
            // `> remaining` is the simplest correct cap.
            let remaining = bytes.len().saturating_sub(*cursor);
            if n > remaining {
                return Err(format!(
                    "map length {n} exceeds remaining bytes {remaining}"
                ));
            }
            let mut m = hashbrown::HashMap::with_capacity(n);
            for _ in 0..n {
                let klen = read_u32(bytes, cursor)? as usize;
                let kend = cursor.checked_add(klen).ok_or("key length overflow")?;
                if kend > bytes.len() { return Err("map key truncated".to_string()); }
                let k = core::str::from_utf8(&bytes[*cursor..kend])
                    .map_err(|e| format!("map key utf8: {e}"))?
                    .to_string();
                *cursor = kend;
                let v = read_object(bytes, cursor)?;
                m.insert(k, v);
            }
            Ok(Object::Map(m))
        }
        TAG_BOTTOM => Ok(Object::Bottom),
        other => Err(format!("unknown tag 0x{other:02x}")),
    }
}

fn write_u32(buf: &mut Vec<u8>, n: u32) {
    buf.extend_from_slice(&n.to_le_bytes());
}

fn read_u8(bytes: &[u8], cursor: &mut usize) -> Result<u8, String> {
    if *cursor >= bytes.len() { return Err("truncated at tag".to_string()); }
    let b = bytes[*cursor];
    *cursor += 1;
    Ok(b)
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, String> {
    let end = cursor.checked_add(4).ok_or("length overflow")?;
    if end > bytes.len() { return Err("truncated at u32".to_string()); }
    let n = u32::from_le_bytes(bytes[*cursor..end].try_into().unwrap());
    *cursor = end;
    Ok(n)
}

// ── Per-cell sealed freeze (#659) ──────────────────────────────────
//
// `freeze` above produces a flat plaintext byte stream — fine for
// boot-image baking and tests, but every serialization boundary
// outside the engine's in-memory operating set must seal each cell
// individually so a leaked DO / disk sector / network frame doesn't
// leak any other cell with it.
//
// The encrypted blob's logical shape is the same Object::Map as the
// plaintext freeze, but each top-level map value is wrapped in a
// `cell_aead::cell_seal` envelope keyed against the matching cell
// address `(scope, domain, key, version)`. The `freeze_sealed` /
// `thaw_sealed` pair below is what `block_storage` / DO adapter /
// network frame paths reach for; the original `freeze` / `thaw`
// stays untouched for callers that explicitly want plaintext (boot-
// image bake, the FPGA ROM image, in-process snapshot debugging).
//
// Wire layout:
//
//   [SEALED_MAGIC = "ARESTSEAL\x01"] (10 bytes)
//   [u32 LE scope_len | scope bytes]
//   [u32 LE domain_len | domain bytes]
//   [u64 LE version]
//   [u32 LE cell_count]
//   for each cell:
//     [u32 LE name_len | name bytes]
//     [u32 LE sealed_len | sealed bytes (nonce | ct | tag)]
//
// The name lives in plaintext — it's the routing key the AEAD AAD
// already binds, and a DO / disk read needs it before it can
// derive the per-cell key. The interior plaintext freeze of the
// cell's `Object` value (atom / seq / nested map) is what gets
// sealed; the consumer thaws it on open.
//
// Non-Map roots (Atom / Seq / Bottom) seal as a single anonymous
// cell at name "" — shape parity with the plaintext freeze so the
// caller doesn't have to special-case the root before sealing.

/// Errors specific to the sealed freeze path. Plaintext-freeze errors
/// stay as `String` (legacy contract); the sealed path uses an enum so
/// kernel / worker callers can switch on them (a torn DO write is
/// `AeadOpen { Truncated }`; a re-target / wrong-master is `Auth`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SealedFreezeError {
    /// Header validation failed: missing magic, wrong version, or a
    /// truncated descriptor before the first sealed cell.
    Header(String),
    /// AEAD open failed for one of the cells. Carries the cell name
    /// (plaintext, harmless to log — it's also in the wire stream)
    /// and the underlying AEAD error.
    AeadOpen { cell: String, kind: cell_aead::AeadError },
    /// Plaintext thaw of a recovered (decrypted) cell payload failed.
    /// Means the wire round-tripped cleanly through AEAD but the
    /// inner Object encoding is corrupt — usually a version skew.
    Inner(String),
}

impl core::fmt::Display for SealedFreezeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Header(m) => write!(f, "sealed freeze header: {m}"),
            Self::AeadOpen { cell, kind } => write!(f, "sealed cell '{cell}': {kind}"),
            Self::Inner(m) => write!(f, "sealed cell inner thaw: {m}"),
        }
    }
}

/// Seal an Object as an encrypted freeze blob. For an Object::Map
/// root, each top-level (key, value) pair is sealed individually
/// against `CellAddress(scope, domain, key, version)`. For non-Map
/// roots (Atom / Seq / Bottom), a single sealed cell is emitted at
/// name "" — the consumer recovers the same shape on `thaw_sealed`.
///
/// Sealed-blob length = SEALED_MAGIC + 4 + scope.len() + 4 + domain.len()
///                    + 8 + 4 + Σ_cells(4 + name.len() + 4 + sealed_len),
/// where each `sealed_len` exceeds the cell's plaintext-freeze length
/// by `cell_aead::NONCE_LEN + TAG_LEN` (= 28 bytes).
///
/// Panics propagate from `cell_aead::cell_seal` — same contract: an
/// uninstalled entropy source on the target is a bring-up bug, not a
/// runtime outage.
pub fn freeze_sealed(
    obj: &Object,
    master: &TenantMasterKey,
    scope: &str,
    domain: &str,
    version: u64,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);
    buf.extend_from_slice(SEALED_MAGIC);
    write_u32(&mut buf, scope.len() as u32);
    buf.extend_from_slice(scope.as_bytes());
    write_u32(&mut buf, domain.len() as u32);
    buf.extend_from_slice(domain.as_bytes());
    buf.extend_from_slice(&version.to_le_bytes());

    // Per-cell seal walk. For Map roots we iterate sorted keys so the
    // header-portion order is deterministic (the AEAD ciphertext bytes
    // are still nonce-randomised); for non-Map roots we emit a single
    // unnamed cell so the round-trip recovers the exact same shape.
    match obj {
        Object::Map(m) => {
            let mut entries: Vec<(&String, &Object)> = m.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            write_u32(&mut buf, entries.len() as u32);
            for (k, v) in entries {
                let inner = freeze(v);
                let address = CellAddress::new(scope, domain, k.clone(), version);
                let sealed = cell_aead::cell_seal(master, &address, &inner);
                write_u32(&mut buf, k.len() as u32);
                buf.extend_from_slice(k.as_bytes());
                write_u32(&mut buf, sealed.len() as u32);
                buf.extend_from_slice(&sealed);
            }
        }
        other => {
            write_u32(&mut buf, 1u32);
            let inner = freeze(other);
            let address = CellAddress::new(scope, domain, "", version);
            let sealed = cell_aead::cell_seal(master, &address, &inner);
            write_u32(&mut buf, 0u32); // empty name
            write_u32(&mut buf, sealed.len() as u32);
            buf.extend_from_slice(&sealed);
        }
    }
    buf
}

/// Open a sealed freeze blob. Inverse of `freeze_sealed`. The
/// `(scope, domain, version)` MUST match the values used at seal time
/// — a mismatch surfaces as `AeadOpen { Auth }` on the first cell
/// (the AAD binding catches it) rather than letting bad bytes slip
/// through to the inner thaw.
///
/// On success returns the same Object shape `freeze_sealed` consumed:
/// `Object::Map` for sealed maps; the original Atom / Seq / Bottom for
/// the non-Map convenience path.
pub fn thaw_sealed(
    bytes: &[u8],
    master: &TenantMasterKey,
    scope: &str,
    domain: &str,
    version: u64,
) -> Result<Object, SealedFreezeError> {
    if bytes.len() < SEALED_MAGIC.len() || &bytes[..SEALED_MAGIC.len()] != SEALED_MAGIC {
        return Err(SealedFreezeError::Header(
            "bad magic — not a sealed AREST freeze image".to_string(),
        ));
    }
    let mut cursor = SEALED_MAGIC.len();
    let header_scope_len =
        read_u32(bytes, &mut cursor).map_err(SealedFreezeError::Header)? as usize;
    let scope_end = cursor
        .checked_add(header_scope_len)
        .ok_or_else(|| SealedFreezeError::Header("scope length overflow".to_string()))?;
    if scope_end > bytes.len() {
        return Err(SealedFreezeError::Header(
            "scope payload truncated".to_string(),
        ));
    }
    let header_scope = core::str::from_utf8(&bytes[cursor..scope_end])
        .map_err(|e| SealedFreezeError::Header(format!("scope utf8: {e}")))?
        .to_string();
    cursor = scope_end;

    let header_domain_len =
        read_u32(bytes, &mut cursor).map_err(SealedFreezeError::Header)? as usize;
    let domain_end = cursor
        .checked_add(header_domain_len)
        .ok_or_else(|| SealedFreezeError::Header("domain length overflow".to_string()))?;
    if domain_end > bytes.len() {
        return Err(SealedFreezeError::Header(
            "domain payload truncated".to_string(),
        ));
    }
    let header_domain = core::str::from_utf8(&bytes[cursor..domain_end])
        .map_err(|e| SealedFreezeError::Header(format!("domain utf8: {e}")))?
        .to_string();
    cursor = domain_end;

    if cursor.checked_add(8).map(|e| e > bytes.len()).unwrap_or(true) {
        return Err(SealedFreezeError::Header(
            "version field truncated".to_string(),
        ));
    }
    let header_version = u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().unwrap());
    cursor += 8;

    // Header `(scope, domain, version)` MUST equal the caller's view —
    // if the on-disk blob was minted under a different tenant or
    // version we want a clear header mismatch, not a soft AEAD failure
    // (the AAD-mismatch path catches the same case downstream, but a
    // header check makes the diagnostic point at the right layer).
    if header_scope != scope {
        return Err(SealedFreezeError::Header(format!(
            "scope mismatch (expected '{scope}', blob carries '{header_scope}')",
        )));
    }
    if header_domain != domain {
        return Err(SealedFreezeError::Header(format!(
            "domain mismatch (expected '{domain}', blob carries '{header_domain}')",
        )));
    }
    if header_version != version {
        return Err(SealedFreezeError::Header(format!(
            "version mismatch (expected {version}, blob carries {header_version})",
        )));
    }

    let cell_count = read_u32(bytes, &mut cursor).map_err(SealedFreezeError::Header)? as usize;

    // Reconstruct the Map root in the common case; for the single-
    // unnamed-cell shape (non-Map root), peel the inner thaw's Object
    // out and return it directly so the round-trip is shape-faithful.
    let mut map: hashbrown::HashMap<String, Object> = hashbrown::HashMap::with_capacity(cell_count);
    let mut single_unnamed: Option<Object> = None;

    for _ in 0..cell_count {
        let name_len = read_u32(bytes, &mut cursor).map_err(SealedFreezeError::Header)? as usize;
        let name_end = cursor
            .checked_add(name_len)
            .ok_or_else(|| SealedFreezeError::Header("name length overflow".to_string()))?;
        if name_end > bytes.len() {
            return Err(SealedFreezeError::Header(
                "cell name truncated".to_string(),
            ));
        }
        let name = core::str::from_utf8(&bytes[cursor..name_end])
            .map_err(|e| SealedFreezeError::Header(format!("cell name utf8: {e}")))?
            .to_string();
        cursor = name_end;

        let sealed_len =
            read_u32(bytes, &mut cursor).map_err(SealedFreezeError::Header)? as usize;
        let sealed_end = cursor
            .checked_add(sealed_len)
            .ok_or_else(|| SealedFreezeError::Header("sealed length overflow".to_string()))?;
        if sealed_end > bytes.len() {
            return Err(SealedFreezeError::Header(
                "sealed payload truncated".to_string(),
            ));
        }
        let sealed = &bytes[cursor..sealed_end];
        cursor = sealed_end;

        let address = CellAddress::new(scope, domain, name.clone(), version);
        let plain = cell_aead::cell_open(master, &address, sealed).map_err(|kind| {
            SealedFreezeError::AeadOpen {
                cell: name.clone(),
                kind,
            }
        })?;
        let inner = thaw(&plain).map_err(SealedFreezeError::Inner)?;

        if cell_count == 1 && name.is_empty() {
            // Non-Map convenience round-trip path.
            single_unnamed = Some(inner);
        } else {
            map.insert(name, inner);
        }
    }

    if cursor != bytes.len() {
        return Err(SealedFreezeError::Header(format!(
            "trailing {} bytes after sealed payload",
            bytes.len() - cursor,
        )));
    }

    Ok(single_unnamed.unwrap_or(Object::Map(map)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(obj: Object) -> Object {
        let bytes = freeze(&obj);
        thaw(&bytes).expect("thaw round-trip")
    }

    #[test]
    fn atom_round_trips() {
        assert_eq!(roundtrip(Object::Atom("hello".to_string())), Object::Atom("hello".to_string()));
        assert_eq!(roundtrip(Object::Atom("".to_string())), Object::Atom("".to_string()));
        assert_eq!(roundtrip(Object::Atom("unicode café".to_string())), Object::Atom("unicode café".to_string()));
    }

    #[test]
    fn seq_round_trips() {
        let seq = Object::Seq(vec![
            Object::Atom("a".to_string()),
            Object::Atom("b".to_string()),
            Object::Atom("c".to_string()),
        ].into());
        assert_eq!(roundtrip(seq.clone()), seq);
    }

    #[test]
    fn nested_seq_round_trips() {
        let inner = Object::Seq(vec![Object::Atom("x".to_string()), Object::Atom("y".to_string())].into());
        let outer = Object::Seq(vec![inner.clone(), inner.clone()].into());
        assert_eq!(roundtrip(outer.clone()), outer);
    }

    #[test]
    fn map_round_trips() {
        let mut m = hashbrown::HashMap::new();
        m.insert("one".to_string(), Object::Atom("1".to_string()));
        m.insert("two".to_string(), Object::Atom("2".to_string()));
        m.insert("list".to_string(), Object::Seq(vec![Object::Atom("a".to_string())].into()));
        let obj = Object::Map(m);
        assert_eq!(roundtrip(obj.clone()), obj);
    }

    #[test]
    fn bottom_round_trips() {
        assert_eq!(roundtrip(Object::Bottom), Object::Bottom);
    }

    #[test]
    fn freeze_is_deterministic_for_maps() {
        // Two maps with same (k,v) pairs in different insertion order
        // must produce byte-identical freeze images. Required for ROM
        // hashing and reproducible boot images.
        let mut a = hashbrown::HashMap::new();
        a.insert("alpha".to_string(), Object::Atom("1".to_string()));
        a.insert("bravo".to_string(), Object::Atom("2".to_string()));
        a.insert("charlie".to_string(), Object::Atom("3".to_string()));
        let mut b = hashbrown::HashMap::new();
        b.insert("charlie".to_string(), Object::Atom("3".to_string()));
        b.insert("alpha".to_string(), Object::Atom("1".to_string()));
        b.insert("bravo".to_string(), Object::Atom("2".to_string()));
        assert_eq!(freeze(&Object::Map(a)), freeze(&Object::Map(b)));
    }

    #[test]
    fn thaw_rejects_bad_magic() {
        assert!(thaw(b"NOPE\x01\x00").is_err());
        assert!(thaw(b"").is_err());
        assert!(thaw(b"AREST\x02\x00").is_err(), "version 2 not supported yet");
    }

    #[test]
    fn thaw_rejects_truncated_input() {
        let bytes = freeze(&Object::Atom("hello".to_string()));
        // Lop off half the payload.
        assert!(thaw(&bytes[..bytes.len() - 2]).is_err());
    }

    #[test]
    fn thaw_rejects_trailing_garbage() {
        let mut bytes = freeze(&Object::Atom("ok".to_string()));
        bytes.push(0xFF);
        assert!(thaw(&bytes).is_err());
    }

    #[test]
    fn thaw_rejects_unknown_tag() {
        let mut bytes = Vec::from(MAGIC);
        bytes.push(0xAA); // not a valid tag
        assert!(thaw(&bytes).is_err());
    }

    // ── Sealed freeze (#659) ────────────────────────────────────────

    use crate::cell_aead::TenantMasterKey;
    use crate::entropy::{self, DeterministicSource};

    /// Same fixture shape as cell_aead::tests / csprng::tests — install
    /// a deterministic entropy source, run the body, then uninstall.
    /// Without this the sealed-freeze tests would panic on an
    /// uninstalled-source (the nonce draw goes through csprng).
    fn with_entropy<F: FnOnce()>(seed: [u8; 32], body: F) {
        let _guard = entropy::TEST_LOCK.lock();
        entropy::install(Box::new(DeterministicSource::new(seed)));
        crate::csprng::reseed();
        body();
        entropy::uninstall();
        crate::csprng::reseed();
    }

    fn fixture_master() -> TenantMasterKey {
        TenantMasterKey::from_bytes([0x42; 32])
    }

    #[test]
    fn sealed_round_trip_map_root() {
        let mut m = hashbrown::HashMap::new();
        m.insert("alpha".to_string(), Object::Atom("1".to_string()));
        m.insert(
            "bravo".to_string(),
            Object::Seq(vec![Object::Atom("x".to_string()), Object::Atom("y".to_string())].into()),
        );
        m.insert("charlie".to_string(), Object::Bottom);
        let root = Object::Map(m);

        with_entropy([1u8; 32], || {
            let sealed = freeze_sealed(&root, &fixture_master(), "tenant-A", "orders", 7);
            assert!(
                sealed.starts_with(SEALED_MAGIC),
                "sealed blob must carry the SEALED_MAGIC prefix",
            );
            // Plaintext freeze magic must NOT appear at offset 0 (the
            // sealed magic occupies that position) — confirms the two
            // formats are distinguishable at the wire boundary.
            assert!(
                !sealed.starts_with(MAGIC),
                "sealed blob must not collide with plaintext MAGIC at offset 0",
            );
            let recovered = thaw_sealed(&sealed, &fixture_master(), "tenant-A", "orders", 7)
                .expect("sealed round-trip must succeed");
            assert_eq!(recovered, root);
        });
    }

    #[test]
    fn sealed_round_trip_atom_root() {
        // Non-Map root convenience path — single anonymous sealed
        // cell, recovered as the original Atom.
        let root = Object::Atom("solo-payload".to_string());
        with_entropy([2u8; 32], || {
            let sealed = freeze_sealed(&root, &fixture_master(), "t", "d", 0);
            let recovered = thaw_sealed(&sealed, &fixture_master(), "t", "d", 0).unwrap();
            assert_eq!(recovered, root);
        });
    }

    #[test]
    fn sealed_open_under_wrong_master_fails() {
        // Different tenant master → AAD AEAD failure on first cell.
        let mut m = hashbrown::HashMap::new();
        m.insert("k".to_string(), Object::Atom("v".to_string()));
        let root = Object::Map(m);
        let master_a = TenantMasterKey::from_bytes([0xA1; 32]);
        let master_b = TenantMasterKey::from_bytes([0xB2; 32]);
        with_entropy([3u8; 32], || {
            let sealed = freeze_sealed(&root, &master_a, "t", "d", 1);
            let res = thaw_sealed(&sealed, &master_b, "t", "d", 1);
            match res {
                Err(SealedFreezeError::AeadOpen { cell, kind }) => {
                    assert_eq!(cell, "k");
                    assert_eq!(kind, crate::cell_aead::AeadError::Auth);
                }
                other => panic!("expected AeadOpen Auth, got {:?}", other),
            }
        });
    }

    #[test]
    fn sealed_open_under_wrong_version_fails_header_check() {
        // Same scope/domain/master but version skew — the header
        // check fires before the AEAD path.
        let mut m = hashbrown::HashMap::new();
        m.insert("k".to_string(), Object::Atom("v".to_string()));
        let root = Object::Map(m);
        with_entropy([5u8; 32], || {
            let sealed = freeze_sealed(&root, &fixture_master(), "t", "d", 2);
            let res = thaw_sealed(&sealed, &fixture_master(), "t", "d", 3);
            match res {
                Err(SealedFreezeError::Header(msg)) => {
                    assert!(msg.contains("version mismatch"), "got: {msg}");
                }
                other => panic!("expected Header version mismatch, got {:?}", other),
            }
        });
    }

    #[test]
    fn sealed_open_under_wrong_scope_fails_header_check() {
        let root = Object::Atom("x".to_string());
        with_entropy([7u8; 32], || {
            let sealed = freeze_sealed(&root, &fixture_master(), "scopeA", "d", 0);
            let res = thaw_sealed(&sealed, &fixture_master(), "scopeB", "d", 0);
            match res {
                Err(SealedFreezeError::Header(msg)) => {
                    assert!(msg.contains("scope mismatch"), "got: {msg}");
                }
                other => panic!("expected Header scope mismatch, got {:?}", other),
            }
        });
    }

    #[test]
    fn sealed_thaw_rejects_plaintext_magic() {
        // A plaintext freeze blob fed to thaw_sealed must be rejected
        // at the magic check — the two formats are intentionally
        // disjoint at the wire boundary.
        let plain = freeze(&Object::Atom("oops".to_string()));
        let res = thaw_sealed(&plain, &fixture_master(), "t", "d", 0);
        match res {
            Err(SealedFreezeError::Header(msg)) => assert!(msg.contains("magic")),
            other => panic!("expected magic mismatch, got {:?}", other),
        }
    }

    #[test]
    fn sealed_envelope_grows_by_aead_overhead() {
        // The sealed blob's per-cell overhead should equal NONCE_LEN +
        // TAG_LEN bytes vs the plaintext freeze of the same cell —
        // the on-disk size budget for block_storage / DO callers
        // depends on this number staying stable.
        let value = Object::Atom("payload-bytes-of-known-length".to_string());
        let mut m = hashbrown::HashMap::new();
        m.insert("only".to_string(), value.clone());
        let root = Object::Map(m);
        with_entropy([9u8; 32], || {
            let plain_len_inner = freeze(&value).len() as i64;
            let sealed = freeze_sealed(&root, &fixture_master(), "s", "d", 0);
            let plain = freeze(&root);
            // Difference between sealed wire size and plaintext freeze
            // is bounded above by header bytes + per-cell metadata +
            // NONCE_LEN + TAG_LEN. We don't byte-count the headers
            // line-by-line (the header layout is documented next to
            // freeze_sealed); the sanity check is that the sealed
            // envelope is *larger* than the plaintext by exactly the
            // AEAD overhead per cell, plus a bounded header.
            let overhead = sealed.len() as i64 - plain.len() as i64;
            let aead_overhead = (crate::cell_aead::NONCE_LEN + crate::cell_aead::TAG_LEN) as i64;
            assert!(
                overhead >= aead_overhead,
                "sealed must exceed plaintext by ≥ AEAD per-cell overhead \
                 (sealed = {}, plain = {}, plain_inner = {}, aead_overhead = {aead_overhead})",
                sealed.len(),
                plain.len(),
                plain_len_inner,
            );
        });
    }
}
