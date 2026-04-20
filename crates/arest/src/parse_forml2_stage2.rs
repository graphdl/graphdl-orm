//! Stage-2 applier: Statement cells → Classification cells via grammar rules.
//!
//! #280 meta-circular parser. Stage-2 consumes:
//!
//!   (a) a state populated with `Statement_*` cells from Stage-1
//!       (`parse_forml2_stage1::tokenize_statement`), and
//!   (b) the grammar state from parsing `readings/forml2-grammar.md`,
//!
//! and applies the grammar's derivation rules to emit
//! `Statement has Classification` facts — one per recognized statement
//! kind per Statement.
//!
//! The grammar uses a small, fixed rule shape:
//!
//!   Statement has Classification '<Kind>' iff <conjuncts>
//!
//! where each conjunct is either `Statement has <Token>` (existential —
//! the cell has any fact for this statement) or `Statement has <Token>
//! '<value>'` (literal — the cell has a fact whose <Token> binding
//! equals '<value>'). Token names use underscores internally
//! (`Trailing_Marker`) but appear with spaces in the rule text
//! (`Trailing Marker`).
//!
//! Stage-2 is a focused interpreter for that shape. It does NOT pass
//! through the full compile-to-defs pipeline because the general
//! compiler does not yet support literal-in-consequent derivations
//! (emitting `Classification '<Kind>'` with the literal fixed).
//!
//! Translation from classification to canonical metamodel cells
//! (Noun, Fact Type, Role, …) is the per-kind #280b commits.

extern crate alloc;
use alloc::{string::{String, ToString}, vec::Vec, format};
use hashbrown::HashMap;
use crate::ast::{Object, fetch_or_phi, fact_from_pairs, binding};

/// Classify every Statement in `statements_state` using the grammar
/// rules in `grammar_state`. Returns a new state identical to
/// `statements_state` plus a populated `Statement_has_Classification`
/// cell.
#[cfg(feature = "std-deps")]
pub fn classify_statements(statements_state: &Object, grammar_state: &Object) -> Object {
    let rules = parse_grammar_rules(grammar_state);
    let statement_ids = collect_statement_ids(statements_state);

    let mut classifications: Vec<Object> = Vec::new();
    for stmt_id in &statement_ids {
        for rule in &rules {
            if rule_matches(statements_state, stmt_id, rule) {
                classifications.push(fact_from_pairs(&[
                    ("Statement", stmt_id.as_str()),
                    ("Classification", rule.classification.as_str()),
                ]));
            }
        }
    }

    // Reconstitute state with classifications cell overlaid.
    let mut cells: HashMap<String, Object> = HashMap::new();
    for (name, contents) in crate::ast::cells_iter(statements_state) {
        cells.insert(name.to_string(), contents.clone());
    }
    cells.insert("Statement_has_Classification".to_string(),
                 Object::Seq(classifications.into()));
    Object::Map(cells)
}

/// Translate `Statement has Classification 'Entity Type Declaration'`
/// and `Statement has Classification 'Value Type Declaration'` facts
/// into `Noun` cell facts. #280b step 1.
///
/// The result is a `Vec<Object>` of Noun facts, one per distinct
/// declared Head Noun, with `objectType` set from the classification.
/// The caller merges these into the Noun cell.
#[cfg(feature = "std-deps")]
pub fn translate_nouns(classified_state: &Object) -> Vec<Object> {
    let statement_ids = collect_statement_ids(classified_state);
    statement_ids.iter().filter_map(|stmt_id| {
        let head = head_noun_for(classified_state, stmt_id)?;
        let classifications = classifications_for(classified_state, stmt_id);
        let object_type = if classifications.iter().any(|k| k == "Entity Type Declaration") {
            "entity"
        } else if classifications.iter().any(|k| k == "Value Type Declaration") {
            "value"
        } else {
            return None;
        };
        Some(fact_from_pairs(&[
            ("name", head.as_str()),
            ("objectType", object_type),
            ("worldAssumption", "closed"),
        ]))
    }).collect()
}

fn head_noun_for(state: &Object, stmt_id: &str) -> Option<String> {
    fetch_or_phi("Statement_has_Head_Noun", state)
        .as_seq()?
        .iter()
        .find(|f| binding(f, "Statement") == Some(stmt_id))
        .and_then(|f| binding(f, "Head_Noun").map(String::from))
}

