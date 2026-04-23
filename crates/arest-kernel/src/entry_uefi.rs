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
//
// What this does not do yet (tracked in #344 follow-up commits):
//   * ExitBootServices + hand-off to `kernel_run`.
//   * Wire serial → println! through the kernel's existing
//     `serial.rs` path (currently 16550-specific).
//   * Populate a `BootInfo` from UEFI GetMemoryMap + Graphics Output
//     Protocol, so `memory::init` / the framebuffer work the same
//     way the BIOS path does.
//   * aarch64-unknown-uefi — this entry is target-agnostic, but the
//     kernel body below the arch trait doesn't exist yet.

#![cfg(target_os = "uefi")]

use uefi::prelude::*;

// Global allocator — uefi-rs ships a small wrapper around
// `BootServices::allocate_pool`. Required because the kernel crate
// has `extern crate alloc;` at the top; without a global allocator
// the UEFI bin won't link.
#[global_allocator]
static ALLOCATOR: uefi::allocator::Allocator = uefi::allocator::Allocator;

/// UEFI entry point. `uefi-rs`'s `#[entry]` expands this into the
/// PE32+ `_start` symbol the firmware invokes after loading the
/// image.
#[entry]
fn efi_main() -> Status {
    // `uefi-rs` 0.34 installs the system table as a global, so
    // `uefi::system::with_stdout` reaches the ConOut protocol
    // without threading the table through every call.
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(
            cstr16!("AREST kernel — UEFI scaffold (#344)\r\n")
        );
        let _ = stdout.output_string(
            cstr16!("  step 1 of 8: entry + ConOut online\r\n")
        );
        let _ = stdout.output_string(
            cstr16!("  next:      ExitBootServices + kernel_run handoff\r\n")
        );
    });

    // Scaffold halt. A real boot calls BootServices::exit_boot_services
    // and jumps to kernel_run(BootInfo); until that lands, park here
    // so the firmware doesn't fall through to "no image loaded."
    loop {
        unsafe { core::arch::asm!("pause", options(nomem, nostack)); }
    }
}
