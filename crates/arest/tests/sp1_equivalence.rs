// crates/arest/tests/sp1_equivalence.rs
//
// SP1 "build-once libraries" — Task 0: cold-vs-warm equivalence harness.
//
// PURPOSE
// -------
// This file contains two reusable helpers and one sanity test that every
// later SP1 task uses to prove "cold compile == warm compile" correctness.
//
// COMPILE ENTRY USED
// ------------------
// Mirrors the canonical integration-test pattern from `tests/ring_constraint_enforcement_e2e.rs`
// and `tests/derivation_rule_replace_on_recompile_e2e.rs`:
//   1. `arest::parse_forml2_stage2::parse_to_state_via_stage12(text)` — parse
//      one reading text to a cell-shaped state.
//   2. `arest::compile::compile_to_defs_state(&state)` — compile defs.
//   3. `arest::ast::defs_to_state(&defs, &state)` — overlay defs onto state
//      to produce the final derived Object.
//   4. `arest::evaluate::forward_chain_defs_state(&refs, &d)` — forward-chain
//      derivation rules (skipped when there are no `derivation:rule_*` defs,
//      which is the case for the sanity probe reading).
//
// IDENTITY-AWARE COMPARISON
// -------------------------
// `assert_states_equivalent` reuses `arest::ast::merge_states`, which
// internally calls the private `concat_dedup` → `same_identity` predicate.
// The equivalence check is: merge A into B and verify that every cell's
// fact count is unchanged (all of A's facts were already present in B by
// identity), then do the symmetric check (merge B into A). Fact count is
// measured via `arest::ast::cell_facts_iter`.
//
// NO ENGINE CHANGE — this file is pure test infrastructure.

use arest::ast::{self, Object};

// ─── helper: compile ────────────────────────────────────────────────────────

/// Compile a set of `(filename, text)` readings to their final derived state
/// (the post-`defs_to_state` / post-forward-chain Object).
///
/// `no_lib_cache`: when `true`, sets `AREST_NO_LIB_CACHE=1` in the process
/// environment for the duration of the compile.  This env var gates nothing
/// yet — later SP1 tasks will make the engine honour it to bypass the library
/// cache.  Setting it here ensures the flag is THREADED through from the
/// start, so later tasks can rely on compile_app_to_state already passing it.
///
/// Multiple readings are merged in order via `ast::merge_states` so the
/// helper mirrors what cli/entry.rs does when it folds a list of readings
/// into a single state before compiling.
pub fn compile_app_to_state(readings: &[(&str, &str)], no_lib_cache: bool) -> Object {
    // Set / unset AREST_NO_LIB_CACHE around the compile so the env flag is
    // threaded through to anything that reads it (the SP1 cache honours it
    // independently via the explicit `no_lib_cache` argument below; setting
    // the env too keeps the two in lockstep for any nested read).
    if no_lib_cache {
        // SAFETY: single-threaded test context; setting an env var here does
        // not race against other threads because Rust's test harness runs each
        // `#[test]` fn in isolation when there is only one thread per process
        // (the default).  Tests in this file set/unset the key in a scoped
        // guard pattern so no test leaves a dirty env.
        std::env::set_var("AREST_NO_LIB_CACHE", "1");
    }

    // SP1: route through the build-once-library compile pipeline. COLD
    // (no_lib_cache=true) is the full recompile reference; WARM
    // (no_lib_cache=false) loads the pre-built metamodel library and
    // delta-derives only the app's additions. Both fold the metamodel as
    // the library, so this is a real cold-vs-warm equivalence check (the
    // earlier metamodel-free helper only proved compile determinism).
    let result = arest::sp1::compile_app_with_library(readings, no_lib_cache);

    if no_lib_cache {
        std::env::remove_var("AREST_NO_LIB_CACHE");
    }

    result
}

// ─── helper: equivalence assertion ─────────────────────────────────────────

