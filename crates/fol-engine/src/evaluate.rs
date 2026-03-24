// crates/fol-engine/src/evaluate.rs
//
// Evaluation is applying compiled predicates. That's it.
//
// Constraint verification:  constraints.flat_map(|c| (c.predicate)(ctx)) → [Violation]
// Forward inference:        derivations.flat_map(|d| (d.derive)(ctx)) → [DerivedFact]
// State machine execution:  fold(transition)(initial)(stream) → final_state
// Synthesis:                collect all knowledge about a noun from the compiled model.

use std::collections::HashSet;
use crate::types::*;
use crate::compile::{CompiledModel, EvalContext};

/// Evaluate all compiled constraints against a context.
/// Each constraint is a predicate — evaluation is just function application.
pub fn evaluate(model: &CompiledModel, ctx: &EvalContext) -> Vec<Violation> {
    model.constraints.iter()
        .flat_map(|c| (c.predicate)(ctx))
        .collect()
}

/// Run a compiled state machine by folding events through the transition function.
/// run_machine = fold(transition)(initial)(stream)
pub fn run_machine(
    machine: &crate::compile::CompiledStateMachine,
    events: &[(String, String)],
    ctx: &EvalContext,
) -> String {
    events.iter().fold(machine.initial.clone(), |state, (event, _)| {
        (machine.transition)(&state, event, ctx).unwrap_or(state)
    })
}

/// Convenience: compile + evaluate in one call.
/// Used by tests and as backward-compatible entry point.
pub fn evaluate_ir(
    ir: &ConstraintIR,
    response: &ResponseContext,
    population: &Population,
) -> Vec<Violation> {
    let compiled = crate::compile::compile(ir);
    let ctx = EvalContext { response, population };
    evaluate(&compiled, &ctx)
}

// ── Forward Chaining ─────────────────────────────────────────────────
// Apply all derivation rules until no new facts are produced (fixed point).
// This is the FOL inference engine.

/// Forward chain: apply all derivation rules until no new facts are produced.
/// Returns all derived facts. Mutates population to include derived facts.
///
/// Takes response and population separately to avoid borrow conflicts —
/// creates a fresh EvalContext each iteration since the population is mutated.
pub fn forward_chain(
    model: &CompiledModel,
    response: &ResponseContext,
    population: &mut Population,
) -> Vec<DerivedFact> {
    let mut all_derived: Vec<DerivedFact> = Vec::new();
    let max_iterations = 10; // prevent infinite loops

    for _ in 0..max_iterations {
        let mut new_facts: Vec<DerivedFact> = Vec::new();

        // Build a fresh context each iteration — population changes between iterations
        let ctx = EvalContext { response, population: &*population };

        for derivation in &model.derivations {
            let derived = (derivation.derive)(&ctx, population);
            for fact in derived {
                if !population_contains(population, &fact)
                    && !all_derived.iter().any(|d| same_fact(d, &fact))
                    && !new_facts.iter().any(|d| same_fact(d, &fact))
                {
                    new_facts.push(fact);
                }
            }
        }

        if new_facts.is_empty() {
            break; // Fixed point reached
        }

        // Add new facts to population for next iteration
        for fact in &new_facts {
            add_to_population(population, fact);
        }
        all_derived.extend(new_facts);
    }

    all_derived
}

/// Check if a derived fact already exists in the population.
fn population_contains(population: &Population, fact: &DerivedFact) -> bool {
    population.facts.get(&fact.fact_type_id).map_or(false, |facts| {
        facts.iter().any(|f| {
            f.fact_type_id == fact.fact_type_id
                && f.bindings.len() == fact.bindings.len()
                && f.bindings.iter().all(|b| fact.bindings.contains(b))
        })
    })
}

/// Check if two derived facts represent the same fact.
fn same_fact(a: &DerivedFact, b: &DerivedFact) -> bool {
    a.fact_type_id == b.fact_type_id
        && a.bindings.len() == b.bindings.len()
        && a.bindings.iter().all(|ab| b.bindings.contains(ab))
}

