// crates/arest-kernel/src/ui_apps/keyboard.rs
//
// Virtual keyboard — Rust glue for the fourth launchable Slint app
// (#465 Track QQQQ).
//
// Bridges the touch-driven `Keyboard` Slint component (declared in
// `ui/apps/Keyboard.slint`) to the kernel's PS/2 keyboard ring
// (`crate::arch::uefi::keyboard`). Mirrors the constructor shape
// Track SSS / TTT / VVV established for the prior three apps: a
// `pub fn build_app() -> Result<KeyboardApp, slint::PlatformError>`
// that returns the constructed Window plus a small bag of state.
//
// # Why route through the keyboard ring
//
// The kernel already has a single-consumer keyboard ring populated
// by the IRQ 1 handler (`arch::uefi::keyboard::handle_scancode`,
// Track EE / commit `8d9c14e`). Every existing app drains that ring:
//
//   * The launcher's `drain_keyboard_with_esc_intercept` reads Esc
//     for back-to-launcher and forwards other Unicode keys to the
//     active Slint window.
//   * Doom's `DoomApp::drain_keystrokes_intercept_esc` reads from
//     the same ring and routes through `crate::doom::translate_decoded_key`
//     to the wasmi guest's `reportKeyDown` / `reportKeyUp` exports.
//   * `arch::uefi::slint_input::drain_keyboard_into_slint_window`
//     translates each `DecodedKey` into a Slint `WindowEvent::KeyPressed`
//     + `KeyReleased` pair for the focused window.
//
// By pushing synthesised keystrokes onto the same ring, the virtual
// keyboard slots in transparently — none of the consumers need to
// know whether the byte came from a real PS/2 keyboard or from a
// touch on a Slint key cell. The `push_keystroke(DecodedKey)` API
// added to `arch::uefi::keyboard` is the single new entry point.
//
// # Why the keyboard surfaces its own Window vs. an overlay
//
// The keyboard is implemented as a self-contained `Window`-derived
// Slint component (same shape as Doom / Repl / HateoasBrowser) and
// the launcher swaps to it like any other app. This commit ships
// the simplest wiring; a follow-up can layer overlay support (the
// keyboard floats above another app's window) once the multi-window
// fallback under `MinimalSoftwareWindow` is exercised by another
// flow that proves the show-two-at-once pattern works.
//
// In the meantime, the user-flow on a touch-only display is:
//
//   1. Launcher splash shown, user taps the Keyboard icon.
//   2. Keyboard window opens. User taps keys; each tap pushes a
//      `DecodedKey::Unicode(c)` onto the ring.
//   3. The launcher's super-loop drains the ring next frame; the
//      `drain_keyboard_with_esc_intercept` arm intercepts Esc to
//      swap back to the launcher splash. Other keys would reach the
//      Keyboard's own Slint window (which doesn't consume them — see
//      the FocusScope notes in `Keyboard.slint`) and be silently
//      dropped.
//
// The "useful" usage pattern the milestone unlocks is then: tap
// Keyboard from the launcher, switch back, tap REPL — the REPL now
// receives keystrokes from whatever the keyboard pushed. The
// per-app window swap drains the ring fresh each time so a stale
// burst from the prior app is picked up by the new one. A future
// "always-on overlay" mode that keeps the keyboard visible while
// another app is focused is the natural next step.
//
// # Init policy
//
// The Slint platform must be installed before `build_app()` runs
// (`slint::platform::set_platform(Box::new(UefiSlintPlatform::new(...)))`)
// — Slint refuses to instantiate components otherwise. The launcher's
// `run()` does that on its first call before reaching `build_app()`.
//
// The window is *not* shown here; the caller (launcher) drives the
// Slint-side show / hide based on user navigation.

#![allow(dead_code)]

use alloc::string::String;
use core::cell::Cell;

use slint::{ComponentHandle, SharedString};

use pc_keyboard::DecodedKey;

use crate::arch::uefi::keyboard;
use crate::arch::uefi::slint_backend::Keyboard as KeyboardWindow;

