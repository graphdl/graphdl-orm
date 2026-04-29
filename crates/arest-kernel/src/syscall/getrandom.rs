// crates/arest-kernel/src/syscall/getrandom.rs
//
// Linux x86_64 syscall 318: `getrandom(void *buf, size_t buflen,
// unsigned int flags)`. Per #576 (Track Rand-C2), fourth in the
// userspace-syscall epic #473 after write/exit (#507), openat/close
// (#498), futex (#544). Fills `buf` with up to `buflen` cryptographically
// secure random bytes from the kernel-wide ChaCha20 CSPRNG seeded at
// boot from the installed `entropy::EntropySource` (UEFI RDSEED/RDRAND
// per #569 / #571 in the kernel arm; `cli::entropy_host::HostEntropy
// Source` per #574 in the host CLI arm).
//
// Why a kernel-side scratch buffer
// --------------------------------
// `csprng::random_bytes` takes a `&mut [u8]` — Rust safety requires the
// slice be backed by a valid, exclusive allocation for its lifetime.
// Userspace addresses (under tier-1's identity mapping) coincide with
// kernel addresses, so we *could* hand the userspace pointer in
// directly; but doing so trades the page-fault safety net for a single
// memcpy, and once #527 lands real page tables we'd need a full
// rewrite. Instead we fill a kernel scratch buffer (heap-allocated up
// to the cap) and copy through to userspace via the same identity-map
// pattern `write::do_write` documents — a `core::ptr::copy_nonoverlapping`
// at the end. Once #561 (`copy_to_user`) lands, the final copy switches
// to the validated userspace-write helper and the rest of the handler
// stays unchanged.
//
// Length cap (1 MiB)
// ------------------
// Linux's `getrandom(2)` itself caps at 32 MiB on the kernel side; we
// pick a more conservative 1 MiB ceiling because:
//
//   * The kernel scratch buffer is heap-allocated per call (talc
//     allocator, post-EBS); a 32 MiB transient allocation is a stress
//     test we don't need on the boot path.
//   * libc consumers (musl `getrandom_init` for the stack canary,
//     glibc's `__getrandom_nocancel`) ask for ≤ 256 bytes per call.
//     Anything beyond a page is already a cold-path consumer.
//
// Per POSIX the syscall is allowed to short-read: if userspace asks
// for more than the cap, we fill the cap and return the cap. The libc
// loop `while (filled < want) filled += getrandom(buf+filled, want-
// filled, 0);` handles short reads transparently — same shape musl's
// `__random.c` and glibc's `getentropy.c` use.
//
// Flags handling
// --------------
// Linux defines:
//   * `GRND_NONBLOCK = 0x0001` — return -EAGAIN instead of blocking.
//   * `GRND_RANDOM   = 0x0002` — draw from the blocking /dev/random pool.
//   * `GRND_INSECURE = 0x0004` — return possibly-unseeded bytes.
//
// AREST has one CSPRNG stream; the kernel has no blocking pool, no
// distinction between "random" and "urandom". We accept any flags
// value as a no-op — the entropy source is the same regardless. This
// matches Linux's behaviour from userspace's perspective when the
// random pool is already initialised (the common case after early
// boot): `getrandom(..., 0)` and `getrandom(..., GRND_NONBLOCK)`
// produce identical bytes from identical state.
//
// Pointer dereferencing — tier-1 identity-mapping note
// ----------------------------------------------------
// Same caveat as `write::do_write` and `openat::handle`: under UEFI's
// identity mapping, userspace VAs coincide with kernel VAs, so we
// treat `buf_addr` as a kernel pointer for the final copy. Once #527
// lands real page tables, the copy routes through `copy_to_user` (#561
// — pending). The scratch buffer + per-call CSPRNG fill are
// arch-neutral and won't change.
//
// Edge cases:
//   * `buflen == 0` → returns 0 immediately; pointer is not
//     dereferenced (so a null `buf_addr` with `buflen == 0` is fine).
//   * `buf_addr == 0 && buflen > 0` → returns `-EFAULT`.
//   * `buflen > GETRANDOM_MAX` → cap to `GETRANDOM_MAX`, fill that
//     many bytes, return `GETRANDOM_MAX` (POSIX-conformant short read).

use alloc::vec;

use crate::syscall::dispatch::EFAULT;

/// Maximum number of bytes a single `getrandom` call will fill. See
/// the module-level rationale; values above this are short-read down
/// to this number rather than rejected, matching the POSIX-style
/// short-read contract libc consumers already implement against.
pub const GETRANDOM_MAX: u64 = 1 << 20; // 1 MiB

