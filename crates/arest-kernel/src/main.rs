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
#![feature(naked_functions)]

extern crate alloc;

mod allocator;
mod dma;
mod gdt;
mod http;
mod interrupts;
mod memory;
mod net;
mod pci;
mod repl;
mod serial;
mod syscall;
mod system;
mod userspace;
mod virtio;

use alloc::string::ToString;
use bootloader_api::config::{BootloaderConfig, Mapping};
use bootloader_api::{BootInfo, entry_point};
use core::panic::PanicInfo;

// The default bootloader config leaves `physical_memory` unmapped,
// which breaks `memory::init` (it needs `BootInfo::physical_memory_offset`
// to be Some). Request dynamic mapping so the bootloader picks a
// free virtual range for the full physical-memory window — this is
// what the offset-page-table construction in memory.rs reads, and
// what virtio::init_offset and every later subsystem depends on.
pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut cfg = BootloaderConfig::new_default();
    cfg.mappings.physical_memory = Some(Mapping::Dynamic);
    cfg
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

/// Called by the bootloader the moment we land in 64-bit long mode.
fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    allocator::init();
    gdt::init();
    interrupts::init_idt();
    interrupts::init_pic();

    // Sec-6 ring-3 smoke test mode. Run only the subsystems needed
    // by userspace::launch_test_payload (memory — so map_user_page
    // can allocate user pages) and skip the rest (virtio / net /
    // system / REPL). Diverges into ring 3 and never returns.
    #[cfg(feature = "ring3-smoke")]
    {
        println!("AREST kernel online");
        println!("  mode: ring3-smoke — launching test payload");
        memory::init(boot_info);
        syscall::init();
        userspace::launch_test_payload();
    }

    memory::init(boot_info);
    virtio::init_offset(
        boot_info.physical_memory_offset.into_option()
            .expect("bootloader did not supply physical_memory_offset"),
    );
    // virtio-net bring-up (#262). PCI scan first (banner-visible),
    // then construct the full VirtIONet driver, then wrap it in the
    // `smoltcp::phy::Device` adapter `VirtioPhy` so the interface
    // talks to the real NIC. If any step fails we fall back to the
    // loopback device so in-guest smoke tests still run.
    let virtio_net = pci::find_virtio_net();
    let virtio_phy = virtio::try_init_virtio_net().map(virtio::VirtioPhy::new);
    let virtio_mac = virtio_phy.as_ref().map(|p| p.mac_address());

    net::init(virtio_phy);
    system::init();
    net::register_http(80, arest_http_handler);

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
    match virtio_net {
        Some(dev) => println!(
            "  pci:    virtio-net found at {:02x}:{:02x}.{} (vendor={:#06x} device={:#06x}, BAR0={:#010x})",
            dev.bus, dev.device, dev.function, dev.vendor_id, dev.device_id, dev.bars[0],
        ),
        None => println!("  pci:    no virtio-net device found on legacy PCI bus (loopback only)"),
    }
    match &virtio_mac {
        Some(mac) => println!("  virtio: driver online, smoltcp phy bound, MAC {}", mac),
        None => println!("  virtio: driver not constructed — falling back to loopback"),
    }
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

/// HTTP handler that routes each request through the baked SYSTEM
/// (#265). `system::dispatch` maps the path to a def name, looks up
/// the Func via FetchOrPhi, and ρ-applies it against the baked
/// state D. When no def matches the path, the handler returns a
/// plaintext 404 so curl still gets a human-readable answer.
fn arest_http_handler(req: &http::Request) -> http::Response {
    match system::dispatch(&req.method, &req.path, &req.body) {
        Some(body) => http::Response::ok("text/plain; charset=utf-8", body),
        None => http::Response::not_found(),
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("\n!! AREST kernel panic !!");
    println!("{info}");
    #[cfg(feature = "ring3-smoke")]
    {
        userspace::halt_on_exit(userspace::exit_code::KERNEL_PANIC);
    }
    #[cfg(not(feature = "ring3-smoke"))]
    {
        halt_forever();
    }
}
