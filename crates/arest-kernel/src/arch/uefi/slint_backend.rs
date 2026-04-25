// crates/arest-kernel/src/arch/uefi/slint_backend.rs
//
// Slint software-renderer → UEFI GOP framebuffer adapter (#427).
//
// Slint's `renderer-software` feature ships a CPU-side rasteriser that
// writes into a caller-provided buffer. The integration shape Slint
// documents for MCU / no_std hosts is the
// [`LineBufferProvider`](slint::platform::software_renderer::LineBufferProvider)
// trait: Slint calls `process_line(line, range, render_fn)` for every
// scanline that needs repainting; the host hands `render_fn` a scratch
// buffer, and once `render_fn` returns the host blits those pixels onto
// whatever the display controller actually reads.
//
// Why a per-line scratch + blit rather than handing Slint a slice into
// the GOP MMIO directly:
//   * The GOP framebuffer's pixel layout is `[B, G, R, X]` (BGRX) or
//     `[R, G, B, X]` (RGBX) — both 32-bit, with the trailing byte
//     reserved per UEFI §12.9. The byte order is fixed by the GOP
//     mode info but Slint's `TargetPixel` impls are byte-order-fixed
//     too, so a direct overlay would either tie us to one GOP variant
//     (RGBX-only) or force us to remap the buffer post-render.
//   * Doing the swizzle once per line, in the blit step, decouples
//     the rasteriser from the GOP variant. Slint always renders into
//     `[R, G, B, A]` (matches `PremultipliedRgbaColor`); the blit
//     either copies straight to RGBX or swaps R<->B for BGRX.
//   * MMIO writes are slow under cacheable mappings; a scratch buffer
//     in normal RAM keeps the rasteriser's read-modify-write blends
//     local and limits MMIO traffic to one sequential write per line.
//
// Pixel format choice — `PremultipliedRgbaColor` (32-bit RGBA8):
//   * The GOP framebuffer is 4 bytes per pixel on every UEFI-reachable
//     boot (verified in `entry_uefi.rs` step 4 — `bytes_per_pixel = 4`
//     for both `PixelFormat::Rgb` and `PixelFormat::Bgr` GOP variants;
//     Bitmask / BltOnly fall through to a no-op).
//   * `software_renderer::Rgb565Pixel` (16-bit) would force a 4→2 byte
//     conversion in the inner blit loop and lose colour fidelity. The
//     32-bit option avoids both.
//   * Slint exposes `PremultipliedRgbaColor` as `TargetPixel` in the
//     software renderer crate (`#[repr(C)]` with `red, green, blue,
//     alpha` u8 fields). Memory layout `[R, G, B, A]` lines up with
//     GOP's RGBX directly and needs only an R<->B swap for BGRX.
//
// What this commit adds:
//   * `FramebufferBackend` — the `LineBufferProvider` impl. Owns the
//     captured GOP framebuffer descriptor (pointer + dimensions +
//     stride + RB-swap flag) plus a heap-allocated scratch line
//     buffer reused across `process_line` calls.
//   * `UefiSlintPlatform` — the `slint::platform::Platform` impl.
//     Holds an `Rc<MinimalSoftwareWindow>`, hands it back from
//     `create_window_adapter`, and implements `duration_since_start`
//     against `arch::time::now_ms` so Slint's animation / timer code
//     advances on the same PIT-backed millisecond counter the rest
//     of the kernel uses.
//
// What this commit deliberately does NOT do:
//   * No call sites. `entry_uefi.rs` does not reference this module
//     yet — the wiring lands in #431 (UI bootstrap + main loop).
//     The `#[allow(dead_code)]` on the public surface silences the
//     "never used" warnings until then.
//   * No input / event pump. Pointer + keyboard adapters land in
//     #428; this commit is render-output only.
//   * No `.slint` content. The first `.slint` file lands in #436;
//     until then `MinimalSoftwareWindow` boots empty.

#![allow(dead_code)]

use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::ops::Range;
use core::time::Duration;

use slint::platform::software_renderer::{
    LineBufferProvider, MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, PlatformError, WindowAdapter};

use crate::arch::time::now_ms;

