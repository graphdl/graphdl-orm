// crates/arest-kernel/src/virtio_mmio.rs
//
// virtio-mmio transport for the aarch64 + armv7 UEFI paths (#368/#369
// aarch64, #388 armv7 widening). Sibling of `virtio.rs` — the latter
// drives virtio-over-PCI on x86_64 (legacy PIO ConfigurationAccess +
// PciTransport). QEMU's `virt` machine — both the aarch64 and armv7
// variants — exposes virtio devices as MMIO slots at 0x0a00_0000
// instead, each slot 0x200 bytes wide, up to 32 slots. This module is
// the aarch64 / armv7 analogue of `pci.rs` + `virtio.rs`'s bring-up
// halves.
//
// ── Cross-arch shape ─────────────────────────────────────────────────
//
// The transport body is arch-neutral: volatile u32 reads probe the
// MMIO slot headers, and `virtio-drivers`' `MmioTransport` carries
// the rest of the device-side handshake. Two arch-specific seams
// flow through `cfg`-resolved re-exports rather than per-arm code:
//
//   1. `virtio_drivers::PhysAddr` is a type alias for `usize`, so it
//      naturally narrows to 32 bits on armv7 and stays 64-bit on
//      aarch64 — `paddr_u64 as PhysAddr` is the only cast that
//      truncates on the 32-bit arm, and only in the lower bits where
//      the DMA pool's `(u64, u64)` return value already fits by the
//      32-bit-PA invariant `arch::armv7::memory::init` enforces.
//   2. `arch::memory::with_dma_pool` resolves to either
//      `arch::aarch64::memory::with_dma_pool` or
//      `arch::armv7::memory::with_dma_pool` via the per-arm `pub use`
//      in `arch/mod.rs`. Both have signature
//      `with_dma_pool<R>(f: impl FnOnce(&mut DmaPool) -> R) -> Option<R>`,
//      and `DmaPool::alloc` returns `Option<(u64, u64)>` from the
//      shared `dma` module — so the HAL impl below compiles
//      identically on both arms with no per-arch arms.
//
// ── MMIO slot layout (QEMU virt) ─────────────────────────────────────
//
// Every slot starts with a `VirtIOHeader` (virtio-drivers crate type).
// The first u32 is the magic value 0x74726976 ("virt" LE). If a slot
// has no device wired, reads return 0. device_id = 0 also indicates
// an absent slot.
//
//   slot_base(n) = 0x0a00_0000 + n * 0x200         (0 ≤ n < 32)
//   slot_size     = 0x200
//
// ── Scope ────────────────────────────────────────────────────────────
//
// * `AarchMmioHal` — `virtio_drivers::Hal` impl using the DMA pool
//   carved in `arch::{aarch64,armv7}::memory`. Identity phys↔virt
//   translation (UEFI firmware identity-maps RAM + MMIO under AAVMF
//   on aarch64 and ArmVirtPkg on armv7). The "Aarch" prefix is
//   historical — the type is shared between the aarch64 and armv7
//   arms now, but renaming would churn callers in the aarch64 entry
//   harness and the armv7 entry harness (#346d) hasn't landed yet.
// * `scan_mmio_slots` — iterate 32 QEMU-virt MMIO slots and return
//   every slot whose header magic matches.
// * `find_virtio_net` / `find_virtio_blk` — filter by virtio
//   `device_id` (1 = Network, 2 = Block per the virtio spec).
// * `try_init_virtio_net` / `try_init_virtio_blk` — construct the
//   MmioTransport + corresponding VirtIO device driver.
// * `init_offset` — HAL phys_offset seed (kept for symmetry with
//   `virtio::init_offset`; always called with 0 on aarch64 + armv7
//   UEFI since the firmware identity-maps in both cases).
//
// The module lives as a separate file rather than an
// `#[cfg(target_arch = "aarch64")]` block at the bottom of `virtio.rs`
// because the existing `virtio.rs` x86_64 body pulls in
// `x86_64::structures::paging::mapper::Translate` and the virtio-
// drivers `PciTransport` at module scope; both of those are gated on
// `target_arch = "x86_64"` inside their source crates, so co-located
// aarch64 / armv7 code inside the same file would fight the module-
// level imports. A parallel module keeps the two arch families
// cleanly separable -- same way `arch::aarch64` and `arch::armv7`
// are siblings of `arch::uefi` rather than conditional blocks in a
// shared file.

