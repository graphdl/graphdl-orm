// crates/fol-engine/src/parse_forml2.rs
//
// FORML 2 Markdown Parser — reads ORM2 readings directly, produces ConstraintIR.
//
// Replaces both:
//   - parse_rule.rs (`:=` derivation syntax → FORML 2 `iff`/`if`/attribute style)
//   - src/claims/*.ts (TypeScript parser → Rust/WASM)
//
// The parser recognizes Halpin's established verbalization patterns:
//   - Entity types: "X(.Ref) is an entity type."
//   - Value types: "X is a value type." + "The possible values of X are ..."
//   - Subtypes: "X is a subtype of Y."
//   - Fact types: lines with 2+ known nouns connected by verb text
//   - Constraints: "Each X R at most one Y", "exactly one", frequency, deontic
//   - Derivation rules: iff, if, subtype derivation, aggregation, attribute style
//   - State machines: instance facts for SM Definition, Status, Transition
//
// Sources: Halpin & Curland, ORM 2 Constraint Verbalization (TechReport ORM2-02, 2006)
//          Halpin, Information Modeling and Relational Databases (2001/2008)

use crate::types::*;
use std::collections::HashMap;

/// Parse a FORML 2 markdown document into a ConstraintIR.
pub fn parse_markdown(input: &str) -> Result<ConstraintIR, String> {
    let mut ir = ConstraintIR {
        domain: String::new(),
        nouns: HashMap::new(),
        fact_types: HashMap::new(),
        constraints: vec![],
        state_machines: HashMap::new(),
        derivation_rules: vec![],
    };

    let lines: Vec<&str> = input.lines().collect();

    // ── Pass 1: Extract nouns and domain ────────────────────────────
    for i in 0..lines.len() {
        let line = lines[i].trim();
        if line.is_empty() { continue; }

        // Domain name from H1
        if line.starts_with("# ") && !line.starts_with("## ") && ir.domain.is_empty() {
            ir.domain = line[2..].trim().to_string();
            continue;
        }

        // Skip markdown headers
        if line.starts_with('#') { continue; }

        // Entity type: "X(.Ref) is an entity type."
        if line.ends_with("is an entity type.") {
            if let Some((name, ref_scheme)) = parse_entity_type_decl(line) {
                ir.nouns.entry(name).or_insert(NounDef {
                    object_type: "entity".to_string(),
                    enum_values: None,
                    value_type: None,
                    super_type: None,
                    world_assumption: WorldAssumption::default(),
                    ref_scheme,
                    objectifies: None, subtype_kind: None, rigid: false,
                });
            }
            continue;
        }

        // Value type: "X is a value type."
        if line.ends_with("is a value type.") {
            let name = line.trim_end_matches(" is a value type.").trim().to_string();
            let mut enum_values = None;
            // Check next non-empty line for "The possible values of X are ..."
            for j in (i + 1)..lines.len() {
                let next = lines[j].trim();
                if next.is_empty() { continue; }
                if next.starts_with("The possible values of") {
                    enum_values = parse_enum_values(next);
                }
                break;
            }
            ir.nouns.entry(name).or_insert(NounDef {
                object_type: "value".to_string(),
                enum_values,
                value_type: None,
                super_type: None,
                world_assumption: WorldAssumption::default(),
                ref_scheme: None, objectifies: None, subtype_kind: None, rigid: false,
            });
            continue;
        }

        // Subtype: "X is a subtype of Y."
        if line.contains(" is a subtype of ") && line.ends_with('.') {
            let clean = line.trim_end_matches('.');
            if let Some(idx) = clean.find(" is a subtype of ") {
                let sub = clean[..idx].trim().to_string();
                let sup = clean[idx + 17..].trim().to_string();
                ir.nouns.entry(sub).or_insert(NounDef {
                    object_type: "entity".to_string(),
                    enum_values: None,
                    value_type: None,
                    super_type: Some(sup),
                    world_assumption: WorldAssumption::default(),
                    ref_scheme: None, objectifies: None, subtype_kind: None, rigid: false,
                });
            }
            continue;
        }

        // Mutually exclusive subtypes: "{A, B} are mutually exclusive subtypes of C."
        if line.starts_with('{') && line.contains("subtypes of") {
            // Parse later — nouns should already be declared
            continue;
        }
    }

    // ── Pass 2: Recognize everything by syntax, not by headers ────
    // The reading syntax itself determines what a line is.
    // Headers are ignored — a constraint is a constraint regardless of
    // which section it appears in.
    let noun_names: Vec<String> = ir.nouns.keys().cloned().collect();

    for i in 0..lines.len() {
        let line = lines[i].trim();
        if line.is_empty() { continue; }

        // Skip markdown headers and declarations handled in pass 1
        if line.starts_with('#') { continue; }
        if line.ends_with("is an entity type.") || line.ends_with("is a value type.")
            || (line.contains(" is a subtype of ") && !line.starts_with("Each"))
            || line.starts_with("The possible values of")
            || line.starts_with('{')
            || line.starts_with("This association with")
        { continue; }

        // ── Instance facts (state machines) — recognized by syntax ──
        if line.starts_with("State Machine Definition '")
            || (line.starts_with("Status '") && line.contains("is initial in"))
            || line.starts_with("Transition '")
            || (line.starts_with("Domain '") && line.contains("has")) {
            parse_instance_fact(&mut ir, line, &lines, i);
            continue;
        }

        // ── Derivation rules — recognized by iff/if/attribute/aggregate ──
        if line.contains(" iff ") || line.contains(" is derived as ")
            || (line.starts_with("For each ") && line.contains(" = "))
            || line.contains("count each") || line.contains("sum(") {
            if let Some(rule) = parse_derivation_rule(line, &noun_names) {
                ir.derivation_rules.push(rule);
                continue;
            }
        }

        // ── External UC with context (Halpin TechReport ORM2-02 Sec 2.2) ──
        if line.starts_with("Context:") || line.starts_with("In this context,") {
            if line.starts_with("In this context,") {
                let inner = line.trim_start_matches("In this context,").trim();
                if let Some(constraints) = parse_constraint_line(inner, &noun_names) {
                    ir.constraints.extend(constraints);
                }
            }
            continue;
        }

        // ── Constraints — recognized by quantifier/modality syntax ──
        if line.starts_with("Each ") || line.starts_with("For each ")
            || line.starts_with("It is forbidden") || line.starts_with("It is obligatory")
            || line.starts_with("It is permitted") || line.starts_with("It is possible")
            || line.starts_with("It is impossible") || line.starts_with("It is necessary") {
            if let Some(constraints) = parse_constraint_line(line, &noun_names) {
                ir.constraints.extend(constraints);
                continue;
            }
        }

        // ── Fact types — any line with 2+ known nouns connected by verb text ──
        if let Some((ft_id, ft)) = parse_fact_type(line, &noun_names) {
            ir.fact_types.insert(ft_id, ft);
            continue;
        }
    }

    Ok(ir)
}

