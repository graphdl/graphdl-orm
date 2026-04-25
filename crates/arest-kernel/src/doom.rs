// crates/arest-kernel/src/doom.rs
//
// Doom WASM host-shim (#270/#271). Wire the guest-side Doom port
// (doomgeneric compiled to wasm32) to the UEFI kernel's I/O surface:
// WAD loading, 35 Hz game loop timing, framebuffer blit, save-game
// persistence, and console message routing. All imports live in four
// WebAssembly import-module namespaces matching the groupings used in
// the doomgeneric-wasm sidecar contract:
//
// Signatures below are reconciled against the authoritative
// jacobenget/doom.wasm v0.1.0 (commit dc94345) doom_wasm.h header at
// 24bb772 — six imports drifted from the binary contract on the prior
// wave and `wasmi::Linker::instantiate` will reject the mismatches. The
// `_ptr: i32` arguments are guest-linear-memory offsets (wasm32 makes
// them 4 bytes wide, passed through the host ABI as signed i32 by
// wasmi's `IntoFunc` machinery). `size_t` from the C header is i32 in
// wasm32; `uint64_t` is i64.
//
//   loading:
//     onGameInit(width: i32, height: i32)            - one-time init
//                                                       with framebuffer
//                                                       dimensions in
//                                                       pixels (640x400
//                                                       under our build).
//     wadSizes(num_wads_ptr: i32,
//              total_bytes_ptr: i32)                  - host writes
//                                                       `int32_t` WAD
//                                                       count and
//                                                       `size_t` total
//                                                       byte size into
//                                                       guest memory at
//                                                       the two
//                                                       out-pointers.
//                                                       Count of 0 makes
//                                                       Doom fall back to
//                                                       its built-in
//                                                       shareware path.
//     readWads(wad_buf_ptr: i32,
//              lengths_ptr: i32)                      - guest has
//                                                       allocated
//                                                       `total_bytes` of
//                                                       WAD storage and
//                                                       a per-WAD length
//                                                       array; host
//                                                       copies the WAD
//                                                       blob and the
//                                                       `int32_t` length
//                                                       per WAD into
//                                                       guest memory.
//
//   runtimeControl:
//     timeInMilliseconds() -> i64                    - monotonic
//                                                       milliseconds
//                                                       since boot
//                                                       (`uint64_t` per
//                                                       header).
//                                                       Backed by the
//                                                       UEFI PIT 1 kHz
//                                                       IRQ counter
//                                                       (Track AA,
//                                                       commit be9320d
//                                                       / #379) via
//                                                       `arch::time::
//                                                       now_ms`.
//
//   ui:
//     drawFrame(frame_ptr: i32)                      - present one
//                                                       640x400 ARGB
//                                                       buffer to the
//                                                       linear FB via
//                                                       framebuffer::
//                                                       blit_doom_frame
//                                                       (02bdae1).
//
//   gameSaving:
//     sizeOfSaveGame(gamemap: i32) -> i32            - length of the
//                                                       host-persisted
//                                                       save slot for
//                                                       `gamemap`; 0
//                                                       if none.
//     readSaveGame(gamemap: i32,
//                  out_ptr: i32) -> i32              - copy save bytes
//                                                       into guest
//                                                       memory at
//                                                       out_ptr; return
//                                                       bytes actually
//                                                       written.
//     writeSaveGame(gamemap: i32,
//                   data_ptr: i32,
//                   data_len: i32) -> i32            - persist `data_len`
//                                                       bytes from
//                                                       guest memory at
//                                                       data_ptr; return
//                                                       bytes persisted
//                                                       (0 if
//                                                       unsupported).
//
//   console:
//     onInfoMessage(ptr: i32, len: i32)              - I_Printf-class
//                                                       UTF-8 bytes;
//                                                       mirror to
//                                                       kernel console.
//     onErrorMessage(ptr: i32, len: i32)             - I_Error-class
//                                                       UTF-8 bytes;
//                                                       mirror to
//                                                       kernel console
//                                                       then let the
//                                                       guest trap.
//
// This file is the scaffold: the [`DoomHost`] trait publishes the
// 10-method API contract, [`KernelDoomHost`] stubs every method as a
// not-yet-implemented panic (for the side-effect imports) or a zero
// return (for the pure-query imports), and [`bind_doom_imports`]
// registers each import against a [`wasmi::Linker`] via `func_wrap`.
//
// The kernel_run handoff under UEFI (currently halting after the
// wasmi-tiny-module smoke, see entry_uefi.rs) will grow a call to
// [`bind_doom_imports`] once the real doomgeneric-wasm module loads.
// Until then, this module is only wired into the build so the API
// contract is locked in and the wasmi-side binding shape is exercised
// by `cargo check --target x86_64-unknown-uefi`.
//
// Gating: `#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]`
// in `main.rs`. wasmi is a UEFI-x86_64-only dep — the BIOS target
// triple-faults on load when wasmi is reachable (verified via
// revert 5e8a15e), and the aarch64 UEFI arm is scaffold-only today
// (no kernel_run path yet, so no wasmi caller). The trait itself is
// arch-neutral; when either of those gates drops the cfg can
// broaden to match.

#![cfg(all(target_os = "uefi", target_arch = "x86_64"))]

use alloc::vec;
use alloc::vec::Vec;
use pc_keyboard::{DecodedKey, KeyCode};
use wasmi::{Instance, Linker, Store};

use crate::block::BLOCK_SECTOR_SIZE;
use crate::block_storage::{self, RegionHandle};

/// Host-side callbacks the doomgeneric-wasm guest expects. Every
/// method corresponds to one WASM import across four import-module
/// namespaces (`loading`, `runtimeControl`, `ui`, `gameSaving`,
/// `console`). All pointer / length arguments are `i32` because the
/// guest compiles to `wasm32` — pointers into the guest's linear
/// memory are 32-bit unsigned, passed through the host ABI as signed
/// i32 for wasmi's `IntoFunc` machinery.
///
/// Implementations must translate guest-memory offsets through the
/// `wasmi::Caller`'s exported memory before dereferencing. That
/// translation is not done at the trait layer because each import
/// has a distinct access pattern (read-only WAD dump, pixel blit,
/// save-game round-trip, UTF-8 message copy) and consolidating it
/// here would prematurely commit to a shape the real impl hasn't
/// measured yet. The trampolines in [`bind_doom_imports`] call each
/// trait method with the raw i32 args; memory access lives in the
/// impl.
pub trait DoomHost {
    // --- loading -----------------------------------------------------

    /// `loading.onGameInit`. Invoked once during `D_DoomMain` so the
    /// host can bring up the WAD pipeline before the guest calls
    /// `readWads`. The header passes the framebuffer dimensions in
    /// pixels (`width`, `height`) — under our build that's 640x400
    /// matching `BackBuffer::blit_doom_frame`'s expectations.
    fn on_game_init(&mut self, width: i32, height: i32);

    /// `loading.wadSizes`. Guest asks the host how much memory to
    /// reserve for the WAD payload. Convention: the trampoline owns
    /// guest-memory translation, so the trait method returns the two
    /// values and the trampoline writes them back to the guest's
    /// out-pointers. Tuple is `(num_wads, total_bytes_in_all_wads)`
    /// — `int32_t numberOfWads` and `size_t numberOfTotalBytesInAllWads`
    /// in the C header, both 4 bytes in wasm32. Returning a count of 0
    /// signals the guest to fall back to its built-in shareware WAD.
    fn wad_sizes(&mut self) -> (u32, u32);

    /// `loading.readWads`. Guest has allocated a destination buffer
    /// (sized to `total_bytes` from `wad_sizes`) and a per-WAD length
    /// array (sized to `num_wads` × 4 bytes). Convention: the
    /// trampoline owns guest-memory translation — it forms host-side
    /// `&mut [u8]` / `&mut [i32]` slices for the trait method to fill,
    /// then copies both back into guest memory at the out-pointers.
    /// `wad_out` receives the concatenated WAD bytes; `lengths_out`
    /// receives the per-WAD byte length so the guest can index into
    /// `wad_out`.
    fn read_wads(&mut self, wad_out: &mut [u8], lengths_out: &mut [i32]);

    // --- runtimeControl ---------------------------------------------

    /// `runtimeControl.timeInMilliseconds`. Doom's game loop
    /// accumulates tics against an ms clock — 35 tics/sec expected.
    /// Header declares this as `uint64_t` (monotonically non-
    /// decreasing); we mirror with `i64` since wasmi's host-func ABI
    /// is signed but the bit pattern round-trips. Implementations
    /// that don't have a monotonic ms source available may return 0
    /// — Doom's loop tolerates a stuck clock by simply not advancing
    /// tics, which keeps the WASM module instantiable and lets the
    /// guest reach `drawFrame` for visual smoke.
    fn time_in_milliseconds(&mut self) -> i64;

