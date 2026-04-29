// crates/arest/src/types.rs
//
// Serde derives + per-field `#[serde(...)]` attrs are gated on the
// `std-deps` feature (#588). Under no_std (kernel build), `serde`
// isn't in the crate graph at all — every `derive(Deserialize,
// Serialize)` and every `#[serde(...)]` is therefore wrapped in
// `cfg_attr(feature = "std-deps", ...)` so the type definitions
// themselves stay no_std-clean while round-tripping through serde
// continues to work in the std build.
#[cfg(feature = "std-deps")]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

// Domain struct moved to parse_forml2.rs (#211) — it is a parse-time
// accumulator, not a public type. Re-exported as pub(crate) from there.

// -- Wire format helpers (#287 gap #6) --------------------------------
// Backslash-escape the delimiter `:` and escape char `\` so sentinel
// encodings for ConsequentCellSource / AntecedentSource survive role /
// noun names that legitimately contain either character. Used only by
// the compact single-binding wire format the metamodel flat-read path
// round-trips through; the lossless JSON `json` binding goes through
// serde directly and is unaffected.

fn wire_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => { out.push('\\'); out.push('\\'); }
            ':'  => { out.push('\\'); out.push(':'); }
            _    => out.push(c),
        }
    }
    out
}

fn wire_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(n) => out.push(n),
                None    => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Split once at the first UNESCAPED occurrence of `delim`. `\` in the
/// input can be used to escape `delim` or itself; any other `\<c>`
/// leaves `c` in the left half (wire_unescape handles the final
/// inverse). Returns `None` if no unescaped delimiter is found.
fn wire_split_once_unescaped(s: &str, delim: char) -> Option<(String, String)> {
    let mut left = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            // Take the next char verbatim into the left half. This
            // keeps the \-escape visible to downstream wire_unescape.
            left.push('\\');
            if let Some(n) = chars.next() { left.push(n); }
            continue;
        }
        if c == delim {
            let right: String = chars.collect();
            return Some((left, right));
        }
        left.push(c);
    }
    None
}

#[cfg(test)]
mod wire_tests {
    use super::*;

    #[test]
    fn round_trip_plain_role() {
        let src = ConsequentCellSource::AntecedentRole {
            antecedent_index: 0,
            role: "Fact Type".to_string(),
        };
        assert_eq!(ConsequentCellSource::decode(&src.encode()), src);
    }

    #[test]
    fn round_trip_role_with_colon() {
        let src = ConsequentCellSource::AntecedentRole {
            antecedent_index: 2,
            role: "Weird:Noun".to_string(),
        };
        assert_eq!(ConsequentCellSource::decode(&src.encode()), src);
    }

    #[test]
    fn round_trip_role_with_backslash_and_colon() {
        let src = ConsequentCellSource::AntecedentRole {
            antecedent_index: 7,
            role: "a\\b:c\\:d".to_string(),
        };
        assert_eq!(ConsequentCellSource::decode(&src.encode()), src);
    }

    #[test]
    fn round_trip_noun_with_colon() {
        let src = AntecedentSource::InstancesOfNoun("NS:Thing".to_string());
        assert_eq!(AntecedentSource::decode(&src.encode()), src);
    }

    #[test]
    fn round_trip_absence_of() {
        let src = AntecedentSource::AbsenceOf {
            fact_type: "ft_some:thing".to_string(),
            role: "Weird:Role".to_string(),
        };
        assert_eq!(AntecedentSource::decode(&src.encode()), src);
    }

    #[test]
    fn round_trip_plain_fact_type_id_passes_through() {
        let src = AntecedentSource::FactType("ordinary_ft_id".to_string());
        assert_eq!(src.encode(), "ordinary_ft_id");
        assert_eq!(AntecedentSource::decode(&src.encode()), src);
    }

    #[test]
    fn literal_id_passes_through_unescaped() {
        // Legacy literal ids without any `:` round-trip as bare strings,
        // preserving the pre-enum wire format.
        let src = ConsequentCellSource::Literal("foo_has_bar".to_string());
        assert_eq!(src.encode(), "foo_has_bar");
        assert_eq!(ConsequentCellSource::decode(&src.encode()), src);
    }
}

/// x̄ — a constant asserted into P.
/// Subject noun 'value' predicate object 'value'.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct GeneralInstanceFact {
    pub subject_noun: String,
    pub subject_value: String,
    pub field_name: String,
    pub object_noun: String,
    pub object_value: String,
}

/// World assumption for a noun — determines how absence of facts is interpreted
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "lowercase"))]
pub enum WorldAssumption {
    Closed, // not stated = false (permissions, corporate authority)
    Open,   // not stated = unknown (capabilities, unenumerated abilities)
}

impl Default for WorldAssumption {
    fn default() -> Self {
        WorldAssumption::Closed
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct NounDef {
    pub object_type: String,
    #[cfg_attr(feature = "std-deps", serde(default))]
    pub world_assumption: WorldAssumption,
}

/// How the consequent cell key is determined for a derivation.
///
/// `Literal` (the common case) pins a single cell at compile time — every
/// user-authored `iff`/`if` rule uses this shape, and the compiler lowers
/// it to `Func::constant(Object::atom(&id))` as the first element of each
/// derived tuple.
///
/// `AntecedentRole` pulls the cell key from a role value on one of the
/// antecedent facts. This is the shape implicit metamodel derivations
/// need — e.g. subtype inheritance fires against every user fact type
/// whose supertype appears as a role, and the target cell is that fact
/// type id, read from the antecedent `Fact Type has Role` tuple. The
/// compiler lowers to `Func::compose(role_value_by_name(role), Selector(n))`
/// for the first tuple element, and the evaluator already routes output
/// tuples to `ast::cell_push(&fact.fact_type_id, …)` — see
/// `evaluate::forward_chain_defs_state`.
///
/// Serialized as a sentinel-prefixed string on `consequentCell` bindings
/// so that plain (`Literal`) ids continue to parse and deserialize
/// exactly as before. The `@role:<idx>:<role_name>` prefix tags the
/// `AntecedentRole` form.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase", tag = "kind", content = "value"))]
pub enum ConsequentCellSource {
    /// Single cell keyed by a literal string.
    Literal(String),
    /// Cell key is the value of `role` on the `antecedent_index`-th
    /// antecedent fact at evaluation time.
    AntecedentRole {
        antecedent_index: usize,
        role: String,
    },
}

impl ConsequentCellSource {
    /// The literal cell key if this source is a `Literal`, else empty.
    ///
    /// Many compile-time code paths need the concrete id (to look up the
    /// fact type's reading, its RMAP table, or its role list). Those
    /// paths only apply to `Literal` consequents — dynamic ones resolve
    /// their reading/bindings at evaluation time. Returning empty for
    /// the dynamic case makes existing `is_empty()` guards continue to
    /// skip work appropriately.
    pub fn literal_id(&self) -> &str {
        match self {
            Self::Literal(s) => s.as_str(),
            _ => "",
        }
    }

    /// Is this the default empty-literal consequent? Used by parser
    /// fallbacks that construct a skeleton rule before resolution.
    pub fn is_empty_literal(&self) -> bool {
        matches!(self, Self::Literal(s) if s.is_empty())
    }

    /// Compact wire-format encoding. Literals serialize to bare strings
    /// so the pre-enum wire format (a `consequentFactTypeId` binding
    /// holding the id directly) is preserved. Dynamic shapes use
    /// sentinel-prefixed strings with backslash-escaping of the
    /// delimiter and escape character — so a role name containing `:`
    /// or `\` round-trips intact (#287 gap #6).
    ///
    /// Built via push rather than `format!` so the types module stays
    /// usable from both the no_std library and the std binary.
    pub fn encode(&self) -> String {
        match self {
            Self::Literal(s) => s.clone(),
            Self::AntecedentRole { antecedent_index, role } => {
                let mut out = String::from("@role:");
                out.push_str(&antecedent_index.to_string());
                out.push(':');
                out.push_str(&wire_escape(role));
                out
            }
        }
    }

    /// Inverse of `encode`. Unknown or malformed sentinels fall through
    /// to `Literal` so forward compatibility is preserved: a future
    /// variant unknown to an older reader just looks like a strange
    /// literal id rather than a parse failure.
    pub fn decode(s: &str) -> Self {
        if let Some(rest) = s.strip_prefix("@role:") {
            if let Some((idx_str, role_escaped)) = wire_split_once_unescaped(rest, ':') {
                if let Ok(antecedent_index) = idx_str.parse::<usize>() {
                    return Self::AntecedentRole {
                        antecedent_index,
                        role: wire_unescape(&role_escaped),
                    };
                }
            }
        }
        Self::Literal(s.to_string())
    }
}

impl Default for ConsequentCellSource {
    fn default() -> Self {
        Self::Literal(String::new())
    }
}

/// Where the antecedent fact sequence comes from.
///
/// `FactType(id)` — the common case — the antecedent is the cell's
/// fact list for a declared fact type, same shape every user rule
/// uses.
///
/// `InstancesOfNoun(noun)` — the "all instances of some noun
/// aggregated across every cell" shape the implicit derivations
/// (`#287` subtype inheritance and CWA negation) need. Lowers to
/// `compile::instances_of_noun_func(noun)`, which walks every cell
/// and extracts atoms of any binding keyed by `noun`. Inputs to the
/// per-element step are raw atoms (instance identifiers), not
/// `<<key,val>,…>` binding tuples, so the consequent binding shape
/// must be paired with a `consequent_instance_role` that names the
/// consequent role whose value is the raw atom.
///
/// Wire format for the flat-field (no-JSON) reconstruction path uses
/// `@noun:<name>` as a sentinel. Plain strings decode as `FactType`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase", tag = "kind", content = "value"))]
pub enum AntecedentSource {
    /// Antecedent is the cell's fact list for this FT id.
    FactType(String),
    /// Antecedent is a Seq of raw noun-instance atoms aggregated
    /// across every cell that binds the noun.
    InstancesOfNoun(String),
    /// Guard antecedent: the rule fires only when no fact of
    /// `fact_type` has the primary-antecedent instance bound at
    /// `role`. Used to express CWA negation's participation check
    /// as a standard rule shape rather than a bespoke inlined Func
    /// (`#287` gap #2). Currently only meaningful as a secondary
    /// antecedent when the primary is `InstancesOfNoun`.
    AbsenceOf {
        fact_type: String,
        role: String,
    },
}

