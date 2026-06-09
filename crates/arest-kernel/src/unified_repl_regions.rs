// crates/arest-kernel/src/unified_repl_regions.rs
//
// Cell-driven UnifiedRepl region/layout extraction (#710 Task U2).
//
// This module extracts the pure, UEFI-free cell-query functions so
// they can be unit-tested under the hosted `cargo test` target
// (x86_64-pc-windows-msvc) — the `ui_apps::unified_repl` module is
// gated on `target_os = "uefi"` and cannot host tests directly.
//
// The UnifiedRepl's Slint surface previously had magic pixel values
// and vertical-stretch weights hand-typed directly in
// `ui/apps/UnifiedRepl.slint` (the FIXME at the scrollback pane
// in the commit that landed #510 explicitly called out that these
// should come from readings/ui/monoview.md's Region weights via
// cells, not be hard-typed magic). This module is the lift:
// a pure host-testable cell-query layer that seed functions push
// `UnifiedReplRegion_has_*` facts into, and that the populate
// function reads back to derive the Slint props before the super-loop.
//
// # Design (mirrors launcher_app_set.rs)
//
// The extraction is a pure function over `&Object` (the live SYSTEM
// state snapshot). No Slint types, no UEFI types, no `system::*`
// calls — the caller supplies the snapshot.
//
// The cell schema mirrors `readings/ui/monoview.md`'s Region fact
// types, scoped to the UnifiedRepl's named regions. Two numeric
// attributes are added that monoview.md deliberately left implicit
// (they are Slint-layout primitives that don't belong in the
// spec-level reading, only in the Slint binding layer):
//
//   UnifiedReplRegion_has_PixelWidth   { UnifiedReplRegion, PixelWidth }
//   UnifiedReplRegion_has_MinHeightPx  { UnifiedReplRegion, MinHeightPx }
//   UnifiedReplRegion_has_VertStretch  { UnifiedReplRegion, VertStretch }
//
// # Canonical region list
//
// `UNIFIED_REPL_REGIONS` is the kernel's ordered list of named
// UnifiedRepl layout regions. Adding a new region requires adding
// its name here AND a `RegionLayout` entry in `default_region_layout`.
//
// # Defaults
//
// `default_region_layout` supplies the same numeric values the prior
// hard-coded Slint had. The extraction falls back to these when the
// cells are absent (empty SYSTEM state, early boot before seeding)
// so the visual render is unchanged from the prior baseline.
//
// The seeding function `seed_region_cells` pushes these defaults as
// actual cell facts so the HATEOAS browser can introspect them and
// the readings checker can validate them.

#![allow(dead_code)]

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use arest::ast::{self, Object};

// ── Canonical region name list ─────────────────────────────────────

/// Canonical ordered list of UnifiedRepl layout region names.
/// Order determines the index mapping used by the populate path.
pub const UNIFIED_REPL_REGIONS: &[&str] = &[
    "left-pane",     // The HATEOAS-browse column block (fixed width).
    "resources",     // Resources (Nouns) sidebar sub-column.
    "detail",        // Detail sub-column (rightmost in the left block).
    "typed-surface", // Typed cell-as-screen card (top of the right pane).
    "scrollback",    // REPL scrollback card (grows to fill remaining space).
];

// ── Per-region layout descriptor ──────────────────────────────────

/// The subset of Slint layout properties the UnifiedRepl uses to
/// configure each named region.  Fields that are not applicable for
/// a given region keep their `None` / `1` defaults; the Slint side
/// uses them unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionLayout {
    /// Region identifier (one of `UNIFIED_REPL_REGIONS`).
    pub name: &'static str,
    /// Fixed pixel width (`width: Xpx` in Slint). `None` means the
    /// region uses `horizontal-stretch` to fill remaining space.
    pub pixel_width: Option<u32>,
    /// Minimum pixel height (`min-height: Xpx` in Slint). `None` means
    /// the region has no explicit minimum (Slint layout default).
    pub min_height_px: Option<u32>,
    /// Slint `vertical-stretch` multiplier (default 1, higher = more
    /// vertical space claimed in a VerticalLayout). `None` maps to 1.
    pub vert_stretch: u32,
}

