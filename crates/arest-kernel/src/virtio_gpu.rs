// crates/arest-kernel/src/virtio_gpu.rs
//
// virtio-gpu guest driver wrapper (#371).
//
// Sibling of `virtio.rs` (virtio-net + virtio-blk wrappers). Brings up
// a `virtio_drivers::device::gpu::VirtIOGpu` against the PCI device
// Track AAA's `pci::find_virtio_gpu` (#370) discovers, allocates a 2D
// resource, attaches it to scanout 0, and exposes the resource's DMA-
// backed byte buffer + a `present()` that flushes it to the host.
//
// ── How this slots into the framebuffer pipeline ────────────────────
// The kernel's `framebuffer::Driver` keeps a `front: &'static mut [u8]`
// + two heap-allocated back buffers (see `framebuffer.rs`). On the
// existing GOP path that front buffer is firmware-mapped MMIO; with
// virtio-gpu it's the DMA region the device sees as "resource id 0xbabe"
// (the `RESOURCE_ID_FB` const inside virtio-drivers). Either way the
// triple-buffer / damage-rect / present() machinery above is identical
// — only the front-buffer flush path differs.
//
// `framebuffer::install_virtio_gpu(driver)` (in framebuffer.rs) takes
// ownership of the driver, lifts the buffer pointer behind the `Driver`
// singleton, and on each `present()` calls back into us via
// `flush_active_surface()` to issue `VirtIOGpu::flush()` after the
// memcpy lands.
//
// ── Lifetime story ──────────────────────────────────────────────────
// `VirtIOGpu::setup_framebuffer()` returns `&mut [u8]` whose lifetime
// is tied to `&mut self`. We need `&'static mut [u8]` for
// `framebuffer::install`. The driver is parked in a static `spin::Mutex`
// for the lifetime of the kernel (no shutdown path), so the Dma<H>
// inside the driver is also `'static`. We promote the slice via raw-
// pointer cast — sound because:
//   * the `Mutex<Option<VirtIoGpuDriver>>` keeps the driver alive,
//   * nothing else borrows the surface bytes (the framebuffer::Driver
//     is the sole writer, and `present()` interleaves with `flush()`
//     via the same mutex held in framebuffer.rs),
//   * the kernel is single-threaded — no SMP scheduler yet.
//
// ── Sync shenanigans ────────────────────────────────────────────────
// `VirtIOGpu` (like `VirtIONet`) is `!Sync` because it holds a
// `VirtQueue` which contains `NonNull` pointers into DMA. The
// `spin::Mutex` wrapper provides the Sync impl we need to park it in a
// `static`. Same pattern `block.rs` uses for `VirtIOBlkDevice`.
//
// ── Why we don't use `VirtIOGpu::setup_framebuffer` blindly ─────────
// `setup_framebuffer` returns the buffer at the resolution the device
// reports. The returned `&mut [u8]` is the exact byte layout the host
// blits when `flush()` is called (B8G8R8A8UNORM, 4 bpp, stride =
// width*4). We surface those numbers via `width()/height()` so the
// caller can build a `FrameBufferInfo` with `pixel_format = Bgr` and
// `bytes_per_pixel = 4` to pass through to `framebuffer::install`. No
// further byte-shuffling needed — the existing `write_pixel` Bgr arm
// already matches the format virtio-gpu expects.

use core::ptr::NonNull;

use spin::Mutex;
use virtio_drivers::device::gpu::VirtIOGpu;
use virtio_drivers::transport::DeviceType;
use virtio_drivers::transport::pci::{PciTransport, bus::{DeviceFunction, PciRoot}};

use crate::pci::PciDevice;
use crate::virtio::{KernelHal, PioCam};

/// Fully-typed VirtIOGpu driver handle. Mirrors `VirtIONetDevice` /
/// `VirtIOBlkDevice` in `virtio.rs` — both `KernelHal` (DMA + addr
/// translation) and `PciTransport` (legacy-PIO config access via
/// `PioCam`) are shared with the other virtio drivers.
pub type VirtIoGpuDevice = VirtIOGpu<KernelHal, PciTransport>;

/// Failure shape for the init path. Mirrors `try_init_virtio_*`'s
/// "log + return None" pattern in `virtio.rs` but as an enum so the
/// caller can decide whether to print or panic.
#[derive(Debug, Clone, Copy)]
pub enum InitError {
    /// `PciTransport::new` rejected the BARs / capabilities.
    PciTransport,
    /// The transport's reported device_type was not GPU.
    WrongDeviceType,
    /// `VirtIOGpu::new` failed (DMA pool exhausted, virtqueue setup error).
    VirtIoGpuNew,
    /// `setup_framebuffer` failed (display info / scanout / resource alloc).
    SetupFramebuffer,
}