impl AntecedentSource {
    /// The FT id if this source is `FactType`; empty string otherwise.
    pub fn fact_type_id(&self) -> &str {
        match self {
            Self::FactType(s) => s.as_str(),
            _ => "",
        }
    }

    pub fn is_instances_of_noun(&self) -> bool {
        matches!(self, Self::InstancesOfNoun(_))
    }

    pub fn encode(&self) -> String {
        match self {
            Self::FactType(s) => s.clone(),
            Self::InstancesOfNoun(noun) => {
                let mut out = String::from("@noun:");
                // Noun names round-trip through wire_escape so ":" in a
                // noun name doesn't break subsequent decoders (#287 gap
                // #6). FT ids are treated as opaque strings — a literal
                // id containing ":" still round-trips as `FactType(s)`.
                out.push_str(&wire_escape(noun));
                out
            }
            Self::AbsenceOf { fact_type, role } => {
                let mut out = String::from("@absence:");
                out.push_str(&wire_escape(fact_type));
                out.push(':');
                out.push_str(&wire_escape(role));
                out
            }
        }
    }

    pub fn decode(s: &str) -> Self {
        if let Some(escaped) = s.strip_prefix("@noun:") {
            return Self::InstancesOfNoun(wire_unescape(escaped));
        }
        if let Some(rest) = s.strip_prefix("@absence:") {
            if let Some((ft_escaped, role_escaped)) = wire_split_once_unescaped(rest, ':') {
                return Self::AbsenceOf {
                    fact_type: wire_unescape(&ft_escaped),
                    role: wire_unescape(&role_escaped),
                };
            }
        }
        Self::FactType(s.to_string())
    }
}

impl Default for AntecedentSource {
    fn default() -> Self {
        Self::FactType(String::new())
    }
}

