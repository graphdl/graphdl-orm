// crates/arest/src/induce.rs
//
// Induction engine search primitives (#848-#852).
//
// This file resurrects the historical `induce.rs` (deleted in
// 77edd7b3 per #211 because it had zero production callers and
// self-referential tests) with a NEW purpose: search primitives
// for the platform `induce` Func registered in #846.
//
// Today's primitive is `enumerate_candidates_for_fact_type` —
// given a FactType id and the current cell state, enumerate every
// candidate fact of that shape across the finite domain of each
// role. Domains come from cells (EnumValues for value types, the
// existing entity population for entity types), NOT hardcoded —
// per the "readings as source code" discipline, the SEMANTICS of
// "what's a candidate" lives in the FactType + Noun + EnumValues
// cells (data); this Rust function is just the dispatch wiring.
//
// #849 (forward-chain check) and #850 (constraint-violation gate)
// will add sibling helpers here that consume the candidates this
// function emits. #851 ties them together into the search loop
// that produces Hypothesis Candidates.

use alloc::{string::{String, ToString}, vec::Vec};
use crate::ast::{Object, fetch_or_phi, fact_from_pairs, binding};

/// Enumerate every candidate fact of shape `ft_id` over the finite
/// domain of each role.
///
/// For each role of the FactType:
///   - If the role's noun is a value type (Noun cell entry where
///     `objectType == "value"`), the domain is the EnumValues cell
///     entry for that noun (all `valueN` bindings).
///   - If the role's noun is an entity type, the domain is the
///     existing population of that noun: any fact in any cell that
///     binds that noun id (e.g. `(Coin → c1)`) contributes the
///     bound value. Duplicates collapse via first-occurrence
///     dedup so a noun that appears in many cells doesn't multiply.
///
/// The cartesian product across roles produces the candidate list.
/// Each candidate is shaped like `InstanceFact`:
///
///   <<subjectNoun, X>, <subjectValue, ...>, <fieldName, FT_id>,
///    <objectNoun, Y>, <objectValue, ...>>
///
/// for binary FTs (drop the object pair for unary; append
/// `roleNNoun`/`roleNValue` for ternary+ — mirrors
/// `parse_forml2_stage2::translate_instance_facts_with_ft_ids`).
///
/// Returns an empty vec when:
///   - the FactType id is not declared,
///   - any role has an empty domain (cartesian product over the
///     empty set IS the empty set — there's no valid binding).
pub fn enumerate_candidates_for_fact_type(state: &Object, ft_id: &str) -> Vec<Object> {
    let role_nouns = role_nouns_for_ft(state, ft_id);
    if role_nouns.is_empty() {
        return Vec::new();
    }
    // Domain per role. Empty domain at any position collapses the
    // whole cartesian product (consistent with set-theoretic ∏).
    let domains: Vec<Vec<String>> = role_nouns.iter()
        .map(|noun| domain_for_noun(state, noun))
        .collect();
    if domains.iter().any(|d| d.is_empty()) {
        return Vec::new();
    }
    // Cartesian product. Iterative to avoid recursion stack hits on
    // wide FT shapes. Index vector advances right-to-left.
    let mut out: Vec<Object> = Vec::new();
    let mut indices: Vec<usize> = vec![0; domains.len()];
    loop {
        let bindings: Vec<&str> = indices.iter()
            .enumerate()
            .map(|(i, &j)| domains[i][j].as_str())
            .collect();
        out.push(build_candidate_fact(ft_id, &role_nouns, &bindings));
        if !advance_indices(&mut indices, &domains) { break; }
    }
    out
}

/// Read role nouns for a FactType id from the Role cell, ordered by
/// `position`. Returns an empty vec when the FactType id is unknown
/// or carries no roles.
fn role_nouns_for_ft(state: &Object, ft_id: &str) -> Vec<String> {
    // Confirm the FT itself exists; if not, no candidates regardless
    // of what roles happen to be lying around.
    let ft_cell = fetch_or_phi("FactType", state);
    let ft_seq = match ft_cell.as_seq() {
        Some(s) => s,
        None => return Vec::new(),
    };
    if !ft_seq.iter().any(|f| binding(f, "id") == Some(ft_id)) {
        return Vec::new();
    }
    let role_cell = fetch_or_phi("Role", state);
    let role_seq = match role_cell.as_seq() {
        Some(s) => s,
        None => return Vec::new(),
    };
    let mut with_pos: Vec<(usize, String)> = role_seq.iter()
        .filter_map(|r| {
            if binding(r, "factType") != Some(ft_id) { return None; }
            let pos: usize = binding(r, "position")?.parse().ok()?;
            let noun = binding(r, "nounName")?.to_string();
            Some((pos, noun))
        })
        .collect();
    with_pos.sort_by_key(|(p, _)| *p);
    with_pos.into_iter().map(|(_, n)| n).collect()
}

