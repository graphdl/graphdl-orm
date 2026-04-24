// crates/arest-kernel/src/entry_uefi_aarch64.rs
//
// aarch64-unknown-uefi entry point (#344 cross-arch). Sibling of
// `entry_uefi.rs` (x86_64-unknown-uefi). Split into two files because
// the two arms diverge on the panic handler (x86_64 uses raw port I/O
// to COM1; aarch64 uses raw MMIO to PL011 at 0x0900_0000) and because
// the pre-EBS heap + SSE init that entry_uefi.rs runs has no aarch64
// analogue yet — the aarch64 arm is a compile-only scaffold, halting
// via `wfi` after printing a banner.
//
// Scope of THIS commit (compile-only scaffold):
//   * `efi_main` — print a banner via `arch::raw_puts` → PL011 MMIO.
//   * `panic` — print a one-line fault marker via PL011, then `wfi` loop.
//
// Deliberately NOT here (matching the x86_64 arm's step-by-step
// progression):
//   * Heap init (no static-BSS LockedHeap allocator yet — no code
//     below the banner needs `alloc::` on this path).
//   * ExitBootServices. Firmware services stay live through the
//     `wfi` loop, which is fine for a banner-only boot — the smoke
//     harness (once its Dockerfile.uefi-aarch64 variant lands)
//     only cares that the banner reaches QEMU's `-serial stdio`.
//   * GetMemoryMap consumption, IDT-equivalent vector table, etc —
//     land alongside the matching x86_64 step once the kernel body
//     has an arch-neutral path that doesn't pull in x86_64 ISA
//     helpers.
//
// Gated on `cfg(all(target_os = "uefi", target_arch = "aarch64"))`
// and lives behind a `mod entry_uefi_aarch64;` in `main.rs` guarded
// by the same cfg so a `cargo check --target x86_64-unknown-uefi`
// ignores it entirely.

#![cfg(all(target_os = "uefi", target_arch = "aarch64"))]

use core::alloc::{GlobalAlloc, Layout};
use uefi::prelude::*;

// Rust's `extern crate alloc;` in `main.rs` requires a
// `#[global_allocator]` to exist, even if no allocation ever
// happens. The x86_64 UEFI entry (`entry_uefi.rs`) supplies a
// real `LockedHeap` over a static-BSS byte array; the aarch64
// scaffold has no allocation-requiring callers reachable from
// `efi_main`, so a panic-on-call "bump" allocator is enough to
// satisfy the linker without growing the binary.
//
// If any allocation IS attempted (e.g. a future caller slips a
// `Vec::new()` into the banner path), the panic handler below
// catches it and writes a visible fault marker to PL011 before
// the `wfi` loop takes over — far better than a silent linker
// error or a UB-ing null pointer.
struct PanicOnAlloc;

unsafe impl GlobalAlloc for PanicOnAlloc {
    unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
        panic!("aarch64-uefi scaffold: no heap (global_allocator not initialised)")
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        panic!("aarch64-uefi scaffold: no heap (dealloc called without alloc)")
    }
}

#[global_allocator]
static ALLOCATOR: PanicOnAlloc = PanicOnAlloc;

/// UEFI entry point for the aarch64 target. `uefi-rs`'s `#[entry]`
/// expands this into the PE32+ `_start` symbol the firmware invokes.
///
/// Prints a fixed banner via `arch::raw_puts` (which writes directly
/// to PL011 UARTDR at 0x0900_0000) and parks in a `wfi` loop.
/// Deliberately avoids firmware ConOut — on aarch64-unknown-uefi,
/// ConOut sometimes routes through a graphics console that the smoke
/// harness's `-serial stdio` listener doesn't see, whereas raw PL011
/// MMIO always reaches the host terminal.
#[entry]
fn efi_main() -> Status {
    // Three-line banner. Fixed strings — no `println!` yet because
    // that routes through `arch::_print`, which goes through the
    // same PL011 path anyway but via `core::fmt::write` + the
    // `Arguments` formatter. For the scaffold's very first line the
    // raw-puts path gives the firmware as little to go wrong as
    // possible: a linear loop of volatile byte writes against a
    // physical address that's guaranteed-mapped on QEMU virt.
    crate::arch::raw_puts("AREST kernel - aarch64-UEFI scaffold\r\n");
    crate::arch::raw_puts("  target: aarch64-unknown-uefi\r\n");
    crate::arch::raw_puts("  next:   ExitBootServices + memory map (follow-ups)\r\n");

    // Halt via wfi loop. Returns `!`, so the `Status` return on the
    // `#[entry]` fn is unreachable — uefi-rs's macro expands the
    // signature check anyway; halt_forever's divergence satisfies
    // both the compiler and the firmware's caller convention.
    crate::arch::halt_forever();
}

/// Panic handler for the aarch64 UEFI path. The x86_64 arm's
/// `entry_uefi.rs` panic handler raw-I/Os COM1 at 0x3F8; here we do
/// the same thing against PL011 MMIO at 0x0900_0000 so a fault
/// surfaces as a visible "!! UEFI kernel panic !!" marker rather
/// than a silent hang.
///
/// Uses a stack-local writer targeting the same PL011 UARTDR the
/// banner writes to — no alloc dependency (so a panic inside an
/// allocator hook can't deadlock), no singleton (so a panic
/// mid-mutation of a future serial-state struct can't fight it).
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;

    /// QEMU virt PL011 UARTDR address. Duplicated from
    /// `arch::aarch64::serial` so the panic path has zero module
    /// dependencies — if an import is what caused the panic, the
    /// fault marker still gets out.
    const UARTDR: *mut u8 = 0x0900_0000 as *mut u8;

    struct RawPl011;
    impl Write for RawPl011 {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            for b in s.bytes() {
                // SAFETY: UARTDR is the QEMU virt PL011 data register,
                // identity-mapped by firmware. Writes are stateless
                // MMIO with no memory-safety impact.
                unsafe { UARTDR.write_volatile(b) };
            }
            Ok(())
        }
    }

    let mut w = RawPl011;
    let _ = w.write_str("\r\n!! UEFI kernel panic (aarch64) !!\r\n");
    let _ = writeln!(w, "{info}");

    loop {
        // SAFETY: `wfi` is unprivileged in EL1 and has no side
        // effects beyond pausing until the next interrupt. `nomem` /
        // `nostack` / `preserves_flags` describe it accurately.
        unsafe {
            core::arch::asm!(
                "wfi",
                options(nomem, nostack, preserves_flags),
            );
        }
    }
}