/// Identity-aware per-cell equivalence assertion.
///
/// Asserts that `a` and `b` represent the same derived state:
///   * Both have the same SET of cell names.
///   * For every cell, the fact sets are equal under the engine's existing
///     identity-aware notion (`same_identity`, reached via `merge_states` /
///     `concat_dedup`).
///
/// HOW THE IDENTITY CHECK WORKS
/// The engine's `merge_states` calls `concat_dedup` which calls the private
/// `same_identity` predicate.  `concat_dedup` is a SET-union: facts already
/// present by identity are not re-appended.  Therefore, if merging the full
/// state A into state B leaves every cell's fact count unchanged, then for
/// every cell all of A's facts are already "in" B by identity — meaning
/// A ⊆ B.  Doing both directions (A ⊆ B and B ⊆ A) proves equality.
///
/// CELL TYPES
/// There are two kinds of cells in a compiled state:
///  - FACT cells (Seq or Map backed): e.g. `Widget`, `FactType`, `Constraint`.
///    These are compared identity-aware via merge.
///  - DEF cells (contain a compiled Func): e.g. `create:Widget`, `derivation:rule_*`.
///    These are compared via structural equality (`==`), since the merge path
///    treats a non-Seq/non-Map value as a single "item" and the identity check
///    does not apply.
///
/// We count facts via `ast::cell_facts_iter`, which handles both Seq-backed
/// and Map-backed (keyed) cells transparently.
pub fn assert_states_equivalent(a: &Object, b: &Object) {
    // Collect cell names from both states.
    let a_cells: std::collections::BTreeMap<String, &Object> = ast::cells_iter(a)
        .into_iter()
        .map(|(name, contents)| (name.to_string(), contents))
        .collect();
    let b_cells: std::collections::BTreeMap<String, &Object> = ast::cells_iter(b)
        .into_iter()
        .map(|(name, contents)| (name.to_string(), contents))
        .collect();

    // A cell that is PRESENT-BUT-EMPTY (phi / empty Seq, zero facts) is
    // semantically identical to an ABSENT cell — both denote "no facts" and
    // both read back as phi via `fetch_or_phi`. The compile pipeline legitimately
    // differs on which empty derived-FT cells it materializes (a warm compile
    // can carry an empty library consequent cell the cold parse-only base never
    // creates), so the cell-NAME-SET equivalence is taken over MEANINGFUL cells
    // only: those with ≥1 fact, or a non-empty non-Seq value (a compiled Func /
    // Atom def cell). Empty cells are compared structurally in the per-cell loop
    // below anyway (phi == phi), so nothing is lost.
    let meaningful = |contents: &Object| -> bool {
        if ast::cell_facts_iter(contents).next().is_some() {
            return true;
        }
        // Non-Seq, non-phi values are def cells (Atom/Func) — meaningful.
        !matches!(contents, Object::Seq(_)) && *contents != Object::phi()
    };
    let a_names: std::collections::BTreeSet<&str> =
        a_cells.iter().filter(|(_, c)| meaningful(c)).map(|(n, _)| n.as_str()).collect();
    let b_names: std::collections::BTreeSet<&str> =
        b_cells.iter().filter(|(_, c)| meaningful(c)).map(|(n, _)| n.as_str()).collect();

    if a_names != b_names {
        let only_in_a: Vec<&&str> = a_names.difference(&b_names).collect();
        let only_in_b: Vec<&&str> = b_names.difference(&a_names).collect();
        panic!(
            "assert_states_equivalent: non-empty cell name sets differ.\n  \
             Only in A: {:?}\n  Only in B: {:?}",
            only_in_a, only_in_b
        );
    }

    // For each cell, check equivalence STRUCTURAL-EQUALITY-FIRST, then
    // identity-aware fall-back.
    //
    //   - STRUCTURAL EQUALITY (`==`) is tried first for EVERY cell. It is
    //     correct for DEF cells (compiled Funcs — `create:Widget`,
    //     `derivation:rule_*`, AND the bare-named `validate` / `debug` /
    //     platform-dispatch defs) which a deterministic compile reproduces
    //     byte-for-byte, and it short-circuits any population cell whose
    //     rows already match in order.  This replaces the old
    //     `name.contains(':')` heuristic, which mis-routed bare-named Func
    //     cells (no `:`) into the population branch — where
    //     `cell_facts_iter` reports 0 facts but `merge_states` concatenates
    //     the Func value as one opaque item, spuriously failing.
    //   - IDENTITY-AWARE fall-back: when structural equality fails, the
    //     cell is treated as a POPULATION cell and compared via
    //     `merge_states` → `concat_dedup` → `same_identity` (the engine's
    //     own dedup predicate), which tolerates row-order differences.  A
    //     genuinely divergent def cell fails structural eq AND the merge
    //     (its single Func item differs by identity), so it is still
    //     caught.
    for name in a_names.iter() {
        let a_contents: &Object = a_cells[*name];
        let b_contents: &Object = b_cells[*name];

        // Fast path / def-cell path: byte-identical contents are equivalent.
        if a_contents == b_contents {
            continue;
        }

        let a_count = ast::cell_facts_iter(a_contents).count();
        let b_count = ast::cell_facts_iter(b_contents).count();

        // Build single-cell mini-states so we can call merge_states.
        // Both mini-states are based on Object::phi() (empty Map) as the
        // base so they each contain exactly one cell.
        let state_a = ast::store(name, a_contents.clone(), &Object::phi());
        let state_b = ast::store(name, b_contents.clone(), &Object::phi());

        // A ⊆ B: merge A into B; merged cell count should equal b_count.
        let merged_ab = ast::merge_states(&state_b, &state_a);
        let merged_ab_contents = ast::fetch_or_phi(name, &merged_ab);
        let merged_ab_count = ast::cell_facts_iter(&merged_ab_contents).count();

        if merged_ab_count != b_count {
            panic!(
                "assert_states_equivalent: cell '{}' has facts in A not present in B \
                 by identity.\n  A fact count: {}\n  B fact count: {}\n  \
                 After merging A into B, count grew to {} (expected {} — no new facts).",
                name, a_count, b_count, merged_ab_count, b_count
            );
        }

        // B ⊆ A: merge B into A; merged cell count should equal a_count.
        let merged_ba = ast::merge_states(&state_a, &state_b);
        let merged_ba_contents = ast::fetch_or_phi(name, &merged_ba);
        let merged_ba_count = ast::cell_facts_iter(&merged_ba_contents).count();

        if merged_ba_count != a_count {
            panic!(
                "assert_states_equivalent: cell '{}' has facts in B not present in A \
                 by identity.\n  A fact count: {}\n  B fact count: {}\n  \
                 After merging B into A, count grew to {} (expected {} — no new facts).",
                name, a_count, b_count, merged_ba_count, a_count
            );
        }
    }
}

