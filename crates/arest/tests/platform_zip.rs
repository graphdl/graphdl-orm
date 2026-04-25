// crates/arest/tests/platform_zip.rs
//
// Integration test for `crates/arest/src/platform/zip.rs` (#404). The
// platform/ subdir is not yet declared as `pub mod platform;` in
// `lib.rs` (the engine-owner agent wires that in a follow-up); to
// keep the module testable today this binary `#[path]`-includes the
// implementation file directly.
//
// `zip.rs` was written with `crate::*` references so that, once the
// `pub mod platform;` line lands, it slots into the lib without
// edits. To make those references resolve in this test binary we
// re-export the relevant `arest` modules at the test crate root
// BEFORE pulling the file in via `#[path]`.

#![allow(dead_code)]

pub use arest::ast;
pub use arest::sync;
pub use arest::command;

#[path = "../src/platform/zip.rs"]
mod zip_under_test;

use ast::{Object, fact_from_pairs};
use hashbrown::HashMap;

/// Spec test from the #404 brief: build a synthetic Directory with
/// three Files, walk + ZIP it, decode the archive, assert byte-
/// identity for every File's payload.
#[test]
fn synthetic_directory_three_files_round_trips() {
    let d = Object::phi();

    let d = ast::cell_push("Directory_has_Name",
        fact_from_pairs(&[("Directory", "rootDir"), ("Name", "rootDir")]), &d);
    let d = ast::cell_push("Directory_has_Name",
        fact_from_pairs(&[("Directory", "subDir"), ("Name", "subDir")]), &d);
    let d = ast::cell_push("Directory_has_parent_Directory",
        fact_from_pairs(&[("Directory", "subDir"), ("parent Directory", "rootDir")]), &d);

    let payloads: [(&str, &str, &str, &[u8]); 3] = [
        ("f1", "alpha.txt", "rootDir", b"alpha contents"),
        ("f2", "beta.bin",  "rootDir", b"\x00\x01\x02\x03\x04\xff"),
        ("f3", "gamma.json","subDir",  b"{\"value\": 7}"),
    ];
    let mut d_acc = d;
    for (id, name, parent, bytes) in payloads.iter() {
        d_acc = ast::cell_push("File_has_Name",
            fact_from_pairs(&[("File", id), ("Name", name)]), &d_acc);
        d_acc = ast::cell_push("File_is_in_Directory",
            fact_from_pairs(&[("File", id), ("Directory", parent)]), &d_acc);
        let cref = zip_under_test::encode_content_ref(bytes);
        d_acc = ast::cell_push("File_has_ContentRef",
            fact_from_pairs(&[("File", id), ("ContentRef", &cref)]), &d_acc);
    }

    let entries = zip_under_test::walk_directory_subtree("rootDir", &d_acc).expect("walk");
    let by_path: HashMap<String, Vec<u8>> = entries.iter().cloned().collect();
    assert_eq!(by_path.get("alpha.txt").unwrap(), &b"alpha contents".to_vec());
    assert_eq!(by_path.get("beta.bin").unwrap(),  &b"\x00\x01\x02\x03\x04\xff".to_vec());
    assert_eq!(by_path.get("subDir/gamma.json").unwrap(), &b"{\"value\": 7}".to_vec());

    let archive = zip_under_test::encode_zip_stored(&entries);
    let decoded = zip_under_test::decode_zip_stored(&archive).expect("decode");
    let decoded_map: HashMap<String, Vec<u8>> = decoded.into_iter().collect();
    for (path, expected) in entries {
        assert_eq!(decoded_map.get(&path).expect("path present"), &expected);
    }
}

/// End-to-end: zip a Directory subtree, then unzip the resulting
/// File into a fresh target Directory, and verify every original
/// File's bytes resurface under the unzip target.
#[test]
fn zip_then_unzip_round_trip() {
    let d = Object::phi();
    let d = ast::cell_push("Directory_has_Name",
        fact_from_pairs(&[("Directory", "src"), ("Name", "src")]), &d);
    let d = ast::cell_push("Directory_has_Name",
        fact_from_pairs(&[("Directory", "dst"), ("Name", "dst")]), &d);

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
        let cref = zip_under_test::encode_content_ref(bytes);
        d_acc = ast::cell_push("File_has_ContentRef",
            fact_from_pairs(&[("File", id), ("ContentRef", &cref)]), &d_acc);
    }

    let (zip_id, d_after_zip) = zip_under_test::zip_directory_in("src", &d_acc).expect("zip");
    let (d_after_unzip, _created) = zip_under_test::unzip_file_in(&zip_id, "dst", &d_after_zip).expect("unzip");

    let in_dir = ast::fetch_or_phi("File_is_in_Directory", &d_after_unzip);
    let in_dst: Vec<String> = in_dir.as_seq().unwrap_or(&[]).iter().filter_map(|f| {
        if ast::binding(f, "Directory") == Some("dst") {
            ast::binding(f, "File").map(|s| s.to_string())
        } else { None }
    }).collect();
    assert!(!in_dst.is_empty(), "expected at least one file under dst");

    let mut found: HashMap<String, Vec<u8>> = HashMap::new();
    for fid in &in_dst {
        let name = zip_under_test::file_name_for(fid, &d_after_unzip).expect("name");
        let bytes = zip_under_test::file_content_bytes(fid, &d_after_unzip).expect("bytes");
        found.insert(name, bytes);
    }
    for (_id, name, bytes) in payloads.iter() {
        assert_eq!(found.get(*name).expect("name present in dst"), &bytes.to_vec());
    }
}

/// CRC-32 vector smoke (covers the inlined polynomial constant).
#[test]
fn crc32_vectors() {
    assert_eq!(zip_under_test::crc32_for_test(b""),         0x0000_0000);
    assert_eq!(zip_under_test::crc32_for_test(b"a"),        0xE8B7_BE43);
    assert_eq!(zip_under_test::crc32_for_test(b"123456789"), 0xCBF4_3926);
}
