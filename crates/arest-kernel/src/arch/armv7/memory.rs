// crates/arest-kernel/src/arch/armv7/memory.rs
//
// Page-table access and physical frame allocation for the armv7 UEFI
// boot path (#387, armv7 analogue of `arch::aarch64::memory` from
// reference commits 3d4b3b0 / b72b2b6). Consumes the UEFI-provided
// `MemoryMapOwned` that `boot::exit_boot_services` returns, stands up
// a boot-time frame allocator behind the same accessor surface the
// aarch64 UEFI arm publishes, and carves a 2 MiB DMA pool out of the
// reclaimable region of that map for a future virtio-mmio bring-up
// (#388).
//
// Divergence from the aarch64 UEFI arm (`arch::aarch64::memory`):
//
//   * Pointer width is 32-bit on armv7 (`target_pointer_width = 32` in
//     `arest-kernel-armv7-uefi.json`). Every physical address fits in
//     a `u32`. The aarch64 arm uses `u64` PA throughout — here the
//     allocator's `reserved` field, the `allocate_frame` return, and
//     the local PA / size arithmetic all narrow to `u32`. We still
//     widen at the firmware boundary (`MemoryDescriptor::phys_start`
//     + `page_count` are spec-`u64`) and at the `dma::DmaPool` /
//     `dma::carve_dma_region` boundary (the shared `dma` module is
//     `u64`-addressed and stays untouched per the cross-arch contract
//     — it works fine on 32-bit armv7 because every value we hand it
//     fits in the lower 32 bits).
//
//   * No `OffsetPageTable` construction (same as aarch64). x86_64's
//     UEFI arm builds an `x86_64::structures::paging::OffsetPageTable`
//     because virtio's HAL::share calls `pt.translate_addr(va)` to
//     round-trip kernel VAs back to PAs through a page-walker. On
//     armv7 UEFI the firmware identity-maps physical RAM under
//     ArmVirtPkg (QEMU virt-armv7), so phys == virt across every
//     address the kernel cares about; the future armv7 virtio-mmio
//     port (#388) will translate by casting directly rather than
//     walking a page table.
//
//   * No TTBR0/TTBR1 inspection (TTBR is armv7's CR3 analogue, just
//     like aarch64's TTBR_EL1). UEFI firmware on QEMU virt-armv7
//     leaves TTBR pointing at a set of LPAE / short-descriptor page
//     tables covering RAM + device MMIO; the kernel rides those
//     tables for the rest of boot and never installs its own. A
//     later commit that stands up AREST-managed page tables would
//     read TTBR via a `cortex-a` crate helper or one-line inline-asm;
//     not needed for the memory bring-up this commit targets.
//
// Armv7 UEFI post-ExitBootServices state (QEMU virt-armv7 + ArmVirtPkg):
//   * CPU is in PL1 / kernel mode with paging + caches on.
//   * Firmware's page tables identity-map every RAM region plus MMIO
//     the firmware touched (PL011 @ 0x0900_0000, virtio-mmio slots @
//     0x0a00_0000 + 0x200*n, GIC, etc. — same physical layout as the
//     aarch64 virt machine, only the ISA differs). phys == virt.
//   * `MemoryMapOwned` is the firmware-returned snapshot of the
//     memory map at the moment of `boot::exit_boot_services`. Each
//     `MemoryDescriptor` carries `ty`, `phys_start`, `page_count`.
//     Frames marked `CONVENTIONAL` are free for OS use; after EBS
//     the `BOOT_SERVICES_CODE` / `BOOT_SERVICES_DATA` regions also
//     become reclaimable (UEFI spec §7.2.6, applied per #381 on the
//     x86_64 UEFI arm) — the firmware teardown is complete and
//     nothing the kernel owns lives in those pages.
//
// This module exposes the same public surface as the aarch64 UEFI
// arm for the subset armv7 actually uses:
//   * `init(memory_map)` — called once from a future
//     `entry_uefi_armv7::efi_main` (deferred to #346d / its successor)
//     right after `boot::exit_boot_services`; parks the frame
//     allocator + DMA pool in spin-locked statics. Returns the
//     physical-memory offset (always 0 on UEFI — ArmVirtPkg
//     identity-maps under QEMU virt-armv7, so phys == virt).
//   * `with_frame_allocator(f)` — accessor helper.
//   * `with_dma_pool(f)` — accessor for the carved DMA region.
//   * `usable_frame_count()` — convenience for the boot banner.
//
// The `dma` / `DmaPool` surface parallels the aarch64 UEFI arm
// exactly (#367) — carve a contiguous 2 MiB chunk out of the
// firmware memory map before building the frame allocator, reserve
// that range so the two allocators never collide. That lets the
// upcoming armv7 virtio-mmio port (#388) reach `memory::with_dma_pool`
// via the same code the x86_64 / aarch64 HAL paths use.

