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
//   * IRQ 1 (PS/2 keyboard, vector 33 after PIC remap) — drives
//     `arch::uefi::keyboard::handle_scancode`. Reads the scancode
//     from port 0x60, hands it to `pc-keyboard`'s decoder, and
//     pushes any resulting `DecodedKey` onto the kernel-side ring
//     for later drain by the boot smoke or (in #365 scope) the
//     UEFI REPL pump. From #364.
//   * IRQ 2..15 (vectors 34..47) wired to a default handler that just
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
//   * REPL drain pump — the keyboard ring is fed from IRQ 1 but
//     nothing on the UEFI path drains it into the line editor yet.
//     #365 wires the pump alongside a UEFI-reachable REPL.
//   * Page-fault / GP-fault / #UD handlers — kernel ring-0 only on
//     the UEFI path today; ring-3 descent and its associated fault
//     decoding lands alongside a UEFI syscall path.

use crate::println;
use pic8259::ChainedPics;
use spin::{Mutex, Once};
use x86_64::instructions::port::Port;
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

/// IRQ 12 (PS/2 mouse, aux device) → vector 44 on the slave PIC.
/// Kept a free const rather than an `InterruptIndex` variant because
/// it's gated on `feature = "slint"` (the only pointer-ring consumer),
/// and a cfg'd enum variant would muddy `as_u8`. From #598.
#[cfg(feature = "slint")]
const MOUSE_VECTOR: u8 = PIC_2_OFFSET + 4;

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
/// Diagnostic: how many times the IRQ-1 keyboard ISR has actually run.
/// Splits "QEMU raised the line" (visible host-side via `info irq`)
/// from "the guest serviced the vector" — the missing middle of the
/// input pipeline the launcher's periodic diag line brackets with
/// `keyboard::total_enqueued()` / `repl::EVAL_COUNT`.
pub static IRQ1_COUNT: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

