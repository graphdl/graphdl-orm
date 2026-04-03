// crates/fol-engine/src/validate.rs
//
// Schema validation via engine self-evaluation.
//
// Instead of procedural checks (csdp.rs), this module converts a domain's
// ConstraintIR into a Population of core metamodel facts and evaluates the
// validation domain's compiled constraints against it.
//
// The validation model is compiled from validation.md's readings at startup.
// The engine validates schemas using the same constraint evaluator it uses
// for domain logic. The readings ARE the validation rules.

use std::collections::HashMap;
use crate::types::*;
use crate::compile::CompiledModel;
use crate::evaluate;

/// Convert a domain's ConstraintIR into a Population of core metamodel facts.
///
/// Each noun, fact type, role, constraint, and derivation rule in the domain
/// becomes a fact instance in the metamodel population. The validation model's
/// constraints are then evaluated against this population.
///
/// Fact type IDs use the reading text from core.md (e.g., "Noun has Object Type").
pub fn ir_to_metamodel_population(ir: &ConstraintIR) -> Population {
    let mut facts: HashMap<String, Vec<FactInstance>> = HashMap::new();

    let mut push = |ft_id: &str, bindings: Vec<(&str, &str)>| {
        facts.entry(ft_id.to_string()).or_default().push(FactInstance {
            fact_type_id: ft_id.to_string(),
            bindings: bindings.into_iter().map(|(n, v)| (n.to_string(), v.to_string())).collect(),
        });
    };

    // Nouns → "Noun has Object Type"
    for (name, def) in &ir.nouns {
        push("Noun has Object Type", vec![("Noun", name), ("Object Type", &def.object_type)]);
    }

    // Subtypes → "Noun is subtype of Noun"
    for (name, super_type) in &ir.subtypes {
        push("Noun is subtype of Noun", vec![("Noun", name), ("Noun", super_type)]);
    }

    // Fact types → "Graph Schema has Reading", "Graph Schema has Role", "Graph Schema has Arity"
    for (ft_id, ft) in &ir.fact_types {
        push("Graph Schema has Reading", vec![("Graph Schema", ft_id), ("Reading", &ft.reading)]);

        let arity = ft.roles.len().to_string();
        push("Graph Schema has Arity", vec![("Graph Schema", ft_id), ("Arity", &arity)]);

        for role in &ft.roles {
            let role_id = format!("{}:{}", ft_id, role.role_index);
            push("Graph Schema has Role", vec![("Graph Schema", ft_id), ("Role", &role_id)]);
            push("Noun plays Role", vec![("Noun", &role.noun_name), ("Role", &role_id)]);
        }
    }

    // Constraints → "Constraint is of Constraint Type", "Constraint spans Role"
    for constraint in &ir.constraints {
        push("Constraint is of Constraint Type", vec![
            ("Constraint", &constraint.id), ("Constraint Type", &constraint.kind),
        ]);
        for span in &constraint.spans {
            let role_id = format!("{}:{}", span.fact_type_id, span.role_index);
            push("Constraint spans Role", vec![("Constraint", &constraint.id), ("Role", &role_id)]);
        }
    }

    // Derivation rules → dependency facts
    for rule in &ir.derivation_rules {
        push("Derivation Rule produces Graph Schema", vec![
            ("Derivation Rule", &rule.id), ("Graph Schema", &rule.consequent_fact_type_id),
        ]);
        for antecedent in &rule.antecedent_fact_type_ids {
            push("Derivation Rule has antecedent Graph Schema", vec![
                ("Derivation Rule", &rule.id), ("Graph Schema", antecedent),
            ]);
        }
    }

    // Derived: "Derivation Rule depends on Derivation Rule"
    // Rule A depends on Rule B iff A has antecedent G and B produces G.
    let produces: HashMap<&str, &str> = ir.derivation_rules.iter()
        .map(|r| (r.consequent_fact_type_id.as_str(), r.id.as_str()))
        .collect();

    for rule in &ir.derivation_rules {
        for antecedent in &rule.antecedent_fact_type_ids {
            if let Some(&producer_id) = produces.get(antecedent.as_str()) {
                if producer_id != rule.id {
                    push("Derivation Rule depends on Derivation Rule", vec![
                        ("Derivation Rule", &rule.id), ("Derivation Rule", producer_id),
                    ]);
                }
            }
        }
    }

    Population { facts }
}

