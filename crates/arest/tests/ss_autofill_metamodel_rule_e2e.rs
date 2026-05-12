// crates/arest/tests/ss_autofill_metamodel_rule_e2e.rs
//
// #891 acceptance pin: SS (Subset) auto-fill materialisation must
// continue to work after the hardcoded per-SS-Constraint Rust loop
// in `compile.rs::compile_derivations` is replaced by a single
// FORML 2 metamodel derivation rule expressed in
// `readings/core/derivation.md`.
//
// The compile-time loop (was `compile.rs:2444-2463`) emits, for
// every SS Constraint whose `subset_autofill` span marker is
// `Some(true)`, a synthetic `DerivationRuleDef` with
// `FactType(antecedent_ft)` antecedent + `Literal(consequent_ft)`
// consequent. At forward-chain time each rule fires per
// antecedent-FT fact and pushes the same bindings into the
// consequent-FT cell — the "auto-fill" behaviour the SS span's
// subset-marker requests.
//
// Spec: declaring SS constraint
//   "If some Academic heads some Department then that Academic
//    works for that Department"
// with `subset_autofill: Some(true)` on the antecedent span, plus
// FactTypes `Academic_heads_Department` (ft_heads) and
// `Academic_works_for_Department` (ft_works), plus a fact
// `<<Academic, 'A1'>, <Department, 'D1'>>` in ft_heads must, after
// compile + forward-chain, materialise the same `<<Academic,
// 'A1'>, <Department, 'D1'>>` fact in ft_works.
//
// On a build where the loop has been deleted but the metamodel
// rule is absent, this test must FAIL (the autofilled fact never
// lands). On a build where the loop is deleted AND the metamodel
// rule is present, this test must PASS — equivalence with the
// loop output.

use arest::ast;
use arest::types::{ConstraintDef, SpanDef};

/// Build a Constraint cell fact for a `ConstraintDef`. Mirrors the
/// crate-internal `parse_forml2::constraint_to_fact_test` shape
/// (which is `pub(crate)` and unavailable to integration tests).
/// The JSON binding round-trips through `cell_index_from_state`'s
/// lossless std-deps path, so `subset_autofill: Some(true)` is
/// preserved into the `ConstraintDef` the metamodel rule reads.
fn constraint_fact(c: &ConstraintDef) -> ast::Object {
    let json = serde_json::to_string(c).expect("ConstraintDef serializes");
    let mut pairs: Vec<(String, String)> = vec![
        ("id".into(),       c.id.clone()),
        ("kind".into(),     c.kind.clone()),
        ("modality".into(), c.modality.clone()),
        ("text".into(),     c.text.clone()),
        ("json".into(),     json),
    ];
    if let Some(e) = &c.entity { pairs.push(("entity".into(), e.clone())); }
    // `span<i>_*` fields are read by the flat fallback path in
    // `cell_index_from_state` when `json` isn't available — keep
    // them for parity with `parse_forml2::constraint_to_fact_test`
    // even though the JSON-binding wins under std-deps.
    for (i, span) in c.spans.iter().enumerate() {
        pairs.push((format!("span{}_factTypeId", i), span.fact_type_id.clone()));
        pairs.push((format!("span{}_roleIndex",  i), span.role_index.to_string()));
    }
    let refs: Vec<(&str, &str)> = pairs.iter()
        .map(|(k, v)| (k.as_str(), v.as_str())).collect();
    ast::fact_from_pairs(&refs)
}

