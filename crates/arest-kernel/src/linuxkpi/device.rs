// crates/arest-kernel/src/linuxkpi/device.rs
//
// Linux `struct device` shim — the universal "I am a hardware/virtual
// node on a bus" handle every Linux driver attaches to. Stores
// driver-private data, parent/child links, and the bus dispatch back-
// pointer.
//
// What we model
// -------------
// Linux's real `struct device` is enormous (~700 bytes, ~50 fields)
// because it tracks runtime PM, sysfs, kobject refcounts, devres,
// driver matching, etc. The vendored `virtio_input.c` reaches into
// only a handful of fields:
//
//   * `dev->parent` — set on `vi->idev->dev.parent = &vdev->dev` so
//     udev / sysfs can walk the topology. We expose it but don't act
//     on it; AREST has no sysfs.
//   * `dev_set_drvdata` / `dev_get_drvdata` — opaque void* slot for
//     the driver's per-device state. `input_set_drvdata` rides on
//     this. Mandatory for `virtio_input.c`'s `vi->vdev->priv = vi`
//     dance.
//   * `device_register` / `device_unregister` — bus-attach lifecycle
//     hook. `input_register_device` calls these internally; on a
//     real Linux box this triggers udev hotplug events. We use it
//     only as the trigger to release `devm_*` allocations.
//
// Implementation
// --------------
// `struct device` is a C-ABI struct (declared in our stub
// `vendor/linux/include/linux/device.h`). The Rust side mirrors the
// layout one-for-one so a `*mut c_void` from C is interchangeable
// with a `&mut Device` here.
//
// `dev_set_drvdata` writes into a single `void *` field embedded in
// the struct. No registry lookup needed for that — it's just a field
// store. The registry below is kept around for `device_register` to
// track the live set (so devm_release can find them later).

use alloc::collections::BTreeSet;
use core::ffi::c_void;
use spin::{Mutex, Once};

/// Wire-compatible mirror of the `struct device` declared in
/// `vendor/linux/include/linux/device.h`. Layout MUST match the C
/// header exactly — extending here requires extending there. Field
/// order matches Linux's own ordering for the fields we model so a
/// future expansion stays drop-in.
///
/// Padding is left to the compiler; both sides are `#[repr(C)]`.
#[repr(C)]
pub struct Device {
    /// Parent device on the bus topology (used for sysfs path
    /// composition in real Linux). We store but don't dispatch on it.
    pub parent: *mut Device,
    /// Driver-private data — written by `dev_set_drvdata`, read by
    /// `dev_get_drvdata`. Opaque void *.
    pub driver_data: *mut c_void,
    /// Bus dispatch table — set by the bus type that owns this device
    /// (e.g. virtio_bus). Opaque to non-bus code.
    pub bus: *mut c_void,
    /// Device's release callback. Called from `device_unregister` so
    /// the driver can free embedded state. NULL means "no release
    /// needed" (devm_* still runs).
    pub release: Option<unsafe extern "C" fn(*mut Device)>,
}

// SAFETY: Device contains raw pointers but our single-threaded
// kernel never aliases a Device across CPUs. Send is required so
// the live-set BTreeSet can hold device addresses cast to usize.
unsafe impl Send for Device {}
unsafe impl Sync for Device {}

/// Set of registered device addresses (cast to usize for ordering).
/// Populated by `device_register`, drained by `device_unregister`.
/// Pure bookkeeping — the registry exists so `device_unregister` has
/// a place to drive `devm_release_all` from.
static REGISTRY: Once<Mutex<BTreeSet<usize>>> = Once::new();

pub fn init() {
    REGISTRY.call_once(|| Mutex::new(BTreeSet::new()));
}

/// `device_register(dev)` — register a Linux device on its bus. On
/// real Linux this triggers driver matching, sysfs node creation,
/// uevent dispatch. Here we just record the device pointer in the
/// registry so `device_unregister` can drive devm release.
///
/// Returns 0 on success — Linux's `int device_register(...)`
/// convention. NULL `dev` is a hard error (Linux returns -EINVAL);
/// we mirror that with -22 (the EINVAL value on every modern Linux).
#[no_mangle]
pub extern "C" fn device_register(dev: *mut Device) -> core::ffi::c_int {
    if dev.is_null() {
        return -22; // -EINVAL
    }
    if let Some(reg) = REGISTRY.get() {
        reg.lock().insert(dev as usize);
    }
    0
}

/// `device_unregister(dev)` — un-register from the bus. Drives
/// devm release + invokes the release callback if set.
#[no_mangle]
pub extern "C" fn device_unregister(dev: *mut Device) {
    if dev.is_null() {
        return;
    }
    if let Some(reg) = REGISTRY.get() {
        reg.lock().remove(&(dev as usize));
    }
    super::alloc::devm_release_all(dev as *mut c_void);
    // SAFETY: we hand the device pointer back to the driver's release
    // callback. Driver-side responsibility to ensure the embedded
    // state is in a state safe to free.
    unsafe {
        if let Some(release) = (*dev).release {
            release(dev);
        }
    }
}

/// `dev_set_drvdata(dev, data)` — store the driver's private state
/// pointer in the device. Used by `input_set_drvdata` to attach the
/// per-device input state.
#[no_mangle]
pub extern "C" fn dev_set_drvdata(dev: *mut Device, data: *mut c_void) {
    if dev.is_null() {
        return;
    }
    // SAFETY: caller hands us a valid Device pointer; we write a
    // single field. No re-entrancy concern — kernel is single-thread.
    unsafe {
        (*dev).driver_data = data;
    }
}

/// `dev_get_drvdata(dev)` — load the driver's private state pointer.
/// NULL if `dev` is NULL or no `dev_set_drvdata` has been done.
#[no_mangle]
pub extern "C" fn dev_get_drvdata(dev: *mut Device) -> *mut c_void {
    if dev.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: caller guarantees `dev` is a valid Device.
    unsafe { (*dev).driver_data }
}

/// `put_device(dev)` — Linux refcount drop. We don't refcount on the
/// foundation slice (single-threaded, no async lifetime concerns), so
/// this is a no-op. Kept as a named export so the C side can call it.
#[no_mangle]
pub extern "C" fn put_device(_dev: *mut Device) {}

/// `get_device(dev)` — Linux refcount bump. No-op for the same reason
/// `put_device` is. Returns the device pointer unchanged.
#[no_mangle]
pub extern "C" fn get_device(dev: *mut Device) -> *mut Device {
    dev
}