// ── Hardcoded defaults (prior baseline) ───────────────────────────

/// Returns the default `RegionLayout` for each canonical region,
/// matching the magic values that were previously hand-typed in
/// `ui/apps/UnifiedRepl.slint`.
///
/// Used as the fallback when cells are absent and as the seed source
/// for `seed_region_cells`.
pub fn default_region_layout() -> Vec<RegionLayout> {
    vec![
        RegionLayout {
            name: "left-pane",
            pixel_width: Some(520),
            min_height_px: None,
            vert_stretch: 1,
        },
        RegionLayout {
            name: "resources",
            pixel_width: Some(160),
            min_height_px: None,
            vert_stretch: 1,
        },
        RegionLayout {
            name: "detail",
            pixel_width: Some(200),
            min_height_px: None,
            vert_stretch: 1,
        },
        RegionLayout {
            name: "typed-surface",
            pixel_width: None,
            min_height_px: Some(200),
            vert_stretch: 1,
        },
        RegionLayout {
            name: "scrollback",
            pixel_width: None,
            min_height_px: Some(200),
            vert_stretch: 2,
        },
    ]
}

// ── Cell seeding ───────────────────────────────────────────────────

/// Push `UnifiedReplRegion_has_*` facts for every canonical region
/// into `state` and return the extended state.
///
/// This is the "seed" counterpart to `region_layouts_from_cells` —
/// calling `seed_region_cells` followed by `region_layouts_from_cells`
/// on the same state yields `default_region_layout()` exactly.
///
/// `system::apply` is the caller's responsibility. This function only
/// constructs the cell-extended state; it does not install it into the
/// live SYSTEM.
pub fn seed_region_cells(state: &Object) -> Object {
    use arest::ast::{cell_push, fact_from_pairs};

    let defaults = default_region_layout();
    let mut acc = state.clone();

    for layout in &defaults {
        // PixelWidth — only emit when `Some`.
        if let Some(w) = layout.pixel_width {
            acc = cell_push(
                "UnifiedReplRegion_has_PixelWidth",
                fact_from_pairs(&[
                    ("UnifiedReplRegion", layout.name),
                    ("PixelWidth", &w.to_string()),
                ]),
                &acc,
            );
        }
        // MinHeightPx — only emit when `Some`.
        if let Some(h) = layout.min_height_px {
            acc = cell_push(
                "UnifiedReplRegion_has_MinHeightPx",
                fact_from_pairs(&[
                    ("UnifiedReplRegion", layout.name),
                    ("MinHeightPx", &h.to_string()),
                ]),
                &acc,
            );
        }
        // VertStretch — emit unconditionally (always meaningful).
        acc = cell_push(
            "UnifiedReplRegion_has_VertStretch",
            fact_from_pairs(&[
                ("UnifiedReplRegion", layout.name),
                ("VertStretch", &layout.vert_stretch.to_string()),
            ]),
            &acc,
        );
    }

    acc
}

// ── Cell extraction ────────────────────────────────────────────────

