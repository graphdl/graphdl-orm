// crates/arest-kernel/src/arch/armv7/mod.rs
//
// armv7 UEFI arch arm (#346 cross-arch scaffold). Sibling of the
// aarch64 UEFI arm (`arch::aarch64`) and the x86_64 UEFI arm
// (`arch::uefi`). Gated on
// `cfg(all(target_os = "uefi", target_arch = "arm"))` from
// `arch/mod.rs`. Exists so a `cargo build --target
// arest-kernel-armv7-uefi.json -Z build-std=core,compiler_builtins,alloc`
// compiles the kernel crate end-to-end without needing to touch the
// existing x86_64 / aarch64 paths.
//
// Why a separate arm rather than a shared `arch::arm`: aarch64 and
// armv7 are distinct ISAs (32-bit vs 64-bit pointer width, different
// register banks, different interrupt models). They share a few
// peripherals on QEMU virt — the PL011 UART address is identical
// (0x0900_0000 on both), and the virt machine layout is similar —
// but the EL/exception model, page-table format, and ABI all
// diverge. Mirroring the file shape rather than glob-re-exporting is
// the same pattern `arch::uefi` (x86_64) and `arch::aarch64` use.
//
// Why not the built-in `armv7a-none-eabi` rust target: that target
// produces ELF for a bare-metal loader that doesn't exist in our
// pipeline (we'd need to write our own `_start` + boot stub). UEFI
// gives us PE32+ + firmware-managed paging + ConIn/ConOut + memory
// map + identity-mapped PL011 MMIO for free — same firmware contract
// the aarch64-unknown-uefi arm rides. Since rustc has no
// `armv7-unknown-uefi` built-in (rustc's UEFI targets are 64-bit
// only), this arm builds against a custom target JSON shipped in
// `crates/arest-kernel/arest-kernel-armv7-uefi.json` — `arch:"arm"`,
// 32-bit pointer width, MSVC linker flavor with `/machine:arm`,
// thumb2 features for `wfi`. Bundle the JSON path with `--target
// crates/arest-kernel/arest-kernel-armv7-uefi.json -Z
// build-std=core,compiler_builtins,alloc` to produce a `.efi` that
// QEMU's ArmVirtPkg firmware can boot.
//
// Scope of THIS commit (compile-only scaffold):
//   * `_print(args)` — PL011 MMIO writer at 0x0900_0000 (QEMU's `virt`
//     machine exposes the PL011 at the same address on both the
//     32-bit and 64-bit ARM virt variants). See `serial.rs`.
//   * `init_console()` — no-op placeholder (PL011 needs no init on
//     QEMU virt; firmware leaves it enabled across hand-off).
//   * `halt_forever()` — `wfi`-based idle loop. armv7 has `wfi` as a
//     thumb2 mnemonic (ARM ARM B1.10.7) — same instruction the
//     aarch64 arm uses, just emitted under a different ISA encoding.
//   * `runtime_stub` — a no-op `#[global_allocator]` + `#[panic_handler]`
//     pair so `cargo build --target armv7-...` links without a real
//     runtime harness. Both items are SCAFFOLDING and get replaced
//     wholesale by `entry_uefi_armv7.rs` when #346d lands. See
//     `runtime_stub.rs` for the full rationale.
//   * `msvc_shims` — MSVC-ARM CRT helper symbols (`__rt_udiv`,
//     `__chkstk`, `__u64tod`, ...) re-routed to compiler_builtins's
//     AEABI / cross-platform names. Required because rust-lld
//     emits these libcalls under `is-like-msvc + arch=arm` and the
//     standard sysroot doesn't ship a vcruntime import lib for the
//     ARM-UEFI environment. See `msvc_shims.rs` for the full list
//     and per-symbol rationale.
//
// Deliberately NOT here (tracked for follow-up commits matching the
// aarch64 arm's step-by-step progression):
//   * `init_memory(memory_map)` — UEFI memory-map → frame allocator
//     + DMA pool. Lands in #346b alongside the aarch64-shape memory
//     module (parallel to `arch::aarch64::memory`).
//   * `virtio_mmio` transport — UEFI virtio-mmio scanner + driver.
//     Lands in #346c.
//   * `entry_uefi_armv7.rs` runtime harness — pre/post-EBS banner,
//     `boot::exit_boot_services`, and the `panic_handler` that
//     surfaces faults via PL011. Lands in #346d.
//   * `init_gdt_and_interrupts` analogue — armv7 vector base address
//     (VBAR) + GIC routing. Lands when IRQ-driven subsystems come
//     online.
//   * NEON/VFP enable — armv7 has VFPv3 / NEON gated behind CPACR;
//     UEFI on QEMU virt leaves them disabled by default. The custom
//     target JSON ships `+soft-float` so no FP instructions are
//     emitted today; if a later kernel dep needs hardware FP, add a
//     CPACR-write helper here parallel to `arch::uefi::enable_sse`.

mod msvc_shims;
mod runtime_stub;
mod serial;

// `_print` is the callee of the crate-wide `print!` / `println!`
// macros (declared in `arch/mod.rs`). The scaffold has no
// `println!` call site reachable on armv7-uefi yet (the runtime
// harness lands in #346d), but keeping `_print` re-exported means
// any shared kernel module that drops its target gate and starts
// calling `println!` resolves immediately, rather than tripping on
// a missing `arch::_print`. `#[allow(unused_imports)]` silences the
// "unused" warning until that first `println!` call site lands.
#[allow(unused_imports)]
pub use serial::{_print, raw_puts};

/// Initialise the architecture's console. Under UEFI on armv7 the
/// PL011 is live from firmware-boot (QEMU virt keeps it enabled
/// across hand-off — same behavior as the aarch64 virt variant), so
/// this is a no-op. Kept as the named entry point so the shared
/// kernel body can call `arch::init_console()` target-agnostically
/// once it drops its x86_64-only gates.
pub fn init_console() {
    // Intentionally empty — see module docstring.
}

/// Drive the kernel's idle loop. `wfi` (Wait For Interrupt) is the
/// armv7 analogue of x86's `hlt` — suspends the core until the next
/// IRQ / FIQ / abort arrives. armv7 has `wfi` natively in both ARM
/// and thumb2 encodings (ARM ARM B1.10.7); the inline-asm mnemonic
/// matches the aarch64 arm character-for-character — only the
/// underlying machine encoding differs.
///
/// With no IRQ infrastructure yet the loop parks forever, which is
/// the correct behavior for the banner-only scaffold the runtime
/// harness (#346d) will land: the smoke verifies the boot banner
/// and expects the kernel to stay alive.
///
/// Once follow-up commits install a vector table + GIC, the loop
/// becomes wakeup-driven and callers of `arch::halt_forever` start
/// seeing real IRQ-driven progress.
pub fn halt_forever() -> ! {
    loop {
        // SAFETY: `wfi` is unprivileged in PL1 (kernel mode) on
        // armv7 and has no side effects beyond pausing instruction
        // fetch until the next interrupt. `nomem` + `nostack` +
        // `preserves_flags` describe it accurately to the compiler.
        unsafe {
            core::arch::asm!(
                "wfi",
                options(nomem, nostack, preserves_flags),
            );
        }
    }
}
