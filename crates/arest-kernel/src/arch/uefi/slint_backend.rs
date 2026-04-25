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

// Slint design system + base components (#436 / #452).
//
// Track MMM's #436 (commit `0b32b17`) wired Track YY's #432 design
// tokens + Track JJJ's #433 fonts + #434 icons into `ui/AppShell.slint`
// + 5 base components, but the `slint-build` dep was unreachable from
// `build.rs` at the time (the cfg-gated `[target.cfg(...).build-
// dependencies]` block in Cargo.toml never resolved against the host
// triple). MMM worked around with the inline `slint::slint!{}` proc-
// macro, which routes through `slint-macros` → `i-slint-compiler` and
// crucially does NOT enable the compiler's `software-renderer` feature
// — so `EmbedForSoftwareRenderer` (the only no_std-friendly font path
// in slint v1.16) was inaccessible and text rendering would have
// panicked at runtime with "No font fallback found".
//
// Track QQQ #452 lifts the cfg gate (slint-build is now an
// unconditional `[build-dependencies]` entry in Cargo.toml) and
// replaces the proc-macro callsite with `slint::include_modules!()`.
// The build script (`build.rs`) now invokes
// `slint_build::compile_with_config("ui/AppShell.slint",
// EmbedResourcesKind::EmbedForSoftwareRenderer)`, which:
//   1. Compiles the `.slint` source tree to Rust under
//      `$OUT_DIR/AppShell.rs`.
//   2. Pre-rasterises a glyph atlas for every (character, size,
//      weight) tuple referenced by the design system (Inter for the
//      sans family, JetBrains Mono for `code`).
//   3. Emits `Renderer::register_bitmap_font(&BITMAP_FONT_DATA)`
//      calls into each component's init code so the bitmap fonts
//      auto-register with the `SoftwareRenderer` Slint hands back via
//      `WindowAdapter::renderer()` — no manual font wiring needed
//      here. (`register_bitmap_font` is the only no_std font entry
//      point on `i_slint_core::renderer::Renderer`;
//      `register_font_from_memory` and `register_font_from_path` are
//      both `#[cfg(feature = "std")]` / gated on `systemfonts`, which
//      we deliberately do NOT enable on the MCU recipe.)
//
// `crate::fonts::INTER_REGULAR` / `JETBRAINS_MONO_REGULAR` (Track
// JJJ #433) stay exposed as `pub static &[u8]` slices for any future
// runtime path that needs raw TrueType bytes (e.g. shaping a glyph
// set outside the Slint pipeline) but are not consumed by the
// embedded-glyph path: the slint compiler bakes the glyph atlas at
// host build time using `fontique`, not at runtime.
slint::include_modules!();

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

    /// Build a platform that hands out a caller-supplied
    /// `MinimalSoftwareWindow`. Sibling of `new()` for callers that
    /// already hold an `Rc<MinimalSoftwareWindow>` they want to
    /// share with `draw_if_needed`-side rendering. Used by the
    /// boot launcher (#431 Track UUU) so the Rust super-loop and
    /// Slint's `create_window_adapter` agree on the same physical
    /// surface — both ends end up cloning the same `Rc`, so the
    /// renderer's `render_by_line` writes into the dirty region
    /// the active component just marked.
    ///
    /// The window is taken as-is; the caller is responsible for
    /// calling `MinimalSoftwareWindow::set_size` (or accepting the
    /// default) before constructing components. Typical wiring:
    /// build the window with `MinimalSoftwareWindow::new(buffer
    /// type)`, set its size to the framebuffer dimensions, then
    /// hand a clone here.
    pub fn with_window(window: Rc<MinimalSoftwareWindow>) -> Self {
        Self { window: RefCell::new(Some(window)) }
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

// ---------------------------------------------------------------
// Foreign-toolkit composition glue (#489 Track LLLL)
// ---------------------------------------------------------------

/// Build a `slint::Image` over a `ForeignSurface`'s current pixel
/// buffer. The integration seam between the foreign-toolkit
/// compositor (`crate::composer`) and Slint's scene tree.
///
/// Same pattern Track VVV's #455 Doom path uses to push a WASM-
/// rendered 640x400 BGRA frame at Slint each tic
/// (`ui_apps::doom::tic` L698-L723): `SharedPixelBuffer::clone_
/// from_slice` detaches an owned copy of the source bytes,
/// `Image::from_rgba8` wraps that buffer as a Slint `Image`. Slint
/// v1.16's `Image` API requires the underlying pixel data to be
/// `'static` (the `SharedPixelBuffer` itself owns its bytes via
/// an internal `SharedVector`), so the per-frame copy is mandatory
/// — there's no zero-copy path that lets Slint borrow our backing
/// `Mutex<Vec<u8>>` directly.
///
/// Cost: one `width * height * 4`-byte memcpy per call. At 800x600
/// RGBA8 that's 1.92 MB per composite — comparable to VVV's Doom
/// 1.024 MB cost at 35 Hz, well under one frame's CPU budget at
/// any sensible refresh rate. Adapters that want to avoid the cost
/// on idle frames must check `surface.is_dirty()` (or track their
/// own generation counter) before calling — same idea VVV's Doom
/// uses with its `last_gen` cache.
///
/// Channel handling matches `composer::PixelFormat`:
///   * `Rgba8` — direct `clone_from_slice`. Bytes are already in
///     Slint's expected `[R, G, B, A]` layout.
///   * `Bgra8` — per-pixel R<->B swap into a pre-allocated
///     `SharedPixelBuffer`, same conversion VVV's #455 Doom path
///     does for its BGRA->RGBA per-frame translation.
///
/// `ForeignSurface` is intentionally NOT a Slint component — Slint
/// components are declared in `.slint` source and codegen'd at
/// build time, not runtime. The composer's job is to produce a
/// Slint-compatible `Image` value that any `.slint` file can bind
/// to via an `in property <image>` declaration. The future Qt /
/// GTK app `.slint` files will look identical to Doom's:
/// `image source: root.foreign-frame;` driven by a Rust-side glue
/// that calls `set_foreign_frame(surface_to_image(&surface))` on
/// each frame.
pub fn surface_to_image(surface: &crate::composer::ForeignSurface) -> slint::Image {
    use crate::composer::PixelFormat;

    let bytes = surface.pixels.lock();
    let w = surface.width;
    let h = surface.height;
    match surface.format {
        PixelFormat::Rgba8 => {
            // Direct clone — `clone_from_slice` reinterprets the
            // `&[u8]` slice as `&[Rgba8Pixel]` (the `rgb::AsPixels`
            // impl) and copies into the SharedPixelBuffer's owned
            // SharedVector storage. After this returns the surface's
            // Mutex is unlocked and Slint owns its own copy.
            let pixel_buf: slint::SharedPixelBuffer<slint::Rgba8Pixel> =
                slint::SharedPixelBuffer::clone_from_slice(bytes.as_slice(), w, h);
            slint::Image::from_rgba8(pixel_buf)
        }
        PixelFormat::Bgra8 => {
            // R<->B swap into a fresh SharedPixelBuffer. Per-pixel
            // 4-byte read + write; same shape Doom's per-frame
            // BGRA->RGBA translation uses (ui_apps::doom::tic
            // L698-L719).
            let mut pixel_buf: slint::SharedPixelBuffer<slint::Rgba8Pixel> =
                slint::SharedPixelBuffer::new(w, h);
            let dst = pixel_buf.make_mut_bytes();
            // Both buffers are the same length by construction
            // (composer::create_surface allocates exactly w*h*4
            // bytes; SharedPixelBuffer::new allocates w*h*4 bytes
            // for Rgba8Pixel). Defensive equal-length check
            // mirrors the Doom path's same guard.
            let expected = (w as usize) * (h as usize) * 4;
            if bytes.len() == expected && dst.len() == expected {
                let src = bytes.as_slice();
                let mut i = 0;
                while i < expected {
                    dst[i] = src[i + 2]; // R = src[B-slot+2]
                    dst[i + 1] = src[i + 1]; // G unchanged
                    dst[i + 2] = src[i]; // B = src[R-slot]
                    dst[i + 3] = src[i + 3]; // A unchanged
                    i += 4;
                }
            }
            slint::Image::from_rgba8(pixel_buf)
        }
    }
}

/// Forwarding `LineBufferProvider` impl for a `&mut FramebufferBackend`.
///
/// Slint's `SoftwareRenderer::render_by_line` consumes the
/// `LineBufferProvider` argument by value (`impl LineBufferProvider`),
/// which would force the caller to either re-construct the backend
/// every frame (re-allocating the scratch buffer) or use a wrapper
/// struct. Implementing the trait for `&mut FramebufferBackend` lets
/// the boot launcher's super-loop (#431 Track UUU) hold the backend
/// once outside the loop and pass `&mut backend` into
/// `render_by_line` each iteration, with the renderer borrowing the
/// scratch via `&mut self -> &mut **self` in `process_line`. No
/// per-frame allocation, same trait surface.
impl LineBufferProvider for &mut FramebufferBackend {
    type TargetPixel = PremultipliedRgbaColor;

    fn process_line(
        &mut self,
        line: usize,
        range: Range<usize>,
        render_fn: impl FnOnce(&mut [Self::TargetPixel]),
    ) {
        // Forward to the inner impl. `*self` is `&mut FramebufferBackend`;
        // the `(*self)` deref + method call routes to the by-value
        // `process_line` defined above, with the same borrow semantics.
        (*self).process_line(line, range, render_fn);
    }
}

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

    /// AppShell construction smoke test (#436 / #452). Installs the
    /// `UefiSlintPlatform` and constructs `AppShell` — exercises the
    /// design-system .slint pipeline (parser, codegen, struct layout,
    /// `register_bitmap_font` call sites emitted by
    /// `EmbedForSoftwareRenderer`) end to end. With Track QQQ #452's
    /// slint-build wiring in place, the auto-emitted
    /// `Renderer::register_bitmap_font(&BITMAP_FONT_DATA)` calls in
    /// `AppShell::new()`'s init body run against the
    /// `MinimalSoftwareWindow` renderer the platform hands out — so
    /// construction now exercises the no_std font path too, not just
    /// the codegen surface.
    ///
    /// No render call here: rendering needs a backing framebuffer
    /// (covered by the `process_line_*` tests above) and the bitmap-
    /// font registration is the only piece this test exists to
    /// guard. A future #431-wired smoke test under qemu UEFI will
    /// drive `draw_if_needed` against `FramebufferBackend` to verify
    /// glyphs land on screen.
    #[test]
    fn appshell_constructs_under_minimal_window() {
        // Slint refuses to instantiate a component before
        // `set_platform` has run. `set_platform` returns Err on
        // the second call, which is harmless if a previous test
        // in the same binary already installed the platform.
        let platform = UefiSlintPlatform::new(800, 600);
        let _ = slint::platform::set_platform(alloc::boxed::Box::new(platform));

        // Construct AppShell. If the slint compiler / codegen
        // misfired (malformed .slint, missing import, type error,
        // or the embedded bitmap font wasn't registered correctly),
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

    /// `surface_to_image` round-trips a `composer::ForeignSurface`'s
    /// `Rgba8` pixel buffer into a `slint::Image` with the right
    /// dimensions. Lifetime check more than pixel-content check —
    /// the goal is to confirm `clone_from_slice` produces an owned
    /// `SharedPixelBuffer` that survives the original surface's
    /// `Mutex` going out of scope (which is what makes the
    /// per-frame call shape sound under Slint's `'static` Image
    /// requirement).
    #[test]
    fn surface_to_image_rgba8_roundtrip() {
        use crate::composer::{create_surface, register_toolkit, ToolkitRenderer, ForeignSurface};
        // Need at least one toolkit registered or the surface is
        // created without an owning renderer; that's fine for this
        // test — we never call `compose_frame` or `paint`. The
        // surface still owns its pixel buffer.
        struct NoopRenderer;
        impl ToolkitRenderer for NoopRenderer {
            fn paint(&self, _: &ForeignSurface) {}
        }
        register_toolkit("test_slint_backend", alloc::sync::Arc::new(NoopRenderer));
        let surface = create_surface("test_slint_backend", 4, 2);
        // Write a known pattern into the surface's pixel buffer
        // so we can assert `surface_to_image` reads it correctly.
        {
            let mut buf = surface.pixels.lock();
            for (i, byte) in buf.iter_mut().enumerate() {
                *byte = (i & 0xFF) as u8;
            }
        }
        let image = super::surface_to_image(&surface);
        assert_eq!(image.size().width, 4);
        assert_eq!(image.size().height, 2);
    }
}