// ─── sanity test ────────────────────────────────────────────────────────────

/// Compile a trivial app twice (both no_lib_cache=true) and assert the two
/// resulting states are equivalent.  This is the "cold compile is
/// self-consistent" sanity check — it proves that:
///   * the compile pipeline is deterministic (same input → same output), and
///   * `assert_states_equivalent` correctly passes on identical results.
///
/// The reading declares a simple entity type with one value-type attribute:
///   Widget(.id) is an entity type.
///   Widget has Label.
///   Each Widget has at most one Label.
///   Label is a value type.
#[test]
fn cold_compile_is_self_consistent() {
    let readings: &[(&str, &str)] = &[(
        "probe.md",
        "Widget(.id) is an entity type.\n\
         Widget has Label.\n\
           Each Widget has at most one Label.\n\
         Label is a value type.\n",
    )];

    let state_a = compile_app_to_state(readings, true);
    let state_b = compile_app_to_state(readings, true);

    // Both compiles must produce identical states.
    assert_states_equivalent(&state_a, &state_b);
}

// ─── SP1 cold == warm equivalence (the hard gate) ────────────────────────────

/// SP1 Task 2: a WARM Widget compile (on the pre-built metamodel library)
/// must produce an identity-aware-identical state to a COLD full recompile.
/// This exercises the build-once library cache + the scoped `#836` drop +
/// the seeded-delta chain over the app's delta, and asserts equivalence
/// over ALL cells — including the library-derived `Function` /
/// `Function_belongs_to_Domain` the warm path KEEPS and extends rather than
/// re-derives.
#[test]
fn warm_widget_equals_cold() {
    let app: &[(&str, &str)] = &[(
        "probe.md",
        "Widget(.id) is an entity type.\n\
         Widget has Label.\n\
           Each Widget has at most one Label.\n\
         Label is a value type.\n",
    )];

    let cold = compile_app_to_state(app, /*no_lib_cache=*/ true);
    let warm = compile_app_to_state(app, /*no_lib_cache=*/ false);

    assert_states_equivalent(&cold, &warm);
}

