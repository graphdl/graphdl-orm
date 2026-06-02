// crates/arest-kernel/src/synthetic_fs/dev.rs
//
// `/dev/*` special-device resolver (#537, #538, #475c — the third
// subtree of the synthetic-fs epic after `/proc/*` (#534/#535) and ahead
// of the `/sys/*` track). Models the POSIX character devices every libc
// + shell assumes exist:
//
//   * `/dev/null`   — the bit bucket. Reads return EOF (0 bytes);
//                     writes are discarded but report the full count.
//   * `/dev/zero`   — an endless source of zero bytes. Reads fill the
//                     caller's buffer with `0x00`; writes are discarded.
//   * `/dev/random` — the kernel entropy source. Reads fill the caller's
//                     buffer with CSPRNG bytes (`arest::csprng`, the same
//                     ChaCha20 stream `getrandom(2)` draws from); writes
//                     are rejected (the device is read-only).
//   * `/dev/tty`    — the controlling terminal, bound to the kernel's
//                     primary console (#538). Reads source the same
//                     console input fd 0 drains (the keyboard ring);
//                     writes go to the same console output fd 1/2 drive
//                     (UEFI ConOut pre-EBS + the UART serial console,
//                     via `crate::print!`). Unlike the three #537
//                     devices, `/dev/tty`'s bytes are NOT computable from
//                     the path alone — its read source and write sink are
//                     live kernel console state — so the table tags it
//                     with `ReadKind::Console` / `WriteKind::Console`
//                     *behaviour markers* and the read/write syscall
//                     handlers translate those markers to the existing
//                     console abstractions (the same `keyboard_source` /
//                     `print!` sink the std streams use). The table stays
//                     the single source of truth for *which* device has
//                     console semantics; the handlers own the side effect.
//
// Why a data-driven device table rather than per-path branches
// ------------------------------------------------------------
// The AREST loop mandate is "prefer predicate readings, remove
// procedural code". A naive implementation would scatter
// `if path == "/dev/null" { … } else if path == "/dev/zero" { … }`
// branches across the read handler, the write handler, and the openat
// access-mode check — three copies of the same path-discrimination,
// drifting out of sync as devices are added. Instead every device is
// one row in `DEVICES`, a `&[DeviceSpec]` table, and the syscall
// handlers consult a single predicate (`lookup`) that returns the
// device's *behaviour* (how reads fill, whether writes are allowed).
// Adding a device whose bytes are computable from the path (`/dev/full`
// → -ENOSPC on write) is a pure one-row table edit. `/dev/tty` (#538)
// added one row plus one new behaviour-marker per direction
// (`ReadKind::Console` / `WriteKind::Console`) because its bytes come
// from live console state, not the path — the handler gains a single
// match arm per direction that routes the marker to the console; the
// path-discrimination still lives only in the table.
//
// Relationship to `synthetic_fs::resolve`
// ---------------------------------------
// `resolve(path)` (the snapshot resolver feeding `openat`'s path-exists
// check and the HTTP `file_serve` fallback) returns a finite `Vec<u8>`.
// That shape is the right one for path-existence ("does `/dev/zero`
// resolve? yes") and for the HTTP read of a device (a GET on
// `/dev/zero` can't stream forever, so it returns a bounded preview).
// But the *streaming* fd semantics — endless zeros, fresh entropy per
// read, EOF on `/dev/null` — are a different shape that a `Vec<u8>`
// snapshot can't carry. So `resolve` delegates path-existence to this
// module's `is_device` + `snapshot`, while the `read`/`write` syscall
// handlers consult `lookup` for the per-fd streaming behaviour. The two
// surfaces share one source of truth (the `DEVICES` table) so they can
// never disagree about which paths are devices.

use alloc::vec;
use alloc::vec::Vec;