use spin::Mutex;
use uefi::mem::memory_map::{MemoryMap, MemoryMapOwned, MemoryType};

use crate::dma::{self, DmaPool, RegionKind};

// ---------------------------------------------------------------------------
// Global singletons
// ---------------------------------------------------------------------------

/// The boot-time frame allocator. Valid after `init()`.
static FRAME_ALLOCATOR: Mutex<Option<UefiFrameAllocator>> = Mutex::new(None);

/// Dedicated contiguous DMA pool for the future armv7 virtio-mmio
/// port (#388). Parallels the aarch64 UEFI arm's `DMA_POOL` — carved
/// out of the firmware memory map before `FRAME_ALLOCATOR` is handed
/// the same map, so the two allocators never collide. `None` only if
/// no reclaimable region (`CONVENTIONAL` / `BOOT_SERVICES_*`) fits
/// the configured pool size (unlikely on any reasonable QEMU
/// virt-armv7 guest; same graceful-degrade path the aarch64 / x86_64
/// arms have).
static DMA_POOL: Mutex<Option<DmaPool>> = Mutex::new(None);

/// Size of the DMA pool in 4 KiB pages (= 2 MiB). Matches the
/// aarch64 / x86_64 arms' `DMA_POOL_PAGES` so a future virtio-drivers-
/// on-armv7 bring-up with `NET_QUEUE_SIZE = 16` (two virtqueues +
/// rx/tx buffer pools) fits identically.
const DMA_POOL_PAGES: usize = 512;

// ---------------------------------------------------------------------------
// Public init
// ---------------------------------------------------------------------------

/// Initialise the memory subsystem from the firmware-provided memory
/// map returned by `boot::exit_boot_services`.
///
/// Must be called exactly once, immediately after `exit_boot_services`
/// (while the firmware's identity-mapped page tables are still the
/// live TTBR). Returns the physical-memory offset (always 0 on
/// UEFI — the firmware identity-maps under QEMU virt-armv7 +
/// ArmVirtPkg so phys == virt for every address the kernel touches).
///
/// Returns a `u32` (not `u64` like the aarch64 arm) because armv7
/// is 32-bit — a `u64` offset can't represent anything the kernel
/// can actually reach via a kernel pointer, so the type system
/// makes the truncation explicit at the boundary instead of letting
/// a u64 propagate into call sites that would silently `as u32` it.
///
/// # Safety
/// Caller must have just returned from `boot::exit_boot_services` so
/// (a) the page tables in TTBR are stable for the kernel's lifetime,
/// and (b) no other CPU is racing us on the page tables or the
/// memory map.
pub fn init(memory_map: MemoryMapOwned) -> u32 {
    // ArmVirtPkg's post-EBS page tables identity-map physical RAM,
    // so the "offset mapping" is zero — same convention as the
    // aarch64 / x86_64 UEFI arms.
    let phys_offset: u32 = 0;

    // Carve a dedicated contiguous DMA pool out of the firmware's
    // memory map BEFORE the general-purpose frame allocator sees it,
    // then tell the allocator about the reserved range so frames
    // inside it never leave both pools. Mirrors the aarch64 UEFI
    // arm (arch::aarch64::memory::init) so a future virtio-drivers-
    // on-armv7 HAL can reach `memory::with_dma_pool` identically on
    // all three arms.
    //
    // The carve helper crosses the `dma` module boundary in `u64`
    // (the shared `dma::carve_dma_region` is `u64`-typed and
    // un-modifiable per the #387 contract); we narrow back to `u32`
    // for our local `reserved` field so the allocator's hot-path
    // arithmetic stays in the native pointer width. Every `u64`
    // value crossing the boundary fits in the low 32 bits — UEFI
    // armv7 has a 32-bit physical address space by definition.
    let dma_capacity_bytes = (DMA_POOL_PAGES * dma::PAGE_SIZE) as u64;
    let (reserved, pool) =
        match carve_from_memory_map(&memory_map, dma_capacity_bytes) {
            Some((start, end)) => {
                // `phys_offset` widened to `u64` only to satisfy
                // `DmaPool::new`'s shared-module signature; the
                // value is 0 so no real widening happens.
                let p = DmaPool::new(start, DMA_POOL_PAGES, phys_offset as u64);
                // Narrow `(start, end)` back to `u32` for the
                // allocator's reserved-range filter — both fit by
                // the 32-bit-PA invariant above.
                (Some((start as u32, end as u32)), Some(p))
            }
            None => (None, None),
        };

    let frame_alloc = UefiFrameAllocator::new(memory_map, reserved);

    *DMA_POOL.lock() = pool;
    *FRAME_ALLOCATOR.lock() = Some(frame_alloc);

    phys_offset
}

