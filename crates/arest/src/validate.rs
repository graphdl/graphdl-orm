// crates/arest/src/validate.rs
//
// Schema validation via engine self-evaluation.
//
// This module converts a domain's Domain into a Population of core metamodel
// facts and evaluates the validation domain's compiled constraints against it.
//
// The validation model is compiled from validation.md's readings at startup.
// The engine validates schemas using the same constraint evaluator it uses
// for domain logic. The readings ARE the validation rules.

use std::collections::HashMap;
use crate::types::*;
/// Convert a domain's Domain into a Population of core metamodel facts.
///
/// Each noun, fact type, role, constraint, and derivation rule in the domain
/// becomes a fact instance in the metamodel population. The validation model's
/// constraints are then evaluated against this population.
///
/// Fact type IDs use the reading text from core.md (e.g., "Noun has Object Type").
/// Convert a domain's Domain into a Population (compatibility wrapper).
pub fn ir_to_metamodel_population(ir: &Domain) -> Population {
    crate::ast::state_to_population(&ir_to_metamodel_state(ir))
}

/// Convert a domain's Domain into an Object state of core metamodel facts.
pub fn ir_to_metamodel_state(ir: &Domain) -> crate::ast::Object {
    use crate::ast::{Object, cell_push, fact_from_pairs};
    let mut state = Object::phi();

    // Nouns
    for (name, def) in &ir.nouns {
        state = cell_push("Noun has Object Type", fact_from_pairs(&[("Noun", name), ("Object Type", &def.object_type)]), &state);
    }

    // Subtypes
    for (name, super_type) in &ir.subtypes {
        state = cell_push("Noun is subtype of Noun", fact_from_pairs(&[("Noun", name), ("Noun", super_type)]), &state);
    }

    // Fact types
    for (ft_id, ft) in &ir.fact_types {
        let arity = ft.roles.len().to_string();
        state = cell_push("Graph Schema has Reading", fact_from_pairs(&[("Graph Schema", ft_id), ("Reading", &ft.reading)]), &state);
        state = cell_push("Graph Schema has Arity", fact_from_pairs(&[("Graph Schema", ft_id), ("Arity", &arity)]), &state);
        for role in &ft.roles {
            let role_id = format!("{}:{}", ft_id, role.role_index);
            state = cell_push("Graph Schema has Role", fact_from_pairs(&[("Graph Schema", ft_id), ("Role", &role_id)]), &state);
            state = cell_push("Noun plays Role", fact_from_pairs(&[("Noun", &role.noun_name), ("Role", &role_id)]), &state);
        }
    }

    // Constraints
    for constraint in &ir.constraints {
        state = cell_push("Constraint is of Constraint Type", fact_from_pairs(&[
            ("Constraint", &constraint.id), ("Constraint Type", &constraint.kind),
        ]), &state);
        for span in &constraint.spans {
            let role_id = format!("{}:{}", span.fact_type_id, span.role_index);
            state = cell_push("Constraint spans Role", fact_from_pairs(&[("Constraint", &constraint.id), ("Role", &role_id)]), &state);
        }
    }

    // Derivation rules
    for rule in &ir.derivation_rules {
        state = cell_push("Derivation Rule produces Graph Schema", fact_from_pairs(&[
            ("Derivation Rule", &rule.id), ("Graph Schema", &rule.consequent_fact_type_id),
        ]), &state);
        for antecedent in &rule.antecedent_fact_type_ids {
            state = cell_push("Derivation Rule has antecedent Graph Schema", fact_from_pairs(&[
                ("Derivation Rule", &rule.id), ("Graph Schema", antecedent),
            ]), &state);
        }
    }

    // Derived: rule dependencies
    let produces: HashMap<&str, &str> = ir.derivation_rules.iter()
        .map(|r| (r.consequent_fact_type_id.as_str(), r.id.as_str()))
        .collect();
    for rule in &ir.derivation_rules {
        for antecedent in &rule.antecedent_fact_type_ids {
            if let Some(&producer_id) = produces.get(antecedent.as_str()) {
                if producer_id != rule.id {
                    state = cell_push("Derivation Rule depends on Derivation Rule", fact_from_pairs(&[
                        ("Derivation Rule", &rule.id), ("Derivation Rule", producer_id),
                    ]), &state);
                }
            }
        }
    }

    state
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_ir() -> Domain {
        let mut ir = Domain {
            domain: "test".to_string(),
            nouns: HashMap::new(),
            fact_types: HashMap::new(),
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![], general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(), objectifications: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
        };
        ir.nouns.insert("Person".to_string(), NounDef {
            object_type: "entity".to_string(),
            world_assumption: WorldAssumption::default(),
        });
        ir.nouns.insert("Name".to_string(), NounDef {
            object_type: "value".to_string(),
            world_assumption: WorldAssumption::default(),
        });
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Person has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        ir
    }

    #[test]
    fn population_contains_noun_facts() {
        let ir = simple_ir();
        let pop = ir_to_metamodel_population(&ir);

        let noun_facts = pop.facts.get("Noun has Object Type").unwrap();
        assert_eq!(noun_facts.len(), 2);
        assert!(noun_facts.iter().any(|f|
            f.bindings.iter().any(|(_, v)| v == "Person")
        ));
    }

    #[test]
    fn population_contains_role_facts() {
        let ir = simple_ir();
        let pop = ir_to_metamodel_population(&ir);

        let role_facts = pop.facts.get("Noun plays Role").unwrap();
        assert_eq!(role_facts.len(), 2); // one per role in the fact type
    }

    #[test]
    fn population_contains_schema_facts() {
        let ir = simple_ir();
        let pop = ir_to_metamodel_population(&ir);

        let schema_facts = pop.facts.get("Graph Schema has Reading").unwrap();
        assert_eq!(schema_facts.len(), 1);
        assert!(schema_facts[0].bindings.iter().any(|(_, v)| v == "Person has Name"));
    }

    #[test]
    fn population_contains_arity() {
        let ir = simple_ir();
        let pop = ir_to_metamodel_population(&ir);

        let arity_facts = pop.facts.get("Graph Schema has Arity").unwrap();
        assert_eq!(arity_facts.len(), 1);
        assert!(arity_facts[0].bindings.iter().any(|(_, v)| v == "2"));
    }

    #[test]
    fn population_contains_constraint_facts() {
        let mut ir = simple_ir();
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Alethic".to_string(),
            text: "Each Person has at most one Name.".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            deontic_operator: None, entity: None, clauses: None,
            set_comparison_argument_length: None, min_occurrence: None, max_occurrence: None,
        });

        let pop = ir_to_metamodel_population(&ir);
        let constraint_facts = pop.facts.get("Constraint is of Constraint Type").unwrap();
        assert_eq!(constraint_facts.len(), 1);
        assert!(constraint_facts[0].bindings.iter().any(|(_, v)| v == "UC"));
    }

    #[test]
    fn derivation_dependency_computed() {
        let mut ir = simple_ir();
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Person has Full Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        // Rule A produces ft2, Rule B has antecedent ft2
        ir.derivation_rules.push(DerivationRuleDef {
            id: "rule-a".to_string(),
            text: "derive full name".to_string(),
            antecedent_fact_type_ids: vec!["ft1".to_string()],
            consequent_fact_type_id: "ft2".to_string(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![],
            match_on: vec![],
            consequent_bindings: vec![],
        });
        ir.derivation_rules.push(DerivationRuleDef {
            id: "rule-b".to_string(),
            text: "derive from full name".to_string(),
            antecedent_fact_type_ids: vec!["ft2".to_string()],
            consequent_fact_type_id: "ft1".to_string(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![],
            match_on: vec![],
            consequent_bindings: vec![],
        });

        let pop = ir_to_metamodel_population(&ir);
        let deps = pop.facts.get("Derivation Rule depends on Derivation Rule").unwrap();
        // rule-b depends on rule-a (antecedent ft2 = rule-a's consequent)
        // rule-a depends on rule-b (antecedent ft1 = rule-b's consequent)
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn undeclared_noun_produces_missing_role_fact() {
        let mut ir = simple_ir();
        // Add a fact type referencing an undeclared noun
        ir.fact_types.insert("ft-bad".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Person has Age".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Age".to_string(), role_index: 1 }, // Age not declared
            ],
        });

        let pop = ir_to_metamodel_population(&ir);
        // Role facts exist for both roles (including undeclared noun)
        let role_facts = pop.facts.get("Noun plays Role").unwrap();
        // But there's no "Noun has Object Type" for Age
        let noun_facts = pop.facts.get("Noun has Object Type").unwrap();
        assert!(!noun_facts.iter().any(|f| f.bindings.iter().any(|(_, v)| v == "Age")));
        // The role for Age exists but references an undeclared noun
        assert!(role_facts.iter().any(|f| f.bindings.iter().any(|(_, v)| v == "Age")));
    }
}