// Slint design system + base components (#436).
//
// Pulls Track YY's #432 design tokens + Track JJJ's #433 fonts +
// #434 icons into the kernel UI by importing the .slint files
// under `crates/arest-kernel/ui/` via the inline `slint::slint!`
// macro.
//
// Why `slint!` (proc-macro) instead of `slint-build` (build-script):
//   * Cargo evaluates `[target.'cfg(...)'.build-dependencies]` cfg
//     expressions against the *host* triple, not the target — so
//     the existing `slint-build = "1.16"` declaration in Cargo.toml
//     gated to `cfg(all(target_os = "uefi", target_arch = "x86_64"))`
//     never reaches this build script's dep graph (host is always
//     Windows / Linux). `use slint_build` in `build.rs` would fail
//     to resolve. Migrating to an unconditional `[build-dependencies]`
//     block is a Cargo.toml edit owned by Track II / #426 and out
//     of scope for this commit.
//   * The proc-macro path doesn't need the build-script dep — it
//     ships under the runtime `slint` crate, which IS available on
//     the UEFI x86_64 target via the existing
//     `[target.cfg(...).dependencies]` block.
//
// Font rendering caveat:
//   * `slint!` routes through `slint-macros` → `i-slint-compiler`
//     *without* the `software-renderer` feature. So we cannot use
//     `EmbedForSoftwareRenderer` (which needs that feature to
//     pre-rasterise glyph atlases into `BitmapFont` statics). The
//     default mode (`EmbedAllResources`) embeds TTF bytes raw and
//     emits `register_font_from_memory(...).unwrap()` calls in
//     component init — but `register_font_from_memory` is std-only
//     (gated by Slint's `systemfonts` feature, off in our MCU
//     recipe), so the unwrap panics.
//   * Therefore the .slint files below reference fonts by `family`
//     name only and DO NOT `import "...ttf";` the vendored TTFs.
//     Component construction succeeds; text rendering panics with
//     "No font fallback found" until either (a) `slint-build` is
//     wired and switches to `BitmapFont`, or (b) the runtime
//     registers fonts via a different path. Both are tracked
//     under #431 (UI bootstrap + main loop).
//   * The smoke test in `#[cfg(test)] mod tests` only constructs
//     `AppShell` — no rendering, no panic — which exercises the
//     design-system pipeline end to end (parser, codegen, struct
//     layout) without depending on the runtime font path.
//
// `#![include_path = "ui"]` makes file imports inside the macro
// resolve relative to `<CARGO_MANIFEST_DIR>/ui/`.
slint::slint! {
    import { AppShell, Theme, ThemeMode } from "ui/AppShell.slint";

    export { AppShell, Theme, ThemeMode }
}

/// GOP-side pixel layout. The kernel framebuffer is always 4 bytes
/// per pixel under UEFI; the only thing that changes between mode
/// variants is whether the byte order is `[R, G, B, _]` (RGBX, the
/// `PixelFormat::Rgb` GOP variant) or `[B, G, R, _]` (BGRX, the
/// `PixelFormat::Bgr` GOP variant).
///
/// Slint's `PremultipliedRgbaColor` `TargetPixel` is `[R, G, B, A]`
/// in memory (repr(C) with `red, green, blue, alpha`), so the RGBX
/// case is a direct memcpy with the alpha byte landing in the
/// firmware-ignored reserved slot. The BGRX case needs an R<->B
/// swap during blit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramebufferPixelOrder {
    /// `[R, G, B, _]` per pixel. Matches Slint's
    /// `PremultipliedRgbaColor` layout — direct copy.
    Rgbx,
    /// `[B, G, R, _]` per pixel. Needs R<->B swap during blit.
    Bgrx,
}

/// Adapter that lets Slint's software renderer write into the kernel
/// framebuffer one scanline at a time.
///
/// Holds a raw `*mut u8` pointer at the GOP framebuffer base plus the
/// mode descriptor captured by `entry_uefi.rs` (width / height in
/// pixels, stride in pixels, byte order). A heap-allocated scratch
/// buffer lives alongside the descriptor and is reused across
/// `process_line` calls — every call resizes it to the rendered
/// range, hands a slice to Slint's `render_fn`, then blits the
/// result to MMIO.
///
/// # Safety / lifetime contract
///
/// The framebuffer pointer is treated as `'static`: callers must
/// pass the same firmware-mapped GOP base that
/// `crate::framebuffer::install` holds, valid for the rest of boot.
/// `entry_uefi.rs` `mem::forget`s the GOP `ScopedProtocol` to keep
/// the mapping live, so this is sound; it would not be sound to
/// hand `FramebufferBackend::new` a pointer that gets freed before
/// the kernel halts.
pub struct FramebufferBackend {
    /// GOP framebuffer base. `*mut u8` rather than `&'static mut [u8]`
    /// because the live framebuffer is also held by
    /// `crate::framebuffer::Driver` and we don't want to claim
    /// exclusive borrow at the type level — the kernel runs single-
    /// threaded at boot, so concurrent writes are impossible by
    /// construction, but the borrow checker can't see that.
    fb_ptr: *mut u8,
    /// Width of the visible framebuffer in pixels.
    width: usize,
    /// Height of the visible framebuffer in pixels.
    height: usize,
    /// Stride in pixels (NOT bytes). Equal to `width` on the common
    /// case but firmware may pad rows for alignment, so the GOP
    /// `mode_info.stride()` is preserved verbatim.
    stride: usize,
    /// Byte order at each 4-byte slot.
    pixel_order: FramebufferPixelOrder,
    /// Reusable scratch buffer for one rendered scanline. Slint's
    /// `process_line` may render any sub-range of the row, so the
    /// scratch is grown to `width` once at construction and reused
    /// for every `process_line` call (only the requested sub-slice
    /// is handed to `render_fn`, and only that sub-slice is blitted).
    scratch: Vec<PremultipliedRgbaColor>,
}

