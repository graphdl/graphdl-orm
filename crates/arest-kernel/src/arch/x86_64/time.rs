// crates/arest-kernel/src/arch/x86_64/time.rs
//
// PIT-backed monotonic millisecond counter (#180 follow-up).
//
// Programs the 8254 PIT channel 0 at ~1 kHz (1.193 MHz / 1193 → 1000.16
// Hz, ~0.016 % drift) in square-wave mode. Each fire increments a
// `static AtomicU64` ms counter. Readable from any context via
// `now_ms()`; advanced from the `timer_handler` IRQ the interrupts
// module installs at PIC vector 32.
//
// Why this is worth a module rather than two lines inside interrupts.rs:
//
//   * Doom's game loop (#270/#271) runs a tic accumulator against an
//     ms clock — the shim's `timeInMilliseconds` import reads `now_ms`
//     directly. Needs to be callable from the wasmi host binding, so
//     it lives in a named module rather than behind the IRQ file.
//   * `arch::halt_forever` notes (line ~110) that the idle loop stays
//     busy-poll because "the sole IRQ currently unmasked in the PIC"
//     is the keyboard. With the timer online, any future revision can
//     switch to `hlt`-then-poll — the timer ticks wake CPU out of `hlt`
//     cheaply. Not changed in this commit; the hook is now available.
//   * Net/blk retry/timeout code (today using cycle counters or naive
//     poll counts) can wait on an ms budget once this is live.
//
// Resolution trade-off: 1 kHz is ~0.05 % of CPU time spent in the IRQ
// handler on a modern x86 (the handler is ~5 insns: atomic add, EOI,
// iretq). Drift at ~0.016 % is fine for a 35 Hz game loop and for
// human-scale timeouts. TSC would be ns-res but needs per-boot
// calibration; not worth the complexity here.

use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::instructions::port::Port;

/// PIT base clock, per the Intel 8254 datasheet. Fixed across every
/// PC-compatible since the AT.
const PIT_FREQUENCY_HZ: u32 = 1_193_182;

/// Target tick rate. 1 kHz → 1 ms resolution, the smallest unit
/// `now_ms()` returns. Fits Doom's 35 Hz tic cadence with >25x
/// oversampling and supports sub-second network timeouts.
const TARGET_HZ: u32 = 1_000;

/// PIT divisor. Actual firing rate is PIT_FREQUENCY_HZ / PIT_DIVISOR
/// ≈ 1000.16 Hz; ~0.016 % drift. Well within what Doom's game loop
/// and our network timers tolerate.
const PIT_DIVISOR: u16 = (PIT_FREQUENCY_HZ / TARGET_HZ) as u16;

/// PIT channel 0 data register. Writing two bytes here (low then
/// high, per the lobyte/hibyte access mode we program) sets the
/// channel's reload value, which is the divisor.
const PIT_CHANNEL0_DATA: u16 = 0x40;

/// PIT command register. One byte selects channel, access mode,
/// operating mode, and BCD/binary counting.
const PIT_COMMAND: u16 = 0x43;

/// Monotonic millisecond counter. Starts at 0 the moment `init()`
/// programs the PIT; only moves forward. Relaxed ordering is fine:
/// there's one writer (the IRQ handler) and many readers, with no
/// happens-before dependency on the counter's value.
static MILLIS: AtomicU64 = AtomicU64::new(0);

/// Program PIT channel 0 at ~1 kHz. The IDT vector + PIC unmask live
/// in `interrupts.rs`; this function only touches the PIT itself, so
/// it's safe to call before `init_idt` (divisor writes are latched
/// without firing anything until the PIC route opens).
///
/// Called from `arch::init_gdt_and_interrupts()` after the PIC has
/// been remapped, so the first tick is serviced by `timer_handler`
/// and not routed to a reserved CPU-exception vector.
pub fn init() {
    // SAFETY: writes to fixed I/O ports 0x43 / 0x40 — this is how
    // every PC OS programs the PIT. No memory involved.
    unsafe {
        let mut cmd = Port::<u8>::new(PIT_COMMAND);
        // 0x36 = 0011_0110b: channel 0, lobyte/hibyte access, mode 3
        // (square wave, auto-reload), binary counting. Mode 3 fires
        // at half-rate on one edge and the other on the other edge;
        // the IRQ line toggles every half-period so we still get
        // PIT_FREQUENCY_HZ / divisor full cycles per second on the
        // IRQ line.
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
/// context including interrupt handlers and the panic path — it's
/// a single relaxed atomic load with no locks.
pub fn now_ms() -> u64 {
    MILLIS.load(Ordering::Relaxed)
}
