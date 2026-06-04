// crates/arest/tests/subtype_join_antecedent_supertype_ft_e2e.rs
//
// Engine derivation-resolver gap (subtype-join → supertype FT).
//
// `resolve_derivation_rule` forms a forward-chain join between a
// relating fact type and a target fact type by matching a shared role
// NOUN-NAME. When a derivation rule's join clause has a SUBTYPE subject
// but the target FT is declared on its SUPERTYPE, the clause never
// resolves to the supertype-keyed FT and the join never forms — the
// derivation materialises nothing.
//
// Concrete shape (from ns-2):
//   Resource belongs to Domain
//     iff Resource is instance of Noun
//     and that Noun belongs to Domain
// where `Noun` is a SUBTYPE of `Function` (Noun < Function) and
// `belongs to Domain` is a fact type declared on `Function`. The
// `that Noun belongs to Domain` clause is Noun-keyed; the FT
// `belongs to Domain` is Function-keyed. Without the subtype→supertype
// bridge the clause does not resolve and `Resource belongs to Domain`
// derives empty.
//
// Spec: subtype instances ARE supertype instances, so the join must
// form (the Noun id playing the `Function` role of the target FT) and
// `Resource_belongs_to_Domain` must materialise `<Resource 'r1',
// Domain 'd1'>`.

use arest::ast;

#[test]
fn subtype_subject_join_resolves_up_to_supertype_ft_and_materialises() {
    // Function is the supertype; Noun < Function. `belongs to Domain`
    // is declared on Function. The Noun instance 'n1' plays the
    // Function role of a `belongs to Domain` fact (legal: a Noun IS a
    // Function), and Resource 'r1' is instance of that Noun 'n1'.
    // The rule should therefore derive Resource 'r1' belongs to Domain
    // 'd1'.
    let src = "\
        Function(.id) is an entity type.\n\
        Domain(.id) is an entity type.\n\
        Resource(.id) is an entity type.\n\
        Noun is a subtype of Function.\n\
        Function belongs to Domain.\n\
        Resource is instance of Noun.\n\
        Resource belongs to Domain.\n\
        Function 'n1' belongs to Domain 'd1'.\n\
        Resource 'r1' is instance of Noun 'n1'.\n\
        * Resource belongs to Domain iff Resource is instance of Noun and that Noun belongs to Domain.\n\
    ";
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src)
        .expect("parse must succeed");

    // Sanity: the rule's consequent resolves to `Resource_belongs_to_Domain`.
    let dr_cell = ast::fetch_or_phi("DerivationRule", &state);
    let consequent_ids: Vec<String> = dr_cell.as_seq()
        .map(|s| s.iter()
            .filter_map(|f| ast::binding(f, "consequentFactTypeId").map(String::from))
            .collect())
        .unwrap_or_default();
    assert!(consequent_ids.iter().any(|id| id == "Resource_belongs_to_Domain"),
        "DerivationRule.consequentFactTypeId must resolve to \
         `Resource_belongs_to_Domain`; got {:?}", consequent_ids);

    let defs = arest::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);
    let derivation_refs_owned: Vec<(String, ast::Func)> = ast::cells_iter(&d)
        .into_iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();
    let derivation_refs: Vec<(&str, &ast::Func)> = derivation_refs_owned.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _derived) = arest::evaluate::forward_chain_defs_state(
        &derivation_refs, &d);

    // Collect (Resource, Domain) pairs from the derived cell.
    let cell = ast::fetch_cell_seq("Resource_belongs_to_Domain", &new_d);
    let entries: Vec<&ast::Object> = cell.as_seq()
        .map(|s| s.iter().collect()).unwrap_or_default();
    let pairs: Vec<(String, String)> = entries.iter().filter_map(|f| {
        let kv_pairs = f.as_seq()?;
        let mut resource = None;
        let mut domain = None;
        for p in kv_pairs.iter() {
            let kv = match p.as_seq() { Some(kv) => kv, None => continue };
            let role = match kv.first().and_then(|k| k.as_atom()) { Some(r) => r, None => continue };
            let val = kv.get(1).and_then(|v| v.as_atom()).map(String::from);
            match role {
                "Resource" => resource = val,
                "Domain" => domain = val,
                _ => {}
            }
        }
        Some((resource?, domain?))
    }).collect();

    assert!(pairs.iter().any(|(r, dm)| r == "r1" && dm == "d1"),
        "Resource_belongs_to_Domain must contain <Resource 'r1', Domain 'd1'> \
         (derived via the subtype-join: Resource 'r1' is instance of Noun 'n1', \
         and that Noun — a Function subtype — belongs to Domain 'd1'); \
         cell entries: {:?}", entries);
}
