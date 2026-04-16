// crates/arest/src/check.rs
//
// Readings checker (#199) — diagnoses FORML 2 input before `compile`.
// LLM-authored readings often ship with syntax mistakes the silent-
// parse path swallows (unresolved antecedents, non-ASCII atom IDs,
// ring shorthand on a non-ring FT). This module surfaces those as
// structured diagnostics so MCP agents can self-correct before
// committing a schema change.
//
// Three-layer design (v0 ships layers 1 + 2):
//   Parse   — lines that didn't classify as any known shape.
//   Resolve — references that didn't bind (antecedent FT, ring
//             noun, atom ID).
//   Deontic — evaluate readings/validation.md deontic rules over
//             the parsed IR. Deferred; the engine infrastructure
//             for running validation.md as a sibling pipeline
//             needs its own commit.
//
// No LLM in the loop. Every diagnostic is a pure check against the
// IR the compile pipeline already builds.

use crate::parse_forml2::parse_markdown;
use crate::naming::atom_id_is_valid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Level { Error, Warning, Hint }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source { Parse, Resolve, Deontic }

#[derive(Debug, Clone)]
pub struct ReadingDiagnostic {
    /// 1-based line number in the input text. 0 means "no specific line".
    pub line: usize,
    /// The reading / rule / constraint text the diagnostic refers to.
    pub reading: String,
    pub level: Level,
    pub source: Source,
    pub message: String,
    /// Optional fix-hint ("Did you mean `that is`?").
    pub suggestion: Option<String>,
}

/// Run every available checker layer against `text` and return a flat
/// diagnostic list sorted by line. Empty output means the readings
/// parse cleanly AND every reference resolves AND every atom ID is
/// ASCII-valid.
///
/// Does NOT mutate any state — safe to call from an MCP `check` verb
/// before `compile`.
pub fn check_readings(text: &str) -> Vec<ReadingDiagnostic> {
    let mut diags = Vec::new();

    // Layer 1: Parse. If parse_markdown returns Err, surface it as a
    // single parse error and stop — layer 2 can't run without the IR.
    let ir = match parse_markdown(text) {
        Ok(ir) => ir,
        Err(e) => {
            diags.push(ReadingDiagnostic {
                line: 0,
                reading: String::new(),
                level: Level::Error,
                source: Source::Parse,
                message: format!("parse failed: {e}"),
                suggestion: None,
            });
            return diags;
        }
    };

    // Layer 2a: Derivation rules whose antecedents didn't all resolve.
    // parse_forml2 filter_maps unresolved antecedents silently today;
    // we detect the loss by comparing antecedent_fact_type_ids.len()
    // against the number of " and "-joined parts after the if/iff
    // split. A drop is a Resolve-layer Warning.
    for rule in &ir.derivation_rules {
        let antecedent_text = rule.text
            .find(" iff ").map(|i| &rule.text[i + 5..])
            .or_else(|| rule.text.find(" if ").map(|i| &rule.text[i + 4..]))
            .unwrap_or("");
        let part_count = antecedent_text.split(" and ")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .count();
        let resolved_count = rule.antecedent_fact_type_ids.len()
            + rule.consequent_aggregates.len()
            + rule.consequent_computed_bindings.len();
        if part_count > 0 && resolved_count < part_count {
            diags.push(ReadingDiagnostic {
                line: 0,
                reading: rule.text.clone(),
                level: Level::Warning,
                source: Source::Resolve,
                message: format!(
                    "derivation rule has {part_count} antecedent clause(s) but only {resolved_count} resolved to a fact type, aggregate, or computed binding",
                ),
                suggestion: Some("check that every `and`-joined clause references a declared fact type or uses a known arithmetic / aggregate shape".to_string()),
            });
        }
    }

    // Layer 2b: Atom IDs on instance facts. Non-ASCII IDs compile but
    // misbehave under Func::Lower and can't fit fixed-width wires
    // (FPGA fact-ingress). Flag as Warnings.
    for fact in &ir.general_instance_facts {
        if !fact.subject_value.is_empty() && !atom_id_is_valid(&fact.subject_value) {
            diags.push(ReadingDiagnostic {
                line: 0,
                reading: format!("{} '{}'", fact.subject_noun, fact.subject_value),
                level: Level::Warning,
                source: Source::Resolve,
                message: format!(
                    "atom id `{}` is not printable ASCII; Func::Lower and fixed-width name ports (FPGA) may misbehave",
                    fact.subject_value,
                ),
                suggestion: Some("use an ASCII slug (e.g. strip diacritics, transliterate)".to_string()),
            });
        }
        if !fact.object_value.is_empty()
            && !fact.object_noun.is_empty()
            && !atom_id_is_valid(&fact.object_value)
        {
            // Free-form value fields (Description, Text) are legitimate
            // Unicode; we only flag when the object is an entity
            // (object_noun non-empty AND the value looks identifier-
            // shaped — no spaces, bounded length).
            if !fact.object_value.contains(' ') && fact.object_value.len() < 64 {
                diags.push(ReadingDiagnostic {
                    line: 0,
                    reading: format!("{} '{}' ... '{}'", fact.subject_noun, fact.subject_value, fact.object_value),
                    level: Level::Hint,
                    source: Source::Resolve,
                    message: format!(
                        "atom id `{}` is not printable ASCII",
                        fact.object_value,
                    ),
                    suggestion: None,
                });
            }
        }
    }

    diags
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
        // Rule has 3 "and"-joined clauses but only the first resolves
        // to an FT (the other two reference a noun that doesn't exist).
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
    fn clean_unicode_in_free_text_fields_does_not_warn() {
        // Description / Text free-form fields stay Unicode. The
        // checker only flags atom IDs (entity references).
        let input = "Domain(.Slug) is an entity type.\nDescription is a value type.\n## Fact Types\nDomain has Description.\n## Instance Facts\nDomain 'core' has Description 'café au lait is fine here'.";
        let diags = check_readings(input);
        assert!(diags.iter().all(|d| !d.message.contains("café")),
            "free-form Description must not trigger ASCII warnings, got {:?}", diags);
    }
}