// ── Entity type declaration ─────────────────────────────────────────

fn parse_entity_type_decl(line: &str) -> Option<(String, Option<Vec<String>>)> {
    let prefix = line.trim_end_matches(" is an entity type.").trim();
    if prefix == line { return None; }

    if let Some(paren_start) = prefix.find("(.") {
        let name = prefix[..paren_start].trim().to_string();
        let paren_end = prefix.rfind(')')?;
        let ref_str = &prefix[paren_start + 2..paren_end];
        let refs: Vec<String> = ref_str.split(", ").map(|s| s.trim().to_string()).collect();
        Some((name, Some(refs)))
    } else if let Some(paren_start) = prefix.find('(') {
        // Alternate form: "X(Y)" without dot
        let name = prefix[..paren_start].trim().to_string();
        let paren_end = prefix.rfind(')')?;
        let ref_str = &prefix[paren_start + 1..paren_end];
        let refs: Vec<String> = ref_str.split(", ").map(|s| s.trim().trim_start_matches('.').to_string()).collect();
        Some((name, Some(refs)))
    } else {
        Some((prefix.to_string(), None))
    }
}

fn parse_enum_values(line: &str) -> Option<Vec<String>> {
    let after_are = line.split(" are ").nth(1)?;
    let trimmed = after_are.trim_end_matches('.');
    let values: Vec<String> = trimmed.split(", ")
        .map(|s| s.trim().trim_matches('\'').to_string())
        .collect();
    Some(values)
}