/// A derivation rule in the IR — compiled to a DeriveFn at compile time.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct DerivationRuleDef {
    pub id: String,
    pub text: String,
    /// The reading/condition that must hold. Each antecedent source is
    /// either a declared fact type id (the common case) or an
    /// instances-of-noun source (the implicit-derivation shape).
    pub antecedent_sources: Vec<AntecedentSource>,
    /// Where the consequent's cell key comes from. `Literal(..)` is the
    /// common case (every user-authored rule) and preserves the
    /// pre-enum wire format. `AntecedentRole { .. }` lets a single rule
    /// fan out across all user fact types matching the antecedent,
    /// which the four implicit-derivation shapes (subtype inheritance,
    /// CWA negation, …) expand into readings (`#287`).
    #[cfg_attr(feature = "std-deps", serde(default))]
    pub consequent_cell: ConsequentCellSource,
    /// When the antecedent source is `InstancesOfNoun`, the per-element
    /// input to the per-fact step is a raw atom (the instance id), not
    /// a binding tuple. This field names the consequent role whose
    /// value becomes that atom — the result is a one-pair binding
    /// sequence `<<role, instance>>`. Empty for FactType-sourced rules
    /// (they inherit antecedent bindings via `Func::Id` as before).
    #[cfg_attr(feature = "std-deps", serde(default, skip_serializing_if = "String::is_empty"))]
    pub consequent_instance_role: String,
    /// Derivation kind for compile dispatch
    pub kind: DerivationKind,
    /// For Join rules: noun names that must have equal values across all
    /// antecedent facts that mention them. Enforces both:
    /// - Value join keys (e.g., "Squish VIN" matches Vehicle↔Candidate↔Listing)
    /// - Entity consistency (e.g., "Chrome Style Candidate" is the same entity
    ///   across candidate_squishvin and candidate_trim facts)
    #[cfg_attr(feature = "std-deps", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub join_on: Vec<String>,
    /// For Join rules: cross-noun match predicates. Each pair (left, right)
    /// requires that the value of left contains the value of right (case-insensitive).
    /// Used for fuzzy matching like "Chrome Trim" contains "Listing Trim".
    #[cfg_attr(feature = "std-deps", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub match_on: Vec<(String, String)>,
    /// For Join rules: noun names to include in the consequent bindings.
    /// If empty, all bindings from the joined facts are included.
    #[cfg_attr(feature = "std-deps", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub consequent_bindings: Vec<String>,
    /// Inline numeric comparisons attached to individual antecedents.
    ///
    /// Halpin FORML Example 5 (W3C position paper):
    ///   Each LargeUSCity is a City that is in Country 'US'
    ///                    and has Population >= 1000000.
    /// The `>= 1000000` is recorded as an AntecedentFilter pinned to the
    /// `has Population` antecedent. At compile time the rule becomes
    /// Filter(p) : P over the filtered antecedent facts.
    #[cfg_attr(feature = "std-deps", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub antecedent_filters: Vec<AntecedentFilter>,
    /// Consequent roles whose values are computed from arithmetic over
    /// antecedent role values. Halpin FORML attribute-style definitions:
    ///   * Box has Volume iff Box has Size and Volume is Size * Size * Size.
    /// The `Volume is Size * Size * Size` clause populates this with one
    /// ConsequentComputedBinding { role: "Volume", expr: ... }.
    #[cfg_attr(feature = "std-deps", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub consequent_computed_bindings: Vec<ConsequentComputedBinding>,
    /// Consequent roles whose values come from aggregating an image set
    /// (Codd §2.3.4 + Backus Insert). Halpin FORML:
    ///   * Fact Type has Arity iff Arity is the count of Role where Fact
    ///     Type has Role.
    #[cfg_attr(feature = "std-deps", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub consequent_aggregates: Vec<ConsequentAggregate>,
    /// Clauses the parser saw in the derivation body but could not
    /// classify into any known form (FT reference, comparison,
    /// aggregate, computed binding, anaphora, negation). The checker
    /// reports these directly — no parallel heuristic needed.
    #[cfg_attr(feature = "std-deps", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub unresolved_clauses: Vec<String>,
    /// Per-antecedent string-literal role equality filters. FORML 2 grammar
    /// readings use this for token matching, e.g.
    ///   `Statement has Classification 'Entity Type Declaration' iff
    ///    Statement has Trailing Marker 'is an entity type'`
    /// where the antecedent fact type `Statement has Trailing Marker`
    /// must have its `Trailing Marker` role equal to the literal
    /// `'is an entity type'`.
    #[cfg_attr(feature = "std-deps", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub antecedent_role_literals: Vec<AntecedentRoleLiteral>,
    /// Consequent roles bound to fixed string literals. Used by grammar
    /// readings whose consequent specifies a role's value, e.g.
    ///   `Statement has Classification 'Entity Type Declaration'`.
    /// The `Classification` role is bound to the literal; the remaining
    /// consequent roles inherit bindings from the antecedent fact.
    #[cfg_attr(feature = "std-deps", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub consequent_role_literals: Vec<ConsequentRoleLiteral>,
}

// ── Hand-rolled canonical JSON writer for `DerivationRuleDef` ────────
//
// Parser stage 2 (`bootstrap_grammar_state`) keys the rule cache on the
// JSON-serialized form of each `DerivationRuleDef`. We can't depend on
// `serde_json::to_string` from the no_std build (#588 stage2 blocker),
// so this hand-rolled writer reproduces the exact byte sequence serde
// emits — see `derivation_rule_def_canonical_json_matches_serde` in
// the std-host test below for the byte-for-byte fixture.
//
// Encoding rules — every field below mirrors what `serde_json` would
// produce given the `#[serde(...)]` attributes on the struct. Edits to
// any field ordering, rename, or `skip_serializing_if` predicate must
// be reflected here AND verified against the serde fixture.

/// Append `s` JSON-escaped (with surrounding quotes) to `out`. Matches
/// `serde_json`'s default escape policy: `"` → `\"`, `\` → `\\`, BS,
/// FF, LF, CR, TAB → their two-char escapes, all other C0 controls →
/// `\u00XX`. Non-ASCII UTF-8 bytes pass through unescaped (serde_json
/// does the same unless `unicode-escapes` is requested, which we don't).
fn json_escape(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                // Other C0 controls escape as \u00XX.
                let n = c as u32;
                out.push_str("\\u00");
                let hi = (n >> 4) & 0xf;
                let lo = n & 0xf;
                out.push(hex_digit(hi as u8));
                out.push(hex_digit(lo as u8));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn hex_digit(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + (n - 10)) as char,
        _ => '0',
    }
}

/// Append a `usize` as a JSON integer.
fn json_write_usize(out: &mut String, n: usize) {
    // alloc::string already provides the implementation we want.
    out.push_str(&n.to_string());
}

