// crates/arest/src/parse_forml2.rs
//
// FORML 2 Parser â€” FFP composition of recognizer functions.
//
// Per the paper: parse: R â†’ Î¦ (Theorem 2).
// parse = Î±(recognize) : lines
// recognize = tryâ‚ ; tryâ‚‚ ; ... ; tryâ‚™
//
// Each recognizer: &str â†’ Option<ParseAction>
// The ? operator IS the conditional form âŸ¨COND, is_some, unwrap, âŠ¥âŸ©.
// No if/else chains. Pattern matching via strip_suffix/strip_prefix/find.

use crate::types::*;
use std::collections::HashMap;

/// Metadata for a noun that is stored on Domain maps, not on NounDef.
#[derive(Default, Clone)]
struct NounMeta {
    super_type: Option<String>,
    ref_scheme: Option<Vec<String>>,
    objectifies: Option<String>,
}

/// What a recognizer produces when it matches a line.
enum ParseAction {
    SetDomain(String),
    AddNoun(String, NounDef, NounMeta),
    MarkAbstract(String),
    AddPartition(String, Vec<String>),
    AddFactType(String, FactTypeDef),
    AddConstraint(ConstraintDef),
    AddDerivation(DerivationRuleDef),
    AddInstanceFact(String), // raw line for instance fact parsing
    AddNamedSpan(String, Vec<String>), // span_name, role nouns
    AddAutofillSpan(String),           // span_name
    Skip,
}

// =========================================================================
// Recognizers â€” pure functions: &str â†’ Option<ParseAction>
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
        object_type: "entity".into(),
        world_assumption: WorldAssumption::default(),
    }, NounMeta { ref_scheme, ..Default::default() }))
}

fn try_value_type(line: &str) -> Option<ParseAction> {
    let name = line.strip_suffix(" is a value type.")?.trim().to_string();
    Some(ParseAction::AddNoun(name, NounDef {
        object_type: "value".into(),
        world_assumption: WorldAssumption::default(),
    }, NounMeta::default()))
}

