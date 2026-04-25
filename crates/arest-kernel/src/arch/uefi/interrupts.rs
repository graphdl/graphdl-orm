// crates/arest-kernel/src/arch/uefi/interrupts.rs
//
// Kernel-owned IDT for the UEFI x86_64 path (#363, extended in #379).
// Sibling of `arch::x86_64::interrupts` — same x86_64 silicon, but
// the UEFI boot path lands in a state where the firmware has already
// torn down its own IDT inside `boot::exit_boot_services`. There is
// no pre-wired IDT to "reprogram"; we install one from scratch the
// first time the kernel needs to handle a CPU exception.
//
// What this module installs:
//
//   * #BP (int 3, vector 3) — software breakpoint. The boot banner
//     fires `arch::breakpoint()` once `init_interrupts` has loaded
//     the IDT, expecting the handler to print + iretq back so the
//     next println! confirms the round-trip worked. From #363.
//   * #DF (vector 8) — double fault. Last-resort safety net — if the
//     CPU triple-faults the box silently reboots, so even with no
//     other handlers wired, having a #DF entry that prints + halts
//     gives the smoke harness a visible failure mode for any
//     unhandled exception. From #363.
//   * IRQ 0 (PIT timer, vector 32 after PIC remap) — drives
//     `arch::uefi::time::tick`. The 1 kHz tick gives the kernel a
//     monotonic millisecond counter (`arch::time::now_ms`) so the
//     shared kernel body's Doom tic accumulator, net retry budgets,
//     and any `hlt`-then-poll idle work identically on UEFI as on
//     BIOS. From #379.
//   * IRQ 1..15 (vectors 33..47) wired to a default handler that just
//     EOIs and returns. Defensive — once `sti` is on, firmware-leftover
//     pending IRQs (RTC, mouse, COM2 from before EBS) can fire into
//     the IDT; without these stubs they'd hit unpopulated vectors and
//     trigger #GP -> #DF -> triple-fault.
//   * Vectors 48..255 wired to a "spurious" handler that just iretqs.
//     Same defensive shape — covers any stray APIC / IPI fire that
//     could otherwise hit a gap.
//
// What is NOT here yet (#344f / future):
//   * GDT / TSS — firmware's GDT and CR3 stay live through boot. The
//     #DF handler runs on the firmware-supplied stack rather than a
//     dedicated IST entry, which is sufficient for "print + halt"
//     but not for stack-overflow recovery.
//   * PS/2 keyboard (IRQ 1) handler with real decode + dispatch.
//     The default IRQ 1 handler currently just EOIs; #344f / #364
//     wire the keyboard pipeline.
//   * Page-fault / GP-fault / #UD handlers — kernel ring-0 only on
//     the UEFI path today; ring-3 descent and its associated fault
//     decoding lands alongside a UEFI syscall path.

use crate::println;
use pic8259::ChainedPics;
use spin::{Mutex, Once};
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

/// Base vectors for the two cascaded PICs. Chosen to sit right
/// after the 32 CPU exception slots reserved by the architecture.
/// Same values the BIOS arm picks (see `arch::x86_64::interrupts`)
/// so the IRQ→vector mapping is identical across boot paths.
pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

/// Mapping of hardware IRQ → IDT vector. Mirrors the BIOS arm's
/// `InterruptIndex` so any future shared IRQ-handling code can
/// resolve the same vector numbers regardless of boot path.
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer    = PIC_1_OFFSET,
    Keyboard = PIC_1_OFFSET + 1,
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Cascaded PIC pair under a spin lock. Same construction the BIOS
/// arm uses — UEFI firmware does NOT permanently disable the legacy
/// 8259 PIC on QEMU+OVMF; it leaves the PIC physically present but
/// fully masked. Re-running the standard ICW1..ICW4 init sequence
/// remaps IRQ 0..15 from vectors 0x08..0x0F (collision with #DF and
/// other CPU exceptions) to vectors 32..47, just as on the BIOS path.
///
/// Constructed with `new_contiguous(PIC_1_OFFSET)` so PIC1 owns
/// vectors 32..39 and PIC2 owns 40..47 — `notify_end_of_interrupt`
/// then routes the EOI to the right PIC for any vector in the pair.
pub static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new_contiguous(PIC_1_OFFSET) });

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
/// What this populates (extended in #379 for the IRQ 0 timer):
///   * #BP and #DF — the original #363 surface.
///   * IRQ 0 (vector 32) → `timer_handler`.
///   * IRQ 1 (vector 33) → `default_irq_handler` — placeholder until
///     #344f / #364 wires the real keyboard handler.
///   * Vectors 34..47 → `default_irq_handler` (PIC IRQ 2..15 — RTC,
///     mouse, COM ports — defensive stubs so a firmware-pending IRQ
///     doesn't trigger an unpopulated-vector triple fault once the
///     PIC unmasks them).
///   * Vectors 48..255 → `spurious_handler` (defensive — covers any
///     stray APIC / IPI fire we don't know about).
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

        // IRQ 0 — PIT timer. The handler bumps the ms counter and
        // EOIs. Vector 32 because of the PIC remap done by
        // `pic_init` below.
        idt[InterruptIndex::Timer.as_u8()].set_handler_fn(timer_handler);

        // Defensive: IRQ 1..15 (vectors 33..47) get a stub handler.
        // Without this, a firmware-leftover pending IRQ that fires
        // immediately after `sti` (e.g. RTC, mouse, COM2 from
        // before EBS) would hit an unpopulated vector and triple-
        // fault the box. The stub EOIs to both PICs (since we
        // don't know which line fired without checking ISR) and
        // returns. The keyboard slot is included here so #344f /
        // #364 can override it with a real handler later without
        // changing any other site.
        for vec in (PIC_1_OFFSET + 1)..=(PIC_2_OFFSET + 7) {
            idt[vec].set_handler_fn(default_irq_handler);
        }

        // Defensive: vectors 48..255 get a spurious-IRQ stub. Covers
        // any stray APIC / IPI / firmware leftover that we don't
        // know about — better an immediate iretq than a triple-fault
        // restart with no diagnostic.
        for vec in (PIC_2_OFFSET + 8)..=255u8 {
            idt[vec].set_handler_fn(spurious_handler);
        }

        idt
    });
    idt.load();
}