impl FramebufferBackend {
    /// Build a new adapter. `fb_ptr` is the GOP framebuffer base
    /// (firmware-mapped MMIO, `'static` for the kernel's boot);
    /// `width` / `height` are the visible mode dimensions in pixels;
    /// `stride` is the row stride in pixels (NOT bytes); `order` is
    /// the byte order at each 4-byte pixel slot.
    ///
    /// # Safety
    ///
    /// Caller asserts that `fb_ptr` points to at least `stride *
    /// height * 4` bytes of writable framebuffer memory, and that no
    /// other code will write to those bytes concurrently with this
    /// backend's `process_line` calls. Concurrent reads (e.g. the
    /// firmware compositor scanning out to the panel) are fine — the
    /// adapter only writes.
    pub unsafe fn new(
        fb_ptr: *mut u8,
        width: usize,
        height: usize,
        stride: usize,
        order: FramebufferPixelOrder,
    ) -> Self {
        let scratch = vec![PremultipliedRgbaColor::default(); width];
        Self { fb_ptr, width, height, stride, pixel_order: order, scratch }
    }

    /// Visible framebuffer width in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Visible framebuffer height in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// GOP byte order at each 4-byte slot.
    pub fn pixel_order(&self) -> FramebufferPixelOrder {
        self.pixel_order
    }
}

impl LineBufferProvider for FramebufferBackend {
    /// Slint's `TargetPixel` impls all rely on a fixed in-memory
    /// layout. We pick the 32-bit one (`PremultipliedRgbaColor`,
    /// `[R, G, B, A]` per pixel) because the GOP framebuffer is
    /// 4 bytes per pixel — see the module docstring for the full
    /// rationale.
    type TargetPixel = PremultipliedRgbaColor;