#[test]
fn ss_autofill_lands_consequent_fact_in_consequent_ft_cell() {
    // Build the state cells: Noun, FactType, Role, Constraint, and
    // one antecedent fact.
    let mut state = ast::Object::phi();
    // Noun: Academic, Department
    state = ast::cell_push("Noun", ast::fact_from_pairs(&[
        ("name", "Academic"), ("objectType", "entity"),
        ("worldAssumption", "open"), ("referenceScheme", "id"),
    ]), &state);
    state = ast::cell_push("Noun", ast::fact_from_pairs(&[
        ("name", "Department"), ("objectType", "entity"),
        ("worldAssumption", "open"), ("referenceScheme", "id"),
    ]), &state);
    // FactType ft_heads: Academic heads Department
    state = ast::cell_push("FactType", ast::fact_from_pairs(&[
        ("id", "ft_heads"), ("reading", "Academic heads Department"), ("arity", "2"),
    ]), &state);
    state = ast::cell_push("Role", ast::fact_from_pairs(&[
        ("factType", "ft_heads"), ("nounName", "Academic"), ("position", "0"),
    ]), &state);
    state = ast::cell_push("Role", ast::fact_from_pairs(&[
        ("factType", "ft_heads"), ("nounName", "Department"), ("position", "1"),
    ]), &state);
    // FactType ft_works: Academic works for Department
    state = ast::cell_push("FactType", ast::fact_from_pairs(&[
        ("id", "ft_works"), ("reading", "Academic works for Department"), ("arity", "2"),
    ]), &state);
    state = ast::cell_push("Role", ast::fact_from_pairs(&[
        ("factType", "ft_works"), ("nounName", "Academic"), ("position", "0"),
    ]), &state);
    state = ast::cell_push("Role", ast::fact_from_pairs(&[
        ("factType", "ft_works"), ("nounName", "Department"), ("position", "1"),
    ]), &state);
    // SS Constraint: ft_heads ⊆ ft_works with autofill on the
    // antecedent span.
    let cdef = ConstraintDef {
        id: "ss1".to_string(),
        kind: "SS".to_string(),
        modality: "Alethic".to_string(),
        text: "If some Academic heads some Department then that \
               Academic works for that Department".to_string(),
        spans: vec![
            SpanDef { fact_type_id: "ft_heads".to_string(), role_index: 0,
                      subset_autofill: Some(true) },
            SpanDef { fact_type_id: "ft_works".to_string(), role_index: 0,
                      subset_autofill: None },
        ],
        entity: None,
        deontic_operator: None,
        set_comparison_argument_length: None,
        clauses: None,
        min_occurrence: None,
        max_occurrence: None,
        predicate: None,
    };
    state = ast::cell_push("Constraint", constraint_fact(&cdef), &state);
    // Antecedent fact: <<Academic, 'A1'>, <Department, 'D1'>> in ft_heads
    state = ast::cell_push("ft_heads", ast::fact_from_pairs(&[
        ("Academic", "A1"), ("Department", "D1"),
    ]), &state);

    // Compile and lift the derivations into the def-map.
    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    // Forward chain over every derivation:* def — covers the SS
    // auto-fill rule (whether emitted by the pre-#891 Rust loop or
    // by the post-#891 metamodel rule expressed in
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

    // Assertion: ft_works must contain a fact whose Academic role
    // binds to 'A1' AND Department role binds to 'D1' — that is the
    // SS-autofilled fact. Without the autofill step (loop removed,
    // no metamodel rule), the cell would be empty.
    let works_cell = ast::fetch_or_phi("ft_works", &new_d);
    let entries: Vec<&ast::Object> = works_cell.as_seq()
        .map(|s| s.iter().collect()).unwrap_or_default();

    // Walk every fact's bindings; collect (Academic, Department) pairs.
    let pairs: Vec<(String, String)> = entries.iter().filter_map(|f| {
        let pairs = f.as_seq()?;
        let mut acad: Option<String> = None;
        let mut dept: Option<String> = None;
        for p in pairs.iter() {
            let kv = match p.as_seq() { Some(kv) => kv, None => continue };
            let role = kv.first().and_then(|k| k.as_atom())?;
            let val = kv.get(1).and_then(|v| v.as_atom())?;
            if role == "Academic"   { acad = Some(val.to_string()); }
            if role == "Department" { dept = Some(val.to_string()); }
        }
        Some((acad?, dept?))
    }).collect();

    assert!(pairs.iter().any(|(a, d)| a == "A1" && d == "D1"),
        "ft_works must contain <<Academic 'A1'>, <Department 'D1'>> \
         (SS-autofilled from ft_heads); cell entries: {:?}", entries);
}
