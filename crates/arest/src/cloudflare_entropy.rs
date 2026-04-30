// crates/arest/src/cloudflare_entropy.rs
//
// Cloudflare Worker entropy adapter (#572 / Rand-T4). Implements
// `arest::entropy::EntropySource` against Web Crypto's
// `crypto.getRandomValues(Uint8Array)` — the per-V8-isolate CSPRNG every
// Workers runtime exposes. Mirror of the host CLI adapter
// (`cli/entropy_host.rs`, #574); differs only in how `getrandom` is
// routed under the hood.
//
// ## Why this adapter exists
//
// The engine MUST stay agnostic about where its seed material comes
// from. The same `csprng` ChaCha20 module compiles into the FPGA soft-
// core (#569 RDSEED), the bare-metal x86_64 / aarch64 kernel (#569 /
// #570 silicon RNG), the UEFI kernel (#571 EFI_RNG_PROTOCOL fallback),
// and the host CLI (#574 OS getrandom). The Cloudflare Worker target is
// just one more substrate — but it's the only one the CSPRNG would
// otherwise reach without an installed source, because the worker build
// flips the `cloudflare` feature on (which keeps `not(no_std)`-gated
// modules in scope) and the WASM-bindgen entry point `__wasm_init`
// fires before any HTTP route handler can call `csprng::random_bytes`.
// Without an `entropy::install(...)` here, the lazy-seed panic in
// `csprng::seed_from_entropy` would fire on the first `random_bytes`
// call from a route handler — exactly the failure mode this whole
// substrate exists to prevent.
//
// ## Why `getrandom`, not `web_sys::Crypto` directly
//
// `getrandom = { version = "0.2", features = ["js"] }` (Cargo.toml line
// 222-223) already routes wasm32-unknown-unknown calls through
// `js-sys::Function`-loaded `crypto.getRandomValues`. The crate handles:
//   * fetching the per-isolate `crypto` global,
//   * batching > 65 536-byte requests into 64 KiB chunks (Web Crypto's
//     hard cap per call),
//   * falling back through `globalThis.crypto` / `self.crypto` /
//     `process.versions` so the same artifact runs in CF Workers,
//     browsers, and Node.js Workers shims.
//
// Doing the wasm-bindgen plumbing ourselves would duplicate ~80 lines
// of code already battle-tested across the Rust wasm ecosystem, AND
// would diverge from `cli/entropy_host.rs` (#574) — which we deliberately
// want to mirror so the trait's "any target installs one source at boot"
// shape stays uniform.
//
// ## Wired from `cloudflare::__wasm_init`
//
// The `#[wasm_bindgen(start)]`-decorated function in `cloudflare.rs`
// runs once when the WASM module is first instantiated by the Worker
// runtime. It already calls `console_error_panic_hook::set_once()`;
// `install_worker_entropy()` joins it as the second boot-time step.
// Because `entropy::install` REPLACES the previously installed source
// (entropy.rs:116), this must run before any route handler can fire,
// not from inside one. The wasm-bindgen `start` attribute guarantees
// that ordering.

use alloc::boxed::Box;
use crate::entropy::{EntropyError, EntropySource};

/// Cloudflare Worker entropy source backed by Web Crypto's
/// `crypto.getRandomValues(Uint8Array)`. Stateless wrapper around
/// `getrandom::getrandom` — on `wasm32-unknown-unknown` (the Worker
/// target) the dependency is built with `features = ["js"]`, which
/// routes every call through wasm-bindgen → `js-sys::Function` →
/// `globalThis.crypto.getRandomValues`. V8 manages the underlying
/// entropy bootstrap (per-isolate CSPRNG, reseeded out of band by the
/// host kernel); from Rust's perspective this is "the OS RNG" with no
/// extra plumbing.
///
/// `Send + Sync` is satisfied trivially: zero state, every call hits
/// the JS-side `crypto` afresh. The trait's `Send + Sync` bound is
/// what lets `entropy::GLOBAL_SOURCE` live behind a spin lock.
pub struct WorkerEntropySource;

