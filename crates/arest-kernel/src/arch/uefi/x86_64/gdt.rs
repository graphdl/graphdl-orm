// crates/arest-kernel/src/arch/uefi/x86_64/gdt.rs
//
// Kernel-owned Global Descriptor Table for the UEFI x86_64 path
// (#552, the "ring-3 gate" foundation #521 needs to actually drop a
// loaded ELF binary into ring 3). Replaces the firmware-inherited
// GDT — which carries no DPL=3 (user) descriptors and no TSS — with
// a 5-entry table that:
//
//   * Index 0 — null. Required by the architecture.
//   * Index 1 — kernel CS. DPL=0, 64-bit code, L=1. Selector 0x08.
//     The kernel keeps running here after `lgdt`. SYSCALL loads
//     CS from STAR[47:32]; we set STAR[47:32]=0x08 in
//     `arch::uefi::x86_64::syscall_msr` so the SYSCALL entry-stub
//     resumes in this segment.
//   * Index 2 — kernel DS / SS. DPL=0, data. Selector 0x10. Loaded
//     into DS / ES / SS after the GDT switch (FS / GS keep their
//     firmware-supplied bases until the syscall MSR setup overwrites
//     IA32_FS_BASE / IA32_GS_BASE — which it doesn't yet, since the
//     kernel hasn't grown per-CPU storage).
//   * Index 3 — user CS. DPL=3, 64-bit code, L=1. Selector 0x1B
//     (0b0001_1011 — index 3, GDT, RPL 3).
//     The trampoline pushes this in the `iretq` frame's CS slot to
//     drop the CPU into ring 3. The selector RPL must be 3 — the
//     CPU's `iretq` checks the new CS RPL against the new CPL it
//     would adopt.
//   * Index 4 — user DS / SS. DPL=3, data. Selector 0x23
//     (0b0010_0011 — index 4, GDT, RPL 3).
//     The trampoline pushes this in the `iretq` frame's SS slot.
//
// What `install()` does
// ---------------------
// 1. Build the 5-entry table in a `'static` `Once`-guarded slot so
//    the `lgdt` reference stays valid for the rest of boot.
// 2. `lgdt` it.
// 3. Far-return through the new kernel CS via the standard
//    `pushq KERNEL_CS; lea 1f(%rip), %rax; pushq %rax; lretq; 1:`
//    sequence (the only way to reload CS in long mode — `mov` to CS
//    is illegal). We go through the x86_64 crate's `CS::set_reg`
//    helper which does exactly this.
// 4. Reload DS, ES, SS to KERNEL_DS. FS / GS are managed via their
//    base MSRs, not selectors, in long mode — we leave them alone.
//
// Note on SYSRET and the GDT layout
// ---------------------------------
// The task spec mandates the layout USER_CS=0x1B / USER_SS=0x23
// (user-CS at index 3, user-DS at index 4). That layout is the
// classic IRETQ-friendly convention but is NOT compatible with
// SYSRET — SYSRET wants the user CS at base+16 and user SS at
// base+8 from a single STAR base, which forces user-DS to come
// BEFORE user-CS in the GDT (the inverse of what we have). We
// resolve this by:
//
//   * Programming STAR's SYSRET base via the formula the task
//     specifies — (USER_CS - 16) << 48. SYSRET would load CS=USER_CS
//     correctly but SS would land on index 2 (kernel-DS) with RPL=3
//     — incompatible with the kernel-DS DPL=0 descriptor.
//   * NOT using SYSRET to return from syscalls — `syscall_entry` in
//     the sibling module returns via `iretq` (which builds a frame
//     with USER_CS / USER_SS and pops it to ring 3, the same gate
//     the trampoline uses on first entry). This sidesteps the SS
//     mismatch entirely.
//
// Future work could re-shuffle the GDT to a SYSRET-friendly layout
// (user-DS before user-CS), update the constants here, and switch
// the syscall stub to SYSRETQ for a faster return path. Tier-1 just
// wants the path to work.
//
// Why a fixed-size `[Entry; 5]` rather than `GlobalDescriptorTable`
// -----------------------------------------------------------------
// The x86_64 crate's `GlobalDescriptorTable<MAX>` defaults `MAX = 8`.
// That works fine — we use it directly. The 5-entry footprint is
// driven by the spec: null + kernel CS + kernel DS + user CS + user
// DS. No TSS slot here — the TSS lives in `tss.rs` and gets appended
// to the GDT separately so the TSS module can own its descriptor's
// lifecycle (the TSS descriptor needs the TSS's stable address,
// which the TSS module computes after stack allocation).
//
// Idempotency
// -----------
// `install()` is `Once`-guarded — a second call is a no-op. The
// boot path calls it exactly once from `arch::uefi::install_x86_64
// _userspace_gate` (this module's parent). The TSS module calls
// `append_tss_descriptor` on the GDT after `install` runs — that
// path is also `Once`-guarded.

