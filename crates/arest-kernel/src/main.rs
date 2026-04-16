// crates/arest-kernel/src/main.rs
//
// AREST bare-metal kernel entry point. Runs under `x86_64-unknown-none`
// (stable) with the rust-osdev `bootloader` crate supplying a
// Multiboot2-compatible stub that drops us into 64-bit long mode with
// paging already turned on and a populated `BootInfo` on the stack.
//
// Current boot pipeline:
//   BIOS / UEFI
//     └─> bootloader (Multiboot2 stage, built by arest-kernel-image)
//           └─> kernel_main(&'static mut BootInfo) -> !
//                 └─> allocator::init() — 1 MiB static heap
//                 └─> SERIAL.lock() banner
//                 └─> hlt loop
//
// Today this is MVP plumbing — a kernel that wakes up, brings up the
// allocator and serial console, prints a banner, and halts. No AREST
// engine integration yet; that follows #174 landing no_std-clean
// versions of the core modules (#182 baked metamodel, #183 REPL).

#![no_std]
#![no_main]

extern crate alloc;

mod allocator;
mod serial;

use alloc::string::ToString;
use bootloader_api::{BootInfo, entry_point};
use core::panic::PanicInfo;

entry_point!(kernel_main);

/// Called by the bootloader the moment we land in 64-bit long mode.
/// `boot_info` carries the memory map, framebuffer handle, and other
/// platform detail the bootloader gathered from the firmware. We
/// ignore it for now — the MVP goal is "a running kernel that
/// proves the toolchain and image pipeline work end-to-end."
fn kernel_main(_boot_info: &'static mut BootInfo) -> ! {
    allocator::init();

    println!("AREST kernel online");
    println!("  target: x86_64-unknown-none");
    println!("  heap:   1 MiB static (#178)");

    // Prove the allocator works — allocate a String and echo it.
    // Once #182 lands this becomes the baked-metamodel thaw path.
    let greeting = "heap is live".to_string();
    println!("  alloc: {greeting}");

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
    println!("\n!! AREST kernel panic !!");
    println!("{info}");
    halt_forever();
}
