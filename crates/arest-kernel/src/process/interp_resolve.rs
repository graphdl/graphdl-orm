// crates/arest-kernel/src/process/interp_resolve.rs
//
// #522: the kernel's program-interpreter resolver — the platform-binding
// runtime function that turns a PT_INTERP path (e.g.
// `/lib/ld-musl-x86_64.so.1`) into the interpreter ELF's bytes by reading
// it out of the population P.
//
// Why this is not an architectural choice
// ---------------------------------------
// `elf::load_program` takes the interpreter source as an injected
// `resolve_interp` closure precisely so the *production* source isn't
// baked into the pure loader. AREST's model settles what that source is:
// the whitepaper's Platform Binding (§4.4) registers runtime functions
// into DEFS that read facts from P, and the FILE cell IS P — so a file,
// including an interpreter binary, is a `File` fact in P (a
// `File_has_Name` + `File_has_ContentRef` pair, #398). There is no
// "vendored asset vs /lib mount vs ESP" fork: the interpreter lives in
// the population, resolved through the SAME File-cell surface
// `syscall::openat` already resolves a path against. This module is that
// resolver.
//
// Resolution order mirrors `openat::handle`:
//   1. `synthetic_fs::resolve` — the /proc-style virtual surface
//      (uncommon for an interpreter, but kept for parity with openat).
//   2. The File-cell graph: `File_has_Name` → cell id
//      (`openat::lookup_file_cell_id_in`), then the whole content via
//      `syscall::read::read_file_cell_bytes` (inline bytes are pure and
//      host-available; the off-disk region shape reads through
//      `block_storage`, UEFI x86_64, and is None elsewhere).

use alloc::vec::Vec;
use arest::ast::Object;

use crate::syscall::openat::lookup_file_cell_id_in;
use crate::syscall::read::read_file_cell_bytes;
use crate::synthetic_fs;

/// Resolve `path` to the interpreter ELF's bytes against the LIVE SYSTEM
/// population. Returns `None` when no SYSTEM state is installed or the
/// path names no readable file. Pass this (or a closure delegating to
/// it) as the `resolve_interp` argument to `elf::load_program`.
pub fn resolve_interp_bytes(path: &[u8]) -> Option<Vec<u8>> {
    crate::system::with_state(|state| resolve_interp_bytes_in(path, state))?
}

/// Pure-state version of `resolve_interp_bytes` — resolves against the
/// supplied `state` rather than the live SYSTEM. The testable surface,
/// mirroring `openat::lookup_file_cell_id_in`.
pub fn resolve_interp_bytes_in(path: &[u8], state: &Object) -> Option<Vec<u8>> {
    // PT_INTERP paths are byte strings; a non-UTF-8 path can't name a
    // synthetic file or a File `File_has_Name` (both stored as text).
    let path_str = core::str::from_utf8(path).ok()?;
    // (1) /proc-style synthetic file first, matching openat's order.
    if let Some(bytes) = synthetic_fs::resolve(path_str) {
        return Some(bytes);
    }
    // (2) The interpreter as a File fact in P: name → cell id → bytes.
    // `read_file_cell_bytes` is the host-available File-content reader
    // (the read(2) File-fd path's core): the inline ContentRef shape is
    // decoded purely, the off-disk region shape reads via block_storage
    // (UEFI x86_64) and is None elsewhere. count = u64::MAX reads the
    // whole file — both the inline clamp and the region clamp saturate,
    // so there's no overflow. (file_serve's region reader is UEFI-gated;
    // routing through read.rs keeps this resolver host-compilable.)
    let cell_id = lookup_file_cell_id_in(path_str, state)?;
    read_file_cell_bytes(&cell_id, 0, u64::MAX, state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arest::ast::{cell_push, fact_from_pairs};

    /// An interpreter staged as a `File` fact in P (name → inline
    /// ContentRef) resolves to its bytes through the same File-cell
    /// surface `openat` uses. This is the production path — a real
    /// ld-musl binary is just a `File` in the population.
    #[test]
    fn resolves_interpreter_from_file_cell_inline() {
        // ELF-ish sentinel bytes, hex-encoded as an inline ContentRef.
        let cref = "<INLINE,7f454c460201>";
        let phi = Object::phi();
        let d = cell_push(
            "File_has_Name",
            fact_from_pairs(&[("File", "f1"), ("Name", "/lib/ld-musl-x86_64.so.1")]),
            &phi,
        );
        let state = cell_push(
            "File_has_ContentRef",
            fact_from_pairs(&[("File", "f1"), ("ContentRef", cref)]),
            &d,
        );

        let got = resolve_interp_bytes_in(b"/lib/ld-musl-x86_64.so.1", &state)
            .expect("interpreter File resolves to its bytes");
        assert_eq!(got, alloc::vec![0x7f, b'E', b'L', b'F', 0x02, 0x01]);
    }

    /// A /proc-style synthetic path resolves through `synthetic_fs`
    /// without needing a File fact — parity with openat's first branch.
    #[test]
    fn resolves_synthetic_path_without_file_cell() {
        let empty = Object::phi();
        // /proc/cpuinfo is a modelled synthetic file; its exact bytes are
        // synthetic_fs's concern — we only assert the resolver routes to
        // it (non-empty) rather than falling through to the File cell.
        let got = resolve_interp_bytes_in(b"/proc/cpuinfo", &empty);
        assert!(
            got.map(|b| !b.is_empty()).unwrap_or(false),
            "synthetic path resolves via synthetic_fs"
        );
    }

    /// A path that is neither a synthetic file nor a named File returns
    /// `None` — the loader maps that to InterpreterUnavailable.
    #[test]
    fn unknown_path_resolves_to_none() {
        let empty = Object::phi();
        assert_eq!(
            resolve_interp_bytes_in(b"/lib/ld-musl-x86_64.so.1", &empty),
            None
        );
    }
}
