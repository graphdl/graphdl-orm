// crates/arest-kernel/src/arch/mod.rs
//
// Per-target arch abstraction (#344 step 2). Introduced by the UEFI
// pivot so the shared kernel body can call through one facade —
// `arch::init_console`, `arch::init_gdt_and_interrupts`,
// `arch::init_memory`, `arch::breakpoint`, `arch::halt_forever` — and
// the x86_64 / aarch64 impls live below it.
//
// Gated on `not(target_os = "uefi")` for now: the UEFI scaffold
// (`entry_uefi.rs`) is still self-contained and doesn't reach through
// arch. Step 4 (ExitBootServices + kernel_run handoff) is where the
// UEFI path starts driving the same arch facade.

#![cfg(not(target_os = "uefi"))]

pub mod x86_64;

// Re-export the active target's items at `arch::` so callers can
// write `crate::arch::serial::_print` / `crate::arch::_print` target-
// agnostically. The glob is narrow today (one arch), but the shape
// is what the aarch64 impl will plug into.
pub use x86_64::*;
