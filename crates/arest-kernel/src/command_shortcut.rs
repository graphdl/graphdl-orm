// crates/arest-kernel/src/command_shortcut.rs
//
// CommandShortcut cell — fact-driven keyboard shortcut dispatch (Task U3c).
//
// # Problem
//
// `drain_keyboard_with_esc_intercept` in `ui_apps::launcher` silently
// drops ALL `DecodedKey::RawKey(_)` entries (arrow keys, function keys,
// modifiers, navigation cluster). The documented comment at lines
// 992-1002 reads:
//
//   "RawKey entries (Arrow keys, Function row, modifiers, navigation
//    cluster) are dropped silently here. The REPL app's history walk
//    uses Up/Down which the pc-keyboard US-104 layout decodes to RawKey
//    — so history navigation is broken when the REPL is launched
//    through the launcher."
//
// This is the ROOT BUG: Up/Down history navigation works when the REPL
// receives keystrokes via `drain_keyboard_into_slint_window` (which has
// the full RawKey → Slint mapping table from `slint_input.rs`), but is
// silently discarded in the launcher's Esc-intercept drain path — which
// is the NORMAL UEFI boot path.
//
// # Design
//
// A `Command_has_Shortcut` fact cell is seeded into the SYSTEM graph at
// boot, recording which RawKey-class shortcuts should be forwarded to
// the active Slint window rather than dropped:
//
//   Command_has_Shortcut { Command: "history-up",   Shortcut: "ArrowUp"   }
//   Command_has_Shortcut { Command: "history-down", Shortcut: "ArrowDown" }
//   Command_has_Shortcut { Command: "clear",        Shortcut: "Ctrl-L"    }
//   Command_has_Shortcut { Command: "back",         Shortcut: "Escape"    }
//
// `is_forwarded_raw_shortcut(key_name, state) -> bool` reads this cell
// and returns `true` when `key_name` matches a seeded shortcut's
// `Shortcut` binding — making the forward/drop decision a fact-driven
// lookup rather than a hardcoded `RawKey(_) => { /* drop */ }` arm.
//
// `resolve_command(key_name, state) -> Option<String>` is the companion
// that resolves the command name for a shortcut key — mirrors
// `back_action::resolve_shortcut` but operates on the CommandShortcut
// cell rather than the BackAction cell.
//
// # Shortcut key name convention
//
// Shortcut names in the cell follow the `slint_input.rs` + Slint key
// constant vocabulary:
//   * Arrow keys: "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight".
//   * Navigation cluster: "Insert", "Home", "End", "PageUp", "PageDown".
//   * Modifiers + letter combos: "Ctrl-L" (Control+L as a named combo).
//   * Special keys: "Escape" (already handled by the Unicode '\u{001b}'
//     intercept above, included here for completeness and discoverability).
//
// `rawkey_to_shortcut_name` in `ui_apps::launcher` maps a
// `pc_keyboard::KeyCode` to one of these names so the RawKey arm can
// do the fact lookup without knowing the full table.
//
// # Host-testability
//
// This module is pure (no Slint, no UEFI, no `system::*` calls). Tests
// exercise:
//   - `resolve_command`: reads the cell and returns the command name.
//   - `is_forwarded_raw_shortcut`: returns true for seeded ArrowUp/Down.
//   - Cell round-trip: seed → read.
//   - `COMMAND_SHORTCUT_CELL` constant matches the canonical cell name.
//
// # Cell schema
//
//   Command_has_Shortcut { Command: <command-id>, Shortcut: <key-name> }
//
// Cell name: `COMMAND_SHORTCUT_CELL`

#![allow(dead_code)]

use arest::ast::{self, Object};

// ── Constants ──────────────────────────────────────────────────────

/// Cell name for the CommandShortcut shortcut cell.
/// Both `seed_command_shortcut_cell` and `resolve_command` use this name
/// so there is a single source of truth.
pub const COMMAND_SHORTCUT_CELL: &str = "Command_has_Shortcut";

