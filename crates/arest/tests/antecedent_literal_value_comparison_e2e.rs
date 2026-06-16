// crates/arest/tests/antecedent_literal_value_comparison_e2e.rs
//
// End-to-end pin: FORML 2 derivation rules whose body compares a role
// value to a numeric LITERAL — `Item1 has Weight1 greater than 5` (text
// phrase) and `Item1 has Weight1 < 5` (symbolic operator) — must
//   (a) parse so the resulting DerivationRuleDef carries a non-empty
//       `antecedent_filters` vec recording the (role, op, value) tuple,
//       with the TEXT phrase and the SYMBOLIC operator mapping to the
//       SAME canonical op symbol, AND
//   (b) compile + forward-chain so `Item_is_big` contains ONLY items
//       whose Weight is strictly greater than 5 and `Item_is_small`
//       only those strictly less than 5 — with the boundary item
//       (Weight == 5) excluded from BOTH.
//
// This is the role-op-LITERAL sibling of cross_antecedent_value_comparison_e2e
// (which pins the role-op-ROLE path). The IR (`AntecedentFilter`,
// types.rs) and evaluator (`comparator_primitive` / `apply_compare` over
// f64) were already present; the parser-side gap was that
// `peel_trailing_comparator` (parse_forml2.rs) recognised only SYMBOLIC
// trailing operators (`> 5`). It was extended to ALSO accept TEXT phrases
// (`greater than`, `less than`, `equal to`, …, with an optional leading
// `is`) mapped to the same canonical op, so both surface forms feed the
// identical Filter primitive. Without the parser-side glue a text-phrase
// comparison clause is silently dropped and the rule fans out over every
// row (the symptom the throwaway `cmp-test*` probe apps showed: an empty
// derived cell because the clause failed to resolve).

use arest::ast::{self, Object};

#[test]
fn literal_value_comparison_text_and_symbolic_parse_and_filter_forward_chain() {
    // Fixture: three Items with Weights 3 / 9 / 5. The boundary item
    // 'c' (Weight 5) must satisfy NEITHER `> 5` nor `< 5`.
    //   Item 'a' Weight 3  -> small
    //   Item 'b' Weight 9  -> big
    //   Item 'c' Weight 5  -> neither
    // `is big` uses the TEXT phrase `greater than 5`; `is small` uses the
    // SYMBOLIC operator `< 5`. Both must drive the same literal-filter.
    let src = "\
        Item(.id) is an entity type.\n\
        Weight(.id) is an entity type.\n\
        Item has Weight.\n\
        Item is big.\n\
        Item is small.\n\
        Item 'a' has Weight '3'.\n\
        Item 'b' has Weight '9'.\n\
        Item 'c' has Weight '5'.\n\
        * Item1 is big iff Item1 has Weight1 greater than 5.\n\
        * Item1 is small iff Item1 has Weight1 < 5.\n\
    ";
    let state = arest::parse_forml2_stage2::parse_to_state_via_stage12(src)
        .expect("parse must succeed");

    // Sanity: both consequent FTs resolve.
    let dr_cell = ast::fetch_or_phi("DerivationRule", &state);
    let consequent_ids: Vec<String> = dr_cell.as_seq()
        .map(|s| s.iter()
            .filter_map(|f| ast::binding(f, "consequentFactTypeId").map(String::from))
            .collect())
        .unwrap_or_default();
    assert!(consequent_ids.iter().any(|id| id == "Item_is_big"),
        "DerivationRule.consequentFactTypeId must resolve to `Item_is_big`; got {:?}",
        consequent_ids);
    assert!(consequent_ids.iter().any(|id| id == "Item_is_small"),
        "DerivationRule.consequentFactTypeId must resolve to `Item_is_small`; got {:?}",
        consequent_ids);

    // The CRITICAL parser-side pin: BOTH surface forms must populate
    // `antecedent_filters` with a (role="Weight", op, value=5) entry.
    // The text phrase `greater than` must canonicalise to ">" — the
    // SAME symbol the symbolic operator `<` lands on for `is small`.
    let idx = arest::compile::cell_index_from_state(&state);
    let has_filter = |op: &str| idx.derivation_rules.iter().any(|r|
        r.antecedent_filters.iter().any(|f|
            f.role == "Weight" && f.op == op && f.value == 5.0
        )
    );
    assert!(has_filter(">"),
        "TEXT phrase `greater than 5` must record an AntecedentFilter \
         (Weight, >, 5) after parse + resolve; rules = {:?}",
        idx.derivation_rules.iter().map(|r|
            (r.id.clone(), r.antecedent_filters.clone())
        ).collect::<Vec<_>>());
    assert!(has_filter("<"),
        "SYMBOLIC operator `< 5` must record an AntecedentFilter \
         (Weight, <, 5) after parse + resolve; rules = {:?}",
        idx.derivation_rules.iter().map(|r|
            (r.id.clone(), r.antecedent_filters.clone())
        ).collect::<Vec<_>>());

    // Forward-chain assertions.
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

    let item_ids_in_cell = |cell_name: &str, state: &Object| -> Vec<String> {
        // FT cells are Map-backed (keyed by full-tuple identity);
        // `fetch_cell_seq` normalises Map -> Seq (mirrors the sibling test).
        let cell = ast::fetch_cell_seq(cell_name, state);
        cell.as_seq()
            .map(|s| s.iter()
                .filter_map(|f| f.as_seq().and_then(|pairs| pairs.iter().find_map(|p| {
                    let kv = p.as_seq()?;
                    let role = kv.first()?.as_atom()?;
                    if role == "Item" { kv.get(1)?.as_atom().map(String::from) } else { None }
                })))
                .collect())
            .unwrap_or_default()
    };

    let big = item_ids_in_cell("Item_is_big", &new_d);
    assert!(big.contains(&"b".to_string()),
        "Item_is_big must include 'b' (Weight 9 > 5); got {:?}", big);
    assert!(!big.contains(&"a".to_string()),
        "Item_is_big must NOT include 'a' (Weight 3 is not > 5); got {:?}", big);
    assert!(!big.contains(&"c".to_string()),
        "Item_is_big must NOT include boundary 'c' (Weight 5 is not > 5); got {:?}", big);

    let small = item_ids_in_cell("Item_is_small", &new_d);
    assert!(small.contains(&"a".to_string()),
        "Item_is_small must include 'a' (Weight 3 < 5); got {:?}", small);
    assert!(!small.contains(&"b".to_string()),
        "Item_is_small must NOT include 'b' (Weight 9 is not < 5); got {:?}", small);
    assert!(!small.contains(&"c".to_string()),
        "Item_is_small must NOT include boundary 'c' (Weight 5 is not < 5); got {:?}", small);
}
