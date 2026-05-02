// crates/arest-foundation/src/lib.rs
//
// Foundation primitives for AREST (#686).
//
// Hosts the leaf-pure no_std-reachable modules that sit underneath the
// engine: synchronisation primitives, entropy + CSPRNG stack, crypto +
// per-cell AEAD, naming / json_min / time_shim helpers. The engine
// crate (`arest`) re-exports each module so existing call sites
// (`arest::sync`, `arest::entropy`, …) keep working unchanged.
//
// Why a separate crate, not a module: cargo only re-runs codegen for
// crates whose source tree has changed. Touching `ast.rs` no longer
// invalidates the entropy/AEAD codegen; the engine's incremental cycle
// drops from "rebuild every leaf module too" to "rebuild only the
// engine modules that changed".
//
// Contract for code in this crate:
//   1. NO `use` of `arest::*` — this crate sits below the engine and
//      cannot reach back up.
//   2. NO `std::*` imports outside an explicit `#[cfg(not(feature = "no_std"))]`
//      gate. Every primitive here must compile under `--no-default-features
//      --features no_std` so the kernel + WASM targets stay clean.
//   3. Test cells live next to their module (`#[cfg(test)] mod tests`).
//      No integration tests — those belong in the consuming crate.

#![cfg_attr(feature = "no_std", no_std)]

extern crate alloc;

pub mod sync;
