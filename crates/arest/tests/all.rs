// crates/arest/tests/all.rs
//
// Single integration-test binary that includes the three large
// integration suites as submodules. Cargo previously built each
// `tests/*.rs` as a separate binary, paying the full arest rlib +
// dep static-link cost per file (3-5 s on Windows MSVC × 3 binaries).
// One merged binary links the same code once.
//
// `sec_2_platform_fallback_audit.rs` and `cluster_udp.rs` stay separate
// (declared in Cargo.toml as their own `[[test]]` entries):
//   * sec_2 — must observe the process-global PLATFORM_FALLBACK
//     registry untainted by other suites' install_platform_fn calls.
//   * cluster_udp — gated on the `cluster` feature; bundling would
//     force every `cargo test` invocation to either pass --features
//     cluster (slower) or skip the cluster tests entirely.
//
// `autotests = false` in Cargo.toml disables the per-file auto-target
// detection so the source files included via `mod` below are not also
// built as standalone test binaries.

mod integration;
mod properties;
mod e3_integration;
