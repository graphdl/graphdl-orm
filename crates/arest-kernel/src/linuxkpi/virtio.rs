// crates/arest-kernel/src/linuxkpi/virtio.rs
//
// Linux virtio core shim. The vendored `virtio_input.c` is built on
// top of Linux's virtio bus + virtqueue layer — it never touches the
// underlying transport (PCI / MMIO / channel-IO) directly. So our
// shim sits at the same level: provide `struct virtio_device`,
// `struct virtqueue`, and the `virtio_find_vqs` / `virtio_cread*` /
// `virtqueue_*` thunks that virtio-input calls.
//
// Underneath, AREST has its own pure-Rust `virtio-drivers` integration
// (`crates/arest-kernel/src/virtio.rs`) that talks directly to the PCI
// or MMIO transport via the `rcore-os/virtio-drivers` crate. The
// linuxkpi side wraps that — when virtio-input asks for a virtqueue
// and the foundation slice ships, we'll defer to AREST's existing
// VirtIO transport. On THIS slice (foundation only), the goal is
// clean linkage; the wiring to the real transport is #459b.
//
// What we model
// -------------
// Linux's virtio surface layered onto our shim:
//
//   * `struct virtio_device` — per-device handle. Holds the bus
//     vtable (`config`), the embedded `device`, and a `void *priv`
//     for the driver to stash state on. virtio-input writes to
//     `vdev->priv` and reads `vdev->dev`, `vdev->config->del_vqs`,
//     `vdev->index`.
//   * `struct virtqueue` — per-queue handle. Holds a back-pointer
//     to its `virtio_device` (so callbacks can find their device)
//     plus opaque transport state. virtio-input only reaches
//     `vq->vdev->priv` and the standalone `virtqueue_*` ABI.
//   * `virtio_find_vqs(...)` — request N virtqueues with names +
//     callbacks. On the foundation slice this returns -ENODEV
//     (no underlying transport wired); the caller's driver-probe
//     fails cleanly, which is the correct behaviour for a slice
//     that doesn't yet do device discovery.
//   * `virtqueue_add_inbuf` / `virtqueue_add_outbuf` /
//     `virtqueue_get_buf` / `virtqueue_kick` /
//     `virtqueue_get_vring_size` / `virtqueue_detach_unused_buf` —
//     standalone virtqueue ABI. Foundation-slice stubs return
//     conservative defaults (0 for size, NULL for get_buf, etc).
//   * `virtio_cread*` / `virtio_cwrite*` — config-space read/write
//     thunks. virtio-input reads its device's name, serial,
//     vendor/product, and per-axis abs-info via these. Foundation
//     stubs return zeroed memory (matches an absent device).
//   * `virtio_has_feature(vdev, feat)` — feature-bit query. Returns
//     true unconditionally on the foundation slice so virtio-input's
//     `VIRTIO_F_VERSION_1` check passes (real wiring restores
//     correct semantics in #459b).
//   * `virtio_device_ready(vdev)` / `virtio_reset_device(vdev)` —
//     device lifecycle hooks. No-ops on the slice.

use core::ffi::{c_char, c_int, c_void};

/// `struct virtio_device` — Linux's per-device handle. Layout
/// matches the C stub `vendor/linux/include/linux/virtio.h`. Field
/// order matters because the vendored virtio_input.c reaches them
/// by name through the `vdev->X` syntax (which compiles to an
/// offset-based load, so an out-of-order Rust mirror would silently
/// misalign).
#[repr(C)]
pub struct VirtioDevice {
    /// Per-device unique index (used in `virtio%d/input0` phys
    /// strings). Real Linux assigns this from a global counter; we
    /// hardcode 0 since the foundation slice never instantiates a
    /// real device.
    pub index: c_int,
    /// Driver-private pointer. virtio-input writes its per-device
    /// `struct virtio_input *` here.
    pub priv_: *mut c_void,
    /// Embedded `struct device` — at known offset for `&vdev->dev`
    /// expressions in the driver.
    pub dev: super::device::Device,
    /// Bus-supplied vtable. virtio-input reaches `vdev->config->
    /// del_vqs(vdev)` on probe failure / remove. NULL on the
    /// foundation slice — the driver's failure paths handle a NULL
    /// del_vqs check, but those paths only fire if probe got past
    /// the early VIRTIO_F_VERSION_1 check (which we currently
    /// short-circuit to -ENODEV via `virtio_find_vqs`).
    pub config: *mut VirtioConfigOps,
    /// `struct virtio_device_id id` — populated by the bus when
    /// matching this device against a registered driver's
    /// id_table. Zeroed on the foundation slice.
    pub id: super::driver::VirtioDeviceId,
}

