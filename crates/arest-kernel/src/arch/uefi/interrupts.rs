// crates/arest-kernel/src/arch/uefi/interrupts.rs
//
// Kernel-owned IDT for the UEFI x86_64 path (#363). Sibling of
// `arch::x86_64::interrupts` — same x86_64 silicon, but the UEFI
// boot path lands in a state where the firmware has already torn
// down its own IDT inside `boot::exit_boot_services`. There is no
// pre-wired IDT to "reprogram"; we install one from scratch the
// first time the kernel needs to handle a CPU exception.
//
// Today this commit installs only the two CPU-exception handlers
// the boot banner immediately exercises:
//
//   * #BP (int 3, vector 3) — software breakpoint. The boot banner
//     fires `arch::breakpoint()` once `init_interrupts` has loaded
//     the IDT, expecting the handler to print + iretq back so the
//     next println! confirms the round-trip worked.
//   * #DF (vector 8) — double fault. Last-resort safety net — if the
//     CPU triple-faults the box silently reboots, so even with no
//     other handlers wired, having a #DF entry that prints + halts
//     gives the smoke harness a visible failure mode for any
//     unhandled exception.
//
// What is NOT here yet (out of scope for #363, tracked in #344f / #379):
//   * GDT / TSS — firmware's GDT and CR3 stay live through boot. The
//     #DF handler runs on the firmware-supplied stack rather than a
//     dedicated IST entry, which is sufficient for "print + halt"
//     but not for stack-overflow recovery.
//   * 8259 PIC remap or APIC programming — no hardware IRQ vectors
//     are populated. PIT timer and PS/2 keyboard wiring are #344f.
//   * Page-fault / GP-fault / #UD handlers — kernel ring-0 only on
//     the UEFI path today; ring-3 descent and its associated fault
//     decoding lands alongside a UEFI syscall path.

use crate::println;
use spin::Once;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

/// IDT instance. Built on the first call to `init_interrupts` and
/// kept alive for the rest of the kernel's lifetime — `Once` keeps
/// the value pinned in `.bss` so the `lidt` reference stays valid
/// as long as the kernel runs.
static IDT: Once<InterruptDescriptorTable> = Once::new();

/// Build the IDT and load it into the CPU via `lidt`. Call once,
/// from `kernel_run_uefi` after `init_memory` — the heap and frame
/// allocator must be live so the `Once` initializer can run, and the
/// firmware's post-EBS state must be settled (no more BootServices
/// callbacks reaching for their own gates).
///
/// Idempotent: a second call is a no-op (Once already populated).
/// The IDT lives in `.bss`-backed static memory, so the lidt-loaded
/// pointer stays valid for the rest of boot — the firmware's
/// teardown does NOT reclaim our PE image's static data.
pub fn init_interrupts() {
    let idt = IDT.call_once(|| {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        // Double-fault uses the firmware's stack rather than a
        // dedicated IST entry — we don't reprogram the GDT/TSS on
        // UEFI yet (#344f scope). Sufficient for "print + halt"
        // diagnostics; a stack-overflow #DF would still triple-
        // fault the box, but that's the same baseline as the
        // firmware-only state we replaced.
        idt.double_fault.set_handler_fn(double_fault_handler);
        idt
    });
    idt.load();
}

/// Fire a software breakpoint (`int3`). Mirrors the BIOS arm's
/// `arch::breakpoint` helper so the shared boot-banner smoke is
/// callable target-agnostically. Panics until `init_interrupts`
/// has loaded the IDT — the firmware's post-EBS state has no
/// breakpoint gate, so a pre-init `int3` would double-fault.
///
/// Wraps the inline asm directly rather than going through the
/// `x86_64` crate's `int3()` so the call site stays explicit about
/// what instruction it is firing — the BIOS arm uses the wrapper
/// for the same reason; either form decodes to a single `cc` byte.
pub fn breakpoint() {
    // SAFETY: `int3` is a one-byte software interrupt that the
    // architecture documents as always safe to execute. The
    // installed handler iretqs back unconditionally, so control
    // resumes at the next instruction with no register clobbers.
    unsafe {
        core::arch::asm!("int3", options(nomem, nostack));
    }
}

/// Breakpoint (#BP, vector 3) handler. Prints the trapped frame
/// and iretqs back to the caller. Mirrors the BIOS arm's handler
/// so a debugger setting an int3 in shared kernel code surfaces
/// identically on either boot path.
extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    println!("EXCEPTION: BREAKPOINT\n{stack_frame:#?}");
}

/// Double-fault (#DF, vector 8) handler. UEFI boot path has no
/// IST stack switch yet (#344f), so the handler runs on the
/// firmware-supplied stack — sufficient for a `println!` + halt
/// pair; a real recovery path would need a dedicated stack to
/// survive a stack-overflow #DF.
///
/// `extern "x86-interrupt"` with `-> !` because #DF is a
/// non-recoverable exception — the architecture forbids iretq
/// once the error code is on the stack. Halt the CPU rather than
/// returning into a corrupt state.
extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    panic!("EXCEPTION: DOUBLE FAULT\n{stack_frame:#?}");
}