/// Return the list of classification names attached to a given
/// Statement id.
#[cfg(feature = "std-deps")]
pub fn classifications_for(state: &Object, statement_id: &str) -> Vec<String> {
    fetch_or_phi("Statement_has_Classification", state)
        .as_seq()
        .map(|facts| facts.iter()
            .filter(|f| binding(f, "Statement") == Some(statement_id))
            .filter_map(|f| binding(f, "Classification").map(String::from))
            .collect())
        .unwrap_or_default()
}

/// A single grammar rule in Stage-2's reduced form.
#[derive(Debug, Clone)]
struct GrammarRule {
    /// The classification this rule emits (e.g. "Entity Type Declaration").
    classification: String,
    /// Conjuncts of the antecedent. All must hold.
    conjuncts: Vec<Conjunct>,
}

#[derive(Debug, Clone)]
enum Conjunct {
    /// `Statement has <Token> '<value>'` — the statement's cell
    /// must have a fact whose <Token> binding equals <value>.
    TokenLiteral { token: String, value: String },
    /// `Statement has <Token>` — the statement's cell must have
    /// any fact (existential).
    TokenExists { token: String },
    /// `Statement has Classification '<Kind>'` — another
    /// classification must hold for the same statement
    /// (e.g. Value Constraint iff Enum Values Declaration).
    HasClassification { kind: String },
}

/// Parse the grammar state's `DerivationRule` cell into our reduced
/// rule form. Rules that don't fit the Stage-2 shape are skipped.
#[cfg(feature = "std-deps")]
fn parse_grammar_rules(grammar_state: &Object) -> Vec<GrammarRule> {
    fetch_or_phi("DerivationRule", grammar_state)
        .as_seq()
        .map(|facts| facts.iter()
            .filter_map(|f| binding(f, "text").map(String::from))
            .filter_map(|text| parse_rule_text(&text))
            .collect())
        .unwrap_or_default()
}

/// Parse a single rule's text: `Statement has Classification '<Kind>'
/// iff <conjuncts>`. Returns None if the text doesn't fit.
fn parse_rule_text(text: &str) -> Option<GrammarRule> {
    let (head, body) = text.split_once(" iff ")?;
    let classification = head
        .strip_prefix("Statement has Classification '")?
        .strip_suffix("'")?
        .to_string();
    let conjuncts: Vec<Conjunct> = body
        .split(" and ")
        .filter_map(|c| parse_conjunct(c.trim()))
        .collect();
    if conjuncts.is_empty() {
        return None;
    }
    Some(GrammarRule { classification, conjuncts })
}

fn parse_conjunct(text: &str) -> Option<Conjunct> {
    let rest = text.strip_prefix("Statement has ")?;
    // Strip trailing period if present (rules come from the derivation
    // text which should already be trimmed, but be defensive).
    let rest = rest.trim_end_matches('.');
    if let Some(kind_lit) = rest.strip_prefix("Classification '").and_then(|s| s.strip_suffix("'")) {
        return Some(Conjunct::HasClassification { kind: kind_lit.to_string() });
    }
    // `<Token> '<value>'`
    if let Some(q_start) = rest.find(" '") {
        let token = rest[..q_start].to_string();
        let value = rest[q_start + 2..].trim_end_matches('\'').to_string();
        return Some(Conjunct::TokenLiteral {
            token: token.replace(' ', "_"),
            value,
        });
    }
    // `<Token>` existential
    Some(Conjunct::TokenExists { token: rest.replace(' ', "_") })
}

fn collect_statement_ids(state: &Object) -> Vec<String> {
    fetch_or_phi("Statement", state)
        .as_seq()
        .map(|facts| facts.iter()
            .filter_map(|f| binding(f, "id").map(String::from))
            .collect())
        .unwrap_or_default()
}

fn rule_matches(state: &Object, stmt_id: &str, rule: &GrammarRule) -> bool {
    rule.conjuncts.iter().all(|c| conjunct_matches(state, stmt_id, c))
}