/// Command id for history-up (ArrowUp in the REPL).
pub const CMD_HISTORY_UP: &str = "history-up";

/// Command id for history-down (ArrowDown in the REPL).
pub const CMD_HISTORY_DOWN: &str = "history-down";

/// Command id for clear (Ctrl-L in the REPL).
pub const CMD_CLEAR: &str = "clear";

/// Command id for back-to-launcher (Escape key).
pub const CMD_BACK: &str = "back";

/// Shortcut key name for the Up arrow.
pub const SHORTCUT_ARROW_UP: &str = "ArrowUp";

/// Shortcut key name for the Down arrow.
pub const SHORTCUT_ARROW_DOWN: &str = "ArrowDown";

/// Shortcut key name for clear (Ctrl-L).
pub const SHORTCUT_CTRL_L: &str = "Ctrl-L";

/// Shortcut key name for Escape.
pub const SHORTCUT_ESCAPE: &str = "Escape";

// ── Seeding ────────────────────────────────────────────────────────

/// Push the four canonical `Command_has_Shortcut` facts into `state`
/// and return the extended state:
///
///   history-up   → ArrowUp
///   history-down → ArrowDown
///   clear        → Ctrl-L
///   back         → Escape
///
/// Called from `launcher.rs` at boot (after `seed_back_action_cell`)
/// so the full command-shortcut map is present in the SYSTEM cell graph
/// as first-class facts. `system::apply_unchecked` is the caller's
/// responsibility; this function only constructs the cell-extended
/// state.
///
/// Idempotent-ish: `cell_push` appends to the sequence, so calling
/// this twice produces duplicate facts. The `resolve_command` reader
/// uses `find_map` which returns on the first match, so duplicates are
/// harmless (first-seeded fact wins).
pub fn seed_command_shortcut_cell(state: &Object) -> Object {
    use arest::ast::{cell_push, fact_from_pairs};

    // history-up → ArrowUp
    let s = cell_push(
        COMMAND_SHORTCUT_CELL,
        fact_from_pairs(&[
            ("Command", CMD_HISTORY_UP),
            ("Shortcut", SHORTCUT_ARROW_UP),
        ]),
        state,
    );

    // history-down → ArrowDown
    let s = cell_push(
        COMMAND_SHORTCUT_CELL,
        fact_from_pairs(&[
            ("Command", CMD_HISTORY_DOWN),
            ("Shortcut", SHORTCUT_ARROW_DOWN),
        ]),
        &s,
    );

    // clear → Ctrl-L
    let s = cell_push(
        COMMAND_SHORTCUT_CELL,
        fact_from_pairs(&[
            ("Command", CMD_CLEAR),
            ("Shortcut", SHORTCUT_CTRL_L),
        ]),
        &s,
    );

    // back → Escape
    cell_push(
        COMMAND_SHORTCUT_CELL,
        fact_from_pairs(&[
            ("Command", CMD_BACK),
            ("Shortcut", SHORTCUT_ESCAPE),
        ]),
        &s,
    )
}

// ── Resolution ─────────────────────────────────────────────────────

/// Read the `Command_has_Shortcut` cell from `state` and return
/// the command id bound to `shortcut_key` as an owned `String`, or
/// `None` when no fact matches.
///
/// Returns `Option<alloc::string::String>` (owned) rather than a
/// borrowed `&str` because `fetch_or_phi` returns a locally-owned
/// `Object` from which borrows cannot escape the function.
///
/// This is the pure, host-testable cell lookup that the launcher's
/// RawKey-forwarding path calls. When multiple facts bind the same
/// `Shortcut` (unusual but permitted), the first matching fact wins —
/// earlier-seeded facts have priority, consistent with
/// `back_action::resolve_shortcut`'s forward-iteration semantics.
pub fn resolve_command(shortcut_key: &str, state: &Object) -> Option<alloc::string::String> {
    use alloc::string::ToString;
    let cell = ast::fetch_or_phi(COMMAND_SHORTCUT_CELL, state);
    let facts = cell.as_seq()?.to_vec();
    facts.iter().find_map(|fact| {
        let key = ast::binding(fact, "Shortcut")?;
        if key != shortcut_key {
            return None;
        }
        ast::binding(fact, "Command").map(|s| s.to_string())
    })
}

