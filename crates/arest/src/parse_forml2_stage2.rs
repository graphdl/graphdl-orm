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

/// Translate noun-shaping classifications into `Noun` cell facts.
/// #280b step 1.
///
/// Considers every Statement that carries a Head Noun plus one of
/// these classifications:
///
/// - `Entity Type Declaration` → objectType = "entity".
/// - `Value Type Declaration`  → objectType = "value".
/// - `Abstract Declaration`    → objectType = "abstract" (overrides
///   entity/value per the existing parser: `Foo is abstract` on a
///   line after `Foo is an entity type` wins).
///
/// Grouped by Head Noun: one Noun fact per distinct name, with the
/// most specific objectType across its classifications applied.
#[cfg(feature = "std-deps")]
pub fn translate_nouns(classified_state: &Object) -> Vec<Object> {
    use alloc::collections::BTreeMap;
    let statement_ids = collect_statement_ids(classified_state);
    let mut by_noun: BTreeMap<String, &'static str> = BTreeMap::new();
    for stmt_id in &statement_ids {
        let Some(head) = head_noun_for(classified_state, stmt_id) else { continue };
        let classifications = classifications_for(classified_state, stmt_id);
        let ot = if classifications.iter().any(|k| k == "Abstract Declaration") {
            Some("abstract")
        } else if classifications.iter().any(|k| k == "Entity Type Declaration") {
            Some("entity")
        } else if classifications.iter().any(|k| k == "Value Type Declaration") {
            Some("value")
        } else {
            None
        };
        if let Some(new_ot) = ot {
            let slot = by_noun.entry(head).or_insert(new_ot);
            // Abstract wins over entity/value; otherwise keep existing.
            if new_ot == "abstract" {
                *slot = "abstract";
            }
        }
    }
    by_noun.into_iter().map(|(name, ot)| {
        fact_from_pairs(&[
            ("name", name.as_str()),
            ("objectType", ot),
            ("worldAssumption", "closed"),
        ])
    }).collect()
}

/// Translate `Subtype Declaration` classifications into `Subtype` cell
/// facts: `(subtype, supertype)` pairs. The subtype is the Statement's
/// Head Noun; the supertype is the noun at Role Position 1 (the only
/// other role reference in `A is a subtype of B`).
///
/// Partition Declaration is NOT handled here — it needs multi-role
/// extraction (`A is partitioned into B, C, D`) and will land in a
/// later commit.
#[cfg(feature = "std-deps")]
pub fn translate_subtypes(classified_state: &Object) -> Vec<Object> {
    let statement_ids = collect_statement_ids(classified_state);
    statement_ids.iter().filter_map(|stmt_id| {
        let classifications = classifications_for(classified_state, stmt_id);
        if !classifications.iter().any(|k| k == "Subtype Declaration") {
            return None;
        }
        let sub = head_noun_for(classified_state, stmt_id)?;
        let sup = role_noun_at_position(classified_state, stmt_id, 1)?;
        Some(fact_from_pairs(&[
            ("subtype", sub.as_str()),
            ("supertype", sup.as_str()),
        ]))
    }).collect()
}

/// Translate `Fact Type Reading` classifications into `FactType` +
/// `Role` cell facts. Returns `(fact_type_facts, role_facts)`.
///
/// Exclusions: Statements whose Fact Type Reading classification is
/// an artifact of declaring a noun (Entity Type / Value Type /
/// Subtype / Abstract / Enum Values Declaration) or asserting an
/// instance (Instance Fact) are NOT emitted as fact types. The
/// current FORML 2 corpus relies on this separation — the noun-
/// declaration shape `Customer is an entity type` also matches Fact
/// Type Reading because it has a Role Reference.
#[cfg(feature = "std-deps")]
pub fn translate_fact_types(classified_state: &Object) -> (Vec<Object>, Vec<Object>) {
    let statement_ids = collect_statement_ids(classified_state);
    let mut ft_facts: Vec<Object> = Vec::new();
    let mut role_facts: Vec<Object> = Vec::new();
    for stmt_id in &statement_ids {
        let classifications = classifications_for(classified_state, stmt_id);
        if !classifications.iter().any(|k| k == "Fact Type Reading") {
            continue;
        }
        // Exclude declarative shapes that incidentally match.
        const EXCLUDE: &[&str] = &[
            "Entity Type Declaration",
            "Value Type Declaration",
            "Subtype Declaration",
            "Abstract Declaration",
            "Enum Values Declaration",
            "Instance Fact",
            "Partition Declaration",
        ];
        if classifications.iter().any(|k| EXCLUDE.iter().any(|e| e == k)) {
            continue;
        }
        let roles = role_refs_for(classified_state, stmt_id);
        let Some(text) = statement_text(classified_state, stmt_id) else { continue };
        let reading = text;
        let id = reading.replace(' ', "_");
        ft_facts.push(fact_from_pairs(&[
            ("id", id.as_str()),
            ("reading", reading.as_str()),
            ("arity", &roles.len().to_string()),
        ]));
        for (i, noun_name) in roles.iter().enumerate() {
            role_facts.push(fact_from_pairs(&[
                ("factType", id.as_str()),
                ("nounName", noun_name.as_str()),
                ("position", &i.to_string()),
            ]));
        }
    }
    (ft_facts, role_facts)
}

