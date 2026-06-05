// crates/arest-kernel/src/framebuffer.rs
//
// Triple-buffered linear framebuffer with damage tracking. Sits on
// top of a firmware-mapped front buffer byte slice (UEFI
// `GraphicsOutputProtocol` on most arms; virtio-gpu DMA-backed
// surface on the x86_64 UEFI virtio-gpu path — see
// `install_virtio_gpu`). Drawing API writes to one of two heap-
// allocated back buffers; `present()` copies the dirty rect from
// the active back buffer onto the front buffer and swaps to the
// other back. Three buffers total → producer never stalls waiting
// for the consumer (would matter if we had vsync signalling;
// without it, triple == double for stall behaviour but the chain
// is in place for when virtio-gpu or a real display controller
// lands and starts gating present() on flips).
//
// Drawing pipeline:
//   1. caller code calls `framebuffer::with_back(|back| back.draw_*())`
//   2. all draws hit the active back buffer + extend the dirty rect
//   3. caller calls `framebuffer::present()` to flush the dirty rect
//      onto the front buffer; the next `with_back` switches to the
//      other back, leaving the just-presented one as a "previous
//      frame" available for diff-based partial updates.
//
// Damage tracking: each draw extends a per-back-buffer
// `DirtyRect`. `present()` memcpies just `[x0..x1] x [y0..y1]`
// rather than the full surface — at 1280x720x24bpp the worst case
// is 2.7 MB per present, but a 50x50 widget update is 7.5 KB.
//
// What's exposed:
//   * `init(...)` — install the front buffer + allocate two
//                    back buffers from the heap.
//   * `info()`    — surface metadata (width / height / format).
//   * `with_back(|back| ...)` — borrow the active back buffer for
//                    direct drawing primitive calls.
//   * `present()` — copy the dirty rect to the front and rotate
//                    to the other back.
//   * `front_fnv1a()` / `back_fnv1a()` — FNV-1a hash of the
//                    respective buffer for smoke-test assertions.
//
// virtio-gpu (#371) is the production GPU stack — see
// `install_virtio_gpu` for the DMA-backed surface install path.
// The firmware-mapped GOP framebuffer is the fallback when no
// virtio-gpu device is present, and the demo path that #270/#271
// drove (Doom blit + paint smoke).

use alloc::{vec, vec::Vec};
use spin::Mutex;

/// Pixel layout of the firmware-mapped surface. Tracks the variants
/// the kernel actually populates from `GraphicsOutputProtocol`'s
/// `PixelFormat` (UEFI §12.9): RGB / BGR linear pixel buffers, plus a
/// greyscale fallback the draw helpers honour. `Bitmask` and `BltOnly`
/// surfaces flow through the `Unknown` arm — `write_pixel` becomes a
/// no-op rather than corrupting the surface.
///
/// Variant order + naming kept stable from the prior `bootloader_api::
/// info::PixelFormat` alias used while the BIOS path was alive (#380),
/// so `entry_uefi.rs`'s `match gop_fmt_idx` arms keep compiling without
/// touching the GOP-format table they were tuned against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PixelFormat {
    /// `[R, G, B]` byte order at each pixel slot.
    Rgb,
    /// `[B, G, R]` byte order at each pixel slot.
    Bgr,
    /// 8-bit greyscale (`U8`). Single channel; draw helpers average
    /// `(r + g + b) / 3` for write.
    U8,
    /// Unmapped / reserved variants (Bitmask, BltOnly). Draw helpers
    /// silently skip writes rather than corrupting the surface.
    Unknown,
}

/// Surface metadata for the firmware-mapped framebuffer. Same field
/// shape the prior `bootloader_api::info::FrameBufferInfo` carried, so
/// the UEFI entries that constructed it from `GraphicsOutputProtocol`
/// keep compiling without touching the constructor sites.
///
/// `byte_len` / `width` / `height` / `stride` / `bytes_per_pixel`
/// follow the GOP convention: `stride` is in pixels (not bytes), and
/// `bytes_per_pixel * stride * height` is `byte_len` exactly when the
/// firmware-reported surface is contiguous (which it is for both OVMF
/// and AAVMF on QEMU virt — see UEFI §12.9 PixelsPerScanLine).
#[derive(Clone, Copy, Debug)]
pub struct FrameBufferInfo {
    pub byte_len: usize,
    pub width: usize,
    pub height: usize,
    pub pixel_format: PixelFormat,
    pub bytes_per_pixel: usize,
    pub stride: usize,
}

/// Singleton driver state. `None` until `install` (or
/// `install_virtio_gpu`) runs and `None` forever if the firmware
/// didn't supply a framebuffer (text-mode boot).
static FB: Mutex<Option<Driver>> = Mutex::new(None);

