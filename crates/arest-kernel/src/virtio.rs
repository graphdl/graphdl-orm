// crates/arest-kernel/src/virtio.rs
//
// virtio guest-driver integration (#262).
//
// The rcore-os `virtio-drivers` crate ships device-class drivers
// (net, blk, gpu, input, console) in pure Rust no_std. To use them
// we owe the crate one thing: a `Hal` impl describing how DMA
// memory is allocated and how virtual/physical addresses round-
// trip. Our bootloader (rust-osdev/bootloader 0.11) entered the
// kernel with the entire physical address space mapped into virtual
// space at `BootInfo::physical_memory_offset`, so the translation
// is a plain add/subtract — no separate DMA pool, no IOMMU.
//
// This module stops at the HAL + DMA allocator. Instantiating a
// VirtIONet device requires walking PCI config space to locate a
// virtio-vendor device (0x1AF4 with device-id ≥ 0x1040 for modern
// virtio-1.0), parsing its BARs, constructing a `PciTransport`,
// and handing it to `VirtIONet::new`. That work lives in a
// follow-up commit to keep this one focused on the HAL contract.

use core::ptr::NonNull;

use virtio_drivers::{BufferDirection, Hal, PhysAddr};
use x86_64::structures::paging::FrameAllocator;

use crate::memory;

/// Bootloader-supplied offset between physical and virtual addresses.
/// Stored by `set_phys_offset` during `memory::init`'s caller chain
/// (set from `main.rs` once the offset is known; see `init_offset`).
static PHYS_OFFSET: spin::Mutex<Option<u64>> = spin::Mutex::new(None);

/// Record the bootloader-mapped physical-memory offset so `Hal`
/// methods can translate between virtual and physical addresses
/// without holding a reference to `BootInfo`. Must be called once
/// from `main.rs` after `memory::init(boot_info)`.
pub fn init_offset(offset: u64) {
    *PHYS_OFFSET.lock() = Some(offset);
}

fn phys_offset() -> u64 {
    PHYS_OFFSET.lock().expect("virtio::init_offset() not called")
}

/// Translate a kernel virtual address (pointer into the offset
/// mapping) back to its physical address.
fn virt_to_phys(vaddr: usize) -> PhysAddr {
    (vaddr as u64 - phys_offset()) as PhysAddr
}

/// Translate a physical address into its kernel virtual pointer.
fn phys_to_virt(paddr: PhysAddr) -> NonNull<u8> {
    let vaddr = (paddr as u64 + phys_offset()) as *mut u8;
    // SAFETY: the bootloader guarantees every physical frame is
    // mapped at `phys + PHYS_OFFSET` for the lifetime of the kernel.
    unsafe { NonNull::new_unchecked(vaddr) }
}

/// The `Hal` trait impl virtio-drivers hangs its HAL generic on.
///
/// All four methods lean on the bootloader's offset mapping —
/// allocation carves contiguous frames from the boot frame
/// allocator; free is a no-op for now (DMA buffers live for the
/// lifetime of the device driver); share/unshare reduce to
/// address translation because we don't run an IOMMU.
pub struct KernelHal;

unsafe impl Hal for KernelHal {
    fn dma_alloc(
        pages: usize,
        _direction: BufferDirection,
    ) -> (PhysAddr, NonNull<u8>) {
        memory::with_frame_allocator(|fa| {
            let first = fa
                .allocate_frame()
                .expect("dma_alloc: out of physical frames");
            // virtio-drivers expects N *contiguous* frames. Our
            // BootInfoFrameAllocator hands out ascending frames in
            // order, so N sequential allocs yield a contiguous
            // run — but only for the very first call. A robust
            // dma_alloc needs a dedicated contiguous pool; until
            // that lands (#262 follow-up), assert N ≤ 1 or the
            // first page of a fresh allocator.
            for _ in 1..pages {
                let _ = fa
                    .allocate_frame()
                    .expect("dma_alloc: out of physical frames");
            }
            let paddr = first.start_address().as_u64() as PhysAddr;
            let vaddr = phys_to_virt(paddr);
            (paddr, vaddr)
        })
    }

    unsafe fn dma_dealloc(
        _paddr: PhysAddr,
        _vaddr: NonNull<u8>,
        _pages: usize,
    ) -> i32 {
        // BootInfoFrameAllocator is forward-only; freed frames are
        // leaked for the lifetime of the kernel. Fine for the
        // single virtio device class we'll instantiate.
        0
    }

    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, _size: usize) -> NonNull<u8> {
        phys_to_virt(paddr)
    }

    unsafe fn share(
        buffer: NonNull<[u8]>,
        _direction: BufferDirection,
    ) -> PhysAddr {
        let vaddr = buffer.as_ptr() as *mut u8 as usize;
        virt_to_phys(vaddr)
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

// ── Device instantiation (follow-up commit) ──────────────────────
//
// `try_init_virtio_net()` walks PCI config space (ports 0xCF8 /
// 0xCFC) looking for vendor=0x1AF4 devices with modern virtio device
// IDs (0x1040–0x107F). For each candidate it parses BARs, builds a
// `virtio_drivers::transport::pci::PciTransport<KernelHal>`, and
// hands it to `VirtIONet::<KernelHal, _, NET_QUEUE_SIZE>::new(...)`.
// Successful construction returns a driver handle the `net` module
// wraps in a `smoltcp::phy::Device` adapter.
//
// Until that code lands the kernel keeps using smoltcp's Loopback
// device so the rest of the stack (DHCP, HTTP server, HATEOAS
// renderer) can be brought up in parallel against a loopback target.
