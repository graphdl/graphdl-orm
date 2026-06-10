// crates/arest-kernel/src/syscall/getdents64.rs
//
// Linux x86_64 syscall 217: `getdents64(int fd, struct linux_dirent64
// *dirp, unsigned int count)` — directory enumeration
// (getdents64-file-population; pairs with openat's Directory fds).
//
// Tier-1 directory model
// ----------------------
// A directory is a PROJECTION, not a stored object. The child list of
// `<p>` is synthesized per call from two disjoint sources:
//
//   * `synthetic_fs::list_children(<p>)` — the fixed tables (`/` roots,
//     `/dev/*` devices, `/proc/*` rendered files).
//   * The `File_has_Name` facts: every name strictly under `<p>/`
//     contributes its NEXT path segment — an exact remaining segment is
//     a regular file (DT_REG), a deeper one is a subdirectory (DT_DIR).
//     Duplicate segments dedupe (two files under `/a/` yield one `a`
//     child of `/`). This is a derivation-shaped read — flagged on
//     procedural-code-to-substrate as a substrate-native candidate; the
//     kernel hot path keeps the procedural form.
//
// `.` and `..` are emitted first (both DT_DIR) for readdir fidelity —
// busybox `ls -a` expects them; plain `ls` filters dot-names itself.
//
// struct linux_dirent64 (from `<linux/dirent.h>`; layout is ABI-fixed):
//
//   u64  d_ino;      // offset  0
//   i64  d_off;      // offset  8 — cookie of the NEXT entry
//   u16  d_reclen;   // offset 16 — total record length, 8-aligned
//   u8   d_type;     // offset 18 — DT_* constant
//   char d_name[];   // offset 19 — NUL-terminated name
//
// Return: bytes written into `dirp`; 0 at end-of-directory; negative
// errno on failure (`-EBADF` unknown fd, `-ENOTDIR` non-directory fd,
// `-EFAULT` null buffer, `-EINVAL` buffer too small for the next
// entry — Linux semantics).
//
// The per-fd cursor lives on `FdEntry::Directory` — repeated calls page
// through the child list and the cursor survives across calls (musl's
// readdir issues getdents64 until 0). The child list is re-synthesized
// per call: a File created between two getdents64 calls may shift the
// listing (same class of anomaly POSIX permits for readdir).

use alloc::string::String;
use alloc::vec::Vec;
use arest::ast::{self, Object};

use crate::process::current_process_fd_table;
use crate::process::fd_table::FdEntry;
use crate::syscall::dispatch::{EBADF, EFAULT, EINVAL};

/// `SYS_getdents64 = 217` per `arch/x86/entry/syscalls/syscall_64.tbl`.
pub const SYS_GETDENTS64: u64 = 217;

/// `ENOTDIR = 20` per `<asm-generic/errno-base.h>`.
pub const ENOTDIR: i64 = 20;

/// `d_type` values from `<dirent.h>`.
pub const DT_CHR: u8 = 2;
pub const DT_DIR: u8 = 4;
pub const DT_REG: u8 = 8;

/// Fixed header size before `d_name` (8 + 8 + 2 + 1).
const DIRENT_HEADER: usize = 19;

/// Handle `getdents64(fd, dirp, count)`.
pub fn handle(fd: u64, dirp: u64, count: u64) -> i64 {
    let fd = fd as i32;
    if dirp == 0 {
        return -EFAULT;
    }

    // Snapshot the directory path + cursor from the fd entry.
    let (path, cursor) = match current_process_fd_table(|maybe| match maybe {
        Some(table) => match table.lookup(fd) {
            Some(FdEntry::Directory { path, cursor }) => {
                Ok((path.clone(), *cursor))
            }
            Some(_) => Err(-ENOTDIR),
            None => Err(-EBADF),
        },
        None => Err(-EBADF),
    }) {
        Ok(pair) => pair,
        Err(errno) => return errno,
    };

    // Synthesize the child list (deterministic order — see children()).
    let children = children(&path);
    if cursor as usize >= children.len() {
        return 0; // end of directory
    }

    // Serialize entries from the cursor until the buffer is full.
    let mut written: usize = 0;
    let mut emitted: u64 = 0;
    for (idx, (name, d_type)) in children.iter().enumerate().skip(cursor as usize) {
        let reclen = align8(DIRENT_HEADER + name.len() + 1);
        if written + reclen > count as usize {
            if emitted == 0 {
                // First entry doesn't fit at all — Linux returns EINVAL.
                return -EINVAL;
            }
            break;
        }
        // SAFETY: writes into the userspace buffer at dirp+written for
        // reclen bytes, bounded by `count` (checked above). Tier-1
        // identity mapping makes the raw pointer valid; #561's
        // copy_to_user hardening applies here the same as read.rs.
        unsafe {
            let base = (dirp as usize + written) as *mut u8;
            // d_ino — a stable per-name hash (tier-1 has no inode table).
            write_u64(base, 0, fnv1a64(name.as_bytes()));
            // d_off — cookie of the NEXT entry (its child index + 1).
            write_u64(base, 8, (idx as u64 + 1) as u64);
            // d_reclen.
            write_u16(base, 16, reclen as u16);
            // d_type.
            *base.add(18) = *d_type;
            // d_name + NUL, then zero-pad to the 8-aligned reclen.
            for (i, b) in name.as_bytes().iter().enumerate() {
                *base.add(DIRENT_HEADER + i) = *b;
            }
            for i in (DIRENT_HEADER + name.len())..reclen {
                *base.add(i) = 0;
            }
        }
        written += reclen;
        emitted += 1;
    }

    // Advance the per-fd cursor by the number of entries emitted.
    let _ = current_process_fd_table(|maybe| {
        if let Some(table) = maybe {
            if let Some(FdEntry::Directory { cursor, .. }) = table.lookup_mut(fd) {
                *cursor += emitted;
            }
        }
        0i64
    });

    written as i64
}

