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
//         arest-kernel-image crate in a follow-up commit)
//           └─> kernel_main(&'static mut BootInfo) -> !
//                 └─> SERIAL.lock() banner
//                 └─> hlt loop
//
// Today this is MVP plumbing — a kernel that wakes up, writes a
// banner over the 8250 UART (COM1 @ 0x3F8), and halts. No AREST
// engine integration yet; that follows #174 landing no_std-clean
// versions of the core modules (#182 baked metamodel, #183 REPL).
// Uses only `core` primitives plus `uart_16550` (serial driver) and
// `spin` (lock) — `alloc` enters once the allocator is wired (#178).

#![no_std]
#![no_main]

mod serial;

use bootloader_api::{BootInfo, entry_point};
use core::panic::PanicInfo;

entry_point!(kernel_main);

/// Called by the bootloader the moment we land in 64-bit long mode.
/// `boot_info` carries the memory map, framebuffer handle, and other
/// platform detail the bootloader gathered from the firmware. We
/// ignore it for now — the MVP goal is "a running kernel that
/// proves the toolchain and image pipeline work end-to-end."
fn kernel_main(_boot_info: &'static mut BootInfo) -> ! {
    println!("AREST kernel online");
    println!("  target: x86_64-unknown-none");
    println!("  stage:  MVP boot, no engine wired yet (#182)");
    halt_forever();
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
    // Best-effort panic banner over serial. Use the raw writer
    // (bypassing the lock) in case the panic fired while the lock
    // was held — a poisoned lock would deadlock the panic path.
    println!("\n!! AREST kernel panic !!");
    println!("{info}");
    halt_forever();
}