/// Append an `f64` as a JSON number, matching `serde_json`'s output.
/// `serde_json` uses `ryu`; Rust's `Debug` impl on `f64` produces the
/// same shortest-roundtrip representation for every finite value
/// the canonical-json fixture exercises (including integer-valued
/// doubles like `1000000.0` — the Halpin LargeUSCity case). NaN /
/// infinity round-trip through `serde_json` would error; we mirror by
/// emitting `null` (serde_json does the same when `serialize_f64`
/// hits a non-finite value via the default config).
fn json_write_f64(out: &mut String, v: f64) {
    if !v.is_finite() {
        out.push_str("null");
        return;
    }
    // `{:?}` emits e.g. `1.0`, `1000000.0`, `-3.14`, `0.1` — same shape
    // as `ryu::Buffer::format_finite`.
    let s = alloc::format!("{:?}", v);
    out.push_str(&s);
}

impl DerivationRuleDef {
    /// Hand-rolled canonical JSON serialization. Produces byte-identical
    /// output to `serde_json::to_string(self)` so existing rule-cache
    /// keys (which hash the serialized form) keep hitting after the
    /// no_std stage2 gate flips off serde dependence.
    ///
    /// Field walk order, casing, and `skip_serializing_if` semantics
    /// must stay locked to the `#[serde(...)]` attributes on the
    /// struct definition above. The std-host test
    /// `derivation_rule_def_canonical_json_matches_serde` covers a
    /// representative cross-section; any new field needs a fixture row.
    pub fn to_canonical_json(&self) -> String {
        let mut out = String::with_capacity(128);
        out.push('{');

        // 1. id
        out.push_str("\"id\":");
        json_escape(&mut out, &self.id);

        // 2. text
        out.push_str(",\"text\":");
        json_escape(&mut out, &self.text);

        // 3. antecedentSources (no skip — always present).
        out.push_str(",\"antecedentSources\":[");
        for (i, src) in self.antecedent_sources.iter().enumerate() {
            if i > 0 { out.push(','); }
            antecedent_source_write(&mut out, src);
        }
        out.push(']');

        // 4. consequentCell (no skip — always present).
        out.push_str(",\"consequentCell\":");
        consequent_cell_source_write(&mut out, &self.consequent_cell);

        // 5. consequentInstanceRole (skip if empty).
        if !self.consequent_instance_role.is_empty() {
            out.push_str(",\"consequentInstanceRole\":");
            json_escape(&mut out, &self.consequent_instance_role);
        }

        // 6. kind (always present).
        out.push_str(",\"kind\":");
        derivation_kind_write(&mut out, &self.kind);

        // 7. joinOn (skip if empty Vec).
        if !self.join_on.is_empty() {
            out.push_str(",\"joinOn\":[");
            for (i, s) in self.join_on.iter().enumerate() {
                if i > 0 { out.push(','); }
                json_escape(&mut out, s);
            }
            out.push(']');
        }

        // 8. matchOn (skip if empty Vec). Vec<(String,String)> → array of
        //    2-element arrays.
        if !self.match_on.is_empty() {
            out.push_str(",\"matchOn\":[");
            for (i, (a, b)) in self.match_on.iter().enumerate() {
                if i > 0 { out.push(','); }
                out.push('[');
                json_escape(&mut out, a);
                out.push(',');
                json_escape(&mut out, b);
                out.push(']');
            }
            out.push(']');
        }

        // 9. consequentBindings (skip if empty Vec).
        if !self.consequent_bindings.is_empty() {
            out.push_str(",\"consequentBindings\":[");
            for (i, s) in self.consequent_bindings.iter().enumerate() {
                if i > 0 { out.push(','); }
                json_escape(&mut out, s);
            }
            out.push(']');
        }

        // 10. antecedentFilters (skip if empty Vec).
        if !self.antecedent_filters.is_empty() {
            out.push_str(",\"antecedentFilters\":[");
            for (i, f) in self.antecedent_filters.iter().enumerate() {
                if i > 0 { out.push(','); }
                antecedent_filter_write(&mut out, f);
            }
            out.push(']');
        }

        // 11. consequentComputedBindings (skip if empty Vec).
        if !self.consequent_computed_bindings.is_empty() {
            out.push_str(",\"consequentComputedBindings\":[");
            for (i, b) in self.consequent_computed_bindings.iter().enumerate() {
                if i > 0 { out.push(','); }
                consequent_computed_binding_write(&mut out, b);
            }
            out.push(']');
        }

        // 12. consequentAggregates (skip if empty Vec).
        if !self.consequent_aggregates.is_empty() {
            out.push_str(",\"consequentAggregates\":[");
            for (i, a) in self.consequent_aggregates.iter().enumerate() {
                if i > 0 { out.push(','); }
                consequent_aggregate_write(&mut out, a);
            }
            out.push(']');
        }

        // 13. unresolvedClauses (skip if empty Vec).
        if !self.unresolved_clauses.is_empty() {
            out.push_str(",\"unresolvedClauses\":[");
            for (i, s) in self.unresolved_clauses.iter().enumerate() {
                if i > 0 { out.push(','); }
                json_escape(&mut out, s);
            }
            out.push(']');
        }

        // 14. antecedentRoleLiterals (skip if empty Vec).
        if !self.antecedent_role_literals.is_empty() {
            out.push_str(",\"antecedentRoleLiterals\":[");
            for (i, l) in self.antecedent_role_literals.iter().enumerate() {
                if i > 0 { out.push(','); }
                antecedent_role_literal_write(&mut out, l);
            }
            out.push(']');
        }

        // 15. consequentRoleLiterals (skip if empty Vec).
        if !self.consequent_role_literals.is_empty() {
            out.push_str(",\"consequentRoleLiterals\":[");
            for (i, l) in self.consequent_role_literals.iter().enumerate() {
                if i > 0 { out.push(','); }
                consequent_role_literal_write(&mut out, l);
            }
            out.push(']');
        }

        out.push('}');
        out
    }
}