    // --- ui ---------------------------------------------------------

    /// `ui.drawFrame`. Once per tic the guest calls this with its
    /// 640x400 ARGB framebuffer (DOOMGENERIC_RESX x DOOMGENERIC_RESY
    /// x 4 bytes, `[B, G, R, A]` in memory — the little-endian form
    /// of Doom's `0xAARRGGBB`, total `1_024_000` bytes). The
    /// [`bind_doom_imports`] trampoline resolves the guest `memory`
    /// export, copies the frame slice out of linear memory into a
    /// host-side buffer, and passes it here as `&[u8]`. The host
    /// blits it onto the linear framebuffer via
    /// `framebuffer::with_back` + `BackBuffer::blit_doom_frame`
    /// (02bdae1), then presents.
    ///
    /// Signature takes the already-copied byte slice rather than the
    /// raw guest pointer so the trait stays focused on "what to do
    /// with the frame" — guest-memory translation lives in the
    /// trampoline, where the `wasmi::Caller` is in scope.
    fn draw_frame(&mut self, frame: &[u8]);

    // --- gameSaving -------------------------------------------------

    /// `gameSaving.sizeOfSaveGame`. Guest queries the host-persisted
    /// save-slot length for the given `gamemap` (which level the save
    /// is for) before calling `readSaveGame`. Returns 0 if no save
    /// exists, which lets the guest skip the read entirely.
    fn size_of_save_game(&mut self, gamemap: i32) -> i32;

    /// `gameSaving.readSaveGame`. Guest has allocated a buffer of the
    /// size returned by `sizeOfSaveGame(gamemap)` and passes its
    /// offset. Convention: the trampoline owns guest-memory
    /// translation — it forms a host-side `&mut [u8]` slice for the
    /// trait method to fill, then copies the bytes back into guest
    /// memory. Returns the number of bytes actually written
    /// (`size_t` in the C header, fits an i32 since it is bounded by
    /// the slice length).
    fn read_save_game(&mut self, gamemap: i32, out: &mut [u8]) -> i32;

    /// `gameSaving.writeSaveGame`. Guest asks the host to persist a
    /// save-game payload for `gamemap`. Convention: the trampoline
    /// owns guest-memory translation — it copies `data_len` bytes
    /// out of guest memory at `data_ptr` into a host-side `&[u8]`
    /// before calling. Returns bytes persisted (0 if unsupported,
    /// per the header's "0 if unsupported" contract).
    fn write_save_game(&mut self, gamemap: i32, data: &[u8]) -> i32;

    // --- console ----------------------------------------------------

    /// `console.onInfoMessage`. Guest's `I_Printf` / `DEH_printf` /
    /// game-logic status messages. UTF-8 bytes at `ptr..ptr+len` in
    /// guest memory; the [`bind_doom_imports`] trampoline resolves
    /// the guest `memory` export, copies the slice out, validates
    /// UTF-8 (silent drop on decode failure — Doom text is
    /// canonically ASCII), and hands the decoded `&str` here. Mirror
    /// to the kernel console.
    ///
    /// Signature matches `draw_frame`'s shape: the trampoline owns
    /// guest-memory translation; the impl just writes to serial.
    fn on_info_message(&mut self, message: &str);

    /// `console.onErrorMessage`. Guest's `I_Error` — fatal path. Same
    /// trampoline-owned UTF-8 decode as `on_info_message`. The host
    /// prints the message then lets the guest `unreachable` / abort;
    /// the `call_indirect` trap surfaces back up through the wasmi
    /// `Result`.
    fn on_error_message(&mut self, message: &str);
}

/// Default in-kernel implementation of [`DoomHost`]. Side-effect
/// imports that aren't yet wired panic with a descriptive message so
/// that an early guest call into an unimplemented import surfaces as
/// a visible kernel panic (caught by the `entry_uefi.rs` panic
/// handler's raw-COM1 writer) rather than silently returning. Pure-
/// query imports return zero, which is a safe "no data" signal to
/// the guest (Doom checks `sizeOfSaveGame() == 0` and skips the load
/// path).
///
/// Wave status:
///   * `draw_frame` — live (#373). Calls `framebuffer::with_back` +
///     `BackBuffer::blit_doom_frame` + `framebuffer::present`; a
///     no-op when the framebuffer driver isn't installed.
///   * `on_game_init` — live (#384). Prints a one-line `doom: game
///     init` marker to serial so the boot log shows the guest
///     reaching `D_DoomMain`. Now receives `(width, height)` per
///     doom_wasm.h header; the host already knows the framebuffer
///     dimensions so the values are observational.
///   * `on_info_message` / `on_error_message` — live (#384). Route
///     UTF-8 bytes out of guest memory to the kernel serial console
///     with `doom: info:` / `doom: ERROR:` prefixes. Silent drop on
///     malformed UTF-8 so a corrupt message can't itself trap Doom.
///   * `time_in_milliseconds` — live (#374). Signature widened to
///     `i64` (#395) to match the header's `uint64_t` declaration;
///     body now returns `crate::arch::time::now_ms() as i64`,
///     reading the UEFI arch arm's PIT 1 kHz IRQ counter that
///     Track AA landed in commit `be9320d` (#379). The smoke
///     verified the counter advances 0 -> 10 across a 10 ms wait,
///     so Doom's 35 Hz tic accumulator will see real progress as
///     soon as #376 instantiates the guest. The `u64 -> i64` cast
///     is bit-pattern-preserving below `i64::MAX` ms (~292 million
///     years), well clear of any realistic uptime.
///   * `size_of_save_game` / `read_save_game` / `write_save_game` —
///     live (#375). Persist save slots into a reserved sub-range of
///     the virtio-blk disk via `block_storage::reserve_region`
///     (commit ad2889c). 64 slots × 65 sectors per slot, based at
///     sector 1024 — well clear of the #337 checkpoint at sector 0.
///     Each slot is one header sector ("DOOMSAV1" magic + length +
///     CRC-32) followed by up to 64 data sectors (32 KiB cap, ample
///     for Doom's typical sub-1-KiB save shape). Magic / CRC failure
///     reads as length-0 ("no save"), so a fresh disk or a torn write
///     surface as cleanly as a missing slot. See "Save-slot
///     persistence (#375)" section below for the full layout.
///
/// The real impl is filled in incrementally alongside the
/// doomgeneric-wasm module landing:
///   * `wad_sizes` / `read_wads` — live (#383). Feed the baked
///     `DOOM_WAD` bytes (DOOM 1 Shareware v1.9 IWAD, sourced from
///     the id Software 1993 shareware release and baked via
///     build.rs -> `$OUT_DIR/doom_wad.rs`, re-exported through
///     `crate::doom_wad::DOOM_WAD`) into the guest. When the WAD is
///     absent (fresh clone that skipped the binary stage) the trait
///     falls back to `(0, 0)`, which per doom_wasm.h "If
///     numberOfWads remains 0, Doom loads shareware WAD" makes the
///     guest use the copy embedded in its own rodata (jacobenget/
///     doom.wasm v0.1.0 ships the Shareware WAD inline — see
///     doom_assets/README.md's "WAD bundling" note).
pub struct KernelDoomHost;

impl KernelDoomHost {
    /// Construct a fresh host. No initialization state yet — each
    /// impl method will stand up whatever subsystem it fronts on
    /// first call. Kept as an explicit constructor so future growth
    /// (embedded WAD offsets, save-slot allocator, etc.) has a
    /// single entry point.
    pub const fn new() -> Self {
        Self
    }
}

impl Default for KernelDoomHost {
    fn default() -> Self {
        Self::new()
    }
}

impl DoomHost for KernelDoomHost {
    fn on_game_init(&mut self, _width: i32, _height: i32) {
        // Observational — the guest is telling us it's alive and has
        // begun `D_DoomMain`. Print one line so the serial log shows
        // the handoff. `width`/`height` arrive from the guest's view
        // of the framebuffer dimensions; under our build the host
        // already configures `BackBuffer::blit_doom_frame` for
        // 640x400 BGRA so the values are observational only — a
        // mismatch here would surface as a frame-size assertion on
        // the next `draw_frame` call rather than something this stub
        // needs to reject.
        crate::println!("doom: game init");
    }