/// 24-bit RGB colour. Channel order at the wire is decided by the
/// framebuffer's `PixelFormat` — `write_pixel` shuffles bytes
/// accordingly, callers always pass logical RGB.
#[derive(Clone, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK:   Color = Color { r: 0x00, g: 0x00, b: 0x00 };
    pub const WHITE:   Color = Color { r: 0xFF, g: 0xFF, b: 0xFF };
    pub const RED:     Color = Color { r: 0xFF, g: 0x00, b: 0x00 };
    pub const GREEN:   Color = Color { r: 0x00, g: 0xFF, b: 0x00 };
    pub const BLUE:    Color = Color { r: 0x00, g: 0x00, b: 0xFF };
    pub const YELLOW:  Color = Color { r: 0xFF, g: 0xFF, b: 0x00 };
    pub const CYAN:    Color = Color { r: 0x00, g: 0xFF, b: 0xFF };
    pub const MAGENTA: Color = Color { r: 0xFF, g: 0x00, b: 0xFF };
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color { Color { r, g, b } }
}

/// Inclusive-exclusive bounding box of bytes touched since the last
/// `present()`. `None` means "buffer fully clean — present is a
/// no-op." `Some` means "rows y0..y1, cols x0..x1 are dirty and
/// need to be copied to the front."
#[derive(Clone, Copy)]
struct DirtyRect {
    x0: usize, y0: usize, x1: usize, y1: usize,
}

impl DirtyRect {
    fn empty() -> Option<Self> { None }
    fn extend(opt: &mut Option<Self>, x0: usize, y0: usize, x1: usize, y1: usize) {
        match opt {
            Some(r) => {
                r.x0 = r.x0.min(x0);
                r.y0 = r.y0.min(y0);
                r.x1 = r.x1.max(x1);
                r.y1 = r.y1.max(y1);
            }
            None => *opt = Some(Self { x0, y0, x1, y1 }),
        }
    }
}

/// One of the two heap-allocated back buffers. Mirrors the front
/// buffer's byte layout exactly so `present()` can do straight
/// row-wise memcpy without per-pixel format conversion.
pub struct BackBuffer {
    pub(crate) bytes: Vec<u8>,
    info: FrameBufferInfo,
    dirty: Option<DirtyRect>,
}

impl BackBuffer {
    fn new(info: FrameBufferInfo, byte_len: usize) -> Self {
        Self { bytes: vec![0u8; byte_len], info, dirty: None }
    }

    pub fn info(&self) -> FrameBufferInfo { self.info }

    /// Fill the whole back buffer with one colour. Marks the entire
    /// surface dirty.
    pub fn clear(&mut self, c: Color) {
        let (w, h) = (self.info.width, self.info.height);
        self.fill_rect(0, 0, w, h, c);
    }

    /// Write a single pixel at `(x, y)`. Out-of-bounds is silently
    /// dropped — callers don't need to clamp before pixel-art code.
    pub fn put_pixel(&mut self, x: usize, y: usize, c: Color) {
        if x >= self.info.width || y >= self.info.height { return; }
        let bpp = self.info.bytes_per_pixel;
        let off = y * self.info.stride * bpp + x * bpp;
        write_pixel(&mut self.bytes[off..off + bpp], self.info.pixel_format, c);
        DirtyRect::extend(&mut self.dirty, x, y, x + 1, y + 1);
    }

