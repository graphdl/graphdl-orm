// crates/arest/tests/valuetyped_join_key_projection_e2e.rs
//
// End-to-end pin: a derivation rule that JOINS two fact types on a
// VALUE-TYPED role must project the *other* role of the second
// antecedent into the consequent head.
//
// Repro (FORML 2):
//   Problem(.id) is an entity type.
//   Count(.id) is an entity type.
//   Shape is a value type.            <- the JOIN KEY is value-typed
//     The possible values of Shape are 'gather', 'relabel'.
//   Problem wins by Shape.            <- antecedent 0: (Problem, Shape)
//   Shape has confidence Count.       <- antecedent 1: (Shape, Count)
//   Problem ranks Shape at Count.     <- consequent:  (Problem, Shape, Count)
//   * Problem1 ranks Shape1 at Count1 iff
//       Problem1 wins by Shape1 and Shape1 has confidence Count1.
//
// The two antecedents JOIN on the value-typed role `Shape`. The
// consequent must carry all three roles. The bug: the join forms
// (Problem='p1', Shape='gather') but DROPS the `Count` role projected
// from the second antecedent, leaving Count UNBOUND (and a SQL
// `NOT NULL constraint failed: ...count_id` on projection).
//
// ROOT CAUSE (resolver, not compiler). The second antecedent clause
// `Shape1 has confidence Count1` resolves through `resolve_fact_type`
// in parse_forml2.rs, which uses `find_nouns` to extract the clause's
// role set. The bundled metamodel declares `Confidence is a value
// type` (readings/core/outcomes.md). `find_nouns` matches noun names
// case-INsensitively, so it greedily matches the word `confidence`
// inside the VERB phrase, inflating the role set from [Shape, Count]
// to [Shape, confidence, Count]. The catalog has no FT with that
// 3-noun role set, so the verb/role-set rho-lookup MISSES, the clause
// is dropped, and the rule collapses to a SINGLE antecedent — whose
// `compile_join_derivation` n==1 branch cannot project `Count`.
//
// This is therefore a *metamodel-context* bug: the SAME readings parsed
// in ISOLATION resolve both antecedents fine (no `Confidence` noun to
// collide with). The pin must reproduce the real engine environment,
// so it folds `metamodel_corpus()` ahead of the repro — exactly as the
// CLI / live MCP engine does.
//
// FIX: `resolve_fact_type` now tries an exact-reading match FIRST
// (normalise the clause's tokens — strip Halpin subscripts, lowercase —
// and compare to each declared FT's reading), mirroring the guard
// `resolve_consequent_strict` already carries for the head. An exact
// reading match is the most specific resolution and is immune to the
// noun-extraction inflation.
//
// Authority: Halpin §9.7 p383 — a conceptual join asserts the same
// instance plays both roles, so the joined role MUST propagate across
// the join; joining on a value type is legal (§13.3 p672, §5.4 p189).
// `Mapping ORM to Datalog` §5 p9 — the rule IS Ullman-safe (`Count1`
// occurs in a positive body literal), so this is a resolution/
// projection bug, not an unsafe rule.

use arest::ast;

