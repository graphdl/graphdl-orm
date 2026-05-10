// crates/arest/tests/unary_derivation_e2e.rs
//
// End-to-end pin (#866 follow-up): exercise the full apps/tasks-style
// readings → compile → forward-chain → cell-table flow for derivation
// rules whose CONSEQUENT is a unary fact type
// (`Task is parallelizable`, `Task is recommended`,
//  `Task is file-conflicting`).
//
// The unit test `user_unary_iff_rule_fires_forward_chain_over_population_at_compile_time`
// in compile.rs::schema_tests passed against a SIMPLIFIED fixture (one
// antecedent, no unary FT in the BODY). The real bug surfaces only when
// the unary FT also appears in the rule body (positively, as in
// `Task is recommended iff … and Task is parallelizable`, or negatively
// as in `Task is parallelizable iff … and Task is not file-conflicting`).
// Without a body-antecedent unary path, the compile-time consequent FT
// resolution can mask gaps in the antecedent FT resolution that only
// appear when the unary FT id needs to roundtrip through the catalog.
//
// The test mirrors the apps/tasks/readings/app.md derivation block
// verbatim and asserts each per-FT cell materializes with the expected
// rows after the 2-stratum forward chain (positive rules to fixpoint
// then negation rules — same order the CLI uses in cli/entry.rs).

use arest::ast::{self, Object};