/// Validate a domain's IR against a compiled validation model.
///
/// The validation model is compiled from core.md + validation.md at startup.
/// This function converts the domain IR to a metamodel population and
/// evaluates the validation constraints against it.
pub fn validate_schema(
    validation_model: &CompiledModel,
    domain_ir: &ConstraintIR,
) -> Vec<Violation> {
    let population = ir_to_metamodel_population(domain_ir);
    let response = ResponseContext { text: String::new(), sender_identity: None, fields: None };
    let mut violations = evaluate::evaluate_via_ast(validation_model, &response, &population);

    // Objectification atomicity check (Halpin, "Objectification and Atomicity", 2020):
    // If a noun objectifies a fact type, that fact type must have a spanning UC.
    violations.extend(check_objectification_atomicity(domain_ir));
    violations.extend(check_nary_uc_arity(domain_ir));
    violations.extend(check_subtype_derivation_rules(domain_ir));
    violations.extend(check_constraint_implication(domain_ir));
    violations.extend(check_satisfiability(domain_ir));
    violations.extend(check_redundancy(domain_ir));

    violations
}

/// Check n-ary fact types (3+ roles) have UCs spanning at least n-1 roles.
/// A simple UC on a single role of a ternary is an arity violation (Halpin TechReport ORM2-02 Sec 2.1.3).
pub fn check_nary_uc_arity(ir: &ConstraintIR) -> Vec<Violation> {
    let mut violations = Vec::new();

    for (ft_id, ft) in &ir.fact_types {
        let arity = ft.roles.len();
        if arity < 3 { continue; } // only check ternary and higher

        // Find all UCs on this fact type
        let ucs_on_ft: Vec<&ConstraintDef> = ir.constraints.iter()
            .filter(|c| c.kind == "UC" && c.spans.iter().any(|s| s.fact_type_id == *ft_id))
            .collect();

        for uc in &ucs_on_ft {
            let span_count = uc.spans.iter()
                .filter(|s| s.fact_type_id == *ft_id)
                .count();

            if span_count < arity - 1 {
                violations.push(Violation {
                    constraint_id: uc.id.clone(),
                    constraint_text: format!(
                        "UC on {}-ary fact type '{}' spans only {} role(s), must span at least {}",
                        arity, ft.reading, span_count, arity - 1
                    ),
                    detail: format!(
                        "A uniqueness constraint on an n-ary fact type must span at least n-1 roles. \
                         '{}' has {} roles but this UC spans only {}. \
                         Consider decomposing into binary fact types.",
                        ft.reading, arity, span_count
                    ),
                    alethic: true,
                });
            }
        }
    }

    violations
}

/// Check that derived subtypes have derivation rules.
/// Without subtype_kind on NounDef, this check is deferred until the parser
/// can distinguish derived vs asserted subtypes through derivation rule presence.
pub fn check_subtype_derivation_rules(_ir: &ConstraintIR) -> Vec<Violation> {
    Vec::new()
}

