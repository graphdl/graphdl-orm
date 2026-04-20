// crates/arest/src/parse_forml2.rs
//
// FORML 2 Parser -- FFP composition of recognizer functions.
//
// Per the paper: parse: R -> Phi (Theorem 2).
// parse = alpha(recognize) : lines
// recognize = try1 ; try2 ; ... ; tryn
//
// Each recognizer: &str -> Option<ParseAction>
// The ? operator IS the conditional form <COND, is_some, unwrap, _|_>.
// No if/else chains. Pattern matching via strip_suffix/strip_prefix/find.

use crate::types::*;
use hashbrown::HashMap;

/// Parse-time accumulator. MODULE-PRIVATE. The parser is the only
/// producer; every external consumer reads Object state via
/// `parse_to_state`. This struct is NOT IR — it is the parser's
/// working memory and vanishes as soon as parse completes.
///
/// Invariant: `ctx_to_state` is a private conversion helper that
/// wraps `cells` in `Object::Map`. Callers outside this module see
/// only `Object` per Thm 2 (parse: R → Φ).
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "std-deps", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
struct ParseCtx {
    #[cfg_attr(feature = "std-deps", serde(default))]
    derivation_rules: Vec<DerivationRuleDef>,
    /// Object cells produced by apply_action. This IS the parse output;
    /// only derivation_rules still shadows a cell because pass 2b
    /// mutates its shape (resolve_derivation_rule).
    ///
    /// #283 target: delete the remaining typed cache and make this the
    /// sole parser state. Migration in progress — named_spans,
    /// autofill_spans, ref_schemes, subtypes, enum_values,
    /// general_instance_facts, nouns, fact_types, and constraints
    /// already migrated to their respective cells.
    #[cfg_attr(feature = "std-deps", serde(skip))]
    cells: HashMap<String, Vec<crate::ast::Object>>,
}
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

// Bootstrap mode flag â€” set by lib::create_impl while loading bundled
// metamodel readings, so the metamodel namespace guard (#23) is bypassed
// for cross-file redeclarations within the canonical metamodel. Apps must
// NOT set this flag; user-domain compiles always hit the guard.
// Under std, thread_local keeps bootstrap/strict flags per-thread so
// parallel test threads don't collide. Under no_std (single-core
// kernel), AtomicBool is fine â€” there are no parallel test runners.
#[cfg(not(feature = "no_std"))]
thread_local! {
    static BOOTSTRAP_MODE: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
    static STRICT_MODE: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

#[cfg(not(feature = "no_std"))]
pub fn set_bootstrap_mode(on: bool) { BOOTSTRAP_MODE.with(|b| b.set(on)); }
#[cfg(not(feature = "no_std"))]
fn is_bootstrap_mode() -> bool { BOOTSTRAP_MODE.with(|b| b.get()) }
#[cfg(not(feature = "no_std"))]
#[allow(dead_code)]
pub(crate) fn set_strict_mode(on: bool) { STRICT_MODE.with(|b| b.set(on)); }
#[cfg(not(feature = "no_std"))]
fn is_strict_mode() -> bool { STRICT_MODE.with(|b| b.get()) }

#[cfg(feature = "no_std")]
static BOOTSTRAP_MODE_ATOMIC: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "no_std")]
static STRICT_MODE_ATOMIC: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "no_std")]
pub fn set_bootstrap_mode(on: bool) { BOOTSTRAP_MODE_ATOMIC.store(on, core::sync::atomic::Ordering::Relaxed); }
#[cfg(feature = "no_std")]
fn is_bootstrap_mode() -> bool { BOOTSTRAP_MODE_ATOMIC.load(core::sync::atomic::Ordering::Relaxed) }
#[cfg(feature = "no_std")]
#[allow(dead_code)]
pub(crate) fn set_strict_mode(on: bool) { STRICT_MODE_ATOMIC.store(on, core::sync::atomic::Ordering::Relaxed); }
#[cfg(feature = "no_std")]
fn is_strict_mode() -> bool { STRICT_MODE_ATOMIC.load(core::sync::atomic::Ordering::Relaxed) }

pub(crate) const METAMODEL_NOUNS: &[&str] = &[
    "Noun",
    "Fact Type",
    "Role",
    "Constraint",
    "State Machine Definition",
    "Transition",
    "Status",
    "Event Type",
    "Domain Change",
];

/// Metadata for a noun that is stored on ParseCtx maps, not on NounDef.
#[derive(Default, Clone)]
struct NounMeta {
    super_type: Option<String>,
    ref_scheme: Option<Vec<String>>,
}

/// What a recognizer produces when it matches a line.
enum ParseAction {
    AddNoun(String, NounDef, NounMeta),
    MarkAbstract(String),
    AddPartition(String, Vec<String>),
    AddFactType(String, FactTypeDef, Option<String>),
    AddConstraint(ConstraintDef),
    AddDerivation(DerivationRuleDef),
    AddInstanceFact(String), // raw line for instance fact parsing
    AddNamedSpan(String, Vec<String>), // span_name, role nouns
    AddAutofillSpan(String),           // span_name
    Skip,
}

// =========================================================================
// Recognizers -- pure functions: &str -> Option<ParseAction>
// The ? operator replaces all if/else branching.
// =========================================================================

fn try_header(line: &str) -> Option<ParseAction> {
    line.starts_with('#').then_some(ParseAction::Skip)
}

fn try_entity_type(line: &str) -> Option<ParseAction> {
    let before = line.strip_suffix(" is an entity type.")?;
    let (name, ref_scheme) = parse_entity_decl(before.trim())?;
    Some(ParseAction::AddNoun(name, NounDef {
        object_type: "entity".into(),
        world_assumption: WorldAssumption::default(),
    }, NounMeta { ref_scheme, ..Default::default() }))
}

fn try_value_type(line: &str) -> Option<ParseAction> {
    let name = line.strip_suffix(" is a value type.")?.trim().to_string();
    Some(ParseAction::AddNoun(name, NounDef {
        object_type: "value".into(),
        world_assumption: WorldAssumption::default(),
    }, NounMeta::default()))
}

fn try_subtype(line: &str) -> Option<ParseAction> {
    let clean = line.strip_suffix('.')?;
    let idx = clean.find(" is a subtype of ")?;
    let sub = clean[..idx].trim().to_string();
    let sup = clean[idx + 17..].trim().to_string();
    Some(ParseAction::AddNoun(sub, NounDef {
        object_type: "entity".into(),
        world_assumption: WorldAssumption::default(),
    }, NounMeta { super_type: Some(sup), ..Default::default() }))
}

fn try_abstract(line: &str) -> Option<ParseAction> {
    let name = line.strip_suffix(" is abstract.")?.trim().to_string();
    Some(ParseAction::MarkAbstract(name))
}

fn try_partition(line: &str) -> Option<ParseAction> {
    let clean = line.strip_suffix('.')?;
    let idx = clean.find(" is partitioned into ")?;
    let sup = clean[..idx].trim().to_string();
    let subs = clean[idx + 21..].split(',').map(|s| s.trim().into()).collect();
    Some(ParseAction::AddPartition(sup, subs))
}

fn try_enum_values(line: &str) -> Option<ParseAction> {
    line.starts_with("The possible values of").then_some(ParseAction::Skip)
}

fn try_exclusive_subtypes(line: &str) -> Option<ParseAction> {
    (line.starts_with('{') && line.contains("subtypes of")).then_some(ParseAction::Skip)
}

fn try_association(line: &str) -> Option<ParseAction> {
    line.starts_with("This association with").then_some(ParseAction::Skip)
}

fn try_totality(line: &str) -> Option<ParseAction> {
    let rest = line.strip_prefix("Each ")?;
    let idx = rest.find(" is a ")?;
    rest.contains(" or ").then(|| ParseAction::MarkAbstract(rest[..idx].trim().into()))
}

fn try_ring(line: &str, noun_names: &[String]) -> Option<ParseAction> {
    // Strip trailing documentation annotation like " (irreflexive)" or
    // " (symmetric)" — users write `No X R itself. (irreflexive)` to
    // label the kind inline, and the parenthetical must not block the
    // ` itself` / conditional-form recognition that follows.
    let clean = strip_trailing_annotation(line.trim_end_matches('.').trim_end())
        .trim_end_matches('.');

    // IR: "No A R itself." -- simple irreflexive
    // AC: "No A may cycle back to itself via one or more traversals through R."
    if let Some(rest) = clean.strip_prefix("No ") {
        // AC pattern: "No A may cycle back to itself ..."
        if rest.contains(" may cycle back to itself") {
            // Extract the entity type (first word(s) matching a noun)
            let entity = noun_names.iter()
                .find(|n| rest.starts_with(n.as_str()))
                .cloned()
                .unwrap_or_default();
            return Some(ParseAction::AddConstraint(ConstraintDef {
                id: String::new(), kind: "AC".into(), modality: "alethic".into(),
                deontic_operator: None, text: clean.into(),
                spans: vec![], set_comparison_argument_length: None, clauses: None,
                entity: if entity.is_empty() { None } else { Some(entity) },
                min_occurrence: None, max_occurrence: None,
            }));
        }
        // IR pattern: "No A R itself" -- must end with " itself" and have a known noun
        if rest.ends_with(" itself") {
            let entity = noun_names.iter()
                .find(|n| rest.starts_with(n.as_str()))
                .cloned()
                .unwrap_or_default();
            if !entity.is_empty() {
                return Some(ParseAction::AddConstraint(ConstraintDef {
                    id: String::new(), kind: "IR".into(), modality: "alethic".into(),
                    deontic_operator: None, text: clean.into(),
                    spans: vec![], set_comparison_argument_length: None, clauses: None,
                    entity: Some(entity), min_occurrence: None, max_occurrence: None,
                }));
            }
        }
        return None;
    }

    // Conditional ring patterns: "If A1 R A2 [and ...] then [it is impossible that] ..."
    clean.starts_with("If ").then_some(())?;
    let then_idx = clean.find(" then ")?;
    let antecedent = &clean[3..then_idx]; // everything after "If " up to " then "
    let consequent = &clean[then_idx + 6..]; // everything after " then "

    // All role tokens in the antecedent must share the same base noun type.
    // Extract words that match known nouns (with or without trailing digit subscripts).
    let role_bases: Vec<&str> = antecedent
        .split_whitespace()
        .filter_map(|word| {
            let w = word.trim_end_matches(',');
            let (base, _) = parse_role_token(w);
            noun_names.iter().any(|n| n == base).then_some(base)
        })
        .collect();

    // Need at least 2 role tokens in antecedent, all with the same base
    (role_bases.len() >= 2).then_some(())?;
    let first_base = role_bases[0];
    role_bases.iter().all(|b| *b == first_base).then_some(())?;

    // Also check consequent contains the same base noun (subscripted or plain)
    let consequent_has_same_noun = {
        let effective = if consequent.starts_with("it is impossible that ") {
            &consequent["it is impossible that ".len()..]
        } else {
            consequent
        };
        effective.split_whitespace().any(|word| {
            let w = word.trim_end_matches(',');
            let (base, _) = parse_role_token(w);
            base == first_base
        })
    };
    consequent_has_same_noun.then_some(())?;

    let has_and = antecedent.contains(" and ");
    let impossible = consequent.starts_with("it is impossible that ");
    let itself_in_consequent = consequent.contains(" itself");
    let is_not_in_antecedent = antecedent.contains(" is not ");

    let kind = match (has_and, impossible, itself_in_consequent, is_not_in_antecedent) {
        // AS: no and, impossible, no itself -- "If A1 R A2 then it is impossible that A2 R A1"
        (false, true, false, _)  => "AS",
        // RF: no and, not impossible, itself in consequent -- "If A1 R some A2 then A1 R itself"
        (false, false, true, _)  => "RF",
        // SY: no and, not impossible, no itself -- "If A1 R A2 then A2 R A1"
        (false, false, false, _) => "SY",
        // AT: and, impossible, "is not" in antecedent -- "If A1 R A2 and A1 is not A2 then impossible A2 R A1"
        (true, true, _, true)    => "AT",
        // IT: and, impossible, no "is not" -- "If A1 R A2 and A2 R A3 then impossible A1 R A3"
        (true, true, _, false)   => "IT",
        // TR: and, not impossible -- "If A1 R A2 and A2 R A3 then A1 R A3"
        (true, false, _, _)      => "TR",
        // Unrecognized combination -- not a ring constraint
        _ => return None,
    };

    Some(ParseAction::AddConstraint(ConstraintDef {
        id: String::new(), kind: kind.into(), modality: "alethic".into(),
        deontic_operator: None, text: clean.into(),
        spans: vec![], set_comparison_argument_length: None, clauses: None,
        // The ring noun is the common base shared by every role token
        // in the antecedent (enforced above). Carry it through so
        // check_ring_completeness can match this constraint to its FT
        // without relying on span0_factTypeId.
        entity: Some(first_base.to_string()),
        min_occurrence: None, max_occurrence: None,
    }))
}

/// True when `clause` starts with `for each <Noun>` and is followed
/// by at least one more declared noun reference (the predicate over
/// the universally-quantified variable). Accepts universal-quantifier
/// antecedents like
///     for each Authority that applies to that Support Response,
///       that Support Response satisfies that Authority
/// so the overall derivation rule is not flagged as unresolved.
fn is_universal_quantifier_clause(clause: &str, noun_names: &[String]) -> bool {
    let trimmed = clause.trim();
    let Some(after) = trimmed.strip_prefix("for each ") else { return false; };
    // Must mention a declared noun after `for each`.
    noun_names.iter().any(|n| after.starts_with(n.as_str()))
        // ...and at least one more noun reference in the tail.
        && noun_names.iter().any(|n| {
            let needle = format!(" {}", n);
            after.contains(&needle)
        })
}

/// True when `clause` has the shape `<Noun> is extracted from <Noun>`
/// or `<Noun> is derived from <Noun>`. Both operands must be declared.
/// Used for ML-style computed bindings (free-text extraction,
/// classifier outputs) where the underlying extractor is registered
/// at runtime. Classification here suppresses the false-unresolved
/// noise; the actual extraction function lives in DEFS.
fn is_extraction_clause(clause: &str, noun_names: &[String]) -> bool {
    let trimmed = clause.trim().trim_end_matches('.');
    [" is extracted from ", " is derived from "].iter().any(|kw| {
        let Some(idx) = trimmed.find(kw) else { return false; };
        let lhs = trimmed[..idx].trim();
        let rhs = trimmed[idx + kw.len()..].trim();
        let is_noun = |s: &str| noun_names.iter().any(|n| n == s);
        is_noun(lhs) && is_noun(rhs)
    })
}

/// Strip existential / anaphoric quantifiers from FT references so
/// `Feature Request concerns some API Product` resolves against the
/// declared `Feature Request concerns API Product`. Only ` some ` and
/// ` that ` (as whole-word tokens) are removed — the surrounding
/// noun / verb text is untouched.
fn strip_existential_quantifiers(clause: &str) -> String {
    clause
        .replace(" some ", " ")
        .replace(" that ", " ")
        .replace("  ", " ")
        .trim()
        .to_string()
}

/// True when `clause` has the shape `<Noun> has <Noun> '<literal>'`
/// with both nouns declared. Accepts state-machine status filters and
/// enum-value filters where the underlying FT isn't always declared
/// textually (e.g. Status is SM-managed).
fn is_noun_has_noun_literal(clause: &str, noun_names: &[String]) -> bool {
    let trimmed = clause.trim().trim_end_matches('.');
    let re = regex::Regex::new(r"^(.+?) has (.+?) '[^']*'$").expect("static");
    let Some(caps) = re.captures(trimmed) else { return false; };
    let subj = caps.get(1).map(|m| m.as_str().trim()).unwrap_or_default();
    let attr = caps.get(2).map(|m| m.as_str().trim()).unwrap_or_default();
    let is_noun = |s: &str| noun_names.iter().any(|n| n == s);
    is_noun(subj) && is_noun(attr)
}

/// #276 Category G — iteratively expand relative-clause `that`-chains
/// into explicit conjunctions.
///
/// `<head> that <verb phrase>` rewrites to
/// `<head> and <last noun of head> <verb phrase>` so the downstream
/// ` and `-split produces two clauses that both resolve against
/// declared FTs. The expansion runs repeatedly until no expandable
/// ` that ` remains, so nested forms
///
///   Source Request is for Resource Declaration that has Base Path
///
/// flatten to
///
///   Source Request is for Resource Declaration
///   Resource Declaration has Base Path
///
/// Back-reference anaphora (`that <Noun> ...`) is untouched — the
/// existing anaphora classifier handles those join-key forms.
///
/// Safety rail: expansion is skipped when the `<head>` portion does
/// not itself resolve to a declared FT. Blindly rewriting a head
/// that isn't in the catalog (e.g. the 5-ary `Billable Request is
/// for Customer and Meter Endpoint and VIN and Date` from auth.md,
/// whose binary slice `Billable Request is for Customer` doesn't
/// exist) would replace a single unresolved warning with two, making
/// the diagnostic output noisier. When the head fails to resolve,
/// the original clause stays intact and falls through to the
/// downstream classifier cascade.
fn expand_that_relatives(
    antecedent: &str,
    noun_names: &[String],
    catalog: &SchemaCatalog,
) -> String {
    let mut current = antecedent.to_string();
    loop {
        let positions: Vec<usize> = current
            .match_indices(" that ")
            .map(|(i, _)| i)
            .collect();
        let expand_at = positions.into_iter().find(|&i| {
            let tail = &current[i + " that ".len()..];
            let tail_trim = tail.trim_start();
            if is_that_anaphora_ref(tail_trim, noun_names) { return false; }
            // Only expand when the head — text up to this ` that ` —
            // resolves to a declared FT. Otherwise leave the clause
            // for downstream classifiers to handle whole.
            let head = &current[..i];
            head_resolves(head, noun_names, catalog)
        });
        let Some(pos) = expand_at else { break; };
        let head = &current[..pos];
        let tail = &current[pos + " that ".len()..];
        let Some(last_noun) = find_last_noun_in(head, noun_names) else { break; };
        let expanded = alloc::format!("{} and {} {}", head, last_noun, tail);
        if expanded == current { break; }
        current = expanded;
    }
    current
}

/// True when the text up to this point resolves to a declared FT
/// via the schema catalog. Used as a pre-flight check before
/// expanding a `that`-relative — we only want to split when the
/// left side is known-good.
fn head_resolves(head: &str, noun_names: &[String], catalog: &SchemaCatalog) -> bool {
    let found = find_nouns(head, noun_names);
    if found.is_empty() { return false; }
    let base_refs: Vec<String> = found.iter()
        .map(|(_, _, n)| parse_role_token(n).0.to_string())
        .collect();
    let role_refs: Vec<&str> = base_refs.iter().map(|s| s.as_str()).collect();
    let verb = match found.len() {
        1 => head[found[0].1..].trim(),
        _ => head[found[0].1..found[1].0].trim(),
    };
    let verb_opt = (!verb.is_empty()).then_some(verb);
    catalog.resolve(&role_refs, verb_opt).is_some()
        || catalog.resolve(&role_refs, None).is_some()
}

/// Find the last declared noun appearing in `text`, longest-first.
fn find_last_noun_in(text: &str, noun_names: &[String]) -> Option<String> {
    let found = find_nouns(text, noun_names);
    found.last().map(|(_, _, name)| parse_role_token(name).0.to_string())
}

/// True when `tail` (text immediately after `that `) starts with a
/// noun reference rather than a verb phrase. Noun references take
/// three forms: plain noun, subscripted noun (`Person3`), and
/// hyphen-bound role name (`expires- Timestamp`). Used by
/// `expand_that_relatives` to skip anaphora — back-references to a
/// previously-bound role shouldn't be rewritten into conjunctions.
fn is_that_anaphora_ref(tail: &str, noun_names: &[String]) -> bool {
    // Shape 1 + 2: <Noun> or <Noun><digits>
    if noun_names.iter().any(|n| {
        let Some(after) = tail.strip_prefix(n.as_str()) else { return false; };
        let after_subscript = after.trim_start_matches(|c: char| c.is_ascii_digit());
        matches!(
            after_subscript.chars().next(),
            None | Some(' ') | Some('.') | Some(','),
        )
    }) { return true; }
    // Shape 3: <word>- <Noun>, i.e. hyphen-bound role prefix.
    // The prefix is a single whitespace-free token followed by `- `.
    // `cached- Timestamp`, `override- Fetcher` both fit.
    let Some(hyphen_idx) = tail.find("- ") else { return false; };
    let prefix = &tail[..hyphen_idx];
    if prefix.is_empty() || prefix.contains(' ') { return false; }
    let after_hyphen = &tail[hyphen_idx + "- ".len()..];
    noun_names.iter().any(|n| {
        let Some(after) = after_hyphen.strip_prefix(n.as_str()) else { return false; };
        matches!(
            after.chars().next(),
            None | Some(' ') | Some('.') | Some(','),
        )
    })
}

/// #275 Category C — `<Noun> is '<literal>'` or `<Noun> is not
/// '<literal>'` is a ref-scheme-value filter over the noun's
/// identity. Optional leading role-binding qualifiers (`other `,
/// `that `, `some `, `each `, `any `) and numeric subscripts on the
/// noun (`Source1`, `Customer2`) are stripped before the match. The
/// clause body in a derivation rule uses this form to select the
/// entity whose ref scheme value equals the literal — equivalent to
/// `Noun has <RefSchemeVT> '<literal>'`.
fn is_entity_ref_scheme_literal(clause: &str, noun_names: &[String]) -> bool {
    let trimmed = clause.trim().trim_end_matches('.');
    // Strip a single leading role qualifier. Only one per clause is
    // idiomatic in Halpin readings; stripping every occurrence would
    // widen the match beyond intent.
    let stripped = ["other ", "that ", "some ", "each ", "any ", "the ", "a ", "an "]
        .iter()
        .fold(trimmed, |s, q| s.strip_prefix(q).unwrap_or(s));
    let re = regex::Regex::new(r"^(.+?) (?:is not|is) '[^']*'$").expect("static");
    let Some(caps) = re.captures(stripped) else { return false; };
    let raw_subj = caps.get(1).map(|m| m.as_str().trim()).unwrap_or_default();
    let (base, _) = parse_role_token(raw_subj);
    noun_names.iter().any(|n| n == base)
}

/// True when `clause` has the shape `<Noun> is (a|an) <Noun>` with
/// both sides resolving to declared nouns. Treated as a typing
/// predicate rather than a fact-type reference.
fn is_subtype_instance_check(clause: &str, noun_names: &[String]) -> bool {
    let trimmed = clause.trim();
    [" is a ", " is an "].iter().any(|kw| {
        let Some(idx) = trimmed.find(kw) else { return false; };
        let lhs = trimmed[..idx].trim();
        let rhs = trimmed[idx + kw.len()..].trim();
        let is_noun = |s: &str| noun_names.iter().any(|n| n == s);
        is_noun(lhs) && is_noun(rhs)
    })
}

/// True when `clause` uses a word-based comparator
/// (`exceeds`, `is greater than`, `is less than`, `is at least`,
///  `is at most`, `is more than`, `equals`, `is equal to`)
/// and both operand sides reference a declared noun. The payload
/// itself isn't compiled here — classification only suppresses
/// the "unresolved clause" diagnostic for the legitimate comparison
/// form.
/// #277 Category F — `<FT-reference> within|before|after <tail>` is
/// a binary FT lookup with an implicit range filter on the trailing
/// role. Recognised when splitting on the range operator yields a
/// head that resolves through the catalog; the tail is left as an
/// anaphoric binding. Patterns like `Log Entry has Timestamp within
/// that Interval` and `Timestamp is before that Fresh Until` appear
/// across service-health.md, data-pipeline.md, and eu-law corpora.
fn is_range_filter_clause(
    clause: &str,
    noun_names: &[String],
    catalog: &SchemaCatalog,
) -> bool {
    const RANGE_OPS: &[&str] = &[" within ", " before ", " after "];
    RANGE_OPS.iter().any(|op| {
        let Some(idx) = clause.find(op) else { return false; };
        let head = clause[..idx].trim();
        head_resolves(head, noun_names, catalog)
    })
}

/// #277 Category F — bare-value tail comparisons like
/// `HTTP Status of 500 or more`, `HTTP Status of 500 or less`,
/// `HTTP Status of at least 500`, `HTTP Status of at most 500`.
/// The FT reference is the subject noun; the `of <N> <comparator>`
/// tail is an implicit comparator filter on the value side.
fn is_bare_value_comparison(clause: &str, noun_names: &[String]) -> bool {
    const TAILS: &[&str] = &[
        " or more", " or less", " or greater", " or fewer",
    ];
    let trimmed = clause.trim().trim_end_matches('.');
    let ends_with_tail = TAILS.iter().any(|t| trimmed.ends_with(t));
    if !ends_with_tail { return false; }
    // The clause must contain " of " followed by a numeric literal
    // and reference at least one declared noun on the left side.
    let Some(of_idx) = trimmed.find(" of ") else { return false; };
    let head = trimmed[..of_idx].trim();
    let head_has_noun = noun_names.iter().any(|n| {
        head == n
            || head.starts_with(&alloc::format!("{} ", n))
            || head.ends_with(&alloc::format!(" {}", n))
            || head.contains(&alloc::format!(" {} ", n))
    });
    if !head_has_noun { return false; }
    // Token after " of " must be a numeric literal (decimal, possibly
    // signed). Reject quoted-value forms which belong to the
    // ref-scheme-literal classifier.
    let after_of = trimmed[of_idx + " of ".len()..].trim_start();
    let first_token = after_of.split_whitespace().next().unwrap_or("");
    first_token.parse::<f64>().is_ok()
}

fn is_word_comparator_clause(clause: &str, noun_names: &[String]) -> bool {
    const COMPARATORS: &[&str] = &[
        " exceeds ", " is greater than ", " is less than ",
        " is at least ", " is at most ", " is more than ",
        " equals ", " is equal to ",
    ];
    COMPARATORS.iter().any(|kw| {
        let Some(idx) = clause.find(kw) else { return false; };
        let lhs = clause[..idx].trim();
        let rhs = clause[idx + kw.len()..].trim();
        let side_has_noun = |side: &str| noun_names.iter().any(|n| {
            // Whole-side match or noun as a whole-word substring.
            side == n
                || side.starts_with(&format!("{} ", n))
                || side.ends_with(&format!(" {}", n))
                || side.contains(&format!(" {} ", n))
        });
        side_has_noun(lhs) && side_has_noun(rhs)
    })
}

/// Strip a trailing ` (word-or-phrase)` documentation annotation.
/// Used by `try_ring` so readings of the form
/// `No X R itself. (irreflexive)` or
/// `If some X1 R some X2 then that X2 R that X1. (symmetric)`
/// parse the same as the un-annotated forms.
fn strip_trailing_annotation(text: &str) -> &str {
    let trimmed = text.trim_end();
    let Some(close_rel) = trimmed.strip_suffix(')') else { return text; };
    let Some(open_pos) = close_rel.rfind(" (") else { return text; };
    let prefix = &close_rel[..open_pos];
    // The bit between `(` and `)` must not itself contain parentheses
    // (avoid truncating legitimate content with nested annotations).
    let inner = &close_rel[open_pos + 2..];
    if inner.contains('(') || inner.contains(')') { return text; }
    prefix
}

/// try_ring_shorthand â€” ORM 2 intuitive-icon parity for ring constraints.
///
/// Accepts terse adjectival form appended to a ring fact-type reading:
///   `Category has parent Category is acyclic.`
///   `Task blocks Task is irreflexive.`
///   `Person is sibling of Person is symmetric.`
/// instead of Halpin's canonical prose ("No Category may cycle back to
/// itself via one or more traversals through has parent."). Maps the
/// adjective 1-to-1 to the existing 8 ring constraint kinds
/// (IR/AS/AT/SY/IT/TR/AC/RF) that compile.rs already knows how to
/// evaluate, so no compile-side change is needed.
///
/// Discrimination vs non-ring "X is Y" sentences: the reading LHS must
/// mention the same base noun at least twice (before and after the
/// verb). That rules out `Noun is irreflexive` as a bare adjective
/// claim about a noun.
fn try_ring_shorthand(line: &str, noun_names: &[String]) -> Option<ParseAction> {
    let clean = line.trim_end_matches('.').trim();
    let re = regex::Regex::new(
        r"^(.+?)\s+is\s+(irreflexive|asymmetric|antisymmetric|symmetric|intransitive|transitive|acyclic|reflexive)$"
    ).expect("static regex compiles");
    let caps = re.captures(clean)?;
    let reading = caps.get(1)?.as_str().trim();
    let adj = caps.get(2)?.as_str();
    let kind = match adj {
        "irreflexive"   => "IR",
        "asymmetric"    => "AS",
        "antisymmetric" => "AT",
        "symmetric"     => "SY",
        "intransitive"  => "IT",
        "transitive"    => "TR",
        "acyclic"       => "AC",
        "reflexive"     => "RF",
        _               => return None,
    };
    // Ring-shorthand requires the reading to mention the same base noun
    // at least twice â€” otherwise it's ambiguous with a bare-adjective
    // claim (`X is symmetric` on some non-fact-type X).
    let base_counts: hashbrown::HashMap<&str, usize> = reading
        .split_whitespace()
        .filter_map(|w| {
            let w = w.trim_end_matches(',').trim_end_matches('.');
            let (base, _) = parse_role_token(w);
            noun_names.iter().any(|n| n == base).then_some(base)
        })
        .fold(hashbrown::HashMap::new(), |mut acc, b| {
            *acc.entry(b).or_insert(0) += 1;
            acc
        });
    let (&ring_noun, _) = base_counts.iter().find(|(_, &c)| c >= 2)?;
    Some(ParseAction::AddConstraint(ConstraintDef {
        id: String::new(), kind: kind.into(), modality: "alethic".into(),
        deontic_operator: None, text: clean.into(),
        spans: vec![], set_comparison_argument_length: None, clauses: None,
        entity: Some(ring_noun.to_string()),
        min_occurrence: None, max_occurrence: None,
    }))
}

