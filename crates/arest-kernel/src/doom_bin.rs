// crates/arest-kernel/src/doom_bin.rs
//
// Baked Doom WASM binary surface (#372). The binary is
// `doom_assets/doom.wasm`, sourced from
// https://github.com/jacobenget/doom.wasm/releases/tag/v0.1.0
// (see doom_assets/README.md for full provenance, signature
// discrepancies vs src/doom.rs, and the GPL-2.0 licensing caveat).
//
// The actual `include_bytes!` lives in the `$OUT_DIR/doom_assets.rs`
// produced by build.rs — the file we `include!` from below. That
// indirection keeps build-time path handling (Windows vs Linux
// path-separator normalisation, `\\?\` UNC-prefix stripping, the
// "fresh clone without the binary staged" fallback) out of the
// source tree and into `build.rs`, which already does the same
// dance for the UI bundle in `$OUT_DIR/ui_assets.rs`.
//
// The re-exported constant is `pub static DOOM_WASM: &[u8]`:
//   * Non-empty iff `crates/arest-kernel/doom_assets/doom.wasm` was
//     present at build time.
//   * Empty (`&[]`) otherwise, so `wasmi::Module::new(&engine,
//     DOOM_WASM)` at the call site (future kernel_run handoff)
//     returns an error rather than unsafely dereferencing null.
//     Callers should gate on `DOOM_WASM.is_empty()` before the
//     wasmi load path.
//
// Gating rationale (mirrors src/doom.rs):
//   wasmi — the only consumer of this blob — is cfg-gated on
//   `cfg(all(target_os = "uefi", target_arch = "x86_64"))` in
//   Cargo.toml (the BIOS bootloader triple-faults on load when the
//   wasmi runtime is reachable, verified via revert 5e8a15e; the
//   aarch64 UEFI arm has no kernel_run -> no wasmi caller). There's
//   no point baking a 4.35 MiB blob into the other build targets,
//   so this module is cfg-gated to match.

#![cfg(all(target_os = "uefi", target_arch = "x86_64"))]

// Pulls in `pub static DOOM_WASM: &[u8] = include_bytes!("…");` (or
// `&[]` when the binary is absent). The generated file carries its
// own `#[allow(dead_code)]` because the kernel_run handoff hasn't
// yet landed a `wasmi::Module::new(&engine, DOOM_WASM)` caller —
// tracked under #376 / the main session's entry_uefi work. Dropping
// the allow after that call lands is a single-line follow-up.
include!(concat!(env!("OUT_DIR"), "/doom_assets.rs"));
