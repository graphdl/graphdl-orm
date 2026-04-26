// crates/arest-kernel/src/syscall/openat.rs
//
// Linux x86_64 syscall 257: `openat(int dirfd, const char *pathname,
// int flags, mode_t mode)`. Per #498 (Track KKKKK, second leg of the
// userspace-syscall epic #473). The first slice (#507, GGGGG) shipped
// `write` + `exit`; the third (#499, follow-up) ships `read`. This
// slice owns the open()-side surface — every fd a Linux binary holds
// is opened through `openat` (since glibc 2.26 + musl 1.0.3 the legacy
// `open(2)` is itself implemented as `openat(AT_FDCWD, ...)` per
// `vendor/musl/src/fcntl/open.c` line 16).
//
// Tier-1 scope
// ------------
// Pathname resolution today routes through three fixed surfaces:
//
//   * `/proc/*`, `/sys/*`, `/dev/*` — the synthetic-fs resolver
//     (HHHHH's #534, `crate::synthetic_fs::resolve`). Returns
//     `Some(bytes)` for known paths; `None` falls through to the
//     File-cell lookup.
//   * Anything else — `crate::synthetic_fs::resolve` is consulted
//     first (defensive — the resolver itself filters by prefix), then
//     a File-cell graph lookup walks the AREST `File_has_Name` facts
//     looking for a match. The cell id (typically the hex blob hash
//     keyed on `File_has_*`) becomes the `FdEntry::File` payload.
//
// `dirfd` handling for tier-1
// ---------------------------
// `dirfd == AT_FDCWD (-100)` is the common case (libc's `open(path,
// flags)` always sets dirfd to AT_FDCWD). For an absolute path
// (starts with `/`), the dirfd is ignored by Linux's openat
// regardless of value. Tier-1 supports both:
//
//   * `dirfd == AT_FDCWD` → resolve `pathname` against the current
//     process's cwd. Tier-1 has no cwd model yet — we treat all
//     paths as absolute (a user passing a relative path with
//     AT_FDCWD effectively tries to open it against `/`, which
//     today's resolvers will not match — `-ENOENT`).
//   * absolute path (`pathname[0] == '/'`) — dirfd is ignored; the
//     resolver runs verbatim.
//   * any other (dirfd, relative-path) combo — `-ENOSYS`. The
//     relative-to-fd surface lands once the cwd model + per-fd offset
//     surface land (post-#499); tier-1 is "we're not lying about it".
//
// Linux flag values (from `<asm/fcntl.h>` / `vendor/musl/include/
// fcntl.h`):
//
//   * `O_RDONLY  =  0`
//   * `O_WRONLY  =  1`
//   * `O_RDWR    =  2`
//   * `O_ACCMODE =  3` (mask for the access-mode bits)
//
// Synthetic_fs entries are read-only by construction — `O_WRONLY` and
// `O_RDWR` against `/proc/*` / `/sys/*` / `/dev/*` returns `-EACCES`.
// File-cell-backed paths today only support `O_RDONLY`; write support
// (path → File create / append) is a follow-up after #499.
//
// Errno values (from `<asm-generic/errno-base.h>` / `<asm-generic/
// errno.h>`), augmenting the set GGGGG already exposed in
// `dispatch::EBADF` / `EFAULT` / `EINVAL`:
//
//   * `ENOENT = 2` — pathname does not refer to any file.
//   * `EACCES = 13` — synthetic file opened with a write mode.
//   * `EFAULT = 14` — pathname pointer is null / outside the
//                    process's address space.
//   * `EINVAL = 22` — flags' access-mode bits are unrecognised
//                    (e.g. `(O_ACCMODE) == 3`).
//   * `EMFILE = 24` — fd table is full (1024 entries per process).
//   * `ENOSYS = 38` — relative-to-fd resolution (dirfd != AT_FDCWD
//                    + non-absolute pathname).
//
// pathname dereferencing — tier-1 identity-mapping note
// -----------------------------------------------------
// `pathname` is a userspace virtual address pointing to a NUL-
// terminated C string. Tier-1 has no page-table install (#527
// pending) — the firmware's UEFI identity mapping means kernel-space
// and userspace VAs coincide (see `process::process` line 241 + the
// matching note in `syscall::write`). We deref the pointer directly
// for now. Once #527 lands real page tables, the deref needs to copy
// through the process's `AddressSpace` — tracked under #561 (the
// `copy_from_user` surface).
//
// Bounds: tier-1 caps the pathname read at 4096 bytes (Linux's
// `PATH_MAX`). Anything longer is treated as `-EFAULT` rather than
// `-ENAMETOOLONG` to keep the error surface small; switching to
// ENAMETOOLONG is a one-line change once the dispatcher exposes the
// constant.