/// Check schema satisfiability — detect contradictory constraints that
/// prevent any valid population from existing (Halpin PhD thesis Ch 6).
pub fn check_satisfiability(ir: &ConstraintIR) -> Vec<Violation> {
    let mut violations = Vec::new();

    // Check 1: Frequency constraint with min > max
    for c in &ir.constraints {
        if c.kind == "FC" {
            if let (Some(min), Some(max)) = (c.min_occurrence, c.max_occurrence) {
                if min > max {
                    violations.push(Violation {
                        constraint_id: c.id.clone(),
                        constraint_text: format!("Unsatisfiable: frequency min ({}) > max ({})", min, max),
                        detail: format!(
                            "Constraint '{}' requires at least {} but at most {} occurrences. \
                             No valid population can satisfy this.",
                            c.text, min, max
                        ),
                        alethic: true,
                    });
                }
            }
        }
    }

    // Check 2: Mandatory on a role + exclusion on same fact type's population
    // This catches: "Each X must R some Y" + "No X may R any Y"
    for c in &ir.constraints {
        if c.kind != "MC" { continue; }
        for span in &c.spans {
            // Look for an exclusion constraint on the same fact type
            let has_exclusion = ir.constraints.iter().any(|c2| {
                (c2.kind == "XC" || c2.kind == "forbidden") &&
                c2.spans.iter().any(|s2| s2.fact_type_id == span.fact_type_id)
            });
            if has_exclusion {
                violations.push(Violation {
                    constraint_id: format!("unsat:mc+xc:{}", span.fact_type_id),
                    constraint_text: format!(
                        "Unsatisfiable: mandatory + exclusion on '{}'",
                        span.fact_type_id
                    ),
                    detail: format!(
                        "Fact type '{}' has both a mandatory constraint (each entity must participate) \
                         and an exclusion constraint (no entity may participate). \
                         No valid population can satisfy both.",
                        span.fact_type_id
                    ),
                    alethic: true,
                });
            }
        }
    }

    violations
}

/// Detect strong redundancy in the named set (Codd 1970 Sec 2.2.1).
/// A relation is strongly redundant if it contains a projection derivable
/// from other projections in the named set.
///
/// Practical check: if a fact type's roles are a subset of another fact type's
/// roles (same nouns), the smaller fact type may be redundant (derivable by projection).
pub fn check_redundancy(ir: &ConstraintIR) -> Vec<Violation> {
    let mut violations = Vec::new();

    let ft_entries: Vec<(&String, &FactTypeDef)> = ir.fact_types.iter().collect();

    for (i, (ft_id_a, ft_a)) in ft_entries.iter().enumerate() {
        let nouns_a: std::collections::HashSet<&str> = ft_a.roles.iter()
            .map(|r| r.noun_name.as_str()).collect();

        for (ft_id_b, ft_b) in ft_entries.iter().skip(i + 1) {
            let nouns_b: std::collections::HashSet<&str> = ft_b.roles.iter()
                .map(|r| r.noun_name.as_str()).collect();

            // If A's nouns are a proper subset of B's nouns, A might be derivable from B
            if nouns_a.is_subset(&nouns_b) && nouns_a.len() < nouns_b.len() {
                violations.push(Violation {
                    constraint_id: format!("redundant:{}:{}", ft_id_a, ft_id_b),
                    constraint_text: format!(
                        "Possible redundancy: '{}' may be derivable from '{}' by projection",
                        ft_a.reading, ft_b.reading
                    ),
                    detail: format!(
                        "The roles of '{}' ({:?}) are a subset of '{}' ({:?}). \
                         This fact type may be strongly redundant (Codd 1970 Sec 2.2.1) — \
                         derivable by projecting the larger relation.",
                        ft_a.reading, nouns_a, ft_b.reading, nouns_b
                    ),
                    alethic: false, // informational
                });
            } else if nouns_b.is_subset(&nouns_a) && nouns_b.len() < nouns_a.len() {
                violations.push(Violation {
                    constraint_id: format!("redundant:{}:{}", ft_id_b, ft_id_a),
                    constraint_text: format!(
                        "Possible redundancy: '{}' may be derivable from '{}' by projection",
                        ft_b.reading, ft_a.reading
                    ),
                    detail: format!(
                        "The roles of '{}' ({:?}) are a subset of '{}' ({:?}). \
                         This fact type may be strongly redundant (Codd 1970 Sec 2.2.1) \
                         — derivable by projecting the larger relation.",
                        ft_b.reading, nouns_b, ft_a.reading, nouns_a
                    ),
                    alethic: false,
                });
            }
        }
    }

    violations
}

