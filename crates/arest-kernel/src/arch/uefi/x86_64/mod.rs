// crates/arest-kernel/src/arch/uefi/x86_64/mod.rs
//
// x86_64-specific subsidiary modules for the UEFI arch arm. Lives
// under `arch/uefi/x86_64/` rather than directly under `arch/uefi/`
// because the modules here speak architecturally-x86 features
// (GDT / TSS / SYSCALL MSRs) that have aarch64 / armv7 analogues
// but ship as separate modules in those arms (GICv3 / EL0 vs.
// LDT-derived equivalents). Sister modules — `interrupts`,
// `keyboard`, `memory`, `pointer`, `serial`, `slint_*`, `time` —
// also live under `arch/uefi/` directly because, while x86_64-only
// today, they have direct aarch64 / armv7 equivalents in the
// existing tree (the IDT shape is mostly the same, the PL011 / 16550
// split is named per-arm in `serial.rs`, etc.). The four modules
// here have no aarch64 / armv7 analogue YET; nesting them under
// `x86_64/` makes that explicit at the directory level.
//
// Modules
// -------
//   * `gdt`           — Global Descriptor Table with kernel + user
//                        code/data segments (KERNEL_CS / KERNEL_DS /
//                        USER_CS / USER_SS).
//   * `tss`           — Task State Segment with RSP0 + IST stacks.
//   * `syscall_msr`   — IA32_LSTAR / IA32_STAR / IA32_FMASK / EFER
//                        programming.
//   * `syscall_entry` — naked asm stub IA32_LSTAR points at; saves
//                        user state, calls the dispatcher, returns
//                        via IRETQ.
//
// Boot path
// ---------
// `install_userspace_gate()` orchestrates the four modules in the
// right order:
//   1. `tss::install()`         — allocate stacks, build TSS.
//   2. `gdt::install(tss)`      — build GDT + TSS descriptor, lgdt,
//                                  reload selectors, ltr.
//   3. `syscall_msr::install(syscall_entry as u64)`
//                                — wire LSTAR / STAR / FMASK / EFER.
// After this returns, ring 3 is reachable via IRETQ in
// `process::trampoline::invoke`.
//
// Idempotency
// -----------
// Each sub-module's `install()` is `Once`-guarded; calling
// `install_userspace_gate()` multiple times is a no-op after the
// first invocation. The boot path calls it exactly once, but the
// guard means the unit tests can drive it from multiple `#[test]`
// fns without conflict.

pub mod gdt;
pub mod syscall_entry;
pub mod syscall_msr;
pub mod tss;

/// Install the GDT, TSS, and SYSCALL MSRs in the order required for
/// the kernel to be able to drop to ring 3 + receive `syscall`
/// instructions back. Idempotent — `Once`-guarded internally.
///
/// Must run AFTER the global allocator is live (the TSS module
/// calls `alloc::alloc::alloc` for its IST stacks) and AFTER the
/// firmware has surrendered ExitBootServices (the GDT install
/// invalidates any firmware-installed segment selectors the
/// post-EBS body might still rely on; firmware code paths can no
/// longer be reached after this point).
pub fn install_userspace_gate() {
    // 1. Build the TSS first so the GDT install can append a
    //    `tss_segment` descriptor pointing at it.
    let tss = tss::install();
    // 2. Build the GDT (5 segment descriptors + TSS descriptor),
    //    `lgdt` it, reload CS / DS / ES / SS, `ltr` the TSS.
    let _tss_selector = gdt::install(tss);
    // 3. Wire the SYSCALL MSRs so a `syscall` from ring 3 traps
    //    into our entry stub.
    syscall_msr::install(syscall_entry::syscall_entry as *const () as u64);
}
