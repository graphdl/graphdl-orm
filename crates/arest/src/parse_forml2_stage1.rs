//! Stage-1 tokenizer: text → `Statement` cells.
//!
//! #285 two-stage bootstrap. Stage-1 is a minimal Rust parser that
//! tokenizes each FORML 2 statement into the `Statement` / `Role
//! Reference` cell shape that `readings/forml2-grammar.md` (#284)
//! expects. Stage-2 applies the grammar's derivation rules to populate
//! the downstream metamodel cells.
//!
//! Stage-1 does NO classification — it only extracts:
//!
//! - `Statement has Text` — the canonical trimmed, period-stripped
//!   statement text.
//! - `Statement has Head Noun` — the first declared noun mentioned in
//!   the text (longest-first match per Theorem 1).
//! - `Statement has Verb` — the text between the head noun and the
//!   second role's noun (or the tail for unaries).
//! - `Statement has Trailing Marker` — the suffix phrase starting
//!   with a known marker keyword (`is an entity type`, `is abstract`,
//!   `is acyclic`, etc.) when present.
//! - `Statement has Derivation Marker` — ORM 2 `*` / `**` / `+`.
//! - `Statement has Quantifier` — leading quantifier (`each`, `at most
//!   one`, ...) when present.
//! - `Statement has Role Reference` — one `Role Reference` per matched
//!   noun, each with its `Role Position`, `Head Noun`, and optional
//!   `Literal Value`.

extern crate alloc;
use alloc::{string::{String, ToString}, vec::Vec, format};
use hashbrown::HashMap;
use crate::ast::{Object, fact_from_pairs};

type Cells = HashMap<String, Vec<Object>>;

/// Tokenize a single FORML 2 statement into the Stage-1 cell shape.
///
/// - `statement_id`: unique id for this Statement (caller-assigned).
/// - `text`: the raw statement text (trailing period allowed; this
///   function trims it).
/// - `nouns`: declared noun names. Matching uses longest-first.
///
/// Returns a `Cells` map the caller can merge into a larger state.
pub fn tokenize_statement(statement_id: &str, text: &str, nouns: &[String]) -> Cells {
    let mut cells: Cells = HashMap::new();
    let canonical = text.trim().trim_end_matches('.').trim();

    // 1. Strip trailing ORM 2 derivation marker.
    let (body, derivation_marker) = strip_derivation_marker(canonical);

    // 2. Strip leading quantifier.
    let (body, quantifier) = strip_leading_quantifier(body);

    // 3. Longest-first noun matching over the body.
    let mut sorted_nouns: Vec<&str> = nouns.iter().map(|s| s.as_str()).collect();
    sorted_nouns.sort_by(|a, b| b.len().cmp(&a.len()));
    let role_refs = match_role_references(body, &sorted_nouns);

    // 4. Head Noun + Verb derivation from role refs.
    let head_noun = role_refs.first().map(|r| r.noun.clone());
    let verb = extract_verb(body, &role_refs);

    // 5. Trailing Marker.
    let trailing_marker = extract_trailing_marker(body, &role_refs);

    // --- Statement cell ---
    push(&mut cells, "Statement", fact_from_pairs(&[
        ("id", statement_id), ("name", statement_id),
    ]));
    push(&mut cells, "Statement_has_Text", fact_from_pairs(&[
        ("Statement", statement_id), ("Text", canonical),
    ]));
    if let Some(hn) = head_noun.as_ref() {
        push(&mut cells, "Statement_has_Head_Noun", fact_from_pairs(&[
            ("Statement", statement_id), ("Head_Noun", hn),
        ]));
    }
    if let Some(v) = verb.as_ref() {
        if !v.is_empty() {
            push(&mut cells, "Statement_has_Verb", fact_from_pairs(&[
                ("Statement", statement_id), ("Verb", v),
            ]));
        }
    }
    if let Some(tm) = trailing_marker.as_ref() {
        push(&mut cells, "Statement_has_Trailing_Marker", fact_from_pairs(&[
            ("Statement", statement_id), ("Trailing_Marker", tm),
        ]));
    }
    if let Some(q) = quantifier.as_ref() {
        push(&mut cells, "Statement_has_Quantifier", fact_from_pairs(&[
            ("Statement", statement_id), ("Quantifier", q),
        ]));
    }
    if let Some(dm) = derivation_marker.as_ref() {
        push(&mut cells, "Statement_has_Derivation_Marker", fact_from_pairs(&[
            ("Statement", statement_id), ("Derivation_Marker", dm),
        ]));
    }
    // Existential marker: any role has a literal value. Lets the grammar
    // classify Instance Facts without walking the role list.
    if role_refs.iter().any(|r| r.literal.is_some()) {
        push(&mut cells, "Statement_has_Literal_Role", fact_from_pairs(&[
            ("Statement", statement_id), ("Literal_Role", "true"),
        ]));
    }
    // --- Role References ---
    for (i, rr) in role_refs.iter().enumerate() {
        let role_id = format!("{statement_id}:role:{i}");
        push(&mut cells, "Role_Reference", fact_from_pairs(&[
            ("id", role_id.as_str()), ("name", role_id.as_str()),
        ]));
        push(&mut cells, "Statement_has_Role_Reference", fact_from_pairs(&[
            ("Statement", statement_id), ("Role_Reference", role_id.as_str()),
        ]));
        push(&mut cells, "Role_Reference_has_Head_Noun", fact_from_pairs(&[
            ("Role_Reference", role_id.as_str()), ("Head_Noun", rr.noun.as_str()),
        ]));
        push(&mut cells, "Role_Reference_has_Role_Position", fact_from_pairs(&[
            ("Role_Reference", role_id.as_str()), ("Role_Position", &i.to_string()),
        ]));
        if let Some(lit) = rr.literal.as_ref() {
            push(&mut cells, "Role_Reference_has_Literal_Value", fact_from_pairs(&[
                ("Role_Reference", role_id.as_str()), ("Literal_Value", lit.as_str()),
            ]));
        }
    }

    cells
}

