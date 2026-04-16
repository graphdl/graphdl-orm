// crates/arest/src/verbalize.rs
//
// compile-^1: Recover FORML 2 readings from compiled constraints.
//
// Given a ConstraintDef (the IR representation), produce the FORML 2
// verbalization following Halpin & Curland's patterns (TechReport ORM2-02).
//
// This is the inverse of parse_forml2.rs -> compile.rs. Together they
// close the specification equivalence loop (Theorem 2 of the AREST paper):
//   verbalize  .  compile-^1  .  compile  .  parse = id

use crate::types::*;
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

/// Verbalize a constraint back to its FORML 2 reading.
pub fn verbalize_constraint(constraint: &ConstraintDef, ir: &Domain) -> String {
    let modal = match (constraint.modality.as_str(), constraint.deontic_operator.as_deref()) {
        ("deontic", Some("forbidden")) => "It is forbidden that ",
        ("deontic", Some("obligatory")) => "It is obligatory that ",
        ("deontic", Some("permitted")) => "It is permitted that ",
        _ => "",
    };

    // Resolve fact type reading and role nouns from spans
    let span_info = constraint.spans.first().map(|span| {
        let ft = ir.fact_types.get(&span.fact_type_id);
        let reading = ft.map(|f| f.reading.as_str()).unwrap_or(&span.fact_type_id);
        let nouns: Vec<&str> = ft.map(|f| f.roles.iter().map(|r| r.noun_name.as_str()).collect())
            .unwrap_or_default();
        (reading.to_string(), nouns)
    });

    // Derive (noun_a, noun_b, reading) via expression combinators; None cases fall through to defaults.
    let resolved: Option<(String, &str, &str)> = span_info.as_ref().and_then(|(reading, nouns)| {
        match nouns.len() {
            0 => None,
            1 => Some((reading.clone(), nouns[0], "")),
            _ => Some((reading.clone(), nouns[0], nouns[1])),
        }
    });

    match resolved {
        None if span_info.is_none() => constraint.text.clone(),
        None => format!("{}{}", modal, constraint.text),
        Some((reading, noun_a, noun_b)) => {
            let predicate = extract_predicate(&reading, noun_a, noun_b);
            match constraint.kind.as_str() {
                "UC" => {
                    if constraint.modality == "deontic" && constraint.deontic_operator.as_deref() == Some("forbidden") {
                        format!("It is forbidden that the same {} {} more than one {}.", noun_a, predicate, noun_b)
                    } else {
                        format!("{}Each {} {} at most one {}.", modal, noun_a, predicate, noun_b)
                    }
                }
                "MC" => format!("{}Each {} {} some {}.", modal, noun_a, predicate, noun_b),
                "FC" => {
                    let min = constraint.min_occurrence.map(|n| n.to_string()).unwrap_or_default();
                    let max = constraint.max_occurrence.map(|n| n.to_string()).unwrap_or_default();
                    if !min.is_empty() && !max.is_empty() {
                        format!("{}Each {} {} at least {} and at most {} {}.", modal, noun_a, predicate, min, max, noun_b)
                    } else if !min.is_empty() {
                        format!("{}Each {} {} at least {} {}.", modal, noun_a, predicate, min, noun_b)
                    } else {
                        format!("{}Each {} {} at most {} {}.", modal, noun_a, predicate, max, noun_b)
                    }
                }
                "forbidden" => format!("It is forbidden that {}.", constraint.text.trim_end_matches('.')),
                "obligatory" => format!("It is obligatory that {}.", constraint.text.trim_end_matches('.')),
                "permitted" => format!("It is permitted that {}.", constraint.text.trim_end_matches('.')),
                _ => format!("{}{}", modal, constraint.text),
            }
        }
    }
}

