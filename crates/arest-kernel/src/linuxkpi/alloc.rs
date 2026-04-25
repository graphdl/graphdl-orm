// crates/arest-kernel/src/linuxkpi/alloc.rs
//
// Linux kmalloc / kfree / kzalloc + the device-managed (`devm_*`)
// family. All routes through Rust's `alloc::alloc::alloc` global
// allocator (which on AREST UEFI is talc, per #443 — see
// Cargo.toml's `talc = "4.4"` dependency note + entry_uefi.rs's
// `#[global_allocator]`).
//
// Linux kmalloc semantics
// -----------------------
// * `kmalloc(size, flags)` returns a pointer to at-least-`size` bytes
//   of kernel-heap memory, naturally aligned for any standard type.
//   `flags` selects the allocation context (GFP_KERNEL = sleepable,
//   GFP_ATOMIC = non-sleeping). On a single-threaded kernel like ours
//   the distinction is moot — every allocation runs on the boot CPU
//   with no scheduler in sight, so we ignore `flags` entirely.
// * `kzalloc` is `kmalloc` followed by zeroing.
// * `kfree(ptr)` accepts NULL (Linux contract — drivers depend on
//   `kfree(NULL)` being a no-op).
//
// Size header for kfree
// ---------------------
// Rust's `dealloc` requires the original `Layout` (size + alignment),
// but Linux's `kfree` only takes a pointer. We bridge this by carving
// a leading `usize` header that records the size, then handing the
// caller a pointer past the header. `kfree` reads the header, rebuilds
// the `Layout`, and routes through `dealloc`. Same idiom rust-osdev's
// `linked_list_allocator` examples use for C interop.
//
// Alignment is hard-coded to 16 bytes — the Linux kernel guarantees
// kmalloc returns memory aligned to `ARCH_KMALLOC_MINALIGN` which on
// x86_64 is `__alignof__(unsigned long long)` = 8, but several drivers
// assume 16 (XSAVE area, AVX intrinsics in some networking paths).
// 16 is the safe over-approximation; talc handles either fine.
//
// devm_* lifetime tracking
// ------------------------
// `devm_kzalloc(dev, size, flags)` ties the allocation's lifetime to
// the supplied `device` — when the device unbinds, every devm_* alloc
// against it auto-frees. We implement this with a `BTreeMap<DeviceId,
// Vec<*mut u8>>` keyed on the device pointer; `devm_kfree` removes
// the entry, `device_unregister` drains the whole vector. On the
// foundation slice no driver actually unbinds, so the drain path is
// dead code today — but the bookkeeping keeps it honest for #459b+.

use alloc::alloc::{alloc, dealloc, Layout};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::ffi::{c_int, c_void};
use core::ptr;
use spin::{Mutex, Once};

/// Hard-coded alignment for every kmalloc — see module docstring.
const KMALLOC_ALIGN: usize = 16;

/// Send-safe wrapper around a heap-allocation pointer. The pointer
/// is opaque from the pool's perspective — only ever paired with the
/// matching `kfree` / `dealloc`. Single-threaded kernel: we never
/// alias the pointer across CPUs, but we still need Send for the
/// `BTreeMap<usize, Vec<DevmPtr>>` to be Sync inside a static Once.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct DevmPtr(*mut u8);

// SAFETY: the wrapped *mut u8 is a heap allocation owned exclusively
// by this pool until devm_kfree / devm_release_all consumes it.
// Single-threaded kernel — no concurrent access ever happens; the
// Send/Sync impls only exist to satisfy the static Once's Sync bound.
unsafe impl Send for DevmPtr {}
unsafe impl Sync for DevmPtr {}

/// Per-device devm_* allocation pool. Key is the raw `device *`
/// pointer cast to `usize` (only used for equality, never deref'd
/// inside this module). Value is the list of header-pointer pairs
/// returned to the caller — devm_kfree removes one, devm_release
/// (called from device_unregister) frees the whole list.
static DEVM_POOL: Once<Mutex<BTreeMap<usize, Vec<DevmPtr>>>> = Once::new();

/// Initialise the devm pool. Idempotent.
pub fn init() {
    DEVM_POOL.call_once(|| Mutex::new(BTreeMap::new()));
}

