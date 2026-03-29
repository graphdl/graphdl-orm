// crates/fol-engine/src/verbalize.rs
//
// compile⁻¹: Recover FORML 2 readings from compiled constraints.
//
// Given a ConstraintDef (the IR representation), produce the FORML 2
// verbalization following Halpin & Curland's patterns (TechReport ORM2-02).
//
// This is the inverse of parse_forml2.rs → compile.rs. Together they
// close the specification equivalence loop (Theorem 2 of the AREST paper):
//   verbalize ∘ compile⁻¹ ∘ compile ∘ parse = id

use crate::types::*;

/// Verbalize a constraint back to its FORML 2 reading.
pub fn verbalize_constraint(constraint: &ConstraintDef, ir: &ConstraintIR) -> String {
    let modal = match (constraint.modality.as_str(), constraint.deontic_operator.as_deref()) {
        ("deontic", Some("forbidden")) => "It is forbidden that ",
        ("deontic", Some("obligatory")) => "It is obligatory that ",
        ("deontic", Some("permitted")) => "It is permitted that ",
        _ => "",
    };

    // Resolve fact type reading and role nouns from spans
    let (reading, role_nouns) = if let Some(span) = constraint.spans.first() {
        let ft = ir.fact_types.get(&span.fact_type_id);
        let reading = ft.map(|f| f.reading.as_str()).unwrap_or(&span.fact_type_id);
        let nouns: Vec<&str> = ft.map(|f| f.roles.iter().map(|r| r.noun_name.as_str()).collect())
            .unwrap_or_default();
        (reading.to_string(), nouns)
    } else {
        return constraint.text.clone(); // fallback to original text
    };

    let (noun_a, noun_b) = if role_nouns.len() >= 2 {
        (role_nouns[0], role_nouns[1])
    } else if role_nouns.len() == 1 {
        (role_nouns[0], "")
    } else {
        return format!("{}{}", modal, constraint.text);
    };

    // Extract predicate from reading: text between first and second noun
    let predicate = extract_predicate(&reading, noun_a, noun_b);

    match constraint.kind.as_str() {
        "UC" => {
            if constraint.modality == "deontic" && constraint.deontic_operator.as_deref() == Some("forbidden") {
                format!("It is forbidden that the same {} {} more than one {}.", noun_a, predicate, noun_b)
            } else {
                format!("{}Each {} {} at most one {}.", modal, noun_a, predicate, noun_b)
            }
        }
        "MC" => {
            format!("{}Each {} {} some {}.", modal, noun_a, predicate, noun_b)
        }
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
        "forbidden" => {
            format!("It is forbidden that {}.", constraint.text.trim_end_matches('.'))
        }
        "obligatory" => {
            format!("It is obligatory that {}.", constraint.text.trim_end_matches('.'))
        }
        "permitted" => {
            format!("It is permitted that {}.", constraint.text.trim_end_matches('.'))
        }
        _ => {
            format!("{}{}", modal, constraint.text)
        }
    }
}

/// Verbalize an entity type declaration.
pub fn verbalize_noun(name: &str, def: &NounDef) -> String {
    match def.object_type.as_str() {
        "entity" => {
            if let Some(ref refs) = def.ref_scheme {
                format!("{}(.{}) is an entity type.", name, refs.join(", "))
            } else {
                format!("{} is an entity type.", name)
            }
        }
        "value" => {
            let mut s = format!("{} is a value type.", name);
            if let Some(ref values) = def.enum_values {
                let quoted: Vec<String> = values.iter().map(|v| format!("'{}'", v)).collect();
                s.push_str(&format!("\n  The possible values of {} are {}.", name, quoted.join(", ")));
            }
            s
        }
        _ => format!("{} is an entity type.", name),
    }
}

/// Verbalize a subtype declaration.
pub fn verbalize_subtype(name: &str, def: &NounDef) -> Option<String> {
    def.super_type.as_ref().map(|sup| format!("{} is a subtype of {}.", name, sup))
}

/// Verbalize a fact type.
pub fn verbalize_fact_type(ft: &FactTypeDef) -> String {
    format!("{}.", ft.reading)
}

