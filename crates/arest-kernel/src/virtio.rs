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

use crate::{dma, memory};

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

/// Translate a kernel virtual address back to its physical address
/// by walking the active page table.
///
/// `share()` can be called with buffers that live *outside* the
/// bootloader's offset-mapped physical-memory window — in particular,
/// rx/tx buffers are `alloc::vec::Vec`s backed by `HEAP_STORAGE` in the
/// kernel's BSS segment, which the bootloader maps at the kernel's
/// linked virtual addresses, not at `phys + phys_offset`. Subtracting
/// `phys_offset` from those vaddrs yields garbage (observed pre-fix:
/// every rx descriptor pointed to a bogus low-RAM paddr, so QEMU's DMA
/// went nowhere and the guest saw zero packets — #268).
///
/// Page-walking handles both cases uniformly: offset-mapped DMA pages
/// resolve the same way kernel ELF-mapped BSS resolves. The buffer is
/// required to be physically contiguous; virtio descriptors describe
/// a single `(addr, len)` range. Our heap is a contiguous static BSS
/// segment so any `Vec` inside it is physically contiguous, and the
/// DMA pool is contiguous by construction.
fn virt_to_phys(vaddr: usize) -> PhysAddr {
    use x86_64::structures::paging::mapper::Translate;
    use x86_64::VirtAddr;
    let virt = VirtAddr::new(vaddr as u64);
    memory::with_page_table(|pt| {
        pt.translate_addr(virt)
            .expect("virt_to_phys: kernel virtual address is unmapped")
            .as_u64() as PhysAddr
    })
}

/// Translate a physical address into its kernel virtual pointer via
/// the bootloader's offset mapping (not through a page-table walk —
/// the offset mapping is an identity translation by construction).
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
        // #268: virtio-drivers issues several `dma_alloc` calls during
        // `VirtIONet::new` (one per virtqueue + one per rx/tx buffer
        // pool). Each returned range must be page-aligned and physically
        // contiguous; the descriptor rings treat the stored addresses
        // as raw pointers into their own buffers. We satisfy that by
        // bump-allocating out of a dedicated pool carved at `memory::
        // init` time (see `crates/arest-kernel/src/dma.rs`), so every
        // call lands in one contiguous physical window and never
        // collides with the BootInfoFrameAllocator.
        let (paddr_u64, vaddr_u64) = memory::with_dma_pool(|pool| {
            pool.alloc(pages)
                .expect("dma_alloc: DMA pool exhausted (bump DMA_POOL_PAGES)")
        })
        .expect("dma_alloc: DMA pool not carved (no usable region big enough)");

        let paddr = paddr_u64 as PhysAddr;
        // SAFETY: the bootloader guarantees that every physical frame
        // inside the DMA carve-out is mapped at `paddr + phys_offset`
        // for the lifetime of the kernel, and the carver ensures the
        // range is non-zero, page-aligned, and usable RAM.
        let vaddr = unsafe { NonNull::new_unchecked(vaddr_u64 as *mut u8) };

        // Per the Hal trait contract (hal.rs:90 in virtio-drivers), the
        // returned pages must be zeroed. virtio-drivers relies on this
        // for the virtqueue's used-ring `idx` to read as zero at setup
        // — otherwise `can_pop()` returns true on the first poll and
        // `peek_used` reads a garbage descriptor id out of the stale
        // memory (observed as token=60547 pre-fix, #268). BIOS-backed
        // QEMU happens to boot with zero RAM, but relying on that is
        // fragile; explicit zeroing makes the contract hold regardless
        // of what was in the carved region before boot.
        //
        // SAFETY: `vaddr` was just handed back by the pool, so no other
        // pointer aliases the region; the region is `pages * PAGE_SIZE`
        // bytes of kernel-owned writable memory.
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
        // The DmaPool cursor is one-way; freed pages are leaked for the
        // lifetime of the kernel. Fine for the single virtio device
        // class the kernel currently instantiates — the pool is sized
        // (`DMA_POOL_PAGES` in memory.rs) to cover the device's full
        // lifetime footprint.
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
use virtio_drivers::device::blk::VirtIOBlk;

/// Queue size for the virtio-net rx/tx virtqueues. 16 is enough for
/// bring-up; production scaling revisits this based on the cost curve.
pub const NET_QUEUE_SIZE: usize = 16;

