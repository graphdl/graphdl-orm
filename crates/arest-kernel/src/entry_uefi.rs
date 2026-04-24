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
use uefi::boot::MemoryType;

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
///
/// Boot pipeline (#344 step 4 — partial):
///   1. ConOut online (firmware-managed, init_console no-op on UEFI).
///   2. Pre-EBS banner via `println!` → ConOut.
///   3. `boot::exit_boot_services` — firmware tears down. After
///      this, the system table is invalidated and `with_stdout`
///      silently no-ops.
///   4. `arch::switch_to_post_ebs_serial` flips `_print` onto the
///      direct-I/O 16550 path. Same COM1 line QEMU's `-serial
///      stdio` is wired to, so the banner survives the hand-off
///      unbroken on the host terminal.
///   5. Post-EBS banner via `println!` → 16550. Proves the cutover
///      works end-to-end.
///   6. `arch::init_memory(memory_map)` (step 4c) — consume the
///      firmware memory map, install the OffsetPageTable + frame
///      allocator singletons behind the same accessor surface the
///      BIOS arm publishes, and print a post-init banner proving
///      the page-table singleton is live.
///   7. Halt. Step 4d (kernel_run handoff) wires the arch-neutral
///      kernel body once its subsystems (virtio / net / blk / repl)
///      drop their `cfg(not(target_os = "uefi"))` gates.
#[entry]
fn efi_main() -> Status {
    crate::arch::init_console();

    // ASCII hyphens — keeps the line printable on bare COM1, which
    // most OVMF builds downcode UCS-2 -> ASCII on. The kernel itself
    // happily transcodes BMP glyphs through ConOut, but the smoke
    // harness reads stdout via QEMU's `-serial stdio`, where the
    // round-trip survives only if the banner is ASCII.
    println!("AREST kernel - UEFI scaffold (#344)");
    println!("  step 4 of 8: ExitBootServices + post-EBS serial");
    println!("  pre-EBS:  ConOut active (firmware-managed)");

    // SAFETY: `boot::exit_boot_services` walks the current memory
    // map, gets the firmware's signature lock, and tears down
    // BootServices. The returned `MemoryMapOwned` is a stable copy
    // of the map the firmware handed us. We hand it straight into
    // `arch::init_memory` (step 4c) which flattens the CONVENTIONAL
    // regions into a frame allocator and stands up the page-table
    // singleton.
    let memory_map = unsafe { boot::exit_boot_services(MemoryType::LOADER_DATA) };

    // Firmware ConOut is now invalid. Switch `_print` onto the
    // direct-I/O 16550 path BEFORE the next println! so the
    // banner doesn't disappear into a no-op.
    crate::arch::switch_to_post_ebs_serial();

    println!("  post-EBS: 16550 COM1 active (kernel-managed)");

    // Step 4c: consume the firmware memory map, install the paging
    // + frame-allocator singletons. `init_memory` returns the
    // physical-memory offset (always 0 on UEFI — firmware identity-
    // maps RAM), matching the shape of the BIOS arm's facade.
    let _phys_offset = crate::arch::init_memory(memory_map);

    // Proves the page-table singleton is live post-EBS: going
    // through `memory::usable_frame_count()` forces a `FRAME_ALLOCATOR.lock()`
    // + a pass over the descriptor iterator, so a hung lock or a
    // malformed memory map surfaces here rather than silently at
    // first allocation inside kernel_run.
    let frame_count = crate::arch::memory::usable_frame_count();
    let usable_mib = (frame_count * 4096) / (1024 * 1024);
    println!(
        "  mem:      {frame_count} frames usable ({usable_mib} MiB) (UEFI memory map)"
    );
    println!("  next:        kernel_run handoff (step 4d)");

    // Scaffold halt. Step 4d wires `kernel_run(phys_offset)` once
    // the shared kernel body subsystems are UEFI-capable.
    loop {
        unsafe { core::arch::asm!("pause", options(nomem, nostack)); }
    }
}