/// Resolve the finite domain for a noun, dispatching on its
/// `objectType` from the Noun cell.
fn domain_for_noun(state: &Object, noun_name: &str) -> Vec<String> {
    let object_type = noun_object_type(state, noun_name);
    match object_type.as_deref() {
        Some("value") => enum_values_for_noun(state, noun_name),
        // Entity (or unknown — treat as entity per the conservative
        // default in compile.rs:953). Walk cells for facts that bind
        // that noun id.
        _ => entity_population_for_noun(state, noun_name),
    }
}

/// Read `objectType` for a noun from the Noun cell. Returns `None`
/// if the noun is undeclared.
fn noun_object_type(state: &Object, noun_name: &str) -> Option<String> {
    let cell = fetch_or_phi("Noun", state);
    let seq = cell.as_seq()?;
    for f in seq.iter() {
        if binding(f, "name") == Some(noun_name) {
            return binding(f, "objectType").map(String::from);
        }
    }
    None
}

/// Read enum values for a value-typed noun from the EnumValues
/// cell. Returns an empty vec when no row matches.
fn enum_values_for_noun(state: &Object, noun_name: &str) -> Vec<String> {
    let cell = fetch_or_phi("EnumValues", state);
    let seq = match cell.as_seq() {
        Some(s) => s,
        None => return Vec::new(),
    };
    for f in seq.iter() {
        if binding(f, "noun") != Some(noun_name) { continue; }
        return (0..)
            .map_while(|i| {
                let key = alloc::format!("value{i}");
                binding(f, &key).map(String::from)
            })
            .collect();
    }
    Vec::new()
}

/// Walk every cell in `state` and collect distinct values bound to
/// the noun id. Mirrors `compile::instances_of_noun_func`'s shape:
/// any binding whose key equals `noun_name` contributes its value.
/// Also reads InstanceFact's `subjectNoun`/`objectNoun` keying so
/// raw stage-2 output (before per-FT cell projection) participates.
fn entity_population_for_noun(state: &Object, noun_name: &str) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut push_unique = |v: &str, seen: &mut Vec<String>| {
        if !v.is_empty() && !seen.iter().any(|s| s == v) {
            seen.push(v.to_string());
        }
    };
    // Walk all cells. Each cell is `<CELL, name, contents>`; the
    // `cells_iter` helper isn't exported, so we walk the Map / Seq
    // representation directly here. Either Object::Map or Object::Seq
    // of cell triples is supported (Backus §13.3.4 + AREST `store`).
    let cells: Vec<(String, Object)> = match state {
        Object::Map(m) => m.iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        Object::Seq(items) => items.iter()
            .filter_map(|c| {
                let parts = c.as_seq()?;
                if parts.len() == 3 && parts[0].as_atom() == Some("CELL") {
                    Some((parts[1].as_atom()?.to_string(), parts[2].clone()))
                } else {
                    None
                }
            })
            .collect(),
        _ => Vec::new(),
    };
    for (_cell_name, contents) in cells.iter() {
        let Some(facts) = contents.as_seq() else { continue };
        for fact in facts.iter() {
            // Per-FT projection shape: binding key IS the noun name.
            if let Some(v) = binding(fact, noun_name) {
                push_unique(v, &mut seen);
            }
            // Raw InstanceFact shape: subject/object pairs keyed by
            // `subjectNoun`/`subjectValue` (and roleN counterparts).
            if binding(fact, "subjectNoun") == Some(noun_name) {
                if let Some(v) = binding(fact, "subjectValue") {
                    push_unique(v, &mut seen);
                }
            }
            if binding(fact, "objectNoun") == Some(noun_name) {
                if let Some(v) = binding(fact, "objectValue") {
                    push_unique(v, &mut seen);
                }
            }
            // Ternary+ role positions.
            for n in 2.. {
                let noun_key = alloc::format!("role{}Noun", n);
                let Some(other) = binding(fact, &noun_key) else { break };
                if other == noun_name {
                    let val_key = alloc::format!("role{}Value", n);
                    if let Some(v) = binding(fact, &val_key) {
                        push_unique(v, &mut seen);
                    }
                }
            }
        }
    }
    seen
}

/// Advance the cartesian-product index vector. Returns false when
/// the rightmost rollover would carry past position 0 (i.e., the
/// product has been fully enumerated).
fn advance_indices(indices: &mut [usize], domains: &[Vec<String>]) -> bool {
    let mut i = indices.len();
    while i > 0 {
        i -= 1;
        if indices[i] + 1 < domains[i].len() {
            indices[i] += 1;
            for j in (i + 1)..indices.len() {
                indices[j] = 0;
            }
            return true;
        }
    }
    false
}