/// The constructed Keyboard Window plus its small per-app state.
/// Returned from `build_app` so the boot UI launcher (Track UUU
/// #431) can `show()` / `hide()` the window like the other apps.
pub struct KeyboardApp {
    /// The Slint window. `ComponentHandle` requires the inner
    /// `Keyboard` component to stay alive for the duration of the
    /// event loop; `KeyboardApp` holds it by value to make that
    /// lifetime explicit.
    pub window: KeyboardWindow,
}

impl KeyboardApp {}

/// Construct the Keyboard Window and wire its `key-pressed` callback
/// to the kernel keyboard ring. Same shape as `ui_apps::repl::build_app`
/// / `ui_apps::doom::build_app` / `ui_apps::hateoas::build_app`.
///
/// On every tap the Slint side fires `key-pressed(string)` with the
/// codepoint as a `SharedString`. We decode the first char (every
/// codepoint we declare in `Keyboard.slint` is exactly one Unicode
/// scalar — printable letter, digit, punctuation, or one of the
/// control codepoints U+0008/U+0009/U+000A/U+001B/U+007F that Slint
/// uses for Backspace / Tab / Return / Escape / Delete) and push a
/// `DecodedKey::Unicode(c)` onto the ring via
/// `arch::uefi::keyboard::push_keystroke`.
///
/// We also bump a `Cell<u64>` counter and update the window's
/// `status-text` so the user gets feedback that the tap was seen.
/// The counter is `Cell<u64>` (not `RefCell`) because we only ever
/// `get` + `set` it, and the `Cell` API is enough for that — no
/// borrow tracking needed.
///
/// Failure modes:
///   * `slint::PlatformError` from `KeyboardWindow::new()` —
///     propagated to the caller so the launcher can decide whether
///     to abort boot or fall through to a degraded splash.
pub fn build_app() -> Result<KeyboardApp, slint::PlatformError> {
    let window = KeyboardWindow::new()?;

    // Press counter — incremented on every successful dispatch so
    // the status footer reflects activity. `Cell<u64>` clones into
    // the closure as a copy; no Rc needed because the closure
    // captures the window weak ref (which holds the only handle to
    // the live Slint instance) and the counter as Copy state.
    //
    // We could parametrise the counter via Slint's in-out property
    // surface, but the status-text update keeps the wiring local
    // and avoids growing the Slint surface.
    let press_count = alloc::rc::Rc::new(Cell::new(0u64));

    // Wire the key-pressed callback. This is the load-bearing line
    // for the whole app: every keystroke routes through here.
    {
        let weak = window.as_weak();
        let press_count = press_count.clone();
        window.on_key_pressed(move |codepoint: SharedString| {
            // The Slint side guarantees codepoint is non-empty (every
            // declared Key has either a `label` or `codepoint` of
            // at least one char), but defend against a future
            // refactor that introduces an empty cell — `chars().next()`
            // returning None means "drop this dispatch".
            let Some(c) = codepoint.as_str().chars().next() else {
                crate::println!("keyboard: empty codepoint on key-pressed; dropping");
                return;
            };

            // Push the decoded keystroke onto the ring. Every
            // consumer of the ring (the launcher's per-frame drain,
            // the REPL pump, Doom's drainer when active) sees it on
            // its next read just as if the IRQ 1 handler had fed
            // the corresponding scancode through the pc-keyboard
            // decoder. The launcher's Esc intercept (Unicode
            // U+001B) still works against synthesised input —
            // that's how the user navigates back to the launcher
            // from the Keyboard app via the on-screen "Esc"
            // sequence (which the current Keyboard.slint surface
            // doesn't expose explicitly, but a future mode could
            // add — for now, the navigation back happens via a
            // real Esc keystroke from the host or via a QEMU
            // monitor command in development).
            keyboard::push_keystroke(DecodedKey::Unicode(c));

            // Bump the counter and reflect it in the footer. We
            // resolve the weak reference inside the closure so the
            // closure stays decoupled from the window's lifetime —
            // if the launcher has dropped the window, `upgrade()`
            // returns None and we silently skip the status update
            // (the dispatch above already happened, which is the
            // semantically important side effect).
            let next = press_count.get().wrapping_add(1);
            press_count.set(next);
            if let Some(window) = weak.upgrade() {
                let label = format_key_label(c);
                let msg = alloc::format!(
                    "Keyboard — dispatched {next} keystroke(s); last: {label}"
                );
                window.set_status_text(SharedString::from(msg));
            }
        });
    }

    // Theme / mode toggles are passive forwards — Slint already
    // mutates its local state inside the handler. The Rust-side
    // hooks exist so a future `ThemePref` cell can persist the
    // mode choice without re-touching Keyboard.slint.
    window.on_theme_toggled(|| {});
    window.on_mode_changed(|| {});

    Ok(KeyboardApp { window })
}

