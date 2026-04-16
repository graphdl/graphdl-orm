// crates/arest-kernel/src/main.rs
//
// AREST bare-metal kernel entry point. Runs under `x86_64-unknown-none`
// (stable) with the rust-osdev `bootloader` crate supplying a
// Multiboot2-compatible stub that drops us into 64-bit long mode with
// paging already turned on and a populated `BootInfo` on the stack.
//
// Current boot pipeline:
//   BIOS / UEFI
//     └─> bootloader (Multiboot2 stage, GRUB-compatible, built by the
//         bootloader-image crate in a follow-up commit)
//           └─> kernel_main(&'static mut BootInfo) -> !
//                 └─> hlt loop
//
// Today this is MVP plumbing — a kernel that wakes up, writes a
// banner over the 8250 UART (COM1 @ 0x3F8), and halts. No AREST
// engine integration yet; that follows #174 landing no_std-clean
// versions of the core modules (#182 baked metamodel, #183 REPL).
// Everything below uses only `core` primitives — `alloc` comes in
// once we wire the allocator (#178).

#![no_std]
#![no_main]

use bootloader_api::{BootInfo, entry_point};
use core::panic::PanicInfo;

entry_point!(kernel_main);

/// Called by the bootloader the moment we land in 64-bit long mode.
/// `boot_info` carries the memory map, framebuffer handle, and other
/// platform detail the bootloader gathered from the firmware. We
/// ignore it for now — the MVP goal is "a running kernel that
/// proves the toolchain and image pipeline work end-to-end."
fn kernel_main(_boot_info: &'static mut BootInfo) -> ! {
    write_serial(b"AREST kernel online\n");
    halt_forever();
}

/// Minimal 8250 UART driver. COM1 lives at I/O port 0x3F8 on every
/// PC-compatible machine QEMU exposes, so this needs no firmware
/// discovery to work inside QEMU's default x86_64 guest. Each byte
/// waits for the transmit-holding-register-empty bit (THRE, bit 5 of
/// LSR @ 0x3FD) before issuing the `out` instruction.
fn write_serial(bytes: &[u8]) {
    const DATA: u16 = 0x3F8;
    const LSR: u16 = 0x3FD;
    const THRE: u8 = 0x20;

    for &b in bytes {
        unsafe {
            while (inb(LSR) & THRE) == 0 {}
            outb(DATA, b);
        }
    }
}

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags));
    value
}

#[inline]
unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
}

/// Park the CPU in a `hlt` loop. Using `hlt` (vs. a busy spin) drops
/// the core into the C1 halt state so QEMU reports 0% CPU instead of
/// pinning a host thread at 100% while waiting for interrupts.
fn halt_forever() -> ! {
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Best-effort panic banner over serial. We cannot use `format!`
    // (no allocator yet — #178) so just write the raw panic marker;
    // source location plumbing follows once write_fmt is in.
    write_serial(b"\n!! AREST kernel panic !!\n");
    let _ = info;
    halt_forever();
}
