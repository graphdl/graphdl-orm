// crates/arest-kernel/src/usb_uart.rs
//
// USB-UART device-side serial (#392) — research deliverable + module
// scaffold. NOT a working driver. Stubs return `unimplemented!()` /
// `Unsupported` so nothing accidentally calls the IO path before the
// real bring-up commits land. Compiles, exposes the public surface
// (`init`, `write_bytes`, `read_byte`), and leaves the body for the
// follow-up sub-chain to fill in piece by piece.
//
// ── Why this module exists ───────────────────────────────────────────
//
// Issue #393 wires the AREST kernel to boot on a real Nexus 5X / 6P
// (aarch64) and Nexus 5 (armv7) — the same fastboot boot.img path
// that #390 / #391 already exercises in QEMU. On real hardware there
// is no PL011 pin breakout and no `-serial stdio` to watch the kernel
// banner with, so the only practical out-of-band log channel is the
// phone's USB port: when the SoC's USB controller is configured as a
// CDC-ACM (Communications Device Class — Abstract Control Model)
// USB *gadget*, the host PC enumerates the phone as `/dev/ttyACM0`
// (Linux) / `COMn` (Windows) and reads bytes off it the same way it
// would read off a USB-serial dongle.
//
// "Gadget" is the Linux-kernel-ism for "device side of USB" — the
// Nexus is the *peripheral*, the host PC is the *host*. This is the
// opposite direction from the `usbd-serial` crate's typical no_std
// embedded-Rust use case (where the MCU presents AS a serial device
// to a developer's PC), and IS what we want — we are not consuming
// a USB serial dongle plugged into the phone, we ARE the serial
// dongle the host PC sees.
//
// The point of this commit is to land the *interface* the kernel
// banner code will eventually call (`println!`-shaped writes that
// route to `/dev/ttyACM0` on the host) without committing to any
// particular USB stack implementation yet — that's research-heavy
// enough to deserve its own sub-chain. Subsequent commits land each
// of the three layers below in turn.
//
// ── Hardware: which USB controller is on the phone? ─────────────────
//
// Nexus 5 (hammerhead, MSM8974, armv7):
//   * Qualcomm MSM-USB ("ci13xxx" / "msm-otg" in the downstream
//     kernel). ChipIdea-derived USB 2.0 OTG controller, dual-role.
//     MMIO-mapped, IRQ-driven. Quirky enough that mainline Linux
//     carries a Qualcomm-specific glue driver
//     (drivers/usb/chipidea/ci_hdrc_msm.c).
//
// Nexus 5X (bullhead, MSM8992, aarch64) and Nexus 6P (angler,
// MSM8994, aarch64):
//   * Synopsys DesignWare USB 3.0 (dwc3). MMIO-mapped, IRQ-driven,
//     supports dual-role (host + peripheral). Mainline Linux drives
//     this with drivers/usb/dwc3/dwc3-qcom.c. Exposes a generic
//     dwc3 register interface that is also used by Intel, NXP, and
//     Rockchip SoCs — so a single dwc3 driver can light up many
//     boards.
//
// Both controllers are fundamentally MMIO + IRQ. Neither is on a PCI
// bus. Both have a Qualcomm-specific PHY block in front (HSIC / QMP
// PHY) that needs vendor-supplied init writes before USB enumeration
// will succeed. That PHY init is the chunk most likely to need
// device-tree-derived register addresses; the dwc3 / chipidea core
// itself is more or less a standard part.
//
// ── Software: what does the Rust no_std USB ecosystem look like? ────
//
// The Rust embedded HAL (rust-embedded WG) ships a layered USB stack
// designed for Cortex-M MCUs but written in `no_std` and largely
// platform-neutral above the controller-driver seam:
//
//   * `usb-device` (https://crates.io/crates/usb-device) — the spine.
//     Provides `UsbBus` (controller HAL trait), `UsbDevice` (the
//     device-side state machine: SET_ADDRESS, GET_DESCRIPTOR,
//     SET_CONFIGURATION, etc.), `UsbBusAllocator`, and the
//     `UsbClass` trait that class implementations register through.
//     Pure no_std. Tier-1 in the embedded-Rust ecosystem (~3M
//     downloads). The `UsbBus` trait is what a controller driver
//     for dwc3 / msm-otg would implement.
//
//   * `usbd-serial` (https://crates.io/crates/usbd-serial) — the
//     CDC-ACM class implementation we want. Plugs into `usb-device`
//     as a `UsbClass` and exposes a byte-stream API
//     (`SerialPort::write` / `SerialPort::read`) that maps onto the
//     CDC-ACM bulk-IN / bulk-OUT endpoints. This is the layer that
//     a host PC's USB-serial driver enumerates as `/dev/ttyACM0`.
//
//   * Per-controller HAL crates implement `UsbBus` for specific
//     silicon. Examples that ship today:
//       - `synopsys-usb-otg` — covers STM32's Synopsys-derived OTG
//         block. SAME core IP family as Qualcomm's dwc3 (both
//         Synopsys DesignWare USB 3.0), but the register layout is
//         off-spec by the time both vendors have wrapped it. Worth
//         studying as a structural reference but not directly
//         drop-in for Qualcomm's flavor.
//       - `stm32-usbd`, `atsamd-usb`, `rp2040-hal::usb` — other
//         vendor stacks that implement the same `UsbBus` trait
//         against their respective controllers. Each is ~2-3 KLOC
//         of MMIO + IRQ glue plus per-endpoint FIFO management.
//
//   * `embassy-usb` (https://crates.io/crates/embassy-usb) — the
//     async-first sibling stack. Same overall layering (controller
//     trait + class implementations + host-visible state machine)
//     but built around `async fn` and the embassy executor. Better
//     fit if/when the AREST kernel grows an async runtime; not a
//     fit today since the kernel banner path is purely synchronous.
//
// Assessment: `usb-device` + `usbd-serial` is the right top of the
// stack for AREST. The work that *cannot* be reused from the existing
// embedded-Rust ecosystem is a `UsbBus` impl for Qualcomm's dwc3
// (Nexus 5X / 6P) and another for the chipidea/msm-otg controller
// (Nexus 5). Neither exists today as a standalone crate. The closest
// reference implementations are:
//
//   * Linux's drivers/usb/dwc3/* (GPLv2 — readable as a register-
//     layout reference but not directly portable to the AREST tree).
//   * The Synopsys USB 3.0 Programming Guide (publicly available
//     PDF) — the authoritative register reference.
//   * `synopsys-usb-otg` (MIT/Apache-2.0) for structural patterns
//     (how to lay out `UsbBus`-impl FIFO accounting, IRQ handling,
//     endpoint state).
//
// Plan: pull `usb-device` + `usbd-serial` as deps in the next commit
// (#392 follow-up A), hand-roll a `UsbBus` impl against the dwc3
// MMIO register set in commit B, wire IRQ routing through the GIC
// in commit C, and drop the `unimplemented!()` stubs in this file
// in favor of real `SerialPort::write` calls in commit D.
//
// ── Why CDC-ACM specifically? ───────────────────────────────────────
//
// CDC-ACM is the USB device class that emulates a serial port over
// USB. Two endpoints (bulk-IN, bulk-OUT) for the data stream, one
// optional interrupt-IN endpoint for line-state notifications. The
// host-side kernel (Linux, Windows, macOS) ships a built-in driver
// (`cdc-acm.ko` on Linux, `usbser.sys` on Windows ≥10) that binds
// to any device advertising USB class 0x02 + subclass 0x02 + protocol
// 0x01. No host-side driver install needed. This is the same class
// used by Arduino, every microcontroller dev board with a built-in
// USB-serial bridge, and Android's "kernel debug serial" path.
//
// Alternatives we are NOT pursuing in this commit:
//
//   * USB Mass Storage / "fastboot" — the Android bootloader uses
//     a custom protocol over bulk endpoints; the kernel banner
//     doesn't need full fastboot semantics, just bytes-out.
//   * USB DbC (Debug Capability) — newer (USB 3.0+), spec'd for
//     exactly this use case but requires xHCI host-side and a host
//     stack that knows DbC. Linux ≥4.16 supports it; the dev-laptop
//     story is still patchy enough that CDC-ACM is the safer first
//     bet.
//   * Custom vendor class — would force an INF / udev install on
//     every dev machine. Defeats the point.
//   * RNDIS (USB ethernet) — far heavier than what we need; would
//     give us a network interface to the phone but with the cost of
//     a TCP server on the device side.
//
// CDC-ACM is the right granularity: bytes in, bytes out, host PC
// sees `/dev/ttyACM0`. Same shape the existing `print!` / `println!`
// macros expect from `arch::_print`.
//
// ── Dependency chain (top of stack to bottom) ───────────────────────
//
//   1. KERNEL CONSUMER (this module's public surface):
//        usb_uart::write_bytes(&[u8])
//        usb_uart::read_byte() -> Option<u8>
//
//   2. CDC-ACM CLASS IMPLEMENTATION:
//        usbd-serial::SerialPort wrapping the bulk endpoints.
//        Sees byte streams; emits USB control + bulk transfers.
//
//   3. USB DEVICE-MODE FRAMEWORK:
//        usb-device::UsbDevice + UsbBusAllocator. Owns the device
//        descriptor, configuration descriptor, control-pipe state
//        machine, endpoint allocation. Calls down into UsbBus for
//        actual MMIO transactions.
//
//   4. CONTROLLER DRIVER (UsbBus impl — does NOT exist yet for
//      Qualcomm silicon):
//        Implements UsbBus for the specific controller. Owns the
//        MMIO base, the FIFO accounting, the IRQ handler. Translates
//        `endpoint_write_packet` calls into the dwc3 / msm-otg
//        register sequences that actually push bytes onto the wire.
//
//   5. SoC PLATFORM GLUE:
//        PHY init writes (Qualcomm-specific), clock enable, IRQ
//        routing through the GIC, MMIO region mapping. This is the
//        "you must run these vendor-supplied magic numbers before
//        USB will work" layer. Largely lifted from the downstream
//        Nexus kernel sources (msm-3.4, msm-3.10) which are GPLv2
//        and publicly available on AOSP.
//
//   6. CPU INTERRUPT INFRASTRUCTURE:
//        GICv2 driver (aarch64) / GIC distributor + CPU interface
//        (armv7). The aarch64 arm of the kernel does not have a GIC
//        driver yet — that lands as part of the IRQ-driven subsystems
//        groundwork (referenced in arch::aarch64::mod.rs's deferred
//        list).
//
// Each layer is independently testable. The follow-up sub-chain
// breaks the work down accordingly:
//
//   #392-A  Pull usb-device + usbd-serial deps; replace these stubs
//           with calls into a *fake* UsbBus that just buffers bytes
//           (so we can unit-test the CDC-ACM endpoint state machine
//           on the host).
//   #392-B  dwc3 UsbBus impl against MMIO. Aarch64 only (Nexus 5X
//           and 6P).
//   #392-C  GICv2 driver + IRQ wire-up. Unblocks the dwc3 IRQ
//           handler.
//   #392-D  Banner re-route: arch::_print calls into usb_uart
//           alongside PL011, so the kernel banner reaches both QEMU
//           `-serial stdio` and the host PC's `/dev/ttyACM0`.
//   #392-E  msm-otg UsbBus impl for Nexus 5 (armv7). Last in the
//           chain because the chipidea controller is more vendor-
//           quirky than dwc3 and AREST already has aarch64 hardware
//           working before then.
//
// ── Cfg-gating ──────────────────────────────────────────────────────
//
// Gated on `target_arch = "aarch64"` AND `target_arch = "arm"`. The
// aarch64 gate covers Nexus 5X / 6P; the armv7 gate covers Nexus 5.
// The x86_64 build (BIOS + UEFI) elides the module entirely — there
// is no x86 USB-gadget use case for AREST today. The `arch = "arm"`
// branch ALSO elides the actual stub bodies for now, since the
// chipidea driver (#392-E) lands last and there is no point exposing
// `unimplemented!()` panics on a target where no caller will reach
// them this commit chain.
//
// ── Public surface (this commit) ────────────────────────────────────
//
//   pub fn init() -> Result<(), Error>
//       Bring up the controller, allocate endpoints, present
//       descriptor table to host. Today: returns `Unsupported`.
//   pub fn write_bytes(buf: &[u8])
//       Push bytes to the host via the bulk-IN endpoint. Today:
//       `unimplemented!()`.
//   pub fn read_byte() -> Option<u8>
//       Pull one byte from the bulk-OUT endpoint, non-blocking.
//       Today: `unimplemented!()`.
//
// No callers reach these functions in this commit. The aarch64 and
// armv7 entry harnesses (`entry_uefi_aarch64.rs`, `entry_uefi_armv7.rs`)
// continue to print exclusively via PL011 MMIO, which works under
// QEMU. Real-hardware Nexus boot (#393) is what eventually wires
// the kernel banner through here.
//
// ── Reminder: this is a SCAFFOLD, NOT a working driver ──────────────
//
// Nothing in this file talks to real USB hardware. The research
// summary above is the deliverable; the function bodies are
// placeholders. The next commit in this sub-chain (#392-A) is what
// adds the first dep and starts the actual implementation work.

