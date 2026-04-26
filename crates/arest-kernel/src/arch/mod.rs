// crates/arest-kernel/src/arch/mod.rs
//
// Per-target arch facade (#344 step 2 + step 3). The shared kernel
// body talks to the silicon through this module — `arch::_print`,
// `arch::init_console`, `arch::init_gdt_and_interrupts`,
// `arch::init_memory`, `arch::breakpoint`, `arch::halt_forever` — and
// per-target submodules supply the implementations.
//
// Three UEFI arms are wired:
//
//   * `uefi/`    — x86_64 UEFI arm (16550 UART, GDT/TSS, IDT + 8259
//                   PIC remap, x86_64 port I/O + CR0/CR4 control,
//                   PIT timer, PS/2 keyboard, OffsetPageTable, slint
//                   backend). Named `uefi` for historical reasons —
//                   it's actually the x86_64 UEFI arm. Gated on
//                   `all(target_os = "uefi", target_arch = "x86_64")`.
//   * `aarch64/` — aarch64 UEFI arm (PL011 MMIO at 0x0900_0000, virtio-
//                   mmio bring-up, `wfi` idle loop). Active under
//                   `all(target_os = "uefi", target_arch = "aarch64")`.
//   * `armv7/`   — armv7 UEFI arm (PL011 MMIO + virtio-mmio + MSVC ARM
//                   CRT shims). Active under `all(target_os = "uefi",
//                   target_arch = "arm")`.
//
// The `print!` / `println!` macros are declared here (not in any of
// the arms) so the same macro definitions resolve on every target —
// only the `_print` callee is arch-specific.

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

// Host-target arm (#579 Track QQQQQ — extract `lib.rs` so `cargo test
// --lib` runs the inline `#[cfg(test)]` modules). On any non-UEFI
// target — `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`,
// `aarch64-apple-darwin` — the kernel modules that reach
// `crate::print!` / `crate::println!` (e.g. `crate::syscall::write::do_write`,
// `crate::block::*`, `crate::virtio::*` — though most virtio / block
// callers are themselves UEFI-gated) need a `_print` symbol to resolve
// against. The stub swallows the format args; tests that observe stdout
// inject their own sink (see `syscall::write::do_write`'s `&mut dyn
// FnMut(&[u8])` parameter) so this stub is unreachable from the test
// runner. Lives here rather than in a sibling `arch::host` submodule
// because there's nothing else host-specific the kernel needs to publish
// — `init_console` / `init_memory` / `halt_forever` etc. are only ever
// called from the per-arch UEFI entry harnesses, never from
// crate-neutral code.
#[cfg(not(target_os = "uefi"))]
pub fn _print(_args: core::fmt::Arguments<'_>) {
    // No-op — host tests inject their own sinks where they care about
    // stdout. The presence of this symbol is what matters: it makes
    // `crate::print!` / `crate::println!` macro expansion type-check
    // on the host target so non-test code that reaches the macros
    // (e.g. `syscall::write`'s production sink) compiles cleanly.
}

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