/// Read the `UnifiedReplRegion_has_*` cells from `state` and return
/// one `RegionLayout` per canonical region, in `UNIFIED_REPL_REGIONS`
/// order.
///
/// Regions absent from the cells fall back to `default_region_layout`
/// values so the visual output is unchanged when cells are absent
/// (empty SYSTEM state, early boot before seeding runs).
///
/// This is the primary cell-driven extraction: the Slint side reads
/// the properties produced by `populate_region_props` (which calls
/// this function) rather than literal pixel values.
pub fn region_layouts_from_cells(state: &Object) -> Vec<RegionLayout> {
    // Collect PixelWidth bindings: region_name → u32.
    let pw_cell = ast::fetch_cell_seq("UnifiedReplRegion_has_PixelWidth", state);
    let mut pixel_widths: alloc::collections::BTreeMap<String, u32> =
        alloc::collections::BTreeMap::new();
    if let Some(facts) = pw_cell.as_seq() {
        for fact in facts {
            let region = ast::binding(fact, "UnifiedReplRegion").map(|s| s.to_string());
            let width = ast::binding(fact, "PixelWidth")
                .and_then(|v| v.parse::<u32>().ok());
            if let (Some(r), Some(w)) = (region, width) {
                pixel_widths.insert(r, w);
            }
        }
    }

    // Collect MinHeightPx bindings: region_name → u32.
    let mh_cell = ast::fetch_cell_seq("UnifiedReplRegion_has_MinHeightPx", state);
    let mut min_heights: alloc::collections::BTreeMap<String, u32> =
        alloc::collections::BTreeMap::new();
    if let Some(facts) = mh_cell.as_seq() {
        for fact in facts {
            let region = ast::binding(fact, "UnifiedReplRegion").map(|s| s.to_string());
            let height = ast::binding(fact, "MinHeightPx")
                .and_then(|v| v.parse::<u32>().ok());
            if let (Some(r), Some(h)) = (region, height) {
                min_heights.insert(r, h);
            }
        }
    }

    // Collect VertStretch bindings: region_name → u32.
    let vs_cell = ast::fetch_cell_seq("UnifiedReplRegion_has_VertStretch", state);
    let mut vert_stretches: alloc::collections::BTreeMap<String, u32> =
        alloc::collections::BTreeMap::new();
    if let Some(facts) = vs_cell.as_seq() {
        for fact in facts {
            let region = ast::binding(fact, "UnifiedReplRegion").map(|s| s.to_string());
            let stretch = ast::binding(fact, "VertStretch")
                .and_then(|v| v.parse::<u32>().ok());
            if let (Some(r), Some(s)) = (region, stretch) {
                vert_stretches.insert(r, s);
            }
        }
    }

    // Build the per-region layout vector in UNIFIED_REPL_REGIONS order,
    // falling back to defaults for any attribute absent from the cells.
    let defaults = default_region_layout();
    UNIFIED_REPL_REGIONS
        .iter()
        .map(|name| {
            // Find the default for this name.
            let default = defaults
                .iter()
                .find(|d| d.name == *name)
                .cloned()
                .unwrap_or(RegionLayout {
                    name,
                    pixel_width: None,
                    min_height_px: None,
                    vert_stretch: 1,
                });

            RegionLayout {
                name,
                pixel_width: pixel_widths
                    .get(*name)
                    .copied()
                    .or(default.pixel_width),
                min_height_px: min_heights
                    .get(*name)
                    .copied()
                    .or(default.min_height_px),
                vert_stretch: vert_stretches
                    .get(*name)
                    .copied()
                    .unwrap_or(default.vert_stretch),
            }
        })
        .collect()
}

/// Convenience: return the `RegionLayout` for a single named region.
/// Used by `populate_region_props` in `ui_apps::unified_repl` to
/// extract individual regions without indexing the full vector.
pub fn region_layout_for(name: &str, state: &Object) -> RegionLayout {
    region_layouts_from_cells(state)
        .into_iter()
        .find(|r| r.name == name)
        .unwrap_or(RegionLayout {
            // Static-lifetime name pointer trick: since `name` isn't in
            // UNIFIED_REPL_REGIONS we hand back the default with an empty
            // literal. The only callers supply names from UNIFIED_REPL_REGIONS.
            name: "unknown",
            pixel_width: None,
            min_height_px: None,
            vert_stretch: 1,
        })
}

// ── Tests ─────────────────────────────────────────────────────────
//
// Pure cell-extraction functions — compiled unconditionally (no UEFI
// gate, no Slint gate) so they run under `cargo test --lib
// --target x86_64-pc-windows-msvc` alongside the other hosted tests.
//
// These tests cover the core cell-driven region-layout extraction for
// #710 Task U2: seed cells, assert that `region_layouts_from_cells`
// derives the expected `RegionLayout` structs.

#[cfg(test)]
mod tests {
    use super::*;
    use arest::ast::Object;

    // ── Default / empty state ─────────────────────────────────────────

