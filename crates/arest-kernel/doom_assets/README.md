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

---

doom1.wad — provenance (#383)
=============================

source:   https://distro.ibiblio.org/slitaz/sources/packages/d/doom1.wad
          (ibiblio.org mirror of the id Software DOOM 1 Shareware
          v1.9 IWAD, unchanged since id's 1993 shareware release)
asset:    doom1.wad
size:     4,196,020 bytes (4.00 MiB)
sha256:   1d7d43be501e67d927e415e0b8f3e29c3bf33075e859721816f652a526cac771
magic:    "IWAD" (0x49574144), 1264 directory entries at offset 0x3fb7b4
license:  id Software 1993 Shareware — redistribution of the
          UNMODIFIED binary WAD file is permitted gratis. The
          shareware is NOT an OSI-approved license, but
          redistribution specifically is allowed (the contentious
          bits are commercial re-sale and modification, neither of
          which applies to embedding the raw WAD in a kernel
          image). Full terms at
          https://doomwiki.org/wiki/Shareware_clause.
baked:    via crates/arest-kernel/build.rs (emits $OUT_DIR/doom_wad.rs)
baked on: 2026-04-24, AREST issue #383

Purpose:
  Provides the real IWAD bytes (textures, sprites, level maps,
  sound effects, music) that the Doom engine loads at runtime.
  Sibling of doom.wasm above — that binary is the engine, this is
  the data the engine loads. Together they give us a complete Doom
  runtime; without this file, `wad_sizes` returns `(0, 0)` and the
  guest engine falls back to the Shareware WAD embedded in its own
  rodata (jacobenget/doom.wasm v0.1.0 ships one inline, adding
  ~3 MiB of redundancy to the WASM blob; see "WAD bundling" note
  above). So this external WAD makes the embedded copy redundant —
  once this file is baked, the `doom.wasm` file's embedded WAD is
  effectively dead weight on the image. A future cleanup could
  rebuild doom.wasm from source with the embedded WAD stripped to
  recover ~3 MiB. Tracked under #396.

Layout (IWAD header):
  bytes  0..4   = "IWAD" magic.
  bytes  4..8   = int32 LE, number of lumps (1264 for v1.9).
  bytes  8..12  = int32 LE, byte offset of the directory table
                   (0x3fb7b4).
  bytes 12..    = lump data blobs — each indexed by the directory
                   entries (16 bytes each: offset / length / name).

Consumer:
  `src/doom_wad.rs` re-exports the baked bytes as `DOOM_WAD: &[u8]`.
  `src/doom.rs` -> `KernelDoomHost::wad_sizes` returns
  `(1, DOOM_WAD.len())` when the WAD is present, and
  `KernelDoomHost::read_wads` copies the bytes into the guest's
  scratch buffer + records the length in `lengths_out[0]`. Per the
  jacobenget/doom.wasm contract, the first WAD is always the IWAD;
  PWADs (if we ever ship any) would follow as lengths_out[1..].

Licensing vs the GPL-2.0 doom.wasm:
  The WAD file is data, not executable code, and it pre-dates id's
  1997 Doom engine source release (id open-sourced the engine under
  GPL-2.0 in 1999; the engine is at v1.9 of the binary Doom release
  from 1995, but the open-sourced engine is derived from the 1997
  "Doom source code" package). The Shareware WAD is NOT covered by
  the engine's GPL-2.0 — it has its own restrictive shareware
  license that happens to permit redistribution. Embedding the WAD
  in the kernel therefore does NOT expand the GPL-2.0 surface the
  doom.wasm bake already established; it adds a separate "id
  Software Shareware" restriction, which the kernel image must
  preserve the "redistribute unmodified + free of charge" clauses
  of. A release of AREST that wants to ship a commercial Doom
  engine would need to strip both this file AND the embedded copy
  in doom.wasm (the latter requires a rebuild of the wasm blob).