#![cfg(any(target_arch = "aarch64", target_arch = "arm"))]

use core::fmt;

/// Errors the USB-UART subsystem can surface to callers. Kept small
/// and additive — the driver-implementation commits will widen this
/// (probably with `BusFault`, `EndpointStall`, `HostNotConnected`,
/// etc.) once the real `UsbBus` impl lands. For now `Unsupported` is
/// the only variant `init` ever returns, and it never reaches a
/// caller because nothing invokes `init` yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The USB-UART subsystem has not been brought up on this target
    /// yet. Returned by `init` until the Qualcomm dwc3 / chipidea
    /// driver work in the #392 sub-chain lands.
    Unsupported,
    /// Reserved for the controller-driver follow-ups. Listed here so
    /// the enum is `non_exhaustive`-shaped from day one and adding
    /// variants in later commits is purely additive.
    #[allow(dead_code)]
    BusFault,
    /// Reserved (see `BusFault`).
    #[allow(dead_code)]
    HostNotConnected,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported       => f.write_str("usb_uart: subsystem not yet implemented (#392 scaffold)"),
            Error::BusFault          => f.write_str("usb_uart: USB bus fault"),
            Error::HostNotConnected  => f.write_str("usb_uart: no host PC enumerated"),
        }
    }
}

/// Bring the USB controller up as a CDC-ACM gadget. On success the
/// host PC enumerates the phone as `/dev/ttyACM0` (Linux) / a `COMn`
/// port (Windows) and the kernel can start streaming banner bytes
/// through `write_bytes`.
///
/// Today this is a stub that always returns `Err(Error::Unsupported)`.
/// The real bring-up sequence (per the research above) is:
///
///   1. Map the controller MMIO region (dwc3 base for Nexus 5X/6P,
///      ChipIdea base for Nexus 5).
///   2. Run the Qualcomm-specific PHY init writes.
///   3. Allocate a `UsbBusAllocator` from the controller-driver crate
///      (#392-B / #392-E).
///   4. Construct a `usbd_serial::SerialPort` against that bus.
///   5. Construct a `usb_device::UsbDevice` with a CDC-ACM class
///      descriptor (vendor 0x18d1 = Google, product TBD).
///   6. Enable the controller IRQ via the GIC.
///   7. Stash the `UsbDevice` + `SerialPort` in module statics so
///      `write_bytes` / `read_byte` can reach them.
///
/// Subsequent commits in the #392 sub-chain replace the stub body
/// step-by-step. The signature is stable.
pub fn init() -> Result<(), Error> {
    // Scaffold: no controller driver yet. See module docstring for
    // the implementation roadmap.
    Err(Error::Unsupported)
}

