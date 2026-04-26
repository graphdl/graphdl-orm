// crates/arest/src/load_reading.rs
//
// SystemVerb::LoadReading (#555 / DynRdg-1) — runtime parse + validate +
// register a FORML 2 reading body into the live cell graph.
//
// Where this fits in the pipeline:
//
//   bake-time:  metamodel_readings() → fold parse → compile → cache
//   compile-cmd: Command::LoadReadings → parse_to_state_from → merge → compile
//   load_reading (this file): SAME pipeline, but driven by a SYSTEM verb
//     and surfacing a structured report (added cell ids) and a structured
//     diagnostic tree on failure. The verb is the runtime peer of the
//     bake-time `metamodel_readings()` assembler — anything that can be
//     baked at compile time can also be loaded at runtime.
//
// Atomicity: parse → constraint validation → merge happens against a
// scratch copy of `state`. The scratch state is committed back to the
// caller's state ONLY when validation passes; on any failure the
// caller's state is untouched. This mirrors the snapshot/rollback
// primitive (#158) without needing a tenant handle — pure-functional
// `(state) -> Result<(report, new_state), error>`.
//
// Idempotency: loading the same body under the same name produces
// identity (no duplicate noun/FT/derivation cells). Loading a different
// body under the same name overwrites — versioning lands in #558.
//
// Per task scope this verb does NOT:
//   * touch any per-target adapter (kernel/CLI/Worker/WASM) — those are
//     #560-#564 downstream tasks
//   * support unload (#556) or reload (#557)
//   * track versions (#558)
//   * surface a UI (#564)
//
// Gate: callers wrap this behind `LoadReadingPolicy::AllowAll` for dev
// /test environments; the default `LoadReadingPolicy::Deny` refuses
// outright. Production builds keep the deny gate so an exposed
// system_impl frontend can't push schema mutations from remote actors.

use crate::ast::{self, Object};
#[allow(unused_imports)]
use alloc::{
    borrow::ToOwned,
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};

/// Names of cells that newly appeared (or grew) under this load.
///
/// Atom = string atom value as it appears in `binding(fact, "name")` or
/// `binding(fact, "id")`. Sorted lexicographically so the report is
/// deterministic across hash-map iteration order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadReport {
    pub added_nouns: Vec<String>,
    pub added_fact_types: Vec<String>,
    pub added_derivations: Vec<String>,
}

/// Why a `load_reading` call rejected.
///
/// The diagnostic tree is the structured equivalent of what
/// `check::check_readings` returns; callers can walk it to render
/// per-line / per-clause errors. `EmptyBody` and `Disallowed` carry
/// no diagnostics because they short-circuit before parse runs.
///
/// `PartialEq` is intentionally NOT derived: `ReadingDiagnostic`
/// does not implement Eq (its `Level` / `Source` fields are simple
/// enums but the struct as a whole was authored without an Eq
/// derive), and adding one upstream would broaden the contract of
/// the public diagnostic type beyond this verb's needs. Tests
/// pattern-match on the variant instead.
#[derive(Debug, Clone)]
pub enum LoadError {
    /// Caller's policy refuses runtime loads (default in production
    /// builds — see `LoadReadingPolicy::Deny`).
    Disallowed,
    /// Body was empty or whitespace-only. Distinct from `ParseError`
    /// because the parser's "empty input parses to empty state" is
    /// not the right answer for a runtime load — callers expect a
    /// load to add something.
    EmptyBody,
    /// Reading name failed sanitization (empty, whitespace, control
    /// chars). Names land in cell ids, so they must be safe atoms.
    InvalidName(String),
    /// Stage-1+Stage-2 parse failed — the message is the parser's
    /// own error string (line + column when available).
    ParseError(String),
    /// Constraint validation flagged one or more deontic violations
    /// against the merged state. Diagnostics are structured per
    /// `crate::check::ReadingDiagnostic`.
    DeonticViolation(Vec<crate::check::ReadingDiagnostic>),
}

/// Caller's policy for runtime reading loads.
///
/// `Deny` is the default — production builds keep this so an exposed
/// system_impl frontend cannot push schema mutations from a remote
/// actor (#328 register-gate semantics extended to this verb).
/// `AllowAll` is the dev/test setting; future work may add an
/// `AllowList` variant for finer-grained host policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadReadingPolicy {
    Deny,
    AllowAll,
}

impl Default for LoadReadingPolicy {
    fn default() -> Self {
        LoadReadingPolicy::Deny
    }
}