/// Detect redundant constraints via implication rules (Halpin PhD thesis Ch 6).
///
/// Known implications:
/// - UC on each individual role of a binary fact type → the spanning UC is redundant
///   (individual UCs already imply the pair can't repeat)
/// - Frequency(1,1) ≡ UC + MC (report as combined "exactly one")
/// - UC on a role where the noun is a subtype → check if supertype UC already covers it
pub fn check_constraint_implication(ir: &ConstraintIR) -> Vec<Violation> {
    let mut violations = Vec::new();

    for (ft_id, ft) in &ir.fact_types {
        if ft.roles.len() != 2 { continue; } // only binaries for now

        // Find UCs on this fact type
        let ucs: Vec<&ConstraintDef> = ir.constraints.iter()
            .filter(|c| c.kind == "UC" && c.spans.iter().any(|s| s.fact_type_id == *ft_id))
            .collect();

        // Check: UC on role 0 AND UC on role 1 → spanning UC is implied (1:1)
        let has_uc_role0 = ucs.iter().any(|c| {
            let spans: Vec<_> = c.spans.iter().filter(|s| s.fact_type_id == *ft_id).collect();
            spans.len() == 1 && spans[0].role_index == 0
        });
        let has_uc_role1 = ucs.iter().any(|c| {
            let spans: Vec<_> = c.spans.iter().filter(|s| s.fact_type_id == *ft_id).collect();
            spans.len() == 1 && spans[0].role_index == 1
        });
        let has_spanning = ucs.iter().any(|c| {
            c.spans.iter().filter(|s| s.fact_type_id == *ft_id).count() == 2
        });

        if has_uc_role0 && has_uc_role1 && has_spanning {
            violations.push(Violation {
                constraint_id: format!("implied-spanning:{}", ft_id),
                constraint_text: format!(
                    "Spanning UC on '{}' is redundant — implied by individual role UCs (1:1)",
                    ft.reading
                ),
                detail: format!(
                    "Both roles of '{}' have individual UCs, making the spanning UC redundant. \
                     Individual UCs on a binary already imply the spanning UC.",
                    ft.reading
                ),
                alethic: false, // informational, not a rejection
            });
        }

        // Check: FC(1,1) on a role is equivalent to UC + MC
        for c in &ir.constraints {
            if c.kind != "FC" { continue; }
            if !c.spans.iter().any(|s| s.fact_type_id == *ft_id) { continue; }
            if c.min_occurrence == Some(1) && c.max_occurrence == Some(1) {
                violations.push(Violation {
                    constraint_id: format!("fc11-equiv:{}", c.id),
                    constraint_text: format!(
                        "Frequency(1,1) on '{}' is equivalent to UC + MC (exactly one)",
                        ft.reading
                    ),
                    detail: format!(
                        "Frequency constraint with min=1, max=1 is equivalent to \
                         'exactly one' (uniqueness + mandatory combined)."
                    ),
                    alethic: false,
                });
            }
        }
    }

    violations
}