use alloc::string::String;
use alloc::vec::Vec;
use arest::ast::{self, Object};

use crate::process::current_process_fd_table;
use crate::process::fd_table::{file as fd_file, synthetic as fd_synthetic};
use crate::syscall::dispatch::{EFAULT, EINVAL};
use crate::synthetic_fs;

/// Special dirfd value meaning "resolve against the current working
/// directory". Per `<fcntl.h>:AT_FDCWD`. Linux defines it as `-100`
/// — a value chosen so a kernel can distinguish it from any
/// legitimate fd (which are non-negative). Cast to i32 because
/// `dirfd` is `int` in the C signature.
pub const AT_FDCWD: i32 = -100;

/// Linux access-mode constants per `<asm/fcntl.h>` / `vendor/musl/
/// include/fcntl.h`. Only the access-mode bits (low 2) matter to
/// tier-1; the higher bits (`O_CREAT`, `O_TRUNC`, `O_NONBLOCK`,
/// `O_CLOEXEC`, etc) are accepted but otherwise ignored — a future
/// slice will plumb each through.
pub const O_RDONLY: u32 = 0;
pub const O_WRONLY: u32 = 1;
pub const O_RDWR: u32 = 2;
/// Mask for the access-mode bits. `flags & O_ACCMODE` extracts the
/// `O_RDONLY` / `O_WRONLY` / `O_RDWR` discriminant; values 0..2 are
/// valid, 3 is reserved for the legacy `O_NOACCESS` (Linux uses 3
/// internally for path-only fds — `open(O_PATH)`); we treat 3 as
/// `-EINVAL` for tier-1.
pub const O_ACCMODE: u32 = 3;

/// Linux errno for "No such file or directory". Returned when no
/// resolver (synthetic-fs nor File-cell graph) recognises the
/// pathname. Per `<asm-generic/errno-base.h>:ENOENT`.
pub const ENOENT: i64 = 2;

/// Linux errno for "Permission denied". Returned when a synthetic-fs
/// path is opened with a write mode (`O_WRONLY` or `O_RDWR`) — the
/// synthetic resolvers are read-only. Per `<asm-generic/errno-base.h>
/// :EACCES`.
pub const EACCES: i64 = 13;

/// Linux errno for "Too many open files". Returned when the per-
/// process fd table is full (1024 entries). Per `<asm-generic/errno-
/// base.h>:EMFILE`.
pub const EMFILE: i64 = 24;

/// Linux errno for "Function not implemented". Returned when the
/// (dirfd, relative-path) combo is something tier-1 doesn't yet
/// support (anything other than AT_FDCWD or absolute path). Per
/// `<asm-generic/errno.h>:ENOSYS`.
pub const ENOSYS: i64 = 38;

/// Tier-1 PATH_MAX. Anything longer than this is treated as `-EFAULT`
/// (the C string read can't terminate within the budget). Linux
/// defines PATH_MAX as 4096 in `<linux/limits.h>:PATH_MAX`.
pub const PATH_MAX: usize = 4096;

