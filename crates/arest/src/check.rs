// crates/arest/src/check.rs
//
// Readings checker (#199, #213, #214) — diagnostics as a ρ-application
// over cells.
//
// Per Backus FFP and AREST Theorem 2 / Theorem 5: the checker is a
// Func tree applied via ast::apply, not Rust control flow. Its top
// level is
//
//   check_readings_func = Concat ∘ [ layer₁, …, layer₅ ]
//
// where each layerᵢ reads one or more cells from D and emits a
// sequence of diagnostic Objects. Rust only parses the raw text,
// applies the Func, and decodes the diagnostic sequence back to the
// public `Vec<ReadingDiagnostic>` shape at the API boundary.
//
// The five layer bodies remain Rust functions for now (each wrapped
// in a Func::Native leaf) because they read multiple cells and
// format messages; the composition itself is the Func tree. Further
// FFP lowering can push per-layer logic (`ApplyToAll`, `Filter`,
// `Selector`) down into the leaves over time.

use crate::ast::{Object, binding, fetch_or_phi, Func};
use crate::parse_forml2::parse_to_state;
use crate::naming::atom_id_is_valid;
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned, sync::Arc};

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

// ── Atom constants for Level / Source encoding ──────────────────────

const LVL_ERROR:   &str = "Error";
const LVL_WARNING: &str = "Warning";
const LVL_HINT:    &str = "Hint";
const SRC_PARSE:   &str = "parse";
const SRC_RESOLVE: &str = "resolve";
const SRC_DEONTIC: &str = "deontic";

fn encode_diag(d: &ReadingDiagnostic) -> Object {
    let mut map = hashbrown::HashMap::new();
    map.insert("line".to_string(),    Object::atom(&d.line.to_string()));
    map.insert("reading".to_string(), Object::atom(&d.reading));
    map.insert("level".to_string(),   Object::atom(match d.level {
        Level::Error   => LVL_ERROR,
        Level::Warning => LVL_WARNING,
        Level::Hint    => LVL_HINT,
    }));
    map.insert("source".to_string(),  Object::atom(match d.source {
        Source::Parse   => SRC_PARSE,
        Source::Resolve => SRC_RESOLVE,
        Source::Deontic => SRC_DEONTIC,
    }));
    map.insert("message".to_string(), Object::atom(&d.message));
    if let Some(s) = d.suggestion.as_ref() {
        map.insert("suggestion".to_string(), Object::atom(s));
    }
    Object::Map(map)
}

fn decode_diag(obj: &Object) -> Option<ReadingDiagnostic> {
    let map = obj.as_map()?;
    let line = map.get("line").and_then(|o| o.as_atom())
        .and_then(|s| s.parse().ok()).unwrap_or(0);
    let reading = map.get("reading").and_then(|o| o.as_atom())
        .unwrap_or("").to_string();
    let level = match map.get("level").and_then(|o| o.as_atom()) {
        Some(LVL_ERROR)   => Level::Error,
        Some(LVL_HINT)    => Level::Hint,
        _                 => Level::Warning,
    };
    let source = match map.get("source").and_then(|o| o.as_atom()) {
        Some(SRC_PARSE)   => Source::Parse,
        Some(SRC_DEONTIC) => Source::Deontic,
        _                 => Source::Resolve,
    };
    let message = map.get("message").and_then(|o| o.as_atom())
        .unwrap_or("").to_string();
    let suggestion = map.get("suggestion").and_then(|o| o.as_atom())
        .map(String::from);
    Some(ReadingDiagnostic { line, reading, level, source, message, suggestion })
}

fn encode_diags(diags: Vec<ReadingDiagnostic>) -> Object {
    Object::seq(diags.iter().map(encode_diag).collect())
}

fn decode_diags(obj: &Object) -> Vec<ReadingDiagnostic> {
    obj.as_seq()
        .map(|s| s.iter().filter_map(decode_diag).collect())
        .unwrap_or_default()
}

/// Wrap a Rust layer `state -> Vec<ReadingDiagnostic>` as a Func leaf
/// that consumes the state Object and emits the encoded diagnostic
/// sequence. Each layer is thus a ρ-application over the cells it
/// reads; the top-level check_readings_func composes them via Concat.
fn layer_native<F>(rust_layer: F) -> Func
where F: Fn(&Object) -> Vec<ReadingDiagnostic> + Send + Sync + 'static {
    Func::Native(Arc::new(move |state| encode_diags(rust_layer(state))))
}