    /// One scanline render + blit cycle.
    ///
    /// Slint hands us:
    ///   * `line` — the y coordinate of the row Slint wants to paint.
    ///   * `range` — the x sub-range within that row that needs
    ///     repainting (Slint clips to its dirty region).
    ///   * `render_fn` — a one-shot closure that, when called with a
    ///     mutable slice the size of `range`, rasterises whatever
    ///     widgets / text / images intersect that sub-row.
    ///
    /// Our job is to call `render_fn` against a scratch slice and
    /// then blit the resulting pixels to MMIO at
    /// `fb_ptr + (line * stride + range.start) * 4`, with an R<->B
    /// swap if the GOP variant is BGRX.
    ///
    /// Out-of-bounds requests are silently dropped (matches the
    /// no-op-on-clip policy `framebuffer::with_back` follows for the
    /// non-Slint draw_*) — Slint's clipping should keep us in bounds
    /// already, so an out-of-bounds request means caller error and
    /// dropping the line is the safest response under no_std.
    fn process_line(
        &mut self,
        line: usize,
        range: Range<usize>,
        render_fn: impl FnOnce(&mut [Self::TargetPixel]),
    ) {
        // Drop out-of-bounds requests. Slint's own clipping should
        // make these unreachable in practice, but defending against
        // a malformed range is one extra `if` and avoids a UB write
        // past the end of the framebuffer.
        if line >= self.height {
            return;
        }
        let end = range.end.min(self.width);
        let start = range.start.min(end);
        let span = end - start;
        if span == 0 {
            return;
        }

        // Hand `render_fn` a fresh slice (zero-init) so previous
        // line content does not bleed through if the renderer only
        // writes some of the slots. `clear` reuses the existing
        // capacity — no per-line allocation.
        let scratch = &mut self.scratch[..span];
        for px in scratch.iter_mut() {
            *px = PremultipliedRgbaColor::default();
        }
        render_fn(scratch);

        // Blit. One sequential write per pixel into MMIO. The R<->B
        // swap for BGRX is a per-pixel branch; the loop is small
        // enough to autovectorise on the RGBX path (4-byte memcpy
        // semantics), and the BGRX path's branch is predictable
        // (constant for the lifetime of the backend).
        //
        // SAFETY: bounds checked above (`line < self.height`,
        // `start + span <= self.width <= self.stride`). The
        // framebuffer is `stride * height * 4` bytes wide; the
        // worst write offset is `(line * stride + start + span - 1) *
        // 4 + 3`, which is `<= (line * stride + width) * 4 - 1` and
        // therefore inside the firmware mapping.
        let row_offset_bytes = line * self.stride * 4 + start * 4;
        match self.pixel_order {
            FramebufferPixelOrder::Rgbx => {
                // [R, G, B, A] in memory matches GOP RGBX directly.
                // Alpha lands in the firmware-reserved slot, which
                // GOP §12.9 mandates be ignored — the panel sees an
                // opaque pixel regardless of what we write here.
                for (i, px) in scratch.iter().enumerate() {
                    let dst = unsafe { self.fb_ptr.add(row_offset_bytes + i * 4) };
                    unsafe {
                        core::ptr::write_volatile(dst.add(0), px.red);
                        core::ptr::write_volatile(dst.add(1), px.green);
                        core::ptr::write_volatile(dst.add(2), px.blue);
                        core::ptr::write_volatile(dst.add(3), 0);
                    }
                }
            }
            FramebufferPixelOrder::Bgrx => {
                // GOP BGRX wants [B, G, R, X]. Swap red <-> blue
                // from Slint's [R, G, B, A] layout.
                for (i, px) in scratch.iter().enumerate() {
                    let dst = unsafe { self.fb_ptr.add(row_offset_bytes + i * 4) };
                    unsafe {
                        core::ptr::write_volatile(dst.add(0), px.blue);
                        core::ptr::write_volatile(dst.add(1), px.green);
                        core::ptr::write_volatile(dst.add(2), px.red);
                        core::ptr::write_volatile(dst.add(3), 0);
                    }
                }
            }
        }
    }
}

/// `slint::platform::Platform` impl for the UEFI x86_64 boot path.
///
/// Slint expects exactly one `Platform` per process; install it
/// via `slint::platform::set_platform(Box::new(...))` once at boot.
/// The platform owns the window adapter (`MinimalSoftwareWindow`,
/// from the software renderer crate) and the duration provider
/// (the kernel's PIT-backed `arch::time::now_ms`).
///
/// This is a single-threaded, no-event-loop platform — Slint's own
/// `run_event_loop` returns `NoEventLoopProvider`, so the caller
/// must drive the render loop manually:
///   1. `window().request_redraw()` when something changes.
///   2. `window().draw_if_needed(|renderer| renderer.render_by_line(backend))`
///      from the kernel main loop.
///   3. `slint::platform::update_timers_and_animations()` on each
///      tick to keep animations advancing.
///
/// The main-loop wiring lives in #431; this commit just publishes
/// the platform type.
pub struct UefiSlintPlatform {
    /// The single window adapter Slint will hand back from
    /// `create_window_adapter`. `RefCell` because Slint takes the
    /// window via shared `&self` and we need interior mutability to
    /// hand out the `Rc` clone — `MinimalSoftwareWindow` itself is
    /// `Rc`-managed, so this is just an extra layer of "we promise
    /// to only ever hand out the same `Rc`."
    ///
    /// The `Option` lets the caller take ownership of the original
    /// `Rc` after `set_platform` (so the kernel main loop can call
    /// `draw_if_needed` on it directly) without dragging Slint's
    /// `Window` accessor around.
    window: RefCell<Option<Rc<MinimalSoftwareWindow>>>,
}

