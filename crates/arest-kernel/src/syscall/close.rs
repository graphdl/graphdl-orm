// crates/arest-kernel/src/syscall/close.rs
//
// Linux x86_64 syscall 3: `close(int fd)`. Releases the per-process
// fd-table slot held by `fd`, returning 0 on success and `-EBADF` on
// an unknown fd. Per #498 (Track KKKKK), the open-side counterpart of
// `openat` (#498 sibling). The fd-table itself is the richer
// `process::fd_table::FdTable` (file-cell-backed or synthetic-fs-
// backed entries) populated by `openat`; the legacy
// `process::Process::fd_table` (the seeded `Vec<FdEntry>` carrying
// stdin / stdout / stderr) is NOT touched by this handler — closing
// a standard stream returns `-EBADF` per the documented tier-1
// limitation in `process::fd_table` (the unification lands once
// `read` ships and the fd surface is consolidated).
//
// errno return
// ------------
// Linux's `close(2)` returns one of:
//
//   * `0`       — success.
//   * `-EBADF`  — `fd` is not a valid open fd in this process.
//   * `-EINTR`  — interrupted by a signal (we don't model signals).
//   * `-EIO`    — I/O error flushing buffered writes (we don't buffer).
//
// Tier-1 only ever returns `0` or `-EBADF` — the buffered-write +
// signal surfaces are out of scope for the foundation slice.
//
// What this handler does NOT do (intentionally):
//   * Refcount reduction (Linux's `struct file` carries a refcount
//     that `dup` / `dup2` increments). Tier-1 has no `dup` yet so
//     every fd is single-owner; release is unconditional.
//   * Cleanup of associated resources (close on a virtio-blk fd
//     would unmap the region, etc). The fd-table entry today holds
//     only a cell id or a path string — no kernel-side resource is
//     bound, so dropping the entry is the entire cleanup.
//   * Audit logging (the syscall trace surface lands with #530).

use crate::process::current_process_fd_table;
use crate::syscall::dispatch::EBADF;

/// Handle a `close(fd)` syscall. Returns 0 on success, `-EBADF` if
/// `fd` is not open in the current process's fd table.
///
/// The `fd` argument is the rdi register cast to `i32` per the Linux
/// convention — fds are signed because the negative-errno return
/// shares the same register width.
///
/// Edge cases:
///   * `fd < 3` (stdin / stdout / stderr) — returns `-EBADF`. The
///     standard streams live on the legacy `Vec<FdEntry>` and aren't
///     in this handler's table; closing them is unsupported under
///     tier-1.
///   * No current process installed — returns `-EBADF` (no fd table
///     to look in). Symmetric with `openat`'s ENOSYS path; the close
///     handler picks EBADF because the user meant to close an fd
///     that, by definition, doesn't exist on a not-yet-installed
///     process.
pub fn handle(fd: i32) -> i64 {
    current_process_fd_table(|maybe_table| match maybe_table {
        Some(table) => match table.release(fd) {
            Ok(()) => 0,
            Err(()) => -EBADF,
        },
        None => -EBADF,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::address_space::AddressSpace;
    use crate::process::fd_table::synthetic;
    use crate::process::{current_process_install, current_process_uninstall, Process};

    /// Helper: install a fresh Process so the handler has somewhere
    /// to release fds against. Mirrors the helper in the openat tests.
    fn install_test_process() {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(7, address_space);
        current_process_install(proc);
    }

    /// `close(fd)` on a freshly-allocated fd returns 0 and frees the
    /// slot.
    #[test]
    fn close_valid_fd_returns_zero() {
        install_test_process();
        // Allocate an fd directly through the fd table.
        let fd = current_process_fd_table(|t| {
            t.expect("process installed")
                .allocate(synthetic("/proc/cpuinfo"))
                .expect("allocate")
        });
        assert!(fd >= 3);
        let result = handle(fd);
        assert_eq!(result, 0);
        // Subsequent lookup should miss.
        let lookup = current_process_fd_table(|t| {
            t.and_then(|t| t.lookup(fd).cloned())
        });
        assert!(lookup.is_none());
        current_process_uninstall();
    }

    /// `close(fd)` on an unknown fd returns `-EBADF`.
    #[test]
    fn close_unknown_fd_returns_minus_ebadf() {
        install_test_process();
        let result = handle(99);
        assert_eq!(result, -EBADF);
        current_process_uninstall();
    }

    /// Closing the same fd twice returns `-EBADF` on the second call.
    #[test]
    fn double_close_returns_minus_ebadf() {
        install_test_process();
        let fd = current_process_fd_table(|t| {
            t.expect("process installed")
                .allocate(synthetic("/proc/cpuinfo"))
                .expect("allocate")
        });
        assert_eq!(handle(fd), 0);
        assert_eq!(handle(fd), -EBADF);
        current_process_uninstall();
    }

    /// `close(0)` / `close(1)` / `close(2)` return `-EBADF` —
    /// standard streams aren't in this handler's table per the
    /// tier-1 limitation.
    #[test]
    fn close_standard_streams_returns_minus_ebadf_under_tier_1() {
        install_test_process();
        assert_eq!(handle(0), -EBADF);
        assert_eq!(handle(1), -EBADF);
        assert_eq!(handle(2), -EBADF);
        current_process_uninstall();
    }

    /// `close` with no current process installed returns `-EBADF`.
    #[test]
    fn close_with_no_process_returns_minus_ebadf() {
        // Defensive: make sure no leftover from a prior test.
        current_process_uninstall();
        assert_eq!(handle(3), -EBADF);
    }

    /// `close(fd)` with a negative fd returns `-EBADF`. The fd table
    /// only stores non-negative keys, so any negative lookup misses.
    #[test]
    fn close_negative_fd_returns_minus_ebadf() {
        install_test_process();
        assert_eq!(handle(-1), -EBADF);
        assert_eq!(handle(-100), -EBADF);
        current_process_uninstall();
    }
}
