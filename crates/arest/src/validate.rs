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

use hashbrown::HashMap;
use crate::types::*;
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

/// Convert a domain's Domain into an Object state of core metamodel facts.
pub fn ir_to_metamodel_state(ir: &Domain) -> crate::ast::Object {
    use crate::ast::{Object, cell_push, fact_from_pairs};
    // foldl(cell_push, phi, α(noun → fact)) for each category
    let state = ir.nouns.iter().fold(Object::phi(), |acc, (name, def)|
        cell_push("Noun has Object Type", fact_from_pairs(&[("Noun", name), ("Object Type", &def.object_type)]), &acc));

    let state = ir.subtypes.iter().fold(state, |acc, (name, super_type)|
        cell_push("Noun is subtype of Noun", fact_from_pairs(&[("Noun", name), ("Noun", super_type)]), &acc));

    let state = ir.fact_types.iter().fold(state, |acc, (ft_id, ft)| {
        let arity = ft.roles.len().to_string();
        let acc = cell_push("Fact Type has Reading", fact_from_pairs(&[("Fact Type", ft_id), ("Reading", &ft.reading)]), &acc);
        let acc = cell_push("Fact Type has Arity", fact_from_pairs(&[("Fact Type", ft_id), ("Arity", &arity)]), &acc);
        ft.roles.iter().fold(acc, |a, role| {
            let role_id = format!("{}:{}", ft_id, role.role_index);
            let a = cell_push("Fact Type has Role", fact_from_pairs(&[("Fact Type", ft_id), ("Role", &role_id)]), &a);
            cell_push("Noun plays Role", fact_from_pairs(&[("Noun", &role.noun_name), ("Role", &role_id)]), &a)
        })
    });

    let state = ir.constraints.iter().fold(state, |acc, constraint| {
        let acc = cell_push("Constraint is of Constraint Type", fact_from_pairs(&[
            ("Constraint", &constraint.id), ("Constraint Type", &constraint.kind)]), &acc);
        constraint.spans.iter().fold(acc, |a, span| {
            let role_id = format!("{}:{}", span.fact_type_id, span.role_index);
            cell_push("Constraint spans Role", fact_from_pairs(&[("Constraint", &constraint.id), ("Role", &role_id)]), &a)
        })
    });

    let state = ir.derivation_rules.iter().fold(state, |acc, rule| {
        let acc = cell_push("Derivation Rule produces Fact Type", fact_from_pairs(&[
            ("Derivation Rule", &rule.id), ("Fact Type", &rule.consequent_fact_type_id)]), &acc);
        rule.antecedent_fact_type_ids.iter().fold(acc, |a, antecedent|
            cell_push("Derivation Rule has antecedent Fact Type", fact_from_pairs(&[
                ("Derivation Rule", &rule.id), ("Fact Type", antecedent)]), &a))
    });

    // Derived: rule dependencies
    let produces: HashMap<&str, &str> = ir.derivation_rules.iter()
        .map(|r| (r.consequent_fact_type_id.as_str(), r.id.as_str())).collect();
    let state = ir.derivation_rules.iter().fold(state, |acc, rule|
        rule.antecedent_fact_type_ids.iter().fold(acc, |a, antecedent|
            produces.get(antecedent.as_str())
                .filter(|&&pid| pid != rule.id)
                .map(|&pid| cell_push("Derivation Rule depends on Derivation Rule",
                    fact_from_pairs(&[("Derivation Rule", &rule.id), ("Derivation Rule", pid)]), &a))
                .unwrap_or(a)));

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
    fn state_contains_noun_facts() {
        use crate::ast::{fetch_or_phi, binding};
        let ir = simple_ir();
        let state = ir_to_metamodel_state(&ir);

        let noun_facts = fetch_or_phi("Noun has Object Type", &state);
        let facts = noun_facts.as_seq().unwrap();
        assert_eq!(facts.len(), 2);
        assert!(facts.iter().any(|f| binding(f, "Noun") == Some("Person")));
    }

    #[test]
    fn state_contains_role_facts() {
        use crate::ast::fetch_or_phi;
        let ir = simple_ir();
        let state = ir_to_metamodel_state(&ir);

        let role_facts = fetch_or_phi("Noun plays Role", &state);
        assert_eq!(role_facts.as_seq().unwrap().len(), 2);
    }

    #[test]
    fn state_contains_schema_facts() {
        use crate::ast::{fetch_or_phi, binding};
        let ir = simple_ir();
        let state = ir_to_metamodel_state(&ir);

        let schema_facts = fetch_or_phi("Fact Type has Reading", &state);
        let facts = schema_facts.as_seq().unwrap();
        assert_eq!(facts.len(), 1);
        assert!(binding(&facts[0], "Reading") == Some("Person has Name"));
    }

    #[test]
    fn state_contains_arity() {
        use crate::ast::{fetch_or_phi, binding};
        let ir = simple_ir();
        let state = ir_to_metamodel_state(&ir);

        let arity_facts = fetch_or_phi("Fact Type has Arity", &state);
        let facts = arity_facts.as_seq().unwrap();
        assert_eq!(facts.len(), 1);
        assert!(binding(&facts[0], "Arity") == Some("2"));
    }

    #[test]
    fn state_contains_constraint_facts() {
        use crate::ast::{fetch_or_phi, binding};
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

        let state = ir_to_metamodel_state(&ir);
        let constraint_facts = fetch_or_phi("Constraint is of Constraint Type", &state);
        let facts = constraint_facts.as_seq().unwrap();
        assert_eq!(facts.len(), 1);
        assert!(binding(&facts[0], "Constraint Type") == Some("UC"));
    }

    #[test]
    fn derivation_dependency_computed() {
        use crate::ast::{fetch_or_phi};
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
            consequent_bindings: vec![], antecedent_filters: vec![], consequent_computed_bindings: vec![], consequent_aggregates: vec![],
        });
        ir.derivation_rules.push(DerivationRuleDef {
            id: "rule-b".to_string(),
            text: "derive from full name".to_string(),
            antecedent_fact_type_ids: vec!["ft2".to_string()],
            consequent_fact_type_id: "ft1".to_string(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![],
            match_on: vec![],
            consequent_bindings: vec![], antecedent_filters: vec![], consequent_computed_bindings: vec![], consequent_aggregates: vec![],
        });

        let state = ir_to_metamodel_state(&ir);
        let deps = fetch_or_phi("Derivation Rule depends on Derivation Rule", &state);
        assert_eq!(deps.as_seq().unwrap().len(), 2);
    }

    #[test]
    fn undeclared_noun_produces_missing_role_fact() {
        use crate::ast::{fetch_or_phi, binding};
        let mut ir = simple_ir();
        ir.fact_types.insert("ft-bad".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Person has Age".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Age".to_string(), role_index: 1 },
            ],
        });

        let state = ir_to_metamodel_state(&ir);
        let role_facts = fetch_or_phi("Noun plays Role", &state);
        let noun_facts = fetch_or_phi("Noun has Object Type", &state);
        let roles = role_facts.as_seq().unwrap();
        let nouns = noun_facts.as_seq().unwrap();
        assert!(!nouns.iter().any(|f| binding(f, "Noun") == Some("Age")));
        assert!(roles.iter().any(|f| binding(f, "Noun") == Some("Age")));
    }
}
