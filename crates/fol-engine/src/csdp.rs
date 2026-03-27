// crates/fol-engine/src/csdp.rs
//
// CSDP — Conceptual Schema Design Procedure (Halpin, Ch. 5-8)
//
// Validates a ConstraintIR against Halpin's quality checks:
//   Step 1: Undeclared noun check, non-elementary fact check
//   Step 4: Arity check (UC must span n-1 roles for arity n)
//   Step 6: Missing subtype constraint (XO/TO for partitions)
//   Step 7: Missing ring constraint (self-referential binaries)

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use crate::types::ConstraintIR;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsdpViolation {
    #[serde(rename = "type")]
    pub violation_type: String,
    pub message: String,
    pub fix: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fact_type_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsdpResult {
    pub valid: bool,
    pub violations: Vec<CsdpViolation>,
}

const RING_KINDS: &[&str] = &["IR", "AS", "AT", "SY", "IT", "TR", "AC"];

pub fn validate_csdp(ir: &ConstraintIR) -> CsdpResult {
    let mut violations = Vec::new();
    let declared_nouns: HashSet<&str> = ir.nouns.keys().map(|s| s.as_str()).collect();

    // Step 1: undeclared noun check
    for (ft_id, ft) in &ir.fact_types {
        for role in &ft.roles {
            if !declared_nouns.contains(role.noun_name.as_str()) {
                violations.push(CsdpViolation {
                    violation_type: "undeclared_noun".to_string(),
                    message: format!("Role references noun '{}' which is not declared.", role.noun_name),
                    fix: format!("Add '{}' to the nouns.", role.noun_name),
                    fact_type_id: Some(ft_id.clone()),
                });
            }
        }
    }

    // Step 1: non-elementary fact check
    for (ft_id, ft) in &ir.fact_types {
        let mut stripped = ft.reading.clone();
        for role in &ft.roles {
            stripped = stripped.replace(&role.noun_name, "");
        }
        if stripped.to_lowercase().contains(" and ") {
            violations.push(CsdpViolation {
                violation_type: "non_elementary_fact".to_string(),
                message: format!("Reading '{}' may conjoin independent assertions.", ft.reading),
                fix: format!("Split '{}' into separate elementary fact types.", ft.reading),
                fact_type_id: Some(ft_id.clone()),
            });
        }
    }

    // Step 4: arity check for UCs
    for constraint in &ir.constraints {
        if constraint.kind != "UC" { continue }
        if constraint.spans.is_empty() { continue }
        let ft_id = &constraint.spans[0].fact_type_id;
        let ft = match ir.fact_types.get(ft_id) {
            Some(ft) => ft,
            None => continue,
        };
        let arity = ft.roles.len();
        let uc_span = constraint.spans.len();
        if arity >= 3 && uc_span < arity - 1 {
            violations.push(CsdpViolation {
                violation_type: "arity_violation".to_string(),
                message: format!(
                    "UC on '{}' spans {} of {} roles. For arity {}, UC must span at least {} roles.",
                    ft.reading, uc_span, arity, arity, arity - 1
                ),
                fix: format!("Split '{}' into binary fact types, or extend the UC.", ft.reading),
                fact_type_id: Some(ft_id.clone()),
            });
        }
    }

    // Step 6: missing subtype constraint
    let mut subtypes_by_super: HashMap<&str, Vec<&str>> = HashMap::new();
    for (name, noun) in &ir.nouns {
        if let Some(ref st) = noun.super_type {
            subtypes_by_super.entry(st.as_str()).or_default().push(name.as_str());
        }
    }
    for (supertype, subs) in &subtypes_by_super {
        let has_constraint = ir.constraints.iter().any(|c| {
            (c.kind == "XO" || c.kind == "TO") && c.text.contains(supertype) && subs.iter().any(|s| c.text.contains(s))
        });
        if !has_constraint {
            violations.push(CsdpViolation {
                violation_type: "missing_subtype_constraint".to_string(),
                message: format!("Subtypes [{}] of '{}' have no totality or exclusion constraint.", subs.join(", "), supertype),
                fix: format!("Add a totality (TO) and/or exclusion (XO) constraint for the {} subtype partition.", supertype),
                fact_type_id: None,
            });
        }
    }

    // Step 7: missing ring constraint
    for (ft_id, ft) in &ir.fact_types {
        if ft.roles.len() != 2 { continue }
        if ft.roles[0].noun_name != ft.roles[1].noun_name { continue }

        let has_ring = ir.constraints.iter().any(|c| {
            c.spans.iter().any(|s| s.fact_type_id == *ft_id) && RING_KINDS.contains(&c.kind.as_str())
        });
        if !has_ring {
            violations.push(CsdpViolation {
                violation_type: "missing_ring_constraint".to_string(),
                message: format!("'{}' is self-referential but has no ring constraint.", ft.reading),
                fix: format!("Add a ring constraint (IR, AS, SY, TR, AC) to '{}'.", ft.reading),
                fact_type_id: Some(ft_id.clone()),
            });
        }
    }

    CsdpResult { valid: violations.is_empty(), violations }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

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

    #[test]
    fn valid_schema_passes() {
        let mut ir = empty_ir();
        ir.nouns.insert("Person".to_string(), NounDef { object_type: "entity".to_string(), enum_values: None, value_type: None, super_type: None, world_assumption: WorldAssumption::default(), ref_scheme: None });
        ir.nouns.insert("Name".to_string(), NounDef { object_type: "value".to_string(), enum_values: None, value_type: None, super_type: None, world_assumption: WorldAssumption::default(), ref_scheme: None });
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Person has Name".to_string(),
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }, RoleDef { noun_name: "Name".to_string(), role_index: 1 }],
        });
        let result = validate_csdp(&ir);
        assert!(result.valid);
    }

    #[test]
    fn undeclared_noun_detected() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Person has Name".to_string(),
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        // "Person" not in nouns
        let result = validate_csdp(&ir);
        assert!(!result.valid);
        assert_eq!(result.violations[0].violation_type, "undeclared_noun");
    }

    #[test]
    fn self_referential_binary_needs_ring_constraint() {
        let mut ir = empty_ir();
        ir.nouns.insert("Academic".to_string(), NounDef { object_type: "entity".to_string(), enum_values: None, value_type: None, super_type: None, world_assumption: WorldAssumption::default(), ref_scheme: None });
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Academic audits Academic".to_string(),
            roles: vec![RoleDef { noun_name: "Academic".to_string(), role_index: 0 }, RoleDef { noun_name: "Academic".to_string(), role_index: 1 }],
        });
        let result = validate_csdp(&ir);
        assert!(!result.valid);
        assert_eq!(result.violations[0].violation_type, "missing_ring_constraint");
    }

    #[test]
    fn arity_violation_on_ternary() {
        let mut ir = empty_ir();
        ir.nouns.insert("A".to_string(), NounDef { object_type: "entity".to_string(), enum_values: None, value_type: None, super_type: None, world_assumption: WorldAssumption::default(), ref_scheme: None });
        ir.nouns.insert("B".to_string(), NounDef { object_type: "entity".to_string(), enum_values: None, value_type: None, super_type: None, world_assumption: WorldAssumption::default(), ref_scheme: None });
        ir.nouns.insert("C".to_string(), NounDef { object_type: "entity".to_string(), enum_values: None, value_type: None, super_type: None, world_assumption: WorldAssumption::default(), ref_scheme: None });
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "A R B C".to_string(),
            roles: vec![
                RoleDef { noun_name: "A".to_string(), role_index: 0 },
                RoleDef { noun_name: "B".to_string(), role_index: 1 },
                RoleDef { noun_name: "C".to_string(), role_index: 2 },
            ],
        });
        // UC spans only 1 of 3 roles (needs at least 2)
        ir.constraints.push(ConstraintDef {
            id: "uc1".to_string(), kind: "UC".to_string(), modality: "Alethic".to_string(),
            text: String::new(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            ..Default::default()
        });
        let result = validate_csdp(&ir);
        assert!(result.violations.iter().any(|v| v.violation_type == "arity_violation"));
    }
}