/// Diagnostic snapshot of the interrupt-delivery state: (IF, master-PIC
/// IMR, ISR, IRR). IMR = masked lines; ISR = lines marked in-service
/// (a stuck bit here means a missing EOI is blocking that priority and
/// everything below it); IRR = lines raised and waiting. Read via the
/// standard OCW3 sequence on the master PIC's command port.
pub fn pic_diag() -> (bool, u8, u8, u8) {
    let if_on = x86_64::instructions::interrupts::are_enabled();
    let mut cmd = Port::<u8>::new(0x20);
    let mut data = Port::<u8>::new(0x21);
    // SAFETY: standard 8259A OCW3 reads — port 0x21 IMR read is
    // side-effect-free; writing 0x0a / 0x0b to 0x20 selects IRR / ISR
    // for the next read of 0x20. All documented PC-architecture ports.
    unsafe {
        let imr: u8 = data.read();
        cmd.write(0x0au8);
        let irr: u8 = cmd.read();
        cmd.write(0x0bu8);
        let isr: u8 = cmd.read();
        (if_on, imr, isr, irr)
    }
}

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
/// What this populates (extended in #379 for the IRQ 0 timer, in
/// #364 for the IRQ 1 keyboard):
///   * #BP and #DF — the original #363 surface.
///   * IRQ 0 (vector 32) → `timer_handler`.
///   * IRQ 1 (vector 33) → `keyboard_handler` (#364) — reads scancode
///     from port 0x60, decodes via `pc-keyboard`, and pushes the
///     resulting `DecodedKey` onto the `arch::uefi::keyboard` ring.
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
        // #PF + #GP (#527): wired so early-userspace faults name
        // themselves. Before this, vector 14 was unpopulated and a
        // ring-3 instruction fetch on a supervisor-only identity page
        // escalated into an undiagnosable DOUBLE FAULT.
        idt.page_fault.set_handler_fn(page_fault_handler);
        idt.general_protection_fault
            .set_handler_fn(general_protection_handler);

        // IRQ 0 — PIT timer. The handler bumps the ms counter and
        // EOIs. Vector 32 because of the PIC remap done by
        // `pic_init` below.
        idt[InterruptIndex::Timer.as_u8()].set_handler_fn(timer_handler);

        // IRQ 1 — PS/2 keyboard (#364). Reads port 0x60, runs the
        // byte through the `pc-keyboard` decoder, and pushes the
        // decoded keystroke onto the kernel-side ring. The ring is
        // drained by the boot smoke today; the UEFI REPL pump (#365)
        // becomes the production drainer.
        //
        // #628 Profile-4: gated on `feature = "repl"`. When OFF
        // (the headless `--no-default-features --features server`
        // profile), the `keyboard_handler` function and the
        // `arch::uefi::keyboard` module are both elided, so the
        // line wouldn't compile. The matching IRQ 1 mask bit is
        // also kept set in `pic_init` below (the PIC mask byte
        // becomes 0xFD instead of 0xFC) so no firmware-pending
        // PS/2 IRQ can fire into the unpopulated vector 33 slot.
        // To remove the residual triple-fault risk if a stray
        // PS/2 IRQ somehow bypassed the mask, we route vector 33
        // through `default_irq_handler` (the EOI-only stub) when
        // `repl` is off — the `cfg_attr` selector on the function
        // call below picks the right handler.
        #[cfg(feature = "repl")]
        idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_handler);
        #[cfg(not(feature = "repl"))]
        idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(default_irq_handler);

        // Defensive: IRQ 2..15 (vectors 34..47) get a stub handler.
        // Without this, a firmware-leftover pending IRQ that fires
        // immediately after `sti` (e.g. RTC, mouse, COM2 from
        // before EBS) would hit an unpopulated vector and triple-
        // fault the box. The stub EOIs to both PICs (since we
        // don't know which line fired without checking ISR) and
        // returns.
        for vec in (PIC_1_OFFSET + 2)..=(PIC_2_OFFSET + 7) {
            idt[vec].set_handler_fn(default_irq_handler);
        }

        // IRQ 12 — PS/2 mouse (#598). Overrides the defensive stub the
        // loop above just set on vector 44 (aux device, slave PIC).
        // Gated on `feature = "slint"`: the pointer ring it feeds is
        // drained only by the slint launcher. With slint off, vector 44
        // keeps the EOI-only stub and IRQ 12 stays masked (see
        // `pic_init`), so there's no path to reach `mouse_handler`.
        #[cfg(feature = "slint")]
        idt[MOUSE_VECTOR].set_handler_fn(mouse_handler);

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
/// with CPU-exception slots), then unmask IRQ 0 (PIT timer) and
/// IRQ 1 (PS/2 keyboard). All other lines stay masked so a stray
/// RTC / mouse / COM IRQ doesn't fire into the defensive stubs and
/// burn cycles for no observable effect.
///
/// Keyboard decoder is initialised here (rather than in
/// `init_interrupts`) so the lazy `Once` payload is populated
/// BEFORE the IRQ 1 mask is cleared — otherwise a firmware-pending
/// scancode that fires between unmask and the first decoder-feed
/// would land in `keyboard_handler` with `KEYBOARD.get()` returning
/// `None`, dropping the byte. Order matters.
///
/// SAFETY: programs the legacy 8259 ICW sequence over ports
/// 0x20/0x21/0xA0/0xA1. UEFI firmware leaves these ports wired even
/// post-EBS on QEMU+OVMF; the same `Pic8259::initialize` sequence
/// the BIOS arm uses works byte-for-byte here.
pub fn pic_init() {
    // Stand up the `pc-keyboard` decoder singleton before the IRQ
    // mask is cleared — see docstring for the ordering rationale.
    // #628 Profile-4: gated on `feature = "repl"`. The decoder
    // singleton lives in `arch::uefi::keyboard`, which is itself
    // feature-gated (the module decl in `arch/uefi/mod.rs` is
    // `#[cfg(feature = "repl")]`); skip the call when the module
    // is absent. The matching IRQ 1 mask bit also stays set
    // below so no PS/2 scancode can fire into the missing
    // decoder.
    #[cfg(feature = "repl")]
    super::keyboard::init();

    // Bring the PS/2 aux (mouse) device online before its IRQ line
    // opens (#598). Gated on `feature = "slint"` — the pointer ring it
    // feeds is slint-only. Bounded polls inside, so a missing aux
    // device can't wedge boot. Mirrors the keyboard decoder bootstrap
    // above: configure the device before the mask clears so the first
    // packet has somewhere to land.
    #[cfg(feature = "slint")]
    super::mouse::init();

    // SAFETY: ICW programming sequence — driven entirely through the
    // PIC's documented port pair. No memory state is touched. Same
    // call the BIOS arm makes from `init_pic`.
    unsafe {
        let mut pics = PICS.lock();
        pics.initialize();
        // PIC mask bytes (bit set = MASKED, bit clear = unmasked).
        // Computed per feature so each profile opens exactly the lines
        // it has both a handler and a consumer for:
        //
        //   * IRQ 0  (timer, PIC1 bit 0) — ALWAYS open. #655: keeping
        //     bit 0 clear is load-bearing — a masked timer freezes
        //     `arch::time::now_ms`, stalling smoltcp's DHCPv4 / TCP
        //     retransmit timers at t=0 forever.
        //   * IRQ 1  (keyboard, PIC1 bit 1) — open with `repl`. With
        //     repl off, the decoder + IDT vector 33 are both absent
        //     (#628), so the line stays masked and no scancode fires
        //     into an unpopulated slot.
        //   * IRQ 2  (cascade, PIC1 bit 2) + IRQ 12 (mouse, PIC2 bit 4)
        //     — open with `slint` (#598). The mouse hangs off the slave
        //     PIC, so its packets reach the CPU only if the master's
        //     IRQ 2 cascade is ALSO open; opening PIC2 bit 4 alone would
        //     silently drop every mouse IRQ.
        //
        // Every other line stays masked so a stray RTC / COM IRQ doesn't
        // burn cycles in the defensive stub.
        #[allow(unused_mut)]
        let mut pic1: u8 = 0xFE; // IRQ 0 (timer) open
        #[allow(unused_mut)]
        let mut pic2: u8 = 0xFF;
        #[cfg(feature = "repl")]
        {
            pic1 &= !0x02; // IRQ 1 keyboard
        }
        #[cfg(feature = "slint")]
        {
            pic1 &= !0x04; // IRQ 2 cascade (required for any slave IRQ)
            pic2 &= !0x10; // IRQ 12 mouse
        }
        pics.write_masks(pic1, pic2);
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

/// Page-fault (#PF, vector 14) handler. Prints CR2 (the faulting
/// address) + the architectural error-code decode + the frame, then
/// panics — tier-1 has no demand paging and exactly one process, so
/// any #PF is a kernel bug or an unmapped/unauthorised guest access;
/// "loud and precise" beats the prior behaviour, where the unwired
/// vector escalated every #PF (e.g. ring-3's first instruction fetch
/// on a supervisor-only identity page) into an undiagnosable DOUBLE
/// FAULT.
extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: x86_64::structures::idt::PageFaultErrorCode,
) {
    let cr2 = x86_64::registers::control::Cr2::read_raw();
    panic!(
        "EXCEPTION: PAGE FAULT\ncr2 (accessed address): {cr2:#x}\nerror: {error_code:?}\n{stack_frame:#?}"
    );
}

/// General-protection (#GP, vector 13) handler. Same rationale as
/// #PF above: the common early-userspace faults (bad segment use,
/// privileged instruction at CPL3, non-canonical access) should name
/// themselves instead of double-faulting.
extern "x86-interrupt" fn general_protection_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    panic!(
        "EXCEPTION: GENERAL PROTECTION FAULT\nerror code: {error_code:#x}\n{stack_frame:#?}"
    );
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

/// PS/2 keyboard (IRQ 1, vector 33) handler (#364). Reads the
/// scancode byte from port 0x60, hands it to the
/// `arch::uefi::keyboard` ring (which feeds the byte through the
/// `pc-keyboard` decoder and stashes any resulting `DecodedKey`),
/// then EOIs the primary PIC.
///
/// Same shape as the BIOS arm's `keyboard_handler`: keep the
/// in-ISR work bounded, EOI before any potentially-blocking
/// dispatch (here the dispatch is just a lock + push, which we do
/// inline because there is no REPL drainer on UEFI yet).
///
/// EOI is sent at the END so the next scancode can fire only after
/// the ring write completes — important on a slow drainer because
/// the ring is bounded and the ISR drops oldest under back-pressure;
/// late EOI here would surface as a spurious double-pop on the
/// drainer side rather than a silent drop.
///
/// #628 Profile-4: gated on `feature = "repl"`. `super::keyboard`
/// is also feature-gated, so this function would otherwise fail
/// to compile with `repl` off. With `repl` off, IRQ 1 stays masked
/// at the PIC (`pic_init` writes 0xFD instead of 0xFC) AND the IDT
/// vector 33 install is skipped (`init_interrupts`) — there's no
/// path to reach this function in that profile.
#[cfg(feature = "repl")]
extern "x86-interrupt" fn keyboard_handler(_stack_frame: InterruptStackFrame) {
    // Diagnostic: monotonic ISR-invocation count (see `irq1_count`).
    // Relaxed suffices — single CPU, read only by the diag line.
    IRQ1_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    // SAFETY: 0x60 is the PS/2 keyboard data port — a documented
    // PC-architecture port that returns the most recent scancode
    // byte. Single read, no side effects beyond clearing the
    // controller's output buffer.
    let mut port = Port::<u8>::new(0x60);
    let scancode: u8 = unsafe { port.read() };

    super::keyboard::handle_scancode(scancode);

    // SAFETY: `notify_end_of_interrupt` writes the EOI command byte
    // (0x20) to PIC1's command port. Standard PIC EOI sequence;
    // idempotent and tolerant of being called from any ring 0
    // context.
    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }
}

