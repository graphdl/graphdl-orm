// crates/arest/tests/subtype_metamodel_rule_e2e.rs
//
// #890 acceptance pin: subtype-inheritance materialisation must
// continue to work after the hardcoded per-(sub, sup, ft) Rust
// synthesizer loop in `compile.rs` is replaced by a single FORML 2
// metamodel derivation rule expressed in `readings/core/core.md`.
//
// The compile-time loop (compile.rs:2378-2409) emits, for every
// (subtype, supertype) declared and every Fact Type whose Reading
// has the supertype playing a Role, a synthetic DerivationRuleDef
// with `InstancesOfNoun(subtype)` antecedent + Literal supertype-FT
// consequent. At forward-chain time those rules fire per-subtype-
// instance and push a `<<supertype-role, instance-id>>` binding
// into the supertype-FT cell — that is the "Resource is inherited
// instance of Noun" fact (core.md §332).
//
// Spec: declaring `Car is a subtype of Vehicle.` plus `Vehicle has
// Color.` plus a Car instance must, after compile + forward-chain,
// materialise `<<Vehicle, '1'>>` in `Vehicle_has_Color`. (The Color
// value is propagated separately by the ingest path; what the loop
// itself synthesizes is the supertype-membership presence binding,
// which is what every downstream consumer — UC enforcement, deontic
// constraints, role bindings — needs to see.)
//
// On a build where the loop has been deleted but the metamodel rule
// is absent, this test must FAIL (no inherited binding lands).
// On a build where the loop is deleted AND the metamodel rule is
// present, this test must PASS — equivalence with the loop output.

use arest::ast;

#[test]
fn subtype_inheritance_lands_supertype_membership_in_supertype_ft_cell() {
    let src = "\
        Vehicle(.id) is an entity type.\n\
        Color is a value type.\n\
        Car is a subtype of Vehicle.\n\
        Vehicle has Color.\n\
        Car '1' has Color 'red'.\n\
    ";
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src)
        .expect("parse must succeed");
    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    // Forward chain over every derivation:* def — covers both user
    // rules and the synthetic subtype-inheritance rules (whether
    // emitted by the legacy Rust loop or by the post-#890 metamodel
    // rule expressed in readings/core/core.md).
    let derivation_refs_owned: Vec<(String, ast::Func)> = ast::cells_iter(&d)
        .into_iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();
    let derivation_refs: Vec<(&str, &ast::Func)> = derivation_refs_owned.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _derived) = arest::evaluate::forward_chain_defs_state(
        &derivation_refs, &d);

    // Assertion: Vehicle_has_Color must contain at least one fact
    // whose Vehicle role binds to '1' — that is the inherited
    // supertype-membership emission. Without the inheritance step
    // (loop removed, no metamodel rule), the cell would either be
    // empty or contain only user-pushed facts (none in this fixture).
    let vh_cell = ast::fetch_or_phi("Vehicle_has_Color", &new_d);
    let entries: Vec<&ast::Object> = vh_cell.as_seq()
        .map(|s| s.iter().collect()).unwrap_or_default();

    // Walk every fact's bindings; collect the Vehicle-role atom values.
    let vehicle_ids: Vec<String> = entries.iter().filter_map(|f| {
        let pairs = f.as_seq()?;
        for p in pairs.iter() {
            let kv = match p.as_seq() { Some(kv) => kv, None => continue };
            let role = kv.first().and_then(|k| k.as_atom())?;
            if role == "Vehicle" {
                return kv.get(1).and_then(|v| v.as_atom()).map(String::from);
            }
        }
        None
    }).collect();

    assert!(vehicle_ids.iter().any(|v| v == "1"),
        "Vehicle_has_Color must contain Vehicle '1' (inherited from \
         Car '1' via subtype-inheritance fanout); cell entries: {:?}",
        entries);
}