/// Synthesize the FULL ordered child list of `path`: `.` + `..`, then
/// the synthetic-fs children, then the File-graph next-segment children
/// (deduped, sorted; segments already named by synthetic-fs dedupe too).
fn children(path: &str) -> Vec<(String, u8)> {
    let mut out: Vec<(String, u8)> = Vec::new();
    out.push((String::from("."), DT_DIR));
    out.push((String::from(".."), DT_DIR));

    let mut seen: Vec<String> = Vec::new();
    if let Some(synth) = crate::synthetic_fs::list_children(path) {
        for (name, kind) in synth {
            let d_type = match kind {
                crate::synthetic_fs::ChildKind::Dir => DT_DIR,
                crate::synthetic_fs::ChildKind::File => DT_REG,
                crate::synthetic_fs::ChildKind::CharDevice => DT_CHR,
            };
            seen.push(name.clone());
            out.push((name, d_type));
        }
    }

    let mut file_children = crate::system::with_state(|state| {
        file_graph_children_in(path, state)
    })
    .unwrap_or_default();
    file_children.retain(|(name, _)| !seen.contains(name));
    out.extend(file_children);
    out
}

/// File-graph half of `children` — pure-state for fixture tests. Every
/// `File_has_Name` fact strictly under `<dir>/` contributes its next
/// path segment: the whole remainder ⇒ a file (DT_REG); a deeper path
/// ⇒ a subdirectory (DT_DIR). Sorted + deduped (a dir segment beats a
/// same-named file segment, which cannot happen for real File names
/// anyway since a name is either exact or deeper).
pub fn file_graph_children_in(dir: &str, state: &Object) -> Vec<(String, u8)> {
    let prefix = if dir == "/" {
        String::from("/")
    } else {
        alloc::format!("{}/", dir)
    };
    let mut out: Vec<(String, u8)> = Vec::new();
    let cell = ast::fetch_or_phi("File_has_Name", state);
    if let Some(facts) = cell.as_seq() {
        for fact in facts {
            let Some(name) = ast::binding(fact, "Name") else { continue };
            let Some(rest) = name.strip_prefix(prefix.as_str()) else { continue };
            if rest.is_empty() {
                continue;
            }
            let (segment, d_type) = match rest.find('/') {
                Some(i) => (&rest[..i], DT_DIR),
                None => (rest, DT_REG),
            };
            if segment.is_empty() {
                continue;
            }
            if !out.iter().any(|(s, _)| s == segment) {
                out.push((String::from(segment), d_type));
            }
        }
    }
    out.sort();
    out
}

/// Round `n` up to the next multiple of 8 (dirent records are 8-aligned
/// so the next record's u64 fields stay naturally aligned).
fn align8(n: usize) -> usize {
    (n + 7) & !7
}

/// FNV-1a over the name bytes — a stable synthetic `d_ino` (tier-1 has
/// no inode table; userspace only needs inos to be consistent within a
/// listing).
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

unsafe fn write_u64(base: *mut u8, offset: usize, v: u64) {
    for (i, b) in v.to_le_bytes().iter().enumerate() {
        *base.add(offset + i) = *b;
    }
}

