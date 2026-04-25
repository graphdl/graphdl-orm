// crates/arest-kernel/src/linuxkpi/driver.rs
//
// Linux driver registration + bus-type plumbing. The vendored
// `virtio_input.c` declares a `struct virtio_driver virtio_input_driver
// = { .id_table = ..., .probe = ..., .remove = ... }` and registers
// it via the `module_virtio_driver(...)` macro, which expands into a
// constructor calling `register_virtio_driver(&virtio_input_driver)`.
//
// What we model
// -------------
// Linux's bus model has two layers:
//   * `struct bus_type` — the bus class (pci_bus_type, virtio_bus,
//     usb_bus_type, etc). Owns the match callback that pairs a
//     `device` with a candidate `driver`.
//   * `struct device_driver` (and subclass `struct virtio_driver`) —
//     a single driver registered on a bus. Holds an `id_table` of
//     (vendor, device) pairs the driver claims, plus probe / remove
//     callbacks invoked when the bus matches a device against the
//     driver.
//
// On the foundation slice we model just enough for `register_virtio_
// driver` to succeed at link time. Actual driver match + probe
// dispatch is #459b — that needs a virtio bus walker that hands each
// candidate device to every registered driver's id_table. For now,
// `register_virtio_driver` records the driver in a static table so a
// future bus walker can find it.

use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_void};
use spin::{Mutex, Once};

/// `struct device_driver` — base driver descriptor. Layout matches
/// the C struct in `vendor/linux/include/linux/device/driver.h`.
/// Shared by every bus's driver subclass via leading-field embedding.
#[repr(C)]
pub struct DeviceDriver {
    pub name: *const c_char,
    pub owner: *mut c_void, // struct module * — opaque on AREST
}

/// `struct virtio_driver` — virtio-bus subclass of `device_driver`.
/// Matches the C layout in `vendor/linux/include/linux/virtio.h`. The
/// fields we rely on: `driver` (base class — Linux composition idiom),
/// `id_table` + `feature_table` (driver capability advertising),
/// `probe` / `remove` (per-device lifecycle hooks).
#[repr(C)]
pub struct VirtioDriver {
    pub driver: DeviceDriver,
    pub id_table: *const VirtioDeviceId,
    pub feature_table: *const u32,
    pub feature_table_size: u32,
    pub feature_table_legacy: *const u32,
    pub feature_table_size_legacy: u32,
    /// `int (*probe)(struct virtio_device *)` — bus calls this when
    /// it matches an unbound device against this driver.
    pub probe: Option<unsafe extern "C" fn(*mut super::virtio::VirtioDevice) -> c_int>,
    /// `void (*scan)(struct virtio_device *)` — optional post-probe
    /// hook (used by virtio-blk for partition scan). NULL on
    /// virtio-input.
    pub scan: Option<unsafe extern "C" fn(*mut super::virtio::VirtioDevice)>,
    pub remove: Option<unsafe extern "C" fn(*mut super::virtio::VirtioDevice)>,
    pub config_changed: Option<unsafe extern "C" fn(*mut super::virtio::VirtioDevice)>,
    pub freeze: Option<unsafe extern "C" fn(*mut super::virtio::VirtioDevice) -> c_int>,
    pub restore: Option<unsafe extern "C" fn(*mut super::virtio::VirtioDevice) -> c_int>,
}

// SAFETY: VirtioDriver holds only function pointers + read-only
// id-table pointers. Every callback runs in the kernel's single-CPU
// boot context, never aliased.
unsafe impl Send for VirtioDriver {}
unsafe impl Sync for VirtioDriver {}

/// `struct virtio_device_id` — vendor/device-id pair the driver
/// claims. Populated as a static array in the driver source. Layout
/// matches C in `vendor/linux/include/uapi/linux/virtio_ids.h`.
#[repr(C)]
pub struct VirtioDeviceId {
    pub device: u32,
    pub vendor: u32,
}

/// Send-safe wrapper around a `*mut VirtioDriver`. The pointer
/// references a statically allocated `struct virtio_driver` on the
/// C side (e.g. the `static struct virtio_driver virtio_input_driver`
/// definition in virtio_input.c lives in .data for the lifetime of
/// the kernel). Single-threaded kernel — no concurrent access; the
/// Send/Sync impls only exist to satisfy the static Once's Sync bound.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DriverRef(pub *mut VirtioDriver);

unsafe impl Send for DriverRef {}
unsafe impl Sync for DriverRef {}

/// Registry of every `register_virtio_driver`-ed driver. A future
/// bus walker (virtio-pci / virtio-mmio side) drains this to attempt
/// matches. On the foundation slice it grows by exactly one entry
/// (the vendored virtio_input_driver) and never shrinks.
static VIRTIO_DRIVERS: Once<Mutex<Vec<DriverRef>>> = Once::new();

pub fn init() {
    VIRTIO_DRIVERS.call_once(|| Mutex::new(Vec::new()));
}

/// `register_virtio_driver(drv)` — record the driver so a future bus
/// walker can match candidate devices against its `id_table`. Returns
/// 0 on success — Linux convention.
///
/// On the foundation slice this just appends to the static table. The
/// vendored `virtio_input.c`'s `module_virtio_driver` macro generates
/// a `__init virtio_input_driver_init` thunk that calls this; landing
/// here proves the C side links cleanly.
#[no_mangle]
pub extern "C" fn register_virtio_driver(drv: *mut VirtioDriver) -> c_int {
    if drv.is_null() {
        return -22; // -EINVAL
    }
    if let Some(reg) = VIRTIO_DRIVERS.get() {
        reg.lock().push(DriverRef(drv));
    }
    0
}

/// `unregister_virtio_driver(drv)` — module-exit pair to
/// `register_virtio_driver`. Removes the driver from the registry.
/// Not exercised on the foundation slice (the kernel never unloads
/// its built-in modules) but exported for ABI completeness.
#[no_mangle]
pub extern "C" fn unregister_virtio_driver(drv: *mut VirtioDriver) {
    if let Some(reg) = VIRTIO_DRIVERS.get() {
        let target = DriverRef(drv);
        reg.lock().retain(|&d| d != target);
    }
}

/// Snapshot the registered drivers. Used by the future virtio-bus
/// walker (#459b) to iterate match candidates. Returns the wrapped
/// pointers; caller unwraps `DriverRef::0` to get the raw pointer.
pub fn registered_virtio_drivers() -> Vec<DriverRef> {
    VIRTIO_DRIVERS
        .get()
        .map(|r| r.lock().clone())
        .unwrap_or_default()
}