/// Role head nouns for a Statement, ordered by Role Position.
fn role_refs_for(state: &Object, stmt_id: &str) -> Vec<String> {
    let refs = fetch_or_phi("Statement_has_Role_Reference", state);
    let Some(refs_seq) = refs.as_seq() else { return Vec::new() };
    let role_ids: Vec<String> = refs_seq.iter()
        .filter(|f| binding(f, "Statement") == Some(stmt_id))
        .filter_map(|f| binding(f, "Role_Reference").map(String::from))
        .collect();
    let positions = fetch_or_phi("Role_Reference_has_Role_Position", state);
    let pos_seq = positions.as_seq();
    let head_nouns = fetch_or_phi("Role_Reference_has_Head_Noun", state);
    let hn_seq = head_nouns.as_seq();
    let mut with_pos: Vec<(usize, String)> = role_ids.iter().filter_map(|id| {
        let pos_s = pos_seq.as_ref()?.iter()
            .find(|f| binding(f, "Role_Reference") == Some(id.as_str()))
            .and_then(|f| binding(f, "Role_Position").map(String::from))?;
        let pos: usize = pos_s.parse().ok()?;
        let noun = hn_seq.as_ref()?.iter()
            .find(|f| binding(f, "Role_Reference") == Some(id.as_str()))
            .and_then(|f| binding(f, "Head_Noun").map(String::from))?;
        Some((pos, noun))
    }).collect();
    with_pos.sort_by_key(|(p, _)| *p);
    with_pos.into_iter().map(|(_, n)| n).collect()
}

fn statement_text(state: &Object, stmt_id: &str) -> Option<String> {
    fetch_or_phi("Statement_has_Text", state)
        .as_seq()?
        .iter()
        .find(|f| binding(f, "Statement") == Some(stmt_id))
        .and_then(|f| binding(f, "Text").map(String::from))
}

/// Translate `Instance Fact` classifications into `InstanceFact` cell
/// facts. Binary instance-fact shape (subject + field + object):
///
///   subjectNoun = role 0's head noun
///   subjectValue = role 0's literal
///   fieldName = Statement's Verb token
///   objectNoun = role 1's head noun (if present)
///   objectValue = role 1's literal (if present)
///
/// Unary instance-facts (value assertions like `Customer 'alice' is
/// active`) currently emit with empty objectNoun/objectValue.
#[cfg(feature = "std-deps")]
pub fn translate_instance_facts(classified_state: &Object) -> Vec<Object> {
    let statement_ids = collect_statement_ids(classified_state);
    let mut out: Vec<Object> = Vec::new();
    for stmt_id in &statement_ids {
        let classifications = classifications_for(classified_state, stmt_id);
        if !classifications.iter().any(|k| k == "Instance Fact") {
            continue;
        }
        let roles = role_refs_with_literals(classified_state, stmt_id);
        if roles.is_empty() { continue; }
        let verb = statement_verb(classified_state, stmt_id).unwrap_or_default();
        let subject_noun = &roles[0].0;
        let subject_value = roles[0].1.as_deref().unwrap_or("");
        let (object_noun, object_value) = roles.get(1)
            .map(|(n, lit)| (n.as_str(), lit.as_deref().unwrap_or("")))
            .unwrap_or(("", ""));
        out.push(fact_from_pairs(&[
            ("subjectNoun",  subject_noun.as_str()),
            ("subjectValue", subject_value),
            ("fieldName",    verb.as_str()),
            ("objectNoun",   object_noun),
            ("objectValue",  object_value),
        ]));
    }
    out
}