impl UefiSlintPlatform {
    /// Build a platform whose `MinimalSoftwareWindow` is sized to
    /// the framebuffer the caller already captured. Caller passes
    /// width / height in pixels; the size is set on the window so
    /// Slint's first layout pass sees the correct surface.
    ///
    /// Buffer-reuse mode (`RepaintBufferType::ReusedBuffer`) is the
    /// right default for a long-lived MMIO target: Slint paints only
    /// the dirty region each frame instead of redrawing the whole
    /// surface. `NewBuffer` would force a full repaint per frame —
    /// fine for a triple-buffered display where the scan-out target
    /// rotates, wrong for a single GOP framebuffer the panel reads
    /// continuously.
    pub fn new(width: u32, height: u32) -> Self {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        window.set_size(slint::PhysicalSize::new(width, height));
        Self { window: RefCell::new(Some(window)) }
    }

    /// Take ownership of the window. After this returns, subsequent
    /// `create_window_adapter` calls fail (the window has been
    /// handed out). Intended for the kernel main loop in #431, which
    /// needs a direct `Rc<MinimalSoftwareWindow>` to call
    /// `draw_if_needed` against the `FramebufferBackend`.
    pub fn take_window(&self) -> Option<Rc<MinimalSoftwareWindow>> {
        self.window.borrow_mut().take()
    }
}

impl Platform for UefiSlintPlatform {
    /// Hand Slint the single window adapter we hold. Called once,
    /// during the first component instantiation — Slint stores the
    /// returned `Rc` and reuses it for the lifetime of the program.
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        // Clone the `Rc` rather than taking it: Slint owns the
        // returned `Rc`, but the kernel main loop also needs to
        // call `draw_if_needed` on the same window. Cloning keeps
        // both paths pointing at the same underlying
        // `MinimalSoftwareWindow`.
        match self.window.borrow().as_ref() {
            Some(w) => Ok(w.clone() as Rc<dyn WindowAdapter>),
            None => Err(PlatformError::NoPlatform),
        }
    }

    /// PIT-backed monotonic millisecond counter. Slint's animation
    /// + timer code reads this to advance frame state. `arch::time::now_ms`
    /// returns 0 until `arch::init_time` runs (it's an `AtomicU64`
    /// initialised to 0, only incremented from the IRQ 0 handler),
    /// so the platform must be created AFTER `init_time` for
    /// animations to behave sensibly. Returning `Duration::ZERO`
    /// pre-init is harmless — Slint's first frame is a fresh paint
    /// anyway.
    fn duration_since_start(&self) -> Duration {
        Duration::from_millis(now_ms())
    }

    /// Slint's `debug()` builtin in `.slint` files routes here. We
    /// forward to the kernel's `println!` so any `.slint` debug
    /// output appears on the post-EBS 16550 alongside the rest of
    /// the boot log.
    fn debug_log(&self, arguments: core::fmt::Arguments) {
        crate::println!("[slint] {}", arguments);
    }
}

// SAFETY: `FramebufferBackend` holds a `*mut u8` (the GOP MMIO base)
// which is `!Send` by default. The kernel runs single-threaded at
// boot — there is no SMP scheduler, no other CPU can race on this
// pointer — so the `Send`/`Sync` markers are sound under our
// concurrency model. They aren't currently required by any Slint API
// surface (Slint with `unsafe-single-threaded` doesn't demand them on
// platform types), but adding them costs nothing and unblocks future
// callers that might want to stash the backend in a `static`.
//
// Intentionally narrow scope — only `FramebufferBackend` gets the
// markers. `UefiSlintPlatform` holds `Rc<MinimalSoftwareWindow>`
// which is itself `!Send` + `!Sync`, and adding markers there would
// be unsound (Slint's `Window` interior mutability is single-thread
// safe, not Sync-safe).
unsafe impl Send for FramebufferBackend {}
unsafe impl Sync for FramebufferBackend {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke-test `process_line` against a heap-backed pseudo-
    /// framebuffer. We allocate 4 bytes per pixel for a 4x2 surface,
    /// hand its base pointer to `FramebufferBackend::new`, then drive
    /// `process_line` with a `render_fn` that fills the scratch with
    /// a known colour. The blit's expected output depends on the
    /// `FramebufferPixelOrder` we picked, so we test both variants.
    ///
    /// The test runs as an ordinary cargo test (host target), not
    /// under the UEFI build — there's no MMIO involved, just a
    /// `Vec<u8>` standing in for the framebuffer.
    #[test]
    fn process_line_blits_rgbx() {
        let mut fb = vec![0u8; 4 * 2 * 4]; // 4 px wide, 2 rows, 4 bpp
        let ptr = fb.as_mut_ptr();
        let mut backend = unsafe {
            FramebufferBackend::new(ptr, 4, 2, 4, FramebufferPixelOrder::Rgbx)
        };
        backend.process_line(1, 1..3, |line| {
            assert_eq!(line.len(), 2);
            line[0] = PremultipliedRgbaColor { red: 0x11, green: 0x22, blue: 0x33, alpha: 0xFF };
            line[1] = PremultipliedRgbaColor { red: 0x44, green: 0x55, blue: 0x66, alpha: 0xFF };
        });
        // Row 0 untouched.
        assert_eq!(&fb[..16], &[0; 16]);
        // Row 1, cols 0 untouched + cols 1..3 written + col 3 untouched.
        assert_eq!(fb[16], 0); // col 0 byte 0
        // Col 1: [R=0x11, G=0x22, B=0x33, X=0]
        assert_eq!(&fb[20..24], &[0x11, 0x22, 0x33, 0x00]);
        // Col 2: [R=0x44, G=0x55, B=0x66, X=0]
        assert_eq!(&fb[24..28], &[0x44, 0x55, 0x66, 0x00]);
        // Col 3 untouched.
        assert_eq!(&fb[28..32], &[0; 4]);
    }

