// crates/arest-kernel/src/launcher_app_set.rs
//
// Cell-driven AppLauncher app-set extraction (#709 Task U1).
//
// This module extracts the pure, UEFI-free cell-query functions so
// they can be unit-tested under the hosted `cargo test` target
// (x86_64-pc-windows-msvc) — the `ui_apps::launcher` module is
// gated on `target_os = "uefi"` and cannot host tests directly.
//
// The launcher's navigation switch in `ui_apps::launcher::run()`
// imports these functions via `crate::launcher_app_set::*` and uses
// them to populate the Slint `app-names` property and to resolve
// `app-selected(idx)` → `Active` variant at runtime.
//
// # Design
//
// The AppLauncher's button set is derived from the
// `LaunchableApp_has_Symbol` cells seeded by
// `ui_apps::registry::build_slint_component_state`. Each fact in
// that cell binds `{ LaunchableApp: <slug>, Symbol: <SlintType> }`.
// The slug is the kernel-internal identifier.
//
// The extraction is a pure function over `&Object` (the live SYSTEM
// state snapshot). No Slint types, no UEFI types, no `system::*`
// calls — the caller supplies the snapshot.
//
// # Canonical slug list
//
// `LAUNCHER_APP_SLUGS` is the kernel's ordered list of navigable app
// slugs. Its order determines the index mapping for
// `app-selected(idx)`: position 0 is unified-repl, position 1 is
// doom (cfg-gated). (#598: keyboard left the list — it docks inside
// UnifiedRepl now rather than launching as an app.) Adding a new app
// requires adding its slug here AND a navigation arm in the
// `ui_apps::launcher` switch.
//
// `app-launcher` itself is NOT in the list — you cannot launch the
// launcher from within itself. The `app-launcher` slug may appear in
// the cell (it is registered there), but `launcher_app_slugs_from_cells`
// filters it out.

#![allow(dead_code)]

use alloc::collections::BTreeSet;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use arest::ast::{self, Object};

/// Canonical ordered list of launchable app slugs the kernel navigation
/// switch understands, excluding `app-launcher` itself.
///
/// Order here MUST be stable: `on_app_selected(idx)` in
/// `ui_apps::launcher` resolves `idx` → `Active` by position in the
/// list returned by `launcher_app_slugs_from_cells`.
pub const LAUNCHER_APP_SLUGS: &[&str] = &[
    "unified-repl", // idx 0 → Active::UnifiedRepl
    "doom",         // idx 1 → Active::Doom (cfg-gated in navigation switch)
];

/// Read the `LaunchableApp_has_Symbol` cell from `state` and return the
/// ordered list of app slugs registered there, filtered to only those
/// present in `LAUNCHER_APP_SLUGS` (in `LAUNCHER_APP_SLUGS` order).
///
/// This is the primary cell-driven extraction: the rendered button set
/// equals the intersection of what the cells contain and what the
/// kernel navigation switch can handle. Slugs in `LAUNCHER_APP_SLUGS`
/// that are absent from the cells are silently excluded. Cell slugs not
/// in `LAUNCHER_APP_SLUGS` are ignored.
///
/// On a non-doom build (`cfg(not(feature = "doom"))`), the `doom` slug
/// is always excluded even if it is present in the cells.
pub fn launcher_app_slugs_from_cells(state: &Object) -> Vec<String> {
    // Collect the set of slugs present in the cell.
    let cell = ast::fetch_or_phi("LaunchableApp_has_Symbol", state);
    let registered: BTreeSet<String> = cell
        .as_seq()
        .unwrap_or(&[])
        .iter()
        .filter_map(|fact| {
            ast::binding(fact, "LaunchableApp").map(|s| s.to_string())
        })
        .collect();

    // Return slugs in LAUNCHER_APP_SLUGS order, filtered to those
    // registered in the cells. cfg-filter doom on non-doom builds.
    LAUNCHER_APP_SLUGS
        .iter()
        .filter(|slug| {
            // Doom is only navigable when the doom feature is on.
            #[cfg(not(feature = "doom"))]
            if **slug == "doom" {
                return false;
            }
            registered.contains(**slug)
        })
        .map(|s| s.to_string())
        .collect()
}

