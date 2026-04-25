// crates/arest-kernel/src/arch/uefi/slint_input.rs
//
// Slint input adapter (#428): drains the post-decode keyboard ring
// (`arch::uefi::keyboard::read_keystroke`, populated by the IRQ 1
// handler — Track EE / commit `8d9c14e`) and translates each pending
// `pc_keyboard::DecodedKey` into a paired
// `slint::platform::WindowEvent::KeyPressed` + `KeyReleased` dispatched
// to the caller-supplied `slint::Window`.
//
// Why paired press+release per ring entry:
//   The ring stores `DecodedKey`, which the decoder produces only on
//   key make (not key break — `pc-keyboard` swallows the release scan
//   codes after using them to update its modifier-state singleton).
//   So we have no real release signal to forward. For most Slint UI
//   that's fine: text input, button activation, focus traversal, and
//   menu shortcuts all key off the press edge. To keep widgets that DO
//   look at release (held-key visuals, key-up shortcuts) from getting
//   stuck in a "permanently held" state, we synthesise the matching
//   release immediately after the press. The combined press+release
//   pair runs against a single drain entry; Slint sees the press and
//   release back-to-back in the same dispatch pass.
//
// Why pure / single-pass:
//   The eventual main loop (#431) calls this once per frame in between
//   render and animation tick. A single pass over whatever the ring
//   has accumulated since the previous frame keeps frame budget
//   bounded and avoids re-entry against the IRQ writer (each
//   `read_keystroke` is a single `Mutex::lock` pair). The function
//   never blocks: when the ring is empty it returns 0 on the first
//   `read_keystroke()` and exits.
//
// Why `try_dispatch_event` errors are swallowed:
//   `Window::try_dispatch_event` only fails on `Resized` (renderer
//   resize) and `CloseRequested` (hide failure) — neither variant is
//   produced here. The remaining key/pointer variants are infallible
//   in practice. Returning `Result` from this helper would force every
//   caller to handle a case that can't fire; counting dispatched
//   events is the more useful return.
//
// Mouse: out of scope this commit. The UEFI arm has no PS/2 mouse
// IRQ wired (#364 stopped at IRQ 1 / keyboard); when a mouse ring
// lands, it'll get a sibling `drain_mouse_into_slint_window` here.

use pc_keyboard::{DecodedKey, KeyCode};
use slint::SharedString;
use slint::platform::{Key, WindowEvent};

use super::keyboard;

/// Drain every `DecodedKey` currently sitting in the
/// `arch::uefi::keyboard` ring and dispatch each as a
/// `KeyPressed` + `KeyReleased` pair to `window`.
///
/// Returns the number of *ring entries* drained (== number of
/// press+release pairs dispatched). One ring entry produces two
/// `WindowEvent`s, but the count returned is the entry count so
/// callers can compare it against `keyboard::pending()`-shaped
/// metrics directly.
///
/// Non-blocking. Returns 0 immediately when the ring is empty.
/// Single-pass: only entries already enqueued at call time are
/// drained — entries the IRQ handler appends mid-drain wait for the
/// next call. This keeps frame budget bounded and avoids livelock if
/// keys are arriving faster than the renderer can clear them.
///
/// Errors from `Window::try_dispatch_event` are swallowed: the
/// variants this adapter emits (`KeyPressed`, `KeyReleased`) are
/// infallible against the current Slint surface (only `Resized` and
/// `CloseRequested` can fail — see api.rs, `try_dispatch_event`).
#[allow(dead_code)]
pub fn drain_keyboard_into_slint_window(window: &slint::Window) -> usize {
    let mut count = 0usize;
    while let Some(decoded) = keyboard::read_keystroke() {
        let text = decoded_key_to_text(decoded);
        // Only dispatch if we got a non-empty SharedString — an empty
        // text would be a no-op key event with no meaningful payload
        // (Slint keys all carry at least one char). RawKey variants
        // we don't have a Slint mapping for (multimedia keys, OEM
        // keys, etc.) are dropped silently rather than synthesised
        // as garbage events.
        if !text.is_empty() {
            // Press + release pair. `text` is a SharedString
            // (refcounted Arc-of-bytes), so the clone is cheap.
            let _ = window.try_dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            let _ = window.try_dispatch_event(WindowEvent::KeyReleased { text });
        }
        count += 1;
    }
    count
}

