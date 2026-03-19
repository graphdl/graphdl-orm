// crates/constraint-eval/src/evaluate.rs
//
// Evaluation is applying compiled predicates. That's it.
//
// exec-symbols: evaluate_constraint(constraint)(population)
// Here: compiled.constraints.iter().flat_map(|c| (c.predicate)(ctx))

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
/// exec-symbols: run_machine(machine)(stream) = fold(transition)(initial)(stream)
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

        let result = evaluate_ir(&ir, &empty_response(), &population);
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
        });
        ir.nouns.insert("SupportResponse".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None, value_type: None, super_type: None,
        });
        ir.nouns.insert("APIProduct".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None, value_type: None, super_type: None,
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
}