/// Push a byte slice to the host PC over the CDC-ACM bulk-IN
/// endpoint. Blocks (busy-waits) until the controller's TX FIFO
/// accepts every byte; on a real implementation this will eventually
/// gain an async / non-blocking variant.
///
/// Today this is a stub. Calling it before `init` succeeds is a
/// programming error — the function `unimplemented!()`s rather than
/// silently dropping bytes so a too-early `arch::_print` re-routing
/// trips an obvious panic in development.
///
/// No caller reaches this in the current commit; the aarch64 / armv7
/// banner path still goes exclusively through PL011 MMIO.
pub fn write_bytes(_buf: &[u8]) {
    unimplemented!(
        "usb_uart::write_bytes — controller driver not yet implemented (#392 scaffold)"
    );
}

/// Pull one byte from the CDC-ACM bulk-OUT endpoint, non-blocking.
/// `None` = host hasn't sent anything since the last poll.
///
/// Used by the future REPL bring-up on real hardware (#393 follow-up)
/// — the host PC's terminal emulator types into `/dev/ttyACM0` and
/// keystrokes flow through here into the kernel REPL line buffer.
///
/// Today this is a stub for the same reason `write_bytes` is. No
/// caller reaches it.
pub fn read_byte() -> Option<u8> {
    unimplemented!(
        "usb_uart::read_byte — controller driver not yet implemented (#392 scaffold)"
    );
}
