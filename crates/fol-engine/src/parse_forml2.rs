// crates/fol-engine/src/parse_forml2.rs
//
// FORML 2 Parser — FFP composition of recognizer functions.
//
// Per the paper: parse: R → Φ (Theorem 2).
// parse = α(recognize) : lines
// recognize = try₁ ; try₂ ; ... ; tryₙ
//
// Each recognizer: &str → Option<ParseAction>
// The ? operator IS the conditional form ⟨COND, is_some, unwrap, ⊥⟩.
// No if/else chains. Pattern matching via strip_suffix/strip_prefix/find.

use crate::types::*;
use std::collections::HashMap;

/// What a recognizer produces when it matches a line.
enum ParseAction {
    SetDomain(String),
    AddNoun(String, NounDef),
    MarkAbstract(String),
    AddPartition(String, Vec<String>),
    AddFactType(String, FactTypeDef),
    AddConstraint(ConstraintDef),
    AddDerivation(DerivationRuleDef),
    AddInstanceFact(String), // raw line for instance fact parsing
    Skip,
}

// =========================================================================
// Recognizers — pure functions: &str → Option<ParseAction>
// The ? operator replaces all if/else branching.
// =========================================================================

fn try_domain(line: &str) -> Option<ParseAction> {
    let rest = line.strip_prefix("# ")?;
    (!rest.starts_with('#')).then(|| ParseAction::SetDomain(rest.trim().into()))
}

fn try_header(line: &str) -> Option<ParseAction> {
    line.starts_with('#').then_some(ParseAction::Skip)
}

fn try_entity_type(line: &str) -> Option<ParseAction> {
    let before = line.strip_suffix(" is an entity type.")?;
    let (name, ref_scheme) = parse_entity_decl(before.trim())?;
    Some(ParseAction::AddNoun(name, NounDef {
        object_type: "entity".into(), enum_values: None, value_type: None,
        super_type: None, world_assumption: WorldAssumption::default(),
        ref_scheme, objectifies: None, subtype_kind: None, rigid: false,
        backed_by: None,
    }))
}

fn try_value_type(line: &str) -> Option<ParseAction> {
    let name = line.strip_suffix(" is a value type.")?.trim().to_string();
    Some(ParseAction::AddNoun(name, NounDef {
        object_type: "value".into(), enum_values: None, value_type: None,
        super_type: None, world_assumption: WorldAssumption::default(),
        ref_scheme: None, objectifies: None, subtype_kind: None, rigid: false,
        backed_by: None,
    }))
}

fn try_subtype(line: &str) -> Option<ParseAction> {
    let clean = line.strip_suffix('.')?;
    let idx = clean.find(" is a subtype of ")?;
    let sub = clean[..idx].trim().to_string();
    let sup = clean[idx + 17..].trim().to_string();
    Some(ParseAction::AddNoun(sub, NounDef {
        object_type: "entity".into(), enum_values: None, value_type: None,
        super_type: Some(sup), world_assumption: WorldAssumption::default(),
        ref_scheme: None, objectifies: None, subtype_kind: None, rigid: false,
        backed_by: None,
    }))
}

fn try_abstract(line: &str) -> Option<ParseAction> {
    let name = line.strip_suffix(" is abstract.")?.trim().to_string();
    Some(ParseAction::MarkAbstract(name))
}

fn try_partition(line: &str) -> Option<ParseAction> {
    let clean = line.strip_suffix('.')?;
    let idx = clean.find(" is partitioned into ")?;
    let sup = clean[..idx].trim().to_string();
    let subs = clean[idx + 21..].split(',').map(|s| s.trim().into()).collect();
    Some(ParseAction::AddPartition(sup, subs))
}

fn try_enum_values(line: &str) -> Option<ParseAction> {
    line.starts_with("The possible values of").then_some(ParseAction::Skip)
}

fn try_exclusive_subtypes(line: &str) -> Option<ParseAction> {
    (line.starts_with('{') && line.contains("subtypes of")).then_some(ParseAction::Skip)
}

fn try_association(line: &str) -> Option<ParseAction> {
    line.starts_with("This association with").then_some(ParseAction::Skip)
}

fn try_totality(line: &str) -> Option<ParseAction> {
    let rest = line.strip_prefix("Each ")?;
    let idx = rest.find(" is a ")?;
    rest.contains(" or ").then(|| ParseAction::MarkAbstract(rest[..idx].trim().into()))
}

fn try_deontic(line: &str) -> Option<ParseAction> {
    let (operator, rest) = line.strip_prefix("It is obligatory that ").map(|r| ("obligatory", r))
        .or_else(|| line.strip_prefix("It is forbidden that ").map(|r| ("forbidden", r)))
        .or_else(|| line.strip_prefix("It is permitted that ").map(|r| ("permitted", r)))?;
    Some(ParseAction::AddConstraint(ConstraintDef {
        id: String::new(), kind: operator.into(), modality: "deontic".into(),
        deontic_operator: Some(operator.into()),
        text: line.trim_end_matches('.').into(),
        spans: vec![], set_comparison_argument_length: None, clauses: None,
        entity: None, min_occurrence: None, max_occurrence: None,
    }))
}

