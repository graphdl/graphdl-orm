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
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

const MAGIC: &[u8] = b"AREST\x01";
const TAG_ATOM: u8 = 0x00;
const TAG_SEQ: u8 = 0x01;
const TAG_MAP: u8 = 0x02;
const TAG_BOTTOM: u8 = 0x03;

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
            let mut items = Vec::with_capacity(n);
            for _ in 0..n { items.push(read_object(bytes, cursor)?); }
            Ok(Object::Seq(items.into()))
        }
        TAG_MAP => {
            let n = read_u32(bytes, cursor)? as usize;
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
}