impl WorkerEntropySource {
    /// New stateless worker entropy source. The constructor does no I/O
    /// — V8 handles per-isolate pooling. First entropy use is the first
    /// `fill` call, which traps into `crypto.getRandomValues`.
    pub const fn new() -> Self {
        Self
    }

    /// Convenience constructor returning a boxed trait object. Lets the
    /// install site at `cloudflare::__wasm_init` read as
    /// `entropy::install(WorkerEntropySource::boxed())` without an
    /// extra `Box::new(...)` wrap — same shape as `HostEntropySource::
    /// boxed()` so the per-target adapters stay symmetrical.
    pub fn boxed() -> Box<dyn EntropySource> {
        Box::new(Self::new())
    }
}

impl Default for WorkerEntropySource {
    fn default() -> Self {
        Self::new()
    }
}

impl EntropySource for WorkerEntropySource {
    /// Fill `buf` from Web Crypto. The `getrandom` crate's contract is
    /// "all-or-nothing": on success the entire slice is filled (the
    /// crate internally batches > 64 KiB requests across multiple
    /// `crypto.getRandomValues` calls). On failure no partial fill is
    /// reported. We translate that into the trait's short-read
    /// semantics by returning `Ok(buf.len())` for success and mapping
    /// every error (including the "no `crypto` global" case that would
    /// fire if this somehow ran outside a Worker isolate) to
    /// `EntropyError::Fault` — the outer `entropy::fill` retry loop is
    /// the right call regardless.
    ///
    /// Empty buffer is a no-op and returns `Ok(0)` — matches the
    /// trait's "wrote n bytes starting at buf[0]" semantics without
    /// crossing the WASM-JS boundary for zero work.
    fn fill(&mut self, buf: &mut [u8]) -> Result<usize, EntropyError> {
        if buf.is_empty() {
            return Ok(0);
        }
        getrandom::getrandom(buf).map_err(|_| EntropyError::Fault)?;
        Ok(buf.len())
    }
}

// ── Boot-time install hook ──────────────────────────────────────────

/// Install the worker entropy source into the engine's global slot.
/// Called once from `cloudflare::__wasm_init` (the
/// `#[wasm_bindgen(start)]` boot path) before any route handler can
/// fire. Must run before any `csprng::random_*` call — the lazy-seed
/// path in `csprng::seed_from_entropy` panics with "no entropy source
/// installed" otherwise.
///
/// `entropy::install` REPLACES any previously installed source, so
/// calling this from the start function is the only correct place: it
/// runs exactly once per WASM module instance, before any handler
/// thread starts. Calling from inside a handler would race other
/// handlers that may have already called `random_*`.
pub fn install_worker_entropy() {
    crate::entropy::install(WorkerEntropySource::boxed());
}

