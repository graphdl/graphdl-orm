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
    // Set / unset AREST_NO_LIB_CACHE around the compile.
    if no_lib_cache {
        // SAFETY: single-threaded test context; setting an env var here does
        // not race against other threads because Rust's test harness runs each
        // `#[test]` fn in isolation when there is only one thread per process
        // (the default).  Tests in this file set/unset the key in a scoped
        // guard pattern so no test leaves a dirty env.
        std::env::set_var("AREST_NO_LIB_CACHE", "1");
    }

    let result = compile_app_to_state_inner(readings);

    if no_lib_cache {
        std::env::remove_var("AREST_NO_LIB_CACHE");
    }

    result
}

fn compile_app_to_state_inner(readings: &[(&str, &str)]) -> Object {
    // Parse and merge all readings into one state.
    let merged_state = readings.iter().fold(Object::phi(), |acc, (_name, text)| {
        let parsed = arest::parse_forml2_stage2::parse_to_state_via_stage12(text)
            .unwrap_or_else(|e| panic!("parse reading failed: {}", e));
        if matches!(acc, Object::Seq(ref s) if s.is_empty()) {
            parsed
        } else {
            ast::merge_states(&acc, &parsed)
        }
    });

    // Compile defs, overlay onto state.
    let defs = arest::compile::compile_to_defs_state(&merged_state);
    let d = ast::defs_to_state(&defs, &merged_state);

    // Forward-chain derivation rules if any exist (same pattern as
    // cli/entry.rs and the existing e2e tests).
    let derivation_refs_owned: Vec<(String, ast::Func)> = ast::cells_iter(&d)
        .into_iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();

    if derivation_refs_owned.is_empty() {
        return d;
    }

    let derivation_refs: Vec<(&str, &ast::Func)> = derivation_refs_owned.iter()
        .map(|(n, f)| (n.as_str(), f))
        .collect();
    let (final_d, _derived) = arest::evaluate::forward_chain_defs_state(&derivation_refs, &d);
    final_d
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

    // Check same set of cell names.
    let a_names: std::collections::BTreeSet<&str> =
        a_cells.keys().map(|s| s.as_str()).collect();
    let b_names: std::collections::BTreeSet<&str> =
        b_cells.keys().map(|s| s.as_str()).collect();

    if a_names != b_names {
        let only_in_a: Vec<&&str> = a_names.difference(&b_names).collect();
        let only_in_b: Vec<&&str> = b_names.difference(&a_names).collect();
        panic!(
            "assert_states_equivalent: cell name sets differ.\n  \
             Only in A: {:?}\n  Only in B: {:?}",
            only_in_a, only_in_b
        );
    }

    // For each cell, check equivalence using the appropriate strategy.
    //
    // Strategy selection:
    //   - POPULATION cells (name has no `:` — e.g. `Widget`, `FactType`):
    //     identity-aware fact-set equality via `merge_states` → `concat_dedup`
    //     → `same_identity`.  This is the same predicate the engine itself
    //     uses for dedup.
    //   - DEF cells (name contains `:` — e.g. `create:Widget`, `derivation:rule_*`):
    //     structural equality (`==`).  Compiled Funcs are stored as Atom or
    //     nested Seq values; `concat_dedup` would treat Atom contents as a
    //     single opaque item, not as a set of facts.  Structural equality is
    //     both correct and cheaper.  (Same distinction used in entry.rs and
    //     derivation_rule_replace_on_recompile_e2e.rs.)
    for name in a_names.iter() {
        let a_contents: &Object = a_cells[*name];
        let b_contents: &Object = b_cells[*name];

        if name.contains(':') {
            // DEF cell: compiled Func — structural equality.
            if a_contents != b_contents {
                panic!(
                    "assert_states_equivalent: def cell '{}' differs between A and B.\n  \
                     A: {:?}\n  B: {:?}",
                    name, a_contents, b_contents
                );
            }
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