#![cfg(all(target_os = "uefi", any(target_arch = "aarch64", target_arch = "arm")))]
// On armv7 the entry harness that consumes this module
// (`entry_uefi_armv7.rs`) hasn't landed yet — that's #346d / #389 —
// so every public item below is currently unreferenced on the armv7
// build. Silence the resulting `dead_code` cascade until those land.
// On aarch64 the consumer (`entry_uefi_aarch64.rs`) is already wired
// so the gate has no effect there.
#![cfg_attr(target_arch = "arm", allow(dead_code))]

extern crate alloc;

use core::ptr::NonNull;

use virtio_drivers::{BufferDirection, Hal, PhysAddr};

use crate::dma;

/// Bootloader-supplied offset between physical and virtual addresses.
/// Always 0 on aarch64 UEFI (firmware identity-maps), but the
/// `init_offset` / `phys_offset` shape mirrors `virtio::init_offset`
/// so the two arms' HAL plumbing stays symmetric.
static PHYS_OFFSET: spin::Mutex<Option<u64>> = spin::Mutex::new(None);

/// Record the physical-memory offset. Must be called once from
/// `entry_uefi_aarch64::efi_main` before any virtio driver construction.
pub fn init_offset(offset: u64) {
    *PHYS_OFFSET.lock() = Some(offset);
}

fn phys_offset() -> u64 {
    PHYS_OFFSET.lock().expect("virtio_mmio::init_offset() not called")
}

/// `Hal` impl for virtio-drivers on aarch64 UEFI.
///
/// All four methods lean on the firmware's identity mapping:
///   * `dma_alloc` bump-allocates from `arch::aarch64::memory`'s DMA
///     pool (carved in #367) — contiguous, pre-reserved out of the
///     general-purpose frame allocator.
///   * `share`/`unshare` reduce to identity translation because we
///     don't run an IOMMU and phys == virt under AAVMF.
///   * `mmio_phys_to_virt` likewise: the firmware identity-maps the
///     MMIO region covering 0x0a00_0000 + virtio-mmio slots.
pub struct AarchMmioHal;

unsafe impl Hal for AarchMmioHal {
    fn dma_alloc(
        pages: usize,
        _direction: BufferDirection,
    ) -> (PhysAddr, NonNull<u8>) {
        let (paddr_u64, vaddr_u64) = crate::arch::memory::with_dma_pool(|pool| {
            pool.alloc(pages)
                .expect("dma_alloc: DMA pool exhausted (bump DMA_POOL_PAGES)")
        })
        .expect("dma_alloc: DMA pool not carved (no usable region big enough)");

        let paddr = paddr_u64 as PhysAddr;
        // SAFETY: DMA pool was carved out of firmware-identity-mapped RAM;
        // kernel-virtual pointer equals paddr under AAVMF's identity map.
        let vaddr = unsafe { NonNull::new_unchecked(vaddr_u64 as *mut u8) };

        // Per the Hal trait contract, returned pages must be zeroed —
        // virtio-drivers relies on used-ring `idx` reading as zero at
        // queue setup. Same zeroing the x86_64 arm does in `virtio.rs`.
        //
        // SAFETY: `vaddr` was just handed back by the pool, so no
        // other pointer aliases the region; the range is
        // `pages * PAGE_SIZE` bytes of kernel-owned writable memory.
        unsafe {
            core::ptr::write_bytes(vaddr.as_ptr(), 0, pages * dma::PAGE_SIZE);
        }

        (paddr, vaddr)
    }