    fn wad_sizes(&mut self) -> (u32, u32) {
        // #383. Report the baked `DOOM_WAD` (DOOM 1 Shareware v1.9
        // IWAD, ~4 MiB) when build.rs managed to stage the binary,
        // otherwise fall through to `(0, 0)` which — per doom_wasm.h
        // "If numberOfWads remains 0, Doom loads shareware WAD" —
        // tells the guest engine to use the WAD embedded in its own
        // rodata (the jacobenget/doom.wasm binary carries a copy of
        // the same Shareware WAD; see doom_assets/README.md for the
        // internal-vs-external tradeoff).
        //
        // Contract per the trampoline in `bind_doom_imports`
        // (verified against the `wadSizes` func_wrap above):
        //   * tuple.0 = number of WADs (here: 1 for the single IWAD,
        //                or 0 for the absent-binary fallback).
        //   * tuple.1 = total bytes across all WADs (= DOOM_WAD.len()
        //                because we only ship one WAD).
        //
        // `read_wads` below MUST be kept in sync with these numbers
        // — the trampoline calls `wad_sizes` twice (once for the
        // guest's `wadSizes` import, once to size the scratch buffers
        // before `readWads`), so drift between the two calls would
        // surface as the trampoline allocating the wrong length and
        // either truncating the WAD or over-reading into garbage
        // memory.
        if crate::doom_wad::DOOM_WAD.is_empty() {
            (0, 0)
        } else {
            (1, crate::doom_wad::DOOM_WAD.len() as u32)
        }
    }

    fn read_wads(&mut self, wad_out: &mut [u8], lengths_out: &mut [i32]) {
        // #383. Copy the baked IWAD bytes into the trampoline-formed
        // scratch buffer and record its length in the per-WAD length
        // array. The trampoline sizes `wad_out` to the `total_bytes`
        // returned by `wad_sizes` and `lengths_out` to `num_wads`;
        // since we report (1, DOOM_WAD.len()) both slices should be
        // exactly the right shape — a length mismatch here would
        // indicate the trampoline's two `wad_sizes` calls disagreed,
        // which shouldn't happen with a pure-query impl but is
        // guarded against below via `copy_from_slice`'s length-check
        // behaviour (panics loudly rather than writing garbage).
        //
        // When the WAD is absent (fresh clone) the slices are empty
        // and both copies are no-ops — matching the `(0, 0)` return
        // from `wad_sizes`.
        let wad = crate::doom_wad::DOOM_WAD;
        if wad.is_empty() {
            return;
        }
        // Guard against a trampoline that under-allocated. In
        // practice the two `wad_sizes` calls inside the trampoline
        // return identical values, so these conditions shouldn't fire
        // — but a silent truncate is worse than a visible drop for a
        // pipeline bug, so bail cleanly rather than copying a
        // partial WAD the guest would then misparse.
        if wad_out.len() < wad.len() || lengths_out.is_empty() {
            return;
        }
        wad_out[..wad.len()].copy_from_slice(wad);
        lengths_out[0] = wad.len() as i32;
    }

    fn time_in_milliseconds(&mut self) -> i64 {
        // Header declares `uint64_t timeInMilliseconds`; the trait
        // returns `i64` so the wasmi host-func ABI marshals a 64-bit
        // value. Doom's game loop accumulates tics against this
        // clock at 35 Hz and only cares about monotonic deltas, so
        // any monotonic ms source suffices — we use the UEFI arch
        // arm's PIT 1 kHz IRQ counter, exposed as
        // `arch::time::now_ms() -> u64` (Track AA, commit be9320d /
        // #379). The smoke verified the counter advances 0 -> 10
        // across a 10 ms wait, so the 35 Hz tic accumulator inside
        // Doom will see real progress as soon as the guest is
        // instantiated (#376).
        //
        // The `u64 -> i64` cast is bit-pattern-preserving for any
        // clock value below `i64::MAX` ms (~292 million years), so
        // any realistic uptime round-trips through wasmi's signed
        // ABI back to the guest's `uint64_t` cleanly.
        crate::arch::time::now_ms() as i64
    }

    fn draw_frame(&mut self, frame: &[u8]) {
        // #373. Frame is the 640x400 BGRA buffer Doom wrote — already
        // copied out of guest linear memory by the trampoline in
        // [`bind_doom_imports`]. Hand it to the framebuffer driver's
        // Doom-shaped blit path (9c4984d unblocked both 3bpp BIOS and
        // 4bpp UEFI GOP), then present so the dirty rect lands on the
        // front buffer the firmware / display is reading.
        //
        // `with_back` returns `None` when the framebuffer driver
        // wasn't installed (text-mode boot, or pre-GOP init path);
        // the `?`-style `Option` return threads that through as a
        // silent no-op, matching `framebuffer::present`'s behaviour.
        // This keeps the host-shim safe to wire before the
        // framebuffer subsystem is online — the imports all resolve,
        // the guest's `drawFrame` calls just don't render until the
        // display surface is up.
        crate::framebuffer::with_back(|back| back.blit_doom_frame(frame));
        crate::framebuffer::present();
    }

    fn size_of_save_game(&mut self, gamemap: i32) -> i32 {
        // #375. Resolve the slot, read its one-sector header, validate
        // the magic + CRC, and return the recorded length. Any failure
        // along the path (slot index out of range, region reserve
        // refused because the disk is unmounted or undersized, header
        // I/O failure, magic mismatch, CRC mismatch) collapses to 0 —
        // the guest interprets 0 as "no save present for this gamemap"
        // and skips the matching `read_save_game` call entirely, which
        // is the safe behavior whether the disk is genuinely empty or
        // we just can't read it. Errors are silent (no `crate::println`
        // chatter) because Doom calls this every time the player opens
        // the load menu and a noisy log on a fresh disk would drown
        // the rest of boot.
        match read_save_header(gamemap) {
            Some(header) => header.length as i32,
            None => 0,
        }
    }

    fn read_save_game(&mut self, gamemap: i32, out: &mut [u8]) -> i32 {
        // #375. Two-phase read:
        //   1. Read + validate the header sector. Bail with 0 on any
        //      magic / CRC failure (matches `size_of_save_game`'s
        //      contract — if it would have returned 0, this returns
        //      0 too, even if the guest calls through anyway).
        //   2. Read the data sectors covering `header.length` bytes
        //      into a sector-sized scratch vec, verify CRC32, and
        //      copy `min(header.length, out.len())` bytes into `out`.
        //
        // RegionHandle::read takes whole-sector buffers, so we round
        // `header.length` up to the next 512-byte boundary for the
        // scratch allocation, then truncate when copying out. The
        // 64-sector cap on slot data (`SAVE_DATA_SECTORS`) bounds the
        // allocation at 32 KiB.
        let header = match read_save_header(gamemap) {
            Some(h) => h,
            None => return 0,
        };
        let region = match save_slot_region(gamemap) {
            Some(r) => r,
            None => return 0,
        };
        let length = header.length as usize;
        if length == 0 {
            return 0;
        }
        let data_sectors = length.div_ceil(BLOCK_SECTOR_SIZE);
        if data_sectors as u64 > SAVE_DATA_SECTORS {
            return 0;
        }
        let mut data = vec![0u8; data_sectors * BLOCK_SECTOR_SIZE];
        if region.read(SAVE_HEADER_SECTOR_OFFSET + 1, &mut data).is_err() {
            return 0;
        }
        // Verify CRC over the in-range data. A mismatch here means the
        // header was clean but the data sectors are torn — treat as
        // "no save".
        if crc32_bytes(&data[..length]) != header.crc32 {
            return 0;
        }
        let to_copy = core::cmp::min(length, out.len());
        out[..to_copy].copy_from_slice(&data[..to_copy]);
        to_copy as i32
    }

    fn write_save_game(&mut self, gamemap: i32, data: &[u8]) -> i32 {
        // #375. Build the header (magic + length + CRC32 over `data`),
        // write the data sectors first, then the header sector, then
        // flush. Header-last ordering mirrors `block_storage::checkpoint`
        // (#337): a torn write that loses the header leaves the slot
        // with a stale magic / mismatched CRC, so the next read falls
        // back to "no save" rather than surfacing inconsistent bytes.
        //
        // Returns `data.len()` on full success, 0 on any I/O failure.
        // The guest's contract for write is "bytes persisted (0 if
        // unsupported)" — anything short of a full success is reported
        // as 0 so Doom can surface a visible save-failed error rather
        // than spinning on a partial write.
        if data.len() > SAVE_DATA_BYTES_MAX {
            return 0;
        }
        let region = match save_slot_region(gamemap) {
            Some(r) => r,
            None => return 0,
        };
        // Round data up to whole sectors with zero padding. The CRC is
        // taken over the original `data` (not the padded buffer) so a
        // future shorter save in the same slot won't accidentally
        // CRC-collide with stale tail bytes.
        let data_sectors = if data.is_empty() {
            0
        } else {
            data.len().div_ceil(BLOCK_SECTOR_SIZE)
        };
        if data_sectors > 0 {
            let mut padded = vec![0u8; data_sectors * BLOCK_SECTOR_SIZE];
            padded[..data.len()].copy_from_slice(data);
            if region
                .write(SAVE_HEADER_SECTOR_OFFSET + 1, &padded)
                .is_err()
            {
                return 0;
            }
        }
        let header = SaveHeader {
            magic: *SAVE_MAGIC,
            length: data.len() as u32,
            crc32: crc32_bytes(data),
        };
        let mut header_sector = [0u8; BLOCK_SECTOR_SIZE];
        header.encode(&mut header_sector);
        if region
            .write(SAVE_HEADER_SECTOR_OFFSET, &header_sector)
            .is_err()
        {
            return 0;
        }
        if region.flush().is_err() {
            return 0;
        }
        data.len() as i32
    }