    /// Filled rectangle. Clipped against the framebuffer bounds.
    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, c: Color) {
        let info = self.info;
        let x_end = x.saturating_add(w).min(info.width);
        let y_end = y.saturating_add(h).min(info.height);
        if x >= x_end || y >= y_end { return; }
        let bpp = info.bytes_per_pixel;
        let stride_bytes = info.stride * bpp;
        for row in y..y_end {
            let row_start = row * stride_bytes;
            for col in x..x_end {
                let off = row_start + col * bpp;
                write_pixel(&mut self.bytes[off..off + bpp], info.pixel_format, c);
            }
        }
        DirtyRect::extend(&mut self.dirty, x, y, x_end, y_end);
    }

    /// Bresenham line `(x0, y0) -> (x1, y1)`. Integer-only — no FP
    /// state required. Per-pixel clipping via `put_pixel`.
    pub fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Color) {
        let dx = (x1 - x0).abs();
        let sx: i32 = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy: i32 = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut x = x0;
        let mut y = y0;
        loop {
            if x >= 0 && y >= 0 {
                self.put_pixel(x as usize, y as usize, c);
            }
            if x == x1 && y == y1 { break; }
            let e2 = err * 2;
            if e2 >= dy { err += dy; x += sx; }
            if e2 <= dx { err += dx; y += sy; }
        }
    }

    /// Single 8x8 glyph from the embedded font. Bits in each row
    /// are LSB → leftmost. Glyphs missing from the font render as
    /// a solid block (`0xFF`-filled byte per row) so absent letters
    /// are visually obvious.
    pub fn draw_glyph(&mut self, x: usize, y: usize, ch: char, fg: Color) {
        let bitmap = font::glyph(ch);
        for (row_idx, row) in bitmap.iter().enumerate() {
            for col_idx in 0..8 {
                if row & (1 << col_idx) != 0 {
                    self.put_pixel(x + col_idx, y + row_idx, fg);
                }
            }
        }
    }

    /// ASCII string at `(x, y)`. 8-pixel column stride per char;
    /// line wrap is the caller's problem.
    pub fn draw_text(&mut self, x: usize, y: usize, s: &str, fg: Color) {
        let mut cx = x;
        for ch in s.chars() {
            self.draw_glyph(cx, y, ch, fg);
            cx += 8;
        }
    }

    /// FNV-1a hash of the entire backing byte slice. Used by the
    /// boot-time paint smoke to publish a deterministic checksum
    /// over serial — the host harness asserts a known-good value.
    pub fn fnv1a(&self) -> u64 {
        fnv1a(&self.bytes)
    }

    /// Blit a Doom-format 640x400 frame into the back buffer,
    /// centered at 1x scale. `src` is the raw bytes of Doom's
    /// `DG_ScreenBuffer` — `640 * 400 * 4 = 1_024_000` bytes, stored
    /// as `0xAARRGGBB` little-endian, which is `[B, G, R, A]` in
    /// memory. Alpha is ignored (Doom always writes opaque pixels).
    ///
    /// Centering: a 1280x720 surface gets a 320-col border on each
    /// side and a 160-row border top and bottom. Smaller surfaces
    /// clip from the bottom-right corner. The borders stay whatever
    /// colour the caller painted them before calling (typical:
    /// black-filled once at boot, then blit the central rect each
    /// frame).
    ///
    /// Pixel format: 4bpp Bgr/Rgb target surfaces (UEFI §12.9
    /// mandates a reserved byte after the RGB triple, so every
    /// GOP-reachable boot reports bpp=4). 3bpp surfaces are also
    /// supported for the legacy text-mode framebuffer shape OVMF
    /// occasionally reports under `-vga std`. Other formats are a
    /// no-op (same policy `write_pixel` follows — never corrupt
    /// the surface).
    ///
    /// On 4bpp target surfaces the trailing reserved byte is zeroed;
    /// GOP firmware + QEMU's GPU both ignore it, so the colour stays
    /// correct.
    ///
    /// This is the #270/#271 Doom-host-shim's `drawFrame` import
    /// implementation. ~1 MB source read + ~0.75 MB (3bpp) or ~1 MB
    /// (4bpp) destination write per call; autovectorises into a
    /// row-wise copy on the Bgr path (trivial stride match).
    pub fn blit_doom_frame(&mut self, src: &[u8]) {
        const DOOM_W: usize = 640;
        const DOOM_H: usize = 400;
        const SRC_STRIDE: usize = DOOM_W * 4;

        // Size / format gates. Silent no-op on mismatch — matches
        // the clipping / format policy of the other draw_* methods.
        if src.len() < DOOM_H * SRC_STRIDE {
            return;
        }
        let info = self.info;
        let bpp = info.bytes_per_pixel;
        if bpp != 3 && bpp != 4 {
            return;
        }
        let swap_rb = match info.pixel_format {
            PixelFormat::Bgr => false,
            PixelFormat::Rgb => true,
            PixelFormat::U8 | PixelFormat::Unknown => return,
        };

        // Centered placement. `saturating_sub` guards against
        // framebuffers smaller than the Doom frame — in that case
        // the blit starts at (0, 0) and clips.
        let x_off = info.width.saturating_sub(DOOM_W) / 2;
        let y_off = info.height.saturating_sub(DOOM_H) / 2;
        let cols = DOOM_W.min(info.width.saturating_sub(x_off));
        let rows = DOOM_H.min(info.height.saturating_sub(y_off));
        if cols == 0 || rows == 0 {
            return;
        }
        let dst_stride_bytes = info.stride * bpp;

        for dy in 0..rows {
            let src_row = dy * SRC_STRIDE;
            let dst_row = (y_off + dy) * dst_stride_bytes + x_off * bpp;
            for dx in 0..cols {
                let so = src_row + dx * 4;
                let d_off = dst_row + dx * bpp;
                // Doom source is `[B, G, R, A]` in memory (little-
                // endian 0xAARRGGBB). Target byte order depends on
                // PixelFormat.
                if swap_rb {
                    // Rgb target: R first, then G, then B.
                    self.bytes[d_off]     = src[so + 2];
                    self.bytes[d_off + 1] = src[so + 1];
                    self.bytes[d_off + 2] = src[so];
                } else {
                    // Bgr target: B first — matches Doom's byte order.
                    self.bytes[d_off]     = src[so];
                    self.bytes[d_off + 1] = src[so + 1];
                    self.bytes[d_off + 2] = src[so + 2];
                }
                if bpp == 4 {
                    // GOP-reserved / XRGB alpha byte. Zero rather
                    // than copying src[so+3] (Doom's alpha) because
                    // callers may have pre-filled the surround with
                    // a non-zero byte for the same slot and GPU
                    // firmware may still honour it; zero is the
                    // documented "ignore me" value under UEFI §12.9.
                    self.bytes[d_off + 3] = 0;
                }
            }
        }
        DirtyRect::extend(
            &mut self.dirty,
            x_off, y_off,
            x_off + cols, y_off + rows,
        );
    }
}