/// Build a single InstanceFact-shaped candidate. Mirrors
/// `parse_forml2_stage2::translate_instance_facts_with_ft_ids`'s
/// canonical layout: 5-pair prefix (subject + field + object) plus
/// one (roleNNoun, roleNValue) pair per additional role. Unary FTs
/// emit empty objectNoun/objectValue (same as the stage-2 fallback).
fn build_candidate_fact(
    ft_id: &str,
    role_nouns: &[String],
    role_values: &[&str],
) -> Object {
    let subject_noun = role_nouns[0].as_str();
    let subject_value = role_values[0];
    let (object_noun, object_value) = role_nouns.get(1)
        .map(|n| (n.as_str(), role_values[1]))
        .unwrap_or(("", ""));
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(
        5 + 2 * role_nouns.len().saturating_sub(2));
    pairs.push(("subjectNoun".to_string(),  subject_noun.to_string()));
    pairs.push(("subjectValue".to_string(), subject_value.to_string()));
    pairs.push(("fieldName".to_string(),    ft_id.to_string()));
    pairs.push(("objectNoun".to_string(),   object_noun.to_string()));
    pairs.push(("objectValue".to_string(),  object_value.to_string()));
    for (i, noun) in role_nouns.iter().enumerate().skip(2) {
        pairs.push((alloc::format!("role{}Noun", i),  noun.clone()));
        pairs.push((alloc::format!("role{}Value", i), role_values[i].to_string()));
    }
    let pair_refs: Vec<(&str, &str)> = pairs.iter()
        .map(|(k, v)| (k.as_str(), v.as_str())).collect();
    fact_from_pairs(&pair_refs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{cell, Object};

    /// Build a synthetic state carrying Noun + FactType + Role +
    /// EnumValues + (optional) InstanceFact cells. Mirrors the
    /// shape stage-2 emits so the function exercises real cell
    /// reads, not handcrafted accessors.
    fn make_state(
        nouns: &[(&str, &str)],            // (name, objectType)
        fact_types: &[(&str, &str, usize)], // (id, reading, arity)
        roles: &[(&str, &str, usize)],     // (factType, nounName, position)
        enum_values: &[(&str, &[&str])],   // (noun, values)
        instance_facts: &[Vec<(&str, &str)>],
    ) -> Object {
        let noun_facts: Vec<Object> = nouns.iter()
            .map(|(n, t)| fact_from_pairs(&[("name", n), ("objectType", t)]))
            .collect();
        let ft_facts: Vec<Object> = fact_types.iter()
            .map(|(id, reading, arity)| {
                let arity_s = arity.to_string();
                fact_from_pairs(&[
                    ("id", *id),
                    ("reading", *reading),
                    ("arity", arity_s.as_str()),
                ])
            })
            .collect();
        let role_facts: Vec<Object> = roles.iter()
            .map(|(ft, n, pos)| {
                let pos_s = pos.to_string();
                fact_from_pairs(&[
                    ("factType", *ft),
                    ("nounName", *n),
                    ("position", pos_s.as_str()),
                ])
            })
            .collect();
        let enum_facts: Vec<Object> = enum_values.iter()
            .map(|(noun, vals)| {
                let mut pairs: Vec<(String, String)> = Vec::new();
                pairs.push(("noun".to_string(), (*noun).to_string()));
                for (i, v) in vals.iter().enumerate() {
                    pairs.push((alloc::format!("value{i}"), (*v).to_string()));
                }
                let pair_refs: Vec<(&str, &str)> = pairs.iter()
                    .map(|(k, v)| (k.as_str(), v.as_str())).collect();
                fact_from_pairs(&pair_refs)
            })
            .collect();
        let inst_facts: Vec<Object> = instance_facts.iter()
            .map(|pairs| {
                let pair_refs: Vec<(&str, &str)> = pairs.iter()
                    .map(|(k, v)| (*k, *v)).collect();
                fact_from_pairs(&pair_refs)
            })
            .collect();
        Object::seq(vec![
            cell("Noun",         Object::Seq(noun_facts.into())),
            cell("FactType",     Object::Seq(ft_facts.into())),
            cell("Role",         Object::Seq(role_facts.into())),
            cell("EnumValues",   Object::Seq(enum_facts.into())),
            cell("InstanceFact", Object::Seq(inst_facts.into())),
        ])
    }

    /// Single role over a value type: enumeration matches the
    /// EnumValues cell row 1:1. Drives the value-type domain branch.
    /// For the `Coin has Side` binary FT, with one Coin instance
    /// `c1` and Side ∈ {heads, tails}, we expect 2 candidates:
    /// `<c1, heads>` and `<c1, tails>`.
    #[test]
    fn single_role_enum_value_type_yields_one_per_value() {
        let state = make_state(
            &[("Coin", "entity"), ("Side", "value")],
            &[("Coin_has_Side", "Coin has Side", 2)],
            &[
                ("Coin_has_Side", "Coin", 0),
                ("Coin_has_Side", "Side", 1),
            ],
            &[("Side", &["heads", "tails"])],
            // Single Coin instance c1 — supplies the entity-side
            // domain for role 0 via the InstanceFact cell.
            &[vec![
                ("subjectNoun",  "Coin"),
                ("subjectValue", "c1"),
                ("fieldName",    "Coin_exists"),
                ("objectNoun",   ""),
                ("objectValue",  ""),
            ]],
        );
        let candidates = enumerate_candidates_for_fact_type(&state, "Coin_has_Side");
        assert_eq!(candidates.len(), 2,
            "expected one candidate per Side value (heads, tails); got {:?}",
            candidates);
        let pairs: Vec<(Option<&str>, Option<&str>, Option<&str>, Option<&str>)> =
            candidates.iter().map(|c| (
                binding(c, "subjectNoun"),
                binding(c, "subjectValue"),
                binding(c, "objectNoun"),
                binding(c, "objectValue"),
            )).collect();
        assert!(pairs.contains(&(Some("Coin"), Some("c1"), Some("Side"), Some("heads"))),
            "missing <c1, heads>; got {:?}", pairs);
        assert!(pairs.contains(&(Some("Coin"), Some("c1"), Some("Side"), Some("tails"))),
            "missing <c1, tails>; got {:?}", pairs);
        // fieldName must be the canonical FT id (not a raw verb)
        // since this primitive operates on declared FactType ids.
        for c in candidates.iter() {
            assert_eq!(binding(c, "fieldName"), Some("Coin_has_Side"));
        }
    }

    /// Cartesian product across two roles: 2 entities × 2 enum
    /// values = 4 candidates. Confirms the index advancer.
    #[test]
    fn two_role_binary_ft_yields_cartesian_product() {
        let state = make_state(
            &[("Coin", "entity"), ("Side", "value")],
            &[("Coin_has_Side", "Coin has Side", 2)],
            &[
                ("Coin_has_Side", "Coin", 0),
                ("Coin_has_Side", "Side", 1),
            ],
            &[("Side", &["heads", "tails"])],
            &[
                vec![
                    ("subjectNoun",  "Coin"),
                    ("subjectValue", "c1"),
                    ("fieldName",    "Coin_exists"),
                    ("objectNoun",   ""),
                    ("objectValue",  ""),
                ],
                vec![
                    ("subjectNoun",  "Coin"),
                    ("subjectValue", "c2"),
                    ("fieldName",    "Coin_exists"),
                    ("objectNoun",   ""),
                    ("objectValue",  ""),
                ],
            ],
        );
        let candidates = enumerate_candidates_for_fact_type(&state, "Coin_has_Side");
        assert_eq!(candidates.len(), 4,
            "expected 2 Coins × 2 Sides = 4 candidates; got {:?}",
            candidates);
        // All four (Coin, Side) pairs must be present exactly once.
        let pairs: Vec<(Option<&str>, Option<&str>)> =
            candidates.iter().map(|c| (
                binding(c, "subjectValue"),
                binding(c, "objectValue"),
            )).collect();
        for coin in &["c1", "c2"] {
            for side in &["heads", "tails"] {
                assert!(pairs.contains(&(Some(*coin), Some(*side))),
                    "missing <{}, {}>; got {:?}", coin, side, pairs);
            }
        }
    }

    /// Empty domain at any role collapses the cartesian product.
    /// Side has no enum values declared and no entity instances —
    /// so `Coin has Side` yields zero candidates regardless of how
    /// many Coins there are.
    #[test]
    fn empty_domain_yields_empty_seq() {
        let state = make_state(
            &[("Coin", "entity"), ("Side", "value")],
            &[("Coin_has_Side", "Coin has Side", 2)],
            &[
                ("Coin_has_Side", "Coin", 0),
                ("Coin_has_Side", "Side", 1),
            ],
            // No EnumValues row for Side → role 1's domain is empty.
            &[],
            // Coin still has an instance — role 0's domain is non-empty.
            &[vec![
                ("subjectNoun",  "Coin"),
                ("subjectValue", "c1"),
                ("fieldName",    "Coin_exists"),
                ("objectNoun",   ""),
                ("objectValue",  ""),
            ]],
        );
        let candidates = enumerate_candidates_for_fact_type(&state, "Coin_has_Side");
        assert!(candidates.is_empty(),
            "expected empty result when role domain is empty; got {:?}",
            candidates);
    }
}
