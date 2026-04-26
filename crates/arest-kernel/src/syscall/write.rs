// crates/arest-kernel/src/syscall/write.rs
//
// Linux x86_64 syscall 1: `write(int fd, const void *buf, size_t count)`.
// First write-side surface in #473's userspace syscall epic. Tier-1
// scope is intentionally narrow: only `fd == 1` (stdout) is recognised;
// every other fd returns `-EBADF`. Reads (#508), opens (#509), and
// arbitrary fd table mutations (#560) land in follow-up tracks.
//
// Stdout routing
// --------------
// Bytes written to fd 1 route through the kernel's serial console via
// `crate::print!`, which fans into `arch::_print(format_args!(...))` —
// `arch::uefi::serial::_print` on x86_64 UEFI (UART 16550 post-EBS,
// firmware ConOut pre-EBS), `arch::aarch64::serial::_print` on aarch64
// UEFI (PL011 MMIO), `arch::armv7::serial::_print` on armv7 UEFI
// (PL011 MMIO). The print macro accepts a `core::fmt::Arguments<'_>`
// produced by `format_args!`, so we wrap the byte slice in a
// `core::str::from_utf8`-best-effort view: well-formed UTF-8 prints
// verbatim; invalid sequences (which a real C program with a
// non-UTF-8 locale could emit) get replaced with `\u{FFFD}` via
// `core::str::from_utf8_unchecked`-style lossy conversion.
//
// Why not call `arch::uefi::console::print_str` directly
// ------------------------------------------------------
// The task spec mentions `arch::uefi::console::print_str` as the
// target surface, but that exact symbol doesn't exist in the kernel
// today — the print path is routed through the `crate::print!` macro
// which expands to `crate::arch::_print(format_args!(...))`. Calling
// the macro keeps the write handler arch-neutral: the same source
// compiles on all three UEFI arms (x86_64 / aarch64 / armv7) without
// any per-arm cfg branch. When #552 lands the SYSCALL MSR entry
// (x86_64-only), the arch-neutral handler is still correct; only
// the entry-side asm differs per arm.
//
// Why a sink trait
// ----------------
// The `do_write` function takes a `&mut dyn FnMut(&[u8])` so the unit
// tests can mock the console (record bytes into a `Vec<u8>` and assert
// on the result) without actually touching the kernel's serial port.
// Production routes through the `KernelConsoleSink` static (declared
// in `handle`) which calls `crate::print!`. Same shape as
// `crate::composer`'s `RustTestRenderer` — separating the side-effect
// from the data marshalling so the data-path is testable.
//
// Pointer dereferencing — tier-1 identity-mapping note
// ----------------------------------------------------
// The `buf` argument is a userspace virtual address. Tier-1 has no
// page-table install (#527 pending), and the firmware's UEFI
// identity mapping means kernel-space and userspace VAs coincide
// (see `process::process` line 241 for the same rationale used by
// the auxv `AT_RANDOM` setup). So we treat `buf` as a kernel
// pointer for now: `core::slice::from_raw_parts(buf as *const u8,
// count)` produces a slice we can hand to `from_utf8_lossy`. Once
// #527 lands real page tables, this will need to copy through the
// process's `AddressSpace` (validate the VA range is mapped, walk
// the segments, etc.) — tracked under #561 (the `copy_from_user`
// surface).
//
// Null pointer + zero count
// -------------------------
// Per Linux: `write(fd, NULL, 0)` is a no-op return-0; `write(fd,
// NULL, n>0)` returns `-EFAULT`. We mirror that: if `count == 0`
// the function returns 0 immediately (no pointer deref); otherwise
// a null `buf` returns `-EFAULT`.

use core::slice;
use core::str;

use crate::syscall::dispatch::{EBADF, EFAULT};

/// File-descriptor number for stdout per POSIX
/// (`<unistd.h>:STDOUT_FILENO`). Linux libc defines it as 1; the
/// constant is here so the handler reads as code rather than as a
/// magic number.
pub const STDOUT_FD: u64 = 1;