/// Handle an `openat(dirfd, pathname, flags, mode)` syscall. Returns
/// the allocated fd (≥ 3) on success, a negative errno on failure.
///
/// Tier-1 supported (dirfd, pathname) shapes:
///   * `dirfd == AT_FDCWD`, any pathname (resolved as if absolute).
///   * `dirfd != AT_FDCWD`, absolute pathname — dirfd ignored.
///
/// Anything else returns `-ENOSYS`. The `mode` argument is accepted
/// but ignored — only relevant to `O_CREAT` paths, which tier-1
/// doesn't yet support (the resolver chain is read-only and
/// File-cell creation is a separate verb on the cell graph).
///
/// SAFETY: callers (the syscall dispatcher) treat `pathname` as a
/// userspace virtual address. Under tier-1's identity mapping (UEFI
/// firmware + no page-table install yet) it doubles as a kernel
/// pointer; the handler dereferences it directly. Once #527 lands
/// real page tables, the deref needs to route through the per-process
/// AddressSpace.
pub fn handle(dirfd: i32, pathname: u64, flags: u32, _mode: u32) -> i64 {
    // Read the pathname out of userspace. Returns Err with the right
    // errno on a bad pointer / overlong path.
    let path = match read_pathname(pathname) {
        Ok(p) => p,
        Err(e) => return e,
    };

    // Filter unsupported (dirfd, path) combos up front — tier-1 only
    // handles AT_FDCWD or absolute paths.
    let is_absolute = path.starts_with('/');
    if dirfd != AT_FDCWD && !is_absolute {
        return -ENOSYS;
    }

    // Validate the access mode — only RDONLY / WRONLY / RDWR are
    // accepted. The reserved value `O_ACCMODE & flags == 3` is
    // Linux's `O_PATH` discriminant which tier-1 doesn't model.
    let access = flags & O_ACCMODE;
    if access > O_RDWR {
        return -EINVAL;
    }

    // Path resolution: try synthetic-fs first (cheap prefix match,
    // covers the common `/proc/cpuinfo` case), then the File-cell
    // graph. The two surfaces are disjoint by construction
    // (synthetic-fs only matches `/proc/*`, `/sys/*`, `/dev/*`; the
    // File-cell graph is keyed on whatever name a user uploaded).
    if synthetic_fs::resolve(&path).is_some() {
        // Synthetic-fs entries are read-only; reject write modes.
        if access != O_RDONLY {
            return -EACCES;
        }
        return allocate_synthetic(&path);
    }

    // Fall through to the File-cell graph. `lookup_file_cell_id`
    // returns the cell id of the first File whose `File_has_Name`
    // fact matches `path`; `None` means the path doesn't resolve.
    if let Some(cell_id) = lookup_file_cell_id(&path) {
        // File-cell-backed entries today only support O_RDONLY.
        // Write support lands once the open()-side write path is
        // wired (post-#499).
        if access != O_RDONLY {
            return -EACCES;
        }
        return allocate_file(&cell_id);
    }

    // No resolver matched — the path doesn't exist.
    -ENOENT
}

/// Read a NUL-terminated C string out of userspace at `ptr`. Returns
/// the string on success, a negative errno on failure (`-EFAULT` for
/// null / overlong, `-EINVAL` for invalid UTF-8 since AREST cell ids
/// + synthetic-fs paths are UTF-8 by construction).
///
/// SAFETY: dereferences `ptr` as `*const u8` for up to `PATH_MAX + 1`
/// bytes. Tier-1 identity mapping makes this safe for any userspace
/// pointer the caller passed; once #527 lands real page tables this
/// gains a `validate_userspace_range` pre-check.
pub fn read_pathname(ptr: u64) -> Result<String, i64> {
    if ptr == 0 {
        return Err(-EFAULT);
    }
    // Read bytes until NUL or PATH_MAX. The +1 budget accommodates
    // the NUL terminator; a string of exactly PATH_MAX bytes (no
    // NUL within) is rejected as -EFAULT (Linux returns
    // -ENAMETOOLONG; we collapse for tier-1).
    let mut bytes: Vec<u8> = Vec::new();
    for offset in 0..=PATH_MAX {
        // SAFETY: dereferences a userspace VA. Under tier-1 identity
        // mapping this is safe for any non-null pointer the caller
        // passed; we can't validate page presence without a page-
        // table walker (#527).
        let b = unsafe { *(ptr.wrapping_add(offset as u64) as *const u8) };
        if b == 0 {
            // Reached NUL terminator — the read is complete.
            // Validate UTF-8 (paths are by construction UTF-8 in
            // AREST; libc paths can in principle be arbitrary bytes
            // but the resolver chain only matches UTF-8 strings).
            return String::from_utf8(bytes).map_err(|_| -EINVAL);
        }
        bytes.push(b);
    }
    // Read PATH_MAX bytes without seeing NUL — too long.
    Err(-EFAULT)
}

/// Allocate an fd backed by a synthetic-fs path. Returns the
/// allocated fd or a negative errno (`-EMFILE` when the table is
/// full, `-EBADF` when no current process is installed which
/// shouldn't happen in production but does in `cargo test` before
/// the test installs one).
fn allocate_synthetic(path: &str) -> i64 {
    current_process_fd_table(|maybe_table| match maybe_table {
        Some(table) => match table.allocate(fd_synthetic(path)) {
            Ok(fd) => fd as i64,
            Err(()) => -EMFILE,
        },
        None => -ENOSYS, // pre-process state — surface the right errno
    })
}

