// crates/arest/src/parse_forml2.rs
//
// FORML 2 Parser -- FFP composition of recognizer functions.
//
// Per the paper: parse: R -> Phi (Theorem 2).
// parse = alpha(recognize) : lines
// recognize = try1 ; try2 ; ... ; tryn
//
// Each recognizer: &str -> Option<ParseAction>
// The ? operator IS the conditional form <COND, is_some, unwrap, _|_>.
// No if/else chains. Pattern matching via strip_suffix/strip_prefix/find.

use crate::types::*;
use std::collections::HashMap;

/// Metamodel-reserved noun names. User domains MUST NOT redeclare these —
/// the metamodel bootstrap owns them. The FORML2 parser rejects any attempt
/// by a user domain to shadow these. The first (bootstrap) declaration is
/// allowed; once present in `existing_nouns`, redeclaration is rejected.
// Bootstrap mode flag — set by lib::create_impl while loading bundled
// metamodel readings, so the metamodel namespace guard (#23) is bypassed
// for cross-file redeclarations within the canonical metamodel. Apps must
// NOT set this flag; user-domain compiles always hit the guard.
thread_local! {
    static BOOTSTRAP_MODE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(crate) fn set_bootstrap_mode(on: bool) {
    BOOTSTRAP_MODE.with(|b| b.set(on));
}

fn is_bootstrap_mode() -> bool {
    BOOTSTRAP_MODE.with(|b| b.get())
}

pub(crate) const METAMODEL_NOUNS: &[&str] = &[
    "Noun",
    "Graph Schema",
    "Role",
    "Constraint",
    "State Machine Definition",
    "Transition",
    "Status",
    "Event Type",
    "Domain Change",
];

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
// Recognizers -- pure functions: &str -> Option<ParseAction>
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

    // IR: "No A R itself." -- simple irreflexive
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
        // IR pattern: "No A R itself" -- must end with " itself" and have a known noun
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
    clean.starts_with("If ").then_some(())?;
    let then_idx = clean.find(" then ")?;
    let antecedent = &clean[3..then_idx]; // everything after "If " up to " then "
    let consequent = &clean[then_idx + 6..]; // everything after " then "

    // All role tokens in the antecedent must share the same base noun type.
    // Extract words that match known nouns (with or without trailing digit subscripts).
    let role_bases: Vec<&str> = antecedent
        .split_whitespace()
        .filter_map(|word| {
            let w = word.trim_end_matches(',');
            let (base, _) = parse_role_token(w);
            noun_names.iter().any(|n| n == base).then_some(base)
        })
        .collect();

    // Need at least 2 role tokens in antecedent, all with the same base
    (role_bases.len() >= 2).then_some(())?;
    let first_base = role_bases[0];
    role_bases.iter().all(|b| *b == first_base).then_some(())?;

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
    consequent_has_same_noun.then_some(())?;

    let has_and = antecedent.contains(" and ");
    let impossible = consequent.starts_with("it is impossible that ");
    let itself_in_consequent = consequent.contains(" itself");
    let is_not_in_antecedent = antecedent.contains(" is not ");

    let kind = match (has_and, impossible, itself_in_consequent, is_not_in_antecedent) {
        // AS: no and, impossible, no itself -- "If A1 R A2 then it is impossible that A2 R A1"
        (false, true, false, _)  => "AS",
        // RF: no and, not impossible, itself in consequent -- "If A1 R some A2 then A1 R itself"
        (false, false, true, _)  => "RF",
        // SY: no and, not impossible, no itself -- "If A1 R A2 then A2 R A1"
        (false, false, false, _) => "SY",
        // AT: and, impossible, "is not" in antecedent -- "If A1 R A2 and A1 is not A2 then impossible A2 R A1"
        (true, true, _, true)    => "AT",
        // IT: and, impossible, no "is not" -- "If A1 R A2 and A2 R A3 then impossible A1 R A3"
        (true, true, _, false)   => "IT",
        // TR: and, not impossible -- "If A1 R A2 and A2 R A3 then A1 R A3"
        (true, false, _, _)      => "TR",
        // Unrecognized combination -- not a ring constraint
        _ => return None,
    };

    Some(ParseAction::AddConstraint(ConstraintDef {
        id: String::new(), kind: kind.into(), modality: "alethic".into(),
        deontic_operator: None, text: clean.into(),
        spans: vec![], set_comparison_argument_length: None, clauses: None,
        entity: None, min_occurrence: None, max_occurrence: None,
    }))
}