    fn on_info_message(&mut self, message: &str) {
        // Mirror the guest's `I_Printf` / `DEH_printf` line to the
        // kernel serial console with a `doom: info: ` prefix so the
        // log stream stays grep-able when Doom output is interleaved
        // with other kernel chatter. The trampoline in
        // [`bind_doom_imports`] has already copied the bytes out of
        // guest linear memory and validated UTF-8, so all we do here
        // is forward.
        crate::println!("doom: info: {}", message);
    }

    fn on_error_message(&mut self, message: &str) {
        // Same path as `on_info_message` but louder prefix — Doom's
        // `I_Error` is the fatal shutdown funnel (bad WAD lump,
        // failed level load, assertion trip), and when it fires the
        // guest typically follows with a `call_indirect`-to-
        // unreachable trap that surfaces through wasmi as a runtime
        // error. Visibility matters more here than stream
        // separation, so we route to the same serial writer as info
        // messages — just with `ERROR` where a structured log shim
        // would later set a severity level.
        crate::println!("doom: ERROR: {}", message);
    }
}

/// Register all 10 Doom host-shim imports against a `Linker<T>` where
/// `T` is the store-data type implementing [`DoomHost`]. Each import
/// is looked up under its WASM module namespace
/// (`loading` / `runtimeControl` / `ui` / `gameSaving` / `console`)
/// and name, wrapped via `Linker::func_wrap` so wasmi's `IntoFunc`
/// machinery handles the parameter / return marshaling.
///
/// The function-type signatures here are reconciled against the
/// authoritative jacobenget/doom.wasm v0.1.0 (commit dc94345)
/// `doom_wasm.h` header at 24bb772 — `wasmi::Linker::instantiate`
/// matches imports by exact `(params, results)` shape, so any drift
/// from the binary contract surfaces as `LinkerError::CannotFindDefinitionForImport`
/// at instantiate time. Most imports are `(i32, ...)`-shaped because
/// guest pointers / `int32_t` / `size_t` are all 4 bytes in wasm32;
/// `runtimeControl.timeInMilliseconds` is the lone i64 (header
/// declares it `uint64_t`).
///
/// Call once at module-instantiate time, AFTER `linker` is
/// constructed and BEFORE `linker.instantiate(&mut store, &module)`.
/// Expected call-site shape (reached from kernel_run once the
/// doomgeneric-wasm payload is loaded):
///
/// ```ignore
/// let engine = wasmi::Engine::default();
/// let module = wasmi::Module::new(&engine, DOOM_WASM)?;
/// let mut store = wasmi::Store::new(&engine, KernelDoomHost::new());
/// let mut linker = wasmi::Linker::<KernelDoomHost>::new(&engine);
/// bind_doom_imports(&mut linker);
/// let instance = linker.instantiate(&mut store, &module)?.start(&mut store)?;
/// // ...drive instance.get_typed_func::<_, _>("_start") etc.
/// ```
///
/// The `expect` calls on each `func_wrap` catch the double-
/// registration case — `LinkerError::DuplicateDefinition` — which is
/// a programmer error (someone called `bind_doom_imports` twice on
/// the same linker). Not recoverable at runtime; the panic here is
/// louder than a silently-ignored second registration.
pub fn bind_doom_imports<T: DoomHost + 'static>(linker: &mut Linker<T>) {
    // loading.*
    linker
        .func_wrap(
            "loading",
            "onGameInit",
            |mut caller: wasmi::Caller<'_, T>, width: i32, height: i32| {
                caller.data_mut().on_game_init(width, height);
            },
        )
        .expect("doom: duplicate import loading.onGameInit");
    // loading.wadSizes — `(int32_t *numberOfWads, size_t *numberOfTotalBytesInAllWads) -> ()`.
    // The trait method returns the pair; this trampoline writes both
    // 32-bit values back into guest memory at the supplied out-
    // pointers. Memory-export resolution and write failures are silent
    // no-ops for parity with `ui.drawFrame` / `console.*` — a
    // mid-call error surfaces as the guest seeing a stale (zero-
    // initialized) out-buffer, which falls through to the shareware
    // fallback Doom already handles cleanly.
    linker
        .func_wrap(
            "loading",
            "wadSizes",
            |mut caller: wasmi::Caller<'_, T>, num_wads_ptr: i32, total_bytes_ptr: i32| {
                let (num_wads, total_bytes) = caller.data_mut().wad_sizes();
                let memory = match caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                {
                    Some(m) => m,
                    None => return,
                };
                let _ = memory.write(
                    &mut caller,
                    num_wads_ptr as usize,
                    &num_wads.to_le_bytes(),
                );
                let _ = memory.write(
                    &mut caller,
                    total_bytes_ptr as usize,
                    &total_bytes.to_le_bytes(),
                );
            },
        )
        .expect("doom: duplicate import loading.wadSizes");
    // loading.readWads — `(uint8_t *wadDataDestination, int32_t *byteLengthOfEachWad) -> ()`.
    // The trampoline forms host-side scratch slices, lets the trait
    // method fill them, then writes both back into guest memory. While
    // the impl is stubbed (#383), `wad_sizes` reports zero WADs / zero
    // bytes so the slices are empty and both writes are no-ops; the
    // function still has to exist with the right `(i32, i32) -> ()`
    // type for `wasmi::Linker::instantiate` to accept the module.
    linker
        .func_wrap(
            "loading",
            "readWads",
            |mut caller: wasmi::Caller<'_, T>, wad_buf_ptr: i32, lengths_ptr: i32| {
                // Mirror `wad_sizes` to size the host scratch buffers.
                // While the stub returns (0, 0) both slices are empty;
                // once #383 lands the trampoline will need to
                // remember the prior `wad_sizes` answer (a
                // `Cell`-style cache on the host) instead of re-
                // querying — the contract is that `wad_sizes` is
                // always called first by Doom, so a re-query here
                // matches the eventual real answer.
                let (num_wads, total_bytes) = caller.data_mut().wad_sizes();
                let mut wad_buf = vec![0u8; total_bytes as usize];
                let mut lengths_buf = vec![0i32; num_wads as usize];
                caller.data_mut().read_wads(&mut wad_buf, &mut lengths_buf);
                let memory = match caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                {
                    Some(m) => m,
                    None => return,
                };
                let _ = memory.write(&mut caller, wad_buf_ptr as usize, &wad_buf);
                // `int32_t` array: serialize little-endian since
                // wasm is fixed-LE.
                let mut lengths_le: Vec<u8> = Vec::with_capacity(lengths_buf.len() * 4);
                for n in &lengths_buf {
                    lengths_le.extend_from_slice(&n.to_le_bytes());
                }
                let _ = memory.write(&mut caller, lengths_ptr as usize, &lengths_le);
            },
        )
        .expect("doom: duplicate import loading.readWads");

    // runtimeControl.*
    // `uint64_t timeInMilliseconds(void)` — mirrored as `() -> i64` on
    // the wasmi side; the trait returns `i64` and the host clock is
    // `arch::time::now_ms() as i64`.
    linker
        .func_wrap(
            "runtimeControl",
            "timeInMilliseconds",
            |mut caller: wasmi::Caller<'_, T>| -> i64 {
                caller.data_mut().time_in_milliseconds()
            },
        )
        .expect("doom: duplicate import runtimeControl.timeInMilliseconds");

    // ui.*
    //
    // drawFrame is the one import where the trampoline itself has to
    // reach into guest linear memory — the guest passes a pointer to
    // its 640x400 BGRA framebuffer (1_024_000 bytes) and the host has
    // to translate that offset through the wasmi `Caller`'s
    // `memory` export before it can hand bytes to
    // `DoomHost::draw_frame`. The trampoline:
    //   1. resolves the guest `memory` export (silent no-op if the
    //      module has no memory — treat like a frame the host didn't
    //      see, which keeps the linker tolerant of scaffold guests),
    //   2. copies the frame slice out into a host-side `Vec<u8>`
    //      using `Memory::read` — ~1 MB per call, 35x per second
    //      under Doom's tic rate (~35 MB/s allocation churn; fine
    //      for the UEFI boot-time scaffold, pool-able later if the
    //      alloc cost shows up in a profile),
    //   3. dispatches to the trait method with the copied bytes.
    // Guest-memory access failures (out-of-range pointer, missing
    // memory export) are treated as "skip this frame" rather than a
    // trap — the next tic will try again and the host-side screen
    // just stalls one frame. Louder error routing can grow in once
    // the `onErrorMessage` path is wired (#376 follow-up).
    linker
        .func_wrap(
            "ui",
            "drawFrame",
            |mut caller: wasmi::Caller<'_, T>, frame_ptr: i32| {
                const DOOM_FRAME_LEN: usize = 640 * 400 * 4;
                let memory = match caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                {
                    Some(m) => m,
                    None => return,
                };
                let mut frame = vec![0u8; DOOM_FRAME_LEN];
                if memory
                    .read(&caller, frame_ptr as usize, &mut frame)
                    .is_err()
                {
                    return;
                }
                caller.data_mut().draw_frame(&frame);
            },
        )
        .expect("doom: duplicate import ui.drawFrame");

    // gameSaving.*
    // `size_t sizeOfSaveGame(int32_t gameSaveId)` — `(i32) -> i32`
    // (`size_t` is 4 bytes in wasm32). The `gamemap` argument is
    // which level the save is for; the trait method does the lookup
    // and returns the byte length (or 0 for "no save").
    linker
        .func_wrap(
            "gameSaving",
            "sizeOfSaveGame",
            |mut caller: wasmi::Caller<'_, T>, gamemap: i32| -> i32 {
                caller.data_mut().size_of_save_game(gamemap)
            },
        )
        .expect("doom: duplicate import gameSaving.sizeOfSaveGame");
    // `size_t readSaveGame(int32_t gameSaveId, uint8_t *dataDestination)`
    // — `(i32, i32) -> i32`. Trampoline forms a host-side scratch
    // buffer sized to the host's prior `size_of_save_game(gamemap)`
    // answer, lets the trait method fill it, then copies the written
    // bytes back into guest memory. Returns the byte count actually
    // written (`size_t` per the header). While #375 is open the trait
    // returns 0 so the buffer is empty and the write is a no-op.
    linker
        .func_wrap(
            "gameSaving",
            "readSaveGame",
            |mut caller: wasmi::Caller<'_, T>, gamemap: i32, out_ptr: i32| -> i32 {
                let len = caller.data_mut().size_of_save_game(gamemap);
                let len_usize = match usize::try_from(len) {
                    Ok(v) => v,
                    Err(_) => return 0,
                };
                let mut buf = vec![0u8; len_usize];
                let written = caller.data_mut().read_save_game(gamemap, &mut buf);
                let written_usize = usize::try_from(written).unwrap_or(0).min(buf.len());
                let memory = match caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                {
                    Some(m) => m,
                    None => return 0,
                };
                if memory
                    .write(&mut caller, out_ptr as usize, &buf[..written_usize])
                    .is_err()
                {
                    return 0;
                }
                written
            },
        )
        .expect("doom: duplicate import gameSaving.readSaveGame");
    // `size_t writeSaveGame(int32_t gameSaveId, uint8_t *data, size_t length)`
    // — `(i32, i32, i32) -> i32`. Trampoline copies `length` bytes
    // out of guest memory at `data_ptr` into a host-side `Vec<u8>`,
    // then hands the slice to the trait method. Returns the byte
    // count actually persisted (or 0 if unsupported, per the header).
    linker
        .func_wrap(
            "gameSaving",
            "writeSaveGame",
            |mut caller: wasmi::Caller<'_, T>,
             gamemap: i32,
             data_ptr: i32,
             data_len: i32|
             -> i32 {
                let len = match usize::try_from(data_len) {
                    Ok(v) => v,
                    Err(_) => return 0,
                };
                let memory = match caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                {
                    Some(m) => m,
                    None => return 0,
                };
                let mut buf = vec![0u8; len];
                if memory
                    .read(&caller, data_ptr as usize, &mut buf)
                    .is_err()
                {
                    return 0;
                }
                caller.data_mut().write_save_game(gamemap, &buf)
            },
        )
        .expect("doom: duplicate import gameSaving.writeSaveGame");

    // console.*
    //
    // Both onInfoMessage and onErrorMessage are (ptr, len) pairs into
    // guest linear memory carrying UTF-8 bytes. Same trampoline
    // shape as `ui.drawFrame`:
    //   1. resolve guest `memory` export (silent no-op if absent —
    //      matches drawFrame's tolerance for scaffold guests),
    //   2. copy `len` bytes into a host-side `Vec<u8>` via
    //      `Memory::read` (messages are short, typically <256 bytes,
    //      so allocation cost is trivial),
    //   3. validate UTF-8 with `core::str::from_utf8`; silently drop
    //      on decode failure — Doom text is canonically ASCII and
    //      the `onErrorMessage` path especially must not itself trap
    //      on malformed bytes,
    //   4. dispatch to the trait method with the decoded `&str`.
    // A negative `len` (guest bug) would wrap to a huge usize via
    // `as usize`; clamp via `try_into`, dropping on conversion
    // failure so a malformed call doesn't stall the kernel on a
    // 4 GiB allocation.
    linker
        .func_wrap(
            "console",
            "onInfoMessage",
            |mut caller: wasmi::Caller<'_, T>, ptr: i32, len: i32| {
                let Ok(len) = usize::try_from(len) else { return };
                let memory = match caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                {
                    Some(m) => m,
                    None => return,
                };
                let mut buf = vec![0u8; len];
                if memory.read(&caller, ptr as usize, &mut buf).is_err() {
                    return;
                }
                let Ok(message) = core::str::from_utf8(&buf) else { return };
                caller.data_mut().on_info_message(message);
            },
        )
        .expect("doom: duplicate import console.onInfoMessage");
    linker
        .func_wrap(
            "console",
            "onErrorMessage",
            |mut caller: wasmi::Caller<'_, T>, ptr: i32, len: i32| {
                let Ok(len) = usize::try_from(len) else { return };
                let memory = match caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                {
                    Some(m) => m,
                    None => return,
                };
                let mut buf = vec![0u8; len];
                if memory.read(&caller, ptr as usize, &mut buf).is_err() {
                    return;
                }
                let Ok(message) = core::str::from_utf8(&buf) else { return };
                caller.data_mut().on_error_message(message);
            },
        )
        .expect("doom: duplicate import console.onErrorMessage");
}