fn push(cells: &mut Cells, name: &str, fact: Object) {
    cells.entry(name.to_string()).or_default().push(fact);
}

fn strip_derivation_marker(text: &str) -> (&str, Option<String>) {
    if let Some(before) = text.strip_suffix(" **") {
        return (before, Some("derived-and-stored".to_string()));
    }
    if let Some(before) = text.strip_suffix(" *") {
        return (before, Some("fully-derived".to_string()));
    }
    if let Some(before) = text.strip_suffix(" +") {
        return (before, Some("semi-derived".to_string()));
    }
    (text, None)
}

const QUANTIFIERS: &[&str] = &[
    "at most one ",
    "at least one ",
    "exactly one ",
    "at most ",
    "at least ",
    "Each ",
    "each ",
    "Some ",
    "some ",
    "No ",
    "no ",
];

fn strip_leading_quantifier(text: &str) -> (&str, Option<String>) {
    for q in QUANTIFIERS {
        if let Some(rest) = text.strip_prefix(q) {
            return (rest, Some(q.trim().to_lowercase()));
        }
    }
    (text, None)
}

#[derive(Debug, Clone)]
struct RoleRef {
    noun: String,
    literal: Option<String>,
    /// Byte offset where the noun match starts in the body.
    start: usize,
    /// Byte offset where the noun match ends (exclusive). Excludes any
    /// following `'literal'` — that's tracked by `span_end`.
    end: usize,
    /// Effective end including any `' literal '` that followed. Used
    /// by the verb extractor so `Customer 'alice' places Order` yields
    /// verb = "places", not "'alice' places".
    span_end: usize,
}