/// Return `true` when `shortcut_key` is in the `Command_has_Shortcut`
/// cell in `state` — i.e. it is a shortcut that should be forwarded
/// to the active Slint window rather than dropped.
///
/// Called by `drain_keyboard_with_esc_intercept`'s RawKey arm in
/// `ui_apps::launcher`. When this returns `true` the caller maps the
/// `KeyCode` to its Slint `SharedString` representation and dispatches
/// a `KeyPressed` + `KeyReleased` pair; when `false` the entry is
/// dropped (the existing behaviour for unknown raw keys).
///
/// Shortcut names use the naming convention documented at the module
/// level: "ArrowUp", "ArrowDown", "Ctrl-L", "Escape".
///
/// Note: "Escape" is included in the seeded facts for completeness and
/// discoverability. In practice the Unicode '\u{001b}' arm in
/// `drain_keyboard_with_esc_intercept` intercepts Esc BEFORE the
/// RawKey arm is reached — the US-104 layout decodes Escape as Unicode,
/// not as a RawKey — so `is_forwarded_raw_shortcut("Escape", ...)` is
/// never called for the Esc keystroke in normal operation.
pub fn is_forwarded_raw_shortcut(shortcut_key: &str, state: &Object) -> bool {
    resolve_command(shortcut_key, state).is_some()
}

// ── Tests ──────────────────────────────────────────────────────────
//
// Pure cell-extraction functions — compiled unconditionally (no UEFI
// gate, no Slint gate) so they run under `cargo test --lib
// --target x86_64-pc-windows-msvc` alongside the other hosted tests.
//
// These tests cover the CommandShortcut cell round-trip for Task U3c:
// seed the cell, assert that `resolve_command` / `is_forwarded_raw_shortcut`
// return the correct values; verify edge-cases (empty state, unknown
// shortcut, multiple shortcuts).

#[cfg(test)]
mod tests {
    use super::*;
    use arest::ast::Object;

    // ── Constants sanity ──────────────────────────────────────────────

    /// COMMAND_SHORTCUT_CELL is the expected cell name.
    #[test]
    fn command_shortcut_cell_name_is_correct() {
        assert_eq!(COMMAND_SHORTCUT_CELL, "Command_has_Shortcut");
    }

    /// CMD_* constants match expected strings.
    #[test]
    fn command_id_constants_are_correct() {
        assert_eq!(CMD_HISTORY_UP, "history-up");
        assert_eq!(CMD_HISTORY_DOWN, "history-down");
        assert_eq!(CMD_CLEAR, "clear");
        assert_eq!(CMD_BACK, "back");
    }

    /// SHORTCUT_* constants match expected strings.
    #[test]
    fn shortcut_key_constants_are_correct() {
        assert_eq!(SHORTCUT_ARROW_UP, "ArrowUp");
        assert_eq!(SHORTCUT_ARROW_DOWN, "ArrowDown");
        assert_eq!(SHORTCUT_CTRL_L, "Ctrl-L");
        assert_eq!(SHORTCUT_ESCAPE, "Escape");
    }

    // ── Empty state ───────────────────────────────────────────────────

    /// Empty state → resolve_command returns None for any key.
    #[test]
    fn empty_state_resolve_returns_none() {
        let state = Object::phi();
        assert_eq!(resolve_command("ArrowUp", &state).as_deref(), None);
        assert_eq!(resolve_command("ArrowDown", &state).as_deref(), None);
        assert_eq!(resolve_command("Ctrl-L", &state).as_deref(), None);
        assert_eq!(resolve_command("Escape", &state).as_deref(), None);
    }

