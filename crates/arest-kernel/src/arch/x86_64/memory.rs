// crates/arest-kernel/src/arch/x86_64/memory.rs
//
// Page-table access and physical frame allocation for the AREST kernel.
// Lives under `arch/x86_64/` (#344 step 2); the kernel body reaches it
// through `crate::arch::memory`. Still coupled to `bootloader_api`'s
// BootInfo shape — the shared BootInfo abstraction is a step-4 concern
// once UEFI ExitBootServices lands.
//
// The bootloader (rust-osdev/bootloader 0.11) enters the kernel in 64-bit
// long mode with:
//   • a 4-level identity-plus-offset page table already active
//   • `BootInfo::physical_memory_offset` — the virtual address at which the
//     *entire* physical address space is mapped (offset mapping)
//   • `BootInfo::memory_regions` — a slice of `MemoryRegion` records
//     describing which physical frames the OS may use
//
// This module exposes:
//   • `init(boot_info)` — called once from `kernel_main`; builds the
//     `OffsetPageTable` and the `BootInfoFrameAllocator`, then parks them in
//     spin-locked statics.
//   • `with_page_table(f)` / `with_frame_allocator(f)` — accessor helpers
//     that let other modules borrow the singletons without moving them.
//   • `usable_frame_count()` — convenience for the boot banner.

use bootloader_api::info::{MemoryRegionKind, MemoryRegions};
use bootloader_api::BootInfo;
use spin::Mutex;
use x86_64::structures::paging::{FrameAllocator, OffsetPageTable, PhysFrame, Size4KiB};
use x86_64::{PhysAddr, VirtAddr};

use crate::dma::{self, DmaPool, RegionKind};

// ---------------------------------------------------------------------------
// Global singletons
// ---------------------------------------------------------------------------

/// The active `OffsetPageTable`. Valid after `init()`.
static PAGE_TABLE: Mutex<Option<OffsetPageTable<'static>>> = Mutex::new(None);

/// The boot-time frame allocator. Valid after `init()`.
static FRAME_ALLOCATOR: Mutex<Option<BootInfoFrameAllocator>> = Mutex::new(None);

/// The dedicated contiguous DMA pool for virtio-drivers (#268). Backed
/// by a chunk of physical memory carved out of the bootloader's memory
/// map before `FRAME_ALLOCATOR` is handed the same map — so the two
/// allocators never collide. Populated by `init()`; `None` on hardware
/// where no usable region has enough contiguous space (the kernel
/// then falls back to loopback networking).
static DMA_POOL: Mutex<Option<DmaPool>> = Mutex::new(None);

/// Size of the DMA pool in 4 KiB pages (= 2 MiB). Sized for the virtio-
/// net bring-up on `NET_QUEUE_SIZE = 16`: two virtqueues + rx/tx buffer
/// pools (`NET_QUEUE_SIZE * NET_BUF_LEN` each) + driver metadata and
/// plenty of headroom. Bump once here if a future device class (virtio-
/// blk, virtio-gpu) needs more than virtio-net's footprint.
const DMA_POOL_PAGES: usize = 512;

// ---------------------------------------------------------------------------
// Public init
// ---------------------------------------------------------------------------

/// Initialise the memory subsystem from `BootInfo`.
///
/// Must be called exactly once, early in `kernel_main`, before any code that
/// needs page-table access or physical frame allocation.
pub fn init(boot_info: &'static BootInfo) {
    // SAFETY: The bootloader guarantees that `physical_memory_offset` is the
    // base of a complete physical-memory mapping that was established before
    // our entry point was called, and that the mapping remains valid for the
    // entire lifetime of the kernel ('static).
    let phys_offset_virt = VirtAddr::new(boot_info.physical_memory_offset.into_option()
        .expect("bootloader did not supply physical_memory_offset"));
    let phys_offset: u64 = phys_offset_virt.as_u64();

    let page_table = unsafe { build_offset_page_table(phys_offset_virt) };

    // Carve a dedicated contiguous DMA pool out of the bootloader's memory
    // map *before* constructing the general-purpose frame allocator, and
    // tell the allocator about the reserved range so the same physical
    // frames never leave both pools.
    //
    // Collecting regions onto the stack lets the pure-logic carver (which
    // we can test on the host) do the heavy lifting without pulling
    // bootloader types into dma.rs.
    let dma_capacity_bytes = (DMA_POOL_PAGES * dma::PAGE_SIZE) as u64;
    let (reserved, pool) =
        match carve_from_memory_regions(&boot_info.memory_regions, dma_capacity_bytes) {
            Some((start, end)) => {
                let pool = DmaPool::new(start, DMA_POOL_PAGES, phys_offset);
                (Some((start, end)), Some(pool))
            }
            None => (None, None),
        };

    let frame_alloc = BootInfoFrameAllocator::new(&boot_info.memory_regions, reserved);

    *PAGE_TABLE.lock() = Some(page_table);
    *DMA_POOL.lock() = pool;
    *FRAME_ALLOCATOR.lock() = Some(frame_alloc);
}