fn match_role_references(text: &str, sorted_nouns: &[&str]) -> Vec<RoleRef> {
    let mut refs: Vec<RoleRef> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Require word boundary: previous char is non-alphanumeric (or start).
        let at_boundary = i == 0 || {
            let prev = bytes[i - 1];
            !prev.is_ascii_alphanumeric() && prev != b'_'
        };
        if !at_boundary {
            i += 1;
            continue;
        }
        let rest = &text[i..];
        // Longest-first match.
        if let Some(noun) = sorted_nouns.iter().find(|n| {
            rest.starts_with(**n) && {
                let end = i + n.len();
                end == bytes.len() || {
                    let next = bytes[end];
                    !next.is_ascii_alphanumeric() && next != b'_'
                }
            }
        }) {
            let start = i;
            let end = i + noun.len();
            // Skip if this match is entirely inside an earlier match.
            let nested = refs.iter().any(|r| start >= r.start && end <= r.end);
            if !nested {
                let (literal, span_end) = extract_following_literal_span(text, end);
                refs.push(RoleRef {
                    noun: (*noun).to_string(),
                    literal,
                    start,
                    end,
                    span_end,
                });
                i = span_end;
                continue;
            }
        }
        i += 1;
    }
    refs
}

/// Extract a single-quoted literal immediately after a noun match.
/// Halpin's `<Noun> '<value>'` instance-fact / ref-scheme-literal form.
/// Returns `(literal, span_end)` where `span_end` is the byte offset
/// immediately after the closing `'` (or `from` if no literal).
fn extract_following_literal_span(text: &str, from: usize) -> (Option<String>, usize) {
    let rest = &text[from..];
    let after_ws = rest.trim_start();
    let ws_len = rest.len() - after_ws.len();
    if !after_ws.starts_with('\'') {
        return (None, from);
    }
    let body = &after_ws[1..];
    match body.find('\'') {
        Some(end) => {
            let literal = body[..end].to_string();
            // from + ws_len + 1 (opening quote) + end + 1 (closing quote)
            let span_end = from + ws_len + 1 + end + 1;
            (Some(literal), span_end)
        }
        None => (None, from),
    }
}

fn extract_verb(text: &str, refs: &[RoleRef]) -> Option<String> {
    match refs.len() {
        0 => None,
        1 => {
            let tail = &text[refs[0].span_end..];
            Some(tail.trim().to_string())
        }
        _ => {
            let between = &text[refs[0].span_end..refs[1].start];
            Some(between.trim().to_string())
        }
    }
}

/// Trailing markers are suffixes that signal statement kind. The list
/// comes from the Rust classifier's fixed set. Stage-2 derivation
/// rules match against these marker atoms, not against raw prose.
const TRAILING_MARKERS: &[&str] = &[
    "is an entity type",
    "is a value type",
    "is abstract",
    "is acyclic",
    "is asymmetric",
    "is antisymmetric",
    "is intransitive",
    "is irreflexive",
    "is reflexive",
    "is symmetric",
    "is transitive",
    "are mutually exclusive",
    "is partitioned into",
    "is a subtype of",
];