/// Handle a `write(fd, buf, count)` syscall. Returns the number of
/// bytes written on success (always == `count` for fd 1, which never
/// short-writes), or a negative `errno` on failure.
///
/// Tier-1 supported fds:
///   * `1` (stdout) → routes to the kernel serial console.
///
/// Every other fd returns `-EBADF`.
///
/// Edge cases:
///   * `count == 0` → returns 0 immediately, regardless of `buf`.
///   * `buf == 0 && count > 0` → returns `-EFAULT`.
///
/// SAFETY: callers (the syscall dispatcher) treat `buf` as a userspace
/// virtual address. Under the tier-1 identity mapping (UEFI firmware
/// + no page-table install yet) it doubles as a kernel pointer; the
/// handler dereferences it directly. Once #527 lands real page tables,
/// the deref needs to route through the per-process AddressSpace.
pub fn handle(fd: u64, buf: u64, count: u64) -> i64 {
    // Reject any fd other than stdout. Tier-1 only opens fd 0/1/2 in
    // the Process constructor (`Serial` for all three); the handler
    // currently routes only fd 1. fd 0 is read-only (handled by #508);
    // fd 2 (stderr) is symmetric to fd 1 but deferred so the demo
    // surface stays minimal — adding it is a one-line `STDOUT_FD |
    // STDERR_FD` change in the next slice.
    if fd != STDOUT_FD {
        return -EBADF;
    }
    // Fast-path: zero-length write is a no-op per POSIX. Doing this
    // check before the buf-null check lets the test suite exercise
    // `write(1, NULL, 0)` without panicking on a null deref.
    if count == 0 {
        return 0;
    }
    if buf == 0 {
        return -EFAULT;
    }
    do_write(buf, count, &mut |bytes| {
        // Lossy UTF-8 conversion — a non-UTF-8 byte sequence (which a
        // C program could emit via a printf("\xff\xff")) prints as
        // U+FFFD replacement chars rather than dropping the byte.
        // crate::print! accepts a `core::fmt::Arguments` so we wrap
        // the lossy `str` in `format_args!`; the underlying serial
        // path handles the (transcoded) UCS-2 / UART byte stream.
        match str::from_utf8(bytes) {
            Ok(s) => crate::print!("{}", s),
            Err(_) => {
                // Invalid UTF-8 sequence — print byte-by-byte, replacing
                // out-of-range bytes with U+FFFD. Avoids pulling in
                // `alloc::string::String::from_utf8_lossy` which would
                // be a heap allocation per write and a synchronisation
                // hazard inside the print path.
                for &b in bytes {
                    if b < 0x80 {
                        crate::print!("{}", b as char);
                    } else {
                        crate::print!("\u{FFFD}");
                    }
                }
            }
        }
    })
}

