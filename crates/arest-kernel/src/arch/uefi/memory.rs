// crates/arest-kernel/src/arch/uefi/memory.rs
//
// Page-table access and physical frame allocation for the UEFI boot
// path (#344 step 4c). Parallels `arch::x86_64::memory` but consumes
// the UEFI-provided `MemoryMapOwned` rather than `bootloader_api`'s
// `BootInfo`, so the two paths can feed the same downstream accessor
// API (`with_page_table`, `with_frame_allocator`, `usable_frame_count`).
//
// UEFI post-ExitBootServices state (x86_64):
//   * CPU is in 64-bit long mode, paging on, CR3 points at a 4-level
//     page table the firmware set up.
//   * The firmware's page tables identity-map every RAM region, plus
//     any MMIO the firmware touched. That means phys == virt for
//     every address the kernel cares about — i.e. `phys_offset = 0`.
//     The `OffsetPageTable` we construct here inherits that identity
//     mapping as its "offset mapping" and keeps working for the rest
//     of boot until the kernel installs its own page tables.
//   * `MemoryMapOwned` is the firmware-returned snapshot of the
//     memory map at the moment of `boot::exit_boot_services`. Each
//     `MemoryDescriptor` carries `ty`, `phys_start`, `page_count`.
//     Frames marked `CONVENTIONAL` are free for OS use; after EBS
//     the `BOOT_SERVICES_CODE` / `BOOT_SERVICES_DATA` regions also
//     become reclaimable (UEFI spec §7.2.6). For the step-4c
//     bring-up we only claim `CONVENTIONAL` — the boot-services
//     regions still hold our in-flight data structures (the
//     `MemoryMapOwned` itself lives in `LOADER_DATA` which *is*
//     reserved), so keeping them off-limits is the conservative
//     choice. A later step can enlarge the pool once the freshly
//     mapped heap has copied anything we care about out of them.
//
// This module exposes the same public surface as the BIOS arm:
//   * `init(memory_map)` — called once from `entry_uefi::efi_main`
//     right after `boot::exit_boot_services`; builds the
//     `OffsetPageTable` and the frame allocator, parks them in
//     spin-locked statics.
//   * `with_page_table(f)` / `with_frame_allocator(f)` — accessor
//     helpers.
//   * `usable_frame_count()` — convenience for the boot banner.
//
// The `dma` / `DmaPool` surface that the x86_64 arm carves is NOT
// re-exported here yet — UEFI path kernel_run integration (step 4d)
// doesn't reach virtio yet, so a DMA pool would be dead weight. The
// BIOS path's `arch::memory::with_dma_pool` caller (`virtio::HAL`) is
// cfg-gated out of the UEFI build.

use spin::Mutex;
use uefi::mem::memory_map::{MemoryMap, MemoryMapOwned, MemoryType};
use x86_64::structures::paging::{FrameAllocator, OffsetPageTable, PhysFrame, Size4KiB};
use x86_64::{PhysAddr, VirtAddr};

// ---------------------------------------------------------------------------
// Global singletons
// ---------------------------------------------------------------------------

/// The active `OffsetPageTable`. Valid after `init()`.
static PAGE_TABLE: Mutex<Option<OffsetPageTable<'static>>> = Mutex::new(None);

/// The boot-time frame allocator. Valid after `init()`.
static FRAME_ALLOCATOR: Mutex<Option<UefiFrameAllocator>> = Mutex::new(None);

// ---------------------------------------------------------------------------
// Public init
// ---------------------------------------------------------------------------

/// Initialise the memory subsystem from the firmware-provided memory
/// map returned by `boot::exit_boot_services`.
///
/// Must be called exactly once, immediately after `exit_boot_services`
/// (while the firmware's identity-mapped page tables are still the
/// live CR3). Returns the physical-memory offset (always 0 on UEFI —
/// phys == virt under the firmware's identity mapping).
///
/// # Safety
/// Caller must have just returned from `boot::exit_boot_services` so
/// (a) the page tables in CR3 are stable for the kernel's lifetime,
/// and (b) no other CPU is racing us on the page tables or the memory
/// map.
pub fn init(memory_map: MemoryMapOwned) -> u64 {
    // UEFI's post-EBS page tables identity-map physical RAM, so the
    // "offset mapping" is zero.
    let phys_offset_virt = VirtAddr::new(0);

    // SAFETY: post-EBS the firmware hands us a stable CR3 pointing at
    // page tables that cover the full RAM identity-mapped. We take
    // ownership of them for the lifetime of the kernel.
    let page_table = unsafe { build_offset_page_table(phys_offset_virt) };

    let frame_alloc = UefiFrameAllocator::new(memory_map);

    *PAGE_TABLE.lock() = Some(page_table);
    *FRAME_ALLOCATOR.lock() = Some(frame_alloc);

    0
}

// ---------------------------------------------------------------------------
// Accessor helpers — same shape as arch::x86_64::memory
// ---------------------------------------------------------------------------

