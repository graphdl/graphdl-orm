// crates/arest-kernel/src/arch/aarch64/mod.rs
//
// aarch64 UEFI arch arm (#344 cross-arch scaffold). Companion to the
// x86_64 `arch::uefi` arm — gated on
// `cfg(all(target_arch = "aarch64", target_os = "uefi"))` from
// `arch/mod.rs`. Exists so `cargo check --target aarch64-unknown-uefi`
// compiles the whole kernel crate end-to-end without needing to touch
// the x86_64 bring-up paths.
//
// Scope of THIS commit (compile-only scaffold):
//   * `_print(args)` — PL011 MMIO writer at 0x0900_0000 (QEMU virt
//     machine's standard UART address). See `serial.rs`.
//   * `init_console()` — no-op placeholder (PL011 needs no init on
//     QEMU virt; firmware leaves it enabled).
//   * `halt_forever()` — `wfi`-based idle loop. Aarch64's analogue of
//     x86's `hlt`; stops the core until an interrupt fires.
//
// Deliberately NOT here (tracked for follow-up commits matching the
// x86_64 arm's step-by-step progression):
//   * `init_memory(memory_map)` — would consume UEFI's memory map and
//     build whatever page-table abstraction the aarch64 kernel body
//     ends up with. Blocked on the kernel's arch-neutral paging trait.
//   * `init_gdt_and_interrupts` equivalent — on aarch64 this becomes
//     EL exception vector table setup (VBAR_EL1) + GICv2/v3 IRQ
//     routing. Lands when IRQ-driven subsystems come online.
//   * `enable_sse` equivalent — aarch64 has NEON/SIMD enabled by
//     default under UEFI, so the CR0/CR4 flip the x86 arm performs
//     has no analogue. If later kernel deps need it, add a
//     CPACR_EL1 FP-enable wrapper here.
//   * ExitBootServices cutover — `arch::uefi` uses a POST_EBS atomic
//     to swap ConOut for direct-I/O 16550; this arm writes PL011 MMIO
//     directly from the start, so no swap is needed for the banner.
//     A future `switch_to_post_ebs_serial` hook can still land if
//     post-EBS serial ever needs different behavior (e.g. after the
//     firmware maps a different virtual address over the UART).
//
// Design note on why this is a sibling of `arch::uefi` rather than a
// rename of it: `arch::uefi` today is x86_64-specific (16550 UART,
// x86_64 port I/O, CR0/CR4 control). Renaming it `arch::uefi_x86_64`
// would ripple through `arch/mod.rs` re-exports and every `arch::uefi::`
// use site. Adding `arch::aarch64` gated on
// `cfg(all(target_arch = "aarch64", target_os = "uefi"))` and gating
// the existing `arch::uefi` on `cfg(all(target_arch = "x86_64", target_os = "uefi"))`
// in `arch/mod.rs` is a narrower change — and `arch/mod.rs`'s glob
// re-export `pub use aarch64::*` gives the shared kernel body the
// same `arch::_print` / `arch::halt_forever` shape regardless of
// which arm is active.

mod serial;

// `_print` is the callee of the crate-wide `print!` / `println!`
// macros (declared in `arch/mod.rs`). Today the scaffold writes its
// banner via `raw_puts` directly, so no `println!` call site is
// reachable on aarch64-uefi — but keeping `_print` re-exported means
// any shared kernel module that drops its target gate and starts
// calling `println!` resolves immediately, rather than tripping on
// a missing `arch::_print`. Marked `#[allow(unused_imports)]` to
// silence the "unused" warning until that first `println!` call
// site lands.
#[allow(unused_imports)]
pub use serial::{_print, raw_puts};

/// Initialise the architecture's console. Under UEFI on aarch64 the
/// PL011 is live from firmware-boot (QEMU virt keeps it enabled
/// across hand-off), so this is a no-op. Kept as the named entry
/// point so the shared kernel body can call `arch::init_console()`
/// target-agnostically once it drops its x86_64-only gates.
pub fn init_console() {
    // Intentionally empty — see module docstring.
}

/// Drive the kernel's idle loop. `wfi` (Wait For Interrupt) is the
/// aarch64 analogue of x86's `hlt` — suspends the core until the
/// next IRQ / FIQ / SError arrives. With no IRQ infrastructure yet
/// the loop parks forever, which is the correct behavior for the
/// banner-only scaffold: the smoke harness verifies the boot banner
/// and expects the kernel to stay alive.
///
/// Once follow-up commits install a vector table + GIC, the loop
/// becomes wakeup-driven and callers of `arch::halt_forever` start
/// seeing real IRQ-driven progress.
pub fn halt_forever() -> ! {
    loop {
        // SAFETY: `wfi` is unprivileged in EL1 and has no side effects
        // beyond pausing instruction fetch until the next interrupt.
        // `nomem` + `nostack` + `preserves_flags` describe it
        // accurately to the compiler.
        unsafe {
            core::arch::asm!(
                "wfi",
                options(nomem, nostack, preserves_flags),
            );
        }
    }
}
