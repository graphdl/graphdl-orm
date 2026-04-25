// crates/arest-kernel/src/pci.rs
//
// Legacy x86 PCI configuration-space scanner (#262).
//
// QEMU's virtio-net-pci device appears at some (bus, device, function)
// triple on the emulated PCI bus. The bootloader does not initialise
// the PCI bus for us, so the kernel has to enumerate — which means
// issuing the legacy PIO handshake on I/O ports 0xCF8 (CONFIG_ADDRESS)
// and 0xCFC (CONFIG_DATA).
//
// The 0xCF8 word is a packed `(enable, bus, device, function, offset)`
// tuple; writing it selects the PCI device register that a subsequent
// 0xCFC read/write accesses. This is the same mechanism every pre-PCIe
// x86 OS used before ECAM / MMIO-CAM replaced it; the ECAM path is
// MMIO-based and requires the ACPI MCFG table to locate. Legacy PIO
// works on every QEMU x86_64 configuration out of the box, so we
// prefer it for bring-up.
//
// This module stops at device discovery: `scan_devices()` yields every
// `(bus, dev, func)` that has a real device attached. `find_virtio()`
// filters for the Red Hat / Qumranet virtio vendor (0x1AF4) and
// modern-virtio device-id range (0x1040–0x107F). The virtio-net-pci
// device reports device_id = 0x1041.
//
// Driver instantiation (building a `PciTransport` + `VirtIONet` from
// the discovered BARs) lives in a follow-up; this module is the
// discovery half.

use alloc::vec::Vec;
use x86_64::instructions::port::Port;

/// Red Hat / Qumranet PCI vendor ID. Every virtio device uses it.
pub const VIRTIO_VENDOR: u16 = 0x1AF4;

/// Modern-virtio device-id window (virtio 1.0+, 2014-). Legacy pre-1.0
/// devices used 0x1000–0x103F, which we deliberately skip — virtio-drivers
/// only speaks the modern transport.
pub const VIRTIO_MODERN_DEVICE_LO: u16 = 0x1040;
pub const VIRTIO_MODERN_DEVICE_HI: u16 = 0x107F;

/// Offset within the PCI modern-virtio device-id range that identifies
/// a network device. virtio-net-pci = VIRTIO_MODERN_DEVICE_LO + 1.
pub const VIRTIO_NET_DEVICE_ID: u16 = 0x1041;

/// Modern-virtio block device. Spec: device-type 2 → 0x1040 + 2.
/// Used by `find_virtio_blk` for the Storage-2 bring-up (#335).
pub const VIRTIO_BLK_DEVICE_ID: u16 = 0x1042;

/// Modern-virtio GPU device. Spec: device-type 16 → 0x1040 + 16 = 0x1050.
/// Used by `find_virtio_gpu` for the framebuffer / display bring-up
/// (#370). Some emulators also expose a transitional 0x1010 device-id;
/// we deliberately match only the modern 0x1050 to stay consistent
/// with the rest of this module's modern-only stance.
pub const VIRTIO_GPU_DEVICE_ID: u16 = 0x1050;

/// Modern-virtio input device. Spec: device-type 18 (VIRTIO_ID_INPUT)
/// → 0x1040 + 18 = 0x1052. QEMU's `virtio-keyboard-pci` and
/// `virtio-tablet-pci` both enumerate at this device-id — they're
/// distinguished only at the virtio-input config-space layer
/// (VIRTIO_INPUT_CFG_EV_BITS query reveals EV_KEY-only vs
/// EV_KEY+EV_ABS capability sets). Used by `find_virtio_input_devices`
/// for the linuxkpi virtio-input wire-up (#464). Returns Vec because
/// QEMU typically exposes both keyboard and tablet on the same guest;
/// every other `find_virtio_*` helper above returns a singleton because
/// only one of that device-class is interesting.
pub const VIRTIO_INPUT_DEVICE_ID: u16 = 0x1052;

/// One enumerated PCI device. Fields come straight from the standard
/// PCI Type-0 header; callers that only need the identity ignore the
/// BARs, and driver-instantiating callers read them.
#[derive(Debug, Clone, Copy)]
pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    /// Six 32-bit BARs per the Type-0 header. A value of 0 means the
    /// BAR is unused. MMIO BARs clear the low bit; I/O BARs set it.
    pub bars: [u32; 6],
}

