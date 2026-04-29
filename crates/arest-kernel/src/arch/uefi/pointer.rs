// crates/arest-kernel/src/arch/uefi/pointer.rs
//
// Pointer event ring for the UEFI x86_64 path (#460 Track AAAA,
// foundation for the linuxkpi virtio-input wiring at #459b). Sibling
// of `arch::uefi::keyboard` — same ring-buffer shape, different
// payload.
//
// Why a separate module from keyboard
// -----------------------------------
//   * Different payload type. Keyboard ring carries `pc-keyboard`
//     `DecodedKey` (post-decode keystrokes for the REPL line editor
//     + Slint key dispatch). Pointer ring carries `PointerEvent`
//     (cursor moves, button clicks, scroll wheel) which the future
//     Slint windowing surface (#459d) will translate into
//     `slint::WindowEvent::PointerMoved` / `PointerPressed` /
//     `PointerReleased` / `PointerScrolled`.
//   * Different feeders. Keyboard ring is fed from the IRQ 1
//     scancode handler in `interrupts.rs`. Pointer ring will be fed
//     by the linuxkpi virtio-input driver in #459b — by way of
//     `linuxkpi::input::input_event(EV_REL/EV_ABS/...)` which lands
//     in `pointer::push(...)`. Keeping the rings separate avoids a
//     payload sum-type that one consumer (Slint) wouldn't fully
//     unwrap.
//
// Capacity matches keyboard.rs (64 slots) for the same reason: a
// stalled drainer can't pin the IRQ handler against a full buffer.
// Drop-oldest under back-pressure.

use alloc::collections::VecDeque;
use core::sync::atomic::{AtomicI32, Ordering};
use spin::Mutex;

/// One pointer event. Mirrors the subset of Linux input events that
/// virtio-input emits for a typical mouse / touchpad / touchscreen
/// device: relative motion, absolute position, button state, scroll.
///
/// Encoded as a flat enum (rather than a sum of separate event
/// types) so the ring stores one `PointerEvent` per push — matches
/// how `slint::WindowEvent` is shaped on the consumer side.
#[derive(Debug, Clone, Copy)]
pub enum PointerEvent {
    /// Relative motion delta (mouse). `dx, dy` in device units —
    /// accumulator at the consumer side resolves them to a screen
    /// coordinate.
    RelMove { dx: i32, dy: i32 },
    /// Absolute position (touchscreen / tablet). `x, y` in device
    /// space — the consumer applies the device's calibration to map
    /// to screen pixels.
    AbsMove { x: i32, y: i32 },
    /// Scroll-wheel delta. Positive = away from user, negative =
    /// toward. `delta` is in detents (real Linux REL_WHEEL is one
    /// click per +/-1).
    Scroll { delta: i32 },
    /// Button press / release. `button` is the Linux input-event
    /// `BTN_*` code (BTN_LEFT = 0x110, BTN_RIGHT = 0x111, BTN_MIDDLE
    /// = 0x112, BTN_TOUCH = 0x14a). `pressed = true` for a press,
    /// `false` for a release.
    Button { button: u32, pressed: bool },
    /// Sync barrier (EV_SYN / SYN_REPORT). Marks the end of a logical
    /// event group — the consumer should commit any accumulated
    /// state at this point. Linux drivers emit one per "frame" of
    /// input.
    Sync,
}

/// Capacity of the pending-event ring. Same value `keyboard.rs` uses
/// (64) — picked because at the maximum sane mouse rate (1 kHz USB
/// HID reports, each emitting ~3-5 events) the ring drains every
/// frame at 60 fps and never approaches half-full. 64 is a safe
/// over-allocation.
const RING_CAP: usize = 64;

/// Single global ring of pending pointer events. Single-writer (the
/// linuxkpi `input_event` thunk), multi-reader (currently zero
/// drainers; #459b plumbs the drain into the launcher per-frame).
///
/// `Mutex` not `RefCell` so the IRQ-context push side never trips
/// the borrow checker if the drainer is mid-iteration when an event
/// arrives.
static RING: Mutex<VecDeque<PointerEvent>> = Mutex::new(VecDeque::new());

/// Push one event onto the ring. Drops the oldest entry first if the
/// ring is full — same back-pressure shape `keyboard::handle_scancode`
/// uses. Designed to be called from the linuxkpi `input_event`
/// translation path (which itself runs at IRQ-or-tick context).
pub fn push(event: PointerEvent) {
    let mut ring = RING.lock();
    if ring.len() >= RING_CAP {
        ring.pop_front();
    }
    ring.push_back(event);
}

/// Pop the oldest pending event, if any. Non-blocking — returns
/// `None` when the ring is empty. Intended drainer is the launcher
/// super-loop's per-frame Slint event dispatch (#459b).
pub fn read_event() -> Option<PointerEvent> {
    RING.lock().pop_front()
}

/// Number of pending events. Useful for the boot smoke (idle vs
/// motion observed) and for back-pressure logic in any future
/// drainer that wants to throttle its work against input rate.
pub fn pending() -> usize {
    RING.lock().len()
}

/// Drain everything in the ring into a caller-supplied closure. The
/// closure is invoked once per event in FIFO order. Useful for the
/// per-frame Slint dispatch path that wants to consume the whole
/// queue in one pass without per-event lock overhead.
pub fn drain<F: FnMut(PointerEvent)>(mut f: F) {
    let mut ring = RING.lock();
    while let Some(e) = ring.pop_front() {
        f(e);
    }
}

