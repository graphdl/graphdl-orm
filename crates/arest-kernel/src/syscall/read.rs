// crates/arest-kernel/src/syscall/read.rs
//
// Linux x86_64 syscall 0: `read(int fd, void *buf, size_t count)`.
// Per #508 (stdin / keyboard-ring read track). The first half of the
// read/write pair that rounds out the tier-1 file-descriptor surface:
// `write` (#507 / syscall 1) drains bytes to fd 1 (stdout → serial);
// `read` (#508 / syscall 0) fills bytes from fd 0 (stdin → keyboard ring).
//
// Linux x86_64 number: `__NR_read = 0`
// (`linux/arch/x86/include/uapi/asm/unistd_64.h`).
//
// Tier-1 read semantics for fd 0
// --------------------------------
// The kernel keyboard ring (`arch::uefi::keyboard`) carries
// `pc_keyboard::DecodedKey` entries fed by the IRQ 1 handler (via
// `handle_scancode`). The read handler drains the ring non-blockingly:
//
//   * `DecodedKey::Unicode(c)` → encode `c` as UTF-8 and append the
//     bytes to the caller's buffer (up to `count` bytes total).
//   * `DecodedKey::RawKey(_)` → skip. Raw keys are pre-Unicode
//     (modifier-only, media keys, etc.); there's no meaningful byte
//     representation in the Linux TTY model.
//
// Returns the number of bytes written into the buffer (0 if the ring
// was empty — non-blocking, no busy-loop).
//
// POSIX non-blocking behaviour
// ----------------------------
// Linux `read(0, buf, n)` on a non-blocking file descriptor returns 0
// immediately when there's nothing to read (or -EAGAIN in O_NONBLOCK
// mode). The keyboard ring is inherently non-blocking — it's a fixed-
// size `VecDeque` populated by an IRQ handler; there's no blocking wait
// primitive in tier-1. We return 0 (empty read) to signal EOF/no-data,
// which glibc / musl handle by treating stdin as exhausted for the
// current call and retrying on the next select/poll cycle.
//
// Tier-1 identity-mapped pointer model
// -------------------------------------
// Follows the precedent set by `ioctl::handle` (TIOCGWINSZ / TCGETS)
// and `getrandom::handle`: user pointer `buf` (rsi) is treated as a
// kernel-space pointer under the tier-1 identity mapping (no real page
// tables until #527). The same `core::ptr::write` + offset pattern
// `getrandom` uses for its copy-to-user step applies here. A null `buf`
// with non-zero `count` returns `-EFAULT`; `count == 0` returns 0
// without touching the pointer (POSIX no-op). Once #527 lands real page
// tables and #561 lands `copy_to_user`, this function gains a
// `validate_userspace_range` pre-check — the existing offset-write
// pattern is forward-compatible.
//
// Source abstraction for testability
// ------------------------------------
// The handler mirrors `write::handle` / `write::do_write`: `handle`
// is the production entry that calls the UEFI keyboard ring; `do_read`
// takes a `&mut dyn FnMut() -> Option<u32>` keystroke source so unit
// tests can inject known Unicode codepoints without touching the real
// ring (which lives behind `#[cfg(all(target_os = "uefi", ...))]` and
// `#[cfg(feature = "repl")]`). The source returns `Option<u32>` —
// `Some(codepoint)` or `None` when drained.
//
// Device-fd reads (#537)
// -----------------------
// Beyond fd 0 (stdin), a process can `openat` a `/dev/*` special device
// (#537 — `/dev/null`, `/dev/zero`, `/dev/random`), which lands a
// `FdEntry::Synthetic { path }` in the per-process fd table. A `read` of
// such an fd routes through the device's behaviour (`synthetic_fs::
// device_behavior`): `/dev/null` returns EOF (0 bytes), `/dev/zero`
// fills the buffer with zeros, `/dev/random` fills it with CSPRNG bytes.
// The fill is sourced from `synthetic_fs::device_read` (the single
// device-table source of truth) and copied into the caller's buffer via
// the same identity-mapped `core::ptr::write` pattern the keyboard path
// uses. Non-device synthetic fds (`/proc/*`, `/sys/*`) and File-cell fds
// are out of #537 scope and still return `-EBADF` here (the full VFS
// read lands in a later track) — this slice wires the device subtree
// only, which is what `/dev/null`-style behaviour needs.
//
// errno values used
// -----------------
//   EBADF  =  9  — fd is not 0 (stdin) and not a readable `/dev/*`
//                  device fd. #508 scope was fd 0 only; #537 adds the
//                  device-fd path; the full VFS read lands later.
//   EFAULT = 14  — null buf with non-zero count.

