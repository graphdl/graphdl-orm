// crates/arest/tests/transitivity_metamodel_rule_e2e.rs
//
// #892 acceptance pin: transitive-closure materialisation must
// continue to work after the hardcoded per-(ft1, ft2) Rust loop in
// `compile.rs::compile_derivations` is replaced by a single FORML 2
// metamodel derivation rule expressed in `readings/core/derivation.md`.
//
// The compile-time loop (was `compile.rs:2514-2559` pre-#892) emits,
// for every pair of binary Fact Types (ft1, ft2) where
// `ft1.roles[1].noun_name == ft2.roles[0].noun_name` (i.e. ft1 ends
// at the same noun ft2 begins at), a synthetic `DerivationRuleDef`
// with:
//   - antecedents: `FactType(ft1)` + `FactType(ft2)`
//   - consequent_cell: `Literal("_transitive_<ft1>_<ft2>")`
//   - kind: `DerivationKind::Transitivity`
//   - join_on: the shared join-noun
//   - consequent_bindings: `[src_noun, dst_noun]` (the unshared
//     endpoint nouns of ft1 and ft2 respectively).
// At forward-chain time each rule fires per matching antecedent-pair
// and pushes a `<<src_noun, src_id>, <dst_noun, dst_id>>` fact into
// the synthesized transitive cell.
//
// Spec: declaring `Person has City` (Person→City binary FT) plus
// `City is in Country` (City→Country binary FT), plus
// `Person 'p1' has City 'c1'.` plus `City 'c1' is in Country 'us'.`
// must, after compile + forward-chain, materialise
// `<<Person, 'p1'>, <Country, 'us'>>` in
// `_transitive_Person_has_City_City_is_in_Country`.
//
// On a build where the loop has been deleted but the metamodel rule
// is absent, this test must FAIL (no transitive binding lands).
// On a build where the loop is deleted AND the metamodel rule is
// present, this test must PASS — equivalence with the loop output.

use arest::ast;

#[test]
fn transitivity_lands_inferred_fact_in_transitive_ft_cell() {
    // Two binary FTs sharing a join noun (`City`). The first FT's
    // second role is City, the second FT's first role is City. The
    // pre-#892 loop emits a single transitivity rule for this pair;
    // the post-#892 metamodel rule emits the same pair (and every
    // other (ft1, ft2) combination that shares an inner noun) via
    // one `Concat . [per-pair Func]` lift.
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

    // Forward chain over every derivation:* def — covers the
    // transitivity rule (whether emitted by the pre-#892 Rust loop
    // or by the post-#892 metamodel rule expressed in
    // `readings/core/derivation.md`).
    let derivation_refs_owned: Vec<(String, ast::Func)> = ast::cells_iter(&d)
        .into_iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();
    let derivation_refs: Vec<(&str, &ast::Func)> = derivation_refs_owned.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _derived) = arest::evaluate::forward_chain_defs_state(
        &derivation_refs, &d);

    // Assertion: the synthetic transitive cell
    // `_transitive_Person_has_City_City_is_in_Country` must contain
    // a fact whose Person role binds to 'p1' AND Country role binds
    // to 'us' — that is the transitivity-inferred fact. Without the
    // transitivity step (loop removed, no metamodel rule), the cell
    // would be empty.
    let cell_name = "_transitive_Person_has_City_City_is_in_Country";
    let cell = ast::fetch_or_phi(cell_name, &new_d);
    let entries: Vec<&ast::Object> = cell.as_seq()
        .map(|s| s.iter().collect()).unwrap_or_default();

    // Walk every fact's bindings; collect (Person, Country) pairs.
    let pairs: Vec<(String, String)> = entries.iter().filter_map(|f| {
        let pairs = f.as_seq()?;
        let mut person: Option<String> = None;
        let mut country: Option<String> = None;
        for p in pairs.iter() {
            let kv = match p.as_seq() { Some(kv) => kv, None => continue };
            let role = kv.first().and_then(|k| k.as_atom())?;
            let val = kv.get(1).and_then(|v| v.as_atom())?;
            if role == "Person"  { person  = Some(val.to_string()); }
            if role == "Country" { country = Some(val.to_string()); }
        }
        Some((person?, country?))
    }).collect();

    assert!(pairs.iter().any(|(p, c)| p == "p1" && c == "us"),
        "{} must contain <<Person 'p1'>, <Country 'us'>> \
         (transitively inferred from Person→City→Country); cell entries: {:?}",
        cell_name, entries);
}
