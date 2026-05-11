// crates/arest/tests/cross_antecedent_comparison_e2e.rs
//
// End-to-end pin (#907): FORML 2 readings of the form
//   * Task2 is preceded iff Task1 has lower Task ID than Task2
//     and Task1 touches Source File and Task2 touches Source File.
// must (a) parse so that the resulting DerivationRuleDef has a
// non-empty `antecedent_role_comparisons` vec, and (b) compile +
// forward-chain such that the `Task_is_preceded` cell carries ONLY
// the (Task2 with higher id) facts whose Task ID is strictly greater
// than some Task1 in the same Source File.
//
// The IR field (`DerivationRuleDef.antecedent_role_comparisons`)
// landed in a0cfb318 and the compile-side wiring (a post-join Filter
// predicate built from each AntecedentRoleComparison) landed in
// e639bd43. Without the parser-side recognition of
// "<Noun1> has lower <Role> than <Noun2>" this field stays empty and
// the rule fans out across every (Task1, Task2) pair sharing a
// Source File. The test pins the parser-resolved rule + the
// forward-chain output so #906 (apps/tasks parallelization) can land
// as a one-paragraph readings change rather than a parser patch.

use arest::ast::{self, Object};

#[test]
fn cross_antecedent_role_comparison_parses_to_ir_and_filters_forward_chain() {
    // Fixture: 3 tasks all touching the same Source File. Task ids
    // 1 / 2 / 3. The rule says Task2 is preceded by Task1 iff Task1's
    // Task ID is LOWER than Task2's Task ID AND they share a Source
    // File. With three tasks sharing one file the resulting pairs
    // (preceded set) must be:
    //   { Task '2' (preceded by '1'), Task '3' (preceded by '1' or '2') }.
    // Without the comparison filter every pair (1,2),(1,3),(2,3),
    // (2,1),(3,1),(3,2),(1,1),(2,2),(3,3) would fire. The presence of
    // the strict `<` comparator must cut that down to the directional
    // half.
    let src = "\
        Task(.id) is an entity type.\n\
        Source File(.path) is an entity type.\n\
        Task ID is a value type.\n\
        Task has Task ID.\n\
        Task is preceded.\n\
        Task touches Source File.\n\
        Task '1' has Task ID '1'.\n\
        Task '2' has Task ID '2'.\n\
        Task '3' has Task ID '3'.\n\
        Task '1' touches Source File 'src/foo.rs'.\n\
        Task '2' touches Source File 'src/foo.rs'.\n\
        Task '3' touches Source File 'src/foo.rs'.\n\
        * Task2 is preceded iff Task1 has lower Task ID than Task2 and Task1 touches Source File and Task2 touches Source File.\n\
    ";
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src)
        .expect("parse must succeed");

    // Sanity: rule's `consequentFactTypeId` resolves to `Task_is_preceded`.
    let dr_cell = ast::fetch_or_phi("DerivationRule", &state);
    let consequent_ids: Vec<String> = dr_cell.as_seq()
        .map(|s| s.iter()
            .filter_map(|f| ast::binding(f, "consequentFactTypeId").map(String::from))
            .collect())
        .unwrap_or_default();
    assert!(consequent_ids.iter().any(|id| id == "Task_is_preceded"),
        "DerivationRule.consequentFactTypeId must resolve to `Task_is_preceded`; got {:?}",
        consequent_ids);

    // The CRITICAL assertion this pin defends: parse + resolve must
    // produce a rule whose IR carries a non-empty
    // `antecedent_role_comparisons` vec recording the
    // (lhs_antecedent_index, "Task ID", "<", rhs_antecedent_index,
    // "Task ID") triple. Without it the comparison clause is silently
    // dropped and the join fans out unfiltered.
    let idx = arest::compile::cell_index_from_state(&state);
    let has_comparison = idx.derivation_rules.iter().any(|r|
        !r.antecedent_role_comparisons.is_empty()
            && r.antecedent_role_comparisons.iter().any(|c|
                c.lhs_role == "Task ID" && c.rhs_role == "Task ID" && c.op == "<"
            )
    );
    assert!(has_comparison,
        "DerivationRule.antecedent_role_comparisons must carry a (Task ID, <, Task ID) \
         entry after parse + resolve; rules = {:?}",
        idx.derivation_rules.iter().map(|r|
            (r.id.clone(), r.antecedent_role_comparisons.clone())
        ).collect::<Vec<_>>());

    // Forward-chain assertion: the `Task_is_preceded` cell must
    // contain Task '2' and Task '3' (each has at least one Task1 with
    // a strictly-lower Task ID), and MUST NOT contain Task '1' (no
    // Task1 with a strictly-lower id exists in the population).
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

    let task_ids_in_cell = |cell_name: &str, state: &Object| -> Vec<String> {
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
    };
    let preceded = task_ids_in_cell("Task_is_preceded", &new_d);
    assert!(preceded.contains(&"2".to_string()),
        "Task_is_preceded must include Task '2' (Task '1' has lower id and shares file); \
         got {:?}", preceded);
    assert!(preceded.contains(&"3".to_string()),
        "Task_is_preceded must include Task '3' (Task '1' / '2' both have lower id and \
         share the file); got {:?}", preceded);
    assert!(!preceded.contains(&"1".to_string()),
        "Task_is_preceded must NOT include Task '1' (no task has a strictly-lower id); \
         got {:?}", preceded);
}