use crate::process::current_process_fd_table;
use crate::process::fd_table::FdEntry;
use crate::syscall::dispatch::{EBADF, EFAULT};
use crate::synthetic_fs::{self, ReadKind};

/// Linux x86_64 syscall number for `read(fd, buf, count)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_read` (= 0).
/// The vendored musl tree confirms the same value at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_read`. Routes to
/// `read::handle`, which drains decoded Unicode keystrokes from the
/// kernel keyboard ring into the caller's buffer. Per #508.
pub const SYS_READ: u64 = 0;

/// File-descriptor number for stdin per POSIX
/// (`<unistd.h>:STDIN_FILENO`). Linux libc defines it as 0; the
/// constant is here so the handler reads as code rather than as a
/// magic number.
pub const STDIN_FD: u64 = 0;

/// Handle a `read(fd, buf, count)` syscall.
///
/// * `fd == 0` (stdin): drain available decoded keystrokes from the
///   kernel keyboard ring. Each `DecodedKey::Unicode(c)` is encoded as
///   UTF-8 and appended to the caller's buffer up to `count` bytes.
///   `DecodedKey::RawKey(_)` entries are skipped (no byte-level
///   representation). Returns the number of bytes written (0 if the
///   ring was empty — non-blocking).
///
/// * `fd != 0`: returns `-EBADF`. File reads land in a later track
///   (#560 VFS); tier-1 only supports fd 0 on the read side.
///
/// Edge cases:
///   * `count == 0` → returns 0 immediately, regardless of `buf`.
///   * `buf == 0 && count > 0` → returns `-EFAULT`.
///
/// SAFETY: `buf` is treated as a kernel-space pointer under tier-1's
/// identity mapping. The null check above guards against the common
/// mistake; a non-null but unmapped address would fault under real page
/// tables (future #527 / #561 add the validate step).
pub fn handle(fd: u64, buf: u64, count: u64) -> i64 {
    // POSIX no-op: a zero-length read returns 0 without touching `buf`,
    // regardless of which fd it targets (so a probe `read(fd, NULL, 0)`
    // succeeds). Checked before the fd dispatch so every fd shares the
    // short-circuit.
    if count == 0 {
        return 0;
    }
    if buf == 0 {
        return -EFAULT;
    }

    if fd == STDIN_FD {
        return do_read(buf, count, &mut keyboard_source());
    }

    // Beyond stdin: a `/dev/*` device fd opened via `openat` (#537).
    // Resolve the fd in the per-process fd table; if it's a synthetic
    // device path, fill per the device's read behaviour. Anything else
    // (non-device synthetic, File-cell fd, unknown fd) → -EBADF.
    read_device_fd(fd, buf, count)
}

