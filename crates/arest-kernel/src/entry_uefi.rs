// crates/arest-kernel/src/entry_uefi.rs
//
// UEFI entry point (#344). Only compiled for `target_os = "uefi"` —
// the BIOS path (`x86_64-unknown-none`) keeps its `bootloader_api`
// entry in `main.rs`'s top-level gated body.
//
// Step 1 scaffold: bring the kernel up under UEFI far enough to
// prove `uefi-rs` links, `#[entry]` wires a `_start` symbol the
// firmware picks up, and `ConOut` reaches the serial console. The
// real arch-neutral `kernel_run(BootInfo)` lands after step 2 of the
// pivot (arch trait extraction) — this file stays tiny until then.
//
// What this gives us today:
//   * `cargo build --target x86_64-unknown-uefi --release` produces
//     an `EFI` executable.
//   * Boot under QEMU-OVMF:
//       qemu-system-x86_64 -bios OVMF.fd -kernel arest-kernel.efi
//     prints the AREST scaffold banner via firmware ConOut.
//   * BIOS path is untouched — existing x86_64-unknown-none build
//     still produces the same kernel image.
//   * `println!` (#344 step 3) routes through `arch::_print`, whose
//     UEFI implementation (`arch::uefi::serial::_print`) writes via
//     ConOut. Same macro the BIOS path uses — no UEFI-specific
//     printing call sites in shared kernel code.
//
// What this does not do yet (tracked in #344 follow-up commits):
//   * ExitBootServices + hand-off to `kernel_run` (step 4).
//   * Real arch serial driver post-ExitBootServices (16550 on
//     x86_64-uefi → COM1 in QEMU; PL011 on aarch64-uefi → virt
//     pl011 in QEMU). Until then `_print` writes silently no-op
//     after firmware services tear down.
//   * Populate a `BootInfo` from UEFI GetMemoryMap + Graphics Output
//     Protocol, so `memory::init` / the framebuffer work the same
//     way the BIOS path does (step 4).
//   * aarch64-unknown-uefi — this entry is target-agnostic, but the
//     kernel body below the arch trait doesn't exist yet (step 5).

#![cfg(target_os = "uefi")]

use uefi::prelude::*;

use crate::println;

// Global allocator — uefi-rs ships a small wrapper around
// `BootServices::allocate_pool`. Required because the kernel crate
// has `extern crate alloc;` at the top; without a global allocator
// the UEFI bin won't link, and `arch::uefi::_print` needs `String`
// allocation to format args before transcoding to UCS-2.
#[global_allocator]
static ALLOCATOR: uefi::allocator::Allocator = uefi::allocator::Allocator;

/// UEFI entry point. `uefi-rs`'s `#[entry]` expands this into the
/// PE32+ `_start` symbol the firmware invokes after loading the
/// image.
#[entry]
fn efi_main() -> Status {
    // ConOut is already configured by the firmware; init_console is
    // a no-op on UEFI but kept as the named entry so step 4's shared
    // kernel_run can call it target-agnostically.
    crate::arch::init_console();

    // ASCII hyphens — keeps the line printable on bare COM1, which
    // most OVMF builds downcode UCS-2 → ASCII on. The kernel itself
    // happily transcodes BMP glyphs through ConOut, but the smoke
    // harness reads stdout via QEMU's `-serial stdio`, where the
    // round-trip survives only if the banner is ASCII.
    println!("AREST kernel - UEFI scaffold (#344)");
    println!("  step 3 of 8: println! routed through arch::_print");
    println!("  next:        ExitBootServices + kernel_run handoff");

    // Scaffold halt. A real boot calls BootServices::exit_boot_services
    // and jumps to kernel_run(BootInfo); until that lands, park here
    // so the firmware doesn't fall through to "no image loaded."
    loop {
        unsafe { core::arch::asm!("pause", options(nomem, nostack)); }
    }
}