    #[test]
    fn process_line_blits_bgrx_with_swap() {
        let mut fb = vec![0u8; 4 * 1 * 4];
        let ptr = fb.as_mut_ptr();
        let mut backend = unsafe {
            FramebufferBackend::new(ptr, 4, 1, 4, FramebufferPixelOrder::Bgrx)
        };
        backend.process_line(0, 0..1, |line| {
            line[0] = PremultipliedRgbaColor { red: 0xAA, green: 0xBB, blue: 0xCC, alpha: 0xFF };
        });
        // BGRX layout: [B=0xCC, G=0xBB, R=0xAA, X=0]
        assert_eq!(&fb[0..4], &[0xCC, 0xBB, 0xAA, 0x00]);
    }

    /// AppShell construction smoke test (#436). Installs the
    /// `UefiSlintPlatform` and constructs `AppShell` without
    /// rendering — exercises the design-system .slint pipeline
    /// (parser, codegen, struct layout) end to end without
    /// depending on the runtime font path.
    ///
    /// No render call: the inline `slint!` macro's
    /// `EmbedAllResources` mode would emit
    /// `register_font_from_memory(...).unwrap()` calls if the
    /// .slint files imported any TTF — see the rationale comment
    /// above the `slint::slint!` block — and `register_font_from_memory`
    /// is std-only in our MCU feature recipe, so it would panic.
    /// The .slint files reference fonts by family name only, so
    /// construction is panic-free; render-time text would still
    /// panic until #431 wires the slint-build glyph-atlas pipeline
    /// or a runtime font registration callback.
    #[test]
    fn appshell_constructs_under_minimal_window() {
        // Slint refuses to instantiate a component before
        // `set_platform` has run. `set_platform` returns Err on
        // the second call, which is harmless if a previous test
        // in the same binary already installed the platform.
        let platform = UefiSlintPlatform::new(800, 600);
        let _ = slint::platform::set_platform(alloc::boxed::Box::new(platform));

        // Construct AppShell. If the slint compiler / codegen
        // misfired (malformed .slint, missing import, type error),
        // this would fail to compile or panic at construction.
        let shell = super::AppShell::new().expect("AppShell::new failed");
        shell.set_app_title("AREST".into());
        shell.set_status("Ready".into());
    }

    #[test]
    fn process_line_drops_out_of_bounds() {
        let mut fb = vec![0u8; 4 * 2 * 4];
        let ptr = fb.as_mut_ptr();
        let mut backend = unsafe {
            FramebufferBackend::new(ptr, 4, 2, 4, FramebufferPixelOrder::Rgbx)
        };
        // y >= height: silently drop.
        backend.process_line(2, 0..4, |_| panic!("render_fn must not run"));
        // Empty range: silently drop.
        backend.process_line(0, 2..2, |_| panic!("render_fn must not run"));
        // Range past width: clipped to width, render_fn sees only
        // the in-bounds pixels.
        backend.process_line(0, 2..6, |line| {
            assert_eq!(line.len(), 2);
            line[0] = PremultipliedRgbaColor { red: 1, green: 2, blue: 3, alpha: 0xFF };
            line[1] = PremultipliedRgbaColor { red: 4, green: 5, blue: 6, alpha: 0xFF };
        });
        assert_eq!(&fb[8..12], &[1, 2, 3, 0]);
        assert_eq!(&fb[12..16], &[4, 5, 6, 0]);
    }
}