/// Read from a `/dev/*` device fd (#537, #538). Looks the fd up in the
/// current process's fd table; when it resolves to a `Synthetic` device
/// path, fills the caller's buffer per the device's `ReadKind`:
///
///   * `Eof` / `Zeros` / `Random` (`/dev/null`, `/dev/zero`,
///     `/dev/random`) — bytes are table-computable; sourced from
///     `synthetic_fs::device_read` and copied into the buffer.
///   * `Console` (`/dev/tty`, #538) — bytes come from the kernel's
///     primary console input, the same keyboard ring fd 0 drains. We
///     route through `do_read(buf, count, &mut keyboard_source())` —
///     the identical stdin path — so `/dev/tty` and fd 0 deliver byte-
///     for-byte the same input (a process can `read` either and see the
///     same keystrokes). The device table tells us *that* this fd is
///     console-sourced; the existing stdin machinery does the fill.
///
/// Returns `-EBADF` for any fd that isn't a readable device (unknown fd,
/// File-cell fd, non-device synthetic path, or no process installed).
///
/// SAFETY: same identity-mapped buffer write as `do_read` — `buf` is
/// non-null (caller checked) and the fill never exceeds `count` bytes.
fn read_device_fd(fd: u64, buf: u64, count: u64) -> i64 {
    // i32 is the fd-table key width; a fd that doesn't fit (e.g. a huge
    // u64 from a malformed call) can't be in the table → -EBADF.
    let Ok(fd_i32) = i32::try_from(fd) else {
        return -EBADF;
    };

    // Extract the device path out of the fd table inside the lock, then
    // drop the lock before sourcing bytes (the CSPRNG fill for
    // `/dev/random` and the keyboard ring for `/dev/tty` each take their
    // own lock — don't nest).
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

    // Bound the request the same way `do_read` does (the slice / pointer
    // arithmetic needs `count <= isize::MAX`).
    if count > isize::MAX as u64 {
        return -EFAULT;
    }

    // Consult the device table for the read behaviour. `None` means the
    // synthetic path isn't a `/dev/*` device (e.g. a `/proc/*` fd) — out
    // of device-fd scope, return -EBADF.
    let Some(behavior) = synthetic_fs::device_behavior(&path) else {
        return -EBADF;
    };

    // `/dev/tty` (`ReadKind::Console`) sources from the live console
    // input — the same keyboard ring fd 0 uses. Route through the
    // identical stdin path so the two fds deliver the same keystrokes.
    // The table-computable devices fall through to `device_read`.
    if behavior.read == ReadKind::Console {
        return do_read(buf, count, &mut keyboard_source());
    }

    // Table-computable devices (`/dev/null` → EOF, `/dev/zero` → zeros,
    // `/dev/random` → CSPRNG): source the bytes from the single device-
    // table entry point. (`Console` was handled above; `device_read`
    // returns `None` for it, which would be -EBADF — but we never reach
    // here for it.)
    let Some(bytes) = synthetic_fs::device_read(&path, count as usize) else {
        return -EBADF;
    };

    // Copy the filled bytes into the caller's buffer via the identity-
    // mapped pointer write (same pattern as `do_read` / `getrandom::
    // fill_userspace`). `bytes.len() <= count` by construction so no
    // out-of-bounds write is possible.
    for (i, &b) in bytes.iter().enumerate() {
        // SAFETY: `buf` is non-null (caller checked), `i < bytes.len()
        // <= count <= isize::MAX`, and tier-1's identity mapping makes
        // the address valid kernel memory.
        unsafe {
            core::ptr::write((buf + i as u64) as *mut u8, b);
        }
    }
    bytes.len() as i64
}

/// Production keystroke source: pops from the UEFI keyboard ring.
///
/// Returns a closure that yields `Some(codepoint)` for each
/// `DecodedKey::Unicode(c)` and `None` when the ring is empty or
/// the next entry is a `RawKey`. Compiled only on the UEFI x86_64
/// target with the `repl` feature, where the keyboard ring is live.
/// On all other targets (host unit-test builds, aarch64, armv7) the
/// fallback below is used — it always returns `None`, which makes the
/// production `handle` behave as "ring empty / no stdin" on those
/// builds, which is correct (no keyboard hardware on those paths).
#[cfg(all(target_os = "uefi", target_arch = "x86_64", feature = "repl"))]
fn keyboard_source() -> impl FnMut() -> Option<u32> {
    || {
        use crate::arch::uefi::keyboard::read_keystroke;
        use pc_keyboard::DecodedKey;
        loop {
            match read_keystroke() {
                None => return None,
                Some(DecodedKey::Unicode(c)) => return Some(c as u32),
                Some(DecodedKey::RawKey(_)) => {
                    // Skip non-Unicode keys — no byte representation in
                    // the Linux TTY model; continue draining so the next
                    // Unicode key (if any) is found in one call.
                    continue;
                }
            }
        }
    }
}

/// Fallback keystroke source for non-UEFI builds (host unit tests,
/// aarch64 UEFI, armv7 UEFI). Always returns `None` — the keyboard
/// ring is only live on the UEFI x86_64 + `repl`-feature path.
/// Unit tests override this via `do_read` directly, injecting their
/// own source closure rather than going through `handle`.
#[cfg(not(all(target_os = "uefi", target_arch = "x86_64", feature = "repl")))]
fn keyboard_source() -> impl FnMut() -> Option<u32> {
    || None
}

