// crates/arest-kernel/src/arch/uefi/time.rs
//
// PIT-backed monotonic millisecond counter for the UEFI x86_64 path
// (#379). Sibling of `arch::x86_64::time` — same 8254 PIT silicon,
// same 1 kHz cadence, same `AtomicU64` accessor surface — wired up
// from the UEFI side so the shared kernel body's `arch::time::now_ms()`
// call sites (Doom tic accumulator, net retry budgets, future
// `hlt`-then-poll idle) work identically on both boot paths.
//
// The IDT vector + 8259 PIC unmask live in `interrupts.rs`; this
// module owns only the PIT itself and the millisecond counter. Same
// split the BIOS arm uses, for the same reason: the timer counter is
// a stable atom with multiple readers (game loop, network stack,
// boot banner) and a single writer (the IRQ 0 handler), so its
// surface area is independent of how the IRQ gets routed.
//
// Why mode 3 (square wave) and not mode 2 (rate generator):
//   * Both fire at PIT_FREQUENCY_HZ / divisor on the IRQ line. Mode 3
//     is what the BIOS arm uses (cadence-tested); using the same mode
//     here keeps the divisor / drift / handler-overhead numbers
//     identical between the two arms, which makes "the kernel ticks
//     at the same rate on both paths" trivially auditable from the
//     boot banner.
//   * Mode 2 would give a slightly cleaner waveform but firmware
//     leaves the PIT in mode 0 / mode 3 historically; either choice
//     is valid post-EBS, the 1 kHz cadence is the contract.

use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::instructions::port::Port;

/// PIT base clock, per the Intel 8254 datasheet. Fixed across every
/// PC-compatible since the AT — the firmware boot mode does not
/// change the underlying oscillator, so post-EBS UEFI sees the same
/// frequency the BIOS arm sees.
const PIT_FREQUENCY_HZ: u32 = 1_193_182;

/// Target tick rate. 1 kHz → 1 ms resolution, matching the BIOS
/// arm so `arch::time::now_ms()` returns the same units regardless
/// of boot path. Doom's 35 Hz tic accumulator and human-scale
/// network timeouts both target this resolution.
const TARGET_HZ: u32 = 1_000;

/// PIT divisor. Actual firing rate is PIT_FREQUENCY_HZ / PIT_DIVISOR
/// ≈ 1000.16 Hz; ~0.016 % drift. Identical to the BIOS arm.
const PIT_DIVISOR: u16 = (PIT_FREQUENCY_HZ / TARGET_HZ) as u16;

/// PIT channel 0 data register. Writing two bytes here (low then
/// high, per the lobyte/hibyte access mode we program) sets the
/// channel's reload value, which is the divisor.
const PIT_CHANNEL0_DATA: u16 = 0x40;

/// PIT command register. One byte selects channel, access mode,
/// operating mode, and BCD/binary counting.
const PIT_COMMAND: u16 = 0x43;

/// Monotonic millisecond counter. Starts at 0 the moment `init()`
/// programs the PIT; only moves forward. Relaxed ordering: there's
/// one writer (the IRQ 0 handler in `interrupts.rs`) and many
/// readers, with no happens-before dependency on the counter's
/// value.
static MILLIS: AtomicU64 = AtomicU64::new(0);

/// Program PIT channel 0 at ~1 kHz. The IDT vector + PIC unmask live
/// in `interrupts.rs`; this function only touches the PIT itself.
///
/// Called from `arch::uefi::interrupts::init_interrupts()` after the
/// PIC has been remapped + the IRQ 0 vector populated, so the first
/// tick is serviced by `timer_handler` and not routed to a reserved
/// CPU-exception vector or an unpopulated IDT slot (either of which
/// would triple-fault the box).
pub fn init() {
    // SAFETY: writes to fixed I/O ports 0x43 / 0x40 — this is how
    // every PC OS programs the PIT. Same access pattern the BIOS
    // arm uses; UEFI firmware does not lock or remap these ports.
    unsafe {
        let mut cmd = Port::<u8>::new(PIT_COMMAND);
        // 0x36 = 0011_0110b: channel 0, lobyte/hibyte access, mode 3
        // (square wave, auto-reload), binary counting. Same word the
        // BIOS arm writes — see `arch::x86_64::time::init` for the
        // mode 3 rationale.
        cmd.write(0x36);
        let mut data = Port::<u8>::new(PIT_CHANNEL0_DATA);
        data.write((PIT_DIVISOR & 0xFF) as u8);
        data.write((PIT_DIVISOR >> 8) as u8);
    }
}

/// Advance the millisecond counter by one. Called from the IRQ 0
/// handler in `interrupts.rs` on every PIT fire.
pub fn tick() {
    MILLIS.fetch_add(1, Ordering::Relaxed);
}

/// Monotonic milliseconds since `init()`. Safe to call from any
/// context including interrupt handlers and the panic path — it is
/// a single relaxed atomic load with no locks.
pub fn now_ms() -> u64 {
    MILLIS.load(Ordering::Relaxed)
}
