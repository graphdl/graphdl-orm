// crates/arest-kernel/src/syscall/writev.rs
//
// Linux x86_64 syscall 20: `writev(fd, iov, iovcnt)` — scatter-gather
// write. Per #476e (interactive ash bring-up).
//
// Linux x86_64 number: `__NR_writev = 20`
// (`linux/arch/x86/include/uapi/asm/unistd_64.h`); confirmed by the
// vendored musl tree at `vendor/musl/arch/x86_64/bits/syscall.h.in`.
//
// Why tier-1 needs this at all
// ----------------------------
// musl's stdio flush path (`__stdio_write`) emits writev, NOT write:
// every printf / fputs / fflush through a `FILE*` lands here, gathering
// the FILE's internal buffer and the caller's tail in one call.
// Observed in the #476e ash smoke: command OUTPUT rendered fine
// (busybox's `full_write` → plain `write`) while ash's PROMPT vanished
// into `-ENOSYS` — the shell worked but looked dead on serial.
//
// Semantics
// ---------
// Each `struct iovec { void *iov_base; size_t iov_len; }` (16 bytes on
// x86_64) is delegated IN ORDER to `write::handle(fd, base, len)` — so
// writev inherits write's whole per-fd story (fd 1/2 → console sink,
// `/dev/*` device fds, EBADF/EFAULT shapes) without a second
// implementation that could drift. Gather atomicity (one uninterrupted
// stream) holds trivially: tier-1 is single-core with exactly one
// process, so nothing interleaves between segment writes.
//
// Edge cases per writev(2):
//   * `iovcnt == 0`           → 0 (no-op success).
//   * `iovcnt > IOV_MAX`      → -EINVAL.
//   * `iov == NULL` (cnt > 0) → -EFAULT.
//   * zero-length entries     → skipped (contribute 0 bytes).
//   * total length overflowing ssize_t → -EINVAL (Linux checks the
//     sum *before* transferring).
//   * a segment erroring after earlier segments delivered → return the
//     byte count so far (POSIX partial-write rule); erroring on the
//     FIRST segment → propagate that errno.
//
// Tier-1 identity-mapped pointer model: the iovec array is read with
// `read_unaligned` u64 loads at `iov + i*16` / `+8` — same
// treat-user-pointer-as-kernel-pointer pattern as `read` /
// `getrandom`, with the same forward-compat note (#561 copy_from_user
// will add a validate step). Unaligned loads because nothing forces a
// guest to 8-align its iovec array, and `core::ptr::read` on an
// unaligned pointer is UB even where x86 tolerates it.

use crate::syscall::dispatch::{EFAULT, EINVAL};
use crate::syscall::write;

/// Linux x86_64 syscall number for `writev(fd, iov, iovcnt)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_writev` (= 20).
pub const SYS_WRITEV: u64 = 20;

/// Linux's `UIO_MAXIOV` — the maximum iovec count a single writev
/// accepts (`linux/include/uapi/linux/uio.h`). Above this → -EINVAL.
pub const IOV_MAX: u64 = 1024;

/// `sizeof(struct iovec)` on x86_64: `{ void *iov_base; size_t
/// iov_len; }` — two 8-byte fields.
const IOVEC_SIZE: u64 = 16;

