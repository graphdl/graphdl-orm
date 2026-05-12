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
///   check_readings_func = Concat ∘ [ layer₁, layer₂, layer₃, layer₄ ]
///
/// The composition is explicit FFP; layer bodies stay Native for now
/// because several read multiple cells and format messages. Future
/// work (#214 cont.) can lower each layer body into `ApplyToAll`,
/// `Filter`, `Construction`, and binding-extract primitives.
///
/// MC4b (#751): the singular-naming layer moved out — it is now a
/// deontic constraint in `readings/core/core.md` that flows through
/// the validate dispatch and surfaces as a `Violation` rather than a
/// `ReadingDiagnostic`. The Rust heuristic that lived here is gone.
pub fn check_readings_func() -> Func {
    Func::compose(
        Func::Concat,
        Func::construction(vec![
            layer_native(check_unresolved_clauses),
            layer_native(check_ring_validity),
            layer_native(check_ring_completeness),
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

/// Layer 1: unresolved antecedent analysis.
///
/// A real ρ-application over three cells — `UnresolvedClause`,
/// `FactType`, and `Noun` — not a string echo of the parser's raw
/// output. For each unresolved clause the parser flagged, this
/// layer independently re-inspects the clause and reports which
/// declared fact types share nouns with it. That grounds the
/// suggestion in the current schema rather than a static string,
/// so authors see the candidate FTs they could have meant. Per the
/// paper's §Distributed Evaluation, diagnostics are pure functions
/// of the cell state; this keeps them that way.
fn check_unresolved_clauses(state: &Object) -> Vec<ReadingDiagnostic> {
    let fact_types = fetch_or_phi("FactType", state);
    let nouns = fetch_or_phi("Noun", state);
    let noun_names: Vec<String> = nouns.as_seq()
        .map(|facts| facts.iter()
            .filter_map(|n| binding(n, "name").map(String::from))
            .collect())
        .unwrap_or_default();
    fetch_or_phi("UnresolvedClause", state).as_seq()
        .map(|facts| facts.iter().map(|f| {
            let clause = binding(f, "clause").unwrap_or("");
            let reading = binding(f, "ruleText").unwrap_or("");
            let suggestion = suggest_similar_fact_types(clause, &noun_names, &fact_types);
            ReadingDiagnostic {
                line: 0,
                reading: reading.to_string(),
                level: Level::Warning,
                source: Source::Resolve,
                message: format!(
                    "antecedent clause did not resolve to a declared fact type: `{}`",
                    clause,
                ),
                suggestion: Some(suggestion),
            }
        }).collect())
        .unwrap_or_default()
}

/// Join `clause` against the `FactType` cell: for each FT whose
/// reading shares at least one declared noun with the clause,
/// surface it as a candidate. Paper Eq. 11's demux form — `Filter`
/// the FT sequence on noun-overlap with the offending clause.
fn suggest_similar_fact_types(
    clause: &str,
    noun_names: &[String],
    fact_types: &Object,
) -> String {
    let clause_nouns: Vec<&str> = noun_names.iter()
        .filter(|n| clause.contains(n.as_str()))
        .map(String::as_str)
        .collect();
    if clause_nouns.is_empty() {
        return "check that the clause references a declared fact type, or uses a recognised form (comparison, aggregate, computed binding)".to_string();
    }
    let candidates: Vec<String> = fact_types.as_seq()
        .map(|fts| fts.iter()
            .filter_map(|ft| binding(ft, "reading").map(String::from))
            .filter(|reading| clause_nouns.iter().any(|n| reading.contains(n)))
            .take(3)
            .collect())
        .unwrap_or_default();
    match candidates.is_empty() {
        true => format!(
            "the clause mentions {} but no declared fact type spans those nouns yet",
            clause_nouns.join(", "),
        ),
        false => format!(
            "did you mean one of: {}?",
            candidates.join("; "),
        ),
    }
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
/// Suppression patterns are now read from the `Constraint` cell as a
/// `permitted` deontic permission (see `readings/core/validation.md`
/// ` It is permitted that a Fact Type has no Constraint … when the
/// Reading … contains a capitalized-word-prefixed form of its Ring
/// Noun, or when some Noun ending in that Ring Noun is declared
/// elsewhere in the corpus.`). `RingCompletenessSuppression::from_state`
/// reads the permission and enables the two pattern matchers
/// accordingly; if no permission is registered (e.g. a bare
/// `check_readings(user_text)` call with no metamodel context) it
/// falls back to `boot()` which preserves the legacy behaviour.
fn check_ring_completeness(state: &Object) -> Vec<ReadingDiagnostic> {
    let ft_cell = fetch_or_phi("FactType", state);
    let role_cell = fetch_or_phi("Role", state);
    let constraint_cell = fetch_or_phi("Constraint", state);
    let noun_names: Vec<String> = fetch_or_phi("Noun", state).as_seq()
        .map(|ns| ns.iter()
            .filter_map(|n| binding(n, "name").map(|s| s.to_string()))
            .collect())
        .unwrap_or_default();
    let suppression = RingCompletenessSuppression::from_state(state);

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

            // Suppression read from the validation.md permission. Each
            // enabled pattern matcher gets a chance to suppress; ring
            // hints are advisory and false positives from tokenization
            // are strictly worse than a missed hint.
            let reading = binding(ft, "reading").unwrap_or("");
            if suppression.suppresses(reading, ring_noun, &noun_names) {
                return None;
            }

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

/// #865: ring-completeness suppression as a typed table, read from the
/// `Constraint` cell as a `permitted` deontic permission declared in
/// `readings/core/validation.md`. Replaces the hand-rolled byte-walker
/// and inline suppression layers in `check_ring_completeness`.
///
/// Two pattern matchers compose into the suppression. The cell text
/// names each matcher via a sentinel substring; `from_state` enables
/// the matcher iff the corresponding permission constraint is present
/// in the cell. The `boot()` variant enables both — preserving the
/// legacy behaviour for callers that parse raw user text without the
/// metamodel context (so the historic compound-noun suppression keeps
/// working in `check_readings(user_text)`).
///
/// Future patterns: extend `RingCompletenessSuppression`, add another
/// sentinel substring + matcher, and add another `It is permitted that
/// … <new-sentinel> …` reading. No Rust change to `check_ring_completeness`.
#[derive(Debug, Clone)]
struct RingCompletenessSuppression {
    /// Pattern: the stored FT reading contains a `<Capitalized>
    /// <ring_noun>` pair — evidence that at least one role was a
    /// compound noun the parser's longest-first noun matcher missed.
    match_capitalized_prefix: bool,
    /// Pattern: a noun ending in `<ring_noun>` is declared elsewhere —
    /// e.g. `Biometric Data` next to `Data(.id)`. The corpus-wide
    /// ambiguity is enough to suppress the ring hint.
    match_compound_suffix_declared: bool,
}

/// Sentinel substrings the permission text MUST contain for each
/// pattern matcher to be enabled. Kept in sync with the prose in
/// `readings/core/validation.md` § "Ring Constraint Completeness".
const SENTINEL_CAPITALIZED_PREFIX: &str = "capitalized-word-prefixed";
const SENTINEL_COMPOUND_SUFFIX:    &str = "ending in";

impl RingCompletenessSuppression {
    /// Legacy fallback — both pattern matchers enabled. Matches the
    /// hand-coded behaviour before #865 and keeps `check_readings`
    /// working when the input does not carry the validation.md
    /// permission constraint.
    fn boot() -> Self {
        RingCompletenessSuppression {
            match_capitalized_prefix: true,
            match_compound_suffix_declared: true,
        }
    }

    /// Read suppression patterns from the `Constraint` cell. A `permitted`
    /// deontic constraint whose text carries the sentinel substring for a
    /// pattern enables that matcher. When no permitted-modality ring
    /// suppression constraints are present in the cell at all, fall back
    /// to `boot()` so legacy callers without the metamodel context
    /// continue to behave as before.
    fn from_state(state: &Object) -> Self {
        let constraint_cell = fetch_or_phi("Constraint", state);
        let perm_texts: Vec<&str> = constraint_cell.as_seq()
            .map(|cs| cs.iter()
                .filter(|c| binding(c, "deonticOperator") == Some("permitted"))
                .filter_map(|c| binding(c, "text"))
                // Restrict to the ring-completeness family: every
                // permission in this family mentions ring constraints
                // in its body.
                .filter(|t| t.contains("Constraint Type 'IR'"))
                .collect())
            .unwrap_or_default();
        if perm_texts.is_empty() {
            return Self::boot();
        }
        RingCompletenessSuppression {
            match_capitalized_prefix:
                perm_texts.iter().any(|t| t.contains(SENTINEL_CAPITALIZED_PREFIX)),
            match_compound_suffix_declared:
                perm_texts.iter().any(|t| t.contains(SENTINEL_COMPOUND_SUFFIX)),
        }
    }

    /// True iff some enabled pattern matcher fires on the
    /// `(reading, ring_noun, declared_nouns)` triple. The two matchers
    /// implement the prose in `readings/core/validation.md`:
    ///   1. `match_capitalized_prefix` — reading contains
    ///      `<Capitalized word> <ring_noun>`.
    ///   2. `match_compound_suffix_declared` — some declared noun ends
    ///      in ` <ring_noun>`.
    fn suppresses(&self, reading: &str, ring_noun: &str, nouns: &[String]) -> bool {
        if ring_noun.is_empty() { return false; }
        if self.match_capitalized_prefix
            && reading_contains_capitalized_prefix(reading, ring_noun)
        {
            return true;
        }
        if self.match_compound_suffix_declared
            && nouns.iter().any(|n| noun_ends_with_space_target(n, ring_noun))
        {
            return true;
        }
        false
    }
}

/// True iff `text` contains an occurrence of `target` that is
/// immediately preceded by a Capitalized word (ASCII uppercase
/// followed by at least one lowercase letter) and a single space.
/// Implements the `match_capitalized_prefix` pattern declared in
/// `readings/core/validation.md`.
fn reading_contains_capitalized_prefix(text: &str, target: &str) -> bool {
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
                let word_end = start - 1;
                let mut word_start = word_end;
                while word_start > 0 && bytes[word_start - 1] != b' ' {
                    word_start -= 1;
                }
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

/// True iff `n` ends with ` <target>` — i.e. `<target>` is the bare
/// last word of a compound noun. Implements the
/// `match_compound_suffix_declared` pattern declared in
/// `readings/core/validation.md`.
fn noun_ends_with_space_target(n: &str, target: &str) -> bool {
    !target.is_empty()
        && n.ends_with(target)
        && n.len() > target.len()
        && n.as_bytes()[n.len() - target.len() - 1] == b' '
}

/// MC4b (#751): the singular-naming heuristic ("noun looks like a
/// plural of `<base>y`") moved out of check.rs. It is now expressed
/// as `It is forbidden that each Noun has a name that ends with
/// 'ies'.` in `readings/core/core.md`, compiled by the deontic
/// translator with a per-fact text predicate, and surfaced as a
/// `Violation` through the validate dispatch. The Rust check
/// disappeared with that move.

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

    /// #276 Category G — single-level `that`-relative expansion.
    /// `<head> that <tail>` expands to `<head> and <last_noun_of_head>
    /// <tail>` during antecedent preprocessing so both halves resolve
    /// against declared FTs. Mirrors the `Customer has Country Code
    /// that is an EEA Country Code` pattern from eu-compliance.md.
    #[test]
    fn category_g_single_that_relative_expands() {
        let input = "Customer(.Id) is an entity type.\n\
                     Country Code is a value type.\n\
                     EEA Country Code is a value type.\n\
                     ## Fact Types\n\
                     Customer has Country Code.\n\
                     Country Code is an EEA Country Code.\n\
                     ## Derivation Rules\n\
                     Customer has Country Code if Customer has Country Code that is an EEA Country Code.";
        let diags = check_readings(input);
        let unresolved: Vec<_> = diags.iter()
            .filter(|d| d.source == Source::Resolve && d.level == Level::Warning)
            .filter(|d| d.message.contains("that is an EEA Country Code"))
            .collect();
        assert!(unresolved.is_empty(),
            "`Customer has Country Code that is an EEA Country Code` must expand + resolve. Full diags: {:#?}", diags);
    }

    /// #276 Category G — nested `that`-relative expansion.
    /// `<head1> that <verb> <X> that <verb> <Y>` iteratively expands
    /// into three conjoined clauses. Mirrors `Source Request is for
    /// Resource Declaration that has Base Path` and the deeper chains
    /// in source-routing.md.
    #[test]
    fn category_g_nested_that_relative_expands() {
        let input = "Source Request(.Id) is an entity type.\n\
                     Resource Declaration(.Id) is an entity type.\n\
                     Base Path is a value type.\n\
                     ## Fact Types\n\
                     Source Request is for Resource Declaration.\n\
                     Resource Declaration has Base Path.\n\
                     ## Derivation Rules\n\
                     Source Request is for Resource Declaration if Source Request is for Resource Declaration that has Base Path.";
        let diags = check_readings(input);
        let unresolved: Vec<_> = diags.iter()
            .filter(|d| d.source == Source::Resolve && d.level == Level::Warning)
            .filter(|d| d.message.contains("that has Base Path"))
            .collect();
        assert!(unresolved.is_empty(),
            "`Source Request is for Resource Declaration that has Base Path` must expand + resolve. Full diags: {:#?}", diags);
    }

    /// Layer-1 refactor — the suggestion must name declared fact
    /// types that share nouns with the unresolved clause, not a
    /// static string. Proves the checker joins `UnresolvedClause`
    /// with `FactType` via ρ-application rather than echoing the
    /// parser's output.
    #[test]
    fn unresolved_clause_suggestion_names_similar_fact_types() {
        let input = "Order(.Id) is an entity type.\n\
                     Amount is a value type.\n\
                     Customer(.Id) is an entity type.\n\
                     ## Fact Types\n\
                     Order has Amount.\n\
                     Order has Customer.\n\
                     ## Derivation Rules\n\
                     + Order has Amount if Order has Mystery.";
        let diags = check_readings(input);
        let mystery_warning = diags.iter()
            .find(|d| d.message.contains("Order has Mystery"))
            .expect("expected Order has Mystery warning");
        let suggestion = mystery_warning.suggestion.as_ref().expect("suggestion present");
        assert!(suggestion.contains("Order has Amount") || suggestion.contains("Order has Customer"),
            "suggestion must name declared FT candidates involving `Order`, got {:?}", suggestion);
    }

    /// #277 Category F — `<Noun> has <Noun> within <anaphora>` is
    /// a binary FT reference with an implicit range filter on the
    /// trailing role. Must not fire unresolved. Pattern appears
    /// 3 times in service-health.md.
    #[test]
    fn category_f_range_within_filter() {
        let input = "Log Entry(.Id) is an entity type.\n\
                     Interval(.Id) is an entity type.\n\
                     Timestamp is a value type.\n\
                     ## Fact Types\n\
                     Log Entry has Timestamp.\n\
                     ## Derivation Rules\n\
                     Log Entry has Timestamp if Log Entry has Timestamp within that Interval.";
        let diags = check_readings(input);
        let unresolved: Vec<_> = diags.iter()
            .filter(|d| d.source == Source::Resolve && d.level == Level::Warning)
            .filter(|d| d.message.contains("within that Interval"))
            .collect();
        assert!(unresolved.is_empty(),
            "`Log Entry has Timestamp within that Interval` must resolve (binary FT + range filter). Full diags: {:#?}", diags);
    }

    /// #277 Category F — bare `<Noun> of N or more` / `N or less`
    /// value comparison form. Mirrors the `HTTP Status of 500 or
    /// more` pattern from service-health.md.
    #[test]
    fn category_f_bare_or_more_or_less() {
        let input = "Request(.Id) is an entity type.\n\
                     HTTP Status is a value type.\n\
                     ## Fact Types\n\
                     Request has HTTP Status.\n\
                     ## Derivation Rules\n\
                     Request has HTTP Status if HTTP Status of 500 or more.";
        let diags = check_readings(input);
        let unresolved: Vec<_> = diags.iter()
            .filter(|d| d.source == Source::Resolve && d.level == Level::Warning)
            .filter(|d| d.message.contains("HTTP Status of 500 or more"))
            .collect();
        assert!(unresolved.is_empty(),
            "`HTTP Status of 500 or more` must resolve as a bare-value filter. Full diags: {:#?}", diags);
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
        // Concat ∘ Construction([…]) with exactly 4 layers. This is
        // the paper-aligned shape (Backus Concat + Construction).
        // MC4b (#751) dropped the singular-naming layer; the
        // equivalent diagnostic now flows from the deontic constraint
        // path into the violations stream.
        let func = check_readings_func();
        match &func {
            Func::Compose(outer, inner) => {
                assert!(matches!(**outer, Func::Concat),
                    "top-level must compose Concat onto the construction");
                match &**inner {
                    Func::Construction(layers) => assert_eq!(layers.len(), 4,
                        "check_readings_func must expose exactly 4 layer Funcs"),
                    other => panic!("inner must be Construction, got {:?}", other),
                }
            }
            other => panic!("top-level Func shape broke: {:?}", other),
        }
    }

    #[test]
    fn reading_contains_capitalized_prefix_only_fires_on_compound_nouns() {
        // Positive: "Personal Data" has "Personal" as capitalized prefix.
        assert!(super::reading_contains_capitalized_prefix(
            "Data is processed in manner that ensures appropriate security of Personal Data",
            "Data",
        ));
        // Negative: "Data or Data" — "or" is lowercase, no compound.
        assert!(!super::reading_contains_capitalized_prefix("Data or Data", "Data"));
        // Negative: "Monitoring Body takes Monitoring Body" — `takes` is lowercase.
        assert!(!super::reading_contains_capitalized_prefix(
            "Monitoring Body takes Monitoring Body",
            "Monitoring Body",
        ));
        // Negative: "Data Subject where Data Subject" — `where` is lowercase.
        assert!(!super::reading_contains_capitalized_prefix(
            "Data Subject where Data Subject",
            "Data Subject",
        ));
        // Negative: an acronym like "GDPR Data" — "GDPR" has no lowercase
        // letters, so it doesn't count as a "Capitalized word" for our
        // compound-noun heuristic.
        assert!(!super::reading_contains_capitalized_prefix("GDPR Data processes Data", "Data"));
    }

    /// #865: the ring-completeness suppression now reads its pattern
    /// matchers from the `Constraint` cell as a `permitted` deontic
    /// permission declared in `readings/core/validation.md`. This pin
    /// asserts the read-from-cell mechanism is wired in three ways:
    ///
    ///   1. With no permission in the cell, `from_state` falls back to
    ///      `boot()` (both matchers enabled) so legacy `check_readings`
    ///      callers without metamodel context keep working.
    ///   2. A synthesised state with ONLY the capitalized-prefix
    ///      sentinel enables JUST that matcher (compound-suffix matcher
    ///      stays disabled) — proves the cell-text drives selection.
    ///   3. The validation.md-loaded metamodel state has the permission
    ///      registered: `from_state` enables both matchers from the cell
    ///      rather than via the boot fallback.
    #[test]
    fn ring_completeness_suppression_reads_pattern_set_from_permission_cell() {
        use crate::ast::{Object, fact_from_pairs, cell_push};

        // (1) Empty state → boot fallback: both matchers enabled.
        let empty = Object::phi();
        let s0 = super::RingCompletenessSuppression::from_state(&empty);
        assert!(s0.match_capitalized_prefix && s0.match_compound_suffix_declared,
            "empty state must fall back to boot() with both matchers enabled");

        // (2) Synthesized Constraint cell with ONLY a capitalized-prefix
        // permission. The compound-suffix sentinel ("ending in") is
        // absent from the text, so that matcher must stay off — proving
        // the suppression flags really come from the cell, not from a
        // hardcoded Rust default.
        let perm = fact_from_pairs(&[
            ("id",               "test-perm-cap-only"),
            ("kind",             "UC"),
            ("modality",         "deontic"),
            ("deonticOperator",  "permitted"),
            ("text",             "It is permitted that a Fact Type has no Constraint of Constraint Type 'IR', 'AS', 'AT', 'SY', 'IT', 'TR', or 'AC' spanning its Roles when the Reading contains a capitalized-word-prefixed form of its Ring Noun."),
        ]);
        let synth = cell_push("Constraint", perm, &Object::phi());
        let s1 = super::RingCompletenessSuppression::from_state(&synth);
        assert!( s1.match_capitalized_prefix,
            "capitalized-prefix sentinel in cell text must enable that matcher");
        assert!(!s1.match_compound_suffix_declared,
            "missing 'ending in' sentinel must leave compound-suffix matcher off; got {:?}", s1);

        // (3) End-to-end: parse the bundled metamodel corpus (which
        // includes readings/core/validation.md) and confirm the
        // Constraint cell carries a permission that enables BOTH
        // matchers via `from_state`. This proves the round-trip:
        // validation.md → translate_deontic_constraints → Constraint
        // cell → RingCompletenessSuppression::from_state.
        let metamodel_state = parse_to_state(&crate::metamodel_corpus())
            .expect("metamodel must parse");
        let s2 = super::RingCompletenessSuppression::from_state(&metamodel_state);
        assert!(s2.match_capitalized_prefix && s2.match_compound_suffix_declared,
            "validation.md permission must populate the Constraint cell so both matchers enable; got {:?}", s2);
    }

    /// #750 — parser-level fix for the compound-noun problem. When a
    /// fact-type reading contains a compound noun in role position
    /// (`Personal Data Breach is breach of security leading to loss of
    /// Personal Data`), the parser must produce Role facts whose
    /// `nounName` is the compound noun, not the bare-word suffix.
    ///
    /// Driving readings used here: only `Data(.id)` is explicitly
    /// declared as an entity type. `Personal Data` and
    /// `Personal Data Breach` appear inline with `(.…)` annotations
    /// in role positions; the parser auto-declares them so Stage-1's
    /// longest-first noun matcher can recognize the compound nouns
    /// when tokenizing the FT reading.
    ///
    /// This test asserts the parser-level invariant directly
    /// (Role cell contents and the stored FT reading) so it stays
    /// meaningful even after the `check_ring_completeness` suppression
    /// heuristic at check.rs:373-407 is removed.
    #[test]
    fn compound_noun_inline_id_preserved_in_role_noun_name() {
        let input = "\
Data(.id) is an entity type.
Personal Data Breach(.id) is breach of security leading to loss of Personal Data(.id).
";
        let state = parse_to_state(input).expect("parse");

        // The fact type's two roles must both bind to compound nouns,
        // not the bare-word `Data` suffix.
        let ft_id_for_target: Option<String> = fetch_or_phi("FactType", &state).as_seq()
            .and_then(|fts| fts.iter().find_map(|ft| {
                let reading = binding(ft, "reading").unwrap_or("");
                if reading.contains("breach of security") {
                    binding(ft, "id").map(|s| s.to_string())
                } else { None }
            }));
        let target_id = ft_id_for_target.expect("the breach-of-security FT must register");
        let target_role_nouns: Vec<String> = fetch_or_phi("Role", &state).as_seq()
            .map(|rs| rs.iter()
                .filter(|r| binding(r, "factType").map(|s| s.to_string()) == Some(target_id.clone()))
                .filter_map(|r| binding(r, "nounName").map(|s| s.to_string()))
                .collect())
            .unwrap_or_default();
        assert_eq!(target_role_nouns.len(), 2,
            "binary FT must produce two Role facts; got {:?}",
            target_role_nouns);
        assert!(target_role_nouns.iter().all(|n| n != "Data"),
            "no role of the breach FT may bind to the bare suffix `Data` (compound noun was lost in tokenization); got {:?}",
            target_role_nouns);
        assert!(target_role_nouns.iter().any(|n| n == "Personal Data Breach"),
            "first role must bind to compound noun `Personal Data Breach`; got {:?}",
            target_role_nouns);
        assert!(target_role_nouns.iter().any(|n| n == "Personal Data"),
            "second role must bind to compound noun `Personal Data`; got {:?}",
            target_role_nouns);
    }
}