// ── Tests ────────────────────────────────────────────────────────────
//
// The host CI runs these as plain `cargo test --features cloudflare,
// debug-def --lib` calls on x86_64-{linux,macos,windows}. On every host
// target, `getrandom` resolves to the OS-syscall backend (Linux
// getrandom(2), macOS getentropy(2), Windows BCryptGenRandom) — the
// SAME `getrandom::getrandom` API the wasm32 target calls under the
// `js` feature. The behaviour we assert is interface-level (filled
// buffer, non-zero output, distinct successive draws), so a passing
// host test pins the Rust-side glue; the wasm-bindgen routing is
// validated at the integration / wrangler-dev level.
//
// We can't deterministically assert "is this random" — a real CSPRNG
// emitting all-zero would be a 1-in-2^256 freak event, not a unit-test
// failure mode. What we CAN assert:
//
//   * `fill` writes the full buffer (`Ok(n) == buf.len()`).
//   * The result isn't all-zero for a 32-byte request — the only way
//     a healthy CSPRNG produces 32 zero bytes is by being broken or
//     not actually filling, which is exactly what we want to detect.
//   * Two consecutive fills produce different output.
//   * Empty buffer returns `Ok(0)` without crossing the boundary.
//   * The boxed constructor produces a usable `Box<dyn EntropySource>`.
//   * The source can be installed into the global slot and pulled
//     through `entropy::fill` without panic.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entropy;

    #[test]
    fn fill_writes_full_buffer() {
        let mut src = WorkerEntropySource::new();
        let mut buf = [0u8; 64];
        let n = src.fill(&mut buf).expect("CSPRNG must succeed");
        assert_eq!(n, buf.len(), "fill must report writing the whole buffer");
    }

    #[test]
    fn fill_32_bytes_is_not_all_zero() {
        // 32 bytes from a healthy CSPRNG being all-zero has probability
        // 2^-256 — well below "test flake threshold". If this fails, the
        // RNG is genuinely broken or the adapter never reached the
        // platform RNG.
        let mut src = WorkerEntropySource::new();
        let mut buf = [0u8; 32];
        src.fill(&mut buf).expect("CSPRNG must succeed");
        assert!(
            buf.iter().any(|&b| b != 0),
            "32 bytes from getRandomValues must contain at least one non-zero byte"
        );
    }

    #[test]
    fn fill_empty_buffer_is_noop() {
        let mut src = WorkerEntropySource::new();
        let mut buf: [u8; 0] = [];
        let n = src.fill(&mut buf).expect("empty fill must succeed");
        assert_eq!(n, 0);
    }

    #[test]
    fn two_consecutive_fills_differ() {
        // Probability of two 32-byte CSPRNG draws colliding is 2^-256.
        // If this trips, the adapter is returning a cached buffer or
        // the underlying RNG is stuck.
        let mut src = WorkerEntropySource::new();
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        src.fill(&mut a).unwrap();
        src.fill(&mut b).unwrap();
        assert_ne!(
            a, b,
            "consecutive 32-byte fills must produce different output"
        );
    }

    #[test]
    fn boxed_constructor_produces_usable_source() {
        // Smoke test — exercises the Box<dyn EntropySource> path that
        // `cloudflare::__wasm_init` uses for installation. A failure
        // here would surface as a build error (trait dispatch
        // mismatch) rather than a runtime panic, but the test pins the
        // API so future trait changes catch this site too.
        let mut src = WorkerEntropySource::boxed();
        let mut buf = [0u8; 16];
        src.fill(&mut buf).expect("boxed source must fill");
        assert!(buf.iter().any(|&b| b != 0));
    }

    #[test]
    fn install_and_pull_through_global_slot() {
        // End-to-end: install the worker source, pull bytes via the
        // engine's `entropy::fill`, observe a non-zero buffer. Mirrors
        // exactly the path `csprng::seed_from_entropy` will take when
        // re-keying the ChaCha20 stream from the worker RNG on first
        // `random_bytes` call after `__wasm_init`.
        //
        // Holds the cross-module test lock so a concurrent
        // `entropy::tests::*` case isn't installing a Deterministic /
        // AlwaysFault source while we're proving the worker path
        // works.
        let _guard = entropy::TEST_LOCK.lock();
        entropy::install(WorkerEntropySource::boxed());
        let mut buf = [0u8; 32];
        let result = entropy::fill(&mut buf);
        entropy::uninstall();
        result.expect("entropy::fill must succeed with WorkerEntropySource installed");
        assert!(
            buf.iter().any(|&b| b != 0),
            "entropy::fill via worker source must produce non-zero bytes"
        );
    }

    #[test]
    fn install_worker_entropy_routes_through_global_slot() {
        // Mirror of the boot-path call: `install_worker_entropy()` from
        // `cloudflare::__wasm_init`. Asserts that after this single
        // call, `entropy::fill` returns non-zero bytes — the same
        // guarantee `csprng::seed_from_entropy` relies on at first use.
        let _guard = entropy::TEST_LOCK.lock();
        install_worker_entropy();
        let mut buf = [0u8; 32];
        let result = entropy::fill(&mut buf);
        entropy::uninstall();
        result.expect("entropy::fill must succeed after install_worker_entropy");
        assert!(
            buf.iter().any(|&b| b != 0),
            "post-install_worker_entropy fill must produce non-zero bytes"
        );
    }
}
