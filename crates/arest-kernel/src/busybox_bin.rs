// crates/arest-kernel/src/busybox_bin.rs
//
// Baked busybox static-ELF surface (#525/#526). The binary is the
// build script's own product: `--features busybox` cross-compiles
// vendor/busybox (clang --target=x86_64-unknown-linux-musl), links it
// against the #524 musl libc.a with ld.lld, strips it, and emits
// `$OUT_DIR/busybox_assets.rs` with the `include_bytes!` of the
// stripped ELF — the file we `include!` below. Same indirection as
// `doom_bin.rs`/`ui_assets.rs`: path normalisation stays in build.rs.
//
// The re-exported constant is `pub static BUSYBOX_ELF: &[u8]` — the
// complete `ELF 64-bit LSB executable, x86-64, statically linked`
// multi-call binary (ash/sh/cat/echo/head/ls/tail/wc as of #526).
//
// Gating: `cfg(busybox_built)` is emitted by build.rs ONLY when the
// whole busybox pass succeeded (vendor tree + cross-toolchain + musl
// libc.a + final link). When the pass degrades (missing clang, fresh
// clone without vendor trees) the cfg is absent, this module —
// and every consumer, e.g. `system::seed_busybox_file_cells` —
// compiles out, and the kernel boots without a /bin/busybox File fact.
//
// Why a File fact and not a special exec path: per AREST's Platform
// Binding (§4.4), files ARE the population — the FILE cell is P, and a
// binary is a `File_has_Name` + `File_has_ContentRef` pair (#398)
// resolved through the same surface `openat`/`interp_resolve` already
// walk. `system::seed_busybox_file_cells` installs that pair at boot.

#![cfg(busybox_built)]

include!(concat!(env!("OUT_DIR"), "/busybox_assets.rs"));
