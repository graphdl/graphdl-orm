// crates/arest-kernel/src/back_action.rs
//
// BackAction cell — fact-driven back-navigation dispatch (Task U3b).
//
// # Problem
//
// The launcher's keyboard drain (`drain_keyboard_with_esc_intercept`)
// performs the "navigate back to launcher" logic as three inline
// procedural statements repeated for every active app arm:
//
//   hide(app_window);
//   show(launcher);
//   *nav = Active::Launcher;
//
// This block is duplicated for UnifiedRepl, Keyboard, and Doom.
// Any future "back-button" path (a UI button that fires back-navigation)
// would add a third copy. Per the fact-driven build directive, duplicated
// procedural navigation paths must be unified: the action is a fact, the
// dispatch is a single helper that both keyboard and button paths call.
//
// # Design
//
// A `BackAction` fact is seeded into the SYSTEM cell graph at boot:
//
//   BackAction_has_Shortcut { BackAction: "back", Shortcut: "Escape" }
//
// This records, as a first-class fact, that the "back" action is bound
// to the Escape key. The cell can later be extended with additional
// shortcut bindings (e.g. a gamepad "B" button) without changing the
// dispatch code.
//
// `dispatch_shortcut(shortcut, nav, ...)` is the unified helper that
// both the key path (Esc from `drain_keyboard_with_esc_intercept`) and
// any future button path call. It reads the `BackAction_has_Shortcut`
// cell to resolve `shortcut → action`, then executes the action. Today
// the only action is `"back"` → hide current app / show launcher /
// set nav to `Active::Launcher`.
//
// # Host-testability
//
// This module is pure (no Slint, no UEFI, no `system::*` calls). Tests
// exercise:
//   - `resolve_shortcut`: reads the cell and returns the action name.
//   - `BackAction` cell round-trip: seed → read.
//   - `BACK_SHORTCUT` constant matches the canonical cell name.
//
// The UEFI-gated `dispatch_shortcut` helper in `ui_apps::launcher` is
// NOT here — it lives next to the `NavState`/`Active` types it mutates,
// and calls `resolve_shortcut` (from here) to do the cell lookup.
// This split keeps the fact-resolution layer host-testable while the
// Slint/NavState mutation stays in the launcher's gated module.
//
// # Cell schema
//
//   BackAction_has_Shortcut { BackAction: <action-id>, Shortcut: <key-name> }
//
// Cell name: `BACK_ACTION_CELL`
// Canonical action id: `BACK_ACTION_ID = "back"`
// Canonical shortcut: `BACK_SHORTCUT_KEY = "Escape"`

#![allow(dead_code)]

use arest::ast::{self, Object};

// ── Constants ──────────────────────────────────────────────────────

/// Cell name for the BackAction shortcut cell.
/// Both `seed_back_action_cell` and `resolve_shortcut` use this name
/// so there is a single source of truth.
pub const BACK_ACTION_CELL: &str = "BackAction_has_Shortcut";

/// The canonical BackAction identifier — the "back to launcher"
/// action. All shortcut facts whose `BackAction` binding equals this
/// id are treated as triggers for the back-navigation dispatch.
pub const BACK_ACTION_ID: &str = "back";

/// The shortcut key name bound to the back action at boot.
/// `resolve_shortcut("Escape", state)` returns `Some("back")` when
/// this fact is seeded.
pub const BACK_SHORTCUT_KEY: &str = "Escape";

// ── Seeding ────────────────────────────────────────────────────────

/// Push the canonical `BackAction_has_Shortcut` fact for `"Escape"
/// → "back"` into `state` and return the extended state.
///
/// Called from `launcher.rs` at boot so the association is present
/// in the SYSTEM cell graph as a first-class fact. `system::apply`
/// is the caller's responsibility; this function only constructs the
/// cell-extended state.
///
/// Idempotent-ish: `cell_push` appends to the sequence, so calling
/// this twice produces two facts with identical content. The
/// `resolve_shortcut` reader uses `find_map` which returns on the
/// first match, so duplicates are harmless.
pub fn seed_back_action_cell(state: &Object) -> Object {
    use arest::ast::{cell_push, fact_from_pairs};
    cell_push(
        BACK_ACTION_CELL,
        fact_from_pairs(&[
            ("BackAction", BACK_ACTION_ID),
            ("Shortcut", BACK_SHORTCUT_KEY),
        ]),
        state,
    )
}