/// Wrapper around the active virtio-gpu driver instance.
///
/// Stores both the driver handle and the resolution + byte-length the
/// host advertised at scanout setup time. The byte-buffer pointer is
/// reconstructed on demand via `framebuffer_buffer()` rather than
/// stashed here — the `Dma<H>` inside the driver owns the storage and
/// we keep it alive for the lifetime of `Self`.
pub struct VirtIoGpuDriver {
    device: VirtIoGpuDevice,
    width: u32,
    height: u32,
    /// Cached pointer to the start of the framebuffer DMA region.
    /// Captured during `init_from_pci` after `setup_framebuffer` runs.
    /// Lives as long as `device` (the `Dma<H>` is owned by the driver).
    fb_ptr: NonNull<u8>,
    /// Byte length of the framebuffer DMA region. Equals
    /// `width * height * 4` (B8G8R8A8UNORM).
    fb_len: usize,
}

// SAFETY: VirtIoGpuDevice is !Send/!Sync only because it owns DMA-region
// pointers (NonNull<u8>) inside the VirtQueue / Dma fields. We
// guarantee single-CPU access to the driver via the static
// `Mutex<Option<VirtIoGpuDriver>>` below; the same shape `virtio.rs`
// uses for VirtIONet and `block.rs` uses for VirtIOBlk.
unsafe impl Send for VirtIoGpuDriver {}
unsafe impl Sync for VirtIoGpuDriver {}

impl VirtIoGpuDriver {
    /// Width of scanout 0 in pixels (B8G8R8A8UNORM).
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height of scanout 0 in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Byte length of the framebuffer surface. `width * height * 4`.
    pub fn buffer_len(&self) -> usize {
        self.fb_len
    }

    /// Borrow the framebuffer DMA region as a mutable byte slice. The
    /// returned slice lives as long as `&mut self`; promoting it to
    /// `'static` (see `framebuffer.rs::install_virtio_gpu`) requires
    /// caller-side justification — namely that this driver is parked in
    /// a static for the rest of the kernel's lifetime.
    pub fn framebuffer_buffer(&mut self) -> &mut [u8] {
        // SAFETY: `fb_ptr` was produced by `setup_framebuffer`'s call
        // to `Dma::raw_slice` — non-null, page-aligned, exclusive to
        // `self.device`'s `Dma<H>`. `fb_len` is the matching capacity.
        // The `&mut self` borrow ensures no other code is touching the
        // driver (and therefore the DMA region) for the lifetime of
        // the returned slice.
        unsafe { core::slice::from_raw_parts_mut(self.fb_ptr.as_ptr(), self.fb_len) }
    }

    /// Submit the current framebuffer contents to scanout 0.
    /// Wraps `VirtIOGpu::flush` (transfer_to_host_2d + resource_flush
    /// per virtio-gpu spec sec 5.7.6.7 / 5.7.6.8). Returns `Ok(())` on
    /// success, otherwise the underlying virtio-drivers error.
    pub fn present(&mut self) -> virtio_drivers::Result<()> {
        self.device.flush()
    }
}