/// How a device fills a `read(fd, buf, count)` request. The read handler
/// matches on this to source bytes for a device-backed fd; no procedural
/// per-path branch is needed at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadKind {
    /// Reads return end-of-file immediately — 0 bytes, no data. This is
    /// `/dev/null`'s read semantics: a reader sees the stream as already
    /// exhausted.
    Eof,
    /// Reads return an endless stream of zero bytes — the handler fills
    /// the whole requested `count` with `0x00`. This is `/dev/zero`.
    Zeros,
    /// Reads return cryptographically secure random bytes from the
    /// kernel CSPRNG (`arest::csprng::random_bytes`). The handler fills
    /// the whole requested `count` with fresh entropy. This is
    /// `/dev/random`. (#577 later distinguishes the blocking-pool
    /// semantics; #537 wires both `random` and a future `urandom` to the
    /// single available CSPRNG stream.)
    Random,
    /// Reads source the kernel's primary console input — the same
    /// keyboard ring / stdin source fd 0 drains (`arch::uefi::keyboard::
    /// read_keystroke`). This is `/dev/tty` (#538). Unlike the other
    /// variants, the bytes are NOT computed from the path: the read
    /// handler routes this marker to the same `keyboard_source` closure
    /// it uses for fd 0, so `read::device_read` cannot satisfy it from
    /// the table alone (it returns `None` for `Console`, signalling the
    /// handler to take the console path). See `read::handle`.
    Console,
}

/// Whether a device accepts `write(fd, buf, count)`. Kept as its own
/// enum (rather than a bare `bool`) so a future `/dev/full` (writes
/// fail with -ENOSPC) plugs in as a third variant without reshaping the
/// table or the write handler's match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteKind {
    /// Writes are silently discarded; the syscall reports the full byte
    /// count as written (the data goes nowhere). This is the
    /// `/dev/null` + `/dev/zero` write semantics.
    Discard,
    /// Writes are rejected — the device is read-only. The write handler
    /// maps this to `-EBADF` (the fd was opened read-only; Linux returns
    /// EBADF for a write to an O_RDONLY fd). This is `/dev/random`.
    Reject,
    /// Writes go to the kernel's primary console output — the same
    /// UEFI ConOut + UART serial console fd 1/2 drive (`crate::print!` →
    /// `arch::_print`). This is `/dev/tty` (#538). The write handler
    /// routes this marker through the same `do_write` console sink the
    /// std streams use, then reports the full byte count (the console
    /// never short-writes in tier-1). Like `ReadKind::Console`, the
    /// bytes' destination is live console state, so the behaviour is a
    /// marker the handler interprets rather than a discard the table can
    /// perform itself. A device tagged `Console` for writes also accepts
    /// an `O_WRONLY` / `O_RDWR` open (`openat`'s `device_accepts_write`
    /// treats it like `Discard`). See `write::handle`.
    Console,
}

/// The behaviour of one special device — how reads fill, how writes are
/// handled. Returned by `lookup` so the syscall handlers can drive their
/// byte-level semantics off a value rather than a path string compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceBehavior {
    pub read: ReadKind,
    pub write: WriteKind,
}

/// One row in the device table — the absolute path plus its behaviour.
/// The whole `/dev/*` surface is the `DEVICES` slice of these; adding a
/// device is appending a row.
#[derive(Debug, Clone, Copy)]
struct DeviceSpec {
    path: &'static str,
    behavior: DeviceBehavior,
}

/// The complete `/dev/*` device table. Single source of truth: both the
/// `resolve` snapshot path (existence + HTTP preview) and the
/// `read`/`write` syscall handlers (streaming behaviour) derive from
/// this slice, so they can never disagree about which paths are devices
/// or what each one does.
///
/// Ordered for readability (null, zero, random, tty); `lookup` is a
/// linear scan, which is fine for a small table consulted once per
/// `open`/`read`/`write` of a device fd (a low-rate event relative to
/// the byte fill the result drives).
const DEVICES: &[DeviceSpec] = &[
    DeviceSpec {
        path: "/dev/null",
        behavior: DeviceBehavior {
            read: ReadKind::Eof,
            write: WriteKind::Discard,
        },
    },
    DeviceSpec {
        path: "/dev/zero",
        behavior: DeviceBehavior {
            read: ReadKind::Zeros,
            write: WriteKind::Discard,
        },
    },
    DeviceSpec {
        path: "/dev/random",
        behavior: DeviceBehavior {
            read: ReadKind::Random,
            write: WriteKind::Reject,
        },
    },
    DeviceSpec {
        // The controlling terminal, bound to the kernel's primary
        // console (#538). Read sources console input (keyboard ring /
        // fd 0 stdin); write targets console output (UEFI ConOut + UART
        // serial, the fd 1/2 sink). Both directions are `Console`
        // behaviour markers the syscall handlers translate to the live
        // console abstractions — the table can't produce the bytes
        // itself (see `ReadKind::Console` / `WriteKind::Console`).
        path: "/dev/tty",
        behavior: DeviceBehavior {
            read: ReadKind::Console,
            write: WriteKind::Console,
        },
    },
];