// ── Resolution ─────────────────────────────────────────────────────

/// Read the `BackAction_has_Shortcut` cell from `state` and return
/// the action id bound to `shortcut_key` as an owned `String`, or
/// `None` when no fact matches.
///
/// Returns `Option<alloc::string::String>` (owned) rather than a
/// borrowed `&str` because `fetch_or_phi` returns a locally-owned
/// `Object` from which borrows cannot escape the function.
///
/// This is the pure, host-testable cell lookup that `dispatch_shortcut`
/// (in `ui_apps::launcher`) calls. Separating the resolution from the
/// dispatch keeps the fact-reading logic outside the UEFI/Slint gate.
///
/// When multiple facts bind the same `Shortcut` (unusual but
/// permitted), the first matching fact wins — earlier-seeded facts
/// have priority, consistent with forward-iteration semantics used
/// for uniqueness checks across this codebase.
pub fn resolve_shortcut(shortcut_key: &str, state: &Object) -> Option<alloc::string::String> {
    use alloc::string::ToString;
    let cell = ast::fetch_or_phi(BACK_ACTION_CELL, state);
    let facts = cell.as_seq()?.to_vec();
    facts.iter().find_map(|fact| {
        let key = ast::binding(fact, "Shortcut")?;
        if key != shortcut_key {
            return None;
        }
        ast::binding(fact, "BackAction").map(|s| s.to_string())
    })
}

// ── Tests ──────────────────────────────────────────────────────────
//
// Pure cell-extraction functions — compiled unconditionally (no UEFI
// gate, no Slint gate) so they run under `cargo test --lib
// --target x86_64-pc-windows-msvc` alongside the other hosted tests.
//
// These tests cover the BackAction cell round-trip for Task U3b:
// seed the cell, assert that `resolve_shortcut` returns the correct
// action id; verify edge-cases (empty state, unknown shortcut,
// multiple shortcuts).

#[cfg(test)]
mod tests {
    use super::*;
    use arest::ast::Object;

    // ── Constants sanity ──────────────────────────────────────────────

    /// BACK_ACTION_CELL is the expected cell name.
    #[test]
    fn back_action_cell_name_is_correct() {
        assert_eq!(BACK_ACTION_CELL, "BackAction_has_Shortcut");
    }

    /// BACK_ACTION_ID is "back".
    #[test]
    fn back_action_id_is_back() {
        assert_eq!(BACK_ACTION_ID, "back");
    }

    /// BACK_SHORTCUT_KEY is "Escape".
    #[test]
    fn back_shortcut_key_is_escape() {
        assert_eq!(BACK_SHORTCUT_KEY, "Escape");
    }

    // Helper: unwrap resolve_shortcut to &str for assertions.
    // resolve_shortcut returns Option<String>; .as_deref() gives Option<&str>.
    fn resolve(key: &str, state: &Object) -> Option<alloc::string::String> {
        resolve_shortcut(key, state)
    }

    // ── Empty state ───────────────────────────────────────────────────

    /// Empty state → resolve_shortcut returns None for any key.
    #[test]
    fn empty_state_resolve_returns_none() {
        let state = Object::phi();
        assert_eq!(resolve("Escape", &state).as_deref(), None);
        assert_eq!(resolve("q", &state).as_deref(), None);
    }

    // ── Seed → resolve round-trip ─────────────────────────────────────

    /// Seeding then resolving "Escape" yields "back".
    #[test]
    fn seed_then_resolve_escape_yields_back() {
        let state = seed_back_action_cell(&Object::phi());
        let action = resolve(BACK_SHORTCUT_KEY, &state);
        assert_eq!(action.as_deref(), Some(BACK_ACTION_ID));
    }

