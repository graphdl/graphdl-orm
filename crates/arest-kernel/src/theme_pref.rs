// crates/arest-kernel/src/theme_pref.rs
//
// Cell-driven ThemePref extraction (#U3a Task).
//
// This module extracts the pure, UEFI-free cell-query functions so
// they can be unit-tested under the hosted `cargo test` target
// (x86_64-pc-windows-msvc) — the `ui_apps::launcher` module is
// gated on `target_os = "uefi"` and cannot host tests directly.
//
// # Design (mirrors launcher_app_set.rs / unified_repl_regions.rs)
//
// The theme preference is stored as a `ThemePref_has_Mode` cell fact:
//
//   ThemePref_has_Mode { ThemePref: "ui", Mode: "dark" | "light" }
//
// One canonical instance (`THEME_PREF_ID = "ui"`) carries the
// user's last-selected mode. The extraction is a pure function over
// `&Object` (the live SYSTEM state snapshot). No Slint types, no
// UEFI types, no `system::*` calls — the caller supplies the
// snapshot.
//
// # Send-safe subscriber → super-loop wiring
//
// `crate::system::subscribe_changes` requires a `Send` closure. A
// Slint `ComponentHandle` is `!Send`, so the subscriber CANNOT
// capture or hold a component to reach the Theme global. The
// wiring uses a `Send`-safe `AtomicU8` staging slot:
//
//   subscriber (Send): on a `ThemePref_has_Mode` change, decode the
//   mode and store it in `PENDING_THEME_MODE` (0=none, 1=dark, 2=light).
//   No component access inside the closure.
//
//   super-loop (single-threaded, owns the component handles): each
//   iteration, `swap` the atomic; if non-zero, map to `ThemeMode` and
//   call `.global::<Theme>().set_mode(m)` on the launcher AND the
//   visible landing component (unified_repl) so the toggle is visible.
//
// # Cell constant
//
// `THEME_PREF_CELL` is the cell name string used in every call to
// `cell_push` / `fetch_or_phi`. Centralised here so callers can
// pattern-match on the `changed` slice without spelling the string
// twice.

#![allow(dead_code)]

use arest::ast::{self, Object};

// ── Public constants ───────────────────────────────────────────────

/// Cell name for the theme preference cell.
/// `subscribe_changes` handlers match against this to detect a mode
/// change; `seed_theme_pref_cell` and `toggle_theme_pref` both write
/// facts into this cell.
pub const THEME_PREF_CELL: &str = "ThemePref_has_Mode";

/// The canonical ThemePref instance id. One instance per kernel boot;
/// the mode is a single-valued preference so "ui" is sufficient.
pub const THEME_PREF_ID: &str = "ui";

// ── Mode enum ─────────────────────────────────────────────────────

/// Kernel-side (no Slint) representation of the theme mode.
/// The Slint `ThemeMode::Dark` / `ThemeMode::Light` variants are
/// derived from this in `ui_apps::launcher` where Slint is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePrefMode {
    Dark,
    Light,
}

impl ThemePrefMode {
    /// The string written into the `Mode` binding of the cell fact.
    pub fn as_str(self) -> &'static str {
        match self {
            ThemePrefMode::Dark => "dark",
            ThemePrefMode::Light => "light",
        }
    }

    /// Parse from the cell-fact string. Returns `None` for unrecognised values.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "dark" => Some(ThemePrefMode::Dark),
            "light" => Some(ThemePrefMode::Light),
            _ => None,
        }
    }

    /// Return the opposite mode (used by `toggle_theme_pref`).
    pub fn toggled(self) -> Self {
        match self {
            ThemePrefMode::Dark => ThemePrefMode::Light,
            ThemePrefMode::Light => ThemePrefMode::Dark,
        }
    }
}

// ── Seeding ────────────────────────────────────────────────────────

