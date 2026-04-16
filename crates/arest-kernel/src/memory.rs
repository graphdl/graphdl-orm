// crates/arest-kernel/src/memory.rs
//
// Page-table access and physical frame allocation for the AREST kernel.
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

// ---------------------------------------------------------------------------
// Global singletons
// ---------------------------------------------------------------------------

/// The active `OffsetPageTable`. Valid after `init()`.
static PAGE_TABLE: Mutex<Option<OffsetPageTable<'static>>> = Mutex::new(None);

/// The boot-time frame allocator. Valid after `init()`.
static FRAME_ALLOCATOR: Mutex<Option<BootInfoFrameAllocator>> = Mutex::new(None);

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
    let phys_offset = VirtAddr::new(boot_info.physical_memory_offset.into_option()
        .expect("bootloader did not supply physical_memory_offset"));

    let page_table = unsafe { build_offset_page_table(phys_offset) };
    let frame_alloc = BootInfoFrameAllocator::new(&boot_info.memory_regions);

    *PAGE_TABLE.lock() = Some(page_table);
    *FRAME_ALLOCATOR.lock() = Some(frame_alloc);
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
/// returned at most once.
pub struct BootInfoFrameAllocator {
    regions: &'static MemoryRegions,
    next: usize,
}

impl BootInfoFrameAllocator {
    /// Create a new allocator from the bootloader's memory map.
    ///
    /// Only regions with kind `MemoryRegionKind::Usable` are handed out.
    pub fn new(regions: &'static MemoryRegions) -> Self {
        Self { regions, next: 0 }
    }

    /// Total number of usable 4 KiB frames visible in the memory map.
    pub fn usable_frame_count(&self) -> usize {
        self.usable_frames().count()
    }

    /// Iterator over every usable `PhysFrame` in the memory map.
    fn usable_frames(&self) -> impl Iterator<Item = PhysFrame> {
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