/// SP1 Task 2: the warm path must EXTEND the pre-built library, not freeze
/// it — the app's own schema/population/derivations land on top of the
/// loaded library base. Proven by the app's own nouns appearing in the
/// warm `Noun` cell beyond the library's, AND the warm state carrying the
/// app's `Widget` entity declaration. Shape-independent and premise-safe
/// (does not assume which supertype-union cells the current readings
/// populate — the absorbed-`Function` bridge rules are intentionally
/// disabled in the working tree, so `Function` membership for app nouns is
/// NOT derived; what the app DOES extend is the base schema cells).
#[test]
fn warm_extends_library_with_app_schema() {
    let app: &[(&str, &str)] = &[(
        "probe.md",
        "Widget(.id) is an entity type.\n\
         Widget has Label.\n\
           Each Widget has at most one Label.\n\
         Label is a value type.\n",
    )];

    let lib = arest::sp1::build_metamodel_library();
    let warm = compile_app_to_state(app, /*no_lib_cache=*/ false);

    let noun_count = |st: &Object| ast::cell_facts_iter(&ast::fetch_cell_seq("Noun", st)).count();
    assert!(
        noun_count(&warm) > noun_count(&lib),
        "warm Noun ({}) must extend the library Noun ({}) with the app's \
         own nouns (Widget, Label) — the warm path loads the library and \
         layers the app delta on top",
        noun_count(&warm), noun_count(&lib)
    );
    // The app's Widget entity must be present in the warm Noun population.
    let has_widget = ast::cell_facts_iter(&ast::fetch_cell_seq("Noun", &warm))
        .any(|f| ast::binding(f, "name") == Some("Widget"));
    assert!(has_widget, "warm Noun must contain the app's 'Widget' entity");
}

/// SP1 Task 2 (multi-noun app with an internal reference): a warm compile of
/// an app whose nouns reference each other equals a cold recompile. Gadget
/// and Widget are both app nouns; Widget refers to Gadget. This stresses the
/// app-delta seed across multiple app cells while the metamodel library is
/// reused.
#[test]
fn warm_multi_noun_app_equals_cold() {
    let app: &[(&str, &str)] = &[(
        "app.md",
        "Gadget(.id) is an entity type.\n\
         Gadget has Tag.\n\
           Each Gadget has at most one Tag.\n\
         Tag is a value type.\n\
         Widget(.id) is an entity type.\n\
         Widget refers to Gadget.\n\
           Each Widget refers to at most one Gadget.\n",
    )];

    let cold = compile_app_to_state(app, /*no_lib_cache=*/ true);
    let warm = compile_app_to_state(app, /*no_lib_cache=*/ false);

    assert_states_equivalent(&cold, &warm);
}

/// DIAGNOSTIC (not a gate): print every cell whose fact count differs
/// between cold and warm, plus a sample of the divergent rows. Run with
/// `--ignored --nocapture` to localize a cold!=warm bug.
#[test]
#[ignore]
fn diag_cold_warm_cell_diff() {
    let app: &[(&str, &str)] = &[(
        "probe.md",
        "Widget(.id) is an entity type.\n\
         Widget has Label.\n\
           Each Widget has at most one Label.\n\
         Label is a value type.\n",
    )];
    let cold = compile_app_to_state(app, true);
    let warm = compile_app_to_state(app, false);

    let cold_cells: std::collections::BTreeMap<String, &Object> = ast::cells_iter(&cold)
        .into_iter().map(|(n, c)| (n.to_string(), c)).collect();
    let warm_cells: std::collections::BTreeMap<String, &Object> = ast::cells_iter(&warm)
        .into_iter().map(|(n, c)| (n.to_string(), c)).collect();
    let all: std::collections::BTreeSet<String> = cold_cells.keys()
        .chain(warm_cells.keys()).cloned().collect();
    let phi = Object::phi();
    let mut ndiff = 0;
    for name in &all {
        let c = cold_cells.get(name).copied().unwrap_or(&phi);
        let w = warm_cells.get(name).copied().unwrap_or(&phi);
        if c == w { continue; }
        let cc = ast::cell_facts_iter(c).count();
        let wc = ast::cell_facts_iter(w).count();
        ndiff += 1;
        eprintln!("DIFF cell '{}': cold={} warm={}{}", name, cc, wc,
            if cc == wc { "  (row-order only?)" } else { "" });
        let cold_rows: std::collections::HashSet<String> =
            ast::cell_facts_iter(c).map(|f| f.to_string()).collect();
        let warm_rows: std::collections::HashSet<String> =
            ast::cell_facts_iter(w).map(|f| f.to_string()).collect();
        for r in cold_rows.difference(&warm_rows).take(6) {
            eprintln!("    only-in-COLD: {}", r);
        }
        for r in warm_rows.difference(&cold_rows).take(6) {
            eprintln!("    only-in-WARM: {}", r);
        }
    }
    eprintln!("TOTAL differing cells: {}", ndiff);
}

