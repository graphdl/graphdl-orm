// crates/arest/src/induce.rs
//
// Induction engine: given a state of facts, infer the constraints
// and derivation rules that govern it.
//
// This is the inverse of evaluation -- instead of checking facts against
// rules, we discover rules from facts. Per Halpin's CSDP:
//   Step 4: "Add uniqueness constraints and check arity of fact types"
//   -- look at the population, check which columns have duplicates.
//
// The induction engine implements:
//   - UC induction: which role combinations have unique values?
//   - MC induction: which roles are always populated?
//   - FC induction: which values occur a fixed number of times?
//   - SS induction: is one fact type's population a subset of another's?
//   - Pattern induction: can one fact type be derived from others?

use hashbrown::{HashMap, HashSet};
// types used transitively via crate::types::{FactTypeDef, RoleDef}
use crate::ast::{self, Object};
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

/// An induced constraint -- discovered from population analysis
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InducedConstraint {
    pub kind: String,
    pub fact_type_id: String,
    pub reading: String,
    pub roles: Vec<usize>,
    pub confidence: f64,
    pub evidence: String,
}

/// An induced derivation rule -- discovered from population patterns
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InducedRule {
    pub text: String,
    pub antecedent_fact_type_ids: Vec<String>,
    pub consequent_fact_type_id: String,
    pub confidence: f64,
    pub evidence: String,
}

