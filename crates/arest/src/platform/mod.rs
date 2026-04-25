// crates/arest/src/platform/mod.rs
//
// Engine-level Platform functions that live outside the hardcoded
// `apply_platform` match in `ast.rs`. Each submodule owns a small,
// self-contained Platform body and exposes an `install()` function
// that registers the body into `ast::PLATFORM_FALLBACK` via
// `ast::install_platform_fn`.
//
// Why this layout (rather than another arm in `apply_platform`):
//   - These functions touch larger surface than the hot inner loop
//     (zip codec, future blob handling, future deflate). Keeping
//     them in subdirs of their own avoids bloating ast.rs and keeps
//     `cargo build`-time of the engine proper unaffected.
//   - The runtime-callback registry (`install_platform_fn`) is the
//     standard engine extension point for sync Platform bodies; using
//     it here means the dispatch path is identical to host-installed
//     bodies, and the sec-2 audit (`APPROVED_PLATFORM_FN_NAMES`)
//     remains the single source of truth for what may be installed.
//
// The whole `platform` module is std-only — Platform fns are gated
// behind `#[cfg(not(feature = "no_std"))]` in `ast::apply_platform`
// (returns `Object::Bottom` under no_std). The kernel build never
// reaches the install paths.

#![cfg(not(feature = "no_std"))]

pub mod zip;
