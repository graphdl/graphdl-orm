// crates/arest/tests/derivation_rule_replace_on_recompile_e2e.rs
//
// End-to-end pin (#913): editing a reading's derivation rule body and
// recompiling MUST produce facts from ONLY the new rule, not the union
// of the previous rule and the new one.
//
// Symptom on main (pre-fix): `cor:closure` preserves the prior compile's
// `DerivationRule` cell across recompile. A new rule with different text
// hashes to a new `id`, so concat_dedup keeps BOTH entries. The
// subsequent `compile_to_defs_state` emits `derivation:rule_<hashA>`
// AND `derivation:rule_<hashB>` def cells, and the forward chain fires
// both — the consequent cell ends up holding the UNION of A's matches
// and B's matches.
//
// Spec (readings-as-source-of-truth): rule text lives in readings. The
// compile is a function of the current readings, not the historical
// sequence of rule edits. After recompile, the consequent cell must
// reflect ONLY the rule body present in the readings.
//
// Mirrors the #836 fix's "drop derived cells before forward chain" but
// targets the rule REGISTRY itself (`DerivationRule` meta cell) — the
// rule registry is regenerated from readings on every compile, so
// preserving stale rule registrations across compile breaks the
// readings-as-source-of-truth invariant for the rule set.

use arest::ast::{self, Object};

/// Helper: extract Task ids from a unary FT consequent cell whose
/// entries are shaped `<<Task, id>>`.
fn task_ids_in_cell(cell_name: &str, state: &Object) -> Vec<String> {
    let cell = ast::fetch_or_phi(cell_name, state);
    cell.as_seq()
        .map(|s| s.iter()
            .filter_map(|f| f.as_seq().and_then(|pairs| pairs.iter().find_map(|p| {
                let kv = p.as_seq()?;
                let role = kv.first()?.as_atom()?;
                if role == "Task" { kv.get(1)?.as_atom().map(String::from) } else { None }
            })))
            .collect())
        .unwrap_or_default()
}

