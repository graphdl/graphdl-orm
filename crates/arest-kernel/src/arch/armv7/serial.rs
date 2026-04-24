// crates/arest-kernel/src/arch/armv7/serial.rs
//
// PL011 UART writer for the armv7 UEFI path (#346 cross-arch). Sibling
// of `arch::aarch64::serial` — QEMU's `virt` machine maps a PL011 at
// the fixed physical address 0x0900_0000 on BOTH the aarch64 and
// armv7 (32-bit) variants, so the MMIO surface is identical and we
// just retarget the same volatile-store loop at a 32-bit-pointer-width
// build.
//
// Scaffold scope (this commit — compile-only):
//   * `_print(args)` — format into a `Pl011Writer` and volatile-write
//     each byte at `UARTDR` (offset 0x000). No back-pressure handling
//     under QEMU; if we land on real silicon a follow-up adds the
//     UARTFR busy-poll on bit 5 (TXFF) before each store.
//   * No ConOut / firmware-stdout adapter — same rationale as the
//     aarch64 arm: PL011 MMIO bypasses uefi-rs's stdout abstraction
//     so the boot banner survives ExitBootServices unchanged.
//
// Pipeline (pre-EBS, via firmware):
//   1. `fmt::Write` for `Pl011Writer` walks the formatted bytes.
//   2. For each byte, volatile-write to `UARTDR`.
//
// Addresses on QEMU virt (32-bit ARM — same as aarch64 layout):
//   * 0x0900_0000 + 0x000  UARTDR   data register — write = TX byte
//   * 0x0900_0000 + 0x018  UARTFR   flag register (bit 5 = TXFF)
//
// Only UARTDR is touched by this scaffold; UARTFR busy-polling lands
// when (and if) this arm runs on real hardware. QEMU's emulated UART
// drains at host speed so the FIFO never stays full in practice.

use core::fmt;

/// QEMU virt PL011 MMIO base. QEMU's `hw/arm/virt.c` exposes the same
/// VIRT_UART address in both the 32-bit (`armv7`) and 64-bit (`aarch64`)
/// virt machine variants — so this constant duplicates the aarch64
/// arm's `PL011_BASE` byte-for-byte. The pointer width differs (32-bit
/// here vs 64-bit there), but the address fits in `usize` on both.
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
        // boot services (standard virt platform behavior — same on
        // 32-bit and 64-bit ARM virt machines), and the MMIO register
        // accepts any u8 without memory-safety impact. `write_volatile`
        // prevents the compiler from caching or reordering the store.
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
/// format machinery, just a linear MMIO store loop. Identical shape
/// to `arch::aarch64::serial::raw_puts` so a future shared
/// `entry_uefi_arm` helper can call either arm without retargeting.
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
/// armv7 scaffold is single-threaded and the MMIO register is
/// stateless from the guest's perspective.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    use core::fmt::Write;
    let mut w = Pl011Writer::new();
    let _ = w.write_fmt(args);
}
