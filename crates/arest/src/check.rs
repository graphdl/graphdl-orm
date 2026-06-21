// crates/arest/src/check.rs
//
// Readings checker (#199, #213, #214) — diagnostics as a ρ-application
// over cells.
//
// Per Backus FFP and AREST Theorem 2 / Theorem 5: the checker is a
// Func tree applied via ast::apply, not Rust control flow. Its top
// level is
//
//   check_readings_func = Concat ∘ [ layer₁, …, layer₆ ]
//
// where each layerᵢ reads one or more cells from D and emits a
// sequence of diagnostic Objects. Rust only parses the raw text,
// applies the Func, and decodes the diagnostic sequence back to the
// public `Vec<ReadingDiagnostic>` shape at the API boundary.
//
// The six layer bodies remain Rust functions for now (each wrapped
// in a Func::Native leaf) because they read multiple cells and
// format messages; the composition itself is the Func tree. Further
// FFP lowering can push per-layer logic (`ApplyToAll`, `Filter`,
// `Selector`) down into the leaves over time.

use crate::ast::{Object, binding, fetch_cell_seq, Func};
use crate::parse_forml2::{parse_to_state, find_nouns, parse_role_token};
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
    Object::Map(map.into())
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
///   check_readings_func = Concat ∘ [ layer₁, … , layer₆ ]
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
            layer_native(check_unresolved_instance_facts),
            layer_native(check_ring_validity),
            layer_native(check_ring_completeness),
            layer_native(check_atom_ids),
            layer_native(check_ambiguous_domain_references),
            layer_native(check_computed_bindings_in_multi_antecedent_rules),
            layer_native(check_variable_disjoint_antecedents),
            layer_native(check_effective_widget_agrees_with_most_specific_type),
            layer_native(check_reading_grammar),
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
    let fact_types = fetch_cell_seq("FactType", state);
    let nouns = fetch_cell_seq("Noun", state);
    let noun_names: Vec<String> = nouns.as_seq()
        .map(|facts| facts.iter()
            .filter_map(|n| binding(n, "name").map(String::from))
            .collect())
        .unwrap_or_default();
    fetch_cell_seq("UnresolvedClause", state).as_seq()
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