/// Add a derived fact to the population as a FactInstance.
fn add_to_population(population: &mut Population, fact: &DerivedFact) {
    let instance = FactInstance {
        fact_type_id: fact.fact_type_id.clone(),
        bindings: fact.bindings.clone(),
    };
    population.facts.entry(fact.fact_type_id.clone())
        .or_insert_with(Vec::new)
        .push(instance);
}

// ── Synthesis ────────────────────────────────────────────────────────
// Collect all knowledge about a noun from the compiled model.

/// Synthesize: collect all knowledge about a noun from the compiled model.
pub fn synthesize(
    model: &CompiledModel,
    ir: &ConstraintIR,
    noun_name: &str,
    depth: usize,
) -> SynthesisResult {
    let index = &model.noun_index;

    let world_assumption = index.world_assumptions.get(noun_name)
        .cloned()
        .unwrap_or(WorldAssumption::Closed);

    // 1. Find all fact types where this noun plays a role
    let participates_in: Vec<FactTypeSummary> = index.noun_to_fact_types
        .get(noun_name)
        .map(|fts| fts.iter().filter_map(|(ft_id, role_idx)| {
            ir.fact_types.get(ft_id).map(|ft| FactTypeSummary {
                id: ft_id.clone(),
                reading: ft.reading.clone(),
                role_index: *role_idx,
            })
        }).collect())
        .unwrap_or_default();

    // 2. Find all constraints spanning those fact types
    let mut seen_constraint_ids = HashSet::new();
    let applicable_constraints: Vec<ConstraintSummary> = participates_in.iter()
        .flat_map(|ft_summary| {
            index.fact_type_to_constraints.get(&ft_summary.id)
                .cloned()
                .unwrap_or_default()
        })
        .filter(|cid| seen_constraint_ids.insert(cid.clone()))
        .filter_map(|cid| {
            index.constraint_index.get(&cid).and_then(|&idx| {
                model.constraints.get(idx).map(|cc| {
                    // Look up the original constraint def for kind info
                    let cdef = ir.constraints.iter().find(|c| c.id == cid);
                    ConstraintSummary {
                        id: cid.clone(),
                        text: cc.text.clone(),
                        kind: cdef.map(|c| c.kind.clone()).unwrap_or_default(),
                        modality: format!("{:?}", cc.modality),
                        deontic_operator: cdef.and_then(|c| c.deontic_operator.clone()),
                    }
                })
            })
        })
        .collect();

    // 3. Find state machines for this noun
    let state_machines: Vec<StateMachineSummary> = index.noun_to_state_machines
        .get(noun_name)
        .and_then(|&idx| model.state_machines.get(idx))
        .map(|sm| {
            vec![StateMachineSummary {
                noun_name: sm.noun_name.clone(),
                statuses: sm.statuses.clone(),
                current_status: Some(sm.initial.clone()),
                valid_transitions: sm.transition_table.iter()
                    .filter(|(from, _, _)| *from == sm.initial)
                    .map(|(_, _, event)| event.clone())
                    .collect(),
            }]
        })
        .unwrap_or_default();

    // 4. Find related nouns (other nouns in shared fact types)
    let mut seen_related = HashSet::new();
    let related_nouns: Vec<RelatedNoun> = if depth > 0 {
        participates_in.iter()
            .flat_map(|ft_summary| {
                ir.fact_types.get(&ft_summary.id)
                    .map(|ft| {
                        ft.roles.iter()
                            .filter(|r| r.noun_name != noun_name)
                            .filter(|r| seen_related.insert(r.noun_name.clone()))
                            .map(|r| {
                                let wa = index.world_assumptions.get(&r.noun_name)
                                    .cloned()
                                    .unwrap_or(WorldAssumption::Closed);
                                RelatedNoun {
                                    name: r.noun_name.clone(),
                                    via_fact_type: ft_summary.id.clone(),
                                    via_reading: ft.reading.clone(),
                                    world_assumption: wa,
                                }
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
            .collect()
    } else {
        Vec::new()
    };

    // 5. Derived facts would need population context; leave empty for static synthesis.
    let derived_facts = Vec::new();

    SynthesisResult {
        noun_name: noun_name.to_string(),
        world_assumption,
        participates_in,
        applicable_constraints,
        state_machines,
        derived_facts,
        related_nouns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn empty_ir() -> ConstraintIR {
        ConstraintIR {
            domain: "test".to_string(),
            nouns: HashMap::new(),
            fact_types: HashMap::new(),
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![],
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

    fn make_noun(object_type: &str) -> NounDef {
        NounDef {
            object_type: object_type.to_string(),
            enum_values: None,
            value_type: None,
            super_type: None,
            world_assumption: WorldAssumption::default(),
        }
    }

    #[test]
    fn test_no_constraints_no_violations() {
        let ir = empty_ir();
        let result = evaluate_ir(&ir, &empty_response(), &empty_population());
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
            world_assumption: WorldAssumption::default(),
        });
        ir.nouns.insert("SupportResponse".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None,
            value_type: None,
            super_type: None,
            world_assumption: WorldAssumption::default(),
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
            min_occurrence: None,
            max_occurrence: None,
        });

        let response = ResponseContext {
            text: "Hello — how can I help?".to_string(),
            sender_identity: None,
            fields: None,
        };

        let result = evaluate_ir(&ir, &response, &empty_population());
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
            world_assumption: WorldAssumption::default(),
        });
        ir.nouns.insert("SupportResponse".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None,
            value_type: None,
            super_type: None,
            world_assumption: WorldAssumption::default(),
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
            min_occurrence: None,
            max_occurrence: None,
        });

        let response = ResponseContext {
            text: "Hello, how can I help you today?".to_string(),
            sender_identity: None,
            fields: None,
        };

        let result = evaluate_ir(&ir, &response, &empty_population());
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
            min_occurrence: None,
            max_occurrence: None,
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

        let result = evaluate_ir(&ir, &empty_response(), &population);
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
            kind: "IR".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "No Person manages itself".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("Person".to_string(), "p1".to_string()), ("Person".to_string(), "p1".to_string())],
            },
        ]);
        let population = Population { facts };

        let result = evaluate_ir(&ir, &empty_response(), &population);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Irreflexive"));
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
            min_occurrence: None,
            max_occurrence: None,
        });

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

        let result = evaluate_ir(&ir, &empty_response(), &population);
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
            min_occurrence: None,
            max_occurrence: None,
        });

        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Person".to_string(), "p1".to_string())],
        }]);
        let population = Population { facts };

        let result = evaluate_ir(&ir, &empty_response(), &population);
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
            min_occurrence: None,
            max_occurrence: None,
        });

        let result = evaluate_ir(&ir, &empty_response(), &empty_population());
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
            min_occurrence: None,
            max_occurrence: None,
        });

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

        let result = evaluate_ir(&ir, &empty_response(), &population);
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
            world_assumption: WorldAssumption::default(),
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
            min_occurrence: None,
            max_occurrence: None,
        });

        let mut facts = HashMap::new();
        facts.insert("ft2".to_string(), vec![FactInstance {
            fact_type_id: "ft2".to_string(),
            bindings: vec![("Customer".to_string(), "c1".to_string()), ("Email".to_string(), "a@b.com".to_string())],
        }]);
        let population = Population { facts };

        let result = evaluate_ir(&ir, &empty_response(), &population);
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
            min_occurrence: None,
            max_occurrence: None,
        });

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

        let result = evaluate_ir(&ir, &empty_response(), &population);
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
            world_assumption: WorldAssumption::default(),
        });
        ir.nouns.insert("SupportResponse".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None,
            value_type: None,
            super_type: None,
            world_assumption: WorldAssumption::default(),
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
            min_occurrence: None,
            max_occurrence: None,
        });

        let response = ResponseContext {
            text: "Here is some help for you.".to_string(),
            sender_identity: Some(String::new()),
            fields: None,
        };

        let result = evaluate_ir(&ir, &response, &empty_population());
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
            min_occurrence: None,
            max_occurrence: None,
        });

        let response = ResponseContext {
            text: "Hello".to_string(),
            sender_identity: Some(String::new()),
            fields: None,
        };

        let result = evaluate_ir(&ir, &response, &empty_population());
        assert_eq!(result.len(), 1);
        assert!(result[0].detail.contains("SenderIdentity"));
    }

    /// Regression: constraints spanning multiple fact types that share a value-type noun
    /// must not produce duplicate violations. collect_enum_values deduplicates by noun name.
    #[test]
    fn test_no_duplicate_violations_for_multi_span_constraints() {
        let mut ir = empty_ir();
        ir.nouns.insert("FieldName".to_string(), NounDef {
            object_type: "value".to_string(),
            enum_values: Some(vec!["EndpointSlug".to_string(), "Title".to_string()]),
            value_type: Some("string".to_string()),
            super_type: None,
            world_assumption: WorldAssumption::default(),
        });
        ir.nouns.insert("SupportResponse".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None, value_type: None, super_type: None,
            world_assumption: WorldAssumption::default(),
        });
        ir.nouns.insert("APIProduct".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None, value_type: None, super_type: None,
            world_assumption: WorldAssumption::default(),
        });
        // Three fact types that all reference FieldName — simulates multi-span constraint
        for i in 1..=3 {
            ir.fact_types.insert(format!("ft{}", i), FactTypeDef {
                reading: format!("SupportResponse names APIProduct by FieldName ({})", i),
                roles: vec![
                    RoleDef { noun_name: "SupportResponse".to_string(), role_index: 0 },
                    RoleDef { noun_name: "APIProduct".to_string(), role_index: 1 },
                    RoleDef { noun_name: "FieldName".to_string(), role_index: 2 },
                ],
            });
        }
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("obligatory".to_string()),
            text: "It is obligatory that SupportResponse names APIProduct by FieldName 'Title'.".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft3".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let response = ResponseContext {
            text: "test response without required field names".to_string(),
            sender_identity: None,
            fields: None,
        };

        let result = evaluate_ir(&ir, &response, &empty_population());
        // Should produce exactly 1 violation per unique noun, not 3 duplicates
        let field_name_violations: Vec<_> = result.iter()
            .filter(|v| v.detail.contains("FieldName"))
            .collect();
        assert_eq!(field_name_violations.len(), 1,
            "Expected 1 FieldName violation, got {}. Violations: {:?}",
            field_name_violations.len(),
            field_name_violations.iter().map(|v| &v.detail).collect::<Vec<_>>());
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
            min_occurrence: None,
            max_occurrence: None,
        });

        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Person".to_string(), "p1".to_string())],
        }]);
        let population = Population { facts };

        let result = evaluate_ir(&ir, &empty_response(), &population);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Equality violation"));
    }

    // ── Forward Inference & Synthesis Tests ───────────────────────────

    #[test]
    fn test_subtype_inheritance_derivation() {
        let mut ir = empty_ir();

        // Vehicle is a supertype, Car is a subtype of Vehicle
        ir.nouns.insert("Vehicle".to_string(), make_noun("entity"));
        ir.nouns.insert("Car".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None,
            value_type: None,
            super_type: Some("Vehicle".to_string()),
            world_assumption: WorldAssumption::default(),
        });
        ir.nouns.insert("License".to_string(), make_noun("entity"));

        // Fact type: Vehicle has License
        ir.fact_types.insert("ft_vehicle_license".to_string(), FactTypeDef {
            reading: "Vehicle has License".to_string(),
            roles: vec![
                RoleDef { noun_name: "Vehicle".to_string(), role_index: 0 },
                RoleDef { noun_name: "License".to_string(), role_index: 1 },
            ],
        });

        // Fact type: Car has Color (to give Car instances)
        ir.fact_types.insert("ft_car_color".to_string(), FactTypeDef {
            reading: "Car has Color".to_string(),
            roles: vec![
                RoleDef { noun_name: "Car".to_string(), role_index: 0 },
            ],
        });

        let compiled = crate::compile::compile(&ir);

        // Verify subtype inheritance derivation was compiled
        let subtype_derivations: Vec<_> = compiled.derivations.iter()
            .filter(|d| d.kind == DerivationKind::SubtypeInheritance)
            .collect();
        assert!(!subtype_derivations.is_empty(),
            "Expected at least one subtype inheritance derivation");

        // Test forward chaining with a population that has a Car instance
        let mut facts = HashMap::new();
        facts.insert("ft_car_color".to_string(), vec![FactInstance {
            fact_type_id: "ft_car_color".to_string(),
            bindings: vec![("Car".to_string(), "my_car".to_string())],
        }]);
        let mut population = Population { facts };
        let response = empty_response();

        let derived = forward_chain(&compiled, &response, &mut population);

        // Car "my_car" should inherit Vehicle's fact type
        let inheritance_facts: Vec<_> = derived.iter()
            .filter(|d| d.derived_by.contains("subtype"))
            .collect();
        assert!(!inheritance_facts.is_empty(),
            "Expected subtype inheritance to derive facts for Car instance");
    }

    #[test]
    fn test_modus_ponens_from_subset() {
        let mut ir = empty_ir();

        ir.nouns.insert("Person".to_string(), make_noun("entity"));
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Person hasLicense".to_string(),
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            reading: "Person hasInsurance".to_string(),
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        // SS constraint: hasLicense -> hasInsurance
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
            min_occurrence: None,
            max_occurrence: None,
        });

        let compiled = crate::compile::compile(&ir);

        // Verify modus ponens derivation was compiled
        let mp_derivations: Vec<_> = compiled.derivations.iter()
            .filter(|d| d.kind == DerivationKind::ModusPonens)
            .collect();
        assert!(!mp_derivations.is_empty(),
            "Expected a modus ponens derivation from SS constraint");

        // Forward chain: p1 hasLicense -> should derive p1 hasInsurance
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Person".to_string(), "p1".to_string())],
        }]);
        let mut population = Population { facts };
        let response = empty_response();

        let derived = forward_chain(&compiled, &response, &mut population);

        let insurance_facts: Vec<_> = derived.iter()
            .filter(|d| d.fact_type_id == "ft2")
            .collect();
        assert_eq!(insurance_facts.len(), 1,
            "Expected modus ponens to derive hasInsurance for p1");
        assert_eq!(insurance_facts[0].bindings, vec![("Person".to_string(), "p1".to_string())]);
        assert_eq!(insurance_facts[0].confidence, Confidence::Definitive);
    }

    #[test]
    fn test_cwa_vs_owa_negation() {
        let mut ir = empty_ir();

        // CWA noun: GovernmentPower (not stated = false)
        ir.nouns.insert("GovernmentPower".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None,
            value_type: None,
            super_type: None,
            world_assumption: WorldAssumption::Closed,
        });
        // OWA noun: IndividualRight (not stated = unknown)
        ir.nouns.insert("IndividualRight".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None,
            value_type: None,
            super_type: None,
            world_assumption: WorldAssumption::Open,
        });

        ir.nouns.insert("Authority".to_string(), make_noun("entity"));

        // Fact type involving CWA noun
        ir.fact_types.insert("ft_power".to_string(), FactTypeDef {
            reading: "GovernmentPower grants Authority".to_string(),
            roles: vec![
                RoleDef { noun_name: "GovernmentPower".to_string(), role_index: 0 },
                RoleDef { noun_name: "Authority".to_string(), role_index: 1 },
            ],
        });
        // Fact type involving OWA noun
        ir.fact_types.insert("ft_right".to_string(), FactTypeDef {
            reading: "IndividualRight protects Authority".to_string(),
            roles: vec![
                RoleDef { noun_name: "IndividualRight".to_string(), role_index: 0 },
                RoleDef { noun_name: "Authority".to_string(), role_index: 1 },
            ],
        });

        let compiled = crate::compile::compile(&ir);

        // CWA derivation should exist for GovernmentPower
        let cwa_derivations: Vec<_> = compiled.derivations.iter()
            .filter(|d| d.kind == DerivationKind::ClosedWorldNegation)
            .collect();

        let cwa_for_power = cwa_derivations.iter()
            .any(|d| d.id.contains("GovernmentPower"));
        assert!(cwa_for_power,
            "Expected CWA negation derivation for GovernmentPower");

        // No CWA derivation for IndividualRight (it's OWA)
        let cwa_for_right = cwa_derivations.iter()
            .any(|d| d.id.contains("IndividualRight"));
        assert!(!cwa_for_right,
            "Expected NO CWA negation derivation for IndividualRight (OWA noun)");

        // Forward chain with a population where GovernmentPower exists
        // but doesn't participate in ft_power
        let mut facts = HashMap::new();
        // GovernmentPower "tax" exists (via some other fact type)
        facts.insert("ft_other".to_string(), vec![FactInstance {
            fact_type_id: "ft_other".to_string(),
            bindings: vec![("GovernmentPower".to_string(), "tax".to_string())],
        }]);
        let mut population = Population { facts };
        let response = empty_response();

        let derived = forward_chain(&compiled, &response, &mut population);

        // Under CWA, "tax" doesn't participate in ft_power -> derive negation
        let negation_facts: Vec<_> = derived.iter()
            .filter(|d| d.derived_by.contains("cwa_negation") && d.reading.contains("NOT"))
            .collect();
        assert!(!negation_facts.is_empty(),
            "Expected CWA to derive negation for GovernmentPower 'tax' not in ft_power");
        assert_eq!(negation_facts[0].confidence, Confidence::Definitive);
    }

    #[test]
    fn test_synthesis_basic() {
        let mut ir = empty_ir();

        ir.nouns.insert("Customer".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None,
            value_type: None,
            super_type: None,
            world_assumption: WorldAssumption::Closed,
        });
        ir.nouns.insert("Name".to_string(), make_noun("value"));
        ir.nouns.insert("Email".to_string(), make_noun("value"));

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
            min_occurrence: None,
            max_occurrence: None,
        });

        let compiled = crate::compile::compile(&ir);
        let result = synthesize(&compiled, &ir, "Customer", 1);

        assert_eq!(result.noun_name, "Customer");
        assert_eq!(result.world_assumption, WorldAssumption::Closed);

        // Customer participates in two fact types
        assert_eq!(result.participates_in.len(), 2,
            "Customer should participate in ft1 and ft2. Got: {:?}",
            result.participates_in);

        // One constraint applies to Customer
        assert_eq!(result.applicable_constraints.len(), 1,
            "Expected 1 constraint for Customer. Got: {:?}",
            result.applicable_constraints);
        assert_eq!(result.applicable_constraints[0].id, "c1");

        // Related nouns: Name and Email
        assert_eq!(result.related_nouns.len(), 2,
            "Expected 2 related nouns. Got: {:?}", result.related_nouns);
        let related_names: Vec<_> = result.related_nouns.iter()
            .map(|r| r.name.as_str())
            .collect();
        assert!(related_names.contains(&"Name"), "Expected Name as related noun");
        assert!(related_names.contains(&"Email"), "Expected Email as related noun");
    }

    #[test]
    fn test_synthesis_empty_noun() {
        let ir = empty_ir();
        let compiled = crate::compile::compile(&ir);
        let result = synthesize(&compiled, &ir, "NonExistent", 1);

        assert_eq!(result.noun_name, "NonExistent");
        assert!(result.participates_in.is_empty());
        assert!(result.applicable_constraints.is_empty());
        assert!(result.state_machines.is_empty());
        assert!(result.related_nouns.is_empty());
    }

    #[test]
    fn test_forward_chain_fixed_point() {
        // Verify forward chaining reaches a fixed point (no infinite loops)
        let mut ir = empty_ir();
        ir.nouns.insert("A".to_string(), make_noun("entity"));
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "A exists".to_string(),
            roles: vec![RoleDef { noun_name: "A".to_string(), role_index: 0 }],
        });

        let compiled = crate::compile::compile(&ir);

        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("A".to_string(), "a1".to_string())],
        }]);
        let mut population = Population { facts };
        let response = empty_response();

        // Should terminate even if derivations produce facts
        let derived = forward_chain(&compiled, &response, &mut population);
        // Just verify it terminates — the exact count depends on CWA rules
        assert!(derived.len() < 100, "Forward chaining should reach fixed point quickly");
    }

    #[test]
    fn test_transitivity_derivation() {
        let mut ir = empty_ir();

        ir.nouns.insert("City".to_string(), make_noun("entity"));
        ir.nouns.insert("State".to_string(), make_noun("entity"));
        ir.nouns.insert("Country".to_string(), make_noun("entity"));

        // City isIn State
        ir.fact_types.insert("ft_city_state".to_string(), FactTypeDef {
            reading: "City isIn State".to_string(),
            roles: vec![
                RoleDef { noun_name: "City".to_string(), role_index: 0 },
                RoleDef { noun_name: "State".to_string(), role_index: 1 },
            ],
        });
        // State isIn Country
        ir.fact_types.insert("ft_state_country".to_string(), FactTypeDef {
            reading: "State isIn Country".to_string(),
            roles: vec![
                RoleDef { noun_name: "State".to_string(), role_index: 0 },
                RoleDef { noun_name: "Country".to_string(), role_index: 1 },
            ],
        });

        let compiled = crate::compile::compile(&ir);

        // Should have a transitivity derivation for City->State->Country
        let trans_derivations: Vec<_> = compiled.derivations.iter()
            .filter(|d| d.kind == DerivationKind::Transitivity)
            .collect();
        assert!(!trans_derivations.is_empty(),
            "Expected transitivity derivation for City->State->Country chain");

        // Forward chain: Austin isIn Texas, Texas isIn USA -> Austin (transitively) in USA
        let mut facts = HashMap::new();
        facts.insert("ft_city_state".to_string(), vec![FactInstance {
            fact_type_id: "ft_city_state".to_string(),
            bindings: vec![
                ("City".to_string(), "Austin".to_string()),
                ("State".to_string(), "Texas".to_string()),
            ],
        }]);
        facts.insert("ft_state_country".to_string(), vec![FactInstance {
            fact_type_id: "ft_state_country".to_string(),
            bindings: vec![
                ("State".to_string(), "Texas".to_string()),
                ("Country".to_string(), "USA".to_string()),
            ],
        }]);
        let mut population = Population { facts };
        let response = empty_response();

        let derived = forward_chain(&compiled, &response, &mut population);

        let transitive_facts: Vec<_> = derived.iter()
            .filter(|d| d.derived_by.contains("transitivity"))
            .collect();
        assert!(!transitive_facts.is_empty(),
            "Expected transitivity to derive Austin->USA relationship");

        // Verify the derived fact connects City to Country
        let city_country = transitive_facts.iter().find(|d| {
            d.bindings.iter().any(|(_, v)| v == "Austin")
                && d.bindings.iter().any(|(_, v)| v == "USA")
        });
        assert!(city_country.is_some(),
            "Expected derived fact linking Austin to USA. Derived: {:?}", transitive_facts);
    }

    #[test]
    fn test_world_assumption_default_is_closed() {
        assert_eq!(WorldAssumption::default(), WorldAssumption::Closed);
    }

    #[test]
    fn test_backward_compatible_deserialization() {
        // Old IR JSON without derivation_rules or world_assumption should still parse
        let json = r#"{
            "domain": "test",
            "nouns": {
                "Customer": {
                    "objectType": "entity",
                    "enumValues": null,
                    "valueType": null,
                    "superType": null
                }
            },
            "factTypes": {},
            "constraints": [],
            "stateMachines": {}
        }"#;

        let ir: ConstraintIR = serde_json::from_str(json).expect("Should parse old IR format");
        assert_eq!(ir.domain, "test");
        assert!(ir.derivation_rules.is_empty());
        let customer = ir.nouns.get("Customer").unwrap();
        assert_eq!(customer.world_assumption, WorldAssumption::Closed);
    }
}