unsafe impl Send for VirtioDevice {}
unsafe impl Sync for VirtioDevice {}

/// `struct virtio_config_ops` — bus-supplied vtable. Real Linux
/// has ~20 entries; virtio-input touches only `del_vqs`. We expose
/// a minimal subset for the C-side layout to match.
#[repr(C)]
pub struct VirtioConfigOps {
    pub del_vqs: Option<unsafe extern "C" fn(*mut VirtioDevice)>,
}

/// `struct virtqueue` — per-queue handle. virtio-input reaches:
///   * `vq->vdev` — back-pointer to the device.
///   * `vq->index` — queue index (0 = events, 1 = status on
///     virtio-input).
///   * `vq->callback` — the callback the bus invokes when the queue
///     gets a notification.
///   * Anything else is opaque.
#[repr(C)]
pub struct Virtqueue {
    pub vdev: *mut VirtioDevice,
    pub index: u32,
    pub callback: Option<unsafe extern "C" fn(*mut Virtqueue)>,
    /// Transport-specific opaque pointer. The PCI / MMIO transports
    /// stash their per-queue ring state here. NULL on the foundation
    /// slice.
    pub priv_: *mut c_void,
}

unsafe impl Send for Virtqueue {}
unsafe impl Sync for Virtqueue {}

pub fn init() {
    // No state to initialise on this slice. The bus walker that
    // would track live devices lives in the existing
    // `crate::virtio` module today, not here — wiring the linuxkpi
    // shim into that walker is #459b scope.
}

/// `virtio_find_vqs(vdev, nvqs, vqs[], cbs[], names[], desc)` —
/// allocate `nvqs` virtqueues and stash them in `vqs[]`. virtio-
/// input asks for two: "events" (host → guest events) and "status"
/// (guest → host LED/sound updates).
///
/// Foundation-slice behaviour: return -ENODEV (-19). The driver's
/// probe path treats this as a hard error and bails cleanly through
/// `err_init_vq`. This is the correct foundation behaviour — we're
/// not yet wiring the AREST transport in (that's #459b), so claiming
/// success and handing back NULL queues would invite the driver to
/// dereference NULL on the first virtqueue_get_buf call.
#[no_mangle]
pub extern "C" fn virtio_find_vqs(
    _vdev: *mut VirtioDevice,
    _nvqs: u32,
    _vqs: *mut *mut Virtqueue,
    _callbacks: *const Option<unsafe extern "C" fn(*mut Virtqueue)>,
    _names: *const *const c_char,
    _desc: *mut c_void,
) -> c_int {
    -19 // -ENODEV
}

/// `virtqueue_add_inbuf(vq, sg, num, data, gfp)` — submit a buffer
/// for the device to fill. Foundation stub returns -ENODEV.
#[no_mangle]
pub extern "C" fn virtqueue_add_inbuf(
    _vq: *mut Virtqueue,
    _sg: *mut c_void,
    _num: u32,
    _data: *mut c_void,
    _gfp: c_int,
) -> c_int {
    -19
}

/// `virtqueue_add_outbuf(vq, sg, num, data, gfp)` — submit a buffer
/// for the device to read. Foundation stub returns -ENODEV.
#[no_mangle]
pub extern "C" fn virtqueue_add_outbuf(
    _vq: *mut Virtqueue,
    _sg: *mut c_void,
    _num: u32,
    _data: *mut c_void,
    _gfp: c_int,
) -> c_int {
    -19
}

/// `virtqueue_get_buf(vq, len)` — pop a completed buffer. Foundation
/// stub returns NULL (no buffers ever completed).
#[no_mangle]
pub extern "C" fn virtqueue_get_buf(_vq: *mut Virtqueue, _len: *mut u32) -> *mut c_void {
    core::ptr::null_mut()
}

/// `virtqueue_kick(vq)` — notify the device of new buffers. Foundation
/// stub is a no-op (no device to notify).
#[no_mangle]
pub extern "C" fn virtqueue_kick(_vq: *mut Virtqueue) -> bool {
    false
}