/// Verbalize an entire IR back to a FORML 2 document.
pub fn verbalize_ir(ir: &ConstraintIR) -> String {
    let mut lines: Vec<String> = Vec::new();

    // Domain header
    if !ir.domain.is_empty() {
        lines.push(format!("# {}", ir.domain));
        lines.push(String::new());
    }

    // Entity types
    let entities: Vec<(&String, &NounDef)> = ir.nouns.iter()
        .filter(|(_, d)| d.object_type == "entity" && d.super_type.is_none())
        .collect();
    if !entities.is_empty() {
        lines.push("## Entity Types".to_string());
        lines.push(String::new());
        for (name, def) in &entities {
            lines.push(verbalize_noun(name, def));
        }
        lines.push(String::new());
    }

    // Subtypes
    let subtypes: Vec<(&String, &NounDef)> = ir.nouns.iter()
        .filter(|(_, d)| d.super_type.is_some())
        .collect();
    if !subtypes.is_empty() {
        for (name, def) in &subtypes {
            if let Some(s) = verbalize_subtype(name, def) {
                lines.push(s);
            }
        }
        lines.push(String::new());
    }

    // Value types
    let values: Vec<(&String, &NounDef)> = ir.nouns.iter()
        .filter(|(_, d)| d.object_type == "value")
        .collect();
    if !values.is_empty() {
        lines.push("## Value Types".to_string());
        lines.push(String::new());
        for (name, def) in &values {
            lines.push(verbalize_noun(name, def));
        }
        lines.push(String::new());
    }

    // Fact types
    if !ir.fact_types.is_empty() {
        lines.push("## Fact Types".to_string());
        lines.push(String::new());
        for ft in ir.fact_types.values() {
            lines.push(verbalize_fact_type(ft));
        }
        lines.push(String::new());
    }

    // Constraints
    let alethic: Vec<&ConstraintDef> = ir.constraints.iter()
        .filter(|c| c.modality != "deontic")
        .collect();
    if !alethic.is_empty() {
        lines.push("## Constraints".to_string());
        lines.push(String::new());
        for c in &alethic {
            lines.push(verbalize_constraint(c, ir));
        }
        lines.push(String::new());
    }

    // Deontic constraints
    let deontic: Vec<&ConstraintDef> = ir.constraints.iter()
        .filter(|c| c.modality == "deontic")
        .collect();
    if !deontic.is_empty() {
        lines.push("## Deontic Constraints".to_string());
        lines.push(String::new());
        for c in &deontic {
            lines.push(verbalize_constraint(c, ir));
        }
        lines.push(String::new());
    }

    lines.join("\n")
}

fn extract_predicate(reading: &str, noun_a: &str, noun_b: &str) -> String {
    if let Some(start) = reading.find(noun_a) {
        let after_a = start + noun_a.len();
        if let Some(b_pos) = reading[after_a..].find(noun_b) {
            return reading[after_a..after_a + b_pos].trim().to_string();
        }
    }
    // Fallback: return everything after first noun
    reading.split_whitespace().skip(1).collect::<Vec<_>>().join(" ")
}

// ── Tests ───────────────────────────────────────────────────────────

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
        let uc_v = verbalize_constraint(uc, &ir);
        let mc_v = verbalize_constraint(mc, &ir);
        // UC verbalizes to "at most one", MC to "some" — canonical separated forms
        assert!(uc_v.contains("at most one"), "UC should say 'at most one', got: {}", uc_v);
        assert!(mc_v.contains("some"), "MC should say 'some', got: {}", mc_v);
    }

    #[test]
    fn verbalize_noun_entity() {
        let def = NounDef {
            object_type: "entity".to_string(),
            enum_values: None, value_type: None, super_type: None,
            world_assumption: WorldAssumption::default(),
            ref_scheme: Some(vec!["Email".to_string()]), objectifies: None, subtype_kind: None, rigid: false,
        };
        assert_eq!(verbalize_noun("Customer", &def), "Customer(.Email) is an entity type.");
    }

    #[test]
    fn verbalize_noun_value_with_enum() {
        let def = NounDef {
            object_type: "value".to_string(),
            enum_values: Some(vec!["M".to_string(), "F".to_string()]),
            value_type: None, super_type: None,
            world_assumption: WorldAssumption::default(),
            ref_scheme: None, objectifies: None, subtype_kind: None, rigid: false,
        };
        let v = verbalize_noun("Gender", &def);
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