/// Per-buffer length for the VirtIONet rx pool. Standard Ethernet MTU
/// is 1500 plus header — 2048 rounds up comfortably.
pub const NET_BUF_LEN: usize = 2048;

/// The fully-typed VirtIONet driver handle.
pub type VirtIONetDevice = VirtIONet<KernelHal, PciTransport, NET_QUEUE_SIZE>;

/// The fully-typed VirtIOBlk driver handle (#335). virtio-drivers' blk
/// module hard-codes its virtqueue size to 16 and does not expose it as
/// a const generic, so nothing to tune here.
pub type VirtIOBlkDevice = VirtIOBlk<KernelHal, PciTransport>;

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
    let transport = match PciTransport::new::<KernelHal, _>(&mut root, device_function) {
        Ok(t) => t,
        Err(e) => {
            crate::println!("  virtio: PciTransport::new failed: {:?}", e);
            return None;
        }
    };
    if !matches!(transport_device_type(&transport), Some(DeviceType::Network)) {
        crate::println!("  virtio: transport device_type is not Network");
        return None;
    }
    // `VirtIONet::new` currently returns DmaError against the forward-
    // only BootInfoFrameAllocator: virtio-drivers does several
    // `dma_alloc` calls for virtqueues + rx/tx pools, and our HAL
    // doesn't yet hand back guaranteed-contiguous multi-page regions
    // across call boundaries. Follow-up (#268 kernel side) needs a
    // dedicated contiguous DMA pool carved at boot.
    match VirtIONet::<KernelHal, _, NET_QUEUE_SIZE>::new(transport, NET_BUF_LEN) {
        Ok(d) => Some(d),
        Err(e) => {
            crate::println!("  virtio: VirtIONet::new failed: {:?}", e);
            None
        }
    }
}

/// Read the device_type off a constructed PciTransport. Trait methods
/// on `Transport` expose it but the concrete PciTransport struct also
/// stores it directly; we use the trait-level accessor to stay
/// transport-agnostic if virtio-mmio ever shares this code path.
fn transport_device_type(transport: &PciTransport) -> Option<DeviceType> {
    use virtio_drivers::transport::Transport;
    Some(transport.device_type())
}

/// Bring up the virtio-blk driver against the first matching PCI
/// device (#335). Returns `Some(driver)` on success, `None` when no
/// virtio-blk is present or construction failed. Shares the
/// `KernelHal` + `PioCam` infrastructure with `try_init_virtio_net`;
/// the two drivers co-exist by virtue of the DMA pool being sized for
/// both (`DMA_POOL_PAGES` in memory.rs).
pub fn try_init_virtio_blk() -> Option<VirtIOBlkDevice> {
    let dev = crate::pci::find_virtio_blk()?;
    let device_function = DeviceFunction {
        bus: dev.bus,
        device: dev.device,
        function: dev.function,
    };
    let mut root = PciRoot::new(PioCam);
    let transport = match PciTransport::new::<KernelHal, _>(&mut root, device_function) {
        Ok(t) => t,
        Err(e) => {
            crate::println!("  virtio-blk: PciTransport::new failed: {:?}", e);
            return None;
        }
    };
    if !matches!(transport_device_type(&transport), Some(DeviceType::Block)) {
        crate::println!("  virtio-blk: transport device_type is not Block");
        return None;
    }
    match VirtIOBlk::<KernelHal, _>::new(transport) {
        Ok(d) => Some(d),
        Err(e) => {
            crate::println!("  virtio-blk: VirtIOBlk::new failed: {:?}", e);
            None
        }
    }
}

// ── smoltcp::phy::Device adapter (#262 final mile) ──────────────
//
// smoltcp drives its TCP/IP stack through a `phy::Device` trait
// whose contract is rx/tx with borrowed tokens. VirtIONet speaks
// a different contract — Result-returning `receive() / send()`
// with owned RxBuffer / TxBuffer — so we bridge.
//
// Borrow checker wrinkle: `Device::receive` hands back BOTH an
// RxToken and a TxToken from one `&mut self` call. Both need to
// touch the NIC when consumed. Rust's aliasing rules forbid two
// `&mut VirtIONetDevice` from one borrow, so the tokens carry raw
// pointers back into the driver instead. This is sound because:
//
//   1. smoltcp consumes the tokens strictly sequentially within
//      one `poll` tick — RxToken.consume runs, finishes, then
//      TxToken.consume runs or is dropped. No aliasing in time.
//   2. The kernel is single-threaded; no other CPU is reading
//      through the pointer concurrently.
//   3. The token lifetime `'a` is tied to `&mut self`, so Rust
//      enforces that the pointer never outlives the borrow.
//
// RxBuffer recycling: VirtIONet allocates a pool of rx buffers at
// `new()` time. After smoltcp reads the packet we must hand the
// buffer back via `recycle_rx_buffer` or the pool starves.