/// Map a single `DecodedKey` into the `SharedString` payload Slint
/// expects in `WindowEvent::KeyPressed`/`KeyReleased`.
///
/// Two cases:
///   * `Unicode(c)` — pass `c` through. The control characters the
///     `pc-keyboard` US-104 layout emits (Backspace=U+0008, Tab=U+0009,
///     Return=U+000A, Escape=U+001B, Delete=U+007F, plus ASCII space
///     and printables) align *exactly* with the codepoints Slint's
///     `key_codes` table assigns to its named `Key` constants
///     (`Key::Backspace`, `Key::Tab`, `Key::Return`, `Key::Escape`,
///     `Key::Delete`, `Key::Space`). So no mapping table is needed for
///     the Unicode branch — the byte values match by construction.
///     See `i_slint_common::key_codes::for_each_keys!` macro
///     (Backspace=0x08, Tab=0x09, Return=0x0A, Escape=0x1B,
///     Delete=0x7F, Space=0x20).
///
///   * `RawKey(KeyCode)` — `pc-keyboard` returns this for any
///     scancode the layout doesn't translate to a Unicode char:
///     arrow keys, function keys (F1..F12), modifiers (LShift, RShift,
///     LControl, RControl, LAlt, RAltGr, LWin, RWin), Insert, Home,
///     End, PageUp, PageDown, multimedia keys, etc. We hand-map the
///     ones Slint has named `Key` constants for; everything else
///     (multimedia, OEM, JIS extras, Apps/Menu, NumpadLock) returns
///     an empty string and the caller drops the event.
///
/// The `SharedString` is built via `Key`'s `Into<SharedString>` impl
/// (which goes `Key -> char -> SharedString`) for special keys, and
/// via `char`'s `Into<SharedString>` for the Unicode branch.
fn decoded_key_to_text(decoded: DecodedKey) -> SharedString {
    match decoded {
        DecodedKey::Unicode(c) => c.into(),
        DecodedKey::RawKey(code) => raw_keycode_to_slint_key(code)
            .map(SharedString::from)
            .unwrap_or_default(),
    }
}