#[test]
fn apps_tasks_unary_derivations_materialize_per_ft_cells_after_forward_chain() {
    // Mirrors apps/tasks/readings/app.md: the schema declares three
    // unary FTs (`Task is parallelizable`, `Task is file-conflicting`,
    // `Task is recommended`) plus the binary FTs they depend on, then
    // three derivation rules whose consequent is a unary FT and whose
    // body references either another unary FT (positive: `recommended
    // iff … and parallelizable`) or its negation (`parallelizable iff …
    // and not file-conflicting`). The ring FT rule for
    // `file-conflicting` exercises the per-tuple equi-join path.
    //
    // Fixture is two pending Tasks both touching one Source File. With
    // both 'p0' and Status 'pending', without any in_progress task,
    // both should land in `Task_is_parallelizable` and
    // `Task_is_recommended`; neither should land in
    // `Task_is_file-conflicting`.
    let src = "\
        Task(.id) is an entity type.\n\
        Source File(.path) is an entity type.\n\
        Task Status is a value type.\n\
        Task Readiness is a value type.\n\
        Task Priority is a value type.\n\
        Task has Task Status.\n\
        Task has Task Readiness.\n\
        Task has Task Priority.\n\
        Task is parallelizable.\n\
        Task is file-conflicting.\n\
        Task is recommended.\n\
        Task touches Source File.\n\
        Task '1' has Task Status 'pending'.\n\
        Task '2' has Task Status 'pending'.\n\
        Task '1' has Task Priority 'p0'.\n\
        Task '2' has Task Priority 'p0'.\n\
        Task '1' has Task Readiness 'ready'.\n\
        Task '2' has Task Readiness 'ready'.\n\
        Task '1' touches Source File 'src/foo.rs'.\n\
        Task '2' touches Source File 'src/bar.rs'.\n\
        * Task2 is file-conflicting iff Task1 has Task Status 'in_progress' and Task1 touches Source File and Task2 touches Source File.\n\
        * Task is parallelizable iff Task has Task Readiness 'ready' and Task is not file-conflicting.\n\
        * Task is recommended iff Task has Task Readiness 'ready' and Task has Task Priority 'p0' and Task is parallelizable.\n\
    ";
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src)
        .expect("parse must succeed");

    // Sanity: each unary FT must register on the metamodel `FactType`
    // cell. If parse drops them, every later assertion is meaningless.
    let ft_cell = ast::fetch_or_phi("FactType", &state);
    let ft_ids: Vec<String> = ft_cell.as_seq()
        .map(|s| s.iter()
            .filter_map(|f| ast::binding(f, "id").map(String::from))
            .collect())
        .unwrap_or_default();
    for expected in [
        "Task_is_parallelizable",
        "Task_is_file-conflicting",
        "Task_is_recommended",
    ] {
        assert!(ft_ids.iter().any(|id| id == expected),
            "FactType cell must include unary FT `{}`; got {:?}",
            expected, ft_ids);
    }

    // Sanity: each rule's `consequentFactTypeId` must resolve to the
    // matching unary FT (NOT a verb-only slop, NOT empty). Without this
    // the consequent literal is malformed and the cell never lands.
    let dr_cell = ast::fetch_or_phi("DerivationRule", &state);
    let consequent_ids: Vec<String> = dr_cell.as_seq()
        .map(|s| s.iter()
            .filter_map(|f| ast::binding(f, "consequentFactTypeId").map(String::from))
            .collect())
        .unwrap_or_default();
    for expected in [
        "Task_is_parallelizable",
        "Task_is_file-conflicting",
        "Task_is_recommended",
    ] {
        assert!(consequent_ids.iter().any(|id| id == expected),
            "DerivationRule.consequentFactTypeId must resolve to `{}`; got {:?}",
            expected, consequent_ids);
    }

    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    // 2-stratum joint-fixpoint forward chain — exact mirror of
    // cli/entry.rs. `forward_chain_stratified` alternates stratum 1
    // (positive) ↔ stratum 2 (negation-guarded) until both are at a
    // joint fixpoint. Naïve sequential `chain(s1) -> chain(s2)`
    // misses positive consequents whose antecedents are populated
    // only by stratum-2 rules (#866 follow-up).
    let collect_derivs = |prefix: &str, state: &Object| -> Vec<(String, ast::Func)> {
        ast::cells_iter(state).into_iter()
            .filter(|(n, _)| n.starts_with(prefix))
            .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, state)))
            .collect()
    };
    let stratum1 = collect_derivs("derivation:rule_", &d);
    let stratum2 = collect_derivs("derivation_strat2:rule_", &d);
    let refs1: Vec<(&str, &ast::Func)> = stratum1.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let refs2: Vec<(&str, &ast::Func)> = stratum2.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _derived) = arest::evaluate::forward_chain_stratified(
        &refs1, &refs2, &d, 100);

    // Helper: extract the Task ids from a per-FT cell whose entries are
    // shaped `<<Task, id>>` (unary FT, single role).
    let task_ids_in_cell = |cell_name: &str| -> Vec<String> {
        let cell = ast::fetch_or_phi(cell_name, &new_d);
        cell.as_seq()
            .map(|s| s.iter()
                .filter_map(|f| f.as_seq().and_then(|pairs| pairs.iter().find_map(|p| {
                    let kv = p.as_seq()?;
                    let role = kv.first()?.as_atom()?;
                    if role == "Task" { kv.get(1)?.as_atom().map(String::from) } else { None }
                })))
                .collect())
            .unwrap_or_default()
    };

    let parallelizable = task_ids_in_cell("Task_is_parallelizable");
    let file_conflicting = task_ids_in_cell("Task_is_file-conflicting");
    let recommended = task_ids_in_cell("Task_is_recommended");

    // Spec: no in_progress task touches any file, so file-conflicting
    // is empty. Both pending+ready+p0 tasks land in parallelizable AND
    // recommended.
    assert!(file_conflicting.is_empty(),
        "Task_is_file-conflicting must be empty (no in_progress task in fixture); \
         got {:?}", file_conflicting);
    assert!(parallelizable.contains(&"1".to_string())
        && parallelizable.contains(&"2".to_string()),
        "Task_is_parallelizable must include both Task '1' and Task '2' \
         (both ready and not file-conflicting); got parallelizable={:?}, \
         file_conflicting={:?}", parallelizable, file_conflicting);
    assert!(recommended.contains(&"1".to_string())
        && recommended.contains(&"2".to_string()),
        "Task_is_recommended must include both Task '1' and Task '2' \
         (both ready, p0, and parallelizable); got recommended={:?}, \
         parallelizable={:?}, file_conflicting={:?}",
         recommended, parallelizable, file_conflicting);
}

