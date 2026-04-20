//! Stage-2 applier: Statement cells → Classification cells via grammar rules.
//!
//! #280 meta-circular parser. Stage-2 consumes:
//!
//!   (a) a state populated with `Statement_*` cells from Stage-1
//!       (`parse_forml2_stage1::tokenize_statement`), and
//!   (b) the grammar state from parsing `readings/forml2-grammar.md`,
//!
//! and applies the grammar's derivation rules (compiled through the
//! standard `compile_to_defs_state` + `forward_chain_defs_state`
//! pipeline) to emit `Statement has Classification` facts — one per
//! recognized statement kind per Statement.
//!
//! The grammar uses a small, fixed rule shape:
//!
//!   Statement has Classification '<Kind>' iff Statement has <Token>
//!     ['<value>']
//!
//! Literal values on consequent and antecedent roles flow through
//! DerivationRuleDef::consequent_role_literals and
//! DerivationRuleDef::antecedent_role_literals (#286). Stage-2 no
//! longer has a focused interpreter for this shape; it just merges
//! grammar + statements, compiles, forward-chains, and returns the
//! enriched state.
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
    // Merge Stage-1 statement cells with grammar cells so
    // `compile_to_defs_state` sees both the nouns/fact-types/rules
    // declared by the grammar and the Statement facts they apply to.
    let merged = crate::ast::merge_states(statements_state, grammar_state);
    let defs = crate::compile::compile_to_defs_state(&merged);
    let with_defs = crate::ast::defs_to_state(&defs, &merged);
    let deriv_funcs: Vec<(&str, &crate::ast::Func)> = defs.iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, f)| (n.as_str(), f))
        .collect();
    let (final_state, _) =
        crate::evaluate::forward_chain_defs_state(&deriv_funcs, &with_defs);
    final_state
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
        let ot = if classifications.iter().any(|k|
            k == "Abstract Declaration" || k == "Partition Declaration")
        {
            // Partition Declaration marks the supertype abstract
            // (ORM 2: a partitioned type has no direct instances;
            // every instance is in exactly one subtype).
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

/// Translate `Partition Declaration` classifications into `Subtype`
/// cell facts — one `(subtype, supertype)` pair per subtype in the
/// comma-separated list. Shape: `A is partitioned into B, C, D` →
/// (B, A), (C, A), (D, A). The supertype's abstractness flows
/// through `translate_nouns` which treats Partition Declaration as
/// an abstract-marking classification.
#[cfg(feature = "std-deps")]
pub fn translate_partitions(classified_state: &Object) -> Vec<Object> {
    let statement_ids = collect_statement_ids(classified_state);
    let mut out: Vec<Object> = Vec::new();
    for stmt_id in &statement_ids {
        let classifications = classifications_for(classified_state, stmt_id);
        if !classifications.iter().any(|k| k == "Partition Declaration") {
            continue;
        }
        let Some(sup) = head_noun_for(classified_state, stmt_id) else { continue };
        let roles = role_refs_for(classified_state, stmt_id);
        for sub in roles.iter().skip(1) {
            out.push(fact_from_pairs(&[
                ("subtype", sub.as_str()),
                ("supertype", sup.as_str()),
            ]));
        }
    }
    out
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

/// Translate `Ring Constraint` classifications into `Constraint` cell
/// facts. Each ring adjective maps to a two-letter ORM 2 kind code:
///
///   is irreflexive   → IR
///   is asymmetric    → AS
///   is antisymmetric → AT
///   is symmetric     → SY
///   is intransitive  → IT
///   is transitive    → TR
///   is acyclic       → AC
///   is reflexive     → RF
///
/// The Constraint fact carries `kind`, `modality="alethic"`,
/// `text` (Statement text), and `entity` (Head Noun). Spans
/// (fact_type_id resolution) are left empty — a follow-up
/// commit will populate them once the FactType cell exists.
#[cfg(feature = "std-deps")]
pub fn translate_ring_constraints(classified_state: &Object) -> Vec<Object> {
    let statement_ids = collect_statement_ids(classified_state);
    let mut out: Vec<Object> = Vec::new();
    for stmt_id in &statement_ids {
        let classifications = classifications_for(classified_state, stmt_id);
        if !classifications.iter().any(|k| k == "Ring Constraint") {
            continue;
        }
        let Some(marker) = trailing_marker_for(classified_state, stmt_id) else { continue };
        let Some(kind) = ring_adjective_to_kind(&marker) else { continue };
        let text = statement_text(classified_state, stmt_id).unwrap_or_default();
        let entity = head_noun_for(classified_state, stmt_id).unwrap_or_default();
        out.push(fact_from_pairs(&[
            ("id",       text.as_str()),
            ("kind",     kind),
            ("modality", "alethic"),
            ("text",     text.as_str()),
            ("entity",   entity.as_str()),
        ]));
    }
    out
}

/// Translate `Derivation Rule` classifications into `DerivationRule`
/// cell facts. Stage-2 emits a minimal skeleton — id + text —
/// matching the existing cell shape's `id` / `text` /
/// `consequentFactTypeId` / `json` bindings. Full Halpin resolution
/// (join keys, antecedent filters, consequent bindings,
/// consequent aggregates) stays in the Rust classifier for now and
/// will migrate in a follow-up commit once the
/// FactType + Role cells have been populated by Stage-2 earlier in
/// the pipeline.
#[cfg(feature = "std-deps")]
pub fn translate_derivation_rules(classified_state: &Object) -> Vec<Object> {
    let statement_ids = collect_statement_ids(classified_state);
    let mut out: Vec<Object> = Vec::new();
    for stmt_id in &statement_ids {
        let classifications = classifications_for(classified_state, stmt_id);
        if !classifications.iter().any(|k| k == "Derivation Rule") {
            continue;
        }
        let text = statement_text(classified_state, stmt_id).unwrap_or_default();
        out.push(fact_from_pairs(&[
            ("id",                   text.as_str()),
            ("text",                 text.as_str()),
            ("consequentFactTypeId", ""),
        ]));
    }
    out
}

/// Translate `Enum Values Declaration` classifications into
/// `EnumValues` cell facts. Each statement contributes one fact with
/// `noun` bound to the Head Noun and one `value0`, `value1`, …
/// binding per captured enum value (same shape as
/// `enum_values_for_noun` expects — see parse_forml2::upsert_enum_values).
///
/// The Value Type `Noun` fact is still emitted by `translate_nouns`
/// from the preceding `Priority is a value type.` statement — this
/// translator only contributes the value list.
#[cfg(feature = "std-deps")]
pub fn translate_enum_values(classified_state: &Object) -> Vec<Object> {
    let statement_ids = collect_statement_ids(classified_state);
    let mut out: Vec<Object> = Vec::new();
    for stmt_id in &statement_ids {
        let classifications = classifications_for(classified_state, stmt_id);
        if !classifications.iter().any(|k| k == "Enum Values Declaration") {
            continue;
        }
        let Some(noun) = head_noun_for(classified_state, stmt_id) else { continue };
        let values = enum_values_for(classified_state, stmt_id);
        if values.is_empty() { continue; }
        let mut pairs: Vec<(String, String)> = Vec::new();
        pairs.push(("noun".to_string(), noun));
        for (i, v) in values.iter().enumerate() {
            pairs.push((alloc::format!("value{i}"), v.clone()));
        }
        let pairs_ref: Vec<(&str, &str)> = pairs.iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        out.push(fact_from_pairs(&pairs_ref));
    }
    out
}

fn enum_values_for(state: &Object, stmt_id: &str) -> Vec<String> {
    fetch_or_phi("Statement_has_Enum_Value", state)
        .as_seq()
        .map(|facts| facts.iter()
            .filter(|f| binding(f, "Statement") == Some(stmt_id))
            .filter_map(|f| binding(f, "Enum_Value").map(String::from))
            .collect())
        .unwrap_or_default()
}

/// Translate `Deontic Constraint` classifications into `Constraint`
/// cell facts with modality="deontic" and the stripped deontic
/// operator. Entity defaults to the Head Noun of the body (after
/// the `It is X that` prefix was stripped by Stage-1).
#[cfg(feature = "std-deps")]
pub fn translate_deontic_constraints(classified_state: &Object) -> Vec<Object> {
    let statement_ids = collect_statement_ids(classified_state);
    let mut out: Vec<Object> = Vec::new();
    for stmt_id in &statement_ids {
        let classifications = classifications_for(classified_state, stmt_id);
        if !classifications.iter().any(|k| k == "Deontic Constraint") {
            continue;
        }
        let text = statement_text(classified_state, stmt_id).unwrap_or_default();
        let op = deontic_operator_for(classified_state, stmt_id).unwrap_or_default();
        let entity = head_noun_for(classified_state, stmt_id).unwrap_or_default();
        out.push(fact_from_pairs(&[
            ("id",               text.as_str()),
            ("kind",             "UC"),
            ("modality",         "deontic"),
            ("deonticOperator",  op.as_str()),
            ("text",             text.as_str()),
            ("entity",           entity.as_str()),
        ]));
    }
    out
}

fn deontic_operator_for(state: &Object, stmt_id: &str) -> Option<String> {
    fetch_or_phi("Statement_has_Deontic_Operator", state)
        .as_seq()?
        .iter()
        .find(|f| binding(f, "Statement") == Some(stmt_id))
        .and_then(|f| binding(f, "Deontic_Operator").map(String::from))
}

fn ring_adjective_to_kind(marker: &str) -> Option<&'static str> {
    match marker {
        "is irreflexive"   => Some("IR"),
        "is asymmetric"    => Some("AS"),
        "is antisymmetric" => Some("AT"),
        "is symmetric"     => Some("SY"),
        "is intransitive"  => Some("IT"),
        "is transitive"    => Some("TR"),
        "is acyclic"       => Some("AC"),
        "is reflexive"     => Some("RF"),
        _                  => None,
    }
}

fn trailing_marker_for(state: &Object, stmt_id: &str) -> Option<String> {
    fetch_or_phi("Statement_has_Trailing_Marker", state)
        .as_seq()?
        .iter()
        .find(|f| binding(f, "Statement") == Some(stmt_id))
        .and_then(|f| binding(f, "Trailing_Marker").map(String::from))
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

/// Collect all Statement ids from the `Statement` cell.
fn collect_statement_ids(state: &Object) -> Vec<String> {
    fetch_or_phi("Statement", state)
        .as_seq()
        .map(|facts| facts.iter()
            .filter_map(|f| binding(f, "id").map(String::from))
            .collect())
        .unwrap_or_default()
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
    fn translate_ring_constraints_covers_all_eight_adjectives() {
        for (text, nouns, expected_kind) in [
            ("Category has parent Category is acyclic.",    vec!["Category"], "AC"),
            ("Person is parent of Person is irreflexive.",  vec!["Person"],   "IR"),
            ("Person loves Person is symmetric.",           vec!["Person"],   "SY"),
            ("Thing owns Thing is asymmetric.",             vec!["Thing"],    "AS"),
            ("Thing owns Thing is antisymmetric.",          vec!["Thing"],    "AT"),
            ("Thing owns Thing is transitive.",             vec!["Thing"],    "TR"),
            ("Thing owns Thing is intransitive.",           vec!["Thing"],    "IT"),
            ("Thing owns Thing is reflexive.",              vec!["Thing"],    "RF"),
        ] {
            let stmt = stage1_state("s1", text, &nouns);
            let classified = classify_statements(&stmt, &grammar_state());
            let constraints = super::translate_ring_constraints(&classified);
            assert_eq!(constraints.len(), 1, "text={:?}", text);
            assert_eq!(binding(&constraints[0], "kind"), Some(expected_kind),
                "text={:?}", text);
            assert_eq!(binding(&constraints[0], "modality"), Some("alethic"));
        }
    }

    #[test]
    fn translate_ring_constraints_skips_non_ring_statements() {
        let stmt = stage1_state(
            "s1", "Customer is an entity type.", &["Customer"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let constraints = super::translate_ring_constraints(&classified);
        assert!(constraints.is_empty());
    }

    #[test]
    fn translate_derivation_rules_captures_text() {
        let stmt = stage1_state(
            "s1",
            "Customer has Full Name iff Customer has First Name.",
            &["Customer", "Full Name", "First Name"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let rules = super::translate_derivation_rules(&classified);
        assert_eq!(rules.len(), 1);
        assert!(binding(&rules[0], "text").unwrap()
                .contains("Customer has Full Name iff"));
    }

    #[test]
    fn translate_derivation_rules_skips_non_derivations() {
        let stmt = stage1_state("s1", "Customer is an entity type.", &["Customer"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let rules = super::translate_derivation_rules(&classified);
        assert!(rules.is_empty());
    }

    #[test]
    fn deontic_constraint_is_classified_for_obligatory() {
        let stmt = stage1_state(
            "s1", "It is obligatory that Customer has Email.",
            &["Customer", "Email"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&classified, "s1");
        assert!(kinds.iter().any(|k| k == "Deontic Constraint"),
            "expected Deontic Constraint; got {:?}", kinds);
    }

    #[test]
    fn translate_deontic_constraints_emits_with_operator_and_entity() {
        let stmt = stage1_state(
            "s1", "It is forbidden that Support Response uses Dash.",
            &["Support Response", "Dash"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let constraints = super::translate_deontic_constraints(&classified);
        assert_eq!(constraints.len(), 1);
        assert_eq!(binding(&constraints[0], "modality"), Some("deontic"));
        assert_eq!(binding(&constraints[0], "deonticOperator"), Some("forbidden"));
        assert_eq!(binding(&constraints[0], "entity"), Some("Support Response"));
    }

    #[test]
    fn enum_values_declaration_is_classified() {
        let stmt = stage1_state(
            "s1",
            "The possible values of Priority are 'low', 'medium', 'high'.",
            &["Priority"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&classified, "s1");
        assert!(kinds.iter().any(|k| k == "Enum Values Declaration"),
            "expected Enum Values Declaration; got {:?}", kinds);
    }

    #[test]
    fn translate_enum_values_emits_value_list_for_noun() {
        let stmt = stage1_state(
            "s1",
            "The possible values of Priority are 'low', 'medium', 'high'.",
            &["Priority"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let facts = super::translate_enum_values(&classified);
        assert_eq!(facts.len(), 1);
        let f = &facts[0];
        assert_eq!(binding(f, "noun"), Some("Priority"));
        assert_eq!(binding(f, "value0"), Some("low"));
        assert_eq!(binding(f, "value1"), Some("medium"));
        assert_eq!(binding(f, "value2"), Some("high"));
    }

    #[test]
    fn partition_declaration_is_classified() {
        let stmt = stage1_state(
            "s1", "Animal is partitioned into Cat, Dog, Bird.",
            &["Animal", "Cat", "Dog", "Bird"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&classified, "s1");
        assert!(kinds.iter().any(|k| k == "Partition Declaration"),
            "expected Partition Declaration; got {:?}", kinds);
    }

    #[test]
    fn translate_partitions_emits_subtype_facts_and_marks_supertype_abstract() {
        let stmt = stage1_state(
            "s1", "Animal is partitioned into Cat, Dog, Bird.",
            &["Animal", "Cat", "Dog", "Bird"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let subtypes = super::translate_partitions(&classified);
        let subs: Vec<_> = subtypes.iter()
            .filter_map(|f| binding(f, "subtype").map(String::from))
            .collect();
        assert_eq!(subs, vec!["Cat", "Dog", "Bird"]);
        for s in &subtypes {
            assert_eq!(binding(s, "supertype"), Some("Animal"));
        }
        // translate_nouns must see Partition Declaration as a signal
        // to mark Animal as abstract.
        let nouns = super::translate_nouns(&classified);
        let animal = nouns.iter()
            .find(|f| binding(f, "name") == Some("Animal"))
            .expect("Animal noun fact");
        assert_eq!(binding(animal, "objectType"), Some("abstract"));
    }
}