/// Shared work for the write handler — separated from `handle` so the
/// unit tests can inject a mock sink without touching the kernel's
/// serial port. The dispatcher (production path) feeds this through
/// `handle` with the kernel `print!` sink; tests feed it a `Vec<u8>`
/// recorder.
///
/// `sink` is a closure rather than a trait object so callers can
/// capture per-call state (e.g., a `&mut Vec<u8>` for the test's
/// recorder) without a heap allocation. Same shape `core::fmt::write`
/// + `core::fmt::Write` use under the hood.
///
/// Returns the count written on success — always equals `count` for
/// the success path; partial writes don't happen on serial since the
/// underlying port driver buffers internally and the print macro
/// flushes per-call. Returns `-EFAULT` if the slice can't be formed
/// (only happens if the count is large enough to overflow `isize`,
/// which would be a malicious caller).
///
/// SAFETY: `buf` is dereferenced as a `*const u8` for `count` bytes.
/// Caller is responsible for the validity of the range — the handler's
/// `if buf == 0` check guards null, but a non-null but unmapped pointer
/// would page-fault here. Under tier-1's identity mapping (no real
/// page tables yet) the only way to hit this is a deliberately bogus
/// pointer, which the dispatcher's `from userspace` invariant
/// precludes; once #527 lands real page tables this function gains a
/// `validate_userspace_range` pre-check.
pub fn do_write(buf: u64, count: u64, sink: &mut dyn FnMut(&[u8])) -> i64 {
    // `from_raw_parts` requires `count <= isize::MAX`. A larger count
    // is a malformed call — return `-EFAULT` so libc surfaces it as a
    // generic bad-address rather than panicking inside the slice
    // constructor.
    if count > isize::MAX as u64 {
        return -EFAULT;
    }
    let bytes: &[u8] = unsafe { slice::from_raw_parts(buf as *const u8, count as usize) };
    sink(bytes);
    count as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// `write(1, "hello", 5)` routes to the console sink and returns 5.
    /// The mock sink records the bytes; the test asserts the recorded
    /// payload matches the input.
    #[test]
    fn write_to_stdout_routes_to_sink_and_returns_count() {
        let payload = b"hello";
        let mut recorded: Vec<u8> = Vec::new();
        let result = do_write(payload.as_ptr() as u64, payload.len() as u64, &mut |bytes| {
            recorded.extend_from_slice(bytes);
        });
        assert_eq!(result, payload.len() as i64);
        assert_eq!(recorded.as_slice(), payload);
    }

    /// `write(5, ..., 10)` — fd 5 isn't open in tier-1; handler
    /// returns `-EBADF`. The buf / count are ignored on this path
    /// because the fd check fires first (matches Linux's behaviour:
    /// invalid fd is checked before pointer validity).
    #[test]
    fn write_to_unsupported_fd_returns_ebadf() {
        // Use a non-null but arbitrary buf; the handler should never
        // dereference it because the fd check trips first.
        let payload = b"unused";
        let result = handle(5, payload.as_ptr() as u64, payload.len() as u64);
        assert_eq!(result, -EBADF);
    }

    /// `write(0, ..., 10)` — stdin isn't write-side; same `-EBADF`.
    /// Future read handler (#508) will accept fd 0; the write side
    /// stays rejected.
    #[test]
    fn write_to_stdin_returns_ebadf() {
        let payload = b"unused";
        let result = handle(0, payload.as_ptr() as u64, payload.len() as u64);
        assert_eq!(result, -EBADF);
    }

    /// `write(2, ..., 10)` — stderr is currently unsupported per the
    /// tier-1 minimal surface (only stdout). Returns `-EBADF`. Adding
    /// stderr is a one-line change in a follow-up.
    #[test]
    fn write_to_stderr_returns_ebadf_under_tier_1() {
        let payload = b"unused";
        let result = handle(2, payload.as_ptr() as u64, payload.len() as u64);
        assert_eq!(result, -EBADF);
    }

    /// `write(1, NULL, 0)` is a POSIX no-op — returns 0 without
    /// dereferencing. Tested before the EFAULT check because the
    /// count==0 short-circuit must precede the buf-null check.
    #[test]
    fn write_zero_count_returns_zero_even_with_null_buf() {
        let result = handle(STDOUT_FD, 0, 0);
        assert_eq!(result, 0);
    }

    /// `write(1, NULL, 10)` returns `-EFAULT`. Linux behaviour for a
    /// null buf with non-zero count.
    #[test]
    fn write_null_buf_with_count_returns_efault() {
        let result = handle(STDOUT_FD, 0, 10);
        assert_eq!(result, -EFAULT);
    }

    /// `do_write` short-circuits when count exceeds isize::MAX.
    /// Returns `-EFAULT` rather than constructing a malformed slice.
    /// (Production callers can never reach this — the dispatcher
    /// passes the raw rdx register, which a malicious userspace could
    /// in principle set to a huge value.)
    #[test]
    fn do_write_rejects_oversized_count() {
        let mut sink = |_bytes: &[u8]| panic!("sink should not be invoked");
        // Use a non-null buf so the count check is exercised, not
        // some upstream null guard (do_write doesn't check null —
        // that's handle's job).
        let buf = 0x1000_u64;
        let result = do_write(buf, (isize::MAX as u64) + 1, &mut sink);
        assert_eq!(result, -EFAULT);
    }

    /// Mock sink receives bytes verbatim — including non-UTF-8 ones.
    /// `do_write` passes the raw byte slice to the sink without any
    /// UTF-8 transcoding; only `handle` does that on its way to the
    /// console. Validates that the test path can exercise binary
    /// payloads cleanly.
    #[test]
    fn do_write_passes_binary_bytes_through() {
        let payload: [u8; 4] = [0xff, 0x00, 0xfe, 0x80];
        let mut recorded: Vec<u8> = Vec::new();
        let result = do_write(payload.as_ptr() as u64, payload.len() as u64, &mut |bytes| {
            recorded.extend_from_slice(bytes);
        });
        assert_eq!(result, 4);
        assert_eq!(recorded.as_slice(), &payload);
    }
}