    /// Empty state → defaults for every canonical region.
    /// The visual render must be identical to the prior hardcoded Slint.
    #[test]
    fn empty_state_yields_default_layouts() {
        let layouts = region_layouts_from_cells(&Object::phi());
        let defaults = default_region_layout();
        assert_eq!(layouts.len(), defaults.len(), "count mismatch");
        for (got, want) in layouts.iter().zip(defaults.iter()) {
            assert_eq!(got, want, "mismatch for region '{}'", got.name);
        }
    }

    /// The region count matches UNIFIED_REPL_REGIONS.
    #[test]
    fn region_count_matches_canonical_list() {
        let layouts = region_layouts_from_cells(&Object::phi());
        assert_eq!(layouts.len(), UNIFIED_REPL_REGIONS.len());
    }

    /// The region names appear in UNIFIED_REPL_REGIONS order.
    #[test]
    fn region_names_are_in_canonical_order() {
        let layouts = region_layouts_from_cells(&Object::phi());
        for (layout, canonical) in layouts.iter().zip(UNIFIED_REPL_REGIONS.iter()) {
            assert_eq!(
                layout.name, *canonical,
                "expected '{}', got '{}'",
                canonical, layout.name
            );
        }
    }

    // ── Seed → extract round-trip ────────────────────────────────────

    /// seed_region_cells → region_layouts_from_cells reproduces
    /// default_region_layout() exactly.
    #[test]
    fn seed_then_extract_reproduces_defaults() {
        let state = seed_region_cells(&Object::phi());
        let layouts = region_layouts_from_cells(&state);
        let defaults = default_region_layout();
        assert_eq!(layouts.len(), defaults.len());
        for (got, want) in layouts.iter().zip(defaults.iter()) {
            assert_eq!(got, want, "seed/extract mismatch for '{}'", got.name);
        }
    }

    // ── Per-attribute cell reading ───────────────────────────────────

    /// Seeding a custom PixelWidth for `left-pane` overrides only that field.
    #[test]
    fn custom_pixel_width_overrides_default() {
        use arest::ast::{cell_push, fact_from_pairs};
        let state = cell_push(
            "UnifiedReplRegion_has_PixelWidth",
            fact_from_pairs(&[
                ("UnifiedReplRegion", "left-pane"),
                ("PixelWidth", "640"),
            ]),
            &Object::phi(),
        );
        let layouts = region_layouts_from_cells(&state);
        let left_pane = layouts.iter().find(|r| r.name == "left-pane").unwrap();
        assert_eq!(
            left_pane.pixel_width,
            Some(640),
            "expected overridden 640px, got {:?}",
            left_pane.pixel_width
        );
        // Other fields still fall back to default.
        assert_eq!(left_pane.vert_stretch, 1);
    }

    /// Seeding a custom MinHeightPx for `scrollback` overrides only that field.
    #[test]
    fn custom_min_height_overrides_default_for_scrollback() {
        use arest::ast::{cell_push, fact_from_pairs};
        let state = cell_push(
            "UnifiedReplRegion_has_MinHeightPx",
            fact_from_pairs(&[
                ("UnifiedReplRegion", "scrollback"),
                ("MinHeightPx", "300"),
            ]),
            &Object::phi(),
        );
        let layouts = region_layouts_from_cells(&state);
        let scrollback = layouts.iter().find(|r| r.name == "scrollback").unwrap();
        assert_eq!(
            scrollback.min_height_px,
            Some(300),
            "expected overridden 300px, got {:?}",
            scrollback.min_height_px
        );
        // VertStretch falls back to default (2 for scrollback).
        assert_eq!(scrollback.vert_stretch, 2);
    }

    /// Seeding a custom VertStretch for `scrollback` overrides only that field.
    #[test]
    fn custom_vert_stretch_overrides_default_for_scrollback() {
        use arest::ast::{cell_push, fact_from_pairs};
        let state = cell_push(
            "UnifiedReplRegion_has_VertStretch",
            fact_from_pairs(&[
                ("UnifiedReplRegion", "scrollback"),
                ("VertStretch", "3"),
            ]),
            &Object::phi(),
        );
        let layouts = region_layouts_from_cells(&state);
        let scrollback = layouts.iter().find(|r| r.name == "scrollback").unwrap();
        assert_eq!(
            scrollback.vert_stretch,
            3,
            "expected overridden stretch=3, got {}",
            scrollback.vert_stretch
        );
        // MinHeightPx falls back to default (200 for scrollback).
        assert_eq!(scrollback.min_height_px, Some(200));
    }