/// Minimal isolation: a derivation rule whose CONSEQUENT is a unary FT
/// AND whose body references ANOTHER unary FT positively as an
/// antecedent. This is the exact apps/tasks case
/// (`Task is recommended iff … and Task is parallelizable`) reduced to
/// its smallest reproduction. The simplified unit test in compile.rs
/// (a single binary antecedent) didn't catch this — that's why the
/// e2e signal lagged.
#[test]
fn unary_ft_in_rule_body_as_positive_antecedent_resolves_and_fires() {
    let src = "\
        Task(.id) is an entity type.\n\
        Task Status is a value type.\n\
        Task has Task Status.\n\
        Task is parallelizable.\n\
        Task is recommended.\n\
        Task '1' has Task Status 'pending'.\n\
        * Task is parallelizable iff Task has Task Status 'pending'.\n\
        * Task is recommended iff Task is parallelizable.\n\
    ";
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src)
        .expect("parse must succeed");

    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    let derivation_refs_owned: Vec<(String, ast::Func)> = ast::cells_iter(&d)
        .into_iter()
        .filter(|(n, _)| n.starts_with("derivation:rule_"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();
    let derivation_refs: Vec<(&str, &ast::Func)> = derivation_refs_owned.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _derived) = arest::evaluate::forward_chain_defs_state(
        &derivation_refs, &d);

    let task_ids_in_cell = |cell_name: &str| -> Vec<String> {
        let cell = ast::fetch_or_phi(cell_name, &new_d);
        cell.as_seq()
            .map(|s| s.iter()
                .filter_map(|f| f.as_seq().and_then(|pairs| pairs.iter().find_map(|p| {
                    let kv = p.as_seq()?;
                    let role = kv.first()?.as_atom()?;
                    if role == "Task" { kv.get(1)?.as_atom().map(String::from) } else { None }
                })))
                .collect())
            .unwrap_or_default()
    };

    let parallelizable = task_ids_in_cell("Task_is_parallelizable");
    let recommended = task_ids_in_cell("Task_is_recommended");
    assert!(parallelizable.contains(&"1".to_string()),
        "Task_is_parallelizable must include Task 1; got {:?}", parallelizable);
    assert!(recommended.contains(&"1".to_string()),
        "Task_is_recommended must include Task 1 (it's parallelizable); \
         got recommended={:?}, parallelizable={:?}",
         recommended, parallelizable);
}

/// Regression for the actual apps/tasks rule shape: a unary-consequent
/// rule with TWO positive antecedents — one binary FT, one unary FT.
/// The single-antecedent case is covered by
/// `unary_ft_in_rule_body_as_positive_antecedent_resolves_and_fires`,
/// but the multi-antecedent existence-check path
/// (`compile_explicit_derivation` `_` arm) wasn't exercised against
/// unary-FT antecedents until #866 surfaced it.
#[test]
fn unary_consequent_with_two_positive_antecedents_fires_per_subject() {
    let src = "\
        Task(.id) is an entity type.\n\
        Task Status is a value type.\n\
        Task Priority is a value type.\n\
        Task has Task Status.\n\
        Task has Task Priority.\n\
        Task is parallelizable.\n\
        Task is recommended.\n\
        Task '1' has Task Status 'pending'.\n\
        Task '1' has Task Priority 'p0'.\n\
        * Task is parallelizable iff Task has Task Status 'pending'.\n\
        * Task is recommended iff Task has Task Priority 'p0' and Task is parallelizable.\n\
    ";
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src)
        .expect("parse must succeed");

    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    let derivation_refs_owned: Vec<(String, ast::Func)> = ast::cells_iter(&d)
        .into_iter()
        .filter(|(n, _)| n.starts_with("derivation:rule_"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();
    let derivation_refs: Vec<(&str, &ast::Func)> = derivation_refs_owned.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _derived) = arest::evaluate::forward_chain_defs_state(
        &derivation_refs, &d);

    let task_ids_in_cell = |cell_name: &str| -> Vec<String> {
        let cell = ast::fetch_or_phi(cell_name, &new_d);
        cell.as_seq()
            .map(|s| s.iter()
                .filter_map(|f| f.as_seq().and_then(|pairs| pairs.iter().find_map(|p| {
                    let kv = p.as_seq()?;
                    let role = kv.first()?.as_atom()?;
                    if role == "Task" { kv.get(1)?.as_atom().map(String::from) } else { None }
                })))
                .collect())
            .unwrap_or_default()
    };

    let parallelizable = task_ids_in_cell("Task_is_parallelizable");
    let recommended = task_ids_in_cell("Task_is_recommended");
    assert!(parallelizable.contains(&"1".to_string()),
        "Task_is_parallelizable must include Task 1; got {:?}", parallelizable);
    assert!(recommended.contains(&"1".to_string()),
        "Task_is_recommended must include Task 1 (it's p0 and parallelizable); \
         got recommended={:?}, parallelizable={:?}",
         recommended, parallelizable);
}

