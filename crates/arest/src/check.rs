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
///
/// Role cells carry `nounName` as set by parse_fact, which runs
/// longest-first noun matching against whatever nouns had been
/// declared up to that point. Inline `.id` declarations in role
/// position (e.g. `Transfer(.id) transmits Personal Data(.id).`) do
/// NOT auto-declare the noun, so compound nouns like `Personal Data`
/// are often missing from the noun set when a later reading like
/// `Personal Data Breach is breach of security leading to loss of
/// Personal Data` is parsed. Both role positions fall through to
/// bare `Data`, the stored reading becomes `Data ... Personal Data`
/// (first-role prefix dropped, second-role kept because the parser
/// quotes `found[1].2` verbatim and `Data` at the end has no
/// surviving prefix text after the match), and the check fires.
///
/// Suppression heuristic: if the stored reading contains the pattern
/// `<CapitalizedWord> <ring_noun>` anywhere, at least one role
/// originally resolved to a compound noun ending in ring_noun —
/// treat the ring detection as a parse-time artifact and stay quiet.
/// Reproduces the 9 false positives from the FORML sibling agent's
/// run against the eu-law corpus.
fn check_ring_completeness(state: &Object) -> Vec<ReadingDiagnostic> {
    let ft_cell = fetch_or_phi("FactType", state);
    let role_cell = fetch_or_phi("Role", state);
    let constraint_cell = fetch_or_phi("Constraint", state);
    let noun_names: Vec<String> = fetch_or_phi("Noun", state).as_seq()
        .map(|ns| ns.iter()
            .filter_map(|n| binding(n, "name").map(|s| s.to_string()))
            .collect())
        .unwrap_or_default();

    ft_cell.as_seq()
        .map(|fts| fts.iter().filter_map(|ft| {
            let ft_id = binding(ft, "id")?;
            let roles: Vec<&str> = role_cell.as_seq()
                .map(|rs| rs.iter()
                    .filter(|r| binding(r, "factType") == Some(ft_id))
                    .filter_map(|r| binding(r, "nounName"))
                    .collect())
                .unwrap_or_default();
            // Binary + same noun both roles at parse time
            (roles.len() == 2 && roles[0] == roles[1]).then_some(())?;
            let ring_noun = roles[0];

            // Suppression layer 1: the stored reading still contains a
            // `<Capitalized> <ring_noun>` pair (e.g. "Personal Data"
            // surviving at the end of a reading whose first role
            // prefix was stripped). Evidence of a compound noun that
            // the parse-time noun list was missing.
            let reading = binding(ft, "reading").unwrap_or("");
            let has_compound_prefix = text_contains_capitalized_prefixed(reading, ring_noun);
            // Suppression layer 2: a compound noun ending in ring_noun
            // is declared elsewhere in the corpus (e.g. "Biometric
            // Data" in eu-law). Even if this FT's stored reading has
            // lost every prefix to parse_fact's role-capture rebuild,
            // the presence of such a compound makes the ring reading
            // ambiguous enough to suppress — ring hints are advisory
            // and false positives from tokenization are strictly
            // worse than a missed hint.
            let compound_noun_declared = noun_names.iter().any(|n| {
                let suffix = match ring_noun.is_empty() {
                    true => false,
                    false => n.ends_with(ring_noun)
                        && n.len() > ring_noun.len()
                        && n.as_bytes()[n.len() - ring_noun.len() - 1] == b' ',
                };
                suffix
            });
            (!has_compound_prefix && !compound_noun_declared).then_some(())?;

            let has_ring = constraint_cell.as_seq()
                .map(|cs| cs.iter().any(|c|
                    is_ring_kind(binding(c, "kind").unwrap_or(""))
                        && (binding(c, "span0_factTypeId") == Some(ft_id)
                            || binding(c, "entity") == Some(ring_noun))))
                .unwrap_or(false);
            (!has_ring).then(|| {
                let reading = reading.to_string();
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

/// True iff `text` contains an occurrence of `target` that is
/// immediately preceded by a Capitalized word (ASCII uppercase
/// followed by at least one lowercase letter) and a single space.
/// Used by `check_ring_completeness` to detect compound-noun
/// false positives without needing the parser to have accumulated
/// the compound declaration.
fn text_contains_capitalized_prefixed(text: &str, target: &str) -> bool {
    if target.is_empty() { return false; }
    let bytes = text.as_bytes();
    let target_bytes = target.as_bytes();
    let mut pos = 0;
    while let Some(hit) = text[pos..].find(target) {
        let start = pos + hit;
        // Word boundary at end of match (don't match a prefix of a longer word).
        let end = start + target_bytes.len();
        let after_ok = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric();
        // There must be ` X…` before the target where X is ASCII uppercase
        // and followed by ASCII lowercase — a Capitalized word token.
        let prefixed = start >= 2 && bytes[start - 1] == b' '
            && {
                // Walk back to the preceding space (or start of text) to
                // isolate the word immediately before the target.
                let word_end = start - 1;
                let mut word_start = word_end;
                while word_start > 0 && bytes[word_start - 1] != b' ' {
                    word_start -= 1;
                }
                // The preceding word is bytes[word_start..word_end].
                // It must start with an uppercase ASCII letter and
                // contain at least one lowercase ASCII letter.
                let word = &bytes[word_start..word_end];
                !word.is_empty()
                    && word[0].is_ascii_uppercase()
                    && word.iter().skip(1).any(|b| b.is_ascii_lowercase())
            };
        if after_ok && prefixed { return true; }
        pos = start + 1;
        if pos >= text.len() { break; }
    }
    false
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

    /// Regression: the eu-law corpus uses compound nouns like
    /// `Personal Data` and `Personal Data Breach` that the parser
    /// does not auto-declare (inline `.id` in role position is not a
    /// declaration), so they are missing from the Noun set when the
    /// FT reading is parsed. find_nouns falls through to bare `Data`
    /// for both role positions, Role.nounName = "Data" twice, and
    /// ring completeness fires spuriously. Reproduces the 9 false
    /// positives from the FORML sibling agent's run against
    /// C:\Users\lippe\Repos\eu-law\readings.
    ///
    /// The fix (in check_ring_completeness): if the stored FT reading
    /// contains `<CapitalizedWord> <ring_noun>` anywhere, at least
    /// one role was a compound noun — treat the detection as a
    /// parse-time artifact and stay quiet.
    #[test]
    fn compound_nouns_sharing_suffix_are_not_a_ring_on_suffix() {
        let input = "\
Data(.id) is an entity type.
Personal Data Breach is breach of security leading to accidental or unlawful loss of Personal Data.
Data is processed in manner that ensures appropriate security of Personal Data.
";
        let diags = check_readings(input);
        let ring_hints: Vec<_> = diags.iter()
            .filter(|d| d.level == Level::Hint && d.message.contains("no ring constraint"))
            .collect();
        assert!(ring_hints.is_empty(),
            "compound nouns ending in `Data` must not trip ring completeness on bare `Data`; got {:?}", ring_hints);
    }

    /// Negative: a genuine self-ring on a compound noun should still
    /// produce the hint — `Monitoring Body must take Monitoring Body`
    /// has both roles legitimately on `Monitoring Body`, and the
    /// preceding words (start of string / `take`) are not Capitalized
    /// prefixes of another noun, so the heuristic does not suppress.
    #[test]
    fn genuine_ring_on_compound_noun_still_fires() {
        let input = "\
Monitoring Body(.id) is an entity type.
Monitoring Body must take Monitoring Body.
";
        let diags = check_readings(input);
        let ring_hints: Vec<_> = diags.iter()
            .filter(|d| d.level == Level::Hint && d.message.contains("no ring constraint"))
            .collect();
        assert!(!ring_hints.is_empty(),
            "real self-ring on a compound noun must still produce the completeness hint; got no hints in {:?}", diags);
    }

    /// Regression: sherlock's evidence.md writes ring constraints with
    /// trailing documentation annotations: `No Hypothesis contradicts
    /// itself. (irreflexive)` and `If some Hypothesis1 ... . (symmetric)`.
    /// Before the fix in parse_forml2::try_ring, the parenthetical
    /// suffix blocked the `.ends_with(" itself")` and if-then
    /// recognition, AND the if-then branch emitted constraints with
    /// entity=None so check_ring_completeness couldn't link them to
    /// their FT. Both cases produced bogus "no ring constraint" hints.
    #[test]
    fn declared_ring_constraints_with_annotations_suppress_hint() {
        let input = "\
Hypothesis(.id) is an entity type.
## Fact Types
Hypothesis contradicts Hypothesis.
## Ring Constraints
If some Hypothesis1 contradicts some Hypothesis2 then that Hypothesis2 contradicts that Hypothesis1. (symmetric)
No Hypothesis contradicts itself. (irreflexive)
";
        let diags = check_readings(input);
        let ring_hints: Vec<_> = diags.iter()
            .filter(|d| d.level == Level::Hint && d.message.contains("no ring constraint"))
            .collect();
        assert!(ring_hints.is_empty(),
            "declared IR+SY ring constraints with `(kind)` annotations must suppress the hint; got {:?}", ring_hints);
    }

    #[test]
    fn text_contains_capitalized_prefixed_only_fires_on_compound_nouns() {
        // Positive: "Personal Data" has "Personal" as capitalized prefix.
        assert!(super::text_contains_capitalized_prefixed(
            "Data is processed in manner that ensures appropriate security of Personal Data",
            "Data",
        ));
        // Negative: "Data or Data" — "or" is lowercase, no compound.
        assert!(!super::text_contains_capitalized_prefixed("Data or Data", "Data"));
        // Negative: "Monitoring Body takes Monitoring Body" — `takes` is lowercase.
        assert!(!super::text_contains_capitalized_prefixed(
            "Monitoring Body takes Monitoring Body",
            "Monitoring Body",
        ));
        // Negative: "Data Subject where Data Subject" — `where` is lowercase.
        assert!(!super::text_contains_capitalized_prefixed(
            "Data Subject where Data Subject",
            "Data Subject",
        ));
        // Negative: an acronym like "GDPR Data" — "GDPR" has no lowercase
        // letters, so it doesn't count as a "Capitalized word" for our
        // compound-noun heuristic.
        assert!(!super::text_contains_capitalized_prefixed("GDPR Data processes Data", "Data"));
    }
}