/// DIAGNOSTIC: trace the `id` noun + its Function_belongs_to_Domain row
/// across the library, the cold-merged, and the warm-merged states.
#[test]
#[ignore]
fn diag_id_noun_domain() {
    let app: &[(&str, &str)] = &[(
        "probe.md",
        "Widget(.id) is an entity type.\n\
         Widget has Label.\n\
           Each Widget has at most one Label.\n\
         Label is a value type.\n",
    )];
    let dump = |label: &str, st: &Object| {
        let noun = ast::fetch_cell_seq("Noun", st);
        for f in ast::cell_facts_iter(&noun) {
            if ast::binding(f, "name") == Some("id") {
                eprintln!("[{}] Noun id: homeDomain={:?} objectType={:?}",
                    label, ast::binding(f, "homeDomain"), ast::binding(f, "objectType"));
            }
        }
        let fbd = ast::fetch_cell_seq("Function_belongs_to_Domain", st);
        for f in ast::cell_facts_iter(&fbd) {
            if ast::binding(f, "Function") == Some("id") {
                eprintln!("[{}] Function_belongs_to_Domain id -> Domain={:?}",
                    label, ast::binding(f, "Domain"));
            }
        }
    };
    let lib = arest::sp1::build_metamodel_library();
    dump("LIB", &lib);
    let cold = compile_app_to_state(app, true);
    dump("COLD", &cold);
    let warm = compile_app_to_state(app, false);
    dump("WARM", &warm);
}

// ─── SP1 Task 3: chained dependency-library builds ──────────────────────────

/// Compile an ORDERED list of reading-layers (each but the last is a
/// dependency library; the last is the app) to the final derived state.
/// Mirrors `compile_app_to_state` over a dependency chain: WARM builds each
/// dependency layer's derived LFP (chained on its predecessors) and
/// delta-derives the app on the chain union; COLD merges all layers and
/// full-recompiles (the reference). Threads `AREST_NO_LIB_CACHE` like the
/// single-layer helper.
pub fn compile_layers_to_state(layers: &[&[(&str, &str)]], no_lib_cache: bool) -> Object {
    if no_lib_cache {
        std::env::set_var("AREST_NO_LIB_CACHE", "1");
    }
    let result = arest::sp1::compile_layers_with_library(layers, no_lib_cache);
    if no_lib_cache {
        std::env::remove_var("AREST_NO_LIB_CACHE");
    }
    result
}

/// SP1 Task 3: a 2-layer compile (one dependency dir + the app dir) WARM
/// (chained library load) must equal a COLD full recompile. The app's
/// `Widget refers to Gadget` references the dependency layer's `Gadget`, so
/// the app delta-derives across a library boundary — the chain must carry
/// the dependency's derived cells as the prior, not re-derive or drop them.
#[test]
fn warm_two_layer_equals_cold() {
    let lib: &[(&str, &str)] = &[(
        "lib.md",
        "Gadget(.id) is an entity type.\n\
         Gadget has Tag.\n\
           Each Gadget has at most one Tag.\n\
         Tag is a value type.\n",
    )];
    let app: &[(&str, &str)] = &[(
        "app.md",
        "Widget(.id) is an entity type.\n\
         Widget refers to Gadget.\n\
           Each Widget refers to at most one Gadget.\n",
    )];
    let cold = compile_layers_to_state(&[lib, app], /*no_lib_cache=*/ true);
    let warm = compile_layers_to_state(&[lib, app], /*no_lib_cache=*/ false);
    assert_states_equivalent(&cold, &warm);
}