/// Channel-layout-aware pixel write. Bgr is what QEMU's standard
/// VGA / OVMF GOP reports; Rgb covers physical hardware that swaps
/// the byte order. U8 is a greyscale fallback; Unknown / unmapped
/// formats are silently skipped rather than corrupting the surface.
fn write_pixel(slot: &mut [u8], format: PixelFormat, c: Color) {
    match format {
        PixelFormat::Rgb => { slot[0] = c.r; slot[1] = c.g; slot[2] = c.b; }
        PixelFormat::Bgr => { slot[0] = c.b; slot[1] = c.g; slot[2] = c.r; }
        PixelFormat::U8  => {
            slot[0] = ((u16::from(c.r) + u16::from(c.g) + u16::from(c.b)) / 3) as u8;
        }
        PixelFormat::Unknown => {}
    }
}

/// FNV-1a hash. Cheap pure-Rust hash — no SIMD intrinsics, no
/// table lookups, suitable for `no_std`. Used for deterministic
/// "did the frame change" checksums in smoke tests.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes.iter() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// The triple-buffer driver. Holds the (mapped) front buffer plus
/// two heap-allocated back buffers; `draw_idx` selects which back
/// is currently the draw target. `present()` copies the dirty
/// region of the active back to the front, then flips `draw_idx`.
///
/// `backend` decides what (if anything) `present()` does AFTER the
/// memcpy lands. `Gop` is the existing path — once bytes are in the
/// firmware-mapped MMIO region the GPU picks them up on its own
/// vsync. `VirtioGpu` requires an explicit transfer_to_host_2d +
/// resource_flush submission via `virtio_gpu::flush_active_surface`
/// so the host actually sees the new pixels (#371).
struct Driver {
    info: FrameBufferInfo,
    front: &'static mut [u8],
    backs: [BackBuffer; 2],
    draw_idx: usize,
    presents: u64,
    backend: FrontBackend,
}

/// Where the front buffer's bytes ultimately end up.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FrontBackend {
    /// Firmware-mapped MMIO surface (UEFI GraphicsOutputProtocol).
    /// The GPU reads it on its own vsync; nothing more for
    /// `present()` to do.
    Gop,
    /// virtio-gpu DMA-backed 2D resource attached to scanout 0 (#371).
    /// `present()` calls `virtio_gpu::flush_active_surface()` to
    /// submit `RESOURCE_FLUSH` (virtio-gpu spec sec 5.7.6.7) so the
    /// host actually blits the new pixels to the display.
    VirtioGpu,
}

impl Driver {
    fn active_back(&mut self) -> &mut BackBuffer {
        &mut self.backs[self.draw_idx]
    }

    /// Memcpy the active back buffer's dirty rect onto the front
    /// buffer, then rotate to the other back so the next draw
    /// cycle starts on a clean surface.
    fn present(&mut self) {
        let bpp = self.info.bytes_per_pixel;
        let stride_bytes = self.info.stride * bpp;
        let back = &mut self.backs[self.draw_idx];
        if let Some(rect) = back.dirty.take() {
            let row_byte_start = rect.x0 * bpp;
            let row_byte_end   = rect.x1 * bpp;
            for row in rect.y0..rect.y1 {
                let off = row * stride_bytes;
                self.front[off + row_byte_start..off + row_byte_end]
                    .copy_from_slice(&back.bytes[off + row_byte_start..off + row_byte_end]);
            }
            self.presents = self.presents.wrapping_add(1);
            // Backend-specific submission. GOP firmware picks the
            // bytes up on its next vsync; virtio-gpu needs an
            // explicit resource_flush so the host blits to the
            // attached scanout (spec sec 5.7.6.7) — without it the
            // DMA buffer changes invisibly to the display.
            if let FrontBackend::VirtioGpu = self.backend {
                #[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
                let _ = crate::virtio_gpu::flush_active_surface();
            }
        }
        self.draw_idx ^= 1;
    }
}

/// Install the front buffer + allocate two heap-backed back
/// buffers. Caller hands in the firmware-provided byte slice
/// (raw ptr + length) plus the format metadata.
///
/// # Safety
///
/// `buffer_ptr` + `buffer_len` must describe the live firmware-
/// mapped framebuffer region (lives `'static` for the kernel's
/// boot). No other code may hold a reference to those bytes when
/// this is called.
pub unsafe fn install(info: FrameBufferInfo, buffer_ptr: *mut u8, buffer_len: usize) {
    let front: &'static mut [u8] = unsafe {
        core::slice::from_raw_parts_mut(buffer_ptr, buffer_len)
    };
    let backs = [
        BackBuffer::new(info, buffer_len),
        BackBuffer::new(info, buffer_len),
    ];
    *FB.lock() = Some(Driver {
        info,
        front,
        backs,
        draw_idx: 0,
        presents: 0,
        backend: FrontBackend::Gop,
    });
}