    unsafe fn dma_dealloc(
        _paddr: PhysAddr,
        _vaddr: NonNull<u8>,
        _pages: usize,
    ) -> i32 {
        // Bump-only pool: freed pages are leaked for the lifetime of
        // the kernel. Matches the x86_64 `virtio::KernelHal` behavior.
        0
    }

    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, _size: usize) -> NonNull<u8> {
        // Identity mapping — firmware maps MMIO at its physical address.
        let vaddr = (paddr as u64 + phys_offset()) as *mut u8;
        // SAFETY: firmware identity-maps the full MMIO window under
        // AAVMF for the lifetime of the kernel.
        unsafe { NonNull::new_unchecked(vaddr) }
    }

    unsafe fn share(
        buffer: NonNull<[u8]>,
        _direction: BufferDirection,
    ) -> PhysAddr {
        // Identity translation: phys == virt under UEFI identity mapping.
        // The x86_64 arm walks the OffsetPageTable for `share` because
        // its buffers can live outside the offset-mapped physmem window
        // (kernel BSS heap); on aarch64 UEFI everything the kernel
        // touches is identity-mapped, so we return the virtual address
        // directly, minus the `phys_offset` (= 0).
        let vaddr = buffer.as_ptr() as *mut u8 as usize;
        (vaddr as u64).saturating_sub(phys_offset()) as PhysAddr
    }

    unsafe fn unshare(
        _paddr: PhysAddr,
        _buffer: NonNull<[u8]>,
        _direction: BufferDirection,
    ) {
        // No-op: identity-offset mapping means "share" was already
        // just an address translation; nothing to undo.
    }
}

// ── MMIO slot discovery ───────────────────────────────────────────────

/// QEMU aarch64 `virt` machine virtio-mmio slot base (hw/arm/virt.c
/// `VIRT_MMIO`). Firmware identity-maps this region, and every slot
/// is 0x200 bytes wide. Slots are contiguous in address order, one
/// device per slot, 32 slots total (QEMU's default).
pub const VIRTIO_MMIO_BASE: usize = 0x0a00_0000;
/// Size of a single virtio-mmio slot (VirtIOHeader + config space).
pub const VIRTIO_MMIO_SLOT_SIZE: usize = 0x200;
/// Number of MMIO slots QEMU-virt exposes by default.
pub const VIRTIO_MMIO_SLOT_COUNT: usize = 32;

/// Magic value at the start of every live virtio-mmio slot. ASCII
/// "virt" little-endian (0x76 0x69 0x72 0x74). Reads from an empty
/// slot return 0x0, letting us filter live slots without probing
/// further.
pub const VIRTIO_MMIO_MAGIC: u32 = 0x7472_6976;

/// One discovered MMIO slot. Mirror of `pci::PciDevice` for the
/// virtio-mmio world. `base_paddr` is the slot's MMIO base; `index`
/// is the slot index (0..32) useful for banner lines and debugging.
#[derive(Debug, Clone, Copy)]
pub struct MmioVirtioDevice {
    /// Slot index within QEMU's 32-slot virtio-mmio window.
    pub index: usize,
    /// Physical base address of the slot. MMIO-mapped by firmware.
    pub base_paddr: usize,
    /// virtio device ID from the slot header (1 = Network, 2 = Block,
    /// etc. — see virtio spec §4.2.2.1).
    pub device_id: u32,
    /// Vendor ID from the slot header. QEMU emits 0x554d4551 ("QEMU"
    /// little-endian) for all mmio-virtio devices.
    pub vendor_id: u32,
    /// Transport version. 2 = modern virtio (spec 1.0+), 1 = legacy
    /// (spec 0.9.5). QEMU's aarch64 virt machine reports version 2.
    pub version: u32,
}

