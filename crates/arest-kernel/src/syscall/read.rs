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
// uses. Non-device synthetic fds (`/proc/*`, `/sys/*`) are still out of
// scope here and return `-EBADF` (the full VFS read of those renderers
// lands in a later track) — this slice wires the device subtree only.
//
// General file-fd reads (#499)
// ----------------------------
// `openat` of a path that resolves through the File-cell graph (a File
// entity's `File_has_Name` fact) lands a `FdEntry::File { cell_id,
// offset }` in the fd table (see `syscall::openat`). #499 makes such an
// fd readable: the read handler sources bytes from the File's
// `File_has_ContentRef` fact via `file_serve::read_file_cell` — the
// SAME predicate + `ContentRef` decode the HTTP `/file/{id}/content`
// route uses, so a File reads byte-for-byte identically over either
// surface. The per-fd `offset` is the read cursor: each read returns up
// to `count` bytes from `offset`, then advances the cursor by the number
// of bytes delivered, so sequential reads walk the file and a read at
// `offset == len` returns 0 (EOF). If the backing File's content fact
// has vanished (or is malformed) the handler returns `-EBADF`.
//
// errno values used
// -----------------
//   EBADF  =  9  — fd is not 0 (stdin), not a readable `/dev/*`
//                  device fd, and not a resolvable File-cell fd. #508
//                  scope was fd 0 only; #537 adds the device-fd path;
//                  #499 adds the general File-cell read.
//   EFAULT = 14  — null buf with non-zero count.

use alloc::string::String;
use alloc::vec::Vec;
use arest::ast::{self, Object};

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
/// * `fd != 0`: resolve the fd in the per-process fd table. A
///   `/dev/*` device fd (#537) fills per the device's read behaviour;
///   a File-cell fd (#499) reads bytes from the File's
///   `File_has_ContentRef` and advances the per-fd cursor. An unknown
///   fd, a non-device synthetic fd (`/proc/*`, `/sys/*`), or a File
///   whose content fact has vanished → `-EBADF`.
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

    // Beyond stdin: an fd opened via `openat`. Resolve it in the
    // per-process fd table and route on the backing kind — a `/dev/*`
    // device fill (#537/#538) or a general File-cell read (#499).
    read_open_fd(fd, buf, count)
}

/// What a non-stdin fd resolves to in the fd table, distilled out of
/// the table lock so the byte-sourcing (which takes its own locks — the
/// CSPRNG, the keyboard ring, or the SYSTEM cell graph) happens after
/// the table lock is dropped. `read_open_fd` matches on this.
enum ReadTarget {
    /// `FdEntry::Synthetic { path }` — a `/dev/*` device (or a
    /// non-device synthetic path, which `read_device` rejects).
    Synthetic { path: alloc::string::String },
    /// `FdEntry::File { cell_id, offset }` — a File-cell-backed fd. The
    /// `cell_id` keys `File_has_ContentRef`; `offset` is the current
    /// read cursor.
    File {
        cell_id: alloc::string::String,
        offset: u64,
    },
}

/// Resolve a non-stdin fd in the current process's fd table and route
/// to the right byte source: a `/dev/*` device fill (#537/#538) or a
/// general File-cell read (#499).
///
/// The fd-table lookup runs inside the table lock; the resolved target
/// (path or cell-id + offset) is copied out and the lock dropped before
/// any byte sourcing, because each source takes its own lock (the
/// CSPRNG for `/dev/random`, the keyboard ring for `/dev/tty`, the
/// SYSTEM cell graph for a File). Returns `-EBADF` for an unknown fd
/// (or no process installed).
///
/// SAFETY: delegates the buffer write to `read_device` / `read_file`,
/// which use the same identity-mapped `core::ptr::write` as `do_read`.
fn read_open_fd(fd: u64, buf: u64, count: u64) -> i64 {
    // i32 is the fd-table key width; a fd that doesn't fit (e.g. a huge
    // u64 from a malformed call) can't be in the table → -EBADF.
    let Ok(fd_i32) = i32::try_from(fd) else {
        return -EBADF;
    };

    // Resolve the backing kind out of the fd table inside the lock,
    // then drop the lock before sourcing bytes.
    let target = current_process_fd_table(|maybe_table| {
        let table = maybe_table?;
        match table.lookup(fd_i32) {
            Some(FdEntry::Synthetic { path }) => Some(ReadTarget::Synthetic {
                path: path.clone(),
            }),
            Some(FdEntry::File { cell_id, offset }) => Some(ReadTarget::File {
                cell_id: cell_id.clone(),
                offset: *offset,
            }),
            None => None,
        }
    });
    let Some(target) = target else {
        return -EBADF;
    };

    // Bound the request the same way `do_read` does (the slice / pointer
    // arithmetic needs `count <= isize::MAX`).
    if count > isize::MAX as u64 {
        return -EFAULT;
    }

    match target {
        ReadTarget::Synthetic { path } => read_device(&path, buf, count),
        ReadTarget::File { cell_id, offset } => {
            read_file(fd_i32, &cell_id, offset, buf, count)
        }
    }
}