    /// Empty state → is_forwarded_raw_shortcut returns false.
    #[test]
    fn empty_state_is_forwarded_returns_false() {
        let state = Object::phi();
        assert!(!is_forwarded_raw_shortcut("ArrowUp", &state));
        assert!(!is_forwarded_raw_shortcut("ArrowDown", &state));
        assert!(!is_forwarded_raw_shortcut("unknown", &state));
    }

    // ── Seed → resolve round-trip ─────────────────────────────────────

    /// After seeding, ArrowUp resolves to history-up.
    #[test]
    fn seed_then_resolve_arrow_up_yields_history_up() {
        let state = seed_command_shortcut_cell(&Object::phi());
        assert_eq!(
            resolve_command(SHORTCUT_ARROW_UP, &state).as_deref(),
            Some(CMD_HISTORY_UP),
        );
    }

    /// After seeding, ArrowDown resolves to history-down.
    #[test]
    fn seed_then_resolve_arrow_down_yields_history_down() {
        let state = seed_command_shortcut_cell(&Object::phi());
        assert_eq!(
            resolve_command(SHORTCUT_ARROW_DOWN, &state).as_deref(),
            Some(CMD_HISTORY_DOWN),
        );
    }

    /// After seeding, Ctrl-L resolves to clear.
    #[test]
    fn seed_then_resolve_ctrl_l_yields_clear() {
        let state = seed_command_shortcut_cell(&Object::phi());
        assert_eq!(
            resolve_command(SHORTCUT_CTRL_L, &state).as_deref(),
            Some(CMD_CLEAR),
        );
    }

    /// After seeding, Escape resolves to back.
    #[test]
    fn seed_then_resolve_escape_yields_back() {
        let state = seed_command_shortcut_cell(&Object::phi());
        assert_eq!(
            resolve_command(SHORTCUT_ESCAPE, &state).as_deref(),
            Some(CMD_BACK),
        );
    }

    /// All four seeded shortcuts are resolvable.
    #[test]
    fn all_four_shortcuts_resolve_after_seed() {
        let state = seed_command_shortcut_cell(&Object::phi());
        assert!(resolve_command(SHORTCUT_ARROW_UP, &state).is_some());
        assert!(resolve_command(SHORTCUT_ARROW_DOWN, &state).is_some());
        assert!(resolve_command(SHORTCUT_CTRL_L, &state).is_some());
        assert!(resolve_command(SHORTCUT_ESCAPE, &state).is_some());
    }

    // ── is_forwarded_raw_shortcut ─────────────────────────────────────

    /// ArrowUp and ArrowDown return true (history navigation).
    #[test]
    fn arrow_keys_are_forwarded_after_seed() {
        let state = seed_command_shortcut_cell(&Object::phi());
        assert!(is_forwarded_raw_shortcut("ArrowUp", &state));
        assert!(is_forwarded_raw_shortcut("ArrowDown", &state));
    }

    /// Ctrl-L returns true (clear command).
    #[test]
    fn ctrl_l_is_forwarded_after_seed() {
        let state = seed_command_shortcut_cell(&Object::phi());
        assert!(is_forwarded_raw_shortcut("Ctrl-L", &state));
    }

    /// Escape returns true (included for completeness).
    #[test]
    fn escape_is_forwarded_after_seed() {
        let state = seed_command_shortcut_cell(&Object::phi());
        assert!(is_forwarded_raw_shortcut("Escape", &state));
    }

    /// An unknown key returns false even after seeding.
    #[test]
    fn unknown_key_is_not_forwarded_after_seed() {
        let state = seed_command_shortcut_cell(&Object::phi());
        assert!(!is_forwarded_raw_shortcut("F1", &state));
        assert!(!is_forwarded_raw_shortcut("Tab", &state));
        assert!(!is_forwarded_raw_shortcut("", &state));
        assert!(!is_forwarded_raw_shortcut("ArrowLeft", &state));
    }

    // ── Unknown shortcut ─────────────────────────────────────────────