/// Handle a `writev(fd, iov, iovcnt)` syscall. Delegates each iovec
/// entry to `write::handle` in order and returns the total byte count
/// (see the module docs for the full edge-case table).
///
/// SAFETY: `iov` is treated as a kernel pointer under tier-1's
/// identity mapping (same as every user-pointer syscall); entries are
/// read with unaligned loads. The per-segment `base` pointers are
/// validated by `write::handle` (null → -EFAULT).
pub fn handle(fd: u64, iov: u64, iovcnt: u64) -> i64 {
    // writev(2): zero iovecs transfer zero bytes — success, no
    // pointer deref (a probe `writev(fd, NULL, 0)` succeeds).
    if iovcnt == 0 {
        return 0;
    }
    if iovcnt > IOV_MAX {
        return -EINVAL;
    }
    if iov == 0 {
        return -EFAULT;
    }

    let mut total: i64 = 0;
    for i in 0..iovcnt {
        // SAFETY: identity-mapped iovec array; unaligned u64 loads of
        // the two fields (module docs).
        let base = unsafe { core::ptr::read_unaligned((iov + i * IOVEC_SIZE) as *const u64) };
        let len = unsafe { core::ptr::read_unaligned((iov + i * IOVEC_SIZE + 8) as *const u64) };
        if len == 0 {
            continue;
        }

        // Linux checks the TOTAL against ssize_t before transferring:
        // a sum that can't be represented is -EINVAL, not a partial
        // count.
        let Some(would) = (total as u64).checked_add(len) else {
            return -EINVAL;
        };
        if would > i64::MAX as u64 {
            return -EINVAL;
        }

        let n = write::handle(fd, base, len);
        if n < 0 {
            // Partial-progress rule: an error after delivered bytes
            // reports the bytes; an error on the first segment is the
            // caller's errno.
            return if total == 0 { n } else { total };
        }
        total += n;
        if (n as u64) < len {
            // Short segment write — report what landed (write's
            // console sink never short-writes, but device fds may).
            break;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syscall::dispatch::EBADF;

    /// Build an iovec array over the given (ptr, len) pairs, returning
    /// the backing storage (kept alive by the caller holding the Vec).
    fn iovec_array(entries: &[(u64, u64)]) -> alloc::vec::Vec<u64> {
        let mut raw = alloc::vec::Vec::with_capacity(entries.len() * 2);
        for &(base, len) in entries {
            raw.push(base);
            raw.push(len);
        }
        raw
    }

    /// SYS_WRITEV is 20 per Linux x86_64 unistd_64.h — drifting breaks
    /// every musl stdio flush.
    #[test]
    fn sys_writev_constant_is_20() {
        assert_eq!(SYS_WRITEV, 20);
    }

    /// IOV_MAX matches Linux's UIO_MAXIOV.
    #[test]
    fn iov_max_matches_linux_uio_maxiov() {
        assert_eq!(IOV_MAX, 1024);
    }

    /// `writev(fd, NULL, 0)` — zero iovecs — succeeds with 0 bytes and
    /// never dereferences the pointer.
    #[test]
    fn writev_zero_iovcnt_returns_zero_without_deref() {
        assert_eq!(handle(1, 0, 0), 0);
    }

    /// `writev(fd, NULL, n>0)` → -EFAULT.
    #[test]
    fn writev_null_iov_with_count_returns_efault() {
        assert_eq!(handle(1, 0, 2), -EFAULT);
    }

    /// `iovcnt > IOV_MAX` → -EINVAL (checked before any deref —
    /// the iov pointer here is non-null but bogus).
    #[test]
    fn writev_iovcnt_over_max_returns_einval() {
        assert_eq!(handle(1, 0x1000, IOV_MAX + 1), -EINVAL);
    }

    /// Two stdout segments gather in order: 5 + 7 bytes → 12. (The
    /// console sink prints to the host test harness's stdout — the
    /// return-count is the assertable surface, same as write's tests.)
    #[test]
    fn writev_gathers_two_stdout_segments() {
        let a = b"hello";
        let b = b" world\n";
        let raw = iovec_array(&[
            (a.as_ptr() as u64, a.len() as u64),
            (b.as_ptr() as u64, b.len() as u64),
        ]);
        let n = handle(1, raw.as_ptr() as u64, 2);
        assert_eq!(n, (a.len() + b.len()) as i64);
    }

    /// Zero-length entries are skipped, not errors: [5, 0, 3] → 8.
    #[test]
    fn writev_skips_zero_length_entries() {
        let a = b"prompt";
        let c = b"# ";
        let raw = iovec_array(&[
            (a.as_ptr() as u64, a.len() as u64),
            (0xdead_0000, 0), // base never deref'd at len 0
            (c.as_ptr() as u64, c.len() as u64),
        ]);
        let n = handle(1, raw.as_ptr() as u64, 3);
        assert_eq!(n, (a.len() + c.len()) as i64);
    }

    /// An unwritable fd errors on the FIRST segment with write's errno
    /// (here EBADF for an fd with no table entry / no process).
    #[test]
    fn writev_unknown_fd_propagates_first_segment_errno() {
        let _guard = crate::process::process::CURRENT_PROCESS_TEST_LOCK.lock();
        crate::process::current_process_uninstall();
        let a = b"x";
        let raw = iovec_array(&[(a.as_ptr() as u64, a.len() as u64)]);
        assert_eq!(handle(7, raw.as_ptr() as u64, 1), -EBADF);
    }

    /// A null SEGMENT base inside an otherwise-valid array propagates
    /// write's -EFAULT (first segment, no bytes delivered).
    #[test]
    fn writev_null_segment_base_returns_efault() {
        let raw = iovec_array(&[(0, 4)]);
        assert_eq!(handle(1, raw.as_ptr() as u64, 1), -EFAULT);
    }

    /// A length sum overflowing ssize_t → -EINVAL before transfer.
    #[test]
    fn writev_total_overflow_returns_einval() {
        let a = b"y";
        let raw = iovec_array(&[
            (a.as_ptr() as u64, 1),
            (a.as_ptr() as u64, i64::MAX as u64),
        ]);
        assert_eq!(handle(1, raw.as_ptr() as u64, 2), -EINVAL);
    }

    /// Dispatch wiring: `dispatch(SYS_WRITEV, 1, iov, 2, ...)` routes
    /// here and returns the gathered byte count.
    #[test]
    fn dispatch_sys_writev_routes_and_gathers() {
        use crate::syscall::dispatch::dispatch;
        let a = b"ab";
        let b = b"cde";
        let raw = iovec_array(&[
            (a.as_ptr() as u64, a.len() as u64),
            (b.as_ptr() as u64, b.len() as u64),
        ]);
        let n = dispatch(SYS_WRITEV, 1, raw.as_ptr() as u64, 2, 0, 0, 0);
        assert_eq!(n, 5);
    }
}
