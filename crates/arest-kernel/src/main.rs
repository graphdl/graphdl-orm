// crates/arest-kernel/src/main.rs
//
// AREST bare-metal kernel entry point. Runs under `x86_64-unknown-none`
// (nightly for `abi_x86_interrupt`) with the rust-osdev `bootloader`
// crate supplying a Multiboot2-compatible stub that drops us into
// 64-bit long mode with paging already turned on and a populated
// `BootInfo` on the stack.
//
// Current boot pipeline:
//   BIOS / UEFI
//     └─> bootloader (Multiboot2 stage, built by arest-kernel-image)
//           └─> kernel_main(&'static mut BootInfo) -> !
//                 └─> allocator::init()        — 1 MiB static heap
//                 └─> gdt::init()              — GDT + TSS + IST
//                 └─> interrupts::init_idt()
//                 └─> interrupts::init_pic()   — remap + unmask KB
//                 └─> SERIAL banner
//                 └─> hlt loop (waits for IRQs)
//
// With the PIC live the `hlt` loop wakes on every keyboard scancode.
// The REPL (#183) accumulates keystrokes into a line buffer and
// dispatches commands on Enter. The arest engine is not yet linked
// so all non-built-in input returns a stub message.

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

extern crate alloc;

mod allocator;
mod gdt;
mod http;
mod interrupts;
mod memory;
mod net;
mod repl;
mod serial;
mod virtio;

use alloc::string::ToString;
use bootloader_api::{BootInfo, entry_point};
use core::panic::PanicInfo;

entry_point!(kernel_main);

/// Called by the bootloader the moment we land in 64-bit long mode.
fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    allocator::init();
    gdt::init();
    interrupts::init_idt();
    interrupts::init_pic();
    memory::init(boot_info);
    virtio::init_offset(
        boot_info.physical_memory_offset.into_option()
            .expect("bootloader did not supply physical_memory_offset"),
    );
    net::init();
    net::register_http(80, placeholder_http_handler);

    // Collect memory stats for the banner.
    let frame_count = memory::usable_frame_count();
    let usable_mib  = (frame_count * 4096) / (1024 * 1024);

    println!("AREST kernel online");
    println!("  target: x86_64-unknown-none");
    println!("  heap:   1 MiB static (#178)");
    println!("  gdt:    loaded with TSS + double-fault IST (#179)");
    println!("  idt:    breakpoint + double-fault + keyboard (#181)");
    println!("  pic:    remapped to 32+, keyboard (IRQ 1) unmasked");
    println!("  memory: {usable_mib} MiB usable RAM ({frame_count} x 4 KiB frames) (#180)");
    println!("  net:    smoltcp loopback 127.0.0.1/8 (#261 — virtio-net in #262)");
    println!("  http:   listening on :80 (#264)");

    // Prove the allocator works — allocate a String and echo it.
    let greeting = "heap is live".to_string();
    println!("  alloc: {greeting}");

    // Prove the IDT routes — `int3` should land in our breakpoint
    // handler, print a frame, and return cleanly.
    x86_64::instructions::interrupts::int3();
    println!("  idt:   int3 round-tripped through breakpoint handler");

    println!("  repl:   line-buffered keyboard REPL online (#183)");
    println!();

    // Print initial prompt — REPL is now live.
    repl::init();

    halt_forever();
}

/// Park the CPU in a `hlt` loop. With interrupts enabled, `hlt`
/// wakes on any IRQ (keyboard, timer once added) so per-keypress
/// latency is measured in microseconds instead of busy-spin cycles.
///
/// Each wake drives `net::poll()` so that any TCP / DHCP progress
/// queued since the last IRQ gets processed before we sleep again.
/// Once a dedicated timer IRQ lands (#180 follow-up) we can drop the
/// poll here and let the timer handler schedule it — for now, piggy-
/// backing on keyboard IRQs is good enough for the loopback bring-up.
fn halt_forever() -> ! {
    loop {
        net::poll();
        x86_64::instructions::hlt();
    }
}

/// Placeholder HTTP handler until #265 wires route dispatch through
/// to `arest::system_impl`. Echoes `METHOD path` so a curl against
/// the QEMU port-forward confirms request parsing works end to end.
fn placeholder_http_handler(req: &http::Request) -> http::Response {
    use alloc::string::String;
    let mut body = String::with_capacity(32 + req.method.len() + req.path.len());
    body.push_str("AREST ");
    body.push_str(&req.method);
    body.push(' ');
    body.push_str(&req.path);
    body.push('\n');
    http::Response::ok("text/plain", body.into_bytes())
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("\n!! AREST kernel panic !!");
    println!("{info}");
    halt_forever();
}