/// Three positive antecedents — the apps/tasks `recommended` rule
/// shape exactly: `Task is recommended iff Task has Task Readiness
/// 'ready' and Task has Task Priority 'p0' and Task is parallelizable`.
/// All antecedents are positive so this routes through the
/// `derivation:rule_*` (stratum-1) bucket and the same forward-chain
/// run that materializes `Task_is_parallelizable` reaches a fixpoint
/// firing `Task_is_recommended` after `Task_is_parallelizable` lands.
/// The interleaved-stratum failure (positive consequent depends on a
/// stratum-2 negation-guarded consequent) is covered by
/// `unary_consequent_depending_on_stratum2_negation_guarded_unary_FT`.
#[test]
fn unary_consequent_with_three_positive_antecedents_fires_per_subject() {
    let src = "\
        Task(.id) is an entity type.\n\
        Task Readiness is a value type.\n\
        Task Priority is a value type.\n\
        Task Status is a value type.\n\
        Task has Task Readiness.\n\
        Task has Task Priority.\n\
        Task has Task Status.\n\
        Task is parallelizable.\n\
        Task is recommended.\n\
        Task '1' has Task Status 'pending'.\n\
        Task '1' has Task Readiness 'ready'.\n\
        Task '1' has Task Priority 'p0'.\n\
        * Task is parallelizable iff Task has Task Status 'pending'.\n\
        * Task is recommended iff Task has Task Readiness 'ready' and Task has Task Priority 'p0' and Task is parallelizable.\n\
    ";
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src)
        .expect("parse must succeed");

    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    let derivation_refs_owned: Vec<(String, ast::Func)> = ast::cells_iter(&d)
        .into_iter()
        .filter(|(n, _)| n.starts_with("derivation:rule_"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();
    let derivation_refs: Vec<(&str, &ast::Func)> = derivation_refs_owned.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _derived) = arest::evaluate::forward_chain_defs_state(
        &derivation_refs, &d);

    let task_ids_in_cell = |cell_name: &str| -> Vec<String> {
        let cell = ast::fetch_or_phi(cell_name, &new_d);
        cell.as_seq()
            .map(|s| s.iter()
                .filter_map(|f| f.as_seq().and_then(|pairs| pairs.iter().find_map(|p| {
                    let kv = p.as_seq()?;
                    let role = kv.first()?.as_atom()?;
                    if role == "Task" { kv.get(1)?.as_atom().map(String::from) } else { None }
                })))
                .collect())
            .unwrap_or_default()
    };

    let parallelizable = task_ids_in_cell("Task_is_parallelizable");
    let recommended = task_ids_in_cell("Task_is_recommended");
    assert!(parallelizable.contains(&"1".to_string()),
        "Task_is_parallelizable must include Task 1; got {:?}", parallelizable);
    assert!(recommended.contains(&"1".to_string()),
        "Task_is_recommended must include Task 1 \
         (it's ready+p0+parallelizable); got recommended={:?}, \
         parallelizable={:?}", recommended, parallelizable);
}