    /// Resolving an unknown shortcut key returns None even after seed.
    #[test]
    fn unknown_shortcut_returns_none_after_seed() {
        let state = seed_command_shortcut_cell(&Object::phi());
        assert_eq!(resolve_command("q", &state).as_deref(), None);
        assert_eq!(resolve_command("F1", &state).as_deref(), None);
        assert_eq!(resolve_command("", &state).as_deref(), None);
        assert_eq!(resolve_command("ArrowLeft", &state).as_deref(), None);
    }

    // ── Multiple shortcut bindings ────────────────────────────────────

    /// An extra shortcut can be added and resolves independently.
    #[test]
    fn additional_shortcut_resolves_correctly() {
        use arest::ast::{cell_push, fact_from_pairs};
        let state = seed_command_shortcut_cell(&Object::phi());
        let state = cell_push(
            COMMAND_SHORTCUT_CELL,
            fact_from_pairs(&[("Command", "select-all"), ("Shortcut", "Ctrl-A")]),
            &state,
        );
        assert_eq!(
            resolve_command("Ctrl-A", &state).as_deref(),
            Some("select-all"),
        );
        // Existing shortcuts still resolve.
        assert_eq!(
            resolve_command(SHORTCUT_ARROW_UP, &state).as_deref(),
            Some(CMD_HISTORY_UP),
        );
    }

    // ── Malformed facts ──────────────────────────────────────────────

    /// A fact with only a Command binding (no Shortcut) is skipped.
    #[test]
    fn fact_missing_shortcut_binding_is_skipped() {
        use arest::ast::{cell_push, fact_from_pairs};
        let state = cell_push(
            COMMAND_SHORTCUT_CELL,
            fact_from_pairs(&[("Command", CMD_HISTORY_UP)]), // no Shortcut
            &Object::phi(),
        );
        assert_eq!(resolve_command("ArrowUp", &state).as_deref(), None);
    }

    /// A fact with only a Shortcut binding (no Command) is skipped.
    #[test]
    fn fact_missing_command_binding_is_skipped() {
        use arest::ast::{cell_push, fact_from_pairs};
        let state = cell_push(
            COMMAND_SHORTCUT_CELL,
            fact_from_pairs(&[("Shortcut", "ArrowUp")]), // no Command
            &Object::phi(),
        );
        // resolve_command finds the Shortcut match but `ast::binding(fact,
        // "Command")` returns None → find_map skips it → None.
        assert_eq!(resolve_command("ArrowUp", &state).as_deref(), None);
    }

    // ── First-match semantics ─────────────────────────────────────────

    /// When two facts bind the same Shortcut to different commands, the
    /// first one (earlier-seeded) wins.
    #[test]
    fn first_matching_fact_wins_for_duplicate_shortcut() {
        use arest::ast::{cell_push, fact_from_pairs};
        let state = seed_command_shortcut_cell(&Object::phi());
        // Add a conflicting "ArrowUp → jump-page" fact after the canonical ones.
        let state = cell_push(
            COMMAND_SHORTCUT_CELL,
            fact_from_pairs(&[("Command", "jump-page"), ("Shortcut", "ArrowUp")]),
            &state,
        );
        // The first match ("history-up") is returned.
        assert_eq!(
            resolve_command("ArrowUp", &state).as_deref(),
            Some(CMD_HISTORY_UP),
        );
    }

    // ── is_forwarded_raw_shortcut with no seed ────────────────────────

    /// Seeding only one shortcut: only that key is forwarded.
    #[test]
    fn partial_seed_only_forwards_seeded_keys() {
        use arest::ast::{cell_push, fact_from_pairs};
        let state = cell_push(
            COMMAND_SHORTCUT_CELL,
            fact_from_pairs(&[("Command", CMD_HISTORY_UP), ("Shortcut", SHORTCUT_ARROW_UP)]),
            &Object::phi(),
        );
        assert!(is_forwarded_raw_shortcut("ArrowUp", &state));
        assert!(!is_forwarded_raw_shortcut("ArrowDown", &state));
        assert!(!is_forwarded_raw_shortcut("Ctrl-L", &state));
    }
}
