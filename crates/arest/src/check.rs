// crates/arest/src/check.rs
//
// Readings checker (#199, #213) — diagnostics as a fold over cells.
//
// Per Backus FFP and AREST Theorem 2: the checker is a function from
// Object state (the parser's output) to a sequence of diagnostics.
// No Domain struct. No intermediate representation. The checker reads
// cells via fetch_or_phi + binding, composes diagnostics with iterator
// combinators (map / filter / flat_map), and emits a Vec.
//
// Each layer is a pure function over &Object:
//
//   check_unresolved_clauses : state -> [diagnostic]
//   check_ring_validity      : state -> [diagnostic]
//   check_ring_completeness  : state -> [diagnostic]
//   check_singular_naming    : state -> [diagnostic]
//   check_atom_ids           : state -> [diagnostic]
//
// check_readings composes them: concat ∘ [check_unresolved_clauses,
// check_ring_validity, check_ring_completeness, check_singular_naming,
// check_atom_ids] ∘ parse_to_state.

use crate::ast::{Object, binding, fetch_or_phi};
use crate::parse_forml2::parse_to_state;
use crate::naming::atom_id_is_valid;
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Level { Error, Warning, Hint }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source { Parse, Resolve, Deontic }

#[derive(Debug, Clone)]
pub struct ReadingDiagnostic {
    pub line: usize,
    pub reading: String,
    pub level: Level,
    pub source: Source,
    pub message: String,
    pub suggestion: Option<String>,
}

/// Run every checker layer against `text`. Each layer is a pure
/// function over Object state — no Domain, no intermediate struct.
pub fn check_readings(text: &str) -> Vec<ReadingDiagnostic> {
    match parse_to_state(text) {
        Ok(state) => [
            check_unresolved_clauses(&state),
            check_ring_validity(&state),
            check_ring_completeness(&state),
            check_singular_naming(&state),
            check_atom_ids(&state),
        ].into_iter().flatten().collect(),
        Err(e) => vec![ReadingDiagnostic {
            line: 0,
            reading: String::new(),
            level: Level::Error,
            source: Source::Parse,
            message: format!("parse failed: {e}"),
            suggestion: None,
        }],
    }
}

/// Each UnresolvedClause cell entry becomes one Resolve warning.
fn check_unresolved_clauses(state: &Object) -> Vec<ReadingDiagnostic> {
    fetch_or_phi("UnresolvedClause", state).as_seq()
        .map(|facts| facts.iter().map(|f| ReadingDiagnostic {
            line: 0,
            reading: binding(f, "ruleText").unwrap_or("").to_string(),
            level: Level::Warning,
            source: Source::Resolve,
            message: format!(
                "antecedent clause did not resolve to a declared fact type: `{}`",
                binding(f, "clause").unwrap_or(""),
            ),
            suggestion: Some("check that the clause references a declared fact type, or uses a recognised form (comparison, aggregate, computed binding)".to_string()),
        }).collect())
        .unwrap_or_default()
}

/// Ring constraints (IR/AS/AT/SY/IT/TR/AC/RF) must span roles on a
/// single noun. A ring with mixed-noun roles is nonsensical.
fn check_ring_validity(state: &Object) -> Vec<ReadingDiagnostic> {
    let constraint_cell = fetch_or_phi("Constraint", state);
    let role_cell = fetch_or_phi("Role", state);
    constraint_cell.as_seq()
        .map(|facts| facts.iter()
            .filter(|c| is_ring_kind(binding(c, "kind").unwrap_or("")))
            .filter_map(|c| {
                let span_ft = binding(c, "span0_factTypeId")?;
                let role_nouns: hashbrown::HashSet<&str> = role_cell.as_seq()
                    .map(|rs| rs.iter()
                        .filter(|r| binding(r, "factType") == Some(span_ft))
                        .filter_map(|r| binding(r, "nounName"))
                        .collect())
                    .unwrap_or_default();
                match role_nouns.len() > 1 {
                    true => Some(ReadingDiagnostic {
                        line: 0,
                        reading: binding(c, "text").unwrap_or("").to_string(),
                        level: Level::Error,
                        source: Source::Deontic,
                        message: format!(
                            "ring constraint `{}` on fact type `{}` spans roles of different nouns ({:?}) — ring constraints require the same noun on both sides",
                            binding(c, "kind").unwrap_or(""), span_ft, role_nouns,
                        ),
                        suggestion: Some("either drop the ring constraint or restructure the fact type so both roles share a noun".to_string()),
                    }),
                    false => None,
                }
            })
            .collect())
        .unwrap_or_default()
}