// ── Save-slot persistence (#375) ────────────────────────────────────
//
// Doom's save imports (`sizeOfSaveGame` / `readSaveGame` /
// `writeSaveGame`) land here. The on-disk layout is a fixed-size table
// reserved out of the virtio-blk disk via
// `block_storage::reserve_region` (#394 / commit ad2889c) so it sits
// well above the #337 checkpoint footprint at sector 0..N.
//
// Layout:
//
//   Slot stride    = 65 sectors  = 32.5 KiB  (1 header + 64 data).
//   Slot count     = 64.
//   Base sector    = 1024.
//   Footprint      = 1024 + 64*65 = 5184 sectors = 2.59 MiB.
//
// The 2.59 MiB total fits inside the harness's current 8 MiB virtio-blk
// disk image and well clear of the checkpoint body's practical upper
// bound (the kernel's checkpoint state is KB-scale today; sector 1024
// leaves ~512 KiB of headroom for checkpoint growth before we collide,
// per the module-level "rule of thumb" in `block_storage`).
//
// 32 KiB per slot is generous: Doom's save shape is typically <1 KiB
// (one player, current level state, a few flags). Anything that grows
// past 32 KiB is a malformed save and is rejected with 0.
//
// Per-slot header (one sector, only the first 16 bytes are populated;
// the rest is zero-padded so a torn previous-write doesn't leak into
// the new header on a partial commit):
//
//   bytes  0..8   — magic "DOOMSAV1" (distinguishes from the #337
//                   `AREST-K1` checkpoint magic so a misdirected read
//                   fails fast).
//   bytes  8..12  — `length: u32 LE`  — actual save-data byte count.
//   bytes 12..16  — `crc32:  u32 LE`  — CRC-32/IEEE over the first
//                   `length` data bytes.
//   bytes 16..512 — zero.
//
// CRC and magic are validated together: a mismatch on either is treated
// as "no save" and the slot reads as length-0. This trades the
// possibility of silently dropping a torn save for the safety of never
// surfacing inconsistent bytes to Doom.
//
// Round-trip example (sanity-check the arithmetic):
//   `write_save_game(5, &[1, 2, 3])`:
//     * slot 5 base sector  = 1024 + 5 * 65 = 1349.
//     * data sector         = 1350 (offset 1 inside the region).
//     * header sector       = 1349 (offset 0 inside the region).
//     * `padded` = [1, 2, 3, 0, 0, ..., 0] (512 bytes); written.
//     * header = { magic="DOOMSAV1", length=3, crc32=CRC32([1,2,3]) }.
//     * header sector written, region flushed.
//     * returns 3.
//   `read_save_game(5, &mut out)` afterwards:
//     * reads header, validates magic + that CRC arithmetic isn't done
//       yet (we only trust crc after data is read);
//     * `data_sectors = ceil(3/512) = 1`, allocates 512-byte buf,
//       region.read(1, buf);
//     * `crc32_bytes(buf[..3]) == header.crc32` — yes;
//     * copies `min(3, out.len())` bytes; returns that count.