/// Install the framebuffer singleton on top of a virtio-gpu DMA-backed
/// 2D resource (#371). Mirrors `install` for the GOP path but the
/// front-buffer bytes are the virtio-gpu surface returned by the
/// driver's `framebuffer_buffer()`, and `present()` additionally
/// issues a `resource_flush` after the dirty-rect memcpy so the host
/// actually blits the new pixels to scanout 0.
///
/// The driver itself must already be parked in
/// `virtio_gpu::install(...)` before this call so `present()`'s
/// flush callback can reach it through the singleton.
///
/// # Safety
///
/// `buffer_ptr` + `buffer_len` must describe the virtio-gpu DMA region
/// returned by `VirtIoGpuDriver::framebuffer_buffer()`. The driver
/// lives in `virtio_gpu::GPU` for the rest of the kernel's lifetime,
/// so the underlying `Dma<H>` storage is `'static`. No other code may
/// hold a reference to those bytes when this is called — the
/// framebuffer driver becomes the sole writer.
pub unsafe fn install_virtio_gpu(
    info: FrameBufferInfo,
    buffer_ptr: *mut u8,
    buffer_len: usize,
) {
    let front: &'static mut [u8] = unsafe {
        core::slice::from_raw_parts_mut(buffer_ptr, buffer_len)
    };
    let backs = [
        BackBuffer::new(info, buffer_len),
        BackBuffer::new(info, buffer_len),
    ];
    *FB.lock() = Some(Driver {
        info,
        front,
        backs,
        draw_idx: 0,
        presents: 0,
        backend: FrontBackend::VirtioGpu,
    });
}

/// Surface metadata, lock-free. `None` if the driver wasn't
/// installed (text-mode boot).
pub fn info() -> Option<FrameBufferInfo> {
    FB.lock().as_ref().map(|d| d.info)
}

/// Borrow the active back buffer for a closure. Closure reads /
/// writes via the `BackBuffer` API; everything is committed when
/// `present()` runs. Closure does not run if the driver isn't
/// initialised.
pub fn with_back<R>(f: impl FnOnce(&mut BackBuffer) -> R) -> Option<R> {
    let mut guard = FB.lock();
    guard.as_mut().map(|d| f(d.active_back()))
}

/// Copy the dirty rect of the active back buffer to the front
/// buffer, then rotate to the other back. Cheap when nothing was
/// drawn since the last present (dirty rect is `None` → no-op
/// memcpy, just the rotation).
pub fn present() {
    if let Some(d) = FB.lock().as_mut() {
        d.present();
    }
}

/// FNV-1a checksum of the front buffer (what the display sees).
/// `None` if the driver isn't initialised.
pub fn front_fnv1a() -> Option<u64> {
    FB.lock().as_ref().map(|d| fnv1a(d.front))
}

/// FNV-1a checksum of the active back buffer (next thing the
/// display will see after `present()`). `None` if the driver
/// isn't initialised.
pub fn back_fnv1a() -> Option<u64> {
    FB.lock().as_mut().map(|d| d.active_back().fnv1a())
}

/// Number of `present()` calls that found a non-empty dirty rect
/// (i.e. actually copied bytes). Boot banner uses this to confirm
/// the buffer chain has cycled.
pub fn presents() -> u64 {
    FB.lock().as_ref().map(|d| d.presents).unwrap_or(0)
}

/// The firmware GOP framebuffer (raw MMIO ptr + its surface info),
/// captured at boot independent of the `FB` Driver singleton. The Slint
/// launcher renders straight into this surface via its own `*mut u8`
/// view -- entry_uefi hands `launcher::run` the original GOP ptr, while
/// `FB` may have switched its front to a secondary virtio-gpu DMA
/// surface -- and QEMU's SDL primary display is this GOP, so it's what
/// `/screen` must read to capture the live UI.
static GOP_SCREEN: Mutex<Option<(usize, FrameBufferInfo)>> = Mutex::new(None);

/// Record the GOP MMIO base + its surface info for `/screen` snapshots.
/// Called once at boot with the same `gop_ptr` handed to `launcher::run`,
/// so a snapshot reads exactly the surface the launcher paints.
pub fn set_gop_screen(ptr: usize, info: FrameBufferInfo) {
    *GOP_SCREEN.lock() = Some((ptr, info));
}

/// Snapshot the live GOP screen as tightly-packed RGB8 (`width*height*3`)
/// for the `/screen` see-and-drive endpoint -- the launcher's actual
/// render target, decoded per its pixel format. `None` until
/// `set_gop_screen` has run (no GOP at boot, or on the host test build).
pub fn snapshot_gop_rgb() -> Option<(Vec<u8>, usize, usize)> {
    let snap = *GOP_SCREEN.lock();
    let (ptr, info) = snap?;
    if ptr == 0 {
        return None;
    }
    // SAFETY: `ptr` + `info.byte_len` describe the firmware-mapped GOP
    // MMIO captured at boot (lives 'static, single-threaded). Read-only.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, info.byte_len) };
    let rgb = crate::screenshot::framebuffer_to_rgb(
        bytes,
        info.width,
        info.height,
        info.stride,
        info.bytes_per_pixel,
        info.pixel_format,
    );
    Some((rgb, info.width, info.height))
}

// ── Cursor sprite (#596) ─────────────────────────────────────────────
//
// The cursor-sprite painter lives here (rather than in
// `ui_apps::launcher`) so it compiles on the host target
// (`x86_64-pc-windows-msvc` / `x86_64-unknown-linux-gnu`) and its
// inline tests can run via `cargo test --lib`. `ui_apps` is gated on
// `#[cfg(all(target_os = "uefi", ...))]` and the host test runner
// never sees it; `framebuffer` is unconditionally compiled (lib.rs
// `pub mod framebuffer;` carries no cfg gate).
//
// `launcher.rs::paint_cursor_sprite` delegates here via
// `framebuffer::paint_cursor_sprite_into`.