fn try_instance_fact(line: &str) -> Option<ParseAction> {
    // An instance fact contains a quoted value: NounName 'value' predicate ...
    // Must contain at least one single-quoted value and not be a constraint or enum.
    let has_quote = line.contains('\'');
    let is_enum = line.contains("The possible values of");
    let is_constraint_prefix = line.starts_with("Each ") || line.starts_with("For each ")
        || line.starts_with("It is ") || line.starts_with("No ")
        || line.starts_with("If ") || line.starts_with("Context:");
    (has_quote && !is_enum && !is_constraint_prefix)
        .then(|| ParseAction::AddInstanceFact(line.into()))
}

fn try_derivation(line: &str) -> Option<ParseAction> {
    // " if " mid-sentence is a derivation rule (Consequent if Antecedent).
    // Lines starting with "If ... then ..." are conditional derivation rules.
    // Lines starting with "If " without " then " are constraints.
    let has_if = line.contains(" if ") && !line.starts_with("If ");
    let is_conditional = line.starts_with("If ") && line.contains(" then ");
    let has_marker = line.contains(" iff ")
        || has_if
        || is_conditional
        || line.contains(" is derived as ")
        || (line.starts_with("For each ") && line.contains(" = "))
        || line.contains("count each")
        || line.contains("sum(");
    has_marker.then(|| {
        let clean = line.trim_end_matches('.');
        ParseAction::AddDerivation(DerivationRuleDef {
            id: String::new(), text: clean.into(),
            antecedent_fact_type_ids: vec![], consequent_fact_type_id: String::new(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![], match_on: vec![], consequent_bindings: vec![],
        })
    })
}

fn try_constraint(line: &str, noun_names: &[String]) -> Option<ParseAction> {
    let starts_ok = line.starts_with("Each ") || line.starts_with("No ");
    starts_ok.then(|| ())?;
    let c = parse_constraint(line, noun_names)?;
    Some(ParseAction::AddConstraint(c))
}

fn try_fact_type(line: &str, noun_names: &[String]) -> Option<ParseAction> {
    // Instance facts have a quoted value immediately after the subject noun:
    // NounName 'value' predicate ...
    // Fact type readings may contain quotes later (object position) but not
    // right after the first noun. Check by finding the first noun and seeing
    // if a quote follows it.
    for noun in noun_names {
        let prefix = format!("{} '", noun);
        if line.starts_with(&prefix) { return None; }
    }
    let (ft_id, ft_def) = parse_fact(line, noun_names)?;
    Some(ParseAction::AddFactType(ft_id, ft_def))
}

// =========================================================================
// Main parser — fold recognizers over lines
// =========================================================================

/// Parse with pre-existing nouns from other domains.
/// Domains are NORMA tabs. Nouns are global across the UoD.
pub fn parse_markdown_with_nouns(input: &str, existing_nouns: &HashMap<String, NounDef>) -> Result<ConstraintIR, String> {
    let mut ir = ConstraintIR {
        domain: String::new(), nouns: existing_nouns.clone(), fact_types: HashMap::new(),
        constraints: vec![], state_machines: HashMap::new(), derivation_rules: vec![],
        general_instance_facts: vec![],
    };
    parse_into(&mut ir, input)?;
    Ok(ir)
}

pub fn parse_markdown(input: &str) -> Result<ConstraintIR, String> {
    let mut ir = ConstraintIR {
        domain: String::new(), nouns: HashMap::new(), fact_types: HashMap::new(),
        constraints: vec![], state_machines: HashMap::new(), derivation_rules: vec![],
        general_instance_facts: vec![],
    };
    parse_into(&mut ir, input)?;
    Ok(ir)
}

fn parse_into(ir: &mut ConstraintIR, input: &str) -> Result<(), String> {

    let lines: Vec<String> = input.lines().map(|s| s.to_string()).collect();

    // Pass 1: α(recognize_noun) : lines — extract nouns and domain
    for i in 0..lines.len() {
        let line = lines[i].trim();
        let action = None
            .or_else(|| try_domain(line))
            .or_else(|| try_header(line))
            .or_else(|| try_entity_type(line))
            .or_else(|| try_value_type(line))
            .or_else(|| try_subtype(line))
            .or_else(|| try_abstract(line))
            .or_else(|| try_partition(line))
            .or_else(|| try_exclusive_subtypes(line))
            .or_else(|| try_enum_values(line));

        apply_action(ir, action, &lines, i);

        // Look ahead for enum values after value type declaration
        if line.ends_with(" is a value type.") {
            let name = line.trim_end_matches(" is a value type.").trim();
            for j in (i + 1)..lines.len() {
                let next = lines[j].trim();
                if next.is_empty() { continue; }
                if next.starts_with("The possible values of") {
                    if let Some(noun) = ir.nouns.get_mut(name) {
                        noun.enum_values = parse_enum(next);
                    }
                }
                break;
            }
        }
    }

    // Pass 2: α(recognize_reading) : lines — fact types, constraints, derivations
    // Sorted longest-first for Theorem 1 (unambiguous longest-first matching)
    let mut noun_names: Vec<String> = ir.nouns.keys().cloned().collect();
    noun_names.sort_by(|a, b| b.len().cmp(&a.len()));

    for i in 0..lines.len() {
        let line = lines[i].trim();
        if line.is_empty() { continue; }

        // Skip lines already handled in pass 1
        let is_pass1 = try_entity_type(line).is_some()
            || try_value_type(line).is_some()
            || (try_subtype(line).is_some() && !line.starts_with("Each"))
            || try_abstract(line).is_some()
            || try_partition(line).is_some()
            || try_enum_values(line).is_some()
            || try_exclusive_subtypes(line).is_some()
            || try_association(line).is_some()
            || try_header(line).is_some();

        if is_pass1 { continue; }

        // Totality → mark abstract (but don't skip — still parse as constraint)
        if let Some(action) = try_totality(line) {
            apply_action(ir, Some(action), &lines, i);
        }

        let action = None
            .or_else(|| try_derivation(line))
            .or_else(|| try_deontic(line))
            .or_else(|| try_constraint(line, &noun_names))
            .or_else(|| try_fact_type(line, &noun_names))
            .or_else(|| try_instance_fact(line));

        // Split "exactly one" constraints into UC + MC
        match &action {
            Some(ParseAction::AddConstraint(c)) if line.contains("exactly one") => {
                let mut uc = c.clone(); uc.kind = "UC".into();
                let mut mc = c.clone(); mc.kind = "MC".into();
                ir.constraints.push(uc);
                ir.constraints.push(mc);
            }
            _ => { apply_action(ir, action, &lines, i); }
        }
    }

    Ok(())
}

/// Resolve a derivation rule's text into structured fact type references.
///
/// Splits on " if "/" iff " to get consequent and antecedent parts,
/// then matches each part's nouns against ir.fact_types by role noun names.
/// Anaphoric "that X" references are stripped to bare noun name "X".
fn resolve_derivation_rule(rule: &mut DerivationRuleDef, ir: &ConstraintIR) {
    // Longest-first noun list for Theorem 1 matching
    let mut noun_names: Vec<String> = ir.nouns.keys().cloned().collect();
    noun_names.sort_by(|a, b| b.len().cmp(&a.len()));

    // Split on " iff " or " if " to get (consequent, antecedent_text)
    let (consequent_text, antecedent_text) = rule.text
        .find(" iff ")
        .map(|i| (&rule.text[..i], &rule.text[i + 5..]))
        .or_else(|| rule.text.find(" if ")
            .map(|i| (&rule.text[..i], &rule.text[i + 4..])))
        .unwrap_or((&rule.text, ""));

    // Split antecedent on " and " to get individual conditions
    let antecedent_parts: Vec<&str> = antecedent_text
        .split(" and ")
        .map(|s| s.trim().trim_end_matches('.'))
        .filter(|s| !s.is_empty())
        .collect();

    // Strip "that " prefix from noun references in a text fragment,
    // returning the cleaned text.
    let strip_anaphora = |text: &str| -> String {
        text.replace("that ", "")
    };

    // Find the fact type whose roles and verb match a text fragment.
    // When multiple fact types share the same roles, the verb disambiguates.
    let resolve_fact_type = |fragment: &str| -> Option<String> {
        let cleaned = strip_anaphora(fragment);
        let found_nouns: Vec<(usize, usize, String)> = find_nouns(&cleaned, &noun_names);
        let noun_names_only: Vec<String> = found_nouns.iter()
            .map(|(_, _, name)| name.clone())
            .collect();

        // Extract the verb: text between the first and second noun
        let verb = found_nouns.windows(2)
            .next()
            .map(|pair| cleaned[pair[0].1..pair[1].0].trim().to_string())
            .unwrap_or_default();

        // First try: match roles AND verb against the fact type reading
        ir.fact_types.iter()
            .find(|(_, ft)| {
                let role_nouns: Vec<&String> = ft.roles.iter()
                    .map(|r| &r.noun_name)
                    .collect();
                let roles_match = noun_names_only.len() == role_nouns.len()
                    && noun_names_only.iter().all(|n| role_nouns.contains(&n));
                let verb_match = !verb.is_empty() && ft.reading.contains(&verb);
                roles_match && verb_match
            })
            .or_else(|| {
                // Fallback: match roles only (for cases where verb matching fails)
                ir.fact_types.iter()
                    .find(|(_, ft)| {
                        let role_nouns: Vec<&String> = ft.roles.iter()
                            .map(|r| &r.noun_name)
                            .collect();
                        noun_names_only.len() == role_nouns.len()
                            && noun_names_only.iter().all(|n| role_nouns.contains(&n))
                    })
            })
            .map(|(id, _)| id.clone())
    };

    // Detect "that X" anaphoric references — nouns preceded by "that " in
    // antecedent parts become join keys.
    let join_keys: Vec<String> = antecedent_parts.iter()
        .flat_map(|part| {
            noun_names.iter().filter_map(|noun| {
                let pattern = format!("that {}", noun);
                part.contains(&pattern).then(|| noun.clone())
            }).collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    // Resolve consequent
    rule.consequent_fact_type_id = resolve_fact_type(consequent_text).unwrap_or_default();

    // Resolve antecedents
    rule.antecedent_fact_type_ids = antecedent_parts.iter()
        .filter_map(|part| resolve_fact_type(part))
        .collect();

    // Deduplicate join keys
    let mut seen = std::collections::HashSet::new();
    rule.join_on = join_keys.into_iter()
        .filter(|k| seen.insert(k.clone()))
        .collect();

    // Set rule ID from consequent
    rule.id = rule.consequent_fact_type_id.clone();
}

/// Apply a parse action to the IR accumulator.
fn apply_action(ir: &mut ConstraintIR, action: Option<ParseAction>, lines: &[String], idx: usize) {
    let Some(action) = action else { return };
    match action {
        ParseAction::SetDomain(d) => { if ir.domain.is_empty() { ir.domain = d; } }
        ParseAction::AddNoun(name, def) => {
            let entry = ir.nouns.entry(name).or_insert_with(|| def.clone());
            // Merge: subtype/abstract/refscheme/objectifies declarations update existing nouns
            if def.super_type.is_some() { entry.super_type = def.super_type; }
            if def.objectifies.is_some() { entry.objectifies = def.objectifies; }
            if def.ref_scheme.is_some() && entry.ref_scheme.is_none() { entry.ref_scheme = def.ref_scheme; }
            if def.enum_values.is_some() && entry.enum_values.is_none() { entry.enum_values = def.enum_values; }
            if def.object_type == "abstract" { entry.object_type = "abstract".into(); }
        }
        ParseAction::MarkAbstract(name) => {
            if let Some(noun) = ir.nouns.get_mut(&name) { noun.object_type = "abstract".into(); }
        }
        ParseAction::AddPartition(sup, subs) => {
            if let Some(noun) = ir.nouns.get_mut(&sup) { noun.object_type = "abstract".into(); }
            for sub in subs {
                ir.nouns.entry(sub).or_insert(NounDef {
                    object_type: "entity".into(), enum_values: None, value_type: None,
                    super_type: Some(sup.clone()), world_assumption: WorldAssumption::default(),
                    ref_scheme: None, objectifies: None, subtype_kind: None, rigid: false,
                    backed_by: None,
                });
            }
        }
        ParseAction::AddFactType(id, def) => {
            // Check if this fact type connects a noun to External System.
            // Identified by roles, not by reading text (readings may be internationalized).
            if def.roles.len() == 2 {
                let role0_noun = &def.roles[0].noun_name;
                let role1_noun = &def.roles[1].noun_name;
                if role1_noun == "External System" {
                    if let Some(noun) = ir.nouns.get_mut(role0_noun) {
                        noun.backed_by = Some("External System".into());
                    }
                } else if role0_noun == "External System" {
                    if let Some(noun) = ir.nouns.get_mut(role1_noun) {
                        noun.backed_by = Some("External System".into());
                    }
                }
            }
            ir.fact_types.entry(id).or_insert(def);
        }
        ParseAction::AddConstraint(c) => { ir.constraints.push(c); }
        ParseAction::AddDerivation(mut r) => {
            resolve_derivation_rule(&mut r, ir);
            ir.derivation_rules.push(r);
        }
        ParseAction::AddInstanceFact(raw) => {
            let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
            parse_instance_fact(ir, &raw, &line_refs, idx);
        }
        ParseAction::Skip => {}
    }
}

// =========================================================================
// Pure extraction functions (no if/else — use ? and strip_prefix/suffix)
// =========================================================================

fn parse_entity_decl(text: &str) -> Option<(String, Option<Vec<String>>)> {
    let paren = text.find('(');
    match paren {
        Some(p) => {
            let name = text[..p].trim().to_string();
            let inner = text[p + 1..].trim_end_matches(')');
            let refs: Vec<String> = inner.split(',')
                .map(|s| s.trim().trim_start_matches('.').to_string())
                .filter(|s| !s.is_empty())
                .collect();
            Some((name, Some(refs).filter(|r| !r.is_empty())))
        }
        None => Some((text.trim().to_string(), None))
    }
}

fn parse_enum(line: &str) -> Option<Vec<String>> {
    let after = line.split(" are ").nth(1)?;
    Some(after.trim_end_matches('.').split(", ").map(|s| s.trim().trim_matches('\'').into()).collect())
}

/// Canonical Graph Schema ID from role nouns and verb.
/// The ID is the key in DEFS. Two readings with the same roles and verb
/// (just different voice) produce the same ID.
fn graph_schema_id(role_nouns: &[&str], verb: &str) -> String {
    let verb_part = verb.to_lowercase().replace(' ', "_");
    let mut parts = vec![role_nouns[0], &verb_part];
    parts.extend(&role_nouns[1..]);
    parts.join("_")
}

fn parse_fact(line: &str, noun_names: &[String]) -> Option<(String, FactTypeDef)> {
    let clean = line.trim_end_matches('.');
    let found = find_nouns(clean, noun_names);
    (found.len() >= 2).then(|| ())?;

    let predicate = clean[found[0].1..found[1].0].trim();
    (!predicate.is_empty()).then(|| ())?;

    let reading = format!("{} {} {}", found[0].2, predicate, found[1].2);
    let roles: Vec<RoleDef> = found.iter().enumerate()
        .map(|(i, (_, _, name))| RoleDef { noun_name: name.clone(), role_index: i })
        .collect();

    Some((reading.clone(), FactTypeDef { reading, roles }))
}

fn parse_constraint(line: &str, noun_names: &[String]) -> Option<ConstraintDef> {
    let clean = line.trim_end_matches('.');
    let stripped = clean.replace("Each ", "").replace("each ", "")
        .replace("at most one ", "").replace("exactly one ", "")
        .replace("at least one ", "").replace("some ", "");
    let found = find_nouns(&stripped, noun_names);
    (!found.is_empty()).then(|| ())?;

    let kind = ["exactly one", "at most one", "at least", "some ", "No "].iter()
        .find(|k| clean.contains(*k))
        .map(|k| match *k {
            "at most one" => "UC",
            "exactly one" => "MC",
            "some " | "at least" => "MC",
            "No " => "XC",
            _ => "UC",
        })
        .unwrap_or("UC");

    // Derive fact type ID from the nouns in the constraint.
    // The fact type reading = "Noun1 predicate Noun2" — extracted from the stripped text.
    let ft_id = if found.len() >= 2 {
        let predicate = stripped[found[0].1..found[1].0].trim();
        if !predicate.is_empty() {
            format!("{} {} {}", found[0].2, predicate, found[1].2)
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let spans: Vec<SpanDef> = found.iter().enumerate()
        .map(|(i, _)| SpanDef { fact_type_id: ft_id.clone(), role_index: i, subset_autofill: None })
        .collect();

    Some(ConstraintDef {
        id: String::new(), kind: kind.into(), modality: "alethic".into(),
        deontic_operator: None, text: clean.into(), spans,
        set_comparison_argument_length: None, clauses: None,
        entity: None, min_occurrence: None, max_occurrence: None,
    })
}

/// Find nouns in text — longest-first matching with word boundaries.
/// Returns (start, end, name) tuples sorted by position.
fn find_nouns(text: &str, noun_names: &[String]) -> Vec<(usize, usize, String)> {
    let mut sorted: Vec<&String> = noun_names.iter().collect();
    sorted.sort_by(|a, b| b.len().cmp(&a.len()));

    let mut matches = Vec::new();
    let mut used: Vec<(usize, usize)> = Vec::new();

    for name in &sorted {
        let mut pos = 0;
        while let Some(found) = text[pos..].find(name.as_str()) {
            let start = pos + found;
            let end = start + name.len();
            let before_ok = start == 0 || !text.as_bytes()[start - 1].is_ascii_alphanumeric();
            let after_ok = end >= text.len() || !text.as_bytes()[end].is_ascii_alphanumeric();
            let no_overlap = !used.iter().any(|&(s, e)| start < e && end > s);

            if before_ok && after_ok && no_overlap {
                matches.push((start, end, name.to_string()));
                used.push((start, end));
            }
            pos = start + 1;
            if pos >= text.len() { break; }
        }
    }

    matches.sort_by_key(|m| m.0);
    matches
}

// =========================================================================
// Instance fact parsing (state machines)
// =========================================================================

fn parse_instance_fact(ir: &mut ConstraintIR, line: &str, lines: &[&str], idx: usize) {
    let clean = line.trim_end_matches('.');

    // State Machine Definition
    if let Some(name) = extract_quoted(clean, "State Machine Definition '") {
        let sm = ir.state_machines.entry(name.clone()).or_insert(StateMachineDef {
            noun_name: String::new(), statuses: vec![], transitions: vec![],
        });
        if let Some(noun) = extract_quoted(clean, "is for Noun '") {
            sm.noun_name = noun;
        }
        return;
    }

    // Status (initial)
    if let (Some(status), Some(sm_name)) = (
        extract_quoted(clean, "Status '"),
        extract_quoted(clean, "is initial in State Machine Definition '"),
    ) {
        let sm = ir.state_machines.entry(sm_name).or_insert(StateMachineDef {
            noun_name: String::new(), statuses: vec![], transitions: vec![],
        });
        if !sm.statuses.contains(&status) { sm.statuses.insert(0, status); }
        return;
    }

    // Transition
    if let Some(event) = extract_quoted(clean, "Transition '") {
        let from = extract_quoted(clean, "is from Status '")
            .or_else(|| look_ahead(lines, idx, &event, "is from Status '"))
            .or_else(|| extract_quoted(clean, "is from Subscription Status '"))
            .or_else(|| extract_quoted(clean, "is from Incident Status '"));
        let to = extract_quoted(clean, "is to Status '")
            .or_else(|| look_ahead(lines, idx, &event, "is to Status '"))
            .or_else(|| extract_quoted(clean, "is to Subscription Status '"))
            .or_else(|| extract_quoted(clean, "is to Incident Status '"));

        if let (Some(from), Some(to)) = (from, to) {
            for sm in ir.state_machines.values_mut() {
                if sm.statuses.contains(&from) || sm.statuses.contains(&to) {
                    if !sm.statuses.contains(&from) { sm.statuses.push(from.clone()); }
                    if !sm.statuses.contains(&to) { sm.statuses.push(to.clone()); }
                    sm.transitions.push(TransitionDef { from, to, event, guard: None });
                    return;
                }
            }
            if let Some(sm) = ir.state_machines.values_mut().next() {
                if !sm.statuses.contains(&from) { sm.statuses.push(from.clone()); }
                if !sm.statuses.contains(&to) { sm.statuses.push(to.clone()); }
                sm.transitions.push(TransitionDef { from, to, event, guard: None });
            }
        }
        return;
    }

    // General instance fact: NounName 'value' predicate NounName 'value'.
    // Longest-first matching against declared nouns (Theorem 1).
    // The fact is x-bar (constant) asserted into P.
    parse_general_instance_fact(ir, clean);
}

fn parse_general_instance_fact(ir: &mut ConstraintIR, line: &str) {
    // Longest-first noun matching (Theorem 1, step 3)
    let mut noun_names: Vec<String> = ir.nouns.keys().cloned().collect();
    noun_names.sort_by(|a, b| b.len().cmp(&a.len()));

    // bu(match_subject, line) — find the first noun that matches as subject
    let subject = noun_names.iter()
        .filter_map(|noun| {
            let prefix = format!("{} '", noun);
            line.starts_with(&prefix).then(|| {
                let after = &line[prefix.len()..];
                after.find('\'').map(|end| (noun.clone(), after[..end].to_string(), after[end + 1..].trim()))
            })?
        })
        .next();

    let (subject_noun, subject_value, rest) = match subject {
        Some((n, v, r)) => (n, v, r),
        None => return,
    };

    // bu(match_object, rest) — find the object noun+value in the remainder
    let object_match = noun_names.iter()
        .filter_map(|noun| {
            let obj_prefix = format!("{} '", noun);
            rest.find(&obj_prefix).and_then(|pred_end| {
                let predicate = rest[..pred_end].trim();
                let obj_rest = &rest[pred_end + obj_prefix.len()..];
                obj_rest.find('\'').map(|end| (predicate.to_string(), noun.clone(), obj_rest[..end].to_string()))
            })
        })
        .next();

    let fact = match object_match {
        Some((predicate, object_noun, object_value)) => {
            // Field name: drop generic verbs ("has", "is"), keep descriptive predicates.
            // "has URI" → "uri". "reads from ClickHouse Table" → "readsFrom".
            let field = match predicate.trim() {
                "has" | "is" | "belongs to" | "is of" => to_camel_case(&object_noun),
                _ => to_camel_case(&predicate),
            };
            Some(GeneralInstanceFact {
                subject_noun,
                subject_value,
                field_name: field,
                object_noun,
                object_value,
            })
        }
        None => extract_value_fact(rest).map(|(predicate, value)| GeneralInstanceFact {
            subject_noun,
            subject_value,
            field_name: to_camel_case(&predicate),
            object_noun: String::new(),
            object_value: value,
        }),
    };

    if let Some(f) = fact { ir.general_instance_facts.push(f); }
}

fn extract_value_fact(rest: &str) -> Option<(String, String)> {
    let last_q_end = rest.rfind('\'')?;
    let before_last = &rest[..last_q_end];
    let val_start = before_last.rfind('\'')?;
    let value = before_last[val_start + 1..].to_string();
    let predicate = before_last[..val_start].trim().to_string();
    Some((predicate, value))
}

fn to_camel_case(s: &str) -> String {
    let words: Vec<&str> = s.split_whitespace()
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() { return String::new(); }
    let mut result = words[0].to_lowercase();
    for word in &words[1..] {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            result.push(first.to_uppercase().next().unwrap_or(first));
            result.extend(chars);
        }
    }
    result
}

fn extract_quoted(text: &str, prefix: &str) -> Option<String> {
    let start = text.find(prefix)? + prefix.len();
    let end = text[start..].find('\'')?;
    Some(text[start..start + end].into())
}

fn look_ahead(lines: &[&str], idx: usize, event: &str, pattern: &str) -> Option<String> {
    lines.iter().skip(idx + 1)
        .take_while(|l| l.trim().starts_with("Transition"))
        .find(|l| l.contains(&format!("Transition '{}'", event)) && l.contains(pattern))
        .and_then(|l| extract_quoted(l.trim().trim_end_matches('.'), pattern))
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_types() {
        let ir = parse_markdown("Customer(.Name) is an entity type.\nOrder(.OrderId) is an entity type.").unwrap();
        assert_eq!(ir.nouns.len(), 2);
        assert!(ir.nouns.contains_key("Customer"));
        assert!(ir.nouns.contains_key("Order"));
    }

    #[test]
    fn value_types_with_enum() {
        let ir = parse_markdown("Priority is a value type.\n  The possible values of Priority are 'low', 'medium', 'high'.").unwrap();
        assert_eq!(ir.nouns["Priority"].object_type, "value");
        assert_eq!(ir.nouns["Priority"].enum_values.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn subtypes() {
        let ir = parse_markdown("Request(.id) is an entity type.\nSupport Request is a subtype of Request.").unwrap();
        assert_eq!(ir.nouns["Support Request"].super_type.as_ref().unwrap(), "Request");
    }

    #[test]
    fn abstract_noun() {
        let ir = parse_markdown("Request(.id) is an entity type.\nRequest is abstract.").unwrap();
        assert_eq!(ir.nouns["Request"].object_type, "abstract");
    }

    #[test]
    fn partition_implies_abstract() {
        let ir = parse_markdown("Request(.id) is an entity type.\nRequest is partitioned into Support Request, Feature Request.").unwrap();
        assert_eq!(ir.nouns["Request"].object_type, "abstract");
        assert_eq!(ir.nouns["Support Request"].super_type.as_ref().unwrap(), "Request");
    }

    #[test]
    fn totality_implies_abstract() {
        let input = "Request(.id) is an entity type.\nSupport Request is a subtype of Request.\nEach Request is a Support Request or a Feature Request.";
        let ir = parse_markdown(input).unwrap();
        assert_eq!(ir.nouns["Request"].object_type, "abstract");
    }

    #[test]
    fn fact_types() {
        let input = "Customer(.Name) is an entity type.\nOrder(.OrderId) is an entity type.\nOrder was placed by Customer.";
        let ir = parse_markdown(input).unwrap();
        assert!(!ir.fact_types.is_empty());
    }

    #[test]
    fn exactly_one_splits_to_uc_mc() {
        let input = "Person(.Name) is an entity type.\nCountry(.Code) is an entity type.\nPerson was born in Country.\nEach Person was born in exactly one Country.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "UC"));
        assert!(ir.constraints.iter().any(|c| c.kind == "MC"));
    }

    #[test]
    fn deontic_constraints() {
        let input = "Response(.id) is an entity type.\nIt is obligatory that each Response is professional.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.modality == "deontic"));
    }

    #[test]
    fn domain_from_h1() {
        let ir = parse_markdown("# Support\n\nCustomer(.Name) is an entity type.").unwrap();
        assert_eq!(ir.domain, "Support");
    }

    #[test]
    fn derivation_rules() {
        let input = "X(.id) is an entity type.\nY(.id) is an entity type.\nX has Y iff some condition.";
        let ir = parse_markdown(input).unwrap();
        assert!(!ir.derivation_rules.is_empty());
    }

    #[test]
    fn instance_facts_value() {
        let input = "Domain(.Slug) is an entity type.\nVisibility is a value type.\n  The possible values of Visibility are 'public', 'private'.\n## Fact Types\nDomain has Visibility.\n## Instance Facts\nDomain 'support' has Visibility 'public'.";
        let ir = parse_markdown(input).unwrap();
        assert_eq!(ir.general_instance_facts.len(), 1);
        assert_eq!(ir.general_instance_facts[0].subject_noun, "Domain");
        assert_eq!(ir.general_instance_facts[0].subject_value, "support");
        assert_eq!(ir.general_instance_facts[0].object_value, "public");
    }

    #[test]
    fn instance_facts_noun_to_noun() {
        let input = "API Endpoint(.Path) is an entity type.\nClickHouse Table(.Name) is an entity type.\n## Fact Types\nAPI Endpoint reads from ClickHouse Table.\n## Instance Facts\nAPI Endpoint '/data/:vin' reads from ClickHouse Table 'sources.currentResources'.";
        let ir = parse_markdown(input).unwrap();
        assert_eq!(ir.general_instance_facts.len(), 1);
        assert_eq!(ir.general_instance_facts[0].subject_noun, "API Endpoint");
        assert_eq!(ir.general_instance_facts[0].subject_value, "/data/:vin");
        assert_eq!(ir.general_instance_facts[0].object_noun, "ClickHouse Table");
        assert_eq!(ir.general_instance_facts[0].object_value, "sources.currentResources");
    }

    #[test]
    fn instance_facts_multiple() {
        let input = "Domain(.Slug) is an entity type.\nVisibility is a value type.\nDomain has Visibility.\nDomain 'support' has Visibility 'public'.\nDomain 'core' has Visibility 'private'.";
        let ir = parse_markdown(input).unwrap();
        assert_eq!(ir.general_instance_facts.len(), 2);
    }

    #[test]
    fn backed_by_from_roles() {
        let input = "Vehicle Specs(.VIN) is an entity type.\nExternal System(.Name) is an entity type.\nVehicle Specs is backed by External System.";
        let ir = parse_markdown(input).unwrap();
        assert_eq!(ir.nouns["Vehicle Specs"].backed_by.as_deref(), Some("External System"));
    }

    #[test]
    fn backed_by_reverse_role_order() {
        let input = "External System(.Name) is an entity type.\nLog Entry(.id) is an entity type.\nExternal System backs Log Entry.";
        let ir = parse_markdown(input).unwrap();
        assert_eq!(ir.nouns["Log Entry"].backed_by.as_deref(), Some("External System"));
    }

    #[test]
    fn not_backed_by_without_external_system() {
        let input = "Customer(.Name) is an entity type.\nOrder(.Id) is an entity type.\nCustomer places Order.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.nouns["Customer"].backed_by.is_none());
        assert!(ir.nouns["Order"].backed_by.is_none());
    }

    #[test]
    fn derivation_rule_extracts_fact_types() {
        let input = "User(.Email) is an entity type.\nDomain(.Slug) is an entity type.\nOrg Role is a value type.\nOrganization(.Slug) is an entity type.\n## Fact Types\nUser has Org Role in Organization.\nDomain belongs to Organization.\nUser accesses Domain.\n## Derivation Rules\nUser accesses Domain if User has Org Role in Organization and Domain belongs to that Organization.";
        let ir = parse_markdown(input).unwrap();
        assert!(!ir.derivation_rules.is_empty());
        let rule = &ir.derivation_rules[0];
        assert!(!rule.consequent_fact_type_id.is_empty(), "consequent should be resolved");
        assert!(!rule.antecedent_fact_type_ids.is_empty(), "antecedents should be resolved");
        assert!(rule.antecedent_fact_type_ids.len() >= 2, "should have at least 2 antecedents");
    }

    #[test]
    fn derivation_rule_identifies_join_keys() {
        let input = "User(.Email) is an entity type.\nDomain(.Slug) is an entity type.\nOrg Role is a value type.\nOrganization(.Slug) is an entity type.\n## Fact Types\nUser has Org Role in Organization.\nDomain belongs to Organization.\nUser accesses Domain.\n## Derivation Rules\nUser accesses Domain if User has Org Role in Organization and Domain belongs to that Organization.";
        let ir = parse_markdown(input).unwrap();
        let rule = &ir.derivation_rules[0];
        assert!(rule.join_on.contains(&"Organization".to_string()), "Organization should be a join key (referenced with 'that')");
    }

    #[test]
    fn graph_schema_id_from_roles_and_verb() {
        // Binary fact type: "User owns Organization"
        assert_eq!(
            graph_schema_id(&["User", "Organization"], "owns"),
            "User_owns_Organization"
        );
        // Verb with spaces: "was placed by"
        assert_eq!(
            graph_schema_id(&["Order", "Customer"], "was placed by"),
            "Order_was_placed_by_Customer"
        );
        // Ring constraint with subscripts
        assert_eq!(
            graph_schema_id(&["Person1", "Person2"], "is parent of"),
            "Person1_is_parent_of_Person2"
        );
        // Unary: "Customer is active"
        assert_eq!(
            graph_schema_id(&["Customer"], "is active"),
            "Customer_is_active"
        );
    }
}