/// Magic bytes for the save-slot header. Distinct from the #337
/// `AREST-K1` checkpoint magic so a misdirected read at the wrong
/// sector is recognized and rejected rather than silently parsed.
const SAVE_MAGIC: &[u8; 8] = b"DOOMSAV1";

/// First sector of the Doom save table. Picked well clear of the #337
/// checkpoint footprint (sector 0 + body sectors 1..N).
const SAVE_BASE_SECTOR: u64 = 1024;

/// Number of save slots in the table. Doom only ever uses 0..7
/// (`load1`..`load8` in the menu), but the table is over-provisioned
/// so a future mod that exposes more slots doesn't reshape the disk.
const SAVE_SLOT_COUNT: u64 = 64;

/// Per-slot data sectors (excluding the header sector). 64 sectors =
/// 32 KiB max payload — comfortably above Doom's typical sub-1-KiB
/// save shape.
const SAVE_DATA_SECTORS: u64 = 64;

/// Per-slot stride in sectors: 1 header + `SAVE_DATA_SECTORS` data.
const SAVE_SLOT_STRIDE: u64 = 1 + SAVE_DATA_SECTORS;

/// Header sector offset within a slot's region (always 0 — the header
/// is always the first sector). Named for clarity at the call site.
const SAVE_HEADER_SECTOR_OFFSET: u64 = 0;

/// Maximum save data length in bytes. Anything larger is rejected at
/// `write_save_game` time so we never write a header whose `length`
/// field exceeds the data sector capacity.
const SAVE_DATA_BYTES_MAX: usize = (SAVE_DATA_SECTORS as usize) * BLOCK_SECTOR_SIZE;

/// Build a `RegionHandle` for the given gamemap's save slot, or `None`
/// if the gamemap is out of range or `block_storage::reserve_region`
/// refuses (no virtio-blk device, range past disk capacity).
fn save_slot_region(gamemap: i32) -> Option<RegionHandle> {
    if gamemap < 0 {
        return None;
    }
    let slot = gamemap as u64;
    if slot >= SAVE_SLOT_COUNT {
        return None;
    }
    let base = SAVE_BASE_SECTOR + slot * SAVE_SLOT_STRIDE;
    block_storage::reserve_region(base, SAVE_SLOT_STRIDE).ok()
}

/// Read + validate the header sector for a slot. Returns `None` if the
/// region can't be reserved, the header read fails, the magic doesn't
/// match, or the recorded length exceeds the slot's data capacity.
/// (CRC validation is deferred to `read_save_game` since it needs the
/// data bytes; here we only confirm the header is structurally valid.)
fn read_save_header(gamemap: i32) -> Option<SaveHeader> {
    let region = save_slot_region(gamemap)?;
    let mut sector = [0u8; BLOCK_SECTOR_SIZE];
    region.read(SAVE_HEADER_SECTOR_OFFSET, &mut sector).ok()?;
    let header = SaveHeader::decode(&sector);
    if &header.magic != SAVE_MAGIC {
        return None;
    }
    if header.length as usize > SAVE_DATA_BYTES_MAX {
        return None;
    }
    Some(header)
}

#[derive(Debug, Clone, Copy)]
struct SaveHeader {
    magic: [u8; 8],
    length: u32,
    crc32: u32,
}

impl SaveHeader {
    fn encode(self, buf: &mut [u8; BLOCK_SECTOR_SIZE]) {
        // Zero the entire sector first so the unused 16..512 range is
        // deterministic — keeps the on-disk image reproducible across
        // writes and prevents stale bytes from a torn previous header
        // from looking like meaningful state to a debugging hex-dump.
        buf.fill(0);
        buf[0..8].copy_from_slice(&self.magic);
        buf[8..12].copy_from_slice(&self.length.to_le_bytes());
        buf[12..16].copy_from_slice(&self.crc32.to_le_bytes());
    }

    fn decode(buf: &[u8; BLOCK_SECTOR_SIZE]) -> Self {
        let mut magic = [0u8; 8];
        magic.copy_from_slice(&buf[0..8]);
        let length = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
        let crc32 = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
        Self { magic, length, crc32 }
    }
}