#[test]
fn valuetyped_join_key_projects_other_role_of_second_antecedent() {
    let repro = "\
        Problem(.id) is an entity type.\n\
        Count(.id) is an entity type.\n\
        Shape is a value type.\n\
        The possible values of Shape are 'gather', 'relabel'.\n\
        Problem wins by Shape.\n\
        Shape has confidence Count.\n\
        Problem ranks Shape at Count.\n\
        * Problem1 ranks Shape1 at Count1 iff Problem1 wins by Shape1 and Shape1 has confidence Count1.\n\
        Problem 'p1' wins by Shape 'gather'.\n\
        Shape 'gather' has confidence Count '3'.\n\
    ";
    // Fold the bundled metamodel ahead of the repro, mirroring the live
    // CLI / MCP engine. This is what brings the colliding `Confidence`
    // value type into `noun_names` and triggers the bug. (Parsing the
    // repro alone does NOT reproduce it.)
    let combined = format!("{}\n\n{}", arest::metamodel_corpus(), repro);
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(&combined)
        .expect("parse must succeed");

    // The rule must resolve BOTH antecedents (the pre-fix bug dropped the
    // second one, `Shape_has_confidence_Count`). Assert directly on the
    // resolved IR so a regression points straight at the resolver.
    let idx = arest::compile::cell_index_from_state(&state);
    let rule = idx.derivation_rules.iter()
        .find(|r| r.text.contains("ranks") && r.text.contains("confidence"))
        .expect("the `Problem ranks Shape at Count` rule must be present");
    let ant_ids: Vec<String> = rule.antecedent_sources.iter()
        .map(|s| s.fact_type_id().to_string()).collect();
    assert!(ant_ids.iter().any(|id| id == "Problem_wins_by_Shape"),
        "rule must retain the `Problem_wins_by_Shape` antecedent; got {:?}", ant_ids);
    assert!(ant_ids.iter().any(|id| id == "Shape_has_confidence_Count"),
        "BUG (resolver): the value-typed-join second antecedent \
         `Shape_has_confidence_Count` was DROPPED (its verb word `confidence` \
         collided with the metamodel `Confidence` value type, inflating the \
         clause role set so the catalog lookup missed). antecedents = {:?}",
        ant_ids);

    // Forward-chain.
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

    // Read the `Problem_ranks_Shape_at_Count` cell as a Seq of facts; each
    // fact is a Seq of <role, value> pairs. (FT cells are Map-backed;
    // `fetch_cell_seq` normalises Map -> Seq — mirrors the sibling tests.)
    let cell = ast::fetch_cell_seq("Problem_ranks_Shape_at_Count", &new_d);
    let facts: Vec<Vec<(String, String)>> = cell.as_seq()
        .map(|s| s.iter()
            .filter_map(|f| f.as_seq().map(|pairs| pairs.iter()
                .filter_map(|p| {
                    let kv = p.as_seq()?;
                    let role = kv.first()?.as_atom()?.to_string();
                    let val = kv.get(1)?.as_atom()?.to_string();
                    Some((role, val))
                })
                .collect::<Vec<_>>()))
            .collect())
        .unwrap_or_default();

    assert!(!facts.is_empty(),
        "Problem_ranks_Shape_at_Count must contain the derived tuple; got empty cell");

    // Find the (Problem='p1', Shape='gather') tuple.
    let tuple = facts.iter().find(|f|
        f.iter().any(|(r, v)| r == "Problem" && v == "p1")
        && f.iter().any(|(r, v)| r == "Shape" && v == "gather")
    ).unwrap_or_else(|| panic!(
        "Problem_ranks_Shape_at_Count must contain a (Problem='p1', Shape='gather') \
         tuple; got {:?}", facts));

    // The join key role IS projected (proves the join formed at all).
    assert!(tuple.iter().any(|(r, v)| r == "Shape" && v == "gather"),
        "tuple must carry Shape='gather'; got {:?}", tuple);

    // THE BUG PIN: the `Count` role — the OTHER role of the second
    // antecedent `Shape has confidence Count` — must be projected with
    // value '3'. Pre-fix this role is UNBOUND/absent.
    let count_binding = tuple.iter().find(|(r, _)| r == "Count");
    assert!(count_binding.is_some(),
        "BUG: the `Count` role (projected from the 2nd antecedent across the \
         value-typed `Shape` join) must be present in the consequent tuple; \
         it is UNBOUND. tuple = {:?}", tuple);
    assert_eq!(count_binding.unwrap().1, "3",
        "the `Count` role must carry value '3' (from `Shape 'gather' has \
         confidence Count '3'`); tuple = {:?}", tuple);
}