/// Internally-tagged enum (`tag = "kind", content = "value"`) with
/// `rename_all = "camelCase"`. Note: the camelCase rename only applies
/// to the variant *identifiers* — struct-variant inner fields
/// (`antecedent_index`, `role`, etc.) keep their declared snake_case,
/// because no `rename_all` is on the inner struct portion. This matches
/// observed `serde_json` output:
///   `{"kind":"antecedentRole","value":{"antecedent_index":0,"role":"…"}}`
fn consequent_cell_source_write(out: &mut String, src: &ConsequentCellSource) {
    match src {
        ConsequentCellSource::Literal(s) => {
            out.push_str("{\"kind\":\"literal\",\"value\":");
            json_escape(out, s);
            out.push('}');
        }
        ConsequentCellSource::AntecedentRole { antecedent_index, role } => {
            out.push_str("{\"kind\":\"antecedentRole\",\"value\":{\"antecedent_index\":");
            json_write_usize(out, *antecedent_index);
            out.push_str(",\"role\":");
            json_escape(out, role);
            out.push_str("}}");
        }
    }
}

/// Same shape as `ConsequentCellSource` — internally-tagged enum.
/// Inner struct-variant fields stay snake_case (`fact_type`, `role`).
fn antecedent_source_write(out: &mut String, src: &AntecedentSource) {
    match src {
        AntecedentSource::FactType(s) => {
            out.push_str("{\"kind\":\"factType\",\"value\":");
            json_escape(out, s);
            out.push('}');
        }
        AntecedentSource::InstancesOfNoun(s) => {
            out.push_str("{\"kind\":\"instancesOfNoun\",\"value\":");
            json_escape(out, s);
            out.push('}');
        }
        AntecedentSource::AbsenceOf { fact_type, role } => {
            out.push_str("{\"kind\":\"absenceOf\",\"value\":{\"fact_type\":");
            json_escape(out, fact_type);
            out.push_str(",\"role\":");
            json_escape(out, role);
            out.push_str("}}");
        }
    }
}

/// Plain unit-variant enum with `rename_all = "camelCase"` →
/// just the camelCase variant name as a JSON string.
fn derivation_kind_write(out: &mut String, k: &DerivationKind) {
    let s = match k {
        DerivationKind::SubtypeInheritance => "subtypeInheritance",
        DerivationKind::ModusPonens => "modusPonens",
        DerivationKind::Transitivity => "transitivity",
        DerivationKind::ClosedWorldNegation => "closedWorldNegation",
        DerivationKind::Join => "join",
    };
    out.push('"');
    out.push_str(s);
    out.push('"');
}

fn antecedent_filter_write(out: &mut String, f: &AntecedentFilter) {
    out.push_str("{\"antecedentIndex\":");
    json_write_usize(out, f.antecedent_index);
    out.push_str(",\"role\":");
    json_escape(out, &f.role);
    out.push_str(",\"op\":");
    json_escape(out, &f.op);
    out.push_str(",\"value\":");
    json_write_f64(out, f.value);
    out.push('}');
}

fn antecedent_role_literal_write(out: &mut String, l: &AntecedentRoleLiteral) {
    out.push_str("{\"antecedentIndex\":");
    json_write_usize(out, l.antecedent_index);
    out.push_str(",\"role\":");
    json_escape(out, &l.role);
    out.push_str(",\"value\":");
    json_escape(out, &l.value);
    out.push('}');
}

fn consequent_role_literal_write(out: &mut String, l: &ConsequentRoleLiteral) {
    out.push_str("{\"role\":");
    json_escape(out, &l.role);
    out.push_str(",\"value\":");
    json_escape(out, &l.value);
    out.push('}');
}

fn consequent_computed_binding_write(out: &mut String, b: &ConsequentComputedBinding) {
    out.push_str("{\"role\":");
    json_escape(out, &b.role);
    out.push_str(",\"expr\":");
    arith_expr_write(out, &b.expr);
    out.push('}');
}

/// External-tagged enum (no `tag` / `content` attribute):
///   * Newtype variant `RoleRef(String)`     → `{"roleRef":"…"}`
///   * Newtype variant `Literal(f64)`        → `{"literal":<num>}`
///   * Tuple variant `Op(String, Box, Box)`  →
///     `{"op":["<sym>",<left>,<right>]}` — tuple of more than one
///     element serializes as a JSON array.
fn arith_expr_write(out: &mut String, e: &ArithExpr) {
    match e {
        ArithExpr::RoleRef(s) => {
            out.push_str("{\"roleRef\":");
            json_escape(out, s);
            out.push('}');
        }
        ArithExpr::Literal(v) => {
            out.push_str("{\"literal\":");
            json_write_f64(out, *v);
            out.push('}');
        }
        ArithExpr::Op(op, l, r) => {
            out.push_str("{\"op\":[");
            json_escape(out, op);
            out.push(',');
            arith_expr_write(out, l);
            out.push(',');
            arith_expr_write(out, r);
            out.push_str("]}");
        }
    }
}

fn consequent_aggregate_write(out: &mut String, a: &ConsequentAggregate) {
    out.push_str("{\"role\":");
    json_escape(out, &a.role);
    out.push_str(",\"op\":");
    json_escape(out, &a.op);
    out.push_str(",\"targetRole\":");
    json_escape(out, &a.target_role);
    out.push_str(",\"sourceFactTypeId\":");
    json_escape(out, &a.source_fact_type_id);
    out.push_str(",\"groupKeyRole\":");
    json_escape(out, &a.group_key_role);
    out.push('}');
}

/// Numeric comparison that further restricts a derivation antecedent.
/// See `DerivationRuleDef::antecedent_filters`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct AntecedentFilter {
    /// Index into `antecedent_sources` that this filter restricts.
    pub antecedent_index: usize,
    /// Role name whose value is compared (e.g., "Population").
    pub role: String,
    /// Comparison op: one of `">="`, `"<="`, `">"`, `"<"`, `"="`, `"!="`.
    /// `<>` input is normalized to `!=`.
    pub op: String,
    /// Numeric RHS literal.
    pub value: f64,
}

/// String-literal equality filter pinned to an antecedent role.
/// See `DerivationRuleDef::antecedent_role_literals`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct AntecedentRoleLiteral {
    /// Index into `antecedent_sources` that this filter restricts.
    pub antecedent_index: usize,
    /// Role name (noun name) whose value must equal `value`.
    pub role: String,
    /// Required literal value (string equality).
    pub value: String,
}

/// Fixed string literal bound to a consequent role.
/// See `DerivationRuleDef::consequent_role_literals`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct ConsequentRoleLiteral {
    /// Role name (noun name) in the consequent fact type.
    pub role: String,
    /// Literal string value to bind to that role.
    pub value: String,
}