/// Format a single dispatched codepoint as a short, user-readable
/// label for the status footer. Printable ASCII renders as itself
/// (wrapped in single quotes); the handful of control codepoints we
/// emit get a named alias so the user can tell what was sent.
///
/// We don't try to cover every Unicode control codepoint — only the
/// ones the on-screen keyboard actually emits. Anything else falls
/// through to a `U+xxxx` hex form, which is informative without
/// requiring the full Unicode tables in the kernel binary.
fn format_key_label(c: char) -> String {
    match c {
        '\u{0008}' => "Backspace".into(),
        '\u{0009}' => "Tab".into(),
        '\u{000A}' => "Enter".into(),
        '\u{000D}' => "Return".into(),
        '\u{001B}' => "Esc".into(),
        '\u{007F}' => "Delete".into(),
        ' ' => "Space".into(),
        c if (' '..='~').contains(&c) => alloc::format!("'{c}'"),
        c => alloc::format!("U+{:04X}", c as u32),
    }
}

// ── Tests ─────────────────────────────────────────────────────────
//
// `arest-kernel`'s bin target has `test = false` (Cargo.toml L33),
// so these `#[cfg(test)]` cases are reachable only when the crate is
// re-shaped into a lib for hosted testing — same convention the
// other `ui_apps` modules use. They document the intended behaviour
// and form a smoke battery for the day the kernel grows a lib facade.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_key_label_printable_ascii() {
        assert_eq!(format_key_label('a'), "'a'");
        assert_eq!(format_key_label('Z'), "'Z'");
        assert_eq!(format_key_label('5'), "'5'");
        assert_eq!(format_key_label('!'), "'!'");
        assert_eq!(format_key_label('~'), "'~'");
    }

    #[test]
    fn format_key_label_named_controls() {
        assert_eq!(format_key_label(' '), "Space");
        assert_eq!(format_key_label('\u{0008}'), "Backspace");
        assert_eq!(format_key_label('\u{0009}'), "Tab");
        assert_eq!(format_key_label('\u{000A}'), "Enter");
        assert_eq!(format_key_label('\u{000D}'), "Return");
        assert_eq!(format_key_label('\u{001B}'), "Esc");
        assert_eq!(format_key_label('\u{007F}'), "Delete");
    }

    #[test]
    fn format_key_label_falls_back_to_hex() {
        // Pick a non-ASCII codepoint we don't name explicitly.
        assert_eq!(format_key_label('\u{00E9}'), "U+00E9"); // é
        assert_eq!(format_key_label('\u{2603}'), "U+2603"); // snowman
    }

    /// Smoke test: `build_app()` constructs without panicking. Mirrors
    /// the shape of `doom::tests::build_app_constructs_under_minimal_window`
    /// and the `appshell_constructs_under_minimal_window` test in
    /// `slint_backend.rs`. Installs a `UefiSlintPlatform` so the Slint
    /// codegen for `Keyboard` runs through `register_bitmap_font` +
    /// the component constructor.
    #[test]
    fn build_app_constructs_under_minimal_window() {
        use crate::arch::uefi::slint_backend::UefiSlintPlatform;
        let platform = UefiSlintPlatform::new(1024, 720);
        let _ = slint::platform::set_platform(alloc::boxed::Box::new(platform));

        let _app = build_app().expect("Keyboard construction failed");
    }
}
