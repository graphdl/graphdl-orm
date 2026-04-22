// crates/arest-kernel-image/tests/dma_pool.rs
//
// Host-side unit tests for the DMA-pool bump allocator + region-carving
// helpers in crates/arest-kernel/src/dma.rs. The arest-kernel crate is
// bin-only + no_std, so we can't `cargo test` it directly — instead we
// `#[path]`-include the module source here and exercise it on the host
// target. The module is written to be pure-logic (u64 addresses, no
// x86_64-target-specific intrinsics) precisely so this test can share
// its source directly, avoiding the mirror-drift problem that hurts
// the userbuf_validation.rs pattern.
//
// The tests cover the three invariants virtio-drivers depends on:
//
//   1. `alloc` hands back page-aligned, physically-contiguous ranges
//      across multiple calls (needed because virtqueue setup and rx/tx
//      pools are separate `dma_alloc` calls — they must land in one run).
//   2. `alloc` returns None on exhaustion (so callers see a clean error
//      instead of colliding with a neighbouring region).
//   3. `carve_dma_region` selects a usable region big enough *and*
//      aligns the start to 4 KiB. Frames inside the carved range must
//      be hidden from the BootInfoFrameAllocator so the two allocators
//      never hand back the same physical frame.
//
// These are the three properties the #268 handoff calls out as the
// root cause of `VirtIONet::new -> DmaError` under the forward-only
// boot frame allocator.

#[path = "../../arest-kernel/src/dma.rs"]
mod dma;

use dma::{DmaPool, RegionKind, carve_dma_region, PAGE_SIZE};

// ── DmaPool tests ───────────────────────────────────────────────────

#[test]
fn alloc_returns_contiguous_pages_across_calls() {
    // Exactly the virtio-drivers scenario: several `dma_alloc` calls
    // for virtqueues + rx/tx pools that must land adjacent in physical
    // memory. If the bump allocator drifts between calls, smoltcp's
    // virtqueue setup sees corrupt descriptor rings.
    let mut pool = DmaPool::new(0x100_0000, 512, 0);
    let (p1, _) = pool.alloc(2).expect("first alloc");
    let (p2, _) = pool.alloc(3).expect("second alloc");
    let (p3, _) = pool.alloc(1).expect("third alloc");
    assert_eq!(p1, 0x100_0000);
    assert_eq!(p2, p1 + 2 * PAGE_SIZE as u64);
    assert_eq!(p3, p2 + 3 * PAGE_SIZE as u64);
}

#[test]
fn alloc_returns_page_aligned_addresses() {
    let mut pool = DmaPool::new(0x200_0000, 8, 0);
    for _ in 0..4 {
        let (paddr, _) = pool.alloc(1).expect("alloc");
        assert_eq!(paddr & (PAGE_SIZE as u64 - 1), 0);
    }
}

#[test]
fn alloc_none_when_exhausted() {
    let mut pool = DmaPool::new(0x300_0000, 3, 0);
    assert!(pool.alloc(2).is_some());
    assert!(pool.alloc(1).is_some());
    // Pool is now exactly full — any further alloc must return None.
    assert!(pool.alloc(1).is_none());
}

#[test]
fn alloc_none_when_request_exceeds_remaining() {
    let mut pool = DmaPool::new(0x400_0000, 4, 0);
    assert!(pool.alloc(3).is_some());
    // 1 page remains; a 2-page request cannot be satisfied.
    assert!(pool.alloc(2).is_none());
    // But a 1-page request still succeeds.
    assert!(pool.alloc(1).is_some());
}

#[test]
fn alloc_zero_pages_is_none() {
    // A zero-page DMA allocation is nonsensical — return None so
    // callers surface the bug instead of silently reusing the cursor.
    let mut pool = DmaPool::new(0x500_0000, 4, 0);
    assert!(pool.alloc(0).is_none());
}