/// Derive a human-readable display label from a LaunchableApp slug.
/// Splits on `-`, title-cases each word, joins with a space.
///
/// Examples:
///   "unified-repl"  → "Unified Repl"
///   "doom"          → "Doom"
pub fn slug_to_display_name(slug: &str) -> String {
    slug.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    let upper: String = first.to_uppercase().collect();
                    upper + chars.as_str()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Return the display labels for the launcher buttons, in the same order
/// as `launcher_app_slugs_from_cells`. This is the value pushed directly
/// to the Slint `app-names` property.
pub fn launcher_app_display_names(state: &Object) -> Vec<String> {
    launcher_app_slugs_from_cells(state)
        .iter()
        .map(|slug| slug_to_display_name(slug))
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────
//
// Pure cell-extraction functions — compiled unconditionally (no UEFI
// gate, no Slint gate) so they run under `cargo test --lib
// --target x86_64-pc-windows-msvc` alongside the other hosted tests.
//
// These tests cover the core cell-driven app-set extraction for #709
// Task U1: seed `LaunchableApp_has_Symbol` cells, assert that
// `launcher_app_slugs_from_cells` / `launcher_app_display_names`
// reflect only the registered slugs in the expected order.

#[cfg(test)]
mod tests {
    use super::*;
    use arest::ast::{cell_push, fact_from_pairs};

    /// Seed one `LaunchableApp_has_Symbol` fact for `slug`.
    fn seed_app(slug: &str, symbol: &str, state: &Object) -> Object {
        cell_push(
            "LaunchableApp_has_Symbol",
            fact_from_pairs(&[("LaunchableApp", slug), ("Symbol", symbol)]),
            state,
        )
    }

    /// Seed only the non-doom apps (app-launcher, unified-repl)
    /// so tests that run without `--features doom` exercise the filter.
    fn seed_non_doom_apps() -> Object {
        let s = Object::phi();
        let s = seed_app("app-launcher", "AppLauncher", &s);
        seed_app("unified-repl", "UnifiedRepl", &s)
    }

    // ── Slug extraction ───────────────────────────────────────────────

    /// Empty state → empty slug list (no apps seeded in cells).
    #[test]
    fn empty_state_yields_empty_slug_list() {
        let slugs = launcher_app_slugs_from_cells(&Object::phi());
        assert!(slugs.is_empty(), "expected empty: {slugs:?}");
    }

    /// Only the `app-launcher` slug seeded → it is excluded from the
    /// display list (you can't launch the launcher from within itself).
    #[test]
    fn app_launcher_slug_excluded_from_set() {
        let state = seed_app("app-launcher", "AppLauncher", &Object::phi());
        let slugs = launcher_app_slugs_from_cells(&state);
        assert!(
            !slugs.iter().any(|s| s == "app-launcher"),
            "app-launcher must be excluded: {slugs:?}"
        );
    }

    /// unified-repl seeded → it appears in LAUNCHER_APP_SLUGS order.
    #[test]
    fn non_doom_apps_appear_in_canonical_order() {
        let state = seed_non_doom_apps();
        let slugs = launcher_app_slugs_from_cells(&state);
        // unified-repl must appear.
        assert!(
            slugs.iter().any(|s| s == "unified-repl"),
            "unified-repl missing: {slugs:?}"
        );
    }

    /// A slug not in LAUNCHER_APP_SLUGS (e.g. a future `"settings"` app)
    /// that is seeded in cells is ignored — the kernel navigation switch
    /// can only handle known slugs.
    #[test]
    fn unknown_slug_in_cells_is_ignored() {
        let s = seed_non_doom_apps();
        let state = seed_app("settings", "Settings", &s);
        let slugs = launcher_app_slugs_from_cells(&state);
        assert!(
            !slugs.iter().any(|s| s == "settings"),
            "unknown slug must be ignored: {slugs:?}"
        );
    }

    /// Seeding the same slug twice (duplicate cell push) yields it only
    /// once — the `BTreeSet` deduplication makes the extraction idempotent.
    #[test]
    fn duplicate_cell_entries_deduplicated() {
        let mut state = Object::phi();
        state = seed_app("unified-repl", "UnifiedRepl", &state);
        state = seed_app("unified-repl", "UnifiedRepl", &state); // duplicate
        let slugs = launcher_app_slugs_from_cells(&state);
        let count = slugs.iter().filter(|s| *s == "unified-repl").count();
        assert_eq!(count, 1, "expected exactly one unified-repl entry: {slugs:?}");
    }

    /// A cell that has `LaunchableApp_has_Symbol` but with `Symbol` binding
    /// only (no `LaunchableApp` binding) is gracefully skipped.
    #[test]
    fn malformed_fact_without_launchable_app_binding_skipped() {
        // Seed a fact with only a Symbol role — missing the LaunchableApp.
        use arest::ast::fact_from_pairs;
        let state = cell_push(
            "LaunchableApp_has_Symbol",
            fact_from_pairs(&[("Symbol", "OrphanSymbol")]),
            &Object::phi(),
        );
        let slugs = launcher_app_slugs_from_cells(&state);
        assert!(
            slugs.is_empty(),
            "malformed fact must not produce a slug: {slugs:?}"
        );
    }

    // ── Display-name derivation ───────────────────────────────────────

    /// slug_to_display_name title-cases each hyphen-separated word.
    #[test]
    fn slug_to_display_name_titlecases_words() {
        assert_eq!(slug_to_display_name("unified-repl"), "Unified Repl");
        assert_eq!(slug_to_display_name("doom"), "Doom");
        assert_eq!(slug_to_display_name("app-launcher"), "App Launcher");
    }

    /// slug_to_display_name handles an empty string without panicking.
    #[test]
    fn slug_to_display_name_empty_string() {
        assert_eq!(slug_to_display_name(""), "");
    }

    /// Display names mirror the slug list order and use slug_to_display_name.
    #[test]
    fn display_names_match_slug_order_and_title_case() {
        let state = seed_non_doom_apps();
        let slugs = launcher_app_slugs_from_cells(&state);
        let names = launcher_app_display_names(&state);
        assert_eq!(slugs.len(), names.len(), "slugs and names must be same length");
        for (slug, name) in slugs.iter().zip(names.iter()) {
            let expected = slug_to_display_name(slug);
            assert_eq!(name, &expected, "slug {slug} → expected {expected}, got {name}");
        }
    }

    /// Seeding unified-repl produces the display name "Unified Repl".
    #[test]
    fn display_names_for_non_doom_set_match_expected() {
        let state = seed_non_doom_apps();
        let names = launcher_app_display_names(&state);
        assert!(
            names.iter().any(|n| n == "Unified Repl"),
            "expected 'Unified Repl': {names:?}"
        );
    }
}
