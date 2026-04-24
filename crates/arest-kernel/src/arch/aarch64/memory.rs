// crates/arest-kernel/src/arch/aarch64/memory.rs
//
// Page-table access and physical frame allocation for the aarch64 UEFI
// boot path (#366, aarch64 analogue of `arch::uefi::memory`). Consumes
// the UEFI-provided `MemoryMapOwned` that `boot::exit_boot_services`
// returns, stands up a boot-time frame allocator behind the same
// accessor surface the x86_64-UEFI arm publishes.
//
// Divergence from the x86_64 UEFI arm:
//
//   * No `OffsetPageTable` construction. x86_64 needs the rust-osdev
//     `x86_64::structures::paging::OffsetPageTable` because virtio's
//     HAL::share calls `pt.translate_addr(va)` to round-trip kernel
//     VAs back to PAs through a page-walker. On aarch64 UEFI the
//     firmware identity-maps physical RAM under AAVMF (QEMU virt
//     layout), so phys == virt across every address the kernel cares
//     about; the aarch64 virtio-mmio path translates by casting
//     directly rather than walking a page table.
//
//   * TTBR0/TTBR1 (aarch64's CR3 analogue) is not inspected. UEFI
//     firmware leaves TTBR1_EL1 pointing at a set of page tables
//     covering RAM + device MMIO; the kernel rides those tables for
//     the rest of boot and never installs its own. A later commit
//     that stands up AREST-managed page tables would read TTBR_EL1
//     via a `cortex-a` crate helper or a one-line inline-asm; not
//     needed for the virtio bring-up this commit targets.
//
// Aarch64 UEFI post-ExitBootServices state (QEMU virt + AAVMF):
//   * CPU is at EL1 with paging + caches on.
//   * Firmware's page tables identity-map every RAM region plus MMIO
//     the firmware touched (PL011 @ 0x0900_0000, virtio-mmio slots @
//     0x0a00_0000 + 0x200*n, GIC, etc.). phys == virt.
//   * `MemoryMapOwned` is the firmware-returned snapshot of the
//     memory map at the moment of `boot::exit_boot_services`. Each
//     `MemoryDescriptor` carries `ty`, `phys_start`, `page_count`.
//     Frames marked `CONVENTIONAL` are free for OS use.
//
// This module exposes the same public surface as the x86_64-UEFI arm
// for the subset aarch64 actually uses:
//   * `init(memory_map)` — called once from `entry_uefi_aarch64::efi_main`
//     right after `boot::exit_boot_services`; parks the frame allocator
//     + DMA pool in spin-locked statics.
//   * `with_frame_allocator(f)` — accessor helper.
//   * `with_dma_pool(f)` — accessor for the carved DMA region (#367).
//   * `usable_frame_count()` — convenience for the boot banner.
//
// The `dma` / `DmaPool` surface parallels the x86_64-UEFI arm exactly
// (#367) — carve a contiguous 2 MiB chunk out of the firmware memory
// map before building the frame allocator, reserve that range so the
// two allocators never collide. That lets a future aarch64 virtio
// bring-up reach `memory::with_dma_pool` via the same code the x86_64
// HAL uses.

use spin::Mutex;
use uefi::mem::memory_map::{MemoryMap, MemoryMapOwned, MemoryType};

use crate::dma::{self, DmaPool, RegionKind};

// ---------------------------------------------------------------------------
// Global singletons
// ---------------------------------------------------------------------------

/// The boot-time frame allocator. Valid after `init()`.
static FRAME_ALLOCATOR: Mutex<Option<UefiFrameAllocator>> = Mutex::new(None);

/// Dedicated contiguous DMA pool for virtio-drivers. Parallels the
/// x86_64-UEFI arm's `DMA_POOL` — carved out of the firmware memory
/// map before `FRAME_ALLOCATOR` is handed the same map, so the two
/// allocators never collide. `None` only if no CONVENTIONAL region
/// fits the configured pool size (unlikely on any reasonable QEMU
/// guest; same graceful-degrade path the x86_64 arm has).
static DMA_POOL: Mutex<Option<DmaPool>> = Mutex::new(None);

/// Size of the DMA pool in 4 KiB pages (= 2 MiB). Matches the x86_64
/// arm's `DMA_POOL_PAGES` so a future virtio-drivers-on-aarch64 bring-
/// up with `NET_QUEUE_SIZE = 16` (two virtqueues + rx/tx buffer pools)
/// fits identically.
const DMA_POOL_PAGES: usize = 512;

// ---------------------------------------------------------------------------
// Public init
// ---------------------------------------------------------------------------

