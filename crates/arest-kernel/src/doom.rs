// crates/arest-kernel/src/doom.rs
//
// Doom WASM host-shim (#270/#271). Wire the guest-side Doom port
// (doomgeneric compiled to wasm32) to the UEFI kernel's I/O surface:
// WAD loading, 35 Hz game loop timing, framebuffer blit, save-game
// persistence, and console message routing. All imports live in four
// WebAssembly import-module namespaces matching the groupings used in
// the doomgeneric-wasm sidecar contract:
//
//   loading:
//     onGameInit(argc: i32, argv_ptr: i32)           - notify host the
//                                                       guest is ready
//                                                       for WAD data;
//                                                       argv_ptr points
//                                                       into guest linear
//                                                       memory.
//     wadSizes(out_ptr: i32) -> i32                  - write i32 count +
//                                                       length table into
//                                                       guest memory at
//                                                       out_ptr; return
//                                                       total bytes
//                                                       needed.
//     readWads(buffer_ptr: i32) -> i32               - copy concatenated
//                                                       WAD blobs into
//                                                       guest memory
//                                                       starting at
//                                                       buffer_ptr;
//                                                       return bytes
//                                                       written.
//
//   runtimeControl:
//     timeInMilliseconds() -> i32                    - monotonic ms
//                                                       since boot.
//                                                       Sourced from
//                                                       arch::time::now_ms
//                                                       (603b77a).
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
//     sizeOfSaveGame() -> i32                        - length of the
//                                                       host-persisted
//                                                       save slot; 0
//                                                       if none.
//     readSaveGame(buf_ptr: i32)                     - copy save bytes
//                                                       into guest
//                                                       memory at
//                                                       buf_ptr.
//     writeSaveGame(buf_ptr: i32, len: i32)          - persist `len`
//                                                       bytes from
//                                                       guest memory
//                                                       at buf_ptr.
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
    /// `readWads`. `argc` / `argv_ptr` mirror the usual `main(argc,
    /// argv)` surface — `argv_ptr` points into guest linear memory at
    /// an array of `i32` offsets, each pointing to a null-terminated
    /// UTF-8 command-line argument.
    fn on_game_init(&mut self, argc: i32, argv_ptr: i32);

    /// `loading.wadSizes`. Guest asks the host how much memory to
    /// reserve for the WAD payload. The host writes a `u32` WAD count
    /// followed by a table of `u32` lengths into guest memory at
    /// `out_ptr`. Returns the total number of bytes the guest needs
    /// to allocate for the subsequent `readWads` copy.
    fn wad_sizes(&mut self, out_ptr: i32) -> i32;

    /// `loading.readWads`. Guest has allocated a buffer of the size
    /// returned by `wadSizes` and passes its offset. The host copies
    /// the WAD blobs concatenated in the order the length table
    /// announced. Returns bytes written (= `wadSizes` return value
    /// on success).
    fn read_wads(&mut self, buffer_ptr: i32) -> i32;

    // --- runtimeControl ---------------------------------------------

    /// `runtimeControl.timeInMilliseconds`. Doom's game loop
    /// accumulates tics against an ms clock — 35 tics/sec expected.
    /// Backed on UEFI by `arch::time::now_ms`; returned as `i32` to
    /// match the doomgeneric-wasm ABI (wraps every ~24 days, which
    /// Doom's delta-based loop tolerates).
    fn time_in_milliseconds(&mut self) -> i32;

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
    /// save-slot length before calling `readSaveGame`. Returns 0 if
    /// no save exists, which lets the guest skip the read entirely.
    fn size_of_save_game(&mut self) -> i32;

    /// `gameSaving.readSaveGame`. Guest has allocated a buffer of the
    /// size returned by `sizeOfSaveGame` and passes its offset. Host
    /// copies the save payload in; guest replays it through
    /// `M_ReadFile` / `P_UnArchive*`.
    fn read_save_game(&mut self, buf_ptr: i32);

    /// `gameSaving.writeSaveGame`. Guest asks the host to persist
    /// `len` bytes from guest memory at `buf_ptr`. Under UEFI this
    /// will land on the virtio-blk checkpoint pipeline (#337) once
    /// the save-slot namespace is carved; until then the stub panics.
    fn write_save_game(&mut self, buf_ptr: i32, len: i32);

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
///     reaching `D_DoomMain`; argv synthesis is deferred.
///   * `on_info_message` / `on_error_message` — live (#384). Route
///     UTF-8 bytes out of guest memory to the kernel serial console
///     with `doom: info:` / `doom: ERROR:` prefixes. Silent drop on
///     malformed UTF-8 so a corrupt message can't itself trap Doom.
///   * `size_of_save_game` — returns 0 ("no save present") per
///     trait contract; the matching `read_save_game` impl is still
///     gated on #375's block_storage reserved-region API.
///
/// The real impl is filled in incrementally alongside the
/// doomgeneric-wasm module landing:
///   * `wad_sizes` / `read_wads` — once the WAD bytes ship embedded
///     in the kernel image (or are served from virtio-blk).
///   * `time_in_milliseconds` — wire to `arch::time::now_ms` with
///     a `u64 -> i32` truncation.
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
    fn on_game_init(&mut self, _argc: i32, _argv_ptr: i32) {
        // Observational — the guest is telling us it's alive and has
        // begun `D_DoomMain`. Print one line so the serial log shows
        // the handoff. `argv_ptr` translation is deferred: the host
        // doesn't yet synthesize a command-line for the guest (that
        // wave lands alongside the embedded WAD work in
        // `on_game_init` / `wad_sizes` / `read_wads` — see the
        // module-level TODO list).
        crate::println!("doom: game init");
    }

    fn wad_sizes(&mut self, _out_ptr: i32) -> i32 {
        // Pure-query — return 0 so the guest reads an empty WAD
        // table rather than panicking at import time. A 0 here makes
        // Doom's init path fail with its own "no IWAD found" error,
        // which is a legible failure mode during scaffold stages.
        0
    }

    fn read_wads(&mut self, _buffer_ptr: i32) -> i32 {
        // Wad transfer is a side-effect on guest memory — no safe
        // zero-return. Panic until the real impl lands.
        panic!("doom: read_wads not yet implemented");
    }

    fn time_in_milliseconds(&mut self) -> i32 {
        // Pure-query. Zero is a valid ms clock reading at t=0, so
        // stubbing zero is defensible; the real impl (one line,
        // `arch::time::now_ms() as i32`) lands with wave 6.
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

    fn size_of_save_game(&mut self) -> i32 {
        // Pure-query. Zero = no save, which is the correct reading
        // on a fresh boot regardless of wiring: the guest interprets
        // 0 as "no save slot present" and skips the `read_save_game`
        // call path entirely (which is exactly what we need while
        // `read_save_game` / `write_save_game` are still gated on
        // the block_storage reserved-region API — see #375 TODOs
        // below).
        //
        // TODO(#372 / #375): once the doomgeneric-wasm binary lands
        // and its save-game format is fixed (doomgeneric ports
        // typically hard-code a 352256-byte slot), return the actual
        // persisted slot length from the block_storage reserved
        // region. Returning a non-zero value here without the
        // corresponding `read_save_game` impl would cause the guest
        // to call through and panic, so the TODO is intentionally
        // paired with the #375 block_storage API gap.
        0
    }

    fn read_save_game(&mut self, _buf_ptr: i32) {
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
        // Guarded by the `size_of_save_game() == 0` check on the
        // guest side — if the guest still calls through before the
        // API lands, treat it as a contract violation and panic.
        panic!("doom: read_save_game not yet implemented (see #375 TODO)");
    }

    fn write_save_game(&mut self, _buf_ptr: i32, _len: i32) {
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
        // Deferred until the `block_storage` API grows a
        // `reserve_region(base, sectors)` primitive — see #375.
        panic!("doom: write_save_game not yet implemented (see #375 TODO)");
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
/// machinery handles the i32 parameter / return marshaling.
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
            |mut caller: wasmi::Caller<'_, T>, argc: i32, argv_ptr: i32| {
                caller.data_mut().on_game_init(argc, argv_ptr);
            },
        )
        .expect("doom: duplicate import loading.onGameInit");
    linker
        .func_wrap(
            "loading",
            "wadSizes",
            |mut caller: wasmi::Caller<'_, T>, out_ptr: i32| -> i32 {
                caller.data_mut().wad_sizes(out_ptr)
            },
        )
        .expect("doom: duplicate import loading.wadSizes");
    linker
        .func_wrap(
            "loading",
            "readWads",
            |mut caller: wasmi::Caller<'_, T>, buffer_ptr: i32| -> i32 {
                caller.data_mut().read_wads(buffer_ptr)
            },
        )
        .expect("doom: duplicate import loading.readWads");

    // runtimeControl.*
    linker
        .func_wrap(
            "runtimeControl",
            "timeInMilliseconds",
            |mut caller: wasmi::Caller<'_, T>| -> i32 {
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
    linker
        .func_wrap(
            "gameSaving",
            "sizeOfSaveGame",
            |mut caller: wasmi::Caller<'_, T>| -> i32 {
                caller.data_mut().size_of_save_game()
            },
        )
        .expect("doom: duplicate import gameSaving.sizeOfSaveGame");
    linker
        .func_wrap(
            "gameSaving",
            "readSaveGame",
            |mut caller: wasmi::Caller<'_, T>, buf_ptr: i32| {
                caller.data_mut().read_save_game(buf_ptr);
            },
        )
        .expect("doom: duplicate import gameSaving.readSaveGame");
    linker
        .func_wrap(
            "gameSaving",
            "writeSaveGame",
            |mut caller: wasmi::Caller<'_, T>, buf_ptr: i32, len: i32| {
                caller.data_mut().write_save_game(buf_ptr, len);
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
