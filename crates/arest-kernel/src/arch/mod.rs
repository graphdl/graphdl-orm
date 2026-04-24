// crates/arest-kernel/src/arch/mod.rs
//
// Per-target arch facade (#344 step 2 + step 3). The shared kernel
// body talks to the silicon through this module — `arch::_print`,
// `arch::init_console`, `arch::init_gdt_and_interrupts`,
// `arch::init_memory`, `arch::breakpoint`, `arch::halt_forever` — and
// per-target submodules supply the implementations.
//
// Today two arms are wired:
//
//   * `x86_64/`  — full kernel surface for the BIOS path (16550 UART,
//                   GDT/TSS, IDT + 8259 PIC, OffsetPageTable, idle loop).
//                   Active under `not(target_os = "uefi")`.
//   * `uefi/`    — minimal UEFI surface: `_print` + `init_console`
//                   route through the firmware's ConOut protocol so
//                   the existing `println!` macros work pre-Exit­Boot­
//                   Services. The rest of the kernel surface lands
//                   with step 4 (kernel_run handoff). Active under
//                   `target_os = "uefi"`.
//
// The `print!` / `println!` macros are declared here (not in either
// arm) so the same macro definitions resolve on both targets — only
// the `_print` callee is arch-specific.

#[cfg(not(target_os = "uefi"))]
pub mod x86_64;
#[cfg(not(target_os = "uefi"))]
pub use x86_64::*;

#[cfg(target_os = "uefi")]
pub mod uefi;
#[cfg(target_os = "uefi")]
pub use uefi::*;

/// Crate-wide `print!`. Routes to `$crate::arch::_print`, which is
/// supplied by the active arch arm above.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::arch::_print(format_args!($($arg)*)));
}

/// Crate-wide `println!`. Same routing as `print!`, with a trailing
/// newline.
#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}