/// Push a `ThemePref_has_Mode` fact for the canonical `"ui"` instance
/// into `state` and return the extended state.
///
/// This is the "seed" counterpart to `read_theme_pref_mode` — calling
/// `seed_theme_pref_cell` followed by `read_theme_pref_mode` on the
/// same state yields `initial_mode` exactly.
///
/// `system::apply` is the caller's responsibility. This function only
/// constructs the cell-extended state; it does not install it into the
/// live SYSTEM.
pub fn seed_theme_pref_cell(initial_mode: ThemePrefMode, state: &Object) -> Object {
    use arest::ast::{cell_push, fact_from_pairs};
    cell_push(
        THEME_PREF_CELL,
        fact_from_pairs(&[
            ("ThemePref", THEME_PREF_ID),
            ("Mode", initial_mode.as_str()),
        ]),
        state,
    )
}

// ── Reading ────────────────────────────────────────────────────────

/// Read the `ThemePref_has_Mode` cell from `state` and return the
/// mode for the canonical `"ui"` instance.
///
/// The cell may contain multiple facts (if seeded multiple times or if
/// the state was toggled). The **last** fact whose `ThemePref` binding
/// equals `THEME_PREF_ID` wins — this matches the natural append
/// semantics of `cell_push`: the most-recently pushed fact carries the
/// current preference.
///
/// Returns `ThemePrefMode::Dark` (the design.md default) when the cell
/// is absent or contains no matching fact.
pub fn read_theme_pref_mode(state: &Object) -> ThemePrefMode {
    let cell = ast::fetch_or_phi(THEME_PREF_CELL, state);
    let facts = cell.as_seq().unwrap_or(&[]);

    // Walk in reverse so the last-pushed mode wins.
    facts.iter().rev().find_map(|fact| {
        let pref_id = ast::binding(fact, "ThemePref")?;
        if pref_id != THEME_PREF_ID {
            return None;
        }
        let mode_str = ast::binding(fact, "Mode")?;
        ThemePrefMode::from_str(mode_str)
    })
    .unwrap_or(ThemePrefMode::Dark)
}

// ── Toggle ─────────────────────────────────────────────────────────

/// Push a new `ThemePref_has_Mode` fact that flips the current mode
/// and return the extended state. The current mode is read from
/// `state` via `read_theme_pref_mode` so the toggle is idempotent:
/// calling `toggle_theme_pref` on a state that already has the target
/// mode is a no-op at the visual level (the new fact's mode equals the
/// current one → no change), but the cell does gain an extra fact entry
/// (harmless append).
///
/// `system::apply` is the caller's responsibility.
pub fn toggle_theme_pref(state: &Object) -> Object {
    let current = read_theme_pref_mode(state);
    seed_theme_pref_cell(current.toggled(), state)
}

// ── AtomicU8 staging slot (Send-safe subscriber signal) ───────────

/// Staging slot for the Send-safe subscriber → super-loop handoff.
///
/// Values:
///   0 — no pending change
///   1 — pending change: switch to Dark
///   2 — pending change: switch to Light
///
/// Written by the `subscribe_changes` handler (which must be `Send`
/// and therefore cannot hold a Slint `ComponentHandle`). Read + swapped
/// to zero by the super-loop's per-frame check, which owns the
/// component handles and can safely call `.global::<Theme>().set_mode`.
///
/// `AtomicU8` is `Send + Sync`, satisfying the subscriber closure bound.
pub static PENDING_THEME_MODE: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);

/// Encode a `ThemePrefMode` as the `u8` stored in `PENDING_THEME_MODE`.
pub fn mode_to_pending(mode: ThemePrefMode) -> u8 {
    match mode {
        ThemePrefMode::Dark => 1,
        ThemePrefMode::Light => 2,
    }
}

