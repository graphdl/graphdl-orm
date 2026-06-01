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

use crate::process::current_process_fd_table;
use crate::process::fd_table::FdEntry;
use crate::syscall::dispatch::{EBADF, EFAULT};
use crate::synthetic_fs::{self, WriteKind};

/// File-descriptor number for stdout per POSIX
/// (`<unistd.h>:STDOUT_FILENO`). Linux libc defines it as 1; the
/// constant is here so the handler reads as code rather than as a
/// magic number.
pub const STDOUT_FD: u64 = 1;

/// File-descriptor number for stderr per POSIX
/// (`<unistd.h>:STDERR_FILENO`). Linux libc defines it as 2. Both
/// stdout (fd 1) and stderr (fd 2) route to the kernel serial console
/// in tier-1 — there is a single output stream (the UEFI serial port
/// or ConOut), so merging them is the correct tier-1 behaviour.
pub const STDERR_FD: u64 = 2;

/// Handle a `write(fd, buf, count)` syscall. Returns the number of
/// bytes written on success (always == `count` for fd 1/2, which never
/// short-writes), or a negative `errno` on failure.
///
/// Tier-1 supported fds:
///   * `1` (stdout) → routes to the kernel serial console.
///   * `2` (stderr) → routes to the kernel serial console (same sink as
///     stdout; tier-1 has a single output stream).
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
    // Fast-path: zero-length write is a no-op per POSIX. Doing this
    // check before the fd / buf checks lets the test suite exercise
    // `write(1, NULL, 0)` without panicking on a null deref, and a
    // zero-length write to a device fd succeeds as a no-op too.
    if count == 0 {
        return 0;
    }

    // Beyond the standard streams: a `/dev/*` device fd opened via
    // `openat` (#537). A write to a discard device (`/dev/null`,
    // `/dev/zero`) consumes the bytes and reports the full count; a
    // write to a read-only device (`/dev/random`) returns -EBADF. This
    // is dispatched before the buf-null check because a discard device
    // still validates its buffer (a real `write(fd, NULL, n>0)` is
    // -EFAULT even to /dev/null).
    if fd != STDOUT_FD && fd != STDERR_FD {
        return write_device_fd(fd, buf, count);
    }

    if buf == 0 {
        return -EFAULT;
    }
    do_write(buf, count, &mut console_sink)
}

/// The kernel-console sink shared by every write that reaches the
/// primary console: fd 1 (stdout), fd 2 (stderr), and the `/dev/tty`
/// device fd (#538 — `WriteKind::Console`). Routes `bytes` to
/// `crate::print!`, which fans into `arch::_print` (UEFI ConOut pre-EBS,
/// UART 16550 / PL011 serial post-EBS — see the module header). Sharing
/// one sink keeps the three console write paths byte-identical: a
/// `write(1, …)`, a `write(2, …)`, and a `write(/dev/tty fd, …)` all
/// transcode and emit the same way, so `/dev/tty` is genuinely "the
/// console" rather than a parallel implementation that could drift.
///
/// Lossy UTF-8 conversion — a non-UTF-8 byte sequence (which a C program
/// could emit via `printf("\xff\xff")`) prints as U+FFFD replacement
/// chars rather than dropping the byte. `crate::print!` accepts a
/// `core::fmt::Arguments` so we wrap the lossy `str` in `format_args!`;
/// the underlying serial path handles the (transcoded) UCS-2 / UART
/// byte stream.
fn console_sink(bytes: &[u8]) {
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
}

