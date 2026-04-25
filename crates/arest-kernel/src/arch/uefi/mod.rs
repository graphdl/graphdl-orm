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
//   * GDT / TSS reprogramming — firmware's GDT and CR3 stay live
//     through boot. The IDT below installs without touching
//     descriptor tables; #344f scope picks up GDT replacement when
//     hardware IRQs (PIT timer + PS/2 keyboard) need a TSS-backed
//     IST stack switch.
//   * 8259 PIC / APIC programming — `init_interrupts` populates only
//     the CPU-exception slots (#BP, #DF). Hardware-IRQ vector wiring
//     is #344f.
//
// What this commit adds (step 4d prep):
//   * `enable_sse`, `halt_forever` — CPU-level primitives identical in
//     shape to the x86_64 arm. Both `x86_64-unknown-none` and
//     `x86_64-unknown-uefi` run on the same silicon, so these are
//     target-os-agnostic; they live in each arm rather than a shared
//     sub-module to match the existing per-arm structure. Pre-requisite
//     for any UEFI kernel_run path that touches floating-point
//     (wasmi's f32/f64 ops — #270/#271).
//
// What #363 adds:
//   * `interrupts` module — kernel-owned IDT for the UEFI x86_64 path.
//     `init_interrupts()` builds + `lidt`-loads a `Once<InterruptDescriptorTable>`
//     populated with breakpoint + double-fault handlers. `breakpoint()`
//     fires `int3` so the boot banner can prove the IDT is live via a
//     round-trip smoke. Re-exported here so the entry calls
//     `arch::init_interrupts()` / `arch::breakpoint()` target-agnostically.

pub mod interrupts;
pub mod memory;
mod serial;

pub use interrupts::{breakpoint, init_interrupts};
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

/// Configure CR0/CR4 so SSE / SSE2 instructions don't fault. Same
/// bits the BIOS arm flips (see `arch::x86_64::enable_sse`) — both
/// targets run on x86_64 silicon with the same default mode bits,
/// so any non-trivial dep that emits SSE (wasmi's f32/f64 ops —
/// #270/#271) needs this before first use.
///
/// Callable after ExitBootServices; UEFI firmware leaves the host
/// with the same default CR0.EM=1 / CR4.OSFXSR=0 the bootloader
/// hands to the BIOS arm.
pub fn enable_sse() {
    use ::x86_64::registers::control::{Cr0, Cr0Flags, Cr4, Cr4Flags};
    // SAFETY: writing CR0/CR4 is a one-shot CPU-mode change that
    // matches what every x86_64 OS does once on entry. No memory
    // safety concern; we're flipping CPU feature bits.
    unsafe {
        let mut cr0 = Cr0::read();
        cr0.remove(Cr0Flags::EMULATE_COPROCESSOR);
        cr0.insert(Cr0Flags::MONITOR_COPROCESSOR);
        Cr0::write(cr0);

        let mut cr4 = Cr4::read();
        cr4.insert(Cr4Flags::OSFXSR | Cr4Flags::OSXMMEXCPT_ENABLE);
        Cr4::write(cr4);
    }
}

/// Drive the kernel's idle loop. Unlike the BIOS arm's
/// `halt_forever` (which busy-polls smoltcp because the keyboard is
/// the only unmasked IRQ), the UEFI arm has no IRQ infrastructure
/// yet — a plain `hlt` would hang forever because nothing wakes it.
/// Use a pause-loop as an interim: cheaper than `spin_loop` alone on
/// SMT cores, no dependency on timer / keyboard IRQs.
///
/// Once step 4d installs the UEFI IDT + PIT, this swaps for the
/// same `hlt`-then-poll shape the BIOS arm eventually wants.
pub fn halt_forever() -> ! {
    loop {
        unsafe { core::arch::asm!("pause", options(nomem, nostack)); }
    }
}