// ── Fact type parsing ───────────────────────────────────────────────

fn parse_fact_type(line: &str, noun_names: &[String]) -> Option<(String, FactTypeDef)> {
    let clean = line.trim_end_matches('.');
    if clean.is_empty() { return None; }

    let found = find_nouns_in_text(clean, noun_names);
    if found.len() < 2 { return None; }

    // Binary fact type: first two nouns with predicate between them
    let predicate = clean[found[0].end..found[1].start].trim();
    if predicate.is_empty() { return None; }

    let reading = format!("{} {} {}", found[0].name, predicate, found[1].name);
    let ft_id = reading.clone();

    let mut roles = vec![
        RoleDef { noun_name: found[0].name.clone(), role_index: 0 },
        RoleDef { noun_name: found[1].name.clone(), role_index: 1 },
    ];

    // N-ary: additional nouns become additional roles
    for (i, noun_match) in found.iter().enumerate().skip(2) {
        roles.push(RoleDef { noun_name: noun_match.name.clone(), role_index: i });
    }

    Some((ft_id, FactTypeDef { reading, roles }))
}

// ── Constraint parsing ─────────────────────────────────────────────

fn parse_constraint_line(line: &str, noun_names: &[String]) -> Option<Vec<ConstraintDef>> {
    let clean = line.trim_end_matches('.');

    // Determine modality
    let (modality, deontic_op, text) = if clean.starts_with("It is forbidden that ") {
        ("deontic".to_string(), Some("forbidden".to_string()), clean.trim_start_matches("It is forbidden that ").trim())
    } else if clean.starts_with("It is obligatory that ") {
        ("deontic".to_string(), Some("obligatory".to_string()), clean.trim_start_matches("It is obligatory that ").trim())
    } else if clean.starts_with("It is permitted that ") {
        ("deontic".to_string(), Some("permitted".to_string()), clean.trim_start_matches("It is permitted that ").trim())
    } else {
        ("alethic".to_string(), None, clean)
    };

    let mut constraints = Vec::new();
    let found = find_nouns_in_text(text, noun_names);

    // "Each X R exactly one Y" → UC + MC
    if text.contains("exactly one") && text.starts_with("Each ") && found.len() >= 2 {
        let reading = infer_reading(&found, text);
        let ft_id = reading.clone();
        constraints.push(make_constraint("UC", &modality, deontic_op.clone(), line, &ft_id, 0, None, None));
        constraints.push(make_constraint("MC", &modality, deontic_op, line, &ft_id, 0, None, None));
        return Some(constraints);
    }

    // "Each X R at most one Y" → UC
    if text.contains("at most one") && found.len() >= 2 {
        let reading = infer_reading(&found, text);
        constraints.push(make_constraint("UC", &modality, deontic_op, line, &reading, 0, None, None));
        return Some(constraints);
    }

    // "Each X R some Y" → MC
    if text.starts_with("Each ") && text.contains(" some ") && !text.contains("at most") && found.len() >= 2 {
        let reading = infer_reading(&found, text);
        constraints.push(make_constraint("MC", &modality, deontic_op, line, &reading, 0, None, None));
        return Some(constraints);
    }

    // "Each X R at least N and at most M Y" → FC
    if text.contains("at least") && text.contains("at most") {
        let reading = infer_reading(&found, text);
        let min = extract_number_after(text, "at least");
        let max = extract_number_after(text, "at most");
        constraints.push(make_constraint("FC", &modality, deontic_op, line, &reading, 0, min, max));
        return Some(constraints);
    }

    // "the same X R more than one Y" (negative UC, forbidden form)
    if text.contains("more than one") && found.len() >= 2 {
        let reading = infer_reading(&found, text);
        constraints.push(make_constraint("UC", &modality, deontic_op, line, &reading, 0, None, None));
        return Some(constraints);
    }

    // Deontic obligation/prohibition without specific pattern — store as text constraint
    if deontic_op.is_some() && found.len() >= 1 {
        let reading = if found.len() >= 2 { infer_reading(&found, text) } else { text.to_string() };
        let kind = match deontic_op.as_deref() {
            Some("forbidden") => "forbidden",
            Some("obligatory") => "obligatory",
            Some("permitted") => "permitted",
            _ => "UC",
        };
        constraints.push(make_constraint(kind, &modality, deontic_op, line, &reading, 0, None, None));
        return Some(constraints);
    }

    if constraints.is_empty() { None } else { Some(constraints) }
}