/// Initialise the memory subsystem from the firmware-provided memory
/// map returned by `boot::exit_boot_services`.
///
/// Must be called exactly once, immediately after `exit_boot_services`
/// (while the firmware's identity-mapped page tables are still the
/// live TTBR1_EL1). Returns the physical-memory offset (always 0 on
/// UEFI — the firmware identity-maps under QEMU virt + AAVMF so
/// phys == virt for every address the kernel touches).
///
/// # Safety
/// Caller must have just returned from `boot::exit_boot_services` so
/// (a) the page tables in TTBR1_EL1 are stable for the kernel's
/// lifetime, and (b) no other CPU is racing us on the page tables or
/// the memory map.
pub fn init(memory_map: MemoryMapOwned) -> u64 {
    // AAVMF's post-EBS page tables identity-map physical RAM, so the
    // "offset mapping" is zero — same convention as x86_64 UEFI.
    let phys_offset: u64 = 0;

    // Carve a dedicated contiguous DMA pool out of the firmware's
    // memory map BEFORE the general-purpose frame allocator sees it,
    // then tell the allocator about the reserved range so frames
    // inside it never leave both pools. Mirrors the x86_64 UEFI arm
    // (arch::uefi::memory::init) so a future virtio-drivers-on-aarch64
    // HAL can reach `memory::with_dma_pool` identically on both arms.
    let dma_capacity_bytes = (DMA_POOL_PAGES * dma::PAGE_SIZE) as u64;
    let (reserved, pool) =
        match carve_from_memory_map(&memory_map, dma_capacity_bytes) {
            Some((start, end)) => {
                let p = DmaPool::new(start, DMA_POOL_PAGES, phys_offset);
                (Some((start, end)), Some(p))
            }
            None => (None, None),
        };

    let frame_alloc = UefiFrameAllocator::new(memory_map, reserved);

    *DMA_POOL.lock() = pool;
    *FRAME_ALLOCATOR.lock() = Some(frame_alloc);

    0
}

/// Scan the UEFI memory map and hand the first-N CONVENTIONAL regions
/// to `dma::carve_dma_region` in the same `(u64, u64, RegionKind)`
/// shape the x86_64 arm uses. Stack-allocated 32-entry buffer matches
/// the x86_64 arm's `MAX_REGIONS` — QEMU + AAVMF emits on the order of
/// a dozen descriptor entries for a 256 MiB guest.
fn carve_from_memory_map(
    map: &MemoryMapOwned,
    capacity_bytes: u64,
) -> Option<(u64, u64)> {
    const MAX_REGIONS: usize = 32;
    const PAGE_SIZE: u64 = 4096;
    let mut buf: [(u64, u64, RegionKind); MAX_REGIONS] =
        [(0, 0, RegionKind::Other); MAX_REGIONS];
    let mut n = 0usize;
    for d in map.entries() {
        if n >= MAX_REGIONS {
            break;
        }
        let start = d.phys_start;
        let end = start + d.page_count * PAGE_SIZE;
        let kind = if d.ty == MemoryType::CONVENTIONAL {
            RegionKind::Usable
        } else {
            RegionKind::Other
        };
        buf[n] = (start, end, kind);
        n += 1;
    }
    dma::carve_dma_region(&buf[..n], capacity_bytes)
}

// ---------------------------------------------------------------------------
// Accessor helpers — same shape as arch::uefi::memory (minus
// `with_page_table`, which has no aarch64 analogue without dragging
// in `cortex-a` for TTBR1_EL1 access).
// ---------------------------------------------------------------------------

/// Call `f` with a mutable reference to the global `UefiFrameAllocator`.
///
/// # Panics
/// Panics if `init()` has not been called yet.
pub fn with_frame_allocator<R>(f: impl FnOnce(&mut UefiFrameAllocator) -> R) -> R {
    let mut guard = FRAME_ALLOCATOR.lock();
    f(guard.as_mut().expect("aarch64 uefi memory::init() not called"))
}

/// Call `f` with a mutable reference to the global `DmaPool`, if one
/// was carved at boot. Returns `None` when `init()` ran on a memory
/// map where no CONVENTIONAL region fit the configured pool size —
/// a future virtio-drivers-on-aarch64 HAL then panics on `dma_alloc`,
/// same graceful-fail behavior the x86_64-UEFI arm has. Same signature
/// as `arch::uefi::memory::with_dma_pool` so the virtio HAL compiles
/// against either arm's `arch::memory::` re-export.
pub fn with_dma_pool<R>(f: impl FnOnce(&mut DmaPool) -> R) -> Option<R> {
    let mut guard = DMA_POOL.lock();
    guard.as_mut().map(f)
}