/// Look up the behaviour of the device at `path`. Returns `Some` for a
/// modelled `/dev/*` device, `None` otherwise (the caller falls through
/// to the next resolver / returns -ENOENT). This is the single predicate
/// the syscall handlers consult — no path-string branching at the call
/// site.
pub fn lookup(path: &str) -> Option<DeviceBehavior> {
    DEVICES
        .iter()
        .find(|d| d.path == path)
        .map(|d| d.behavior)
}

/// True when `path` names a modelled `/dev/*` device. Convenience over
/// `lookup(path).is_some()` for the existence-only callers (the openat
/// access-mode check, the HTTP resolver). Reads as intent at the call
/// site.
pub fn is_device(path: &str) -> bool {
    lookup(path).is_some()
}

/// Stable list of the modelled `/dev/*` paths. Mirrors `proc::PATHS` —
/// used by `synthetic_fs::resolve` enumeration and a future `readdir`
/// over `/dev`.
pub const PATHS: &[&str] = &["/dev/null", "/dev/zero", "/dev/random", "/dev/tty"];

/// Render the bytes a snapshot read of `path` returns — the finite
/// `Vec<u8>` shape `synthetic_fs::resolve` (and through it `openat`'s
/// existence check + the HTTP `file_serve` fallback) needs.
///
/// This is NOT the streaming fd read — that's `device_read`. The
/// snapshot is the bounded view a one-shot reader (an HTTP GET that
/// can't stream forever) sees:
///
///   * `/dev/null`   → empty (EOF — a reader sees no bytes).
///   * `/dev/zero`   → empty. An endless device has no finite snapshot;
///                     returning empty keeps the HTTP `Content-Length`
///                     honest (a GET on `/dev/zero` over HTTP returns 0
///                     bytes rather than hanging). The real zero stream
///                     is delivered through the fd `read` path.
///   * `/dev/random` → empty for the same reason — entropy is a stream,
///                     delivered per-`read`, not a cacheable snapshot.
///   * `/dev/tty`    → empty. The terminal's input is live console
///                     state (the keyboard ring), not a cacheable
///                     snapshot — a one-shot HTTP GET can't drain the
///                     console, so it serves 0 bytes; the real input is
///                     delivered per-`read` from the fd path.
///
/// Every device renders empty: the snapshot's job is purely "does this
/// path resolve?" (a `Some` answer), while the byte semantics live in
/// `device_read` / the write handler. Returning `Some(empty)` lets
/// `openat` allocate an fd and the HTTP path serve a 0-length 200.
pub fn snapshot(path: &str) -> Option<Vec<u8>> {
    if is_device(path) {
        // Every device's snapshot is empty — see the doc comment. The
        // streaming bytes come from `device_read` on the fd path.
        Some(Vec::new())
    } else {
        None
    }
}