/// Scan the bootloader's memory map and call `dma::carve_dma_region` on
/// the result. Collects up to `MAX_REGIONS` entries onto a stack array
/// so the pure-logic carver can stay no-alloc and slice-based. The
/// bootloader realistically emits well under 32 regions on any x86_64
/// machine (even a full Q35 machine with PCIe + ACPI + UEFI typically
/// reports fewer than 16); 32 is a generous ceiling.
fn carve_from_memory_regions(
    regions: &MemoryRegions,
    capacity_bytes: u64,
) -> Option<(u64, u64)> {
    const MAX_REGIONS: usize = 32;
    let mut buf: [(u64, u64, RegionKind); MAX_REGIONS] =
        [(0, 0, RegionKind::Other); MAX_REGIONS];
    let mut n = 0usize;
    for r in regions.iter() {
        if n >= MAX_REGIONS {
            break;
        }
        let kind = match r.kind {
            MemoryRegionKind::Usable => RegionKind::Usable,
            _ => RegionKind::Other,
        };
        buf[n] = (r.start, r.end, kind);
        n += 1;
    }
    dma::carve_dma_region(&buf[..n], capacity_bytes)
}

// ---------------------------------------------------------------------------
// Accessor helpers
// ---------------------------------------------------------------------------

/// Call `f` with a mutable reference to the global `OffsetPageTable`.
///
/// # Panics
/// Panics if `init()` has not been called yet.
pub fn with_page_table<R>(f: impl FnOnce(&mut OffsetPageTable<'static>) -> R) -> R {
    let mut guard = PAGE_TABLE.lock();
    f(guard.as_mut().expect("memory::init() not called"))
}

/// Call `f` with a mutable reference to the global `BootInfoFrameAllocator`.
///
/// # Panics
/// Panics if `init()` has not been called yet.
pub fn with_frame_allocator<R>(f: impl FnOnce(&mut BootInfoFrameAllocator) -> R) -> R {
    let mut guard = FRAME_ALLOCATOR.lock();
    f(guard.as_mut().expect("memory::init() not called"))
}

/// Call `f` with a mutable reference to the global `DmaPool`, if one was
/// carved at boot. Returns `None` to the caller if `init()` ran on a
/// memory map where no usable region fit the configured pool size —
/// `virtio::KernelHal::dma_alloc` then panics, which is the same
/// behavior the old boot-allocator path had on allocation failure.
pub fn with_dma_pool<R>(f: impl FnOnce(&mut DmaPool) -> R) -> Option<R> {
    let mut guard = DMA_POOL.lock();
    guard.as_mut().map(f)
}

/// Return the number of 4 KiB usable frames reported by the bootloader.
/// Available after `init()`.
pub fn usable_frame_count() -> usize {
    with_frame_allocator(|fa| fa.usable_frame_count())
}

// ---------------------------------------------------------------------------
// OffsetPageTable construction
// ---------------------------------------------------------------------------

/// Build an `OffsetPageTable` that uses `physical_memory_offset` to translate
/// physical page-table addresses to virtual ones.
///
/// # Safety
/// The caller must guarantee:
/// 1. `phys_offset` is the base of a complete physical-memory mapping.
/// 2. The mapping is valid for `'static`.
/// 3. This function is called at most once.
unsafe fn build_offset_page_table(phys_offset: VirtAddr) -> OffsetPageTable<'static> {
    use x86_64::registers::control::Cr3;
    use x86_64::structures::paging::PageTable;

    let (l4_frame, _) = Cr3::read();
    let phys = l4_frame.start_address();
    let virt = phys_offset + phys.as_u64();
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();

    // SAFETY: see contract on the outer unsafe fn.
    OffsetPageTable::new(&mut *page_table_ptr, phys_offset)
}

// ---------------------------------------------------------------------------
// BootInfoFrameAllocator
// ---------------------------------------------------------------------------

/// A `FrameAllocator` that hands out 4 KiB frames from the `MemoryRegions`
/// slice supplied by the bootloader.
///
/// Frames are yielded in ascending physical address order; each frame is
/// returned at most once. When a DMA reservation range is set, frames
/// inside that range are hidden from the iterator so they stay reserved
/// for the `DmaPool` and never collide with general-purpose allocation.
pub struct BootInfoFrameAllocator {
    regions: &'static MemoryRegions,
    next: usize,
    /// Half-open `(start, end)` physical range reserved for the DMA
    /// pool. `None` if no pool was carved at boot.
    reserved: Option<(u64, u64)>,
}

impl BootInfoFrameAllocator {
    /// Create a new allocator from the bootloader's memory map. Only
    /// regions with kind `MemoryRegionKind::Usable` are handed out, and
    /// frames whose start address falls inside `reserved` (the DMA
    /// pool carve-out) are skipped.
    pub fn new(regions: &'static MemoryRegions, reserved: Option<(u64, u64)>) -> Self {
        Self { regions, next: 0, reserved }
    }

