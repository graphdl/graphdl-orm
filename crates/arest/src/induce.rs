// crates/arest/src/induce.rs
//
// Induction engine: given a population of facts, infer the constraints
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

use std::collections::{HashMap, HashSet};
use crate::types::*;

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
pub fn induce_state(ir: &Domain, state: &crate::ast::Object) -> InductionResult {
    induce(ir, &crate::ast::state_to_population(state))
}

/// Induce constraints and rules from a population and its schema.
pub fn induce(ir: &Domain, population: &Population) -> InductionResult {
    let mut constraints = Vec::new();
    let mut rules = Vec::new();

    // Gather population statistics
    let total_facts: usize = population.facts.values().map(|f| f.len()).sum();
    let entities: HashSet<String> = population.facts.values()
        .flat_map(|facts| facts.iter().flat_map(|f| f.bindings.iter().map(|(_, v)| v.clone())))
        .collect();

    // -- UC Induction -------------------------------------------------
    // For each fact type, check which role combinations have unique values.
    for (ft_id, facts) in &population.facts {
        if facts.is_empty() { continue; }
        let ft = match ir.fact_types.get(ft_id) {
            Some(ft) => ft,
            None => continue,
        };
        let arity = ft.roles.len();

        // Check single-role uniqueness
        for role_idx in 0..arity {
            let mut values: HashMap<String, usize> = HashMap::new();
            for fact in facts {
                if let Some((_, val)) = fact.bindings.get(role_idx) {
                    *values.entry(val.clone()).or_insert(0) += 1;
                }
            }
            let max_count = values.values().max().copied().unwrap_or(0);
            if max_count <= 1 && facts.len() > 1 {
                constraints.push(InducedConstraint {
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
            }

            // FC induction: check if values occur a fixed number of times
            let counts: HashSet<usize> = values.values().cloned().collect();
            if counts.len() == 1 && facts.len() > 2 {
                let n = *counts.iter().next().unwrap();
                if n > 1 {
                    constraints.push(InducedConstraint {
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
                }
            }
        }

        // Check compound uniqueness (pair of roles)
        if arity >= 2 {
            for i in 0..arity {
                for j in (i+1)..arity {
                    let mut tuples: HashSet<(String, String)> = HashSet::new();
                    let mut is_unique = true;
                    for fact in facts {
                        let a = fact.bindings.get(i).map(|(_, v)| v.clone()).unwrap_or_default();
                        let b = fact.bindings.get(j).map(|(_, v)| v.clone()).unwrap_or_default();
                        if !tuples.insert((a, b)) {
                            is_unique = false;
                            break;
                        }
                    }
                    if is_unique && facts.len() > 1 {
                        // Only report compound UC if neither role is individually unique
                        let role_i_unique = constraints.iter().any(|c|
                            c.kind == "UC" && c.fact_type_id == *ft_id && c.roles == vec![i]
                        );
                        let role_j_unique = constraints.iter().any(|c|
                            c.kind == "UC" && c.fact_type_id == *ft_id && c.roles == vec![j]
                        );
                        if !role_i_unique && !role_j_unique {
                            constraints.push(InducedConstraint {
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
                            });
                        }
                    }
                }
            }
        }
    }

    // -- MC Induction -------------------------------------------------
    // For each entity that participates in ANY fact type, check if it
    // participates in ALL instances of other fact types.
    for (ft_id, facts) in &population.facts {
        let ft = match ir.fact_types.get(ft_id) {
            Some(ft) => ft,
            None => continue,
        };
        if ft.roles.len() < 2 { continue; }

        // Collect all entity values for the first role
        let role0_noun = &ft.roles[0].noun_name;
        let role0_values: HashSet<String> = facts.iter()
            .filter_map(|f| f.bindings.first().map(|(_, v)| v.clone()))
            .collect();

        // Check if ALL known instances of this noun participate in this fact type
        let all_instances: HashSet<String> = population.facts.values()
            .flat_map(|fts| fts.iter().flat_map(|f|
                f.bindings.iter()
                    .filter(|(n, _)| n == role0_noun)
                    .map(|(_, v)| v.clone())
            ))
            .collect();

        if !all_instances.is_empty() && role0_values == all_instances && all_instances.len() > 1 {
            constraints.push(InducedConstraint {
                kind: "MC".to_string(),
                fact_type_id: ft_id.clone(),
                reading: ft.reading.clone(),
                roles: vec![0],
                confidence: if all_instances.len() >= 3 { 0.8 } else { 0.5 },
                evidence: format!(
                    "All {} known {} instances participate in '{}'",
                    all_instances.len(), role0_noun, ft.reading
                ),
            });
        }
    }

    // -- SS Induction -------------------------------------------------
    // Check if one fact type's entity population is a subset of another's.
    let ft_entities: HashMap<String, HashSet<String>> = population.facts.iter()
        .filter_map(|(ft_id, facts)| {
            let ft = ir.fact_types.get(ft_id)?;
            if ft.roles.is_empty() { return None; }
            let _noun = &ft.roles[0].noun_name;
            let vals: HashSet<String> = facts.iter()
                .filter_map(|f| f.bindings.first().map(|(_, v)| v.clone()))
                .collect();
            Some((ft_id.clone(), vals))
        })
        .collect();

    let ft_ids: Vec<String> = ft_entities.keys().cloned().collect();
    for i in 0..ft_ids.len() {
        for j in 0..ft_ids.len() {
            if i == j { continue; }
            let a = &ft_entities[&ft_ids[i]];
            let b = &ft_entities[&ft_ids[j]];
            if !a.is_empty() && a.is_subset(b) && a != b {
                let reading_a = ir.fact_types.get(&ft_ids[i]).map(|f| f.reading.as_str()).unwrap_or("?");
                let reading_b = ir.fact_types.get(&ft_ids[j]).map(|f| f.reading.as_str()).unwrap_or("?");
                constraints.push(InducedConstraint {
                    kind: "SS".to_string(),
                    fact_type_id: ft_ids[i].clone(),
                    reading: format!("pop('{}') subset_of pop('{}')", reading_a, reading_b),
                    roles: vec![0],
                    confidence: 0.7,
                    evidence: format!(
                        "All {} entities in '{}' also appear in '{}'",
                        a.len(), reading_a, reading_b
                    ),
                });
            }
        }
    }

    // -- Derivation Rule Induction ------------------------------------
    // Check if any fact type's population can be fully explained by
    // joining other fact types.
    for (ft_id, facts) in &population.facts {
        let ft = match ir.fact_types.get(ft_id) {
            Some(ft) => ft,
            None => continue,
        };
        if ft.roles.len() < 2 || facts.len() < 2 { continue; }

        // For each pair of other fact types, check if joining them
        // on a common noun produces this fact type's population
        for (other_a_id, other_a_facts) in &population.facts {
            if other_a_id == ft_id { continue; }
            let other_a_ft = match ir.fact_types.get(other_a_id) {
                Some(f) => f,
                None => continue,
            };

            for (other_b_id, other_b_facts) in &population.facts {
                if other_b_id == ft_id || other_b_id == other_a_id { continue; }
                let other_b_ft = match ir.fact_types.get(other_b_id) {
                    Some(f) => f,
                    None => continue,
                };

                // Find common noun between other_a and other_b
                let a_nouns: HashSet<&str> = other_a_ft.roles.iter().map(|r| r.noun_name.as_str()).collect();
                let b_nouns: HashSet<&str> = other_b_ft.roles.iter().map(|r| r.noun_name.as_str()).collect();
                let common: Vec<&str> = a_nouns.intersection(&b_nouns).cloned().collect();

                if common.is_empty() { continue; }

                // Join on common noun -- does the result match our target?
                let join_noun = common[0];
                let mut joined: HashSet<(String, String)> = HashSet::new();
                for a_fact in other_a_facts {
                    let a_join_val = a_fact.bindings.iter().find(|(n, _)| n == join_noun).map(|(_, v)| v);
                    let a_other_val = a_fact.bindings.iter().find(|(n, _)| n != join_noun).map(|(_, v)| v);
                    if let (Some(jv), Some(av)) = (a_join_val, a_other_val) {
                        for b_fact in other_b_facts {
                            let b_join_val = b_fact.bindings.iter().find(|(n, _)| n == join_noun).map(|(_, v)| v);
                            let b_other_val = b_fact.bindings.iter().find(|(n, _)| n != join_noun).map(|(_, v)| v);
                            if let (Some(bjv), Some(bv)) = (b_join_val, b_other_val) {
                                if jv == bjv {
                                    joined.insert((av.clone(), bv.clone()));
                                }
                            }
                        }
                    }
                }

                // Check if joined results match the target fact type
                let target: HashSet<(String, String)> = facts.iter()
                    .filter(|f| f.bindings.len() >= 2)
                    .map(|f| (f.bindings[0].1.clone(), f.bindings[1].1.clone()))
                    .collect();

                if !joined.is_empty() && joined == target {
                    rules.push(InducedRule {
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
                    });
                }
            }
        }
    }

    InductionResult {
        constraints,
        rules,
        population_stats: PopulationStats {
            fact_type_count: population.facts.len(),
            total_facts,
            entity_count: entities.len(),
        },
    }
}
