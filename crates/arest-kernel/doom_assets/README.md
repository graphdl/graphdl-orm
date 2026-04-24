doom.wasm — provenance
======================

source:   https://github.com/jacobenget/doom.wasm/releases/tag/v0.1.0
asset:    doom-v0.1.0.wasm
size:     4,559,928 bytes (4.35 MiB)
license:  GPL-2.0 (upstream) — see licensing caveat in doom_bin.rs
baked:    via crates/arest-kernel/build.rs (emits $OUT_DIR/doom_assets.rs)
baked on: 2026-04-24, AREST issue #372

imports (10):
  loading.onGameInit          (i32, i32)       -> ()
  loading.wadSizes            (i32, i32)       -> ()
  loading.readWads            (i32, i32)       -> ()
  runtimeControl.timeInMilliseconds ()         -> i64
  ui.drawFrame                (i32)            -> ()
  gameSaving.sizeOfSaveGame   (i32)            -> i32
  gameSaving.readSaveGame     (i32, i32)       -> i32
  gameSaving.writeSaveGame    (i32, i32, i32)  -> i32
  console.onInfoMessage       (i32, i32)       -> ()
  console.onErrorMessage      (i32, i32)       -> ()

exports (4 funcs + 14 globals + 1 memory):
  initGame       () -> ()
  reportKeyDown  (i32) -> ()
  reportKeyUp    (i32) -> ()
  tickGame       () -> ()
  plus KEY_* i32 const globals and `memory` (min = 72 pages ≈ 4.7 MiB).

Signature notes vs crates/arest-kernel/src/doom.rs (as of 2026-04-24):
  The import NAMES match the shim's `bind_doom_imports` exactly, but
  six of ten SIGNATURES drift from the shim as landed in f3be6d4:
    * timeInMilliseconds: binary returns i64; shim wraps as i32.
    * wadSizes:           binary takes (i32,i32)->(); shim has
                          (i32)->i32.
    * readWads:           binary takes (i32,i32)->(); shim has
                          (i32)->i32.
    * sizeOfSaveGame:     binary takes (i32)->i32; shim has ()->i32.
    * readSaveGame:       binary takes (i32,i32)->i32; shim has
                          (i32)->().
    * writeSaveGame:      binary takes (i32,i32,i32)->i32; shim has
                          (i32,i32)->().
  `wasmi::Module::new(&engine, DOOM_WASM)` succeeds regardless — the
  signature check happens at `linker.instantiate(&mut store, &module)`
  time. Track B agent (owning doom.rs) needs to reconcile the shim
  signatures with the binary before the kernel can actually
  instantiate the Doom guest. See github.com/jacobenget/doom.wasm/
  blob/24bb772/src/doom_wasm.h for the authoritative interface.

WAD bundling:
  The 4.35 MiB size is inflated by what appears to be the Doom
  Shareware WAD (~3 MiB) embedded in the binary's rodata — returning
  0 from `wadSizes` causes the guest to fall back to that embedded
  copy (see upstream README). Shareware DOOM has been distributed
  gratis since 1993 but its license is not OSI-approved; verify
  redistribution terms before shipping AREST externally.

Licensing caveat:
  Upstream is GPL-2.0. `arest-kernel` is declared MIT in its
  Cargo.toml. Embedding a GPL-2.0 artifact into an MIT-licensed
  binary does NOT re-license AREST itself, but the combined
  kernel+WASM distribution inherits the GPL-2.0 restrictions of
  the embedded artifact. Before publishing a kernel image that
  includes this binary:
    - confirm the user accepts the GPL-2.0 inheritance, OR
    - defer the bake until a signature-compatible MIT/BSD/Apache-2.0
      port is identified, OR
    - rebuild doom.wasm from our own doomgeneric fork under our
      own license terms (but doomgeneric is GPL-2.0 too — Doom's
      engine source is GPL-2.0 since 1999).
  See issue #372 for the decision trail.
