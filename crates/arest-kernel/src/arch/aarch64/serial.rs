// crates/arest-kernel/src/arch/aarch64/serial.rs
//
// PL011 UART writer for the aarch64 UEFI path (#344 cross-arch). The
// aarch64 analogue of `arch::x86_64::serial`'s 16550 driver — under
// QEMU's `virt` machine the PL011 is memory-mapped at the fixed
// physical address 0x0900_0000, which is part of the standard virt
// platform layout (QEMU hw/arm/virt.c, `VIRT_UART`). UEFI firmware
// on this machine identity-maps this region during boot, so raw
// volatile writes against that address land on the UART FIFO without
// any extra MMU set-up.
//
// Scaffold scope (this commit — compile-only):
//   * `_print(args)` — format into a fixed-size stack buffer, then
//     volatile-write each byte at `UARTDR` (offset 0x000). No
//     back-pressure handling — for a banner-only scaffold dropping
//     bytes under a full TX FIFO is acceptable; QEMU's PL011 model
//     drains at host speed so the FIFO never stays full in practice.
//   * No `switch_to_post_ebs_serial` analogue yet — the aarch64 arm
//     has no ExitBootServices machinery (that lands in follow-ups
//     alongside the memory-map + IDT-equivalent work).
//
// Pipeline (pre-EBS, via firmware):
//   1. `fmt::Write` for `Pl011Writer` walks the formatted bytes.
//   2. For each byte, volatile-write to `UARTDR`.
//
// Why no ConOut path like the x86_64 UEFI arm has: the aarch64 entry
// prints its banner via raw MMIO directly, bypassing firmware ConOut.
// That keeps the scaffold self-contained — no dependency on
// `uefi::system::with_stdout`, which on aarch64-unknown-uefi sometimes
// routes through a graphics-console that doesn't reach QEMU's
// `-serial stdio` the way COM1 does on x86. PL011 MMIO is what the
// smoke harness (Dockerfile variant, out of scope this commit) will
// watch once it's online.
//
// Addresses on QEMU virt (AArch64):
//   * 0x0900_0000 + 0x000  UARTDR   data register — write = TX byte
//   * 0x0900_0000 + 0x018  UARTFR   flag register (bit 5 = TXFF)
//
// Only UARTDR is touched by this scaffold; the busy-poll on UARTFR is
// deferred — QEMU's emulated UART never back-pressures so polling is
// a pure cost. If a later commit runs on real hardware, add the
// `while UARTFR & TXFF != 0 {}` busy-wait before each write.

use core::fmt;

/// QEMU virt PL011 MMIO base (hw/arm/virt.c `VIRT_UART`).
const PL011_BASE: usize = 0x0900_0000;

/// PL011 data register offset. Writes transmit; reads dequeue.
const UARTDR_OFFSET: usize = 0x000;

/// `fmt::Write` adapter that pushes each byte into UARTDR via a
/// volatile store. Zero-sized — the MMIO address is a fixed constant
/// rather than runtime state, matching how the PL011 is laid out on
/// QEMU's virt machine.
pub struct Pl011Writer;

impl Pl011Writer {
    pub const fn new() -> Self {
        Self
    }
}

impl fmt::Write for Pl011Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // SAFETY: `PL011_BASE + UARTDR_OFFSET` is a fixed physical
        // address corresponding to QEMU's virt machine PL011 UART.
        // The address is identity-mapped by UEFI firmware during
        // boot services (standard virt platform behavior), and the
        // MMIO register accepts any u8 without memory-safety impact.
        // `write_volatile` prevents the compiler from caching or
        // reordering the store.
        let dr = (PL011_BASE + UARTDR_OFFSET) as *mut u8;
        for b in s.bytes() {
            unsafe { dr.write_volatile(b) };
        }
        Ok(())
    }
}

/// Write a raw byte string directly to the PL011 without going
/// through `fmt::Arguments`. Useful for the very first banner line
/// before any allocator state is set up — no heap dependency, no
/// format machinery, just a linear MMIO store loop.
pub fn raw_puts(s: &str) {
    let dr = (PL011_BASE + UARTDR_OFFSET) as *mut u8;
    for b in s.bytes() {
        // SAFETY: see `Pl011Writer::write_str`.
        unsafe { dr.write_volatile(b) };
    }
}

/// Called by the crate-wide `print!` / `println!` macros (declared in
/// `arch/mod.rs`). Writes formatted arguments through a stack-
/// instantiated `Pl011Writer` — no statics, no locking, since the
/// aarch64 scaffold is single-threaded and the MMIO register is
/// stateless from the guest's perspective.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    use core::fmt::Write;
    let mut w = Pl011Writer::new();
    let _ = w.write_fmt(args);
}