/// try_subset -- SS: "If some A R1 some B then that A R2 that B."
/// Distinguishes from ring: subset has multiple different base noun types.
fn try_subset(line: &str, noun_names: &[String]) -> Option<ParseAction> {
    let clean = line.trim_end_matches('.');
    // Must start with "If " and contain " then "
    clean.starts_with("If ").then_some(())?;
    let then_idx = clean.find(" then ")?;
    let antecedent = &clean[3..then_idx];
    let consequent = &clean[then_idx + 6..];

    // Antecedent must contain "some" (existential), consequent must contain "that" (back-ref)
    antecedent.contains("some ").then_some(())?;
    consequent.contains("that ").then_some(())?;

    // Collect base noun types from antecedent using find_nouns (handles multi-word nouns)
    let stripped_ant = antecedent.replace("some ", "").replace("that ", "");
    let ant_found = find_nouns(&stripped_ant, noun_names);
    let ant_bases: Vec<&str> = ant_found.iter().map(|(_, _, n)| n.as_str()).collect();

    (ant_bases.len() >= 2).then_some(())?;

    // Subset has multiple DIFFERENT base noun types (distinguishes from ring which has all same)
    let first = ant_bases[0];
    (!ant_bases.iter().all(|b| b == &first)).then_some(())?;

    // Build spans: [0] = subset (antecedent), [1] = superset (consequent)
    // SpanDef with empty fact_type_id -- resolve_constraint_schema fills it in later
    let spans = vec![
        SpanDef { fact_type_id: String::new(), role_index: 0, subset_autofill: None },
        SpanDef { fact_type_id: String::new(), role_index: 0, subset_autofill: None },
    ];

    Some(ParseAction::AddConstraint(ConstraintDef {
        id: String::new(), kind: "SS".into(), modality: "alethic".into(),
        deontic_operator: None, text: clean.into(),
        spans, set_comparison_argument_length: None, clauses: None,
        entity: None, min_occurrence: None, max_occurrence: None,
    }))
}

/// try_equality -- EQ: "...if and only if..." or "all or none of the following hold:..."
fn try_equality(line: &str) -> Option<ParseAction> {
    let clean = line.trim_end_matches('.');
    let matches = clean.contains(" if and only if ")
        || clean.to_lowercase().starts_with("all or none of the following hold");
    matches.then_some(())?;
    Some(ParseAction::AddConstraint(ConstraintDef {
        id: String::new(), kind: "EQ".into(), modality: "alethic".into(),
        deontic_operator: None, text: clean.into(),
        spans: vec![], set_comparison_argument_length: None, clauses: None,
        entity: None, min_occurrence: None, max_occurrence: None,
    }))
}

/// try_set_comparison -- XO, XC, OR
/// Patterns:
///   "For each A, exactly one of the following holds: ..." -> XO
///   "For each A, at most one of the following holds: ..."  -> XC
///   "For each A, at least one of the following holds: ..." -> OR (inclusive disjunction)
///   "Each A R1 some B1 or R2 some B2."                   -> OR (DMaC disjunctive MC)
fn try_set_comparison(line: &str, noun_names: &[String]) -> Option<ParseAction> {
    let clean = line.trim_end_matches('.');

    // "For each X, <quantifier> of the following holds: clause1; clause2..."
    if let Some(rest) = clean.strip_prefix("For each ") {
        let comma = rest.find(',')?;
        let entity = rest[..comma].trim().to_string();
        let body = rest[comma + 1..].trim();

        let (kind, after_quant) = if let Some(r) = body.strip_prefix("exactly one of the following holds:") {
            ("XO", r)
        } else if let Some(r) = body.strip_prefix("at most one of the following holds:") {
            ("XC", r)
        } else if let Some(r) = body.strip_prefix("at least one of the following holds:") {
            ("OR", r)
        } else {
            return None;
        };

        let clauses: Vec<String> = after_quant
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        return Some(ParseAction::AddConstraint(ConstraintDef {
            id: String::new(), kind: kind.into(), modality: "alethic".into(),
            deontic_operator: None, text: clean.into(),
            spans: vec![], set_comparison_argument_length: Some(clauses.len()),
            clauses: Some(clauses),
            entity: Some(entity), min_occurrence: None, max_occurrence: None,
        }));
    }

    // "Each A R1 some B1 or R2 some B2." -- DMaC disjunctive MC -> OR
    if let Some(rest) = clean.strip_prefix("Each ") {
        // Must contain " or " and reference known nouns
        rest.contains(" or ").then_some(())?;
        // Find a known entity noun at the start
        let entity = noun_names.iter().find(|n| rest.starts_with(n.as_str()))?.clone();
        let after = rest[entity.len()..].trim();
        // Must have " or " in the remainder (not " or a/an " as in totality)
        after.contains(" or ").then_some(())?;
        // Exclude totality pattern: "Each X is a Y or a Z" (handled by try_totality)
        let or_idx = after.find(" or ")?;
        let after_or = &after[or_idx + 4..];
        // Totality uses "a " / "an " after "or"; disjunctive MC uses a predicate verb
        let is_totality = after_or.starts_with("a ") || after_or.starts_with("an ");
        (!is_totality).then_some(())?;

        let clauses = vec![
            after[..or_idx].trim().to_string(),
            after_or.trim().to_string(),
        ];

        return Some(ParseAction::AddConstraint(ConstraintDef {
            id: String::new(), kind: "OR".into(), modality: "alethic".into(),
            deontic_operator: None, text: clean.into(),
            spans: vec![], set_comparison_argument_length: Some(clauses.len()),
            clauses: Some(clauses),
            entity: Some(entity), min_occurrence: None, max_occurrence: None,
        }));
    }

    None
}

/// try_frequency -- FC: "Each A R at least {k} and at most {m} B."
/// MUST fire before try_constraint because "at least 1" (digit) is FC
/// while "at least one" (word) is MC. try_constraint would misclassify it.
fn try_frequency(line: &str, noun_names: &[String]) -> Option<ParseAction> {
    let clean = line.trim_end_matches('.');
    let rest = clean.strip_prefix("Each ")?;

    // Pattern: "at least {digit}" somewhere in line
    let at_least_idx = rest.find("at least ")?;
    let after_al = &rest[at_least_idx + 9..];

    // The digit must come immediately after "at least " (not a word like "one")
    let min_end = after_al.find(|c: char| !c.is_ascii_digit())?;
    (min_end > 0).then_some(())?; // no digit found
    let min_str = &after_al[..min_end];
    let min_val: usize = min_str.parse().ok()?;

    // Look for optional "and at most {digit}"
    let max_val: Option<usize> = after_al[min_end..].find("at most ")
        .and_then(|i| {
            let after_am = &after_al[min_end + i + 8..];
            let max_end = after_am.find(|c: char| !c.is_ascii_digit()).unwrap_or(after_am.len());
            (max_end > 0).then_some(())?;
            after_am[..max_end].parse().ok()
        });

    // Find the entity noun(s) to build spans
    let stripped = clean
        .replace("Each ", "")
        .replace(&format!("at least {}", min_str), "")
        .replace("and at most", "");
    // Remove max digit if present
    let stripped = if let Some(mv) = max_val {
        stripped.replace(&mv.to_string(), "")
    } else {
        stripped
    };
    let found = find_nouns(&stripped, noun_names);

    let spans: Vec<SpanDef> = found.iter().enumerate()
        .map(|(i, _)| SpanDef { fact_type_id: String::new(), role_index: i, subset_autofill: None })
        .collect();

    Some(ParseAction::AddConstraint(ConstraintDef {
        id: String::new(), kind: "FC".into(), modality: "alethic".into(),
        deontic_operator: None, text: clean.into(),
        spans, set_comparison_argument_length: None, clauses: None,
        entity: None, min_occurrence: Some(min_val), max_occurrence: max_val,
    }))
}

/// try_external_uc -- UC (external uniqueness and context pattern)
/// Patterns:
///   "For each B1 and B2, at most one A R1 that B1 and R2 that B2."
///   "Context: F1; F2. In this context, each B1, B2 combination is associated with at most one A."
fn try_external_uc(line: &str, noun_names: &[String]) -> Option<ParseAction> {
    let clean = line.trim_end_matches('.');

    // Context pattern: starts with "Context:"
    if clean.starts_with("Context:") {
        if clean.contains("at most one") || clean.contains("combination is associated with") {
            // Find the entity noun mentioned in "at most one A"
            let entity = noun_names.iter().find(|n| {
                let pattern = format!("at most one {}", n);
                clean.contains(&pattern)
            }).cloned();
            return Some(ParseAction::AddConstraint(ConstraintDef {
                id: String::new(), kind: "UC".into(), modality: "alethic".into(),
                deontic_operator: None, text: clean.into(),
                spans: vec![], set_comparison_argument_length: None, clauses: None,
                entity, min_occurrence: None, max_occurrence: None,
            }));
        }
        return None;
    }

    // "For each B1 and B2, at most one A ..."
    if let Some(rest) = clean.strip_prefix("For each ") {
        // Must have "at most one" in the body
        clean.contains("at most one").then_some(())?;
        // Must have " and " in the "For each" list (external UC uses "B1 and B2")
        let comma_idx = rest.find(',')?;
        let quantified = &rest[..comma_idx];
        quantified.contains(" and ").then_some(())?;

        // Find the entity noun after "at most one"
        let after_amo = clean.find("at most one ")?;
        let noun_start = after_amo + 12;
        let entity = noun_names.iter().find(|n| {
            clean[noun_start..].starts_with(n.as_str())
        }).cloned();

        return Some(ParseAction::AddConstraint(ConstraintDef {
            id: String::new(), kind: "UC".into(), modality: "alethic".into(),
            deontic_operator: None, text: clean.into(),
            spans: vec![], set_comparison_argument_length: None, clauses: None,
            entity, min_occurrence: None, max_occurrence: None,
        }));
    }

    None
}

fn try_deontic(line: &str) -> Option<ParseAction> {
    let (operator, rest) = line.strip_prefix("It is obligatory that ").map(|r| ("obligatory", r))
        .or_else(|| line.strip_prefix("It is forbidden that ").map(|r| ("forbidden", r)))
        .or_else(|| line.strip_prefix("It is permitted that ").map(|r| ("permitted", r)))?;
    // Extract the entity noun: first capitalized phrase after the operator prefix.
    // e.g., "each Support Response uses Dash" -> entity = "Support Response"
    let entity_rest = rest.strip_prefix("each ").or(Some(rest)).unwrap();
    // The entity is the leading capitalized words before a lowercase verb
    let entity: String = entity_rest.split_whitespace()
        .take_while(|w| w.chars().next().map_or(false, |c| c.is_uppercase()))
        .collect::<Vec<&str>>()
        .join(" ");
    let entity_opt = if entity.is_empty() { None } else { Some(entity) };
    // Create a placeholder span so resolve_constraint_schema can populate it
    Some(ParseAction::AddConstraint(ConstraintDef {
        id: String::new(), kind: operator.into(), modality: "deontic".into(),
        deontic_operator: Some(operator.into()),
        text: line.trim_end_matches('.').into(),
        spans: vec![SpanDef { fact_type_id: String::new(), role_index: 0, subset_autofill: None }],
        set_comparison_argument_length: None, clauses: None,
        entity: entity_opt, min_occurrence: None, max_occurrence: None,
    }))
}

fn try_instance_fact(line: &str) -> Option<ParseAction> {
    // An instance fact contains a quoted value: NounName 'value' predicate ...
    // Must contain at least one single-quoted value and not be a constraint or enum.
    let has_quote = line.contains('\'');
    let is_enum = line.contains("The possible values of");
    let is_constraint_prefix = line.starts_with("Each ") || line.starts_with("For each ")
        || line.starts_with("It is ") || line.starts_with("No ")
        || line.starts_with("If ") || line.starts_with("Context:");
    (has_quote && !is_enum && !is_constraint_prefix)
        .then(|| ParseAction::AddInstanceFact(line.into()))
}