/// Return the number of 4 KiB usable frames reported by the firmware
/// memory map. Available after `init()`.
pub fn usable_frame_count() -> usize {
    with_frame_allocator(|fa| fa.usable_frame_count())
}

// ---------------------------------------------------------------------------
// UefiFrameAllocator
// ---------------------------------------------------------------------------

/// A boot-time frame allocator that hands out 4 KiB frames from the
/// firmware's `MemoryMapOwned`.
///
/// Frames are yielded in the order the firmware reports them (usually
/// physical-address ascending); each frame is returned at most once.
/// Only descriptors with `ty == MemoryType::CONVENTIONAL` contribute.
///
/// Unlike the x86_64-UEFI arm's `UefiFrameAllocator`, this one yields
/// bare `u64` physical addresses rather than `x86_64::structures::
/// paging::PhysFrame` — the `x86_64` crate's paging types are gated on
/// `target_arch = "x86_64"` internally, so they're unreachable on the
/// aarch64 build. Callers that need a frame use the raw u64; a future
/// aarch64 page-table manager can wrap it in an aarch64 frame type.
pub struct UefiFrameAllocator {
    /// Owned buffer carrying the firmware's memory descriptors. Kept
    /// alive for the lifetime of the allocator so the descriptor slice
    /// we iterate over stays valid.
    map: MemoryMapOwned,
    /// Monotonically increasing cursor over the flattened usable-frame
    /// sequence. Matches the pattern the x86_64 arm's
    /// `UefiFrameAllocator::next` uses.
    next: usize,
    /// Half-open `(start, end)` physical range reserved for the DMA
    /// pool. Frames whose start address falls inside this range are
    /// skipped so the two allocators never hand out the same page.
    /// Mirrors the x86_64 arm's `UefiFrameAllocator::reserved`.
    reserved: Option<(u64, u64)>,
}

// SAFETY: `MemoryMapOwned` holds a `NonNull<[u8]>` under the covers
// (the firmware-allocated descriptor buffer) which is not auto-`Send`.
// The kernel is single-threaded at boot; after EBS we own the buffer
// exclusively and mediate all access through the `spin::Mutex` around
// the singleton. Concurrent CPUs (SMP bring-up) are not online until
// well after `init()` returns. Same pattern the x86_64-UEFI arm uses
// on its `UefiFrameAllocator`.
unsafe impl Send for UefiFrameAllocator {}

impl UefiFrameAllocator {
    /// Build a new allocator from the firmware's memory map, excluding
    /// frames that fall inside the `reserved` DMA range (if any).
    pub fn new(map: MemoryMapOwned, reserved: Option<(u64, u64)>) -> Self {
        Self { map, next: 0, reserved }
    }

    /// Total number of usable 4 KiB frames visible in the memory map,
    /// after DMA-reserved frames are excluded.
    pub fn usable_frame_count(&self) -> usize {
        self.usable_frames().count()
    }

    /// Iterator over every usable 4 KiB physical frame address in the
    /// memory map, minus any that fall inside the DMA carve-out.
    ///
    /// Flattens each `CONVENTIONAL` descriptor's `[phys_start,
    /// phys_start + page_count * 4 KiB)` range into individual 4 KiB
    /// frame start addresses. Descriptor order is preserved; within
    /// each descriptor, frames emerge in ascending address order.
    fn usable_frames(&self) -> impl Iterator<Item = u64> + '_ {
        const PAGE_SIZE: u64 = 4096;
        let reserved = self.reserved;
        self.map
            .entries()
            .filter(|d| d.ty == MemoryType::CONVENTIONAL)
            .flat_map(|d| {
                let start = d.phys_start;
                let count = d.page_count;
                (0..count).map(move |i| start + i * PAGE_SIZE)
            })
            .filter(move |&addr| {
                // Skip the DMA carve-out window. Exactly the x86_64
                // arm's `UefiFrameAllocator::usable_frames` filter.
                match reserved {
                    Some((r_start, r_end)) => !(addr >= r_start && addr < r_end),
                    None => true,
                }
            })
    }

    /// Allocate a single 4 KiB frame and return its physical address.
    /// Returns `None` on exhaustion. Mirrors the `FrameAllocator<Size4KiB>`
    /// impl the x86_64-UEFI arm exposes — aarch64 lacks the `x86_64`
    /// crate's `PhysFrame` type, so we return the raw u64 PA directly.
    #[allow(dead_code)]
    pub fn allocate_frame(&mut self) -> Option<u64> {
        let frame = self.usable_frames().nth(self.next);
        self.next += 1;
        frame
    }
}