/// Shared work for the read handler — separated from `handle` so unit
/// tests can inject a mock keystroke source without touching the kernel
/// keyboard ring. Mirrors the `write::do_write` / `sink` pattern.
///
/// `source` is a closure that returns `Some(Unicode codepoint as u32)`
/// for each available keystroke, or `None` when there's nothing left.
/// This function calls `source` repeatedly until:
///   (a) `source` returns `None` (ring drained), or
///   (b) the next keystroke's UTF-8 encoding would overflow `count`.
///
/// Each codepoint is encoded to UTF-8 and the bytes are written
/// directly into the caller's buffer at `buf + offset` via
/// `core::ptr::write`. Returns the total number of bytes written as
/// a non-negative `i64`.
///
/// Returns `-EFAULT` if `count > isize::MAX` (same guard `do_write`
/// uses — a malicious caller could set rdx to a huge value; reject
/// before any pointer arithmetic).
///
/// SAFETY: `buf` is non-null (caller checked) and points into a valid
/// buffer of at least `count` bytes under tier-1's identity mapping.
/// The write loop advances by the UTF-8 byte length of each char and
/// stops before exceeding `count` — no out-of-bounds write is possible.
pub fn do_read(buf: u64, count: u64, source: &mut dyn FnMut() -> Option<u32>) -> i64 {
    if count > isize::MAX as u64 {
        return -EFAULT;
    }
    let capacity = count as usize;
    let mut written: usize = 0;

    loop {
        let Some(codepoint) = source() else { break };

        // Decode the codepoint to a char; skip invalid codepoints.
        let Some(c) = char::from_u32(codepoint) else { continue };

        // Encode the char to UTF-8.
        let mut utf8_buf = [0u8; 4];
        let encoded: &[u8] = c.encode_utf8(&mut utf8_buf).as_bytes();

        // Stop if the encoded bytes won't fit in the remaining buffer.
        if written + encoded.len() > capacity {
            break;
        }

        // Write each byte into the caller's buffer at the correct offset.
        // SAFETY: `buf` is non-null (caller checked), `written + i <
        // capacity <= count <= isize::MAX`, and the identity mapping
        // means the address is valid kernel memory in tier-1.
        for (i, &byte) in encoded.iter().enumerate() {
            unsafe {
                core::ptr::write((buf + (written + i) as u64) as *mut u8, byte);
            }
        }
        written += encoded.len();
    }

    written as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    // ---------------------------------------------------------------
    // Constant value test
    // ---------------------------------------------------------------

    /// `SYS_READ` is 0 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_read`.
    /// Static check so a future renumbering surfaces in the test diff.
    #[test]
    fn sys_read_constant_is_zero() {
        assert_eq!(SYS_READ, 0, "SYS_READ must be 0 per Linux x86_64 unistd_64.h");
    }

    /// `STDIN_FD` is 0 — matches POSIX `<unistd.h>:STDIN_FILENO`.
    #[test]
    fn stdin_fd_constant_is_zero() {
        assert_eq!(STDIN_FD, 0);
    }

    // ---------------------------------------------------------------
    // fd validation tests
    // ---------------------------------------------------------------

    /// `read(1, ..., ...)` — fd 1 (stdout) is not readable in tier-1;
    /// returns `-EBADF`. Matches Linux: only fd 0 is the read-side
    /// stdin; fd 1/2 are write-side.
    #[test]
    fn read_non_stdin_fd_returns_ebadf() {
        let mut buf = [0u8; 16];
        let result = handle(1, buf.as_mut_ptr() as u64, buf.len() as u64);
        assert_eq!(result, -EBADF, "read(fd=1, ...) must return -EBADF");
    }

    /// `read(2, ..., ...)` — fd 2 (stderr) also returns `-EBADF`.
    #[test]
    fn read_stderr_fd_returns_ebadf() {
        let mut buf = [0u8; 16];
        let result = handle(2, buf.as_mut_ptr() as u64, buf.len() as u64);
        assert_eq!(result, -EBADF);
    }

    /// `read(u64::MAX, ..., ...)` — arbitrary invalid fd → -EBADF.
    #[test]
    fn read_arbitrary_fd_returns_ebadf() {
        let mut buf = [0u8; 16];
        let result = handle(u64::MAX, buf.as_mut_ptr() as u64, buf.len() as u64);
        assert_eq!(result, -EBADF);
    }

    // ---------------------------------------------------------------
    // Edge-case tests (fd == 0, buf / count special values)
    // ---------------------------------------------------------------

    /// `read(0, buf, 0)` — zero count is a POSIX no-op; returns 0
    /// without dereferencing buf (even if buf is null).
    #[test]
    fn read_zero_count_returns_zero_even_with_null_buf() {
        let result = handle(STDIN_FD, 0, 0);
        assert_eq!(result, 0, "read with count=0 must return 0");
    }

    /// `read(0, NULL, n>0)` — null buf with non-zero count → -EFAULT.
    /// Linux behaviour: bad address returns EFAULT before any I/O.
    #[test]
    fn read_null_buf_with_nonzero_count_returns_efault() {
        let result = handle(STDIN_FD, 0, 16);
        assert_eq!(result, -EFAULT);
    }

    // ---------------------------------------------------------------
    // do_read tests — inject mock keystroke sources
    // ---------------------------------------------------------------

    /// `do_read` with a source yielding `'A'` then `None` — writes
    /// 0x41 into the buffer and returns 1.
    #[test]
    fn do_read_single_ascii_char_written_to_buffer() {
        let mut buf = [0u8; 16];
        let mut chars = ['A' as u32].iter().copied();
        let result = do_read(
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            &mut || chars.next(),
        );
        assert_eq!(result, 1, "one ASCII char must produce 1 byte");
        assert_eq!(buf[0], b'A');
    }

    /// Multiple ASCII chars — `hello` → 5 bytes.
    #[test]
    fn do_read_multiple_ascii_chars_written_correctly() {
        let mut buf = [0u8; 16];
        let mut chars = b"hello".iter().map(|&b| b as u32);
        let result = do_read(
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            &mut || chars.next(),
        );
        assert_eq!(result, 5);
        assert_eq!(&buf[..5], b"hello");
    }

    /// Empty source (ring was empty) → returns 0, buffer unchanged.
    #[test]
    fn do_read_empty_source_returns_zero() {
        let mut buf = [0xffu8; 16];
        let result = do_read(buf.as_mut_ptr() as u64, buf.len() as u64, &mut || None);
        assert_eq!(result, 0, "empty source must return 0 bytes");
        // Buffer must be untouched.
        assert!(buf.iter().all(|&b| b == 0xff));
    }

    /// Multi-byte UTF-8: U+00E9 (é) encodes as [0xC3, 0xA9] (2 bytes).
    #[test]
    fn do_read_two_byte_utf8_char_encoded_correctly() {
        let mut buf = [0u8; 16];
        let c = 'é' as u32; // U+00E9
        let mut chars = core::iter::once(c);
        let result = do_read(
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            &mut || chars.next(),
        );
        assert_eq!(result, 2, "é must encode to 2 UTF-8 bytes");
        assert_eq!(buf[0], 0xC3);
        assert_eq!(buf[1], 0xA9);
    }

    /// U+4E2D (中) encodes as [0xE4, 0xB8, 0xAD] (3 bytes).
    #[test]
    fn do_read_three_byte_utf8_char_encoded_correctly() {
        let mut buf = [0u8; 16];
        let c = '中' as u32; // U+4E2D
        let mut chars = core::iter::once(c);
        let result = do_read(
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            &mut || chars.next(),
        );
        assert_eq!(result, 3, "中 must encode to 3 UTF-8 bytes");
        assert_eq!(buf[0], 0xE4);
        assert_eq!(buf[1], 0xB8);
        assert_eq!(buf[2], 0xAD);
    }

    /// Count stops at buffer capacity — a source with more chars than
    /// the buffer can hold is drained only up to `count` bytes.
    #[test]
    fn do_read_stops_at_count_boundary() {
        // Buffer holds 3 bytes; source provides 6 ASCII chars.
        let mut buf = [0u8; 3];
        let mut chars = b"abcdef".iter().map(|&b| b as u32);
        let result = do_read(
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            &mut || chars.next(),
        );
        assert_eq!(result, 3, "must stop after filling the buffer");
        assert_eq!(&buf, b"abc");
    }

    /// A multi-byte char that doesn't fit is not partially written —
    /// if the remaining capacity is 1 byte and the next char encodes
    /// to 2 bytes, the loop stops and the char is not emitted.
    #[test]
    fn do_read_partial_multibyte_char_not_written() {
        // 1-byte buffer; source yields 'é' (2 bytes). Char must not
        // be partially written.
        let mut buf = [0xffu8; 1];
        let c = 'é' as u32;
        let mut chars = core::iter::once(c);
        let result = do_read(
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            &mut || chars.next(),
        );
        assert_eq!(result, 0, "partial multi-byte char must not be written");
        assert_eq!(buf[0], 0xff, "buffer must be untouched");
    }

    /// `do_read` rejects oversized count (> isize::MAX) → -EFAULT.
    /// Mirrors the `do_write` guard — a malicious rdx value is caught
    /// before any pointer arithmetic.
    #[test]
    fn do_read_rejects_oversized_count() {
        let buf_addr = 0x1000_u64; // non-null; guard fires before deref
        let result = do_read(buf_addr, (isize::MAX as u64) + 1, &mut || None);
        assert_eq!(result, -EFAULT);
    }

    // ---------------------------------------------------------------
    // Dispatch wiring tests
    // ---------------------------------------------------------------

    /// `dispatch(SYS_READ, 0, buf, count, ...)` routes to `read::handle`
    /// and, with the host keyboard ring empty, returns 0.
    #[test]
    fn dispatch_sys_read_stdin_empty_returns_zero() {
        use crate::syscall::dispatch::{dispatch, SYS_READ};
        let mut buf = [0u8; 16];
        let result = dispatch(
            SYS_READ,
            STDIN_FD,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            0,
            0,
            0,
        );
        // On the host test build the keyboard ring is unavailable
        // (`keyboard_source` always returns None) so 0 bytes are read.
        assert_eq!(result, 0);
    }

    /// `dispatch(SYS_READ, fd=1, ...)` → -EBADF via the dispatch table.
    /// Verifies the dispatcher routes SYS_READ (0) and the fd guard fires.
    #[test]
    fn dispatch_sys_read_wrong_fd_returns_ebadf() {
        use crate::syscall::dispatch::{dispatch, SYS_READ};
        let mut buf = [0u8; 16];
        let result = dispatch(
            SYS_READ,
            1, // fd=1 (stdout) — not readable
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            0,
            0,
            0,
        );
        assert_eq!(result, -EBADF);
    }

    /// Mix of ASCII chars from the mock source, verified via heap buffer.
    /// Exercises the `do_read` → buffer-write path with a `Vec`-backed
    /// destination (safe because `Vec<u8>` is a contiguous heap alloc).
    #[test]
    fn do_read_into_heap_buffer_matches_expected_bytes() {
        // Allocate a Vec large enough, then pass its raw pointer.
        let expected = b"rust";
        let count = expected.len();
        let mut heap_buf: Vec<u8> = alloc::vec![0u8; count];
        let mut chars = expected.iter().map(|&b| b as u32);
        let result = do_read(
            heap_buf.as_mut_ptr() as u64,
            count as u64,
            &mut || chars.next(),
        );
        assert_eq!(result, count as i64);
        assert_eq!(&heap_buf[..count], expected);
    }

    // ---------------------------------------------------------------
    // /dev/* device-fd read tests (#537)
    // ---------------------------------------------------------------
    //
    // These exercise the fd → fd-table → device-read dispatch added in
    // #537. They allocate a device fd directly through the fd table
    // (`synthetic("/dev/...")`) rather than via `openat` to keep the
    // test focused on `read::handle`; the openat→fd-table wiring is
    // covered by the openat tests.

    use crate::process::address_space::AddressSpace;
    use crate::process::fd_table::synthetic;
    use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
    use crate::process::{current_process_install, current_process_uninstall, Process};

    /// Install a fresh Process and allocate a device fd against `path`.
    /// Returns the fd. Caller must hold `CURRENT_PROCESS_TEST_LOCK`.
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

    /// `read` of a `/dev/null` fd returns 0 (EOF) and does not touch the
    /// buffer — matches `/dev/null`'s read contract.
    #[test]
    fn read_dev_null_fd_returns_eof() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let fd = install_with_device_fd("/dev/null");
        let mut buf = [0xAAu8; 16];
        let result = handle(fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
        assert_eq!(result, 0, "/dev/null read must return 0 (EOF)");
        // Buffer untouched (no bytes written).
        assert!(buf.iter().all(|&b| b == 0xAA), "buffer must be untouched");
        current_process_uninstall();
    }

    /// `read` of a `/dev/zero` fd fills the whole buffer with zero bytes
    /// and returns the count.
    #[test]
    fn read_dev_zero_fd_fills_with_zeros() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let fd = install_with_device_fd("/dev/zero");
        let mut buf = [0xFFu8; 32];
        let result = handle(fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
        assert_eq!(result, 32, "/dev/zero read must fill the whole request");
        assert!(buf.iter().all(|&b| b == 0), "every byte must be 0x00");
        current_process_uninstall();
    }

    /// `read` of a `/dev/random` fd fills the buffer with cryptographically
    /// secure random bytes (returns the count). Three properties are
    /// pinned, matching #577's spec for a CSPRNG-backed `/dev/random`:
    ///
    ///   1. RETURNS THE FULL COUNT — a 16-byte read returns 16.
    ///   2. BYTES COME FROM THE CSPRNG — under a deterministic entropy
    ///      seed the device read is byte-for-byte equal to a direct
    ///      `arest::csprng::random_bytes` draw from the same reseeded
    ///      state. This proves the device is wired to the ChaCha20 CSPRNG
    ///      (`getrandom(2)`'s source), not a weak/predictable stand-in:
    ///      swap in a different source and the equality breaks. Comparing
    ///      against a fresh draw avoids pinning a hand-computed keystream
    ///      constant while still nailing provenance to the exact stream.
    ///   3. READS ARE NON-REPEATING ACROSS CALLS — two successive reads
    ///      on the same fd yield different bytes (the ChaCha20 counter
    ///      advances). A fixed/zero source — the failure mode #577 guards
    ///      against — would return identical buffers and fail here.
    #[test]
    fn read_dev_random_fd_fills_with_random_bytes() {
        use arest::entropy::{self, DeterministicSource};

        // Serialise entropy install/body/uninstall against concurrent
        // tests touching the global CSPRNG (same rationale as
        // getrandom::tests::TEST_ENTROPY_LOCK).
        static TEST_ENTROPY_LOCK: spin::Mutex<()> = spin::Mutex::new(());
        let _entropy_guard = TEST_ENTROPY_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();

        // (2) Provenance reference: the exact bytes the kernel CSPRNG
        // emits for the first 16-byte draw after a reseed under this
        // deterministic seed. Computed here, then reproduced via the
        // device path below — the two must match to the byte.
        //
        // `DeterministicSource` advances an internal counter per `fill`,
        // and `csprng::reseed()` does NOT reset that counter — so to make
        // the device read derive the SAME ChaCha20 key as `expected`, a
        // FRESH source (counter at 0) is installed before each reseed.
        entropy::install(alloc::boxed::Box::new(DeterministicSource::new([11u8; 32])));
        arest::csprng::reseed();
        let mut expected = [0u8; 16];
        arest::csprng::random_bytes(&mut expected);
        // A non-zero stream is a precondition for the equality check to
        // be meaningful (a zero source would trivially "match" a zero
        // device read), so assert it explicitly.
        assert!(
            expected.iter().any(|&b| b != 0),
            "CSPRNG keystream for the test seed must be non-zero"
        );

        // Re-seed the CSPRNG from a fresh source at counter 0 so the
        // device read below draws the same first 16 bytes as `expected`.
        entropy::install(alloc::boxed::Box::new(DeterministicSource::new([11u8; 32])));
        arest::csprng::reseed();
        let fd = install_with_device_fd("/dev/random");

        // (1) First read fills the whole 16-byte request.
        let mut buf = [0u8; 16];
        let result = handle(fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
        assert_eq!(result, 16, "/dev/random read must fill the whole request");
        // (2) ...with the exact CSPRNG keystream — proves the bytes come
        // from `arest::csprng`, not a weak source or a zero/fixed fill.
        assert_eq!(
            buf, expected,
            "/dev/random must yield the kernel CSPRNG keystream byte-for-byte"
        );

        // (3) A second read on the same fd must advance the stream — the
        // buffers must differ. A fixed or zero source would repeat here.
        let mut buf2 = [0u8; 16];
        let result2 = handle(fd as u64, buf2.as_mut_ptr() as u64, buf2.len() as u64);
        assert_eq!(result2, 16, "second /dev/random read must also fill 16 bytes");
        assert_ne!(
            buf, buf2,
            "/dev/random reads must be non-repeating across calls (not a fixed pattern)"
        );

        current_process_uninstall();
        entropy::uninstall();
        arest::csprng::reseed();
    }

    /// `read` of an unknown fd (not stdin, not a device fd) returns
    /// `-EBADF` — the fd-table lookup misses.
    #[test]
    fn read_unknown_fd_returns_ebadf() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        // Install a process but allocate no fds; fd 7 is not in the table.
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(7, address_space);
        current_process_install(proc);
        let mut buf = [0u8; 16];
        let result = handle(7, buf.as_mut_ptr() as u64, buf.len() as u64);
        assert_eq!(result, -EBADF);
        current_process_uninstall();
    }

    /// A non-device synthetic fd (`/proc/cpuinfo`) is out of #537 read
    /// scope — `read` returns `-EBADF` (the full VFS read lands later).
    #[test]
    fn read_non_device_synthetic_fd_returns_ebadf() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let fd = install_with_device_fd("/proc/cpuinfo");
        let mut buf = [0u8; 16];
        let result = handle(fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
        assert_eq!(result, -EBADF);
        current_process_uninstall();
    }

    /// `read(dev_fd, NULL, n>0)` returns `-EFAULT` — a null buffer is
    /// rejected before the device fill (the device path doesn't bypass
    /// the buffer-validity check).
    #[test]
    fn read_dev_zero_null_buf_returns_efault() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let fd = install_with_device_fd("/dev/zero");
        let result = handle(fd as u64, 0, 16);
        assert_eq!(result, -EFAULT);
        current_process_uninstall();
    }

    // ---------------------------------------------------------------
    // /dev/tty device-fd read tests (#538)
    // ---------------------------------------------------------------
    //
    // `/dev/tty` reads source the kernel's primary console input — the
    // same keyboard ring fd 0 drains. On the host test build the real
    // keyboard ring is unavailable (`keyboard_source` always returns
    // `None`), so a `handle`-level read of `/dev/tty` returns 0 (empty
    // ring), exactly like a `read(0, …)` on an idle stdin. The crucial
    // assertion is that it returns 0 (the console path) and NOT -EBADF
    // (which is what a mis-wired `/dev/tty` would yield if it fell
    // through to `device_read`, since `device_read` returns `None` for
    // the `Console` marker). The byte-delivery half of the contract —
    // that console input actually lands in the buffer — is verified via
    // `do_read` with an injected source (the same seam the fd-0 tests
    // use), since the host build can't push into the real ring.

    /// `read` of a `/dev/tty` fd takes the console-input path: with the
    /// host keyboard ring empty it returns 0 (no input), the same as an
    /// idle `read(0, …)`. It must NOT return -EBADF — that would mean
    /// the device fell through to the table `device_read` instead of the
    /// keyboard source. This pins the routing of the `Console` read
    /// marker to the stdin path.
    #[test]
    fn read_dev_tty_fd_routes_to_console_input_empty_ring_returns_zero() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let fd = install_with_device_fd("/dev/tty");
        let mut buf = [0xAAu8; 16];
        let result = handle(fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
        assert_eq!(
            result, 0,
            "/dev/tty read must take the console path (0 on empty ring), not -EBADF"
        );
        assert_ne!(result, -EBADF, "/dev/tty must not fall through to device_read");
        // Empty ring → no bytes written, buffer untouched (same as fd 0).
        assert!(buf.iter().all(|&b| b == 0xAA), "buffer must be untouched");
        current_process_uninstall();
    }

    /// `/dev/tty`'s console-input source is byte-for-byte the same as
    /// fd 0's: a `read(/dev/tty fd, …)` and a `read(0, …)` both drain the
    /// keyboard ring through `do_read(buf, count, &mut keyboard_source())`.
    /// On the host both see an empty ring and return 0 — this asserts the
    /// two paths agree (the contract that `/dev/tty` IS the same console
    /// input as stdin), so a regression that wired `/dev/tty` to a
    /// different source would diverge here.
    #[test]
    fn read_dev_tty_matches_stdin_fd0_on_empty_ring() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let fd = install_with_device_fd("/dev/tty");
        let mut tty_buf = [0u8; 8];
        let tty_result = handle(fd as u64, tty_buf.as_mut_ptr() as u64, tty_buf.len() as u64);
        // fd 0 (stdin) read of the same empty ring.
        let mut stdin_buf = [0u8; 8];
        let stdin_result = handle(STDIN_FD, stdin_buf.as_mut_ptr() as u64, stdin_buf.len() as u64);
        assert_eq!(
            tty_result, stdin_result,
            "/dev/tty and fd 0 must source the same console input"
        );
        assert_eq!(tty_result, 0, "both empty-ring reads return 0 on host");
        current_process_uninstall();
    }

    /// The console-input source `/dev/tty` shares with fd 0 delivers
    /// keystrokes into the buffer. Verified through `do_read` with an
    /// injected source (the production `/dev/tty` path is `do_read(buf,
    /// count, &mut keyboard_source())`; this exercises the same `do_read`
    /// with a mock source standing in for the keyboard ring the host
    /// build can't populate). Confirms the read side returns console
    /// INPUT bytes, not EOF/zeros — i.e. `/dev/tty` is an input device.
    #[test]
    fn read_dev_tty_console_source_delivers_input_bytes() {
        // Mock the console-input source the way the keyboard ring would
        // feed `do_read` for a `/dev/tty` read: a sequence of decoded
        // Unicode codepoints, then `None` when drained.
        let mut buf = [0u8; 16];
        let mut keystrokes = b"tty-in".iter().map(|&b| b as u32);
        let result = do_read(
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            &mut || keystrokes.next(),
        );
        assert_eq!(result, 6, "console input must land in the buffer");
        assert_eq!(&buf[..6], b"tty-in", "/dev/tty read returns console input bytes");
    }
}