/// Helper: parse readings, compile defs, and forward chain. Mirrors the
/// CLI's compile path (cli/entry.rs:602-801) for a single-stratum case.
///
/// #913: mirrors the `cli/entry.rs` filter that strips
/// `READINGS_DERIVED_META_CELLS` from the prior state before merge so
/// the rule registry is rebuilt from current readings each compile.
fn compile_and_chain(src: &str, prior: &Object) -> Object {
    let prior_stripped = ast::drop_readings_derived_meta_cells(prior);
    let parsed = arest::parse_forml2::parse_to_state_from(src, &prior_stripped)
        .expect("parse must succeed");
    let merged = ast::merge_states(&prior_stripped, &parsed);
    let defs = arest::compile::compile_to_defs_state(&merged);
    let d = ast::defs_to_state(&defs, &merged);

    // #836 — drop derived consequent cells before forward chain so the
    // LFP recomputes from primary facts. Mirrors cli/entry.rs.
    let derived_cells: hashbrown::HashSet<String> = {
        let mut out: hashbrown::HashSet<String> = hashbrown::HashSet::new();
        let drule_cell = ast::fetch_or_phi("DerivationRule", &d);
        if let Some(facts) = drule_cell.as_seq() {
            for fact in facts.iter() {
                let Some(encoded) = ast::binding(fact, "consequentFactTypeId") else { continue };
                let cell_name = arest::types::ConsequentCellSource::decode(encoded)
                    .literal_id().to_string();
                if !cell_name.is_empty() { out.insert(cell_name); }
            }
        }
        out
    };
    let d = if derived_cells.is_empty() { d } else {
        let mut new_map: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        for (name, contents) in ast::cells_iter(&d).into_iter() {
            if derived_cells.contains(name) {
                new_map.insert(name.to_string(), Object::phi());
            } else {
                new_map.insert(name.to_string(), contents.clone());
            }
        }
        Object::Map(new_map)
    };

    let derivation_refs_owned: Vec<(String, ast::Func)> = ast::cells_iter(&d)
        .into_iter()
        .filter(|(n, _)| n.starts_with("derivation:rule_"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();
    let derivation_refs: Vec<(&str, &ast::Func)> = derivation_refs_owned.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _derived) = arest::evaluate::forward_chain_defs_state(
        &derivation_refs, &d);
    new_d
}

/// THE pin for #913: edit a reading's rule body, recompile, and the
/// consequent cell must reflect ONLY the new rule body.
///
/// Fixture:
///   Two tasks: Task 1 (status='pending'), Task 2 (status='in_progress')
///   Rule A: `Task is recommended iff Task has Task Status 'pending'`
///   Rule B: `Task is recommended iff Task has Task Status 'in_progress'`
///
/// Compile A: `Task_is_recommended` = [Task 1]
/// Compile B against the post-A state: `Task_is_recommended` MUST = [Task 2]
///
/// If the bug fires, the post-B `Task_is_recommended` contains [Task 1, Task 2]
/// because both rules' def cells are emitted and both fire.
#[test]
fn editing_rule_body_drops_stale_rule_on_recompile() {
    let src_a = "\
        Task(.id) is an entity type.\n\
        Task Status is a value type.\n\
        Task has Task Status.\n\
        Task is recommended.\n\
        Task '1' has Task Status 'pending'.\n\
        Task '2' has Task Status 'in_progress'.\n\
        * Task is recommended iff Task has Task Status 'pending'.\n\
    ";
    let src_b = "\
        Task(.id) is an entity type.\n\
        Task Status is a value type.\n\
        Task has Task Status.\n\
        Task is recommended.\n\
        Task '1' has Task Status 'pending'.\n\
        Task '2' has Task Status 'in_progress'.\n\
        * Task is recommended iff Task has Task Status 'in_progress'.\n\
    ";

    // First compile (rule A): only Task 1 (pending) is recommended.
    let d_after_a = compile_and_chain(src_a, &Object::phi());
    let recommended_a = task_ids_in_cell("Task_is_recommended", &d_after_a);
    assert!(recommended_a.contains(&"1".to_string()),
        "after compile-A Task_is_recommended must contain Task 1 (pending); got {:?}",
        recommended_a);
    assert!(!recommended_a.contains(&"2".to_string()),
        "after compile-A Task_is_recommended must NOT contain Task 2 (in_progress); \
         got {:?}", recommended_a);

    // Strip the previous compile's derived cells and def cells from the
    // recompile seed — this is the same shape cli/entry.rs feeds into the
    // second compile loop (prior_population in cli/entry.rs filters out
    // `:` def cells). The DerivationRule cell IS preserved on main
    // because it doesn't carry a `:`, and that's the bug.
    let prior_for_b: Object = {
        let cells: hashbrown::HashMap<String, Object> = ast::cells_iter(&d_after_a)
            .into_iter()
            .filter(|(name, _)| !name.contains(':'))
            .map(|(name, contents)| (name.to_string(), contents.clone()))
            .collect();
        Object::Map(cells)
    };

    // Second compile (rule B): only Task 2 (in_progress) must be
    // recommended. Task 1 must NOT appear — the prior rule A is gone
    // from the readings.
    let d_after_b = compile_and_chain(src_b, &prior_for_b);
    let recommended_b = task_ids_in_cell("Task_is_recommended", &d_after_b);
    assert!(recommended_b.contains(&"2".to_string()),
        "after compile-B Task_is_recommended must contain Task 2 (in_progress, \
         the only match for rule B); got {:?}", recommended_b);
    assert!(!recommended_b.contains(&"1".to_string()),
        "after compile-B Task_is_recommended must NOT contain Task 1 \
         (rule A was replaced by rule B in the readings — readings are \
         the source of truth for the rule set); got {:?}. \
         If Task 1 is present, the stale rule from compile-A is still \
         firing (#913).", recommended_b);

    // Belt-and-suspenders: `DerivationRule` cell on the post-B state
    // must contain exactly ONE rule (rule B), not two.
    let dr_cell = ast::fetch_or_phi("DerivationRule", &d_after_b);
    let user_rule_count = dr_cell.as_seq()
        .map(|s| s.iter()
            .filter(|f| {
                // Filter out grammar / metamodel rules (those whose text
                // begins with a Title-cased grammar concept). User rule
                // text starts with `Task is`.
                ast::binding(f, "text")
                    .map(|t| t.starts_with("Task is recommended"))
                    .unwrap_or(false)
            })
            .count())
        .unwrap_or(0);
    assert_eq!(user_rule_count, 1,
        "DerivationRule cell must contain exactly ONE `Task is recommended` \
         rule after compile-B (the current rule body); got {} entries. \
         The stale rule A from compile-A must be dropped on recompile.",
        user_rule_count);
}