/// Role head nouns AND literal values for a Statement, ordered by
/// Role Position. Returns `Vec<(noun, Option<literal>)>`.
fn role_refs_with_literals(state: &Object, stmt_id: &str) -> Vec<(String, Option<String>)> {
    let refs = fetch_or_phi("Statement_has_Role_Reference", state);
    let Some(refs_seq) = refs.as_seq() else { return Vec::new() };
    let role_ids: Vec<String> = refs_seq.iter()
        .filter(|f| binding(f, "Statement") == Some(stmt_id))
        .filter_map(|f| binding(f, "Role_Reference").map(String::from))
        .collect();
    let positions = fetch_or_phi("Role_Reference_has_Role_Position", state);
    let pos_seq = positions.as_seq();
    let head_nouns = fetch_or_phi("Role_Reference_has_Head_Noun", state);
    let hn_seq = head_nouns.as_seq();
    let literals = fetch_or_phi("Role_Reference_has_Literal_Value", state);
    let lit_seq = literals.as_seq();
    let mut with_pos: Vec<(usize, String, Option<String>)> = role_ids.iter().filter_map(|id| {
        let pos_s = pos_seq.as_ref()?.iter()
            .find(|f| binding(f, "Role_Reference") == Some(id.as_str()))
            .and_then(|f| binding(f, "Role_Position").map(String::from))?;
        let pos: usize = pos_s.parse().ok()?;
        let noun = hn_seq.as_ref()?.iter()
            .find(|f| binding(f, "Role_Reference") == Some(id.as_str()))
            .and_then(|f| binding(f, "Head_Noun").map(String::from))?;
        let literal = lit_seq.as_ref()
            .and_then(|s| s.iter()
                .find(|f| binding(f, "Role_Reference") == Some(id.as_str()))
                .and_then(|f| binding(f, "Literal_Value").map(String::from)));
        Some((pos, noun, literal))
    }).collect();
    with_pos.sort_by_key(|(p, _, _)| *p);
    with_pos.into_iter().map(|(_, n, l)| (n, l)).collect()
}

fn statement_verb(state: &Object, stmt_id: &str) -> Option<String> {
    fetch_or_phi("Statement_has_Verb", state)
        .as_seq()?
        .iter()
        .find(|f| binding(f, "Statement") == Some(stmt_id))
        .and_then(|f| binding(f, "Verb").map(String::from))
}

