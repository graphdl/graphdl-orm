// crates/arest-kernel/src/linuxkpi/io.rs
//
// Linux MMIO accessors — `ioremap`, `iounmap`, `read{b,w,l,q}` /
// `write{b,w,l,q}`. Drivers use these to access device registers
// mapped into the kernel's virtual address space.
//
// AREST simplification
// --------------------
// Linux runs with paging enabled and dynamic IOMMU mappings. Device
// MMIO regions need explicit `ioremap(phys, size)` to populate page-
// table entries before access. AREST under UEFI runs with the
// firmware's identity-mapped page tables: every physical address is
// reachable by a virtual address equal to it. So `ioremap` is just
// the identity function — return the input as a `void *`.
//
// `iounmap` is the symmetric no-op: nothing was allocated by
// `ioremap`, nothing to free. Future-proofing: if a driver later
// needs a remap with different cache attributes (write-combining for
// framebuffers, etc), the `ioremap_*` family expands; on the
// foundation slice we stop at the basic identity map.
//
// The `read*` / `write*` family are `core::ptr::read_volatile` /
// `write_volatile` of the matching width. Volatile is mandatory for
// device registers (the compiler must NOT cache or reorder the
// access — registers can have read-side-effects or change between
// successive reads). Linux's macros guarantee this; ours do too.
//
// The vendored `virtio_input.c` doesn't reach `ioremap` directly
// (the virtio-pci / virtio-mmio transport calls it on its behalf),
// but the broader linuxkpi surface needs them for any future driver
// (touchscreen, WiFi MAC) and the C-side header `vendor/linux/
// include/linux/io.h` declares the prototypes that link against
// these.

use core::ffi::c_void;
use core::ptr::{read_volatile, write_volatile};

/// `ioremap(phys, size)` — map device MMIO into kernel virtual
/// address space. On AREST/UEFI the firmware identity-maps the
/// physical address space, so the mapping is the identity function.
/// `size` is recorded only for documentation — not enforced.
///
/// Returns NULL only if `phys` is 0 (Linux drivers treat NULL as
/// "ioremap failed"). Real failures (region overlap, page-table OOM)
/// can't happen in our identity-map model.
#[no_mangle]
pub extern "C" fn ioremap(phys: u64, _size: usize) -> *mut c_void {
    if phys == 0 {
        return core::ptr::null_mut();
    }
    phys as *mut c_void
}

/// `ioremap_wc(phys, size)` — write-combining variant. Same identity
/// map; the cache-attribute distinction is moot when we don't have a
/// per-page MTRR / PAT story (AREST inherits whatever cacheability
/// UEFI set up for the region, which on QEMU is uncached for MMIO
/// BARs anyway). Documented for the future; works today.
#[no_mangle]
pub extern "C" fn ioremap_wc(phys: u64, size: usize) -> *mut c_void {
    ioremap(phys, size)
}

/// `iounmap(addr)` — release a mapping. No-op on the identity map.
#[no_mangle]
pub extern "C" fn iounmap(_addr: *mut c_void) {}

// ---- read* / write* -------------------------------------------------
//
// One pair per width. All are `volatile` to keep the compiler from
// caching or reordering. `extern "C"` so the C side links by symbol
// name; the identical Rust signatures let internal callers use them
// directly without going through FFI.

/// 8-bit MMIO read. SAFETY contract: caller passes a pointer to a
/// device register (or an identity-mapped physical-address-shaped
/// pointer); volatile read is sound on any addressable u8 location.
#[no_mangle]
pub unsafe extern "C" fn readb(addr: *const c_void) -> u8 {
    // SAFETY: caller guarantees `addr` is a readable u8. Volatile
    // read prevents the compiler from caching the value.
    unsafe { read_volatile(addr as *const u8) }
}

/// 16-bit MMIO read. Same SAFETY contract as `readb`.
#[no_mangle]
pub unsafe extern "C" fn readw(addr: *const c_void) -> u16 {
    unsafe { read_volatile(addr as *const u16) }
}

/// 32-bit MMIO read. Same SAFETY contract as `readb`.
#[no_mangle]
pub unsafe extern "C" fn readl(addr: *const c_void) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

/// 64-bit MMIO read. Same SAFETY contract as `readb`. Note: not all
/// PCIe controllers split a 64-bit read into a single transaction;
/// real Linux's `readq` issues a pair of `readl`s on 32-bit hosts.
/// We're 64-bit-only on the foundation slice (x86_64 UEFI), so a
/// single 64-bit access is sound.
#[no_mangle]
pub unsafe extern "C" fn readq(addr: *const c_void) -> u64 {
    unsafe { read_volatile(addr as *const u64) }
}

/// 8-bit MMIO write. SAFETY: caller guarantees `addr` is a writable
/// u8 location.
#[no_mangle]
pub unsafe extern "C" fn writeb(val: u8, addr: *mut c_void) {
    unsafe { write_volatile(addr as *mut u8, val) }
}

/// 16-bit MMIO write. Same contract as `writeb`.
#[no_mangle]
pub unsafe extern "C" fn writew(val: u16, addr: *mut c_void) {
    unsafe { write_volatile(addr as *mut u16, val) }
}

/// 32-bit MMIO write. Same contract as `writeb`.
#[no_mangle]
pub unsafe extern "C" fn writel(val: u32, addr: *mut c_void) {
    unsafe { write_volatile(addr as *mut u32, val) }
}

/// 64-bit MMIO write. Same contract as `writeb`.
#[no_mangle]
pub unsafe extern "C" fn writeq(val: u64, addr: *mut c_void) {
    unsafe { write_volatile(addr as *mut u64, val) }
}

pub fn init() {
    // No state to initialise. Function provided for parity with the
    // other linuxkpi sub-modules so `linuxkpi::init()` can call
    // `io::init()` without a special case.
}