/// PS/2 mouse (IRQ 12, vector 44) handler (#598). Reads the aux byte
/// from port 0x60, feeds it to the `arch::uefi::mouse` accumulator
/// (which pushes `PointerEvent`s on a completed packet), then EOIs.
/// IRQ 12 is on the slave PIC, so `notify_end_of_interrupt` routes the
/// EOI to both the slave and the master chip.
///
/// Same in-ISR discipline as `keyboard_handler`: one port read + a
/// bounded feed, EOI last. Gated on `feature = "slint"` to match the
/// `arch::uefi::mouse` module and the IRQ 12 unmask in `pic_init`.
#[cfg(feature = "slint")]
extern "x86-interrupt" fn mouse_handler(_stack_frame: InterruptStackFrame) {
    // SAFETY: 0x60 is the shared PS/2 data port; for an aux IRQ it
    // returns the next mouse-packet byte. Single read; clears the
    // controller output buffer.
    let mut port = Port::<u8>::new(0x60);
    let byte: u8 = unsafe { port.read() };
    super::mouse::handle_aux_byte(byte);
    // SAFETY: standard PIC EOI. For a slave-PIC vector the chained
    // impl sends the EOI to both PIC2 and PIC1.
    unsafe {
        PICS.lock().notify_end_of_interrupt(MOUSE_VECTOR);
    }
}

/// Stub handler for IRQ vectors 34..47 (PIC IRQ 2..15). EOIs the
/// PIC so the line doesn't stay latched, but does no other work —
/// real per-IRQ handlers replace this slot when they come online.
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