/// Check that all objectified fact types have spanning uniqueness constraints.
/// An objectified fact type without a spanning UC violates atomicity.
pub fn check_objectification_atomicity(ir: &ConstraintIR) -> Vec<Violation> {
    let mut violations = Vec::new();

    for (noun_name, objectified_reading) in &ir.objectifications {
        // Find the fact type being objectified
        let ft = ir.fact_types.get(objectified_reading);
        if ft.is_none() {
            violations.push(Violation {
                constraint_id: format!("objectification:{}", noun_name),
                constraint_text: format!("{} objectifies unknown fact type '{}'", noun_name, objectified_reading),
                detail: format!("Fact type '{}' not found in schema", objectified_reading),
                alethic: true,
            });
            continue;
        }
        let ft = ft.unwrap();
        let arity = ft.roles.len();

        // Check if a spanning UC exists (a UC that spans ALL roles of the fact type)
        let has_spanning_uc = ir.constraints.iter().any(|c| {
            if c.kind != "UC" { return false; }
            // A spanning UC has spans covering all roles of this fact type
            let spans_this_ft: Vec<&SpanDef> = c.spans.iter()
                .filter(|s| s.fact_type_id == *objectified_reading)
                .collect();
            spans_this_ft.len() == arity
        });

        if !has_spanning_uc {
            violations.push(Violation {
                constraint_id: format!("objectification:{}", noun_name),
                constraint_text: format!(
                    "Objectification of '{}' by {} requires a spanning uniqueness constraint",
                    objectified_reading, noun_name
                ),
                detail: format!(
                    "{} objectifies '{}' (arity {}) but no UC spans all {} roles. \
                     Objectification requires a spanning UC to ensure atomicity \
                     (Halpin, \"Objectification and Atomicity\", 2020).",
                    noun_name, objectified_reading, arity, arity
                ),
                alethic: true,
            });
        }
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_ir() -> ConstraintIR {
        let mut ir = ConstraintIR {
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
        // Person plays 3 roles (ft1:0, ft1:1→Name, ft-bad:0, ft-bad:1→Age)
        // But there's no "Noun has Object Type" for Age
        let noun_facts = pop.facts.get("Noun has Object Type").unwrap();
        assert!(!noun_facts.iter().any(|f| f.bindings.iter().any(|(_, v)| v == "Age")));
        // The role for Age exists but references an undeclared noun
        assert!(role_facts.iter().any(|f| f.bindings.iter().any(|(_, v)| v == "Age")));
    }

    // ── Objectification atomicity ───────────────────────────────

    #[test]
    fn objectification_without_spanning_uc_produces_violation() {
        let mut ir = simple_ir();
        // "Marriage" objectifies "Person has Name" — but this fact type has
        // a UC only on Person (not spanning both roles)
        ir.nouns.insert("Marriage".to_string(), NounDef {
            object_type: "entity".to_string(),
            world_assumption: WorldAssumption::default(),
        });
        ir.objectifications.insert("Marriage".to_string(), "ft1".to_string());
        ir.constraints.push(ConstraintDef {
            id: "uc-person-name".to_string(),
            kind: "UC".to_string(),
            modality: "Alethic".to_string(),
            text: "Each Person has at most one Name.".to_string(),
            // UC on role 0 only — NOT spanning (doesn't cover role 1)
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            deontic_operator: None, entity: None, clauses: None,
            set_comparison_argument_length: None, min_occurrence: None, max_occurrence: None,
        });

        let violations = check_objectification_atomicity(&ir);
        assert_eq!(violations.len(), 1, "expected objectification violation");
        assert!(violations[0].detail.contains("spanning"), "detail: {}", violations[0].detail);
    }

    #[test]
    fn objectification_with_spanning_uc_passes() {
        let mut ir = simple_ir();
        ir.nouns.insert("Marriage".to_string(), NounDef {
            object_type: "entity".to_string(),
            world_assumption: WorldAssumption::default(),
        });
        ir.objectifications.insert("Marriage".to_string(), "ft1".to_string());
        // Spanning UC: spans ALL roles (both role 0 and role 1)
        ir.constraints.push(ConstraintDef {
            id: "uc-spanning".to_string(),
            kind: "UC".to_string(),
            modality: "Alethic".to_string(),
            text: "Each Person, Name combination occurs at most once.".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 1, subset_autofill: None },
            ],
            deontic_operator: None, entity: None, clauses: None,
            set_comparison_argument_length: None, min_occurrence: None, max_occurrence: None,
        });

        let violations = check_objectification_atomicity(&ir);
        assert_eq!(violations.len(), 0, "no violation expected for spanning UC");
    }

    // ── N-ary UC arity check ────────────────────────────────────

    #[test]
    fn ternary_with_simple_uc_produces_violation() {
        let mut ir = simple_ir();
        // Add a ternary fact type
        ir.fact_types.insert("ft-ternary".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Plan charges Price per Interval".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Plan".to_string(), role_index: 0 },
                RoleDef { noun_name: "Price".to_string(), role_index: 1 },
                RoleDef { noun_name: "Interval".to_string(), role_index: 2 },
            ],
        });
        // Simple UC on one role — arity violation
        ir.constraints.push(ConstraintDef {
            id: "uc-bad".to_string(),
            kind: "UC".to_string(),
            modality: "Alethic".to_string(),
            text: "Each Plan charges at most one Price.".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft-ternary".to_string(), role_index: 0, subset_autofill: None }],
            deontic_operator: None, entity: None, clauses: None,
            set_comparison_argument_length: None, min_occurrence: None, max_occurrence: None,
        });

        let violations = check_nary_uc_arity(&ir);
        assert_eq!(violations.len(), 1, "expected arity violation for simple UC on ternary");
        assert!(violations[0].detail.contains("decomposing"), "detail: {}", violations[0].detail);
    }

    #[test]
    fn ternary_with_n_minus_1_uc_passes() {
        let mut ir = simple_ir();
        ir.fact_types.insert("ft-ternary".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Plan charges Price per Interval".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Plan".to_string(), role_index: 0 },
                RoleDef { noun_name: "Price".to_string(), role_index: 1 },
                RoleDef { noun_name: "Interval".to_string(), role_index: 2 },
            ],
        });
        // UC spanning 2 of 3 roles (n-1) — valid
        ir.constraints.push(ConstraintDef {
            id: "uc-good".to_string(),
            kind: "UC".to_string(),
            modality: "Alethic".to_string(),
            text: "For each Plan and Interval, at most one Price.".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft-ternary".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft-ternary".to_string(), role_index: 2, subset_autofill: None },
            ],
            deontic_operator: None, entity: None, clauses: None,
            set_comparison_argument_length: None, min_occurrence: None, max_occurrence: None,
        });

        let violations = check_nary_uc_arity(&ir);
        assert_eq!(violations.len(), 0, "no violation for n-1 UC on ternary");
    }

    // ── Subtype derivation rules ──────────────────────────────

    #[test]
    fn subtype_derivation_check_is_deferred() {
        // subtype_kind was removed from NounDef. The check is a no-op
        // until derivation rule presence can determine derived vs asserted.
        let ir = simple_ir();
        let violations = check_subtype_derivation_rules(&ir);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn binary_with_simple_uc_is_fine() {
        // Binary fact types with simple UC are normal (many-to-one)
        let mut ir = simple_ir();
        ir.constraints.push(ConstraintDef {
            id: "uc-binary".to_string(),
            kind: "UC".to_string(),
            modality: "Alethic".to_string(),
            text: "Each Person has at most one Name.".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            deontic_operator: None, entity: None, clauses: None,
            set_comparison_argument_length: None, min_occurrence: None, max_occurrence: None,
        });

        let violations = check_nary_uc_arity(&ir);
        assert_eq!(violations.len(), 0, "binary fact types with simple UC are valid");
    }

    // ── Constraint implication ──────────────────────────────────

    #[test]
    fn redundant_spanning_uc_detected() {
        let mut ir = simple_ir();
        // UC on role 0
        ir.constraints.push(ConstraintDef {
            id: "uc-r0".to_string(), kind: "UC".to_string(),
            modality: "Alethic".to_string(), text: "Each Person has at most one Name.".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            deontic_operator: None, entity: None, clauses: None,
            set_comparison_argument_length: None, min_occurrence: None, max_occurrence: None,
        });
        // UC on role 1
        ir.constraints.push(ConstraintDef {
            id: "uc-r1".to_string(), kind: "UC".to_string(),
            modality: "Alethic".to_string(), text: "Each Name belongs to at most one Person.".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 1, subset_autofill: None }],
            deontic_operator: None, entity: None, clauses: None,
            set_comparison_argument_length: None, min_occurrence: None, max_occurrence: None,
        });
        // Spanning UC (redundant — implied by the two individual UCs)
        ir.constraints.push(ConstraintDef {
            id: "uc-spanning".to_string(), kind: "UC".to_string(),
            modality: "Alethic".to_string(), text: "Each Person, Name combination occurs at most once.".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 1, subset_autofill: None },
            ],
            deontic_operator: None, entity: None, clauses: None,
            set_comparison_argument_length: None, min_occurrence: None, max_occurrence: None,
        });

        let violations = check_constraint_implication(&ir);
        assert!(violations.iter().any(|v| v.detail.contains("redundant")),
            "expected redundancy warning, got: {:?}", violations);
    }

    #[test]
    fn fc_1_1_detected_as_equivalent() {
        let mut ir = simple_ir();
        ir.constraints.push(ConstraintDef {
            id: "fc-11".to_string(), kind: "FC".to_string(),
            modality: "Alethic".to_string(), text: "Each Person has exactly 1 Name.".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            deontic_operator: None, entity: None, clauses: None,
            set_comparison_argument_length: None,
            min_occurrence: Some(1), max_occurrence: Some(1),
        });

        let violations = check_constraint_implication(&ir);
        assert!(violations.iter().any(|v| v.detail.contains("exactly one")),
            "expected FC(1,1) equivalence note, got: {:?}", violations);
    }

    #[test]
    fn no_implication_for_normal_schema() {
        let ir = simple_ir(); // no constraints at all
        let violations = check_constraint_implication(&ir);
        assert_eq!(violations.len(), 0);
    }

    // ── Satisfiability ──────────────────────────────────────────

    #[test]
    fn frequency_min_greater_than_max_is_unsatisfiable() {
        let mut ir = simple_ir();
        ir.constraints.push(ConstraintDef {
            id: "fc-bad".to_string(), kind: "FC".to_string(),
            modality: "Alethic".to_string(), text: "impossible frequency".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            deontic_operator: None, entity: None, clauses: None,
            set_comparison_argument_length: None,
            min_occurrence: Some(5), max_occurrence: Some(2),
        });
        let violations = check_satisfiability(&ir);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].detail.contains("No valid population"), "got: {}", violations[0].detail);
    }

    #[test]
    fn valid_frequency_is_satisfiable() {
        let mut ir = simple_ir();
        ir.constraints.push(ConstraintDef {
            id: "fc-ok".to_string(), kind: "FC".to_string(),
            modality: "Alethic".to_string(), text: "valid".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            deontic_operator: None, entity: None, clauses: None,
            set_comparison_argument_length: None,
            min_occurrence: Some(1), max_occurrence: Some(5),
        });
        let violations = check_satisfiability(&ir);
        assert_eq!(violations.len(), 0);
    }

    // ── Redundancy ──────────────────────────────────────────────

    #[test]
    fn subset_roles_detected_as_redundant() {
        let mut ir = simple_ir();
        // ft1 has roles {Person, Name}
        // Add ft2 with roles {Person, Name, Age} — ft1 is a subset
        ir.nouns.insert("Age".to_string(), NounDef {
            object_type: "value".to_string(),
            world_assumption: WorldAssumption::default(),
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Person has Name and Age".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
                RoleDef { noun_name: "Age".to_string(), role_index: 2 },
            ],
        });
        let violations = check_redundancy(&ir);
        assert!(violations.iter().any(|v| v.detail.contains("projecting")),
            "expected redundancy, got: {:?}", violations);
    }

    #[test]
    fn no_redundancy_for_independent_fact_types() {
        let mut ir = simple_ir();
        ir.nouns.insert("Country".to_string(), NounDef {
            object_type: "entity".to_string(),
            world_assumption: WorldAssumption::default(),
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Person was born in Country".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Country".to_string(), role_index: 1 },
            ],
        });
        // {Person, Name} and {Person, Country} — neither is subset of the other
        let violations = check_redundancy(&ir);
        assert_eq!(violations.len(), 0);
    }
}
