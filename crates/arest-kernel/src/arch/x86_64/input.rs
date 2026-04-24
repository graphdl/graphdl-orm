// crates/arest-kernel/src/arch/x86_64/input.rs
//
// Doom-oriented key event ring buffer. Tracks press / release events
// the way Doom's game loop expects — separate from the REPL's line
// buffer, which only wants decoded Unicode chars.
//
// Flow:
//
//   scancode (IRQ 1)
//     -> pc-keyboard `Keyboard::add_byte` -> `KeyEvent { code, state }`
//     -> `input::push_event` translates to Doom key code + direction
//     -> VecDeque stash (bounded by RING_CAP)
//
//   Doom main loop (via the shim in #270)
//     -> `input::pop_event()` drains oldest-first
//     -> maps to the guest's internal `evtype_keydown` / `evtype_keyup`.
//
// Why a separate ring from the REPL's existing keystroke path: Doom
// cares about key *state* transitions (KEY_FIRE held while moving
// forward), not about line-editing semantics. The REPL path stays
// unchanged — the keyboard handler now forwards the decoded Unicode
// char to the REPL AND translates the raw KeyEvent into a DoomKey
// event here. Either consumer can ignore the other's view of the
// keystroke.
//
// Capacity: 64 slots. Design-doc suggestion from the Doom host-shim
// plan — a typical Doom frame consumes 0-5 events. Full-ring policy
// is drop-oldest, which bounds the lag between hardware and game
// loop even if the game stalls.

use alloc::collections::VecDeque;
use pc_keyboard::{KeyCode, KeyEvent, KeyState};
use spin::Mutex;

/// Doom key codes. Subset of `doomgeneric/doomkeys.h` — the ones
/// reachable from a PS/2 keyboard. ASCII printables pass through as
/// themselves; special keys get the high-bit-set codes the Doom
/// engine uses internally.
pub mod key_code {
    pub const KEY_RIGHTARROW: u8 = 0xae;
    pub const KEY_LEFTARROW:  u8 = 0xac;
    pub const KEY_UPARROW:    u8 = 0xad;
    pub const KEY_DOWNARROW:  u8 = 0xaf;
    pub const KEY_USE:        u8 = 0xa2;
    pub const KEY_FIRE:       u8 = 0xa3;
    pub const KEY_ESCAPE:     u8 = 27;
    pub const KEY_ENTER:      u8 = 13;
    pub const KEY_TAB:        u8 = 9;
    pub const KEY_BACKSPACE:  u8 = 0x7f;
    pub const KEY_RSHIFT:     u8 = 0xb6;
    pub const KEY_RALT:       u8 = 0xb8;
}

/// Press-or-release event destined for Doom. Carries the Doom key
/// code (not the raw scancode) so the consumer doesn't need to know
/// about `pc-keyboard`'s internal vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoomKeyEvent {
    Pressed(u8),
    Released(u8),
}

const RING_CAP: usize = 64;

static RING: Mutex<VecDeque<DoomKeyEvent>> = Mutex::new(VecDeque::new());

/// Translate a `pc-keyboard::KeyEvent` into a DoomKeyEvent + push
/// onto the ring. Called from the keyboard IRQ handler alongside the
/// existing REPL forwarding path. Drops oldest if the ring is full.
///
/// Printable ASCII letter + digit + symbol keys emit their ASCII
/// code as the Doom key. Special keys (arrows, shift, ctrl, esc,
/// ...) translate via the table in `translate_keycode`. Unknown
/// keys are silently ignored.
pub fn push_event(event: &KeyEvent) {
    let Some(doom_key) = translate_keycode(event.code) else {
        return;
    };
    let pushed = match event.state {
        KeyState::Down => DoomKeyEvent::Pressed(doom_key),
        KeyState::Up => DoomKeyEvent::Released(doom_key),
        // Treat PowerOnSelfTest / single-shot atomic events as a
        // press-then-release pair so the game's state tracker sees
        // the same transitions it would from a held-and-released key.
        KeyState::SingleShot => {
            let mut ring = RING.lock();
            enqueue(&mut ring, DoomKeyEvent::Pressed(doom_key));
            enqueue(&mut ring, DoomKeyEvent::Released(doom_key));
            return;
        }
    };
    let mut ring = RING.lock();
    enqueue(&mut ring, pushed);
}

/// Pop the oldest pending event, if any. Called from the Doom host
/// shim's input poll on every tic (#270/#271).
pub fn pop_event() -> Option<DoomKeyEvent> {
    RING.lock().pop_front()
}