/// Fill the caller's buffer from a `/dev/*` device (#537, #538) named by
/// `path`, per the device's `ReadKind`:
///
///   * `Eof` / `Zeros` / `Random` (`/dev/null`, `/dev/zero`,
///     `/dev/random`) — bytes are table-computable; sourced from
///     `synthetic_fs::device_read` and copied into the buffer.
///   * `Console` (`/dev/tty`, #538) — bytes come from the kernel's
///     primary console input, the same keyboard ring fd 0 drains. We
///     route through `do_read(buf, count, &mut keyboard_source())` —
///     the identical stdin path — so `/dev/tty` and fd 0 deliver byte-
///     for-byte the same input.
///
/// Returns `-EBADF` for a synthetic path that isn't a `/dev/*` device
/// (e.g. a `/proc/*` fd — the non-device synthetic read lands later).
///
/// SAFETY: same identity-mapped buffer write as `do_read` — `buf` is
/// non-null (caller checked) and the fill never exceeds `count` bytes.
fn read_device(path: &str, buf: u64, count: u64) -> i64 {
    // Consult the device table for the read behaviour. `None` means the
    // synthetic path isn't a `/dev/*` device (e.g. a `/proc/*` fd) — out
    // of device-fd scope, return -EBADF.
    let Some(behavior) = synthetic_fs::device_behavior(path) else {
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
    let Some(bytes) = synthetic_fs::device_read(path, count as usize) else {
        return -EBADF;
    };

    copy_to_user(buf, &bytes);
    bytes.len() as i64
}

/// Read up to `count` bytes from the File-cell `cell_id` starting at the
/// fd's current cursor `offset` (#499). Sources bytes through
/// `read_file_cell_bytes` — which reads the same `File_has_ContentRef`
/// predicate + decodes the same `ContentRef` shapes the HTTP
/// `/file/{id}/content` route uses — so a File reads byte-for-byte
/// identically over either surface.
///
/// On a successful read of N bytes the fd's cursor is advanced by N (via
/// `lookup_mut`) so the next read continues where this one stopped; a
/// read at or past EOF returns 0 and leaves the cursor unchanged. The
/// byte sourcing runs against a SYSTEM snapshot taken with `with_state`
/// (its own lock), AFTER the fd-table lock from `read_open_fd` was
/// dropped — the cursor advance re-takes the fd-table lock below.
///
/// Returns:
///   * `>= 0` — bytes delivered (0 = EOF).
///   * `-EBADF` — the File's `File_has_ContentRef` fact has vanished /
///     is malformed, or SYSTEM isn't initialised. The fd is open but no
///     longer points at readable content.
///
/// SAFETY: same identity-mapped buffer write as `do_read` — `buf` is
/// non-null (caller checked) and `bytes.len() <= count` by construction.
fn read_file(fd: i32, cell_id: &str, offset: u64, buf: u64, count: u64) -> i64 {
    // Source the window `[offset, offset + count)` from the File's
    // ContentRef. `with_state` returns `None` when SYSTEM isn't
    // initialised; `read_file_cell_bytes` returns `None` when the
    // content fact is gone / malformed / off-disk-on-host. Either is
    // -EBADF (open fd, no readable content here).
    let bytes = match crate::system::with_state(|state| {
        read_file_cell_bytes(cell_id, offset, count, state)
    }) {
        Some(Some(bytes)) => bytes,
        _ => return -EBADF,
    };

    copy_to_user(buf, &bytes);

    // Advance the read cursor by the number of bytes delivered so the
    // next read continues after them (and a follow-up read at EOF
    // returns 0). Re-take the fd-table lock — it was dropped in
    // `read_open_fd` before this byte sourcing. A zero-byte read (EOF)
    // leaves the cursor where it was. If the fd vanished underneath us
    // (single-threaded tier-1 makes this impossible today), the advance
    // is simply skipped — the bytes were already delivered.
    let advanced = bytes.len() as u64;
    if advanced > 0 {
        current_process_fd_table(|maybe_table| {
            if let Some(table) = maybe_table {
                if let Some(FdEntry::File { offset, .. }) = table.lookup_mut(fd) {
                    *offset = offset.saturating_add(advanced);
                }
            }
        });
    }

    bytes.len() as i64
}

/// Copy `bytes` into the caller's buffer at `buf` via the identity-
/// mapped pointer write (same pattern as `do_read` / `getrandom::
/// fill_userspace`). The caller guarantees `buf` is non-null and that
/// `bytes.len() <= count <= isize::MAX`, so no out-of-bounds write is
/// possible.
///
/// SAFETY: `buf` is non-null (every caller checks) and each write index
/// is `< bytes.len() <= count <= isize::MAX`; tier-1's identity mapping
/// makes the address valid kernel memory.
fn copy_to_user(buf: u64, bytes: &[u8]) {
    for (i, &b) in bytes.iter().enumerate() {
        unsafe {
            core::ptr::write((buf + i as u64) as *mut u8, b);
        }
    }
}

/// Read up to `count` bytes starting at byte `offset` from the File
/// entity `cell_id`, sourced from its `File_has_ContentRef` fact in
/// `state` (#499). This is the cell-driven byte source the general
/// `read(2)` File-fd path pulls through: the SAME `File_has_ContentRef`
/// predicate the HTTP `/file/{id}/content` route reads, so a File
/// reads byte-for-byte identically over either surface.
///
/// Return value:
///   * `Some(bytes)` with `bytes.len() <= count` — the window
///     `[offset, offset + count)` clamped to the file's end. At or past
///     EOF (`offset >= total_len`) this is `Some(empty)`, reported by
///     the handler as 0 bytes (EOF). A zero-byte File yields the same.
///   * `None` — the cell id has no `File_has_ContentRef` fact, the atom
///     is malformed, or it is an off-disk `<REGION,..>` blob on a build
///     without the block device (host tests). The handler maps `None`
///     to `-EBADF` (open fd, no readable content) — distinct from EOF.
///
/// ContentRef shapes (per `readings/os/filesystem.md` + #401): the
/// inline form — bare lowercase hex or `<INLINE,hex>` — is decoded here
/// directly (pure, no block device, so this stays host-testable). The
/// off-disk `<REGION,base,len>` form needs `block_storage`, which is
/// UEFI x86_64 only; there it is delegated to
/// `file_serve::read_region_content`, and on every other target a
/// region blob reads as `None` (the host has no persistence disk).
fn read_file_cell_bytes(
    cell_id: &str,
    offset: u64,
    count: u64,
    state: &Object,
) -> Option<Vec<u8>> {
    let cref = lookup_content_ref(cell_id, state)?;

    // Decode the inline shape (bare hex / <INLINE,hex>) directly. A
    // `<REGION,..>` atom is NOT inline — `decode_inline_content_ref`
    // returns `None` for it, and we fall to the region path below.
    if let Some(inline) = decode_inline_content_ref(&cref) {
        let total_len = inline.len() as u64;
        // At/past EOF or a zero-count probe → empty (a clean EOF), not a
        // miss — the File is valid, just exhausted.
        if count == 0 || offset >= total_len {
            return Some(Vec::new());
        }
        let start = offset as usize;
        // Clamp the high end to the file; `count` bounds the request.
        let want = (offset.saturating_add(count)).min(total_len) as usize;
        return Some(inline[start..want].to_vec());
    }

    // Off-disk region blob. Block storage is UEFI x86_64 only.
    read_region_content_bytes(&cref, offset, count)
}

/// Off-disk `<REGION,base,len>` content for `read_file_cell_bytes`.
/// On UEFI x86_64 (where `block_storage` exists) this delegates to
/// `file_serve::read_region_content` after clamping the window; on
/// every other target there is no persistence disk, so a region blob
/// is unreadable and this returns `None` (→ -EBADF). Split out behind
/// the gate so the inline path — and its tests — compile on the host.
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
fn read_region_content_bytes(cref: &str, offset: u64, count: u64) -> Option<Vec<u8>> {
    // The region's declared length lives in the atom; decode it once to
    // clamp the window and to recognise the immediate-EOF case.
    let total_len = region_byte_len(cref)?;
    if count == 0 || offset >= total_len {
        return Some(Vec::new());
    }
    let start = offset;
    let end = offset.saturating_add(count - 1).min(total_len - 1);
    crate::file_serve::read_region_content(cref, start, end)
}

/// Host / non-x86_64-UEFI stub: no block device, so an off-disk region
/// blob can't be read — surfaces as `-EBADF`. (Inline content never
/// reaches here; it's decoded in `read_file_cell_bytes`.)
#[cfg(not(all(target_os = "uefi", target_arch = "x86_64")))]
fn read_region_content_bytes(_cref: &str, _offset: u64, _count: u64) -> Option<Vec<u8>> {
    None
}

/// Parse the declared byte length out of a `<REGION,base,len>` atom.
/// Used only on the UEFI region path to clamp the read window.
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
fn region_byte_len(cref: &str) -> Option<u64> {
    let inner = cref.strip_prefix('<')?.strip_suffix('>')?;
    let rest = inner.strip_prefix("REGION")?.strip_prefix(',')?;
    let mut parts = rest.split(',');
    let _base = parts.next()?.trim().parse::<u64>().ok()?;
    let len = parts.next()?.trim().parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(len)
}

/// Look up the `File_has_ContentRef` atom for `cell_id` in `state`.
/// Mirrors `file_serve::lookup_content_ref` / `openat::
/// lookup_file_cell_id_in` — the single fact-driven predicate every
/// File-content consumer reads — but lives here (ungated) so the read
/// path resolves content on the host test target as well as on UEFI.
fn lookup_content_ref(cell_id: &str, state: &Object) -> Option<String> {
    let cell = ast::fetch_or_phi("File_has_ContentRef", state);
    cell.as_seq()?.iter().find_map(|fact| {
        if ast::binding(fact, "File") == Some(cell_id) {
            ast::binding(fact, "ContentRef").map(|s| s.into())
        } else {
            None
        }
    })
}

/// Decode the INLINE ContentRef shape into raw bytes. Two forms (per
/// `readings/os/filesystem.md` + #401, matching the engine's
/// `arest::platform::zip` codec):
///   * `<INLINE,deadbeef>` — the tagged inline form.
///   * bare lowercase hex (today's encoder output) — interpreted as
///     inline bytes.
/// An empty atom decodes to an empty buffer (a zero-byte File). Returns
/// `None` for a `<REGION,..>` atom (handled off-disk elsewhere) or for
/// a hex-decode error (odd length / non-hex), which the caller surfaces
/// as -EBADF.
fn decode_inline_content_ref(cref: &str) -> Option<Vec<u8>> {
    // Tagged region form is explicitly NOT inline.
    if cref.starts_with("<REGION") {
        return None;
    }
    if let Some(inner) = cref.strip_prefix("<INLINE,").and_then(|s| s.strip_suffix('>')) {
        return decode_hex(inner);
    }
    // Bare lowercase hex (the current encoder output). An empty atom is
    // a valid zero-byte File.
    decode_hex(cref)
}

/// Decode a lowercase/uppercase hex string into bytes. `None` on an
/// odd-length input or a non-hex digit. Empty input → empty buffer.
fn decode_hex(s: &str) -> Option<Vec<u8>> {
    let bs = s.as_bytes();
    if bs.len() % 2 != 0 {
        return None;
    }
    let mut out: Vec<u8> = Vec::with_capacity(bs.len() / 2);
    let mut i = 0;
    while i + 1 < bs.len() {
        let hi = hex_nibble(bs[i])?;
        let lo = hex_nibble(bs[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Some(out)
}

/// Single hex digit → nibble value. `None` for a non-hex byte.
fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
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

    // ---------------------------------------------------------------
    // General File-cell fd read tests (#499)
    // ---------------------------------------------------------------
    //
    // Two layers of coverage:
    //   * `read_file_cell_bytes` (pure, below) — the offset / clamp /
    //     EOF / decode math against an explicit `Object` fixture, no
    //     global SYSTEM, no process.
    //   * `read::handle` (integration, further below) — the full fd →
    //     fd-table → File-cell dispatch: a `FdEntry::File { cell_id,
    //     offset }` fd reads bytes from `File_has_ContentRef` and the
    //     per-fd cursor advances across calls. These drive the real
    //     `handle` against an installed SYSTEM, so they hold both the
    //     process and SYSTEM test locks.

    use crate::process::fd_table::{file as fd_file, FdEntry as RichFdEntry};
    use crate::system;
    use crate::system::tests::SYSTEM_STATE_TEST_LOCK;
    use arest::ast::{self, fact_from_pairs};

    /// Build an `Object` carrying a single `File_has_ContentRef` fact —
    /// the fixture the pure `read_file_cell_bytes` tests read.
    fn state_with_content(cell_id: &str, cref: &str) -> Object {
        ast::cell_push(
            "File_has_ContentRef",
            fact_from_pairs(&[("File", cell_id), ("ContentRef", cref)]),
            &Object::phi(),
        )
    }

    // ── read_file_cell_bytes — pure offset / clamp / EOF / decode ──

    /// Inline content from offset 0 with a covering count returns the
    /// whole file. `48656c6c6f` = "Hello".
    #[test]
    fn read_file_cell_bytes_inline_from_start() {
        let s = state_with_content("a", "48656c6c6f");
        assert_eq!(read_file_cell_bytes("a", 0, 64, &s), Some(b"Hello".to_vec()));
    }

    /// A `count` smaller than the file truncates to `count` bytes.
    #[test]
    fn read_file_cell_bytes_count_smaller_than_file() {
        let s = state_with_content("a", "48656c6c6f");
        assert_eq!(read_file_cell_bytes("a", 0, 3, &s), Some(b"Hel".to_vec()));
    }

    /// A non-zero offset reads the remainder. "Hello"[2..] = "llo".
    #[test]
    fn read_file_cell_bytes_mid_offset() {
        let s = state_with_content("a", "48656c6c6f");
        assert_eq!(read_file_cell_bytes("a", 2, 64, &s), Some(b"llo".to_vec()));
    }

    /// A window past the end clamps to the file (short read). offset 3,
    /// count 64 on a 5-byte file → "lo".
    #[test]
    fn read_file_cell_bytes_window_past_end_clamps() {
        let s = state_with_content("a", "48656c6c6f");
        assert_eq!(read_file_cell_bytes("a", 3, 64, &s), Some(b"lo".to_vec()));
    }

    /// A read exactly AT the end is an empty (EOF) result — `Some`, not
    /// `None`, because the File is valid (just exhausted).
    #[test]
    fn read_file_cell_bytes_at_eof_is_empty_some() {
        let s = state_with_content("a", "48656c6c6f");
        assert_eq!(read_file_cell_bytes("a", 5, 64, &s), Some(Vec::new()));
    }

    /// A read PAST the end is likewise empty-EOF, not a miss.
    #[test]
    fn read_file_cell_bytes_past_eof_is_empty_some() {
        let s = state_with_content("a", "48656c6c6f");
        assert_eq!(read_file_cell_bytes("a", 100, 64, &s), Some(Vec::new()));
    }

    /// A zero-byte File (empty ContentRef) is immediate EOF.
    #[test]
    fn read_file_cell_bytes_zero_byte_file_is_immediate_eof() {
        let s = state_with_content("z", "");
        assert_eq!(read_file_cell_bytes("z", 0, 64, &s), Some(Vec::new()));
    }

    /// The `<INLINE,hex>` tagged form decodes identically to bare hex.
    #[test]
    fn read_file_cell_bytes_inline_tagged_form() {
        let s = state_with_content("a", "<INLINE,48656c6c6f>");
        assert_eq!(read_file_cell_bytes("a", 0, 64, &s), Some(b"Hello".to_vec()));
    }

    /// A cell id with no `File_has_ContentRef` fact returns `None`
    /// (→ -EBADF), distinct from the empty-EOF case.
    #[test]
    fn read_file_cell_bytes_missing_fact_is_none() {
        let s = state_with_content("a", "48656c6c6f");
        assert_eq!(read_file_cell_bytes("ghost", 0, 64, &s), None);
    }

    /// A malformed (non-hex / odd-length) ContentRef returns `None`.
    #[test]
    fn read_file_cell_bytes_malformed_is_none() {
        let s = state_with_content("a", "xyz");
        assert_eq!(read_file_cell_bytes("a", 0, 64, &s), None);
    }

    /// On the host test target there is no block device, so an off-disk
    /// `<REGION,..>` blob is unreadable → `None` (→ -EBADF). (On UEFI
    /// x86_64 the region path delegates to block storage instead.)
    #[test]
    fn read_file_cell_bytes_region_unreadable_on_host_is_none() {
        let s = state_with_content("a", "<REGION,8192,131072>");
        assert_eq!(read_file_cell_bytes("a", 0, 64, &s), None);
    }

    /// Walking inline content in `count`-sized chunks by advancing the
    /// offset reassembles it and then hits the empty-EOF result — the
    /// exact loop the handler drives across sequential `read(2)` calls.
    #[test]
    fn read_file_cell_bytes_sequential_chunks_reassemble_then_eof() {
        // "Hello, world!" = 13 bytes.
        let s = state_with_content("a", "48656c6c6f2c20776f726c6421");
        let mut assembled: Vec<u8> = Vec::new();
        let mut offset: u64 = 0;
        loop {
            let chunk = read_file_cell_bytes("a", offset, 4, &s).expect("resolves");
            if chunk.is_empty() {
                break;
            }
            offset += chunk.len() as u64;
            assembled.extend_from_slice(&chunk);
        }
        assert_eq!(assembled, b"Hello, world!");
        assert_eq!(offset, 13);
    }

    /// Stage a `File_has_ContentRef` fact for `cell_id` onto the live
    /// SYSTEM state (on top of whatever `init()` baked, so the validate
    /// gate stays satisfied). Caller must hold `SYSTEM_STATE_TEST_LOCK`.
    fn stage_file_content(cell_id: &str, cref: &str) {
        system::init();
        let base = system::with_state(|s| s.clone()).expect("init ran");
        let next = ast::cell_push(
            "File_has_ContentRef",
            fact_from_pairs(&[("File", cell_id), ("ContentRef", cref)]),
            &base,
        );
        system::apply(next).expect("apply File facts");
    }

    /// Install a fresh Process and allocate a File-cell fd against
    /// `cell_id`. Returns the fd. Caller must hold
    /// `CURRENT_PROCESS_TEST_LOCK`.
    fn install_with_file_fd(cell_id: &str) -> i32 {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(7, address_space);
        current_process_install(proc);
        current_process_fd_table(|t| {
            t.expect("process installed")
                .allocate(fd_file(cell_id))
                .expect("allocate file fd")
        })
    }

    /// `read` of a File fd returns the File's content bytes (sourced
    /// from `File_has_ContentRef`). `48656c6c6f` = "Hello".
    #[test]
    fn read_file_fd_returns_content_bytes() {
        let _sys = SYSTEM_STATE_TEST_LOCK.lock();
        let _proc = CURRENT_PROCESS_TEST_LOCK.lock();
        stage_file_content("file-hello", "48656c6c6f");
        let fd = install_with_file_fd("file-hello");

        let mut buf = [0u8; 16];
        let n = handle(fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
        assert_eq!(n, 5, "read must return the 5 content bytes");
        assert_eq!(&buf[..5], b"Hello");

        current_process_uninstall();
    }

    /// A second `read` after the first consumed the whole file returns
    /// 0 (EOF) — the per-fd cursor advanced past the end. This is the
    /// "0 on EOF" half of the #499 contract, verified across two
    /// sequential `handle` calls on the same fd.
    #[test]
    fn read_file_fd_second_read_hits_eof_returns_zero() {
        let _sys = SYSTEM_STATE_TEST_LOCK.lock();
        let _proc = CURRENT_PROCESS_TEST_LOCK.lock();
        stage_file_content("file-eof", "48656c6c6f"); // "Hello"
        let fd = install_with_file_fd("file-eof");

        let mut buf = [0u8; 16];
        // First read drains the whole 5-byte file.
        let first = handle(fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
        assert_eq!(first, 5);
        // Second read starts at the advanced cursor (== len) → EOF.
        let second = handle(fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
        assert_eq!(second, 0, "read at EOF must return 0");

        current_process_uninstall();
    }

    /// Sequential short reads walk the file: each `read` advances the
    /// cursor, so reading in 3-byte chunks reassembles the content and
    /// then hits EOF. Confirms the cursor advance persists in the fd
    /// table across `handle` calls.
    #[test]
    fn read_file_fd_sequential_short_reads_walk_and_advance_cursor() {
        let _sys = SYSTEM_STATE_TEST_LOCK.lock();
        let _proc = CURRENT_PROCESS_TEST_LOCK.lock();
        // "Hello, world!" = 13 bytes.
        stage_file_content("file-walk", "48656c6c6f2c20776f726c6421");
        let fd = install_with_file_fd("file-walk");

        let mut assembled: Vec<u8> = Vec::new();
        loop {
            let mut buf = [0u8; 3];
            let n = handle(fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
            assert!(n >= 0, "read must not error mid-walk, got {}", n);
            if n == 0 {
                break; // EOF
            }
            assembled.extend_from_slice(&buf[..n as usize]);
        }
        assert_eq!(assembled, b"Hello, world!");

        // The fd's cursor must now sit at end-of-file (13).
        let cursor = current_process_fd_table(|t| {
            t.and_then(|t| match t.lookup(fd) {
                Some(RichFdEntry::File { offset, .. }) => Some(*offset),
                _ => None,
            })
        });
        assert_eq!(cursor, Some(13), "cursor must rest at EOF after the walk");

        current_process_uninstall();
    }

    /// `read` of a File fd whose `File_has_ContentRef` fact is absent
    /// (e.g. the backing content was retracted) returns `-EBADF` — the
    /// fd is open but no longer points at readable content. Distinct
    /// from a clean EOF (which returns 0).
    #[test]
    fn read_file_fd_missing_content_returns_ebadf() {
        let _sys = SYSTEM_STATE_TEST_LOCK.lock();
        let _proc = CURRENT_PROCESS_TEST_LOCK.lock();
        // Init SYSTEM but stage NO ContentRef for this cell id.
        system::init();
        // Allocate a File fd whose cell id has no content fact.
        let fd = install_with_file_fd("file-ghost-no-content");

        let mut buf = [0u8; 16];
        let n = handle(fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
        assert_eq!(n, -EBADF, "File fd with no ContentRef must be -EBADF");

        current_process_uninstall();
    }

    /// `read(file_fd, NULL, n>0)` returns `-EFAULT` before any byte
    /// sourcing — the null-buffer guard fires regardless of fd kind.
    #[test]
    fn read_file_fd_null_buf_returns_efault() {
        let _sys = SYSTEM_STATE_TEST_LOCK.lock();
        let _proc = CURRENT_PROCESS_TEST_LOCK.lock();
        stage_file_content("file-null", "48656c6c6f");
        let fd = install_with_file_fd("file-null");

        let n = handle(fd as u64, 0, 16);
        assert_eq!(n, -EFAULT);

        current_process_uninstall();
    }
}
