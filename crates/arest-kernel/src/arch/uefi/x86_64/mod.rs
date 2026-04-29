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
//   * `entropy`       — RDSEED + RDRAND-backed `EntropySource` for
//                        `arest::entropy`'s global slot (#569). Probes
//                        CPUID at construction; `install_entropy()`
//                        below registers it during boot so `csprng`
//                        / AT_RANDOM / `getrandom` see real-random
//                        bytes from the silicon RNG.
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

pub mod entropy;
pub mod efi_rng;
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

/// Register the x86_64 hardware entropy source (RDSEED with RDRAND
/// fallback) into `arest::entropy`'s process-wide slot (#569). After
/// this returns, `arest::csprng::random_bytes` — and every downstream
/// consumer (AT_RANDOM stack canary in #575, `getrandom` syscall in
/// #577, etc.) — sees real-random bytes from the silicon RNG instead
/// of the deterministic placeholder the slot defaults to.
///
/// The CPUID probe runs once during construction, so calling this
/// from boot has constant cost regardless of whether the host CPU
/// has RDSEED. On vintage silicon with neither instruction, the
/// installed source still resolves but every `fill()` reports
/// `EntropyError::HardwareUnavailable` — leaving the door open for
/// the UEFI EFI_RNG_PROTOCOL fallback (#571) to chain in.
///
/// Idempotent at the install-side level (re-installing replaces the
/// previously-registered source per `arest::entropy::install`'s
/// docstring), but the boot path calls this exactly once.
pub fn install_entropy() {
    install_entropy_with_seed(None)
}

/// Variant that chains the firmware-captured `EFI_RNG_PROTOCOL` seed
/// (#571) onto the silicon path. When `boot_seed` is `Some`, hardware
/// faults fall through to a stretched keystream derived from the
/// firmware-provided 32 bytes — preventing the `csprng::seed_from_entropy`
/// panic that takes down POST `/arest/entity` (#614) on QEMU.
///
/// The seed must be captured pre-`boot::exit_boot_services` (see
/// `efi_rng::capture_boot_seed`); this function is the post-EBS sink.
pub fn install_entropy_with_seed(boot_seed: Option<[u8; efi_rng::SEED_LEN]>) {
    let primary = entropy::X86_64HwEntropy::new();
    let fallback = boot_seed.map(entropy::BootSeedEntropy::new);
    arest::entropy::install(alloc::boxed::Box::new(
        entropy::ChainedEntropy::new(primary, fallback),
    ));
}