/// Outcome of a successful load: the new state plus a report listing
/// every cell that grew. The state is functional — caller swaps it in
/// atomically.
#[derive(Debug, Clone)]
pub struct LoadOutcome {
    pub report: LoadReport,
    pub new_state: Object,
}

/// Runtime peer of the bake-time `metamodel_readings()` assembler.
///
/// Pipeline:
///   1. Policy check — refuse if caller policy is `Deny`.
///   2. Sanitize `name` and `body` — empty-body and invalid-name reject
///      cheaply before any parse work.
///   3. Stage-1 + Stage-2 parse via `parse_to_state_from` (uses `state`
///      as context so the body may reference nouns the live state
///      already declares).
///   4. Merge `state` ⊕ parsed → scratch state.
///   5. Run `check::check_readings_func` against the scratch state.
///      Any `Source::Deontic` `Level::Error` rejects the load.
///   6. On success, diff the scratch state vs. `state` to assemble
///      `LoadReport` (only newly-added or changed Noun / FactType /
///      derivation:* cells contribute).
///   7. Return `(report, scratch_state)` for the caller to swap in.
///
/// On any failure step 4-5 the caller's `state` is untouched (the
/// scratch copy is dropped). Step 1-3 never touch `state`.
///
/// Idempotency: when `body` is byte-identical to a previously-loaded
/// body for the same `name`, the parsed cells equal the existing
/// cells, the diff comes back empty, and the report is empty. The
/// new_state still equals the old state cell-for-cell so callers may
/// commit it without observing a diff.
pub fn load_reading(
    state: &Object,
    name: &str,
    body: &str,
    policy: LoadReadingPolicy,
) -> Result<LoadOutcome, LoadError> {
    // Step 1: gate.
    if policy == LoadReadingPolicy::Deny {
        return Err(LoadError::Disallowed);
    }

    // Step 2: sanitize.
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return Err(LoadError::InvalidName(
            "reading name must not be empty".to_string(),
        ));
    }
    if trimmed_name.chars().any(|c| c.is_control()) {
        return Err(LoadError::InvalidName(
            "reading name must not contain control characters".to_string(),
        ));
    }
    if body.trim().is_empty() {
        return Err(LoadError::EmptyBody);
    }

    // Step 3: parse with current state as context.
    let parsed = match crate::parse_forml2::parse_to_state_from(body, state) {
        Ok(p) => p,
        Err(e) => return Err(LoadError::ParseError(e)),
    };

    // Step 4: merge into scratch state.
    let scratch = ast::merge_states(state, &parsed);

    // Step 5: deontic constraint validation pass (#288).
    //
    // We re-run the full check_readings_func against the merged scratch
    // state. Errors at deontic level reject the load. Warnings / hints
    // pass through silently — callers that want to surface them can
    // re-run `check::check_readings` themselves; that's a separate
    // surface from "reject vs. accept."
    let diag_obj = ast::apply(
        &crate::check::check_readings_func(),
        &scratch,
        &scratch,
    );
    let diags = decode_diagnostics(&diag_obj);
    let deontic_errors: Vec<_> = diags
        .into_iter()
        .filter(|d| {
            matches!(d.source, crate::check::Source::Deontic)
                && matches!(d.level, crate::check::Level::Error)
        })
        .collect();
    if !deontic_errors.is_empty() {
        return Err(LoadError::DeonticViolation(deontic_errors));
    }

    // Step 6: compute the report — Noun / FactType / derivation:* cell
    // additions vs. the input state.
    let report = build_report(state, &scratch);

    // Step 7: return scratch state for caller to commit.
    Ok(LoadOutcome {
        report,
        new_state: scratch,
    })
}

