// crates/arest-kernel/src/synthetic_fs/dev.rs
//
// `/dev/*` special-device resolver (#537, #475c — the third subtree of
// the synthetic-fs epic after `/proc/*` (#534/#535) and ahead of the
// `/sys/*` track). Models the three POSIX character devices every libc
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
// Adding a future device (`/dev/full` → -ENOSPC on write, `/dev/tty`
// → console) is a one-row table edit; the handlers don't change.
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
/// Ordered for readability (null, zero, random); `lookup` is a linear
/// scan, which is fine for a three-row table consulted once per
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
pub const PATHS: &[&str] = &["/dev/null", "/dev/zero", "/dev/random"];

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
///
/// All three render empty: the snapshot's job is purely "does this path
/// resolve?" (a `Some` answer), while the byte semantics live in
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
///   * `Eof`    → empty `Vec` (read returns 0 — end of file).
///   * `Zeros`  → `capacity` zero bytes (read fills the whole buffer).
///   * `Random` → `capacity` CSPRNG bytes (read fills the whole buffer
///                with fresh entropy from `arest::csprng`).
///
/// Returns `None` when `path` is not a modelled device — the read
/// handler maps that to `-EBADF` (the fd's path didn't resolve to a
/// readable device).
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
    };
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table models exactly the three #537 devices.
    #[test]
    fn paths_lists_the_three_devices() {
        assert_eq!(PATHS, &["/dev/null", "/dev/zero", "/dev/random"]);
        assert_eq!(DEVICES.len(), 3);
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

    /// `lookup` / `is_device` return `None` / false for non-device
    /// paths — including `/dev/` paths we don't model and non-`/dev`
    /// paths entirely.
    #[test]
    fn lookup_unknown_path_returns_none() {
        assert!(lookup("/dev/sda").is_none());
        assert!(lookup("/dev").is_none());
        assert!(lookup("/proc/cpuinfo").is_none());
        assert!(lookup("").is_none());
        assert!(!is_device("/dev/tty"));
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

    // NOTE: `/dev/random`'s `device_read` exercises `arest::csprng::
    // random_bytes`, which panics if no entropy source is installed.
    // The handler-level test (`read::tests`) installs a deterministic
    // source before reading `/dev/random`; the pure-table tests here
    // stay entropy-free so they need no entropy fixture.
}