/// Complete induction result
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InductionResult {
    pub constraints: Vec<InducedConstraint>,
    pub rules: Vec<InducedRule>,
    pub population_stats: PopulationStats,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PopulationStats {
    pub fact_type_count: usize,
    pub total_facts: usize,
    pub entity_count: usize,
}

/// Induce constraints and rules from an Object state and its schema.
pub fn induce_state(state: &Object) -> InductionResult {
    // Build local fact_types map from state cells (no Domain needed).
    let role_cell = ast::fetch_or_phi("Role", state);
    let fact_types: HashMap<String, crate::types::FactTypeDef> = ast::fetch_or_phi("FactType", state).as_seq()
        .map(|facts| facts.iter().filter_map(|f| {
            let id = ast::binding(f, "id")?.to_string();
            let reading = ast::binding(f, "reading").unwrap_or("").to_string();
            let roles: Vec<crate::types::RoleDef> = role_cell.as_seq()
                .map(|rs| rs.iter()
                    .filter(|r| ast::binding(r, "factType") == Some(&id))
                    .map(|r| crate::types::RoleDef {
                        noun_name: ast::binding(r, "nounName").unwrap_or("").to_string(),
                        role_index: ast::binding(r, "position").and_then(|v| v.parse().ok()).unwrap_or(0),
                    }).collect()).unwrap_or_default();
            Some((id, crate::types::FactTypeDef { schema_id: String::new(), reading, readings: vec![], roles }))
        }).collect()).unwrap_or_default();

    let mut constraints = Vec::new();
    let mut rules = Vec::new();

    // Gather all cells (fact_type_id -> facts)
    let cells: Vec<(String, Vec<Vec<(String, String)>>)> = ast::cells_iter(state).into_iter().map(|(ft_id, contents)| {
        let facts: Vec<Vec<(String, String)>> = contents.as_seq().map(|fact_objs| {
            fact_objs.iter().map(|fact| {
                fact.as_seq().map(|pairs| {
                    pairs.iter().filter_map(|pair| {
                        let items = pair.as_seq()?;
                        Some((items.get(0)?.as_atom()?.to_string(), items.get(1)?.as_atom()?.to_string()))
                    }).collect::<Vec<(String, String)>>()
                }).unwrap_or_default()
            }).collect()
        }).unwrap_or_default();
        (ft_id.to_string(), facts)
    }).collect();

    // Population statistics
    let total_facts: usize = cells.iter().map(|(_, facts)| facts.len()).sum();
    let entities: HashSet<String> = cells.iter()
        .flat_map(|(_, facts)| facts.iter().flat_map(|f| f.iter().map(|(_, v)| v.clone())))
        .collect();

    let ft_ref = &fact_types;

    // -- UC Induction -------------------------------------------------
    let uc_new: Vec<InducedConstraint> = cells.iter()
        .filter(|(_, facts)| !facts.is_empty())
        .filter_map(|(ft_id, facts)| ft_ref.get(ft_id).map(|ft| (ft_id, facts, ft)))
        .flat_map(|(ft_id, facts, ft)| {
            let arity = ft.roles.len();

            // Single-role uniqueness + FC
            let single: Vec<InducedConstraint> = (0..arity).flat_map(|role_idx| {
                let values: HashMap<String, usize> = facts.iter()
                    .filter_map(|fact| fact.get(role_idx).map(|(_, val)| val.clone()))
                    .fold(HashMap::new(), |mut m, val| { *m.entry(val).or_insert(0) += 1; m });

                let max_count = values.values().max().copied().unwrap_or(0);

                let uc_constraint = (max_count <= 1 && facts.len() > 1).then(|| InducedConstraint {
                    kind: "UC".to_string(),
                    fact_type_id: ft_id.clone(),
                    reading: ft.reading.clone(),
                    roles: vec![role_idx],
                    confidence: if facts.len() >= 3 { 0.9 } else { 0.6 },
                    evidence: format!(
                        "All {} values in role {} are unique across {} facts",
                        values.len(), ft.roles.get(role_idx).map(|r| r.noun_name.as_str()).unwrap_or("?"), facts.len()
                    ),
                });

                // FC induction
                let counts: HashSet<usize> = values.values().cloned().collect();
                let fc_constraint = (counts.len() == 1 && facts.len() > 2)
                    .then(|| counts.iter().next().copied())
                    .flatten()
                    .filter(|n| *n > 1)
                    .map(|n| InducedConstraint {
                        kind: "FC".to_string(),
                        fact_type_id: ft_id.clone(),
                        reading: ft.reading.clone(),
                        roles: vec![role_idx],
                        confidence: if facts.len() >= 4 { 0.8 } else { 0.5 },
                        evidence: format!(
                            "Every {} value occurs exactly {} times across {} facts",
                            ft.roles.get(role_idx).map(|r| r.noun_name.as_str()).unwrap_or("?"), n, facts.len()
                        ),
                    });

                uc_constraint.into_iter().chain(fc_constraint)
            }).collect();

            // Check compound uniqueness (pair of roles)
            let compound: Vec<InducedConstraint> = (arity >= 2).then(|| {
                (0..arity).flat_map(|i| {
                    ((i+1)..arity).filter_map(|j| {
                        let is_unique = facts.iter().try_fold(
                            HashSet::<(String, String)>::new(),
                            |mut tuples, fact| {
                                let a = fact.get(i).map(|(_, v)| v.clone()).unwrap_or_default();
                                let b = fact.get(j).map(|(_, v)| v.clone()).unwrap_or_default();
                                if tuples.insert((a, b)) { Some(tuples) } else { None }
                            }
                        ).is_some();

                        (is_unique && facts.len() > 1).then_some(())?;
                        let role_i_unique = single.iter().any(|c|
                            c.kind == "UC" && c.fact_type_id == *ft_id && c.roles == vec![i]
                        );
                        let role_j_unique = single.iter().any(|c|
                            c.kind == "UC" && c.fact_type_id == *ft_id && c.roles == vec![j]
                        );
                        (!role_i_unique && !role_j_unique).then_some(())?;
                        Some(InducedConstraint {
                            kind: "UC".to_string(),
                            fact_type_id: ft_id.clone(),
                            reading: ft.reading.clone(),
                            roles: vec![i, j],
                            confidence: if facts.len() >= 3 { 0.85 } else { 0.5 },
                            evidence: format!(
                                "All ({}, {}) combinations are unique across {} facts",
                                ft.roles.get(i).map(|r| r.noun_name.as_str()).unwrap_or("?"),
                                ft.roles.get(j).map(|r| r.noun_name.as_str()).unwrap_or("?"),
                                facts.len()
                            ),
                        })
                    }).collect::<Vec<_>>()
                }).collect()
            }).unwrap_or_default();

            single.into_iter().chain(compound)
        })
        .collect();
    constraints.extend(uc_new);

    // -- MC Induction -------------------------------------------------
    let mc_new: Vec<InducedConstraint> = cells.iter()
        .filter_map(|(ft_id, facts)| ft_ref.get(ft_id).map(|ft| (ft_id, facts, ft)))
        .filter(|(_, _, ft)| ft.roles.len() >= 2)
        .filter_map(|(ft_id, facts, ft)| {
            let role0_noun = &ft.roles[0].noun_name;
            let role0_values: HashSet<String> = facts.iter()
                .filter_map(|f| f.first().map(|(_, v)| v.clone()))
                .collect();

            let all_instances: HashSet<String> = cells.iter()
                .flat_map(|(_, fts)| fts.iter().flat_map(|f|
                    f.iter()
                        .filter(|(n, _)| n == role0_noun)
                        .map(|(_, v)| v.clone())
                ))
                .collect();

            if !all_instances.is_empty() && role0_values == all_instances && all_instances.len() > 1 {
                Some(InducedConstraint {
                    kind: "MC".to_string(),
                    fact_type_id: ft_id.clone(),
                    reading: ft.reading.clone(),
                    roles: vec![0],
                    confidence: if all_instances.len() >= 3 { 0.8 } else { 0.5 },
                    evidence: format!(
                        "All {} known {} instances participate in '{}'",
                        all_instances.len(), role0_noun, ft.reading
                    ),
                })
            } else {
                None
            }
        })
        .collect();
    constraints.extend(mc_new);

    // -- SS Induction -------------------------------------------------
    let ft_entities: HashMap<String, HashSet<String>> = cells.iter()
        .filter_map(|(ft_id, facts)| {
            let ft = ft_ref.get(ft_id).filter(|ft| !ft.roles.is_empty())?;
            let _ = ft;
            let vals: HashSet<String> = facts.iter()
                .filter_map(|f| f.first().map(|(_, v)| v.clone()))
                .collect();
            Some((ft_id.clone(), vals))
        })
        .collect();

    let ft_ids: Vec<String> = ft_entities.keys().cloned().collect();
    let ft_ent_ref = &ft_entities;
    let ft_ids_ref = &ft_ids;
    let ss_new: Vec<InducedConstraint> = (0..ft_ids.len())
        .flat_map(|i| (0..ft_ids_ref.len()).filter_map(move |j| {
            (i != j).then_some(())?;
            let a = &ft_ent_ref[&ft_ids_ref[i]];
            let b = &ft_ent_ref[&ft_ids_ref[j]];
            if !a.is_empty() && a.is_subset(b) && a != b {
                let reading_a = ft_ref.get(&ft_ids_ref[i]).map(|f| f.reading.as_str()).unwrap_or("?");
                let reading_b = ft_ref.get(&ft_ids_ref[j]).map(|f| f.reading.as_str()).unwrap_or("?");
                Some(InducedConstraint {
                    kind: "SS".to_string(),
                    fact_type_id: ft_ids_ref[i].clone(),
                    reading: format!("pop('{}') subset_of pop('{}')", reading_a, reading_b),
                    roles: vec![0],
                    confidence: 0.7,
                    evidence: format!(
                        "All {} entities in '{}' also appear in '{}'",
                        a.len(), reading_a, reading_b
                    ),
                })
            } else {
                None
            }
        }))
        .collect();
    constraints.extend(ss_new);

    // -- Derivation Rule Induction ------------------------------------
    let new_rules: Vec<InducedRule> = cells.iter()
        .filter_map(|(ft_id, facts)| ft_ref.get(ft_id).map(|ft| (ft_id, facts, ft)))
        .filter(|(_, facts, ft)| ft.roles.len() >= 2 && facts.len() >= 2)
        .flat_map(|(ft_id, facts, ft)| {
            let target: HashSet<(String, String)> = facts.iter()
                .filter(|f| f.len() >= 2)
                .map(|f| (f[0].1.clone(), f[1].1.clone()))
                .collect();

            cells.iter()
                .filter_map(|(other_a_id, other_a_facts)| {
                    (other_a_id != ft_id).then_some(())?;
                    ft_ref.get(other_a_id).map(|ft_a| (other_a_id, other_a_facts, ft_a))
                })
                .flat_map(|(other_a_id, other_a_facts, other_a_ft)| {
                    cells.iter()
                        .filter_map(|(other_b_id, other_b_facts)| {
                            (other_b_id != ft_id && other_b_id != other_a_id).then_some(())?;
                            ft_ref.get(other_b_id).map(|ft_b| (other_b_id, other_b_facts, ft_b))
                        })
                        .filter_map(|(other_b_id, other_b_facts, other_b_ft)| {
                            let a_nouns: HashSet<&str> = other_a_ft.roles.iter().map(|r| r.noun_name.as_str()).collect();
                            let b_nouns: HashSet<&str> = other_b_ft.roles.iter().map(|r| r.noun_name.as_str()).collect();
                            let common: Vec<&str> = a_nouns.intersection(&b_nouns).cloned().collect();
                            (!common.is_empty()).then_some(())?;

                            let join_noun = common[0];
                            let joined: HashSet<(String, String)> = other_a_facts.iter()
                                .filter_map(|a_fact| {
                                    let a_join_val = a_fact.iter().find(|(n, _)| n == join_noun).map(|(_, v)| v)?;
                                    let a_other_val = a_fact.iter().find(|(n, _)| n != join_noun).map(|(_, v)| v)?;
                                    Some((a_join_val, a_other_val))
                                })
                                .flat_map(|(jv, av)| {
                                    other_b_facts.iter()
                                        .filter_map(|b_fact| {
                                            let b_join_val = b_fact.iter().find(|(n, _)| n == join_noun).map(|(_, v)| v)?;
                                            let b_other_val = b_fact.iter().find(|(n, _)| n != join_noun).map(|(_, v)| v)?;
                                            if jv == b_join_val {
                                                Some((av.clone(), b_other_val.clone()))
                                            } else {
                                                None
                                            }
                                        })
                                        .collect::<Vec<_>>()
                                })
                                .collect();

                            if !joined.is_empty() && joined == target {
                                Some(InducedRule {
                                    text: format!(
                                        "{} := {} and {}",
                                        ft.reading, other_a_ft.reading, other_b_ft.reading
                                    ),
                                    antecedent_fact_type_ids: vec![other_a_id.clone(), other_b_id.clone()],
                                    consequent_fact_type_id: ft_id.clone(),
                                    confidence: 0.9,
                                    evidence: format!(
                                        "Joining '{}' and '{}' on {} produces exactly the {} facts in '{}'",
                                        other_a_ft.reading, other_b_ft.reading, join_noun,
                                        target.len(), ft.reading
                                    ),
                                })
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect();
    rules.extend(new_rules);

    InductionResult {
        constraints,
        rules,
        population_stats: PopulationStats {
            fact_type_count: cells.len(),
            total_facts,
            entity_count: entities.len(),
        },
    }
}