/// Verbalize an entity type declaration.
pub fn verbalize_noun(name: &str, def: &NounDef, ir: &Domain) -> String {
    match def.object_type.as_str() {
        "entity" => {
            if let Some(refs) = ir.ref_schemes.get(name) {
                format!("{}(.{}) is an entity type.", name, refs.join(", "))
            } else {
                format!("{} is an entity type.", name)
            }
        }
        "value" => {
            let base = format!("{} is a value type.", name);
            let enum_suffix = ir.enum_values.get(name).map(|values| {
                let quoted: Vec<String> = values.iter().map(|v| format!("'{}'", v)).collect();
                format!("\n  The possible values of {} are {}.", name, quoted.join(", "))
            }).unwrap_or_default();
            format!("{}{}", base, enum_suffix)
        }
        _ => format!("{} is an entity type.", name),
    }
}

/// Verbalize a subtype declaration.
pub fn verbalize_subtype(name: &str, ir: &Domain) -> Option<String> {
    ir.subtypes.get(name).map(|sup| format!("{} is a subtype of {}.", name, sup))
}

/// Verbalize a fact type.
pub fn verbalize_fact_type(ft: &FactTypeDef) -> String {
    format!("{}.", ft.reading)
}

/// Build a titled section: header lines + body lines + trailing blank, or empty if body is empty.
fn ir_section(title: Option<&str>, body: Vec<String>) -> Vec<String> {
    body.is_empty().then(Vec::new).unwrap_or_else(|| {
        let header: Vec<String> = title
            .map(|t| vec![format!("## {}", t), String::new()])
            .unwrap_or_default();
        header.into_iter()
            .chain(body.into_iter())
            .chain(core::iter::once(String::new()))
            .collect()
    })
}

/// Verbalize an entire IR back to a FORML 2 document.
pub fn verbalize_ir(ir: &Domain) -> String {
    // Domain header as an optional pair of lines.
    let header: Vec<String> = (!ir.domain.is_empty())
        .then(|| vec![format!("# {}", ir.domain), String::new()])
        .unwrap_or_default();

    // Entity types (non-subtypes)
    let entities: Vec<String> = ir.nouns.iter()
        .filter(|(name, d)| d.object_type == "entity" && !ir.subtypes.contains_key(*name))
        .map(|(name, def)| verbalize_noun(name, def, ir))
        .collect();

    // Subtypes (no header section)
    let subtype_lines: Vec<String> = ir.subtypes.keys()
        .filter_map(|name| verbalize_subtype(name, ir)).collect();

    // Value types
    let values: Vec<String> = ir.nouns.iter()
        .filter(|(_, d)| d.object_type == "value")
        .map(|(name, def)| verbalize_noun(name, def, ir))
        .collect();

    // Fact types
    let fact_types: Vec<String> = ir.fact_types.values().map(verbalize_fact_type).collect();

    // Constraints (alethic)
    let alethic: Vec<String> = ir.constraints.iter()
        .filter(|c| c.modality != "deontic")
        .map(|c| verbalize_constraint(c, ir))
        .collect();

    // Deontic constraints
    let deontic: Vec<String> = ir.constraints.iter()
        .filter(|c| c.modality == "deontic")
        .map(|c| verbalize_constraint(c, ir))
        .collect();

    let lines: Vec<String> = header.into_iter()
        .chain(ir_section(Some("Entity Types"), entities))
        .chain(ir_section(None, subtype_lines))
        .chain(ir_section(Some("Value Types"), values))
        .chain(ir_section(Some("Fact Types"), fact_types))
        .chain(ir_section(Some("Constraints"), alethic))
        .chain(ir_section(Some("Deontic Constraints"), deontic))
        .collect();

    lines.join("\n")
}

fn extract_predicate(reading: &str, noun_a: &str, noun_b: &str) -> String {
    reading.find(noun_a)
        .map(|start| start + noun_a.len())
        .and_then(|after_a| reading[after_a..].find(noun_b).map(|b_pos| (after_a, b_pos)))
        .map(|(after_a, b_pos)| reading[after_a..after_a + b_pos].trim().to_string())
        // Fallback: return everything after first noun
        .unwrap_or_else(|| reading.split_whitespace().skip(1).collect::<Vec<_>>().join(" "))
}

