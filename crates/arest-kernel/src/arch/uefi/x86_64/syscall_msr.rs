// crates/arest-kernel/src/arch/uefi/x86_64/syscall_msr.rs
//
// Wire the SYSCALL/SYSRET MSRs so a `syscall` instruction issued
// from ring 3 traps into our entry stub (`syscall_entry::entry`).
// Third leg of the #552 ring-3 gate (paired with `gdt.rs` for
// segment descriptors and `tss.rs` for kernel stacks).
//
// What this module programs
// -------------------------
//   * IA32_LSTAR (0xC0000082) — target RIP for SYSCALL. The CPU
//     loads RIP from this MSR on every `syscall`. We point it at
//     `syscall_entry::entry`, the naked asm stub that saves user
//     state and dispatches into `crate::syscall::dispatch`.
//
//   * IA32_STAR  (0xC0000081) — segment selectors for SYSCALL /
//     SYSRET. Layout (Intel SDM Vol 3, 5.8.8):
//       bits 31:0  — IP for sysenter/sysexit (32-bit; long mode
//                    ignores).
//       bits 47:32 — SYSCALL CS / SS base. CPU loads CS from
//                    bits 47:32, SS from bits 47:32 + 8.
//       bits 63:48 — SYSRET CS / SS base. In 64-bit mode CPU loads
//                    CS from bits 63:48 + 16, SS from bits 63:48 + 8.
//
//     Per the task spec: STAR = (USER_CS - 16) << 48 | KERNEL_CS << 32.
//     With KERNEL_CS=0x08, USER_CS=0x1B: bits 47:32 = 0x08,
//     bits 63:48 = 0x0B.
//       SYSCALL: CS=0x08 (KERNEL_CS), SS=0x10 (KERNEL_DS) — matches
//         our GDT layout. ✓
//       SYSRET:  CS=0x0B+0x10=0x1B (USER_CS), SS=0x0B+0x08=0x13.
//         The SS would land on GDT slot 2 (kernel-DS) with RPL=3 —
//         a mismatch. We sidestep this by NOT calling SYSRET — the
//         syscall entry stub returns via IRETQ (which builds a
//         frame with USER_CS / USER_SS and pops it), the same gate
//         the trampoline uses on first entry. See `syscall_entry`
//         for the IRETQ return path.
//
//   * IA32_FMASK (0xC0000084) — RFLAGS mask applied on SYSCALL.
//     Bits SET in FMASK are CLEARED in RFLAGS on SYSCALL entry. We
//     set bit 9 (IF — interrupt flag) so the kernel's syscall
//     handler runs with interrupts disabled. Keep the value lean
//     (just IF) — masking too many bits would change the kernel's
//     observed RFLAGS state in subtle ways.
//
//   * IA32_EFER bit 0 (SCE — System Call Extensions). Setting this
//     bit enables the SYSCALL / SYSRET instructions; without it,
//     `syscall` traps as #UD. UEFI firmware on QEMU+OVMF leaves
//     this bit OFF; we flip it on. The other EFER bits (LME, LMA,
//     NXE) are already set by the firmware as part of the long-
//     mode bring-up — we use `Efer::update` to flip just SCE while
//     preserving the rest.
//
// Ordering
// --------
// Must run AFTER the GDT install (the kernel CS / DS selectors the
// MSRs reference must already be valid in the loaded GDT) and AFTER
// the TSS install (the syscall stub's RSP-switch path reads the
// TSS's RSP0 slot — which won't matter until the IDT entries get
// IST indexes wired in #553-followup, but the ordering invariant is
// cleanest if we never see a half-built TSS).
//
// Idempotency
// -----------
// `Once`-guarded — a second call is a no-op. The boot path calls
// this exactly once after `gdt::install` and `tss::install`.

use spin::Once;
use x86_64::registers::model_specific::{Efer, EferFlags, LStar, Msr, SFMask};
use x86_64::VirtAddr;

use super::gdt::{KERNEL_CS, USER_CS};

/// Once-guard so re-entrant `install` is a no-op.
static INSTALLED: Once<()> = Once::new();

/// IA32_STAR MSR address (per Intel SDM). The x86_64 crate ships a
/// `Star::write_raw` API but its `write` wrapper enforces SYSRET-CS
/// at base+16 with RPL=3 — which our task-spec layout (USER_CS=0x1B)
/// satisfies but only through the raw u16 path. Using the raw MSR
/// id directly avoids the type wrapper for clarity.
const IA32_STAR_MSR: u32 = 0xC0000081;