/// CRC-32/IEEE over `data`. Inlined here rather than depending on
/// `crc32fast` because (a) the kernel doesn't already have that crate
/// (verified against `Cargo.toml` at #375 implementation time) and a
/// new dep would touch a file outside this sub-task's ownership, and
/// (b) Doom save data is sub-KiB so a table-free implementation costs
/// microseconds. Algorithm matches `block_storage::crc32` exactly so
/// the Doom save format is verifiable with the same off-line tooling
/// that inspects #337 checkpoints.
fn crc32_bytes(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

// ── Keyboard pump (#377) ────────────────────────────────────────────
//
// Track EE landed `arch::uefi::keyboard` (commit 8d9c14e) — a 64-slot
// ring of `pc_keyboard::DecodedKey` populated by the IRQ 1 handler.
// jacobenget/doom.wasm v0.1.0 takes key events through TWO EXPORTS,
// not imports:
//
//   reportKeyDown(doomKey: i32) -> ()
//   reportKeyUp  (doomKey: i32) -> ()
//
// `doomKey` is in [0, 255]: ASCII for printable characters (a-z is
// 97-122 per the header), and special `KEY_*` constants for everything
// else. The KEY_* values come from doomgeneric/doomkeys.h; the binary
// also re-exports them as i32 const globals so a host could discover
// them at instantiate time, but the header pins the numeric values
// (0xae for KEY_RIGHTARROW, 0xa3 for KEY_FIRE, etc.) so we hard-code
// the subset we need rather than walking globals at boot. This matches
// what the BIOS arm's `arch::x86_64::input::key_code` already does
// (and where the constants below are sourced from).
//
// The mechanism is host-push, not guest-poll: the Doom game loop
// expects `reportKeyDown` / `reportKeyUp` to have updated the engine's
// internal key-state array BEFORE `tickGame` is called for the next
// frame. The intended call order from the per-frame driver in
// `kernel_run` (once #376 lands) is:
//
//   loop {
//       doom::pump_keys_into_guest(&mut store, &instance)?;
//       tick_game.call(&mut store, ())?;
//       // sleep until next 35Hz tick
//   }
//
// ── Press/release fidelity gap ──────────────────────────────────────
//
// The keyboard ring buffer stores `DecodedKey` (post-`pc-keyboard`
// translation), NOT `KeyEvent` (pre-translation, with `KeyState::Down`
// / `KeyState::Up`). That's a one-way collapse — by the time a
// keystroke lands on the ring, the press/release distinction has been
// erased. The BIOS arm sidesteps this by stashing `KeyEvent` directly
// (see `arch::x86_64::input::DoomKeyEvent` — `Pressed(u8)` /
// `Released(u8)`), but the UEFI ring was sized for the #365 REPL pump,
// which only cares about decoded chars.
//
// Workaround in this commit: synthesize a press-then-release pair for
// each `DecodedKey` we drain. Same trick the BIOS arm uses for
// `KeyState::SingleShot` — Doom's input layer treats a paired
// down/up as a momentary tap, which is correct for menu navigation
// (Escape, arrows, Enter) and for any one-frame fire input but WRONG
// for held-key behavior:
//   * holding W/Up to walk forward will instead step one tile per
//     scancode auto-repeat;
//   * holding LCtrl/Fire will fire-once-per-scancode, not full-auto;
//   * holding Shift to run will not actually run.
//
// This is acceptable for a smoke / menu / one-shot demo (#377's goal:
// prove the pipeline lights up), not for full gameplay.
//
// TODO(#377): The proper fix needs Track EE to extend
// `arch::uefi::keyboard` with a parallel `KeyEvent` ring (or a
// per-key state-edge log) so this pump can route real Down/Up
// transitions. Proposed API on the keyboard side:
//
//     pub fn read_key_event() -> Option<pc_keyboard::KeyEvent>;
//     pub fn pending_events() -> usize;
//
// implemented by stashing the raw `KeyEvent` from `kb.add_byte` into a
// second `VecDeque<KeyEvent>` alongside the existing `DecodedKey`
// ring. Once that lands, `pump_keys_into_guest` drops the
// down-then-up synthesis and dispatches each event individually
// against `reportKeyDown` or `reportKeyUp`.

/// Doom key constants (subset of doomgeneric/doomkeys.h reachable from
/// a PS/2 keyboard). Mirrors `arch::x86_64::input::key_code` exactly —
/// kept duplicated here rather than reaching across the BIOS-arm
/// module gate (`arch::x86_64::input` is gated on
/// `not(target_os = "uefi")`, so it isn't reachable from this UEFI
/// build).
///
/// `#[allow(dead_code)]` because some of these constants
/// (KEY_F1..KEY_F10, KEY_PAUSE, KEY_RALT, etc.) are only reached
/// through the `translate_decoded_key` match arms — which Rust's
/// dead-code analysis treats as unreachable until the kernel_run
/// handoff (#376) wires `pump_keys_into_guest`. Mirrors the same
/// `#[allow(dead_code)]` already on `doom_assets.rs::DOOM_WASM` for
/// the exact same "scaffolded for the next sub-task" reason.
#[allow(dead_code)]
mod doom_key {
    pub const KEY_RIGHTARROW: u8 = 0xae;
    pub const KEY_LEFTARROW:  u8 = 0xac;
    pub const KEY_UPARROW:    u8 = 0xad;
    pub const KEY_DOWNARROW:  u8 = 0xaf;
    pub const KEY_USE:        u8 = 0xa2;
    pub const KEY_FIRE:       u8 = 0xa3;
    pub const KEY_ESCAPE:     u8 = 27;
    pub const KEY_ENTER:      u8 = 13;
    pub const KEY_TAB:        u8 = 9;
    pub const KEY_BACKSPACE:  u8 = 0x7f;
    pub const KEY_RSHIFT:     u8 = 0xb6;
    pub const KEY_RALT:       u8 = 0xb8;
    pub const KEY_F1:         u8 = 0xbb;
    pub const KEY_F2:         u8 = 0xbc;
    pub const KEY_F3:         u8 = 0xbd;
    pub const KEY_F4:         u8 = 0xbe;
    pub const KEY_F5:         u8 = 0xbf;
    pub const KEY_F6:         u8 = 0xc0;
    pub const KEY_F7:         u8 = 0xc1;
    pub const KEY_F8:         u8 = 0xc2;
    pub const KEY_F9:         u8 = 0xc3;
    pub const KEY_F10:        u8 = 0xc4;
    pub const KEY_PAUSE:      u8 = 0xff;
}

/// Translate a `pc_keyboard::DecodedKey` into a Doom key byte
/// (`doomKey` in the doom_wasm.h vocabulary). Returns `None` for keys
/// Doom doesn't care about (e.g. caps lock LED toggle, media keys,
/// numpad arithmetic). The BIOS arm has the equivalent table over
/// `KeyEvent::code` in `arch::x86_64::input::translate_keycode`; this
/// one operates on `DecodedKey` because that's what the UEFI keyboard
/// ring stores (Track EE / commit 8d9c14e).
///
/// Per the doom_wasm.h header at jacobenget/doom.wasm@24bb772:
///   "If the key in question naturally produces a printable ASCII
///   character: the `doomKey` associated with the key is the ASCII
///   code for that printable character (e.g. 'a' through 'z' have
///   `doomKey` value 97 through 122)."
///
/// So lowercase letters / digits / symbols pass straight through as
/// their byte. `pc_keyboard::layouts::Us104Key` (the layout the
/// keyboard ring is configured with) emits these as
/// `DecodedKey::Unicode` already.
///
/// Control characters that `Us104Key` emits as `DecodedKey::Unicode`:
///   * Escape -> 0x1B  (matches KEY_ESCAPE = 27)
///   * Tab    -> 0x09  (matches KEY_TAB = 9)
///   * Enter  -> 0x0A  (REMAP -> KEY_ENTER = 13 / 0x0D — the layout
///                       emits LF, Doom expects CR)
///   * Backspace -> 0x08  (REMAP -> KEY_BACKSPACE = 0x7F)
///   * Spacebar -> 0x20  (REMAP -> KEY_USE = 0xA2 — Doom's "use" /
///                         door-open binding)
///
/// Special keys that `Us104Key` emits as `DecodedKey::RawKey`
/// (arrows, modifiers, F-keys) translate via the same table the BIOS
/// arm uses. ASCII letters from the Unicode path are lowercased — the
/// header is explicit that 97-122 (lowercase) is the canonical
/// representation, and Doom's input layer uppercases internally for
/// any binding that cares.
///
/// `#[allow(dead_code)]` because callers used to be limited to
/// `pump_keys_into_guest`, which is itself scaffolded for the
/// kernel_run handoff (#376). Track VVV (#455) added a second
/// caller — `crate::ui_apps::doom::DoomApp::drain_keystrokes_intercept_esc`
/// — which needs to translate-then-dispatch one key at a time so
/// the launcher's super-loop can intercept Esc BEFORE the
/// keystroke reaches the guest's `reportKeyDown` export. `pub`
/// because that caller lives in a sibling module.
#[allow(dead_code)]
pub fn translate_decoded_key(key: DecodedKey) -> Option<u8> {
    use doom_key::*;
    match key {
        DecodedKey::Unicode(ch) => match ch {
            // Control-character remaps — see comment above.
            '\u{0008}' => Some(KEY_BACKSPACE),
            '\u{000A}' | '\u{000D}' => Some(KEY_ENTER),
            ' ' => Some(KEY_USE),
            // Pass-through ASCII printables. Lowercase letters land
            // here directly; uppercase letters (when shift is held)
            // get downcast so the `doomKey` always matches the
            // lowercase ASCII code per the header's contract.
            c if c.is_ascii() => {
                let b = c as u32 as u8;
                if b.is_ascii_uppercase() {
                    Some(b.to_ascii_lowercase())
                } else if b.is_ascii_graphic() || b == b'\t' || b == 0x1B {
                    // Includes Tab (0x09) and Escape (0x1B), which
                    // Us104Key emits as Unicode controls and Doom's
                    // numeric values for KEY_TAB / KEY_ESCAPE happen
                    // to match the raw control-character bytes.
                    Some(b)
                } else {
                    None
                }
            }
            // Non-ASCII Unicode (e.g. AltGr-composed glyphs) — Doom
            // can't represent these in the [0, 255] doomKey space, so
            // drop. The header is explicit that out-of-range calls
            // are logged-and-ignored on the guest side; cleaner to
            // not call at all than to send a value that the guest
            // would reject.
            _ => None,
        },
        DecodedKey::RawKey(code) => match code {
            // Cursor keys drive player movement.
            KeyCode::ArrowUp    => Some(KEY_UPARROW),
            KeyCode::ArrowDown  => Some(KEY_DOWNARROW),
            KeyCode::ArrowLeft  => Some(KEY_LEFTARROW),
            KeyCode::ArrowRight => Some(KEY_RIGHTARROW),

            // Modifiers. LCtrl / RCtrl = fire (Doom tradition);
            // Spacebar already mapped to KEY_USE above on the Unicode
            // path; LShift / RShift = run; LAlt / RAltGr = strafe.
            KeyCode::LControl | KeyCode::RControl => Some(KEY_FIRE),
            KeyCode::LShift   | KeyCode::RShift   => Some(KEY_RSHIFT),
            KeyCode::LAlt     | KeyCode::RAltGr   => Some(KEY_RALT),

            // Function keys for menu shortcuts (F2 = save, F3 = load,
            // F5 = detail, F11 = brightness, etc. inside Doom). The
            // header doesn't define KEY_F11 / KEY_F12 so we cap at
            // F10; F11/F12 fall through to None.
            KeyCode::F1  => Some(KEY_F1),
            KeyCode::F2  => Some(KEY_F2),
            KeyCode::F3  => Some(KEY_F3),
            KeyCode::F4  => Some(KEY_F4),
            KeyCode::F5  => Some(KEY_F5),
            KeyCode::F6  => Some(KEY_F6),
            KeyCode::F7  => Some(KEY_F7),
            KeyCode::F8  => Some(KEY_F8),
            KeyCode::F9  => Some(KEY_F9),
            KeyCode::F10 => Some(KEY_F10),

            KeyCode::PauseBreak => Some(KEY_PAUSE),

            _ => None,
        },
    }
}

/// Drain every pending decoded keystroke off the UEFI keyboard ring
/// (`arch::uefi::keyboard`, Track EE / commit 8d9c14e) and forward
/// each one to the Doom guest as a synthetic press-then-release pair
/// against the guest's `reportKeyDown` / `reportKeyUp` exports.
///
/// Intended call site: the per-frame driver in `kernel_run` (the
/// #376 follow-up to the entry_uefi.rs binding smoke), once between
/// every `tickGame` so the engine sees fresh key-state edges before
/// the next render. Returns `Ok(n)` where `n` is the number of
/// keystrokes pumped, or `Err(wasmi::Error)` if the export lookup or
/// invocation fails — the caller is expected to bail to the panic
/// path on `Err` since a missing export means the guest module isn't
/// the binary the shim was reconciled against.
///
/// ── Press/release synthesis ────────────────────────────────────────
///
/// See the module-level "Press/release fidelity gap" comment above.
/// Briefly: the keyboard ring stores `DecodedKey` (post-translation,
/// no Down/Up state) so we can't dispatch real edges. We send a
/// down-then-up pair per event — fine for menu / one-shot inputs,
/// wrong for held movement / fire / run. Tracked under #377 as a
/// follow-up that needs Track EE to expose a sibling `KeyEvent` ring.
///
/// ── Failure handling ───────────────────────────────────────────────
///
/// Out-of-range key bytes (the doom_wasm.h header allows [0, 255] but
/// our `u8 -> i32` widen always lands inside that range) are silently
/// accepted by the guest's logged-error path; we don't need to clamp
/// here. Untranslatable `DecodedKey` values (returned `None` from
/// `translate_decoded_key` — caps lock, media keys, F11 / F12) are
/// dropped without dispatch.
///
/// ── Ring drain semantics ───────────────────────────────────────────
///
/// We drain ALL pending events in one call, not just one. The
/// keyboard ring is bounded at 64 slots and drops oldest under back-
/// pressure (see `arch::uefi::keyboard::handle_scancode`); calling
/// this once per 35 Hz tic with a typical typing rate of ~10 keys/s
/// means we'll see at most 1-2 events per drain, well below the cap.
/// Worst case (user holds a key with auto-repeat, ring fills to 64
/// before we drain): we dispatch 64 events to the guest in a tight
/// loop, which is microseconds; no risk of starving the next tic.
///
/// `#[allow(dead_code)]` because no caller exists yet — this is the
/// scaffold the #376 kernel_run handoff will reach. Drops the allow
/// attribute the moment the per-frame driver lands.
#[allow(dead_code)]
pub fn pump_keys_into_guest<T>(
    store: &mut Store<T>,
    instance: &Instance,
) -> Result<usize, wasmi::Error> {
    // Resolve both exports once per call. `get_typed_func` validates
    // the (param, result) shape against the imported binary, so a
    // signature drift between doom_wasm.h and the bake would surface
    // here as `Err(_)` rather than a silent runtime mis-call.
    let report_down = instance
        .get_typed_func::<i32, ()>(&*store, "reportKeyDown")?;
    let report_up = instance
        .get_typed_func::<i32, ()>(&*store, "reportKeyUp")?;

    let mut pumped: usize = 0;
    while let Some(decoded) = crate::arch::keyboard::read_keystroke() {
        let Some(doom_key) = translate_decoded_key(decoded) else {
            // Untranslatable — drop without dispatch; see contract
            // comment above.
            continue;
        };
        let key_i32 = doom_key as i32;
        // Down then up — synthesize a tap. See the module-level
        // "Press/release fidelity gap" comment for why this is a
        // workaround until Track EE exposes a `KeyEvent` ring.
        report_down.call(&mut *store, key_i32)?;
        report_up.call(&mut *store, key_i32)?;
        pumped += 1;
    }
    Ok(pumped)
}

#[cfg(test)]
mod tests {
    // The kernel binary doesn't run tests (`test = false` in
    // Cargo.toml), and this module is gated behind
    // `cfg(target_os = "uefi", target_arch = "x86_64")` which the
    // host-side `cargo test` can't satisfy directly. Tests live here
    // for two reasons:
    //   1. Documentation — the asserts are an executable spec for the
    //      translate table, easier to read than the match arms in
    //      `translate_decoded_key`.
    //   2. Future-proofing — when the kernel grows a UEFI test
    //      harness, these light up automatically.

    use super::*;
    use pc_keyboard::{DecodedKey, KeyCode};

    #[test]
    fn ascii_letter_lowercase_pass_through() {
        assert_eq!(translate_decoded_key(DecodedKey::Unicode('a')), Some(b'a'));
        assert_eq!(translate_decoded_key(DecodedKey::Unicode('z')), Some(b'z'));
        assert_eq!(translate_decoded_key(DecodedKey::Unicode('m')), Some(b'm'));
    }

    #[test]
    fn ascii_letter_uppercase_downcased() {
        // Header: "the keys 'a' through 'z' have a `doomKey` value of
        // 97 through 122, respectively". So shift+A still emits 0x61
        // (lowercase 'a'), not 0x41.
        assert_eq!(translate_decoded_key(DecodedKey::Unicode('A')), Some(b'a'));
        assert_eq!(translate_decoded_key(DecodedKey::Unicode('Z')), Some(b'z'));
    }

    #[test]
    fn ascii_digits_pass_through() {
        assert_eq!(translate_decoded_key(DecodedKey::Unicode('0')), Some(b'0'));
        assert_eq!(translate_decoded_key(DecodedKey::Unicode('9')), Some(b'9'));
    }

    #[test]
    fn control_character_remaps() {
        // Backspace: Us104Key emits 0x08, Doom expects 0x7F.
        assert_eq!(
            translate_decoded_key(DecodedKey::Unicode('\u{0008}')),
            Some(0x7F),
        );
        // Enter: Us104Key emits 0x0A (LF), Doom expects 0x0D.
        assert_eq!(
            translate_decoded_key(DecodedKey::Unicode('\u{000A}')),
            Some(13),
        );
        assert_eq!(
            translate_decoded_key(DecodedKey::Unicode('\u{000D}')),
            Some(13),
        );
        // Spacebar -> KEY_USE.
        assert_eq!(
            translate_decoded_key(DecodedKey::Unicode(' ')),
            Some(0xA2),
        );
        // Escape and Tab — control chars whose Doom code matches the
        // raw byte (KEY_ESCAPE = 27 = 0x1B; KEY_TAB = 9 = 0x09).
        assert_eq!(
            translate_decoded_key(DecodedKey::Unicode('\u{001B}')),
            Some(27),
        );
        assert_eq!(
            translate_decoded_key(DecodedKey::Unicode('\u{0009}')),
            Some(9),
        );
    }

    #[test]
    fn arrow_keys_translate() {
        assert_eq!(
            translate_decoded_key(DecodedKey::RawKey(KeyCode::ArrowUp)),
            Some(0xAD),
        );
        assert_eq!(
            translate_decoded_key(DecodedKey::RawKey(KeyCode::ArrowDown)),
            Some(0xAF),
        );
        assert_eq!(
            translate_decoded_key(DecodedKey::RawKey(KeyCode::ArrowLeft)),
            Some(0xAC),
        );
        assert_eq!(
            translate_decoded_key(DecodedKey::RawKey(KeyCode::ArrowRight)),
            Some(0xAE),
        );
    }

    #[test]
    fn modifiers_translate() {
        // LCtrl / RCtrl -> KEY_FIRE.
        assert_eq!(
            translate_decoded_key(DecodedKey::RawKey(KeyCode::LControl)),
            Some(0xA3),
        );
        assert_eq!(
            translate_decoded_key(DecodedKey::RawKey(KeyCode::RControl)),
            Some(0xA3),
        );
        // LShift / RShift -> KEY_RSHIFT.
        assert_eq!(
            translate_decoded_key(DecodedKey::RawKey(KeyCode::LShift)),
            Some(0xB6),
        );
        assert_eq!(
            translate_decoded_key(DecodedKey::RawKey(KeyCode::RShift)),
            Some(0xB6),
        );
    }

    #[test]
    fn function_keys_translate() {
        assert_eq!(
            translate_decoded_key(DecodedKey::RawKey(KeyCode::F1)),
            Some(0xBB),
        );
        assert_eq!(
            translate_decoded_key(DecodedKey::RawKey(KeyCode::F10)),
            Some(0xC4),
        );
    }

    #[test]
    fn unsupported_keys_drop() {
        // Non-ASCII Unicode — drop.
        assert_eq!(translate_decoded_key(DecodedKey::Unicode('é')), None);
        // F11 / F12 — header doesn't define them, drop.
        assert_eq!(translate_decoded_key(DecodedKey::RawKey(KeyCode::F11)), None);
        assert_eq!(translate_decoded_key(DecodedKey::RawKey(KeyCode::F12)), None);
        // Caps lock LED toggle — drop.
        assert_eq!(
            translate_decoded_key(DecodedKey::RawKey(KeyCode::CapsLock)),
            None,
        );
    }
}