unsafe fn write_u16(base: *mut u8, offset: usize, v: u16) {
    for (i, b) in v.to_le_bytes().iter().enumerate() {
        *base.add(offset + i) = *b;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arest::ast::{cell_push, fact_from_pairs, Object};

    fn fixture_state(names: &[&str]) -> Object {
        let mut s = Object::phi();
        for n in names {
            s = cell_push(
                "File_has_Name",
                fact_from_pairs(&[("File", "cell-x"), ("Name", n)]),
                &s,
            );
        }
        s
    }

    /// Next-segment synthesis: exact remainder ⇒ file, deeper ⇒ dir,
    /// duplicates dedupe, output sorted.
    #[test]
    fn file_graph_children_segments_and_kinds() {
        let s = fixture_state(&[
            "/etc/motd",
            "/etc/conf/a.toml",
            "/etc/conf/b.toml",
            "/readme.txt",
        ]);
        let root = file_graph_children_in("/", &s);
        assert_eq!(root, alloc::vec![
            (String::from("etc"), DT_DIR),
            (String::from("readme.txt"), DT_REG),
        ]);
        let etc = file_graph_children_in("/etc", &s);
        assert_eq!(etc, alloc::vec![
            (String::from("conf"), DT_DIR),
            (String::from("motd"), DT_REG),
        ]);
        let conf = file_graph_children_in("/etc/conf", &s);
        assert_eq!(conf, alloc::vec![
            (String::from("a.toml"), DT_REG),
            (String::from("b.toml"), DT_REG),
        ]);
        // A non-existent directory has no children.
        assert!(file_graph_children_in("/nope", &s).is_empty());
    }

    /// Record sizes are 8-aligned and the header is ABI-fixed at 19.
    #[test]
    fn dirent_packing_constants() {
        assert_eq!(DIRENT_HEADER, 19);
        assert_eq!(align8(19 + 1 + 1), 24, "1-char name rounds to 24");
        assert_eq!(align8(19 + 5 + 1), 32, "5-char name rounds to 32");
        assert_eq!(align8(24), 24, "exact multiples stay");
    }

    /// Helper: install a fresh Process so the handler has somewhere to
    /// allocate fds against (mirrors openat's `install_test_process`).
    fn install_test_process() {
        use crate::process::address_space::AddressSpace;
        use crate::process::{current_process_install, Process};
        let address_space = AddressSpace::new(0x40_1000);
        current_process_install(Process::new(7, address_space));
    }

    /// Serialization writes well-formed records into a raw buffer and
    /// the cursor pages across calls. Drives `handle` end-to-end with
    /// an installed process (mirrors the openat tests' harness).
    #[test]
    fn handle_serializes_and_pages() {
        use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
        use crate::process::{current_process_fd_table, current_process_uninstall};
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process();
        // Open a Directory fd directly via the table (the openat path
        // is covered by openat's own tests).
        let fd = current_process_fd_table(|t| {
            t.unwrap()
                .allocate(crate::process::fd_table::directory("/dev"))
                .unwrap()
        });

        // First call: a buffer big enough for everything under /dev
        // (., .., null, zero, random, tty — 6 records ≤ 32 bytes each).
        let mut buf = [0u8; 256];
        let n = handle(fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
        assert!(n > 0, "first call returns bytes, got {}", n);

        // Walk the records: collect (name, d_type), validate reclen
        // alignment and NUL termination.
        let mut names: Vec<(String, u8)> = Vec::new();
        let mut off = 0usize;
        while off < n as usize {
            let reclen =
                u16::from_le_bytes([buf[off + 16], buf[off + 17]]) as usize;
            assert_eq!(reclen % 8, 0, "reclen 8-aligned");
            let d_type = buf[off + 18];
            let name_bytes = &buf[off + DIRENT_HEADER..off + reclen];
            let nul = name_bytes.iter().position(|b| *b == 0).expect("NUL");
            names.push((
                String::from_utf8(name_bytes[..nul].to_vec()).expect("utf8"),
                d_type,
            ));
            off += reclen;
        }
        let just_names: Vec<&str> =
            names.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            just_names,
            alloc::vec![".", "..", "null", "zero", "random", "tty"],
            "/dev children in table order behind . and .."
        );
        assert!(
            names.iter().skip(2).all(|(_, t)| *t == DT_CHR),
            "devices are DT_CHR"
        );

        // Second call: cursor at end → 0 (end of directory).
        let n2 = handle(fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
        assert_eq!(n2, 0, "exhausted directory returns 0");

        current_process_uninstall();
    }

    /// A buffer too small for even the first record returns -EINVAL;
    /// a non-directory fd returns -ENOTDIR; an unknown fd -EBADF.
    #[test]
    fn handle_error_paths() {
        use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
        use crate::process::{current_process_fd_table, current_process_uninstall};
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process();
        let dir_fd = current_process_fd_table(|t| {
            t.unwrap()
                .allocate(crate::process::fd_table::directory("/proc"))
                .unwrap()
        });
        let file_fd = current_process_fd_table(|t| {
            t.unwrap()
                .allocate(crate::process::fd_table::file("cell-y"))
                .unwrap()
        });

        let mut buf = [0u8; 8]; // smaller than any record
        assert_eq!(
            handle(dir_fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64),
            -EINVAL
        );
        let mut big = [0u8; 256];
        assert_eq!(
            handle(file_fd as u64, big.as_mut_ptr() as u64, big.len() as u64),
            -ENOTDIR
        );
        assert_eq!(
            handle(9999, big.as_mut_ptr() as u64, big.len() as u64),
            -EBADF
        );
        assert_eq!(handle(dir_fd as u64, 0, 256), -EFAULT);

        current_process_uninstall();
    }
}