    /// Total number of usable 4 KiB frames visible in the memory map,
    /// after the DMA reservation is excluded.
    pub fn usable_frame_count(&self) -> usize {
        self.usable_frames().count()
    }

    /// Iterator over every usable `PhysFrame` in the memory map,
    /// skipping any frame that falls inside the DMA reservation.
    fn usable_frames(&self) -> impl Iterator<Item = PhysFrame> {
        let reserved = self.reserved;
        self.regions
            .iter()
            .filter(|r| r.kind == MemoryRegionKind::Usable)
            .flat_map(|r| {
                let start = PhysAddr::new(r.start);
                let end   = PhysAddr::new(r.end);
                let start_frame = PhysFrame::containing_address(start);
                let end_frame   = PhysFrame::containing_address(end - 1u64);
                PhysFrame::range_inclusive(start_frame, end_frame)
            })
            .filter(move |f| {
                match reserved {
                    Some((rs, re)) => {
                        let addr = f.start_address().as_u64();
                        !(addr >= rs && addr < re)
                    }
                    None => true,
                }
            })
    }
}

// SAFETY: `BootInfoFrameAllocator` hands each frame out exactly once and
// only yields frames that the bootloader marked as `Usable`.
unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        let frame = self.usable_frames().nth(self.next);
        self.next += 1;
        frame
    }
}

// ---------------------------------------------------------------------------
// User-page mapping (Sec-6.1)
// ---------------------------------------------------------------------------

/// Errors reported by `map_user_page` / `remap_user_page_flags`.
#[derive(Debug)]
pub enum MapUserError {
    /// The frame allocator is exhausted.
    OutOfFrames,
    /// The target virtual address is already mapped.
    AlreadyMapped,
    /// The virtual address is not 4 KiB aligned.
    Misaligned,
    /// Any other paging error reported by the x86_64 crate.
    Paging,
}

/// Allocate a fresh 4 KiB physical frame and map it at `virt` with
/// USER_ACCESSIBLE + PRESENT + the supplied extra flags. The TLB
/// is flushed for `virt` before the function returns so the new
/// mapping is visible to the caller on the next access.
///
/// Primary hook for setting up ring-3-accessible pages (user text,
/// user stack, and, once Sec-6.3 lands, per-tenant address spaces).
///
/// # Errors
/// - `Misaligned` if `virt` is not 4 KiB aligned.
/// - `OutOfFrames` if the boot-time allocator has no more frames.
/// - `AlreadyMapped` if `virt` is already bound to a frame.
/// - `Paging` for any other error from `Mapper::map_to`.
pub fn map_user_page(
    virt: VirtAddr,
    extra_flags: x86_64::structures::paging::PageTableFlags,
) -> Result<PhysFrame, MapUserError> {
    use x86_64::instructions::tlb;
    use x86_64::structures::paging::{Mapper, Page, PageTableFlags};

    if virt.as_u64() & 0xFFF != 0 {
        return Err(MapUserError::Misaligned);
    }
    let page: Page<Size4KiB> = Page::containing_address(virt);

    let flags = PageTableFlags::PRESENT
        | PageTableFlags::USER_ACCESSIBLE
        | extra_flags;

    with_page_table(|pt| {
        with_frame_allocator(|fa| {
            let frame = fa.allocate_frame().ok_or(MapUserError::OutOfFrames)?;
            // SAFETY: Frame is freshly allocated from the boot-time
            // allocator and is not mapped anywhere else. No aliasing.
            unsafe {
                pt.map_to(page, frame, flags, fa)
                    .map_err(|e| match e {
                        x86_64::structures::paging::mapper::MapToError::PageAlreadyMapped(_) =>
                            MapUserError::AlreadyMapped,
                        _ => MapUserError::Paging,
                    })?
                    .flush();
            }
            tlb::flush(virt);
            Ok(frame)
        })
    })
}

/// Re-map an existing user page with a new set of flags. Used to
/// flip the user text page from RW (during payload copy) to RX
/// (before the iretq descent). The backing frame is unchanged.
pub fn remap_user_page_flags(
    virt: VirtAddr,
    extra_flags: x86_64::structures::paging::PageTableFlags,
) -> Result<(), MapUserError> {
    use x86_64::instructions::tlb;
    use x86_64::structures::paging::{Mapper, Page, PageTableFlags};

    if virt.as_u64() & 0xFFF != 0 {
        return Err(MapUserError::Misaligned);
    }
    let page: Page<Size4KiB> = Page::containing_address(virt);
    let flags = PageTableFlags::PRESENT
        | PageTableFlags::USER_ACCESSIBLE
        | extra_flags;

    with_page_table(|pt| {
        // SAFETY: page is already mapped (we mapped it via
        // map_user_page just before); we only update the flags.
        unsafe {
            pt.update_flags(page, flags)
                .map_err(|_| MapUserError::Paging)?
                .flush();
        }
        tlb::flush(virt);
        Ok(())
    })
}