#[test]
fn alloc_vaddr_is_paddr_plus_phys_offset() {
    // The bootloader offset mapping stacks the entire physical address
    // space at `phys_offset`. Every DMA page's virtual address must
    // equal its physical address plus that offset, otherwise the HAL's
    // `share`/`unshare` identity translations desync.
    let offset: u64 = 0xFFFF_8000_0000_0000;
    let mut pool = DmaPool::new(0x600_0000, 4, offset);
    let (paddr, vaddr) = pool.alloc(2).expect("alloc");
    assert_eq!(vaddr, paddr + offset);
}

#[test]
fn contains_frame_true_for_frames_inside_pool() {
    let pool = DmaPool::new(0x700_0000, 4, 0);
    assert!(pool.contains_frame(0x700_0000));
    assert!(pool.contains_frame(0x700_0000 + 3 * PAGE_SIZE as u64));
    // The last byte of the last frame is still inside.
    assert!(pool.contains_frame(0x700_0000 + 4 * PAGE_SIZE as u64 - 1));
}

#[test]
fn contains_frame_false_for_frames_outside_pool() {
    let pool = DmaPool::new(0x800_0000, 4, 0);
    // One byte below the pool.
    assert!(!pool.contains_frame(0x800_0000 - 1));
    // The first byte *after* the pool is not included.
    assert!(!pool.contains_frame(0x800_0000 + 4 * PAGE_SIZE as u64));
    // A frame far away.
    assert!(!pool.contains_frame(0x900_0000));
}

#[test]
fn reserved_range_spans_full_capacity() {
    let pool = DmaPool::new(0xA00_0000, 512, 0);
    let (start, end) = pool.reserved_range();
    assert_eq!(start, 0xA00_0000);
    assert_eq!(end, 0xA00_0000 + 512 * PAGE_SIZE as u64);
}

// ── Carving tests ───────────────────────────────────────────────────

#[test]
fn carve_picks_first_usable_region_with_headroom() {
    // First usable region is 1 MiB — too small for 2 MiB request.
    // Second region is "Other" kind — skipped.
    // Third usable region is 8 MiB — fits.
    let regions = [
        (0x0000_0000, 0x0010_0000, RegionKind::Usable),
        (0x0010_0000, 0x0020_0000, RegionKind::Other),
        (0x0020_0000, 0x00A0_0000, RegionKind::Usable),
    ];
    let (start, end) = carve_dma_region(&regions, 2 * 1024 * 1024).expect("carve");
    assert_eq!(start, 0x0020_0000);
    assert_eq!(end, 0x0020_0000 + 2 * 1024 * 1024);
}

#[test]
fn carve_aligns_start_up_to_page() {
    // Region begins at an unaligned address; carved region must start
    // on the next 4 KiB boundary.
    let regions = [
        (0x0100_0123, 0x0200_0000, RegionKind::Usable),
    ];
    let (start, _) = carve_dma_region(&regions, 1024 * 1024).expect("carve");
    assert_eq!(start & (PAGE_SIZE as u64 - 1), 0);
    assert!(start >= 0x0100_0123);
    assert!(start < 0x0100_0123 + PAGE_SIZE as u64);
}

#[test]
fn carve_skips_non_usable_regions() {
    // Only region is Other (bootloader-reserved / MMIO / whatever);
    // the carver must not touch it even if it would fit.
    let regions = [
        (0x0000_0000, 0x1000_0000, RegionKind::Other),
    ];
    assert!(carve_dma_region(&regions, 1024 * 1024).is_none());
}

#[test]
fn carve_returns_none_when_no_region_large_enough() {
    let regions = [
        (0x0000_0000, 0x0000_8000, RegionKind::Usable),
        (0x0000_8000, 0x0001_0000, RegionKind::Usable),
    ];
    assert!(carve_dma_region(&regions, 1024 * 1024).is_none());
}

#[test]
fn carve_accounts_for_alignment_slack_in_fit_check() {
    // Region has exactly 2 MiB of raw bytes but starts one byte past
    // a page boundary. After aligning up by PAGE_SIZE-1 bytes the
    // 2 MiB carve no longer fits — the carver must reject it.
    let start = PAGE_SIZE as u64 + 1;
    let size = 2 * 1024 * 1024;
    let end = start + size;
    let regions = [(start, end, RegionKind::Usable)];
    assert!(carve_dma_region(&regions, size).is_none());
}