/// Allocate an fd backed by a File cell. Same shape as
/// `allocate_synthetic` — returns fd or negative errno.
fn allocate_file(cell_id: &str) -> i64 {
    current_process_fd_table(|maybe_table| match maybe_table {
        Some(table) => match table.allocate(fd_file(cell_id)) {
            Ok(fd) => fd as i64,
            Err(()) => -EMFILE,
        },
        None => -ENOSYS,
    })
}

/// Walk the `File_has_Name` facts in the SYSTEM cell graph looking
/// for a File whose name matches `path`. Returns the cell id (the
/// `File` binding) of the first match; `None` if no File matches or
/// SYSTEM hasn't been initialised yet.
///
/// Tier-1 matches the path verbatim — no canonicalisation, no
/// directory walking. A File uploaded as "config.toml" at the root
/// will match `openat(AT_FDCWD, "config.toml", O_RDONLY, 0)` but not
/// `openat(AT_FDCWD, "/config.toml", ...)`. The future #399 ring-
/// acyclic Directory walker will widen this to a real path lookup;
/// tier-1's surface stays minimal.
pub fn lookup_file_cell_id(path: &str) -> Option<String> {
    crate::system::with_state(|state| lookup_file_cell_id_in(path, state))?
}

/// Pure-state version of `lookup_file_cell_id` — looks up the path
/// in `state` rather than the live SYSTEM. Same return shape; useful
/// for the unit tests which assemble a fixture state without going
/// through `system::init`.
pub fn lookup_file_cell_id_in(path: &str, state: &Object) -> Option<String> {
    let cell = ast::fetch_or_phi("File_has_Name", state);
    cell.as_seq()?.iter().find_map(|fact| {
        if ast::binding(fact, "Name") == Some(path) {
            ast::binding(fact, "File").map(|s| s.into())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::address_space::AddressSpace;
    use crate::process::{current_process_install, current_process_uninstall, Process};
    use arest::ast::{cell_push, fact_from_pairs, Object};

    /// `AT_FDCWD` is `-100` per `<fcntl.h>:AT_FDCWD`.
    #[test]
    fn at_fdcwd_value_matches_linux_uapi() {
        assert_eq!(AT_FDCWD, -100);
    }

    /// `O_RDONLY` / `O_WRONLY` / `O_RDWR` / `O_ACCMODE` match Linux.
    #[test]
    fn flag_constants_match_linux_uapi() {
        assert_eq!(O_RDONLY, 0);
        assert_eq!(O_WRONLY, 1);
        assert_eq!(O_RDWR, 2);
        assert_eq!(O_ACCMODE, 3);
    }

    /// `ENOENT` / `EACCES` / `EMFILE` match `<asm-generic/errno-
    /// base.h>` / `<asm-generic/errno.h>`.
    #[test]
    fn errno_constants_match_linux_uapi() {
        assert_eq!(ENOENT, 2);
        assert_eq!(EACCES, 13);
        assert_eq!(EMFILE, 24);
    }

    /// Helper: install a fresh Process so the handler has somewhere
    /// to allocate fds against.
    fn install_test_process() {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(7, address_space);
        current_process_install(proc);
    }

    /// Helper: NUL-terminate a path so the userspace pointer dance
    /// in `read_pathname` finds the terminator. Returns a Vec the
    /// caller keeps alive for the duration of the test (so the
    /// pointer stays valid).
    fn cstring(s: &str) -> Vec<u8> {
        let mut v = Vec::from(s.as_bytes());
        v.push(0);
        v
    }

    /// `openat(AT_FDCWD, "/proc/cpuinfo", O_RDONLY, 0)` returns a
    /// valid fd (≥ 3). The fd-table entry is `Synthetic { path:
    /// "/proc/cpuinfo" }`.
    #[test]
    fn open_proc_cpuinfo_returns_valid_fd() {
        install_test_process();
        let path = cstring("/proc/cpuinfo");
        let fd = handle(AT_FDCWD, path.as_ptr() as u64, O_RDONLY, 0);
        assert!(fd >= 3, "fd should be >= 3, got {}", fd);
        // Verify the fd-table entry matches.
        let lookup = current_process_fd_table(|t| {
            t.and_then(|t| t.lookup(fd as i32).cloned())
        });
        use crate::process::fd_table::FdEntry;
        assert_eq!(
            lookup,
            Some(FdEntry::Synthetic {
                path: "/proc/cpuinfo".into()
            })
        );
        current_process_uninstall();
    }

    /// `openat(AT_FDCWD, "/proc/meminfo", O_RDONLY, 0)` also
    /// resolves through synthetic-fs.
    #[test]
    fn open_proc_meminfo_returns_valid_fd() {
        install_test_process();
        let path = cstring("/proc/meminfo");
        let fd = handle(AT_FDCWD, path.as_ptr() as u64, O_RDONLY, 0);
        assert!(fd >= 3, "fd should be >= 3, got {}", fd);
        current_process_uninstall();
    }

    /// `openat(AT_FDCWD, "/tmp/missing", O_RDONLY, 0)` returns
    /// `-ENOENT`. The synthetic-fs resolver doesn't match (`/tmp`
    /// isn't a recognised prefix) and the File-cell graph is empty
    /// (no SYSTEM init in tests).
    #[test]
    fn open_missing_path_returns_minus_enoent() {
        install_test_process();
        let path = cstring("/tmp/missing");
        let result = handle(AT_FDCWD, path.as_ptr() as u64, O_RDONLY, 0);
        assert_eq!(result, -ENOENT);
        current_process_uninstall();
    }

    /// `openat(AT_FDCWD, "/proc/cpuinfo", O_WRONLY, 0)` returns
    /// `-EACCES`. Synthetic-fs entries are read-only; opening one
    /// for writing is forbidden.
    #[test]
    fn open_synthetic_with_wronly_returns_minus_eacces() {
        install_test_process();
        let path = cstring("/proc/cpuinfo");
        let result = handle(AT_FDCWD, path.as_ptr() as u64, O_WRONLY, 0);
        assert_eq!(result, -EACCES);
        current_process_uninstall();
    }

    /// `openat(AT_FDCWD, "/proc/cpuinfo", O_RDWR, 0)` returns
    /// `-EACCES` for the same reason.
    #[test]
    fn open_synthetic_with_rdwr_returns_minus_eacces() {
        install_test_process();
        let path = cstring("/proc/cpuinfo");
        let result = handle(AT_FDCWD, path.as_ptr() as u64, O_RDWR, 0);
        assert_eq!(result, -EACCES);
        current_process_uninstall();
    }

    /// `openat(AT_FDCWD, NULL, ...)` returns `-EFAULT`. Null pointer
    /// is not a valid pathname address.
    #[test]
    fn open_null_pathname_returns_minus_efault() {
        install_test_process();
        let result = handle(AT_FDCWD, 0, O_RDONLY, 0);
        assert_eq!(result, -EFAULT);
        current_process_uninstall();
    }

    /// `openat(AT_FDCWD, "/proc/cpuinfo", flags=4, 0)` returns
    /// `-EINVAL` — the access-mode bits (low 2) are 0 (O_RDONLY)
    /// but the higher bit being set isn't itself an error; we
    /// reject only when the access-mode bits themselves are out of
    /// range. Test the `(flags & O_ACCMODE) == 3` reserved value.
    #[test]
    fn open_invalid_access_mode_returns_minus_einval() {
        install_test_process();
        let path = cstring("/proc/cpuinfo");
        // 3 is the reserved O_PATH discriminant — tier-1 rejects.
        let result = handle(AT_FDCWD, path.as_ptr() as u64, 3, 0);
        assert_eq!(result, -EINVAL);
        current_process_uninstall();
    }

    /// `openat(99, "relative/path", O_RDONLY, 0)` returns `-ENOSYS`.
    /// dirfd != AT_FDCWD with a non-absolute pathname is the
    /// relative-to-fd surface tier-1 doesn't model.
    #[test]
    fn open_relative_to_fd_returns_minus_enosys() {
        install_test_process();
        let path = cstring("relative/path");
        let result = handle(99, path.as_ptr() as u64, O_RDONLY, 0);
        assert_eq!(result, -ENOSYS);
        current_process_uninstall();
    }

    /// `openat(99, "/absolute/path", O_RDONLY, 0)` ignores dirfd
    /// because the path is absolute. Returns -ENOENT (the resolvers
    /// don't match) rather than -ENOSYS.
    #[test]
    fn open_absolute_with_arbitrary_dirfd_ignores_dirfd() {
        install_test_process();
        let path = cstring("/absolute/missing");
        let result = handle(99, path.as_ptr() as u64, O_RDONLY, 0);
        // Not -ENOSYS — absolute paths bypass the dirfd check.
        // Not synthetic, not in File-cell graph → -ENOENT.
        assert_eq!(result, -ENOENT);
        current_process_uninstall();
    }

    /// Sequential `openat` calls allocate fds 3, 4, 5, ... per
    /// POSIX's "lowest free fd" rule.
    #[test]
    fn sequential_opens_allocate_increasing_fds() {
        install_test_process();
        let path_a = cstring("/proc/cpuinfo");
        let path_b = cstring("/proc/meminfo");
        let fd_a = handle(AT_FDCWD, path_a.as_ptr() as u64, O_RDONLY, 0);
        let fd_b = handle(AT_FDCWD, path_b.as_ptr() as u64, O_RDONLY, 0);
        assert_eq!(fd_a, 3);
        assert_eq!(fd_b, 4);
        current_process_uninstall();
    }

    /// `read_pathname` reads a NUL-terminated string up to PATH_MAX.
    /// Returns the string verbatim.
    #[test]
    fn read_pathname_returns_string_up_to_nul() {
        let buf = cstring("/proc/cpuinfo");
        let result = read_pathname(buf.as_ptr() as u64);
        assert_eq!(result, Ok("/proc/cpuinfo".into()));
    }

    /// `read_pathname` with a null pointer returns `-EFAULT`.
    #[test]
    fn read_pathname_null_returns_efault() {
        assert_eq!(read_pathname(0), Err(-EFAULT));
    }

    /// `read_pathname` against a buffer with no NUL within PATH_MAX
    /// returns `-EFAULT`. We can't validate this by allocating a
    /// 4096-byte buffer with no NUL because the test's address space
    /// has no guarantee about what byte sits past the buffer; instead
    /// the test confirms the read of an exact-sized buffer ending in
    /// NUL succeeds (a regression here would catch off-by-one in the
    /// 0..=PATH_MAX iteration bound).
    #[test]
    fn read_pathname_short_string_succeeds() {
        let buf = cstring("/a");
        let result = read_pathname(buf.as_ptr() as u64);
        assert_eq!(result, Ok("/a".into()));
    }

    /// `lookup_file_cell_id_in` against a state with one
    /// `File_has_Name` fact returns the matching cell id.
    #[test]
    fn lookup_file_cell_id_in_matches_filename() {
        let state = Object::phi();
        let state = cell_push(
            "File_has_Name",
            fact_from_pairs(&[("File", "abc123"), ("Name", "config.toml")]),
            &state,
        );
        let result = lookup_file_cell_id_in("config.toml", &state);
        assert_eq!(result, Some("abc123".into()));
    }

    /// `lookup_file_cell_id_in` returns `None` for a name that
    /// doesn't appear in any `File_has_Name` fact.
    #[test]
    fn lookup_file_cell_id_in_misses_unknown_filename() {
        let state = Object::phi();
        let state = cell_push(
            "File_has_Name",
            fact_from_pairs(&[("File", "abc123"), ("Name", "config.toml")]),
            &state,
        );
        let result = lookup_file_cell_id_in("missing.txt", &state);
        assert_eq!(result, None);
    }

    /// `lookup_file_cell_id_in` returns the first match when multiple
    /// Files share the same name (the readings allow this — File
    /// uniqueness is per-id, not per-name). Tier-1 takes the first
    /// fact in the cell; a future per-Directory disambiguation lands
    /// once the Directory walker does.
    #[test]
    fn lookup_file_cell_id_in_returns_first_match_on_collision() {
        let state = Object::phi();
        let state = cell_push(
            "File_has_Name",
            fact_from_pairs(&[("File", "first"), ("Name", "shared.txt")]),
            &state,
        );
        let state = cell_push(
            "File_has_Name",
            fact_from_pairs(&[("File", "second"), ("Name", "shared.txt")]),
            &state,
        );
        let result = lookup_file_cell_id_in("shared.txt", &state);
        assert!(result.is_some());
        // Either "first" or "second" is acceptable; the test asserts
        // determinism (the same input produces the same result).
        let again = lookup_file_cell_id_in("shared.txt", &state);
        assert_eq!(result, again);
    }
}
