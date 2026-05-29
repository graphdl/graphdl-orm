// crates/arest/tests/transitivity_metamodel_rule_e2e.rs
//
// task-969 pin: the eager transitive-closure materialisation has been
// REMOVED. Earlier (#892) the compiler lifted the per-(ft1, ft2)
// binary-FT pair synthesis into a single `_transitivity` metamodel
// CompiledDerivation that, every forward-chain round, materialised the
// all-pairs transitive composition of every chaining binary Fact Type
// into synthetic `_transitive_<ft1>_<ft2>` cells (~10342 candidate
// facts on the task-960 app — the dominant metamodel-create cost).
//
// It was deleted as an UNCONSUMED eager materialisation: no consumer
// ever read those cells. SM transition validation joins the explicit
// `Transition_is_from_Status` / `Transition_is_to_Status` /
// `Transition_is_defined_in_State_Machine_Definition` cells directly,
// never a transitive closure; the `command.rs` SM gate's
// `_transitive_Status` / `_transitive_Transition` substring matches
// were dead (the only transitivity def id was `_transitivity`, which
// never contains the substring `_transitive_`); and the TS/MCP layer
// has zero references to `_transitive`. This mirrors the removal of the
// eager CWA-negation complement — see
// `evaluate::tests::test_cwa_vs_owa_negation` and
// `readings/core/derivation.md`.
//
// This test now PINS THE ABSENCE: on the exact fixture that the old
// eager rule materialised a closure for (Person → City → Country
// sharing the `City` join noun), forward-chaining every `derivation:*`
// def must produce NO populated `_transitive_*` closure cell. If a
// future change reintroduces the eager materialisation, this test
// fails — flagging the regression.

use arest::ast;

#[test]
fn no_eager_transitive_closure_cell_is_materialised() {
    // The exact fixture the pre-task-969 eager rule fanned out over:
    // two binary FTs sharing a join noun (`City`). The first FT's
    // second role is City, the second FT's first role is City — the
    // removed rule would have synthesised
    // `_transitive_Person_has_City_City_is_in_Country` holding
    // `<<Person 'p1'>, <Country 'us'>>`. It must no longer appear.
    let src = "\
        Person(.id) is an entity type.\n\
        City(.id) is an entity type.\n\
        Country(.id) is an entity type.\n\
        Person has City.\n\
        City is in Country.\n\
        Person 'p1' has City 'c1'.\n\
        City 'c1' is in Country 'us'.\n\
    ";
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src)
        .expect("parse must succeed");
    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    // No `derivation:*` def may carry the removed `_transitivity`
    // metamodel rule any more.
    assert!(
        !ast::cells_iter(&d).into_iter().any(|(n, _)| n == "derivation:_transitivity"),
        "the eager `_transitivity` metamodel derivation must be gone (task-969); \
         found a `derivation:_transitivity` def");

    // Forward chain over every derivation:* def — this is the round in
    // which the removed rule would have materialised its closure cells.
    let derivation_refs_owned: Vec<(String, ast::Func)> = ast::cells_iter(&d)
        .into_iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();
    let derivation_refs: Vec<(&str, &ast::Func)> = derivation_refs_owned.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _derived) = arest::evaluate::forward_chain_defs_state(
        &derivation_refs, &d);

    // Assertion: NO `_transitive_*` closure cell exists in the
    // post-forward-chain state. (Base/derived cells live unnamespaced;
    // the removed eager rule named its outputs `_transitive_<ft1>_<ft2>`.)
    let transitive_cells: Vec<String> = ast::cells_iter(&new_d)
        .into_iter()
        .map(|(n, _)| n.to_string())
        .filter(|n| n.starts_with("_transitive_"))
        .collect();
    assert!(transitive_cells.is_empty(),
        "no eager `_transitive_*` closure cell may be materialised after \
         forward-chain (task-969 removed the eager transitivity rule); \
         found: {:?}",
        transitive_cells);
}