/// Handle a `getrandom(buf, buflen, flags)` syscall. Fills the userspace
/// buffer with up to `buflen` CSPRNG bytes (capped at `GETRANDOM_MAX`)
/// and returns the number of bytes actually written, or a negative
/// `errno` on failure.
///
/// Edge cases:
///   * `buflen == 0` → returns 0 without dereferencing `buf_addr`
///     (POSIX behaviour; a null pointer with zero length is legal).
///   * `buf_addr == 0 && buflen > 0` → returns `-EFAULT`.
///   * `buflen > GETRANDOM_MAX` → caps at `GETRANDOM_MAX`, returns the
///     cap. Userspace libc loops on short reads.
///
/// `flags` is accepted but ignored — AREST's single CSPRNG stream
/// doesn't distinguish blocking vs non-blocking vs /dev/random.
///
/// SAFETY: callers (the syscall dispatcher) treat `buf_addr` as a
/// userspace virtual address. Under tier-1's identity mapping (UEFI
/// firmware + no page-table install yet) it doubles as a kernel
/// pointer; the handler writes to it directly via
/// `core::ptr::copy_nonoverlapping`. Once #527 lands real page tables,
/// the final copy routes through `copy_to_user` (#561 — pending).
pub fn handle(buf_addr: u64, buflen: u64, _flags: u32) -> i64 {
    // Fast-path: zero-length getrandom is a no-op return-0 per POSIX.
    // Doing this before the buf-null check matches `write`'s pattern
    // and lets a userspace probe `getrandom(NULL, 0, 0)` succeed
    // cleanly.
    if buflen == 0 {
        return 0;
    }
    if buf_addr == 0 {
        return -EFAULT;
    }
    // POSIX short-read cap. Anything above 1 MiB fills exactly 1 MiB
    // and returns 1 MiB; libc's getentropy loop handles the rest.
    let want = if buflen > GETRANDOM_MAX {
        GETRANDOM_MAX
    } else {
        buflen
    };
    fill_userspace(buf_addr, want as usize, &mut |scratch| {
        arest::csprng::random_bytes(scratch);
    })
}