use smoltcp::phy::{self, Checksum, DeviceCapabilities, Medium};
use smoltcp::time::Instant;
use virtio_drivers::device::net::{RxBuffer, TxBuffer};

/// smoltcp Device wrapping a VirtIONet driver.
pub struct VirtioPhy {
    nic: VirtIONetDevice,
}

impl VirtioPhy {
    /// Take ownership of the VirtIONet driver so smoltcp can
    /// drive it. The NIC lives for the lifetime of the phy.
    pub fn new(nic: VirtIONetDevice) -> Self {
        Self { nic }
    }

    /// Expose the NIC's MAC address so the interface can set
    /// itself up with a real hardware address instead of the
    /// loopback placeholder.
    pub fn mac_address(&self) -> smoltcp::wire::EthernetAddress {
        smoltcp::wire::EthernetAddress(self.nic.mac_address())
    }
}

pub struct VirtioRxToken<'a> {
    nic: *mut VirtIONetDevice,
    buf: Option<RxBuffer>,
    _marker: core::marker::PhantomData<&'a mut VirtIONetDevice>,
}

pub struct VirtioTxToken<'a> {
    nic: *mut VirtIONetDevice,
    _marker: core::marker::PhantomData<&'a mut VirtIONetDevice>,
}

impl<'a> phy::RxToken for VirtioRxToken<'a> {
    fn consume<R, F>(mut self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        let buf = self.buf.take().expect("rx token consumed twice");
        let result = f(buf.packet());
        // SAFETY: justified in the module-level comment — single
        // writer, sequential token consumption, `'a` scoped lifetime.
        let nic = unsafe { &mut *self.nic };
        // A failure here means the pool lost the buffer index — the
        // kernel's next receive() will just allocate fresh.
        let _ = nic.recycle_rx_buffer(buf);
        result
    }
}

impl<'a> phy::TxToken for VirtioTxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        // SAFETY: as in RxToken.consume — the pointer is live
        // through `'a` and no other borrower is active.
        let nic = unsafe { &mut *self.nic };
        let mut tx_buf: TxBuffer = nic.new_tx_buffer(len);
        let result = f(tx_buf.packet_mut());
        // If send fails the packet just gets dropped — smoltcp
        // retransmits via its TCP stack; there's no in-band way
        // to report a NIC error back through TxToken.
        let _ = nic.send(tx_buf);
        result
    }
}

impl phy::Device for VirtioPhy {
    type RxToken<'a> = VirtioRxToken<'a>;
    type TxToken<'a> = VirtioTxToken<'a>;

    fn receive(&mut self, _ts: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if !self.nic.can_recv() {
            return None;
        }
        let buf = self.nic.receive().ok()?;
        let nic_ptr: *mut VirtIONetDevice = &mut self.nic;
        let rx = VirtioRxToken {
            nic: nic_ptr,
            buf: Some(buf),
            _marker: core::marker::PhantomData,
        };
        let tx = VirtioTxToken {
            nic: nic_ptr,
            _marker: core::marker::PhantomData,
        };
        Some((rx, tx))
    }

    fn transmit(&mut self, _ts: Instant) -> Option<Self::TxToken<'_>> {
        if !self.nic.can_send() {
            return None;
        }
        let nic_ptr: *mut VirtIONetDevice = &mut self.nic;
        Some(VirtioTxToken {
            nic: nic_ptr,
            _marker: core::marker::PhantomData,
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        // Standard Ethernet MTU — 14-byte header + 1500-byte payload.
        // The NIC pool is 2048 bytes per buffer (`NET_BUF_LEN`), so
        // we've got comfortable headroom even for slightly larger
        // jumbo frames if the host ever negotiates them.
        caps.max_transmission_unit = 1514;
        caps.medium = Medium::Ethernet;
        // No checksum offload — virtio-drivers doesn't advertise it
        // at this API level, so we let smoltcp compute every sum.
        caps.checksum.ipv4 = Checksum::Both;
        caps.checksum.tcp = Checksum::Both;
        caps.checksum.udp = Checksum::Both;
        caps
    }
}