/// Fill up to `capacity` bytes for a streaming `read(fd, …)` of the
/// device at `path`, per its `ReadKind`. Returns the bytes to copy into
/// the caller's buffer (the read handler does the userspace copy). The
/// returned `Vec` length is the number of bytes the `read` reports:
///
///   * `Eof`    → `Some(empty)` (read returns 0 — end of file).
///   * `Zeros`  → `Some(capacity zero bytes)` (fills the whole buffer).
///   * `Random` → `Some(capacity CSPRNG bytes)` (fills the whole buffer
///                with fresh entropy from `arest::csprng`).
///   * `Console`→ `None`. The terminal's input is live console state
///                (the keyboard ring), not bytes this table can
///                synthesise from the path; the read handler detects the
///                `Console` marker via `device_behavior` and routes the
///                read to the same `keyboard_source` it uses for fd 0,
///                so it never relies on this function for `/dev/tty`.
///                Returning `None` keeps this function honest: it only
///                yields bytes for the table-computable devices.
///
/// Returns `None` when `path` is not a modelled device (the read handler
/// maps that to `-EBADF`) and, as above, for the `Console` device
/// (whose bytes are sourced elsewhere). Callers that need to distinguish
/// the two — "not a device" vs "console device, fill from the ring" —
/// consult `device_behavior(path)` first; the read handler does exactly
/// that before calling here.
///
/// `capacity` is the `count` the userspace `read` requested (already
/// bounded by the handler). For `Zeros` / `Random` the device is an
/// endless stream so it always satisfies the full request; there is no
/// short read and no per-fd offset to track (each read is independent).
pub fn device_read(path: &str, capacity: usize) -> Option<Vec<u8>> {
    let behavior = lookup(path)?;
    let bytes = match behavior.read {
        ReadKind::Eof => Vec::new(),
        ReadKind::Zeros => vec![0u8; capacity],
        ReadKind::Random => {
            let mut buf = vec![0u8; capacity];
            // Draw from the kernel-wide CSPRNG — the same ChaCha20 stream
            // `getrandom(2)` uses (`getrandom::handle` → `csprng::
            // random_bytes`). #577 later hardens `/dev/random` with the
            // blocking-pool distinction; #537 wires it to the single
            // available stream.
            arest::csprng::random_bytes(buf.as_mut_slice());
            buf
        }
        // `/dev/tty` reads come from the live keyboard ring, not the
        // table. The read handler takes the console path on this marker
        // and never calls here for it; returning `None` makes a stray
        // call surface as -EBADF rather than a silent empty read.
        ReadKind::Console => return None,
    };
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table models the three #537 devices plus `/dev/tty` (#538).
    /// `PATHS` and `DEVICES` stay in lockstep (same count, same paths).
    #[test]
    fn paths_lists_the_modelled_devices() {
        assert_eq!(PATHS, &["/dev/null", "/dev/zero", "/dev/random", "/dev/tty"]);
        assert_eq!(DEVICES.len(), 4);
        // Every PATHS entry resolves through the table and vice versa —
        // the two surfaces can't drift.
        assert_eq!(DEVICES.len(), PATHS.len());
        for p in PATHS {
            assert!(is_device(p), "{} in PATHS must resolve in DEVICES", p);
        }
    }

    /// `lookup` returns `/dev/null`'s behaviour: EOF reads, discarded
    /// writes.
    #[test]
    fn lookup_null_is_eof_read_discard_write() {
        let b = lookup("/dev/null").expect("null is a device");
        assert_eq!(b.read, ReadKind::Eof);
        assert_eq!(b.write, WriteKind::Discard);
    }

    /// `lookup` returns `/dev/zero`'s behaviour: zero-stream reads,
    /// discarded writes.
    #[test]
    fn lookup_zero_is_zeros_read_discard_write() {
        let b = lookup("/dev/zero").expect("zero is a device");
        assert_eq!(b.read, ReadKind::Zeros);
        assert_eq!(b.write, WriteKind::Discard);
    }

    /// `lookup` returns `/dev/random`'s behaviour: random reads,
    /// rejected writes (read-only device).
    #[test]
    fn lookup_random_is_random_read_reject_write() {
        let b = lookup("/dev/random").expect("random is a device");
        assert_eq!(b.read, ReadKind::Random);
        assert_eq!(b.write, WriteKind::Reject);
    }

    /// `lookup` returns `/dev/tty`'s behaviour: console reads (sourced
    /// from the keyboard ring by the read handler) and console writes
    /// (routed to ConOut + serial by the write handler). The table tags
    /// both directions `Console`; the byte semantics live in the
    /// handlers (#538).
    #[test]
    fn lookup_tty_is_console_read_console_write() {
        let b = lookup("/dev/tty").expect("tty is a device");
        assert_eq!(b.read, ReadKind::Console);
        assert_eq!(b.write, WriteKind::Console);
    }

    /// `lookup` / `is_device` return `None` / false for non-device
    /// paths — including `/dev/` paths we don't model and non-`/dev`
    /// paths entirely.
    #[test]
    fn lookup_unknown_path_returns_none() {
        assert!(lookup("/dev/sda").is_none());
        assert!(lookup("/dev").is_none());
        assert!(lookup("/proc/cpuinfo").is_none());
        assert!(lookup("").is_none());
        // `/dev/tty` IS a device as of #538 — its non-membership in the
        // negative set is what changed.
        assert!(is_device("/dev/tty"));
        assert!(!is_device("/dev/ttyS0"));
        assert!(!is_device("/etc/passwd"));
    }

    /// `snapshot` returns `Some(empty)` for every device (existence
    /// signal for `openat` / HTTP) and `None` for non-devices.
    #[test]
    fn snapshot_is_empty_for_devices_none_otherwise() {
        for p in PATHS {
            let snap = snapshot(p).expect("device has a snapshot");
            assert!(snap.is_empty(), "{} snapshot must be empty", p);
        }
        assert!(snapshot("/dev/sda").is_none());
        assert!(snapshot("/proc/meminfo").is_none());
    }

    /// `device_read` on `/dev/null` returns 0 bytes (EOF) regardless of
    /// the requested capacity.
    #[test]
    fn device_read_null_returns_eof() {
        let bytes = device_read("/dev/null", 4096).expect("null reads");
        assert!(bytes.is_empty(), "/dev/null read must return EOF (0 bytes)");
        // EOF holds for any capacity, including 0.
        assert!(device_read("/dev/null", 0).unwrap().is_empty());
    }

    /// `device_read` on `/dev/zero` fills the whole requested capacity
    /// with zero bytes.
    #[test]
    fn device_read_zero_fills_with_zeros() {
        let bytes = device_read("/dev/zero", 64).expect("zero reads");
        assert_eq!(bytes.len(), 64, "/dev/zero must fill the whole request");
        assert!(bytes.iter().all(|&b| b == 0), "every byte must be 0x00");
        // A zero-capacity read yields an empty buffer (no bytes requested).
        assert!(device_read("/dev/zero", 0).unwrap().is_empty());
    }

    /// `device_read` on an unknown path returns `None` — the read
    /// handler maps that to -EBADF.
    #[test]
    fn device_read_unknown_path_returns_none() {
        assert!(device_read("/dev/sda", 16).is_none());
        assert!(device_read("/proc/cpuinfo", 16).is_none());
    }

    /// `device_read` on `/dev/tty` returns `None` even though it IS a
    /// device — the `Console` read source is the live keyboard ring, not
    /// the table. The read handler detects the `Console` behaviour and
    /// routes to the keyboard source instead of relying on this
    /// function. The `None` here is "not table-computable", distinct in
    /// meaning from the unknown-path `None` (the handler disambiguates
    /// via `lookup`/`device_behavior`). Holds for any capacity.
    #[test]
    fn device_read_tty_returns_none_console_sourced_elsewhere() {
        assert!(
            device_read("/dev/tty", 16).is_none(),
            "/dev/tty bytes come from the keyboard ring, not device_read"
        );
        assert!(device_read("/dev/tty", 0).is_none());
        // But it IS a device — distinct from the unknown-path case above.
        assert!(is_device("/dev/tty"));
        assert_eq!(lookup("/dev/tty").map(|b| b.read), Some(ReadKind::Console));
    }

    // NOTE: `/dev/random`'s `device_read` exercises `arest::csprng::
    // random_bytes`, which panics if no entropy source is installed.
    // The handler-level test (`read::tests::read_dev_random_fd_fills_
    // with_random_bytes`) installs a deterministic source and pins the
    // three #577 properties: the read returns the full requested count,
    // the bytes equal the kernel ChaCha20 CSPRNG keystream byte-for-byte
    // (provenance — not a weak/zero source), and two successive reads
    // differ (non-repeating). The pure-table tests here stay entropy-free
    // so they need no entropy fixture; the randomness contract is proven
    // once, at the handler seam that drives the real syscall path.
}