fn conjunct_matches(state: &Object, stmt_id: &str, c: &Conjunct) -> bool {
    match c {
        Conjunct::TokenLiteral { token, value } => {
            let cell_name = format!("Statement_has_{}", token);
            cell_has(state, &cell_name, stmt_id, Some((token.as_str(), value.as_str())))
        }
        Conjunct::TokenExists { token } => {
            let cell_name = format!("Statement_has_{}", token);
            cell_has(state, &cell_name, stmt_id, None)
        }
        Conjunct::HasClassification { kind } => {
            cell_has(state, "Statement_has_Classification", stmt_id,
                     Some(("Classification", kind.as_str())))
        }
    }
}

fn cell_has(state: &Object, cell_name: &str, stmt_id: &str,
            key_value: Option<(&str, &str)>) -> bool {
    fetch_or_phi(cell_name, state)
        .as_seq()
        .map(|facts| facts.iter().any(|f| {
            binding(f, "Statement") == Some(stmt_id)
                && match key_value {
                    None => true,
                    Some((k, v)) => binding(f, k) == Some(v),
                }
        }))
        .unwrap_or(false)
}

#[cfg(all(test, feature = "std-deps"))]
mod tests {
    use super::*;
    use crate::parse_forml2::parse_to_state;
    use crate::parse_forml2_stage1::tokenize_statement;

    fn grammar_state() -> Object {
        let grammar = include_str!("../../../readings/forml2-grammar.md");
        parse_to_state(grammar).expect("grammar must parse")
    }

    fn stage1_state(statement_id: &str, text: &str, nouns: &[&str]) -> Object {
        let cells = tokenize_statement(
            statement_id, text,
            &nouns.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        );
        let map: HashMap<String, Object> = cells.into_iter()
            .map(|(k, v)| (k, Object::Seq(v.into())))
            .collect();
        Object::Map(map)
    }

