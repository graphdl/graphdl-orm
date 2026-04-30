// crates/arest/src/time_shim.rs
//
// `std::time::Instant::now()` panics on `wasm32-unknown-unknown` —
// the target has no monotonic clock, and the std stub for `Instant`
// is a `panic!("time not implemented on this platform")`. With
// `panic = "abort"` (wasm-pack release default) that panic becomes a
// raw `unreachable` WASM trap, surfacing in the Cloudflare Worker as
// `RuntimeError: unreachable` on every parse call.
//
// Stage-2 of the FORML 2 parser uses `Instant::now()` purely for
// optional `AREST_STAGE12_TRACE` logging — the duration is read but
// never affects parser output. So a shim that returns zero on wasm32
// is functionally identical to the std behavior everywhere a trace
// isn't being printed (i.e. always, in the worker).
//
// Native targets keep the real `std::time::Instant` so trace
// telemetry on the CLI / kernel host still works as designed.
//
// Under `feature = "no_std"` (the kernel build) we route every
// target to the zero-sentinel shim — there is no `std::time` to
// reach for and the only consumer is the gated trace path, which
// is compiled out anyway. This unblocks #588 (lifting Stage-2 to
// no_std): once the shim is no_std-clean the parser cascade no
// longer needs `std::time::Instant` in its closure of imports.

// Real `std::time::Instant` on std-host non-wasm targets.
//
// `not(feature = "no_std")` is critical when `std-deps + no_std` are
// composed together (#592 feature-conflict cleanup): `pub use
// std::time::Instant` would otherwise resolve `std::*` against a
// crate that has `#![no_std]` set at the root, blowing up with
// E0433. The no_std shim below already covers any feature combo
// where `no_std` is on; gating the std re-export to
// `not(feature = "no_std")` makes the two arms truly disjoint.
#[cfg(all(feature = "std-deps", not(target_arch = "wasm32"), not(feature = "no_std")))]
pub use std::time::Instant;

// Zero-sentinel shim on wasm32 (`std::time::Instant::now()` panics
// there) and on no_std builds (no `std::time` available).
#[cfg(any(feature = "no_std", target_arch = "wasm32"))]
#[derive(Clone, Copy)]
pub struct Instant;

#[cfg(any(feature = "no_std", target_arch = "wasm32"))]
impl Instant {
    /// Always returns the same sentinel — wasm32 has no clock to
    /// query and the kernel's monotonic clock isn't piped through
    /// here yet. Cheap (compiles to a single MOV) so call sites can
    /// keep their unconditional `let t = Instant::now()` shape.
    pub fn now() -> Self { Self }

    /// Always zero. The only consumer is the `AREST_STAGE12_TRACE`
    /// logging branch, which is gated on a non-wasm `env::var` and
    /// therefore unreachable in the worker / kernel; if a future
    /// caller asserts on elapsed time they get a clear "all zero"
    /// signal rather than a panic.
    pub fn elapsed(&self) -> core::time::Duration {
        core::time::Duration::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test on the host build — proves the shim compiles and
    /// that the native path still threads through `std::time::Instant`.
    /// The wasm32 path is exercised by the `parse_intercept` tests
    /// running against the deployed Worker (whose binary IS the
    /// wasm32 build).
    #[test]
    fn instant_now_does_not_panic() {
        let t = Instant::now();
        let _ = t.elapsed();
    }
}
