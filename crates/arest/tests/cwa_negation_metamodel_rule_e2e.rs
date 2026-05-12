// crates/arest/tests/cwa_negation_metamodel_rule_e2e.rs
//
// #893 acceptance pin: CWA (Closed-World-Assumption) negation
// materialisation must continue to work after the hardcoded
// per-(CWA-noun, FT, role) Rust loop in
// `compile.rs::compile_derivations` is replaced by a single FORML 2
// metamodel derivation rule expressed in
// `readings/core/derivation.md`.
//
// The compile-time loop (was `compile.rs:2585-2639` pre-#893) emits,
// for every (noun, fact_type, role) triple where the noun has
// `WorldAssumption::Closed` (the default) and plays a role in the
// fact type, a synthetic `DerivationRuleDef` with:
//   - antecedents:
//       `InstancesOfNoun(noun)` (every instance of the noun across
//                                every cell)
//       + `AbsenceOf { fact_type, role: noun }` (no fact of `ft_id`
//                                                binds that instance
//                                                at that role).
//   - consequent_cell: `Literal("_cwa_negation:<ft_id>")` — a
//     dedicated per-FT negation cell keeping negatives out of
//     presence-constraint enumerations.
//   - consequent_instance_role: `_neg_<noun>` — the binding key
//     downstream code knows to look for.
//   - kind: `DerivationKind::ClosedWorldNegation`.
// At forward-chain time each rule fires per Noun instance lacking
// a participation fact in the FT and pushes a
// `<<_neg_<noun>, instance>>` fact into the synthesized negation cell.
//
// Spec: declaring `Person(.id) is an entity type.`,
// `Skill(.id) is an entity type.`,
// `Email(.id) is an entity type.`,
// `Person has Skill.` (binary FT, both nouns play roles),
// `Person has Email.` (a separate FT carrying both alice and bob so
// `instances_of_noun_func("Person")` surfaces both — the
// `Person has Skill` cell only carries alice's participation),
// plus
// `Person 'alice' has Skill 'rust'.`,
// `Person 'alice' has Email 'a@x'.`,
// `Person 'bob'   has Email 'b@x'.` (bob is a Person via the Email
//   FT, but does NOT have a Skill — so CWA negation must surface him
//   in the Person_has_Skill complement cell),
// must, after compile + forward-chain, materialise a fact whose
// `_neg_Person` binding is 'bob' (and not 'alice') in
// `_cwa_negation:Person_has_Skill`.
//
// On a build where the loop has been deleted but the metamodel rule
// is absent, this test must FAIL (no CWA-negation binding lands).
// On a build where the loop is deleted AND the metamodel rule is
// present, this test must PASS — equivalence with the loop output.

use arest::ast;

#[test]
fn cwa_negation_lands_complement_fact_in_negation_cell() {
    // Two entity nouns under default CWA. A binary FT pins both
    // nouns as roles. One Person instance participates in a Skill,
    // one Person instance does not — the second is what the CWA
    // negation rule must surface.
    //
    // The pre-#893 loop fans out one rule per (noun, ft, role)
    // triple; the post-#893 metamodel rule emits the same per-triple
    // tuples via one `Concat . [per-triple Func]` lift.
    let src = "\
        Person(.id) is an entity type.\n\
        Skill(.id) is an entity type.\n\
        Email(.id) is an entity type.\n\
        Person has Skill.\n\
        Person has Email.\n\
        Person 'alice' has Skill 'rust'.\n\
        Person 'alice' has Email 'a@x'.\n\
        Person 'bob' has Email 'b@x'.\n\
    ";
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src)
        .expect("parse must succeed");
    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    // Forward chain over every derivation* def — covers the CWA
    // negation rule (whether emitted by the pre-#893 Rust loop or by
    // the post-#893 metamodel rule expressed in
    // `readings/core/derivation.md`). CWA negation rules carry
    // `uses_negation: true` and emit under `derivation_strat2:` (see
    // `compile.rs:1170` — the chainer runs them in the second
    // stratum after positive rules reach fixpoint so the AbsenceOf
    // guard sees a fully populated FT cell). Pull both prefixes and
    // run a stratified fixpoint so the CWA-negation rule fires
    // after the positive-rule round.
    let stratum1_owned: Vec<(String, ast::Func)> = ast::cells_iter(&d)
        .into_iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();
    let stratum2_owned: Vec<(String, ast::Func)> = ast::cells_iter(&d)
        .into_iter()
        .filter(|(n, _)| n.starts_with("derivation_strat2:"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();
    let stratum1_refs: Vec<(&str, &ast::Func)> = stratum1_owned.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let stratum2_refs: Vec<(&str, &ast::Func)> = stratum2_owned.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _derived) = arest::evaluate::forward_chain_stratified(
        &stratum1_refs, &stratum2_refs, &d, 100);

    // Assertion: the synthetic negation cell
    // `_cwa_negation:Person_has_Skill` must contain a fact whose
    // `_neg_Person` binding is 'bob' — the CWA-inferred complement
    // (alice participates, bob does not).
    let cell_name = "_cwa_negation:Person_has_Skill";
    let cell = ast::fetch_or_phi(cell_name, &new_d);
    let entries: Vec<&ast::Object> = cell.as_seq()
        .map(|s| s.iter().collect()).unwrap_or_default();

    // Walk every fact's bindings; collect each `_neg_Person` atom.
    let neg_persons: Vec<String> = entries.iter().filter_map(|f| {
        let pairs = f.as_seq()?;
        for p in pairs.iter() {
            let kv = match p.as_seq() { Some(kv) => kv, None => continue };
            let role = kv.first().and_then(|k| k.as_atom())?;
            let val  = kv.get(1).and_then(|v| v.as_atom())?;
            if role == "_neg_Person" { return Some(val.to_string()); }
        }
        None
    }).collect();

    assert!(neg_persons.iter().any(|p| p == "bob"),
        "{} must contain <<_neg_Person, 'bob'>> \
         (CWA-inferred from Person has Skill — 'bob' has no Skill); \
         cell entries: {:?}", cell_name, entries);
    assert!(!neg_persons.iter().any(|p| p == "alice"),
        "{} must NOT contain <<_neg_Person, 'alice'>> \
         (alice participates in Skill 'rust', so CWA negation must \
         skip her); cell entries: {:?}", cell_name, entries);
}
