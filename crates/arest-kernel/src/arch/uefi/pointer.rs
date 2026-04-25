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