/// Translate a `pc_keyboard::KeyCode` (the RawKey payload) into the
/// matching Slint `Key` constant. Returns `None` for keycodes Slint
/// has no named constant for (multimedia, OEM 7-13, JIS extras, the
/// Apps / context-menu key on layouts that decode it as a raw code,
/// NumpadLock, PowerOnTestOk / TooManyKeys / RControl2 hidden codes).
///
/// Categories covered:
///   * **Arrow keys** — ArrowUp/Down/Left/Right → Key::UpArrow /
///     DownArrow / LeftArrow / RightArrow.
///   * **Navigation cluster** — Insert/Home/End/PageUp/PageDown →
///     same-named Slint key.
///   * **Function keys** — F1..F12 → Key::F1..F12. (Slint has F13..F24
///     defined too but no PC keyboard emits them via Set 1; the match
///     stops at F12.)
///   * **Modifiers** — LShift/RShift/LControl/RControl/LAlt/RAltGr/
///     LWin/RWin → Key::Shift/ShiftR/Control/ControlR/Alt/AltGr/
///     Meta/MetaR. Slint's Meta constant maps to the "Windows"/Super
///     key, matching `pc-keyboard`'s LWin/RWin semantics.
///   * **System keys** — CapsLock, ScrollLock, PrintScreen, SysRq,
///     PauseBreak → CapsLock/ScrollLock/SysReq/SysReq/Pause. SysRq and
///     PrintScreen both map to `Key::SysReq` (Slint's union of the
///     two — the table in `i_slint_common::key_codes` lists SysReq's
///     muda alias as `PrintScreen`).
///
/// Categories deliberately dropped:
///   * Multimedia (PrevTrack/NextTrack/Mute/VolumeUp/VolumeDown/Play/
///     Stop/Calculator/WWWHome) — Slint has Stop but not the others;
///     dropping the whole class avoids a partial mapping that would
///     surprise Slint apps relying on the modern set.
///   * Apps/Menu — Slint's `Key::Menu` exists, mapped here.
///   * Hidden / synthetic codes (PowerOnTestOk, TooManyKeys,
///     RControl2) — never user-meaningful.
///   * OEM 4..13 and Numpad* digit keys / NumpadLock — these all
///     decode to Unicode characters via the US-104 layout (or their
///     Numpad-cluster equivalents), so the RawKey branch only fires
///     for them on layouts that don't translate. Punting until a
///     non-US layout lands.
fn raw_keycode_to_slint_key(code: KeyCode) -> Option<Key> {
    Some(match code {
        // Arrow cluster.
        KeyCode::ArrowUp => Key::UpArrow,
        KeyCode::ArrowDown => Key::DownArrow,
        KeyCode::ArrowLeft => Key::LeftArrow,
        KeyCode::ArrowRight => Key::RightArrow,

        // Navigation cluster.
        KeyCode::Insert => Key::Insert,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,

        // Function row.
        KeyCode::F1 => Key::F1,
        KeyCode::F2 => Key::F2,
        KeyCode::F3 => Key::F3,
        KeyCode::F4 => Key::F4,
        KeyCode::F5 => Key::F5,
        KeyCode::F6 => Key::F6,
        KeyCode::F7 => Key::F7,
        KeyCode::F8 => Key::F8,
        KeyCode::F9 => Key::F9,
        KeyCode::F10 => Key::F10,
        KeyCode::F11 => Key::F11,
        KeyCode::F12 => Key::F12,

        // Modifiers — left/right pairs map to Slint's L/R variants.
        KeyCode::LShift => Key::Shift,
        KeyCode::RShift => Key::ShiftR,
        KeyCode::LControl => Key::Control,
        KeyCode::RControl => Key::ControlR,
        KeyCode::LAlt => Key::Alt,
        KeyCode::RAltGr => Key::AltGr,
        KeyCode::LWin => Key::Meta,
        KeyCode::RWin => Key::MetaR,

        // System / lock keys.
        KeyCode::CapsLock => Key::CapsLock,
        KeyCode::ScrollLock => Key::ScrollLock,
        KeyCode::PauseBreak => Key::Pause,
        // Slint folds PrintScreen under the SysReq constant — see
        // `i_slint_common::key_codes` (SysReq's muda alias is
        // "PrintScreen").
        KeyCode::PrintScreen | KeyCode::SysRq => Key::SysReq,

        // Apps / context menu key.
        KeyCode::Apps => Key::Menu,

        // Stop multimedia key — Slint has it as Key::Stop.
        KeyCode::Stop => Key::Stop,

        // Everything else (other multimedia, OEM 7..13, JIS extras,
        // NumpadLock, PowerOnTestOk, TooManyKeys, RControl2, plus
        // the printable letter/digit keys that should have been
        // decoded to Unicode by the layout) — drop. Returning None
        // lets the caller skip the dispatch entirely.
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    // Kernel binary does not run tests (test = false in Cargo.toml),
    // but these document the mapping invariants that matter for
    // downstream consumers. Kept as #[cfg(test)] so nothing changes
    // at build time.

    use super::*;

    #[test]
    fn unicode_passthrough_preserves_codepoint() {
        // Slint's Key::Backspace == U+0008, Key::Tab == U+0009,
        // Key::Return == U+000A, Key::Escape == U+001B,
        // Key::Delete == U+007F. The pc-keyboard US-104 layout emits
        // those exact codepoints for the corresponding scancodes,
        // so passing them through as Unicode is sufficient — no
        // RawKey mapping needed for the control-char keys.
        for c in ['\u{0008}', '\u{0009}', '\u{000a}', '\u{001b}', '\u{007f}', ' ', 'a', 'A', '5'] {
            let s = decoded_key_to_text(DecodedKey::Unicode(c));
            assert_eq!(s.as_str().chars().next(), Some(c));
        }
    }

    #[test]
    fn arrow_keys_map_to_slint_arrows() {
        assert_eq!(
            decoded_key_to_text(DecodedKey::RawKey(KeyCode::ArrowUp)),
            SharedString::from(Key::UpArrow),
        );
        assert_eq!(
            decoded_key_to_text(DecodedKey::RawKey(KeyCode::ArrowDown)),
            SharedString::from(Key::DownArrow),
        );
        assert_eq!(
            decoded_key_to_text(DecodedKey::RawKey(KeyCode::ArrowLeft)),
            SharedString::from(Key::LeftArrow),
        );
        assert_eq!(
            decoded_key_to_text(DecodedKey::RawKey(KeyCode::ArrowRight)),
            SharedString::from(Key::RightArrow),
        );
    }

    #[test]
    fn modifiers_map_to_left_right_variants() {
        assert_eq!(
            decoded_key_to_text(DecodedKey::RawKey(KeyCode::LShift)),
            SharedString::from(Key::Shift),
        );
        assert_eq!(
            decoded_key_to_text(DecodedKey::RawKey(KeyCode::RShift)),
            SharedString::from(Key::ShiftR),
        );
        assert_eq!(
            decoded_key_to_text(DecodedKey::RawKey(KeyCode::LControl)),
            SharedString::from(Key::Control),
        );
        assert_eq!(
            decoded_key_to_text(DecodedKey::RawKey(KeyCode::RControl)),
            SharedString::from(Key::ControlR),
        );
        assert_eq!(
            decoded_key_to_text(DecodedKey::RawKey(KeyCode::LAlt)),
            SharedString::from(Key::Alt),
        );
        assert_eq!(
            decoded_key_to_text(DecodedKey::RawKey(KeyCode::RAltGr)),
            SharedString::from(Key::AltGr),
        );
        assert_eq!(
            decoded_key_to_text(DecodedKey::RawKey(KeyCode::LWin)),
            SharedString::from(Key::Meta),
        );
        assert_eq!(
            decoded_key_to_text(DecodedKey::RawKey(KeyCode::RWin)),
            SharedString::from(Key::MetaR),
        );
    }

    #[test]
    fn unmapped_raw_keys_yield_empty_string() {
        // Multimedia keys aren't in Slint's Key enum (apart from Stop)
        // — they should drop silently rather than dispatch a garbage
        // event. PowerOnTestOk is a synthetic code that fires once at
        // boot; same treatment.
        for code in [
            KeyCode::PrevTrack,
            KeyCode::NextTrack,
            KeyCode::Mute,
            KeyCode::VolumeUp,
            KeyCode::VolumeDown,
            KeyCode::Play,
            KeyCode::PowerOnTestOk,
            KeyCode::TooManyKeys,
        ] {
            assert!(decoded_key_to_text(DecodedKey::RawKey(code)).is_empty());
        }
    }

    #[test]
    fn function_keys_map_one_to_one() {
        for (code, key) in [
            (KeyCode::F1, Key::F1),
            (KeyCode::F2, Key::F2),
            (KeyCode::F3, Key::F3),
            (KeyCode::F4, Key::F4),
            (KeyCode::F5, Key::F5),
            (KeyCode::F6, Key::F6),
            (KeyCode::F7, Key::F7),
            (KeyCode::F8, Key::F8),
            (KeyCode::F9, Key::F9),
            (KeyCode::F10, Key::F10),
            (KeyCode::F11, Key::F11),
            (KeyCode::F12, Key::F12),
        ] {
            assert_eq!(
                decoded_key_to_text(DecodedKey::RawKey(code)),
                SharedString::from(key),
            );
        }
    }
}