    /// Regions not in UNIFIED_REPL_REGIONS that appear in cells are ignored.
    #[test]
    fn unknown_region_in_cells_is_ignored() {
        use arest::ast::{cell_push, fact_from_pairs};
        let state = cell_push(
            "UnifiedReplRegion_has_PixelWidth",
            fact_from_pairs(&[
                ("UnifiedReplRegion", "future-region"),
                ("PixelWidth", "999"),
            ]),
            &Object::phi(),
        );
        let layouts = region_layouts_from_cells(&state);
        // No "future-region" entry — UNIFIED_REPL_REGIONS order is authoritative.
        assert!(
            !layouts.iter().any(|r| r.name == "future-region"),
            "unknown region must not appear: {layouts:?}"
        );
        assert_eq!(layouts.len(), UNIFIED_REPL_REGIONS.len());
    }

    /// Malformed PixelWidth value (non-numeric) falls back to the default.
    #[test]
    fn malformed_pixel_width_falls_back_to_default() {
        use arest::ast::{cell_push, fact_from_pairs};
        let state = cell_push(
            "UnifiedReplRegion_has_PixelWidth",
            fact_from_pairs(&[
                ("UnifiedReplRegion", "left-pane"),
                ("PixelWidth", "not-a-number"),
            ]),
            &Object::phi(),
        );
        let layouts = region_layouts_from_cells(&state);
        let left_pane = layouts.iter().find(|r| r.name == "left-pane").unwrap();
        // Falls back to the 520px default.
        assert_eq!(
            left_pane.pixel_width,
            Some(520),
            "malformed value must fall back to default 520: {:?}",
            left_pane.pixel_width
        );
    }

    /// Fact missing the UnifiedReplRegion binding is skipped gracefully.
    #[test]
    fn fact_missing_region_binding_skipped_gracefully() {
        use arest::ast::{cell_push, fact_from_pairs};
        // Only `PixelWidth` binding — no `UnifiedReplRegion`.
        let state = cell_push(
            "UnifiedReplRegion_has_PixelWidth",
            fact_from_pairs(&[("PixelWidth", "999")]),
            &Object::phi(),
        );
        let layouts = region_layouts_from_cells(&state);
        // All regions still fall back to defaults — no panic.
        let defaults = default_region_layout();
        for (got, want) in layouts.iter().zip(defaults.iter()) {
            assert_eq!(got, want, "orphan fact must not pollute '{}'", got.name);
        }
    }

    // ── region_layout_for convenience helper ─────────────────────────

    /// region_layout_for with an empty state returns the default for that name.
    #[test]
    fn region_layout_for_returns_default_on_empty_state() {
        let layout = region_layout_for("scrollback", &Object::phi());
        assert_eq!(layout.name, "scrollback");
        assert_eq!(layout.min_height_px, Some(200));
        assert_eq!(layout.vert_stretch, 2);
    }

    /// region_layout_for after seeding returns the seeded value.
    #[test]
    fn region_layout_for_returns_seeded_value() {
        let state = seed_region_cells(&Object::phi());
        let layout = region_layout_for("resources", &state);
        assert_eq!(layout.name, "resources");
        assert_eq!(layout.pixel_width, Some(160));
        assert_eq!(layout.vert_stretch, 1);
    }

    // ── Baseline-pixel-value assertions ──────────────────────────────
    //
    // These pin the prior hardcoded values so a future change to
    // `default_region_layout()` that silently shifts a pixel value
    // is caught immediately at the test level.

    /// left-pane pixel width baseline is 520px.
    #[test]
    fn left_pane_pixel_width_baseline_is_520() {
        let layout = region_layout_for("left-pane", &Object::phi());
        assert_eq!(layout.pixel_width, Some(520), "left-pane baseline is 520px");
    }

    /// resources column pixel width baseline is 160px.
    #[test]
    fn resources_pixel_width_baseline_is_160() {
        let layout = region_layout_for("resources", &Object::phi());
        assert_eq!(layout.pixel_width, Some(160), "resources baseline is 160px");
    }

