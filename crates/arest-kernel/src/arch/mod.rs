// crates/arest-kernel/src/arch/mod.rs
//
// Per-target arch facade (#344 step 2 + step 3). The shared kernel
// body talks to the silicon through this module — `arch::_print`,
// `arch::init_console`, `arch::init_gdt_and_interrupts`,
// `arch::init_memory`, `arch::breakpoint`, `arch::halt_forever` — and
// per-target submodules supply the implementations.
//
// Today three arms are wired:
//
//   * `x86_64/`  — full kernel surface for the BIOS path (16550 UART,
//                   GDT/TSS, IDT + 8259 PIC, OffsetPageTable, idle loop).
//                   Active under `not(target_os = "uefi")`.
//   * `uefi/`    — x86_64-specific UEFI arm (16550 UART, x86_64 port
//                   I/O + CR0/CR4 control). Named `uefi` for historical
//                   reasons — it's actually an x86_64 UEFI arm. Gated
//                   on `all(target_os = "uefi", target_arch = "x86_64")`
//                   rather than plain `target_os = "uefi"` so the
//                   aarch64 arm below can compile independently.
//   * `aarch64/` — aarch64 UEFI arm (PL011 MMIO at 0x0900_0000, `wfi`
//                   idle loop). Scaffold-only today: `_print`,
//                   `init_console`, `halt_forever`. Active under
//                   `all(target_os = "uefi", target_arch = "aarch64")`.
//                   See `aarch64/mod.rs` for the step-by-step surface
//                   the x86_64 UEFI arm carries that this arm still
//                   needs to grow.
//
// The `print!` / `println!` macros are declared here (not in either
// arm) so the same macro definitions resolve on both targets — only
// the `_print` callee is arch-specific.

#[cfg(not(target_os = "uefi"))]
pub mod x86_64;
#[cfg(not(target_os = "uefi"))]
pub use x86_64::*;

#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
pub mod uefi;
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
pub use uefi::*;

#[cfg(all(target_os = "uefi", target_arch = "aarch64"))]
pub mod aarch64;
#[cfg(all(target_os = "uefi", target_arch = "aarch64"))]
pub use aarch64::*;

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