/// 12×18 pixel arrow cursor bitmap. Row 0 = top of arrow, col 0
/// (leftmost) is the MSB side of the stored `u16`. Bit numbering:
/// bit 11 (`0b100000000000`) is the leftmost pixel, bit 0
/// (`0b000000000001`) is the rightmost. Hot-spot at top-left corner
/// so the on-screen tip matches the pointer position.
///
/// Exported for the launcher's `paint_cursor_sprite` delegation and
/// for the unit tests below.
pub const CURSOR_ARROW: [u16; 18] = [
    0b100000000000,
    0b110000000000,
    0b111000000000,
    0b111100000000,
    0b111110000000,
    0b111111000000,
    0b111111100000,
    0b111111110000,
    0b111111111000,
    0b111111111100,
    0b111111110000,
    0b111110000000,
    0b110011000000,
    0b100011000000,
    0b000001100000,
    0b000001100000,
    0b000000110000,
    0b000000110000,
];

/// Width in pixels of the `CURSOR_ARROW` bitmap.
pub const CURSOR_W: usize = 12;

/// Height in pixels of the `CURSOR_ARROW` bitmap.
pub const CURSOR_H: usize = 18;

/// Scale a device-space absolute value (`0..=value_max` — e.g. a
/// virtio-tablet `EV_ABS` coordinate over QEMU's `0..=32767` range)
/// to a framebuffer pixel coordinate in `[0, extent)`.
///
/// #596: virtio-input absolute pointers report position in their own
/// calibrated range, not screen pixels. The pointer-drain consumer
/// (`launcher::drain_pointer_into_slint_window`) must apply this
/// mapping — per `arch::uefi::pointer::PointerEvent::AbsMove`'s "the
/// consumer applies the device's calibration to map to screen pixels"
/// contract — before the value becomes a cursor coordinate. Without
/// it the cursor lands at e.g. (22000, 17000) on a 1280×800 surface,
/// far off every edge, which reads as "no cursor on screen".
///
/// Guarded + clamped: a non-positive `value_max` or zero `extent`
/// yields 0, negative inputs floor at 0, and a value at or beyond
/// `value_max` maps to the last on-screen pixel (`extent - 1`) rather
/// than off the edge.
pub fn scale_to_extent(value: i32, value_max: i64, extent: usize) -> i32 {
    if value_max <= 0 || extent == 0 {
        return 0;
    }
    let v = value.max(0) as i64;
    let last = extent as i64 - 1;
    ((v * last) / value_max).clamp(0, last) as i32
}

/// Paint the cursor-arrow sprite into a raw `*mut u32` framebuffer.
///
/// Each pixel slot is one `u32` in the framebuffer's native pixel
/// order; `stride` is in pixels (not bytes — same convention the GOP
/// `PixelsPerScanLine` field uses). Only the `1`-bits in
/// `CURSOR_ARROW` are written — background pixels are left untouched
/// (no erase-on-repaint: the save-under responsibility lies with the
/// caller's repaint cycle, i.e. Slint's next `draw_if_needed` will
/// overwrite the cursor region and the caller redraws on top each
/// frame).
///
/// Bounds-checked: any row or column that would fall outside
/// `[0, width) × [0, height)` is skipped silently. The pixel value
/// `0xFFFF_FFFF` (white — valid for both RGBX and BGRX pixel orders)
/// is written at every lit bit.
///
/// # Safety
///
/// `buf` must point to a writable allocation of at least
/// `stride * height` `u32` elements. The call is sound when `buf`
/// is the GOP MMIO base as captured by `launcher::run` (firmware-
/// mapped `'static` MMIO, single-threaded boot) or a heap-allocated
/// `Vec<u32>` in tests.
pub unsafe fn paint_cursor_sprite_into(
    buf: *mut u32,
    width: usize,
    height: usize,
    stride: usize,
    cx: usize,
    cy: usize,
) {
    let pixel: u32 = 0xFFFF_FFFF;
    for (row, mask) in CURSOR_ARROW.iter().enumerate() {
        let y = cy + row;
        if y >= height {
            break;
        }
        for col in 0..CURSOR_W {
            let bit = (mask >> (CURSOR_W - 1 - col)) & 1;
            if bit == 0 {
                continue;
            }
            let x = cx + col;
            if x >= width {
                continue;
            }
            // SAFETY: bounds checked above; caller guarantees allocation.
            unsafe {
                buf.add(y * stride + x).write_volatile(pixel);
            }
        }
    }
}

// ── Cursor-sprite unit tests ──────────────────────────────────────────
//
// Run via `cargo test --lib --target x86_64-pc-windows-msvc` from
// `crates/arest-kernel`. These tests exercise `paint_cursor_sprite_into`
// against a heap-allocated mock framebuffer so no UEFI target is
// required. Mirror of the lock+ring patterns in `pointer.rs` tests.

#[cfg(test)]
mod cursor_tests {
    use super::*;
    use alloc::vec;

