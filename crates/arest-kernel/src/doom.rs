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
//                                                       Returns 0 until
//                                                       the UEFI arch
//                                                       arm grows a
//                                                       timer IRQ —
//                                                       see TODO in
//                                                       the impl body.
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
use wasmi::Linker;

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
///   * `time_in_milliseconds` — signature widened to `i64` (#395) to
///     match the header's `uint64_t` declaration. Body returns 0
///     (TODO #344 step 4d) until the UEFI arch arm grows a timer
///     IRQ + monotonic ms counter mirroring the BIOS arm's
///     `arch::time::now_ms`; doom.rs is UEFI-x86_64-only so the
///     BIOS arm's existing counter isn't reachable.
///   * `size_of_save_game(gamemap)` — returns 0 ("no save present"
///     for that gamemap) per trait contract; the matching
///     `read_save_game` / `write_save_game` impls are still gated on
///     #375's block_storage reserved-region API.
///
/// The real impl is filled in incrementally alongside the
/// doomgeneric-wasm module landing:
///   * `wad_sizes` / `read_wads` — wire in the embedded `doom.wad`
///     bytes once the WAD-load wave (#383) lands. Today the trait
///     reports zero WADs so the guest falls through to its built-in
///     shareware path (per doom_wasm.h "If numberOfWads remains 0,
///     Doom loads shareware WAD").
///   * `read_save_game` / `write_save_game` — wire to virtio-blk
///     save-slot region (#375). Waiting on `block_storage` to grow
///     a reserved-region API that doesn't clobber the #337
///     checkpoint at sector 0.
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
        // TODO(#383): once the embedded `doom.wad` bytes are wired
        // (Track C baked `doom_assets/doom.wasm`; the WAD-load wave
        // is its sibling), return the actual `(num_wads, total_bytes)`
        // pair so the guest can size its allocation. Today we report
        // zero WADs, which per doom_wasm.h ("If numberOfWads remains
        // 0, Doom loads shareware WAD") signals the guest to fall
        // through to its built-in shareware path — a legible scaffold
        // behavior that doesn't require the trampoline to ship real
        // WAD bytes through guest memory yet.
        (0, 0)
    }

    fn read_wads(&mut self, _wad_out: &mut [u8], _lengths_out: &mut [i32]) {
        // TODO(#383): copy the embedded WAD blob and per-WAD length
        // array into the host-side slices the trampoline allocated.
        // While `wad_sizes` returns `(0, 0)` the guest does not call
        // through here — the trampoline forms zero-length slices and
        // this body is a no-op even if the guest does ignore the
        // count and call anyway, so leaving it empty is safer than a
        // panic that could fire mid-tic.
    }

    fn time_in_milliseconds(&mut self) -> i64 {
        // Header declares `uint64_t timeInMilliseconds`; the trait
        // returns `i64` so the wasmi host-func ABI marshals a 64-bit
        // value. Doom's game loop accumulates tics against this
        // clock at 35 Hz, but it only cares about monotonic deltas
        // — a 0 baseline that never advances will let the guest run
        // its init path and reach the first `drawFrame` (the visible
        // smoke for Track A's UEFI handoff) even if the tic counter
        // never increments.
        //
        // TODO(#344 step 4d / #180-followup): once the UEFI arch arm
        // installs an IDT + timer interrupt, expose a UEFI-side
        // `arch::time::now_ms` mirroring the BIOS arm's PIT-driven
        // `AtomicU64` counter and return `arch::time::now_ms() as
        // i64` here. Today the UEFI arm has no IRQ infrastructure
        // (`arch::halt_forever` busy-loops on `pause` for that
        // reason — see `arch::uefi::mod` docstring), so there is no
        // monotonic ms source reachable from the host shim. The
        // cast `u64 -> i64` is the trivial part — bit-pattern
        // preserving for any clock value below `i64::MAX` ms (≈ 292
        // million years), which any realistic uptime stays well
        // under.
        0
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

    fn size_of_save_game(&mut self, _gamemap: i32) -> i32 {
        // Pure-query. Zero = no save for that gamemap, which is the
        // correct reading on a fresh boot regardless of wiring: the
        // guest interprets 0 as "no save slot present" and skips the
        // `read_save_game` call path entirely (which is exactly what
        // we need while `read_save_game` / `write_save_game` are
        // still gated on the block_storage reserved-region API — see
        // #375 TODOs below).
        //
        // TODO(#375): once the block_storage reserved-region API
        // lands, return the actual persisted slot length for the
        // requested `gamemap`. Returning a non-zero value here
        // without the corresponding `read_save_game` impl would cause
        // the guest to call through and panic, so the TODO is
        // intentionally paired with the #375 block_storage API gap.
        0
    }

    fn read_save_game(&mut self, _gamemap: i32, _out: &mut [u8]) -> i32 {
        // TODO(#375): wire to a block_storage reserved-region API.
        //
        // Today `block_storage` only exposes checkpoint semantics
        // (`mount` / `checkpoint` / `last_state` / `smoke_round_trip`)
        // — one global state slot covering sector 0 (header) and
        // sectors 1..N (state bytes), owned by the #337 checkpoint
        // pipeline. There is no per-region sector API that would let
        // Doom claim its own slab without stomping on #337 or sharing
        // a serializer with it. Implementing save/restore here would
        // require adding a reserved-region primitive to
        // `block_storage` (e.g. sectors 1000..1999 as a Doom save
        // region, with its own header + CRC), which this sub-task is
        // explicitly scoped out of (file ownership: doom.rs only).
        //
        // Guarded by the `size_of_save_game(gamemap) == 0` check on
        // the guest side — if the guest still calls through before
        // the API lands, return 0 (zero bytes copied) rather than
        // panicking, so a stray call doesn't crash the kernel mid-
        // tic. The header allows 0 as a legal return ("returns bytes
        // actually copied").
        0
    }

    fn write_save_game(&mut self, _gamemap: i32, _data: &[u8]) -> i32 {
        // TODO(#375): wire to a block_storage reserved-region API.
        //
        // Same shape as `read_save_game` above — `block_storage` has
        // no per-region sector API today; the only primitive is
        // `checkpoint(&[u8])` / `last_state()` which owns the single
        // global state slot and would conflict with #337's kernel-
        // state checkpoint. The sector-level primitives in
        // `crate::block` (`read_sector` / `write_sector` / `flush`)
        // are available, but carving out a Doom save region at e.g.
        // sectors 1000..1999 belongs in `block_storage` (header +
        // CRC + version lives alongside the existing checkpoint
        // header), not scattered across callers.
        //
        // Per the header: returns "bytes persisted (0 if
        // unsupported)". Returning 0 is the documented "unsupported"
        // signal, which is exactly the truth while #375 is open —
        // the guest should treat the save as failed and surface a
        // visible error rather than spinning on an apparent success.
        // Deferred until the `block_storage` API grows a
        // `reserve_region(base, sectors)` primitive — see #375.
        0
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