/// Read a u32 at `addr` via a volatile load. Used to probe MMIO slot
/// headers without going through the `safe_mmio` wrappers inside the
/// virtio-drivers crate (those require a `UniqueMmioPointer` we
/// haven't constructed yet).
///
/// # Safety
/// `addr` must point at firmware-identity-mapped MMIO that reads are
/// safe to issue against. For virtio-mmio slots on QEMU virt this is
/// always true: absent slots return 0, present slots return the
/// header bytes — neither faults.
#[inline]
unsafe fn mmio_read_u32(addr: usize) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

/// Scan the 32 virtio-mmio slots QEMU's aarch64 virt machine exposes
/// and return every slot with a live magic value. Slots whose header
/// reads 0 are skipped — these are the "empty" slots QEMU leaves in
/// place when fewer than 32 virtio devices are configured.
///
/// The scan issues four u32 reads per slot (magic, version,
/// device_id, vendor_id). 32 slots * 4 reads = 128 reads total, all
/// MMIO, all safe — QEMU's emulated UART model and MMIO registers
/// never fault on read.
pub fn scan_mmio_slots() -> alloc::vec::Vec<MmioVirtioDevice> {
    let mut devices = alloc::vec::Vec::new();
    for i in 0..VIRTIO_MMIO_SLOT_COUNT {
        let base = VIRTIO_MMIO_BASE + i * VIRTIO_MMIO_SLOT_SIZE;
        // SAFETY: MMIO reads on identity-mapped QEMU-virt addresses
        // never fault — absent slots return 0, present slots return
        // the header bytes.
        let magic = unsafe { mmio_read_u32(base) };
        if magic != VIRTIO_MMIO_MAGIC {
            continue;
        }
        // SAFETY: same rationale; header fields at +4, +8, +12.
        let version = unsafe { mmio_read_u32(base + 4) };
        let device_id = unsafe { mmio_read_u32(base + 8) };
        let vendor_id = unsafe { mmio_read_u32(base + 12) };
        if device_id == 0 {
            // Valid magic but device_id 0 — QEMU's "magic present,
            // no device wired" placeholder slot. Skip.
            continue;
        }
        devices.push(MmioVirtioDevice {
            index: i,
            base_paddr: base,
            device_id,
            vendor_id,
            version,
        });
    }
    devices
}

/// Find the first virtio-net slot (device_id = 1 per the spec).
/// Returns `None` when the machine wasn't launched with
/// `-device virtio-net-device,...` (note: `-device` is MMIO-backed
/// on QEMU aarch64 virt — `-device virtio-net-pci` would need a PCIe
/// root, which QEMU's virt machine has but our scanner doesn't walk).
pub fn find_virtio_net() -> Option<MmioVirtioDevice> {
    scan_mmio_slots().into_iter().find(|d| d.device_id == 1)
}

/// Find the first virtio-blk slot (device_id = 2 per the spec).
/// Returns `None` when no `-device virtio-blk-device,...` is wired.
pub fn find_virtio_blk() -> Option<MmioVirtioDevice> {
    scan_mmio_slots().into_iter().find(|d| d.device_id == 2)
}

// ── Driver construction ───────────────────────────────────────────────
//
// With an `MmioVirtioDevice` in hand, `try_init_virtio_*` walks:
//   1. Wrap the slot's base address in a `NonNull<VirtIOHeader>`.
//   2. Construct `MmioTransport::new(header, slot_size)` — this
//      validates magic + version + device_id.
//   3. Hand the transport to the device-class driver (VirtIONet /
//      VirtIOBlk). Both take a `Transport` generic and use our
//      `AarchMmioHal` via the type parameter.
//
// The two `try_init_*` shapes mirror `virtio::try_init_virtio_net` /
// `try_init_virtio_blk` so banner-line construction in
// `entry_uefi_aarch64::efi_main` looks identical on both arms.