/// Remap the cascaded 8259 PIC pair so IRQ 0..15 land on vectors
/// 32..47 instead of the firmware-default 0x08..0x0F (which collide
/// with CPU-exception slots). Then unmask only IRQ 0 (the PIT timer)
/// — IRQ 1 (keyboard) is #344f / #364 scope and stays masked here so
/// the boot banner is deterministic regardless of host keyboard
/// activity inside the smoke container.
///
/// SAFETY: programs the legacy 8259 ICW sequence over ports
/// 0x20/0x21/0xA0/0xA1. UEFI firmware leaves these ports wired even
/// post-EBS on QEMU+OVMF; the same `Pic8259::initialize` sequence
/// the BIOS arm uses works byte-for-byte here.
pub fn pic_init() {
    // SAFETY: ICW programming sequence — driven entirely through the
    // PIC's documented port pair. No memory state is touched. Same
    // call the BIOS arm makes from `init_pic`.
    unsafe {
        let mut pics = PICS.lock();
        pics.initialize();
        // 0xFE = 1111_1110 on PIC1 — unmask only IRQ 0 (timer).
        // 0xFF on PIC2 — keep RTC/mouse/etc all masked.
        // Keyboard (IRQ 1, mask bit 0xFD) stays masked here on the
        // UEFI arm because #379 scope is timer-only; #344f / #364
        // will widen the mask when the keyboard handler lands.
        pics.write_masks(0xFE, 0xFF);
    }
}

/// Enable hardware interrupts (`sti`). Must run AFTER `init_interrupts`
/// (so any pending IRQ that fires immediately lands in a registered
/// handler) and AFTER `pic_init` (so vector mapping is at 32+, not
/// 0x08+ where a tick would fire #DF).
///
/// Once enabled, the IRQ 0 timer fires every ~1 ms, advancing the
/// `arch::time::now_ms()` counter. CPU exception handlers (#BP, #DF)
/// continue working as before — they don't depend on `sti` because
/// CPU exceptions can't be masked.
pub fn enable_irqs() {
    x86_64::instructions::interrupts::enable();
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

/// PIT timer (IRQ 0, vector 32) handler. Bumps the millisecond
/// counter and EOIs the primary PIC. Same shape as the BIOS arm's
/// `timer_handler`: keep the work tiny so we don't accumulate
/// latency at 1 kHz (~1000 fires/sec → handler must run in <<1 ms
/// or the tick rate degrades).
///
/// EOI is sent at the end so the next tick can be queued. We don't
/// use `notify_end_of_interrupt` while holding any other lock — the
/// PIC is the only state touched here besides the atomic counter
/// inside `super::time::tick`.
extern "x86-interrupt" fn timer_handler(_stack_frame: InterruptStackFrame) {
    super::time::tick();
    // SAFETY: `notify_end_of_interrupt` writes the EOI command byte
    // (0x20) to the matching PIC's command port. Standard PIC EOI
    // sequence; idempotent and tolerant of being called from any
    // ring 0 context.
    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    }
}

/// Stub handler for IRQ vectors 33..47 (PIC IRQ 1..15). EOIs the
/// PIC so the line doesn't stay latched, but does no other work —
/// real per-IRQ handlers (keyboard #344f, etc.) replace this slot
/// when they come online.
///
/// We don't know which IRQ fired without reading ISR, so we just
/// EOI both PICs unconditionally. This is safe: an EOI to a PIC
/// that didn't have an in-service IRQ is documented as a no-op (it
/// only clears the highest-priority in-service bit, which is 0).
extern "x86-interrupt" fn default_irq_handler(_stack_frame: InterruptStackFrame) {
    // SAFETY: 0x20 to PIC1 / 0xA0 commands a non-specific EOI on
    // each PIC. Standard "I don't know which IRQ" pattern; safe to
    // call from any ring 0 context.
    unsafe {
        let mut pics = PICS.lock();
        // Send EOI to both — the IRQ might have been on either chip.
        // The 8259 documentation makes this idempotent when there
        // is nothing in service on a given chip.
        pics.notify_end_of_interrupt(PIC_2_OFFSET);
        pics.notify_end_of_interrupt(PIC_1_OFFSET);
    }
}

/// Stub handler for vectors 48..255. Just iretqs — no PIC EOI,
/// because these aren't routed through the 8259. Covers the
/// "spurious / unknown" range so a stray firmware-leftover IRQ
/// doesn't triple-fault the box once `sti` is on.
extern "x86-interrupt" fn spurious_handler(_stack_frame: InterruptStackFrame) {
    // Intentionally empty.
}