/// check_readings as a Func tree. Reads cells from the state (passed
/// as apply's operand) and returns a Seq of diagnostic Maps.
///
///   check_readings_func = Concat ∘ [ layer₁, layer₂, layer₃, layer₄, layer₅ ]
///
/// The composition is explicit FFP; layer bodies stay Native for now
/// because several read multiple cells and format messages. Future
/// work (#214 cont.) can lower each layer body into `ApplyToAll`,
/// `Filter`, `Construction`, and binding-extract primitives.
pub fn check_readings_func() -> Func {
    Func::compose(
        Func::Concat,
        Func::construction(vec![
            layer_native(check_unresolved_clauses),
            layer_native(check_ring_validity),
            layer_native(check_ring_completeness),
            layer_native(check_singular_naming),
            layer_native(check_atom_ids),
        ]),
    )
}

/// Run the checker pipeline against `text`.
///
/// Structure: parse → apply(check_readings_func, state, state) → decode.
/// The Rust glue is minimal — it only parses the raw markdown and
/// decodes the diagnostic Seq back into the public struct shape at
/// the API boundary. All diagnostic logic is expressed as the Func
/// tree defined by `check_readings_func`.
pub fn check_readings(text: &str) -> Vec<ReadingDiagnostic> {
    match parse_to_state(text) {
        Ok(state) => {
            let result = crate::ast::apply(&check_readings_func(), &state, &state);
            decode_diags(&result)
        }
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
    // Noun → objectType lookup. Value types (`Prompt Icon is a value
    // type. Suggested Prompt has Prompt Icon.`) carry content, not an
    // identifier, so emoji / non-ASCII object values in those slots
    // are intentional and must not trip the atom-id check.
    let value_type_nouns: hashbrown::HashSet<String> = fetch_or_phi("Noun", state).as_seq()
        .map(|ns| ns.iter()
            .filter(|n| binding(n, "objectType") == Some("value"))
            .filter_map(|n| binding(n, "name").map(|s| s.to_string()))
            .collect())
        .unwrap_or_default();

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

            // Only flag object-value atom IDs when the object is an entity.
            // Value-type objects (e.g. Prompt Icon, Description, URL) hold
            // content, not identifiers — non-ASCII content (emoji, i18n
            // text, Unicode symbols) is legitimate and must not be flagged.
            let object_is_value_type = value_type_nouns.contains(&object_noun);
            let object_diag = (!object_value.is_empty()
                && !object_noun.is_empty()
                && !object_is_value_type
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

    /// #274 Category A — unary derived FT (one role + predicate + `*`/`+`
    /// marker) used as an antecedent in another rule. Before this fix the
    /// resolver required binary-or-higher fact types and rejected unary
    /// synthetics, so 18+ rules in auto.dev plus dozens in eu-law/us-law
    /// fired false "unresolved antecedent" warnings.
    #[test]
    fn category_a_unary_derived_factype_as_antecedent() {
        let input = "Fetcher(.Name) is an entity type.\n\
                     ## Fact Types\n\
                     Fetcher has Speed.\n\
                     Fetcher is proxy-based. +\n\
                     ## Derivation Rules\n\
                     + Fetcher has Speed if Fetcher is proxy-based.";
        let diags = check_readings(input);
        let unresolved: Vec<_> = diags.iter()
            .filter(|d| d.source == Source::Resolve && d.level == Level::Warning)
            .filter(|d| d.message.contains("is proxy-based"))
            .collect();
        assert!(unresolved.is_empty(),
            "`Fetcher is proxy-based` (unary FT with `+` marker) must resolve as antecedent. Full diags: {:#?}", diags);
    }

    /// #274 Category A — unary derivation-rule consequent (no separate FT
    /// declaration, the rule itself introduces the unary) used as an
    /// antecedent in another rule. Mirrors the `Customer is eligible for
    /// trial` pattern in website.md.
    #[test]
    fn category_a_unary_rule_consequent_as_antecedent() {
        let input = "Customer(.Id) is an entity type.\n\
                     Plan(.Name) is an entity type.\n\
                     Invoice(.Id) is an entity type.\n\
                     ## Fact Types\n\
                     Customer has Plan.\n\
                     Customer receives Invoice.\n\
                     ## Derivation Rules\n\
                     Customer is eligible for trial if Customer has Plan 'Free'.\n\
                     Customer receives Invoice if Customer is eligible for trial.";
        let diags = check_readings(input);
        let unresolved: Vec<_> = diags.iter()
            .filter(|d| d.source == Source::Resolve && d.level == Level::Warning)
            .filter(|d| d.message.contains("is eligible for trial"))
            .collect();
        assert!(unresolved.is_empty(),
            "`Customer is eligible for trial` (unary rule consequent) must resolve as antecedent. Full diags: {:#?}", diags);
    }

    /// #275 Category C — `<Noun> is '<literal>'` and `<Noun> is not
    /// '<literal>'` are ref-scheme-value filters that should resolve.
    /// 13+ rules in auto.dev (`Source is 'oem'`, `Email Template is
    /// 'limit-50'`) and widespread elsewhere.
    #[test]
    fn category_c_parameter_atom_in_rule_body() {
        let input = "Source(.Source Name) is an entity type.\n\
                     ## Fact Types\n\
                     Source has priority over Source.\n\
                     ## Derivation Rules\n\
                     Source has priority over Source if Source is 'oem' and other Source is not 'oem'.";
        let diags = check_readings(input);
        let unresolved: Vec<_> = diags.iter()
            .filter(|d| d.source == Source::Resolve && d.level == Level::Warning)
            .filter(|d| d.message.contains(" is 'oem'") || d.message.contains(" is not 'oem'"))
            .collect();
        assert!(unresolved.is_empty(),
            "`Source is 'oem'` / `Source is not 'oem'` must resolve as ref-scheme-value filters. Full diags: {:#?}", diags);
    }

    /// #275 Category C — `<Noun> is '<literal>'` on a named entity with
    /// a ref scheme. Mirrors `Email Template is 'limit-50'` from
    /// website.md.
    #[test]
    fn category_c_ref_scheme_literal_on_named_entity() {
        let input = "Email Template(.Name) is an entity type.\n\
                     Notification(.Id) is an entity type.\n\
                     ## Fact Types\n\
                     Notification is triggered by Email Template.\n\
                     ## Derivation Rules\n\
                     Notification is triggered by Email Template if Email Template is 'limit-50'.";
        let diags = check_readings(input);
        let unresolved: Vec<_> = diags.iter()
            .filter(|d| d.source == Source::Resolve && d.level == Level::Warning)
            .filter(|d| d.message.contains("Email Template is 'limit-50'"))
            .collect();
        assert!(unresolved.is_empty(),
            "`Email Template is 'limit-50'` must resolve as ref-scheme-value filter. Full diags: {:#?}", diags);
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

    /// Regression: robocall-service derivation rules use two antecedent
    /// shapes the resolver previously didn't classify, producing
    /// "antecedent clause did not resolve" warnings:
    ///   - Subtype instance check: `Robocall is an Autodialed Call`
    ///     where Autodialed Call is a declared subtype of Robocall.
    ///   - Word comparator: `Actual Damages Amount exceeds Per Violation Amount`
    ///     where both sides reference declared value types.
    /// Both now resolve via the new branches (7) and (8) in
    /// resolve_derivation_rule.
    #[test]
    fn subtype_check_and_word_comparator_antecedents_resolve() {
        let input = "\
Robocall(.id) is an entity type.
Autodialed Call is a subtype of Robocall.
Prerecorded Call is a subtype of Robocall.
TCPA Violation(.id) is an entity type.
Actual Damages Amount is a value type.
Per Violation Amount is a value type.
## Fact Types
TCPA Violation is for Robocall.
## Derivation Rules
+ TCPA Violation is for Robocall if Robocall is an Autodialed Call.
+ TCPA Violation is for Robocall if Robocall is a Prerecorded Call.
It is permitted that claim Actual Damages Amount if Actual Damages Amount exceeds Per Violation Amount.
";
        let diags = check_readings(input);
        let unresolved: Vec<_> = diags.iter()
            .filter(|d| d.level == Level::Warning
                && d.message.contains("antecedent clause did not resolve"))
            .collect();
        assert!(unresolved.is_empty(),
            "subtype-check / word-comparator antecedents must resolve; got {:?}",
            unresolved.iter().map(|d| &d.message).collect::<Vec<_>>());
    }

    /// #214: check_readings must run through `apply(check_readings_func, …)`.
    /// The Func-tree result, decoded, must equal the direct check output.
    /// Also pins down the structural shape of the top-level Func so a
    /// future refactor can't quietly degrade it back to Rust control flow.
    #[test]
    fn check_readings_func_produces_same_diagnostics_as_api() {
        let input = "\
Person(.Id) is an entity type.\n\
## Fact Types\n\
Person is parent of Person.\n\
";
        // Public API output.
        let via_api = check_readings(input);

        // Direct Func application.
        let state = parse_to_state(input).expect("parse");
        let obj = crate::ast::apply(&check_readings_func(), &state, &state);
        let via_func = decode_diags(&obj);

        assert_eq!(via_api.len(), via_func.len(),
            "Func-driven and API-driven diagnostic counts must agree: api={:?} func={:?}",
            via_api, via_func);
        for (a, f) in via_api.iter().zip(via_func.iter()) {
            assert_eq!(a.level, f.level);
            assert_eq!(a.source, f.source);
            assert_eq!(a.reading, f.reading);
            assert_eq!(a.message, f.message);
        }
    }

    /// #273: legal / prose-heavy corpora often mention a declared noun
    /// in lowercase inside a derivation's antecedent (e.g. "… if
    /// customer ordered Product" against a declared `Customer ordered
    /// Product` fact type). The resolver must tolerate this case drift
    /// without falling back to "antecedent clause did not resolve".
    #[test]
    fn prose_tolerant_lowercase_noun_in_antecedent() {
        let input = "\
Customer(.id) is an entity type.
Product(.id) is an entity type.
Review(.id) is an entity type.
## Fact Types
Customer ordered Product.
Customer wrote Review.
## Derivation Rules
+ Customer wrote Review if customer ordered Product.
";
        let diags = check_readings(input);
        let unresolved: Vec<_> = diags.iter()
            .filter(|d| d.level == Level::Warning
                && d.message.contains("antecedent clause did not resolve"))
            .collect();
        assert!(unresolved.is_empty(),
            "lowercase noun mention in antecedent must resolve; got {:?}",
            unresolved.iter().map(|d| &d.message).collect::<Vec<_>>());
    }

    /// #273: antecedents naturally spell out articles — "the Tool",
    /// "a Party", "an Exemption" — that the resolver needs to see
    /// past. Stripping leading determiners before noun-tuple lookup
    /// keeps the match working without giving up word-boundary
    /// safety inside the rest of the clause.
    #[test]
    fn prose_tolerant_articles_in_antecedent() {
        let input = "\
Customer(.id) is an entity type.
Product(.id) is an entity type.
Review(.id) is an entity type.
## Fact Types
Customer ordered Product.
Customer wrote Review.
## Derivation Rules
+ Customer wrote Review if the Customer ordered a Product.
";
        let diags = check_readings(input);
        let unresolved: Vec<_> = diags.iter()
            .filter(|d| d.level == Level::Warning
                && d.message.contains("antecedent clause did not resolve"))
            .collect();
        assert!(unresolved.is_empty(),
            "article-prefixed nouns in antecedent must resolve; got {:?}",
            unresolved.iter().map(|d| &d.message).collect::<Vec<_>>());
    }

    #[test]
    fn check_readings_func_top_level_is_concat_of_construction() {
        // Structural assertion — the top-level Func must remain
        // Concat ∘ Construction([…]) with exactly 5 layers. This is
        // the paper-aligned shape (Backus Concat + Construction).
        let func = check_readings_func();
        match &func {
            Func::Compose(outer, inner) => {
                assert!(matches!(**outer, Func::Concat),
                    "top-level must compose Concat onto the construction");
                match &**inner {
                    Func::Construction(layers) => assert_eq!(layers.len(), 5,
                        "check_readings_func must expose exactly 5 layer Funcs"),
                    other => panic!("inner must be Construction, got {:?}", other),
                }
            }
            other => panic!("top-level Func shape broke: {:?}", other),
        }
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