// -- Tests -----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_forml2;

    #[test]
    fn verbalize_uc() {
        let input = "# T\n\n## Entity Types\n\nPerson(.Name) is an entity type.\nCountry(.Code) is an entity type.\n\n## Fact Types\n\nPerson was born in Country.\n\n## Constraints\n\nEach Person was born in at most one Country.";
        let ir = parse_forml2::parse_markdown(input).unwrap();
        let uc = ir.constraints.iter().find(|c| c.kind == "UC").unwrap();
        let v = verbalize_constraint(uc, &ir);
        assert!(v.contains("at most one"), "got: {}", v);
        assert!(v.contains("Person"), "got: {}", v);
        assert!(v.contains("Country"), "got: {}", v);
    }

    #[test]
    fn verbalize_mc() {
        let input = "# T\n\nPerson(.Name) is an entity type.\nCountry(.Code) is an entity type.\nPerson was born in Country.\nEach Person was born in some Country.";
        let ir = parse_forml2::parse_markdown(input).unwrap();
        let mc = ir.constraints.iter().find(|c| c.kind == "MC").unwrap();
        let v = verbalize_constraint(mc, &ir);
        assert!(v.contains("some"), "got: {}", v);
    }

    #[test]
    fn verbalize_exactly_one_produces_uc_and_mc() {
        let input = "# T\n\nPerson(.Name) is an entity type.\nCountry(.Code) is an entity type.\nPerson was born in Country.\nEach Person was born in exactly one Country.";
        let ir = parse_forml2::parse_markdown(input).unwrap();
        let uc = ir.constraints.iter().find(|c| c.kind == "UC").unwrap();
        let mc = ir.constraints.iter().find(|c| c.kind == "MC").unwrap();
        // Both UC and MC split from "exactly one" -- verify both exist
        assert_eq!(uc.kind, "UC");
        assert_eq!(mc.kind, "MC");
        // Verbalization returns the constraint text
        let uc_v = verbalize_constraint(uc, &ir);
        let mc_v = verbalize_constraint(mc, &ir);
        assert!(!uc_v.is_empty());
        assert!(!mc_v.is_empty());
    }

    #[test]
    fn verbalize_noun_entity() {
        let def = NounDef {
            object_type: "entity".to_string(),
            world_assumption: WorldAssumption::default(),
        };
        let mut ir = Domain::default();
        ir.ref_schemes.insert("Customer".to_string(), vec!["Email".to_string()]);
        assert_eq!(verbalize_noun("Customer", &def, &ir), "Customer(.Email) is an entity type.");
    }

    #[test]
    fn verbalize_noun_value_with_enum() {
        let def = NounDef {
            object_type: "value".to_string(),
            world_assumption: WorldAssumption::default(),
        };
        let mut ir = Domain::default();
        ir.enum_values.insert("Gender".to_string(), vec!["M".to_string(), "F".to_string()]);
        let v = verbalize_noun("Gender", &def, &ir);
        assert!(v.contains("Gender is a value type."), "got: {}", v);
        assert!(v.contains("'M', 'F'"), "got: {}", v);
    }

    #[test]
    fn round_trip_parse_verbalize() {
        let original = "# Orders\n\n## Entity Types\n\nOrder(.Order Number) is an entity type.\nCustomer(.Name) is an entity type.\n\n## Fact Types\n\nOrder was placed by Customer.\n\n## Constraints\n\nEach Order was placed by at most one Customer.\nEach Order was placed by some Customer.\n";
        let ir = parse_forml2::parse_markdown(original).unwrap();
        let verbalized = verbalize_ir(&ir);

        // Re-parse the verbalized output
        let ir2 = parse_forml2::parse_markdown(&verbalized).unwrap();

        // Same number of nouns, fact types, constraints
        assert_eq!(ir.nouns.len(), ir2.nouns.len(), "noun count mismatch");
        assert_eq!(ir.fact_types.len(), ir2.fact_types.len(), "fact type count mismatch");
        assert_eq!(ir.constraints.len(), ir2.constraints.len(),
            "constraint count mismatch: original {} vs re-parsed {}\nverbalized:\n{}",
            ir.constraints.len(), ir2.constraints.len(), verbalized);
    }
}
