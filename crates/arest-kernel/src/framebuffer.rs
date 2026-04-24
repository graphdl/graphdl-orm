// crates/arest-kernel/src/framebuffer.rs
//
// Linear framebuffer wrapper around the `bootloader_api::BootInfo`
// `framebuffer` field. The bootloader probes the BIOS-firmware VBE
// tables at boot and, when a graphics-mode framebuffer is available,
// fills `BootInfo.framebuffer` with the byte slice + format
// metadata. We grab it once during `init`, stash it behind a
// `spin::Mutex`, and expose pixel-level primitives the rest of the
// kernel calls.
//
// Why no virtio-gpu yet (#269): the bootloader-provided framebuffer
// already gives a usable surface under QEMU's default `-vga std`
// (1024x768x32 typical). virtio-gpu is the right answer for a
// production GPU stack but unnecessary for the boot-time graphics
// demo path that #270/#271 want — those can run against the BIOS
// framebuffer the bootloader already hands us.
//
// What's exposed today:
//   * `init(boot_info)`              — install the framebuffer
//                                       singleton from BootInfo.
//   * `info()`                       — width / height / stride /
//                                       bytes-per-pixel, for the
//                                       boot banner.
//   * `with_buffer(|fb| ...)`        — borrow the byte slice under
//                                       lock to render directly.
//   * `clear(r, g, b)`               — fill the whole surface.
//   * `put_pixel(x, y, r, g, b)`     — write a single pixel,
//                                       respecting the pixel format.
//
// All callers route through the `Mutex` so concurrent writes (REPL
// banner + game tick + future async work) serialise rather than
// tearing — same discipline `arch::serial` uses for COM1.

use bootloader_api::info::{FrameBufferInfo, PixelFormat};
use spin::Mutex;

/// Singleton framebuffer handle. `None` until `init` runs and `None`
/// forever if the bootloader didn't provide one (text-mode boot).
static FRAMEBUFFER: Mutex<Option<Framebuffer>> = Mutex::new(None);

/// Owned wrapper around the bootloader-provided byte buffer + info.
/// Holds a `&'static mut [u8]` because `BootInfo::framebuffer`
/// hands ownership to the kernel for the duration of the boot.
pub struct Framebuffer {
    info: FrameBufferInfo,
    buffer: &'static mut [u8],
}

impl Framebuffer {
    /// Surface metadata — width / height / stride / pixel format /
    /// bytes-per-pixel. Used by the boot banner and any code that
    /// needs to compute pixel offsets without locking the buffer.
    pub fn info(&self) -> FrameBufferInfo {
        self.info
    }

    /// Fill the entire surface with a solid colour. Walks the
    /// framebuffer in raw byte order, writing the pixel-format-
    /// appropriate channel layout per pixel.
    pub fn clear(&mut self, r: u8, g: u8, b: u8) {
        let bpp = self.info.bytes_per_pixel;
        let stride_bytes = self.info.stride * bpp;
        for y in 0..self.info.height {
            let row_start = y * stride_bytes;
            for x in 0..self.info.width {
                let off = row_start + x * bpp;
                write_pixel(&mut self.buffer[off..off + bpp], self.info.pixel_format, r, g, b);
            }
        }
    }

    /// Write a single pixel at `(x, y)`. No-op if out of bounds —
    /// callers guard themselves where the loop dimensions matter,
    /// this is just defence against off-by-one in pixel-art code.
    pub fn put_pixel(&mut self, x: usize, y: usize, r: u8, g: u8, b: u8) {
        if x >= self.info.width || y >= self.info.height {
            return;
        }
        let bpp = self.info.bytes_per_pixel;
        let off = y * self.info.stride * bpp + x * bpp;
        write_pixel(&mut self.buffer[off..off + bpp], self.info.pixel_format, r, g, b);
    }
}

/// Channel-layout-aware pixel write. PixelFormat::Rgb / Bgr / U8
/// are the formats QEMU's standard VGA produces under different
/// firmware configs; the others are uncommon on the BIOS path but
/// included so we silently degrade rather than panic.
fn write_pixel(slot: &mut [u8], format: PixelFormat, r: u8, g: u8, b: u8) {
    match format {
        PixelFormat::Rgb => {
            slot[0] = r;
            slot[1] = g;
            slot[2] = b;
        }
        PixelFormat::Bgr => {
            slot[0] = b;
            slot[1] = g;
            slot[2] = r;
        }
        PixelFormat::U8 => {
            // Greyscale — average the channels.
            slot[0] = ((u16::from(r) + u16::from(g) + u16::from(b)) / 3) as u8;
        }
        // Unknown/Unspecified — leave untouched rather than corrupt.
        _ => {}
    }
}

/// Install the framebuffer singleton from a raw byte pointer +
/// length pair plus the bootloader's metadata. Caller pulls these
/// out of `BootInfo.framebuffer.as_mut()` and hands them in — the
/// indirection keeps `framebuffer::install` from having to know
/// about the field-disjoint borrow rules `BootInfo` access requires
/// when other init paths (notably `arch::init_memory`) need the
/// rest of `BootInfo` afterwards.
///
/// # Safety
///
/// `buffer_ptr` + `buffer_len` must describe the live bootloader-
/// provided framebuffer, which lives for the kernel's entire boot
/// (it's part of the `&'static mut BootInfo` the bootloader passes).
/// No other code may hold a reference to the same bytes when this
/// is called.
pub unsafe fn install(info: FrameBufferInfo, buffer_ptr: *mut u8, buffer_len: usize) {
    // SAFETY: the caller-supplied region is the bootloader's
    // framebuffer mapping, which lives `'static`. The Mutex
    // serialises every subsequent borrow.
    let buffer: &'static mut [u8] = unsafe {
        core::slice::from_raw_parts_mut(buffer_ptr, buffer_len)
    };
    *FRAMEBUFFER.lock() = Some(Framebuffer { info, buffer });
}

/// Borrow the framebuffer info without locking the buffer. Returns
/// `None` if the framebuffer wasn't initialised (text-mode boot or
/// `init` skipped).
pub fn info() -> Option<FrameBufferInfo> {
    FRAMEBUFFER.lock().as_ref().map(|fb| fb.info())
}

/// Borrow the framebuffer for a closure. Locks the singleton for the
/// closure's duration. The closure doesn't run if the framebuffer
/// wasn't initialised. Use this rather than holding the lock
/// across long-running work.
pub fn with_buffer<R>(f: impl FnOnce(&mut Framebuffer) -> R) -> Option<R> {
    let mut guard = FRAMEBUFFER.lock();
    guard.as_mut().map(f)
}
