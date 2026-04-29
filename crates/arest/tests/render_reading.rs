// crates/arest/tests/render_reading.rs
//
// Smoke tests for `readings/ui/render.md` (#593).
//
// 1. The reading parses standalone via `parse_to_state_from`.
// 2. The reading composes (parses + folds) with the existing UI
//    readings — `ui.md`, `monoview.md`, `components.md` — without
//    noun-name collisions or unresolved references, and the merged
//    state passes the readings checker (`check_readings_func`) with
//    no Layer-2 or Layer-3 violations introduced by `render.md`.
//
// Included as a submodule of `tests/all.rs` because the crate's
// `autotests = false` setting disables the per-file auto-target
// detection.

use arest::ast::{self, Object};
use arest::parse_forml2::parse_to_state_from;

const RENDER_MD:     &str = include_str!("../../../readings/ui/render.md");
const UI_MD:         &str = include_str!("../../../readings/ui/ui.md");
const MONOVIEW_MD:   &str = include_str!("../../../readings/ui/monoview.md");
const COMPONENTS_MD: &str = include_str!("../../../readings/ui/components.md");

/// `readings/ui/render.md` parses standalone with no parser error.
#[test]
fn render_md_parses() {
    let empty = Object::phi();
    let result = parse_to_state_from(RENDER_MD, &empty);
    assert!(
        result.is_ok(),
        "render.md should parse cleanly, got: {:?}",
        result.err(),
    );
}

/// `readings/ui/render.md` composes with `ui.md`, `monoview.md`, and
/// `components.md` — the four parse and fold without parser error,
/// and the merged state passes `check_readings_func` without
/// surfacing Layer-2 (ring validity) or Layer-3 (ring completeness)
/// diagnostics introduced by render.md.
///
/// The pre-existing UI readings already contain a handful of cross-
/// reading references the checker expects to see resolved at the
/// merged state; this test fixes the baseline diagnostic count from
/// the ui.md+monoview.md+components.md fold and asserts that adding
/// render.md does not increase it.
#[test]
fn render_md_composes_with_ui_readings() {
    let empty = Object::phi();

    let ui_state         = parse_to_state_from(UI_MD,         &empty).expect("ui.md parses");
    let monoview_state   = parse_to_state_from(MONOVIEW_MD,   &ui_state).expect("monoview.md parses");
    let components_state = parse_to_state_from(COMPONENTS_MD, &monoview_state).expect("components.md parses");

    // Baseline: the pre-#593 fold of ui+monoview+components.
    let baseline_diags = arest::check::check_readings(&fold_corpus(&[
        UI_MD, MONOVIEW_MD, COMPONENTS_MD,
    ]));
    let baseline_errors = baseline_diags.iter()
        .filter(|d| matches!(d.level, arest::check::Level::Error))
        .count();

    // Fold render.md on top of the existing UI corpus.
    let render_state = parse_to_state_from(RENDER_MD, &components_state)
        .expect("render.md composes with the UI readings");

    // Sanity: we got *some* state back — at minimum the Display /
    // Surface / Frame nouns landed somewhere on the cell graph.
    assert_ne!(
        render_state,
        Object::phi(),
        "merged state should not be the empty cell after folding render.md",
    );

    // Re-check the merged corpus end-to-end. render.md must not
    // introduce new Layer-2 / Layer-3 errors.
    let merged_corpus = fold_corpus(&[UI_MD, MONOVIEW_MD, COMPONENTS_MD, RENDER_MD]);
    let merged_diags = arest::check::check_readings(&merged_corpus);
    let merged_errors = merged_diags.iter()
        .filter(|d| matches!(d.level, arest::check::Level::Error))
        .count();

    assert!(
        merged_errors <= baseline_errors,
        "render.md introduced {} new error diagnostic(s); merged_errors={} baseline_errors={}\nmerged diags:\n{:#?}",
        merged_errors.saturating_sub(baseline_errors),
        merged_errors,
        baseline_errors,
        merged_diags,
    );

    // The applied check_readings_func must terminate without panic
    // on the merged state — the second half of the type-check path.
    let merged_state = parse_to_state_from(&merged_corpus, &empty)
        .expect("merged corpus parses");
    let _ = ast::apply(
        &arest::check::check_readings_func(),
        &merged_state,
        &merged_state,
    );
}

/// Concatenate reading bodies with blank-line separators, matching
/// `lib.rs::metamodel_corpus()`'s fold shape so the readings checker
/// sees the same byte stream the bake produces.
fn fold_corpus(readings: &[&str]) -> String {
    readings.iter().fold(String::new(), |mut acc, body| {
        acc.push_str(body);
        acc.push_str("\n\n");
        acc
    })
}