/// THE pin for the apps/tasks #866 e2e failure: a positive-only rule
/// (`Task is recommended iff … and Task is parallelizable`) reads a
/// unary FT cell whose producing rule is itself stratified into the
/// negation-guarded stratum (`Task is parallelizable iff … and Task is
/// not file-conflicting`). The CLI's 2-stratum forward chain
/// (cli/entry.rs):
///   stratum 1: positive `derivation:rule_*` to fixpoint
///   stratum 2: negation-guarded `derivation_strat2:rule_*` to fixpoint
/// fires `recommended` in stratum 1 against an EMPTY
/// `Task_is_parallelizable` cell, then populates `parallelizable` in
/// stratum 2 — but never re-runs the now-firing `recommended` rule.
/// The `Task_is_recommended` cell stays empty.
///
/// Spec fix: stratum 2 must include both the negation-guarded rules
/// AND any positive rule whose antecedent reads a cell stratum 1
/// dropped/produced via a strat2 producer — equivalently, after
/// stratum 2 fires the engine must run another positive-fixpoint pass
/// to converge on transitive consequences. The cleanest correctness
/// fix is to re-run the positive rules after stratum 2 (one more
/// iterated pass — monotonic, so it terminates).
#[test]
fn unary_consequent_depending_on_stratum2_negation_guarded_unary_ft() {
    let src = "\
        Task(.id) is an entity type.\n\
        Task Status is a value type.\n\
        Task has Task Status.\n\
        Task is parallelizable.\n\
        Task is file-conflicting.\n\
        Task is recommended.\n\
        Task '1' has Task Status 'pending'.\n\
        * Task is parallelizable iff Task has Task Status 'pending' and Task is not file-conflicting.\n\
        * Task is recommended iff Task is parallelizable.\n\
    ";
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src)
        .expect("parse must succeed");
    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    // Mirror cli/entry.rs's 2-stratum joint-fixpoint chain.
    let collect_derivs = |prefix: &str, state: &Object| -> Vec<(String, ast::Func)> {
        ast::cells_iter(state).into_iter()
            .filter(|(n, _)| n.starts_with(prefix))
            .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, state)))
            .collect()
    };
    let stratum1 = collect_derivs("derivation:rule_", &d);
    let stratum2 = collect_derivs("derivation_strat2:rule_", &d);
    let refs1: Vec<(&str, &ast::Func)> = stratum1.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let refs2: Vec<(&str, &ast::Func)> = stratum2.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _derived) = arest::evaluate::forward_chain_stratified(
        &refs1, &refs2, &d, 100);

    let task_ids_in_cell = |cell_name: &str| -> Vec<String> {
        let cell = ast::fetch_or_phi(cell_name, &new_d);
        cell.as_seq()
            .map(|s| s.iter()
                .filter_map(|f| f.as_seq().and_then(|pairs| pairs.iter().find_map(|p| {
                    let kv = p.as_seq()?;
                    let role = kv.first()?.as_atom()?;
                    if role == "Task" { kv.get(1)?.as_atom().map(String::from) } else { None }
                })))
                .collect())
            .unwrap_or_default()
    };
    let parallelizable = task_ids_in_cell("Task_is_parallelizable");
    let recommended = task_ids_in_cell("Task_is_recommended");
    assert!(parallelizable.contains(&"1".to_string()),
        "Task_is_parallelizable must contain Task 1 (pending and not \
         file-conflicting); got {:?}", parallelizable);
    assert!(recommended.contains(&"1".to_string()),
        "Task_is_recommended must contain Task 1 (it's parallelizable); \
         got recommended={:?}, parallelizable={:?}",
         recommended, parallelizable);
}

/// Subscript-aware 3-antecedent join (apps/tasks file-conflicting
/// rule shape): `Task2 is file-conflicting iff Task1 has Task Status
/// 'in_progress' and Task1 touches Source File and Task2 touches
/// Source File`. Requires the engine to chain joins across the
/// shared subscript variables Task1 (a0 ∩ a1), Source File (a1 ∩ a2,
/// no subscript = same variable), and bind the consequent's Task2
/// from a2 (subscript "Task2" → a2 role 0).
#[test]
fn unary_consequent_subscript_driven_three_way_join_emits_per_matching_other_subject() {
    let src = "\
        Task(.id) is an entity type.\n\
        Source File(.path) is an entity type.\n\
        Task Status is a value type.\n\
        Task has Task Status.\n\
        Task is file-conflicting.\n\
        Task touches Source File.\n\
        Task '1' has Task Status 'in_progress'.\n\
        Task '1' touches Source File 'src/foo.rs'.\n\
        Task '2' touches Source File 'src/foo.rs'.\n\
        Task '3' touches Source File 'src/bar.rs'.\n\
        * Task2 is file-conflicting iff Task1 has Task Status 'in_progress' and Task1 touches Source File and Task2 touches Source File.\n\
    ";
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src)
        .expect("parse must succeed");
    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    let stratum1: Vec<(String, ast::Func)> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:rule_"))
        .map(|(n, c)| (n.to_string(), ast::metacompose(c, &d)))
        .collect();
    let refs1: Vec<(&str, &ast::Func)> = stratum1.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let stratum2: Vec<(String, ast::Func)> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation_strat2:rule_"))
        .map(|(n, c)| (n.to_string(), ast::metacompose(c, &d)))
        .collect();
    let refs2: Vec<(&str, &ast::Func)> = stratum2.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _derived) = arest::evaluate::forward_chain_stratified(
        &refs1, &refs2, &d, 100);

    let task_ids_in_cell = |cell_name: &str| -> Vec<String> {
        let cell = ast::fetch_or_phi(cell_name, &new_d);
        cell.as_seq()
            .map(|s| s.iter()
                .filter_map(|f| f.as_seq().and_then(|pairs| pairs.iter().find_map(|p| {
                    let kv = p.as_seq()?;
                    let role = kv.first()?.as_atom()?;
                    if role == "Task" { kv.get(1)?.as_atom().map(String::from) } else { None }
                })))
                .collect())
            .unwrap_or_default()
    };
    let conflicting = task_ids_in_cell("Task_is_file-conflicting");
    // Task '2' touches src/foo.rs which Task '1' (in_progress) also
    // touches → Task '2' is file-conflicting. Also Task '1' itself
    // touches src/foo.rs and is in_progress, so '1' also conflicts
    // with itself by the rule (the rule doesn't say Task1 ≠ Task2;
    // semantically every in-progress Task is "blocking itself" via
    // self-overlap on its own file). Task '3' touches a different
    // file so it must NOT be flagged.
    assert!(conflicting.contains(&"2".to_string()),
        "Task '2' must be file-conflicting (touches src/foo.rs which \
         in_progress Task '1' also touches); got {:?}", conflicting);
    assert!(!conflicting.contains(&"3".to_string()),
        "Task '3' must NOT be file-conflicting (it touches src/bar.rs \
         which no in_progress task touches); got {:?}", conflicting);
}