    /// detail column pixel width baseline is 200px.
    #[test]
    fn detail_pixel_width_baseline_is_200() {
        let layout = region_layout_for("detail", &Object::phi());
        assert_eq!(layout.pixel_width, Some(200), "detail baseline is 200px");
    }

    /// typed-surface min-height baseline is 200px.
    #[test]
    fn typed_surface_min_height_baseline_is_200() {
        let layout = region_layout_for("typed-surface", &Object::phi());
        assert_eq!(layout.min_height_px, Some(200), "typed-surface min-height baseline is 200px");
    }

    /// scrollback min-height baseline is 200px.
    #[test]
    fn scrollback_min_height_baseline_is_200() {
        let layout = region_layout_for("scrollback", &Object::phi());
        assert_eq!(layout.min_height_px, Some(200), "scrollback min-height baseline is 200px");
    }

    /// scrollback vertical-stretch baseline is 2.
    #[test]
    fn scrollback_vert_stretch_baseline_is_2() {
        let layout = region_layout_for("scrollback", &Object::phi());
        assert_eq!(layout.vert_stretch, 2, "scrollback vert-stretch baseline is 2");
    }

    // ── Readings-derived layout extraction ───────────────────────────
    //
    // These tests verify that the layout weights in `readings/ui/monoview.md`
    // (the FORML predicate-text source of truth) parse into cells whose
    // values are read back correctly by `region_layouts_from_cells`.
    //
    // The path: monoview.md instance facts → parse_to_state_from →
    // `UnifiedReplRegion_has_PixelWidth` / `_has_MinHeightPx` /
    // `_has_VertStretch` cells → `region_layouts_from_cells` → RegionLayout.
    //
    // This is the "readings-derived" leg of #710 Task 599: the numeric
    // layout values live in monoview.md as FORML facts, not in Rust
    // constants. A change to any pixel value in monoview.md
    // immediately surfaces here as a test failure, and the Rust
    // `default_region_layout()` can be updated to match.

    /// Monoview.md parses without error (smoke test for the new
    /// UnifiedReplRegion entity type + fact types + instance facts).
    #[test]
    fn monoview_md_parses_cleanly() {
        let monoview_md = include_str!("../../../readings/ui/monoview.md");
        let result = arest::parse_forml2::parse_to_state(monoview_md);
        assert!(
            result.is_ok(),
            "readings/ui/monoview.md must parse without error; got: {:?}",
            result.err()
        );
    }

    /// Parsing monoview.md produces a state with non-empty
    /// `UnifiedReplRegion_has_PixelWidth` cells — the readings-derived
    /// fact type is registered and instance facts landed in the cell.
    #[test]
    fn monoview_md_produces_unified_repl_region_pixel_width_cell() {
        let monoview_md = include_str!("../../../readings/ui/monoview.md");
        let state = arest::parse_forml2::parse_to_state(monoview_md)
            .expect("monoview.md must parse");
        let cell = arest::ast::fetch_cell_seq("UnifiedReplRegion_has_PixelWidth", &state);
        assert!(
            cell.as_seq().map(|s| !s.is_empty()).unwrap_or(false),
            "UnifiedReplRegion_has_PixelWidth cell must be non-empty after parsing monoview.md"
        );
    }