    /// After seeding, resolving "Escape" returns Some("back").
    #[test]
    fn resolve_escape_after_seed_is_some_back() {
        let state = seed_back_action_cell(&Object::phi());
        assert_eq!(resolve("Escape", &state).as_deref(), Some("back"));
    }

    // ── Unknown shortcut ─────────────────────────────────────────────

    /// Resolving an unknown shortcut key (not "Escape") returns None
    /// even after the canonical fact is seeded.
    #[test]
    fn unknown_shortcut_returns_none_after_seed() {
        let state = seed_back_action_cell(&Object::phi());
        assert_eq!(resolve("q", &state).as_deref(), None);
        assert_eq!(resolve("B", &state).as_deref(), None);
        assert_eq!(resolve("", &state).as_deref(), None);
    }

    // ── Multiple shortcut bindings ────────────────────────────────────

    /// A second shortcut can be added and resolves independently.
    #[test]
    fn additional_shortcut_resolves_correctly() {
        use arest::ast::{cell_push, fact_from_pairs};
        // Seed the canonical "Escape → back" fact.
        let state = seed_back_action_cell(&Object::phi());
        // Add a hypothetical "B → back" gamepad fact.
        let state = cell_push(
            BACK_ACTION_CELL,
            fact_from_pairs(&[("BackAction", BACK_ACTION_ID), ("Shortcut", "B")]),
            &state,
        );
        // Both shortcuts resolve to "back".
        assert_eq!(resolve("Escape", &state).as_deref(), Some("back"));
        assert_eq!(resolve("B", &state).as_deref(), Some("back"));
    }

    /// Different actions can share the same cell with different
    /// Shortcut keys and resolve independently.
    #[test]
    fn different_actions_resolve_to_own_id() {
        use arest::ast::{cell_push, fact_from_pairs};
        let state = seed_back_action_cell(&Object::phi());
        // Add a hypothetical "forward" action bound to "Tab".
        let state = cell_push(
            BACK_ACTION_CELL,
            fact_from_pairs(&[("BackAction", "forward"), ("Shortcut", "Tab")]),
            &state,
        );
        assert_eq!(resolve("Escape", &state).as_deref(), Some("back"));
        assert_eq!(resolve("Tab", &state).as_deref(), Some("forward"));
    }

    // ── Malformed facts ──────────────────────────────────────────────

    /// A fact with only a BackAction binding (no Shortcut) is skipped.
    #[test]
    fn fact_missing_shortcut_binding_is_skipped() {
        use arest::ast::{cell_push, fact_from_pairs};
        let state = cell_push(
            BACK_ACTION_CELL,
            fact_from_pairs(&[("BackAction", BACK_ACTION_ID)]), // no Shortcut
            &Object::phi(),
        );
        assert_eq!(resolve("Escape", &state).as_deref(), None);
    }

    /// A fact with only a Shortcut binding (no BackAction) is skipped.
    #[test]
    fn fact_missing_back_action_binding_is_skipped() {
        use arest::ast::{cell_push, fact_from_pairs};
        let state = cell_push(
            BACK_ACTION_CELL,
            fact_from_pairs(&[("Shortcut", "Escape")]), // no BackAction
            &Object::phi(),
        );
        // resolve_shortcut finds the Shortcut match but `ast::binding(fact,
        // "BackAction")` returns None → find_map skips it → None.
        assert_eq!(resolve("Escape", &state).as_deref(), None);
    }

    // ── First-match semantics ─────────────────────────────────────────

    /// When two facts bind the same Shortcut to different actions, the
    /// first one (earlier-seeded) wins.
    #[test]
    fn first_matching_fact_wins_for_duplicate_shortcut() {
        use arest::ast::{cell_push, fact_from_pairs};
        // Seed "Escape → back" first.
        let state = seed_back_action_cell(&Object::phi());
        // Add a conflicting "Escape → other" fact after.
        let state = cell_push(
            BACK_ACTION_CELL,
            fact_from_pairs(&[("BackAction", "other"), ("Shortcut", "Escape")]),
            &state,
        );
        // The first match ("back") is returned.
        assert_eq!(resolve("Escape", &state).as_deref(), Some("back"));
    }
}
