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

// armv7 UEFI arm (#346 — first commit of the sub-chain). Scaffold-
// only this commit: PL011 MMIO `_print` + `init_console` no-op +
// `wfi` `halt_forever`, mirroring the aarch64 arm's bring-up shape.
// No memory / DMA / virtio-mmio yet — those are #346b/c. No runtime
// harness yet — that's #346d. Gated on `target_arch = "arm"` to
// pick up only the custom `arest-kernel-armv7-uefi.json` target;
// `target_arch = "aarch64"` (the 64-bit ARM UEFI arm) is a sibling
// arm and stays on its own arch arm above.
#[cfg(all(target_os = "uefi", target_arch = "arm"))]
pub mod armv7;
// `pub use` is unused on this commit: the armv7 arm has no runtime
// harness yet (#346d brings it online), so no caller resolves
// `arch::_print` / `arch::halt_forever` / `arch::init_console` on
// the armv7 build. Mirrors the `#[allow(unused_imports)]` the
// aarch64 arm carries on its `serial::{_print, raw_puts}` re-export
// for the same reason.
#[cfg(all(target_os = "uefi", target_arch = "arm"))]
#[allow(unused_imports)]
pub use armv7::*;

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