/// `virtqueue_get_vring_size(vq)` — query the queue's slot count.
/// Foundation stub returns 0; virtio-input handles the zero case
/// (it skips the fill loop).
#[no_mangle]
pub extern "C" fn virtqueue_get_vring_size(_vq: *mut Virtqueue) -> u32 {
    0
}

/// `virtqueue_detach_unused_buf(vq)` — drain a buffer the driver
/// previously added but never got back. Foundation stub returns NULL.
#[no_mangle]
pub extern "C" fn virtqueue_detach_unused_buf(_vq: *mut Virtqueue) -> *mut c_void {
    core::ptr::null_mut()
}

/// `virtio_cread_bytes(vdev, offset, buf, len)` — read raw bytes
/// from the device's config space. Foundation stub zeroes the
/// destination (safe — virtio-input checks the returned-size byte
/// against zero before using the buffer).
#[no_mangle]
pub extern "C" fn virtio_cread_bytes(
    _vdev: *mut VirtioDevice,
    _offset: u32,
    buf: *mut c_void,
    len: usize,
) {
    if !buf.is_null() && len > 0 {
        // SAFETY: caller hands us a writable buffer of `len` bytes.
        unsafe {
            core::ptr::write_bytes(buf as *mut u8, 0, len);
        }
    }
}

/// `virtio_cwrite_bytes(vdev, offset, buf, len)` — write raw bytes
/// to the device's config space. Foundation stub is a no-op (the
/// host config space is absent).
#[no_mangle]
pub extern "C" fn virtio_cwrite_bytes(
    _vdev: *mut VirtioDevice,
    _offset: u32,
    _buf: *const c_void,
    _len: usize,
) {
}

/// `virtio_cread8(vdev, offset)` — single-byte config read. The
/// `virtio_cread_le(vdev, struct, field, &val)` macro that virtio-
/// input uses expands into one of these per field width. Foundation
/// stub returns 0 — same "device absent" semantics.
#[no_mangle]
pub extern "C" fn virtio_cread8(_vdev: *mut VirtioDevice, _offset: u32) -> u8 {
    0
}

#[no_mangle]
pub extern "C" fn virtio_cread16(_vdev: *mut VirtioDevice, _offset: u32) -> u16 {
    0
}

#[no_mangle]
pub extern "C" fn virtio_cread32(_vdev: *mut VirtioDevice, _offset: u32) -> u32 {
    0
}

#[no_mangle]
pub extern "C" fn virtio_cread64(_vdev: *mut VirtioDevice, _offset: u32) -> u64 {
    0
}

/// `virtio_cwrite8(vdev, offset, val)` — single-byte config write.
/// Foundation stub is a no-op.
#[no_mangle]
pub extern "C" fn virtio_cwrite8(_vdev: *mut VirtioDevice, _offset: u32, _val: u8) {}

#[no_mangle]
pub extern "C" fn virtio_cwrite16(_vdev: *mut VirtioDevice, _offset: u32, _val: u16) {}

#[no_mangle]
pub extern "C" fn virtio_cwrite32(_vdev: *mut VirtioDevice, _offset: u32, _val: u32) {}

#[no_mangle]
pub extern "C" fn virtio_cwrite64(_vdev: *mut VirtioDevice, _offset: u32, _val: u64) {}

/// `virtio_has_feature(vdev, fbit)` — query the device's negotiated
/// feature bits. Foundation stub returns true so virtio-input's
/// `VIRTIO_F_VERSION_1` check at probe entry passes — that lets us
/// see the rest of the linkage flow exercise. Restored to real
/// negotiated semantics in #459b.
#[no_mangle]
pub extern "C" fn virtio_has_feature(_vdev: *mut VirtioDevice, _fbit: u32) -> bool {
    true
}

/// `virtio_device_ready(vdev)` — drive the device into RUNNING
/// status. No-op on the foundation slice (no transport to notify).
#[no_mangle]
pub extern "C" fn virtio_device_ready(_vdev: *mut VirtioDevice) {}

/// `virtio_reset_device(vdev)` — drive the device through RESET.
/// No-op on the foundation slice.
#[no_mangle]
pub extern "C" fn virtio_reset_device(_vdev: *mut VirtioDevice) {}