/// Push one event onto the ring. Naming-aligned alias of `push` —
/// mirrors the `push_keystroke` API on `arch::uefi::keyboard` so
/// every synthetic-input feeder (Slint touch handler, virtio-input
/// driver, future on-screen pointer cell) reads as a symmetric
/// pair `push_keystroke` / `push_pointer_event` without callers
/// having to remember which ring uses which verb.
///
/// The `push_keystroke` companion (Track QQQQ #465) lives on
/// `arch::uefi::keyboard`; the rationale comment there explains why
/// the existing IRQ-driven push path co-exists with a parallel
/// synthetic-input push (single Mutex, drop-oldest under back-
/// pressure, safe to call from a Slint callback in the kernel
/// super-loop). Same shape applies here — `push_pointer_event` is a
/// thin re-export of `push` so #466 Track XXXXX's launcher-side
/// pointer drain has a verb that visually matches the keystroke
/// drain it sits alongside.
pub fn push_pointer_event(event: PointerEvent) {
    push(event);
}

/// Persistent on-screen cursor position. Read by the per-frame
/// cursor-sprite painter (`paint_cursor_sprite`) and seeded into the
/// EV_REL accumulator at the top of every Slint pointer drain so
/// relative motion deltas accumulate against the user's real screen
/// position rather than resetting to (0, 0) every frame. Updated at
/// the end of each `Sync` flush via `set_position`.
///
/// Two `AtomicI32` cells (rather than a `Mutex<(i32, i32)>`) so the
/// cursor-sprite painter — which runs on the kernel super-loop's
/// hot path between `draw_if_needed` and the `pause` idle — never
/// contends with the drain side that's writing the next position.
/// Reads/writes use `Relaxed` ordering: both axes are independent
/// scalars, the painter's "torn" read of one axis from frame N and
/// the other from frame N+1 is visually indistinguishable from a
/// one-frame-late position (sub-millisecond display artefact, no
/// correctness consequence). Same shape `keyboard.rs` doesn't need
/// because keystrokes don't have a "current value" — only a queue.
///
/// Initial value (0, 0) is the pre-motion sentinel that
/// `paint_cursor_sprite` checks for to skip painting the corner
/// arrow on every boot before any pointer device has delivered an
/// event.
static CURSOR_X: AtomicI32 = AtomicI32::new(0);
static CURSOR_Y: AtomicI32 = AtomicI32::new(0);

/// Read the persistent cursor position. Returns `(x, y)` in screen
/// pixels — same coordinate space the cursor-sprite painter and the
/// Slint `WindowEvent::PointerMoved` LogicalPosition use.
///
/// Transitional shim for the launcher's super-loop (#647). The
/// `pointer::drain` consumer in `drain_pointer_into_slint_window`
/// seeds its EV_REL accumulator from this value so deltas survive
/// across frames; the `paint_cursor_sprite` helper reads the same
/// value to know where to draw the arrow. Both call sites want a
/// plain `(i32, i32)` rather than the richer per-window state the
/// future windowing surface (#459d) will track.
pub fn current_position() -> (i32, i32) {
    (CURSOR_X.load(Ordering::Relaxed), CURSOR_Y.load(Ordering::Relaxed))
}

/// Write the persistent cursor position. Called from the Slint
/// pointer drain at each `PointerEvent::Sync` flush, after the
/// EV_REL/EV_ABS accumulator has been dispatched as a single
/// `WindowEvent::PointerMoved`.
///
/// Transitional shim for the launcher's super-loop (#647). See
/// `current_position` for the two-`AtomicI32` rationale.
pub fn set_position(x: i32, y: i32) {
    CURSOR_X.store(x, Ordering::Relaxed);
    CURSOR_Y.store(y, Ordering::Relaxed);
}

// ── Tests ──────────────────────────────────────────────────────────
//
// Kernel `[[bin]]` runs with `test = false` (Cargo.toml L98), so
// `cargo test` does not exercise these — they document the ring
// invariants the launcher-side drainer relies on (`push_pointer_event`
// is a no-data-loss alias of `push`; `drain` walks FIFO; `pending`
// counts unconsumed entries).

#[cfg(test)]
mod tests {
    use super::*;

    /// `push_pointer_event` is wire-compatible with `push`: a value
    /// pushed via the alias comes back through `read_event` byte-for-
    /// byte. The launcher-side `drain_pointer_into_focused_window`
    /// helper (Track XXXXX #466) relies on this — a Slint touch
    /// handler that synthesises a `PointerEvent::Button` via the
    /// alias must be observable to the next super-loop tick's drain.
    #[test]
    fn push_pointer_event_round_trips_through_read() {
        // Drain any leftover entries from prior tests — RING is a
        // module-level static and previous runs may have left
        // entries unconsumed.
        while read_event().is_some() {}

        push_pointer_event(PointerEvent::AbsMove { x: 42, y: 17 });
        match read_event() {
            Some(PointerEvent::AbsMove { x: 42, y: 17 }) => {}
            other => panic!("expected AbsMove(42, 17), got {:?}", other),
        }
        assert!(read_event().is_none());
    }

    /// `set_position` followed by `current_position` round-trips
    /// the cursor coordinate. The launcher-side
    /// `drain_pointer_into_slint_window` (Track XXXXX #466) and
    /// `paint_cursor_sprite` (#596 follow-up) rely on this shim
    /// to seed the EV_REL accumulator and to find the sprite's
    /// draw location respectively (#647).
    #[test]
    fn current_position_round_trips_through_set_position() {
        set_position(123, -45);
        assert_eq!(current_position(), (123, -45));
        set_position(0, 0);
        assert_eq!(current_position(), (0, 0));
    }
}