fn make_constraint(
    kind: &str, modality: &str, deontic_op: Option<String>,
    text: &str, ft_id: &str, role_index: usize,
    min: Option<usize>, max: Option<usize>,
) -> ConstraintDef {
    ConstraintDef {
        id: format!("{}:{}", kind, ft_id),
        kind: kind.to_string(),
        modality: modality.to_string(),
        deontic_operator: deontic_op,
        text: text.to_string(),
        spans: vec![SpanDef { fact_type_id: ft_id.to_string(), role_index, subset_autofill: None }],
        set_comparison_argument_length: None,
        clauses: None,
        entity: None,
        min_occurrence: min,
        max_occurrence: max,
    }
}

fn extract_number_after(text: &str, marker: &str) -> Option<usize> {
    let idx = text.find(marker)? + marker.len();
    let rest = text[idx..].trim();
    rest.split_whitespace().next()?.parse().ok()
}

// ── Derivation rule parsing ─────────────────────────────────────────

fn parse_derivation_rule(line: &str, noun_names: &[String]) -> Option<DerivationRuleDef> {
    let clean = line.trim_end_matches('.');

    // "X iff Y" — full biconditional derivation
    if let Some(iff_idx) = clean.find(" iff ") {
        let consequent_text = clean[..iff_idx].trim();
        let antecedent_text = clean[iff_idx + 5..].trim();

        let consequent_nouns = find_nouns_in_text(consequent_text, noun_names);
        let antecedent_nouns = find_nouns_in_text(antecedent_text, noun_names);

        let consequent_ft = if consequent_nouns.len() >= 2 {
            infer_reading(&consequent_nouns, consequent_text)
        } else {
            consequent_text.to_string()
        };

        let antecedent_fts: Vec<String> = split_conjuncts(antecedent_text)
            .iter()
            .map(|clause| {
                let clause_nouns = find_nouns_in_text(clause, noun_names);
                if clause_nouns.len() >= 2 {
                    infer_reading(&clause_nouns, clause)
                } else {
                    clause.to_string()
                }
            })
            .collect();

        return Some(DerivationRuleDef {
            id: format!("deriv:{}", consequent_ft),
            text: line.to_string(),
            antecedent_fact_type_ids: antecedent_fts,
            consequent_fact_type_id: consequent_ft,
            kind: if antecedent_nouns.len() > 2 { DerivationKind::Join } else { DerivationKind::ModusPonens },
            join_on: vec![],
            match_on: vec![],
            consequent_bindings: vec![],
        });
    }

    // "Each X is a Y who Z" — subtype derivation
    if clean.starts_with("Each ") && clean.contains(" is a ") && clean.contains(" who ") {
        return Some(DerivationRuleDef {
            id: format!("deriv:subtype:{}", clean),
            text: line.to_string(),
            antecedent_fact_type_ids: vec![],
            consequent_fact_type_id: clean.to_string(),
            kind: DerivationKind::SubtypeInheritance,
            join_on: vec![],
            match_on: vec![],
            consequent_bindings: vec![],
        });
    }

    // "For each X: y = expr" — attribute style
    if clean.starts_with("For each ") && clean.contains(": ") && clean.contains(" = ") {
        return Some(DerivationRuleDef {
            id: format!("deriv:attr:{}", clean),
            text: line.to_string(),
            antecedent_fact_type_ids: vec![],
            consequent_fact_type_id: clean.to_string(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![],
            match_on: vec![],
            consequent_bindings: vec![],
        });
    }

    // Aggregate: "X = count each Y who Z"
    if clean.contains("count each") || clean.contains("sum(") {
        return Some(DerivationRuleDef {
            id: format!("deriv:agg:{}", clean),
            text: line.to_string(),
            antecedent_fact_type_ids: vec![],
            consequent_fact_type_id: clean.to_string(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![],
            match_on: vec![],
            consequent_bindings: vec![],
        });
    }

    None
}

// ── Instance fact parsing (state machines) ──────────────────────────

fn parse_instance_fact(ir: &mut ConstraintIR, line: &str, _lines: &[&str], _idx: usize) {
    let clean = line.trim_end_matches('.');

    // "State Machine Definition 'X' is for Noun 'Y'."
    if clean.starts_with("State Machine Definition '") && clean.contains("' is for Noun '") {
        let sm_name = extract_quoted(clean, "State Machine Definition '");
        let noun_name = extract_quoted(clean, "' is for Noun '")
            .or_else(|| extract_quoted(clean, "is for Noun '"));
        if let (Some(sm), Some(noun)) = (sm_name, noun_name) {
            ir.state_machines.entry(sm.clone()).or_insert(StateMachineDef {
                noun_name: noun,
                statuses: vec![],
                transitions: vec![],
            });
        }
        return;
    }

    // "Status 'X' is initial in State Machine Definition 'Y'."
    if clean.starts_with("Status '") && clean.contains("is initial in") {
        let status = extract_quoted(clean, "Status '");
        let sm = extract_quoted(clean, "State Machine Definition '");
        if let (Some(status), Some(sm)) = (status, sm) {
            if let Some(sm_def) = ir.state_machines.get_mut(&sm) {
                if !sm_def.statuses.contains(&status) {
                    sm_def.statuses.insert(0, status); // initial status first
                }
            }
        }
        return;
    }

    // "Transition 'X' is defined in State Machine Definition 'Y'."
    // "Transition 'X' is from Status 'Y'."
    // "Transition 'X' is to Status 'Y'."
    if clean.starts_with("Transition '") {
        let event = extract_quoted(clean, "Transition '");
        if let Some(event) = event {
            if clean.contains("is defined in State Machine Definition '") {
                let sm = extract_quoted(clean, "State Machine Definition '");
                if let Some(sm) = sm {
                    // Ensure transition exists — from/to filled by subsequent lines
                    let sm_def = ir.state_machines.entry(sm).or_insert(StateMachineDef {
                        noun_name: String::new(),
                        statuses: vec![],
                        transitions: vec![],
                    });
                    if !sm_def.transitions.iter().any(|t| t.event == event) {
                        sm_def.transitions.push(TransitionDef {
                            from: String::new(),
                            to: String::new(),
                            event,
                            guard: None,
                        });
                    }
                }
            } else if clean.contains("is from Status '") {
                let status = extract_quoted(clean, "Status '");
                if let Some(status) = status {
                    // Find and update the transition with matching event
                    for sm_def in ir.state_machines.values_mut() {
                        for t in &mut sm_def.transitions {
                            if t.event == event && t.from.is_empty() {
                                t.from = status.clone();
                                if !sm_def.statuses.contains(&status) {
                                    sm_def.statuses.push(status.clone());
                                }
                                break;
                            }
                        }
                    }
                }
            } else if clean.contains("is to Status '") {
                let status = extract_quoted(clean, "Status '");
                if let Some(status) = status {
                    for sm_def in ir.state_machines.values_mut() {
                        for t in &mut sm_def.transitions {
                            if t.event == event && t.to.is_empty() {
                                t.to = status.clone();
                                if !sm_def.statuses.contains(&status) {
                                    sm_def.statuses.push(status.clone());
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
        return;
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

struct NounMatch {
    name: String,
    start: usize,
    end: usize,
}

fn find_nouns_in_text(text: &str, noun_names: &[String]) -> Vec<NounMatch> {
    let mut sorted: Vec<&String> = noun_names.iter().collect();
    sorted.sort_by(|a, b| b.len().cmp(&a.len())); // longest first

    let mut found = Vec::new();
    let mut used_ranges: Vec<(usize, usize)> = Vec::new();

    for noun in sorted {
        // Find all occurrences
        let mut search_start = 0;
        while let Some(pos) = text[search_start..].find(noun.as_str()) {
            let start = search_start + pos;
            let end = start + noun.len();

            // Word boundary check
            let before_ok = start == 0 || !text.as_bytes()[start - 1].is_ascii_alphanumeric();
            let after_ok = end >= text.len() || !text.as_bytes()[end].is_ascii_alphanumeric();

            // Not overlapping with an already-found noun
            let overlaps = used_ranges.iter().any(|(s, e)| start < *e && end > *s);

            if before_ok && after_ok && !overlaps {
                found.push(NounMatch { name: noun.clone(), start, end });
                used_ranges.push((start, end));
            }

            search_start = end;
        }
    }

    found.sort_by_key(|m| m.start);
    found
}

fn infer_reading(found: &[NounMatch], text: &str) -> String {
    if found.len() >= 2 {
        let predicate = text[found[0].end..found[1].start].trim();
        // Strip quantifier phrases from the predicate to get the base fact type reading
        let clean_pred = strip_quantifiers(predicate);
        format!("{} {} {}", found[0].name, clean_pred, found[1].name)
    } else if found.len() == 1 {
        text.to_string()
    } else {
        text.to_string()
    }
}

/// Remove quantifier phrases from a predicate string.
/// "was born in exactly one" → "was born in"
/// "has at most one" → "has"
/// "submits at least 1 and at most 5" → "submits"
/// "was born in some" → "was born in"
fn strip_quantifiers(pred: &str) -> String {
    let quantifiers = [
        "exactly one",
        "at most one",
        "at least one",
        "more than one",
        "some",
        "no",
    ];
    let mut result = pred.to_string();
    for q in &quantifiers {
        result = result.replace(q, "");
    }
    // Also strip "at least N and at most M" patterns
    let re = regex::Regex::new(r"at least \d+\s*(and\s*)?at most \d+").unwrap();
    result = re.replace(&result, "").to_string();
    let re2 = regex::Regex::new(r"at least \d+").unwrap();
    result = re2.replace(&result, "").to_string();
    let re3 = regex::Regex::new(r"at most \d+").unwrap();
    result = re3.replace(&result, "").to_string();
    // Clean up whitespace
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn split_conjuncts(text: &str) -> Vec<String> {
    // Split on " and " but not inside "who ... and ..." clauses
    // Simple heuristic: split on " and " that's not preceded by "who"
    let parts: Vec<&str> = text.split(" and ").collect();
    if parts.len() <= 1 {
        return vec![text.to_string()];
    }

    // Rejoin parts that were split inside a "who" clause
    let mut result = Vec::new();
    let mut current = String::new();
    for part in parts {
        if current.is_empty() {
            current = part.to_string();
        } else if current.contains(" who ") || current.contains(" that ") {
            current = format!("{} and {}", current, part);
        } else {
            result.push(current);
            current = part.to_string();
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

fn extract_quoted(text: &str, prefix: &str) -> Option<String> {
    let idx = text.find(prefix)? + prefix.len();
    let rest = &text[idx..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Entity types ────────────────────────────────────────────

    #[test]
    fn parse_entity_type() {
        let input = "# Test\n\n## Entity Types\n\nCustomer(.Email) is an entity type.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.nouns.contains_key("Customer"));
        assert_eq!(ir.nouns["Customer"].object_type, "entity");
        assert_eq!(ir.nouns["Customer"].ref_scheme, Some(vec!["Email".to_string()]));
    }

    #[test]
    fn parse_entity_type_no_ref_scheme() {
        let input = "# Test\n\nConstraint is an entity type.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.nouns.contains_key("Constraint"));
        assert_eq!(ir.nouns["Constraint"].ref_scheme, None);
    }

    // ── Value types ─────────────────────────────────────────────

    #[test]
    fn parse_value_type() {
        let input = "Gender is a value type.\n  The possible values of Gender are 'M', 'F'.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.nouns.contains_key("Gender"));
        assert_eq!(ir.nouns["Gender"].object_type, "value");
        assert_eq!(ir.nouns["Gender"].enum_values, Some(vec!["M".to_string(), "F".to_string()]));
    }

    // ── Subtypes ────────────────────────────────────────────────

    #[test]
    fn parse_subtype() {
        let input = "Male is a subtype of Person.";
        let ir = parse_markdown(input).unwrap();
        assert_eq!(ir.nouns["Male"].super_type, Some("Person".to_string()));
    }

    // ── Fact types ──────────────────────────────────────────────

    #[test]
    fn parse_binary_fact_type() {
        let input = "# Test\n\n## Entity Types\n\nCustomer(.Email) is an entity type.\nCountry(.Code) is an entity type.\n\n## Fact Types\n\nCustomer was born in Country.";
        let ir = parse_markdown(input).unwrap();
        let ft = ir.fact_types.values().find(|ft| ft.reading.contains("born")).unwrap();
        assert_eq!(ft.roles.len(), 2);
        assert_eq!(ft.roles[0].noun_name, "Customer");
        assert_eq!(ft.roles[1].noun_name, "Country");
    }

    // ── Constraints ─────────────────────────────────────────────

    #[test]
    fn parse_uniqueness_constraint() {
        let input = "# T\n\n## Entity Types\n\nPerson(.Name) is an entity type.\nCountry(.Code) is an entity type.\n\n## Fact Types\n\nPerson was born in Country.\n\n## Constraints\n\nEach Person was born in at most one Country.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "UC"), "expected UC constraint, got: {:?}", ir.constraints.iter().map(|c| &c.kind).collect::<Vec<_>>());
    }

    #[test]
    fn parse_mandatory_constraint() {
        let input = "# T\n\n## Entity Types\n\nPerson(.Name) is an entity type.\nCountry(.Code) is an entity type.\n\n## Fact Types\n\nPerson was born in Country.\n\n## Constraints\n\nEach Person was born in some Country.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "MC"), "expected MC constraint");
    }

    #[test]
    fn parse_exactly_one_as_uc_plus_mc() {
        let input = "# T\n\n## Entity Types\n\nPerson(.Name) is an entity type.\nCountry(.Code) is an entity type.\n\n## Fact Types\n\nPerson was born in Country.\n\n## Constraints\n\nEach Person was born in exactly one Country.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "UC"), "expected UC from 'exactly one'");
        assert!(ir.constraints.iter().any(|c| c.kind == "MC"), "expected MC from 'exactly one'");
    }

    #[test]
    fn parse_frequency_constraint() {
        let input = "# T\n\n## Entity Types\n\nCustomer(.Id) is an entity type.\nRequest(.Id) is an entity type.\n\n## Fact Types\n\nCustomer submits Request.\n\n## Constraints\n\nEach Customer submits at least 1 and at most 5 Request.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "FC"), "expected FC constraint");
    }

    #[test]
    fn parse_deontic_forbidden() {
        let input = "# T\n\n## Entity Types\n\nPerson(.Name) is an entity type.\nCountry(.Code) is an entity type.\n\n## Fact Types\n\nPerson was born in Country.\n\n## Deontic Constraints\n\nIt is forbidden that the same Person was born in more than one Country.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.modality == "deontic"), "expected deontic constraint");
    }

    // ── Derivation rules ────────────────────────────────────────

    #[test]
    fn parse_iff_derivation_rule() {
        let input = "# T\n\n## Entity Types\n\nPerson(.Name) is an entity type.\n\n## Derivation Rules\n\nPerson is a Grandparent iff Person is a parent of some Person who is a parent of some Person.";
        let ir = parse_markdown(input).unwrap();
        assert_eq!(ir.derivation_rules.len(), 1);
        assert!(ir.derivation_rules[0].text.contains("iff"));
    }

    #[test]
    fn parse_attribute_style_derivation() {
        let input = "# T\n\n## Derivation Rules\n\nFor each Person: uncle = brother of parent.";
        let ir = parse_markdown(input).unwrap();
        assert_eq!(ir.derivation_rules.len(), 1);
    }

    #[test]
    fn parse_aggregate_count() {
        let input = "# T\n\n## Derivation Rules\n\nQuantity = count each Academic who has Rank and works for Dept.";
        let ir = parse_markdown(input).unwrap();
        assert_eq!(ir.derivation_rules.len(), 1);
    }

    // ── State machines ──────────────────────────────────────────

    #[test]
    fn parse_state_machine() {
        let input = "# T\n\n## Entity Types\n\nOrder(.Id) is an entity type.\n\n## Instance Facts\n\nState Machine Definition 'Order' is for Noun 'Order'.\nStatus 'Draft' is initial in State Machine Definition 'Order'.\nTransition 'place' is defined in State Machine Definition 'Order'.\n  Transition 'place' is from Status 'Draft'.\n  Transition 'place' is to Status 'Placed'.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.state_machines.contains_key("Order"), "expected Order SM");
        let sm = &ir.state_machines["Order"];
        assert_eq!(sm.noun_name, "Order");
        assert!(sm.transitions.iter().any(|t| t.event == "place" && t.from == "Draft" && t.to == "Placed"),
            "expected place transition, got: {:?}", sm.transitions);
    }

    // ── No headers required ───────────────────────────────────

    #[test]
    fn parse_without_any_headers() {
        // The parser recognizes patterns by syntax, not by section headers
        let input = r#"Order(.Order Number) is an entity type.
Customer(.Name) is an entity type.
Order was placed by Customer.
Each Order was placed by exactly one Customer.
State Machine Definition 'Order' is for Noun 'Order'.
Status 'Draft' is initial in State Machine Definition 'Order'.
Transition 'place' is defined in State Machine Definition 'Order'.
  Transition 'place' is from Status 'Draft'.
  Transition 'place' is to Status 'Placed'.
"#;
        let ir = parse_markdown(input).unwrap();
        assert_eq!(ir.nouns.len(), 2, "expected 2 nouns");
        assert!(!ir.fact_types.is_empty(), "expected fact types");
        assert!(ir.constraints.iter().any(|c| c.kind == "UC"), "expected UC");
        assert!(ir.constraints.iter().any(|c| c.kind == "MC"), "expected MC");
        assert!(ir.state_machines.contains_key("Order"), "expected SM");
    }

    // ── Domain name ─────────────────────────────────────────────

    #[test]
    fn parse_domain_from_h1() {
        let input = "# Orders Domain\n\nSome content.";
        let ir = parse_markdown(input).unwrap();
        assert_eq!(ir.domain, "Orders Domain");
    }

    // ── External UC with context ──────────────────────────────

    #[test]
    fn parse_external_uc_with_context() {
        let input = r#"Room(.Number) is an entity type.
Building(.Name) is an entity type.
Room Nr is a value type.
Room is in Building.
Room has Room Nr.
Context: Room is in Building; Room has Room Nr.
In this context, each Building, Room Nr combination is associated with at most one Room."#;
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "UC"),
            "expected UC from context constraint, constraints: {:?}",
            ir.constraints.iter().map(|c| (&c.kind, &c.text)).collect::<Vec<_>>());
    }

    // ── End-to-end: whitepaper order example ────────────────────

    #[test]
    fn whitepaper_order_example() {
        let markdown = r#"# Orders

## Entity Types

Order(.Order Number) is an entity type.
Customer(.Name) is an entity type.

## Fact Types

Order was placed by Customer.

## Constraints

Each Order was placed by exactly one Customer.

## Instance Facts

State Machine Definition 'Order' is for Noun 'Order'.
Status 'Draft' is initial in State Machine Definition 'Order'.
Transition 'place' is defined in State Machine Definition 'Order'.
  Transition 'place' is from Status 'Draft'.
  Transition 'place' is to Status 'Placed'.
Transition 'ship' is defined in State Machine Definition 'Order'.
  Transition 'ship' is from Status 'Placed'.
  Transition 'ship' is to Status 'Shipped'.
"#;
        let ir = parse_markdown(markdown).unwrap();
        assert_eq!(ir.nouns.len(), 2);
        assert!(ir.constraints.iter().any(|c| c.kind == "UC"), "expected UC");
        assert!(ir.constraints.iter().any(|c| c.kind == "MC"), "expected MC");
        assert!(ir.state_machines.contains_key("Order"));
        let sm = &ir.state_machines["Order"];
        assert_eq!(sm.transitions.len(), 2);
    }
}