fn extract_trailing_marker(text: &str, refs: &[RoleRef]) -> Option<String> {
    // Look only past the last noun match (including any trailing
    // literal). That keeps "is a value type" from matching inside
    // "X is a value type" where X is a noun.
    let start = refs.last().map(|r| r.span_end).unwrap_or(0);
    let tail = text[start..].trim();
    TRAILING_MARKERS.iter()
        .find(|m| tail == **m || tail.starts_with(*m))
        .map(|m| (*m).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::binding;

    fn nouns(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn stmt_binding(cells: &Cells, cell: &str, field: &str) -> Option<String> {
        cells.get(cell)?.first().and_then(|f| binding(f, field).map(String::from))
    }

    #[test]
    fn entity_type_declaration_extracts_head_noun_and_trailing_marker() {
        let c = tokenize_statement("s1", "Customer is an entity type.", &nouns(&["Customer"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Head_Noun", "Head_Noun").as_deref(),
                   Some("Customer"));
        assert_eq!(stmt_binding(&c, "Statement_has_Trailing_Marker", "Trailing_Marker").as_deref(),
                   Some("is an entity type"));
        assert_eq!(stmt_binding(&c, "Statement_has_Text", "Text").as_deref(),
                   Some("Customer is an entity type"));
    }

    #[test]
    fn value_type_declaration_extracts_trailing_marker() {
        let c = tokenize_statement("s1", "Priority is a value type.", &nouns(&["Priority"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Trailing_Marker", "Trailing_Marker").as_deref(),
                   Some("is a value type"));
    }

    #[test]
    fn abstract_declaration_extracts_trailing_marker() {
        let c = tokenize_statement("s1", "Request is abstract.", &nouns(&["Request"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Trailing_Marker", "Trailing_Marker").as_deref(),
                   Some("is abstract"));
    }

    #[test]
    fn binary_fact_type_extracts_head_verb_and_two_role_refs() {
        let c = tokenize_statement("s1", "Customer places Order.",
                                   &nouns(&["Customer", "Order"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Head_Noun", "Head_Noun").as_deref(),
                   Some("Customer"));
        assert_eq!(stmt_binding(&c, "Statement_has_Verb", "Verb").as_deref(),
                   Some("places"));
        assert_eq!(c.get("Role_Reference").map(|v| v.len()), Some(2));
    }

    #[test]
    fn instance_fact_captures_literal_on_role_reference() {
        let c = tokenize_statement("s1", "Customer 'alice' places Order 'o-7'.",
                                   &nouns(&["Customer", "Order"]));
        let rr_lit = c.get("Role_Reference_has_Literal_Value")
            .expect("literals cell must exist");
        assert_eq!(rr_lit.len(), 2);
        let literals: Vec<_> = rr_lit.iter()
            .filter_map(|f| binding(f, "Literal_Value"))
            .collect();
        assert!(literals.contains(&"alice"), "got literals: {:?}", literals);
        assert!(literals.contains(&"o-7"), "got literals: {:?}", literals);
    }

    #[test]
    fn derivation_marker_fully_derived_star() {
        let c = tokenize_statement("s1", "Customer has Full Name *.",
                                   &nouns(&["Customer", "Full Name"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Derivation_Marker", "Derivation_Marker").as_deref(),
                   Some("fully-derived"));
    }

    #[test]
    fn derivation_marker_derived_and_stored_double_star() {
        let c = tokenize_statement("s1", "Order has Total **.",
                                   &nouns(&["Order", "Total"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Derivation_Marker", "Derivation_Marker").as_deref(),
                   Some("derived-and-stored"));
    }

    #[test]
    fn derivation_marker_semi_derived_plus() {
        let c = tokenize_statement("s1", "Person is Grandparent +.",
                                   &nouns(&["Person", "Grandparent"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Derivation_Marker", "Derivation_Marker").as_deref(),
                   Some("semi-derived"));
    }

    #[test]
    fn quantifier_each_stripped_and_captured() {
        let c = tokenize_statement("s1", "Each Customer places at most one Order.",
                                   &nouns(&["Customer", "Order"]));
        // Outer quantifier is `each`; the inner `at most one` precedes Order
        // and is currently captured by the extract_verb step between refs.
        // First-pass Stage-1 keeps the outer quantifier only.
        assert_eq!(stmt_binding(&c, "Statement_has_Quantifier", "Quantifier").as_deref(),
                   Some("each"));
    }

    #[test]
    fn ring_acyclic_marker() {
        let c = tokenize_statement("s1", "Category has parent Category is acyclic.",
                                   &nouns(&["Category"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Trailing_Marker", "Trailing_Marker").as_deref(),
                   Some("is acyclic"));
    }

    #[test]
    fn longest_first_noun_match_wins() {
        let c = tokenize_statement("s1", "Support Request has Priority.",
                                   &nouns(&["Request", "Support Request", "Priority"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Head_Noun", "Head_Noun").as_deref(),
                   Some("Support Request"));
    }

    #[test]
    fn subtype_declaration_verb_captured() {
        let c = tokenize_statement("s1", "Support Request is a subtype of Request.",
                                   &nouns(&["Support Request", "Request"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Head_Noun", "Head_Noun").as_deref(),
                   Some("Support Request"));
        assert_eq!(stmt_binding(&c, "Statement_has_Verb", "Verb").as_deref(),
                   Some("is a subtype of"));
    }
}