/// Carve `size + sizeof::<usize>()` bytes from the global allocator
/// and return a pointer past the size-header. NULL on allocation
/// failure (Linux contract — drivers check the return).
fn raw_kmalloc(size: usize) -> *mut u8 {
    if size == 0 {
        // Linux kmalloc(0) returns ZERO_SIZE_PTR (a sentinel that
        // kfree treats as a no-op). Easier path: alloc 1 byte so the
        // pointer is non-null and non-aliasing; kfree's header read
        // still works because we always carve the header.
        return raw_kmalloc(1);
    }
    let layout = match Layout::from_size_align(size + core::mem::size_of::<usize>(), KMALLOC_ALIGN) {
        Ok(l) => l,
        Err(_) => return ptr::null_mut(),
    };
    // SAFETY: layout has non-zero size (size + 8) and a power-of-two
    // alignment (16). `alloc` is documented to either return a valid
    // ptr satisfying the layout or null on OOM.
    let header = unsafe { alloc(layout) };
    if header.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: we just allocated `size + 8` bytes; writing the size at
    // offset 0 stays within bounds. The header is naturally aligned
    // because the whole block is 16-byte aligned.
    unsafe {
        (header as *mut usize).write(size);
        header.add(core::mem::size_of::<usize>())
    }
}

/// Recover the size from the leading header and route the dealloc.
/// NULL is a valid input — Linux drivers depend on this.
fn raw_kfree(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: we never hand out a ptr that wasn't returned from
    // raw_kmalloc, so the 8 bytes preceding `ptr` are the size header
    // we wrote above.
    let header = unsafe { ptr.sub(core::mem::size_of::<usize>()) };
    let size = unsafe { (header as *const usize).read() };
    let layout = Layout::from_size_align(size + core::mem::size_of::<usize>(), KMALLOC_ALIGN)
        .expect("kmalloc layout must be reconstructable");
    // SAFETY: `header` came from `alloc(layout)` with the same layout
    // we're rebuilding here. No aliasing concern under our single-
    // threaded kernel.
    unsafe {
        dealloc(header, layout);
    }
}

/// `kmalloc(size, gfp)` — Linux's bread-and-butter allocator. `gfp`
/// is ignored; see module docstring.
#[no_mangle]
pub extern "C" fn kmalloc(size: usize, _gfp: c_int) -> *mut c_void {
    raw_kmalloc(size) as *mut c_void
}

/// `kzalloc(size, gfp)` — kmalloc + memset(0). Used by the vendored
/// virtio-input.c probe for the per-device `struct virtio_input`.
#[no_mangle]
pub extern "C" fn kzalloc(size: usize, gfp: c_int) -> *mut c_void {
    let p = kmalloc(size, gfp);
    if !p.is_null() && size != 0 {
        // SAFETY: kmalloc returned at least `size` bytes; zeroing is
        // an in-place write that stays within bounds.
        unsafe {
            ptr::write_bytes(p as *mut u8, 0, size);
        }
    }
    p
}

/// `kfree(ptr)` — must accept NULL (Linux contract).
#[no_mangle]
pub extern "C" fn kfree(ptr: *mut c_void) {
    raw_kfree(ptr as *mut u8);
}

/// `devm_kzalloc(dev, size, gfp)` — device-managed zeroed allocation.
/// Tracked in `DEVM_POOL[dev]` so `device_unregister(dev)` auto-frees.
///
/// Returns NULL if `dev` is NULL (Linux drivers check the return).
#[no_mangle]
pub extern "C" fn devm_kzalloc(dev: *mut c_void, size: usize, gfp: c_int) -> *mut c_void {
    if dev.is_null() {
        return ptr::null_mut();
    }
    let p = kzalloc(size, gfp);
    if !p.is_null() {
        if let Some(pool) = DEVM_POOL.get() {
            pool.lock()
                .entry(dev as usize)
                .or_insert_with(Vec::new)
                .push(DevmPtr(p as *mut u8));
        }
    }
    p
}

/// `devm_kfree(dev, ptr)` — early-release a single devm_* allocation.
/// The caller is permitted (but not required) to call this; otherwise
/// the allocation lives until `device_unregister(dev)`.
#[no_mangle]
pub extern "C" fn devm_kfree(dev: *mut c_void, ptr: *mut c_void) {
    if let Some(pool) = DEVM_POOL.get() {
        let mut pool = pool.lock();
        if let Some(list) = pool.get_mut(&(dev as usize)) {
            let target = DevmPtr(ptr as *mut u8);
            list.retain(|&p| p != target);
        }
    }
    kfree(ptr);
}

/// Drain every devm_* allocation against `dev`. Called from
/// `device_unregister`. No-op if the device has none.
pub fn devm_release_all(dev: *mut c_void) {
    if let Some(pool) = DEVM_POOL.get() {
        if let Some(list) = pool.lock().remove(&(dev as usize)) {
            for p in list {
                raw_kfree(p.0);
            }
        }
    }
}