/// Larger fixture mimicking apps/tasks shape: multiple in_progress
/// tasks, multiple Source File touches, ~4 candidate Task2 values
/// against one in_progress Task1. Verifies the subscript-aware
/// 3-way join scales to the production fixture and emits every
/// matching Task2.
#[test]
fn unary_consequent_subscript_join_scales_to_apps_tasks_size_fixture() {
    let src = "\
        Task(.id) is an entity type.\n\
        Source File(.path) is an entity type.\n\
        Task Status is a value type.\n\
        Task has Task Status.\n\
        Task is file-conflicting.\n\
        Task touches Source File.\n\
        Task '797' has Task Status 'in_progress'.\n\
        Task '802' has Task Status 'pending'.\n\
        Task '815' has Task Status 'pending'.\n\
        Task '821' has Task Status 'pending'.\n\
        Task '900' has Task Status 'pending'.\n\
        Task '797' touches Source File 'crates/arest/src/ast.rs'.\n\
        Task '802' touches Source File 'crates/arest/src/ast.rs'.\n\
        Task '815' touches Source File 'crates/arest/src/ast.rs'.\n\
        Task '821' touches Source File 'crates/arest/src/ast.rs'.\n\
        Task '900' touches Source File 'crates/arest/src/other.rs'.\n\
        * Task2 is file-conflicting iff Task1 has Task Status 'in_progress' and Task1 touches Source File and Task2 touches Source File.\n\
    ";
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src)
        .expect("parse must succeed");
    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);
    let stratum1: Vec<(String, ast::Func)> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:rule_"))
        .map(|(n, c)| (n.to_string(), ast::metacompose(c, &d)))
        .collect();
    let refs1: Vec<(&str, &ast::Func)> = stratum1.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let stratum2: Vec<(String, ast::Func)> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation_strat2:rule_"))
        .map(|(n, c)| (n.to_string(), ast::metacompose(c, &d)))
        .collect();
    let refs2: Vec<(&str, &ast::Func)> = stratum2.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _derived) = arest::evaluate::forward_chain_stratified(
        &refs1, &refs2, &d, 100);

    let task_ids_in_cell = |cell_name: &str| -> Vec<String> {
        let cell = ast::fetch_or_phi(cell_name, &new_d);
        cell.as_seq()
            .map(|s| s.iter()
                .filter_map(|f| f.as_seq().and_then(|pairs| pairs.iter().find_map(|p| {
                    let kv = p.as_seq()?;
                    let role = kv.first()?.as_atom()?;
                    if role == "Task" { kv.get(1)?.as_atom().map(String::from) } else { None }
                })))
                .collect())
            .unwrap_or_default()
    };
    let conflicting = task_ids_in_cell("Task_is_file-conflicting");
    for expected in ["797", "802", "815", "821"] {
        assert!(conflicting.contains(&expected.to_string()),
            "Task '{}' must be file-conflicting (touches ast.rs which \
             in_progress Task '797' also touches); got {:?}",
             expected, conflicting);
    }
    assert!(!conflicting.contains(&"900".to_string()),
        "Task '900' must NOT be file-conflicting (touches other.rs \
         which no in_progress task touches); got {:?}", conflicting);
}