/// Scan the UEFI memory map and hand the first-N reclaimable regions
/// to `dma::carve_dma_region` in the same `(u64, u64, RegionKind)`
/// shape the aarch64 / x86_64 arms use. Stack-allocated 32-entry
/// buffer matches those arms' `MAX_REGIONS` — QEMU + ArmVirtPkg
/// emits on the order of a dozen descriptor entries for a 256 MiB
/// guest, well under the cap.
///
/// Reclaimable types (per #381 on x86_64 UEFI, mirrored here):
///   * `CONVENTIONAL` — free for OS use from boot.
///   * `BOOT_SERVICES_CODE` / `BOOT_SERVICES_DATA` — reclaimable
///     after `ExitBootServices` (UEFI spec §7.2.6) because the
///     firmware teardown is complete and nothing the kernel owns
///     lives in those pages (our PE image is `LOADER_CODE`, the
///     `MemoryMapOwned` buffer is `LOADER_DATA`, the static-BSS heap
///     is inside the PE image). Folding these in alongside
///     `CONVENTIONAL` keeps the DMA pool's candidate window
///     symmetric with the general-purpose frame pool, exactly as on
///     the x86_64 UEFI arm.
///
/// Returns `(carve_start, carve_end)` as `u64` because the shared
/// `dma::carve_dma_region` returns `u64`; the caller (`init`)
/// narrows to `u32` for the per-frame reserved-range filter.
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
        let kind = if matches!(
            d.ty,
            MemoryType::CONVENTIONAL
                | MemoryType::BOOT_SERVICES_CODE
                | MemoryType::BOOT_SERVICES_DATA
        ) {
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
// Accessor helpers — same shape as arch::aarch64::memory (minus
// `with_page_table`, which has no armv7 analogue without dragging
// in a `cortex-a`-style crate for TTBR access).
// ---------------------------------------------------------------------------

/// Call `f` with a mutable reference to the global `UefiFrameAllocator`.
///
/// # Panics
/// Panics if `init()` has not been called yet.
pub fn with_frame_allocator<R>(f: impl FnOnce(&mut UefiFrameAllocator) -> R) -> R {
    let mut guard = FRAME_ALLOCATOR.lock();
    f(guard.as_mut().expect("armv7 uefi memory::init() not called"))
}

/// Call `f` with a mutable reference to the global `DmaPool`, if one
/// was carved at boot. Returns `None` when `init()` ran on a memory
/// map where no reclaimable region (`CONVENTIONAL` or post-EBS
/// `BOOT_SERVICES_*`) fit the configured pool size — a future
/// virtio-drivers-on-armv7 HAL then panics on `dma_alloc`, same
/// graceful-fail behavior the aarch64 / x86_64 UEFI arms have. Same
/// signature as `arch::aarch64::memory::with_dma_pool` so the
/// virtio HAL compiles against any arm's `arch::memory::` re-export.
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
/// Descriptors with `ty` in `{ CONVENTIONAL, BOOT_SERVICES_CODE,
/// BOOT_SERVICES_DATA }` contribute — the last two are reclaimable
/// after `ExitBootServices` per UEFI spec §7.2.6 (#381 policy applied
/// here exactly as on the x86_64 arm). `LOADER_DATA` (where the
/// in-flight `MemoryMapOwned` lives) and `LOADER_CODE` (our PE
/// image) stay off the free list; so does every MMIO/reserved type
/// the firmware reports.
///
/// Unlike the aarch64-UEFI arm's `UefiFrameAllocator` (which yields
/// `u64`), this one yields `u32` physical addresses — armv7 has a
/// 32-bit pointer width and 32-bit physical address space, and the
/// `x86_64` crate's paging types (which would supply a `PhysFrame`
/// abstraction) are gated on `target_arch = "x86_64"` internally so
/// they're unreachable on the armv7 build. Callers that need a frame
/// use the raw `u32` PA; a future armv7 page-table manager can wrap
/// it in an armv7 frame type.
pub struct UefiFrameAllocator {
    /// Owned buffer carrying the firmware's memory descriptors. Kept
    /// alive for the lifetime of the allocator so the descriptor
    /// slice we iterate over stays valid.
    map: MemoryMapOwned,
    /// Monotonically increasing cursor over the flattened usable-
    /// frame sequence. Matches the pattern the aarch64 / x86_64 arms'
    /// `UefiFrameAllocator::next` use.
    next: usize,
    /// Half-open `(start, end)` physical range reserved for the DMA
    /// pool, in the armv7-native `u32` PA width. Frames whose start
    /// address falls inside this range are skipped so the two
    /// allocators never hand out the same page. Mirrors the aarch64
    /// arm's `UefiFrameAllocator::reserved`, narrowed from `u64` per
    /// the 32-bit-PA invariant.
    reserved: Option<(u32, u32)>,
}

// SAFETY: `MemoryMapOwned` holds a `NonNull<[u8]>` under the covers
// (the firmware-allocated descriptor buffer) which is not auto-
// `Send`. The kernel is single-threaded at boot; after EBS we own
// the buffer exclusively and mediate all access through the
// `spin::Mutex` around the singleton. Concurrent CPUs (SMP bring-up)
// are not online until well after `init()` returns. Same pattern
// the aarch64 / x86_64 UEFI arms use on their `UefiFrameAllocator`.
unsafe impl Send for UefiFrameAllocator {}

impl UefiFrameAllocator {
    /// Build a new allocator from the firmware's memory map, excluding
    /// frames that fall inside the `reserved` DMA range (if any).
    pub fn new(map: MemoryMapOwned, reserved: Option<(u32, u32)>) -> Self {
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
    /// Flattens each reclaimable descriptor's `[phys_start,
    /// phys_start + page_count * 4 KiB)` range into individual 4 KiB
    /// frame start addresses. Descriptor order is preserved; within
    /// each descriptor, frames emerge in ascending address order.
    ///
    /// Per-frame arithmetic narrows `phys_start` (`u64`) and
    /// `page_count` (`u64`) to `u32` immediately on entry — armv7's
    /// 32-bit physical address space guarantees the truncation is
    /// lossless for any descriptor the firmware emits, and keeping
    /// the inner arithmetic in `u32` avoids `u64`-multiply libcalls
    /// (`__aeabi_lmul`) the LLVM ARM backend would otherwise emit
    /// for the `start + i * PAGE_SIZE` step.
    fn usable_frames(&self) -> impl Iterator<Item = u32> + '_ {
        const PAGE_SIZE: u32 = 4096;
        let reserved = self.reserved;
        self.map
            .entries()
            .filter(|d| {
                matches!(
                    d.ty,
                    MemoryType::CONVENTIONAL
                        | MemoryType::BOOT_SERVICES_CODE
                        | MemoryType::BOOT_SERVICES_DATA
                )
            })
            .flat_map(|d| {
                let start = d.phys_start as u32;
                let count = d.page_count as u32;
                (0..count).map(move |i| start + i * PAGE_SIZE)
            })
            .filter(move |&addr| {
                // Skip the DMA carve-out window. Exactly the aarch64
                // / x86_64 arms' `UefiFrameAllocator::usable_frames`
                // filter, narrowed to `u32`.
                match reserved {
                    Some((r_start, r_end)) => !(addr >= r_start && addr < r_end),
                    None => true,
                }
            })
    }

    /// Allocate a single 4 KiB frame and return its physical address.
    /// Returns `None` on exhaustion. Mirrors the aarch64 arm's
    /// `allocate_frame` (which also returns a raw integer PA rather
    /// than a typed `PhysFrame`), narrowed from `u64` to `u32` for
    /// armv7's 32-bit address space.
    #[allow(dead_code)]
    pub fn allocate_frame(&mut self) -> Option<u32> {
        let frame = self.usable_frames().nth(self.next);
        self.next += 1;
        frame
    }
}