/// Shared work for the getrandom handler — separated from `handle` so
/// the unit tests can inject a deterministic fill function without
/// touching the global CSPRNG. Production routes `arest::csprng::
/// random_bytes` through this shim; tests can pass a closure that
/// fills the scratch with a fixed pattern (or with the real CSPRNG
/// after installing a `DeterministicSource`).
///
/// `count` is `usize` here because `Vec::resize` and the slice copy
/// take usize; the caller is responsible for the cap (see `handle`'s
/// `GETRANDOM_MAX` short-read).
///
/// Returns `count as i64` on success — never short-fills below the
/// caller-requested count, since the CSPRNG itself is infallible
/// once seeded. Returns `-EFAULT` if `count` exceeds `isize::MAX`
/// (which would overflow the slice length the copy expects); the
/// dispatcher's `GETRANDOM_MAX` cap (1 MiB) makes this unreachable
/// from production callers, but the guard keeps the function honest
/// against a future cap relaxation.
///
/// SAFETY: writes `count` bytes to `buf_addr`. Caller is responsible
/// for the validity of the range — the handler's `if buf_addr == 0`
/// check guards null, but a non-null but unmapped pointer would
/// page-fault at the copy. Same identity-mapping rationale as
/// `write::do_write`.
pub fn fill_userspace(
    buf_addr: u64,
    count: usize,
    fill: &mut dyn FnMut(&mut [u8]),
) -> i64 {
    if count > isize::MAX as usize {
        return -EFAULT;
    }
    // Allocate the scratch fresh per call. The talc allocator handles
    // 1 MiB churns cleanly (the wasmi linear memory churn during Doom
    // is the stress workload that picked talc over linked_list_alloc;
    // see arest-kernel/Cargo.toml line 222). Zero-init via `vec![0u8;
    // count]` gives the fill function a defined-state slice — a
    // bug-prone fill that early-exits would leave zeros rather than
    // uninitialised memory.
    let mut scratch = vec![0u8; count];
    fill(scratch.as_mut_slice());
    // Identity-map copy: under tier-1 the userspace VA equals the
    // kernel VA, so a direct `copy_nonoverlapping` works. Once #527
    // lands real page tables this becomes `copy_to_user(buf_addr,
    // &scratch)` (the validated cross-AS write surface, #561).
    unsafe {
        core::ptr::copy_nonoverlapping(
            scratch.as_ptr(),
            buf_addr as *mut u8,
            count,
        );
    }
    count as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    use arest::entropy::{self, DeterministicSource};

    /// Test fixture mirror of `process::tests::with_deterministic_entropy`
    /// (process/process.rs:673). Installs a deterministic source,
    /// forces a CSPRNG reseed, runs the body, then uninstalls + reseeds
    /// so the next test starts clean. Required because
    /// `handle`/`fill_userspace` reach `arest::csprng::random_bytes`
    /// which panics if no entropy source is installed.
    fn with_deterministic_entropy<F: FnOnce()>(seed: [u8; 32], body: F) {
        entropy::install(alloc::boxed::Box::new(DeterministicSource::new(seed)));
        arest::csprng::reseed();
        body();
        entropy::uninstall();
        arest::csprng::reseed();
    }

    /// `getrandom(buf, 32, 0)` fills all 32 bytes and returns 32. The
    /// deterministic source guarantees the bytes aren't all zero
    /// (a zero-init buffer that the fill skipped would otherwise
    /// pass a "returns 32" check while silently doing nothing).
    #[test]
    fn getrandom_fills_buffer() {
        with_deterministic_entropy([7u8; 32], || {
            let mut buf = [0u8; 32];
            let result = handle(buf.as_mut_ptr() as u64, buf.len() as u64, 0);
            assert_eq!(result, 32);
            // ChaCha20 keystream from the deterministic seed has the
            // not-all-zero property by construction (the cipher's
            // output is uniformly distributed); a 32-byte all-zero
            // run has probability ~2^-256.
            assert!(buf.iter().any(|&b| b != 0),
                "getrandom must actually fill the buffer; got all-zero");
        });
    }

    /// `getrandom(buf, 2 MiB, 0)` short-reads to the 1 MiB cap and
    /// returns 1 MiB. Userspace libc loops on the remainder.
    #[test]
    fn getrandom_short_read_at_cap() {
        with_deterministic_entropy([3u8; 32], || {
            let mut buf: Vec<u8> = vec![0u8; (2 * GETRANDOM_MAX) as usize];
            let result = handle(buf.as_mut_ptr() as u64, 2 * GETRANDOM_MAX, 0);
            assert_eq!(result, GETRANDOM_MAX as i64);
            // The first `GETRANDOM_MAX` bytes should be filled (not
            // all zero); the tail past the cap should remain at its
            // initial zero state.
            assert!(buf[..GETRANDOM_MAX as usize].iter().any(|&b| b != 0),
                "first MiB must be CSPRNG-filled");
            assert!(buf[GETRANDOM_MAX as usize..].iter().all(|&b| b == 0),
                "bytes past the cap must be untouched");
        });
    }

    /// `getrandom(buf, 0, 0)` returns 0 cleanly without dereferencing
    /// `buf`. Lets userspace probe the syscall's existence without
    /// providing a real buffer.
    #[test]
    fn getrandom_zero_length() {
        // No entropy source installed: the zero-length path must NOT
        // reach the CSPRNG (which would panic on missing entropy).
        let result = handle(0xdeadbeef, 0, 0);
        assert_eq!(result, 0);
    }

    /// `getrandom(NULL, 16, 0)` returns `-EFAULT`. Symmetric with
    /// `write(1, NULL, n>0)` — null pointer with non-zero length is
    /// always EFAULT.
    #[test]
    fn getrandom_null_buf_with_count_returns_efault() {
        let result = handle(0, 16, 0);
        assert_eq!(result, -EFAULT);
    }

    /// The dispatch table routes syscall 318 to this handler.
    /// `getrandom(buf, 32, 0)` invoked through `dispatch::dispatch`
    /// returns 32 (the count) — confirms the wire-up.
    #[test]
    fn getrandom_dispatch_route() {
        use crate::syscall::dispatch::{dispatch, SYS_GETRANDOM};
        with_deterministic_entropy([5u8; 32], || {
            let mut buf = [0u8; 32];
            let result = dispatch(
                SYS_GETRANDOM,
                buf.as_mut_ptr() as u64,
                buf.len() as u64,
                0,
                0,
                0,
                0,
            );
            assert_eq!(result, 32);
            assert!(buf.iter().any(|&b| b != 0),
                "dispatch route must reach the real CSPRNG fill");
        });
    }

    /// Flags are ignored — `getrandom(buf, n, GRND_NONBLOCK |
    /// GRND_RANDOM)` produces a successful fill identical in shape
    /// (length, returns) to `getrandom(buf, n, 0)`. AREST has one
    /// entropy stream; no flag changes the behaviour.
    #[test]
    fn getrandom_flags_ignored() {
        with_deterministic_entropy([9u8; 32], || {
            let mut buf = [0u8; 16];
            // 0x0001 = GRND_NONBLOCK; 0x0002 = GRND_RANDOM. Both are
            // accepted as no-ops.
            let result = handle(buf.as_mut_ptr() as u64, buf.len() as u64, 0x0003);
            assert_eq!(result, 16);
            assert!(buf.iter().any(|&b| b != 0));
        });
    }

    /// `fill_userspace` rejects oversized count (> isize::MAX) with
    /// `-EFAULT` rather than panicking inside the slice constructor.
    /// Production callers can never reach this — `handle` caps at
    /// 1 MiB — but the guard keeps the function honest.
    #[test]
    fn fill_userspace_rejects_oversized_count() {
        let mut fill = |_scratch: &mut [u8]| panic!("fill should not be invoked");
        // Use a non-null buf so the count check is exercised, not
        // a null guard (fill_userspace doesn't check null — that's
        // handle's job).
        let buf = 0x1000_u64;
        let result = fill_userspace(buf, (isize::MAX as usize) + 1, &mut fill);
        assert_eq!(result, -EFAULT);
    }

    /// `GETRANDOM_MAX` is 1 MiB exactly. A static check so a future
    /// cap relaxation surfaces in the test diff and the rationale
    /// in the module-level comment can be revisited.
    #[test]
    fn getrandom_max_is_one_mib() {
        assert_eq!(GETRANDOM_MAX, 1024 * 1024);
    }
}
