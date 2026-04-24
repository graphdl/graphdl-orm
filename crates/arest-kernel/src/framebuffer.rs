// crates/arest-kernel/src/framebuffer.rs
//
// Triple-buffered linear framebuffer with damage tracking. Sits on
// top of the bootloader-provided `BootInfo.framebuffer` byte slice
// (the "front" buffer — what the display reads). Drawing API writes
// to one of two heap-allocated back buffers; `present()` copies the
// dirty rect from the active back buffer onto the front buffer and
// swaps to the other back. Three buffers total → producer never
// stalls waiting for the consumer (would matter if we had vsync
// signalling; without it, triple == double for stall behaviour but
// the chain is in place for when virtio-gpu or a real display
// controller lands and starts gating present() on flips).
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
// Why no virtio-gpu yet (#269): the bootloader-provided framebuffer
// already gives a usable surface under QEMU's default `-vga std`
// (1280x720x24bpp typical). virtio-gpu is the right answer for a
// production GPU stack but unnecessary for the boot-time graphics
// demo path that #270/#271 want — those can run against the BIOS
// framebuffer the bootloader already hands us.

use alloc::{vec, vec::Vec};
use bootloader_api::info::{FrameBufferInfo, PixelFormat};
use spin::Mutex;

/// Singleton driver state. `None` until `init` runs and `None`
/// forever if the bootloader didn't provide a framebuffer
/// (text-mode boot).
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
}

/// Channel-layout-aware pixel write. Bgr is what QEMU's standard
/// VGA reports under bootloader_api 0.11; Rgb covers physical
/// hardware that swaps the byte order. U8 is a greyscale fallback;
/// other formats are silently skipped rather than corrupting the
/// surface.
fn write_pixel(slot: &mut [u8], format: PixelFormat, c: Color) {
    match format {
        PixelFormat::Rgb => { slot[0] = c.r; slot[1] = c.g; slot[2] = c.b; }
        PixelFormat::Bgr => { slot[0] = c.b; slot[1] = c.g; slot[2] = c.r; }
        PixelFormat::U8  => {
            slot[0] = ((u16::from(c.r) + u16::from(c.g) + u16::from(c.b)) / 3) as u8;
        }
        _ => {}
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
struct Driver {
    info: FrameBufferInfo,
    front: &'static mut [u8],
    backs: [BackBuffer; 2],
    draw_idx: usize,
    presents: u64,
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
        }
        self.draw_idx ^= 1;
    }
}

/// Install the front buffer + allocate two heap-backed back
/// buffers. Caller hands in the bootloader-provided byte slice
/// (raw ptr + length) plus the format metadata.
///
/// # Safety
///
/// `buffer_ptr` + `buffer_len` must describe the live bootloader-
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
    *FB.lock() = Some(Driver { info, front, backs, draw_idx: 0, presents: 0 });
}

/// Surface metadata, lock-free. `None` if the driver wasn't
/// initialised (text-mode boot).
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
