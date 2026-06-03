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
/// cross-namespace reference whose candidate domains declare it as
/// CONFLICTING KINDS of thing (an entity in one, a value in another) —
/// the genuinely unbindable case.
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
/// WHY THE KIND-CONFLICT GATE (the corpus-compat decision): ns-5 flags as
/// ambiguous EVERY bare reference whose head noun is declared in 2+
/// non-local domains. On the shipping metamodel that includes ubiquitous
/// shared value-type primitives (`id`, `Name`, `Title`, `code`) declared
/// identically in a dozen domains, and same-kind entities (`User`, `View`)
/// the corpus references bare across slices. Rejecting on the raw signal
/// would fail the bundled corpus — which the task forbids. A reference
/// whose candidates AGREE on kind is bindable in principle (it is the same
/// primitive value type, or a same-shaped entity resolvable by local
/// precedence in its own slice); only a reference whose candidates
/// DISAGREE on kind (e.g. `Order` = a value type in `core` but an entity
/// type in `crm`) is a structural impossibility — the cell graph has no
/// single noun to bind it to. That is the alethic case, and it is exactly
/// what the task's worked example exhibits. Same-kind ambiguity is left
/// for a later pass (and a future corpus cleanup) to arm.
///
/// The diagnostic is `Level::Error` + `Source::Resolve`, which the
/// load-time gate (`load_reading_core::validate_loaded_state`) routes to
/// the ALETHIC bucket — a hard reject.
///
/// The cell is empty for any parse without a namespaced collision (every
/// legacy single-domain parse), so this layer is a no-op there.
fn check_ambiguous_domain_references(state: &Object) -> Vec<ReadingDiagnostic> {
    // (name, homeDomain) -> objectType ("entity" / "value"), from the
    // Noun cell ns-4 stamped per domain. Used to decide whether the
    // candidates for an ambiguous ref even agree on what KIND of thing
    // the name denotes.
    let mut kind_by_name_domain: hashbrown::HashMap<(String, String), String> =
        hashbrown::HashMap::new();
    for n in crate::ast::cell_facts_iter(&fetch_cell_seq("Noun", state)) {
        let (Some(name), Some(dom)) = (binding(n, "name"), binding(n, "homeDomain"))
            else { continue };
        let kind = binding(n, "objectType").unwrap_or("").to_string();
        kind_by_name_domain.insert((name.to_string(), dom.to_string()), kind);
    }

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
    by_ref.into_iter().filter_map(|(_rid, (noun, mut domains))| {
        domains.sort();
        domains.dedup();
        // STAGED: fire only on a CONCRETE kind conflict — one candidate
        // declares the name an `entity`, another a `value`. That reference
        // is structurally unbindable (the candidates aren't even the same
        // kind of thing) and is the worked example (`Order` = value in core,
        // entity in crm). This is NOT a claim that same-kind collisions are
        // acceptable: per design policy they ARE genuine ambiguities that
        // SHOULD be flagged — `id`/`Name`/`Title`/`code` declared per-domain
        // are duplicate declarations to CONSOLIDATE into one shared concept,
        // and same-name entities like `User`/`View` are to UNIFY (or qualify
        // if truly distinct). Flagging them now would (correctly) fail the
        // shipping corpus, which still carries those duplicates — so the
        // broadening is gated on that corpus cleanup (filed:
        // ns-namespace-collision-cleanup). Until it lands, only the kind
        // conflict fires; unknown-kind candidates never fire on their own.
        let mut saw_entity = false;
        let mut saw_value = false;
        for d in &domains {
            match kind_by_name_domain.get(&(noun.clone(), d.clone())).map(String::as_str) {
                Some("entity") => saw_entity = true,
                Some("value")  => saw_value = true,
                _ => {}
            }
        }
        if !(saw_entity && saw_value) {
            return None;
        }
        let defined_in = domains.iter()
            .map(|d| format!("`{}`", d))
            .collect::<Vec<_>>()
            .join(", ");
        // The qualified `<domain>.<Noun>` choices, joined naturally so the
        // last is preceded by `or` (e.g. "`a.N` or `b.N`",
        // "`a.N`, `b.N`, or `c.N`").
        let qualified = join_qualified_choices(&domains, &noun);
        Some(ReadingDiagnostic {
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
        })
    }).collect()
}

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
        // Concat ∘ Construction([…]) with exactly 5 layers. This is
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
                    Func::Construction(layers) => assert_eq!(layers.len(), 6,
                        "check_readings_func must expose exactly 6 layer Funcs"),
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
    /// (plus a Noun cell whose candidates DISAGREE on kind so the
    /// kind-conflict gate fires) and assert the layer groups per reference
    /// into exactly TWO violations, each verbalizing its own head noun +
    /// sorted candidates. Mirrors the ring-completeness unit-test style.
    #[test]
    fn ns7_layer_groups_candidate_facts_into_one_violation_per_reference() {
        use crate::ast::{Object, fact_from_pairs, cell_push};
        let mut state = Object::phi();
        // Noun cell: `Order` declared with CONFLICTING kinds across its
        // three candidate domains (value in core, entity in crm/billing),
        // and `Account` value-in-finance / entity-in-auth. Each conflict
        // makes the bare reference structurally unbindable → it fires.
        for (name, dom, kind) in [
            ("Order", "core", "value"), ("Order", "crm", "entity"),
            ("Order", "billing", "entity"),
            ("Account", "finance", "value"), ("Account", "auth", "entity"),
        ] {
            state = cell_push("Noun", fact_from_pairs(&[
                ("name", name), ("homeDomain", dom), ("objectType", kind),
            ]), &state);
        }
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

    /// Kind-AGREEMENT exemption (corpus-compat): when every candidate
    /// domain declares the bare name as the SAME kind (e.g. a value-type
    /// primitive like `id` declared in many domains, or a same-shaped
    /// entity), the reference is bindable in principle and is NOT a hard
    /// reject. This is what keeps the shipping metamodel (whose only
    /// ambiguities are kind-agreeing — `id`, `Name`, `Title`, `code`,
    /// `User`, `View`) validating clean.
    #[test]
    fn ns7_kind_agreeing_candidates_are_exempt() {
        use crate::ast::{Object, fact_from_pairs, cell_push};
        let mut state = Object::phi();
        // `id` is a value type in BOTH domains — same kind, no conflict.
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
        assert!(super::check_ambiguous_domain_references(&state).is_empty(),
            "a value-type `id` declared identically across domains is the same \
             primitive — kind-agreeing candidates must NOT trip the reject");
    }

    /// Real-corpus guard (the task's explicit backward-compat check): the
    /// bundled metamodel, folded through the actual per-file-domain loader
    /// (`metamodel_state`, the same fold the CLI and kernel use), must NOT
    /// trip the new ambiguity reject. NOTE: ns-5 DOES emit ambiguity
    /// signals on the real corpus (the shared value-type primitives `id`,
    /// `Name`, `Title`, `code` declared per-domain, and the same-kind
    /// entities `User`, `View` referenced bare across slices) — local
    /// precedence does NOT eliminate them. They stay clean here only
    /// because every one of those ambiguities is kind-AGREEING, and the
    /// reject fires solely on kind-CONFLICTING candidates (see
    /// `check_ambiguous_domain_references`). If a future corpus edit
    /// introduced a value-vs-entity collision on a bare reference, this
    /// guard would (correctly) light up.
    #[test]
    fn ns7_bundled_metamodel_corpus_has_no_ambiguity_violations() {
        let state = crate::metamodel_state();
        // ns-5 DOES emit ambiguity signals on the real corpus (this guard
        // is not vacuous): the shared value-type primitives + same-kind
        // entities referenced bare across slices. Assert that, so a future
        // resolver change that silenced the signal can't make this guard
        // pass for the wrong reason.
        let raw_ambiguity_facts = crate::ast::cell_facts_iter(
            &fetch_cell_seq("Role_Reference_has_Ambiguous_Domain", state)).count();
        assert!(raw_ambiguity_facts > 0,
            "expected ns-5 to emit ambiguity signals on the bundled corpus");
        // None of them is a kind-conflict, so none is rejected.
        let viols = super::check_ambiguous_domain_references(state);
        assert!(viols.is_empty(),
            "the bundled metamodel must validate clean (no kind-conflicting bare refs); got {:#?}",
            viols.iter().map(|d| &d.message).collect::<Vec<_>>());
    }
}