    /// #596 regression: `scale_to_extent` maps a virtio-tablet device-
    /// space coordinate (0..=32767) into framebuffer pixels, so the
    /// cursor lands on-screen instead of at the raw off-screen value
    /// that produced "no cursor on screen".
    #[test]
    fn scale_to_extent_maps_device_range_into_screen() {
        // Edges and midpoint of QEMU's 0..=32767 abs range → 1280 wide.
        assert_eq!(scale_to_extent(0, 32767, 1280), 0);
        assert_eq!(scale_to_extent(32767, 32767, 1280), 1279);
        assert_eq!(scale_to_extent(16384, 32767, 1280), 639);
        // The repro coordinates (22000, 17000) that landed off-screen
        // when used raw are on-screen once scaled.
        assert!(scale_to_extent(22000, 32767, 1280) < 1280);
        assert!(scale_to_extent(17000, 32767, 800) < 800);
        // A value past the device max clamps to the last pixel.
        assert_eq!(scale_to_extent(40000, 32767, 1280), 1279);
        // Degenerate inputs are guarded, not panics.
        assert_eq!(scale_to_extent(100, 0, 1280), 0);
        assert_eq!(scale_to_extent(100, 32767, 0), 0);
        assert_eq!(scale_to_extent(-5, 32767, 1280), 0);
    }

    /// `paint_cursor_sprite_into` writes the expected white pixels at
    /// the correct framebuffer offsets for a given (cx, cy). We use a
    /// 64×64 buffer (stride = 64) and plant the cursor at (10, 5) so
    /// the full 12×18 sprite fits within bounds.
    ///
    /// For each row we compare every column in `0..CURSOR_W` against
    /// the corresponding bit in `CURSOR_ARROW[row]` — lit bits must
    /// be `0xFFFF_FFFF`, unlit bits must be unchanged (0).
    #[test]
    fn cursor_pixels_written_at_expected_offsets() {
        const W: usize = 64;
        const H: usize = 64;
        const STRIDE: usize = W;
        let mut buf: alloc::vec::Vec<u32> = vec![0u32; STRIDE * H];

        let cx: usize = 10;
        let cy: usize = 5;

        unsafe {
            paint_cursor_sprite_into(buf.as_mut_ptr(), W, H, STRIDE, cx, cy);
        }

        // Verify every pixel in the sprite bounding box for the first
        // two rows (representative; full bitmap coverage via background
        // test below).
        for check_row in 0..2usize {
            let mask = CURSOR_ARROW[check_row];
            for col in 0..CURSOR_W {
                let expected_bit = (mask >> (CURSOR_W - 1 - col)) & 1;
                let pixel = buf[(cy + check_row) * STRIDE + cx + col];
                if expected_bit == 1 {
                    assert_eq!(pixel, 0xFFFF_FFFF,
                        "row {check_row} col {col}: expected white pixel");
                } else {
                    assert_eq!(pixel, 0,
                        "row {check_row} col {col}: expected untouched (0), got {pixel:#010x}");
                }
            }
        }

        // Spot-check: a pixel well outside the sprite bounding box must
        // be untouched (confirms only lit bits are written, not a filled rect).
        let outside = buf[(cy + 3) * STRIDE + cx + CURSOR_W + 1];
        assert_eq!(outside, 0, "pixel outside sprite bbox must be 0");
    }

    /// At position (0, 0) the painter writes the top-left pixel because
    /// `CURSOR_ARROW[0]` has bit 11 set. This confirms that the inner
    /// function does NOT apply the (0,0) sentinel skip — that guard is
    /// the caller's responsibility (`launcher::paint_cursor_sprite`
    /// checks `cx == 0 && cy == 0` and returns before calling here).
    #[test]
    fn paint_at_origin_writes_top_left_pixel() {
        const W: usize = 64;
        const H: usize = 64;
        const STRIDE: usize = W;
        let mut buf: alloc::vec::Vec<u32> = vec![0u32; STRIDE * H];

        unsafe {
            paint_cursor_sprite_into(buf.as_mut_ptr(), W, H, STRIDE, 0, 0);
        }

        // CURSOR_ARROW[0] = 0b100000000000 — bit 11 set → pixel (0, 0) white.
        assert_eq!(buf[0], 0xFFFF_FFFF,
            "top-left pixel must be white when cursor placed at (0,0)");
    }

    /// Clipping guard: positioning the cursor at (W-1, H-1) means only
    /// the top-left pixel of the sprite is in bounds. The function must
    /// not panic or write out-of-bounds (the `Vec` debug-build index
    /// check would catch an OOB `ptr.add(...)` dereference if it
    /// exceeded the allocation).
    #[test]
    fn sprite_clipped_at_right_bottom_edge() {
        const W: usize = 64;
        const H: usize = 64;
        const STRIDE: usize = W;
        let mut buf: alloc::vec::Vec<u32> = vec![0u32; STRIDE * H];

        let cx = W - 1;
        let cy = H - 1;

        // Must not panic — the per-column `if x >= width { continue; }`
        // and per-row `if y >= height { break; }` guards inside
        // `paint_cursor_sprite_into` clamp every write.
        unsafe {
            paint_cursor_sprite_into(buf.as_mut_ptr(), W, H, STRIDE, cx, cy);
        }

        // CURSOR_ARROW[0] bit 11 (leftmost) = 1 → pixel (cx, cy) lit.
        let top_left_bit = (CURSOR_ARROW[0] >> (CURSOR_W - 1)) & 1;
        if top_left_bit == 1 {
            assert_eq!(buf[cy * STRIDE + cx], 0xFFFF_FFFF,
                "only in-bounds pixel should be white");
        }

        // Buffer length must be intact (no realloc / OOB write).
        assert_eq!(buf.len(), STRIDE * H, "buffer length must be unchanged");
    }