/// Bring up the virtio-gpu driver against `dev` (the result of
/// `pci::find_virtio_gpu`). Returns the constructed driver on success.
///
/// Pipeline:
///   1. Build `DeviceFunction` from the PciDevice's bus/device/function
///      tuple (matches the virtio-net / virtio-blk init flow in
///      `virtio.rs::try_init_virtio_*`).
///   2. Construct a `PciRoot` with the legacy-PIO `PioCam` and a
///      `PciTransport` against this slot. virtio-drivers handshakes
///      the device's MMIO BARs through the transport — same code path
///      virtio-net + virtio-blk use.
///   3. Verify the transport's reported device type is GPU. Defensive
///      — `find_virtio_gpu` already filters by device-id 0x1050, so
///      mismatch here would mean the PCI capability layout was
///      malformed.
///   4. Call `VirtIOGpu::new(transport)` — this negotiates virtio
///      features (we accept RING_INDIRECT_DESC + RING_EVENT_IDX), sets
///      up the control + cursor virtqueues, and finishes init.
///   5. Call `setup_framebuffer()` — issues GET_DISPLAY_INFO (sec
///      5.7.6.1), RESOURCE_CREATE_2D (sec 5.7.6.2),
///      RESOURCE_ATTACH_BACKING (sec 5.7.6.4), SET_SCANOUT (sec
///      5.7.6.5). Returns a `&mut [u8]` view of the DMA region; we
///      capture the pointer + length so the wrapper can re-borrow it
///      through `framebuffer_buffer()` later.
pub fn init_from_pci(dev: PciDevice) -> Result<VirtIoGpuDriver, InitError> {
    let device_function = DeviceFunction {
        bus: dev.bus,
        device: dev.device,
        function: dev.function,
    };
    let mut root = PciRoot::new(PioCam);
    let transport = PciTransport::new::<KernelHal, _>(&mut root, device_function)
        .map_err(|_| InitError::PciTransport)?;
    {
        // Trait method imported in a narrow scope so the rest of the
        // module doesn't see two `Transport` extension imports
        // colliding (the gpu module needs the inherent flush()).
        use virtio_drivers::transport::Transport;
        if !matches!(transport.device_type(), DeviceType::GPU) {
            return Err(InitError::WrongDeviceType);
        }
    }
    let mut device = VirtIoGpuDevice::new(transport).map_err(|_| InitError::VirtIoGpuNew)?;

    // Allocate 2D resource + scanout in one call. Returns the DMA
    // slice the device blits from on flush(). We immediately capture
    // the raw pointer + length — the slice borrow is tied to `&mut
    // device`, but the underlying `Dma<H>` storage is owned by the
    // driver and lives as long as `VirtIoGpuDriver`.
    let fb_slice = device
        .setup_framebuffer()
        .map_err(|_| InitError::SetupFramebuffer)?;
    let fb_len = fb_slice.len();
    let fb_ptr = NonNull::new(fb_slice.as_mut_ptr())
        .expect("setup_framebuffer returned a null buffer");

    // Resolution comes back from get_display_info inside
    // setup_framebuffer; re-query it here for the public accessors
    // (cheaper than threading it back through the inherent API).
    // Per virtio-gpu spec sec 5.7.6.7: the buffer the device blits
    // is laid out as `width * height` u32 pixels in B8G8R8A8UNORM
    // order, so width = fb_len / (height * 4). We instead probe
    // `resolution()` for clarity — it's a single GET_DISPLAY_INFO
    // round-trip on the control queue.
    let (width, height) = device
        .resolution()
        .map_err(|_| InitError::SetupFramebuffer)?;

    Ok(VirtIoGpuDriver { device, width, height, fb_ptr, fb_len })
}

// ── Singleton parking lot ──────────────────────────────────────────
//
// The driver lives behind a static `Mutex` so:
//   1. The framebuffer subsystem can call back into `present()` from
//      `framebuffer::present()` without threading the driver through
//      its API.
//   2. The DMA-backed framebuffer surface lives as long as the
//      kernel, satisfying `framebuffer::install_virtio_gpu`'s
//      `'static` lifetime requirement on the buffer.
//   3. Future subsystems (cursor sprite for the REPL, screenshot
//      command) can grab the same driver without re-walking PCI.
//
// `install` takes ownership and parks the driver. `with_driver` is a
// thin closure-borrow accessor for callers (the framebuffer driver's
// flush path uses it).

static GPU: Mutex<Option<VirtIoGpuDriver>> = Mutex::new(None);

/// Park the driver in the singleton. Called once at boot from
/// `entry_uefi.rs::kernel_run_uefi` immediately before
/// `framebuffer::install_virtio_gpu` so the framebuffer install path
/// can reach back through the singleton during `present()`.
pub fn install(driver: VirtIoGpuDriver) {
    *GPU.lock() = Some(driver);
}

/// Borrow the parked driver for a closure. Returns `None` when no
/// driver was installed (no virtio-gpu device on the PCI bus, or init
/// failed).
pub fn with_driver<R>(f: impl FnOnce(&mut VirtIoGpuDriver) -> R) -> Option<R> {
    GPU.lock().as_mut().map(f)
}

/// Submit the parked driver's current framebuffer to scanout 0.
/// Convenience wrapper used by `framebuffer::present()` after the
/// memcpy lands. Returns `false` when no driver is installed (so the
/// framebuffer driver knows to fall through to GOP behaviour) or when
/// `flush()` returned an error (silent — virtio-gpu errors during
/// steady-state flush are typically transient stale fence ids).
pub fn flush_active_surface() -> bool {
    match GPU.lock().as_mut() {
        Some(d) => d.present().is_ok(),
        None => false,
    }
}
