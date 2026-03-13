// crates/constraint-eval/src/evaluate.rs
use crate::types::*;
use std::collections::{HashMap, HashSet};

/// Evaluate all constraints in the IR against a response and population.
pub fn evaluate(
    ir: &ConstraintIR,
    response: &ResponseContext,
    population: &Population,
) -> Vec<Violation> {
    let mut violations = Vec::new();

    for constraint in &ir.constraints {
        match constraint.modality.as_str() {
            "Deontic" => {
                violations.extend(evaluate_deontic(ir, constraint, response));
            }
            "Alethic" => {
                violations.extend(evaluate_alethic(ir, constraint, population));
            }
            _ => {}
        }
    }

    violations
}

// ── Deontic evaluation (text-based) ──────────────────────────────────

fn evaluate_deontic(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    response: &ResponseContext,
) -> Vec<Violation> {
    let operator = constraint.deontic_operator.as_deref().unwrap_or("");
    let text = &response.text;

    match operator {
        "forbidden" => evaluate_forbidden(ir, constraint, text),
        "obligatory" => evaluate_obligatory(ir, constraint, text, response),
        "permitted" => vec![], // Permitted constraints never produce violations
        _ => vec![],
    }
}

fn evaluate_forbidden(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    response_text: &str,
) -> Vec<Violation> {
    // For forbidden constraints, check if the response contains any of the
    // constrained noun's enum values. The constraint text tells us what pattern
    // is forbidden; the noun's enum_values list the specific forbidden values.
    //
    // Example: "It is forbidden that SupportResponse contains ProhibitedText"
    // ProhibitedText.enumValues = ['—', '–', '--']
    // Check if response_text contains any of those values.

    let mut violations = Vec::new();
    let mut seen = HashSet::new();
    let lower_text = response_text.to_lowercase();

    // For each span, find the associated fact type and check all value-type nouns
    // in that fact type for forbidden enum values.
    for span in &constraint.spans {
        let fact_type = match ir.fact_types.get(&span.fact_type_id) {
            Some(ft) => ft,
            None => continue,
        };

        for role in &fact_type.roles {
            if let Some(noun_def) = ir.nouns.get(&role.noun_name) {
                // Only check value-type nouns (entity nouns don't have textual enum values)
                if noun_def.object_type != "value" { continue; }

                if let Some(enum_values) = &noun_def.enum_values {
                    for val in enum_values {
                        let lower_val = val.to_lowercase();
                        if lower_text.contains(&lower_val) {
                            let detail = format!(
                                "Response contains forbidden {}: '{}'",
                                role.noun_name, val
                            );
                            if seen.insert(detail.clone()) {
                                violations.push(Violation {
                                    constraint_id: constraint.id.clone(),
                                    constraint_text: constraint.text.clone(),
                                    detail,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    violations
}

fn evaluate_obligatory(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    response_text: &str,
    response: &ResponseContext,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    let lower_text = response_text.to_lowercase();

    // Check for obligatory field presence
    // Example: "It is obligatory that each SupportResponse has SenderIdentity 'Auto.dev Team <team@auto.dev>'"
    // Look for enum values in the object noun that MUST appear

    for span in &constraint.spans {
        let fact_type = match ir.fact_types.get(&span.fact_type_id) {
            Some(ft) => ft,
            None => continue,
        };

        for role in &fact_type.roles {
            if let Some(noun_def) = ir.nouns.get(&role.noun_name) {
                if noun_def.object_type == "value" {
                    if let Some(enum_values) = &noun_def.enum_values {
                        // For obligatory, at least one enum value must appear
                        let found = enum_values.iter().any(|val| {
                            lower_text.contains(&val.to_lowercase())
                        });
                        if !found && !enum_values.is_empty() {
                            violations.push(Violation {
                                constraint_id: constraint.id.clone(),
                                constraint_text: constraint.text.clone(),
                                detail: format!(
                                    "Response missing obligatory {}: expected one of {:?}",
                                    role.noun_name, enum_values
                                ),
                            });
                        }
                    }
                }
            }
        }
    }

    // Check sender identity if specified
    if let Some(sender) = &response.sender_identity {
        if constraint.text.to_lowercase().contains("senderidentity") && sender.is_empty() {
            violations.push(Violation {
                constraint_id: constraint.id.clone(),
                constraint_text: constraint.text.clone(),
                detail: "Response missing obligatory SenderIdentity".to_string(),
            });
        }
    }

    violations
}

// ── Alethic evaluation (structural) ──────────────────────────────────

fn evaluate_alethic(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    match constraint.kind.as_str() {
        "UC" => evaluate_uniqueness(ir, constraint, population),
        "MC" => evaluate_mandatory(ir, constraint, population),
        "RC" => evaluate_ring(ir, constraint, population),
        "XO" => evaluate_exclusive_or(ir, constraint, population),
        "XC" => evaluate_exclusive_choice(ir, constraint, population),
        "OR" => evaluate_inclusive_or(ir, constraint, population),
        "SS" => evaluate_subset(ir, constraint, population),
        "EQ" => evaluate_equality(ir, constraint, population),
        _ => vec![],
    }
}

fn evaluate_uniqueness(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    let mut violations = Vec::new();

    for span in &constraint.spans {
        let facts = match population.facts.get(&span.fact_type_id) {
            Some(f) => f,
            None => continue,
        };

        let fact_type = match ir.fact_types.get(&span.fact_type_id) {
            Some(ft) => ft,
            None => continue,
        };

        let role = match fact_type.roles.get(span.role_index) {
            Some(r) => r,
            None => continue,
        };

        // Group facts by the spanned role's binding value
        let mut seen: HashMap<String, usize> = HashMap::new();
        for fact in facts {
            if let Some((_, val)) = fact.bindings.iter().find(|(name, _)| name == &role.noun_name) {
                *seen.entry(val.clone()).or_insert(0) += 1;
            }
        }

        for (val, count) in &seen {
            if *count > 1 {
                violations.push(Violation {
                    constraint_id: constraint.id.clone(),
                    constraint_text: constraint.text.clone(),
                    detail: format!(
                        "Uniqueness violation: {} '{}' appears {} times in {}",
                        role.noun_name, val, count, fact_type.reading
                    ),
                });
            }
        }
    }

    violations
}

fn evaluate_mandatory(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    // For each entity instance of the subject noun,
    // check that it participates in at least one fact of the spanned type
    let mut violations = Vec::new();

    for span in &constraint.spans {
        let fact_type = match ir.fact_types.get(&span.fact_type_id) {
            Some(ft) => ft,
            None => continue,
        };

        let role = match fact_type.roles.get(span.role_index) {
            Some(r) => r,
            None => continue,
        };

        let facts = population.facts.get(&span.fact_type_id).cloned().unwrap_or_default();

        // Collect all entity instances of this noun from all facts
        let mut all_instances: HashSet<String> = HashSet::new();
        for (_, fact_list) in &population.facts {
            for fact in fact_list {
                for (name, val) in &fact.bindings {
                    if name == &role.noun_name {
                        all_instances.insert(val.clone());
                    }
                }
            }
        }

        // Check each instance participates in this fact type
        for instance in &all_instances {
            let participates = facts.iter().any(|f| {
                f.bindings.iter().any(|(name, val)| name == &role.noun_name && val == instance)
            });
            if !participates {
                violations.push(Violation {
                    constraint_id: constraint.id.clone(),
                    constraint_text: constraint.text.clone(),
                    detail: format!(
                        "Mandatory violation: {} '{}' does not participate in {}",
                        role.noun_name, instance, fact_type.reading
                    ),
                });
            }
        }
    }

    violations
}

fn evaluate_ring(
    _ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    let mut violations = Vec::new();

    for span in &constraint.spans {
        let facts = match population.facts.get(&span.fact_type_id) {
            Some(f) => f,
            None => continue,
        };

        // Ring irreflexive: no fact should have the same value for both roles
        for fact in facts {
            if fact.bindings.len() >= 2 {
                let first = &fact.bindings[0].1;
                let second = &fact.bindings[1].1;
                if first == second {
                    violations.push(Violation {
                        constraint_id: constraint.id.clone(),
                        constraint_text: constraint.text.clone(),
                        detail: format!(
                            "Ring constraint violation: '{}' references itself",
                            first
                        ),
                    });
                }
            }
        }
    }

    violations
}

fn evaluate_exclusive_or(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    // XO: For each entity, exactly one of the clause fact types holds
    evaluate_set_comparison(ir, constraint, population, |count| count != 1, "exactly one")
}

fn evaluate_exclusive_choice(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    // XC: For each entity, at most one of the clause fact types holds
    evaluate_set_comparison(ir, constraint, population, |count| count > 1, "at most one")
}

fn evaluate_inclusive_or(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    // OR: For each entity, at least one of the clause fact types holds
    evaluate_set_comparison(ir, constraint, population, |count| count < 1, "at least one")
}

fn evaluate_set_comparison(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
    violates: impl Fn(usize) -> bool,
    requirement: &str,
) -> Vec<Violation> {
    let mut violations = Vec::new();

    // Get the entity noun name
    let entity_name = match &constraint.entity {
        Some(name) => name.clone(),
        None => return violations,
    };

    // Collect all entity instances
    let mut instances: HashSet<String> = HashSet::new();
    for (_, facts) in &population.facts {
        for fact in facts {
            for (name, val) in &fact.bindings {
                if name == &entity_name {
                    instances.insert(val.clone());
                }
            }
        }
    }

    // For each instance, count how many clause fact types hold
    let clause_fact_type_ids: Vec<String> = constraint.spans.iter()
        .map(|s| s.fact_type_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    for instance in &instances {
        let mut holding_count = 0;
        for ft_id in &clause_fact_type_ids {
            if let Some(facts) = population.facts.get(ft_id) {
                let holds = facts.iter().any(|f| {
                    f.bindings.iter().any(|(name, val)| name == &entity_name && val == instance)
                });
                if holds {
                    holding_count += 1;
                }
            }
        }

        if violates(holding_count) {
            violations.push(Violation {
                constraint_id: constraint.id.clone(),
                constraint_text: constraint.text.clone(),
                detail: format!(
                    "Set-comparison violation: {} '{}' has {} of {} clause fact types holding, expected {}",
                    entity_name, instance, holding_count, clause_fact_type_ids.len(), requirement
                ),
            });
        }
    }

    violations
}

fn evaluate_subset(
    ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    let mut violations = Vec::new();

    // SS: If fact type A holds for entity X, then fact type B must also hold for X
    if constraint.spans.len() < 2 {
        return violations;
    }

    let a_ft_id = &constraint.spans[0].fact_type_id;
    let b_ft_id = &constraint.spans[1].fact_type_id;

    // Get the entity noun name from the first span's role
    let entity_name = ir.fact_types.get(a_ft_id)
        .and_then(|ft| ft.roles.get(constraint.spans[0].role_index))
        .map(|r| r.noun_name.clone())
        .unwrap_or_default();

    let a_facts = population.facts.get(a_ft_id).cloned().unwrap_or_default();
    let b_facts = population.facts.get(b_ft_id).cloned().unwrap_or_default();

    for a_fact in &a_facts {
        // Use name-based lookup instead of positional index
        if let Some((_, entity_val)) = a_fact.bindings.iter().find(|(name, _)| name == &entity_name) {
            let b_holds = b_facts.iter().any(|bf| {
                bf.bindings.iter().any(|(_, val)| val == entity_val)
            });
            if !b_holds {
                violations.push(Violation {
                    constraint_id: constraint.id.clone(),
                    constraint_text: constraint.text.clone(),
                    detail: format!(
                        "Subset violation: entity '{}' has fact A but not fact B",
                        entity_val
                    ),
                });
            }
        }
    }

    violations
}

fn evaluate_equality(
    _ir: &ConstraintIR,
    constraint: &ConstraintDef,
    population: &Population,
) -> Vec<Violation> {
    let mut violations = Vec::new();

    // EQ: A holds iff B holds (bidirectional subset)
    if constraint.spans.len() < 2 {
        return violations;
    }

    let a_ft_id = &constraint.spans[0].fact_type_id;
    let b_ft_id = &constraint.spans[1].fact_type_id;

    let a_facts = population.facts.get(a_ft_id).cloned().unwrap_or_default();
    let b_facts = population.facts.get(b_ft_id).cloned().unwrap_or_default();

    // Collect entity values from A
    let a_entities: HashSet<String> = a_facts.iter()
        .flat_map(|f| f.bindings.iter().map(|(_, v)| v.clone()))
        .collect();

    let b_entities: HashSet<String> = b_facts.iter()
        .flat_map(|f| f.bindings.iter().map(|(_, v)| v.clone()))
        .collect();

    // A → B
    for entity in a_entities.difference(&b_entities) {
        violations.push(Violation {
            constraint_id: constraint.id.clone(),
            constraint_text: constraint.text.clone(),
            detail: format!("Equality violation: '{}' has fact A but not fact B", entity),
        });
    }

    // B → A
    for entity in b_entities.difference(&a_entities) {
        violations.push(Violation {
            constraint_id: constraint.id.clone(),
            constraint_text: constraint.text.clone(),
            detail: format!("Equality violation: '{}' has fact B but not fact A", entity),
        });
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_ir() -> ConstraintIR {
        ConstraintIR {
            domain: "test".to_string(),
            nouns: HashMap::new(),
            fact_types: HashMap::new(),
            constraints: vec![],
            state_machines: HashMap::new(),
        }
    }

    fn empty_response() -> ResponseContext {
        ResponseContext {
            text: String::new(),
            sender_identity: None,
            fields: None,
        }
    }

    fn empty_population() -> Population {
        Population { facts: HashMap::new() }
    }

    #[test]
    fn test_no_constraints_no_violations() {
        let ir = empty_ir();
        let result = evaluate(&ir, &empty_response(), &empty_population());
        assert!(result.is_empty());
    }

    #[test]
    fn test_forbidden_text_detected() {
        let mut ir = empty_ir();
        ir.nouns.insert("ProhibitedText".to_string(), NounDef {
            object_type: "value".to_string(),
            enum_values: Some(vec!["—".to_string(), "–".to_string()]),
            value_type: Some("string".to_string()),
            super_type: None,
        });
        ir.nouns.insert("SupportResponse".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None,
            value_type: None,
            super_type: None,
        });
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "SupportResponse contains ProhibitedText".to_string(),
            roles: vec![
                RoleDef { noun_name: "SupportResponse".to_string(), role_index: 0 },
                RoleDef { noun_name: "ProhibitedText".to_string(), role_index: 1 },
            ],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("forbidden".to_string()),
            text: "It is forbidden that SupportResponse contains ProhibitedText".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
        });

        let response = ResponseContext {
            text: "Hello — how can I help?".to_string(),
            sender_identity: None,
            fields: None,
        };

        let result = evaluate(&ir, &response, &empty_population());
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("forbidden"));
        assert!(result[0].detail.contains("—"));
    }

    #[test]
    fn test_forbidden_text_clean() {
        let mut ir = empty_ir();
        ir.nouns.insert("ProhibitedText".to_string(), NounDef {
            object_type: "value".to_string(),
            enum_values: Some(vec!["—".to_string()]),
            value_type: Some("string".to_string()),
            super_type: None,
        });
        ir.nouns.insert("SupportResponse".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None,
            value_type: None,
            super_type: None,
        });
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "SupportResponse contains ProhibitedText".to_string(),
            roles: vec![
                RoleDef { noun_name: "SupportResponse".to_string(), role_index: 0 },
                RoleDef { noun_name: "ProhibitedText".to_string(), role_index: 1 },
            ],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("forbidden".to_string()),
            text: "It is forbidden that SupportResponse contains ProhibitedText".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
        });

        let response = ResponseContext {
            text: "Hello, how can I help you today?".to_string(),
            sender_identity: None,
            fields: None,
        };

        let result = evaluate(&ir, &response, &empty_population());
        assert!(result.is_empty());
    }

    #[test]
    fn test_uniqueness_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Customer has Name".to_string(),
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "Each Customer has at most one Name".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
        });

        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("Customer".to_string(), "c1".to_string()), ("Name".to_string(), "Alice".to_string())],
            },
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("Customer".to_string(), "c1".to_string()), ("Name".to_string(), "Bob".to_string())],
            },
        ]);
        let population = Population { facts };

        let result = evaluate(&ir, &empty_response(), &population);
        assert_eq!(result.len(), 1);
        assert!(result[0].detail.contains("Uniqueness violation"));
    }

    #[test]
    fn test_ring_irreflexive_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Person manages Person".to_string(),
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Person".to_string(), role_index: 1 },
            ],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "RC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "No Person manages itself".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
        });

        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("Person".to_string(), "p1".to_string()), ("Person".to_string(), "p1".to_string())],
            },
        ]);
        let population = Population { facts };

        let result = evaluate(&ir, &empty_response(), &population);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Ring constraint"));
    }

    #[test]
    fn test_exclusive_or_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Order isPaid".to_string(),
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            reading: "Order isPending".to_string(),
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "XO".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "For each Order, exactly one holds".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: Some(2),
            clauses: Some(vec!["Order isPaid".to_string(), "Order isPending".to_string()]),
            entity: Some("Order".to_string()),
        });

        // Order o1 has BOTH facts — violates exactly-one
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Order".to_string(), "o1".to_string())],
        }]);
        facts.insert("ft2".to_string(), vec![FactInstance {
            fact_type_id: "ft2".to_string(),
            bindings: vec![("Order".to_string(), "o1".to_string())],
        }]);
        let population = Population { facts };

        let result = evaluate(&ir, &empty_response(), &population);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Set-comparison violation"));
    }

    #[test]
    fn test_subset_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Person hasLicense".to_string(),
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            reading: "Person hasInsurance".to_string(),
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "SS".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "If some Person hasLicense then that Person hasInsurance".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
        });

        // Person p1 has license but no insurance — violates subset
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Person".to_string(), "p1".to_string())],
        }]);
        let population = Population { facts };

        let result = evaluate(&ir, &empty_response(), &population);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Subset violation"));
    }

    #[test]
    fn test_permitted_never_violates() {
        let mut ir = empty_ir();
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("permitted".to_string()),
            text: "It is permitted that SupportResponse offers data retrieval".to_string(),
            spans: vec![],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
        });

        let result = evaluate(&ir, &empty_response(), &empty_population());
        assert!(result.is_empty());
    }

    #[test]
    fn test_exclusive_choice_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Order isPaid".to_string(),
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            reading: "Order isPending".to_string(),
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "XC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "For each Order, at most one holds".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: Some(2),
            clauses: Some(vec!["Order isPaid".to_string(), "Order isPending".to_string()]),
            entity: Some("Order".to_string()),
        });

        // Order o1 has BOTH facts — violates at-most-one
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Order".to_string(), "o1".to_string())],
        }]);
        facts.insert("ft2".to_string(), vec![FactInstance {
            fact_type_id: "ft2".to_string(),
            bindings: vec![("Order".to_string(), "o1".to_string())],
        }]);
        let population = Population { facts };

        let result = evaluate(&ir, &empty_response(), &population);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Set-comparison violation"));
    }

    #[test]
    fn test_mandatory_violation() {
        let mut ir = empty_ir();
        ir.nouns.insert("Customer".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None,
            value_type: None,
            super_type: None,
        });
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Customer has Name".to_string(),
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            reading: "Customer has Email".to_string(),
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Email".to_string(), role_index: 1 },
            ],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "MC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "Each Customer has at least one Name".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
        });

        // Customer c1 appears in Email facts but NOT in Name facts → mandatory violation
        let mut facts = HashMap::new();
        facts.insert("ft2".to_string(), vec![FactInstance {
            fact_type_id: "ft2".to_string(),
            bindings: vec![("Customer".to_string(), "c1".to_string()), ("Email".to_string(), "a@b.com".to_string())],
        }]);
        let population = Population { facts };

        let result = evaluate(&ir, &empty_response(), &population);
        assert_eq!(result.len(), 1);
        assert!(result[0].detail.contains("Mandatory violation"));
        assert!(result[0].detail.contains("c1"));
    }

    #[test]
    fn test_inclusive_or_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Customer hasPhone".to_string(),
            roles: vec![RoleDef { noun_name: "Customer".to_string(), role_index: 0 }],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            reading: "Customer hasEmail".to_string(),
            roles: vec![RoleDef { noun_name: "Customer".to_string(), role_index: 0 }],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "OR".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "For each Customer, at least one of the following holds: hasPhone, hasEmail".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: Some(2),
            clauses: Some(vec!["Customer hasPhone".to_string(), "Customer hasEmail".to_string()]),
            entity: Some("Customer".to_string()),
        });

        // Customer c1 participates in neither fact type → violation
        // (c1 is known from a third unrelated fact type)
        ir.fact_types.insert("ft3".to_string(), FactTypeDef {
            reading: "Customer hasName".to_string(),
            roles: vec![RoleDef { noun_name: "Customer".to_string(), role_index: 0 }],
        });
        let mut facts = HashMap::new();
        facts.insert("ft3".to_string(), vec![FactInstance {
            fact_type_id: "ft3".to_string(),
            bindings: vec![("Customer".to_string(), "c1".to_string())],
        }]);
        let population = Population { facts };

        let result = evaluate(&ir, &empty_response(), &population);
        assert_eq!(result.len(), 1);
        assert!(result[0].detail.contains("Set-comparison violation"));
        assert!(result[0].detail.contains("at least one"));
    }

    #[test]
    fn test_obligatory_missing_enum_value() {
        let mut ir = empty_ir();
        ir.nouns.insert("SenderIdentityValue".to_string(), NounDef {
            object_type: "value".to_string(),
            enum_values: Some(vec!["Auto.dev Team <team@auto.dev>".to_string()]),
            value_type: Some("string".to_string()),
            super_type: None,
        });
        ir.nouns.insert("SupportResponse".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None,
            value_type: None,
            super_type: None,
        });
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "SupportResponse has SenderIdentityValue".to_string(),
            roles: vec![
                RoleDef { noun_name: "SupportResponse".to_string(), role_index: 0 },
                RoleDef { noun_name: "SenderIdentityValue".to_string(), role_index: 1 },
            ],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("obligatory".to_string()),
            text: "It is obligatory that each SupportResponse has SenderIdentity".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
        });

        // Response text does NOT contain the required sender identity value
        let response = ResponseContext {
            text: "Here is some help for you.".to_string(),
            sender_identity: Some(String::new()),
            fields: None,
        };

        let result = evaluate(&ir, &response, &empty_population());
        // Should get violation for missing enum value AND missing sender identity
        assert!(result.len() >= 1);
        let details: Vec<&str> = result.iter().map(|v| v.detail.as_str()).collect();
        assert!(details.iter().any(|d| d.contains("obligatory")));
    }

    #[test]
    fn test_obligatory_sender_identity_empty() {
        let mut ir = empty_ir();
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("obligatory".to_string()),
            text: "It is obligatory that each SupportResponse has SenderIdentity".to_string(),
            spans: vec![],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
        });

        // sender_identity present but empty string → violation
        let response = ResponseContext {
            text: "Hello".to_string(),
            sender_identity: Some(String::new()),
            fields: None,
        };

        let result = evaluate(&ir, &response, &empty_population());
        assert_eq!(result.len(), 1);
        assert!(result[0].detail.contains("SenderIdentity"));
    }

    #[test]
    fn test_equality_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Person isEmployee".to_string(),
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            reading: "Person hasBadge".to_string(),
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "EQ".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "Person isEmployee if and only if Person hasBadge".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
        });

        // Person p1 isEmployee but does NOT hasBadge — violates biconditional
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Person".to_string(), "p1".to_string())],
        }]);
        let population = Population { facts };

        let result = evaluate(&ir, &empty_response(), &population);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Equality violation"));
    }
}