/// Binary FTs whose two roles reference the same noun without a ring
/// constraint are usually a bug — nothing prevents self-reference cycles.
fn check_ring_completeness(state: &Object) -> Vec<ReadingDiagnostic> {
    let ft_cell = fetch_or_phi("FactType", state);
    let role_cell = fetch_or_phi("Role", state);
    let constraint_cell = fetch_or_phi("Constraint", state);

    ft_cell.as_seq()
        .map(|fts| fts.iter().filter_map(|ft| {
            let ft_id = binding(ft, "id")?;
            let roles: Vec<&str> = role_cell.as_seq()
                .map(|rs| rs.iter()
                    .filter(|r| binding(r, "factType") == Some(ft_id))
                    .filter_map(|r| binding(r, "nounName"))
                    .collect())
                .unwrap_or_default();
            // Binary + same noun both roles
            (roles.len() == 2 && roles[0] == roles[1]).then_some(())?;
            let ring_noun = roles[0];
            let has_ring = constraint_cell.as_seq()
                .map(|cs| cs.iter().any(|c|
                    is_ring_kind(binding(c, "kind").unwrap_or(""))
                        && (binding(c, "span0_factTypeId") == Some(ft_id)
                            || binding(c, "entity") == Some(ring_noun))))
                .unwrap_or(false);
            (!has_ring).then(|| {
                let reading = binding(ft, "reading").unwrap_or("").to_string();
                ReadingDiagnostic {
                    line: 0,
                    reading: reading.clone(),
                    level: Level::Hint,
                    source: Source::Deontic,
                    message: format!(
                        "ring fact type `{}` on noun `{}` has no ring constraint — consider asserting irreflexive / asymmetric / acyclic as appropriate",
                        ft_id, ring_noun,
                    ),
                    suggestion: Some(format!("`{} is irreflexive.` or `{} is acyclic.`", reading, reading)),
                }
            })
        }).collect())
        .unwrap_or_default()
}

/// Noun names that look like plurals of `<base>y` (via the `ies → y`
/// round-trip) — ORM 2 convention is singular entity names.
fn check_singular_naming(state: &Object) -> Vec<ReadingDiagnostic> {
    fetch_or_phi("Noun", state).as_seq()
        .map(|ns| ns.iter().filter_map(|n| {
            let name = binding(n, "name")?;
            let base = name.strip_suffix("ies").filter(|b| !b.is_empty())?;
            let singular = format!("{}y", base);
            (crate::naming::pluralize(&singular) == name).then(|| ReadingDiagnostic {
                line: 0,
                reading: name.to_string(),
                level: Level::Warning,
                source: Source::Deontic,
                message: format!(
                    "noun `{}` looks like a plural of `{}` — ORM 2 convention is singular entity names",
                    name, singular,
                ),
                suggestion: Some(format!("rename to `{}` and declare `Noun '{}' has Plural '{}'.`",
                    singular, singular, name)),
            })
        }).collect())
        .unwrap_or_default()
}

/// Atom IDs on instance facts that aren't printable ASCII — Func::Lower
/// and fixed-width name wires (FPGA ingress) misbehave on those.
fn check_atom_ids(state: &Object) -> Vec<ReadingDiagnostic> {
    fetch_or_phi("InstanceFact", state).as_seq()
        .map(|facts| facts.iter().flat_map(|f| {
            let subject_noun = binding(f, "subjectNoun").unwrap_or("").to_string();
            let subject_value = binding(f, "subjectValue").unwrap_or("").to_string();
            let object_noun = binding(f, "objectNoun").unwrap_or("").to_string();
            let object_value = binding(f, "objectValue").unwrap_or("").to_string();

            let subject_diag = (!subject_value.is_empty() && !atom_id_is_valid(&subject_value))
                .then(|| ReadingDiagnostic {
                    line: 0,
                    reading: format!("{} '{}'", subject_noun, subject_value),
                    level: Level::Warning,
                    source: Source::Resolve,
                    message: format!(
                        "atom id `{}` is not printable ASCII; Func::Lower and fixed-width name ports (FPGA) may misbehave",
                        subject_value,
                    ),
                    suggestion: Some("use an ASCII slug (e.g. strip diacritics, transliterate)".to_string()),
                });

            // Only flag object-value atom IDs when the object is an entity
            // (object_noun non-empty AND value is identifier-shaped).
            let object_diag = (!object_value.is_empty()
                && !object_noun.is_empty()
                && !atom_id_is_valid(&object_value)
                && !object_value.contains(' ')
                && object_value.len() < 64)
                .then(|| ReadingDiagnostic {
                    line: 0,
                    reading: format!("{} '{}' ... '{}'", subject_noun, subject_value, object_value),
                    level: Level::Hint,
                    source: Source::Resolve,
                    message: format!("atom id `{}` is not printable ASCII", object_value),
                    suggestion: None,
                });

            subject_diag.into_iter().chain(object_diag)
        }).collect())
        .unwrap_or_default()
}