/// Queue depth — useful for the boot smoke and for the host shim's
/// back-pressure logic.
pub fn pending() -> usize {
    RING.lock().len()
}

fn enqueue(ring: &mut VecDeque<DoomKeyEvent>, ev: DoomKeyEvent) {
    if ring.len() >= RING_CAP {
        ring.pop_front();
    }
    ring.push_back(ev);
}

/// Map `pc-keyboard::KeyCode` to a Doom key byte. Returns `None` for
/// keys Doom doesn't care about (function keys, media keys, numpad
/// arithmetic, etc.). ASCII letters / digits / common symbols pass
/// through as their ASCII codes.
fn translate_keycode(code: KeyCode) -> Option<u8> {
    use self::key_code::*;
    match code {
        // Cursor keys drive player movement.
        KeyCode::ArrowUp    => Some(KEY_UPARROW),
        KeyCode::ArrowDown  => Some(KEY_DOWNARROW),
        KeyCode::ArrowLeft  => Some(KEY_LEFTARROW),
        KeyCode::ArrowRight => Some(KEY_RIGHTARROW),

        // Modifiers. LCtrl = fire (Doom tradition), Space = use.
        KeyCode::LControl | KeyCode::RControl => Some(KEY_FIRE),
        KeyCode::Spacebar                     => Some(KEY_USE),
        KeyCode::LShift   | KeyCode::RShift   => Some(KEY_RSHIFT),
        KeyCode::LAlt     | KeyCode::RAltGr   => Some(KEY_RALT),

        // Menu / dialog navigation.
        KeyCode::Escape    => Some(KEY_ESCAPE),
        KeyCode::Return    => Some(KEY_ENTER),
        KeyCode::Tab       => Some(KEY_TAB),
        KeyCode::Backspace => Some(KEY_BACKSPACE),

        // ASCII letters — pass through as lowercase. Doom's internal
        // keymap uppercases where needed.
        KeyCode::A => Some(b'a'), KeyCode::B => Some(b'b'),
        KeyCode::C => Some(b'c'), KeyCode::D => Some(b'd'),
        KeyCode::E => Some(b'e'), KeyCode::F => Some(b'f'),
        KeyCode::G => Some(b'g'), KeyCode::H => Some(b'h'),
        KeyCode::I => Some(b'i'), KeyCode::J => Some(b'j'),
        KeyCode::K => Some(b'k'), KeyCode::L => Some(b'l'),
        KeyCode::M => Some(b'm'), KeyCode::N => Some(b'n'),
        KeyCode::O => Some(b'o'), KeyCode::P => Some(b'p'),
        KeyCode::Q => Some(b'q'), KeyCode::R => Some(b'r'),
        KeyCode::S => Some(b's'), KeyCode::T => Some(b't'),
        KeyCode::U => Some(b'u'), KeyCode::V => Some(b'v'),
        KeyCode::W => Some(b'w'), KeyCode::X => Some(b'x'),
        KeyCode::Y => Some(b'y'), KeyCode::Z => Some(b'z'),

        // Digits (main row — numpad stays out, since Doom doesn't
        // consult it beyond arrow aliasing).
        KeyCode::Key1 => Some(b'1'), KeyCode::Key2 => Some(b'2'),
        KeyCode::Key3 => Some(b'3'), KeyCode::Key4 => Some(b'4'),
        KeyCode::Key5 => Some(b'5'), KeyCode::Key6 => Some(b'6'),
        KeyCode::Key7 => Some(b'7'), KeyCode::Key8 => Some(b'8'),
        KeyCode::Key9 => Some(b'9'), KeyCode::Key0 => Some(b'0'),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    // Kernel binary does not run tests (test = false in Cargo.toml),
    // but these demonstrate invariants worth preserving if the kernel
    // ever grows a test harness. Kept as #[cfg(test)] so nothing
    // changes at build time.

    use super::*;

    #[test]
    fn ring_drop_oldest_when_full() {
        let mut ring = VecDeque::new();
        for i in 0u8..=(RING_CAP as u8) {
            enqueue(&mut ring, DoomKeyEvent::Pressed(i));
        }
        assert_eq!(ring.len(), RING_CAP);
        // Oldest (0) was dropped; next pop should be 1.
        assert_eq!(ring.pop_front(), Some(DoomKeyEvent::Pressed(1)));
    }
}
