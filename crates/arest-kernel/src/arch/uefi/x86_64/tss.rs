// crates/arest-kernel/src/arch/uefi/x86_64/tss.rs
//
// Task State Segment for the UEFI x86_64 ring-3 gate (#552, paired
// with `gdt.rs` and `syscall_msr.rs`). The TSS in long mode does
// double duty:
//
//   1. `RSP0` (privilege_stack_table[0]) — the stack the CPU loads
//      RSP from on a privilege-level promotion (e.g., `iretq` from
//      ring 3 to ring 0 fires off an IRQ that takes us back to
//      kernel mode; the `int n` / IRQ handler runs on RSP0). We
//      seed this to a 16-KiB kernel stack allocated at boot.
//
//   2. `interrupt_stack_table[0..2]` (IST 1, 2, 3) — the stacks the
//      CPU switches to for IDT entries marked with a non-zero IST
//      index. We allocate three 16-KiB stacks, one each for #DF,
//      #PF, and #GP, so any double / page / general-protection fault
//      taken while the kernel was already on a tight or corrupted
//      stack lands on a fresh stack we trust.
//
// The IDT install in `arch::uefi::interrupts` does NOT yet wire IST
// indexes into the #DF / #PF / #GP entries — those vectors currently
// run on the existing (firmware-supplied) stack. Wiring the IDT
// entries to use these IST stacks is a follow-up that depends on a
// re-export of the IST selectors from this module; the TSS itself
// must exist with valid stacks first, which is what this module
// provides.
//
// Stack allocation
// ----------------
// Each stack is a 16 KiB page-aligned heap region. We use the
// `alloc::alloc::alloc` global allocator (the same talc instance
// `process::address_space` uses) rather than the firmware frame
// allocator, for the same reasons documented in
// `process::address_space` lines 25-43: drop reclaims (we never
// reclaim, but the API matches), the lifetime is tied to the
// allocation handle, and the alloc-vs-frame split mirrors how every
// other kernel-internal stack carve is allocated.
//
// 16 KiB is the conventional kernel stack size on x86_64 (Linux uses
// `THREAD_SIZE` = 16 KiB on x86_64; we match it). Big enough for any
// realistic exception handler, small enough that allocating four of
// them at boot (RSP0 + IST 1/2/3) consumes 64 KiB total — a
// rounding error against the 32 MiB kernel heap.
//
// Why a `static` TSS
// ------------------
// `Descriptor::tss_segment(&'static TaskStateSegment)` requires
// `'static`. The TSS lives once for the kernel's lifetime — same
// shape as the IDT in `arch::uefi::interrupts`. `Once` keeps the
// init lazy, and the resulting `&TaskStateSegment` projection is
// `'static` for the duration of boot.
//
// `unsafe` impl Sync? No — `TaskStateSegment` is `Copy + Sync`
// already (it's a plain `repr(C, packed)` struct of u32 / u64
// fields). The mutation we do — writing the privilege_stack_table
// and interrupt_stack_table fields — happens once during `install()`
// behind the `Once` guard; after that the value is read-only from
// the CPU's perspective.

use alloc::alloc::{alloc, Layout};
use spin::Once;
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

/// Per-IST stack size. 16 KiB matches Linux's `THREAD_SIZE` on
/// x86_64 — small enough that allocating four (RSP0 + IST1/2/3)
/// is a 64-KiB drop in the kernel heap, big enough that a deep
/// fault handler stack walk doesn't overflow.
pub const IST_STACK_SIZE: usize = 16 * 1024;

/// IST index reserved for the #DF (double fault) handler. IDT
/// entries set their `stack_index` to this value to make the CPU
/// switch to `tss.interrupt_stack_table[0]` on entry.
///
/// The IDT in `arch::uefi::interrupts` does not yet use this index
/// — wiring is a follow-up. The constant + the underlying stack
/// allocation are present so the wiring is a one-line change.
pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

/// IST index reserved for the #PF (page fault) handler.
pub const PAGE_FAULT_IST_INDEX: u16 = 1;

/// IST index reserved for the #GP (general protection fault)
/// handler.
pub const GENERAL_PROTECTION_FAULT_IST_INDEX: u16 = 2;

/// Static TSS, populated once by `install()` and `'static` for the
/// rest of boot. The `Descriptor::tss_segment` constructor
/// requires `&'static`; this `Once` is what makes that lifetime
/// honest — the TaskStateSegment value never moves after init.
static TSS: Once<TaskStateSegment> = Once::new();

/// Static RSP0 stack for kernel-side privilege promotions
/// (`iretq` from ring 3 → ring 0 lands here when an IRQ fires while
/// we were in userspace). The pointer is the BASE of the
/// allocation; the stack grows downward, so the value the CPU loads
/// into RSP is `base + IST_STACK_SIZE`.
static RSP0_STACK_BASE: Once<u64> = Once::new();

/// Once-guard so a re-entrant `install` is a no-op. The boot path
/// calls this exactly once.
static INSTALLED: Once<()> = Once::new();