/// Decode a `u8` from `PENDING_THEME_MODE` back to `ThemePrefMode`.
/// Returns `None` for 0 (no pending change).
pub fn pending_to_mode(v: u8) -> Option<ThemePrefMode> {
    match v {
        1 => Some(ThemePrefMode::Dark),
        2 => Some(ThemePrefMode::Light),
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────
//
// Pure cell-extraction functions — compiled unconditionally (no UEFI
// gate, no Slint gate) so they run under `cargo test --lib
// --target x86_64-pc-windows-msvc` alongside the other hosted tests.
//
// These tests cover the core cell-driven theme-preference round-trip
// for Task U3a: seed ThemePref cells, assert that `read_theme_pref_mode`
// reflects the seeded mode; toggle produces the opposite mode.

#[cfg(test)]
mod tests {
    use super::*;
    use arest::ast::Object;

    // ── Defaults / empty state ─────────────────────────────────────────

    /// Empty state → dark (the design.md default).
    #[test]
    fn empty_state_yields_dark_default() {
        let mode = read_theme_pref_mode(&Object::phi());
        assert_eq!(mode, ThemePrefMode::Dark, "expected Dark default on empty state");
    }

    // ── Seed → read round-trip ────────────────────────────────────────

    /// Seeding Dark then reading back yields Dark.
    #[test]
    fn seed_dark_then_read_yields_dark() {
        let state = seed_theme_pref_cell(ThemePrefMode::Dark, &Object::phi());
        let mode = read_theme_pref_mode(&state);
        assert_eq!(mode, ThemePrefMode::Dark);
    }

    /// Seeding Light then reading back yields Light.
    #[test]
    fn seed_light_then_read_yields_light() {
        let state = seed_theme_pref_cell(ThemePrefMode::Light, &Object::phi());
        let mode = read_theme_pref_mode(&state);
        assert_eq!(mode, ThemePrefMode::Light);
    }

    // ── Toggle round-trip ─────────────────────────────────────────────

    /// toggle_theme_pref from Dark → Light.
    #[test]
    fn toggle_from_dark_yields_light() {
        let state = seed_theme_pref_cell(ThemePrefMode::Dark, &Object::phi());
        let toggled = toggle_theme_pref(&state);
        let mode = read_theme_pref_mode(&toggled);
        assert_eq!(mode, ThemePrefMode::Light, "dark → light after toggle");
    }

    /// toggle_theme_pref from Light → Dark.
    #[test]
    fn toggle_from_light_yields_dark() {
        let state = seed_theme_pref_cell(ThemePrefMode::Light, &Object::phi());
        let toggled = toggle_theme_pref(&state);
        let mode = read_theme_pref_mode(&toggled);
        assert_eq!(mode, ThemePrefMode::Dark, "light → dark after toggle");
    }

    /// Double toggle returns to the original mode.
    #[test]
    fn double_toggle_returns_to_original() {
        let state = seed_theme_pref_cell(ThemePrefMode::Dark, &Object::phi());
        let once = toggle_theme_pref(&state);
        let twice = toggle_theme_pref(&once);
        let mode = read_theme_pref_mode(&twice);
        assert_eq!(mode, ThemePrefMode::Dark, "double-toggle must restore original");
    }

    // ── Last-write-wins semantics ─────────────────────────────────────

    /// When the cell has multiple facts, the last one wins.
    #[test]
    fn last_fact_wins_over_earlier_dark() {
        // Seed Dark first, then Light (simulates a toggle persisted twice).
        let state = seed_theme_pref_cell(ThemePrefMode::Dark, &Object::phi());
        let state = seed_theme_pref_cell(ThemePrefMode::Light, &state);
        let mode = read_theme_pref_mode(&state);
        assert_eq!(mode, ThemePrefMode::Light, "last push (Light) must win");
    }

    /// When the cell has multiple facts, the last one wins (Dark after Light).
    #[test]
    fn last_fact_wins_over_earlier_light() {
        let state = seed_theme_pref_cell(ThemePrefMode::Light, &Object::phi());
        let state = seed_theme_pref_cell(ThemePrefMode::Dark, &state);
        let mode = read_theme_pref_mode(&state);
        assert_eq!(mode, ThemePrefMode::Dark, "last push (Dark) must win");
    }

    // ── ThemePrefMode helpers ─────────────────────────────────────────

    /// ThemePrefMode::Dark → "dark".
    #[test]
    fn dark_as_str_is_dark() {
        assert_eq!(ThemePrefMode::Dark.as_str(), "dark");
    }

    /// ThemePrefMode::Light → "light".
    #[test]
    fn light_as_str_is_light() {
        assert_eq!(ThemePrefMode::Light.as_str(), "light");
    }

    /// from_str("dark") → Dark.
    #[test]
    fn from_str_dark() {
        assert_eq!(ThemePrefMode::from_str("dark"), Some(ThemePrefMode::Dark));
    }

    /// from_str("light") → Light.
    #[test]
    fn from_str_light() {
        assert_eq!(ThemePrefMode::from_str("light"), Some(ThemePrefMode::Light));
    }

    /// from_str with an unknown string → None (no panic).
    #[test]
    fn from_str_unknown_returns_none() {
        assert_eq!(ThemePrefMode::from_str("sepia"), None);
        assert_eq!(ThemePrefMode::from_str(""), None);
    }

    /// toggled() inverts the mode.
    #[test]
    fn toggled_inverts_mode() {
        assert_eq!(ThemePrefMode::Dark.toggled(), ThemePrefMode::Light);
        assert_eq!(ThemePrefMode::Light.toggled(), ThemePrefMode::Dark);
    }

    // ── AtomicU8 staging slot helpers ─────────────────────────────────

    /// mode_to_pending → pending_to_mode round-trip for Dark.
    #[test]
    fn pending_round_trip_dark() {
        let v = mode_to_pending(ThemePrefMode::Dark);
        assert_eq!(pending_to_mode(v), Some(ThemePrefMode::Dark));
    }

    /// mode_to_pending → pending_to_mode round-trip for Light.
    #[test]
    fn pending_round_trip_light() {
        let v = mode_to_pending(ThemePrefMode::Light);
        assert_eq!(pending_to_mode(v), Some(ThemePrefMode::Light));
    }

    /// pending_to_mode(0) → None (no pending change sentinel).
    #[test]
    fn pending_to_mode_zero_is_none() {
        assert_eq!(pending_to_mode(0), None);
    }

    // ── THEME_PREF_CELL constant ──────────────────────────────────────

    /// THEME_PREF_CELL is the expected cell name.
    #[test]
    fn theme_pref_cell_name_is_correct() {
        assert_eq!(THEME_PREF_CELL, "ThemePref_has_Mode");
    }

    // ── Ignore unknown ThemePref instances ───────────────────────────

    /// A fact for a different ThemePref id is ignored; default returned.
    #[test]
    fn unknown_pref_id_ignored_returns_default() {
        use arest::ast::{cell_push, fact_from_pairs};
        let state = cell_push(
            THEME_PREF_CELL,
            fact_from_pairs(&[("ThemePref", "other-instance"), ("Mode", "light")]),
            &Object::phi(),
        );
        // Only "other-instance" is in the cell; "ui" is absent → Dark default.
        let mode = read_theme_pref_mode(&state);
        assert_eq!(mode, ThemePrefMode::Dark, "unknown instance must not affect canonical read");
    }

    /// A malformed fact (missing Mode binding) is skipped; default returned.
    #[test]
    fn malformed_fact_missing_mode_returns_default() {
        use arest::ast::{cell_push, fact_from_pairs};
        let state = cell_push(
            THEME_PREF_CELL,
            fact_from_pairs(&[("ThemePref", THEME_PREF_ID)]), // no Mode binding
            &Object::phi(),
        );
        let mode = read_theme_pref_mode(&state);
        assert_eq!(mode, ThemePrefMode::Dark, "malformed fact must fall back to Dark");
    }

    /// A malformed Mode value (not "dark" or "light") is skipped; default returned.
    #[test]
    fn malformed_mode_value_returns_default() {
        use arest::ast::{cell_push, fact_from_pairs};
        let state = cell_push(
            THEME_PREF_CELL,
            fact_from_pairs(&[("ThemePref", THEME_PREF_ID), ("Mode", "sepia")]),
            &Object::phi(),
        );
        let mode = read_theme_pref_mode(&state);
        assert_eq!(mode, ThemePrefMode::Dark, "unrecognised mode must fall back to Dark");
    }
}