/// Write to a `/dev/*` device fd (#537, #538). Looks the fd up in the
/// current process's fd table; when it resolves to a `Synthetic` device
/// path, applies the device's write behaviour:
///
///   * `WriteKind::Discard` (`/dev/null`, `/dev/zero`) — the bytes are
///     consumed and the full `count` is reported as written (a real
///     `/dev/null` accepts any volume and discards it). The buffer is
///     still validated (null `buf` with `count > 0` → `-EFAULT`) so a
///     bad pointer surfaces the same way it would for a real write.
///   * `WriteKind::Console` (`/dev/tty`, #538) — the bytes go to the
///     kernel's primary console output, the same ConOut + serial sink
///     fd 1/2 drive. We route through `do_write(buf, count,
///     &mut console_sink)` — the identical stdout/stderr path — so a
///     `write(/dev/tty fd, …)` reaches the console byte-for-byte the way
///     `write(1, …)` does. Reports the full `count` (the console never
///     short-writes in tier-1). The same null-buffer validation applies.
///   * `WriteKind::Reject` (`/dev/random`) — `-EBADF`. The device is
///     read-only; the fd was opened `O_RDONLY` and writing to it is the
///     same error Linux returns for a write to a read-only fd.
///
/// Returns `-EBADF` for any fd that isn't a `/dev/*` device fd (unknown
/// fd, File-cell fd, non-device synthetic path, no process installed) —
/// preserving the pre-#537 "only fd 1/2 are writable" surface for
/// everything outside the device subtree.
fn write_device_fd(fd: u64, buf: u64, count: u64) -> i64 {
    let Ok(fd_i32) = i32::try_from(fd) else {
        return -EBADF;
    };

    // Pull the synthetic path out of the fd table inside the lock.
    let path = current_process_fd_table(|maybe_table| {
        let table = maybe_table?;
        match table.lookup(fd_i32) {
            Some(FdEntry::Synthetic { path }) => Some(path.clone()),
            _ => None,
        }
    });
    let Some(path) = path else {
        return -EBADF;
    };

    // Consult the single device-table predicate for the write behaviour.
    // `None` → the synthetic path isn't a `/dev/*` device (e.g. a
    // `/proc/*` fd, which is read-only anyway) → -EBADF.
    let Some(behavior) = synthetic_fs::device_behavior(&path) else {
        return -EBADF;
    };

    match behavior.write {
        WriteKind::Discard => {
            // Validate the buffer the same way a real write would: a
            // null `buf` with non-zero count is -EFAULT even for the bit
            // bucket. (`count == 0` was already short-circuited in
            // `handle`.)
            if buf == 0 {
                return -EFAULT;
            }
            // Discard the bytes — nothing reads them — but report the
            // full count as written, which is `/dev/null`'s contract.
            count as i64
        }
        WriteKind::Console => {
            // Validate the buffer like any real write (null buf with
            // count > 0 is -EFAULT). `count == 0` was short-circuited in
            // `handle`.
            if buf == 0 {
                return -EFAULT;
            }
            // Route to the same console sink fd 1/2 use — `/dev/tty` IS
            // the console, so its writes are byte-identical to stdout's.
            do_write(buf, count, &mut console_sink)
        }
        WriteKind::Reject => -EBADF,
    }
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

    /// `write(2, "err", 3)` — stderr routes to the kernel serial console
    /// (same sink as stdout in tier-1). Returns 3 (the byte count written).
    /// Added in #500 (file-state surface) — the one-line `STDOUT_FD |
    /// STDERR_FD` extension anticipated by the original write.rs comment.
    #[test]
    fn write_to_stderr_routes_to_console_and_returns_count() {
        let payload = b"err";
        // Use do_write with a mock sink to verify the bytes reach the
        // sink (same pattern as write_to_stdout_routes_to_sink_and_returns_count).
        let mut recorded: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        let result = do_write(payload.as_ptr() as u64, payload.len() as u64, &mut |bytes| {
            recorded.extend_from_slice(bytes);
        });
        assert_eq!(result, payload.len() as i64);
        assert_eq!(recorded.as_slice(), payload);
        // Also verify handle() accepts fd=2 (does not return -EBADF).
        // We can't easily capture the serial output in a unit test, but
        // verifying the return value is sufficient.
        let handle_result = handle(STDERR_FD, payload.as_ptr() as u64, payload.len() as u64);
        assert_eq!(handle_result, payload.len() as i64, "write(fd=2) must return count");
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

    // ---------------------------------------------------------------
    // /dev/* device-fd write tests (#537)
    // ---------------------------------------------------------------
    //
    // Exercise the fd → fd-table → device-write dispatch added in #537.
    // Device fds are allocated directly through the fd table to keep the
    // test focused on `write::handle`.

    use crate::process::address_space::AddressSpace;
    use crate::process::fd_table::synthetic;
    use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
    use crate::process::{current_process_install, current_process_uninstall, Process};

    /// Install a fresh Process and allocate a device fd against `path`.
    /// Caller must hold `CURRENT_PROCESS_TEST_LOCK`.
    fn install_with_device_fd(path: &str) -> i32 {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(7, address_space);
        current_process_install(proc);
        current_process_fd_table(|t| {
            t.expect("process installed")
                .allocate(synthetic(path))
                .expect("allocate device fd")
        })
    }

    /// `write` to a `/dev/null` fd discards the bytes and returns the
    /// full count — the bit-bucket contract.
    #[test]
    fn write_dev_null_fd_discards_and_returns_count() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let fd = install_with_device_fd("/dev/null");
        let payload = b"this goes nowhere";
        let result = handle(fd as u64, payload.as_ptr() as u64, payload.len() as u64);
        assert_eq!(
            result,
            payload.len() as i64,
            "/dev/null write must report the full count"
        );
        current_process_uninstall();
    }

    /// `write` to a `/dev/zero` fd also discards and returns the count
    /// (zero is write-discard, read-zeros).
    #[test]
    fn write_dev_zero_fd_discards_and_returns_count() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let fd = install_with_device_fd("/dev/zero");
        let payload = b"discarded";
        let result = handle(fd as u64, payload.as_ptr() as u64, payload.len() as u64);
        assert_eq!(result, payload.len() as i64);
        current_process_uninstall();
    }

    /// A large `write` to `/dev/null` reports the full count — the
    /// device accepts any volume (no short write).
    #[test]
    fn write_dev_null_large_count_reports_full_count() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let fd = install_with_device_fd("/dev/null");
        let payload = alloc::vec![0xABu8; 65536];
        let result = handle(fd as u64, payload.as_ptr() as u64, payload.len() as u64);
        assert_eq!(result, payload.len() as i64);
        current_process_uninstall();
    }

    /// `write` to a `/dev/random` fd returns `-EBADF` — the device is
    /// read-only (write-reject).
    #[test]
    fn write_dev_random_fd_returns_ebadf() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let fd = install_with_device_fd("/dev/random");
        let payload = b"nope";
        let result = handle(fd as u64, payload.as_ptr() as u64, payload.len() as u64);
        assert_eq!(result, -EBADF, "/dev/random is read-only");
        current_process_uninstall();
    }

    /// `write(/dev/null, NULL, 0)` is a POSIX no-op — returns 0 (the
    /// zero-count short-circuit fires before the device dispatch).
    #[test]
    fn write_dev_null_zero_count_returns_zero() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let fd = install_with_device_fd("/dev/null");
        let result = handle(fd as u64, 0, 0);
        assert_eq!(result, 0);
        current_process_uninstall();
    }

    /// `write(/dev/null, NULL, n>0)` returns `-EFAULT` — a discard
    /// device still validates its buffer (a bad pointer is an error even
    /// to the bit bucket).
    #[test]
    fn write_dev_null_null_buf_returns_efault() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let fd = install_with_device_fd("/dev/null");
        let result = handle(fd as u64, 0, 16);
        assert_eq!(result, -EFAULT);
        current_process_uninstall();
    }

    /// A non-device synthetic fd (`/proc/cpuinfo`) is read-only — a
    /// `write` to it returns `-EBADF`.
    #[test]
    fn write_non_device_synthetic_fd_returns_ebadf() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let fd = install_with_device_fd("/proc/cpuinfo");
        let payload = b"x";
        let result = handle(fd as u64, payload.as_ptr() as u64, payload.len() as u64);
        assert_eq!(result, -EBADF);
        current_process_uninstall();
    }

    // ---------------------------------------------------------------
    // /dev/tty device-fd write tests (#538)
    // ---------------------------------------------------------------
    //
    // `/dev/tty` writes go to the kernel's primary console output — the
    // same ConOut + serial sink fd 1/2 drive (`console_sink` →
    // `crate::print!`). On the host build `crate::print!` is a no-op
    // stub, so we verify the contract in two complementary ways:
    //   1. The marshalling — that the exact write bytes reach a console
    //      sink — via `do_write` with a captured (mock) sink, the same
    //      seam the stdout/stderr tests use. This is the "writes reach
    //      console output" assertion against a captured console.
    //   2. The routing — that a `handle`-level `write(/dev/tty fd, …)`
    //      takes the `Console` path (returns the full count, not -EBADF
    //      or -EFAULT) — proving the device dispatch wires `/dev/tty` to
    //      the console sink rather than rejecting/discarding it.

    /// The console sink `/dev/tty` writes through delivers the exact
    /// bytes to console output. Captured via a mock sink fed by
    /// `do_write` — the production `/dev/tty` write path is `do_write(
    /// buf, count, &mut console_sink)`; this swaps `console_sink` for a
    /// recorder to assert the bytes that would reach the console. Same
    /// pattern as `write_to_stdout_routes_to_sink_and_returns_count`,
    /// confirming `/dev/tty` output is byte-identical to stdout's.
    #[test]
    fn write_dev_tty_console_sink_receives_exact_bytes() {
        let payload = b"hello tty";
        let mut captured_console: Vec<u8> = Vec::new();
        let result = do_write(payload.as_ptr() as u64, payload.len() as u64, &mut |bytes| {
            captured_console.extend_from_slice(bytes);
        });
        assert_eq!(result, payload.len() as i64, "/dev/tty write returns the full count");
        assert_eq!(
            captured_console.as_slice(),
            payload,
            "/dev/tty write must deliver the exact bytes to console output"
        );
    }

    /// `console_sink` (the shared fd 1/2 + `/dev/tty` console output sink)
    /// runs to completion on both well-formed UTF-8 and invalid byte
    /// sequences without panicking — the lossy path replaces out-of-range
    /// bytes with U+FFFD. Exercises the production sink itself (it routes
    /// to the host `print!` no-op stub, so the assertion is "does not
    /// panic / handles binary input"); the byte-exactness is covered by
    /// the captured-sink test above.
    #[test]
    fn write_dev_tty_console_sink_handles_utf8_and_binary() {
        // Well-formed UTF-8 (multi-byte char included).
        console_sink("ok-é-中".as_bytes());
        // Invalid UTF-8 — must take the byte-by-byte U+FFFD replacement
        // branch without panicking.
        console_sink(&[0xff, 0x41, 0xfe, 0x80]);
        // Empty slice is a no-op.
        console_sink(&[]);
    }

    /// `write` to a `/dev/tty` fd takes the `Console` path: returns the
    /// full byte count (the console never short-writes) and must NOT
    /// return -EBADF (which would mean the device was rejected like a
    /// read-only one). This pins the routing of the `WriteKind::Console`
    /// marker to the console sink through the real `handle` dispatch.
    #[test]
    fn write_dev_tty_fd_routes_to_console_and_returns_count() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let fd = install_with_device_fd("/dev/tty");
        let payload = b"to the console";
        let result = handle(fd as u64, payload.as_ptr() as u64, payload.len() as u64);
        assert_eq!(
            result,
            payload.len() as i64,
            "/dev/tty write must report the full count (console path)"
        );
        assert_ne!(result, -EBADF, "/dev/tty must not be rejected like a read-only device");
        current_process_uninstall();
    }

    /// A large `write` to `/dev/tty` reports the full count — the console
    /// path doesn't short-write (matches the fd 1/2 contract).
    #[test]
    fn write_dev_tty_large_count_reports_full_count() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let fd = install_with_device_fd("/dev/tty");
        let payload = alloc::vec![b'.'; 4096];
        let result = handle(fd as u64, payload.as_ptr() as u64, payload.len() as u64);
        assert_eq!(result, payload.len() as i64);
        current_process_uninstall();
    }

    /// `write(/dev/tty, NULL, 0)` is a POSIX no-op — returns 0 (the
    /// zero-count short-circuit fires before the device dispatch).
    #[test]
    fn write_dev_tty_zero_count_returns_zero() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let fd = install_with_device_fd("/dev/tty");
        let result = handle(fd as u64, 0, 0);
        assert_eq!(result, 0);
        current_process_uninstall();
    }

    /// `write(/dev/tty, NULL, n>0)` returns `-EFAULT` — the console
    /// device still validates its buffer (a bad pointer is an error even
    /// for the terminal, same as a real `write(1, NULL, n)`).
    #[test]
    fn write_dev_tty_null_buf_returns_efault() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let fd = install_with_device_fd("/dev/tty");
        let result = handle(fd as u64, 0, 16);
        assert_eq!(result, -EFAULT);
        current_process_uninstall();
    }
}