use spin::Once;
use x86_64::instructions::segmentation::{Segment, CS, DS, ES, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::PrivilegeLevel;

/// Kernel code-segment selector. GDT index 1, RPL=0.
/// `0x08 = 0b0000_1000` — index 1, GDT, RPL 0.
pub const KERNEL_CS: u16 = 0x08;

/// Kernel data-segment selector. GDT index 2, RPL=0.
/// `0x10 = 0b0001_0000` — index 2, GDT, RPL 0.
pub const KERNEL_DS: u16 = 0x10;

/// User code-segment selector. GDT index 3, RPL=3.
/// `0x1B = 0b0001_1011` — index 3, GDT, RPL 3. The bottom three
/// bits (0b011) are RPL=3 + TI=0; the CPU's `iretq` check refuses
/// any other RPL on a ring-0 → ring-3 transition.
pub const USER_CS: u16 = 0x1B;

/// User stack/data-segment selector. GDT index 4, RPL=3.
/// `0x23 = 0b0010_0011` — index 4, GDT, RPL 3.
pub const USER_SS: u16 = 0x23;

/// Combined GDT + TSS descriptor selector. We build the table
/// in one shot inside `install()` so the `Once` guards both the
/// 5 segment descriptors (built unconditionally on every call)
/// and the TSS descriptor (which depends on the supplied static
/// TSS reference). The selector is the value `ltr` needs.
///
/// `Once` keeps the value pinned in `.bss` so the CPU's `lgdt`-
/// loaded pointer stays valid for the rest of boot.
static GDT_AND_TSS: Once<GdtBundle> = Once::new();

/// Once-guard so the boot path can call `install()` from anywhere
/// (the entry harness for the production path, the unit-test
/// harness for the host build) without double-loading.
static INSTALLED: Once<()> = Once::new();

/// Pre-built GDT + cached TSS descriptor selector. Constructed once
/// from the supplied `&'static TaskStateSegment`; the GDT lives at
/// the static's address for the rest of boot.
struct GdtBundle {
    gdt: GlobalDescriptorTable,
    tss_selector: SegmentSelector,
}

/// SAFETY: `GlobalDescriptorTable` carries no interior mutability
/// beyond the boot-time build; `SegmentSelector` is `Copy`. After
/// `Once::call_once` populates the bundle, neither field is
/// mutated again.
unsafe impl Sync for GdtBundle {}

/// Build the GDT, `lgdt` it, and reload CS / DS / ES / SS to the
/// kernel selectors, then `ltr` the TSS. After this returns:
///   * CS = KERNEL_CS (0x08)
///   * DS / ES / SS = KERNEL_DS (0x10)
///   * FS / GS untouched (their BASE MSRs remain firmware-set)
///   * The GDT contains slots 0..=4 populated with segment
///     descriptors and slots 5..=6 populated with the TSS
///     descriptor's two halves; slot 7 is the null placeholder
///     from `GlobalDescriptorTable::empty()`.
///
/// Idempotent — a second call is a no-op via the `INSTALLED` guard.
///
/// SAFETY: programs CR0/CR3-adjacent CPU state. The GDT pointer is
/// `&'static`; the segment selectors loaded match the descriptors
/// the GDT carries; the far-return through CS uses `CS::set_reg`'s
/// canonical `pushq sel; lea rip+1f; pushq; retfq; 1:` shape.
pub fn install(tss: &'static TaskStateSegment) -> SegmentSelector {
    let bundle = GDT_AND_TSS.call_once(|| {
        let mut g = GlobalDescriptorTable::new();
        // Slot 1 (selector 0x08) — kernel CS, DPL=0, 64-bit code.
        let kernel_cs = g.append(Descriptor::kernel_code_segment());
        // Slot 2 (selector 0x10) — kernel DS, DPL=0, data.
        let kernel_ds = g.append(Descriptor::kernel_data_segment());
        // Slot 3 (selector 0x1B) — user CS, DPL=3, 64-bit code.
        let user_cs = g.append(Descriptor::user_code_segment());
        // Slot 4 (selector 0x23) — user DS, DPL=3, data.
        let user_ds = g.append(Descriptor::user_data_segment());
        // Sanity-check the resulting selectors match the documented
        // constants — if these ever drift, every syscall and every
        // iretq breaks.
        debug_assert_eq!(kernel_cs.0, KERNEL_CS);
        debug_assert_eq!(kernel_ds.0, KERNEL_DS);
        debug_assert_eq!(user_cs.0, USER_CS);
        debug_assert_eq!(user_ds.0, USER_SS);
        // Slots 5+6 — TSS descriptor (SystemSegment, takes 2 entries).
        // The selector points at slot 5; `ltr` consumes it.
        // SAFETY: `tss` is `&'static`, satisfying
        // `Descriptor::tss_segment`'s lifetime requirement.
        let tss_selector = g.append(Descriptor::tss_segment(tss));
        GdtBundle {
            gdt: g,
            tss_selector,
        }
    });
    let tss_selector = bundle.tss_selector;

    INSTALLED.call_once(|| {
        // SAFETY: the GDT lives in `.bss`-backed static storage
        // (the `Once` keeps it pinned for `'static`). The `lgdt`
        // pointer remains valid for the rest of the kernel's
        // lifetime.
        bundle.gdt.load();
        // Reload CS via a far return. `CS::set_reg` knows the
        // x86_64 trick — push the new CS selector + a return RIP,
        // execute `retfq`. We can't use `mov to CS` in long mode.
        // SAFETY: KERNEL_CS points at the kernel-code descriptor we
        // just installed (DPL=0, code, L=1). After the far-return
        // the CPU is still in ring 0 with the same RIP +1 (the
        // jump target the helper computes).
        unsafe {
            CS::set_reg(SegmentSelector::new(1, PrivilegeLevel::Ring0));
        }
        // Reload DS / ES / SS to KERNEL_DS. The `mov` to data
        // segments is allowed in long mode (unlike CS).
        // SAFETY: KERNEL_DS points at the kernel-data descriptor we
        // just installed (DPL=0, data).
        unsafe {
            let kernel_ds = SegmentSelector::new(2, PrivilegeLevel::Ring0);
            DS::set_reg(kernel_ds);
            ES::set_reg(kernel_ds);
            SS::set_reg(kernel_ds);
        }
        // Load the TSS via `ltr`. The TSS lives in module-static
        // storage; the descriptor we just appended to the GDT
        // points at it.
        // SAFETY: `tss_selector` is the result of
        // `gdt.append(Descriptor::tss_segment(tss))` and matches a
        // valid TSS descriptor in the loaded GDT.
        unsafe {
            load_tss(tss_selector);
        }
    });

    tss_selector
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Selector constants encode the documented (index, RPL) pairs.
    /// `0x08 = index 1 RPL 0`, `0x10 = index 2 RPL 0`, `0x1B = index
    /// 3 RPL 3`, `0x23 = index 4 RPL 3`.
    #[test]
    fn kernel_cs_selector_layout() {
        assert_eq!(KERNEL_CS & 0b111, 0, "KERNEL_CS RPL must be 0");
        assert_eq!(KERNEL_CS >> 3, 1, "KERNEL_CS index must be 1");
    }

    #[test]
    fn kernel_ds_selector_layout() {
        assert_eq!(KERNEL_DS & 0b111, 0, "KERNEL_DS RPL must be 0");
        assert_eq!(KERNEL_DS >> 3, 2, "KERNEL_DS index must be 2");
    }

    #[test]
    fn user_cs_selector_layout() {
        assert_eq!(USER_CS & 0b11, 3, "USER_CS RPL must be 3");
        assert_eq!(USER_CS >> 3, 3, "USER_CS index must be 3");
    }

    #[test]
    fn user_ss_selector_layout() {
        assert_eq!(USER_SS & 0b11, 3, "USER_SS RPL must be 3");
        assert_eq!(USER_SS >> 3, 4, "USER_SS index must be 4");
    }
}