/// Halpin FORML arithmetic expression used in attribute-style definitions
/// such as `Duration is End Time - Start Time`. Left-associative and flat
/// (no explicit precedence between + - * /): operators evaluate in the
/// order they appear. Parentheses aren't supported yet.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub enum ArithExpr {
    /// Reference to a role by name; resolved at compile time against the
    /// antecedent fact type's role list.
    RoleRef(String),
    /// Numeric literal baked in at parse time.
    Literal(f64),
    /// Binary operator — op is one of `"+"`, `"-"`, `"*"`, `"/"`.
    Op(String, Box<ArithExpr>, Box<ArithExpr>),
}

/// A consequent-role binding whose value is computed from an arithmetic
/// expression over antecedent role values, e.g. Halpin's
///   `Duration is End Time - Start Time`
/// where `Duration` is the computed role and `End Time - Start Time` is
/// the expression evaluated against each antecedent fact.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct ConsequentComputedBinding {
    /// Consequent role name (e.g., "Duration").
    pub role: String,
    /// Expression tree to evaluate against antecedent bindings.
    pub expr: ArithExpr,
}

/// A consequent-role binding whose value is an aggregate over an image set
/// (Codd 1972 §2.3.4: g_T(x) = {y : (x,y) ∈ T}). Halpin attribute-style:
///   `Arity is the count of Role where Fact Type has Role.`
///   `Order Amount is the sum of Line Item Amount where some Line Item
///    belongs to that Order.`
///
/// Classical relational algebra θ₁ is adequate for queries but not for
/// counting/summing (Codd p.1); AREST closes the gap with Backus's Insert
/// (`/`) and Length, so every aggregate lowers to θ₁ restriction + a fold.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct ConsequentAggregate {
    /// Consequent role name to receive the aggregate value (e.g. "Arity").
    pub role: String,
    /// Operator: one of `"count"`, `"sum"`, `"avg"`, `"min"`, `"max"`.
    pub op: String,
    /// Name of the role being aggregated over in the source FT. For
    /// `count`, this is the entity counted per group — often a noun that
    /// doesn't carry numeric value. For sum/avg/min/max it names the
    /// numeric role whose values are folded.
    pub target_role: String,
    /// Resolved source fact type id extracted from the `where` clause
    /// (e.g., `Fact_Type_has_Role`). Populated during
    /// `resolve_derivation_rule`.
    pub source_fact_type_id: String,
    /// Role name on the source FT that identifies group membership — the
    /// noun that also appears in the consequent as the non-aggregate role.
    /// For `Fact Type has Arity iff Arity is the count of Role where Fact
    /// Type has Role`, this is `"Fact Type"`.
    pub group_key_role: String,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub enum DerivationKind {
    SubtypeInheritance, // X is subtype of Y -> X inherits Y's constraints
    ModusPonens,        // If A then B, A holds -> B holds
    Transitivity,       // A->B, B->C -> A->C
    ClosedWorldNegation, // Not derivable under CWA -> false
    Join,               // Cross-fact-type equi-join on shared noun names
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct FactTypeDef {
    #[cfg_attr(feature = "std-deps", serde(default))]
    pub schema_id: String,
    pub reading: String,
    #[cfg_attr(feature = "std-deps", serde(default))]
    pub readings: Vec<ReadingDef>,
    pub roles: Vec<RoleDef>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct ReadingDef {
    pub text: String,
    pub role_order: Vec<usize>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct RoleDef {
    pub noun_name: String,
    pub role_index: usize,
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct ConstraintDef {
    pub id: String,
    pub kind: String,
    pub modality: String,
    pub deontic_operator: Option<String>,
    pub text: String,
    pub spans: Vec<SpanDef>,
    #[allow(dead_code)] // deserialized from JSON, read by JS callers
    pub set_comparison_argument_length: Option<usize>,
    #[allow(dead_code)] // deserialized from JSON, read by JS callers
    pub clauses: Option<Vec<String>>,
    pub entity: Option<String>,
    pub min_occurrence: Option<usize>,
    pub max_occurrence: Option<usize>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct SpanDef {
    pub fact_type_id: String,
    pub role_index: usize,
    #[allow(dead_code)] // deserialized from JSON, read by JS callers
    pub subset_autofill: Option<bool>,
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct StateMachineDef {
    pub noun_name: String,
    pub statuses: Vec<String>,
    pub transitions: Vec<TransitionDef>,
    /// Explicitly declared initial status. Empty when neither
    /// `Status 'X' is initial in SM Definition 'Y'` was asserted nor
    /// graph-topology inference (source-never-target) gave a unique
    /// answer. The compiled machine fails visibly at first SM call
    /// when empty — per Thm 3, the fold needs s_0.
    #[cfg_attr(feature = "std-deps", serde(default, skip_serializing_if = "String::is_empty"))]
    pub initial: String,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct TransitionDef {
    pub from: String,
    pub to: String,
    pub event: String,
    pub guard: Option<GuardDef>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct GuardDef {
    #[allow(dead_code)] // deserialized from JSON, read by JS callers
    pub fact_type_id: String,
    pub constraint_ids: Vec<String>,
}

// ── Evaluation Types ─────────────────────────────────────────────────

/// The AST state D: a sequence of cells (Backus Sec. 14.3).
/// Population P is ↑FILE:D. DEFS are cells in D.
pub type State = crate::ast::Object;

// Population and FactInstance structs deleted.
// State = Object (sequence of cells). Use ast helpers: fetch_or_phi, cell_push, etc.

/// A constraint violation.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct Violation {
    pub constraint_id: String,
    pub constraint_text: String,
    pub detail: String,
    /// Alethic violations are structural impossibilities (always reject).
    /// Deontic violations are reportable but may not reject.
    #[cfg_attr(feature = "std-deps", serde(default = "default_alethic"))]
    pub alethic: bool,
}

#[allow(dead_code)] // Used by serde default attribute
fn default_alethic() -> bool { true }

// ── Forward Inference & Synthesis Types ──────────────────────────────

/// A fact derived by forward inference
#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct DerivedFact {
    pub fact_type_id: String,
    pub reading: String,
    pub bindings: Vec<(String, String)>,
    pub derived_by: String, // ID of the derivation rule that produced this
    pub confidence: Confidence,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "std-deps", derive(Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "lowercase"))]
pub enum Confidence {
    Definitive, // derived under CWA — fact is definitively true/false
    #[allow(dead_code)] // reserved: used when OWA derivation is implemented
    Incomplete, // derived under OWA — absence doesn't mean false
}

// ── Proof Engine Types ──────────────────────────────────────────────

/// Result of attempting to prove a goal
#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
#[allow(dead_code)] // used by lib.rs WASM export, not by main.rs binary
pub enum ProofStatus {
    /// Goal is proven — a derivation chain exists from axioms to the goal
    Proven,
    /// Goal is disproven — under CWA, the absence of proof means false
    Disproven,
    /// Goal is unknown — under OWA, absence of proof doesn't mean false
    Unknown,
}

/// A single step in a proof tree
#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
#[allow(dead_code)] // used by lib.rs WASM export, not by main.rs binary
pub struct ProofStep {
    /// The fact being proven at this step
    pub fact: String,
    /// How this fact was established
    pub justification: Justification,
    /// Child steps (antecedents that were proven to derive this fact)
    pub children: Vec<ProofStep>,
}

/// How a fact was established in a proof
#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
#[allow(dead_code)] // used by lib.rs WASM export, not by main.rs binary
pub enum Justification {
    /// Fact exists directly in the population (axiom)
    Axiom,
    /// Derived by applying a rule to the child steps
    Derived { rule_id: String, rule_text: String },
    /// Assumed false under Closed World Assumption
    #[allow(dead_code)] // reserved: constructed when prove returns leaf-level CWA negation
    ClosedWorldNegation,
    /// Cannot be proven or disproven under Open World Assumption
    #[allow(dead_code)] // reserved: constructed when prove returns leaf-level OWA unknown
    OpenWorld,
}

/// Complete proof result
#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
#[allow(dead_code)] // used by lib.rs WASM export, not by main.rs binary
pub struct ProofResult {
    pub goal: String,
    pub status: ProofStatus,
    pub proof: Option<ProofStep>,
    pub world_assumption: WorldAssumption,
}

/// Result of synthesizing knowledge about a noun
#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct SynthesisResult {
    pub noun_name: String,
    pub world_assumption: WorldAssumption,
    /// Fact types where this noun plays a role
    pub participates_in: Vec<FactTypeSummary>,
    /// Constraints that apply to this noun
    pub applicable_constraints: Vec<ConstraintSummary>,
    /// State machines for this noun
    pub state_machines: Vec<StateMachineSummary>,
    /// Facts derived by forward chaining
    pub derived_facts: Vec<DerivedFact>,
    /// Related nouns (one hop via shared fact types)
    pub related_nouns: Vec<RelatedNoun>,
}

impl SynthesisResult {
    #[allow(dead_code)] // used by lib.rs WASM export, not by main.rs binary
    pub fn empty(noun_name: &str) -> Self {
        SynthesisResult {
            noun_name: noun_name.to_string(),
            world_assumption: WorldAssumption::default(),
            participates_in: Vec::new(),
            applicable_constraints: Vec::new(),
            state_machines: Vec::new(),
            derived_facts: Vec::new(),
            related_nouns: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct FactTypeSummary {
    pub id: String,
    pub reading: String,
    pub role_index: usize,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct ConstraintSummary {
    pub id: String,
    pub text: String,
    pub kind: String,
    pub modality: String,
    pub deontic_operator: Option<String>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct StateMachineSummary {
    pub noun_name: String,
    pub statuses: Vec<String>,
    pub current_status: Option<String>,
    pub valid_transitions: Vec<String>, // events that can fire from current state
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct RelatedNoun {
    pub name: String,
    pub via_fact_type: String,
    pub via_reading: String,
    pub world_assumption: WorldAssumption,
}

// ── Canonical-JSON tests for `DerivationRuleDef` ─────────────────────
//
// These tests are gated on `std-deps` because the byte-for-byte
// fixture compares against `serde_json::to_string`. The `to_canonical_json`
// method itself is feature-independent — only the comparison oracle
// requires serde.
#[cfg(all(test, feature = "std-deps"))]
mod canonical_json_tests {
    use super::*;

    fn sample_rule_with_filter() -> DerivationRuleDef {
        DerivationRuleDef {
            id: "rule_largeUSCity".to_string(),
            text: "Each LargeUSCity is a City that is in Country 'US' and has Population >= 1000000".to_string(),
            antecedent_sources: alloc::vec![
                AntecedentSource::FactType("city_in_country".to_string()),
                AntecedentSource::FactType("city_population".to_string()),
            ],
            consequent_instance_role: String::new(),
            consequent_cell: ConsequentCellSource::Literal("LargeUSCity_subtype".to_string()),
            kind: DerivationKind::ModusPonens,
            join_on: alloc::vec!["City".to_string()],
            match_on: alloc::vec![("a".to_string(), "b".to_string())],
            consequent_bindings: alloc::vec!["City".to_string()],
            antecedent_filters: alloc::vec![AntecedentFilter {
                antecedent_index: 1,
                role: "Population".to_string(),
                op: ">=".to_string(),
                value: 1_000_000.0,
            }],
            consequent_computed_bindings: alloc::vec![ConsequentComputedBinding {
                role: "Volume".to_string(),
                expr: ArithExpr::Op(
                    "*".to_string(),
                    Box::new(ArithExpr::RoleRef("Size".to_string())),
                    Box::new(ArithExpr::Literal(2.5)),
                ),
            }],
            consequent_aggregates: alloc::vec![ConsequentAggregate {
                role: "Arity".to_string(),
                op: "count".to_string(),
                target_role: "Role".to_string(),
                source_fact_type_id: "Fact_Type_has_Role".to_string(),
                group_key_role: "Fact Type".to_string(),
            }],
            unresolved_clauses: alloc::vec!["weird".to_string()],
            antecedent_role_literals: alloc::vec![AntecedentRoleLiteral {
                antecedent_index: 0,
                role: "Trailing Marker".to_string(),
                value: "is an entity type".to_string(),
            }],
            consequent_role_literals: alloc::vec![ConsequentRoleLiteral {
                role: "Classification".to_string(),
                value: "Entity Type Declaration".to_string(),
            }],
        }
    }

    fn sample_rule_minimal() -> DerivationRuleDef {
        DerivationRuleDef {
            id: "rule_minimal".to_string(),
            text: "Foo iff Bar".to_string(),
            antecedent_sources: alloc::vec![AntecedentSource::FactType("ft1".to_string())],
            consequent_instance_role: String::new(),
            consequent_cell: ConsequentCellSource::Literal("ft2".to_string()),
            kind: DerivationKind::ModusPonens,
            join_on: Vec::new(),
            match_on: Vec::new(),
            consequent_bindings: Vec::new(),
            antecedent_filters: Vec::new(),
            consequent_computed_bindings: Vec::new(),
            consequent_aggregates: Vec::new(),
            unresolved_clauses: Vec::new(),
            antecedent_role_literals: Vec::new(),
            consequent_role_literals: Vec::new(),
        }
    }

    fn sample_rule_grammar_classifier() -> DerivationRuleDef {
        // Mirrors the shape `bootstrap_grammar_state` actually constructs:
        // single antecedent, ConsequentRoleLiterals + AntecedentRoleLiterals
        // populated, every other Vec empty.
        DerivationRuleDef {
            id: "rule_abcd1234".to_string(),
            text: "Statement has Classification 'Entity Type Declaration' iff Statement has Trailing Marker 'is an entity type'".to_string(),
            antecedent_sources: alloc::vec![AntecedentSource::FactType(
                "Statement_has_Trailing_Marker".to_string(),
            )],
            consequent_instance_role: String::new(),
            consequent_cell: ConsequentCellSource::Literal(
                "Statement_has_Classification".to_string(),
            ),
            kind: DerivationKind::ModusPonens,
            join_on: Vec::new(),
            match_on: Vec::new(),
            consequent_bindings: Vec::new(),
            antecedent_filters: Vec::new(),
            consequent_computed_bindings: Vec::new(),
            consequent_aggregates: Vec::new(),
            unresolved_clauses: Vec::new(),
            antecedent_role_literals: alloc::vec![AntecedentRoleLiteral {
                antecedent_index: 0,
                role: "Trailing Marker".to_string(),
                value: "is an entity type".to_string(),
            }],
            consequent_role_literals: alloc::vec![ConsequentRoleLiteral {
                role: "Classification".to_string(),
                value: "Entity Type Declaration".to_string(),
            }],
        }
    }

    fn sample_rule_with_dynamic_consequent_and_absence() -> DerivationRuleDef {
        DerivationRuleDef {
            id: "rule_dynamic".to_string(),
            text: "subtype inheritance".to_string(),
            antecedent_sources: alloc::vec![
                AntecedentSource::InstancesOfNoun("Foo".to_string()),
                AntecedentSource::AbsenceOf {
                    fact_type: "ft_x".to_string(),
                    role: "R".to_string(),
                },
            ],
            consequent_instance_role: "Bar".to_string(),
            consequent_cell: ConsequentCellSource::AntecedentRole {
                antecedent_index: 0,
                role: "Some Role".to_string(),
            },
            kind: DerivationKind::SubtypeInheritance,
            join_on: Vec::new(),
            match_on: Vec::new(),
            consequent_bindings: Vec::new(),
            antecedent_filters: Vec::new(),
            consequent_computed_bindings: Vec::new(),
            consequent_aggregates: Vec::new(),
            unresolved_clauses: Vec::new(),
            antecedent_role_literals: Vec::new(),
            consequent_role_literals: Vec::new(),
        }
    }

    fn sample_rule_with_escapes() -> DerivationRuleDef {
        DerivationRuleDef {
            id: "rule_escapes".to_string(),
            // Mix of all the JSON escape classes the writer must handle:
            // quote, backslash, newline, tab, carriage return, control char,
            // and a non-ASCII pass-through.
            text: "tab\there\nnewline\rCR \"quote\" \\back \u{01}ctrl é".to_string(),
            antecedent_sources: alloc::vec![AntecedentSource::FactType(
                "ft\"weird\\name".to_string(),
            )],
            consequent_instance_role: String::new(),
            consequent_cell: ConsequentCellSource::Literal("c".to_string()),
            kind: DerivationKind::Transitivity,
            join_on: Vec::new(),
            match_on: Vec::new(),
            consequent_bindings: Vec::new(),
            antecedent_filters: Vec::new(),
            consequent_computed_bindings: Vec::new(),
            consequent_aggregates: Vec::new(),
            unresolved_clauses: Vec::new(),
            antecedent_role_literals: Vec::new(),
            consequent_role_literals: Vec::new(),
        }
    }

    /// Byte-for-byte fixture compare against serde_json. This is the
    /// load-bearing contract: `bootstrap_grammar_state` keys the rule
    /// cache on the JSON string, so any diff here means cache misses
    /// for already-baked grammars.
    #[test]
    fn derivation_rule_def_canonical_json_matches_serde() {
        for r in [
            sample_rule_minimal(),
            sample_rule_grammar_classifier(),
            sample_rule_with_filter(),
            sample_rule_with_dynamic_consequent_and_absence(),
            sample_rule_with_escapes(),
        ] {
            let serde_out = serde_json::to_string(&r).expect("serde_json should serialize");
            let canonical = r.to_canonical_json();
            assert_eq!(
                canonical, serde_out,
                "to_canonical_json must match serde_json byte-for-byte"
            );
        }
    }

    /// Round-trip: hand-roll-serialize, then parse via serde_json,
    /// verify the rule re-encodes to the same string. Catches cases
    /// where the writer emits valid JSON but with a shape that confuses
    /// the deserializer (e.g., wrong tag name, wrong inner-field case).
    #[test]
    fn derivation_rule_def_canonical_json_round_trips() {
        for r in [
            sample_rule_minimal(),
            sample_rule_grammar_classifier(),
            sample_rule_with_filter(),
            sample_rule_with_dynamic_consequent_and_absence(),
            sample_rule_with_escapes(),
        ] {
            let canonical = r.to_canonical_json();
            let parsed: DerivationRuleDef = serde_json::from_str(&canonical)
                .unwrap_or_else(|e| panic!("parse failed: {} for {}", e, canonical));
            // Round-trip the parsed value back through the writer and
            // confirm it matches the original. Comparing structs
            // directly is awkward (DerivationRuleDef doesn't impl Eq);
            // re-serializing gives us a deterministic equality check.
            let re_canonical = parsed.to_canonical_json();
            assert_eq!(canonical, re_canonical);
        }
    }
}