/// Allocate the TSS's RSP0 + IST stacks, populate the TSS fields,
/// and hand back a `&'static TaskStateSegment` the GDT module can
/// pass to `Descriptor::tss_segment`. Caller (the GDT module) is
/// responsible for calling `load_tss(selector)` after appending the
/// TSS descriptor — this function does NOT load the TSS, only build
/// it.
///
/// Must run AFTER the global allocator is live (i.e., after the
/// UEFI entry's `init_heap` call). Must run BEFORE
/// `arch::uefi::x86_64::gdt::install()` because the GDT install
/// path calls `Descriptor::tss_segment` against the value this
/// function returns.
///
/// Idempotent — a second call returns the already-built TSS via
/// the `Once`'s cached value.
pub fn install() -> &'static TaskStateSegment {
    let tss_ref = TSS.call_once(|| {
        let mut tss = TaskStateSegment::new();
        tss.privilege_stack_table[0] = VirtAddr::new(allocate_stack_top());
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] =
            VirtAddr::new(allocate_stack_top());
        tss.interrupt_stack_table[PAGE_FAULT_IST_INDEX as usize] =
            VirtAddr::new(allocate_stack_top());
        tss.interrupt_stack_table[GENERAL_PROTECTION_FAULT_IST_INDEX as usize] =
            VirtAddr::new(allocate_stack_top());
        tss
    });

    INSTALLED.call_once(|| {
        // Pre-populate the RSP0 base record so future readers (per-
        // CPU info, debugger) can reach it without traversing the
        // TSS structure.
        let _ = RSP0_STACK_BASE.call_once(|| tss_ref.privilege_stack_table[0].as_u64());
    });

    tss_ref
}

/// Allocate one IST/RSP0 stack and return the TOP-of-stack pointer
/// (= base + IST_STACK_SIZE). The CPU pre-decrements RSP on every
/// push, so we hand it the byte ONE-PAST-the-end of the allocation
/// — same convention every kernel uses for downward-growing stacks.
///
/// Stacks are page-aligned (4 KiB) — over-aligned for x86_64 (the
/// SysV ABI only requires 16-byte) but matches the page-allocator's
/// natural granule. 16 KiB / 4 KiB = 4 pages per stack.
///
/// SAFETY: leaks the allocation. The kernel never reclaims these
/// stacks (their lifetime is the kernel's lifetime); same shape as
/// the IDT static — exception state has the strongest possible
/// liveness requirement and storing it in a leaked allocation is
/// cleaner than a `Box::leak` round-trip.
fn allocate_stack_top() -> u64 {
    // 4 KiB page alignment matches `address_space::PAGE_SIZE` and
    // the underlying frame-allocator granule. Over-aligned for the
    // 16-byte SysV stack alignment requirement.
    const STACK_ALIGN: usize = 4096;
    let layout = Layout::from_size_align(IST_STACK_SIZE, STACK_ALIGN)
        .expect("TSS stack layout is well-formed");
    // SAFETY: `Layout` is well-formed (size > 0, alignment > 0,
    // size + align rounded fits in usize). The allocator returns
    // null only on OOM; we panic in that case because the kernel
    // can't proceed without a TSS stack.
    let base = unsafe { alloc(layout) };
    assert!(!base.is_null(), "TSS stack allocation failed (OOM)");
    // Stack grows downward — return the byte one past the
    // allocation's last byte. The CPU's RSP starts at this value
    // and pre-decrements on each push.
    base as u64 + IST_STACK_SIZE as u64
}

/// Return the TSS descriptor selector that `gdt::install()` placed
/// in the GDT. Available after `gdt::install()` has run; panics
/// otherwise. Used by the boot harness if it needs to verify the
/// TSS load took.
#[allow(dead_code)]
pub fn rsp0_top() -> Option<u64> {
    RSP0_STACK_BASE.get().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// IST stack size matches Linux's THREAD_SIZE on x86_64 (16 KiB).
    /// A drift here would change the per-stack memory budget without
    /// updating the docstrings.
    #[test]
    fn ist_stack_size_is_16_kib() {
        assert_eq!(IST_STACK_SIZE, 16 * 1024);
    }

    /// IST indexes are 0/1/2 — the three slots our boot path
    /// consumes (#DF, #PF, #GP). The `interrupt_stack_table` array
    /// has 7 slots per the spec (Intel SDM Vol 3 7.7) so up to 4
    /// remain free for future use.
    #[test]
    fn ist_indexes_are_distinct_and_in_range() {
        assert_ne!(DOUBLE_FAULT_IST_INDEX, PAGE_FAULT_IST_INDEX);
        assert_ne!(PAGE_FAULT_IST_INDEX, GENERAL_PROTECTION_FAULT_IST_INDEX);
        assert_ne!(DOUBLE_FAULT_IST_INDEX, GENERAL_PROTECTION_FAULT_IST_INDEX);
        assert!((DOUBLE_FAULT_IST_INDEX as usize) < 7);
        assert!((PAGE_FAULT_IST_INDEX as usize) < 7);
        assert!((GENERAL_PROTECTION_FAULT_IST_INDEX as usize) < 7);
    }

    /// The TaskStateSegment construction works on the host build.
    /// Validates the `x86_64` crate is on the host's lib path under
    /// `cargo test --lib`. Cheap smoke that the right crate version
    /// is in the dep graph.
    #[test]
    fn tss_constructs() {
        let _tss: TaskStateSegment = TaskStateSegment::new();
    }
}
