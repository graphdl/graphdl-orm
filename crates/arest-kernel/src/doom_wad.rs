// crates/arest-kernel/src/doom_wad.rs
//
// Baked Doom IWAD binary surface (#383). The binary is
// `doom_assets/doom1.wad`, sourced from the id Software DOOM 1
// Shareware v1.9 release (see doom_assets/README.md for full
// provenance, sha256, and the shareware redistribution license note).
//
// Sibling of `doom_bin.rs`: same include-from-`$OUT_DIR` dance, same
// fresh-clone fallback, same cfg-gate rationale. Where `doom_bin.rs`
// exports the 4.35 MiB Doom-WASM engine blob, this file exports the
// ~4 MiB IWAD whose lumps the engine loads (textures, sprites, level
// maps, sound effects, music).
//
// The actual `include_bytes!` lives in the `$OUT_DIR/doom_wad.rs`
// produced by build.rs — the file we `include!` from below. That
// indirection keeps build-time path handling (Windows vs Linux
// path-separator normalisation, `\\?\` UNC-prefix stripping, the
// "fresh clone without the binary staged" fallback) out of the
// source tree and into `build.rs`, which already does the same
// dance for the UI bundle in `$OUT_DIR/ui_assets.rs` and the WASM
// blob in `$OUT_DIR/doom_assets.rs`.
//
// The re-exported constant is `pub static DOOM_WAD: &[u8]`:
//   * Non-empty iff `crates/arest-kernel/doom_assets/doom1.wad` was
//     present at build time.
//   * Empty (`&[]`) otherwise, in which case `KernelDoomHost::
//     wad_sizes` falls through to returning `(0, 0)` — per the
//     `doom_wasm.h` "If numberOfWads remains 0, Doom loads shareware
//     WAD" contract the guest engine then uses the Shareware WAD
//     that's been embedded in the baked `doom.wasm` binary since
//     Track C's dc94345 bake. That keeps fresh clones building
//     cleanly without a real WAD staged, and lets us skip the
//     WAD-bake step in CI / Docker contexts that don't ship the
//     binary.
//
// Gating rationale (mirrors src/doom.rs / src/doom_bin.rs):
//   wasmi — the only path that reaches this blob — is cfg-gated on
//   `cfg(all(target_os = "uefi", target_arch = "x86_64"))` in
//   Cargo.toml (the BIOS bootloader triple-faults on load when the
//   wasmi runtime is reachable, verified via revert 5e8a15e; the
//   aarch64 UEFI arm has no kernel_run -> no wasmi caller). There's
//   no point baking a 4 MiB blob into the other build targets, so
//   this module is cfg-gated to match.
//
// Licensing:
//   DOOM1.WAD is the id Software DOOM 1 Shareware v1.9 IWAD. id
//   Software's 1993 Shareware license permits unmodified
//   redistribution of the binary WAD file at no charge. The WAD is
//   NOT GPL-2.0 (it is data, not source, and pre-dates id's 1997
//   Doom engine GPL-release) — so embedding it in the kernel does
//   not itself trigger GPL-copyleft obligations on the kernel
//   image. The baked `doom.wasm` engine IS GPL-2.0 and that
//   inheritance is tracked in #396 / doom_bin.rs's top-of-file
//   caveat; this WAD bake is the smaller companion concern.

#![cfg(all(target_os = "uefi", target_arch = "x86_64"))]

// Pulls in `pub static DOOM_WAD: &[u8] = include_bytes!("…");` (or
// `&[]` when the binary is absent). The generated file carries its
// own `#[allow(dead_code)]` because the WAD slice is only consumed
// from the `KernelDoomHost::wad_sizes` / `read_wads` trampolines,
// which are registered via `bind_doom_imports` but not yet reached
// from `kernel_run` under the pre-#376 state (the Doom module
// isn't actually instantiated yet). Dropping the allow after the
// instantiation lands is a single-line follow-up.
include!(concat!(env!("OUT_DIR"), "/doom_wad.rs"));