    /// Background pixels (the `0`-bits in the sprite mask) must not be
    /// erased. Pre-fill the buffer with a sentinel (`0xDEAD_BEEF`) and
    /// confirm that positions outside the lit sprite pixels still hold
    /// the sentinel after painting.
    #[test]
    fn background_pixels_not_erased() {
        const W: usize = 64;
        const H: usize = 64;
        const STRIDE: usize = W;
        let mut buf: alloc::vec::Vec<u32> = vec![0xDEAD_BEEFu32; STRIDE * H];

        let cx: usize = 10;
        let cy: usize = 5;

        unsafe {
            paint_cursor_sprite_into(buf.as_mut_ptr(), W, H, STRIDE, cx, cy);
        }

        // Row above the sprite must be untouched.
        if cy > 0 {
            assert_eq!(buf[(cy - 1) * STRIDE + cx], 0xDEAD_BEEF,
                "pixel above sprite must be untouched");
        }

        // Within the sprite bbox, a zero-bit in row 0 must be untouched.
        // CURSOR_ARROW[0] = 0b100000000000 → col 1 (bit 10) = 0.
        let row0_col1_bit = (CURSOR_ARROW[0] >> (CURSOR_W - 2)) & 1;
        if row0_col1_bit == 0 {
            assert_eq!(buf[cy * STRIDE + cx + 1], 0xDEAD_BEEF,
                "zero-bit pixel inside sprite bbox must be untouched");
        }
    }

    /// Full-bitmap pixel-exact verification: every pixel in the entire
    /// 12×18 sprite bounding box matches the expected bit from
    /// `CURSOR_ARROW`. This is the comprehensive correctness proof.
    #[test]
    fn full_sprite_pixel_exact_verification() {
        const W: usize = 64;
        const H: usize = 64;
        const STRIDE: usize = W;
        let mut buf: alloc::vec::Vec<u32> = vec![0u32; STRIDE * H];

        let cx: usize = 5;
        let cy: usize = 5;

        unsafe {
            paint_cursor_sprite_into(buf.as_mut_ptr(), W, H, STRIDE, cx, cy);
        }

        for (row, mask) in CURSOR_ARROW.iter().enumerate() {
            for col in 0..CURSOR_W {
                let expected_bit = (mask >> (CURSOR_W - 1 - col)) & 1;
                let pixel = buf[(cy + row) * STRIDE + cx + col];
                let expected_pixel = if expected_bit == 1 { 0xFFFF_FFFFu32 } else { 0u32 };
                assert_eq!(pixel, expected_pixel,
                    "mismatch at sprite row {row} col {col}: \
                     expected {expected_pixel:#010x} got {pixel:#010x}");
            }
        }
    }
}

/// Tiny embedded 8x8 ASCII font — only the glyphs the boot-time
/// paint demo writes ("AREST kernel"). Missing chars render as a
/// solid block (`0xFF`-filled byte per row) so an absent letter is
/// visually obvious. Add more chars here as new boot-time text
/// lands; for sustained graphics work (#270/#271 Doom, #129 UI)
/// vendor a full 8x8 PC ROM font (~760 B for ASCII printable).
mod font {
    /// 8x8 glyph: 8 rows, each row's 8 pixels packed LSB-first
    /// into a byte (bit 0 = leftmost column). All-zero rows are
    /// background.
    pub type Glyph = [u8; 8];

    pub fn glyph(ch: char) -> Glyph {
        match ch {
            ' ' => [0; 8],
            'A' => [0x18, 0x24, 0x42, 0x42, 0x7E, 0x42, 0x42, 0x00],
            'E' => [0x7E, 0x02, 0x02, 0x3E, 0x02, 0x02, 0x7E, 0x00],
            'R' => [0x3E, 0x42, 0x42, 0x3E, 0x12, 0x22, 0x42, 0x00],
            'S' => [0x7C, 0x02, 0x02, 0x3C, 0x40, 0x40, 0x3E, 0x00],
            'T' => [0x7F, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x00],
            'k' => [0x02, 0x02, 0x22, 0x12, 0x0E, 0x12, 0x22, 0x00],
            'e' => [0x00, 0x00, 0x3C, 0x42, 0x7E, 0x02, 0x3C, 0x00],
            'r' => [0x00, 0x00, 0x3A, 0x46, 0x02, 0x02, 0x02, 0x00],
            'n' => [0x00, 0x00, 0x3A, 0x46, 0x42, 0x42, 0x42, 0x00],
            'l' => [0x06, 0x02, 0x02, 0x02, 0x02, 0x02, 0x07, 0x00],
            // Missing → solid block makes the gap visible.
            _ => [0xFF; 8],
        }
    }
}