/// RFLAGS mask for SYSCALL: clear IF (bit 9) on entry so the
/// kernel-side SYSCALL handler runs with interrupts disabled.
/// Other bits (DF, TF, IOPL, etc.) we leave to the userspace
/// caller — the kernel handler itself doesn't depend on their
/// values.
pub const SYSCALL_RFLAGS_MASK: u64 = 0x200; // bit 9 = IF

/// Install the SYSCALL / SYSRET MSR programming. Pass the address
/// of the syscall entry stub (the `naked` asm fn in
/// `super::syscall_entry`) — we'll point IA32_LSTAR at it.
///
/// Idempotent — a second call is a no-op via the `INSTALLED` guard.
///
/// SAFETY: writes architecturally-mandated MSRs. The values are
/// computed from compile-time constants (`KERNEL_CS`, `USER_CS`)
/// and the caller-supplied `lstar` address; no userspace input
/// reaches here.
pub fn install(lstar: u64) {
    INSTALLED.call_once(|| {
        // SAFETY: `LStar::write` writes IA32_LSTAR. `lstar` is the
        // kernel-side asm stub's address — a valid 64-bit RIP in
        // the kernel-CS segment.
        LStar::write(VirtAddr::new(lstar));

        // IA32_STAR layout: bits 47:32 = SYSCALL CS, bits 63:48 =
        // SYSRET CS base. Build the 64-bit value by hand because
        // the x86_64 crate's `Star::write` enforces SYSRET CS +
        // SS-equality invariants that our task-spec layout
        // (USER_CS at index 3) doesn't satisfy directly — we work
        // around it by returning from syscalls via IRETQ rather
        // than SYSRETQ. See module docstring for the full story.
        let star_value: u64 =
            ((USER_CS.wrapping_sub(16)) as u64) << 48 | (KERNEL_CS as u64) << 32;
        // SAFETY: writing IA32_STAR is a one-shot configuration of
        // the SYSCALL / SYSRET segment selectors. The values match
        // the GDT layout `gdt::install` produced.
        unsafe {
            let mut star_msr = Msr::new(IA32_STAR_MSR);
            star_msr.write(star_value);
        }

        // IA32_FMASK — clear IF on SYSCALL entry. Other bits remain
        // at the userspace value. `SFMask::write` is safe in the
        // x86_64 crate's API — it doesn't take an unsafe block.
        SFMask::write(x86_64::registers::rflags::RFlags::from_bits_truncate(
            SYSCALL_RFLAGS_MASK,
        ));

        // EFER.SCE — enable SYSCALL / SYSRET. Preserve the other
        // EFER bits (LME / LMA / NXE) the firmware set during long-
        // mode bring-up.
        // SAFETY: `Efer::update` reads the current EFER value,
        // ORs in the SCE bit, writes it back. We're only flipping
        // SCE on; the LME / LMA bits stay set so long mode stays
        // active.
        unsafe {
            Efer::update(|f| {
                f.insert(EferFlags::SYSTEM_CALL_EXTENSIONS);
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FMASK value clears bit 9 (IF) only. Adding more bits would
    /// change the kernel's observed RFLAGS on every syscall in
    /// subtle ways — pin the constant.
    #[test]
    fn syscall_rflags_mask_clears_only_if() {
        assert_eq!(SYSCALL_RFLAGS_MASK, 0x200);
        assert_eq!(SYSCALL_RFLAGS_MASK & (1 << 9), 1 << 9, "IF bit must be set in mask");
        // Verify no other commonly-mistaken bits sneak in.
        assert_eq!(SYSCALL_RFLAGS_MASK & (1 << 1), 0, "reserved bit 1 must NOT be set");
        assert_eq!(SYSCALL_RFLAGS_MASK & (1 << 10), 0, "DF bit must NOT be set");
    }

    /// IA32_STAR MSR id matches the architecturally-defined value.
    /// 0xC000_0081 is the canonical address per Intel SDM Vol 4
    /// MSR table. Drifting from this would silently break SYSCALL
    /// on every CPU.
    #[test]
    fn ia32_star_msr_id_matches_spec() {
        assert_eq!(IA32_STAR_MSR, 0xC000_0081);
    }
}