/// Apps/tasks production-shape fixture: same source dir layout the
/// CLI compiles, with a few in_progress tasks plus other-tasks
/// touching the same files. Mirrors the actual touches/status mix
/// the production tasks.db exhibits.
#[test]
fn apps_tasks_file_conflicting_materializes_with_in_progress_overlap() {
    let src = "\
        Task(.id) is an entity type.\n\
        Source File(.path) is an entity type.\n\
        Task Status is a value type.\n\
        Task Readiness is a value type.\n\
        Task Priority is a value type.\n\
        Task has Task Status.\n\
        Task has Task Readiness.\n\
        Task has Task Priority.\n\
        Task is parallelizable.\n\
        Task is file-conflicting.\n\
        Task is recommended.\n\
        Task touches Source File.\n\
        Task '797' has Task Status 'in_progress'.\n\
        Task '802' has Task Status 'pending'.\n\
        Task '815' has Task Status 'pending'.\n\
        Task '802' has Task Readiness 'ready'.\n\
        Task '815' has Task Readiness 'ready'.\n\
        Task '802' has Task Priority 'p0'.\n\
        Task '815' has Task Priority 'p0'.\n\
        Task '797' touches Source File 'crates/arest/src/ast.rs'.\n\
        Task '802' touches Source File 'crates/arest/src/ast.rs'.\n\
        Task '815' touches Source File 'crates/arest/src/ast.rs'.\n\
        * Task2 is file-conflicting iff Task1 has Task Status 'in_progress' and Task1 touches Source File and Task2 touches Source File.\n\
        * Task is parallelizable iff Task has Task Readiness 'ready' and Task is not file-conflicting.\n\
        * Task is recommended iff Task has Task Readiness 'ready' and Task has Task Priority 'p0' and Task is parallelizable.\n\
    ";
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src)
        .expect("parse must succeed");
    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);
    let stratum1: Vec<(String, ast::Func)> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:rule_"))
        .map(|(n, c)| (n.to_string(), ast::metacompose(c, &d)))
        .collect();
    let refs1: Vec<(&str, &ast::Func)> = stratum1.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let stratum2: Vec<(String, ast::Func)> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation_strat2:rule_"))
        .map(|(n, c)| (n.to_string(), ast::metacompose(c, &d)))
        .collect();
    let refs2: Vec<(&str, &ast::Func)> = stratum2.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _derived) = arest::evaluate::forward_chain_stratified(
        &refs1, &refs2, &d, 100);

    let task_ids_in_cell = |cell_name: &str| -> Vec<String> {
        let cell = ast::fetch_or_phi(cell_name, &new_d);
        cell.as_seq()
            .map(|s| s.iter()
                .filter_map(|f| f.as_seq().and_then(|pairs| pairs.iter().find_map(|p| {
                    let kv = p.as_seq()?;
                    let role = kv.first()?.as_atom()?;
                    if role == "Task" { kv.get(1)?.as_atom().map(String::from) } else { None }
                })))
                .collect())
            .unwrap_or_default()
    };
    let conflicting = task_ids_in_cell("Task_is_file-conflicting");
    let parallelizable = task_ids_in_cell("Task_is_parallelizable");
    let recommended = task_ids_in_cell("Task_is_recommended");
    assert!(conflicting.contains(&"802".to_string()),
        "Task '802' must be file-conflicting; got {:?}", conflicting);
    assert!(conflicting.contains(&"815".to_string()),
        "Task '815' must be file-conflicting; got {:?}", conflicting);
    // Tasks '802' and '815' are file-conflicting (in_progress 797 also
    // touches ast.rs), so they must NOT be parallelizable nor
    // recommended.
    assert!(!parallelizable.contains(&"802".to_string())
        && !parallelizable.contains(&"815".to_string()),
        "Tasks '802'/'815' must NOT be parallelizable (file-conflicting); \
         got parallelizable={:?}", parallelizable);
    // sanity: avoid unused warnings
    let _ = (parallelizable, recommended);
}