fn role_noun_at_position(state: &Object, stmt_id: &str, position: usize) -> Option<String> {
    let refs = fetch_or_phi("Statement_has_Role_Reference", state);
    let refs_seq = refs.as_seq()?;
    let role_ids: Vec<String> = refs_seq.iter()
        .filter(|f| binding(f, "Statement") == Some(stmt_id))
        .filter_map(|f| binding(f, "Role_Reference").map(String::from))
        .collect();
    let positions = fetch_or_phi("Role_Reference_has_Role_Position", state);
    let pos_seq = positions.as_seq()?;
    let head_nouns = fetch_or_phi("Role_Reference_has_Head_Noun", state);
    let hn_seq = head_nouns.as_seq()?;
    // Find the role_id at the requested position.
    let target_id = role_ids.iter().find(|id| {
        pos_seq.iter().any(|f| {
            binding(f, "Role_Reference") == Some(id.as_str())
                && binding(f, "Role_Position") == Some(&position.to_string())
        })
    })?;
    hn_seq.iter()
        .find(|f| binding(f, "Role_Reference") == Some(target_id.as_str()))
        .and_then(|f| binding(f, "Head_Noun").map(String::from))
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
    fn translate_nouns_abstract_wins_over_entity() {
        // Simulate two Statements on the same Head Noun: one Entity
        // Type Declaration + one Abstract Declaration. The merged
        // Noun fact must have objectType="abstract".
        let mut merged: HashMap<String, Object> = HashMap::new();
        for (i, (text, nouns)) in [
            ("Request is an entity type.", vec!["Request"]),
            ("Request is abstract.",       vec!["Request"]),
        ].into_iter().enumerate() {
            let stmt_id = format!("s{}", i);
            let cells = tokenize_statement(
                &stmt_id, text,
                &nouns.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            );
            for (name, facts) in cells {
                let entry = merged.entry(name).or_insert_with(|| Object::Seq(Vec::new().into()));
                let existing = entry.as_seq().map(|s| s.to_vec()).unwrap_or_default();
                let mut combined = existing;
                combined.extend(facts);
                *entry = Object::Seq(combined.into());
            }
        }
        let stmt = Object::Map(merged);
        let classified = classify_statements(&stmt, &grammar_state());
        let noun_facts = super::translate_nouns(&classified);
        assert_eq!(noun_facts.len(), 1);
        assert_eq!(binding(&noun_facts[0], "name"), Some("Request"));
        assert_eq!(binding(&noun_facts[0], "objectType"), Some("abstract"));
    }

    #[test]
    fn translate_subtypes_emits_subtype_fact() {
        let stmt = stage1_state(
            "s1", "Support Request is a subtype of Request.",
            &["Support Request", "Request"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let subtype_facts = super::translate_subtypes(&classified);
        assert_eq!(subtype_facts.len(), 1);
        assert_eq!(binding(&subtype_facts[0], "subtype"), Some("Support Request"));
        assert_eq!(binding(&subtype_facts[0], "supertype"), Some("Request"));
    }

    #[test]
    fn translate_subtypes_skips_non_subtype_statements() {
        let stmt = stage1_state(
            "s1", "Customer is an entity type.", &["Customer"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let subtype_facts = super::translate_subtypes(&classified);
        assert!(subtype_facts.is_empty());
    }

    #[test]
    fn translate_fact_types_emits_ft_and_role_facts_for_binary() {
        let stmt = stage1_state(
            "s1", "Customer places Order.", &["Customer", "Order"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let (ft, roles) = super::translate_fact_types(&classified);
        assert_eq!(ft.len(), 1);
        assert_eq!(binding(&ft[0], "id"), Some("Customer_places_Order"));
        assert_eq!(binding(&ft[0], "reading"), Some("Customer places Order"));
        assert_eq!(binding(&ft[0], "arity"), Some("2"));
        assert_eq!(roles.len(), 2);
        let positions: Vec<String> = roles.iter()
            .filter_map(|r| Some(format!("{}@{}",
                binding(r, "nounName")?,
                binding(r, "position")?)))
            .collect();
        assert!(positions.contains(&"Customer@0".to_string()), "got {:?}", positions);
        assert!(positions.contains(&"Order@1".to_string()), "got {:?}", positions);
    }

    #[test]
    fn translate_fact_types_skips_entity_type_declaration() {
        // `Customer is an entity type` matches Fact Type Reading
        // (has a Role Reference) but is excluded from FT emission.
        let stmt = stage1_state(
            "s1", "Customer is an entity type.", &["Customer"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let (ft, roles) = super::translate_fact_types(&classified);
        assert!(ft.is_empty(), "got FT facts: {:?}", ft);
        assert!(roles.is_empty());
    }

    #[test]
    fn translate_fact_types_skips_subtype_declaration() {
        let stmt = stage1_state(
            "s1", "Support Request is a subtype of Request.",
            &["Support Request", "Request"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let (ft, _) = super::translate_fact_types(&classified);
        assert!(ft.is_empty());
    }

    #[test]
    fn instance_fact_is_classified() {
        let stmt = stage1_state(
            "s1", "Customer 'alice' places Order 'o-7'.",
            &["Customer", "Order"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&classified, "s1");
        assert!(kinds.iter().any(|k| k == "Instance Fact"),
            "expected Instance Fact; got {:?}", kinds);
    }

    #[test]
    fn translate_instance_facts_emits_subject_field_object() {
        let stmt = stage1_state(
            "s1", "Customer 'alice' places Order 'o-7'.",
            &["Customer", "Order"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let facts = super::translate_instance_facts(&classified);
        assert_eq!(facts.len(), 1);
        let f = &facts[0];
        assert_eq!(binding(f, "subjectNoun"),  Some("Customer"));
        assert_eq!(binding(f, "subjectValue"), Some("alice"));
        assert_eq!(binding(f, "fieldName"),    Some("places"));
        assert_eq!(binding(f, "objectNoun"),   Some("Order"));
        assert_eq!(binding(f, "objectValue"),  Some("o-7"));
    }

    #[test]
    fn translate_instance_facts_skips_non_instance_statements() {
        let stmt = stage1_state(
            "s1", "Customer places Order.", &["Customer", "Order"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let facts = super::translate_instance_facts(&classified);
        assert!(facts.is_empty(), "got {:?}", facts);
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