/// Scan every (bus, device, function) on the legacy PCI bus and return
/// every slot that reports a real vendor (`vendor != 0xFFFF`). The
/// result is Vec so callers can iterate / filter without the scanner
/// needing to know what they're looking for.
pub fn scan_devices() -> Vec<PciDevice> {
    let mut devices = Vec::new();
    for bus in 0..=255u16 {
        for device in 0..32u8 {
            for function in 0..8u8 {
                let bus = bus as u8;
                // SAFETY: legacy PCI config-space access is always
                // safe on x86_64 — the worst an invalid slot returns
                // is 0xFFFF'FFFF for the vendor-ID dword.
                let vendor_device = unsafe { read_config_u32(bus, device, function, 0x00) };
                let vendor = (vendor_device & 0xFFFF) as u16;
                if vendor == 0xFFFF {
                    continue;
                }
                let device_id = ((vendor_device >> 16) & 0xFFFF) as u16;

                let mut bars = [0u32; 6];
                for (i, bar) in bars.iter_mut().enumerate() {
                    let offset = 0x10 + (i as u8) * 4;
                    // SAFETY: same as vendor-ID read — PCI config is
                    // PIO-safe on every x86_64 implementation.
                    *bar = unsafe { read_config_u32(bus, device, function, offset) };
                }

                devices.push(PciDevice {
                    bus, device, function, vendor_id: vendor, device_id, bars,
                });

                // Multi-function check: if function 0's header type
                // MSB is clear, the device is single-function and we
                // can skip functions 1..=7 under this device.
                if function == 0 {
                    // SAFETY: same PIO read safety as above.
                    let header = unsafe { read_config_u32(bus, device, 0, 0x0C) };
                    let header_type = ((header >> 16) & 0xFF) as u8;
                    if header_type & 0x80 == 0 {
                        break;
                    }
                }
            }
        }
    }
    devices
}

/// Find the first modern-virtio device of any class, returning its
/// `PciDevice` descriptor.
pub fn find_virtio() -> Option<PciDevice> {
    scan_devices().into_iter().find(|d| {
        d.vendor_id == VIRTIO_VENDOR
            && d.device_id >= VIRTIO_MODERN_DEVICE_LO
            && d.device_id <= VIRTIO_MODERN_DEVICE_HI
    })
}

/// Find a virtio-net-pci device specifically. Returns None when the
/// machine wasn't launched with `-device virtio-net-pci,...` or when
/// QEMU placed the device behind a PCIe bridge we don't walk (this
/// scanner is flat — single host bus only, 256 slots × 8 functions).
pub fn find_virtio_net() -> Option<PciDevice> {
    scan_devices().into_iter().find(|d| {
        d.vendor_id == VIRTIO_VENDOR && d.device_id == VIRTIO_NET_DEVICE_ID
    })
}

/// Find a virtio-blk-pci device — the first matching one if more than
/// one is attached. Used by Storage-2 (#335) to locate the persistence
/// disk. Returns None when the machine wasn't launched with
/// `-device virtio-blk-pci,disable-legacy=on,...`.
pub fn find_virtio_blk() -> Option<PciDevice> {
    scan_devices().into_iter().find(|d| {
        d.vendor_id == VIRTIO_VENDOR && d.device_id == VIRTIO_BLK_DEVICE_ID
    })
}

/// Find a virtio-gpu-pci device — the first matching one if more than
/// one is attached. Used by the framebuffer / display bring-up (#370)
/// to locate the GPU. Returns None when the machine wasn't launched
/// with `-device virtio-gpu-pci`.
pub fn find_virtio_gpu() -> Option<PciDevice> {
    scan_devices().into_iter().find(|d| {
        d.vendor_id == VIRTIO_VENDOR && d.device_id == VIRTIO_GPU_DEVICE_ID
    })
}

/// Find every virtio-input-pci device on the legacy PCI bus. Returns
/// a Vec because QEMU's typical input config exposes both
/// `virtio-keyboard-pci` and `virtio-tablet-pci` simultaneously — both
/// enumerate at vendor 0x1AF4 / device 0x1052, distinguished only by
/// their EV_BITS config-space response. Used by the #464 linuxkpi
/// virtio-input wire-up to feed each discovered slot through the
/// shim's driver-probe path.
///
/// Returns an empty Vec when the machine wasn't launched with any
/// `-device virtio-{keyboard,tablet,mouse}-pci` line. Iteration order
/// matches the legacy PCI bus walk's enumeration order, which is
/// deterministic per (bus, device, function) — and on QEMU that's the
/// order the devices appear on the `-device` command-line. So when
/// QEMU is launched with `-device virtio-keyboard-pci -device
/// virtio-tablet-pci`, the first element is the keyboard slot and the
/// second is the tablet slot. The linuxkpi caller uses this ordering
/// for its `keyboard online (slot N)` / `tablet online (slot N, abs)`
/// banner discrimination on the foundation slice — once the virtio
/// transport is fully wired through the linuxkpi shim (post-#464),
/// the discrimination flips to a real EV_BITS read at probe time.
pub fn find_virtio_input_devices() -> Vec<PciDevice> {
    scan_devices()
        .into_iter()
        .filter(|d| d.vendor_id == VIRTIO_VENDOR && d.device_id == VIRTIO_INPUT_DEVICE_ID)
        .collect()
}

// ── Low-level config access ──────────────────────────────────────

/// # Safety
/// Must only be called from a context where issuing I/O port writes
/// to 0xCF8 and reads from 0xCFC is safe. On x86_64 that's always
/// true in kernel mode; user-mode would page-fault on the insn.
unsafe fn read_config_u32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    // PCI CONFIG_ADDRESS word:
    //   bit 31    — enable (always set when issuing a config transaction)
    //   bits 30:24 — reserved / 0
    //   bits 23:16 — bus
    //   bits 15:11 — device
    //   bits 10:8  — function
    //   bits 7:0   — offset (dword-aligned — low 2 bits ignored)
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
