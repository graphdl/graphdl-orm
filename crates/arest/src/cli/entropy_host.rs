// crates/arest/src/cli/entropy_host.rs
//
// Host CLI entropy adapter (#574). Implements `arest::entropy::EntropySource`
// over the OS-provided RNG by delegating to the `getrandom` crate, which
// already wraps the per-platform syscalls we want:
//
//   * Linux / Android:  `getrandom(2)` (or `/dev/urandom` fallback)
//   * FreeBSD / NetBSD: `getrandom(2)`
//   * macOS / iOS:      `arc4random_buf(3)` / `getentropy(2)`
//   * Windows:          `BCryptGenRandom` (`BCRYPT_USE_SYSTEM_PREFERRED_RNG`)
//
// Why an adapter at all instead of calling `getrandom::getrandom` directly
// from `csprng.rs`: the engine MUST stay agnostic about where its seed
// material comes from. The same `csprng` module compiles into the FPGA
// soft-core (#569 RDRAND), the bare-metal x86_64 kernel (#570 virtio-rng),
// the WASM target (#572 `crypto.getRandomValues`), and the Wine launcher
// (#573 host passthrough). Each target installs its own `EntropySource`
// instance during boot; the engine only sees the trait. This file is the
// host CLI's installer.
//
// Wired from `main.rs`: a single `entropy::install(Box::new(
// HostEntropySource::new()))` call before any `csprng::random_*` path
// can fire. Lazy-seed in `csprng` would otherwise panic with the
// "no entropy source installed" message designed precisely to catch a
// missed install.

use alloc::boxed::Box;
use crate::entropy::{EntropyError, EntropySource};

/// Host-OS entropy source. Stateless wrapper around `getrandom::getrandom`
/// — the heavy lifting (platform detection, syscall vs. fallback, error
/// translation) lives in the upstream crate. Kept as its own type rather
/// than a free function so callers can pass a `Box<dyn EntropySource>`
/// uniformly with the kernel / FPGA adapters.
///
/// `Send + Sync` is satisfied trivially: no interior state, every call
/// hits the OS afresh. The trait's `Send + Sync` bound is what lets the
/// engine's global slot live behind a spin lock.
pub struct HostEntropySource;

impl HostEntropySource {
    /// New stateless host entropy source. The constructor does no I/O —
    /// we don't open `/dev/urandom` or pre-seed anything; the OS handles
    /// pooling. First entropy use is the first `fill` call.
    pub const fn new() -> Self {
        Self
    }

    /// Convenience constructor returning a boxed trait object so the
    /// install site at `main.rs` reads as
    /// `entropy::install(HostEntropySource::boxed())` without an extra
    /// `Box::new(...)` wrap.
    pub fn boxed() -> Box<dyn EntropySource> {
        Box::new(Self::new())
    }
}

impl Default for HostEntropySource {
    fn default() -> Self {
        Self::new()
    }
}

impl EntropySource for HostEntropySource {
    /// Fill `buf` from the OS RNG. The `getrandom` crate's contract is
    /// "all-or-nothing": on success the entire slice is filled, on
    /// failure no partial fill is reported. We translate that into the
    /// trait's short-read semantics by returning `Ok(buf.len())` for
    /// success and mapping every error to `EntropyError::Fault` (the
    /// distinction between transient and permanent at the OS level
    /// isn't usually surfaced in the error type — a retry by the
    /// `entropy::fill` outer loop is the right call either way).
    ///
    /// Empty buffer is a no-op and returns `Ok(0)` — matches the trait's
    /// "wrote n bytes starting at buf[0]" semantics without invoking
    /// the syscall for zero work.
    fn fill(&mut self, buf: &mut [u8]) -> Result<usize, EntropyError> {
        if buf.is_empty() {
            return Ok(0);
        }
        getrandom::getrandom(buf).map_err(|_| EntropyError::Fault)?;
        Ok(buf.len())
    }
}

// ── Tests ────────────────────────────────────────────────────────────
//
// We can't deterministically test "is this random" — a real OS RNG
// emitting all-zero would be a one-in-2^N freak event, not a unit-test
// failure mode. What we CAN assert:
//
//   * `fill` writes the full buffer (`Ok(n) == buf.len()`).
//   * The result isn't all-zero for a 32-byte request — the only way
//     a healthy OS RNG produces 32 zero bytes is by being broken or
//     not actually filling, which is exactly what we want to detect.
//   * Empty buffer returns `Ok(0)` without touching the OS.
//   * The boxed constructor produces a usable `Box<dyn EntropySource>`.
//   * The source can be installed into the global slot and pulled
//     through `entropy::fill` without panic.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entropy;

    #[test]
    fn fill_writes_full_buffer() {
        let mut src = HostEntropySource::new();
        let mut buf = [0u8; 64];
        let n = src.fill(&mut buf).expect("OS RNG must succeed");
        assert_eq!(n, buf.len(), "fill must report writing the whole buffer");
    }

    #[test]
    fn fill_32_bytes_is_not_all_zero() {
        // 32 bytes from a healthy OS RNG being all-zero has probability
        // 2^-256 — well below "test flake threshold". If this fails, the
        // RNG is genuinely broken or the adapter never reached the OS.
        let mut src = HostEntropySource::new();
        let mut buf = [0u8; 32];
        src.fill(&mut buf).expect("OS RNG must succeed");
        assert!(buf.iter().any(|&b| b != 0),
            "32 bytes from getrandom must contain at least one non-zero byte");
    }

    #[test]
    fn fill_empty_buffer_is_noop() {
        let mut src = HostEntropySource::new();
        let mut buf: [u8; 0] = [];
        let n = src.fill(&mut buf).expect("empty fill must succeed");
        assert_eq!(n, 0);
    }

    #[test]
    fn two_consecutive_fills_differ() {
        // Probability of two 32-byte OS RNG draws colliding is 2^-256.
        // If this trips, the adapter is returning a cached buffer or
        // the OS RNG is stuck.
        let mut src = HostEntropySource::new();
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        src.fill(&mut a).unwrap();
        src.fill(&mut b).unwrap();
        assert_ne!(a, b,
            "consecutive 32-byte fills must produce different output");
    }

    #[test]
    fn boxed_constructor_produces_usable_source() {
        // Smoke test — exercises the Box<dyn EntropySource> path that
        // main.rs uses for installation. A failure here would surface
        // as a build error (trait dispatch mismatch) rather than a
        // runtime panic, but the test pins the API so future trait
        // changes catch this site too.
        let mut src = HostEntropySource::boxed();
        let mut buf = [0u8; 16];
        src.fill(&mut buf).expect("boxed source must fill");
        assert!(buf.iter().any(|&b| b != 0));
    }

    #[test]
    fn install_and_pull_through_global_slot() {
        // End-to-end: install the host source, pull bytes via the
        // engine's `entropy::fill`, observe a non-zero buffer. Mirrors
        // exactly the path `csprng::seed_from_entropy` will take when
        // re-keying the ChaCha20 stream from the OS RNG.
        //
        // Holds the cross-module test lock so a concurrent
        // `entropy::tests::*` case isn't installing a Deterministic /
        // AlwaysFault source while we're proving the host path works.
        let _guard = entropy::TEST_LOCK.lock();
        entropy::install(HostEntropySource::boxed());
        let mut buf = [0u8; 32];
        let result = entropy::fill(&mut buf);
        entropy::uninstall();
        result.expect("entropy::fill must succeed with HostEntropySource installed");
        assert!(buf.iter().any(|&b| b != 0),
            "entropy::fill via host source must produce non-zero bytes");
    }
}