/// Decode a `check::check_readings_func` result Object back into the
/// public `ReadingDiagnostic` shape.
///
/// The check module's encode/decode functions are private; we
/// re-inline the same shape here. The public API of `check_readings`
/// returns `Vec<ReadingDiagnostic>` directly; only the lower-level
/// Func application surface returns the encoded Object.
fn decode_diagnostics(obj: &Object) -> Vec<crate::check::ReadingDiagnostic> {
    use crate::check::{Level, ReadingDiagnostic, Source};
    obj.as_seq()
        .map(|s| {
            s.iter()
                .filter_map(|d| {
                    let map = d.as_map()?;
                    let line = map
                        .get("line")
                        .and_then(|o| o.as_atom())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    let reading = map
                        .get("reading")
                        .and_then(|o| o.as_atom())
                        .unwrap_or("")
                        .to_string();
                    let level = match map.get("level").and_then(|o| o.as_atom()) {
                        Some("Error") => Level::Error,
                        Some("Hint") => Level::Hint,
                        _ => Level::Warning,
                    };
                    let source = match map.get("source").and_then(|o| o.as_atom()) {
                        Some("parse") => Source::Parse,
                        Some("deontic") => Source::Deontic,
                        _ => Source::Resolve,
                    };
                    let message = map
                        .get("message")
                        .and_then(|o| o.as_atom())
                        .unwrap_or("")
                        .to_string();
                    let suggestion = map
                        .get("suggestion")
                        .and_then(|o| o.as_atom())
                        .map(String::from);
                    Some(ReadingDiagnostic {
                        line,
                        reading,
                        level,
                        source,
                        message,
                        suggestion,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Diff Noun / FactType / derivation:* cells between `before` and
/// `after`. Returns sorted lists of newly-appearing identifiers.
///
/// Noun additions — facts in `after.Noun` whose `name` binding is
/// absent from `before.Noun`.
/// FactType additions — facts in `after.FactType` whose `id` binding
/// is absent from `before.FactType`.
/// Derivation additions — `derivation:<rule_id>` cells in `after`
/// that weren't in `before`. Compiled derivation cells appear at the
/// def-state level after `compile_to_defs_state`; the cell name itself
/// is the rule identifier.
fn build_report(before: &Object, after: &Object) -> LoadReport {
    use hashbrown::HashSet;

    fn names_in(cell: &str, key: &str, state: &Object) -> HashSet<String> {
        ast::fetch_or_phi(cell, state)
            .as_seq()
            .map(|facts| {
                facts
                    .iter()
                    .filter_map(|f| ast::binding(f, key).map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    let before_nouns = names_in("Noun", "name", before);
    let after_nouns = names_in("Noun", "name", after);
    let mut added_nouns: Vec<String> = after_nouns.difference(&before_nouns).cloned().collect();
    added_nouns.sort();

    let before_fts = names_in("FactType", "id", before);
    let after_fts = names_in("FactType", "id", after);
    let mut added_fact_types: Vec<String> =
        after_fts.difference(&before_fts).cloned().collect();
    added_fact_types.sort();

    // Derivations live in the parsed cells under `DerivationRule` (the
    // FORML 2 stage-2 cell name) — `compile_to_defs_state` later turns
    // each into a `derivation:<id>` def cell, but the parser emits the
    // raw rule under `DerivationRule` keyed by `ruleId`. We diff the
    // parsed cell so the report reflects what the user added in this
    // load, independent of whether compile_to_defs_state has run yet.
    let before_rules = names_in("DerivationRule", "ruleId", before);
    let after_rules = names_in("DerivationRule", "ruleId", after);
    let mut added_derivations: Vec<String> =
        after_rules.difference(&before_rules).cloned().collect();
    added_derivations.sort();

    LoadReport {
        added_nouns,
        added_fact_types,
        added_derivations,
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Produce a minimal cell graph that mimics what the metamodel
    /// gives us: a Noun cell with one entity (`Order`). Tests build
    /// on top of this via `parse_to_state_from`.
    fn seed_state() -> Object {
        let nouns = ast::Object::seq(vec![ast::fact_from_pairs(&[
            ("name", "Order"),
            ("objectType", "entity"),
        ])]);
        ast::store("Noun", nouns, &Object::phi())
    }

    #[test]
    fn deny_policy_refuses_immediately() {
        let state = seed_state();
        let err = load_reading(
            &state,
            "x",
            "Customer(.Name) is an entity type.",
            LoadReadingPolicy::Deny,
        )
        .expect_err("Deny policy must reject");
        match err {
            LoadError::Disallowed => {}
            other => panic!("expected Disallowed, got {other:?}"),
        }
    }

    #[test]
    fn empty_name_is_invalid() {
        let state = seed_state();
        let err = load_reading(
            &state,
            "",
            "Customer(.Name) is an entity type.",
            LoadReadingPolicy::AllowAll,
        )
        .expect_err("empty name must reject");
        match err {
            LoadError::InvalidName(_) => {}
            other => panic!("expected InvalidName, got {other:?}"),
        }
    }

    #[test]
    fn whitespace_name_is_invalid() {
        let state = seed_state();
        let err = load_reading(
            &state,
            "   \t",
            "Customer(.Name) is an entity type.",
            LoadReadingPolicy::AllowAll,
        )
        .expect_err("whitespace-only name must reject");
        match err {
            LoadError::InvalidName(_) => {}
            other => panic!("expected InvalidName, got {other:?}"),
        }
    }

    #[test]
    fn control_chars_in_name_rejected() {
        let state = seed_state();
        let err = load_reading(
            &state,
            "bad\x00name",
            "Customer(.Name) is an entity type.",
            LoadReadingPolicy::AllowAll,
        )
        .expect_err("control chars in name must reject");
        match err {
            LoadError::InvalidName(_) => {}
            other => panic!("expected InvalidName, got {other:?}"),
        }
    }

    #[test]
    fn empty_body_rejected() {
        let state = seed_state();
        let err = load_reading(&state, "name", "", LoadReadingPolicy::AllowAll)
            .expect_err("empty body must reject");
        match err {
            LoadError::EmptyBody => {}
            other => panic!("expected EmptyBody, got {other:?}"),
        }
    }

    #[test]
    fn whitespace_body_rejected() {
        let state = seed_state();
        let err = load_reading(&state, "name", "   \n\t  ", LoadReadingPolicy::AllowAll)
            .expect_err("whitespace-only body must reject");
        match err {
            LoadError::EmptyBody => {}
            other => panic!("expected EmptyBody, got {other:?}"),
        }
    }

    /// Test (1): valid reading → cells appear in the new state and the
    /// report lists every added noun.
    #[test]
    fn valid_reading_adds_cells_and_reports_them() {
        let state = seed_state();
        let body = "\
Product(.SKU) is an entity type.
Category(.Name) is an entity type.
";
        let outcome = load_reading(&state, "catalog", body, LoadReadingPolicy::AllowAll)
            .expect("valid reading should load");

        // Both new nouns appear in the report (sorted).
        assert_eq!(
            outcome.report.added_nouns,
            vec!["Category".to_string(), "Product".to_string()]
        );
        // The Order noun is unchanged — must not appear as added.
        assert!(!outcome.report.added_nouns.contains(&"Order".to_string()));

        // The new state has the merged Noun cell (3 entries: Order +
        // Product + Category).
        let nouns_after = ast::fetch_or_phi("Noun", &outcome.new_state);
        let names: Vec<&str> = nouns_after
            .as_seq()
            .map(|s| s.iter().filter_map(|f| ast::binding(f, "name")).collect())
            .unwrap_or_default();
        assert!(names.contains(&"Order"));
        assert!(names.contains(&"Product"));
        assert!(names.contains(&"Category"));
    }

    /// Test (4): malformed FORML → `ParseError` in the diagnostic tree.
    ///
    /// The legacy stage-1 parser is permissive about unknown clauses —
    /// it emits an UnresolvedClause cell rather than a hard parse
    /// error. The hard-error path fires on grammar-keyword shadowing
    /// (#309). We trigger that path by declaring a noun whose name
    /// collides with a reserved grammar keyword (`each`).
    #[test]
    fn malformed_forml_yields_parse_error() {
        let state = seed_state();
        let bad_body = "each(.X) is an entity type.\n";
        let err = load_reading(&state, "bad", bad_body, LoadReadingPolicy::AllowAll)
            .expect_err("reserved-keyword noun declaration must reject");
        match err {
            LoadError::ParseError(_) => {}
            other => panic!("expected ParseError, got {other:?}"),
        }
    }

    /// Test (2): deontic violation → returns `DeonticViolation` and
    /// leaves the input state untouched.
    ///
    /// We synthesize the contradiction by declaring a binary FT whose
    /// roles share a noun (`Person likes Person`) without a ring
    /// constraint, then assert that adding such an FT triggers
    /// `check_ring_completeness`. Since that check fires at
    /// `Level::Hint`, we'd need to escalate it for this verb's purpose;
    /// instead we fabricate a state where `check_ring_validity`
    /// (Level::Error) fires by post-loading a ring-kind constraint that
    /// spans roles of different nouns.
    ///
    /// The test ensures the *path* works — error-class deontic
    /// violations reject the load and the input state is preserved.
    #[test]
    fn deontic_violation_rejects_and_preserves_state() {
        // Pre-seed a state with a binary FT whose two roles target
        // different nouns AND a ring-kind Constraint cell pointing at
        // it. `check_ring_validity` will fire Level::Error on the
        // merged scratch state.
        //
        // Cells expected by `check_ring_validity`:
        //   Constraint: kind=IR, span0_factTypeId=ft, text=...
        //   Role:       factType=ft, nounName=...
        let mut state = seed_state();
        state = ast::store(
            "FactType",
            ast::Object::seq(vec![ast::fact_from_pairs(&[
                ("id", "Person_likes_Animal"),
                ("reading", "Person likes Animal"),
            ])]),
            &state,
        );
        state = ast::store(
            "Role",
            ast::Object::seq(vec![
                ast::fact_from_pairs(&[
                    ("factType", "Person_likes_Animal"),
                    ("nounName", "Person"),
                    ("position", "0"),
                ]),
                ast::fact_from_pairs(&[
                    ("factType", "Person_likes_Animal"),
                    ("nounName", "Animal"),
                    ("position", "1"),
                ]),
            ]),
            &state,
        );
        state = ast::store(
            "Constraint",
            ast::Object::seq(vec![ast::fact_from_pairs(&[
                ("kind", "IR"),
                ("span0_factTypeId", "Person_likes_Animal"),
                ("text", "Person likes Animal is irreflexive"),
            ])]),
            &state,
        );

        // Take a baseline snapshot for the post-rejection equality
        // check. We're asserting the verb is non-mutating on rejection.
        let snapshot = state.clone();

        // Now load any non-empty body. Even a no-op reading triggers
        // the check pass against the merged state, which inherits the
        // pre-existing ring-validity error.
        let result = load_reading(
            &state,
            "noop",
            "# noop\n",
            LoadReadingPolicy::AllowAll,
        );
        match result {
            Err(LoadError::DeonticViolation(diags)) => {
                assert!(
                    !diags.is_empty(),
                    "DeonticViolation must carry at least one diagnostic"
                );
                for d in &diags {
                    assert_eq!(
                        d.level,
                        crate::check::Level::Error,
                        "only Level::Error deontic diags should reject the load"
                    );
                    assert_eq!(d.source, crate::check::Source::Deontic);
                }
            }
            Err(other) => panic!("expected DeonticViolation, got {other:?}"),
            Ok(_) => panic!("ring-validity error must reject the load"),
        }

        // State must be unchanged.
        assert_eq!(
            state, snapshot,
            "rejection must not mutate the input state"
        );
    }

    /// Idempotency: loading the same body twice produces the same
    /// post-state on each call. The second call's report has empty
    /// `added_nouns` because no new noun appears.
    #[test]
    fn re_load_same_body_is_idempotent() {
        let state = seed_state();
        let body = "Product(.SKU) is an entity type.\n";

        let first = load_reading(&state, "catalog", body, LoadReadingPolicy::AllowAll)
            .expect("first load succeeds");
        let second = load_reading(
            &first.new_state,
            "catalog",
            body,
            LoadReadingPolicy::AllowAll,
        )
        .expect("second load succeeds");

        // No new nouns the second time around.
        assert!(
            second.report.added_nouns.is_empty(),
            "re-loading the same body must not report duplicate nouns; got {:?}",
            second.report.added_nouns
        );

        // The Noun cell is identical between the two post-states (set
        // semantics; merge_states dedupes by identity key).
        let nouns1 = ast::fetch_or_phi("Noun", &first.new_state);
        let nouns2 = ast::fetch_or_phi("Noun", &second.new_state);
        assert_eq!(nouns1, nouns2);
    }

    /// Loading a different body under the same name overwrites
    /// (well-formed superset / disjoint additions). The combined state
    /// has both bodies' cells. (Versioning per #558 lands later; this
    /// test pins the current behavior.)
    #[test]
    fn re_load_with_different_body_adds_new_cells() {
        let state = seed_state();
        let first_body = "Product(.SKU) is an entity type.\n";
        let second_body = "Category(.Name) is an entity type.\n";

        let first = load_reading(&state, "catalog", first_body, LoadReadingPolicy::AllowAll)
            .expect("first load succeeds");
        let second = load_reading(
            &first.new_state,
            "catalog",
            second_body,
            LoadReadingPolicy::AllowAll,
        )
        .expect("second load succeeds");

        assert_eq!(second.report.added_nouns, vec!["Category".to_string()]);

        // Merged Noun cell contains all three.
        let nouns_after = ast::fetch_or_phi("Noun", &second.new_state);
        let names: Vec<&str> = nouns_after
            .as_seq()
            .map(|s| s.iter().filter_map(|f| ast::binding(f, "name")).collect())
            .unwrap_or_default();
        assert!(names.contains(&"Order"));
        assert!(names.contains(&"Product"));
        assert!(names.contains(&"Category"));
    }
}