/// Layer 1b: unresolved instance-fact analysis. Mirrors
/// `check_unresolved_clauses` but for Instance Fact statements whose
/// canonical reading matches no declared fact type — those tuples are
/// mis-filed under their raw verb (see
/// `translate_instance_facts_with_ft_ids` ~line 3459) and never reach
/// the intended fact-type cell, a silent data loss. Surfacing it as a
/// resolve warning gives the author the offending reading + candidate
/// fact types.
fn check_unresolved_instance_facts(state: &Object) -> Vec<ReadingDiagnostic> {
    let fact_types = fetch_cell_seq("FactType", state);
    let nouns = fetch_cell_seq("Noun", state);
    let noun_names: Vec<String> = nouns.as_seq()
        .map(|facts| facts.iter()
            .filter_map(|n| binding(n, "name").map(String::from))
            .collect())
        .unwrap_or_default();
    fetch_cell_seq("UnresolvedInstanceFact", state).as_seq()
        .map(|facts| facts.iter().map(|f| {
            let reading = binding(f, "stmtText").unwrap_or("");
            let verb = binding(f, "verb").unwrap_or("");
            let subject = binding(f, "subjectNoun").unwrap_or("");
            let suggestion = suggest_similar_fact_types(reading, &noun_names, &fact_types);
            ReadingDiagnostic {
                line: 0,
                reading: reading.to_string(),
                level: Level::Warning,
                source: Source::Resolve,
                message: format!(
                    "instance fact did not resolve to a declared fact type (verb `{}` on subject `{}`); the tuple is mis-filed under the raw verb and never reaches the intended fact-type cell",
                    verb, subject,
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
    let constraint_cell = fetch_cell_seq("Constraint", state);
    let role_cell = fetch_cell_seq("Role", state);
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
    let ft_cell = fetch_cell_seq("FactType", state);
    let role_cell = fetch_cell_seq("Role", state);
    let constraint_cell = fetch_cell_seq("Constraint", state);
    let noun_names: Vec<String> = fetch_cell_seq("Noun", state).as_seq()
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
        let constraint_cell = fetch_cell_seq("Constraint", state);
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
    let value_type_nouns: hashbrown::HashSet<String> = fetch_cell_seq("Noun", state).as_seq()
        .map(|ns| ns.iter()
            .filter(|n| binding(n, "objectType") == Some("value"))
            .filter_map(|n| binding(n, "name").map(|s| s.to_string()))
            .collect())
        .unwrap_or_default();

    fetch_cell_seq("InstanceFact", state).as_seq()
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

/// Layer 6 (ns-7, ns-ambiguity-verbalized-reject): reject a bare
/// cross-namespace reference whose head noun is declared in 2+ non-local
/// domains — a genuine ambiguity the cell graph cannot bind to a single
/// noun.
///
/// ns-5 (parse_forml2_stage2::resolve_reference_domains) leaves the
/// SIGNAL on the `Role_Reference_has_Ambiguous_Domain` cell — one fact
/// per `{Role_Reference, Head_Noun, Candidate_Domain}` — but neither
/// rejects nor verbalizes. This layer does both: it groups the
/// per-candidate facts by `Role_Reference` and raises ONE diagnostic per
/// reference whose MESSAGE IS the reading/guidance (per cor:verbalize —
/// the violation text is itself the fix-it instruction): name the head
/// noun, list the colliding domains (sorted, deduped), and show the
/// `<domain>.<Noun>` qualifier (ns-6) the author can pick.
///
/// SCOPE OF THE REJECT: ns-5 flags as ambiguous EVERY bare reference whose
/// head noun is declared in 2+ non-local domains, and this layer rejects
/// every one of them. Same-kind ambiguity is just as genuine as a kind
/// conflict: a bare `Name` declared as a value type in two domains has no
/// single noun to bind to any more than a `value`-vs-`entity` `Order` does.
/// The earlier staging that fired ONLY on a value-vs-entity kind conflict
/// is gone (task `ns-namespace-collision-cleanup`): it existed only to keep
/// the shipping corpus green while it still carried the per-domain duplicate
/// declarations that produced same-kind collisions. That cleanup resolved
/// them — the universal value primitives `id`, `Name`, `Title`, `code` are
/// now declared ONCE in `core` (synthetic ref-scheme shadows defer to it in
/// ns-5), `User` was unified onto one declaration, and the genuinely-distinct
/// `View` references were qualified `view-projection.View` — so the bundled
/// corpus emits NO ambiguity signal and this broadened gate validates it
/// clean. A new same-kind 2-domain collision (in user readings or a future
/// corpus edit) now (correctly) lights up.
///
/// The diagnostic is `Level::Error` + `Source::Resolve`, which the
/// load-time gate (`load_reading_core::validate_loaded_state`) routes to
/// the ALETHIC bucket — a hard reject.
///
/// The cell is empty for any parse without a namespaced collision (every
/// legacy single-domain parse), so this layer is a no-op there.
fn check_ambiguous_domain_references(state: &Object) -> Vec<ReadingDiagnostic> {
    // Group per reference (the unit of one violation): head noun + the
    // set of candidate domains. Iterate refs in sorted order so emitted
    // diagnostics are deterministic regardless of cell/Map order.
    let mut by_ref: alloc::collections::BTreeMap<String, (String, Vec<String>)> =
        alloc::collections::BTreeMap::new();
    for f in crate::ast::cell_facts_iter(&fetch_cell_seq("Role_Reference_has_Ambiguous_Domain", state)) {
        let (Some(rid), Some(noun), Some(dom)) = (
            binding(f, "Role_Reference"),
            binding(f, "Head_Noun"),
            binding(f, "Candidate_Domain"),
        ) else { continue };
        let entry = by_ref.entry(rid.to_string())
            .or_insert_with(|| (noun.to_string(), Vec::new()));
        if !entry.1.iter().any(|d| d == dom) {
            entry.1.push(dom.to_string());
        }
    }
    by_ref.into_iter().map(|(_rid, (noun, mut domains))| {
        domains.sort();
        domains.dedup();
        let defined_in = domains.iter()
            .map(|d| format!("`{}`", d))
            .collect::<Vec<_>>()
            .join(", ");
        // The qualified `<domain>.<Noun>` choices, joined naturally so the
        // last is preceded by `or` (e.g. "`a.N` or `b.N`",
        // "`a.N`, `b.N`, or `c.N`").
        let qualified = join_qualified_choices(&domains, &noun);
        ReadingDiagnostic {
            line: 0,
            reading: noun.clone(),
            level: Level::Error,
            source: Source::Resolve,
            message: format!(
                "`{}` is ambiguous: defined in {}. Qualify it as {}.",
                noun, defined_in, qualified,
            ),
            suggestion: Some(format!(
                "prefix the reference with one of the domains, e.g. `{}.{}`",
                domains.first().map(String::as_str).unwrap_or(""), noun,
            )),
        }
    }).collect()
}

/// Layer 7: computed bindings in multi-antecedent rules
/// (computed-binding-join-silent-empty, arc-agi-3 issue 2).
///
/// Identity/arith computed bindings (`Run is Resource`) are consumed
/// ONLY by the single-antecedent ModusPonens compile branch — the
/// multi-antecedent paths (subscript join, existence-AND) never
/// consult `consequent_computed_bindings`, so the head role never
/// binds and the derived cell is silently EMPTY while the compile
/// reports ok. The worst failure mode for an author: a type-guard
/// antecedent added to a working bridge kills it without a sound.
///
/// Until the join paths learn computed bindings, surface the shape
/// LOUDLY as a Resolve warning with the blessed decomposition: keep
/// the 1-antecedent bridge (identity renames are now TYPED — the
/// membership guard restricts to the head noun's population, so the
/// guard antecedent that motivated the multi-antecedent form is no
/// longer needed for typing) and move any remaining condition into a
/// downstream positive join over the bridged cell.
///
/// Warning, not Error: an Error-level Resolve diagnostic is alethic
/// to `validate_loaded_state` and would REJECT existing apps that
/// carry the (already-inert) shape on their next recompile. The
/// complaint this answers is the SILENCE, not the load.
fn check_computed_bindings_in_multi_antecedent_rules(state: &Object) -> Vec<ReadingDiagnostic> {
    let data = crate::compile::cell_index_from_state(state);
    data.derivation_rules.iter()
        .filter(|rule| !rule.consequent_computed_bindings.is_empty())
        .filter(|rule| {
            // Count REAL fact-type antecedents; InstancesOfNoun
            // sentinels (subtype lifts) don't make a rule a join.
            let ft_antecedents = rule.antecedent_sources.iter()
                .filter(|s| !s.fact_type_id().is_empty())
                .count();
            ft_antecedents >= 2
        })
        // arc-agi-3 issue 2 (FIXED): a multi-antecedent rule whose computed
        // head-binding is an IDENTITY rename now compiles to the identity-rename
        // bridge-join (compile_explicit_derivation path a''), which DOES consult
        // the binding — it re-keys the consequent to the declared head and keeps
        // antecedent literal filters, materializing a correct, non-empty cell.
        // Such rules no longer derive empty, so the "computed bindings are NOT
        // evaluated" warning would now MISDIRECT (it would push authors toward
        // the no-longer-needed bridge decomposition). Suppress it for exactly the
        // shapes the new branch handles; the remaining shapes (arithmetic CBs, or
        // renames the bridge join can't source/join) still fall to the global
        // existence fallback and STILL warrant the warning.
        .filter(|rule| !crate::compile::identity_rename_bridge_join_applies(&data, rule))
        .map(|rule| {
            let renames = rule.consequent_computed_bindings.iter()
                .map(|cb| format!("`{}`", cb.role))
                .collect::<Vec<_>>()
                .join(", ");
            ReadingDiagnostic {
                line: 0,
                reading: rule.text.clone(),
                level: Level::Warning,
                source: Source::Resolve,
                message: format!(
                    "computed bindings ({renames}) in a multi-antecedent rule are NOT \
                     evaluated — the join paths never consult them, so this rule derives \
                     an EMPTY cell despite compiling clean",
                ),
                suggestion: Some(
                    "split the rule: keep the single-antecedent bridge (identity renames \
                     are typed — emitted facts are restricted to the head noun's \
                     population), then express the extra condition as a separate positive \
                     rule joining over the bridged cell".to_string(),
                ),
            }
        })
        .collect()
}

/// Layer 7b: variable-disjoint antecedent (join-warn-variable-disjoint-antecedent,
/// arc-agi-3 Q1). A positive multi-antecedent (join) rule whose body contains a
/// clause sharing NO noun/variable with any other clause cannot equi-join — the
/// engine forms no cross product over disjoint clauses, so the rule derives an
/// EMPTY cell while the compile reports ok. arc hit this with a unit-cost guard
/// (`… and Count1 is unit`) disjoint from the rotation it meant to grade, and
/// debugged it blind. Surface it LOUDLY as a Resolve warning.
///
/// Conservative by design: flags only a clause whose noun TYPES are absent from
/// EVERY other clause (a genuinely isolated antecedent — almost always a range-
/// restriction slip). Subscripted same-type players (`Glyph1`/`Glyph3`) count as
/// shared because the join planner can link them; a fully disconnected component
/// of size >=2 is not flagged (rare, and avoiding false positives matters more).
/// InstancesOfNoun sentinels and lifted guards carry empty FT ids / noun sets and
/// are skipped. Warning, not Error — the complaint is the SILENCE, not the load.
fn check_variable_disjoint_antecedents(state: &Object) -> Vec<ReadingDiagnostic> {
    let data = crate::compile::cell_index_from_state(state);
    let mut diags = Vec::new();
    for rule in data.derivation_rules.iter() {
        let ft_ids: Vec<&str> = rule.antecedent_sources.iter()
            .map(|s| s.fact_type_id())
            .filter(|id| !id.is_empty())
            .collect();
        if ft_ids.len() < 2 { continue; }
        let noun_sets: Vec<Vec<String>> = ft_ids.iter()
            .map(|id| data.fact_types.get(*id)
                .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect::<Vec<_>>())
                .unwrap_or_default())
            .collect();
        // A clause is NOT disjoint if a join_on equi-key or a match_on bridge
        // (subtype<->supertype / cross-role value match) links it to another
        // clause. The name-only test below misses those — it flagged e.g.
        // `Resource belongs to Domain iff Resource is instance of Noun and that
        // Noun belongs to Domain` (Noun<->Function bridged via subtype) as
        // deriving an EMPTY cell when it actually FIRES through the bridge (the
        // perf-hashjoin subtype-bridge path). Treat bridge-linked nouns as
        // shared so the warning fires only on TRULY disjoint clauses (the
        // unit-cost-guard class it was built for).
        let bridge_nouns: Vec<&str> = rule.match_on.iter()
            .flat_map(|(l, r)| [l.as_str(), r.as_str()])
            .chain(rule.join_on.iter().map(|k| k.as_str()))
            .collect();
        let disjoint: Vec<&str> = (0..ft_ids.len())
            .filter(|&i| {
                !noun_sets[i].is_empty()
                    && !noun_sets[i].iter().any(|n|
                        noun_sets.iter().enumerate()
                            .any(|(j, other)| j != i && other.contains(n)))
                    && !noun_sets[i].iter().any(|n| bridge_nouns.contains(&n.as_str()))
            })
            .map(|i| ft_ids[i])
            .collect();
        if disjoint.is_empty() { continue; }
        let names = disjoint.iter().map(|id| format!("`{id}`"))
            .collect::<Vec<_>>().join(", ");
        diags.push(ReadingDiagnostic {
            line: 0,
            reading: rule.text.clone(),
            level: Level::Warning,
            source: Source::Resolve,
            message: format!(
                "antecedent {names} shares no variable with the rest of the rule body \
                 (variable-disjoint) — the equi-join has no key linking it, so this rule \
                 derives an EMPTY cell (FORML2 forms no cross product over disjoint clauses)",
            ),
            suggestion: Some(
                "connect it by sharing a noun/variable with another clause, or fold the \
                 constant onto the related fact type (cost-on-the-relation) rather than a \
                 disjoint guard".to_string(),
            ),
        });
    }
    diags
}

/// audit-entity-datatype Phase 2(c) — widget-agreement drift, layer 8.
///
/// The Phase-2(b) machinery resolves each noun's EFFECTIVE Component
/// Role most-specific-source-first (explicit `Noun prefers Component
/// Role` pin > the noun's Format's implication > its Conceptual Data
/// Type's implication). This layer is the BELT: it re-derives what the
/// hierarchy IMPLIES from the current schema cells and warns when the
/// persisted `Noun_has_Effective_Component_Role` row DISAGREES — the
/// stale-effective shape left behind when a Format/CDT/pin edit lands
/// without the widget cell re-deriving (or when a cell was written by
/// hand). Agreement is checked only for nouns that HAVE both an
/// effective row and a derivable implication; nouns outside the widget
/// vocabulary stay silent.
fn check_effective_widget_agrees_with_most_specific_type(state: &Object) -> Vec<ReadingDiagnostic> {
    use crate::ast::{binding, cell_facts_iter, fetch_or_phi};
    let pair_map = |cell: &str, k1: &str, k2: &str| -> hashbrown::HashMap<String, String> {
        let c = fetch_or_phi(cell, state);
        cell_facts_iter(&c)
            .filter_map(|f| Some((
                binding(f, k1)?.to_string(),
                binding(f, k2)?.to_string(),
            )))
            .collect()
    };
    let effective = pair_map("Noun_has_Effective_Component_Role", "Noun", "Component Role");
    if effective.is_empty() { return Vec::new(); }
    let prefers     = pair_map("Noun_prefers_Component_Role", "Noun", "Component Role");
    let noun_format = pair_map("Noun_has_Format", "Noun", "Format");
    let noun_cdt    = pair_map("Noun_has_Conceptual_Data_Type", "Noun", "Conceptual Data Type");
    let fmt_implies = pair_map("Format_implies_Component_Role", "Format", "Component Role");
    let cdt_implies = pair_map("Conceptual_Data_Type_implies_Component_Role",
        "Conceptual Data Type", "Component Role");

    let mut diags: Vec<ReadingDiagnostic> = effective.iter()
        .filter_map(|(noun, eff)| {
            let implied = prefers.get(noun)
                .or_else(|| noun_format.get(noun).and_then(|f| fmt_implies.get(f)))
                .or_else(|| noun_cdt.get(noun).and_then(|c| cdt_implies.get(c)))?;
            if implied == eff { return None; }
            Some(ReadingDiagnostic {
                line: 0,
                reading: format!("{} has effective Component Role '{}'", noun, eff),
                level: Level::Warning,
                source: Source::Resolve,
                message: format!(
                    "effective Component Role '{}' for noun `{}` disagrees with its \
                     most-specific type's implication '{}' (pin > Format > Conceptual \
                     Data Type) — the widget cell is stale relative to the schema",
                    eff, noun, implied,
                ),
                suggestion: Some(
                    "recompile the app so Noun_has_Effective_Component_Role re-derives, \
                     or declare an explicit `Noun prefers Component Role` pin if the \
                     divergence is intentional".to_string(),
                ),
            })
        })
        .collect();
    // Deterministic order regardless of HashMap iteration.
    diags.sort_by(|a, b| a.reading.cmp(&b.reading));
    diags
}

/// Layer 9 (operating rule `grammatical-fact-readings`): advisory grammar
/// lint on atomic fact-type readings. The operating rule says "atomic fact
/// type readings must be proper grammatical English"; a coined/abbreviated
/// predicate token (`gputs`, `ghit`, `sharekey`, `sameinput`) in the verb
/// phrase silently violates it. This layer isolates each reading's
/// VERB-PHRASE tokens (the whitespace tokens NOT inside any declared-noun
/// span) and flags any token that is not a recognised English word.
///
/// ADVISORY ONLY — `Level::Hint`. It must never reject a load: the
/// load-time gate (`validate_loaded_state`) routes only Error-level
/// diagnostics to the alethic bucket, so a Hint here adds a note and
/// never blocks or fails a compile. False positives from a coinage the
/// substrate legitimately uses are corrected by adding the word to
/// `READING_LEXICON`, not by silencing the layer.
fn check_reading_grammar(state: &Object) -> Vec<ReadingDiagnostic> {
    let noun_names: Vec<String> = fetch_cell_seq("Noun", state).as_seq()
        .map(|ns| ns.iter()
            .filter_map(|n| binding(n, "name").map(|s| s.to_string()))
            .collect())
        .unwrap_or_default();

    fetch_cell_seq("FactType", state).as_seq()
        .map(|fts| fts.iter().filter_map(|ft| {
            let reading = binding(ft, "reading").unwrap_or("");
            let suspects = suspect_reading_tokens(reading, &noun_names);
            (!suspects.is_empty()).then(|| {
                // One diagnostic per reading, listing every suspect token,
                // so a multi-coinage reading reads cleanly in the report.
                let listed = suspects.iter()
                    .map(|t| format!("'{}'", t))
                    .collect::<Vec<_>>()
                    .join(", ");
                ReadingDiagnostic {
                    line: 0,
                    reading: reading.to_string(),
                    level: Level::Hint,
                    source: Source::Resolve,
                    message: format!(
                        "reading token {} is not recognized English — atomic fact type \
                         readings should be grammatical English (operating rule \
                         grammatical-fact-readings); prefer a full phrase over a coined \
                         predicate",
                        listed,
                    ),
                    suggestion: None,
                }
            })
        }).collect())
        .unwrap_or_default()
}

/// The verb-phrase tokens of `reading` that are not recognised English.
///
/// Algorithm (per the `grammatical-fact-readings` operating rule):
///   1. Locate every declared-noun span with `find_nouns` (case-insensitive,
///      longest-first). These are the role players — never verb tokens.
///   2. Split `reading` on whitespace; keep only tokens that fall ENTIRELY
///      outside every noun span (the verb phrase).
///   3. For each such token, strip a trailing digit subscript via
///      `parse_role_token` and lowercase the base. Skip it when it is too
///      short (`< 3`), not purely alphabetic, or present in `READING_LEXICON`.
///   4. Whatever remains is a coined/abbreviated predicate — return it.
fn suspect_reading_tokens(reading: &str, noun_names: &[String]) -> Vec<String> {
    let noun_spans = find_nouns(reading, noun_names);
    let inside_noun = |start: usize, end: usize| -> bool {
        noun_spans.iter().any(|&(ns, ne, _)| start < ne && end > ns)
    };
    let mut out: Vec<String> = Vec::new();
    let bytes = reading.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // Skip ASCII whitespace.
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // Token = run up to the next ASCII whitespace.
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let end = i;
        if inside_noun(start, end) {
            continue;
        }
        let token = &reading[start..end];
        let base = parse_role_token(token).0;
        let w = base.to_lowercase();
        if w.len() < 3 {
            continue;
        }
        if !w.chars().all(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        if READING_LEXICON.contains(&w.as_str()) {
            continue;
        }
        out.push(w);
    }
    out
}

/// Common English words that legitimately appear in atomic fact-type
/// readings — function words plus the grammatical reading verbs/nouns the
/// substrate actually uses. A verb-phrase token NOT in this set (after
/// subscript-strip + lowercasing, length ≥ 3, purely alphabetic) is a
/// suspect coinage that `check_reading_grammar` flags as a Hint.
///
/// This set must contain ONLY grammatical English. Never add a coined
/// predicate (`gputs`, `ghit`, `sharekey`, `sameinput`, `sourcematch`, …)
/// here — those are exactly what the lint exists to surface. Extend it
/// only with genuine function words / real reading verbs as new corpora
/// introduce them.
static READING_LEXICON: &[&str] = &[
    // articles / determiners / conjunctions / prepositions
    "a", "an", "the", "of", "to", "at", "in", "on", "by", "with", "for",
    "from", "into", "onto", "over", "under", "per", "as", "and", "or",
    "not", "no", "than", "then", "that", "this", "these", "those", "where",
    "when", "which", "who", "whose",
    // copulas
    "is", "are", "was", "were", "be", "been", "being", "am",
    // have / do / modals
    "has", "have", "had", "having", "do", "does", "did", "can", "could",
    "may", "might", "must", "shall", "should", "will", "would",
    // common reading verbs / nouns the substrate uses (grammatical — KEEP)
    "includes", "include", "predicts", "predict", "agrees", "agree",
    "spans", "span", "fits", "fit", "considers", "consider", "reads",
    "read", "writes", "write", "relabels", "relabel", "maps", "map",
    "gathers", "gather", "tallies", "tally", "recolors", "recolor",
    "bends", "bend", "sources", "source", "selects", "select", "wins",
    "win", "ranks", "rank", "solved", "done", "value", "values", "count",
    "counts", "total", "size", "name", "names", "id", "key", "keys",
    "input", "output", "mode", "feature", "confidence", "training", "top",
    "set", "same", "shares", "share", "amount",
];

/// Render the `<domain>.<Noun>` qualifier choices as a natural English
/// list with `or` before the final item. `domains` is assumed sorted +
/// deduped by the caller.
fn join_qualified_choices(domains: &[String], noun: &str) -> String {
    let forms: Vec<String> = domains.iter()
        .map(|d| format!("`{}.{}`", d, noun))
        .collect();
    match forms.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, [first])) => format!("{} or {}", first, last),
        Some((last, head)) => format!("{}, or {}", head.join(", "), last),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::fetch_or_phi;

    #[test]
    fn clean_readings_produce_no_diagnostics() {
        let input = "Order(.Order Id) is an entity type.\n## Fact Types\nOrder has Amount.\n## Instance Facts\nOrder 'ord-1' has Amount '100'.";
        let diags = check_readings(input);
        assert!(diags.is_empty(), "expected no diagnostics, got {:?}", diags);
    }

    // ── variable-disjoint antecedent (join-warn-variable-disjoint-antecedent) ──

    #[test]
    fn variable_disjoint_antecedent_warns() {
        // arc's footgun: a guard (`Count1 steps to Count2`) shares no noun with
        // the rotation it grades, so the join can't link it -> empty cell.
        let input = "\
Glyph(.id) is an entity type.\n\
Count(.id) is an entity type.\n\
\n\
## Fact Types\n\
Glyph rotates to Glyph.\n\
Count steps to Count.\n\
Glyph reaches Glyph at Count.\n\
\n\
## Derivation Rules\n\
* Glyph1 reaches Glyph2 at Count1 iff Glyph1 rotates to Glyph2 and Count1 steps to Count2.\n";
        let diags = check_readings(input);
        let hits: Vec<_> = diags.iter()
            .filter(|d| d.message.contains("variable-disjoint")).collect();
        assert_eq!(hits.len(), 1,
            "expected exactly one variable-disjoint warning; got {:?}", diags);
        assert_eq!(hits[0].level, Level::Warning);
        assert!(hits[0].message.contains("Count_steps_to_Count"),
            "warning names the disjoint antecedent; got {}", hits[0].message);
    }

    #[test]
    fn connected_antecedents_do_not_warn() {
        // Both antecedents share the Glyph type (joined on the intermediate),
        // so the rule body is connected -> no variable-disjoint warning.
        let input = "\
Glyph(.id) is an entity type.\n\
\n\
## Fact Types\n\
Glyph rotates to Glyph.\n\
Glyph reaches Glyph.\n\
\n\
## Derivation Rules\n\
* Glyph1 reaches Glyph2 iff Glyph1 rotates to Glyph3 and Glyph3 reaches Glyph2.\n";
        let diags = check_readings(input);
        assert!(diags.iter().all(|d| !d.message.contains("variable-disjoint")),
            "connected antecedents must not warn; got {:?}", diags);
    }

    // ── computed-binding-join-silent-empty (arc-agi-3 issue 2) ──────

    /// arc-agi-3 issue 2 FIXED: the identity-rename bridge-join shape
    /// (`Run has Game State iff … Game State is Status and Run is Resource and
    /// the Run plays some Game`) now compiles to a correct, non-empty cell via
    /// `compile_explicit_derivation` path a'' — the computed head-bindings ARE
    /// consulted (re-keyed to the declared head, antecedent literals preserved).
    /// So the "computed bindings are NOT evaluated" warning must NO LONGER fire
    /// for it (firing would push authors toward a decomposition they no longer
    /// need). Pinned by the engine-level materialization test
    /// `compile::schema_tests::status_bridge_consumer_form_diagnostic_records_engine_behavior`
    /// (FORM B) + the focused `computed_head_rename_keys_declared_head_and_keeps_literal`.
    #[test]
    fn identity_rename_bridge_join_multi_antecedent_no_longer_warns() {
        let input = r#"# Test
Resource(.Reference) is an entity type.
Reference is a value type.
Status is a value type.
Run(.id) is an entity type.
id is a value type.
Game is a value type.
Game State is a value type.

## Fact Types
Resource is currently in Status.
Run plays Game.
Run has Game State.

## Derivation Rules
* Run has Game State iff that Resource is currently in some Status and Game State is Status and Run is Resource and the Run plays some Game.
"#;
        let diags = check_readings(input);
        assert!(diags.iter().all(|d| !d.message.contains("multi-antecedent rule are NOT")),
            "the identity-rename bridge-join shape now compiles correctly (path a'') \
             — the dead-computed-binding warning must be suppressed; got {:?}", diags);
    }

    /// …but a multi-antecedent rule whose computed binding the bridge join can
    /// NOT consume — here an ARITHMETIC binding (`Total is A + B`), not an
    /// identity rename — STILL falls to the global existence fallback and
    /// derives empty, so the warning must STILL fire for it. Keeps the LOUD
    /// signal alive for the shapes the fix does not cover.
    #[test]
    fn arithmetic_computed_binding_in_multi_antecedent_rule_still_warns() {
        // Two antecedents share `Box` (so no variable-disjoint warning), and the
        // computed binding `Total is Width + Height` is ARITHMETIC, not an
        // identity rename — so `identity_rename_bridge_join_applies` is false and
        // the dead-computed-binding warning still fires.
        let input = r#"# Test
Box(.id) is an entity type.
id is a value type.
Width is a value type.
Height is a value type.
Total is a value type.

## Fact Types
Box has Width.
Box has Height.
Box has Total.

## Derivation Rules
* Box has Total iff Box has Width and Box has Height and Total is Width + Height.
"#;
        let diags = check_readings(input);
        let hits: Vec<&ReadingDiagnostic> = diags.iter()
            .filter(|d| d.message.contains("multi-antecedent rule are NOT"))
            .collect();
        assert_eq!(hits.len(), 1,
            "an arithmetic computed binding in a multi-antecedent rule is NOT \
             handled by the identity-rename bridge join, so it must still warn; \
             got {:?}", diags);
        assert_eq!(hits[0].level, Level::Warning);
        assert_eq!(hits[0].source, Source::Resolve);
        assert!(hits[0].message.contains("`Total`"),
            "the warning names the dead binding; got {}", hits[0].message);
        assert!(hits[0].suggestion.as_deref().unwrap_or("").contains("single-antecedent bridge"),
            "the suggestion names the blessed decomposition; got {:?}", hits[0].suggestion);
    }

    /// The blessed 1-antecedent bridge stays silent — computed
    /// bindings there ARE evaluated (the ModusPonens branch).
    #[test]
    fn computed_bindings_in_single_antecedent_bridge_do_not_warn() {
        let input = r#"# Test
Resource(.Reference) is an entity type.
Reference is a value type.
Status is a value type.
Task(.id) is an entity type.
id is a value type.
Task Status is a value type.

## Fact Types
Resource is currently in Status.
Task has Task Status.

## Derivation Rules
* Task has Task Status iff that Resource is currently in some Status and Task Status is Status and Task is Resource.
"#;
        let diags = check_readings(input);
        assert!(diags.iter().all(|d| !d.message.contains("multi-antecedent rule are NOT")),
            "the single-antecedent bridge is the supported shape — no warning; got {:?}",
            diags);
    }

    /// The bundled metamodel must be free of the shape — otherwise
    /// every app load would warn on framework readings.
    #[test]
    #[cfg(not(feature = "no_std"))]
    fn bundled_metamodel_has_no_dead_computed_binding_rules() {
        let corpus = crate::metamodel_corpus();
        let state = parse_to_state(&corpus).expect("metamodel corpus parses");
        let diags = check_computed_bindings_in_multi_antecedent_rules(&state);
        assert!(diags.is_empty(),
            "bundled metamodel readings must not carry computed bindings in \
             multi-antecedent rules; got {:#?}",
            diags.iter().map(|d| &d.reading).collect::<Vec<_>>());
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

    /// Plural-aware layer-1 membership: the enum-superlative
    /// verbalization quantifies over the PLURAL ("among Tasks that
    /// …"). The bare word-set heuristic flagged `Tasks` as an unknown
    /// Title-case token — a false positive on the tasks-app
    /// recommendation rule, surfaced the moment the check battery
    /// reached apps.compile. A plural of a DECLARED noun must pass;
    /// a plural of an UNDECLARED noun must still flag.
    #[test]
    fn plural_of_declared_noun_in_antecedent_does_not_warn() {
        let input = "Task(.Id) is an entity type.\n\
                     Task Priority is a value type.\n\
                     Task Status is a value type.\n\
                     ## Fact Types\n\
                     Task has Task Priority.\n\
                     Task has Task Status.\n\
                     Task Priority is recommended. +\n\
                     ## Derivation Rules\n\
                     + Task Priority is recommended if some Task has the highest Task Priority among Tasks that have Task Status 'in_progress'.";
        let diags = check_readings(input);
        let plural_flags: Vec<_> = diags.iter()
            .filter(|d| d.source == Source::Resolve && d.level == Level::Warning)
            .filter(|d| d.message.contains("among Tasks"))
            .collect();
        assert!(plural_flags.is_empty(),
            "`Tasks` is the plural of the declared `Task` — the superlative \
             antecedent must not flag. Full diags: {:#?}", diags);

        let undeclared = "Order(.Id) is an entity type.\n\
                          ## Fact Types\n\
                          Order has Amount.\n\
                          ## Derivation Rules\n\
                          + Order has Amount if Order has Amount among Mysteries that exist.";
        let diags2 = check_readings(undeclared);
        assert!(diags2.iter().any(|d| d.source == Source::Resolve
                && d.level == Level::Warning
                && d.message.contains("Mysteries")),
            "an UNDECLARED plural must still flag; got {:#?}", diags2);
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

    /// warn-unmatched-instance-facts — an Instance Fact whose verb
    /// resolves to no declared fact type must surface a resolve warning.
    /// Without it the tuple is silently mis-filed under the raw verb
    /// (translate_instance_facts_with_ft_ids ~line 3459), the 941
    /// data-loss class. The declared `has Amount` fact must NOT warn,
    /// guarding against false positives.
    #[test]
    fn unmatched_instance_fact_verb_warns() {
        let input = "Order(.Id) is an entity type.\n\
                     Amount is a value type.\n\
                     Total is a value type.\n\
                     ## Fact Types\n\
                     Order has Amount.\n\
                     ## Instance Facts\n\
                     Order 'o1' has Amount '5'.\n\
                     Order 'o1' has Total '7'.";
        let diags = check_readings(input);
        // `has Total` matches no declared FT -> must surface a resolve warning.
        assert!(
            diags.iter().any(|d| d.source == Source::Resolve
                && d.reading.contains("Total")
                && d.message.contains("did not resolve to a declared fact type")),
            "expected an unresolved-instance-fact warning for `has Total`. Full diags: {:#?}", diags);
        // `has Amount` IS declared -> it must NOT warn (no false positive).
        assert!(
            !diags.iter().any(|d| d.source == Source::Resolve
                && d.reading.contains("Amount")
                && d.message.contains("did not resolve to a declared fact type")),
            "`has Amount` is declared and must not warn. Full diags: {:#?}", diags);
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
        // Concat ∘ Construction([…]) with exactly 7 layers. This is
        // the paper-aligned shape (Backus Concat + Construction).
        // MC4b (#751) dropped the singular-naming layer; the
        // equivalent diagnostic now flows from the deontic constraint
        // path into the violations stream.
        // computed-binding-join-silent-empty added layer 7 (computed
        // bindings in multi-antecedent rules warn loudly).
        // audit-entity-datatype 2(c) added layer 8 (effective widget
        // agrees with the most-specific type's implication).
        // join-warn-variable-disjoint-antecedent added layer 7b (a
        // variable-disjoint antecedent warns), making 9 total.
        // grammatical-fact-readings added layer 9 (check_reading_grammar:
        // coined verb-phrase tokens warn), making 10 total.
        let func = check_readings_func();
        match &func {
            Func::Compose(outer, inner) => {
                assert!(matches!(**outer, Func::Concat),
                    "top-level must compose Concat onto the construction");
                match &**inner {
                    Func::Construction(layers) => assert_eq!(layers.len(), 10,
                        "check_readings_func must expose exactly 10 layer Funcs"),
                    other => panic!("inner must be Construction, got {:?}", other),
                }
            }
            other => panic!("top-level Func shape broke: {:?}", other),
        }
    }

    /// audit-entity-datatype Phase 2(c): the widget-agreement layer
    /// warns when a noun's persisted effective Component Role disagrees
    /// with what the pin > Format > CDT hierarchy implies — and stays
    /// silent when they agree or when no implication is derivable.
    #[test]
    fn effective_widget_drift_warns_and_agreement_stays_silent() {
        use crate::ast::{cell_push, fact_from_pairs, Object};
        let push = |s: Object, cell: &str, pairs: &[(&str, &str)]|
            cell_push(cell, fact_from_pairs(pairs), &s);

        // Drifted: Email's Format implies text-input, but the effective
        // cell says combo-box (stale after a schema edit).
        let mut state = Object::phi();
        state = push(state, "Noun_has_Effective_Component_Role",
            &[("Noun", "Email"), ("Component Role", "combo-box")]);
        state = push(state, "Noun_has_Format",
            &[("Noun", "Email"), ("Format", "email")]);
        state = push(state, "Format_implies_Component_Role",
            &[("Format", "email"), ("Component Role", "text-input")]);
        // Agreeing: Birthday's CDT implies date-picker and the
        // effective row matches — no diagnostic.
        state = push(state, "Noun_has_Effective_Component_Role",
            &[("Noun", "Birthday"), ("Component Role", "date-picker")]);
        state = push(state, "Noun_has_Conceptual_Data_Type",
            &[("Noun", "Birthday"), ("Conceptual Data Type", "date")]);
        state = push(state, "Conceptual_Data_Type_implies_Component_Role",
            &[("Conceptual Data Type", "date"), ("Component Role", "date-picker")]);
        // Pinned: Status prefers combo-box explicitly — the pin IS the
        // most-specific source, so a combo-box effective row agrees
        // even though its CDT would imply something else.
        state = push(state, "Noun_has_Effective_Component_Role",
            &[("Noun", "Status"), ("Component Role", "combo-box")]);
        state = push(state, "Noun_prefers_Component_Role",
            &[("Noun", "Status"), ("Component Role", "combo-box")]);
        state = push(state, "Noun_has_Conceptual_Data_Type",
            &[("Noun", "Status"), ("Conceptual Data Type", "text")]);
        state = push(state, "Conceptual_Data_Type_implies_Component_Role",
            &[("Conceptual Data Type", "text"), ("Component Role", "text-input")]);

        let diags = super::check_effective_widget_agrees_with_most_specific_type(&state);
        assert_eq!(diags.len(), 1,
            "exactly the drifted noun must warn; got {:?}",
            diags.iter().map(|d| &d.reading).collect::<Vec<_>>());
        assert!(diags[0].reading.contains("Email"),
            "the drifted noun is Email; got {:?}", diags[0].reading);
        assert!(diags[0].message.contains("text-input"),
            "the implied role must be named; got {:?}", diags[0].message);
        assert!(matches!(diags[0].level, Level::Warning),
            "drift is advisory (Warning), not a reject");
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

    // ── ns-7 (ns-ambiguity-verbalized-reject) ───────────────────────────
    //
    // ns-5 emits `Role_Reference_has_Ambiguous_Domain {Role_Reference,
    // Head_Noun, Candidate_Domain}` — one fact per candidate domain — for
    // a bare ref that 2+ non-local domains declare. ns-7's check layer
    // groups those per-candidate facts into ONE alethic violation per
    // reference whose verbalized message tells the author to qualify the
    // name. Alethic = Source::Resolve at Level::Error, which the
    // load-time gate (load_reading_core::validate_loaded_state) routes
    // into the alethic-violation bucket (a hard reject).

    /// Build a namespaced context state: a Noun cell of `decls` nouns each
    /// homeDomain-stamped with `domain` (ns-4 annotation). Mirrors the
    /// `ns5_ctx` helper in parse_forml2_stage2's tests.
    fn ns7_ctx(domain: &str, decls: &str) -> Object {
        crate::ast::annotate_noun_domain(
            &parse_to_state(decls).expect("ctx decls parse"), domain)
    }

    /// Drive parse → ns-5 over a file in `local_domain` against `ctx`,
    /// then run the full checker Func tree and decode the diagnostics. The
    /// checker runs over the MERGED state (ctx ⊕ this slice's parse) — the
    /// same shape the real loader validates, so the Noun cell carries every
    /// domain's (homeDomain-keyed) declarations the layer joins against.
    fn ns7_diags(text: &str, ctx: &Object, local_domain: &str) -> Vec<ReadingDiagnostic> {
        let parsed = crate::parse_forml2_stage2::parse_to_state_via_stage12_with_context_domain(
            text, ctx, Some(local_domain)).expect("parses — ns-5 does not reject");
        let state = crate::ast::merge_states(ctx, &parsed);
        let obj = crate::ast::apply(&check_readings_func(), &state, &state);
        decode_diags(&obj)
    }

    /// An ambiguous bare reference (declared by 2+ other domains, none
    /// local) yields EXACTLY ONE alethic violation whose verbalized
    /// message lists the candidate domains and the qualify-with-`<domain>.`
    /// guidance. RED→GREEN driver for the new layer.
    #[test]
    fn ns7_ambiguous_reference_yields_one_alethic_qualify_violation() {
        let core = ns7_ctx("core", "Order is a value type.");
        let crm = ns7_ctx("crm", "Order(.id) is an entity type.");
        let ctx = crate::ast::merge_states(&core, &crm);
        // The local file (domain `reports`) only references `Order`.
        let diags = ns7_diags(
            "Report(.id) is an entity type.\nReport references Order.",
            &ctx, "reports");
        let ambig: Vec<_> = diags.iter()
            .filter(|d| d.source == Source::Resolve
                && d.level == Level::Error
                && d.message.contains("ambiguous"))
            .collect();
        assert_eq!(ambig.len(), 1,
            "an ambiguous bare `Order` must raise exactly ONE alethic violation; got {:?}",
            diags);
        let msg = &ambig[0].message;
        // Verbalizes the head noun, both candidate domains (sorted), and
        // the per-candidate `<domain>.<Noun>` qualify guidance.
        assert!(msg.contains("Order"), "message names the head noun: {msg}");
        assert!(msg.contains("core") && msg.contains("crm"),
            "message lists both candidate domains: {msg}");
        assert!(msg.contains("core.Order") && msg.contains("crm.Order"),
            "message shows the qualified forms to choose from: {msg}");
        assert!(msg.to_lowercase().contains("qualif"),
            "message gives qualify-with-`<domain>.` guidance: {msg}");
        // Candidate domains appear in sorted order (deterministic): the
        // first mention of `core` precedes the first mention of `crm`.
        let i_core = msg.find("core").expect("core named");
        let i_crm = msg.find("crm").expect("crm named");
        assert!(i_core < i_crm, "candidate domains must be sorted (core before crm): {msg}");
        // Exact verbalization contract (the task's worked example, in the
        // checker's backtick house style). The message IS the reading: it
        // tells the author precisely how to disambiguate.
        assert_eq!(msg,
            "`Order` is ambiguous: defined in `core`, `crm`. \
             Qualify it as `core.Order` or `crm.Order`.",
            "exact verbalized message");
    }

    /// End-to-end alethic routing: the genuine ambiguity must surface in
    /// the load-time gate's ALETHIC bucket (a hard reject), not the deontic
    /// one. Proves the `Source::Resolve` + `Level::Error` partition wired
    /// in `load_reading_core::validate_loaded_state`.
    #[test]
    fn ns7_ambiguity_routes_to_alethic_reject_bucket() {
        let core = ns7_ctx("core", "Order is a value type.");
        let crm = ns7_ctx("crm", "Order(.id) is an entity type.");
        let ctx = crate::ast::merge_states(&core, &crm);
        let parsed = crate::parse_forml2_stage2::parse_to_state_via_stage12_with_context_domain(
            "Report(.id) is an entity type.\nReport references Order.",
            &ctx, Some("reports")).expect("parses");
        let state = crate::ast::merge_states(&ctx, &parsed);
        let report = crate::load_reading_core::validate_loaded_state(&state);
        assert!(!report.passes,
            "an unqualified kind-conflicting reference must fail the load gate");
        assert!(report.alethic_violations.iter().any(|d| d.message.contains("ambiguous")),
            "the ambiguity violation must land in the ALETHIC bucket; got alethic={:?} deontic={:?}",
            report.alethic_violations.iter().map(|d| &d.message).collect::<Vec<_>>(),
            report.deontic_violations.iter().map(|d| &d.message).collect::<Vec<_>>());
        assert!(!report.deontic_violations.iter().any(|d| d.message.contains("ambiguous")),
            "the ambiguity violation must NOT be deontic");
    }

    /// A bare reference uniquely resolved (declared in exactly one other
    /// domain) is NOT ambiguous → no ambiguity violation. Guards against a
    /// false positive on every pre-existing unqualified cross-domain ref.
    #[test]
    fn ns7_unique_reference_yields_no_ambiguity_violation() {
        let ctx = ns7_ctx("core", "Status is a value type.\nOrder(.id) is an entity type.");
        let diags = ns7_diags(
            "Shipment(.id) is an entity type.\nShipment has Status.",
            &ctx, "shipping");
        assert!(!diags.iter().any(|d| d.message.contains("ambiguous")),
            "a uniquely-resolved bare `Status` must not raise an ambiguity violation; got {:?}",
            diags);
    }

    /// An explicit `<domain>.<Noun>` (ns-6) qualifier resolves outright —
    /// ns-5 marks it resolved, not ambiguous — so no ambiguity violation
    /// even when the bare name collides across domains.
    #[test]
    fn ns7_explicitly_qualified_reference_yields_no_ambiguity_violation() {
        let core = ns7_ctx("core", "Order is a value type.");
        let crm = ns7_ctx("crm", "Order(.id) is an entity type.");
        let ctx = crate::ast::merge_states(&core, &crm);
        let diags = ns7_diags(
            "Report(.id) is an entity type.\nReport references core.Order.",
            &ctx, "reports");
        assert!(!diags.iter().any(|d| d.message.contains("ambiguous")),
            "an explicitly-qualified `core.Order` must not raise an ambiguity violation; got {:?}",
            diags);
    }

    /// Direct-layer unit: feed a synthesized `Role_Reference_has_Ambiguous_Domain`
    /// cell with TWO refs — one with three candidate facts, one with two —
    /// and assert the layer groups per reference into exactly TWO violations,
    /// each verbalizing its own head noun + sorted candidates. Mirrors the
    /// ring-completeness unit-test style. Post-broadening the gate no longer
    /// consults the candidates' KIND, so no Noun cell is needed here — every
    /// grouped reference is flagged.
    #[test]
    fn ns7_layer_groups_candidate_facts_into_one_violation_per_reference() {
        use crate::ast::{Object, fact_from_pairs, cell_push};
        let mut state = Object::phi();
        // ref A: `Order` ambiguous across crm, core, billing (unsorted).
        for dom in ["crm", "core", "billing"] {
            state = cell_push("Role_Reference_has_Ambiguous_Domain",
                fact_from_pairs(&[
                    ("Role_Reference", "s2:role:0"),
                    ("Head_Noun", "Order"),
                    ("Candidate_Domain", dom),
                ]), &state);
        }
        // ref B: `Account` ambiguous across two domains.
        for dom in ["finance", "auth"] {
            state = cell_push("Role_Reference_has_Ambiguous_Domain",
                fact_from_pairs(&[
                    ("Role_Reference", "s2:role:1"),
                    ("Head_Noun", "Account"),
                    ("Candidate_Domain", dom),
                ]), &state);
        }
        let diags = super::check_ambiguous_domain_references(&state);
        assert_eq!(diags.len(), 2,
            "two distinct ambiguous refs ⇒ exactly two violations (grouped per ref); got {:?}",
            diags);
        // Every emitted diagnostic is an alethic (Resolve/Error) reject.
        assert!(diags.iter().all(|d| d.source == Source::Resolve && d.level == Level::Error),
            "ambiguity violations must be alethic (Source::Resolve, Level::Error); got {:?}",
            diags);
        let order = diags.iter().find(|d| d.message.contains("Order"))
            .expect("a violation for `Order`");
        // Candidates deduped + sorted: billing, core, crm.
        assert!(order.message.contains("billing") && order.message.contains("core")
            && order.message.contains("crm"),
            "Order violation lists all three sorted candidates: {}", order.message);
        let i_billing = order.message.find("billing").unwrap();
        let i_core = order.message.find("core").unwrap();
        let i_crm = order.message.find("crm").unwrap();
        assert!(i_billing < i_core && i_core < i_crm,
            "candidates must be sorted (billing < core < crm): {}", order.message);
        let account = diags.iter().find(|d| d.message.contains("Account"))
            .expect("a violation for `Account`");
        assert!(account.message.contains("auth") && account.message.contains("finance"),
            "Account violation lists both candidates: {}", account.message);
    }

    /// An empty ambiguity cell (the common case — no namespaced collision)
    /// yields no violations, so legacy single-domain parses are unaffected.
    #[test]
    fn ns7_no_ambiguity_facts_yield_no_violations() {
        let empty = Object::phi();
        assert!(super::check_ambiguous_domain_references(&empty).is_empty(),
            "no ambiguity facts ⇒ no violations");
    }

    /// BROADENING PROOF (ns-namespace-collision-cleanup): a GENUINE
    /// same-kind 2-domain collision must now be FLAGGED. `id` declared as
    /// a value type in BOTH domains is the case the old kind-conflict-only
    /// staging deliberately EXEMPTED; the broadened gate rejects it, so the
    /// broadening is proven (not made vacuous). The cleanup removed every
    /// such collision from the bundled corpus (consolidating the universal
    /// primitives to `core`, unifying `User`, qualifying `View`), so this
    /// synthetic case stands in for the class the gate now guards against.
    #[test]
    fn ns7_same_kind_2domain_collision_is_flagged() {
        use crate::ast::{Object, fact_from_pairs, cell_push};
        let mut state = Object::phi();
        // `id` is a value type in BOTH domains — SAME kind, no conflict.
        // Under the broadened gate this is a genuine ambiguity and rejects.
        for dom in ["core", "ui"] {
            state = cell_push("Noun", fact_from_pairs(&[
                ("name", "id"), ("homeDomain", dom), ("objectType", "value"),
            ]), &state);
        }
        for dom in ["core", "ui"] {
            state = cell_push("Role_Reference_has_Ambiguous_Domain",
                fact_from_pairs(&[
                    ("Role_Reference", "s9:role:1"),
                    ("Head_Noun", "id"),
                    ("Candidate_Domain", dom),
                ]), &state);
        }
        let viols = super::check_ambiguous_domain_references(&state);
        assert_eq!(viols.len(), 1,
            "a same-kind (value/value) bare `id` declared in two domains must \
             raise exactly ONE alethic ambiguity violation under the broadened \
             gate; got {:?}", viols);
        assert!(viols[0].source == Source::Resolve && viols[0].level == Level::Error,
            "the violation must be alethic (Source::Resolve, Level::Error)");
        assert_eq!(viols[0].message,
            "`id` is ambiguous: defined in `core`, `ui`. \
             Qualify it as `core.id` or `ui.id`.",
            "same-kind ambiguity verbalizes exactly like any other");
    }

    /// Real-corpus guard (ns-namespace-collision-cleanup): the bundled
    /// metamodel, folded through the actual per-file-domain loader
    /// (`metamodel_state`, the same fold the CLI and kernel use), must
    /// validate CLEAN under the BROADENED gate — i.e. the cleanup resolved
    /// EVERY cross-domain collision, so ns-5 emits NO ambiguity signal at
    /// all and the gate finds nothing to reject.
    ///
    /// Before the cleanup ns-5 emitted ~537 ambiguity facts over 143
    /// references (the universal value primitives `id`/`Name`/`Title`/`code`
    /// declared per-domain, plus the same-kind entities `User`/`View`
    /// referenced bare across slices). The cleanup consolidated each
    /// primitive to ONE declaration in `core` (synthetic ref-scheme shadows
    /// defer to it in `defining_domains_by_name`), unified `User` onto a
    /// single declaration, and qualified the genuinely-distinct `View`
    /// references as `view-projection.View`. The signal is therefore GONE —
    /// which is exactly what lets the broadened gate (no kind-conflict
    /// staging) validate the corpus clean. A future corpus edit that
    /// re-introduced a bare cross-domain collision (same kind or not) would
    /// (correctly) light this guard up.
    #[test]
    fn ns7_bundled_metamodel_corpus_has_no_ambiguity_violations() {
        let state = crate::metamodel_state();
        // The cleanup resolved every collision, so ns-5 emits NO ambiguity
        // signal on the bundled corpus.
        let raw_ambiguity_facts = crate::ast::cell_facts_iter(
            &fetch_cell_seq("Role_Reference_has_Ambiguous_Domain", state)).count();
        assert_eq!(raw_ambiguity_facts, 0,
            "ns-namespace-collision-cleanup resolved every cross-domain \
             collision; ns-5 must emit no ambiguity signal on the bundled \
             corpus, got {} facts", raw_ambiguity_facts);
        // And the broadened gate (which now flags ANY same-kind ambiguity)
        // therefore finds nothing to reject.
        let viols = super::check_ambiguous_domain_references(state);
        assert!(viols.is_empty(),
            "the bundled metamodel must validate clean under the broadened gate; got {:#?}",
            viols.iter().map(|d| &d.message).collect::<Vec<_>>());
    }
}
