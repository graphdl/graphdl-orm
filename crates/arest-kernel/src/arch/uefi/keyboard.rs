// crates/arest-kernel/src/arch/uefi/keyboard.rs
//
// PS/2 keyboard ring buffer for the UEFI x86_64 path (#364). Sibling
// of the BIOS arm's `arch::x86_64::input` (Doom-oriented `KeyEvent`
// ring) — but this module's role is narrower for now: the UEFI REPL
// is #365 scope, so we don't yet have a place to forward a decoded
// Unicode char. Instead, the ring stashes every `DecodedKey` the IRQ
// handler produces, and `read_keystroke()` lets the boot smoke (or a
// future REPL pump) drain the queue without blocking.
//
// Why a separate module from `arch::x86_64::input`:
//   * The x86_64 BIOS arm's `input` module is gated on
//     `not(target_os = "uefi")` (see `arch::mod`), so it isn't
//     reachable from a UEFI build — we can't just call
//     `super::input::push_event` here.
//   * The two rings carry different payloads: BIOS's is `KeyEvent`
//     (raw scancode-level press/release, for Doom's input loop);
//     this UEFI ring is `DecodedKey` (post-`pc-keyboard` translation,
//     suitable for the REPL line editor that #365 will wire). Keeping
//     them in separate modules avoids a cross-cutting shared type
//     that one consumer wouldn't use.
//
// Capacity: 64 slots — same as the BIOS `input` ring. Drop-oldest
// when full so a stalled drainer can't pin the IRQ handler against
// a full buffer.
//
// The 8259-PIC keyboard port (0x60) read + the `pc-keyboard`
// decoder both live in the IRQ handler in `interrupts.rs`. This
// module owns only the post-decode queue + decoder state singleton.

use alloc::collections::VecDeque;
use pc_keyboard::{DecodedKey, HandleControl, Keyboard, ScancodeSet1, layouts};
use spin::{Mutex, Once};

/// Shared `pc-keyboard` decoder state — tracks shift, caps lock,
/// extended-prefix scancodes across consecutive byte reads. The IRQ
/// handler holds the lock just long enough to feed one scancode and
/// extract zero-or-one events; readers (drain side) never touch it,
/// so the lock contention is effectively single-writer.
///
/// `Once` so the singleton lazy-inits exactly once on the first
/// `init()` call — same shape the BIOS arm uses in
/// `arch::x86_64::interrupts::KEYBOARD`.
static KEYBOARD: Once<Mutex<Keyboard<layouts::Us104Key, ScancodeSet1>>> = Once::new();

/// Capacity of the post-decode ring. 64 slots covers ~12 typical
/// keystrokes (each scancode emits 1-2 events between key make and
/// break), well above what a drainer that ticks every kernel idle
/// loop would observe between drains.
const RING_CAP: usize = 64;

/// Ring of decoded keystrokes pending consumer pickup. Single-writer
/// (the keyboard IRQ handler) / multi-reader, but the reader side is
/// expected to be a single REPL pump (#365 scope) — no fan-out today.
static RING: Mutex<VecDeque<DecodedKey>> = Mutex::new(VecDeque::new());

/// Initialise the keyboard decoder singleton. Idempotent — a second
/// call is a no-op. Called from `arch::uefi::interrupts::pic_init`
/// alongside the PIC remap, before the IRQ 1 mask is cleared, so
/// the decoder is ready before the first scancode can fire.
pub fn init() {
    KEYBOARD.call_once(|| {
        Mutex::new(Keyboard::new(
            ScancodeSet1::new(),
            layouts::Us104Key,
            HandleControl::Ignore,
        ))
    });
}

/// Feed one raw scancode through the decoder. If the decoder produces
/// a `DecodedKey` (Unicode or RawKey), enqueue it on the ring. Drops
/// the oldest entry first if the ring is full.
///
/// Called from the IRQ 1 handler in `interrupts.rs` once per port-0x60
/// read. Designed to run in interrupt context: a single `Mutex::lock`
/// pair (decoder + ring), no allocation beyond `VecDeque::push_back`'s
/// amortised growth (which is bounded by `RING_CAP` so the inner
/// buffer is grown at most once over the kernel's lifetime).
pub fn handle_scancode(scancode: u8) {
    let Some(keyboard) = KEYBOARD.get() else {
        // Decoder hasn't been init'd yet — drop the byte. This
        // shouldn't happen at runtime because pic_init() inits the
        // decoder before unmasking the IRQ, but a stray firmware-
        // pending scancode that fires between init_interrupts and
        // pic_init would land here. Safer to drop than to panic in
        // an ISR.
        return;
    };
    let mut kb = keyboard.lock();
    if let Ok(Some(event)) = kb.add_byte(scancode) {
        if let Some(decoded) = kb.process_keyevent(event) {
            let mut ring = RING.lock();
            if ring.len() >= RING_CAP {
                ring.pop_front();
            }
            ring.push_back(decoded);
        }
    }
}

/// Pop the oldest pending keystroke, if any. Non-blocking — returns
/// `None` immediately when the ring is empty. Intended drainer is the
/// REPL pump (#365); the boot smoke uses it to assert the IRQ pipeline
/// is live without depending on actual keyboard input.
pub fn read_keystroke() -> Option<DecodedKey> {
    RING.lock().pop_front()
}

/// Push a fully-decoded keystroke onto the ring directly, bypassing
/// the PS/2 scancode decoder. Designed for synthetic-input feeders
/// like the virtual on-screen keyboard (#465 Track QQQQ): the touch /
/// pointer event in the Slint Keyboard app maps a key cell to a
/// `DecodedKey::Unicode(c)` value and pushes here so the next call to
/// `read_keystroke` (or to one of the per-frame drainers in
/// `arch::uefi::slint_input` / `ui_apps::launcher`) sees it as if it
/// had come from the IRQ 1 path.
///
/// Drops the oldest entry first when the ring is full — same back-
/// pressure shape the `handle_scancode` path uses. Cheap (single
/// `Mutex::lock` + a `VecDeque::push_back`); safe to call from a
/// Slint callback in the kernel super-loop.
pub fn push_keystroke(decoded: DecodedKey) {
    let mut ring = RING.lock();
    if ring.len() >= RING_CAP {
        ring.pop_front();
    }
    ring.push_back(decoded);
}

/// Number of pending keystrokes in the ring. Useful for the boot
/// smoke ("idle" vs "scancode received") and for back-pressure logic
/// in any future drainer that wants to throttle output bandwidth
/// against input rate.
pub fn pending() -> usize {
    RING.lock().len()
}