fn try_derivation(line: &str) -> Option<ParseAction> {
    // ORM 2 derivation markers (* / ** / +) may prefix a derivation rule
    // to visually signal the derivation mode that was already declared by
    // the suffix on the corresponding fact-type reading. The mode itself
    // is carried via the `Fact Type has Derivation Mode` instance fact
    // emitted at reading-parse time; the prefix here is a readability aid
    // that the parser tolerates and strips.
    let stripped = line
        .strip_prefix("** ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))
        .unwrap_or(line);

    // " if " mid-sentence is a derivation rule (Consequent if Antecedent).
    // Lines starting with "If ... then ..." are conditional derivation rules.
    // Lines starting with "If " without " then " are constraints.
    let has_if = stripped.contains(" if ") && !stripped.starts_with("If ");
    let is_conditional = stripped.starts_with("If ") && stripped.contains(" then ");
    let has_marker = stripped.contains(" iff ")
        || has_if
        || is_conditional
        || stripped.contains(" := ")
        || stripped.contains(" is derived as ")
        || (stripped.starts_with("For each ") && stripped.contains(" = "))
        || stripped.contains("count each")
        || stripped.contains("sum(");
    has_marker.then(|| {
        let clean = stripped.trim_end_matches('.');
        ParseAction::AddDerivation(DerivationRuleDef {
            id: String::new(), text: clean.into(),
            antecedent_fact_type_ids: vec![], consequent_fact_type_id: String::new(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![], match_on: vec![], consequent_bindings: vec![], antecedent_filters: vec![], consequent_computed_bindings: vec![], consequent_aggregates: vec![], unresolved_clauses: vec![],
        })
    })
}

/// try_span_naming -- "This span with A, B provides the preferred identification scheme for SpanName."
fn try_span_naming(line: &str) -> Option<ParseAction> {
    let rest = line.strip_prefix("This span with ")?;
    let pivot = rest.find(" provides the preferred identification scheme for ")?;
    let nouns_part = &rest[..pivot];
    let name_part = &rest[pivot + " provides the preferred identification scheme for ".len()..];
    let span_name = name_part.trim_end_matches('.').trim().to_string();
    let role_nouns: Vec<String> = nouns_part.split(',').map(|s| s.trim().to_string()).collect();
    (!span_name.is_empty() && !role_nouns.is_empty())
        .then(|| ParseAction::AddNamedSpan(span_name, role_nouns))
}

/// try_autofill_declaration -- "Constraint Span 'SpanName' autofills from superset."
fn try_autofill_declaration(line: &str) -> Option<ParseAction> {
    let rest = line.strip_prefix("Constraint Span '")?;
    let end_quote = rest.find('\'')?;
    let span_name = rest[..end_quote].to_string();
    let after = rest[end_quote + 1..].trim();
    after.strip_prefix("autofills from superset")?;
    Some(ParseAction::AddAutofillSpan(span_name))
}

fn try_constraint(line: &str, noun_names: &[String]) -> Option<ParseAction> {
    let starts_ok = line.starts_with("Each ") || line.starts_with("No ");
    starts_ok.then(|| ())?;
    let c = parse_constraint(line, noun_names)?;
    Some(ParseAction::AddConstraint(c))
}

fn try_fact_type(line: &str, noun_names: &[String]) -> Option<ParseAction> {
    // Instance facts have a quoted value immediately after the subject noun:
    // NounName 'value' predicate ...
    // Fact type readings may contain quotes later (object position) but not
    // right after the first noun. Check by finding the first noun and seeing
    // if a quote follows it.
    (!noun_names.iter().any(|noun| line.starts_with(&format!("{} '", noun)))).then_some(())?;
    let (ft_id, ft_def, mode) = parse_fact(line, noun_names)?;
    Some(ParseAction::AddFactType(ft_id, ft_def, mode))
}

// =========================================================================
// Main parser -- fold recognizers over lines
// =========================================================================

/// Parse with pre-existing nouns from other domains.
/// Domains are NORMA tabs. Nouns are global across the UoD.
fn parse_markdown_with_nouns(input: &str, existing_nouns: &HashMap<String, NounDef>) -> Result<ParseCtx, String> {
    parse_markdown_with_context(input, existing_nouns, &HashMap::new())
}

fn parse_markdown_with_context(input: &str, existing_nouns: &HashMap<String, NounDef>, existing_fact_types: &HashMap<String, FactTypeDef>) -> Result<ParseCtx, String> {
    // Metamodel namespace protection (security #23):
    // First parse the input in isolation to see which nouns IT actually declares.
    // If the input declares any metamodel-reserved noun AND that noun is already
    // present in `existing_nouns` (i.e. this is a user domain layered on top of
    // the metamodel bootstrap), reject. The bootstrap case (no existing nouns
    // for those names) is allowed to declare them exactly once.
    let mut standalone = ParseCtx {
        derivation_rules: vec![],
        cells: HashMap::new(),
    };
    parse_into(&mut standalone, input)?;
    // Metamodel namespace guard (#23). Skipped during bundled metamodel
    // bootstrap â€” the metamodel is loaded as a series of cross-referencing
    // files and legitimately redeclares the same reserved nouns.
    if !is_bootstrap_mode() {
        if let Some(reserved) = METAMODEL_NOUNS.iter()
            .find(|n| noun_exists(&standalone.cells, n) && existing_nouns.contains_key(**n))
        {
            return Err(format!("metamodel noun '{}' cannot be redeclared", reserved));
        }
    }

    let mut ir = ParseCtx {
        derivation_rules: vec![],
        cells: HashMap::new(),
    };
    // #283 — seed the Noun + FactType + Role cells from the pre-existing
    // typed maps so the parser sees metamodel definitions as already
    // declared.
    for (name, def) in existing_nouns.iter() {
        upsert_noun(&mut ir, name, def);
    }
    for (id, def) in existing_fact_types.iter() {
        upsert_fact_type(&mut ir, id, def);
    }
    parse_into(&mut ir, input)?;
    Ok(ir)
}

/// SSRF defense (#25). Reject URLs that point at internal/loopback/link-local
/// networks, file:// schemes, or internal DNS names. Hardcoded patterns only â€”
/// no DNS resolution, no network I/O. Called during platform_compile to
/// validate External System instance facts before they enter state.
pub fn is_forbidden_url(url: &str) -> bool {
    let trimmed = url.trim();
    let lower = trimmed.to_lowercase();

    // file:// scheme is always forbidden
    match lower.starts_with("file://") {
        true => return true,
        false => {}
    }

    // Extract the host component from http(s) URLs. Non-http schemes fall
    // through and are allowed (the check is scoped to federated HTTP URLs).
    let after_scheme = match lower.strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
    {
        Some(rest) => rest,
        None => return false,
    };

    // Strip userinfo (before '@'), then extract the host.
    let no_userinfo = after_scheme.rfind('@').map(|i| &after_scheme[i + 1..]).unwrap_or(after_scheme);

    // Bracketed IPv6 literal: [addr]:port/path -- must find the closing ']'
    // BEFORE searching for ':' (otherwise we split inside the brackets).
    // Bare host: split on the first '/', '?', or '#' to get the authority,
    // then heuristically detect bare IPv6 (authority has 2+ colons) vs the
    // normal host:port form (one colon).
    let host_bare: &str = match no_userinfo.strip_prefix('[') {
        Some(rest) => rest.find(']').map(|i| &rest[..i]).unwrap_or(rest),
        None => {
            let path_start = no_userinfo.find(|c: char| c == '/' || c == '?' || c == '#')
                .unwrap_or(no_userinfo.len());
            let authority = &no_userinfo[..path_start];
            // Bare IPv6 has multiple ':' in the authority (no port syntax
            // without brackets is well-defined, so treat the entire authority
            // as the host). host:port has exactly one ':' which we strip.
            match authority.matches(':').count() {
                0 => authority,
                1 => authority.split(':').next().unwrap_or(authority),
                _ => authority, // bare IPv6 â€” keep colons for ULA / link-local checks
            }
        }
    };

    // Empty host is bottom-safe â€” treat as forbidden.
    match host_bare.is_empty() {
        true => return true,
        false => {}
    }

    // Exact-name checks
    match host_bare {
        "localhost" | "::1" | "::" | "0.0.0.0" => return true,
        _ => {}
    }

    // Internal DNS suffixes (case-insensitive â€” lower already applied)
    let forbidden_suffix = host_bare.ends_with(".local")
        || host_bare.ends_with(".internal")
        || host_bare.ends_with(".localhost");
    match forbidden_suffix {
        true => return true,
        false => {}
    }

    // IPv4 checks: parse dotted-quad octets. Non-numeric hosts fall through.
    let octets: Vec<u16> = host_bare.split('.')
        .filter_map(|p| p.parse::<u16>().ok())
        .collect();
    let is_ipv4 = octets.len() == 4 && octets.iter().all(|o| *o <= 255);
    match is_ipv4 {
        true => {
            let (a, b) = (octets[0], octets[1]);
            // 127.*.*.* loopback
            // 10.*.*.* private
            // 169.254.*.* link-local (incl. AWS metadata 169.254.169.254)
            // 192.168.*.* private
            // 172.16-31.*.* private
            let forbidden_v4 = a == 127
                || a == 10
                || (a == 169 && b == 254)
                || (a == 192 && b == 168)
                || (a == 172 && b >= 16 && b <= 31);
            match forbidden_v4 {
                true => return true,
                false => {}
            }
        }
        false => {}
    }

    // IPv6 link-local: fe80::/10 â€” first octet of the address
    // is 0xfe and top two bits of the second are 10 (0x80..0xbf).
    // Covers fe80: through febf:.
    let ipv6_linklocal = host_bare.starts_with("fe8")
        || host_bare.starts_with("fe9")
        || host_bare.starts_with("fea")
        || host_bare.starts_with("feb");
    match ipv6_linklocal {
        true => return true,
        false => {}
    }

    // IPv6 unique-local: fc00::/7 (fc00 through fdff)
    let ipv6_ula = host_bare.starts_with("fc") || host_bare.starts_with("fd");
    // Only treat as ULA if the host looks like an IPv6 address (contains ':').
    match ipv6_ula && host_bare.contains(':') {
        true => return true,
        false => {}
    }

    false
}

/// Scan the InstanceFact cell in parsed state and return the first
/// forbidden URL found, if any. Used by platform_compile to reject
/// External System federation to internal/loopback/link-local hosts.
pub fn find_forbidden_instance_url(state: &crate::ast::Object) -> Option<String> {
    use crate::ast::{fetch_or_phi, binding};
    fetch_or_phi("InstanceFact", state)
        .as_seq()
        .and_then(|facts| {
            facts.iter().find_map(|f| {
                let object_value = binding(f, "objectValue")?;
                is_forbidden_url(object_value).then(|| object_value.to_string())
            })
        })
}

/// Parse FORML2 readings directly into an Object state.
/// Every declaration becomes a fact (cell) in state. No intermediate struct.
pub fn parse_to_state(input: &str) -> Result<crate::ast::Object, String> {
    let domain = parse_markdown(input)?;
    Ok(ctx_to_state(&domain))
}

/// Extract nouns directly from the Noun cell in D.
pub fn nouns_from_state(state: &crate::ast::Object) -> HashMap<String, NounDef> {
    use crate::ast::{fetch_or_phi, binding};
    fetch_or_phi("Noun", state)
        .as_seq().map(|facts| facts.iter().filter_map(|f| {
            let name = binding(f, "name")?.to_string();
            let obj_type = binding(f, "objectType").unwrap_or("entity").to_string();
            Some((name, NounDef { object_type: obj_type, world_assumption: WorldAssumption::default() }))
        }).collect())
        .unwrap_or_default()
}

/// Extract fact types directly from the FactType cell in D.
pub fn fact_types_from_state(state: &crate::ast::Object) -> HashMap<String, FactTypeDef> {
    use crate::ast::{fetch_or_phi, binding};
    fetch_or_phi("FactType", state)
        .as_seq().map(|facts| facts.iter().filter_map(|f| {
            let id = binding(f, "id")?.to_string();
            let reading = binding(f, "reading").unwrap_or("").to_string();
            Some((id, FactTypeDef {
                schema_id: String::new(),
                reading,
                readings: vec![],
                roles: vec![], // roles resolved separately if needed
            }))
        }).collect())
        .unwrap_or_default()
}

/// Parse FORML2 readings into an Object state with full context from D.
/// Extracts nouns and fact types directly from cells â€” no ParseCtx struct round-trip.
pub fn parse_to_state_from(input: &str, d: &crate::ast::Object) -> Result<crate::ast::Object, String> {
    let nouns = nouns_from_state(d);
    let fact_types = fact_types_from_state(d);
    let domain = parse_markdown_with_context(input, &nouns, &fact_types)?;
    Ok(ctx_to_state(&domain))
}

/// Legacy: parse with nouns only (no fact type context).
pub fn parse_to_state_with_nouns(input: &str, existing: &crate::ast::Object) -> Result<crate::ast::Object, String> {
    let nouns = nouns_from_state(existing);
    let domain = parse_markdown_with_nouns(input, &nouns)?;
    Ok(ctx_to_state(&domain))
}

/// Convert a ParseCtx to an Object state (sequence of cells).
/// Each category becomes a cell: <CELL, fact_type_id, <facts...>>
fn ctx_to_state(d: &ParseCtx) -> crate::ast::Object {
    use crate::ast::{Object, fact_from_pairs};
    use hashbrown::HashMap;
    // Seed with cells already emitted by apply_action (Constraint,
    // DerivationRule, etc.). Kinds that need cross-ref resolution (Noun,
    // FactType, Role, compound-ref-scheme) are emitted below from typed
    // fields; for write-only kinds already in d.cells, skip re-emission.
    let mut cells: HashMap<String, Vec<Object>> = d.cells.clone();
    let push = |cells: &mut HashMap<String, Vec<Object>>, name: &str, fact: Object| {
        cells.entry(name.to_string()).or_default().push(fact);
    };

    // #283 — Noun cell is populated directly by apply_action; no fallback
    // from a typed map is needed any longer.

    // #283 — FactType + Role cells populated directly by apply_action;
    // no fallback from a typed map is needed any longer.

    // #283 — Constraint cell is populated directly by apply_action
    // (plus the VC post-pass and autofill mutator); no fallback needed.

    // Derivation rules + unresolved-clause diagnostics.
    // apply_action / parse_into's finalize step emits these to d.cells;
    // the fallback path below handles test fixtures built from typed
    // fields only (e.g. ParseCtx literals in evaluate.rs).
    if !cells.contains_key("DerivationRule") {
        for r in &d.derivation_rules {
            // `mut` is only consumed under the `std-deps` feature; silence
            // the no-default-features warning without splitting the decl.
            #[allow(unused_mut)]
            let mut pairs: Vec<(&str, &str)> = vec![
                ("id", r.id.as_str()), ("text", r.text.as_str()),
                ("consequentFactTypeId", r.consequent_fact_type_id.as_str()),
            ];
            #[cfg(feature = "std-deps")]
            let json_blob = serde_json::to_string(r).unwrap_or_default();
            #[cfg(feature = "std-deps")]
            pairs.push(("json", json_blob.as_str()));
            push(&mut cells, "DerivationRule", fact_from_pairs(&pairs));
            for clause in &r.unresolved_clauses {
                push(&mut cells, "UnresolvedClause", fact_from_pairs(&[
                    ("ruleId", r.id.as_str()), ("ruleText", r.text.as_str()),
                    ("clause", clause.as_str()),
                ]));
            }
        }
    }

    // State machines are derived from instance facts at compile time
    // (derive_state_machines_from_facts in compile.rs). The parser does
    // not carry a separate state_machines index.

    // #283 — Instance facts and per-field cells are written directly
    // by `emit_instance_fact` during apply_action, and compound
    // ref-scheme decomposition runs in parse_into's finalize over
    // the InstanceFact cell. There's nothing left to translate from
    // typed state here.

    // Wrap into Object::Map in one pass: each cell becomes Object::Seq(facts).
    let map: HashMap<String, Object> = cells.into_iter()
        .map(|(k, v)| (k, Object::Seq(v.into())))
        .collect();
    Object::Map(map)
}


fn parse_markdown(input: &str) -> Result<ParseCtx, String> {
    let mut ir = ParseCtx {
        derivation_rules: vec![],
        cells: HashMap::new(),
    };
    parse_into(&mut ir, input)?;
    Ok(ir)
}

/// Re-resolve a rules vec given just the typed lookups it needs.
/// No ParseCtx struct required â€” callers pass their HashMaps directly.
pub(crate) fn re_resolve_rules(
    rules: &mut Vec<DerivationRuleDef>,
    nouns: &HashMap<String, NounDef>,
    fact_types: &HashMap<String, FactTypeDef>,
) {
    let mut noun_names: Vec<String> = nouns.keys().cloned().collect();
    noun_names.sort_by(|a, b| b.len().cmp(&a.len()));

    let mut catalog = SchemaCatalog::new();
    fact_types.iter().for_each(|(ft_id, ft)| {
        let role_nouns: Vec<&str> = ft.roles.iter().map(|r| r.noun_name.as_str()).collect();
        // Verb extraction: text after the first noun up to the second
        // (binary+), or everything after the single noun (unary — #274
        // Category A). Without the unary branch the catalog would
        // register `Customer is in EEA` with an empty verb, which
        // collides with every other unary keyed on [customer].
        let verb = noun_names.iter()
            .find(|n| ft.reading.starts_with(n.as_str()))
            .map(|first| {
                let after = &ft.reading[first.len()..];
                noun_names.iter()
                    .find_map(|second| after.find(second.as_str()).map(|pos| after[..pos].trim()))
                    .unwrap_or_else(|| after.trim())
            })
            .unwrap_or("");
        catalog.register(ft_id, &role_nouns, verb, &ft.reading);
    });

    rules.iter_mut().for_each(|rule| {
        resolve_derivation_rule(rule, nouns, fact_types, &catalog);
    });
}

/// Fold indented continuation lines into their derivation-rule head.
///
/// A derivation rule like
/// ```text
/// * Milestone has done Task Count iff done Task Count is the count of Task
///   where Task targets that Milestone
///   and Task has Status 'Done'.
/// ```
/// is authored across three physical lines for readability. The per-line
/// recognizers downstream only see the head, so the `where` and `and`
/// clauses are dropped and the resolver later flags the truncated
/// aggregate as unresolved.
///
/// Join rule: once a line is recognised as a derivation rule (contains
/// ` iff `, ` if ` used as marker, ` := `, or starts with a derivation
/// marker token `*` / `**` / `+`), swallow any subsequent lines that
/// start with whitespace until either a non-indented line appears or
/// the accumulated text terminates with `.`. Other indented lines
/// (constraint blocks under fact type headers, for example) pass
/// through unchanged.
fn join_derivation_continuations(input: &str) -> Vec<String> {
    let raw: Vec<String> = input.lines().map(|s| s.to_string()).collect();
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let line = &raw[i];
        let stripped = line.trim_start();
        let is_derivation_head = stripped.starts_with("* ")
            || stripped.starts_with("** ")
            || stripped.starts_with("+ ")
            || stripped.contains(" iff ")
            || (stripped.contains(" if ") && !stripped.starts_with("If "))
            || stripped.contains(" := ");
        if !is_derivation_head || line.trim_end().ends_with('.') {
            out.push(line.clone());
            i += 1;
            continue;
        }
        // Accumulate until a non-indented line or a `.`-terminated line.
        let mut joined = line.trim_end().to_string();
        let mut j = i + 1;
        while j < raw.len() {
            let cont = &raw[j];
            let is_indented = cont.starts_with(' ') || cont.starts_with('\t');
            if !is_indented || cont.trim().is_empty() { break; }
            joined.push(' ');
            joined.push_str(cont.trim());
            let terminated = joined.ends_with('.');
            j += 1;
            if terminated { break; }
        }
        out.push(joined);
        i = j;
    }
    out
}

fn parse_into(ir: &mut ParseCtx, input: &str) -> Result<(), String> {

    let lines: Vec<String> = join_derivation_continuations(input);

    // Pass 1: alpha(recognize_noun) : lines -- extract nouns and domain
    (0..lines.len()).for_each(|i| {
        let line = lines[i].trim();
        let action = None
            .or_else(|| try_header(line))
            .or_else(|| try_entity_type(line))
            .or_else(|| try_value_type(line))
            .or_else(|| try_subtype(line))
            .or_else(|| try_abstract(line))
            .or_else(|| try_partition(line))
            .or_else(|| try_exclusive_subtypes(line))
            .or_else(|| try_enum_values(line));

        apply_action(ir, action, &lines, i);

        // Look ahead for enum values after value type declaration:
        // Filter(non_empty) âˆ˜ skip(i+1) : lines, then match first result.
        line.strip_suffix(" is a value type.")
            .map(|prefix| prefix.trim())
            .and_then(|name| lines.iter().skip(i + 1)
                .map(|s| s.trim())
                .find(|s| !s.is_empty())
                .filter(|s| s.starts_with("The possible values of"))
                .and_then(parse_enum)
                .map(|vals| (name.to_string(), vals)))
            .into_iter()
            .for_each(|(name, vals)| { upsert_enum_values(ir, &name, &vals); });
    });

    // Pass 2a: collect fact types and instance facts
    // Sorted longest-first for Theorem 1 (unambiguous longest-first matching)
    let mut noun_names: Vec<String> = noun_names(&ir.cells);
    noun_names.sort_by(|a, b| b.len().cmp(&a.len()));

    // Pass 2a: Filter(!pass1 && !pass2b) : lines, then apply fact_type/instance_fact
    (0..lines.len())
        .map(|i| (i, lines[i].trim()))
        .filter(|(_, line)| !line.is_empty())
        .filter(|(_, line)| {
            let is_pass1 = try_entity_type(line).is_some()
                || try_value_type(line).is_some()
                || (try_subtype(line).is_some() && !line.starts_with("Each"))
                || try_abstract(line).is_some()
                || try_partition(line).is_some()
                || try_enum_values(line).is_some()
                || try_exclusive_subtypes(line).is_some()
                || try_association(line).is_some()
                || try_header(line).is_some();
            !is_pass1
        })
        .filter(|(_, line)| {
            // Preserves the original recognizer priority: these recognizers fire before try_fact_type
            let is_pass2b = try_ring(line, &noun_names).is_some()
                || try_ring_shorthand(line, &noun_names).is_some()
                || try_subset(line, &noun_names).is_some()
                || try_equality(line).is_some()
                || try_set_comparison(line, &noun_names).is_some()
                || try_frequency(line, &noun_names).is_some()
                || try_external_uc(line, &noun_names).is_some()
                || try_span_naming(line).is_some()
                || try_autofill_declaration(line).is_some()
                || try_derivation(line).is_some()
                || try_deontic(line).is_some()
                || try_constraint(line, &noun_names).is_some()
                || try_totality(line).is_some();
            !is_pass2b
        })
        .for_each(|(i, line)| {
            let action = try_fact_type(line, &noun_names)
                .or_else(|| try_instance_fact(line));
            apply_action(ir, action, &lines, i);
        });

    // Build schema catalog from collected fact types (via FactType + Role cells, #283)
    let catalog = {
        let mut cat = SchemaCatalog::new();
        fact_types_from_cells(&ir.cells).iter().for_each(|(schema_id, ft)| {
            let role_nouns: Vec<&str> = ft.roles.iter().map(|r| r.noun_name.as_str()).collect();
            // Extract verb from reading: text between first and last noun
            let verb = ft.roles.first()
                .and_then(|r0| {
                    let after = ft.reading.find(&r0.noun_name)
                        .map(|i| &ft.reading[i + r0.noun_name.len()..]);
                    after.map(|a| {
                        ft.roles.last()
                            .and_then(|r1| a.find(&r1.noun_name).map(|j| a[..j].trim()))
                            .unwrap_or(a.trim())
                    })
                })
                .unwrap_or("");
            cat.register(schema_id, &role_nouns, verb, &ft.reading);
        });
        cat
    };

    // Pass 2a.5 (#214 follow-up): register derivation-rule consequents
    // as synthetic FTs in the catalog so later rules can reference
    // them as antecedents. Example: `Customer is in EEA iff ...`
    // declares a derived unary; seven other rules in auto.dev use
    // `Customer is in EEA` as an antecedent. Without this pre-pass
    // the resolver treats each of those as "unresolved" because the
    // catalog only knows declared FTs.
    //
    // Scan: any line containing a derivation marker (` iff `,
    // mid-sentence ` if `, ` := `, `* ` / `+ ` / `** ` prefix) has a
    // consequent on its left-hand side. The consequent text, when
    // tokenized against the noun catalog, yields the role set for a
    // synthetic FT the catalog should know about.
    let mut catalog = catalog;
    register_derivation_consequents_in_catalog(&lines, &noun_names, &mut catalog);

    // Pass 2b: Filter(!pass1) : lines, then apply constraint/derivation/deontic recognizers.
    (0..lines.len())
        .map(|i| (i, lines[i].trim()))
        .filter(|(_, line)| !line.is_empty())
        .filter(|(_, line)| {
            let is_pass1 = try_entity_type(line).is_some()
                || try_value_type(line).is_some()
                || (try_subtype(line).is_some() && !line.starts_with("Each"))
                || try_abstract(line).is_some()
                || try_partition(line).is_some()
                || try_enum_values(line).is_some()
                || try_exclusive_subtypes(line).is_some()
                || try_association(line).is_some()
                || try_header(line).is_some();
            !is_pass1
        })
        .for_each(|(i, line)| {
            // Totality -> mark abstract (but don't skip -- still parse as constraint)
            apply_action(ir, try_totality(line), &lines, i);

            // Try recognizers in priority order.
            // Ring and subset fire before derivation (both match "If...then...").
            // Frequency fires before constraint ("at least 1" digit vs "at least one" word).
            // External UC fires before constraint to handle "For each B1 and B2, at most one...".
            let action = None
                .or_else(|| try_ring(line, &noun_names))
                .or_else(|| try_ring_shorthand(line, &noun_names))
                .or_else(|| try_subset(line, &noun_names))
                .or_else(|| try_equality(line))
                .or_else(|| try_set_comparison(line, &noun_names))
                .or_else(|| try_frequency(line, &noun_names))
                .or_else(|| try_external_uc(line, &noun_names))
                .or_else(|| try_span_naming(line))
                .or_else(|| try_autofill_declaration(line))
                .or_else(|| try_derivation(line))
                .or_else(|| try_deontic(line))
                .or_else(|| try_constraint(line, &noun_names));

            // If no constraint/derivation/deontic matched, this line was already
            // handled in Pass 2a (fact type or instance fact). Skip it.
            let Some(action) = action else { return; };

            // Split inline "exactly one" constraints into UC + MC.
            // Skip the split for set-comparison kinds (XO, XC, OR) which carry their own semantics.
            // Derivation rules resolve through catalog, not through apply_action.
            let set_comparison_kinds = ["XO", "XC", "OR", "SS", "EQ", "FC"];
            match action {
                ParseAction::AddConstraint(ref c)
                    if line.contains("exactly one")
                        && !set_comparison_kinds.contains(&c.kind.as_str()) =>
                {
                    let c = match action { ParseAction::AddConstraint(c) => c, _ => unreachable!() };
                    // "exactly one" = UC + MC. Each gets its own reading as id.
                    let mut uc = resolve_constraint_schema(c.clone(), &noun_names, &catalog, ir);
                    uc.kind = "UC".into();
                    uc.text = uc.text.replace("exactly one", "at most one");
                    uc.id.is_empty().then(|| uc.id = uc.text.clone());
                    let mut mc = resolve_constraint_schema(c, &noun_names, &catalog, ir);
                    mc.kind = "MC".into();
                    mc.text = mc.text.replace("exactly one", "some");
                    mc.id.is_empty().then(|| mc.id = mc.text.clone());
                    #[cfg(feature = "std-deps")]
                    { push_cell(ir, "Constraint", constraint_to_fact(&uc));
                      push_cell(ir, "Constraint", constraint_to_fact(&mc)); }
                    // #283 — Constraint cell is the source; no typed vec.
                    let _ = (uc, mc);
                }
                ParseAction::AddConstraint(c) => {
                    let mut resolved = resolve_constraint_schema(c, &noun_names, &catalog, ir);
                    resolved.id.is_empty().then(|| resolved.id = resolved.text.clone());
                    #[cfg(feature = "std-deps")]
                    push_cell(ir, "Constraint", constraint_to_fact(&resolved));
                    // #283 — Constraint cell is the source; no typed vec.
                    let _ = resolved;
                }
                ParseAction::AddDerivation(mut r) => {
                    let nouns_map = nouns_from_cells(&ir.cells);
                    let fact_types_map = fact_types_from_cells(&ir.cells);
                    resolve_derivation_rule(&mut r, &nouns_map, &fact_types_map, &catalog);
                    ir.derivation_rules.push(r);
                }
                other => { apply_action(ir, Some(other), &lines, i); }
            }
        });

    // Task 6: Value Constraint (VC) -- emit one VC per noun with enum_values.
    // The compiler reads enum values from the EnumValues cell (#283);
    // the ConstraintDef just marks which noun has a value constraint.
    let vcs: Vec<ConstraintDef> = all_enum_value_nouns(&ir.cells).into_iter().map(|noun_name| ConstraintDef {
        id: format!("VC:{}", noun_name),
        kind: "VC".into(),
        modality: "alethic".into(),
        deontic_operator: None,
        text: format!("{} has a value constraint", noun_name),
        spans: vec![],
        set_comparison_argument_length: None,
        clauses: None,
        entity: Some(noun_name),
        min_occurrence: None,
        max_occurrence: None,
    }).collect();
    #[cfg(feature = "std-deps")]
    for c in &vcs { push_cell(ir, "Constraint", constraint_to_fact(c)); }
    // #283 — Constraint cell is the source; no typed vec.
    let _ = vcs;

    // Post-processing: resolve autofill spans.
    // For each autofill span name, find SS constraints whose role nouns
    // match the named span's role nouns, and set subset_autofill = Some(true).
    //
    // #283 — both sides query cells (NamedSpan, AutofillSpan) rather than
    // a parallel typed map. The cells are the parser's only state for
    // span metadata.
    let named_spans_by_name: HashMap<String, Vec<String>> = ir.cells.get("NamedSpan")
        .into_iter()
        .flat_map(|facts| facts.iter())
        .filter_map(|fact| {
            let name = crate::ast::binding(fact, "name")?.to_string();
            let mut roles: Vec<String> = Vec::new();
            let mut i: usize = 0;
            loop {
                let key = alloc::format!("role{i}");
                match crate::ast::binding(fact, &key) {
                    Some(v) => { roles.push(v.to_string()); i += 1; }
                    None => break,
                }
            }
            Some((name, roles))
        })
        .collect();
    let autofill_role_sets: Vec<hashbrown::HashSet<String>> = ir.cells.get("AutofillSpan")
        .into_iter()
        .flat_map(|facts| facts.iter())
        .filter_map(|fact| crate::ast::binding(fact, "name").map(String::from))
        .filter_map(|span_name| named_spans_by_name.get(&span_name))
        .map(|nouns| nouns.iter().cloned().collect())
        .collect();
    // #283 — mutate the Constraint cell in place (subset_autofill on first span
    // for any SS constraint whose role set matches an autofill-role-set).
    #[cfg(feature = "std-deps")]
    mark_ss_constraints_autofill(ir, &autofill_role_sets);

    // Finalize cells from typed fields after all post-processing. This
    // captures mutations applied after the per-arm emission (VC
    // extension, autofill span marking, derivation re-resolution, etc.)
    // and keeps the parse output Object-native per Thm 2.
    #[cfg(feature = "std-deps")]
    {
        use crate::ast::{Object, fact_from_pairs};

        // Noun: #283 — writers emit to the Noun cell during parse.
        // Finalize enriches each Noun fact with superType / referenceScheme
        // / enumValues bindings by joining the sibling cells.
        enrich_noun_cells(ir);

        // FactType + Role: #283 — apply_action's AddFactType branch
        // writes the FactType and Role cells directly via
        // `upsert_fact_type`. No finalize rebuild needed.

        // Constraint: #283 — writers emit to the Constraint cell during
        // parse; mark_ss_constraints_autofill rewrites facts in place.
        // No finalize rebuild needed.

        // DerivationRule + UnresolvedClause: ir.derivation_rules → cells
        let mut dr_facts: Vec<Object> = Vec::with_capacity(ir.derivation_rules.len());
        let mut uc_facts: Vec<Object> = Vec::new();
        for r in &ir.derivation_rules {
            let json = serde_json::to_string(r).unwrap_or_default();
            dr_facts.push(fact_from_pairs(&[
                ("id", r.id.as_str()), ("text", r.text.as_str()),
                ("consequentFactTypeId", r.consequent_fact_type_id.as_str()),
                ("json", json.as_str()),
            ]));
            for clause in &r.unresolved_clauses {
                uc_facts.push(fact_from_pairs(&[
                    ("ruleId", r.id.as_str()), ("ruleText", r.text.as_str()),
                    ("clause", clause.as_str()),
                ]));
            }
        }
        ir.cells.insert("DerivationRule".to_string(), dr_facts);
        if !uc_facts.is_empty() {
            ir.cells.insert("UnresolvedClause".to_string(), uc_facts);
        }

        // StateMachine cells are derived at compile time from instance
        // facts ("State Machine Definition 'X' is for Noun 'Y'" etc.) by
        // derive_state_machines_from_facts. The parser never writes
        // ir.state_machines, so there's nothing to emit here.

        // #283 — InstanceFact + per-field cells are written directly
        // by `emit_instance_fact` at the writer sites (AddFactType
        // derivation-mode expansion and parse_instance_fact). No
        // typed→cell translation pass is needed.

        // Compound reference-scheme decomposition: for each entity with
        // ≥2 ref parts, split instance IDs on '-' from the right and
        // push component facts to {Noun}_has_{Component} cells.
        // #283 — snapshot all_ref_schemes before the cell-mutation loop
        // to avoid borrowing `ir.cells` immutably and mutably in
        // overlapping scopes.
        use hashbrown::HashSet as HBSet;
        let compound_schemes: Vec<(String, Vec<String>)> = all_ref_schemes(&ir.cells)
            .into_iter()
            .filter(|(_, p)| p.len() >= 2)
            .collect();
        for (noun_name, ref_parts) in compound_schemes {
            let ids: HBSet<String> = instance_fact_subject_values_for_noun(&ir.cells, &noun_name)
                .into_iter()
                .collect();
            for id in &ids {
                let parts: Vec<&str> = id.rsplitn(ref_parts.len(), '-').collect::<Vec<_>>();
                let parts: Vec<&str> = parts.into_iter().rev().collect();
                if parts.len() != ref_parts.len() { continue; }
                for (component, value) in ref_parts.iter().zip(parts.iter()) {
                    let cell_name = format!("{}_has_{}",
                        noun_name.replace(' ', "_"), component.replace(' ', "_"));
                    ir.cells.entry(cell_name).or_default().push(fact_from_pairs(&[
                        (noun_name.as_str(), id.as_str()),
                        (component.as_str(), *value),
                    ]));
                }
            }
        }
    }

    // Strict mode: reject undeclared nouns (subtype children, fact type roles).
    if is_strict_mode() {
        let role_noun_names: Vec<String> = ir.cells.get("Role")
            .map(|facts| facts.iter()
                .filter_map(|f| crate::ast::binding(f, "nounName").map(String::from))
                .collect())
            .unwrap_or_default();
        let undeclared: Vec<String> = all_subtype_names(&ir.cells).into_iter()
            .filter(|sub| !noun_exists(&ir.cells, sub))
            .chain(role_noun_names.into_iter()
                .filter(|n| !noun_exists(&ir.cells, n)))
            .collect::<alloc::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        if !undeclared.is_empty() {
            return Err(format!("strict mode: undeclared nouns: {}", undeclared.join(", ")));
        }
    }

    Ok(())
}

/// Recognize a Halpin aggregate antecedent of form
///   `<role> is the <op> of <target> where <where-clause>`
/// where <op> âˆˆ {count, sum, avg, min, max}. The where-clause is a fact-
/// type reading that will be resolved separately against the catalog.
///
/// Returns (consequent_role, op, target_role, where_clause_text). The
/// caller then resolves the where-clause to a source FT id and pins the
/// group_key_role on it.
fn try_parse_aggregate_clause(text: &str, noun_names: &[String]) -> Option<(String, String, String, String)> {
    let t = text.trim().trim_end_matches('.').trim();
    let t = t.strip_prefix("that ").unwrap_or(t);
    // `where <filter>` is optional — `done Task Count is the count of Task`
    // (no where clause) is as valid as the filtered form. The op list
    // covers count/sum/avg/min/max plus their prose equivalents
    // (`earliest` / `latest` / `first` / `last`) which appear in
    // time-series readings like `Date is the earliest Timestamp`.
    let re = regex::Regex::new(
        r"^(.+?) is the (count|sum|avg|min|max|earliest|latest|first|last) of (.+?)(?: where (.+))?$"
    ).expect("static regex compiles");
    let caps = re.captures(t)?;
    let role = caps.get(1)?.as_str().trim().to_string();
    let op = caps.get(2)?.as_str().to_string();
    let target = caps.get(3)?.as_str().trim().to_string();
    let where_clause = caps.get(4).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
    // Target must resolve against the noun catalog — either the full
    // string is a declared noun, or its first space-separated token
    // is (for compound role paths like `LineItem Amount` meaning the
    // Amount role of LineItem). Role name is not required to be
    // declared: derivation rules may introduce implicit role names
    // for derived aggregates (e.g. `done Task Count`) that never
    // appear as standalone entity / value types.
    let target_resolves = noun_names.iter().any(|n| n == &target)
        || target.split_whitespace().next()
            .map_or(false, |first| noun_names.iter().any(|n| n == first));
    if !target_resolves { return None; }
    Some((role, op, target, where_clause))
}

/// Parse an arithmetic antecedent clause of Halpin FORML attribute-style
/// form: `<RoleName> is <expr>` (e.g. `Volume is Size * Size * Size`).
///
/// Returns `Some((role_name, expr))` when the clause matches that shape
/// AND the role name is a declared noun AND the RHS parses cleanly;
/// otherwise `None` so the caller can fall through to fact-type
/// resolution. Aggregate forms (`â€¦ is the sum of â€¦`) are explicitly
/// excluded â€” they're parsed by a later pipeline stage.
fn try_parse_computed_binding(text: &str, noun_names: &[String]) -> Option<(String, crate::types::ArithExpr)> {
    let t = text.trim().trim_end_matches('.').trim();
    let t = t.strip_prefix("that ").unwrap_or(t);
    // Aggregates use `is the <op> of â€¦` â€” skip them here.
    if t.contains(" is the ") { return None; }
    let idx = t.find(" is ")?;
    let lhs = t[..idx].trim();
    let rhs = t[idx + 4..].trim();
    // LHS must be a declared noun (role name).
    if !noun_names.iter().any(|n| n == lhs) { return None; }
    let expr = parse_arithmetic_expr(rhs, noun_names)?;
    Some((lhs.to_string(), expr))
}

/// Tokenize a whitespace-flexible arithmetic expression on `+ - * /` and
/// build a left-associative tree. Operands are either numeric literals
/// (f64::from_str) or declared noun names. No precedence yet â€” `A + B * C`
/// parses as `((A + B) * C)`. Parentheses are not yet supported either.
/// Returns `None` if any token fails to parse as an operand or operator.
fn parse_arithmetic_expr(text: &str, noun_names: &[String]) -> Option<crate::types::ArithExpr> {
    use crate::types::ArithExpr;
    let re = regex::Regex::new(r"\s*([+\-*/])\s*").expect("static regex compiles");
    let mut tokens: Vec<String> = Vec::new();
    let mut cursor = 0usize;
    for m in re.find_iter(text) {
        let head = text[cursor..m.start()].trim();
        if !head.is_empty() { tokens.push(head.to_string()); }
        tokens.push(m.as_str().trim().to_string());
        cursor = m.end();
    }
    let tail = text[cursor..].trim();
    if !tail.is_empty() { tokens.push(tail.to_string()); }
    if tokens.is_empty() { return None; }

    let parse_atom = |token: &str| -> Option<ArithExpr> {
        if let Ok(n) = token.parse::<f64>() { return Some(ArithExpr::Literal(n)); }
        if noun_names.iter().any(|n| n == token) { return Some(ArithExpr::RoleRef(token.to_string())); }
        None
    };

    let mut iter = tokens.into_iter();
    let first = iter.next()?;
    let mut result = parse_atom(&first)?;
    loop {
        let Some(op) = iter.next() else { break };
        if !matches!(op.as_str(), "+" | "-" | "*" | "/") { return None; }
        let next = iter.next()?;
        let rhs = parse_atom(&next)?;
        result = ArithExpr::Op(op, Box::new(result), Box::new(rhs));
    }
    Some(result)
}

/// Strip a trailing numeric comparator (Halpin FORML Example 5: `has Population >= 1000000`)
/// from an antecedent fragment. Returns `(stripped_text, Option<(op, value)>)`.
///
/// Accepts `>=`, `<=`, `>`, `<`, `=`, `!=`, and `<>` â€” the last is normalised
/// to `!=` so compile-time dispatch sees one canonical form. Longer operators
/// (`>=`, `<=`, `!=`, `<>`) are listed first in the alternation so the engine
/// prefers `>=` over `>` on input like `has Amount >= 100`.
fn split_antecedent_comparator(text: &str) -> (String, Option<(String, f64)>) {
    let re = regex::Regex::new(
        r"\s*(>=|<=|!=|<>|>|<|=)\s*(-?\d+(?:\.\d+)?)\s*$"
    ).expect("static regex compiles");
    match re.captures(text) {
        Some(caps) => {
            let whole = caps.get(0).unwrap();
            let stripped = text[..whole.start()].trim_end().to_string();
            let raw_op = caps.get(1).unwrap().as_str();
            let op = if raw_op == "<>" { "!=".to_string() } else { raw_op.to_string() };
            let value: f64 = caps.get(2).unwrap().as_str().parse().unwrap_or(0.0);
            (stripped, Some((op, value)))
        }
        None => (text.to_string(), None),
    }
}

/// Expand possessive syntax in a derivation body clause.
///
/// Pattern: `<Noun1>'s <Noun2>` is syntactic sugar for a join through Noun2:
///   `<Noun1>'s <Noun2> has <X>` â†’ `<Noun1> has <Noun2> and that <Noun2> has <X>`
///
/// This is a pre-processing step applied to the antecedent text before
/// fact-type resolution.  Each possessive token is replaced with an
/// explicit two-clause join so that the anaphora detector in
/// `resolve_derivation_rule` can find the `that <Noun2>` join key.
///
/// Returns `Some(expanded)` when at least one possessive was expanded,
/// `None` when the text contains no `'s` pattern.
///
/// # Examples
/// ```text
/// // Input antecedent clause:
/// "Order's Customer has Age"
/// // Expanded:
/// "Order has Customer and that Customer has Age"
/// ```
pub(crate) fn try_expand_possessive(text: &str, noun_names: &[String]) -> Option<String> {
    // Quick exit â€” no apostrophe means nothing to expand.
    if !text.contains("'s ") {
        return None;
    }

    // Walk the text looking for `<Noun>'s <Noun2>` sequences.
    // We use a simple left-to-right scan: find the first `'s `, identify the
    // noun that ends just before the apostrophe, identify the noun that begins
    // just after the space, then emit the expanded two-clause form.
    let mut result = text.to_string();
    let mut changed = false;

    // Iterate until no more `'s ` tokens remain (handles chained possessives).
    loop {
        let Some(apos_pos) = result.find("'s ") else { break };

        // Find noun1: the longest known noun ending at apos_pos.
        let prefix = &result[..apos_pos];
        let noun1 = noun_names.iter()
            .filter(|n| prefix.ends_with(n.as_str()))
            .max_by_key(|n| n.len())
            .cloned();

        // Find noun2: the longest known noun starting at apos_pos + 3.
        let after = &result[apos_pos + 3..]; // skip `'s `
        let noun2 = noun_names.iter()
            .filter(|n| after.starts_with(n.as_str()))
            .max_by_key(|n| n.len())
            .cloned();

        match (noun1, noun2) {
            (Some(n1), Some(n2)) => {
                // Build the expanded form:
                //   "<prefix-without-n1><n1> has <n2> and that <n2><suffix-without-n2>"
                let n1_start = apos_pos - n1.len();
                let n2_end = apos_pos + 3 + n2.len();
                let before_n1 = &result[..n1_start];
                let after_n2 = &result[n2_end..];
                result = format!(
                    "{}{} has {} and that {}{}",
                    before_n1, n1, n2, n2, after_n2
                );
                changed = true;
            }
            _ => {
                // Unknown noun around the apostrophe â€” leave as-is to avoid
                // corrupting input the parser can't understand.
                break;
            }
        }
    }

    changed.then_some(result)
}

/// Resolve a derivation rule's text into structured fact type references.
///
/// Splits on " if "/" iff " to get consequent and antecedent parts,
/// then matches each part's nouns against ir.fact_types by role noun names.
/// Anaphoric "that X" references are stripped to bare noun name "X".
///
/// Per-antecedent inline numeric comparisons (Halpin FORML Example 5) are
/// extracted via `split_antecedent_comparator` BEFORE fact-type resolution,
/// so `has Population >= 1000000` resolves to the base FT `has Population`
/// with an AntecedentFilter attached restricting that antecedent's population.
/// Temporal predicates are runtime clock checks with no declared FT.
fn is_temporal_predicate(clause: &str) -> bool {
    let l = clause.to_lowercase();
    l.contains("now is ") || l.contains(" in the past") || l.contains(" in the future")
        || l.contains("is current") || l.contains("is expired")
        || l.contains("is fresh") || l.contains("is stale")
}

fn resolve_derivation_rule(
    rule: &mut DerivationRuleDef,
    nouns_map: &HashMap<String, NounDef>,
    fact_types_map: &HashMap<String, FactTypeDef>,
    catalog: &SchemaCatalog,
) {
    // Shim: old code paths referred to `ir.nouns` / `ir.fact_types`.
    // Rebind so the body below compiles unchanged.
    struct IrShim<'a> {
        nouns: &'a HashMap<String, NounDef>,
        fact_types: &'a HashMap<String, FactTypeDef>,
    }
    let ir = IrShim { nouns: nouns_map, fact_types: fact_types_map };
    // Longest-first noun list for Theorem 1 matching
    let mut noun_names: Vec<String> = ir.nouns.keys().cloned().collect();
    noun_names.sort_by(|a, b| b.len().cmp(&a.len()));

    // Pre-process: expand possessive syntax (`X's Y`) into explicit join form
    // (`X has Y and that Y`) so the anaphora detector below can classify the
    // rule as a Join derivation.  Only the antecedent portion is rewritten;
    // the consequent is left unchanged.
    if rule.text.contains("'s ") {
        // Split off everything up to and including the iff/if/`:=` keyword,
        // expand only the antecedent portion, then reassemble.
        let sep_offset = rule.text.find(" := ")
            .map(|i| (i, i + 4))
            .or_else(|| rule.text.find(" iff ").map(|i| (i, i + 5)))
            .or_else(|| rule.text.find(" if ").map(|i| (i, i + 4)));
        if let Some((sep_start, sep_end)) = sep_offset {
            let consequent_part = &rule.text[..sep_start];
            let sep_word = &rule.text[sep_start..sep_end];
            let antecedent_part = &rule.text[sep_end..];
            if let Some(expanded) = try_expand_possessive(antecedent_part, &noun_names) {
                rule.text = format!("{}{}{}", consequent_part, sep_word, expanded);
            }
        }
    }

    // Split on " := ", " iff ", or " if " to get (consequent, antecedent_text)
    let (consequent_text, antecedent_raw) = rule.text
        .find(" := ")
        .map(|i| (&rule.text[..i], &rule.text[i + 4..]))
        .or_else(|| rule.text.find(" iff ")
            .map(|i| (&rule.text[..i], &rule.text[i + 5..])))
        .or_else(|| rule.text.find(" if ")
            .map(|i| (&rule.text[..i], &rule.text[i + 4..])))
        .unwrap_or((&rule.text, ""));

    // #276 Category G — expand `<head> that <verb>` relative clauses
    // into explicit `<head> and <last_noun> <verb>` conjunctions so
    // the downstream split on ` and ` produces resolvable clauses.
    // Back-reference anaphora (`that <Noun>`) is preserved, and the
    // expansion self-guards via `head_resolves` to avoid turning a
    // single unresolved clause into multiple unresolved fragments
    // when the head isn't a declared FT.
    let antecedent_expanded = expand_that_relatives(antecedent_raw, &noun_names, catalog);
    let antecedent_text: &str = antecedent_expanded.as_str();

    // Split antecedent on " and " to get individual conditions
    let antecedent_parts: Vec<&str> = antecedent_text
        .split(" and ")
        .map(|s| s.trim().trim_end_matches('.'))
        .filter(|s| !s.is_empty())
        .collect();

    // Strip quantifier, anaphoric, and determiner words from a text
    // fragment. #273: legal / prose rule bodies spell out articles
    // ("the Tool", "a Party", "an Exemption") that aren't part of
    // the FT identity. Removing them lets the catalog lookup match
    // against the clean `<Noun> <verb> <Noun>` form the FT was
    // declared with. Replacements are space-padded to preserve word
    // boundaries inside the clause (so `the ` inside `theoretical`
    // is untouched).
    let strip_anaphora = |text: &str| -> String {
        let replaced = text
            .replace("that ", "")
            .replace("some ", "")
            .replace("each ", "")
            .replace("any ", "")
            .replace(" the ", " ")
            .replace(" a ", " ")
            .replace(" an ", " ");
        // Leading determiners at the very start of the clause.
        replaced
            .trim_start_matches("the ")
            .trim_start_matches("a ")
            .trim_start_matches("an ")
            .to_string()
    };

    // Resolve a text fragment to a Fact Type ID via rho-lookup through the catalog.
    // Strips subscripts (Person1 â†’ Person) before catalog lookup â€” find_nouns
    // captures the subscripted token, but the catalog keys are base nouns.
    let resolve_fact_type = |fragment: &str| -> Option<String> {
        let cleaned = strip_anaphora(fragment);
        let found_nouns: Vec<(usize, usize, String)> = find_nouns(&cleaned, &noun_names);
        if found_nouns.is_empty() { return None; }
        let base_refs: Vec<String> = found_nouns.iter()
            .map(|(_, _, n)| parse_role_token(n).0.to_string())
            .collect();
        let role_refs: Vec<&str> = base_refs.iter().map(|s| s.as_str()).collect();

        // Verb extraction: text between first and second noun for
        // binary+ clauses; text after the single noun for unary
        // clauses (#274 Category A). Without the unary branch
        // `Customer is in EEA` looks up with empty verb and misses
        // the catalog entry keyed on verb "is in EEA".
        let verb = match found_nouns.len() {
            1 => cleaned[found_nouns[0].1..].trim(),
            _ => cleaned[found_nouns[0].1..found_nouns[1].0].trim(),
        };

        // rho-lookup: try with verb first, then noun set only
        let verb_opt = (!verb.is_empty()).then_some(verb);
        catalog.resolve(&role_refs, verb_opt)
            .or_else(|| catalog.resolve(&role_refs, None))
    };

    // Detect "that X" anaphoric references -- nouns preceded by "that " in
    // antecedent parts become join keys.
    let join_keys: Vec<String> = antecedent_parts.iter()
        .flat_map(|part| {
            noun_names.iter().filter_map(|noun| {
                let pattern = format!("that {}", noun);
                part.contains(&pattern).then(|| noun.clone())
            }).collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    // Resolve consequent
    rule.consequent_fact_type_id = resolve_fact_type(consequent_text).unwrap_or_default();

    // Resolve antecedents, carrying inline-comparator filters AND
    // arithmetic-definitional clauses alongside. A definitional clause
    // like `Volume is Size * Size * Size` does not resolve to a fact
    // type â€” it populates consequent_computed_bindings instead. Filter
    // clauses like `has Population >= 1000000` resolve to the base FT
    // with an AntecedentFilter pinned to that antecedent's position.
    let mut resolved_ids: Vec<String> = Vec::new();
    let mut filters: Vec<crate::types::AntecedentFilter> = Vec::new();
    let mut computed: Vec<crate::types::ConsequentComputedBinding> = Vec::new();
    let mut aggregates: Vec<crate::types::ConsequentAggregate> = Vec::new();
    for part in antecedent_parts.iter() {
        // Aggregate clauses (Halpin `<role> is the <op> of <target> where â€¦`).
        // They resolve the where-clause to a source FT and record the
        // group-key role â€” the non-target role on that FT. Match ahead of
        // the generic definitional path so `â€¦ is the count of â€¦` isn't
        // mistaken for arithmetic.
        if let Some((role, op, target, where_clause)) =
            try_parse_aggregate_clause(part, &noun_names)
        {
            // Resolve where-clause to an FT id via the catalog.
            let (stripped, _) = split_antecedent_comparator(&where_clause);
            if let Some(ft_id) = resolve_fact_type(&stripped) {
                // Group-key role = any role on source FT other than target.
                let group_key_role = ir.fact_types.get(&ft_id)
                    .and_then(|ft| ft.roles.iter().find(|r| r.noun_name != target))
                    .map(|r| r.noun_name.clone())
                    .unwrap_or_default();
                aggregates.push(crate::types::ConsequentAggregate {
                    role,
                    op,
                    target_role: target,
                    source_fact_type_id: ft_id,
                    group_key_role,
                });
            }
            continue;
        }
        // Definitional clauses claim the part outright â€” they bind a
        // consequent role's value and don't belong in antecedent FTs.
        if let Some((role, expr)) = try_parse_computed_binding(part, &noun_names) {
            computed.push(crate::types::ConsequentComputedBinding { role, expr });
            continue;
        }
        // â”€â”€ Classify the clause through existing pipelines â”€â”€â”€â”€â”€â”€â”€
        // Each pipeline already knows its own patterns. We call them
        // in order; the first match wins. No keyword arrays here.

        // (1) Comparator-stripped FT lookup (direct + hyphen fallback + negation fallback)
        let (stripped, comparator) = split_antecedent_comparator(part);
        let dehyphenated = stripped.replace("- ", " ").replace(" -", " ");
        // Strip a trailing `' <value>'` literal (single-quoted) so
        // `Task has Status 'Done'` resolves to the FT `Task has Status`
        // just like its unquoted form. The literal is semantically a
        // filter on the last role, not part of the FT reading.
        let literal_re = regex::Regex::new(r" '[^']*'\s*$").expect("static");
        let destripped_literal = literal_re.replace(&stripped, "").to_string();
        let ft_resolved = resolve_fact_type(&stripped)
            .or_else(|| (dehyphenated != stripped).then(|| resolve_fact_type(&dehyphenated)).flatten())
            .or_else(|| (destripped_literal != stripped)
                .then(|| resolve_fact_type(&destripped_literal)).flatten())
            .or_else(|| {
                let pos = strip_anaphora(part)
                    .replace(" is not ", " is ")
                    .replace(" has no ", " has ")
                    .replace(" does not ", " ");
                let pos = pos.trim_start_matches("no ").trim_start_matches("not ");
                // Strip " where ..." suffix â€” negated clauses with
                // where-filters ("no X is defined in Y where Z")
                // need the base FT without the filter tail.
                let pos = pos.split(" where ").next().unwrap_or(pos);
                resolve_fact_type(pos)
            });

        if let Some(ft_id) = ft_resolved {
            if let Some((op, value)) = comparator.clone() {
                let role = ir.fact_types.get(&ft_id)
                    .and_then(|ft| ft.roles.last())
                    .map(|r| r.noun_name.clone())
                    .unwrap_or_default();
                filters.push(crate::types::AntecedentFilter {
                    antecedent_index: resolved_ids.len(),
                    role, op, value,
                });
            }
            resolved_ids.push(ft_id);
            continue;
        }

        // (2) Comparator already split off a comparison operator â€”
        //     split_antecedent_comparator recognized it, even though
        //     the base FT didn't resolve. The clause IS a comparison.
        if comparator.is_some() { continue; }

        // (3) Aggregate: try_parse_aggregate_clause already knows
        //     count/sum/avg/min/max + where-clause patterns.
        if try_parse_aggregate_clause(part, &noun_names).is_some() { continue; }

        // (4) Computed binding: try_parse_computed_binding already
        //     knows arithmetic and role-assignment patterns.
        if try_parse_computed_binding(part, &noun_names).is_some() { continue; }

        // (5) that-anaphora: back-reference to a noun bound in a
        //     prior clause. Two shapes:
        //     a) "that X has Y" â€” join continuation
        //     b) "X is that Y" â€” anaphoric value assignment
        //        (e.g., "display- Text is that Reference")
        if part.trim().starts_with("that ") && noun_names.iter()
            .any(|n| part.to_lowercase().contains(&n.to_lowercase()))
        { continue; }
        if part.contains(" is that ") || part.contains(" is some ") { continue; }

        // (6) Temporal predicates â€” genuinely new, no existing fn.
        if is_temporal_predicate(part) { continue; }

        // (7) Subtype instance check: `X is a Y` / `X is an Y` where
        //     both X and Y are declared nouns. Subtype membership is
        //     inherent to the schema (Noun-is-subtype-of-Noun facts),
        //     not a separate FT. Recognised so readings like
        //       TCPA Violation is for Robocall ... if Robocall is
        //         an Autodialed Call and ...
        //     don't spuriously flag the subtype check as unresolved.
        if is_subtype_instance_check(part, &noun_names) { continue; }

        // (8) Word-based value comparison: `X exceeds Y`,
        //     `X is greater than Y`, etc., where both operands resolve
        //     against the noun catalog. Complements the ASCII-operator
        //     path in branch (1)/(2) for readings that spell their
        //     comparators out.
        if is_word_comparator_clause(part, &noun_names) { continue; }

        // (8b) #277 Category F — range-filter clauses
        //      `<FT reference> within|before|after <tail>` where the
        //      head alone resolves through the catalog. The tail is
        //      typically anaphora (`that Interval`, `that Fresh Until`)
        //      or a value literal.
        if is_range_filter_clause(part, &noun_names, catalog) { continue; }

        // (8c) #277 Category F — bare-value tail comparisons
        //      `<Noun> of N or more` / `or less` / `or greater`.
        //      Numeric literal only; quoted literals stay with the
        //      ref-scheme-value classifier at (9b).
        if is_bare_value_comparison(part, &noun_names) { continue; }

        // (9) Literal-value filter: `<Noun> has <Noun> '<literal>'`.
        //     Covers state-machine status filters (`Task has Status 'Done'`)
        //     and enum-value filters (`Customer has Tier 'Gold'`) whose
        //     FT isn't always declared textually when the role is
        //     SM-managed or enum-valued. `resolve_fact_type` would miss
        //     it; classify it here as a valid antecedent predicate.
        if is_noun_has_noun_literal(part, &noun_names) { continue; }

        // (9b) Ref-scheme-value filter: `<Noun> is '<literal>'` or
        //      `<Noun> is not '<literal>'`. The entity's ref scheme
        //      value IS its identity, so this clause selects the
        //      entity whose identity equals the literal. Optional
        //      leading role qualifiers (`other Source`, `that
        //      Customer`) are stripped before the match. #275
        //      Category C.
        if is_entity_ref_scheme_literal(part, &noun_names) { continue; }

        // (10) Universal quantifier: `for each <Noun> <predicate>`.
        //      Recognised when the clause starts with `for each` and
        //      contains a declared noun. The compiled form is a
        //      population-level restriction; classification here just
        //      suppresses the noise so legitimate universals don't
        //      flag as unresolved.
        if is_universal_quantifier_clause(part, &noun_names) { continue; }

        // (11) `<Noun> is extracted from <Noun>` / `<Noun> is derived from <Noun>`.
        //      Used for ML-style computed bindings where the RHS is a
        //      free-text source field (e.g. `Category is extracted
        //      from Body`). The extraction function itself is a
        //      runtime primitive; the clause shape is valid here.
        if is_extraction_clause(part, &noun_names) { continue; }

        // (12) Existential-qualified FT reference: `<Noun> <verb> some <Noun>`
        //      or `<Noun> <verb> that <Noun>`. The `some` / `that`
        //      quantifier doesn't change the FT identity; try the
        //      fact-type lookup again with those tokens stripped. Covers
        //      `Feature Request concerns some API Product` style where
        //      the declared FT is `Feature Request concerns API Product`.
        let stripped_quantifiers = strip_existential_quantifiers(part);
        if stripped_quantifiers.as_str() != *part
            && resolve_fact_type(&stripped_quantifiers).is_some()
        { continue; }

        // Nothing classified this clause.
        rule.unresolved_clauses.push(part.to_string());
    }
    rule.antecedent_fact_type_ids = resolved_ids;
    rule.antecedent_filters = filters;
    rule.consequent_computed_bindings = computed;
    rule.consequent_aggregates = aggregates;

    // Deduplicate join keys
    let mut seen = hashbrown::HashSet::new();
    rule.join_on = join_keys.into_iter()
        .filter(|k| seen.insert(k.clone()))
        .collect();

    // Classify: if join keys exist AND at least 2 distinct antecedent fact types share
    // a noun, this is a Join derivation. Rules with "that X" anaphora where X appears
    // in multiple antecedents need an equi-join on X.
    let is_join = !rule.join_on.is_empty()
        && rule.antecedent_fact_type_ids.len() >= 2
        && rule.join_on.iter().any(|key| {
            rule.antecedent_fact_type_ids.iter()
                .filter(|ft_id| ir.fact_types.get(*ft_id)
                    .map_or(false, |ft| ft.roles.iter().any(|r| r.noun_name == *key)))
                .count() >= 2
        });
    is_join.then(|| {
        rule.kind = DerivationKind::Join;
        // Build match_on: pairs of (noun_a, noun_b) for equality matching
        rule.match_on = rule.join_on.iter()
            .map(|key| (key.clone(), key.clone()))
            .collect();
        // Consequent bindings: nouns from the consequent fact type
        rule.consequent_bindings = ir.fact_types.get(&rule.consequent_fact_type_id)
            .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
            .unwrap_or_default();
    });

    // Set rule ID: prefer resolved consequent FT ID, fall back to a
    // sanitized form of the consequent text, then to a hash of rule text.
    // A non-empty ID prevents multiple := rules from sharing the cell
    // `derivation:` in DEFS and clobbering each other.
    rule.id = if !rule.consequent_fact_type_id.is_empty() {
        rule.consequent_fact_type_id.clone()
    } else {
        let cleaned = strip_anaphora(consequent_text);
        let sanitized: String = cleaned.trim().trim_end_matches('.').trim().chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect::<String>()
            .split('_').filter(|s| !s.is_empty())
            .collect::<Vec<_>>().join("_");
        if !sanitized.is_empty() {
            sanitized
        } else {
            // FNV-1a 64-bit over the rule text â€” no hasher dep, no
            // allocation, stable output. Only used as a fallback rule
            // name when the sanitized text collapses to empty, so
            // collisions matter only inside a single domain's rules.
            let mut h: u64 = 0xcbf29ce484222325;
            for b in rule.text.as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            format!("_rule_{h:x}")
        }
    };
}

/// Append a fact to a cell in the ParseCtx's Object-state accumulator.
fn push_cell(ir: &mut ParseCtx, cell: &str, fact: crate::ast::Object) {
    ir.cells.entry(cell.to_string()).or_default().push(fact);
}

/// #283 — Cell-backed lookup for a noun's reference-scheme components.
/// Reads the `RefScheme` cell (populated by `ParseAction::AddNoun`)
/// and decodes the `partN` indexed fields back into a Vec.
fn ref_scheme_for_noun(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
    noun: &str,
) -> Option<Vec<String>> {
    cells.get("RefScheme")?
        .iter()
        .find(|f| crate::ast::binding(f, "noun") == Some(noun))
        .map(|f| {
            let mut parts: Vec<String> = Vec::new();
            let mut i: usize = 0;
            loop {
                let key = alloc::format!("part{i}");
                match crate::ast::binding(f, &key) {
                    Some(v) => { parts.push(v.to_string()); i += 1; }
                    None => break,
                }
            }
            parts
        })
}

/// #283 — Write an instance fact directly to the `InstanceFact` cell
/// and the per-field cell. Replaces pushing into a typed
/// `general_instance_facts: Vec<GeneralInstanceFact>` plus a later
/// translation pass; writers emit cells once, no second pass.
fn emit_instance_fact(
    ir: &mut ParseCtx,
    subject_noun: &str,
    subject_value: &str,
    field_name: &str,
    object_noun: &str,
    object_value: &str,
) {
    push_cell(ir, "InstanceFact", crate::ast::fact_from_pairs(&[
        ("subjectNoun", subject_noun),
        ("subjectValue", subject_value),
        ("fieldName", field_name),
        ("objectNoun", object_noun),
        ("objectValue", object_value),
    ]));
    // Per-field cell: `{fieldName}` cell with (subject, object) pair.
    // The object key is the object noun if present, otherwise the
    // field name itself (for attribute-style instance facts).
    let object_key = if object_noun.is_empty() { field_name } else { object_noun };
    push_cell(ir, field_name, crate::ast::fact_from_pairs(&[
        (subject_noun, subject_value),
        (object_key, object_value),
    ]));
}

/// #283 — Collect every instance fact's subject_value for a given
/// subject noun from the `InstanceFact` cell. Replaces the typed
/// `.general_instance_facts.iter().filter(|f| f.subject_noun == n)
/// .map(|f| f.subject_value)` pattern used by compound ref-scheme
/// decomposition.
fn instance_fact_subject_values_for_noun(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
    subject_noun: &str,
) -> Vec<String> {
    cells.get("InstanceFact")
        .map(|facts| facts.iter()
            .filter(|f| crate::ast::binding(f, "subjectNoun") == Some(subject_noun))
            .filter_map(|f| crate::ast::binding(f, "subjectValue").map(String::from))
            .collect())
        .unwrap_or_default()
}

/// #283 — Enum values for a value type, decoded from the `EnumValues`
/// cell's `value0`, `value1`, ... indexed fields. Replaces
/// `ir.enum_values.get(name)`.
fn enum_values_for_noun(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
    noun: &str,
) -> Vec<String> {
    cells.get("EnumValues")
        .and_then(|facts| facts.iter()
            .find(|f| crate::ast::binding(f, "noun") == Some(noun)))
        .map(|fact| {
            let mut vals: Vec<String> = Vec::new();
            let mut i: usize = 0;
            loop {
                let key = alloc::format!("value{i}");
                match crate::ast::binding(fact, &key) {
                    Some(v) => { vals.push(v.to_string()); i += 1; }
                    None => break,
                }
            }
            vals
        })
        .unwrap_or_default()
}

/// #283 — Every noun name that carries enum values. Replaces
/// `ir.enum_values.keys()`.
fn all_enum_value_nouns(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
) -> Vec<String> {
    cells.get("EnumValues")
        .map(|facts| facts.iter()
            .filter_map(|f| crate::ast::binding(f, "noun").map(String::from))
            .collect())
        .unwrap_or_default()
}

/// #283 — Upsert enum values for a noun on the `EnumValues` cell.
/// Removes any prior fact for this noun before pushing the new one.
fn upsert_enum_values(
    ir: &mut ParseCtx,
    noun: &str,
    values: &[String],
) {
    if let Some(facts) = ir.cells.get_mut("EnumValues") {
        facts.retain(|f| crate::ast::binding(f, "noun") != Some(noun));
    }
    let mut pairs: Vec<(String, String)> = alloc::vec![
        ("noun".to_string(), noun.to_string()),
    ];
    for (i, v) in values.iter().enumerate() {
        pairs.push((alloc::format!("value{i}"), v.clone()));
    }
    let refs: Vec<(&str, &str)> = pairs.iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    push_cell(ir, "EnumValues", crate::ast::fact_from_pairs(&refs));
}

/// #283 — Supertype lookup for a subtype noun via the `Subtype` cell.
/// Replaces `ir.subtypes.get(name)`.
fn supertype_of(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
    subtype: &str,
) -> Option<String> {
    cells.get("Subtype")?
        .iter()
        .find(|f| crate::ast::binding(f, "subtype") == Some(subtype))
        .and_then(|f| crate::ast::binding(f, "supertype").map(String::from))
}

/// #283 — Every subtype noun name present in the `Subtype` cell.
/// Replaces `ir.subtypes.keys()`.
fn all_subtype_names(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
) -> Vec<String> {
    cells.get("Subtype")
        .map(|facts| facts.iter()
            .filter_map(|f| crate::ast::binding(f, "subtype").map(String::from))
            .collect())
        .unwrap_or_default()
}

/// #283 — Upsert a (subtype, supertype) pair on the `Subtype` cell.
/// Removes any prior fact for `subtype` before pushing the new one.
fn upsert_subtype(
    ir: &mut ParseCtx,
    subtype: &str,
    supertype: &str,
) {
    if let Some(facts) = ir.cells.get_mut("Subtype") {
        facts.retain(|f| crate::ast::binding(f, "subtype") != Some(subtype));
    }
    push_cell(ir, "Subtype", crate::ast::fact_from_pairs(&[
        ("subtype", subtype), ("supertype", supertype),
    ]));
}

/// #283 — Iterate every (noun, ref-scheme) pair from the `RefScheme`
/// cell. Replaces `ir.ref_schemes.iter()` and `d.ref_schemes.iter()`.
fn all_ref_schemes(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
) -> Vec<(String, Vec<String>)> {
    cells.get("RefScheme")
        .map(|facts| facts.iter()
            .filter_map(|f| {
                let noun = crate::ast::binding(f, "noun")?.to_string();
                let mut parts: Vec<String> = Vec::new();
                let mut i: usize = 0;
                loop {
                    let key = alloc::format!("part{i}");
                    match crate::ast::binding(f, &key) {
                        Some(v) => { parts.push(v.to_string()); i += 1; }
                        None => break,
                    }
                }
                Some((noun, parts))
            })
            .collect())
        .unwrap_or_default()
}

/// #283 — Every noun name present in the `Noun` cell.
/// Replaces `ir.nouns.keys()`.
fn noun_names(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
) -> Vec<String> {
    cells.get("Noun")
        .map(|facts| facts.iter()
            .filter_map(|f| crate::ast::binding(f, "name").map(String::from))
            .collect())
        .unwrap_or_default()
}

/// #283 — Whether a noun is already declared in the `Noun` cell.
/// Replaces `ir.nouns.contains_key(name)`.
fn noun_exists(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
    name: &str,
) -> bool {
    cells.get("Noun")
        .map(|facts| facts.iter()
            .any(|f| crate::ast::binding(f, "name") == Some(name)))
        .unwrap_or(false)
}

/// #283 — Object-type lookup for a noun via the `Noun` cell.
/// Replaces `ir.nouns.get(name).map(|d| &d.object_type)`.
fn noun_object_type(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
    name: &str,
) -> Option<String> {
    cells.get("Noun")?
        .iter()
        .find(|f| crate::ast::binding(f, "name") == Some(name))
        .and_then(|f| crate::ast::binding(f, "objectType").map(String::from))
}

/// #283 — Rebuild `HashMap<String, NounDef>` from the `Noun` cell.
/// Used at API boundaries that still take a typed map (e.g.
/// `resolve_derivation_rule`). Each call is O(N) over noun count,
/// which is cheap compared with the derivation resolution it seeds.
fn nouns_from_cells(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
) -> HashMap<String, NounDef> {
    cells.get("Noun")
        .map(|facts| facts.iter().filter_map(|f| {
            let name = crate::ast::binding(f, "name")?.to_string();
            let obj_type = crate::ast::binding(f, "objectType").unwrap_or("entity").to_string();
            let wa = match crate::ast::binding(f, "worldAssumption") {
                Some("open") => WorldAssumption::Open,
                _ => WorldAssumption::Closed,
            };
            Some((name, NounDef { object_type: obj_type, world_assumption: wa }))
        }).collect())
        .unwrap_or_default()
}

/// #283 — Push a basic Noun fact (name + objectType + worldAssumption).
/// Enrichment with superType / referenceScheme / enumValues happens at
/// finalize via `enrich_noun_cells`, reading the sibling cells that
/// those writers populate.
fn push_noun_fact(
    ir: &mut ParseCtx,
    name: &str,
    object_type: &str,
    wa: WorldAssumption,
) {
    let wa_str = match wa {
        WorldAssumption::Closed => "closed",
        WorldAssumption::Open => "open",
    };
    push_cell(ir, "Noun", crate::ast::fact_from_pairs(&[
        ("name", name), ("objectType", object_type), ("worldAssumption", wa_str),
    ]));
}

/// #283 — Upsert a noun on the `Noun` cell. Preserves the same semantics
/// as the previous `ir.nouns.entry(name).or_insert_with(def)` branch in
/// `apply_action`:
///   - new: insert `def`.
///   - existing, `def.object_type == "abstract"`: force existing to abstract.
///   - existing, `def.object_type != existing && def.object_type != "abstract"`:
///     overwrite the fact with `def` (conflict detected later in platform_compile).
///   - otherwise: preserve existing.
fn upsert_noun(ir: &mut ParseCtx, name: &str, def: &NounDef) {
    let existing_type = noun_object_type(&ir.cells, name);
    match existing_type.as_deref() {
        None => push_noun_fact(ir, name, &def.object_type, def.world_assumption.clone()),
        Some(existing) if existing != def.object_type.as_str() && def.object_type != "abstract" => {
            if let Some(facts) = ir.cells.get_mut("Noun") {
                facts.retain(|f| crate::ast::binding(f, "name") != Some(name));
            }
            push_noun_fact(ir, name, &def.object_type, def.world_assumption.clone());
        }
        Some(_) if def.object_type == "abstract" => mark_noun_abstract(ir, name),
        _ => {}
    }
}

/// #283 — Flip an existing Noun fact's `objectType` binding to `"abstract"`.
/// No-op if no such fact. Replaces `ir.nouns.get_mut(name).for_each(|n| n.object_type = "abstract".into())`.
fn mark_noun_abstract(ir: &mut ParseCtx, name: &str) {
    let Some(facts) = ir.cells.get_mut("Noun") else { return };
    if let Some(f) = facts.iter_mut()
        .find(|f| crate::ast::binding(f, "name") == Some(name))
    {
        let wa = crate::ast::binding(f, "worldAssumption").unwrap_or("closed").to_string();
        *f = crate::ast::fact_from_pairs(&[
            ("name", name), ("objectType", "abstract"), ("worldAssumption", wa.as_str()),
        ]);
    }
}

/// #283 — In loose mode, auto-create an `entity` noun if undeclared.
/// Replaces `ir.nouns.entry(name).or_insert(NounDef { object_type: "entity", .. })`.
fn ensure_noun_entity(ir: &mut ParseCtx, name: &str) {
    if !noun_exists(&ir.cells, name) {
        push_noun_fact(ir, name, "entity", WorldAssumption::default());
    }
}

/// #283 — Finalize pass: enrich each Noun fact with `superType`,
/// `referenceScheme`, and `enumValues` bindings by joining sibling cells
/// (`Subtype`, `RefScheme`, `EnumValues`). Writers emit basic Noun facts
/// during parse; this pass joins them once downstream consumers are ready.
#[cfg(feature = "std-deps")]
fn enrich_noun_cells(ir: &mut ParseCtx) {
    let Some(facts) = ir.cells.get("Noun").cloned() else { return };
    let enriched: Vec<crate::ast::Object> = facts.iter().map(|f| {
        let name = crate::ast::binding(f, "name").unwrap_or("").to_string();
        let object_type = crate::ast::binding(f, "objectType").unwrap_or("entity").to_string();
        let wa = crate::ast::binding(f, "worldAssumption").unwrap_or("closed").to_string();
        let mut pairs: Vec<(String, String)> = vec![
            ("name".into(), name.clone()),
            ("objectType".into(), object_type.clone()),
            ("worldAssumption".into(), wa),
        ];
        if let Some(st) = supertype_of(&ir.cells, &name) {
            pairs.push(("superType".into(), st));
        }
        let ref_scheme = ref_scheme_for_noun(&ir.cells, &name)
            .or_else(|| (object_type == "entity").then(|| vec!["id".into()]));
        if let Some(rs) = ref_scheme {
            pairs.push(("referenceScheme".into(), rs.join(",")));
        }
        let evs = enum_values_for_noun(&ir.cells, &name);
        if !evs.is_empty() {
            pairs.push(("enumValues".into(), evs.join(",")));
        }
        let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        crate::ast::fact_from_pairs(&refs)
    }).collect();
    ir.cells.insert("Noun".to_string(), enriched);
}

/// #283 — Every fact-type id in the `FactType` cell. Replaces
/// `ir.fact_types.keys()`.
fn fact_type_ids(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
) -> Vec<String> {
    cells.get("FactType")
        .map(|facts| facts.iter()
            .filter_map(|f| crate::ast::binding(f, "id").map(String::from))
            .collect())
        .unwrap_or_default()
}

/// #283 — Whether a fact-type id is declared in the `FactType` cell.
/// Replaces `ir.fact_types.contains_key(id)`.
fn fact_type_exists(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
    id: &str,
) -> bool {
    cells.get("FactType")
        .map(|facts| facts.iter()
            .any(|f| crate::ast::binding(f, "id") == Some(id)))
        .unwrap_or(false)
}

/// #283 — Reading text for a fact-type via the `FactType` cell.
fn fact_type_reading(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
    id: &str,
) -> Option<String> {
    cells.get("FactType")?
        .iter()
        .find(|f| crate::ast::binding(f, "id") == Some(id))
        .and_then(|f| crate::ast::binding(f, "reading").map(String::from))
}

/// #283 — Roles for a fact-type via the `Role` cell. Replaces
/// `ir.fact_types[id].roles`. Roles are returned in `position` order.
fn fact_type_roles(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
    id: &str,
) -> Vec<crate::types::RoleDef> {
    let Some(facts) = cells.get("Role") else { return Vec::new() };
    let mut roles: Vec<crate::types::RoleDef> = facts.iter()
        .filter(|f| crate::ast::binding(f, "factType") == Some(id))
        .filter_map(|f| {
            let noun_name = crate::ast::binding(f, "nounName")?.to_string();
            let position = crate::ast::binding(f, "position")?.parse::<usize>().ok()?;
            Some(crate::types::RoleDef { noun_name, role_index: position })
        })
        .collect();
    roles.sort_by_key(|r| r.role_index);
    roles
}

/// #283 — Rebuild `HashMap<String, FactTypeDef>` from the `FactType` +
/// `Role` cells. Used at API boundaries that still take a typed map
/// (e.g. `resolve_derivation_rule`, `resolve_instance_field`). Each
/// call is O(N) over fact-type + role count, which is cheap compared
/// with the resolution it seeds. `readings` and `schema_id` are
/// minimally reconstructed — downstream consumers that care only
/// about the `reading` string and `roles` vec keep working.
fn fact_types_from_cells(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
) -> HashMap<String, FactTypeDef> {
    let Some(ft_facts) = cells.get("FactType") else { return HashMap::new() };
    ft_facts.iter().filter_map(|f| {
        let id = crate::ast::binding(f, "id")?.to_string();
        let reading = crate::ast::binding(f, "reading").unwrap_or("").to_string();
        let roles = fact_type_roles(cells, &id);
        Some((id.clone(), FactTypeDef {
            schema_id: id,
            reading,
            readings: Vec::new(),
            roles,
        }))
    }).collect()
}

/// #283 — Upsert a fact-type on the `FactType` + `Role` cells.
/// Preserves the `or_insert` semantics of the previous
/// `ir.fact_types.entry(id).or_insert(def)`: first declaration wins.
fn upsert_fact_type(ir: &mut ParseCtx, id: &str, def: &FactTypeDef) {
    if fact_type_exists(&ir.cells, id) {
        return;
    }
    push_cell(ir, "FactType", crate::ast::fact_from_pairs(&[
        ("id", id),
        ("reading", def.reading.as_str()),
        ("arity", &def.roles.len().to_string()),
    ]));
    for role in &def.roles {
        push_cell(ir, "Role", crate::ast::fact_from_pairs(&[
            ("factType", id),
            ("nounName", role.noun_name.as_str()),
            ("position", &role.role_index.to_string()),
        ]));
    }
}

/// #283 — Rebuild `Vec<ConstraintDef>` from the `Constraint` cell.
/// The cell is lossless — `constraint_to_fact` embeds the full JSON
/// encoding of each ConstraintDef under the `json` binding. Callers
/// that want the typed view (tests, resolve paths) go through here.
#[cfg(feature = "std-deps")]
fn constraints_from_cells(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
) -> Vec<ConstraintDef> {
    let Some(facts) = cells.get("Constraint") else { return Vec::new() };
    facts.iter().filter_map(|f| {
        let json = crate::ast::binding(f, "json")?;
        serde_json::from_str::<ConstraintDef>(json).ok()
    }).collect()
}

/// #283 — Apply subset-span autofill to SS constraints whose role set
/// matches an autofill-role-set. Mutates the `Constraint` cell in place
/// by rewriting affected facts with the updated JSON.
#[cfg(feature = "std-deps")]
fn mark_ss_constraints_autofill(
    ir: &mut ParseCtx,
    role_sets: &[hashbrown::HashSet<String>],
) {
    let Some(facts) = ir.cells.get_mut("Constraint") else { return };
    for fact in facts.iter_mut() {
        let Some(kind) = crate::ast::binding(fact, "kind") else { continue };
        if kind != "SS" { continue }
        let Some(json) = crate::ast::binding(fact, "json") else { continue };
        let Ok(mut cdef) = serde_json::from_str::<ConstraintDef>(json) else { continue };
        let matches = role_sets.iter().any(|role_set| {
            role_set.iter().all(|n| cdef.text.contains(n.as_str()))
        });
        if !matches { continue }
        if let Some(span) = cdef.spans.first_mut() {
            span.subset_autofill = Some(true);
        }
        *fact = constraint_to_fact(&cdef);
    }
}

/// Emit a Constraint cell fact with the full constraint JSON (lossless)
/// plus flat fields for check.rs and no_std fallbacks.
#[cfg(all(test, feature = "std-deps"))]
pub(crate) fn constraint_to_fact_test(c: &ConstraintDef) -> crate::ast::Object {
    constraint_to_fact(c)
}
#[cfg(feature = "std-deps")]
fn constraint_to_fact(c: &ConstraintDef) -> crate::ast::Object {
    use crate::ast::fact_from_pairs;
    let json = serde_json::to_string(c).unwrap_or_default();
    let mut pairs: Vec<(String, String)> = vec![
        ("id".into(), c.id.clone()), ("kind".into(), c.kind.clone()),
        ("modality".into(), c.modality.clone()), ("text".into(), c.text.clone()),
        ("json".into(), json),
    ];
    c.deontic_operator.as_ref().map(|op| pairs.push(("deonticOperator".into(), op.clone())));
    c.entity.as_ref().map(|e| pairs.push(("entity".into(), e.clone())));
    pairs.extend(c.spans.iter().enumerate().flat_map(|(i, span)| [
        (format!("span{}_factTypeId", i), span.fact_type_id.clone()),
        (format!("span{}_roleIndex", i), span.role_index.to_string()),
    ]));
    let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    fact_from_pairs(&refs)
}

/// Apply a parse action to the IR accumulator.
///
/// For write-only kinds (Constraint, DerivationRule, NamedSpan,
/// AutofillSpan), this emits Object cells directly — parse produces Φ
/// per Thm 2. Kinds that need in-parse mutation/lookup (Noun, FactType)
/// still accumulate typed fields; ctx_to_state serializes them to
/// cells at finalize.
fn apply_action(ir: &mut ParseCtx, action: Option<ParseAction>, lines: &[String], idx: usize) {
    let Some(action) = action else { return };
    match action {
        ParseAction::AddNoun(name, def, meta) => {
            // #283 — Upsert the Noun cell directly. Conflict detection
            // (e.g. "Each Noun has exactly one Object Type") is the
            // validate pipeline's job at compile time.
            upsert_noun(ir, &name, &def);
            // Populate IR maps from metadata
            // #283 — Subtype cell write (upsert semantics).
            meta.super_type.into_iter().for_each(|st| { upsert_subtype(ir, &name, &st); });
            // #283 — RefScheme cell write. or_insert semantics preserved
            // by checking existing cells first; first noun declaration
            // wins (metamodel guard handles legitimate redeclaration).
            meta.ref_scheme.into_iter().for_each(|rs| {
                let already = ir.cells.get("RefScheme")
                    .map(|facts| facts.iter()
                        .any(|f| crate::ast::binding(f, "noun") == Some(name.as_str())))
                    .unwrap_or(false);
                if !already {
                    let mut pairs: Vec<(String, String)> = alloc::vec![
                        ("noun".to_string(), name.clone()),
                    ];
                    for (i, p) in rs.iter().enumerate() {
                        pairs.push((alloc::format!("part{i}"), p.clone()));
                    }
                    let refs: Vec<(&str, &str)> = pairs.iter()
                        .map(|(k, v)| (k.as_str(), v.as_str()))
                        .collect();
                    push_cell(ir, "RefScheme", crate::ast::fact_from_pairs(&refs));
                }
            });
        }
        ParseAction::MarkAbstract(name) => {
            mark_noun_abstract(ir, &name);
        }
        ParseAction::AddPartition(sup, subs) => {
            mark_noun_abstract(ir, &sup);
            subs.into_iter().for_each(|sub| {
                // In strict mode, don't auto-create undeclared nouns.
                // The post-parse validation will catch them.
                if !is_strict_mode() {
                    ensure_noun_entity(ir, &sub);
                }
                upsert_subtype(ir, &sub, &sup);
            });
        }
        ParseAction::AddFactType(id, def, mode) => {
            // `mode` is Some("fully-derived" | "derived-and-stored" | "semi-derived")
            // when the reading terminated with a `*` / `**` / `+` marker.
            // Emit as a GeneralInstanceFact against the metamodel's
            // `Fact Type has Derivation Mode` binary â€” facts all the way
            // down, no separate ParseCtx field for what is already expressible
            // as an instance fact.
            let reading_for_mode = def.reading.clone();
            // #283 — emit InstanceFact cell directly; no typed Vec.
            mode.into_iter().for_each(|m| {
                emit_instance_fact(ir,
                    "Fact Type", &reading_for_mode,
                    "Derivation Mode", "Derivation Mode", &m);
            });
            // #283 — upsert FactType + Role cells directly.
            upsert_fact_type(ir, &id, &def);
        }
        ParseAction::AddConstraint(c) => {
            // Emit Constraint cell fact directly. Pass 2b does not revisit
            // constraints, so this is the final form. #283 — no typed vec.
            #[cfg(feature = "std-deps")]
            push_cell(ir, "Constraint", constraint_to_fact(&c));
            let _ = c;
        }
        ParseAction::AddDerivation(r) => {
            // Pass 2b (re_resolve_rules) re-populates structured fields on
            // the typed Vec; the corresponding cell fact is (re-)emitted
            // at finalize. Here we only push the typed representation —
            // no cell emission yet, because the rule's JSON shape will
            // change after resolution.
            ir.derivation_rules.push(r);
        }
        ParseAction::AddInstanceFact(raw) => {
            let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
            parse_instance_fact(ir, &raw, &line_refs, idx);
        }
        ParseAction::AddNamedSpan(name, nouns) => {
            // #283 — NamedSpan cell. `role0`, `role1`, ... carry the
            // ordered noun list. Cells are the only store: no typed
            // `ir.named_spans` parallel state.
            let mut pairs: Vec<(String, String)> = alloc::vec![("name".to_string(), name)];
            for (i, n) in nouns.into_iter().enumerate() {
                pairs.push((alloc::format!("role{i}"), n));
            }
            let refs: Vec<(&str, &str)> = pairs.iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            push_cell(ir, "NamedSpan", crate::ast::fact_from_pairs(&refs));
        }
        ParseAction::AddAutofillSpan(name) => {
            // #283 — AutofillSpan cell.
            push_cell(ir, "AutofillSpan", crate::ast::fact_from_pairs(&[
                ("name", name.as_str()),
            ]));
        }
        ParseAction::Skip => {}
    }
}

// =========================================================================
// Pure extraction functions (no if/else -- use ? and strip_prefix/suffix)
// =========================================================================

fn parse_entity_decl(text: &str) -> Option<(String, Option<Vec<String>>)> {
    let paren = text.find('(');
    match paren {
        Some(p) => {
            let name = text[..p].trim().to_string();
            let inner = text[p + 1..].trim_end_matches(')');
            let refs: Vec<String> = inner.split(',')
                .map(|s| s.trim().trim_start_matches('.').to_string())
                .filter(|s| !s.is_empty())
                .collect();
            Some((name, Some(refs).filter(|r| !r.is_empty())))
        }
        None => Some((text.trim().to_string(), None))
    }
}

fn parse_enum(line: &str) -> Option<Vec<String>> {
    let after = line.split(" are ").nth(1)?;
    Some(after.trim_end_matches('.').split(", ").map(|s| s.trim().trim_matches('\'').into()).collect())
}

/// Canonical Fact Type ID from role nouns and verb.
/// The ID is the key in DEFS. Two readings with the same roles and verb
/// (just different voice) produce the same ID.
fn fact_type_id(role_nouns: &[&str], verb: &str) -> String {
    let verb_part = verb.to_lowercase().replace(' ', "_");
    let noun_parts: Vec<String> = role_nouns.iter().map(|n| n.replace(' ', "_")).collect();
    let mut parts: Vec<&str> = vec![&noun_parts[0], &verb_part];
    noun_parts[1..].iter().for_each(|n| parts.push(n));
    parts.join("_")
}

/// Scan the raw reading lines for derivation rules, extract each
/// rule's consequent text, and register the consequent as a synthetic
/// FT in `catalog`. Handles multi-line rules by joining indented
/// continuations exactly as `parse_into` does upstream.
///
/// Derivation markers: `* ` / `** ` / `+ ` prefix, or mid-sentence
/// ` iff ` / ` if ` / ` := `. The consequent is everything to the
/// left of the marker keyword; find_nouns over it yields the role
/// set (and verb = text between the first and last noun) that the
/// catalog indexes by.
///
/// `Customer is in EEA iff Customer has some Address that ...` →
///   catalog registers `derived:Customer_is_in_EEA` keyed on
///   {customer, eea} with verb "is in". Later rules using
///   `Customer is in EEA` as an antecedent then resolve cleanly.
fn register_derivation_consequents_in_catalog(
    lines: &[String],
    noun_names: &[String],
    catalog: &mut SchemaCatalog,
) {
    let joined = join_derivation_continuations(&lines.join("\n"));
    for line in &joined {
        let stripped = line.trim_start()
            .strip_prefix("** ").or_else(|| line.trim_start().strip_prefix("* "))
            .or_else(|| line.trim_start().strip_prefix("+ "))
            .unwrap_or(line.trim_start());
        // Find the left-most derivation marker — everything before it
        // is the consequent reading.
        let marker_idx = [" iff ", " := ", " if "].iter()
            .filter(|kw| stripped.contains(*kw))
            .filter_map(|kw| stripped.find(kw).map(|i| (i, kw.len())))
            .min_by_key(|(i, _)| *i);
        let (sep_idx, sep_len) = match marker_idx {
            Some(m) => m,
            None => continue,
        };
        // Exclude `If ... then ...` — those are conditional
        // ring/constraint forms, not derivation heads.
        if stripped.starts_with("If ") { continue; }
        let consequent = stripped[..sep_idx].trim();

        // Tokenize consequent via the same longest-first noun matcher
        // used downstream. A consequent with zero declared nouns can't
        // be registered as an FT. (Same anaphora-stripping that
        // resolve_fact_type's closure applies — "that X" / "some X"
        // are quantifiers, not part of the FT reading.)
        let cleaned = consequent
            .replace("that ", "")
            .replace("some ", "")
            .replace("each ", "")
            .replace("any ", "");
        let found: Vec<(usize, usize, String)> = find_nouns(&cleaned, noun_names);
        // Need at least one declared noun in the consequent to key
        // the catalog entry. Unary consequents (#274 Category A) —
        // `Customer is in EEA`, `Fetcher is proxy-based`, `Customer is
        // eligible for trial` — carry their verb in the tail after
        // the single noun. Over-capture is prevented in
        // `SchemaCatalog::resolve`, which skips the unique-entry
        // fallback for 1-noun keys and so requires an exact (or
        // reading-contained) verb match on unary lookups.
        if found.is_empty() { continue; }
        let base_refs: Vec<String> = found.iter()
            .map(|(_, _, n)| parse_role_token(n).0.to_string())
            .collect();
        let role_refs: Vec<&str> = base_refs.iter().map(|s| s.as_str()).collect();
        // Verb: text between the first and second noun for binary+
        // consequents; text after the single noun for unaries.
        let verb = match found.len() {
            1 => cleaned[found[0].1..].trim(),
            _ => cleaned[found[0].1..found[1].0].trim(),
        };
        // Synthesise an FT id. Using the fact_type_id helper keeps it
        // consistent with regular FT registration.
        let synthetic_id = alloc::format!("derived:{}", fact_type_id(&role_refs, verb));
        catalog.register(&synthetic_id, &role_refs, verb, consequent);

        // Follow-up: also record (sep_len) is used to avoid unused-
        // variable warnings on platforms that gate the block.
        let _ = sep_len;
    }
}

/// Schema catalog for rho-lookup: noun set -> Fact Type ID.
/// The noun set is the key. The catalog is the DEFS cell.
struct SchemaCatalog {
    /// Sorted noun set -> vec of (schema_id, verb, reading) for disambiguation
    by_noun_set: HashMap<Vec<String>, Vec<(String, String, String)>>,
}

impl SchemaCatalog {
    fn new() -> Self {
        SchemaCatalog { by_noun_set: HashMap::new() }
    }

    fn register(&mut self, schema_id: &str, role_nouns: &[&str], verb: &str, reading: &str) {
        let mut key: Vec<String> = role_nouns.iter().map(|n| {
            let (base, _) = parse_role_token(n);
            base.to_lowercase()
        }).collect();
        key.sort();
        self.by_noun_set
            .entry(key)
            .or_default()
            .push((schema_id.to_string(), verb.to_lowercase(), reading.to_lowercase()));
    }

    /// rho-lookup: noun set -> Fact Type ID.
    /// Resolution strategy (no COND dispatch, just cascading lookup):
    /// 1. Exact verb match
    /// 2. Verb contained in stored reading (handles inverse voice)
    /// 3. Unique entry for noun set (no verb needed) — binary+ only
    ///
    /// The unique-entry fallback is skipped for 1-noun keys (#274
    /// Category A). Unaries carry all their identity in the verb:
    /// without the fallback guard, a clause like `Order has Mystery`
    /// (noun set [order], `Mystery` undeclared) would resolve to any
    /// single unary synthetic keyed on [order] — `Order is pending`,
    /// `Order is cancelled` — regardless of verb. Step 1 and 2 remain
    /// active and catch the legitimate unary matches.
    fn resolve(&self, role_nouns: &[&str], verb: Option<&str>) -> Option<String> {
        let mut key: Vec<String> = role_nouns.iter().map(|n| {
            let (base, _) = parse_role_token(n);
            base.to_lowercase()
        }).collect();
        key.sort();
        let entries = self.by_noun_set.get(&key)?;
        let vb = verb.map(|v| v.to_lowercase());
        let allow_unique_fallback = key.len() >= 2;
        // Exact verb match
        entries.iter()
            .find(|(_, v, _)| vb.as_ref().map_or(false, |vb| v == vb))
            .or_else(||
                // Verb contained in stored reading (inverse voice: "is owned by" matches "owns")
                entries.iter()
                    .find(|(_, _, reading)| vb.as_ref().map_or(false, |vb| reading.contains(vb.as_str())))
            )
            .or_else(||
                // Unique entry for this noun set (binary+ only)
                (allow_unique_fallback && entries.len() == 1).then(|| &entries[0])
            )
            .map(|(id, _, _)| id.clone())
    }
}

/// Resolve a constraint's span fact_type_ids through the schema catalog.
fn resolve_constraint_schema(
    mut constraint: ConstraintDef,
    noun_names: &[String],
    catalog: &SchemaCatalog,
    ir: &ParseCtx,
) -> ConstraintDef {
    // Extract nouns from the constraint text to find the target schema.
    // Strip quantifiers and quoted values before noun matching.
    let mut stripped = constraint.text
        .replace("It is obligatory that ", "").replace("It is forbidden that ", "")
        .replace("It is permitted that ", "")
        .replace("Each ", "").replace("each ", "")
        .replace("at most one ", "").replace("exactly one ", "")
        .replace("at least one ", "").replace("some ", "")
        .replace("No ", "").replace("no ", "");
    // Remove quoted values like 'Overnight' that interfere with noun matching
    while let Some(start) = stripped.find('\'') {
        if let Some(end) = stripped[start + 1..].find('\'') {
            stripped = format!("{}{}", &stripped[..start], &stripped[start + 1 + end + 1..]);
        } else {
            break;
        }
    }
    let found = find_nouns(&stripped, noun_names);

    let resolved_schema = (found.len() >= 2).then(|| {
        let role_nouns: Vec<&str> = found.iter().map(|(_, _, n)| n.as_str()).collect();
        // Extract verb between first two nouns
        let verb_text = stripped[found[0].1..found[1].0].trim();
        let verb = (!verb_text.is_empty()).then_some(verb_text);

        // Primary: rho-lookup through catalog (exact verb, then reading containment, then unique)
        // Secondary: verb containment against ir.fact_types readings (handles inverse voice
        // when multiple schemas share the same noun pair)
        catalog.resolve(&role_nouns, verb)
            .or_else(|| catalog.resolve(&role_nouns, None))
            .or_else(|| {
                // Inverse voice fallback: find schema where constraint verb appears in reading
                // or reading verb appears in constraint text. #283 — cell read.
                let noun_set: hashbrown::HashSet<String> = role_nouns.iter().map(|s| s.to_string()).collect();
                fact_types_from_cells(&ir.cells).into_iter()
                    .filter(|(_, ft)| {
                        let ft_nouns: hashbrown::HashSet<String> = ft.roles.iter()
                            .map(|r| r.noun_name.clone()).collect();
                        ft_nouns == noun_set
                    })
                    .find(|(_, ft)| {
                        verb.map_or(false, |v| {
                            let v_lower = v.to_lowercase();
                            let r_lower = ft.reading.to_lowercase();
                            // Check word stem overlap: words sharing a 3+ char prefix
                            // ("owned"/"owns" share "own", "administered"/"administers" share "administ")
                            let shared_prefix = |a: &str, b: &str| -> usize {
                                a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
                            };
                            v_lower.split_whitespace()
                                .any(|w| w.len() >= 3 && r_lower.split_whitespace()
                                    .any(|rw| rw.len() >= 3 && shared_prefix(w, rw) >= 3))
                        })
                    })
                    .map(|(id, _)| id)
            })
    }).flatten();

    resolved_schema.into_iter().for_each(|schema_id| {
        // Update spans to reference the resolved schema ID.
        // The constrained role is determined by the quantifier position
        // in the verbalization pattern. "Each A R at most one B" constrains
        // A's role (the quantified noun). "It is forbidden that A R B"
        // constrains A's role (the first noun after the prefix).
        // Per Halpin TechReport ORM2-02: the constrained role is the one
        // under the quantifier.
        // #283 — Role cell read.
        let resolved_ft_roles = fact_type_roles(&ir.cells, &schema_id);
        let first_noun_idx = (!resolved_ft_roles.is_empty()).then(|| {
            let first_noun = &found[0].2;
            resolved_ft_roles.iter().position(|r| &r.noun_name == first_noun)
        }).flatten();
        constraint.spans.iter_mut().for_each(|span| {
            span.fact_type_id = schema_id.clone();
            // Set role_index to the first noun's position in the fact type.
            // The first noun in the constraint text is the quantified noun
            // ("Each A", "the same A", "A" after deontic prefix).
            first_noun_idx.into_iter().for_each(|idx| span.role_index = idx);
        });
    });
    constraint
}

/// Parse a role token into (base_noun_name, full_token_with_subscript).
/// "Person1" -> ("Person", "Person1"). "User" -> ("User", "User").
fn parse_role_token(token: &str) -> (&str, &str) {
    let boundary = token
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_digit())
        .last()
        .map(|(i, _)| i)
        .unwrap_or(token.len());
    (&token[..boundary], token)
}

fn parse_fact(line: &str, noun_names: &[String]) -> Option<(String, FactTypeDef, Option<String>)> {
    let period_stripped = line.trim_end_matches('.');
    // ORM 2 derivation marker (Halpin ORM2.pdf p. 8): `**` derived-and-
    // stored, `*` fully-derived, `+` semi-derived. Strip a trailing
    // `" **"`, `" *"`, or `" +"` from the line BEFORE noun detection so
    // the remaining text is a clean reading for both binary
    // (`Customer has Plan. *`) and unary (`Customer is in EEA. *`)
    // fact types. The unary path then has a real predicate tail
    // available; without the pre-strip the marker would dangle inside
    // it and corrupt the catalog verb.
    let (clean, derivation_mode) = strip_derivation_marker(period_stripped.trim_end());
    let found = find_nouns(clean, noun_names);
    // Unaries are only accepted when a derivation marker is explicitly
    // present. A bare `Customer has Amount` with `Amount` undeclared
    // must still fall through the binary path (permissive mode will
    // auto-create `Amount`) rather than silently registering as a
    // unary with a runaway predicate. #274 Category A.
    match (found.len(), derivation_mode.as_ref()) {
        (n, _) if n >= 2 => {},
        (1, Some(_)) => {},
        _ => return None,
    }

    let predicate = match found.len() {
        1 => clean[found[0].1..].trim(),
        _ => clean[found[0].1..found[1].0].trim(),
    };
    (!predicate.is_empty()).then(|| ())?;

    let reading = match found.len() {
        1 => alloc::format!("{} {}", found[0].2, predicate),
        _ => alloc::format!("{} {} {}", found[0].2, predicate, found[1].2),
    };
    let roles: Vec<RoleDef> = found.iter().enumerate()
        .map(|(i, (_, _, name))| RoleDef { noun_name: name.clone(), role_index: i })
        .collect();

    // Build role tokens for schema ID (preserving subscript digits from the source text)
    let role_refs: Vec<&str> = found.iter().map(|(_, _, name)| name.as_str()).collect();
    let schema_id = fact_type_id(&role_refs, predicate);

    let active_reading = ReadingDef {
        text: reading.clone(),
        role_order: (0..roles.len()).collect(),
    };

    Some((
        schema_id.clone(),
        FactTypeDef {
            schema_id,
            reading,
            readings: vec![active_reading],
            roles,
        },
        derivation_mode,
    ))
}

/// Split a trailing ORM 2 derivation marker (` **`, ` *`, ` +`) off a
/// fact type reading. Returns the marker-free reading plus the mode
/// atom. `**` must be checked before `*` since `.strip_suffix(" *")`
/// would match both.
fn strip_derivation_marker(text: &str) -> (&str, Option<String>) {
    if let Some(before) = text.strip_suffix(" **") {
        return (before.trim_end(), Some("derived-and-stored".to_string()));
    }
    if let Some(before) = text.strip_suffix(" *") {
        return (before.trim_end(), Some("fully-derived".to_string()));
    }
    if let Some(before) = text.strip_suffix(" +") {
        return (before.trim_end(), Some("semi-derived".to_string()));
    }
    (text, None)
}

fn parse_constraint(line: &str, noun_names: &[String]) -> Option<ConstraintDef> {
    let clean = line.trim_end_matches('.');
    let stripped = clean.replace("Each ", "").replace("each ", "")
        .replace("at most one ", "").replace("exactly one ", "")
        .replace("at least one ", "").replace("some ", "");
    let found = find_nouns(&stripped, noun_names);
    (!found.is_empty()).then(|| ())?;

    let kind = ["exactly one", "at most one", "at least", "some ", "No "].iter()
        .find(|k| clean.contains(*k))
        .map(|k| match *k {
            "at most one" => "UC",
            "exactly one" => "MC",
            "some " | "at least" => "MC",
            "No " => "XC",
            _ => "UC",
        })
        .unwrap_or("UC");

    // Derive fact type ID from the nouns in the constraint.
    // The fact type reading = "Noun1 predicate Noun2" -- extracted from the stripped text.
    let ft_id = if found.len() >= 2 {
        let predicate = stripped[found[0].1..found[1].0].trim();
        if !predicate.is_empty() {
            format!("{} {} {}", found[0].2, predicate, found[1].2)
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let spans: Vec<SpanDef> = found.iter().enumerate()
        .map(|(i, _)| SpanDef { fact_type_id: ft_id.clone(), role_index: i, subset_autofill: None })
        .collect();

    Some(ConstraintDef {
        id: String::new(), kind: kind.into(), modality: "alethic".into(),
        deontic_operator: None, text: clean.into(), spans,
        set_comparison_argument_length: None, clauses: None,
        entity: None, min_occurrence: None, max_occurrence: None,
    })
}

/// Find nouns in text -- longest-first matching with word boundaries.
/// Returns (start, end, name) tuples sorted by position.
///
/// Exposed to the crate so post-parse checks (e.g. ring completeness
/// in `check.rs`) can re-tokenize a FactType reading against the
/// fully-accumulated Noun set, independent of the parse-time noun
/// list that was available when the FactType was first parsed.
pub(crate) fn find_nouns(text: &str, noun_names: &[String]) -> Vec<(usize, usize, String)> {
    let mut sorted: Vec<&String> = noun_names.iter().collect();
    sorted.sort_by(|a, b| b.len().cmp(&a.len()));

    // #273: prose-heavy rule bodies (legal text, derivations) routinely
    // mention a declared noun in lowercase — "… if interpretation is
    // reasonable" against a capitalised `Interpretation` entity type.
    // We match case-insensitively against ASCII-lowercased copies so
    // that drift doesn't fall through to "antecedent clause did not
    // resolve". ASCII-lowercasing preserves byte length, so indices
    // in `text_lower` map 1:1 back to `text`; the captured token is
    // taken from `text` to preserve the reading-author's casing for
    // downstream ring / join-key consumers.
    let text_lower: String = text.chars().map(|c| c.to_ascii_lowercase()).collect();

    // Foldl over longest-first noun list. Accumulator is (matches, used_ranges).
    // Inner loop over occurrences of `name` in `text` uses Backus's `while`
    // combining form (sequential scan of positions).
    //
    // Halpin ring rules distinguish same-type roles by numeric subscripts
    // (Person1, Person2, Person3 â€” see Example 6 in the FORML position
    // paper). When the match is followed by ASCII digits we treat them
    // as a subscript and extend the captured range to include them; the
    // returned token ("Person3") preserves subscript identity so join-
    // key detection downstream works, and parse_role_token strips it to
    // the base ("Person") before catalog lookup.
    let (mut matches, _): (Vec<(usize, usize, String)>, Vec<(usize, usize)>) = sorted.iter().fold(
        (Vec::new(), Vec::new()),
        |(mut matches, mut used), name| {
            let name_lower: String = name.chars().map(|c| c.to_ascii_lowercase()).collect();
            let mut pos = 0;
            while let Some(found) = text_lower[pos..].find(name_lower.as_str()) {
                let start = pos + found;
                let mut end = start + name_lower.len();
                let before_ok = start == 0 || !text.as_bytes()[start - 1].is_ascii_alphanumeric();
                // Extend end past any trailing ASCII digit subscript.
                while end < text.len() && text.as_bytes()[end].is_ascii_digit() {
                    end += 1;
                }
                // After the (possibly-extended) end, the next byte must
                // not be alphanumeric â€” otherwise the match was part of
                // a longer identifier.
                let after_ok = end >= text.len() || !text.as_bytes()[end].is_ascii_alphanumeric();
                let no_overlap = !used.iter().any(|&(s, e)| start < e && end > s);

                if before_ok && after_ok && no_overlap {
                    // Capture the subscripted token (e.g. "Person3") so
                    // callers distinguish the ring positions. The base
                    // name is recovered via parse_role_token at the
                    // resolve site.
                    let captured = &text[start..end];
                    matches.push((start, end, captured.to_string()));
                    used.push((start, end));
                }
                pos = start + 1;
                if pos >= text.len() { break; }
            }
            (matches, used)
        },
    );

    matches.sort_by_key(|m| m.0);
    matches
}

// =========================================================================
// Instance fact parsing (state machines)
// =========================================================================

fn parse_instance_fact(ir: &mut ParseCtx, line: &str, _lines: &[&str], _idx: usize) {
    let clean = line.trim_end_matches('.');
    parse_general_instance_fact(ir, clean);
}

fn parse_general_instance_fact(ir: &mut ParseCtx, line: &str) {
    // Longest-first noun matching (Theorem 1, step 3)
    let mut noun_names: Vec<String> = noun_names(&ir.cells);
    noun_names.sort_by(|a, b| b.len().cmp(&a.len()));

    // bu(match_subject, line) -- find the first noun that matches as subject
    let subject = noun_names.iter()
        .filter_map(|noun| {
            let prefix = format!("{} '", noun);
            line.starts_with(&prefix).then(|| {
                let after = &line[prefix.len()..];
                after.find('\'').map(|end| (noun.clone(), after[..end].to_string(), after[end + 1..].trim()))
            })?
        })
        .next();

    let (subject_noun, subject_value, rest) = match subject {
        Some((n, v, r)) => (n, v, r),
        None => return,
    };

    // bu(match_object, rest) -- find the object noun+value in the remainder
    let object_match = noun_names.iter()
        .filter_map(|noun| {
            let obj_prefix = format!("{} '", noun);
            rest.find(&obj_prefix).and_then(|pred_end| {
                let predicate = rest[..pred_end].trim();
                let obj_rest = &rest[pred_end + obj_prefix.len()..];
                obj_rest.find('\'').map(|end| (predicate.to_string(), noun.clone(), obj_rest[..end].to_string()))
            })
        })
        .next();

    let fact = match object_match {
        Some((predicate, object_noun, object_value)) => {
            // Resolve field name from declared fact types.
            // The instance fact "A 'x' predicate B 'y'" should match the
            // declared fact type "A predicate B" and use its fact type ID.
            let field = resolve_instance_field(&fact_types_from_cells(&ir.cells), &subject_noun, &predicate, &object_noun);
            Some(GeneralInstanceFact {
                subject_noun,
                subject_value,
                field_name: field,
                object_noun,
                object_value,
            })
        }
        None => extract_value_fact(rest).map(|(predicate, value)| GeneralInstanceFact {
            subject_noun,
            subject_value,
            field_name: to_camel_case(&predicate),
            object_noun: String::new(),
            object_value: value,
        }),
    };

    // #283 — emit cells directly instead of pushing a typed
    // GeneralInstanceFact. `emit_instance_fact` writes both
    // `InstanceFact` and the per-field cell.
    fact.into_iter().for_each(|f| {
        emit_instance_fact(ir,
            &f.subject_noun, &f.subject_value,
            &f.field_name, &f.object_noun, &f.object_value);
    });
}

/// Resolve the field name for an instance fact by looking up the declared fact type.
/// Matches the subject noun and object noun against fact type roles, using the
/// predicate for disambiguation when multiple fact types share the same noun pair.
fn resolve_instance_field(
    fact_types: &HashMap<String, FactTypeDef>,
    subject_noun: &str,
    predicate: &str,
    object_noun: &str,
) -> String {
    // Find fact types where role 0 matches subject and role 1 matches object
    let candidates: Vec<(&str, &FactTypeDef)> = fact_types.iter()
        .filter(|(_, ft)| {
            ft.roles.len() >= 2
                && ft.roles[0].noun_name == subject_noun
                && ft.roles[1].noun_name == object_noun
        })
        .map(|(id, ft)| (id.as_str(), ft))
        .collect();

    // Resolve: find the declared fact type and extract the predicate from it.
    // The predicate in the reading is the canonical field name source.
    let pred_lower = predicate.to_lowercase();
    let matched = candidates.iter()
        .find(|(_, ft)| {
            let r = ft.reading.to_lowercase();
            // Check if the reading contains the predicate words
            pred_lower.split_whitespace().all(|w| r.contains(w))
        })
        .or_else(|| candidates.first());

    if let Some((id, _)) = matched {
        // The field name is the fact type ID. This is the fact type identity.
        id.to_string()
    } else {
        // No declared fact type. Also try reverse role order.
        let reverse = fact_types.iter()
            .find(|(_, ft)| {
                ft.roles.len() >= 2
                    && ft.roles[1].noun_name == subject_noun
                    && ft.roles[0].noun_name == object_noun
            });
        if let Some((_, ft)) = reverse {
            let reading = &ft.reading;
            let after_obj = reading.find(object_noun)
                .map(|i| &reading[i + object_noun.len()..])
                .unwrap_or(reading);
            let before_subj = after_obj.find(subject_noun)
                .map(|i| &after_obj[..i])
                .unwrap_or(after_obj);
            to_camel_case(before_subj.trim())
        } else {
            to_camel_case(predicate)
        }
    }
}

fn extract_value_fact(rest: &str) -> Option<(String, String)> {
    let last_q_end = rest.rfind('\'')?;
    let before_last = &rest[..last_q_end];
    let val_start = before_last.rfind('\'')?;
    let value = before_last[val_start + 1..].to_string();
    let predicate = before_last[..val_start].trim().to_string();
    Some((predicate, value))
}

fn to_camel_case(s: &str) -> String {
    let mut words = s.split_whitespace().filter(|w| !w.is_empty());
    let first = match words.next() {
        Some(w) => w.to_lowercase(),
        None => return String::new(),
    };
    words.fold(first, |mut acc, word| {
        let mut chars = word.chars();
        chars.next().into_iter().for_each(|first_ch| {
            acc.push(first_ch.to_uppercase().next().unwrap_or(first_ch));
        });
        acc.extend(chars);
        acc
    })
}



// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_types() {
        let ir = parse_markdown("Customer(.Name) is an entity type.\nOrder(.OrderId) is an entity type.").unwrap();
        // #283 — nouns live in the Noun cell.
        assert_eq!(super::noun_names(&ir.cells).len(), 2);
        assert!(super::noun_exists(&ir.cells, "Customer"));
        assert!(super::noun_exists(&ir.cells, "Order"));
    }

    #[test]
    fn value_types_with_enum() {
        let ir = parse_markdown("Priority is a value type.\n  The possible values of Priority are 'low', 'medium', 'high'.").unwrap();
        // #283 — Noun object type lives on the Noun cell.
        assert_eq!(super::noun_object_type(&ir.cells, "Priority").as_deref(), Some("value"));
        // #283 — enum values live in the EnumValues cell.
        assert_eq!(super::enum_values_for_noun(&ir.cells, "Priority").len(), 3);
    }

    #[test]
    fn subtypes() {
        let ir = parse_markdown("Request(.id) is an entity type.\nSupport Request is a subtype of Request.").unwrap();
        // #283 — subtype relationships live in the Subtype cell.
        assert_eq!(
            super::supertype_of(&ir.cells, "Support Request"),
            Some("Request".to_string())
        );
    }

    #[test]
    fn abstract_noun() {
        let ir = parse_markdown("Request(.id) is an entity type.\nRequest is abstract.").unwrap();
        // #283 — Noun object type lives on the Noun cell.
        assert_eq!(super::noun_object_type(&ir.cells, "Request").as_deref(), Some("abstract"));
    }

    #[test]
    fn partition_implies_abstract() {
        let ir = parse_markdown("Request(.id) is an entity type.\nRequest is partitioned into Support Request, Feature Request.").unwrap();
        // #283 — Noun object type lives on the Noun cell.
        assert_eq!(super::noun_object_type(&ir.cells, "Request").as_deref(), Some("abstract"));
        assert_eq!(
            super::supertype_of(&ir.cells, "Support Request"),
            Some("Request".to_string())
        );
    }

    #[test]
    fn totality_implies_abstract() {
        let input = "Request(.id) is an entity type.\nSupport Request is a subtype of Request.\nEach Request is a Support Request or a Feature Request.";
        let ir = parse_markdown(input).unwrap();
        // #283 — Noun object type lives on the Noun cell.
        assert_eq!(super::noun_object_type(&ir.cells, "Request").as_deref(), Some("abstract"));
    }

    #[test]
    fn fact_types() {
        let input = "Customer(.Name) is an entity type.\nOrder(.OrderId) is an entity type.\nOrder was placed by Customer.";
        let ir = parse_markdown(input).unwrap();
        // #283 — FactType cell read.
        assert!(!super::fact_type_ids(&ir.cells).is_empty());
    }

    #[test]
    fn exactly_one_splits_to_uc_mc() {
        let input = "Person(.Name) is an entity type.\nCountry(.Code) is an entity type.\nPerson was born in Country.\nEach Person was born in exactly one Country.";
        let ir = parse_markdown(input).unwrap();
        // #283 — Constraint cell reads.
        let cs = super::constraints_from_cells(&ir.cells);
        assert!(cs.iter().any(|c| c.kind == "UC"));
        assert!(cs.iter().any(|c| c.kind == "MC"));
    }

    #[test]
    fn deontic_constraints() {
        let input = "Response(.id) is an entity type.\nIt is obligatory that each Response is professional.";
        let ir = parse_markdown(input).unwrap();
        // #283 — Constraint cell read.
        assert!(super::constraints_from_cells(&ir.cells).iter().any(|c| c.modality == "deontic"));
    }

    #[test]
    fn derivation_rules() {
        let input = "X(.id) is an entity type.\nY(.id) is an entity type.\nX has Y iff some condition.";
        let ir = parse_markdown(input).unwrap();
        assert!(!ir.derivation_rules.is_empty());
    }

    #[test]
    fn instance_facts_value() {
        let input = "Domain(.Slug) is an entity type.\nAccess is a value type.\n  The possible values of Access are 'public', 'private'.\n## Fact Types\nDomain has Access.\n## Instance Facts\nDomain 'support' has Access 'public'.";
        let ir = parse_markdown(input).unwrap();
        // #283 — instance facts live in the InstanceFact cell.
        let facts = ir.cells.get("InstanceFact").expect("InstanceFact cell");
        assert_eq!(facts.len(), 1);
        assert_eq!(crate::ast::binding(&facts[0], "subjectNoun"), Some("Domain"));
        assert_eq!(crate::ast::binding(&facts[0], "subjectValue"), Some("support"));
        assert_eq!(crate::ast::binding(&facts[0], "objectValue"), Some("public"));
    }

    #[test]
    fn instance_facts_noun_to_noun() {
        let input = "API Endpoint(.Path) is an entity type.\nClickHouse Table(.Name) is an entity type.\n## Fact Types\nAPI Endpoint reads from ClickHouse Table.\n## Instance Facts\nAPI Endpoint '/data/:vin' reads from ClickHouse Table 'sources.currentResources'.";
        let ir = parse_markdown(input).unwrap();
        let facts = ir.cells.get("InstanceFact").expect("InstanceFact cell");
        assert_eq!(facts.len(), 1);
        assert_eq!(crate::ast::binding(&facts[0], "subjectNoun"), Some("API Endpoint"));
        assert_eq!(crate::ast::binding(&facts[0], "subjectValue"), Some("/data/:vin"));
        assert_eq!(crate::ast::binding(&facts[0], "objectNoun"), Some("ClickHouse Table"));
        assert_eq!(crate::ast::binding(&facts[0], "objectValue"), Some("sources.currentResources"));
    }

    #[test]
    fn instance_facts_multiple() {
        let input = "Domain(.Slug) is an entity type.\nAccess is a value type.\nDomain has Access.\nDomain 'support' has Access 'public'.\nDomain 'core' has Access 'private'.";
        let ir = parse_markdown(input).unwrap();
        let facts = ir.cells.get("InstanceFact").expect("InstanceFact cell");
        assert_eq!(facts.len(), 2);
    }

    #[test]
    fn instance_fact_noun_uri() {
        let input = "Noun is an entity type.\nURI is a value type.\n## Fact Types\nNoun has URI.\n## Instance Facts\nNoun 'API Product' has URI '/api'.";
        let ir = parse_markdown(input).unwrap();
        let facts = ir.cells.get("InstanceFact").expect("InstanceFact cell");
        assert_eq!(facts.len(), 1);
        let f = &facts[0];
        assert_eq!(crate::ast::binding(f, "subjectNoun"), Some("Noun"));
        assert_eq!(crate::ast::binding(f, "subjectValue"), Some("API Product"));
        assert_eq!(crate::ast::binding(f, "objectValue"), Some("/api"));
    }

    #[test]
    fn derivation_rule_extracts_fact_types() {
        let input = "User(.Email) is an entity type.\nDomain(.Slug) is an entity type.\nOrg Role is a value type.\nOrganization(.Slug) is an entity type.\n## Fact Types\nUser has Org Role in Organization.\nDomain belongs to Organization.\nUser accesses Domain.\n## Derivation Rules\nUser accesses Domain if User has Org Role in Organization and Domain belongs to that Organization.";
        let ir = parse_markdown(input).unwrap();
        assert!(!ir.derivation_rules.is_empty());
        let rule = &ir.derivation_rules[0];
        assert!(!rule.consequent_fact_type_id.is_empty(), "consequent should be resolved");
        assert!(!rule.antecedent_fact_type_ids.is_empty(), "antecedents should be resolved");
        assert!(rule.antecedent_fact_type_ids.len() >= 2, "should have at least 2 antecedents");
    }

    #[test]
    fn derivation_rule_identifies_join_keys() {
        let input = "User(.Email) is an entity type.\nDomain(.Slug) is an entity type.\nOrg Role is a value type.\nOrganization(.Slug) is an entity type.\n## Fact Types\nUser has Org Role in Organization.\nDomain belongs to Organization.\nUser accesses Domain.\n## Derivation Rules\nUser accesses Domain if User has Org Role in Organization and Domain belongs to that Organization.";
        let ir = parse_markdown(input).unwrap();
        let rule = &ir.derivation_rules[0];
        assert!(rule.join_on.contains(&"Organization".to_string()), "Organization should be a join key (referenced with 'that')");
    }

    // â”€â”€ Inline comparisons on antecedent roles (Halpin FORML Example 5) â”€â”€
    //
    // `Each LargeUSCity is a City that is in Country 'US' and has Population >= 1000000.`
    // The parser should:
    //   (1) resolve the base fact type (`has Population` â†’ FT_Population)
    //       without being confused by the trailing comparator;
    //   (2) capture `>=`, `1000000` into DerivationRuleDef::antecedent_filters
    //       pinned to the antecedent's index; and
    //   (3) accept the full Halpin operator set: `>=`, `<=`, `>`, `<`, `=`,
    //       `!=`, and `<>` (which is normalized to `!=`).

    #[test]
    fn derivation_rule_captures_inline_ge_comparison() {
        let input = "City(.Name) is an entity type.\nBig City(.Name) is an entity type.\nPopulation is a value type.\n## Fact Types\nCity has Population.\nBig City has City.\n## Derivation Rules\n* Big City has City iff City has Population >= 1000000.";
        let ir = parse_markdown(input).unwrap();
        let rule = ir.derivation_rules.iter()
            .find(|r| r.text.contains("Big City has City"))
            .expect("derivation rule present");
        assert_eq!(rule.antecedent_filters.len(), 1,
            "one inline comparison expected, got {:#?}", rule.antecedent_filters);
        let f = &rule.antecedent_filters[0];
        assert_eq!(f.op, ">=");
        assert_eq!(f.value, 1_000_000.0);
        assert_eq!(f.role, "Population");
        // Antecedent still resolves to the base fact type â€” the comparator
        // is a filter on it, not a replacement.
        assert!(rule.antecedent_fact_type_ids.iter().any(|id| id.contains("Population")),
            "base FT should still resolve: {:?}", rule.antecedent_fact_type_ids);
    }

    #[test]
    fn derivation_rule_accepts_all_comparison_operators() {
        // Parametric sweep over the six Halpin operators. `<>` normalizes to
        // `!=` at parse time so downstream compile can dispatch on one form.
        for (op_in, op_out) in [
            (">=", ">="), ("<=", "<="),
            (">",  ">"),  ("<",  "<"),
            ("=",  "="),  ("!=", "!="),
            ("<>", "!="),
        ] {
            let input = format!(
                "City(.Name) is an entity type.\nBig City(.Name) is an entity type.\nPopulation is a value type.\n## Fact Types\nCity has Population.\nBig City has City.\n## Derivation Rules\n* Big City has City iff City has Population {op_in} 100.");
            let ir = parse_markdown(&input).expect("parse ok");
            let rule = ir.derivation_rules.iter()
                .find(|r| r.text.contains("Big City has City"))
                .unwrap_or_else(|| panic!("rule missing for op {op_in}"));
            assert_eq!(rule.antecedent_filters.len(), 1, "op {op_in}: filters = {:?}", rule.antecedent_filters);
            assert_eq!(rule.antecedent_filters[0].op, op_out,
                "op {op_in} should normalize to {op_out}, got {}", rule.antecedent_filters[0].op);
            assert_eq!(rule.antecedent_filters[0].value, 100.0, "op {op_in} value");
        }
    }

    #[test]
    fn derivation_rule_handles_float_and_negative_literals() {
        // Float + negative literal should parse; irrelevant whitespace too.
        let input = "Reading(.Name) is an entity type.\nWarm Reading(.Name) is an entity type.\nTemperature is a value type.\n## Fact Types\nReading has Temperature.\nWarm Reading has Reading.\n## Derivation Rules\n* Warm Reading has Reading iff Reading has Temperature > -273.15.";
        let ir = parse_markdown(input).unwrap();
        let rule = ir.derivation_rules.iter()
            .find(|r| r.text.contains("Warm Reading has Reading"))
            .expect("rule present");
        assert_eq!(rule.antecedent_filters.len(), 1);
        assert_eq!(rule.antecedent_filters[0].op, ">");
        assert!((rule.antecedent_filters[0].value - (-273.15)).abs() < 1e-9,
            "expected -273.15, got {}", rule.antecedent_filters[0].value);
    }

    // â”€â”€ Arithmetic definitional clauses (Halpin attribute style) â”€â”€
    //
    // An antecedent clause of shape `<RoleName> is <arith-expr>` defines a
    // consequent role's value from other role values or numeric literals.
    // Supports `+`, `-`, `*`, `/`; left-associative; parentheses not yet
    // supported. These clauses populate `consequent_computed_bindings` and
    // are stripped from `antecedent_fact_type_ids` since they aren't fact
    // types to resolve.

    fn single_op(op: &str, lhs: &str, rhs: &str) -> crate::types::ArithExpr {
        use crate::types::ArithExpr;
        ArithExpr::Op(op.to_string(),
            Box::new(ArithExpr::RoleRef(lhs.to_string())),
            Box::new(ArithExpr::RoleRef(rhs.to_string())))
    }

    #[test]
    fn derivation_rule_captures_computed_binding_with_plus() {
        let input = "Foo(.id) is an entity type.\nVal is a value type.\nDoubled is a value type.\n## Fact Types\nFoo has Val.\nFoo has Doubled.\n## Derivation Rules\n* Foo has Doubled iff Foo has Val and Doubled is Val + Val.";
        let ir = parse_markdown(input).unwrap();
        let rule = ir.derivation_rules.iter()
            .find(|r| r.text.contains("Foo has Doubled"))
            .expect("rule present");
        assert_eq!(rule.consequent_computed_bindings.len(), 1,
            "expected one computed binding, got {:?}", rule.consequent_computed_bindings);
        let cb = &rule.consequent_computed_bindings[0];
        assert_eq!(cb.role, "Doubled");
        assert_eq!(cb.expr, single_op("+", "Val", "Val"));
        // The definitional antecedent doesn't resolve to a fact type;
        // only `Foo has Val` should remain in antecedent_fact_type_ids.
        assert_eq!(rule.antecedent_fact_type_ids.len(), 1,
            "definitional clause must be stripped from antecedents, got {:?}",
            rule.antecedent_fact_type_ids);
    }

    #[test]
    fn derivation_rule_accepts_all_arithmetic_operators() {
        use crate::types::ArithExpr;
        for op in ["+", "-", "*", "/"] {
            let input = format!(
                "Foo(.id) is an entity type.\nA is a value type.\nB is a value type.\nC is a value type.\n## Fact Types\nFoo has A.\nFoo has B.\nFoo has C.\n## Derivation Rules\n* Foo has C iff Foo has A and Foo has B and C is A {op} B.");
            let ir = parse_markdown(&input).expect("parse ok");
            let rule = ir.derivation_rules.iter()
                .find(|r| r.text.contains("Foo has C"))
                .unwrap_or_else(|| panic!("rule missing for op {op}"));
            assert_eq!(rule.consequent_computed_bindings.len(), 1, "op {op} missed");
            let cb = &rule.consequent_computed_bindings[0];
            assert_eq!(cb.role, "C");
            assert_eq!(cb.expr,
                ArithExpr::Op(op.to_string(),
                    Box::new(ArithExpr::RoleRef("A".to_string())),
                    Box::new(ArithExpr::RoleRef("B".to_string()))),
                "op {op}");
        }
    }

    #[test]
    fn derivation_rule_chained_operators_are_left_associative() {
        use crate::types::ArithExpr;
        let input = "Box(.id) is an entity type.\nSize is a value type.\nVolume is a value type.\n## Fact Types\nBox has Size.\nBox has Volume.\n## Derivation Rules\n* Box has Volume iff Box has Size and Volume is Size * Size * Size.";
        let ir = parse_markdown(input).unwrap();
        let rule = ir.derivation_rules.iter()
            .find(|r| r.text.contains("Box has Volume"))
            .expect("rule present");
        assert_eq!(rule.consequent_computed_bindings.len(), 1);
        let cb = &rule.consequent_computed_bindings[0];
        // Size * Size * Size parses as ((Size * Size) * Size).
        let size = || ArithExpr::RoleRef("Size".to_string());
        let inner = ArithExpr::Op("*".to_string(), Box::new(size()), Box::new(size()));
        let outer = ArithExpr::Op("*".to_string(), Box::new(inner), Box::new(size()));
        assert_eq!(cb.expr, outer);
    }

    #[test]
    fn derivation_rule_accepts_numeric_literal_operands() {
        use crate::types::ArithExpr;
        let input = "Foo(.id) is an entity type.\nVal is a value type.\nNext is a value type.\n## Fact Types\nFoo has Val.\nFoo has Next.\n## Derivation Rules\n* Foo has Next iff Foo has Val and Next is Val + 1.";
        let ir = parse_markdown(input).unwrap();
        let rule = ir.derivation_rules.iter()
            .find(|r| r.text.contains("Foo has Next"))
            .expect("rule present");
        let cb = &rule.consequent_computed_bindings[0];
        assert_eq!(cb.role, "Next");
        assert_eq!(cb.expr,
            ArithExpr::Op("+".to_string(),
                Box::new(ArithExpr::RoleRef("Val".to_string())),
                Box::new(ArithExpr::Literal(1.0))));
    }

    // â”€â”€ Aggregate clauses (Codd Â§2.3.4 image set + Backus Insert) â”€â”€
    //
    // Halpin's attribute-style aggregate reads as:
    //   `<role> is the <op> of <target> where <where-clause>`
    // where <op> is one of count/sum/avg/min/max and <where-clause>
    // resolves to a source fact type. The consequent's non-aggregate role
    // becomes the group key (the image-set index in Codd's terms).

    // â”€â”€ Halpin's "attribute style" reduces to relational style + Join â”€â”€
    //
    // Halpin FORML Example 6 gives two equivalent forms of the uncle rule:
    //
    //   Relational:  Define Person1 is an uncle of Person2 as
    //                Person1 is a brother of some Person3 who is a parent
    //                of Person2.
    //
    //   Attribute:   For each Person: uncle = brother of parent.
    //
    // Both assert the same implication. AREST takes the relational form
    // (with `that <join-noun>` anaphora) and routes it through
    // compile_join_derivation â€” so attribute style is structurally
    // redundant. This test uses non-ring fact types (no subscripts) to
    // demonstrate the pattern that makes attribute style unnecessary.

    #[test]
    fn find_nouns_captures_subscripted_tokens() {
        // #197 fix: find_nouns now captures the subscripted form
        // ("Person3") rather than rejecting the match because of the
        // trailing digit. The base noun is recoverable via
        // parse_role_token.
        let noun_names = vec!["Person".to_string()];
        let matches = find_nouns("Person1 is brother of Person3", &noun_names);
        assert_eq!(matches.len(), 2, "two ring positions, got {:?}", matches);
        assert_eq!(matches[0].2, "Person1");
        assert_eq!(matches[1].2, "Person3");
        // Base recovery preserves subscript-free form.
        assert_eq!(parse_role_token(&matches[0].2).0, "Person");
        assert_eq!(parse_role_token(&matches[1].2).0, "Person");
    }

    #[test]
    fn find_nouns_still_rejects_alphanumeric_overruns() {
        // Regression: `Person` in `Personal` is still NOT a match â€”
        // only trailing digits count as subscripts; letters don't.
        let noun_names = vec!["Person".to_string()];
        let matches = find_nouns("Personal belongings", &noun_names);
        assert_eq!(matches.len(), 0, "`Personal` must not match `Person`");
    }

    #[test]
    fn find_nouns_rejects_leading_alphanumeric() {
        // Regression: `Super Person` doesn't match `Person` either â€”
        // the before-boundary check stays strict.
        let noun_names = vec!["Person".to_string()];
        let matches = find_nouns("SuperPerson rules", &noun_names);
        assert_eq!(matches.len(), 0);
    }

    #[test]
    fn path_composition_via_relational_join_parses_as_join() {
        // "Worker reports up via Manager" - the classic 2-hop path.
        // Attribute form would be: For each Worker: up_line = reports_to of reports_to.
        // We write the relational equivalent, which the existing Join
        // path handles natively.
        let input = "Worker(.Id) is an entity type.\nManager(.Id) is an entity type.\nVP(.Id) is an entity type.\n## Fact Types\nWorker reports to Manager.\nManager reports to VP.\nWorker reports up to VP.\n## Derivation Rules\n+ Worker reports up to VP if Worker reports to some Manager and that Manager reports to VP.";
        let ir = parse_markdown(input).unwrap();
        let rule = ir.derivation_rules.iter()
            .find(|r| r.text.contains("reports up to VP"))
            .expect("rule present");
        assert_eq!(rule.kind, crate::types::DerivationKind::Join,
            "two-antecedent rule with `that Manager` should be Join, got {:?}", rule.kind);
        assert!(rule.join_on.iter().any(|k| k == "Manager"),
            "join_on should include Manager, got {:?}", rule.join_on);
        assert_eq!(rule.antecedent_fact_type_ids.len(), 2,
            "two antecedents, got {:?}", rule.antecedent_fact_type_ids);
    }

    // â”€â”€ Ring-constraint shorthand (ORM 2 intuitive-icon parity) â”€â”€
    //
    // ORM 2 Â§2.6 (Halpin 2005) renders ring constraints as icons ("ir",
    // "ac", "as", â€¦) attached to the fact-type shape. In textual form,
    // the equivalent shorthand appends the adjective directly to the
    // reading:
    //
    //   Category has parent Category is acyclic.
    //   Task blocks Task is irreflexive.
    //
    // This is a terser alternative to the canonical prose ("No Category
    // may cycle back to itself via one or more traversals through has
    // parent."). Both forms MUST compile to the same constraint kind.

    #[test]
    fn subset_with_equal_to_clause_parses_as_ss() {
        // Halpin's canonical subset-constraint uses `that is` to assert
        // two roles carry the same value:
        //   If some Customer places some Order then that Order has
        //   Shipping Address that is that Customer's Shipping Address.
        //
        // `equal to` is a natural-English alias for the same clause.
        // Both forms should parse as an SS constraint.
        for linker in ["that is", "equal to"] {
            let input = format!(
                "Customer(.Name) is an entity type.\nOrder(.Id) is an entity type.\nShipping Address is a value type.\n## Fact Types\nCustomer places Order.\nCustomer has Shipping Address.\nOrder has Shipping Address.\n## Constraints\nIf some Customer places some Order then that Order has Shipping Address {linker} that Customer's Shipping Address.");
            let ir = parse_markdown(&input).unwrap_or_else(|e| panic!("linker={linker:?}: {e:?}"));
            // #283 — Constraint cell read.
            let cs = super::constraints_from_cells(&ir.cells);
            let ss: Vec<_> = cs.iter().filter(|c| c.kind == "SS").collect();
            assert!(!ss.is_empty(),
                "linker={linker:?}: expected at least one SS, got kinds {:?}",
                cs.iter().map(|c| &c.kind).collect::<Vec<_>>());
        }
    }

    #[test]
    fn ring_shorthand_acyclic_emits_ac_constraint() {
        let input = "Category(.Name) is an entity type.\n## Fact Types\nCategory has parent Category.\nCategory has parent Category is acyclic.";
        let ir = parse_markdown(input).unwrap();
        // #283 — Constraint cell read.
        let cs = super::constraints_from_cells(&ir.cells);
        let ac: Vec<_> = cs.iter().filter(|c| c.kind == "AC").collect();
        assert_eq!(ac.len(), 1, "expected one AC constraint, got {:?}",
            cs.iter().map(|c| &c.kind).collect::<Vec<_>>());
        assert_eq!(ac[0].entity, Some("Category".to_string()));
    }

    #[test]
    fn ring_shorthand_irreflexive_emits_ir_constraint() {
        let input = "Person(.Name) is an entity type.\n## Fact Types\nPerson is parent of Person.\nPerson is parent of Person is irreflexive.";
        let ir = parse_markdown(input).unwrap();
        // #283 — Constraint cell read.
        let cs = super::constraints_from_cells(&ir.cells);
        let irref: Vec<_> = cs.iter().filter(|c| c.kind == "IR").collect();
        assert_eq!(irref.len(), 1, "expected one IR, got {:?}",
            cs.iter().map(|c| &c.kind).collect::<Vec<_>>());
        assert_eq!(irref[0].entity, Some("Person".to_string()));
    }

    #[test]
    fn ring_shorthand_covers_all_eight_kinds() {
        for (adj, want_kind) in [
            ("irreflexive", "IR"),
            ("asymmetric", "AS"),
            ("antisymmetric", "AT"),
            ("symmetric", "SY"),
            ("intransitive", "IT"),
            ("transitive", "TR"),
            ("acyclic", "AC"),
            ("reflexive", "RF"),
        ] {
            let input = format!(
                "Person(.Name) is an entity type.\n## Fact Types\nPerson is parent of Person.\nPerson is parent of Person is {adj}.");
            let ir = parse_markdown(&input).unwrap_or_else(|e| panic!("parse {adj}: {e:?}"));
            // #283 — Constraint cell read.
            let cs = super::constraints_from_cells(&ir.cells);
            let hits: Vec<_> = cs.iter().filter(|c| c.kind == want_kind).collect();
            assert_eq!(hits.len(), 1, "adj={adj}: expected {want_kind}, got {:?}",
                cs.iter().map(|c| &c.kind).collect::<Vec<_>>());
        }
    }

    #[test]
    fn derivation_rule_captures_count_aggregate() {
        let input = "Thing(.Name) is an entity type.\nPart(.Name) is an entity type.\nArity is a value type.\n## Fact Types\nThing has Part.\nThing has Arity.\n## Derivation Rules\n* Thing has Arity iff Arity is the count of Part where Thing has Part.";
        let ir = parse_markdown(input).unwrap();
        let rule = ir.derivation_rules.iter()
            .find(|r| r.text.contains("Thing has Arity"))
            .expect("rule present");
        assert_eq!(rule.consequent_aggregates.len(), 1,
            "expected one aggregate, got {:?}", rule.consequent_aggregates);
        let a = &rule.consequent_aggregates[0];
        assert_eq!(a.role, "Arity");
        assert_eq!(a.op, "count");
        assert_eq!(a.target_role, "Part");
        assert!(a.source_fact_type_id.contains("Thing") && a.source_fact_type_id.contains("Part"),
            "source_fact_type_id should reference Thing and Part, got {:?}", a.source_fact_type_id);
        assert_eq!(a.group_key_role, "Thing");
        // The where-clause IS the source fact type; it belongs only in the
        // aggregate metadata, not in antecedent_fact_type_ids (otherwise
        // the compile path would double-count it).
        assert!(rule.antecedent_fact_type_ids.is_empty(),
            "aggregate clause must consume the whole antecedent list, got {:?}",
            rule.antecedent_fact_type_ids);
    }

    #[test]
    fn derivation_rule_without_comparison_leaves_filters_empty() {
        // Regression: a plain join rule must not accidentally produce filters.
        let input = "User(.Email) is an entity type.\nDomain(.Slug) is an entity type.\nOrganization(.Slug) is an entity type.\n## Fact Types\nUser belongs to Organization.\nDomain belongs to Organization.\nUser accesses Domain.\n## Derivation Rules\n+ User accesses Domain if User belongs to Organization and Domain belongs to that Organization.";
        let ir = parse_markdown(input).unwrap();
        let rule = &ir.derivation_rules[0];
        assert!(rule.antecedent_filters.is_empty(),
            "expected no filters on plain join rule, got {:?}", rule.antecedent_filters);
    }

    #[test]
    fn fact_type_id_from_roles_and_verb() {
        // Binary fact type: "User owns Organization"
        assert_eq!(
            fact_type_id(&["User", "Organization"], "owns"),
            "User_owns_Organization"
        );
        // Verb with spaces: "was placed by"
        assert_eq!(
            fact_type_id(&["Order", "Customer"], "was placed by"),
            "Order_was_placed_by_Customer"
        );
        // Ring constraint with subscripts
        assert_eq!(
            fact_type_id(&["Person1", "Person2"], "is parent of"),
            "Person1_is_parent_of_Person2"
        );
        // Unary: "Customer is active"
        assert_eq!(
            fact_type_id(&["Customer"], "is active"),
            "Customer_is_active"
        );
        // Multi-word nouns: "Auth Session uses Session Strategy"
        assert_eq!(
            fact_type_id(&["Auth Session", "Session Strategy"], "uses"),
            "Auth_Session_uses_Session_Strategy"
        );
    }

    #[test]
    fn strip_role_subscript() {
        assert_eq!(parse_role_token("Person1"), ("Person", "Person1"));
        assert_eq!(parse_role_token("Person2"), ("Person", "Person2"));
        assert_eq!(parse_role_token("User"), ("User", "User"));
        assert_eq!(parse_role_token("Organization"), ("Organization", "Organization"));
        // Multi-word noun with subscript
        assert_eq!(parse_role_token("Support Request1"), ("Support Request", "Support Request1"));
    }

    #[test]
    fn parse_fact_produces_schema_id() {
        let nouns = vec!["User".to_string(), "Organization".to_string()];
        let (id, ft, _mode) = parse_fact("User owns Organization.", &nouns).unwrap();
        assert_eq!(id, "User_owns_Organization");
        assert_eq!(ft.schema_id, "User_owns_Organization");
        assert_eq!(ft.reading, "User owns Organization");
        assert_eq!(ft.readings.len(), 1);
        assert_eq!(ft.readings[0].role_order, vec![0, 1]);
    }

    #[test]
    fn schema_catalog_resolves_by_noun_set() {
        let mut catalog = SchemaCatalog::new();
        catalog.register("User_owns_Organization", &["User", "Organization"], "owns", "User owns Organization");
        catalog.register("User_administers_Organization", &["User", "Organization"], "administers", "User administers Organization");
        catalog.register("Domain_belongs_to_App", &["Domain", "App"], "belongs to", "Domain belongs to App");

        // Single match by noun set
        assert_eq!(
            catalog.resolve(&["Domain", "App"], None),
            Some("Domain_belongs_to_App".to_string())
        );
        // Ambiguous noun set, disambiguate by verb
        assert_eq!(
            catalog.resolve(&["User", "Organization"], Some("owns")),
            Some("User_owns_Organization".to_string())
        );
        assert_eq!(
            catalog.resolve(&["User", "Organization"], Some("administers")),
            Some("User_administers_Organization".to_string())
        );
        // Inverse voice: catalog alone can't resolve inverse voice for ambiguous noun sets.
        // The full resolution uses word-stem matching against ir.fact_types (resolve_constraint_schema).
        // See inverse_voice_ambiguous_noun_set_resolves test for end-to-end coverage.
        // Reverse order with unique noun set still resolves
        assert_eq!(
            catalog.resolve(&["App", "Domain"], None),
            Some("Domain_belongs_to_App".to_string())
        );
        // No match
        assert_eq!(
            catalog.resolve(&["Foo", "Bar"], None),
            None
        );
    }

    #[test]
    fn inverse_reading_constraint_resolves_to_schema() {
        let input = "# test\n\
            ## Entity Types\n\
            User(.Name) is an entity type.\n\
            Organization(.Name) is an entity type.\n\
            ## Fact Types\n\
            User owns Organization.\n\
            ## Constraints\n\
            Each Organization is owned by at most one User.\n";
        let ir = parse_markdown(input).unwrap();
        // #283 — FactType cell reads. Fact type keyed by Fact Type ID.
        assert!(super::fact_type_exists(&ir.cells, "User_owns_Organization"));
        assert!(!super::fact_type_exists(&ir.cells, "User owns Organization"));
        // The constraint's spans should reference the same schema ID.
        // #283 — Constraint cell read.
        let cs = super::constraints_from_cells(&ir.cells);
        assert!(!cs.is_empty());
        let c = &cs[0];
        assert_eq!(c.spans[0].fact_type_id, "User_owns_Organization",
            "Constraint span should reference Fact Type ID, not reading text");
    }

    #[test]
    fn inverse_voice_ambiguous_noun_set_resolves() {
        let input = "# test\n\
            ## Entity Types\n\
            User(.Name) is an entity type.\n\
            Organization(.Name) is an entity type.\n\
            ## Fact Types\n\
            User owns Organization.\n\
            User administers Organization.\n\
            ## Constraints\n\
            Each Organization is owned by at most one User.\n\
            Each Organization is administered by at most one User.\n";
        let ir = parse_markdown(input).unwrap();
        // #283 — FactType cell reads.
        assert!(super::fact_type_exists(&ir.cells, "User_owns_Organization"));
        assert!(super::fact_type_exists(&ir.cells, "User_administers_Organization"));
        // "is owned by" constraint should resolve to "User_owns_Organization"
        // via word overlap: "owned" matches "owns". #283 — Constraint cell read.
        let cs = super::constraints_from_cells(&ir.cells);
        let owned_constraint = cs.iter()
            .find(|c| c.text.contains("is owned by"))
            .expect("should have 'is owned by' constraint");
        assert_eq!(owned_constraint.spans[0].fact_type_id, "User_owns_Organization",
            "Inverse voice 'is owned by' should resolve to 'owns' schema");
        // "is administered by" should resolve to "User_administers_Organization"
        let admin_constraint = cs.iter()
            .find(|c| c.text.contains("is administered by"))
            .expect("should have 'is administered by' constraint");
        assert_eq!(admin_constraint.spans[0].fact_type_id, "User_administers_Organization",
            "Inverse voice 'is administered by' should resolve to 'administers' schema");
    }

    #[test]
    fn ring_irreflexive() {
        let input = "Person(.Name) is an entity type.\nPerson is a parent of Person.\nNo Person is a parent of itself.";
        let ir = parse_markdown(input).unwrap();
        assert!(super::constraints_from_cells(&ir.cells).iter().any(|c| c.kind == "IR"), "Expected IR constraint, got: {:?}", super::constraints_from_cells(&ir.cells));
    }

    #[test]
    fn ring_asymmetric() {
        let input = "Person(.Name) is an entity type.\nPerson is a parent of Person.\nIf Person1 is a parent of Person2 then it is impossible that Person2 is a parent of Person1.";
        let ir = parse_markdown(input).unwrap();
        assert!(super::constraints_from_cells(&ir.cells).iter().any(|c| c.kind == "AS"), "Expected AS constraint, got: {:?}", super::constraints_from_cells(&ir.cells));
    }

    #[test]
    fn ring_symmetric() {
        let input = "Person(.Name) is an entity type.\nPerson is married to Person.\nIf Person1 is married to Person2 then Person2 is married to Person1.";
        let ir = parse_markdown(input).unwrap();
        assert!(super::constraints_from_cells(&ir.cells).iter().any(|c| c.kind == "SY"), "Expected SY constraint, got: {:?}", super::constraints_from_cells(&ir.cells));
    }

    #[test]
    fn ring_intransitive() {
        let input = "Person(.Name) is an entity type.\nPerson is a parent of Person.\nIf Person1 is a parent of Person2 and Person2 is a parent of Person3 then it is impossible that Person1 is a parent of Person3.";
        let ir = parse_markdown(input).unwrap();
        assert!(super::constraints_from_cells(&ir.cells).iter().any(|c| c.kind == "IT"), "Expected IT constraint, got: {:?}", super::constraints_from_cells(&ir.cells));
    }

    #[test]
    fn ring_transitive() {
        let input = "Person(.Name) is an entity type.\nPerson is an ancestor of Person.\nIf Person1 is an ancestor of Person2 and Person2 is an ancestor of Person3 then Person1 is an ancestor of Person3.";
        let ir = parse_markdown(input).unwrap();
        assert!(super::constraints_from_cells(&ir.cells).iter().any(|c| c.kind == "TR"), "Expected TR constraint, got: {:?}", super::constraints_from_cells(&ir.cells));
    }

    #[test]
    fn ring_acyclic() {
        let input = "Category(.Name) is an entity type.\nCategory contains Category.\nNo Category may cycle back to itself via one or more traversals through contains.";
        let ir = parse_markdown(input).unwrap();
        assert!(super::constraints_from_cells(&ir.cells).iter().any(|c| c.kind == "AC"), "Expected AC constraint, got: {:?}", super::constraints_from_cells(&ir.cells));
    }

    #[test]
    fn subset_constraint() {
        let input = "Person(.Name) is an entity type.\nBook(.Title) is an entity type.\nPerson authored Book.\nPerson reviewed Book.\nIf some Person authored some Book then that Person reviewed that Book.";
        let ir = parse_markdown(input).unwrap();
        assert!(super::constraints_from_cells(&ir.cells).iter().any(|c| c.kind == "SS"), "Expected SS constraint, got: {:?}", super::constraints_from_cells(&ir.cells));
    }

    #[test]
    fn equality_constraint() {
        let input = "Person(.Name) is an entity type.\nBook(.Title) is an entity type.\nPerson authored Book.\nPerson reviewed Book.\nFor each Person, that Person authored some Book if and only if that Person reviewed some Book.";
        let ir = parse_markdown(input).unwrap();
        assert!(super::constraints_from_cells(&ir.cells).iter().any(|c| c.kind == "EQ"), "Expected EQ constraint, got: {:?}", super::constraints_from_cells(&ir.cells));
    }

    #[test]
    fn exclusion_general() {
        let input = "Person(.Name) is an entity type.\nPerson is tenured.\nPerson is contracted.\nFor each Person, at most one of the following holds: that Person is tenured; that Person is contracted.";
        let ir = parse_markdown(input).unwrap();
        assert!(super::constraints_from_cells(&ir.cells).iter().any(|c| c.kind == "XC"), "Expected XC, got: {:?}", super::constraints_from_cells(&ir.cells));
    }

    #[test]
    fn exclusive_or() {
        let input = "Person(.Name) is an entity type.\nPerson is tenured.\nPerson is contracted.\nFor each Person, exactly one of the following holds: that Person is tenured; that Person is contracted.";
        let ir = parse_markdown(input).unwrap();
        assert!(super::constraints_from_cells(&ir.cells).iter().any(|c| c.kind == "XO"), "Expected XO, got: {:?}", super::constraints_from_cells(&ir.cells));
    }

    #[test]
    fn inclusive_or() {
        let input = "Lecturer(.Name) is an entity type.\nDate(.Value) is a value type.\nLecturer is contracted until Date.\nLecturer is tenured.\nEach Lecturer is contracted until some Date or is tenured.";
        let ir = parse_markdown(input).unwrap();
        assert!(super::constraints_from_cells(&ir.cells).iter().any(|c| c.kind == "OR"), "Expected OR, got: {:?}", super::constraints_from_cells(&ir.cells));
    }

    #[test]
    fn uncle_rule_subscripted_ring_join_resolves_antecedents() {
        // #197: derivation rule with 3 subscripted Person references across
        // two ring fact types â€” the "uncle" pattern from Halpin FORML Example 6.
        //
        //   Person1 is uncle of Person2 iff
        //     Person1 is brother of some Person3 and
        //     that Person3 is parent of Person2.
        //
        // The anaphoric `that Person3` uses a subscripted noun.  The join-key
        // detector must recognise "that Person3" as referring to the base noun
        // "Person" and classify the rule as Join with two resolved antecedent
        // fact-type IDs.
        let input = "\
Person(.Name) is an entity type.\n\
Person is brother of Person.\n\
Person is parent of Person.\n\
Person is uncle of Person.\n\
## Derivation Rules\n\
+ Person1 is uncle of Person2 iff Person1 is brother of some Person3 and that Person3 is parent of Person2.\n";
        let ir = parse_markdown(input).unwrap();
        let rule = ir.derivation_rules.iter()
            .find(|r| r.text.contains("uncle"))
            .expect("uncle derivation rule must be present");
        assert_eq!(
            rule.antecedent_fact_type_ids.len(), 2,
            "uncle rule must have 2 antecedents (brother + parent), got {:?}",
            rule.antecedent_fact_type_ids
        );
        assert_eq!(
            rule.kind,
            crate::types::DerivationKind::Join,
            "uncle rule with `that Person3` must be classified as Join, got {:?}",
            rule.kind
        );
        assert!(
            rule.join_on.iter().any(|k| k == "Person"),
            "join_on must include base noun 'Person', got {:?}",
            rule.join_on
        );
    }

    #[test]
    fn frequency_constraint() {
        let input = "Customer(.Name) is an entity type.\nOrder(.Id) is an entity type.\nCustomer places Order.\nEach Customer places at least 1 and at most 5 Order.";
        let ir = parse_markdown(input).unwrap();
        // #283 — Constraint cell read.
        let cs = super::constraints_from_cells(&ir.cells);
        assert!(cs.iter().any(|c| c.kind == "FC"), "Expected FC constraint, got: {:?}", cs);
        let fc = cs.iter().find(|c| c.kind == "FC").unwrap();
        assert_eq!(fc.min_occurrence, Some(1));
        assert_eq!(fc.max_occurrence, Some(5));
    }

    #[test]
    fn value_constraint() {
        let input = "Priority is a value type.\n  The possible values of Priority are 'Low', 'Medium', 'High'.\nTicket(.Id) is an entity type.\nTicket has Priority.";
        let ir = parse_markdown(input).unwrap();
        assert!(super::constraints_from_cells(&ir.cells).iter().any(|c| c.kind == "VC"), "Expected VC constraint, got: {:?}", super::constraints_from_cells(&ir.cells));
    }

    #[test]
    fn external_uniqueness() {
        let input = "Room(.Nr) is an entity type.\nBuilding(.Code) is an entity type.\nRoomNr is a value type.\nRoom is in Building.\nRoom has RoomNr.\nFor each Building and RoomNr, at most one Room is in that Building and has that RoomNr.";
        let ir = parse_markdown(input).unwrap();
        assert!(super::constraints_from_cells(&ir.cells).iter().any(|c| c.kind == "UC"), "Expected UC (external), got: {:?}", super::constraints_from_cells(&ir.cells));
    }

    #[test]
    fn context_pattern() {
        let input = "Room(.Nr) is an entity type.\nBuilding(.Code) is an entity type.\nRoomNr is a value type.\nRoom is in Building.\nRoom has RoomNr.\nContext: Room is in Building; Room has RoomNr. In this context, each Building, RoomNr combination is associated with at most one Room.";
        let ir = parse_markdown(input).unwrap();
        assert!(super::constraints_from_cells(&ir.cells).iter().any(|c| c.kind == "UC"), "Expected UC (context), got: {:?}", super::constraints_from_cells(&ir.cells));
    }

    #[test]
    fn real_domain_produces_schema_ids() {
        let input = "\
# Auth

## Entity Types
Auth Session(.id) is an entity type.
Customer(.Name) is an entity type.
Session Strategy is a value type.

## Fact Types
Auth Session is for Customer.
  Each Auth Session is for exactly one Customer.
Auth Session uses Session Strategy.
  Each Auth Session uses exactly one Session Strategy.
";
        let ir = parse_markdown(input).unwrap();
        // #283 — FactType cell reads. Keys should be underscore format, not reading format.
        assert!(super::fact_type_ids(&ir.cells).iter().all(|key| !key.contains(' ')),
            "Fact type keys should not contain spaces");
        assert!(super::fact_type_exists(&ir.cells, "Auth_Session_is_for_Customer"));
        assert!(super::fact_type_exists(&ir.cells, "Auth_Session_uses_Session_Strategy"));
        // Constraints should reference schema IDs. #283 — Constraint cell read.
        let cs = super::constraints_from_cells(&ir.cells);
        assert!(
            cs.iter().flat_map(|c| c.spans.iter())
                .all(|span| !span.fact_type_id.contains(' ')),
            "Constraint spans should not contain spaces"
        );
    }

    #[test]
    fn span_naming() {
        let input = "Customer(.Email) is an entity type.\nSupport Request(.Id) is an entity type.\nCustomer submits Support Request.\nIf some Support Request has some Email Address and some Customer is identified by that Email Address then that Customer submits that Support Request.\nThis span with Customer, Support Request provides the preferred identification scheme for Customer Submission Match.";
        let ir = parse_markdown(input).unwrap();
        // #283 — span metadata lives in the NamedSpan cell, not a typed map.
        let named_spans = ir.cells.get("NamedSpan")
            .expect("NamedSpan cell must exist");
        let fact = named_spans.iter()
            .find(|f| crate::ast::binding(f, "name") == Some("Customer Submission Match"))
            .expect("NamedSpan fact for 'Customer Submission Match' must exist");
        assert_eq!(crate::ast::binding(fact, "role0"), Some("Customer"));
        assert_eq!(crate::ast::binding(fact, "role1"), Some("Support Request"));
    }

    #[test]
    fn autofill_declaration() {
        let input = "Customer(.Email) is an entity type.\nSupport Request(.Id) is an entity type.\nEmail Address is a value type.\nCustomer submits Support Request.\nCustomer is identified by Email Address.\nSupport Request has Email Address.\nIf some Support Request has some Email Address and some Customer is identified by that Email Address then that Customer submits that Support Request.\nThis span with Customer, Support Request provides the preferred identification scheme for Customer Submission Match.\nConstraint Span 'Customer Submission Match' autofills from superset.";
        let ir = parse_markdown(input).unwrap();
        // #283 — autofill spans live in the AutofillSpan cell.
        let autofill = ir.cells.get("AutofillSpan")
            .expect("AutofillSpan cell must exist");
        assert!(autofill.iter()
            .any(|f| crate::ast::binding(f, "name") == Some("Customer Submission Match")),
            "AutofillSpan cell must carry 'Customer Submission Match'");
        // The SS constraint targeting Customer, Support Request should have autofill enabled.
        // #283 — Constraint cell read.
        let cs = super::constraints_from_cells(&ir.cells);
        let ss = cs.iter().find(|c| c.kind == "SS").expect("Should have SS constraint");
        assert_eq!(ss.spans.iter().any(|s| s.subset_autofill == Some(true)), true, "SS constraint should have autofill enabled");
    }

    #[test]
    fn subset_autofill_derives_facts() {
        let input = "\
Person(.Name) is an entity type.
Department(.Code) is an entity type.
Person works in Department.
Person heads Department.
If some Person heads some Department then that Person works in that Department.
This span with Person, Department provides the preferred identification scheme for Department Leadership.
Constraint Span 'Department Leadership' autofills from superset.";
        let ir = parse_markdown(input).unwrap();
        // #283 — Constraint cell read.
        let cs = super::constraints_from_cells(&ir.cells);
        let ss = cs.iter().find(|c| c.kind == "SS").expect("SS constraint");
        assert!(ss.spans.iter().any(|s| s.subset_autofill == Some(true)), "autofill should be set");
    }

    // -- Deontic constraint fixes (2026-04-03) -------------------------

    #[test]
    fn deontic_forbidden_extracts_entity() {
        let input = "\
Support Response(.id) is an entity type.
Dash is a value type.
  The possible values of Dash are '\u{2014}', '\u{2013}', '--'.
## Fact Types
Support Response uses Dash.
## Deontic Constraints
It is forbidden that Support Response uses Dash.";
        let ir = parse_markdown(input).unwrap();
        // #283 — Constraint cell read.
        let cs = super::constraints_from_cells(&ir.cells);
        let c = cs.iter()
            .find(|c| c.text.contains("forbidden") && c.text.contains("Dash"))
            .expect("Should have forbidden Dash constraint");
        assert_eq!(c.entity, Some("Support Response".into()),
            "Deontic constraint should extract entity from text");
    }

    #[test]
    fn deontic_constraint_has_nonempty_span() {
        let input = "\
Support Response(.id) is an entity type.
Dash is a value type.
  The possible values of Dash are '\u{2014}', '\u{2013}', '--'.
## Fact Types
Support Response uses Dash.
## Deontic Constraints
It is forbidden that Support Response uses Dash.";
        let ir = parse_markdown(input).unwrap();
        // #283 — Constraint cell read.
        let cs = super::constraints_from_cells(&ir.cells);
        let c = cs.iter()
            .find(|c| c.text.contains("forbidden") && c.text.contains("Dash"))
            .unwrap();
        assert!(!c.spans.is_empty(), "Deontic constraint should have at least one span");
        assert!(!c.spans[0].fact_type_id.is_empty(),
            "Span fact_type_id should be resolved, got empty");
        assert_eq!(c.spans[0].fact_type_id, "Support_Response_uses_Dash",
            "Span should reference the correct fact type");
    }

    #[test]
    fn deontic_constraint_has_unique_id() {
        let input = "\
Support Response(.id) is an entity type.
Dash is a value type.
  The possible values of Dash are '\u{2014}', '\u{2013}', '--'.
Markdown Syntax is a value type.
  The possible values of Markdown Syntax are '**', '##'.
## Fact Types
Support Response uses Dash.
Support Response contains Markdown Syntax.
## Deontic Constraints
It is forbidden that Support Response uses Dash.
It is forbidden that Support Response contains Markdown Syntax.";
        let ir = parse_markdown(input).unwrap();
        // #283 — Constraint cell read.
        let cs = super::constraints_from_cells(&ir.cells);
        let deontics: Vec<&ConstraintDef> = cs.iter()
            .filter(|c| c.modality == "deontic")
            .collect();
        assert!(deontics.len() >= 2, "Should have at least 2 deontic constraints");
        // IDs should be unique (not all empty)
        let ids: hashbrown::HashSet<&str> = deontics.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids.len(), deontics.len(),
            "Each deontic constraint should have a unique ID");
        assert!(!ids.contains(""), "No constraint should have an empty ID");
    }

    #[test]
    fn deontic_obligatory_extracts_entity() {
        let input = "\
Support Response(.id) is an entity type.
Pricing Model is a value type.
  The possible values of Pricing Model are 'subscription', 'metered'.
## Fact Types
Support Response conforms to Pricing Model.
## Deontic Constraints
It is obligatory that each Support Response conforms to Pricing Model.";
        let ir = parse_markdown(input).unwrap();
        // #283 — Constraint cell read.
        let cs = super::constraints_from_cells(&ir.cells);
        let c = cs.iter()
            .find(|c| c.text.contains("obligatory") && c.text.contains("Pricing Model"))
            .expect("Should have obligatory Pricing Model constraint");
        assert_eq!(c.entity, Some("Support Response".into()));
        assert_eq!(c.modality, "deontic");
        assert_eq!(c.deontic_operator, Some("obligatory".into()));
    }

    /// Helper: compile IR to defs, then evaluate constraints via defs path.
    fn eval_deontic_defs(ir: &ParseCtx, text: &str) -> Vec<crate::types::Violation> {
        let state = ctx_to_state(ir);
        let defs = crate::compile::compile_to_defs_state(&state);
        let empty_state = crate::ast::Object::phi();
        let def_obj = crate::ast::defs_to_state(&defs, &empty_state);
        let ctx_obj = crate::ast::encode_eval_context_state(text, None, &empty_state);
        defs.iter()
            .filter(|(n, _)| n.starts_with("constraint:"))
            .flat_map(|(name, func)| {
                let result = crate::ast::apply(func, &ctx_obj, &def_obj);
                let is_deontic = name.contains("obligatory") || name.contains("forbidden");
                crate::ast::decode_violations(&result).into_iter().map(move |mut v| {
                    v.alethic = !is_deontic;
                    v
                })
            })
            .collect()
    }

    #[test]
    fn deontic_forbidden_evaluates_enum_match() {
        let input = "\
Support Response(.id) is an entity type.
Dash is a value type.
  The possible values of Dash are '\u{2014}', '\u{2013}', '--'.
## Fact Types
Support Response uses Dash.
## Deontic Constraints
It is forbidden that Support Response uses Dash.";
        let ir = parse_markdown(input).unwrap();
        let violations = eval_deontic_defs(&ir, "Hi -- here is your answer");
        assert!(!violations.is_empty(),
            "Response containing '--' should violate the forbidden Dash constraint");
        assert!(violations.iter().any(|v| v.constraint_text.contains("Dash")),
            "Violation should reference the Dash constraint");
    }

    #[test]
    fn deontic_forbidden_clean_response_no_violations() {
        let input = "\
Support Response(.id) is an entity type.
Dash is a value type.
  The possible values of Dash are '\u{2014}', '\u{2013}', '--'.
## Fact Types
Support Response uses Dash.
## Deontic Constraints
It is forbidden that Support Response uses Dash.";
        let ir = parse_markdown(input).unwrap();
        let violations = eval_deontic_defs(&ir, "Hi, here is your answer with no dashes at all");
        let dash_violations: Vec<_> = violations.iter()
            .filter(|v| v.constraint_text.contains("Dash"))
            .collect();
        assert!(dash_violations.is_empty(),
            "Clean response should not trigger Dash violation");
    }

    #[test]
    fn deontic_multiple_forbidden_enum_constraints() {
        let input = "\
Support Response(.id) is an entity type.
Dash is a value type.
  The possible values of Dash are '\u{2014}', '\u{2013}', '--'.
Markdown Syntax is a value type.
  The possible values of Markdown Syntax are '**', '##', '###', '```'.
## Fact Types
Support Response uses Dash.
Support Response contains Markdown Syntax.
## Deontic Constraints
It is forbidden that Support Response uses Dash.
It is forbidden that Support Response contains Markdown Syntax.";
        let ir = parse_markdown(input).unwrap();
        let violations = eval_deontic_defs(&ir, "## Heading\n\nHere is info -- with **bold** text");
        assert!(violations.iter().any(|v| v.constraint_text.contains("Dash")),
            "Should catch dash violation");
        assert!(violations.iter().any(|v| v.constraint_text.contains("Markdown")),
            "Should catch markdown violation");
    }

    // =====================================================================
    // Task #25 -- is_forbidden_url SSRF defense coverage.
    // =====================================================================

    // --- 1. Loopback IPv4 ---
    #[test]
    fn forbidden_loopback_ipv4_basic() {
        assert!(is_forbidden_url("http://127.0.0.1"));
        assert!(is_forbidden_url("https://127.0.0.1"));
    }

    #[test]
    fn forbidden_loopback_ipv4_alt_octets() {
        assert!(is_forbidden_url("http://127.1.2.3"));
        assert!(is_forbidden_url("http://127.255.255.254"));
    }

    #[test]
    fn forbidden_loopback_ipv4_with_port() {
        assert!(is_forbidden_url("http://127.0.0.1:8080"));
        assert!(is_forbidden_url("https://127.0.0.1:443/admin"));
    }

    #[test]
    fn forbidden_loopback_ipv4_with_path() {
        assert!(is_forbidden_url("http://127.0.0.1/admin/debug?x=1"));
        assert!(is_forbidden_url("https://127.0.0.1/secret#frag"));
    }

    // --- 2. Loopback IPv6 ---
    #[test]
    fn forbidden_loopback_ipv6_bare() {
        // `http://::1` parses with an empty host (the `::` splits host_end
        // at the first ':'), and the empty-host branch returns forbidden.
        assert!(is_forbidden_url("http://::1"));
    }

    // GAP: bracketed IPv6 literals are NOT detected by the current parser.
    // `host_end` splits on the first `:` which sits inside the brackets,
    // so `host` becomes just "[" and the bracket-strip path is never taken.
    // Documented here as an ignored regression so the gap is visible.
    #[test]
fn forbidden_loopback_ipv6_bracketed_with_port() {
        assert!(is_forbidden_url("http://[::1]:8080"));
        assert!(is_forbidden_url("https://[::1]:443/api"));
    }

    // --- 3. Link-local IPv4 (incl. AWS metadata) ---
    #[test]
    fn forbidden_link_local_ipv4() {
        assert!(is_forbidden_url("http://169.254.0.1"));
        assert!(is_forbidden_url("http://169.254.255.255"));
    }

    #[test]
    fn forbidden_aws_metadata_endpoint() {
        assert!(is_forbidden_url("http://169.254.169.254/latest/meta-data/"));
        assert!(is_forbidden_url("https://169.254.169.254"));
    }

    // --- 4. Private IPv4 ranges ---
    #[test]
    fn forbidden_private_10_range() {
        assert!(is_forbidden_url("http://10.0.0.1"));
        assert!(is_forbidden_url("https://10.255.255.254:8000"));
    }

    #[test]
    fn forbidden_private_172_range_lower_boundary() {
        assert!(is_forbidden_url("http://172.16.0.1"));
    }

    #[test]
    fn forbidden_private_172_range_upper_boundary() {
        assert!(is_forbidden_url("http://172.31.255.255"));
    }

    #[test]
    fn forbidden_private_192_168_range() {
        assert!(is_forbidden_url("http://192.168.1.1"));
        assert!(is_forbidden_url("https://192.168.0.1:9200/_cluster"));
    }

    // --- 5. IPv6 link-local ---
    #[test]
    fn forbidden_ipv6_link_local_fe80() {
        assert!(is_forbidden_url("http://fe80::1"));
    }

    #[test]
    fn forbidden_ipv6_link_local_febf() {
        assert!(is_forbidden_url("http://febf::ffff"));
    }

    // GAP: bracketed IPv6 literals are never detected (see note on
    // forbidden_loopback_ipv6_bracketed_with_port). Same underlying bug.
    #[test]
fn forbidden_ipv6_link_local_bracketed() {
        assert!(is_forbidden_url("http://[fe80::1]:8080"));
        assert!(is_forbidden_url("https://[febf::dead:beef]/x"));
    }

    // --- 6. IPv6 Unique Local (ULA) ---
    //
    // GAP: the ULA check requires host_bare.contains(':'), but host_end
    // splits on the first ':' so bare-form `fc00::1` arrives here as
    // "fc00" with no colon. The check never fires. Documented as ignored
    // regressions.
    #[test]
fn forbidden_ipv6_ula_fc00() {
        assert!(is_forbidden_url("http://fc00::1"));
        assert!(is_forbidden_url("http://[fc00::1]:8080"));
    }

    #[test]
fn forbidden_ipv6_ula_fd() {
        assert!(is_forbidden_url("http://fd12::abcd"));
        assert!(is_forbidden_url("https://[fd12:3456::1]/api"));
    }

    // --- 7. file:// scheme ---
    #[test]
    fn forbidden_file_scheme_absolute() {
        assert!(is_forbidden_url("file:///etc/passwd"));
    }

    #[test]
    fn forbidden_file_scheme_with_host() {
        assert!(is_forbidden_url("file://localhost/etc/hosts"));
    }

    #[test]
    fn forbidden_file_scheme_case_insensitive() {
        assert!(is_forbidden_url("FILE:///etc/passwd"));
        assert!(is_forbidden_url("File:///C:/Windows/System32"));
    }

    // --- 8. Internal DNS suffixes ---
    #[test]
    fn forbidden_local_suffix() {
        assert!(is_forbidden_url("http://printer.local"));
        assert!(is_forbidden_url("https://service.local/api"));
    }

    #[test]
    fn forbidden_internal_suffix() {
        assert!(is_forbidden_url("http://api.internal"));
        assert!(is_forbidden_url("https://vault.corp.internal/secret"));
    }

    #[test]
    fn forbidden_localhost_suffix() {
        assert!(is_forbidden_url("http://dev.localhost"));
        assert!(is_forbidden_url("https://app.localhost:3000"));
    }

    // --- 9. Bare localhost ---
    #[test]
    fn forbidden_bare_localhost() {
        assert!(is_forbidden_url("http://localhost"));
        assert!(is_forbidden_url("https://localhost:8080"));
        assert!(is_forbidden_url("http://localhost/api/admin"));
    }

    #[test]
    fn forbidden_zero_host() {
        assert!(is_forbidden_url("http://0.0.0.0"));
        assert!(is_forbidden_url("http://0.0.0.0:8080/"));
    }

    // --- 10. PUBLIC URLs must NOT be rejected ---
    #[test]
    fn allowed_public_https_hostname() {
        assert!(!is_forbidden_url("https://example.com"));
        assert!(!is_forbidden_url("https://example.com/path?x=1"));
    }

    #[test]
    fn allowed_public_ipv4_dns_resolver() {
        assert!(!is_forbidden_url("http://8.8.8.8"));
        assert!(!is_forbidden_url("https://8.8.4.4:443/dns-query"));
    }

    #[test]
    fn allowed_public_api_endpoint() {
        assert!(!is_forbidden_url("https://api.stripe.com/v1/charges"));
        assert!(!is_forbidden_url("https://api.github.com/repos/anthropic/foo"));
    }

    #[test]
    fn allowed_public_172_outside_private_range() {
        // 172.15.x.x and 172.32.x.x are PUBLIC (private = 172.16-31).
        assert!(!is_forbidden_url("http://172.15.0.1"));
        assert!(!is_forbidden_url("http://172.32.0.1"));
    }

    #[test]
    fn allowed_169_non_link_local() {
        // 169.1.0.1 is not link-local (only 169.254.*.* is).
        assert!(!is_forbidden_url("http://169.1.0.1"));
    }

    // --- 11. Edge cases ---
    #[test]
    fn forbidden_empty_string() {
        // Empty string has no scheme; is_forbidden_url falls through -> false.
        // This is documented: non-http schemes are not rejected. The guard is
        // scoped to federated HTTP URLs only. Ensure at least no panic.
        let _ = is_forbidden_url("");
    }

    // --- 12. Metamodel invariants (readings/core.md) ---
    //
    // These tests assert that the authoritative metamodel file exposes the
    // concepts the engine depends on. The canonical ORM 2 derivation markers
    // (*, **, +) attach a Derivation Mode onto a Fact Type; the metamodel
    // must therefore declare the Derivation Mode value type with its three
    // enum values AND a `Fact Type has Derivation Mode` binary fact type.
    //
    // Cites: Halpin, ORM 2 (ORM2.pdf p. 8) â€” iff-rule for full derivation,
    // if-rule for partial; graphical markers * / ** / + for fully-derived /
    // derived-and-stored / semi-derived respectively.

    #[test]
    fn metamodel_declares_derivation_mode_value_type() {
        let core_md = include_str!("../../../readings/core.md");
        let domain = parse_markdown(core_md)
            .expect("metamodel readings/core.md must parse");

        // #283 — Noun cell read.
        let obj_type = super::noun_object_type(&domain.cells, "Derivation Mode")
            .expect("core.md must declare 'Derivation Mode' as a noun");
        assert_eq!(obj_type, "value",
            "'Derivation Mode' must be a value type");

        // #283 — enum values on the EnumValues cell.
        let vals = super::enum_values_for_noun(&domain.cells, "Derivation Mode");
        assert!(!vals.is_empty(), "'Derivation Mode' must have declared enum values");
        assert!(vals.iter().any(|v| v == "fully-derived"),
            "Derivation Mode enum must include 'fully-derived'; got: {:?}", vals);
        assert!(vals.iter().any(|v| v == "derived-and-stored"),
            "Derivation Mode enum must include 'derived-and-stored'; got: {:?}", vals);
        assert!(vals.iter().any(|v| v == "semi-derived"),
            "Derivation Mode enum must include 'semi-derived'; got: {:?}", vals);
    }

    /// #279 — NORMA's structural decomposition primitives for derivation
    /// rule bodies. `JoinPath` + `Join` + `RoleSequence` + `RoleProjection`
    /// map 1:1 onto paper §4 Table 1 FFP primitives (Composition,
    /// Construction, Selector, Condition). Every one of the 13 clause
    /// shapes in `_reports/engine-resolver-gaps.md` decomposes into some
    /// JoinPath form — that's what makes the meta-circular parser in
    /// #280 possible.
    #[test]
    fn metamodel_declares_norma_join_path_decomposition() {
        let core_md = include_str!("../../../readings/core.md");
        let domain = parse_markdown(core_md)
            .expect("metamodel readings/core.md must parse");
        for entity in ["Join Path", "Join", "Role Sequence", "Role Projection", "Join Type"] {
            // #283 — Noun cell read.
            let obj_type = super::noun_object_type(&domain.cells, entity)
                .unwrap_or_else(|| panic!(
                    "core.md must declare NORMA entity '{}' for the meta-circular parser; got nouns: {:?}",
                    entity, super::noun_names(&domain.cells)
                ));
            assert_eq!(obj_type, "entity",
                "'{}' must be an entity type (NORMA ObjectType subtype)", entity);
        }
    }

    /// #279 — NORMA's value-domain primitives. `Bound` + `Value Range`
    /// + `Facet` carry per-value-type constraints as first-class facts
    /// instead of the flat `Minimum`/`Maximum`/... fields we had before.
    /// `Value` is the entity form of a literal instance.
    #[test]
    fn metamodel_declares_norma_value_domain_entities() {
        let core_md = include_str!("../../../readings/core.md");
        let domain = parse_markdown(core_md)
            .expect("metamodel readings/core.md must parse");
        for entity in ["Bound", "Value Range", "Facet", "Value", "Unit", "Dimension", "Textual Constraint"] {
            // #283 — Noun cell read.
            let obj_type = super::noun_object_type(&domain.cells, entity)
                .unwrap_or_else(|| panic!(
                    "core.md must declare NORMA entity '{}'; got nouns: {:?}",
                    entity, super::noun_names(&domain.cells)
                ));
            assert_eq!(obj_type, "entity",
                "'{}' must be an entity type", entity);
        }
    }

    /// #279 — NORMA value types for the new decomposition + value-domain
    /// entities. Enum value types get their enum listed; plain value
    /// types just need `is a value type` declared.
    #[test]
    fn metamodel_declares_norma_value_types() {
        let core_md = include_str!("../../../readings/core.md");
        let domain = parse_markdown(core_md)
            .expect("metamodel readings/core.md must parse");
        // Plain value types — no enum required
        for value_type in ["Regex Pattern", "Lexical Value", "Alias", "Length", "Binary Precision", "Digit Count"] {
            // #283 — Noun cell read.
            let obj_type = super::noun_object_type(&domain.cells, value_type)
                .unwrap_or_else(|| panic!(
                    "core.md must declare NORMA value type '{}'; got: {:?}",
                    value_type, super::noun_names(&domain.cells)
                ));
            assert_eq!(obj_type, "value",
                "'{}' must be a value type", value_type);
        }
        // Clusivity: enum of inclusive / exclusive — via EnumValues cell (#283)
        let clusivity = super::enum_values_for_noun(&domain.cells, "Clusivity");
        assert!(!clusivity.is_empty(), "core.md must declare 'Clusivity' enum");
        assert!(clusivity.iter().any(|v| v == "inclusive"));
        assert!(clusivity.iter().any(|v| v == "exclusive"));
        // Derivation Storage Type: NORMA's {stored, derived, derived-and-stored}
        let storage = super::enum_values_for_noun(&domain.cells, "Derivation Storage Type");
        assert!(!storage.is_empty(), "core.md must declare 'Derivation Storage Type' enum");
        assert!(storage.iter().any(|v| v == "stored"));
        assert!(storage.iter().any(|v| v == "derived"));
    }

    // --- Derivation mode markers on fact type readings ---
    //
    // Halpin ORM 2 attaches graphical markers to derived fact types: `*`
    // for fully derived, `**` for derived and stored, `+` for semi-derived.
    // In the FORML 2 textual form, the marker follows the reading, separated
    // by a space, before the sentence-terminating period. The parser
    // recognizes the marker and stores the corresponding Derivation Mode on
    // the FactTypeDef.

    fn mode_for(domain: &ParseCtx, reading: &str) -> Option<String> {
        // #283 — InstanceFact cell read.
        domain.cells.get("InstanceFact")?
            .iter()
            .find(|f| crate::ast::binding(f, "subjectNoun") == Some("Fact Type")
                   && crate::ast::binding(f, "subjectValue") == Some(reading)
                   && crate::ast::binding(f, "fieldName") == Some("Derivation Mode"))
            .and_then(|f| crate::ast::binding(f, "objectValue").map(String::from))
    }

    #[test]
    fn double_star_marker_attaches_derived_and_stored_mode() {
        let input = "\
Order(.Order Id) is an entity type.
Line Item(.id) is an entity type.
Amount is a value type.
Total is a value type.
Line Item has Amount.
  Each Line Item has at most one Amount.
Order has Total **.
";
        let domain = parse_markdown(input).expect("parse");
        assert_eq!(mode_for(&domain, "Order has Total").as_deref(), Some("derived-and-stored"),
            "`**` marker must attach Derivation Mode 'derived-and-stored'");
    }

    #[test]
    fn plus_marker_attaches_semi_derived_mode() {
        let input = "\
Person(.Name) is an entity type.
Grandparent is a value type.
Person has Grandparent +.
";
        let domain = parse_markdown(input).expect("parse");
        assert_eq!(mode_for(&domain, "Person has Grandparent").as_deref(), Some("semi-derived"),
            "`+` marker must attach Derivation Mode 'semi-derived'");
    }

    #[test]
    fn derivation_rule_with_star_prefix_is_parsed() {
        let input = "\
Customer(.Name) is an entity type.
First Name is a value type.
Last Name is a value type.
Full Name is a value type.
Customer has First Name.
Customer has Last Name.
Customer has Full Name *.

## Derivation Rules
* Customer has Full Name iff Customer has First Name and Customer has Last Name.
";
        let domain = parse_markdown(input).expect("parse");
        let has_rule = domain.derivation_rules.iter()
            .any(|r| r.text.contains("Customer has Full Name") && r.text.contains(" iff "));
        assert!(has_rule,
            "rule with `*` prefix must be captured (prefix is stripped, body parsed normally); \
             derivation_rules = {:?}", domain.derivation_rules);
        // Prefix must have been stripped from the stored rule text.
        assert!(!domain.derivation_rules.iter().any(|r| r.text.starts_with("* ")),
            "the leading `*` marker must be stripped from the rule text");
    }

    #[test]
    fn derivation_rule_with_double_star_prefix_is_parsed() {
        let input = "\
Order(.Order Id) is an entity type.
Line Item(.id) is an entity type.
Amount is a value type.
Total is a value type.
Order has Total **.

## Derivation Rules
** Order has Total iff Order has Line Item and that Line Item has Amount.
";
        let domain = parse_markdown(input).expect("parse");
        let has_rule = domain.derivation_rules.iter()
            .any(|r| r.text.starts_with("Order has Total") && r.text.contains(" iff "));
        assert!(has_rule,
            "`**` prefix must be stripped; derivation_rules = {:?}", domain.derivation_rules);
    }

    #[test]
    fn derivation_rule_with_plus_prefix_is_parsed() {
        let input = "\
Person(.Name) is an entity type.

## Derivation Rules
+ Person is Grandparent if Person is parent of some Person that is parent of some Person.
";
        let domain = parse_markdown(input).expect("parse");
        let has_rule = domain.derivation_rules.iter()
            .any(|r| r.text.contains("Grandparent") && r.text.contains(" if "));
        assert!(has_rule,
            "`+` prefix must be stripped; derivation_rules = {:?}", domain.derivation_rules);
    }

    #[test]
    fn no_marker_does_not_attach_derivation_mode() {
        let input = "\
Customer(.Name) is an entity type.
Email is a value type.
Customer has Email.
";
        let domain = parse_markdown(input).expect("parse");
        assert!(mode_for(&domain, "Customer has Email").is_none(),
            "a reading without a marker must not emit a Derivation Mode fact; \
             instance_facts = {:?}", domain.cells.get("InstanceFact"));
    }

    #[test]
    fn star_marker_attaches_fully_derived_mode() {
        let input = "\
Customer(.Name) is an entity type.
First Name is a value type.
Last Name is a value type.
Full Name is a value type.
Customer has First Name.
  Each Customer has at most one First Name.
Customer has Last Name.
  Each Customer has at most one Last Name.
Customer has Full Name *.
";
        let domain = parse_markdown(input).expect("parse");
        // #283 — FactType cell read.
        let ft_readings: Vec<String> = super::fact_type_ids(&domain.cells).iter()
            .filter_map(|id| super::fact_type_reading(&domain.cells, id))
            .collect();
        assert!(
            ft_readings.iter().any(|r| r == "Customer has Full Name"),
            "'Customer has Full Name' fact type must be present"
        );

        // #283 — InstanceFact cell read.
        let instance_cell = domain.cells.get("InstanceFact");
        let mode_fact = instance_cell.and_then(|facts| facts.iter()
            .find(|f| crate::ast::binding(f, "subjectNoun") == Some("Fact Type")
                   && crate::ast::binding(f, "subjectValue") == Some("Customer has Full Name")
                   && crate::ast::binding(f, "fieldName") == Some("Derivation Mode")));
        assert!(mode_fact.is_some(),
            "`*` marker must emit a 'Fact Type has Derivation Mode' instance fact; \
             instance_facts = {:?}", instance_cell);
        assert_eq!(crate::ast::binding(mode_fact.unwrap(), "objectValue"), Some("fully-derived"));
    }

    #[test]
    fn metamodel_declares_fact_type_has_derivation_mode() {
        let core_md = include_str!("../../../readings/core.md");
        let domain = parse_markdown(core_md)
            .expect("metamodel readings/core.md must parse");

        // #283 — FactType cell reads.
        let ft_readings: Vec<String> = super::fact_type_ids(&domain.cells).iter()
            .filter_map(|id| super::fact_type_reading(&domain.cells, id))
            .collect();
        let ft_exists = ft_readings.iter().any(|r| r == "Fact Type has Derivation Mode");
        assert!(ft_exists,
            "core.md must declare 'Fact Type has Derivation Mode.' so the parser \
             can emit a Fact Type's derivation modality when the */**/+ marker \
             is applied. Got fact type readings: {:?}", ft_readings);
    }

    #[test]
    fn forbidden_empty_host_in_http_url() {
        // http:// with no host -> empty host -> treated as forbidden.
        assert!(is_forbidden_url("http://"));
        assert!(is_forbidden_url("https://"));
    }

    #[test]
    fn forbidden_malformed_url_no_scheme() {
        // No http(s) scheme -> not rejected (non-http schemes fall through).
        assert!(!is_forbidden_url("not a url"));
        assert!(!is_forbidden_url("garbage://////"));
    }

    #[test]
    fn forbidden_url_with_userinfo_loopback() {
        // Userinfo must be stripped before host check.
        assert!(is_forbidden_url("http://user:pass@127.0.0.1"));
        assert!(is_forbidden_url("http://admin@localhost:8080/"));
    }

    #[test]
    fn forbidden_url_with_userinfo_public_allowed() {
        // Userinfo stripped and real host is public -> allowed.
        assert!(!is_forbidden_url("http://user:pass@example.com"));
    }

    #[test]
    fn forbidden_url_trims_whitespace() {
        assert!(is_forbidden_url("  http://127.0.0.1  "));
        assert!(is_forbidden_url("\thttps://localhost\n"));
    }

    #[test]
    fn forbidden_url_is_case_insensitive_host() {
        assert!(is_forbidden_url("http://LOCALHOST"));
        assert!(is_forbidden_url("http://Printer.Local"));
    }

    // =====================================================================
    // Task #23 -- Metamodel namespace parser guard coverage.
    // =====================================================================
    //
    // The parser-level guard lives in `parse_markdown_with_context`. It
    // rejects user domains that redeclare a reserved metamodel noun when
    // that noun is already present in `existing_nouns` (meaning the
    // metamodel bootstrap has populated it). The bootstrap case (no
    // pre-existing nouns) is allowed to declare them exactly once.

    fn metamodel_nouns_map() -> HashMap<String, NounDef> {
        let mut m = HashMap::new();
        for n in METAMODEL_NOUNS {
            m.insert((*n).to_string(), NounDef {
                object_type: "entity".into(),
                world_assumption: WorldAssumption::default(),
            });
        }
        m
    }

    #[test]
    fn metamodel_guard_rejects_noun_redeclaration() {
        let existing = metamodel_nouns_map();
        let input = "# UserDomain\nNoun(.Name) is an entity type.";
        let err = parse_markdown_with_nouns(input, &existing).unwrap_err();
        assert!(err.contains("metamodel noun 'Noun' cannot be redeclared"),
            "expected rejection message for 'Noun', got: {}", err);
    }

    #[test]
    fn metamodel_guard_rejects_constraint_redeclaration() {
        let existing = metamodel_nouns_map();
        let input = "# UserDomain\nConstraint(.Id) is an entity type.";
        let err = parse_markdown_with_nouns(input, &existing).unwrap_err();
        assert!(err.contains("metamodel noun 'Constraint' cannot be redeclared"),
            "expected rejection message for 'Constraint', got: {}", err);
    }

    #[test]
    fn metamodel_guard_accepts_non_reserved_names() {
        let existing = metamodel_nouns_map();
        let input = "# Sales\n\
                     Order(.Id) is an entity type.\n\
                     Customer(.Name) is an entity type.";
        let ir = parse_markdown_with_nouns(input, &existing)
            .expect("non-reserved user domain should parse when metamodel is already present");
        // #283 — nouns live in the Noun cell.
        assert!(super::noun_exists(&ir.cells, "Order"));
        assert!(super::noun_exists(&ir.cells, "Customer"));
        // Existing metamodel nouns remain visible in the merged IR.
        assert!(super::noun_exists(&ir.cells, "Noun"));
    }

    #[test]
    fn metamodel_guard_bootstrap_first_compile_succeeds() {
        // Bootstrap case: `existing_nouns` is empty, so the metamodel itself
        // is being compiled for the first time. Redeclaration guard must NOT
        // fire; the parse must succeed and populate the reserved nouns.
        let empty: HashMap<String, NounDef> = HashMap::new();
        let input = "# Metamodel\n\
                     Noun(.Name) is an entity type.\n\
                     Constraint(.Id) is an entity type.\n\
                     Role(.Id) is an entity type.";
        let ir = parse_markdown_with_nouns(input, &empty)
            .expect("bootstrap compile of metamodel nouns must succeed");
        // #283 — nouns live in the Noun cell.
        assert!(super::noun_exists(&ir.cells, "Noun"));
        assert!(super::noun_exists(&ir.cells, "Constraint"));
        assert!(super::noun_exists(&ir.cells, "Role"));
    }

    #[test]
    fn metamodel_guard_allows_user_domain_not_touching_reserved_before_bootstrap() {
        // Before the bootstrap has run, `existing_nouns` is empty. A user
        // domain that only declares its own names must parse fine.
        let empty: HashMap<String, NounDef> = HashMap::new();
        let input = "# Sales\nOrder(.Id) is an entity type.";
        let ir = parse_markdown_with_nouns(input, &empty).unwrap();
        // #283 — Noun cell read.
        assert!(super::noun_exists(&ir.cells, "Order"));
    }

    // One test per reserved metamodel noun. Each verifies that redeclaring
    // that specific noun is rejected by the parser guard.

    fn assert_reserved_rejected(noun: &str, decl: &str) {
        let existing = metamodel_nouns_map();
        let input = format!("# UserDomain\n{}", decl);
        let err = parse_markdown_with_nouns(&input, &existing).unwrap_err();
        let needle = format!("metamodel noun '{}' cannot be redeclared", noun);
        assert!(err.contains(&needle),
            "expected rejection message '{}', got: {}", needle, err);
    }

    #[test]
    fn metamodel_guard_rejects_reserved_noun() {
        assert_reserved_rejected("Noun", "Noun(.Name) is an entity type.");
    }

    #[test]
    fn metamodel_guard_rejects_reserved_fact_type() {
        assert_reserved_rejected("Fact Type", "Fact Type(.Id) is an entity type.");
    }

    #[test]
    fn metamodel_guard_rejects_reserved_role() {
        assert_reserved_rejected("Role", "Role(.Id) is an entity type.");
    }

    #[test]
    fn metamodel_guard_rejects_reserved_constraint() {
        assert_reserved_rejected("Constraint", "Constraint(.Id) is an entity type.");
    }

    #[test]
    fn metamodel_guard_rejects_reserved_state_machine_definition() {
        assert_reserved_rejected(
            "State Machine Definition",
            "State Machine Definition(.Id) is an entity type.",
        );
    }

    #[test]
    fn metamodel_guard_rejects_reserved_transition() {
        assert_reserved_rejected("Transition", "Transition(.Id) is an entity type.");
    }

    #[test]
    fn metamodel_guard_rejects_reserved_status() {
        assert_reserved_rejected("Status", "Status(.Id) is an entity type.");
    }

    #[test]
    fn metamodel_guard_rejects_reserved_event_type() {
        assert_reserved_rejected("Event Type", "Event Type(.Id) is an entity type.");
    }

    #[test]
    fn metamodel_guard_rejects_reserved_domain_change() {
        assert_reserved_rejected("Domain Change", "Domain Change(.Id) is an entity type.");
    }

    #[test]
    fn compound_ref_scheme_decomposes_instance_ids() {
        use crate::ast::{fetch_or_phi, binding};
        let input = r#"
Thing(.Owner, .Seq) is an entity type.
Owner is a value type.
Seq is a value type.
Color is a value type.
Thing has Color.

## Instance Facts
Thing 'alice-1' has Color 'red'.
Thing 'alice-2' has Color 'blue'.
Thing 'bob-1' has Color 'green'.
"#;
        let ir = parse_markdown(input).unwrap();
        let state = ctx_to_state(&ir);

        // Component cells should exist with decomposed bindings
        let owner_cell = fetch_or_phi("Thing_has_Owner", &state);
        let owners = owner_cell.as_seq().expect("Thing_has_Owner cell must exist");
        assert_eq!(owners.len(), 3, "3 unique instance IDs â†’ 3 owner bindings");
        assert!(owners.iter().any(|f| binding(f, "Thing") == Some("alice-1") && binding(f, "Owner") == Some("alice")));
        assert!(owners.iter().any(|f| binding(f, "Thing") == Some("bob-1") && binding(f, "Owner") == Some("bob")));

        let seq_cell = fetch_or_phi("Thing_has_Seq", &state);
        let seqs = seq_cell.as_seq().expect("Thing_has_Seq cell must exist");
        assert!(seqs.iter().any(|f| binding(f, "Thing") == Some("alice-2") && binding(f, "Seq") == Some("2")));
    }

    #[test]
    fn compound_ref_scheme_handles_multi_hyphen_first_component() {
        use crate::ast::{fetch_or_phi, binding};
        let input = r#"
Widget(.System Name, .Number) is an entity type.
System Name is a value type.
Number is a value type.
Label is a value type.
Widget has Label.

## Instance Facts
Widget 'my-system-3' has Label 'foo'.
"#;
        let ir = parse_markdown(input).unwrap();
        let state = ctx_to_state(&ir);

        let name_cell = fetch_or_phi("Widget_has_System_Name", &state);
        let names = name_cell.as_seq().expect("Widget_has_System_Name must exist");
        // rsplitn(2, '-') on 'my-system-3' â†’ ['my-system', '3']
        assert!(names.iter().any(|f|
            binding(f, "Widget") == Some("my-system-3") &&
            binding(f, "System Name") == Some("my-system")
        ), "multi-hyphen first component should be preserved");
    }

    #[test]
    fn default_ref_scheme_is_id_for_entity_types() {
        use crate::ast::{fetch_or_phi, binding};
        let input = "Person is an entity type.\nColor is a value type.\n";
        let ir = parse_markdown(input).unwrap();
        let state = ctx_to_state(&ir);
        let nouns = fetch_or_phi("Noun", &state);
        let facts = nouns.as_seq().expect("Noun cell");
        let person = facts.iter().find(|f| binding(f, "name") == Some("Person")).unwrap();
        assert_eq!(binding(person, "referenceScheme"), Some("id"), "entity without explicit ref scheme defaults to id");
        let color = facts.iter().find(|f| binding(f, "name") == Some("Color")).unwrap();
        assert_eq!(binding(color, "referenceScheme"), None, "value types get no default ref scheme");
    }

    #[test]
    fn explicit_ref_scheme_overrides_default() {
        use crate::ast::{fetch_or_phi, binding};
        let input = "Case (.nr) is an entity type.\n";
        let ir = parse_markdown(input).unwrap();
        let state = ctx_to_state(&ir);
        let nouns = fetch_or_phi("Noun", &state);
        let facts = nouns.as_seq().expect("Noun cell");
        let case = facts.iter().find(|f| binding(f, "name") == Some("Case")).unwrap();
        assert_eq!(binding(case, "referenceScheme"), Some("nr"), "explicit ref scheme must not be overridden");
    }

    #[test]
    fn strict_mode_rejects_undeclared_partition_subtypes() {
        set_strict_mode(true);
        let input = "Animal is an entity type.\nAnimal is partitioned into Cat, Dog.\n";
        let result = parse_markdown(input);
        set_strict_mode(false);
        assert!(result.is_err(), "strict mode should reject undeclared partition subtypes");
        let err = result.unwrap_err();
        assert!(err.contains("Cat"), "error should mention Cat: {}", err);
        assert!(err.contains("Dog"), "error should mention Dog: {}", err);
    }

    #[test]
    fn loose_mode_auto_creates_partition_subtypes() {
        let input = "Animal is an entity type.\nAnimal is partitioned into Cat, Dog.\n";
        let ir = parse_markdown(input).unwrap();
        // #283 — Noun cell read.
        assert!(super::noun_exists(&ir.cells, "Cat"), "Cat should be auto-created in loose mode");
        assert!(super::noun_exists(&ir.cells, "Dog"), "Dog should be auto-created in loose mode");
    }

    #[test]
    fn dual_quoted_binary_instance_fact() {
        let input = r#"
App(.Name) is an entity type.
Generator(.Name) is an entity type.
App uses Generator.
## Instance Facts
App 'sherlock' uses Generator 'sqlite'.
"#;
        let ir = parse_markdown(input).unwrap();
        // #283 — InstanceFact cell read.
        let facts = ir.cells.get("InstanceFact").expect("InstanceFact cell");
        assert_eq!(facts.len(), 1,
            "Should parse dual-quoted binary instance fact, got: {:?}", facts);
        let f = &facts[0];
        assert_eq!(crate::ast::binding(f, "subjectNoun"), Some("App"));
        assert_eq!(crate::ast::binding(f, "subjectValue"), Some("sherlock"));
        assert_eq!(crate::ast::binding(f, "objectNoun"), Some("Generator"));
        assert_eq!(crate::ast::binding(f, "objectValue"), Some("sqlite"));
    }

    // â”€â”€ Task #198: Possessive join syntax in derivation bodies â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    //
    // "Order has Customer Age iff Order's Customer has Age"
    // should resolve to a Join through Customer, equivalent to:
    //   "Order has Customer Age iff Order has Customer and that Customer has Age"
    //
    // The possessive `Order's Customer` is syntactic sugar for a two-clause
    // anaphoric join; `try_expand_possessive` rewrites it before resolution.

    #[test]
    fn try_expand_possessive_rewrites_possessive_to_join_clauses() {
        // Unit-test the helper directly with a known noun list.
        let nouns = vec!["Order".to_string(), "Customer".to_string(), "Age".to_string()];
        let input = "Order's Customer has Age";
        let expanded = super::try_expand_possessive(input, &nouns)
            .expect("possessive should be expanded");
        assert_eq!(
            expanded,
            "Order has Customer and that Customer has Age",
            "unexpected expansion: {}", expanded
        );
    }

    #[test]
    fn try_expand_possessive_returns_none_when_no_possessive() {
        let nouns = vec!["Order".to_string(), "Customer".to_string()];
        let result = super::try_expand_possessive("Order has Customer", &nouns);
        assert!(result.is_none(), "should return None for text without possessive");
    }

    #[test]
    fn possessive_join_in_derivation_resolves() {
        // "Order has Customer Age iff Order's Customer has Age"
        // The possessive sugar is expanded to:
        //   "Order has Customer Age iff Order has Customer and that Customer has Age"
        // which the resolver classifies as a Join on Customer.
        let input = "\
Order(.Id) is an entity type.\n\
Customer(.Name) is an entity type.\n\
Age is a value type.\n\
Order has Customer.\n\
Customer has Age.\n\
Order has Customer Age.\n\
## Derivation Rules\n\
Order has Customer Age iff Order's Customer has Age.\n";
        let ir = parse_markdown(input).unwrap();
        let rule = ir.derivation_rules.iter()
            .find(|r| r.text.contains("Customer Age"))
            .expect("derivation rule for Customer Age must be present");
        assert_eq!(
            rule.antecedent_fact_type_ids.len(), 2,
            "possessive join rule must have 2 antecedents (Order has Customer + Customer has Age), got {:?}",
            rule.antecedent_fact_type_ids
        );
        assert_eq!(
            rule.kind,
            crate::types::DerivationKind::Join,
            "possessive join rule must be classified as Join, got {:?}",
            rule.kind
        );
        assert!(
            rule.join_on.iter().any(|k| k == "Customer"),
            "join_on must include 'Customer', got {:?}",
            rule.join_on
        );
    }
}
