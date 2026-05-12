// crates/arest/tests/cross_antecedent_value_comparison_e2e.rs
//
// End-to-end pin (#914): FORML 2 readings whose body uses a
// `<Noun1>'s <Role> is less than <Noun2>'s <Role>` cross-antecedent
// value comparison must
//   (a) parse so the resulting DerivationRuleDef carries a non-empty
//       `antecedent_role_comparisons` vec recording the (lhs_index,
//       lhs_role, "<", rhs_index, rhs_role) tuple, AND
//   (b) compile + forward-chain such that the `Task_is_preceded`
//       cell contains ONLY the (Task2 with strictly-higher id) facts
//       whose Task ID is strictly greater than some Task1 in the
//       same Source File.
//
// The IR (`AntecedentRoleComparison`, commit a0cfb318) and compile-
// side post-join Filter (commit e639bd43) are already in. This pin
// covers the parser-side recognition of the natural-language form
// `Task1's Task ID is less than Task2's Task ID`, reusing the
// existing `WordComparatorTable` vocabulary (no parallel keyword
// table). Without the parser-side glue the comparison clause is
// silently dropped and the join fans out across every (Task1, Task2)
// pair sharing a Source File.

use arest::ast::{self, Object};

#[test]
fn cross_antecedent_role_comparison_via_word_comparator_parses_and_filters_forward_chain() {
    // Fixture: three Tasks all touching the same Source File. Task ids
    // 1 / 2 / 3 (lexicographic order matches numeric here). The rule
    // says Task2 is preceded by Task1 iff Task1's Task ID is strictly
    // LESS than Task2's Task ID AND they share a Source File. With
    // three tasks sharing one file the resulting preceded set must be
    //   { Task '2' (preceded by '1'), Task '3' (preceded by '1' or '2') }.
    // Without the comparison filter every pair would fire; the strict
    // `<` comparator cuts that down to the directional half.
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
        * Task2 is preceded iff Task1 has Task ID and Task2 has Task ID and Task1's Task ID is less than Task2's Task ID and Task1 touches Source File and Task2 touches Source File.\n\
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

    // The CRITICAL parser-side pin: parse + resolve must produce a
    // rule whose IR carries a non-empty `antecedent_role_comparisons`
    // vec with `(_, "Task ID", "<", _, "Task ID")`. Without it the
    // word-comparator clause was silently dropped.
    let idx = arest::compile::cell_index_from_state(&state);
    let has_comparison = idx.derivation_rules.iter().any(|r|
        !r.antecedent_role_comparisons.is_empty()
            && r.antecedent_role_comparisons.iter().any(|c|
                c.lhs_role == "Task ID" && c.rhs_role == "Task ID" && c.op == "<"
            )
    );
    assert!(has_comparison,
        "DerivationRule.antecedent_role_comparisons must carry a \
         (Task ID, <, Task ID) entry after parse + resolve; rules = {:?}",
        idx.derivation_rules.iter().map(|r|
            (r.id.clone(), r.antecedent_role_comparisons.clone())
        ).collect::<Vec<_>>());

    // Forward-chain assertion: the `Task_is_preceded` cell must
    // contain Task '2' and Task '3' (each has at least one Task1 with
    // a strictly-lower Task ID), and MUST NOT contain Task '1' (no
    // Task1 with a strictly-lower id exists).
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