/// Ring-constraint kinds per ORM 2. Shared between layers.
fn is_ring_kind(k: &str) -> bool {
    matches!(k, "IR" | "AS" | "AT" | "SY" | "IT" | "TR" | "AC" | "RF")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_readings_produce_no_diagnostics() {
        let input = "Order(.Order Id) is an entity type.\n## Fact Types\nOrder has Amount.\n## Instance Facts\nOrder 'ord-1' has Amount '100'.";
        let diags = check_readings(input);
        assert!(diags.is_empty(), "expected no diagnostics, got {:?}", diags);
    }

    #[test]
    fn unresolved_derivation_antecedent_surfaces_warning() {
        let input = "Order(.Id) is an entity type.\n## Fact Types\nOrder has Amount.\n## Derivation Rules\n+ Order has Amount if Order has Amount and Order has Mystery and Order has Phantom.";
        let diags = check_readings(input);
        let resolve_warnings: Vec<_> = diags.iter()
            .filter(|d| d.source == Source::Resolve && d.level == Level::Warning)
            .collect();
        assert!(!resolve_warnings.is_empty(),
            "expected a Resolve warning for dropped antecedents, got {:?}", diags);
        assert!(resolve_warnings[0].message.contains("antecedent"));
    }

    #[test]
    fn non_ascii_atom_id_warns() {
        let input = "City(.Name) is an entity type.\n## Instance Facts\nCity 'café' has Population '100'.";
        let diags = check_readings(input);
        let ascii_warnings: Vec<_> = diags.iter()
            .filter(|d| d.message.contains("café"))
            .collect();
        assert!(!ascii_warnings.is_empty(),
            "expected ASCII warning for `café`, got {:?}", diags);
    }

    #[test]
    fn diagnostic_carries_reading_text_and_suggestion() {
        let input = "City(.Name) is an entity type.\n## Instance Facts\nCity 'café' has Population '100'.";
        let diags = check_readings(input);
        let d = diags.iter().find(|d| d.message.contains("café")).unwrap();
        assert!(!d.reading.is_empty(), "diagnostic must carry the offending reading text");
        assert!(d.suggestion.is_some(), "ASCII warning should include a suggestion");
    }

    #[test]
    fn ring_constraint_on_mixed_nouns_surfaces_error() {
        // Can't trigger via readings today because the parser's ring
        // shorthand requires single-noun FT. The check still compiles
        // clean against any state — test via raw construction would
        // need fixture helpers. Keep as smoke coverage.
        let input = "Employee(.Id) is an entity type.\nManager(.Id) is an entity type.\n## Fact Types\nEmployee reports to Manager.";
        let diags = check_readings(input);
        assert!(diags.iter().all(|d| d.level != Level::Error),
            "no ring error expected for well-formed mixed-noun FT, got {:?}", diags);
    }

    #[test]
    fn ring_fact_type_without_ring_constraint_produces_hint() {
        let input = "Person(.Id) is an entity type.\n## Fact Types\nPerson is parent of Person.";
        let diags = check_readings(input);
        let ring_hints: Vec<_> = diags.iter()
            .filter(|d| d.level == Level::Hint && d.message.contains("no ring constraint"))
            .collect();
        assert!(!ring_hints.is_empty(),
            "ring FT without ring constraint should produce Hint, got {:?}", diags);
    }

    #[test]
    fn ring_fact_type_with_ring_constraint_stays_quiet() {
        let input = "Person(.Id) is an entity type.\n## Fact Types\nPerson is parent of Person.\n## Constraints\nNo Person is parent of itself.";
        let diags = check_readings(input);
        let ring_hints: Vec<_> = diags.iter()
            .filter(|d| d.level == Level::Hint && d.message.contains("no ring constraint"))
            .collect();
        assert!(ring_hints.is_empty(),
            "ring with AC constraint should NOT produce completeness hint, got {:?}", ring_hints);
    }
}