/// try_subset -- SS: "If some A R1 some B then that A R2 that B."
/// Distinguishes from ring: subset has multiple different base noun types.
fn try_subset(line: &str, noun_names: &[String]) -> Option<ParseAction> {
    let clean = line.trim_end_matches('.');
    // Must start with "If " and contain " then "
    clean.starts_with("If ").then_some(())?;
    let then_idx = clean.find(" then ")?;
    let antecedent = &clean[3..then_idx];
    let consequent = &clean[then_idx + 6..];

    // Antecedent must contain "some" (existential), consequent must contain "that" (back-ref)
    antecedent.contains("some ").then_some(())?;
    consequent.contains("that ").then_some(())?;

    // Collect base noun types from antecedent using find_nouns (handles multi-word nouns)
    let stripped_ant = antecedent.replace("some ", "").replace("that ", "");
    let ant_found = find_nouns(&stripped_ant, noun_names);
    let ant_bases: Vec<&str> = ant_found.iter().map(|(_, _, n)| n.as_str()).collect();

    (ant_bases.len() >= 2).then_some(())?;

    // Subset has multiple DIFFERENT base noun types (distinguishes from ring which has all same)
    let first = ant_bases[0];
    (!ant_bases.iter().all(|b| b == &first)).then_some(())?;

    // Build spans: [0] = subset (antecedent), [1] = superset (consequent)
    // SpanDef with empty fact_type_id -- resolve_constraint_schema fills it in later
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

/// try_equality -- EQ: "...if and only if..." or "all or none of the following hold:..."
fn try_equality(line: &str) -> Option<ParseAction> {
    let clean = line.trim_end_matches('.');
    let matches = clean.contains(" if and only if ")
        || clean.to_lowercase().starts_with("all or none of the following hold");
    matches.then_some(())?;
    Some(ParseAction::AddConstraint(ConstraintDef {
        id: String::new(), kind: "EQ".into(), modality: "alethic".into(),
        deontic_operator: None, text: clean.into(),
        spans: vec![], set_comparison_argument_length: None, clauses: None,
        entity: None, min_occurrence: None, max_occurrence: None,
    }))
}

/// try_set_comparison -- XO, XC, OR
/// Patterns:
///   "For each A, exactly one of the following holds: ..." -> XO
///   "For each A, at most one of the following holds: ..."  -> XC
///   "For each A, at least one of the following holds: ..." -> OR (inclusive disjunction)
///   "Each A R1 some B1 or R2 some B2."                   -> OR (DMaC disjunctive MC)
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

    // "Each A R1 some B1 or R2 some B2." -- DMaC disjunctive MC -> OR
    if let Some(rest) = clean.strip_prefix("Each ") {
        // Must contain " or " and reference known nouns
        rest.contains(" or ").then_some(())?;
        // Find a known entity noun at the start
        let entity = noun_names.iter().find(|n| rest.starts_with(n.as_str()))?.clone();
        let after = rest[entity.len()..].trim();
        // Must have " or " in the remainder (not " or a/an " as in totality)
        after.contains(" or ").then_some(())?;
        // Exclude totality pattern: "Each X is a Y or a Z" (handled by try_totality)
        let or_idx = after.find(" or ")?;
        let after_or = &after[or_idx + 4..];
        // Totality uses "a " / "an " after "or"; disjunctive MC uses a predicate verb
        let is_totality = after_or.starts_with("a ") || after_or.starts_with("an ");
        (!is_totality).then_some(())?;

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

/// try_frequency -- FC: "Each A R at least {k} and at most {m} B."
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
    (min_end > 0).then_some(())?; // no digit found
    let min_str = &after_al[..min_end];
    let min_val: usize = min_str.parse().ok()?;

    // Look for optional "and at most {digit}"
    let max_val: Option<usize> = after_al[min_end..].find("at most ")
        .and_then(|i| {
            let after_am = &after_al[min_end + i + 8..];
            let max_end = after_am.find(|c: char| !c.is_ascii_digit()).unwrap_or(after_am.len());
            (max_end > 0).then_some(())?;
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

/// try_external_uc -- UC (external uniqueness and context pattern)
/// Patterns:
///   "For each B1 and B2, at most one A R1 that B1 and R2 that B2."
///   "Context: F1; F2. In this context, each B1, B2 combination is associated with at most one A."
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

    // "For each B1 and B2, at most one A ..."
    if let Some(rest) = clean.strip_prefix("For each ") {
        // Must have "at most one" in the body
        clean.contains("at most one").then_some(())?;
        // Must have " and " in the "For each" list (external UC uses "B1 and B2")
        let comma_idx = rest.find(',')?;
        let quantified = &rest[..comma_idx];
        quantified.contains(" and ").then_some(())?;

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
    // e.g., "each Support Response uses Dash" -> entity = "Support Response"
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

/// try_span_naming -- "This span with A, B provides the preferred identification scheme for SpanName."
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

/// try_autofill_declaration -- "Constraint Span 'SpanName' autofills from superset."
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
    (!noun_names.iter().any(|noun| line.starts_with(&format!("{} '", noun)))).then_some(())?;
    let (ft_id, ft_def) = parse_fact(line, noun_names)?;
    Some(ParseAction::AddFactType(ft_id, ft_def))
}

// =========================================================================
// Main parser -- fold recognizers over lines
// =========================================================================

/// Parse with pre-existing nouns from other domains.
/// Domains are NORMA tabs. Nouns are global across the UoD.
pub fn parse_markdown_with_nouns(input: &str, existing_nouns: &HashMap<String, NounDef>) -> Result<Domain, String> {
    parse_markdown_with_context(input, existing_nouns, &HashMap::new())
}

pub fn parse_markdown_with_context(input: &str, existing_nouns: &HashMap<String, NounDef>, existing_fact_types: &HashMap<String, FactTypeDef>) -> Result<Domain, String> {
    // Metamodel namespace protection (security #23):
    // First parse the input in isolation to see which nouns IT actually declares.
    // If the input declares any metamodel-reserved noun AND that noun is already
    // present in `existing_nouns` (i.e. this is a user domain layered on top of
    // the metamodel bootstrap), reject. The bootstrap case (no existing nouns
    // for those names) is allowed to declare them exactly once.
    let mut standalone = Domain {
        domain: String::new(), nouns: HashMap::new(), fact_types: HashMap::new(),
        constraints: vec![], state_machines: HashMap::new(), derivation_rules: vec![],
        general_instance_facts: vec![],
        subtypes: HashMap::new(), enum_values: HashMap::new(),
        ref_schemes: HashMap::new(), objectifications: HashMap::new(),
        named_spans: HashMap::new(), autofill_spans: vec![],
    };
    parse_into(&mut standalone, input)?;
    // Metamodel namespace guard (#23). Skipped during bundled metamodel
    // bootstrap — the metamodel is loaded as a series of cross-referencing
    // files and legitimately redeclares the same reserved nouns.
    if !is_bootstrap_mode() {
        if let Some(reserved) = METAMODEL_NOUNS.iter()
            .find(|n| standalone.nouns.contains_key(**n) && existing_nouns.contains_key(**n))
        {
            return Err(format!("metamodel noun '{}' cannot be redeclared", reserved));
        }
    }

    let mut ir = Domain {
        domain: String::new(), nouns: existing_nouns.clone(), fact_types: existing_fact_types.clone(),
        constraints: vec![], state_machines: HashMap::new(), derivation_rules: vec![],
        general_instance_facts: vec![],
        subtypes: HashMap::new(), enum_values: HashMap::new(),
        ref_schemes: HashMap::new(), objectifications: HashMap::new(),
        named_spans: HashMap::new(), autofill_spans: vec![],
    };
    parse_into(&mut ir, input)?;
    Ok(ir)
}

/// SSRF defense (#25). Reject URLs that point at internal/loopback/link-local
/// networks, file:// schemes, or internal DNS names. Hardcoded patterns only —
/// no DNS resolution, no network I/O. Called during platform_compile to
/// validate External System instance facts before they enter state.
pub fn is_forbidden_url(url: &str) -> bool {
    let trimmed = url.trim();
    let lower = trimmed.to_lowercase();

    // file:// scheme is always forbidden
    match lower.starts_with("file://") {
        true => return true,
        false => {}
    }

    // Extract the host component from http(s) URLs. Non-http schemes fall
    // through and are allowed (the check is scoped to federated HTTP URLs).
    let after_scheme = match lower.strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
    {
        Some(rest) => rest,
        None => return false,
    };

    // Strip userinfo (before '@'), then extract the host.
    let no_userinfo = after_scheme.rfind('@').map(|i| &after_scheme[i + 1..]).unwrap_or(after_scheme);

    // Bracketed IPv6 literal: [addr]:port/path -- must find the closing ']'
    // BEFORE searching for ':' (otherwise we split inside the brackets).
    // Bare host: split on the first '/', '?', or '#' to get the authority,
    // then heuristically detect bare IPv6 (authority has 2+ colons) vs the
    // normal host:port form (one colon).
    let host_bare: &str = match no_userinfo.strip_prefix('[') {
        Some(rest) => rest.find(']').map(|i| &rest[..i]).unwrap_or(rest),
        None => {
            let path_start = no_userinfo.find(|c: char| c == '/' || c == '?' || c == '#')
                .unwrap_or(no_userinfo.len());
            let authority = &no_userinfo[..path_start];
            // Bare IPv6 has multiple ':' in the authority (no port syntax
            // without brackets is well-defined, so treat the entire authority
            // as the host). host:port has exactly one ':' which we strip.
            match authority.matches(':').count() {
                0 => authority,
                1 => authority.split(':').next().unwrap_or(authority),
                _ => authority, // bare IPv6 — keep colons for ULA / link-local checks
            }
        }
    };

    // Empty host is bottom-safe — treat as forbidden.
    match host_bare.is_empty() {
        true => return true,
        false => {}
    }

    // Exact-name checks
    match host_bare {
        "localhost" | "::1" | "::" | "0.0.0.0" => return true,
        _ => {}
    }

    // Internal DNS suffixes (case-insensitive — lower already applied)
    let forbidden_suffix = host_bare.ends_with(".local")
        || host_bare.ends_with(".internal")
        || host_bare.ends_with(".localhost");
    match forbidden_suffix {
        true => return true,
        false => {}
    }

    // IPv4 checks: parse dotted-quad octets. Non-numeric hosts fall through.
    let octets: Vec<u16> = host_bare.split('.')
        .filter_map(|p| p.parse::<u16>().ok())
        .collect();
    let is_ipv4 = octets.len() == 4 && octets.iter().all(|o| *o <= 255);
    match is_ipv4 {
        true => {
            let (a, b) = (octets[0], octets[1]);
            // 127.*.*.* loopback
            // 10.*.*.* private
            // 169.254.*.* link-local (incl. AWS metadata 169.254.169.254)
            // 192.168.*.* private
            // 172.16-31.*.* private
            let forbidden_v4 = a == 127
                || a == 10
                || (a == 169 && b == 254)
                || (a == 192 && b == 168)
                || (a == 172 && b >= 16 && b <= 31);
            match forbidden_v4 {
                true => return true,
                false => {}
            }
        }
        false => {}
    }

    // IPv6 link-local: fe80::/10 — first octet of the address
    // is 0xfe and top two bits of the second are 10 (0x80..0xbf).
    // Covers fe80: through febf:.
    let ipv6_linklocal = host_bare.starts_with("fe8")
        || host_bare.starts_with("fe9")
        || host_bare.starts_with("fea")
        || host_bare.starts_with("feb");
    match ipv6_linklocal {
        true => return true,
        false => {}
    }

    // IPv6 unique-local: fc00::/7 (fc00 through fdff)
    let ipv6_ula = host_bare.starts_with("fc") || host_bare.starts_with("fd");
    // Only treat as ULA if the host looks like an IPv6 address (contains ':').
    match ipv6_ula && host_bare.contains(':') {
        true => return true,
        false => {}
    }

    false
}

/// Scan the InstanceFact cell in parsed state and return the first
/// forbidden URL found, if any. Used by platform_compile to reject
/// External System federation to internal/loopback/link-local hosts.
pub fn find_forbidden_instance_url(state: &crate::ast::Object) -> Option<String> {
    use crate::ast::{fetch_or_phi, binding};
    fetch_or_phi("InstanceFact", state)
        .as_seq()
        .and_then(|facts| {
            facts.iter().find_map(|f| {
                let object_value = binding(f, "objectValue")?;
                is_forbidden_url(object_value).then(|| object_value.to_string())
            })
        })
}

/// Parse FORML2 readings directly into an Object state.
/// Every declaration becomes a fact (cell) in state. No intermediate struct.
pub fn parse_to_state(input: &str) -> Result<crate::ast::Object, String> {
    let domain = parse_markdown(input)?;
    Ok(domain_to_state(&domain))
}

/// Extract nouns directly from the Noun cell in D.
pub fn nouns_from_state(state: &crate::ast::Object) -> HashMap<String, NounDef> {
    use crate::ast::{fetch_or_phi, binding};
    fetch_or_phi("Noun", state)
        .as_seq().map(|facts| facts.iter().filter_map(|f| {
            let name = binding(f, "name")?.to_string();
            let obj_type = binding(f, "objectType").unwrap_or("entity").to_string();
            Some((name, NounDef { object_type: obj_type, world_assumption: WorldAssumption::default() }))
        }).collect())
        .unwrap_or_default()
}

/// Extract fact types directly from the GraphSchema cell in D.
pub fn fact_types_from_state(state: &crate::ast::Object) -> HashMap<String, FactTypeDef> {
    use crate::ast::{fetch_or_phi, binding};
    fetch_or_phi("GraphSchema", state)
        .as_seq().map(|facts| facts.iter().filter_map(|f| {
            let id = binding(f, "id")?.to_string();
            let reading = binding(f, "reading").unwrap_or("").to_string();
            Some((id, FactTypeDef {
                schema_id: String::new(),
                reading,
                readings: vec![],
                roles: vec![], // roles resolved separately if needed
            }))
        }).collect())
        .unwrap_or_default()
}

/// Parse FORML2 readings into an Object state with full context from D.
/// Extracts nouns and fact types directly from cells — no Domain struct round-trip.
pub fn parse_to_state_from(input: &str, d: &crate::ast::Object) -> Result<crate::ast::Object, String> {
    let nouns = nouns_from_state(d);
    let fact_types = fact_types_from_state(d);
    let domain = parse_markdown_with_context(input, &nouns, &fact_types)?;
    Ok(domain_to_state(&domain))
}

/// Legacy: parse with nouns only (no fact type context).
pub fn parse_to_state_with_nouns(input: &str, existing: &crate::ast::Object) -> Result<crate::ast::Object, String> {
    let nouns = nouns_from_state(existing);
    let domain = parse_markdown_with_nouns(input, &nouns)?;
    Ok(domain_to_state(&domain))
}

/// Convert a Domain to an Object state (sequence of cells).
/// Each category becomes a cell: <CELL, fact_type_id, <facts...>>
pub fn domain_to_state(d: &Domain) -> crate::ast::Object {
    use crate::ast::{Object, cell_push, fact_from_pairs};
    let mut state = Object::phi();

    // foldl(cell_push("Noun"), state, α(noun_to_fact) : nouns)
    state = d.nouns.iter().fold(state, |acc, (name, def)| {
        let mut pairs: Vec<(String, String)> = vec![
            ("name".into(), name.clone()), ("objectType".into(), def.object_type.clone()),
        ];
        d.subtypes.get(name).map(|st| pairs.push(("superType".into(), st.clone())));
        d.ref_schemes.get(name).map(|rs| pairs.push(("referenceScheme".into(), rs.join(","))));
        d.enum_values.get(name).filter(|evs| !evs.is_empty()).map(|evs| pairs.push(("enumValues".into(), evs.join(","))));
        let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        cell_push("Noun", fact_from_pairs(&refs), &acc)
    });

    // foldl(cell_push, state, α(ft → schema+roles)) : fact_types
    state = d.fact_types.iter().fold(state, |acc, (ft_id, ft)| {
        let with_schema = cell_push("GraphSchema", fact_from_pairs(&[
            ("id", ft_id), ("reading", &ft.reading), ("arity", &ft.roles.len().to_string()),
        ]), &acc);
        ft.roles.iter().fold(with_schema, |a, role| cell_push("Role", fact_from_pairs(&[
            ("graphSchema", ft_id), ("nounName", &role.noun_name), ("position", &role.role_index.to_string()),
        ]), &a))
    });

    // foldl(cell_push("Constraint"), state, α(constraint_to_fact))
    state = d.constraints.iter().fold(state, |acc, c| {
        let mut pairs: Vec<(String, String)> = vec![
            ("id".into(), c.id.clone()), ("kind".into(), c.kind.clone()),
            ("modality".into(), c.modality.clone()), ("text".into(), c.text.clone()),
        ];
        c.deontic_operator.as_ref().map(|op| pairs.push(("deonticOperator".into(), op.clone())));
        c.entity.as_ref().map(|e| pairs.push(("entity".into(), e.clone())));
        pairs.extend(c.spans.iter().enumerate().flat_map(|(i, span)| [
            (format!("span{}_factTypeId", i), span.fact_type_id.clone()),
            (format!("span{}_roleIndex", i), span.role_index.to_string()),
        ]));
        let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        cell_push("Constraint", fact_from_pairs(&refs), &acc)
    });

    // foldl(cell_push("DerivationRule")) + foldl(cell_push("InstanceFact"))
    state = d.derivation_rules.iter().fold(state, |acc, r| cell_push("DerivationRule", fact_from_pairs(&[
        ("id", r.id.as_str()), ("text", r.text.as_str()), ("consequentFactTypeId", r.consequent_fact_type_id.as_str()),
    ]), &acc));

    state = d.general_instance_facts.iter().fold(state, |acc, f| cell_push("InstanceFact", fact_from_pairs(&[
        ("subjectNoun", f.subject_noun.as_str()), ("subjectValue", f.subject_value.as_str()),
        ("fieldName", f.field_name.as_str()), ("objectNoun", f.object_noun.as_str()),
        ("objectValue", f.object_value.as_str()),
    ]), &acc));

    state
}

/// Materialize Domain into entity cells for EntityDB.
pub fn domain_to_entities(d: &Domain, slug: &str) -> String {
    let e: Vec<serde_json::Value> = [
        // α(noun → json) : nouns
        d.nouns.iter().map(|(n, def)| {
            let mut data = serde_json::json!({"name": n, "objectType": &def.object_type});
            d.subtypes.get(n).map(|s| data["superType"] = serde_json::json!(s));
            d.enum_values.get(n).map(|v| data["enumValues"] = serde_json::json!(v));
            d.ref_schemes.get(n).map(|r| data["referenceScheme"] = serde_json::json!(r));
            d.objectifications.get(n).map(|o| data["objectifies"] = serde_json::json!(o));
            serde_json::json!({"id": format!("noun:{}", n), "type": "Noun", "domain": slug, "data": data})
        }).collect::<Vec<_>>(),
        // α(ft → [schema, roles..., reading]) : fact_types
        d.fact_types.iter().flat_map(|(id, ft)| {
            std::iter::once(serde_json::json!({"id": format!("gs:{}", id), "type": "Graph Schema", "domain": slug, "data": {"name": id, "reading": ft.reading}}))
                .chain(ft.roles.iter().enumerate().map(move |(i, role)|
                    serde_json::json!({"id": format!("role:{}:{}", id, i), "type": "Role", "domain": slug, "data": {"nounName": role.noun_name, "position": i, "graphSchema": id}})))
                .chain(std::iter::once(serde_json::json!({"id": format!("reading:{}", id), "type": "Reading", "domain": slug, "data": {"text": ft.reading, "graphSchema": id}})))
        }).collect(),
        // α(constraint → json)
        d.constraints.iter().map(|c| {
            let cid = if c.id.is_empty() { &c.text } else { &c.id };
            serde_json::json!({"id": format!("constraint:{}", cid), "type": "Constraint", "domain": slug,
                "data": {"kind": c.kind, "modality": c.modality, "text": c.text, "spans": c.spans, "entity": c.entity, "minOccurrence": c.min_occurrence, "maxOccurrence": c.max_occurrence}})
        }).collect(),
        // α(sm → [def, statuses..., transitions...])
        d.state_machines.iter().flat_map(|(n, sm)| {
            std::iter::once(serde_json::json!({"id": n, "type": "State Machine Definition", "domain": slug, "data": {"name": n, "forNoun": sm.noun_name}}))
                .chain(sm.statuses.iter().map(move |s| serde_json::json!({"id": format!("status:{}:{}", n, s), "type": "Status", "domain": slug, "data": {"name": s, "stateMachineDefinition": n}})))
                .chain(sm.transitions.iter().map(move |t| serde_json::json!({"id": format!("transition:{}:{}:{}", n, t.from, t.event), "type": "Transition", "domain": slug, "data": {"from": t.from, "to": t.to, "event": t.event, "stateMachineDefinition": n}})))
        }).collect(),
        // α(rule → json) + α(fact → json)
        d.derivation_rules.iter().map(|r| serde_json::json!({"id": format!("rule:{}", r.id), "type": "Derivation Rule", "domain": slug,
            "data": {"ruleId": r.id, "text": r.text, "antecedentFactTypeIds": serde_json::to_string(&r.antecedent_fact_type_ids).unwrap_or_default(),
                "consequentFactTypeId": r.consequent_fact_type_id, "kind": serde_json::to_string(&r.kind).unwrap_or_default(),
                "joinOn": serde_json::to_string(&r.join_on).unwrap_or_default(), "matchOn": serde_json::to_string(&r.match_on).unwrap_or_default(),
                "consequentBindings": serde_json::to_string(&r.consequent_bindings).unwrap_or_default()}})).collect(),
        d.general_instance_facts.iter().map(|f| serde_json::json!({"id": format!("instance-fact:{}:{}:{}", f.subject_noun, f.subject_value, f.field_name), "type": "Instance Fact", "domain": slug,
            "data": {"subjectNoun": f.subject_noun, "subjectValue": f.subject_value, "fieldName": f.field_name, "objectNoun": f.object_noun, "objectValue": f.object_value}})).collect(),
    ].concat();
    serde_json::to_string(&e).unwrap_or_else(|_| "[]".into())
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

    // Pass 1: alpha(recognize_noun) : lines -- extract nouns and domain
    (0..lines.len()).for_each(|i| {
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

        // Look ahead for enum values after value type declaration:
        // Filter(non_empty) ∘ skip(i+1) : lines, then match first result.
        line.strip_suffix(" is a value type.")
            .map(|prefix| prefix.trim())
            .and_then(|name| lines.iter().skip(i + 1)
                .map(|s| s.trim())
                .find(|s| !s.is_empty())
                .filter(|s| s.starts_with("The possible values of"))
                .and_then(parse_enum)
                .map(|vals| (name.to_string(), vals)))
            .into_iter()
            .for_each(|(name, vals)| { ir.enum_values.insert(name, vals); });
    });

    // Pass 2a: collect fact types and instance facts
    // Sorted longest-first for Theorem 1 (unambiguous longest-first matching)
    let mut noun_names: Vec<String> = ir.nouns.keys().cloned().collect();
    noun_names.sort_by(|a, b| b.len().cmp(&a.len()));

    // Pass 2a: Filter(!pass1 && !pass2b) : lines, then apply fact_type/instance_fact
    (0..lines.len())
        .map(|i| (i, lines[i].trim()))
        .filter(|(_, line)| !line.is_empty())
        .filter(|(_, line)| {
            let is_pass1 = try_entity_type(line).is_some()
                || try_value_type(line).is_some()
                || (try_subtype(line).is_some() && !line.starts_with("Each"))
                || try_abstract(line).is_some()
                || try_partition(line).is_some()
                || try_enum_values(line).is_some()
                || try_exclusive_subtypes(line).is_some()
                || try_association(line).is_some()
                || try_header(line).is_some();
            !is_pass1
        })
        .filter(|(_, line)| {
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
            !is_pass2b
        })
        .for_each(|(i, line)| {
            let action = try_fact_type(line, &noun_names)
                .or_else(|| try_instance_fact(line));
            apply_action(ir, action, &lines, i);
        });

    // Build schema catalog from collected fact types
    let catalog = {
        let mut cat = SchemaCatalog::new();
        ir.fact_types.iter().for_each(|(schema_id, ft)| {
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
        });
        cat
    };

    // Pass 2b: Filter(!pass1) : lines, then apply constraint/derivation/deontic recognizers.
    (0..lines.len())
        .map(|i| (i, lines[i].trim()))
        .filter(|(_, line)| !line.is_empty())
        .filter(|(_, line)| {
            let is_pass1 = try_entity_type(line).is_some()
                || try_value_type(line).is_some()
                || (try_subtype(line).is_some() && !line.starts_with("Each"))
                || try_abstract(line).is_some()
                || try_partition(line).is_some()
                || try_enum_values(line).is_some()
                || try_exclusive_subtypes(line).is_some()
                || try_association(line).is_some()
                || try_header(line).is_some();
            !is_pass1
        })
        .for_each(|(i, line)| {
            // Totality -> mark abstract (but don't skip -- still parse as constraint)
            apply_action(ir, try_totality(line), &lines, i);

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
            let Some(action) = action else { return; };

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
                    // "exactly one" = UC + MC. Each gets its own reading as id.
                    let mut uc = resolve_constraint_schema(c.clone(), &noun_names, &catalog, ir);
                    uc.kind = "UC".into();
                    uc.text = uc.text.replace("exactly one", "at most one");
                    uc.id.is_empty().then(|| uc.id = uc.text.clone());
                    let mut mc = resolve_constraint_schema(c, &noun_names, &catalog, ir);
                    mc.kind = "MC".into();
                    mc.text = mc.text.replace("exactly one", "some");
                    mc.id.is_empty().then(|| mc.id = mc.text.clone());
                    ir.constraints.push(uc);
                    ir.constraints.push(mc);
                }
                ParseAction::AddConstraint(c) => {
                    let mut resolved = resolve_constraint_schema(c, &noun_names, &catalog, ir);
                    resolved.id.is_empty().then(|| resolved.id = resolved.text.clone());
                    ir.constraints.push(resolved);
                }
                ParseAction::AddDerivation(mut r) => {
                    resolve_derivation_rule(&mut r, ir, &catalog);
                    ir.derivation_rules.push(r);
                }
                other => { apply_action(ir, Some(other), &lines, i); }
            }
        });

    // Task 6: Value Constraint (VC) -- emit one VC per noun with enum_values.
    // The compiler reads enum values from ir.enum_values;
    // the ConstraintDef just marks which noun has a value constraint.
    ir.constraints.extend(ir.enum_values.keys().cloned().map(|noun_name| ConstraintDef {
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
    }));

    // Post-processing: resolve autofill spans.
    // For each autofill span name, find SS constraints whose role nouns
    // match the named span's role nouns, and set subset_autofill = Some(true).
    let autofill_role_sets: Vec<std::collections::HashSet<String>> = ir.autofill_spans.clone()
        .iter()
        .filter_map(|span_name| ir.named_spans.get(span_name))
        .map(|nouns| nouns.iter().cloned().collect())
        .collect();
    ir.constraints.iter_mut()
        .filter(|cdef| cdef.kind == "SS")
        .filter(|cdef| autofill_role_sets.iter().any(|role_set| {
            role_set.iter().all(|n| cdef.text.contains(n.as_str()))
        }))
        .for_each(|cdef| {
            // Set autofill on the first span (subset span)
            cdef.spans.first_mut()
                .into_iter()
                .for_each(|span| span.subset_autofill = Some(true));
        });

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

    // Resolve a text fragment to a Graph Schema ID via rho-lookup through the catalog.
    let resolve_fact_type = |fragment: &str| -> Option<String> {
        let cleaned = strip_anaphora(fragment);
        let found_nouns: Vec<(usize, usize, String)> = find_nouns(&cleaned, &noun_names);
        let role_refs: Vec<&str> = found_nouns.iter().map(|(_, _, n)| n.as_str()).collect();

        // Extract the verb: text between the first and second noun
        let verb = found_nouns.windows(2)
            .next()
            .map(|pair| cleaned[pair[0].1..pair[1].0].trim())
            .unwrap_or("");

        // rho-lookup: try with verb first, then noun set only
        let verb_opt = (!verb.is_empty()).then_some(verb);
        catalog.resolve(&role_refs, verb_opt)
            .or_else(|| catalog.resolve(&role_refs, None))
    };

    // Detect "that X" anaphoric references -- nouns preceded by "that " in
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

    // Classify: if join keys exist AND at least 2 distinct antecedent fact types share
    // a noun, this is a Join derivation. Rules with "that X" anaphora where X appears
    // in multiple antecedents need an equi-join on X.
    let is_join = !rule.join_on.is_empty()
        && rule.antecedent_fact_type_ids.len() >= 2
        && rule.join_on.iter().any(|key| {
            rule.antecedent_fact_type_ids.iter()
                .filter(|ft_id| ir.fact_types.get(*ft_id)
                    .map_or(false, |ft| ft.roles.iter().any(|r| r.noun_name == *key)))
                .count() >= 2
        });
    is_join.then(|| {
        rule.kind = DerivationKind::Join;
        // Build match_on: pairs of (noun_a, noun_b) for equality matching
        rule.match_on = rule.join_on.iter()
            .map(|key| (key.clone(), key.clone()))
            .collect();
        // Consequent bindings: nouns from the consequent fact type
        rule.consequent_bindings = ir.fact_types.get(&rule.consequent_fact_type_id)
            .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
            .unwrap_or_default();
    });

    // Set rule ID from consequent
    rule.id = rule.consequent_fact_type_id.clone();
}

/// Apply a parse action to the IR accumulator.
fn apply_action(ir: &mut Domain, action: Option<ParseAction>, lines: &[String], idx: usize) {
    let Some(action) = action else { return };
    match action {
        ParseAction::SetDomain(d) => { if ir.domain.is_empty() { ir.domain = d; } }
        ParseAction::AddNoun(name, def, meta) => {
            // Record the declaration faithfully. If the noun already exists with
            // a different object type, the UC "Each Noun has exactly one Object Type"
            // will be caught by the validate pipeline during compile.
            let entry = ir.nouns.entry(name.clone()).or_insert_with(|| def.clone());
            // Explicit redeclaration overwrites (conflict detected in platform_compile)
            (entry.object_type != def.object_type && def.object_type != "abstract")
                .then(|| *entry = def.clone());
            // Merge: subtype/abstract declarations update existing nouns
            (def.object_type == "abstract")
                .then(|| entry.object_type = "abstract".into());
            // Populate IR maps from metadata
            meta.super_type.into_iter().for_each(|st| { ir.subtypes.insert(name.clone(), st); });
            meta.ref_scheme.into_iter().for_each(|rs| { ir.ref_schemes.entry(name.clone()).or_insert(rs); });
            meta.objectifies.into_iter().for_each(|obj| { ir.objectifications.insert(name.clone(), obj); });
        }
        ParseAction::MarkAbstract(name) => {
            ir.nouns.get_mut(&name).into_iter()
                .for_each(|noun| noun.object_type = "abstract".into());
        }
        ParseAction::AddPartition(sup, subs) => {
            ir.nouns.get_mut(&sup).into_iter()
                .for_each(|noun| noun.object_type = "abstract".into());
            subs.into_iter().for_each(|sub| {
                ir.nouns.entry(sub.clone()).or_insert(NounDef {
                    object_type: "entity".into(),
                    world_assumption: WorldAssumption::default(),
                });
                ir.subtypes.insert(sub, sup.clone());
            });
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
// Pure extraction functions (no if/else -- use ? and strip_prefix/suffix)
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

/// Schema catalog for rho-lookup: noun set -> Graph Schema ID.
/// The noun set is the key. The catalog is the DEFS cell.
struct SchemaCatalog {
    /// Sorted noun set -> vec of (schema_id, verb, reading) for disambiguation
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

    /// rho-lookup: noun set -> Graph Schema ID.
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

    let resolved_schema = (found.len() >= 2).then(|| {
        let role_nouns: Vec<&str> = found.iter().map(|(_, _, n)| n.as_str()).collect();
        // Extract verb between first two nouns
        let verb_text = stripped[found[0].1..found[1].0].trim();
        let verb = (!verb_text.is_empty()).then_some(verb_text);

        // Primary: rho-lookup through catalog (exact verb, then reading containment, then unique)
        // Secondary: verb containment against ir.fact_types readings (handles inverse voice
        // when multiple schemas share the same noun pair)
        catalog.resolve(&role_nouns, verb)
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
    }).flatten();

    resolved_schema.into_iter().for_each(|schema_id| {
        // Update spans to reference the resolved schema ID.
        // The constrained role is determined by the quantifier position
        // in the verbalization pattern. "Each A R at most one B" constrains
        // A's role (the quantified noun). "It is forbidden that A R B"
        // constrains A's role (the first noun after the prefix).
        // Per Halpin TechReport ORM2-02: the constrained role is the one
        // under the quantifier.
        let resolved_ft = ir.fact_types.get(&schema_id);
        let first_noun_idx = resolved_ft
            .and_then(|ft| {
                let first_noun = &found[0].2;
                ft.roles.iter().position(|r| &r.noun_name == first_noun)
            });
        constraint.spans.iter_mut().for_each(|span| {
            span.fact_type_id = schema_id.clone();
            // Set role_index to the first noun's position in the fact type.
            // The first noun in the constraint text is the quantified noun
            // ("Each A", "the same A", "A" after deontic prefix).
            first_noun_idx.into_iter().for_each(|idx| span.role_index = idx);
        });
    });
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
    // The fact type reading = "Noun1 predicate Noun2" -- extracted from the stripped text.
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

/// Find nouns in text -- longest-first matching with word boundaries.
/// Returns (start, end, name) tuples sorted by position.
fn find_nouns(text: &str, noun_names: &[String]) -> Vec<(usize, usize, String)> {
    let mut sorted: Vec<&String> = noun_names.iter().collect();
    sorted.sort_by(|a, b| b.len().cmp(&a.len()));

    // Foldl over longest-first noun list. Accumulator is (matches, used_ranges).
    // Inner loop over occurrences of `name` in `text` uses Backus's `while`
    // combining form (sequential scan of positions).
    let (mut matches, _): (Vec<(usize, usize, String)>, Vec<(usize, usize)>) = sorted.iter().fold(
        (Vec::new(), Vec::new()),
        |(mut matches, mut used), name| {
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
            (matches, used)
        },
    );

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

    // bu(match_subject, line) -- find the first noun that matches as subject
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

    // bu(match_object, rest) -- find the object noun+value in the remainder
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

    fact.into_iter().for_each(|f| ir.general_instance_facts.push(f));
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
    let mut words = s.split_whitespace().filter(|w| !w.is_empty());
    let first = match words.next() {
        Some(w) => w.to_lowercase(),
        None => return String::new(),
    };
    words.fold(first, |mut acc, word| {
        let mut chars = word.chars();
        chars.next().into_iter().for_each(|first_ch| {
            acc.push(first_ch.to_uppercase().next().unwrap_or(first_ch));
        });
        acc.extend(chars);
        acc
    })
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
        let input = "Domain(.Slug) is an entity type.\nAccess is a value type.\n  The possible values of Access are 'public', 'private'.\n## Fact Types\nDomain has Access.\n## Instance Facts\nDomain 'support' has Access 'public'.";
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
        let input = "Domain(.Slug) is an entity type.\nAccess is a value type.\nDomain has Access.\nDomain 'support' has Access 'public'.\nDomain 'core' has Access 'private'.";
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
        // All keys should be underscore format, not reading format
        assert!(ir.fact_types.keys().all(|key| !key.contains(' ')),
            "Fact type keys should not contain spaces");
        assert!(ir.fact_types.contains_key("Auth_Session_is_for_Customer"));
        assert!(ir.fact_types.contains_key("Auth_Session_uses_Session_Strategy"));
        // Constraints should reference schema IDs
        assert!(
            ir.constraints.iter().flat_map(|c| c.spans.iter())
                .all(|span| !span.fact_type_id.contains(' ')),
            "Constraint spans should not contain spaces"
        );
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

    // -- Deontic constraint fixes (2026-04-03) -------------------------

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

    /// Helper: compile IR to defs, then evaluate constraints via defs path.
    fn eval_deontic_defs(ir: &crate::types::Domain, text: &str) -> Vec<crate::types::Violation> {
        let state = domain_to_state(ir);
        let defs = crate::compile::compile_to_defs_state(&state);
        let empty_state = crate::ast::Object::phi();
        let def_obj = crate::ast::defs_to_state(&defs, &empty_state);
        let ctx_obj = crate::ast::encode_eval_context_state(text, None, &empty_state);
        defs.iter()
            .filter(|(n, _)| n.starts_with("constraint:"))
            .flat_map(|(name, func)| {
                let result = crate::ast::apply(func, &ctx_obj, &def_obj);
                let is_deontic = name.contains("obligatory") || name.contains("forbidden");
                crate::ast::decode_violations(&result).into_iter().map(move |mut v| {
                    v.alethic = !is_deontic;
                    v
                })
            })
            .collect()
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
        let violations = eval_deontic_defs(&ir, "Hi -- here is your answer");
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
        let violations = eval_deontic_defs(&ir, "Hi, here is your answer with no dashes at all");
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
        let violations = eval_deontic_defs(&ir, "## Heading\n\nHere is info -- with **bold** text");
        assert!(violations.iter().any(|v| v.constraint_text.contains("Dash")),
            "Should catch dash violation");
        assert!(violations.iter().any(|v| v.constraint_text.contains("Markdown")),
            "Should catch markdown violation");
    }

    // =====================================================================
    // Task #25 -- is_forbidden_url SSRF defense coverage.
    // =====================================================================

    // --- 1. Loopback IPv4 ---
    #[test]
    fn forbidden_loopback_ipv4_basic() {
        assert!(is_forbidden_url("http://127.0.0.1"));
        assert!(is_forbidden_url("https://127.0.0.1"));
    }

    #[test]
    fn forbidden_loopback_ipv4_alt_octets() {
        assert!(is_forbidden_url("http://127.1.2.3"));
        assert!(is_forbidden_url("http://127.255.255.254"));
    }

    #[test]
    fn forbidden_loopback_ipv4_with_port() {
        assert!(is_forbidden_url("http://127.0.0.1:8080"));
        assert!(is_forbidden_url("https://127.0.0.1:443/admin"));
    }

    #[test]
    fn forbidden_loopback_ipv4_with_path() {
        assert!(is_forbidden_url("http://127.0.0.1/admin/debug?x=1"));
        assert!(is_forbidden_url("https://127.0.0.1/secret#frag"));
    }

    // --- 2. Loopback IPv6 ---
    #[test]
    fn forbidden_loopback_ipv6_bare() {
        // `http://::1` parses with an empty host (the `::` splits host_end
        // at the first ':'), and the empty-host branch returns forbidden.
        assert!(is_forbidden_url("http://::1"));
    }

    // GAP: bracketed IPv6 literals are NOT detected by the current parser.
    // `host_end` splits on the first `:` which sits inside the brackets,
    // so `host` becomes just "[" and the bracket-strip path is never taken.
    // Documented here as an ignored regression so the gap is visible.
    #[test]
fn forbidden_loopback_ipv6_bracketed_with_port() {
        assert!(is_forbidden_url("http://[::1]:8080"));
        assert!(is_forbidden_url("https://[::1]:443/api"));
    }

    // --- 3. Link-local IPv4 (incl. AWS metadata) ---
    #[test]
    fn forbidden_link_local_ipv4() {
        assert!(is_forbidden_url("http://169.254.0.1"));
        assert!(is_forbidden_url("http://169.254.255.255"));
    }

    #[test]
    fn forbidden_aws_metadata_endpoint() {
        assert!(is_forbidden_url("http://169.254.169.254/latest/meta-data/"));
        assert!(is_forbidden_url("https://169.254.169.254"));
    }

    // --- 4. Private IPv4 ranges ---
    #[test]
    fn forbidden_private_10_range() {
        assert!(is_forbidden_url("http://10.0.0.1"));
        assert!(is_forbidden_url("https://10.255.255.254:8000"));
    }

    #[test]
    fn forbidden_private_172_range_lower_boundary() {
        assert!(is_forbidden_url("http://172.16.0.1"));
    }

    #[test]
    fn forbidden_private_172_range_upper_boundary() {
        assert!(is_forbidden_url("http://172.31.255.255"));
    }

    #[test]
    fn forbidden_private_192_168_range() {
        assert!(is_forbidden_url("http://192.168.1.1"));
        assert!(is_forbidden_url("https://192.168.0.1:9200/_cluster"));
    }

    // --- 5. IPv6 link-local ---
    #[test]
    fn forbidden_ipv6_link_local_fe80() {
        assert!(is_forbidden_url("http://fe80::1"));
    }

    #[test]
    fn forbidden_ipv6_link_local_febf() {
        assert!(is_forbidden_url("http://febf::ffff"));
    }

    // GAP: bracketed IPv6 literals are never detected (see note on
    // forbidden_loopback_ipv6_bracketed_with_port). Same underlying bug.
    #[test]
fn forbidden_ipv6_link_local_bracketed() {
        assert!(is_forbidden_url("http://[fe80::1]:8080"));
        assert!(is_forbidden_url("https://[febf::dead:beef]/x"));
    }

    // --- 6. IPv6 Unique Local (ULA) ---
    //
    // GAP: the ULA check requires host_bare.contains(':'), but host_end
    // splits on the first ':' so bare-form `fc00::1` arrives here as
    // "fc00" with no colon. The check never fires. Documented as ignored
    // regressions.
    #[test]
fn forbidden_ipv6_ula_fc00() {
        assert!(is_forbidden_url("http://fc00::1"));
        assert!(is_forbidden_url("http://[fc00::1]:8080"));
    }

    #[test]
fn forbidden_ipv6_ula_fd() {
        assert!(is_forbidden_url("http://fd12::abcd"));
        assert!(is_forbidden_url("https://[fd12:3456::1]/api"));
    }

    // --- 7. file:// scheme ---
    #[test]
    fn forbidden_file_scheme_absolute() {
        assert!(is_forbidden_url("file:///etc/passwd"));
    }

    #[test]
    fn forbidden_file_scheme_with_host() {
        assert!(is_forbidden_url("file://localhost/etc/hosts"));
    }

    #[test]
    fn forbidden_file_scheme_case_insensitive() {
        assert!(is_forbidden_url("FILE:///etc/passwd"));
        assert!(is_forbidden_url("File:///C:/Windows/System32"));
    }

    // --- 8. Internal DNS suffixes ---
    #[test]
    fn forbidden_local_suffix() {
        assert!(is_forbidden_url("http://printer.local"));
        assert!(is_forbidden_url("https://service.local/api"));
    }

    #[test]
    fn forbidden_internal_suffix() {
        assert!(is_forbidden_url("http://api.internal"));
        assert!(is_forbidden_url("https://vault.corp.internal/secret"));
    }

    #[test]
    fn forbidden_localhost_suffix() {
        assert!(is_forbidden_url("http://dev.localhost"));
        assert!(is_forbidden_url("https://app.localhost:3000"));
    }

    // --- 9. Bare localhost ---
    #[test]
    fn forbidden_bare_localhost() {
        assert!(is_forbidden_url("http://localhost"));
        assert!(is_forbidden_url("https://localhost:8080"));
        assert!(is_forbidden_url("http://localhost/api/admin"));
    }

    #[test]
    fn forbidden_zero_host() {
        assert!(is_forbidden_url("http://0.0.0.0"));
        assert!(is_forbidden_url("http://0.0.0.0:8080/"));
    }

    // --- 10. PUBLIC URLs must NOT be rejected ---
    #[test]
    fn allowed_public_https_hostname() {
        assert!(!is_forbidden_url("https://example.com"));
        assert!(!is_forbidden_url("https://example.com/path?x=1"));
    }

    #[test]
    fn allowed_public_ipv4_dns_resolver() {
        assert!(!is_forbidden_url("http://8.8.8.8"));
        assert!(!is_forbidden_url("https://8.8.4.4:443/dns-query"));
    }

    #[test]
    fn allowed_public_api_endpoint() {
        assert!(!is_forbidden_url("https://api.stripe.com/v1/charges"));
        assert!(!is_forbidden_url("https://api.github.com/repos/anthropic/foo"));
    }

    #[test]
    fn allowed_public_172_outside_private_range() {
        // 172.15.x.x and 172.32.x.x are PUBLIC (private = 172.16-31).
        assert!(!is_forbidden_url("http://172.15.0.1"));
        assert!(!is_forbidden_url("http://172.32.0.1"));
    }

    #[test]
    fn allowed_169_non_link_local() {
        // 169.1.0.1 is not link-local (only 169.254.*.* is).
        assert!(!is_forbidden_url("http://169.1.0.1"));
    }

    // --- 11. Edge cases ---
    #[test]
    fn forbidden_empty_string() {
        // Empty string has no scheme; is_forbidden_url falls through -> false.
        // This is documented: non-http schemes are not rejected. The guard is
        // scoped to federated HTTP URLs only. Ensure at least no panic.
        let _ = is_forbidden_url("");
    }

    #[test]
    fn forbidden_empty_host_in_http_url() {
        // http:// with no host -> empty host -> treated as forbidden.
        assert!(is_forbidden_url("http://"));
        assert!(is_forbidden_url("https://"));
    }

    #[test]
    fn forbidden_malformed_url_no_scheme() {
        // No http(s) scheme -> not rejected (non-http schemes fall through).
        assert!(!is_forbidden_url("not a url"));
        assert!(!is_forbidden_url("garbage://////"));
    }

    #[test]
    fn forbidden_url_with_userinfo_loopback() {
        // Userinfo must be stripped before host check.
        assert!(is_forbidden_url("http://user:pass@127.0.0.1"));
        assert!(is_forbidden_url("http://admin@localhost:8080/"));
    }

    #[test]
    fn forbidden_url_with_userinfo_public_allowed() {
        // Userinfo stripped and real host is public -> allowed.
        assert!(!is_forbidden_url("http://user:pass@example.com"));
    }

    #[test]
    fn forbidden_url_trims_whitespace() {
        assert!(is_forbidden_url("  http://127.0.0.1  "));
        assert!(is_forbidden_url("\thttps://localhost\n"));
    }

    #[test]
    fn forbidden_url_is_case_insensitive_host() {
        assert!(is_forbidden_url("http://LOCALHOST"));
        assert!(is_forbidden_url("http://Printer.Local"));
    }

    // =====================================================================
    // Task #23 -- Metamodel namespace parser guard coverage.
    // =====================================================================
    //
    // The parser-level guard lives in `parse_markdown_with_context`. It
    // rejects user domains that redeclare a reserved metamodel noun when
    // that noun is already present in `existing_nouns` (meaning the
    // metamodel bootstrap has populated it). The bootstrap case (no
    // pre-existing nouns) is allowed to declare them exactly once.

    fn metamodel_nouns_map() -> HashMap<String, NounDef> {
        let mut m = HashMap::new();
        for n in METAMODEL_NOUNS {
            m.insert((*n).to_string(), NounDef {
                object_type: "entity".into(),
                world_assumption: WorldAssumption::default(),
            });
        }
        m
    }

    #[test]
    fn metamodel_guard_rejects_noun_redeclaration() {
        let existing = metamodel_nouns_map();
        let input = "# UserDomain\nNoun(.Name) is an entity type.";
        let err = parse_markdown_with_nouns(input, &existing).unwrap_err();
        assert!(err.contains("metamodel noun 'Noun' cannot be redeclared"),
            "expected rejection message for 'Noun', got: {}", err);
    }

    #[test]
    fn metamodel_guard_rejects_constraint_redeclaration() {
        let existing = metamodel_nouns_map();
        let input = "# UserDomain\nConstraint(.Id) is an entity type.";
        let err = parse_markdown_with_nouns(input, &existing).unwrap_err();
        assert!(err.contains("metamodel noun 'Constraint' cannot be redeclared"),
            "expected rejection message for 'Constraint', got: {}", err);
    }

    #[test]
    fn metamodel_guard_accepts_non_reserved_names() {
        let existing = metamodel_nouns_map();
        let input = "# Sales\n\
                     Order(.Id) is an entity type.\n\
                     Customer(.Name) is an entity type.";
        let ir = parse_markdown_with_nouns(input, &existing)
            .expect("non-reserved user domain should parse when metamodel is already present");
        assert!(ir.nouns.contains_key("Order"));
        assert!(ir.nouns.contains_key("Customer"));
        // Existing metamodel nouns remain visible in the merged IR.
        assert!(ir.nouns.contains_key("Noun"));
    }

    #[test]
    fn metamodel_guard_bootstrap_first_compile_succeeds() {
        // Bootstrap case: `existing_nouns` is empty, so the metamodel itself
        // is being compiled for the first time. Redeclaration guard must NOT
        // fire; the parse must succeed and populate the reserved nouns.
        let empty: HashMap<String, NounDef> = HashMap::new();
        let input = "# Metamodel\n\
                     Noun(.Name) is an entity type.\n\
                     Constraint(.Id) is an entity type.\n\
                     Role(.Id) is an entity type.";
        let ir = parse_markdown_with_nouns(input, &empty)
            .expect("bootstrap compile of metamodel nouns must succeed");
        assert!(ir.nouns.contains_key("Noun"));
        assert!(ir.nouns.contains_key("Constraint"));
        assert!(ir.nouns.contains_key("Role"));
    }

    #[test]
    fn metamodel_guard_allows_user_domain_not_touching_reserved_before_bootstrap() {
        // Before the bootstrap has run, `existing_nouns` is empty. A user
        // domain that only declares its own names must parse fine.
        let empty: HashMap<String, NounDef> = HashMap::new();
        let input = "# Sales\nOrder(.Id) is an entity type.";
        let ir = parse_markdown_with_nouns(input, &empty).unwrap();
        assert!(ir.nouns.contains_key("Order"));
    }

    // One test per reserved metamodel noun. Each verifies that redeclaring
    // that specific noun is rejected by the parser guard.

    fn assert_reserved_rejected(noun: &str, decl: &str) {
        let existing = metamodel_nouns_map();
        let input = format!("# UserDomain\n{}", decl);
        let err = parse_markdown_with_nouns(&input, &existing).unwrap_err();
        let needle = format!("metamodel noun '{}' cannot be redeclared", noun);
        assert!(err.contains(&needle),
            "expected rejection message '{}', got: {}", needle, err);
    }

    #[test]
    fn metamodel_guard_rejects_reserved_noun() {
        assert_reserved_rejected("Noun", "Noun(.Name) is an entity type.");
    }

    #[test]
    fn metamodel_guard_rejects_reserved_graph_schema() {
        assert_reserved_rejected("Graph Schema", "Graph Schema(.Id) is an entity type.");
    }

    #[test]
    fn metamodel_guard_rejects_reserved_role() {
        assert_reserved_rejected("Role", "Role(.Id) is an entity type.");
    }

    #[test]
    fn metamodel_guard_rejects_reserved_constraint() {
        assert_reserved_rejected("Constraint", "Constraint(.Id) is an entity type.");
    }

    #[test]
    fn metamodel_guard_rejects_reserved_state_machine_definition() {
        assert_reserved_rejected(
            "State Machine Definition",
            "State Machine Definition(.Id) is an entity type.",
        );
    }

    #[test]
    fn metamodel_guard_rejects_reserved_transition() {
        assert_reserved_rejected("Transition", "Transition(.Id) is an entity type.");
    }

    #[test]
    fn metamodel_guard_rejects_reserved_status() {
        assert_reserved_rejected("Status", "Status(.Id) is an entity type.");
    }

    #[test]
    fn metamodel_guard_rejects_reserved_event_type() {
        assert_reserved_rejected("Event Type", "Event Type(.Id) is an entity type.");
    }

    #[test]
    fn metamodel_guard_rejects_reserved_domain_change() {
        assert_reserved_rejected("Domain Change", "Domain Change(.Id) is an entity type.");
    }
}