/// Live apps/tasks DB acceptance: load the actual SQLite cell store
/// alongside the app.md readings, run compile + 2-stratum joint
/// fixpoint, and assert all three Task_is_* cells materialise with
/// non-empty content. Skips gracefully when the DB isn't on disk so
/// CI runners without the apps repo cloned don't fail.
#[cfg(feature = "local")]
#[test]
fn apps_tasks_live_db_materializes_all_three_unary_consequents() {
    use std::path::PathBuf;
    let db_path: PathBuf = std::env::var("AREST_TASKS_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Users\lippe\Repos\apps\tasks\tasks.db"));
    if !db_path.exists() {
        eprintln!("[#866-acceptance] skip: {} not present", db_path.display());
        return;
    }
    // Load population state from the live DB (cells only, no defs).
    let conn = rusqlite::Connection::open(&db_path).expect("open tasks.db");
    let mut stmt = conn.prepare("SELECT name, contents FROM cells")
        .expect("prepare");
    let rows: Vec<(String, String)> = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    }).expect("query_map").filter_map(|r| r.ok()).collect();
    drop(stmt);
    let mut prior_population = Object::Map(hashbrown::HashMap::new());
    for (name, contents) in rows {
        if name.contains(':') { continue }  // skip defs
        let obj = Object::parse(&contents);
        prior_population = ast::store(&name, obj, &prior_population);
    }
    // Drop derived cells from the prior population (LFP semantics —
    // mirrors cli/entry.rs's drop step).
    for derived in ["Task_is_parallelizable", "Task_is_recommended",
                     "Task_is_file-conflicting"] {
        prior_population = ast::store(derived, Object::phi(), &prior_population);
    }

    // Parse the apps/tasks readings (chained with metamodel
    // bootstrap) on top of the prior population. Mirrors the
    // cor:closure path in cli/entry.rs.
    let readings_md = std::fs::read_to_string(
        r"C:\Users\lippe\Repos\apps\tasks\readings\app.md")
        .unwrap_or_else(|e| panic!("read app.md: {}", e));
    arest::parse_forml2::set_bootstrap_mode(true);
    let all_readings: Vec<(&str, &str)> = arest::metamodel_readings().into_iter()
        .map(|r| (r.0, r.1))
        .chain(std::iter::once(("apps/tasks/app.md", readings_md.as_str())))
        .collect();
    let state = all_readings.iter().fold(prior_population, |merged, (name, text)| {
        let this = arest::parse_forml2::parse_to_state_from(text, &merged)
            .unwrap_or_else(|e| panic!("parse {}: {}", name, e));
        ast::merge_states(&merged, &this)
    });
    arest::parse_forml2::set_bootstrap_mode(false);

    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    let stratum1: Vec<(String, ast::Func)> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:rule_"))
        .map(|(n, c)| (n.to_string(), ast::metacompose(c, &d)))
        .collect();
    let refs1: Vec<(&str, &ast::Func)> = stratum1.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let stratum2: Vec<(String, ast::Func)> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation_strat2:rule_"))
        .map(|(n, c)| (n.to_string(), ast::metacompose(c, &d)))
        .collect();
    let refs2: Vec<(&str, &ast::Func)> = stratum2.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, derived) = arest::evaluate::forward_chain_stratified(
        &refs1, &refs2, &d, 100);
    eprintln!("[#866-acceptance] derived {} facts", derived.len());

    let task_ids_in_cell = |cell_name: &str| -> Vec<String> {
        let cell = ast::fetch_or_phi(cell_name, &new_d);
        cell.as_seq()
            .map(|s| s.iter()
                .filter_map(|f| f.as_seq().and_then(|pairs| pairs.iter().find_map(|p| {
                    let kv = p.as_seq()?;
                    let role = kv.first()?.as_atom()?;
                    if role == "Task" { kv.get(1)?.as_atom().map(String::from) } else { None }
                })))
                .collect())
            .unwrap_or_default()
    };
    for expected in [
        "Task_is_parallelizable",
        "Task_is_recommended",
        "Task_is_file-conflicting",
    ] {
        let ids = task_ids_in_cell(expected);
        assert!(!ids.is_empty(),
            "{} cell must be non-empty after forward-chain over the \
             apps/tasks population (in_progress + touches overlap exists \
             in tasks.db); got {:?}", expected, ids);
        eprintln!("[#866-acceptance] {} : {} rows ({:?})",
            expected, ids.len(),
            ids.iter().take(5).collect::<Vec<_>>());
    }
}
