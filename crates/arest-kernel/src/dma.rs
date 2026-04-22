// crates/arest-kernel/src/dma.rs
//
// Dedicated contiguous DMA pool for virtio-drivers (#268).
//
// ── Why a dedicated pool ────────────────────────────────────────────
// The bootloader's `BootInfoFrameAllocator` yields 4 KiB frames in
// ascending physical-address order, one per `allocate_frame` call.
// A *single* N-page request via N consecutive calls does therefore
// land N contiguous frames — but `virtio-drivers` issues several
// independent `Hal::dma_alloc` calls during `VirtIONet::new` (one per
// virtqueue plus one per rx/tx buffer pool), and nothing in the boot
// allocator guarantees those allocations stay in a single contiguous
// range. More important: once any *other* subsystem (user pages,
// kernel heap growth) has drawn frames from the boot allocator, the
// DMA allocations are interleaved with unrelated pages — and the
// virtio descriptor rings treat the physical addresses they store as
// raw pointers into their own ring buffers, so any gap between a
// virtqueue's descriptor table and its available/used rings triggers
// `DmaError` at setup time.
//
// The fix is a small, dedicated, contiguous pool carved at boot out
// of a usable memory region *before* the main frame allocator ever
// sees it. Every `Hal::dma_alloc` call bumps the pool's cursor; the
// backing frames are guaranteed adjacent in physical memory, guarant-
// eed 4 KiB-aligned, and guaranteed not to collide with anything the
// rest of the kernel hands out.
//
// ── Pure-logic design ───────────────────────────────────────────────
// Everything in this file is `u64`-addressed, no x86_64 intrinsics,
// no statics, no page-table access. The choice is deliberate: the
// host-side integration tests in `crates/arest-kernel-image/tests/
// dma_pool.rs` `#[path]`-include this source directly so the bump
// and carve logic can be exercised on the host target with `cargo
// test`. Glue that actually talks to paging / frames / virtio-drivers
// lives in `virtio.rs` and `memory.rs`.
//
// ── Not in scope ────────────────────────────────────────────────────
// * SMP-safe locking — single-core kernel, the `spin::Mutex` that
//   guards the pool in `virtio.rs` is enough.
// * IOMMU / VT-d — QEMU's SLiRP doesn't use it; HAL `share`/`unshare`
//   remain identity translations against the bootloader's offset map.
// * Free — the bump cursor is one-way; `Hal::dma_dealloc` is a no-op
//   for the same reason it already is against the boot allocator.

/// 4 KiB page — the virtio-drivers minimum allocation unit.
pub const PAGE_SIZE: usize = 4096;

/// Bump allocator over a reserved contiguous physical region.
///
/// Construct once at boot (`DmaPool::new`) and call `alloc` from the
/// HAL `dma_alloc` shim. Cursor is one-way — freed pages are leaked
/// for the lifetime of the kernel, which is fine for the single
/// virtio-net driver the kernel instantiates.
#[derive(Debug)]
pub struct DmaPool {
    /// Physical base address of the reserved region. Page-aligned.
    base_paddr: u64,
    /// Capacity expressed as a 4 KiB-page count.
    capacity_pages: usize,
    /// Monotonic cursor: index of the next free page within the pool.
    next_page: usize,
    /// Bootloader-supplied physical-to-virtual offset. Every DMA page
    /// is visible to the kernel at `paddr + phys_offset` via the
    /// rust-osdev bootloader's full-physmem offset mapping.
    phys_offset: u64,
}

impl DmaPool {
    /// Create a pool backed by `capacity_pages` 4 KiB frames starting
    /// at `base_paddr`. Panics if `base_paddr` is not page-aligned.
    pub fn new(base_paddr: u64, capacity_pages: usize, phys_offset: u64) -> Self {
        assert_eq!(
            base_paddr & (PAGE_SIZE as u64 - 1),
            0,
            "DmaPool base must be page-aligned",
        );
        Self { base_paddr, capacity_pages, next_page: 0, phys_offset }
    }

    /// Allocate `pages` contiguous 4 KiB frames. Returns the physical
    /// address of the first frame and the kernel-virtual pointer to
    /// the same frame via the bootloader's offset mapping. Returns
    /// `None` on exhaustion or on a zero-page request.
    pub fn alloc(&mut self, pages: usize) -> Option<(u64, u64)> {
        if pages == 0 {
            return None;
        }
        let new_next = self.next_page.checked_add(pages)?;
        if new_next > self.capacity_pages {
            return None;
        }
        let paddr = self.base_paddr + (self.next_page * PAGE_SIZE) as u64;
        let vaddr = paddr + self.phys_offset;
        self.next_page = new_next;
        Some((paddr, vaddr))
    }

    /// The `(start, end)` half-open physical range the pool manages.
    pub fn reserved_range(&self) -> (u64, u64) {
        let end = self.base_paddr + (self.capacity_pages * PAGE_SIZE) as u64;
        (self.base_paddr, end)
    }

    /// True iff `frame_paddr` (a 4 KiB frame start) lies anywhere
    /// inside the reserved range. Used by the BootInfoFrameAllocator
    /// to hide DMA-pool frames from the general-purpose allocator.
    pub fn contains_frame(&self, frame_paddr: u64) -> bool {
        let (start, end) = self.reserved_range();
        frame_paddr >= start && frame_paddr < end
    }
}

/// Coarse memory-region kind, decoupled from the bootloader's enum so
/// the carver stays pure-logic and testable on the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    /// Usable RAM — eligible to back the DMA pool.
    Usable,
    /// Anything else (reserved, bootloader data, MMIO, ACPI, ...).
    Other,
}

/// Find the first usable region with enough headroom to host a
/// page-aligned carved range of `capacity_bytes`. Returns
/// `(carved_start, carved_end)` (half-open, both page-aligned to the
/// extent that `capacity_bytes` is a multiple of `PAGE_SIZE`).
///
/// Called once at boot from `memory::init` *before* the
/// BootInfoFrameAllocator is handed the memory map, so the carved
/// range is excluded from general-purpose allocation.
pub fn carve_dma_region(
    regions: &[(u64, u64, RegionKind)],
    capacity_bytes: u64,
) -> Option<(u64, u64)> {
    let align = PAGE_SIZE as u64;
    for &(start, end, kind) in regions {
        if kind != RegionKind::Usable {
            continue;
        }
        // Round up to the next page boundary. `end - 1` guards against
        // overflow when `start == u64::MAX` (not realistic on x86_64
        // but the check costs nothing).
        let aligned_start = (start + align - 1) & !(align - 1);
        let carved_end = aligned_start.checked_add(capacity_bytes)?;
        if carved_end <= end {
            return Some((aligned_start, carved_end));
        }
    }
    None
}