    #[test]
    fn entity_type_declaration_is_classified() {
        let stmt = stage1_state(
            "s1", "Customer is an entity type.", &["Customer"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&classified, "s1");
        assert!(kinds.iter().any(|k| k == "Entity Type Declaration"),
            "expected Entity Type Declaration classification; got {:?}", kinds);
    }

    #[test]
    fn value_type_declaration_is_classified() {
        let stmt = stage1_state(
            "s1", "Priority is a value type.", &["Priority"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&classified, "s1");
        assert!(kinds.iter().any(|k| k == "Value Type Declaration"),
            "expected Value Type Declaration classification; got {:?}", kinds);
    }

    #[test]
    fn abstract_declaration_is_classified() {
        let stmt = stage1_state("s1", "Request is abstract.", &["Request"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&classified, "s1");
        assert!(kinds.iter().any(|k| k == "Abstract Declaration"),
            "expected Abstract Declaration; got {:?}", kinds);
    }

    #[test]
    fn ring_constraint_is_classified_per_adjective() {
        let cases: &[(&str, &[&str])] = &[
            ("Category has parent Category is acyclic.",  &["Category"]),
            ("Person is parent of Person is irreflexive.", &["Person"]),
            ("Person loves Person is symmetric.",          &["Person"]),
        ];
        for (text, nouns) in cases {
            let stmt = stage1_state("s1", text, nouns);
            let classified = classify_statements(&stmt, &grammar_state());
            let kinds = classifications_for(&classified, "s1");
            assert!(kinds.iter().any(|k| k == "Ring Constraint"),
                "expected Ring Constraint for {:?}; got {:?}", text, kinds);
        }
    }

    #[test]
    fn subtype_declaration_is_classified() {
        let stmt = stage1_state(
            "s1", "Support Request is a subtype of Request.",
            &["Support Request", "Request"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&classified, "s1");
        assert!(kinds.iter().any(|k| k == "Subtype Declaration"),
            "expected Subtype Declaration; got {:?}", kinds);
    }

    #[test]
    fn fact_type_reading_classified_from_existential_role_reference() {
        let stmt = stage1_state(
            "s1", "Customer places Order.", &["Customer", "Order"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&classified, "s1");
        assert!(kinds.iter().any(|k| k == "Fact Type Reading"),
            "expected Fact Type Reading; got {:?}", kinds);
    }

    #[test]
    fn translate_nouns_emits_entity_type_fact() {
        let stmt = stage1_state(
            "s1", "Customer is an entity type.", &["Customer"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let noun_facts = super::translate_nouns(&classified);
        assert_eq!(noun_facts.len(), 1);
        assert_eq!(binding(&noun_facts[0], "name"), Some("Customer"));
        assert_eq!(binding(&noun_facts[0], "objectType"), Some("entity"));
    }

    #[test]
    fn translate_nouns_emits_value_type_fact() {
        let stmt = stage1_state(
            "s1", "Priority is a value type.", &["Priority"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let noun_facts = super::translate_nouns(&classified);
        assert_eq!(noun_facts.len(), 1);
        assert_eq!(binding(&noun_facts[0], "name"), Some("Priority"));
        assert_eq!(binding(&noun_facts[0], "objectType"), Some("value"));
    }

    #[test]
    fn translate_nouns_skips_fact_type_reading_statements() {
        // Fact type readings have Head Noun but no entity/value declaration.
        let stmt = stage1_state(
            "s1", "Customer places Order.", &["Customer", "Order"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let noun_facts = super::translate_nouns(&classified);
        assert!(noun_facts.is_empty(),
            "fact-type readings must not produce Noun facts; got {:?}", noun_facts);
    }

    #[test]
    fn translate_nouns_handles_multiple_statements() {
        // Run each declaration through its own Stage-1 pass, then merge
        // the cells before classify — a tiny end-to-end check.
        let mut merged_cells: HashMap<String, Object> = HashMap::new();
        for (i, (text, nouns)) in [
            ("Customer is an entity type.", vec!["Customer"]),
            ("Priority is a value type.", vec!["Priority"]),
        ].into_iter().enumerate() {
            let stmt_id = format!("s{}", i);
            let cells = tokenize_statement(
                &stmt_id, text,
                &nouns.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            );
            for (name, facts) in cells {
                let entry = merged_cells.entry(name).or_insert_with(|| Object::Seq(Vec::new().into()));
                let existing = entry.as_seq().map(|s| s.to_vec()).unwrap_or_default();
                let mut combined = existing;
                combined.extend(facts);
                *entry = Object::Seq(combined.into());
            }
        }
        let stmt = Object::Map(merged_cells);
        let classified = classify_statements(&stmt, &grammar_state());
        let noun_facts = super::translate_nouns(&classified);
        assert_eq!(noun_facts.len(), 2);
        let by_name: HashMap<String, String> = noun_facts.iter()
            .filter_map(|f| {
                let name = binding(f, "name")?.to_string();
                let ot = binding(f, "objectType")?.to_string();
                Some((name, ot))
            })
            .collect();
        assert_eq!(by_name.get("Customer").map(String::as_str), Some("entity"));
        assert_eq!(by_name.get("Priority").map(String::as_str), Some("value"));
    }

    #[test]
    fn rule_parser_handles_literal_body() {
        let rule = parse_rule_text(
            "Statement has Classification 'Entity Type Declaration' iff Statement has Trailing Marker 'is an entity type'"
        ).unwrap();
        assert_eq!(rule.classification, "Entity Type Declaration");
        assert_eq!(rule.conjuncts.len(), 1);
        match &rule.conjuncts[0] {
            Conjunct::TokenLiteral { token, value } => {
                assert_eq!(token, "Trailing_Marker");
                assert_eq!(value, "is an entity type");
            }
            c => panic!("expected TokenLiteral conjunct, got {:?}", c),
        }
    }

    #[test]
    fn rule_parser_handles_existential_body() {
        let rule = parse_rule_text(
            "Statement has Classification 'Fact Type Reading' iff Statement has Role Reference"
        ).unwrap();
        assert_eq!(rule.classification, "Fact Type Reading");
        match &rule.conjuncts[0] {
            Conjunct::TokenExists { token } => {
                assert_eq!(token, "Role_Reference");
            }
            c => panic!("expected TokenExists conjunct, got {:?}", c),
        }
    }

    #[test]
    fn rule_parser_handles_classification_body() {
        let rule = parse_rule_text(
            "Statement has Classification 'Value Constraint' iff Statement has Classification 'Enum Values Declaration'"
        ).unwrap();
        assert_eq!(rule.classification, "Value Constraint");
        match &rule.conjuncts[0] {
            Conjunct::HasClassification { kind } => {
                assert_eq!(kind, "Enum Values Declaration");
            }
            c => panic!("expected HasClassification conjunct, got {:?}", c),
        }
    }
}