use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::device::blk::VirtIOBlk;
use virtio_drivers::device::net::VirtIONet;

/// Queue size for the virtio-net rx/tx virtqueues. Matches the
/// x86_64 arm's `virtio::NET_QUEUE_SIZE` so rx/tx buffer pool
/// footprints are identical across arms.
pub const NET_QUEUE_SIZE: usize = 16;
/// Per-buffer length for the virtio-net rx pool. Standard Ethernet
/// MTU 1500 plus headers rounded to 2048.
pub const NET_BUF_LEN: usize = 2048;

/// Fully-typed VirtIONet driver handle for the MMIO transport.
/// The `'static` lifetime mirrors the transport's ownership model —
/// the slot base address is firmware-identity-mapped for the
/// lifetime of the kernel so the `'a` on `MmioTransport` is
/// effectively 'static here.
pub type VirtIONetDevice = VirtIONet<AarchMmioHal, MmioTransport<'static>, NET_QUEUE_SIZE>;

/// Fully-typed VirtIOBlk driver handle for the MMIO transport.
pub type VirtIOBlkDevice = VirtIOBlk<AarchMmioHal, MmioTransport<'static>>;

/// Build an MmioTransport for `slot`. Stays `unsafe`-in-the-name
/// because passing a bogus `MmioVirtioDevice` — one the scanner
/// didn't produce — could construct a transport pointing at
/// arbitrary memory. Callers should only pass values returned by
/// `scan_mmio_slots` / `find_virtio_*`.
///
/// # Safety
/// `slot.base_paddr` must be the base of a live virtio-mmio slot
/// returned by `scan_mmio_slots`. The slot's `VIRTIO_MMIO_SLOT_SIZE`
/// bytes must remain valid MMIO for `'static`, which is true under
/// AAVMF identity mapping.
unsafe fn build_transport(slot: MmioVirtioDevice) -> Option<MmioTransport<'static>> {
    let header_ptr = slot.base_paddr as *mut VirtIOHeader;
    // SAFETY: caller contract — see outer doc.
    let header_nn = unsafe { NonNull::new_unchecked(header_ptr) };
    // SAFETY: slot base is MMIO-mapped for the full slot size.
    match unsafe { MmioTransport::new(header_nn, VIRTIO_MMIO_SLOT_SIZE) } {
        Ok(t) => Some(t),
        Err(e) => {
            crate::println!(
                "  virtio-mmio: MmioTransport::new failed on slot {}: {:?}",
                slot.index, e,
            );
            None
        }
    }
}

/// Try to bring up the virtio-net driver against the first matching
/// MMIO slot. Returns `Some(driver)` on success, `None` when no
/// device is present or construction failed.
pub fn try_init_virtio_net() -> Option<VirtIONetDevice> {
    let slot = find_virtio_net()?;
    // SAFETY: slot returned by `find_virtio_net` — valid by construction.
    let transport = unsafe { build_transport(slot)? };
    match VirtIONet::<AarchMmioHal, _, NET_QUEUE_SIZE>::new(transport, NET_BUF_LEN) {
        Ok(d) => Some(d),
        Err(e) => {
            crate::println!("  virtio-mmio: VirtIONet::new failed: {:?}", e);
            None
        }
    }
}

/// Try to bring up the virtio-blk driver against the first matching
/// MMIO slot. Returns `Some(driver)` on success, `None` when no
/// device is present or construction failed.
pub fn try_init_virtio_blk() -> Option<VirtIOBlkDevice> {
    let slot = find_virtio_blk()?;
    // SAFETY: slot returned by `find_virtio_blk` — valid by construction.
    let transport = unsafe { build_transport(slot)? };
    match VirtIOBlk::<AarchMmioHal, _>::new(transport) {
        Ok(d) => Some(d),
        Err(e) => {
            crate::println!("  virtio-mmio: VirtIOBlk::new failed: {:?}", e);
            None
        }
    }
}
