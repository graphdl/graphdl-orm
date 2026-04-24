// crates/arest-kernel/src/arch/uefi/mod.rs
//
// UEFI arch arm (#344 steps 3 + 4). Grows incrementally alongside the
// UEFI pivot: today it supplies the subset of the shared arch facade
// that the UEFI entry has reached — console, serial cutover, and (as
// of step 4c) memory bring-up from the firmware-provided memory map.
//
// What's implemented:
//   * `_print(args)` / `switch_to_post_ebs_serial()` — ConOut before
//     `exit_boot_services`, direct-I/O 16550 on COM1 after (step 4b).
//   * `init_console()` — no-op. ConOut is firmware-managed, the 16550
//     lazy-inits on the first post-EBS write.
//   * `init_memory(memory_map)` — step 4c. Consumes the firmware's
//     `MemoryMapOwned`, stands up the `OffsetPageTable` + frame
//     allocator singletons behind the same accessor API the BIOS arm
//     publishes (`memory::with_page_table`, `memory::with_frame_allocator`,
//     `memory::usable_frame_count`), and returns the physical-memory
//     offset (= 0 on UEFI — firmware identity-maps).
//
// What's deliberately NOT here yet:
//   * `init_gdt_and_interrupts`, `breakpoint`, `halt_forever`,
//     `enable_sse` — land alongside step 4d (kernel_run handoff) once
//     the kernel-body subsystems that depend on them (virtio / net /
//     blk / repl) are UEFI-capable. Until then the entry point halts
//     after proving the page-table singleton is live.

pub mod memory;
mod serial;

pub use serial::{_print, switch_to_post_ebs_serial};

/// Initialise the architecture's console. Under UEFI the firmware has
/// already configured ConOut before transferring control to our entry,
/// so this is a no-op — kept as the named entry point so the shared
/// kernel body can call `arch::init_console()` target-agnostically.
pub fn init_console() {
    // Intentionally empty — see module docstring.
}

/// Initialise the memory subsystem from the UEFI-provided memory map.
/// Consumes the `MemoryMapOwned` that `boot::exit_boot_services`
/// returns, installs the `OffsetPageTable` + frame-allocator
/// singletons, and returns the physical-memory offset (0 on UEFI —
/// the firmware identity-maps, so phys == virt).
///
/// Matches the shape of `arch::x86_64::init_memory(boot_info) -> u64`
/// so the shared kernel body can call `arch::init_memory(...)` without
/// knowing which boot path produced the map.
pub fn init_memory(memory_map: uefi::mem::memory_map::MemoryMapOwned) -> u64 {
    memory::init(memory_map)
}