/// Call `f` with a mutable reference to the global `OffsetPageTable`.
///
/// # Panics
/// Panics if `init()` has not been called yet.
//
// Unused until step 4d wires `kernel_run` on the UEFI path — the BIOS
// arm's `with_page_table` is reached via `map_user_page` /
// `userspace::launch_test_payload`, both `cfg(not(target_os = "uefi"))`
// gated today. Keeping the helper here (rather than adding it in step
// 4d) makes the arch surface symmetric between BIOS and UEFI right
// now, which matters for the shared kernel body call sites.
#[allow(dead_code)]
pub fn with_page_table<R>(f: impl FnOnce(&mut OffsetPageTable<'static>) -> R) -> R {
    let mut guard = PAGE_TABLE.lock();
    f(guard.as_mut().expect("uefi memory::init() not called"))
}

/// Call `f` with a mutable reference to the global `UefiFrameAllocator`.
///
/// # Panics
/// Panics if `init()` has not been called yet.
pub fn with_frame_allocator<R>(f: impl FnOnce(&mut UefiFrameAllocator) -> R) -> R {
    let mut guard = FRAME_ALLOCATOR.lock();
    f(guard.as_mut().expect("uefi memory::init() not called"))
}

/// Return the number of 4 KiB usable frames reported by the firmware
/// memory map. Available after `init()`.
pub fn usable_frame_count() -> usize {
    with_frame_allocator(|fa| fa.usable_frame_count())
}

// ---------------------------------------------------------------------------
// OffsetPageTable construction
// ---------------------------------------------------------------------------

/// Build an `OffsetPageTable` that uses `phys_offset` (= 0 on UEFI) to
/// translate physical page-table addresses to virtual ones.
///
/// # Safety
/// The caller must guarantee:
/// 1. CR3 points at a 4-level page table that stays valid for the
///    kernel's lifetime (true post-`exit_boot_services`).
/// 2. `phys_offset` is the base of a complete physical-memory mapping
///    — zero on UEFI, since the firmware identity-maps.
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
// UefiFrameAllocator
// ---------------------------------------------------------------------------

/// A `FrameAllocator` that hands out 4 KiB frames from the firmware's
/// `MemoryMapOwned`.
///
/// Frames are yielded in the order the firmware reports them (usually
/// physical-address ascending); each frame is returned at most once.
/// Only descriptors with `ty == MemoryType::CONVENTIONAL` contribute
/// — reclaiming `BOOT_SERVICES_*` after EBS is a later step, kept off
/// the first-boot path to avoid accidentally trampling the in-flight
/// `MemoryMapOwned` itself (which lives in `LOADER_DATA`).
pub struct UefiFrameAllocator {
    /// Owned buffer carrying the firmware's memory descriptors. Kept
    /// alive for the lifetime of the allocator so the descriptor slice
    /// we iterate over stays valid.
    map: MemoryMapOwned,
    /// Monotonically increasing cursor over the flattened usable-frame
    /// sequence. Matches the pattern the BIOS arm's
    /// `BootInfoFrameAllocator::next` uses.
    next: usize,
}

// SAFETY: `MemoryMapOwned` holds a `NonNull<[u8]>` under the covers
// (the firmware-allocated descriptor buffer) which is not auto-`Send`.
// The kernel is single-threaded at boot; after EBS we own the buffer
// exclusively and mediate all access through the `spin::Mutex` around
// the singleton. Concurrent CPUs (SMP bring-up) are not online until
// well after `init()` returns. Wrapping is what every other no_std
// OS doing the same firmware-handoff does (redox, rCore), and the
// BIOS arm's equivalent `&'static MemoryRegions` only skips the
// declaration because `bootloader_api` already `impl`'s `Send` itself.
unsafe impl Send for UefiFrameAllocator {}

impl UefiFrameAllocator {
    /// Build a new allocator from the firmware's memory map.
    pub fn new(map: MemoryMapOwned) -> Self {
        Self { map, next: 0 }
    }

    /// Total number of usable 4 KiB frames visible in the memory map.
    pub fn usable_frame_count(&self) -> usize {
        self.usable_frames().count()
    }

    /// Iterator over every usable `PhysFrame` in the memory map.
    ///
    /// Flattens each `CONVENTIONAL` descriptor's `[phys_start,
    /// phys_start + page_count * 4 KiB)` range into individual 4 KiB
    /// `PhysFrame`s. Descriptor order is preserved; within each
    /// descriptor, frames emerge in ascending address order.
    fn usable_frames(&self) -> impl Iterator<Item = PhysFrame> + '_ {
        const PAGE_SIZE: u64 = 4096;
        self.map
            .entries()
            .filter(|d| d.ty == MemoryType::CONVENTIONAL)
            .flat_map(|d| {
                let start = PhysAddr::new(d.phys_start);
                let end = PhysAddr::new(d.phys_start + d.page_count * PAGE_SIZE);
                let start_frame = PhysFrame::containing_address(start);
                // `end` is the exclusive upper bound; back off one byte
                // so `containing_address` lands on the last *included*
                // frame, matching the BIOS arm's `end - 1u64` pattern.
                let end_frame = PhysFrame::containing_address(end - 1u64);
                PhysFrame::range_inclusive(start_frame, end_frame)
            })
    }
}

// SAFETY: `UefiFrameAllocator` hands each frame out exactly once and
// only yields frames the firmware marked `CONVENTIONAL`.
unsafe impl FrameAllocator<Size4KiB> for UefiFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        let frame = self.usable_frames().nth(self.next);
        self.next += 1;
        frame
    }
}