fn try_subtype(line: &str) -> Option<ParseAction> {
    let clean = line.strip_suffix('.')?;
    let idx = clean.find(" is a subtype of ")?;
    let sub = clean[..idx].trim().to_string();
    let sup = clean[idx + 17..].trim().to_string();
    Some(ParseAction::AddNoun(sub, NounDef {
        object_type: "entity".into(),
        world_assumption: WorldAssumption::default(),
    }, NounMeta { super_type: Some(sup), ..Default::default() }))
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

fn try_ring(line: &str, noun_names: &[String]) -> Option<ParseAction> {
    let clean = line.trim_end_matches('.');

    // IR: "No A R itself." â€” simple irreflexive
    // AC: "No A may cycle back to itself via one or more traversals through R."
    if let Some(rest) = clean.strip_prefix("No ") {
        // AC pattern: "No A may cycle back to itself ..."
        if rest.contains(" may cycle back to itself") {
            // Extract the entity type (first word(s) matching a noun)
            let entity = noun_names.iter()
                .find(|n| rest.starts_with(n.as_str()))
                .cloned()
                .unwrap_or_default();
            return Some(ParseAction::AddConstraint(ConstraintDef {
                id: String::new(), kind: "AC".into(), modality: "alethic".into(),
                deontic_operator: None, text: clean.into(),
                spans: vec![], set_comparison_argument_length: None, clauses: None,
                entity: if entity.is_empty() { None } else { Some(entity) },
                min_occurrence: None, max_occurrence: None,
            }));
        }
        // IR pattern: "No A R itself" â€” must end with " itself" and have a known noun
        if rest.ends_with(" itself") {
            let has_noun = noun_names.iter().any(|n| {
                rest.starts_with(n.as_str())
            });
            if has_noun {
                return Some(ParseAction::AddConstraint(ConstraintDef {
                    id: String::new(), kind: "IR".into(), modality: "alethic".into(),
                    deontic_operator: None, text: clean.into(),
                    spans: vec![], set_comparison_argument_length: None, clauses: None,
                    entity: None, min_occurrence: None, max_occurrence: None,
                }));
            }
        }
        return None;
    }

    // Conditional ring patterns: "If A1 R A2 [and ...] then [it is impossible that] ..."
    if !clean.starts_with("If ") { return None; }
    let then_idx = clean.find(" then ")?;
    let antecedent = &clean[3..then_idx]; // everything after "If " up to " then "
    let consequent = &clean[then_idx + 6..]; // everything after " then "

    // All role tokens in the antecedent must share the same base noun type.
    // Extract words that match known nouns (with or without trailing digit subscripts).
    // We collect all noun-like tokens (noun + optional digit suffix) from the antecedent.
    let role_bases: Vec<&str> = antecedent
        .split_whitespace()
        .filter_map(|word| {
            // Strip trailing punctuation
            let w = word.trim_end_matches(',');
            // Check if base (digits stripped) matches a known noun
            let (base, _) = parse_role_token(w);
            noun_names.iter().any(|n| n == base).then_some(base)
        })
        .collect();

    // Need at least 2 role tokens in antecedent, all with the same base
    if role_bases.len() < 2 { return None; }
    let first_base = role_bases[0];
    if !role_bases.iter().all(|b| *b == first_base) { return None; }

    // Also check consequent contains the same base noun (subscripted or plain)
    let consequent_has_same_noun = {
        let effective = if consequent.starts_with("it is impossible that ") {
            &consequent["it is impossible that ".len()..]
        } else {
            consequent
        };
        effective.split_whitespace().any(|word| {
            let w = word.trim_end_matches(',');
            let (base, _) = parse_role_token(w);
            base == first_base
        })
    };
    if !consequent_has_same_noun { return None; }

    let has_and = antecedent.contains(" and ");
    let impossible = consequent.starts_with("it is impossible that ");
    let itself_in_consequent = consequent.contains(" itself");
    let is_not_in_antecedent = antecedent.contains(" is not ");

    let kind = match (has_and, impossible, itself_in_consequent, is_not_in_antecedent) {
        // AS: no and, impossible, no itself â€” "If A1 R A2 then it is impossible that A2 R A1"
        (false, true, false, _)  => "AS",
        // RF: no and, not impossible, itself in consequent â€” "If A1 R some A2 then A1 R itself"
        (false, false, true, _)  => "RF",
        // SY: no and, not impossible, no itself â€” "If A1 R A2 then A2 R A1"
        (false, false, false, _) => "SY",
        // AT: and, impossible, "is not" in antecedent â€” "If A1 R A2 and A1 is not A2 then impossible A2 R A1"
        (true, true, _, true)    => "AT",
        // IT: and, impossible, no "is not" â€” "If A1 R A2 and A2 R A3 then impossible A1 R A3"
        (true, true, _, false)   => "IT",
        // TR: and, not impossible â€” "If A1 R A2 and A2 R A3 then A1 R A3"
        (true, false, _, _)      => "TR",
        // Unrecognized combination â€” not a ring constraint
        _ => return None,
    };

    Some(ParseAction::AddConstraint(ConstraintDef {
        id: String::new(), kind: kind.into(), modality: "alethic".into(),
        deontic_operator: None, text: clean.into(),
        spans: vec![], set_comparison_argument_length: None, clauses: None,
        entity: None, min_occurrence: None, max_occurrence: None,
    }))
}

/// try_subset â€” SS: "If some A Râ‚ some B then that A Râ‚‚ that B."
/// Distinguishes from ring: subset has multiple different base noun types.
fn try_subset(line: &str, noun_names: &[String]) -> Option<ParseAction> {
    let clean = line.trim_end_matches('.');
    // Must start with "If " and contain " then "
    if !clean.starts_with("If ") { return None; }
    let then_idx = clean.find(" then ")?;
    let antecedent = &clean[3..then_idx];
    let consequent = &clean[then_idx + 6..];

    // Antecedent must contain "some" (existential), consequent must contain "that" (back-ref)
    if !antecedent.contains("some ") { return None; }
    if !consequent.contains("that ") { return None; }

    // Collect base noun types from antecedent using find_nouns (handles multi-word nouns)
    let stripped_ant = antecedent.replace("some ", "").replace("that ", "");
    let ant_found = find_nouns(&stripped_ant, noun_names);
    let ant_bases: Vec<&str> = ant_found.iter().map(|(_, _, n)| n.as_str()).collect();

    if ant_bases.len() < 2 { return None; }

    // Subset has multiple DIFFERENT base noun types (distinguishes from ring which has all same)
    let first = ant_bases[0];
    let all_same = ant_bases.iter().all(|b| b == &first);
    if all_same { return None; }

    // Build spans: [0] = subset (antecedent), [1] = superset (consequent)
    // SpanDef with empty fact_type_id â€” resolve_constraint_schema fills it in later
    let spans = vec![
        SpanDef { fact_type_id: String::new(), role_index: 0, subset_autofill: None },
        SpanDef { fact_type_id: String::new(), role_index: 0, subset_autofill: None },
    ];

    Some(ParseAction::AddConstraint(ConstraintDef {
        id: String::new(), kind: "SS".into(), modality: "alethic".into(),
        deontic_operator: None, text: clean.into(),
        spans, set_comparison_argument_length: None, clauses: None,
        entity: None, min_occurrence: None, max_occurrence: None,
    }))
}

/// try_equality â€” EQ: "...if and only if..." or "all or none of the following hold:..."
fn try_equality(line: &str) -> Option<ParseAction> {
    let clean = line.trim_end_matches('.');
    let matches = clean.contains(" if and only if ")
        || clean.to_lowercase().starts_with("all or none of the following hold");
    if !matches { return None; }
    Some(ParseAction::AddConstraint(ConstraintDef {
        id: String::new(), kind: "EQ".into(), modality: "alethic".into(),
        deontic_operator: None, text: clean.into(),
        spans: vec![], set_comparison_argument_length: None, clauses: None,
        entity: None, min_occurrence: None, max_occurrence: None,
    }))
}

/// try_set_comparison â€” XO, XC, OR
/// Patterns:
///   "For each A, exactly one of the following holds: ..." â†’ XO
///   "For each A, at most one of the following holds: ..."  â†’ XC
///   "For each A, at least one of the following holds: ..." â†’ OR (inclusive disjunction)
///   "Each A Râ‚ some Bâ‚ or Râ‚‚ some Bâ‚‚."                   â†’ OR (DMaC disjunctive MC)
fn try_set_comparison(line: &str, noun_names: &[String]) -> Option<ParseAction> {
    let clean = line.trim_end_matches('.');

    // "For each X, <quantifier> of the following holds: clause1; clause2..."
    if let Some(rest) = clean.strip_prefix("For each ") {
        let comma = rest.find(',')?;
        let entity = rest[..comma].trim().to_string();
        let body = rest[comma + 1..].trim();

        let (kind, after_quant) = if let Some(r) = body.strip_prefix("exactly one of the following holds:") {
            ("XO", r)
        } else if let Some(r) = body.strip_prefix("at most one of the following holds:") {
            ("XC", r)
        } else if let Some(r) = body.strip_prefix("at least one of the following holds:") {
            ("OR", r)
        } else {
            return None;
        };

        let clauses: Vec<String> = after_quant
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        return Some(ParseAction::AddConstraint(ConstraintDef {
            id: String::new(), kind: kind.into(), modality: "alethic".into(),
            deontic_operator: None, text: clean.into(),
            spans: vec![], set_comparison_argument_length: Some(clauses.len()),
            clauses: Some(clauses),
            entity: Some(entity), min_occurrence: None, max_occurrence: None,
        }));
    }

    // "Each A Râ‚ some Bâ‚ or Râ‚‚ some Bâ‚‚." â€” DMaC disjunctive MC â†’ OR
    if let Some(rest) = clean.strip_prefix("Each ") {
        // Must contain " or " and reference known nouns
        if !rest.contains(" or ") { return None; }
        // Find a known entity noun at the start
        let entity = noun_names.iter().find(|n| rest.starts_with(n.as_str()))?.clone();
        let after = rest[entity.len()..].trim();
        // Must have " or " in the remainder (not " or a/an " as in totality)
        if !after.contains(" or ") { return None; }
        // Exclude totality pattern: "Each X is a Y or a Z" (handled by try_totality)
        // A disjunctive MC has " or is" or " or has" â€” a verb after "or"
        let or_idx = after.find(" or ")?;
        let after_or = &after[or_idx + 4..];
        // Totality uses "a " / "an " after "or"; disjunctive MC uses a predicate verb
        let is_totality = after_or.starts_with("a ") || after_or.starts_with("an ");
        if is_totality { return None; }

        let clauses = vec![
            after[..or_idx].trim().to_string(),
            after_or.trim().to_string(),
        ];

        return Some(ParseAction::AddConstraint(ConstraintDef {
            id: String::new(), kind: "OR".into(), modality: "alethic".into(),
            deontic_operator: None, text: clean.into(),
            spans: vec![], set_comparison_argument_length: Some(clauses.len()),
            clauses: Some(clauses),
            entity: Some(entity), min_occurrence: None, max_occurrence: None,
        }));
    }

    None
}

/// try_frequency â€” FC: "Each A R at least {k} and at most {m} B."
/// MUST fire before try_constraint because "at least 1" (digit) is FC
/// while "at least one" (word) is MC. try_constraint would misclassify it.
fn try_frequency(line: &str, noun_names: &[String]) -> Option<ParseAction> {
    let clean = line.trim_end_matches('.');
    let rest = clean.strip_prefix("Each ")?;

    // Pattern: "at least {digit}" somewhere in line
    let at_least_idx = rest.find("at least ")?;
    let after_al = &rest[at_least_idx + 9..];

    // The digit must come immediately after "at least " (not a word like "one")
    let min_end = after_al.find(|c: char| !c.is_ascii_digit())?;
    if min_end == 0 { return None; } // no digit found
    let min_str = &after_al[..min_end];
    let min_val: usize = min_str.parse().ok()?;

    // Look for optional "and at most {digit}"
    let max_val: Option<usize> = after_al[min_end..].find("at most ")
        .and_then(|i| {
            let after_am = &after_al[min_end + i + 8..];
            let max_end = after_am.find(|c: char| !c.is_ascii_digit()).unwrap_or(after_am.len());
            if max_end == 0 { return None; }
            after_am[..max_end].parse().ok()
        });

    // Find the entity noun(s) to build spans
    let stripped = clean
        .replace("Each ", "")
        .replace(&format!("at least {}", min_str), "")
        .replace("and at most", "");
    // Remove max digit if present
    let stripped = if let Some(mv) = max_val {
        stripped.replace(&mv.to_string(), "")
    } else {
        stripped
    };
    let found = find_nouns(&stripped, noun_names);

    let spans: Vec<SpanDef> = found.iter().enumerate()
        .map(|(i, _)| SpanDef { fact_type_id: String::new(), role_index: i, subset_autofill: None })
        .collect();

    Some(ParseAction::AddConstraint(ConstraintDef {
        id: String::new(), kind: "FC".into(), modality: "alethic".into(),
        deontic_operator: None, text: clean.into(),
        spans, set_comparison_argument_length: None, clauses: None,
        entity: None, min_occurrence: Some(min_val), max_occurrence: max_val,
    }))
}

/// try_external_uc â€” UC (external uniqueness and context pattern)
/// Patterns:
///   "For each Bâ‚ and Bâ‚‚, at most one A Râ‚ that Bâ‚ and Râ‚‚ that Bâ‚‚."
///   "Context: Fâ‚; Fâ‚‚. In this context, each Bâ‚, Bâ‚‚ combination is associated with at most one A."
fn try_external_uc(line: &str, noun_names: &[String]) -> Option<ParseAction> {
    let clean = line.trim_end_matches('.');

    // Context pattern: starts with "Context:"
    if clean.starts_with("Context:") {
        if clean.contains("at most one") || clean.contains("combination is associated with") {
            // Find the entity noun mentioned in "at most one A"
            let entity = noun_names.iter().find(|n| {
                let pattern = format!("at most one {}", n);
                clean.contains(&pattern)
            }).cloned();
            return Some(ParseAction::AddConstraint(ConstraintDef {
                id: String::new(), kind: "UC".into(), modality: "alethic".into(),
                deontic_operator: None, text: clean.into(),
                spans: vec![], set_comparison_argument_length: None, clauses: None,
                entity, min_occurrence: None, max_occurrence: None,
            }));
        }
        return None;
    }

    // "For each Bâ‚ and Bâ‚‚, at most one A ..."
    if let Some(rest) = clean.strip_prefix("For each ") {
        // Must have "at most one" in the body
        if !clean.contains("at most one") { return None; }
        // Must have " and " in the "For each" list (external UC uses "Bâ‚ and Bâ‚‚")
        let comma_idx = rest.find(',')?;
        let quantified = &rest[..comma_idx];
        if !quantified.contains(" and ") { return None; }

        // Find the entity noun after "at most one"
        let after_amo = clean.find("at most one ")?;
        let noun_start = after_amo + 12;
        let entity = noun_names.iter().find(|n| {
            clean[noun_start..].starts_with(n.as_str())
        }).cloned();

        return Some(ParseAction::AddConstraint(ConstraintDef {
            id: String::new(), kind: "UC".into(), modality: "alethic".into(),
            deontic_operator: None, text: clean.into(),
            spans: vec![], set_comparison_argument_length: None, clauses: None,
            entity, min_occurrence: None, max_occurrence: None,
        }));
    }

    None
}

fn try_deontic(line: &str) -> Option<ParseAction> {
    let (operator, rest) = line.strip_prefix("It is obligatory that ").map(|r| ("obligatory", r))
        .or_else(|| line.strip_prefix("It is forbidden that ").map(|r| ("forbidden", r)))
        .or_else(|| line.strip_prefix("It is permitted that ").map(|r| ("permitted", r)))?;
    // Extract the entity noun: first capitalized phrase after the operator prefix.
    // e.g., "each Support Response uses Dash" â†’ entity = "Support Response"
    let entity_rest = rest.strip_prefix("each ").or(Some(rest)).unwrap();
    // The entity is the leading capitalized words before a lowercase verb
    let entity: String = entity_rest.split_whitespace()
        .take_while(|w| w.chars().next().map_or(false, |c| c.is_uppercase()))
        .collect::<Vec<&str>>()
        .join(" ");
    let entity_opt = if entity.is_empty() { None } else { Some(entity) };
    // Create a placeholder span so resolve_constraint_schema can populate it
    Some(ParseAction::AddConstraint(ConstraintDef {
        id: String::new(), kind: operator.into(), modality: "deontic".into(),
        deontic_operator: Some(operator.into()),
        text: line.trim_end_matches('.').into(),
        spans: vec![SpanDef { fact_type_id: String::new(), role_index: 0, subset_autofill: None }],
        set_comparison_argument_length: None, clauses: None,
        entity: entity_opt, min_occurrence: None, max_occurrence: None,
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

/// try_span_naming â€” "This span with A, B provides the preferred identification scheme for SpanName."
fn try_span_naming(line: &str) -> Option<ParseAction> {
    let rest = line.strip_prefix("This span with ")?;
    let pivot = rest.find(" provides the preferred identification scheme for ")?;
    let nouns_part = &rest[..pivot];
    let name_part = &rest[pivot + " provides the preferred identification scheme for ".len()..];
    let span_name = name_part.trim_end_matches('.').trim().to_string();
    let role_nouns: Vec<String> = nouns_part.split(',').map(|s| s.trim().to_string()).collect();
    (!span_name.is_empty() && !role_nouns.is_empty())
        .then(|| ParseAction::AddNamedSpan(span_name, role_nouns))
}

/// try_autofill_declaration â€” "Constraint Span 'SpanName' autofills from superset."
fn try_autofill_declaration(line: &str) -> Option<ParseAction> {
    let rest = line.strip_prefix("Constraint Span '")?;
    let end_quote = rest.find('\'')?;
    let span_name = rest[..end_quote].to_string();
    let after = rest[end_quote + 1..].trim();
    after.strip_prefix("autofills from superset")?;
    Some(ParseAction::AddAutofillSpan(span_name))
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
// Main parser â€” fold recognizers over lines
// =========================================================================

/// Parse with pre-existing nouns from other domains.
/// Domains are NORMA tabs. Nouns are global across the UoD.
pub fn parse_markdown_with_nouns(input: &str, existing_nouns: &HashMap<String, NounDef>) -> Result<Domain, String> {
    let mut ir = Domain {
        domain: String::new(), nouns: existing_nouns.clone(), fact_types: HashMap::new(),
        constraints: vec![], state_machines: HashMap::new(), derivation_rules: vec![],
        general_instance_facts: vec![],
        subtypes: HashMap::new(), enum_values: HashMap::new(),
        ref_schemes: HashMap::new(), objectifications: HashMap::new(),
        named_spans: HashMap::new(), autofill_spans: vec![],
    };
    parse_into(&mut ir, input)?;
    Ok(ir)
}

/// Parse FORML2 readings directly into a Population.
/// Every declaration becomes a fact in P. No intermediate struct.
pub fn parse_to_population(input: &str) -> Result<Population, String> {
    let domain = parse_markdown(input)?;
    Ok(domain_to_population(&domain))
}

/// Parse FORML2 readings into a Population with existing nouns for cross-domain resolution.
pub fn parse_to_population_with_nouns(input: &str, existing: &Population) -> Result<Population, String> {
    // Extract noun names from the existing population
    let existing_nouns: HashMap<String, NounDef> = existing.facts.get("Noun")
        .map(|facts| facts.iter().filter_map(|f| {
            let name = f.bindings.iter().find(|(k, _)| k == "name").map(|(_, v)| v.clone())?;
            let obj_type = f.bindings.iter().find(|(k, _)| k == "objectType").map(|(_, v)| v.clone()).unwrap_or("entity".into());
            Some((name, NounDef { object_type: obj_type, world_assumption: WorldAssumption::default() }))
        }).collect())
        .unwrap_or_default();
    let domain = parse_markdown_with_nouns(input, &existing_nouns)?;
    Ok(domain_to_population(&domain))
}

/// Convert a Domain (the legacy struct) to a Population of facts.
/// Each category of the struct becomes facts keyed by metamodel fact type.
pub fn domain_to_population(d: &Domain) -> Population {
    let mut facts: HashMap<String, Vec<FactInstance>> = HashMap::new();

    // Nouns -> Noun facts
    for (name, def) in &d.nouns {
        let mut bindings = vec![
            ("name".to_string(), name.clone()),
            ("objectType".to_string(), def.object_type.clone()),
        ];
        if let Some(st) = d.subtypes.get(name) {
            bindings.push(("superType".to_string(), st.clone()));
        }
        if let Some(rs) = d.ref_schemes.get(name) {
            bindings.push(("referenceScheme".to_string(), rs.join(",")));
        }
        if let Some(evs) = d.enum_values.get(name) {
            if !evs.is_empty() {
                bindings.push(("enumValues".to_string(), evs.join(",")));
            }
        }
        facts.entry("Noun".to_string()).or_default().push(FactInstance {
            fact_type_id: "Noun".to_string(),
            bindings,
        });
    }

    // Fact types -> GraphSchema + Role facts
    for (ft_id, ft) in &d.fact_types {
        facts.entry("GraphSchema".to_string()).or_default().push(FactInstance {
            fact_type_id: "GraphSchema".to_string(),
            bindings: vec![
                ("id".to_string(), ft_id.clone()),
                ("reading".to_string(), ft.reading.clone()),
                ("arity".to_string(), ft.roles.len().to_string()),
            ],
        });
        for role in &ft.roles {
            facts.entry("Role".to_string()).or_default().push(FactInstance {
                fact_type_id: "Role".to_string(),
                bindings: vec![
                    ("graphSchema".to_string(), ft_id.clone()),
                    ("nounName".to_string(), role.noun_name.clone()),
                    ("position".to_string(), role.role_index.to_string()),
                ],
            });
        }
    }

    // Constraints -> Constraint facts
    for c in &d.constraints {
        let mut bindings = vec![
            ("id".to_string(), c.id.clone()),
            ("kind".to_string(), c.kind.clone()),
            ("modality".to_string(), c.modality.clone()),
            ("text".to_string(), c.text.clone()),
        ];
        if let Some(ref op) = c.deontic_operator {
            bindings.push(("deonticOperator".to_string(), op.clone()));
        }
        if let Some(ref entity) = c.entity {
            bindings.push(("entity".to_string(), entity.clone()));
        }
        for (i, span) in c.spans.iter().enumerate() {
            bindings.push((format!("span{}_factTypeId", i), span.fact_type_id.clone()));
            bindings.push((format!("span{}_roleIndex", i), span.role_index.to_string()));
        }
        facts.entry("Constraint".to_string()).or_default().push(FactInstance {
            fact_type_id: "Constraint".to_string(),
            bindings,
        });
    }

    // Derivation rules
    for r in &d.derivation_rules {
        facts.entry("DerivationRule".to_string()).or_default().push(FactInstance {
            fact_type_id: "DerivationRule".to_string(),
            bindings: vec![
                ("id".to_string(), r.id.clone()),
                ("text".to_string(), r.text.clone()),
                ("consequentFactTypeId".to_string(), r.consequent_fact_type_id.clone()),
            ],
        });
    }

    // Instance facts
    for f in &d.general_instance_facts {
        facts.entry("InstanceFact".to_string()).or_default().push(FactInstance {
            fact_type_id: "InstanceFact".to_string(),
            bindings: vec![
                ("subjectNoun".to_string(), f.subject_noun.clone()),
                ("subjectValue".to_string(), f.subject_value.clone()),
                ("fieldName".to_string(), f.field_name.clone()),
                ("objectNoun".to_string(), f.object_noun.clone()),
                ("objectValue".to_string(), f.object_value.clone()),
            ],
        });
    }

    Population { facts }
}

pub fn parse_markdown(input: &str) -> Result<Domain, String> {
    let mut ir = Domain {
        domain: String::new(), nouns: HashMap::new(), fact_types: HashMap::new(),
        constraints: vec![], state_machines: HashMap::new(), derivation_rules: vec![],
        general_instance_facts: vec![],
        subtypes: HashMap::new(), enum_values: HashMap::new(),
        ref_schemes: HashMap::new(), objectifications: HashMap::new(),
        named_spans: HashMap::new(), autofill_spans: vec![],
    };
    parse_into(&mut ir, input)?;
    Ok(ir)
}

fn parse_into(ir: &mut Domain, input: &str) -> Result<(), String> {

    let lines: Vec<String> = input.lines().map(|s| s.to_string()).collect();

    // Pass 1: Î±(recognize_noun) : lines â€” extract nouns and domain
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
                    if let Some(vals) = parse_enum(next) {
                        ir.enum_values.insert(name.to_string(), vals);
                    }
                }
                break;
            }
        }
    }

    // Pass 2a: collect fact types and instance facts
    // Sorted longest-first for Theorem 1 (unambiguous longest-first matching)
    let mut noun_names: Vec<String> = ir.nouns.keys().cloned().collect();
    noun_names.sort_by(|a, b| b.len().cmp(&a.len()));

    for i in 0..lines.len() {
        let line = lines[i].trim();
        if line.is_empty() { continue; }

        // Skip pass 1 lines
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

        // Skip lines that belong to Pass 2b (constraints, derivations, deontic)
        // Preserves the original recognizer priority: these recognizers fire before try_fact_type
        let is_pass2b = try_ring(line, &noun_names).is_some()
            || try_subset(line, &noun_names).is_some()
            || try_equality(line).is_some()
            || try_set_comparison(line, &noun_names).is_some()
            || try_frequency(line, &noun_names).is_some()
            || try_external_uc(line, &noun_names).is_some()
            || try_span_naming(line).is_some()
            || try_autofill_declaration(line).is_some()
            || try_derivation(line).is_some()
            || try_deontic(line).is_some()
            || try_constraint(line, &noun_names).is_some()
            || try_totality(line).is_some();
        if is_pass2b { continue; }

        // Only collect fact types and instance facts in this sub-pass
        if let Some(action) = try_fact_type(line, &noun_names) {
            apply_action(ir, Some(action), &lines, i);
        } else if let Some(action) = try_instance_fact(line) {
            apply_action(ir, Some(action), &lines, i);
        }
    }

    // Build schema catalog from collected fact types
    let catalog = {
        let mut cat = SchemaCatalog::new();
        for (schema_id, ft) in &ir.fact_types {
            let role_nouns: Vec<&str> = ft.roles.iter().map(|r| r.noun_name.as_str()).collect();
            // Extract verb from reading: text between first and last noun
            let verb = ft.roles.first()
                .and_then(|r0| {
                    let after = ft.reading.find(&r0.noun_name)
                        .map(|i| &ft.reading[i + r0.noun_name.len()..]);
                    after.map(|a| {
                        ft.roles.last()
                            .and_then(|r1| a.find(&r1.noun_name).map(|j| a[..j].trim()))
                            .unwrap_or(a.trim())
                    })
                })
                .unwrap_or("");
            cat.register(schema_id, &role_nouns, verb, &ft.reading);
        }
        cat
    };

    // Pass 2b: constraints and derivations (with catalog)
    for i in 0..lines.len() {
        let line = lines[i].trim();
        if line.is_empty() { continue; }

        // Skip pass 1 lines
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

        // Totality â†’ mark abstract (but don't skip â€” still parse as constraint)
        if let Some(action) = try_totality(line) {
            apply_action(ir, Some(action), &lines, i);
        }

        // Try recognizers in priority order.
        // Ring and subset fire before derivation (both match "If...then...").
        // Frequency fires before constraint ("at least 1" digit vs "at least one" word).
        // External UC fires before constraint to handle "For each B1 and B2, at most one...".
        let action = None
            .or_else(|| try_ring(line, &noun_names))
            .or_else(|| try_subset(line, &noun_names))
            .or_else(|| try_equality(line))
            .or_else(|| try_set_comparison(line, &noun_names))
            .or_else(|| try_frequency(line, &noun_names))
            .or_else(|| try_external_uc(line, &noun_names))
            .or_else(|| try_span_naming(line))
            .or_else(|| try_autofill_declaration(line))
            .or_else(|| try_derivation(line))
            .or_else(|| try_deontic(line))
            .or_else(|| try_constraint(line, &noun_names));

        // If no constraint/derivation/deontic matched, this line was already
        // handled in Pass 2a (fact type or instance fact). Skip it.
        let Some(action) = action else { continue; };

        // Split inline "exactly one" constraints into UC + MC.
        // Skip the split for set-comparison kinds (XO, XC, OR) which carry their own semantics.
        // Derivation rules resolve through catalog, not through apply_action.
        let set_comparison_kinds = ["XO", "XC", "OR", "SS", "EQ", "FC"];
        match action {
            ParseAction::AddConstraint(ref c)
                if line.contains("exactly one")
                    && !set_comparison_kinds.contains(&c.kind.as_str()) =>
            {
                let c = match action { ParseAction::AddConstraint(c) => c, _ => unreachable!() };
                let mut uc = resolve_constraint_schema(c.clone(), &noun_names, &catalog, ir);
                uc.kind = "UC".into();
                if uc.id.is_empty() { uc.id = format!("UC:{}", uc.text); }
                let mut mc = resolve_constraint_schema(c, &noun_names, &catalog, ir);
                mc.kind = "MC".into();
                if mc.id.is_empty() { mc.id = format!("MC:{}", mc.text); }
                ir.constraints.push(uc);
                ir.constraints.push(mc);
            }
            ParseAction::AddConstraint(c) => {
                let mut resolved = resolve_constraint_schema(c, &noun_names, &catalog, ir);
                if resolved.id.is_empty() {
                    resolved.id = resolved.text.clone();
                }
                ir.constraints.push(resolved);
            }
            ParseAction::AddDerivation(mut r) => {
                resolve_derivation_rule(&mut r, ir, &catalog);
                ir.derivation_rules.push(r);
            }
            other => { apply_action(ir, Some(other), &lines, i); }
        }
    }

    // Task 6: Value Constraint (VC) â€” emit one VC per noun with enum_values.
    // The compiler reads enum values from ir.enum_values;
    // the ConstraintDef just marks which noun has a value constraint.
    let vc_nouns: Vec<String> = ir.enum_values.keys().cloned().collect();
    for noun_name in vc_nouns {
        ir.constraints.push(ConstraintDef {
            id: format!("VC:{}", noun_name),
            kind: "VC".into(),
            modality: "alethic".into(),
            deontic_operator: None,
            text: format!("{} has a value constraint", noun_name),
            spans: vec![],
            set_comparison_argument_length: None,
            clauses: None,
            entity: Some(noun_name),
            min_occurrence: None,
            max_occurrence: None,
        });
    }

    // Post-processing: resolve autofill spans.
    // For each autofill span name, find SS constraints whose role nouns
    // match the named span's role nouns, and set subset_autofill = Some(true).
    for span_name in &ir.autofill_spans.clone() {
        let role_nouns = match ir.named_spans.get(span_name) {
            Some(nouns) => nouns.clone(),
            None => continue,
        };
        let role_set: std::collections::HashSet<&str> = role_nouns.iter().map(|s| s.as_str()).collect();
        for cdef in &mut ir.constraints {
            if cdef.kind != "SS" { continue; }
            // Check if the SS constraint's text references the same role nouns
            let text_nouns: std::collections::HashSet<&str> = role_set.iter()
                .filter(|n| cdef.text.contains(**n))
                .copied()
                .collect();
            if text_nouns == role_set {
                // Set autofill on the first span (subset span)
                if let Some(span) = cdef.spans.first_mut() {
                    span.subset_autofill = Some(true);
                }
            }
        }
    }

    Ok(())
}

/// Resolve a derivation rule's text into structured fact type references.
///
/// Splits on " if "/" iff " to get consequent and antecedent parts,
/// then matches each part's nouns against ir.fact_types by role noun names.
/// Anaphoric "that X" references are stripped to bare noun name "X".
fn resolve_derivation_rule(rule: &mut DerivationRuleDef, ir: &Domain, catalog: &SchemaCatalog) {
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

    // Strip "that " prefix from noun references in a text fragment.
    let strip_anaphora = |text: &str| -> String {
        text.replace("that ", "")
    };

    // Resolve a text fragment to a Graph Schema ID via Ï-lookup through the catalog.
    let resolve_fact_type = |fragment: &str| -> Option<String> {
        let cleaned = strip_anaphora(fragment);
        let found_nouns: Vec<(usize, usize, String)> = find_nouns(&cleaned, &noun_names);
        let role_refs: Vec<&str> = found_nouns.iter().map(|(_, _, n)| n.as_str()).collect();

        // Extract the verb: text between the first and second noun
        let verb = found_nouns.windows(2)
            .next()
            .map(|pair| cleaned[pair[0].1..pair[1].0].trim())
            .unwrap_or("");

        // Ï-lookup: try with verb first, then noun set only
        let verb_opt = (!verb.is_empty()).then_some(verb);
        catalog.resolve(&role_refs, verb_opt)
            .or_else(|| catalog.resolve(&role_refs, None))
    };

    // Detect "that X" anaphoric references â€” nouns preceded by "that " in
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
fn apply_action(ir: &mut Domain, action: Option<ParseAction>, lines: &[String], idx: usize) {
    let Some(action) = action else { return };
    match action {
        ParseAction::SetDomain(d) => { if ir.domain.is_empty() { ir.domain = d; } }
        ParseAction::AddNoun(name, def, meta) => {
            let entry = ir.nouns.entry(name.clone()).or_insert_with(|| def.clone());
            // Merge: subtype/abstract declarations update existing nouns
            if def.object_type == "abstract" { entry.object_type = "abstract".into(); }
            // Populate IR maps from metadata
            if let Some(st) = meta.super_type {
                ir.subtypes.insert(name.clone(), st);
            }
            if let Some(rs) = meta.ref_scheme {
                ir.ref_schemes.entry(name.clone()).or_insert(rs);
            }
            if let Some(obj) = meta.objectifies {
                ir.objectifications.insert(name, obj);
            }
        }
        ParseAction::MarkAbstract(name) => {
            if let Some(noun) = ir.nouns.get_mut(&name) { noun.object_type = "abstract".into(); }
        }
        ParseAction::AddPartition(sup, subs) => {
            if let Some(noun) = ir.nouns.get_mut(&sup) { noun.object_type = "abstract".into(); }
            for sub in subs {
                ir.nouns.entry(sub.clone()).or_insert(NounDef {
                    object_type: "entity".into(),
                    world_assumption: WorldAssumption::default(),
                });
                ir.subtypes.insert(sub, sup.clone());
            }
        }
        ParseAction::AddFactType(id, def) => {
            ir.fact_types.entry(id).or_insert(def);
        }
        ParseAction::AddConstraint(c) => { ir.constraints.push(c); }
        ParseAction::AddDerivation(r) => {
            // Derivation rule resolution happens in Pass 2b with the catalog.
            // This path stores the unresolved rule for later resolution.
            ir.derivation_rules.push(r);
        }
        ParseAction::AddInstanceFact(raw) => {
            let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
            parse_instance_fact(ir, &raw, &line_refs, idx);
        }
        ParseAction::AddNamedSpan(name, nouns) => {
            ir.named_spans.insert(name, nouns);
        }
        ParseAction::AddAutofillSpan(name) => {
            ir.autofill_spans.push(name);
        }
        ParseAction::Skip => {}
    }
}

// =========================================================================
// Pure extraction functions (no if/else â€” use ? and strip_prefix/suffix)
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
    let noun_parts: Vec<String> = role_nouns.iter().map(|n| n.replace(' ', "_")).collect();
    let mut parts: Vec<&str> = vec![&noun_parts[0], &verb_part];
    noun_parts[1..].iter().for_each(|n| parts.push(n));
    parts.join("_")
}

/// Schema catalog for Ï-lookup: noun set â†’ Graph Schema ID.
/// The noun set is the key. The catalog is the DEFS cell.
struct SchemaCatalog {
    /// Sorted noun set â†’ vec of (schema_id, verb, reading) for disambiguation
    by_noun_set: HashMap<Vec<String>, Vec<(String, String, String)>>,
}

impl SchemaCatalog {
    fn new() -> Self {
        SchemaCatalog { by_noun_set: HashMap::new() }
    }

    fn register(&mut self, schema_id: &str, role_nouns: &[&str], verb: &str, reading: &str) {
        let mut key: Vec<String> = role_nouns.iter().map(|n| {
            let (base, _) = parse_role_token(n);
            base.to_lowercase()
        }).collect();
        key.sort();
        self.by_noun_set
            .entry(key)
            .or_default()
            .push((schema_id.to_string(), verb.to_lowercase(), reading.to_lowercase()));
    }

    /// Ï-lookup: noun set â†’ Graph Schema ID.
    /// Resolution strategy (no COND dispatch, just cascading lookup):
    /// 1. Exact verb match
    /// 2. Verb contained in stored reading (handles inverse voice)
    /// 3. Unique entry for noun set (no verb needed)
    fn resolve(&self, role_nouns: &[&str], verb: Option<&str>) -> Option<String> {
        let mut key: Vec<String> = role_nouns.iter().map(|n| {
            let (base, _) = parse_role_token(n);
            base.to_lowercase()
        }).collect();
        key.sort();
        let entries = self.by_noun_set.get(&key)?;
        let vb = verb.map(|v| v.to_lowercase());
        // Exact verb match
        entries.iter()
            .find(|(_, v, _)| vb.as_ref().map_or(false, |vb| v == vb))
            .or_else(||
                // Verb contained in stored reading (inverse voice: "is owned by" matches "owns")
                entries.iter()
                    .find(|(_, _, reading)| vb.as_ref().map_or(false, |vb| reading.contains(vb.as_str())))
            )
            .or_else(||
                // Unique entry for this noun set
                (entries.len() == 1).then(|| &entries[0])
            )
            .map(|(id, _, _)| id.clone())
    }
}

/// Resolve a constraint's span fact_type_ids through the schema catalog.
fn resolve_constraint_schema(
    mut constraint: ConstraintDef,
    noun_names: &[String],
    catalog: &SchemaCatalog,
    ir: &Domain,
) -> ConstraintDef {
    // Extract nouns from the constraint text to find the target schema.
    // Strip quantifiers and quoted values before noun matching.
    let mut stripped = constraint.text
        .replace("It is obligatory that ", "").replace("It is forbidden that ", "")
        .replace("It is permitted that ", "")
        .replace("Each ", "").replace("each ", "")
        .replace("at most one ", "").replace("exactly one ", "")
        .replace("at least one ", "").replace("some ", "")
        .replace("No ", "").replace("no ", "");
    // Remove quoted values like 'Overnight' that interfere with noun matching
    while let Some(start) = stripped.find('\'') {
        if let Some(end) = stripped[start + 1..].find('\'') {
            stripped = format!("{}{}", &stripped[..start], &stripped[start + 1 + end + 1..]);
        } else {
            break;
        }
    }
    let found = find_nouns(&stripped, noun_names);

    if found.len() >= 2 {
        let role_nouns: Vec<&str> = found.iter().map(|(_, _, n)| n.as_str()).collect();
        // Extract verb between first two nouns
        let verb_text = stripped[found[0].1..found[1].0].trim();
        let verb = if verb_text.is_empty() { None } else { Some(verb_text) };

        // Primary: Ï-lookup through catalog (exact verb, then reading containment, then unique)
        // Secondary: verb containment against ir.fact_types readings (handles inverse voice
        // when multiple schemas share the same noun pair)
        if let Some(schema_id) = catalog.resolve(&role_nouns, verb)
            .or_else(|| catalog.resolve(&role_nouns, None))
            .or_else(|| {
                // Inverse voice fallback: find schema where constraint verb appears in reading
                // or reading verb appears in constraint text
                let noun_set: std::collections::HashSet<&str> = role_nouns.iter().copied().collect();
                ir.fact_types.iter()
                    .filter(|(_, ft)| {
                        let ft_nouns: std::collections::HashSet<&str> = ft.roles.iter()
                            .map(|r| r.noun_name.as_str()).collect();
                        ft_nouns == noun_set
                    })
                    .find(|(_, ft)| {
                        verb.map_or(false, |v| {
                            let v_lower = v.to_lowercase();
                            let r_lower = ft.reading.to_lowercase();
                            // Check word stem overlap: words sharing a 3+ char prefix
                            // ("owned"/"owns" share "own", "administered"/"administers" share "administ")
                            let shared_prefix = |a: &str, b: &str| -> usize {
                                a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
                            };
                            v_lower.split_whitespace()
                                .any(|w| w.len() >= 3 && r_lower.split_whitespace()
                                    .any(|rw| rw.len() >= 3 && shared_prefix(w, rw) >= 3))
                        })
                    })
                    .map(|(id, _)| id.clone())
            })
        {
            // Update spans to reference the resolved schema ID.
            // The constrained role is determined by the quantifier position
            // in the verbalization pattern. "Each A R at most one B" constrains
            // A's role (the quantified noun). "It is forbidden that A R B"
            // constrains A's role (the first noun after the prefix).
            // Per Halpin TechReport ORM2-02: the constrained role is the one
            // under the quantifier.
            let resolved_ft = ir.fact_types.get(&schema_id);
            for span in &mut constraint.spans {
                span.fact_type_id = schema_id.clone();
                // Set role_index to the first noun's position in the fact type.
                // The first noun in the constraint text is the quantified noun
                // ("Each A", "the same A", "A" after deontic prefix).
                if found.len() >= 2 {
                    if let Some(ft) = resolved_ft {
                        let first_noun = &found[0].2;
                        if let Some(idx) = ft.roles.iter().position(|r| &r.noun_name == first_noun) {
                            span.role_index = idx;
                        }
                    }
                }
            }
        }
    }
    constraint
}

/// Parse a role token into (base_noun_name, full_token_with_subscript).
/// "Person1" -> ("Person", "Person1"). "User" -> ("User", "User").
fn parse_role_token(token: &str) -> (&str, &str) {
    let boundary = token
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_digit())
        .last()
        .map(|(i, _)| i)
        .unwrap_or(token.len());
    (&token[..boundary], token)
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

    // Build role tokens for schema ID (preserving subscript digits from the source text)
    let role_refs: Vec<&str> = found.iter().map(|(_, _, name)| name.as_str()).collect();
    let schema_id = graph_schema_id(&role_refs, predicate);

    let active_reading = ReadingDef {
        text: reading.clone(),
        role_order: (0..roles.len()).collect(),
    };

    Some((
        schema_id.clone(),
        FactTypeDef {
            schema_id,
            reading,
            readings: vec![active_reading],
            roles,
        },
    ))
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
    // The fact type reading = "Noun1 predicate Noun2" â€” extracted from the stripped text.
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

/// Find nouns in text â€” longest-first matching with word boundaries.
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

fn parse_instance_fact(ir: &mut Domain, line: &str, _lines: &[&str], _idx: usize) {
    let clean = line.trim_end_matches('.');
    parse_general_instance_fact(ir, clean);
}

fn parse_general_instance_fact(ir: &mut Domain, line: &str) {
    // Longest-first noun matching (Theorem 1, step 3)
    let mut noun_names: Vec<String> = ir.nouns.keys().cloned().collect();
    noun_names.sort_by(|a, b| b.len().cmp(&a.len()));

    // bu(match_subject, line) â€” find the first noun that matches as subject
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

    // bu(match_object, rest) â€” find the object noun+value in the remainder
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
            // Resolve field name from declared fact types.
            // The instance fact "A 'x' predicate B 'y'" should match the
            // declared fact type "A predicate B" and use its graph schema ID.
            let field = resolve_instance_field(&ir.fact_types, &subject_noun, &predicate, &object_noun);
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

/// Resolve the field name for an instance fact by looking up the declared fact type.
/// Matches the subject noun and object noun against fact type roles, using the
/// predicate for disambiguation when multiple fact types share the same noun pair.
fn resolve_instance_field(
    fact_types: &HashMap<String, FactTypeDef>,
    subject_noun: &str,
    predicate: &str,
    object_noun: &str,
) -> String {
    // Find fact types where role 0 matches subject and role 1 matches object
    let candidates: Vec<(&str, &FactTypeDef)> = fact_types.iter()
        .filter(|(_, ft)| {
            ft.roles.len() >= 2
                && ft.roles[0].noun_name == subject_noun
                && ft.roles[1].noun_name == object_noun
        })
        .map(|(id, ft)| (id.as_str(), ft))
        .collect();

    // Resolve: find the declared fact type and extract the predicate from it.
    // The predicate in the reading is the canonical field name source.
    let pred_lower = predicate.to_lowercase();
    let matched = candidates.iter()
        .find(|(_, ft)| {
            let r = ft.reading.to_lowercase();
            // Check if the reading contains the predicate words
            pred_lower.split_whitespace().all(|w| r.contains(w))
        })
        .or_else(|| candidates.first());

    if let Some((id, _)) = matched {
        // The field name is the graph schema ID. This is the fact type identity.
        id.to_string()
    } else {
        // No declared fact type. Also try reverse role order.
        let reverse = fact_types.iter()
            .find(|(_, ft)| {
                ft.roles.len() >= 2
                    && ft.roles[1].noun_name == subject_noun
                    && ft.roles[0].noun_name == object_noun
            });
        if let Some((_, ft)) = reverse {
            let reading = &ft.reading;
            let after_obj = reading.find(object_noun)
                .map(|i| &reading[i + object_noun.len()..])
                .unwrap_or(reading);
            let before_subj = after_obj.find(subject_noun)
                .map(|i| &after_obj[..i])
                .unwrap_or(after_obj);
            to_camel_case(before_subj.trim())
        } else {
            to_camel_case(predicate)
        }
    }
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
        assert_eq!(ir.enum_values["Priority"].len(), 3);
    }

    #[test]
    fn subtypes() {
        let ir = parse_markdown("Request(.id) is an entity type.\nSupport Request is a subtype of Request.").unwrap();
        assert_eq!(ir.subtypes["Support Request"], "Request");
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
        assert_eq!(ir.subtypes["Support Request"], "Request");
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
    fn instance_fact_noun_uri() {
        let input = "Noun is an entity type.\nURI is a value type.\n## Fact Types\nNoun has URI.\n## Instance Facts\nNoun 'API Product' has URI '/api'.";
        let ir = parse_markdown(input).unwrap();
        assert_eq!(ir.general_instance_facts.len(), 1);
        let f = &ir.general_instance_facts[0];
        eprintln!("subject_noun={} subject_value={} field_name={} object_noun={} object_value={}",
            f.subject_noun, f.subject_value, f.field_name, f.object_noun, f.object_value);
        assert_eq!(f.subject_noun, "Noun");
        assert_eq!(f.subject_value, "API Product");
        assert_eq!(f.object_value, "/api");
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
        // Multi-word nouns: "Auth Session uses Session Strategy"
        assert_eq!(
            graph_schema_id(&["Auth Session", "Session Strategy"], "uses"),
            "Auth_Session_uses_Session_Strategy"
        );
    }

    #[test]
    fn strip_role_subscript() {
        assert_eq!(parse_role_token("Person1"), ("Person", "Person1"));
        assert_eq!(parse_role_token("Person2"), ("Person", "Person2"));
        assert_eq!(parse_role_token("User"), ("User", "User"));
        assert_eq!(parse_role_token("Organization"), ("Organization", "Organization"));
        // Multi-word noun with subscript
        assert_eq!(parse_role_token("Support Request1"), ("Support Request", "Support Request1"));
    }

    #[test]
    fn parse_fact_produces_schema_id() {
        let nouns = vec!["User".to_string(), "Organization".to_string()];
        let (id, ft) = parse_fact("User owns Organization.", &nouns).unwrap();
        assert_eq!(id, "User_owns_Organization");
        assert_eq!(ft.schema_id, "User_owns_Organization");
        assert_eq!(ft.reading, "User owns Organization");
        assert_eq!(ft.readings.len(), 1);
        assert_eq!(ft.readings[0].role_order, vec![0, 1]);
    }

    #[test]
    fn schema_catalog_resolves_by_noun_set() {
        let mut catalog = SchemaCatalog::new();
        catalog.register("User_owns_Organization", &["User", "Organization"], "owns", "User owns Organization");
        catalog.register("User_administers_Organization", &["User", "Organization"], "administers", "User administers Organization");
        catalog.register("Domain_belongs_to_App", &["Domain", "App"], "belongs to", "Domain belongs to App");

        // Single match by noun set
        assert_eq!(
            catalog.resolve(&["Domain", "App"], None),
            Some("Domain_belongs_to_App".to_string())
        );
        // Ambiguous noun set, disambiguate by verb
        assert_eq!(
            catalog.resolve(&["User", "Organization"], Some("owns")),
            Some("User_owns_Organization".to_string())
        );
        assert_eq!(
            catalog.resolve(&["User", "Organization"], Some("administers")),
            Some("User_administers_Organization".to_string())
        );
        // Inverse voice: catalog alone can't resolve inverse voice for ambiguous noun sets.
        // The full resolution uses word-stem matching against ir.fact_types (resolve_constraint_schema).
        // See inverse_voice_ambiguous_noun_set_resolves test for end-to-end coverage.
        // Reverse order with unique noun set still resolves
        assert_eq!(
            catalog.resolve(&["App", "Domain"], None),
            Some("Domain_belongs_to_App".to_string())
        );
        // No match
        assert_eq!(
            catalog.resolve(&["Foo", "Bar"], None),
            None
        );
    }

    #[test]
    fn inverse_reading_constraint_resolves_to_schema() {
        let input = "# test\n\
            ## Entity Types\n\
            User(.Name) is an entity type.\n\
            Organization(.Name) is an entity type.\n\
            ## Fact Types\n\
            User owns Organization.\n\
            ## Constraints\n\
            Each Organization is owned by at most one User.\n";
        let ir = parse_markdown(input).unwrap();
        // Fact type keyed by Graph Schema ID
        assert!(ir.fact_types.contains_key("User_owns_Organization"));
        assert!(!ir.fact_types.contains_key("User owns Organization"));
        // The constraint's spans should reference the same schema ID
        assert!(!ir.constraints.is_empty());
        let c = &ir.constraints[0];
        assert_eq!(c.spans[0].fact_type_id, "User_owns_Organization",
            "Constraint span should reference Graph Schema ID, not reading text");
    }

    #[test]
    fn inverse_voice_ambiguous_noun_set_resolves() {
        let input = "# test\n\
            ## Entity Types\n\
            User(.Name) is an entity type.\n\
            Organization(.Name) is an entity type.\n\
            ## Fact Types\n\
            User owns Organization.\n\
            User administers Organization.\n\
            ## Constraints\n\
            Each Organization is owned by at most one User.\n\
            Each Organization is administered by at most one User.\n";
        let ir = parse_markdown(input).unwrap();
        // Both fact types present
        assert!(ir.fact_types.contains_key("User_owns_Organization"));
        assert!(ir.fact_types.contains_key("User_administers_Organization"));
        // "is owned by" constraint should resolve to "User_owns_Organization"
        // via word overlap: "owned" matches "owns"
        let owned_constraint = ir.constraints.iter()
            .find(|c| c.text.contains("is owned by"))
            .expect("should have 'is owned by' constraint");
        assert_eq!(owned_constraint.spans[0].fact_type_id, "User_owns_Organization",
            "Inverse voice 'is owned by' should resolve to 'owns' schema");
        // "is administered by" should resolve to "User_administers_Organization"
        let admin_constraint = ir.constraints.iter()
            .find(|c| c.text.contains("is administered by"))
            .expect("should have 'is administered by' constraint");
        assert_eq!(admin_constraint.spans[0].fact_type_id, "User_administers_Organization",
            "Inverse voice 'is administered by' should resolve to 'administers' schema");
    }

    #[test]
    fn ring_irreflexive() {
        let input = "Person(.Name) is an entity type.\nPerson is a parent of Person.\nNo Person is a parent of itself.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "IR"), "Expected IR constraint, got: {:?}", ir.constraints);
    }

    #[test]
    fn ring_asymmetric() {
        let input = "Person(.Name) is an entity type.\nPerson is a parent of Person.\nIf Person1 is a parent of Person2 then it is impossible that Person2 is a parent of Person1.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "AS"), "Expected AS constraint, got: {:?}", ir.constraints);
    }

    #[test]
    fn ring_symmetric() {
        let input = "Person(.Name) is an entity type.\nPerson is married to Person.\nIf Person1 is married to Person2 then Person2 is married to Person1.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "SY"), "Expected SY constraint, got: {:?}", ir.constraints);
    }

    #[test]
    fn ring_intransitive() {
        let input = "Person(.Name) is an entity type.\nPerson is a parent of Person.\nIf Person1 is a parent of Person2 and Person2 is a parent of Person3 then it is impossible that Person1 is a parent of Person3.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "IT"), "Expected IT constraint, got: {:?}", ir.constraints);
    }

    #[test]
    fn ring_transitive() {
        let input = "Person(.Name) is an entity type.\nPerson is an ancestor of Person.\nIf Person1 is an ancestor of Person2 and Person2 is an ancestor of Person3 then Person1 is an ancestor of Person3.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "TR"), "Expected TR constraint, got: {:?}", ir.constraints);
    }

    #[test]
    fn ring_acyclic() {
        let input = "Category(.Name) is an entity type.\nCategory contains Category.\nNo Category may cycle back to itself via one or more traversals through contains.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "AC"), "Expected AC constraint, got: {:?}", ir.constraints);
    }

    #[test]
    fn subset_constraint() {
        let input = "Person(.Name) is an entity type.\nBook(.Title) is an entity type.\nPerson authored Book.\nPerson reviewed Book.\nIf some Person authored some Book then that Person reviewed that Book.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "SS"), "Expected SS constraint, got: {:?}", ir.constraints);
    }

    #[test]
    fn equality_constraint() {
        let input = "Person(.Name) is an entity type.\nBook(.Title) is an entity type.\nPerson authored Book.\nPerson reviewed Book.\nFor each Person, that Person authored some Book if and only if that Person reviewed some Book.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "EQ"), "Expected EQ constraint, got: {:?}", ir.constraints);
    }

    #[test]
    fn exclusion_general() {
        let input = "Person(.Name) is an entity type.\nPerson is tenured.\nPerson is contracted.\nFor each Person, at most one of the following holds: that Person is tenured; that Person is contracted.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "XC"), "Expected XC, got: {:?}", ir.constraints);
    }

    #[test]
    fn exclusive_or() {
        let input = "Person(.Name) is an entity type.\nPerson is tenured.\nPerson is contracted.\nFor each Person, exactly one of the following holds: that Person is tenured; that Person is contracted.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "XO"), "Expected XO, got: {:?}", ir.constraints);
    }

    #[test]
    fn inclusive_or() {
        let input = "Lecturer(.Name) is an entity type.\nDate(.Value) is a value type.\nLecturer is contracted until Date.\nLecturer is tenured.\nEach Lecturer is contracted until some Date or is tenured.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "OR"), "Expected OR, got: {:?}", ir.constraints);
    }

    #[test]
    fn frequency_constraint() {
        let input = "Customer(.Name) is an entity type.\nOrder(.Id) is an entity type.\nCustomer places Order.\nEach Customer places at least 1 and at most 5 Order.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "FC"), "Expected FC constraint, got: {:?}", ir.constraints);
        let fc = ir.constraints.iter().find(|c| c.kind == "FC").unwrap();
        assert_eq!(fc.min_occurrence, Some(1));
        assert_eq!(fc.max_occurrence, Some(5));
    }

    #[test]
    fn value_constraint() {
        let input = "Priority is a value type.\n  The possible values of Priority are 'Low', 'Medium', 'High'.\nTicket(.Id) is an entity type.\nTicket has Priority.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "VC"), "Expected VC constraint, got: {:?}", ir.constraints);
    }

    #[test]
    fn external_uniqueness() {
        let input = "Room(.Nr) is an entity type.\nBuilding(.Code) is an entity type.\nRoomNr is a value type.\nRoom is in Building.\nRoom has RoomNr.\nFor each Building and RoomNr, at most one Room is in that Building and has that RoomNr.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "UC"), "Expected UC (external), got: {:?}", ir.constraints);
    }

    #[test]
    fn context_pattern() {
        let input = "Room(.Nr) is an entity type.\nBuilding(.Code) is an entity type.\nRoomNr is a value type.\nRoom is in Building.\nRoom has RoomNr.\nContext: Room is in Building; Room has RoomNr. In this context, each Building, RoomNr combination is associated with at most one Room.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.constraints.iter().any(|c| c.kind == "UC"), "Expected UC (context), got: {:?}", ir.constraints);
    }

    #[test]
    fn real_domain_produces_schema_ids() {
        let input = "\
# Auth

## Entity Types
Auth Session(.id) is an entity type.
Customer(.Name) is an entity type.
Session Strategy is a value type.

## Fact Types
Auth Session is for Customer.
  Each Auth Session is for exactly one Customer.
Auth Session uses Session Strategy.
  Each Auth Session uses exactly one Session Strategy.
";
        let ir = parse_markdown(input).unwrap();
        let keys: Vec<&String> = ir.fact_types.keys().collect();
        // All keys should be underscore format, not reading format
        for key in &keys {
            assert!(!key.contains(' '), "Fact type key '{}' should not contain spaces", key);
        }
        assert!(ir.fact_types.contains_key("Auth_Session_is_for_Customer"));
        assert!(ir.fact_types.contains_key("Auth_Session_uses_Session_Strategy"));
        // Constraints should reference schema IDs
        for c in &ir.constraints {
            for span in &c.spans {
                assert!(!span.fact_type_id.contains(' '),
                    "Constraint span '{}' should not contain spaces", span.fact_type_id);
            }
        }
    }

    #[test]
    fn span_naming() {
        let input = "Customer(.Email) is an entity type.\nSupport Request(.Id) is an entity type.\nCustomer submits Support Request.\nIf some Support Request has some Email Address and some Customer is identified by that Email Address then that Customer submits that Support Request.\nThis span with Customer, Support Request provides the preferred identification scheme for Customer Submission Match.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.named_spans.contains_key("Customer Submission Match"));
        assert_eq!(ir.named_spans["Customer Submission Match"], vec!["Customer".to_string(), "Support Request".to_string()]);
    }

    #[test]
    fn autofill_declaration() {
        let input = "Customer(.Email) is an entity type.\nSupport Request(.Id) is an entity type.\nEmail Address is a value type.\nCustomer submits Support Request.\nCustomer is identified by Email Address.\nSupport Request has Email Address.\nIf some Support Request has some Email Address and some Customer is identified by that Email Address then that Customer submits that Support Request.\nThis span with Customer, Support Request provides the preferred identification scheme for Customer Submission Match.\nConstraint Span 'Customer Submission Match' autofills from superset.";
        let ir = parse_markdown(input).unwrap();
        // The autofill span should be recorded
        assert!(ir.autofill_spans.contains(&"Customer Submission Match".to_string()));
        // The SS constraint targeting Customer, Support Request should have autofill enabled
        let ss = ir.constraints.iter().find(|c| c.kind == "SS").expect("Should have SS constraint");
        assert_eq!(ss.spans.iter().any(|s| s.subset_autofill == Some(true)), true, "SS constraint should have autofill enabled");
    }

    #[test]
    fn subset_autofill_derives_facts() {
        let input = "\
Person(.Name) is an entity type.
Department(.Code) is an entity type.
Person works in Department.
Person heads Department.
If some Person heads some Department then that Person works in that Department.
This span with Person, Department provides the preferred identification scheme for Department Leadership.
Constraint Span 'Department Leadership' autofills from superset.";
        let ir = parse_markdown(input).unwrap();
        let ss = ir.constraints.iter().find(|c| c.kind == "SS").expect("SS constraint");
        assert!(ss.spans.iter().any(|s| s.subset_autofill == Some(true)), "autofill should be set");
    }

    // â”€â”€ Deontic constraint fixes (2026-04-03) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn deontic_forbidden_extracts_entity() {
        let input = "\
Support Response(.id) is an entity type.
Dash is a value type.
  The possible values of Dash are '\u{2014}', '\u{2013}', '--'.
## Fact Types
Support Response uses Dash.
## Deontic Constraints
It is forbidden that Support Response uses Dash.";
        let ir = parse_markdown(input).unwrap();
        let c = ir.constraints.iter()
            .find(|c| c.text.contains("forbidden") && c.text.contains("Dash"))
            .expect("Should have forbidden Dash constraint");
        assert_eq!(c.entity, Some("Support Response".into()),
            "Deontic constraint should extract entity from text");
    }

    #[test]
    fn deontic_constraint_has_nonempty_span() {
        let input = "\
Support Response(.id) is an entity type.
Dash is a value type.
  The possible values of Dash are '\u{2014}', '\u{2013}', '--'.
## Fact Types
Support Response uses Dash.
## Deontic Constraints
It is forbidden that Support Response uses Dash.";
        let ir = parse_markdown(input).unwrap();
        let c = ir.constraints.iter()
            .find(|c| c.text.contains("forbidden") && c.text.contains("Dash"))
            .unwrap();
        assert!(!c.spans.is_empty(), "Deontic constraint should have at least one span");
        assert!(!c.spans[0].fact_type_id.is_empty(),
            "Span fact_type_id should be resolved, got empty");
        assert_eq!(c.spans[0].fact_type_id, "Support_Response_uses_Dash",
            "Span should reference the correct fact type");
    }

    #[test]
    fn deontic_constraint_has_unique_id() {
        let input = "\
Support Response(.id) is an entity type.
Dash is a value type.
  The possible values of Dash are '\u{2014}', '\u{2013}', '--'.
Markdown Syntax is a value type.
  The possible values of Markdown Syntax are '**', '##'.
## Fact Types
Support Response uses Dash.
Support Response contains Markdown Syntax.
## Deontic Constraints
It is forbidden that Support Response uses Dash.
It is forbidden that Support Response contains Markdown Syntax.";
        let ir = parse_markdown(input).unwrap();
        let deontics: Vec<&ConstraintDef> = ir.constraints.iter()
            .filter(|c| c.modality == "deontic")
            .collect();
        assert!(deontics.len() >= 2, "Should have at least 2 deontic constraints");
        // IDs should be unique (not all empty)
        let ids: std::collections::HashSet<&str> = deontics.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids.len(), deontics.len(),
            "Each deontic constraint should have a unique ID");
        assert!(!ids.contains(""), "No constraint should have an empty ID");
    }

    #[test]
    fn deontic_obligatory_extracts_entity() {
        let input = "\
Support Response(.id) is an entity type.
Pricing Model is a value type.
  The possible values of Pricing Model are 'subscription', 'metered'.
## Fact Types
Support Response conforms to Pricing Model.
## Deontic Constraints
It is obligatory that each Support Response conforms to Pricing Model.";
        let ir = parse_markdown(input).unwrap();
        let c = ir.constraints.iter()
            .find(|c| c.text.contains("obligatory") && c.text.contains("Pricing Model"))
            .expect("Should have obligatory Pricing Model constraint");
        assert_eq!(c.entity, Some("Support Response".into()));
        assert_eq!(c.modality, "deontic");
        assert_eq!(c.deontic_operator, Some("obligatory".into()));
    }

    #[test]
    fn deontic_forbidden_evaluates_enum_match() {
        let input = "\
Support Response(.id) is an entity type.
Dash is a value type.
  The possible values of Dash are '\u{2014}', '\u{2013}', '--'.
## Fact Types
Support Response uses Dash.
## Deontic Constraints
It is forbidden that Support Response uses Dash.";
        let ir = parse_markdown(input).unwrap();
        let model = crate::compile::compile(&ir);
        let response = crate::types::ResponseContext {
            text: "Hi -- here is your answer".into(),
            sender_identity: None,
            fields: None,
        };
        let population = crate::types::Population { facts: std::collections::HashMap::new() };
        let violations = crate::evaluate::evaluate_via_ast(&model, &response, &population);
        assert!(!violations.is_empty(),
            "Response containing '--' should violate the forbidden Dash constraint");
        assert!(violations.iter().any(|v| v.constraint_text.contains("Dash")),
            "Violation should reference the Dash constraint");
    }

    #[test]
    fn deontic_forbidden_clean_response_no_violations() {
        let input = "\
Support Response(.id) is an entity type.
Dash is a value type.
  The possible values of Dash are '\u{2014}', '\u{2013}', '--'.
## Fact Types
Support Response uses Dash.
## Deontic Constraints
It is forbidden that Support Response uses Dash.";
        let ir = parse_markdown(input).unwrap();
        let model = crate::compile::compile(&ir);
        let response = crate::types::ResponseContext {
            text: "Hi, here is your answer with no dashes at all".into(),
            sender_identity: None,
            fields: None,
        };
        let population = crate::types::Population { facts: std::collections::HashMap::new() };
        let violations = crate::evaluate::evaluate_via_ast(&model, &response, &population);
        let dash_violations: Vec<_> = violations.iter()
            .filter(|v| v.constraint_text.contains("Dash"))
            .collect();
        assert!(dash_violations.is_empty(),
            "Clean response should not trigger Dash violation");
    }

    #[test]
    fn deontic_multiple_forbidden_enum_constraints() {
        let input = "\
Support Response(.id) is an entity type.
Dash is a value type.
  The possible values of Dash are '\u{2014}', '\u{2013}', '--'.
Markdown Syntax is a value type.
  The possible values of Markdown Syntax are '**', '##', '###', '```'.
## Fact Types
Support Response uses Dash.
Support Response contains Markdown Syntax.
## Deontic Constraints
It is forbidden that Support Response uses Dash.
It is forbidden that Support Response contains Markdown Syntax.";
        let ir = parse_markdown(input).unwrap();
        let model = crate::compile::compile(&ir);
        // Response with both dashes and markdown
        let response = crate::types::ResponseContext {
            text: "## Heading\n\nHere is info -- with **bold** text".into(),
            sender_identity: None,
            fields: None,
        };
        let population = crate::types::Population { facts: std::collections::HashMap::new() };
        let violations = crate::evaluate::evaluate_via_ast(&model, &response, &population);
        assert!(violations.iter().any(|v| v.constraint_text.contains("Dash")),
            "Should catch dash violation");
        assert!(violations.iter().any(|v| v.constraint_text.contains("Markdown")),
            "Should catch markdown violation");
    }
}
