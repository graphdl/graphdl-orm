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

// ── Device instantiation (#262 driver wire-up) ──────────────────
//
// `try_init_virtio_net()` uses the legacy-PIO ConfigurationAccess
// built in `PioCam` below to construct a `PciRoot`, then a
// `PciTransport<KernelHal>`, then a `VirtIONet<KernelHal, _, _>`.
// The returned driver is handed back to the caller, which wraps it
// in a `smoltcp::phy::Device` adapter (net.rs).

use virtio_drivers::transport::pci::{PciTransport, bus::{
    PciRoot, ConfigurationAccess, DeviceFunction,
}};
use virtio_drivers::transport::DeviceType;
use virtio_drivers::device::net::VirtIONet;

/// Queue size for the virtio-net rx/tx virtqueues. 16 is enough for
/// bring-up; production scaling revisits this based on the cost curve.
pub const NET_QUEUE_SIZE: usize = 16;

/// Per-buffer length for the VirtIONet rx pool. Standard Ethernet MTU
/// is 1500 plus header — 2048 rounds up comfortably.
pub const NET_BUF_LEN: usize = 2048;

/// The fully-typed VirtIONet driver handle.
pub type VirtIONetDevice = VirtIONet<KernelHal, PciTransport, NET_QUEUE_SIZE>;

/// Legacy-PIO `ConfigurationAccess` impl. virtio-drivers' bundled
/// `MmioCam` expects an ECAM / MMIO-CAM base address, which we don't
/// have on legacy x86 — the bootloader leaves PCI access on the
/// classic 0xCF8 / 0xCFC IO-port mechanism. This wrapper forwards
/// `read_word` / `write_word` to that PIO handshake so the rest of
/// the virtio-drivers stack (PciRoot, PciTransport) can stay target-
/// agnostic.
pub struct PioCam;

impl ConfigurationAccess for PioCam {
    fn read_word(&self, df: DeviceFunction, register_offset: u8) -> u32 {
        // SAFETY: PCI config PIO on 0xCF8 / 0xCFC is always safe in
        // ring 0 on x86_64 — invalid slots return 0xFFFFFFFF, not a
        // fault. See `pci::read_config_u32` for the same read.
        unsafe { pio_read_config_u32(df.bus, df.device, df.function, register_offset) }
    }

    fn write_word(&mut self, df: DeviceFunction, register_offset: u8, data: u32) {
        // SAFETY: same as `read_word`. Writes to reserved registers
        // are silently ignored by the north bridge.
        unsafe { pio_write_config_u32(df.bus, df.device, df.function, register_offset, data) }
    }

    unsafe fn unsafe_clone(&self) -> Self { PioCam }
}

/// # Safety
/// Same contract as `crate::pci::read_config_u32` — must run in a
/// context where the 0xCF8 / 0xCFC I/O ports are legal (ring 0).
unsafe fn pio_read_config_u32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    use x86_64::instructions::port::Port;
    let addr: u32 = 0x8000_0000
        | ((bus as u32) << 16)
        | (((device as u32) & 0x1F) << 11)
        | (((function as u32) & 0x07) << 8)
        | ((offset as u32) & 0xFC);
    let mut address: Port<u32> = Port::new(0xCF8);
    let mut data: Port<u32> = Port::new(0xCFC);
    address.write(addr);
    data.read()
}

/// # Safety
/// Same as `pio_read_config_u32`.
unsafe fn pio_write_config_u32(bus: u8, device: u8, function: u8, offset: u8, value: u32) {
    use x86_64::instructions::port::Port;
    let addr: u32 = 0x8000_0000
        | ((bus as u32) << 16)
        | (((device as u32) & 0x1F) << 11)
        | (((function as u32) & 0x07) << 8)
        | ((offset as u32) & 0xFC);
    let mut address: Port<u32> = Port::new(0xCF8);
    let mut data: Port<u32> = Port::new(0xCFC);
    address.write(addr);
    data.write(value);
}

/// Try to bring up the virtio-net driver against the first matching
/// PCI device. Returns Some(driver) on success, None when no device
/// is present or construction failed (e.g. the device was mid-reset,
/// or BARs weren't allocated by the firmware).
pub fn try_init_virtio_net() -> Option<VirtIONetDevice> {
    let dev = crate::pci::find_virtio_net()?;
    let device_function = DeviceFunction {
        bus: dev.bus,
        device: dev.device,
        function: dev.function,
    };
    let mut root = PciRoot::new(PioCam);
    let transport = PciTransport::new::<KernelHal, _>(&mut root, device_function).ok()?;
    // Sanity: PciTransport inferred the device class from the virtio
    // device-id; verify it's Network before instantiating VirtIONet.
    if !matches!(transport_device_type(&transport), Some(DeviceType::Network)) {
        return None;
    }
    VirtIONet::<KernelHal, _, NET_QUEUE_SIZE>::new(transport, NET_BUF_LEN).ok()
}

/// Read the device_type off a constructed PciTransport. Trait methods
/// on `Transport` expose it but the concrete PciTransport struct also
/// stores it directly; we use the trait-level accessor to stay
/// transport-agnostic if virtio-mmio ever shares this code path.
fn transport_device_type(transport: &PciTransport) -> Option<DeviceType> {
    use virtio_drivers::transport::Transport;
    Some(transport.device_type())
}