    /// The readings-derived PixelWidth values match `default_region_layout()`.
    /// This is the end-to-end readings→cells→RegionLayout round-trip.
    #[test]
    fn readings_derived_pixel_widths_match_defaults() {
        let monoview_md = include_str!("../../../readings/ui/monoview.md");
        let state = arest::parse_forml2::parse_to_state(monoview_md)
            .expect("monoview.md must parse");
        let layouts = region_layouts_from_cells(&state);

        // left-pane: 520px from readings.
        let left = layouts.iter().find(|r| r.name == "left-pane").unwrap();
        assert_eq!(
            left.pixel_width, Some(520),
            "readings-derived left-pane pixel_width must be 520; got {:?}",
            left.pixel_width
        );

        // resources: 160px from readings.
        let resources = layouts.iter().find(|r| r.name == "resources").unwrap();
        assert_eq!(
            resources.pixel_width, Some(160),
            "readings-derived resources pixel_width must be 160; got {:?}",
            resources.pixel_width
        );

        // detail: 200px from readings.
        let detail = layouts.iter().find(|r| r.name == "detail").unwrap();
        assert_eq!(
            detail.pixel_width, Some(200),
            "readings-derived detail pixel_width must be 200; got {:?}",
            detail.pixel_width
        );

        // typed-surface: no fixed width (fills remaining space).
        let typed = layouts.iter().find(|r| r.name == "typed-surface").unwrap();
        assert_eq!(
            typed.pixel_width, None,
            "typed-surface must have no fixed pixel_width; got {:?}",
            typed.pixel_width
        );

        // scrollback: no fixed width (fills remaining space).
        let scrollback = layouts.iter().find(|r| r.name == "scrollback").unwrap();
        assert_eq!(
            scrollback.pixel_width, None,
            "scrollback must have no fixed pixel_width; got {:?}",
            scrollback.pixel_width
        );
    }

    /// The readings-derived MinHeightPx values match `default_region_layout()`.
    #[test]
    fn readings_derived_min_heights_match_defaults() {
        let monoview_md = include_str!("../../../readings/ui/monoview.md");
        let state = arest::parse_forml2::parse_to_state(monoview_md)
            .expect("monoview.md must parse");
        let layouts = region_layouts_from_cells(&state);

        // typed-surface: 200px min-height from readings.
        let typed = layouts.iter().find(|r| r.name == "typed-surface").unwrap();
        assert_eq!(
            typed.min_height_px, Some(200),
            "readings-derived typed-surface min_height_px must be 200; got {:?}",
            typed.min_height_px
        );

        // scrollback: 200px min-height from readings.
        let scrollback = layouts.iter().find(|r| r.name == "scrollback").unwrap();
        assert_eq!(
            scrollback.min_height_px, Some(200),
            "readings-derived scrollback min_height_px must be 200; got {:?}",
            scrollback.min_height_px
        );
    }

    /// The readings-derived VertStretch values match `default_region_layout()`.
    #[test]
    fn readings_derived_vert_stretches_match_defaults() {
        let monoview_md = include_str!("../../../readings/ui/monoview.md");
        let state = arest::parse_forml2::parse_to_state(monoview_md)
            .expect("monoview.md must parse");
        let layouts = region_layouts_from_cells(&state);

        // All regions except scrollback: vert_stretch = 1 from readings.
        for name in &["left-pane", "resources", "detail", "typed-surface"] {
            let region = layouts.iter().find(|r| r.name == *name).unwrap();
            assert_eq!(
                region.vert_stretch, 1,
                "readings-derived {} vert_stretch must be 1; got {}",
                name, region.vert_stretch
            );
        }

        // scrollback: vert_stretch = 2 from readings (claims more vertical space).
        let scrollback = layouts.iter().find(|r| r.name == "scrollback").unwrap();
        assert_eq!(
            scrollback.vert_stretch, 2,
            "readings-derived scrollback vert_stretch must be 2; got {}",
            scrollback.vert_stretch
        );
    }

    /// Full readings→cells→defaults round-trip: the layouts derived from
    /// monoview.md cells match `default_region_layout()` exactly.
    /// This is the integration test ensuring readings == Rust defaults.
    #[test]
    fn readings_derived_layouts_match_default_region_layout() {
        let monoview_md = include_str!("../../../readings/ui/monoview.md");
        let state = arest::parse_forml2::parse_to_state(monoview_md)
            .expect("monoview.md must parse");
        let from_readings = region_layouts_from_cells(&state);
        let defaults = default_region_layout();
        assert_eq!(
            from_readings.len(), defaults.len(),
            "readings-derived region count must match default_region_layout()"
        );
        for (got, want) in from_readings.iter().zip(defaults.iter()) {
            assert_eq!(
                got, want,
                "readings-derived layout for '{}' must match default_region_layout();\n  got: {:?}\n  want: {:?}",
                got.name, got, want
            );
        }
    }
}
