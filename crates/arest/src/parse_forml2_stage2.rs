//! Stage-2 applier: Statement cells → Classification cells via grammar rules.
//!
//! #280 meta-circular parser. Stage-2 consumes:
//!
//!   (a) a state populated with `Statement_*` cells from Stage-1
//!       (`parse_forml2_stage1::tokenize_statement`), and
//!   (b) the grammar state from parsing `readings/forml2-grammar.md`,
//!
//! and applies the grammar's derivation rules (compiled through the
//! standard `compile_to_defs_state` + `forward_chain_defs_state`
//! pipeline) to emit `Statement has Classification` facts — one per
//! recognized statement kind per Statement.
//!
//! The grammar uses a small, fixed rule shape:
//!
//!   Statement has Classification '<Kind>' iff Statement has <Token>
//!     ['<value>']
//!
//! Literal values on consequent and antecedent roles flow through
//! DerivationRuleDef::consequent_role_literals and
//! DerivationRuleDef::antecedent_role_literals (#286). Stage-2 no
//! longer has a focused interpreter for this shape; it just merges
//! grammar + statements, compiles, forward-chains, and returns the
//! enriched state.
//!
//! Translation from classification to canonical metamodel cells
//! (Noun, Fact Type, Role, …) is the per-kind #280b commits.

extern crate alloc;
use alloc::{string::{String, ToString}, vec::Vec};
use hashbrown::HashMap;
use crate::ast::{Object, fetch_or_phi, fact_from_pairs, binding};
use crate::time_shim::Instant;

// ── MC2 Stage-2 dispatch tables (#713) ──────────────────────────────
//
// Stage-2's translators used to keep three hardcoded dispatch matrices:
//
//   1. `ring_adjective_to_kind` mapped trailing-marker phrases
//      ("is acyclic" / "is symmetric" / ...) to the ORM 2 ring
//      constraint kind code ("AC" / "SY" / ...).
//   2. `conditional_ring_kind` mapped a 5-tuple of antecedent /
//      consequent boolean shape signals to a ring kind code, used
//      for the conditional ring shape `If X R Y then ...`.
//   3. `translate_deontic_constraints` emitted constraint cell facts
//      with kind="UC" / modality="deontic" for every deontic operator
//      in `("obligatory", "forbidden", "permitted")`.
//
// MC2 lifts each of those three tables to enum-value declarations in
// `readings/forml2-grammar.md` and reads them back via the same cell
// path as MC1's `Vocab`. The boot fallback stays in Rust for the
// chicken-and-egg case where the tables drive the parser that loads
// the readings that define the tables.
//
// Each table struct exposes:
//   - `Self::boot()` — Rust-side fallback constants kept byte-for-byte
//     identical to the readings declarations.
//   - `Self::from_grammar_state(state)` — parallel-enum reader. Reads
//     two enum value lists keyed by `noun:` from the EnumValues cell
//     and zips them by index.

/// Stage-2 ring kind dispatch — `<trailing-marker>` → ORM 2 kind code.
/// Owned `Vec<(String, String)>` so the table outlives any borrowed
/// grammar state (mirrors `Vocab::derivation_markers`).
#[derive(Debug, Clone)]
pub struct RingKindTable {
    /// Pairs of `(trailing-marker, kind-code)` such as
    /// `("is acyclic", "AC")`. Order matches the readings'
    /// parallel-enum declaration order.
    pub markers: Vec<(String, String)>,
}

impl RingKindTable {
    /// Boot table — must stay in sync with the parallel
    /// `Ring Constraint Trailing Marker` and `Ring Constraint Kind
    /// Code` enum-value declarations in `readings/forml2-grammar.md`.
    pub fn boot() -> Self {
        RingKindTable {
            markers: alloc::vec![
                ("is irreflexive".to_string(),   "IR".to_string()),
                ("is asymmetric".to_string(),    "AS".to_string()),
                ("is antisymmetric".to_string(), "AT".to_string()),
                ("is symmetric".to_string(),     "SY".to_string()),
                ("is intransitive".to_string(),  "IT".to_string()),
                ("is transitive".to_string(),    "TR".to_string()),
                ("is acyclic".to_string(),       "AC".to_string()),
                ("is reflexive".to_string(),     "RF".to_string()),
            ],
        }
    }

    /// Build the table by reading the parallel `Ring Constraint
    /// Trailing Marker` / `Ring Constraint Kind Code` enum-value
    /// declarations from a parsed grammar state's EnumValues cell.
    /// Falls back to `boot()` if the lists are missing or have
    /// mismatched lengths.
    pub fn from_grammar_state(state: &Object) -> Self {
        let markers = read_parallel_enum_pair(
            state,
            "Ring Constraint Trailing Marker",
            "Ring Constraint Kind Code",
        );
        match markers {
            Some(m) => RingKindTable { markers: m },
            None => Self::boot(),
        }
    }

    /// Look up the kind code for a trailing marker phrase.
    pub fn kind_for(&self, marker: &str) -> Option<&str> {
        self.markers.iter()
            .find(|(m, _)| m == marker)
            .map(|(_, k)| k.as_str())
    }
}

/// Stage-2 conditional ring matrix — encoded boolean tuple → ORM 2
/// kind code. The encoded pattern names mirror `conditional_ring_kind`'s
/// match arms (see `encode_conditional_ring_pattern`).
#[derive(Debug, Clone)]
pub struct ConditionalRingMatrix {
    /// `(pattern-name, kind-code)` rows in declaration order. Lookup
    /// is by exact pattern-name match.
    pub rows: Vec<(String, String)>,
}

impl ConditionalRingMatrix {
    /// Boot matrix — must stay in sync with the parallel
    /// `Conditional Ring Pattern` / `Conditional Ring Kind Code`
    /// enum-value declarations in `readings/forml2-grammar.md`.
    pub fn boot() -> Self {
        ConditionalRingMatrix {
            rows: alloc::vec![
                ("and+impossible+isnot-ante".to_string(), "AT".to_string()),
                ("and+impossible".to_string(),            "IT".to_string()),
                ("and".to_string(),                       "TR".to_string()),
                ("impossible".to_string(),                "AS".to_string()),
                ("isnot-conse".to_string(),               "AS".to_string()),
                ("itself-conse".to_string(),              "RF".to_string()),
                ("plain".to_string(),                     "SY".to_string()),
            ],
        }
    }

    /// Build the matrix from parallel `Conditional Ring Pattern` /
    /// `Conditional Ring Kind Code` enum-value declarations.
    /// Falls back to `boot()` on any mismatch.
    pub fn from_grammar_state(state: &Object) -> Self {
        let rows = read_parallel_enum_pair(
            state,
            "Conditional Ring Pattern",
            "Conditional Ring Kind Code",
        );
        match rows {
            Some(r) => ConditionalRingMatrix { rows: r },
            None => Self::boot(),
        }
    }

    /// Look up the kind code for an encoded pattern name.
    pub fn kind_for(&self, pattern: &str) -> Option<&str> {
        self.rows.iter()
            .find(|(p, _)| p == pattern)
            .map(|(_, k)| k.as_str())
    }
}

/// Stage-2 deontic shape table — `<deontic-operator>` → emission
/// `(kind, modality)` pair. Currently every operator emits the same
/// shape (`UC` / `deontic`); the table is lifted so future operators
/// can emit distinct shapes without re-touching the translator.
#[derive(Debug, Clone)]
pub struct DeonticShapeTable {
    /// `(operator, kind-code, modality)` rows in declaration order.
    pub rows: Vec<(String, String, String)>,
}

impl DeonticShapeTable {
    /// Boot table — must stay in sync with the parallel
    /// `Deontic Operator` / `Deontic Constraint Kind Code` /
    /// `Deontic Constraint Modality` enum-value declarations.
    pub fn boot() -> Self {
        DeonticShapeTable {
            rows: alloc::vec![
                ("obligatory".to_string(), "UC".to_string(), "deontic".to_string()),
                ("forbidden".to_string(),  "UC".to_string(), "deontic".to_string()),
                ("permitted".to_string(),  "UC".to_string(), "deontic".to_string()),
            ],
        }
    }

    /// Build the table from parallel `Deontic Operator` /
    /// `Deontic Constraint Kind Code` / `Deontic Constraint Modality`
    /// enum-value declarations.  Falls back to `boot()` on any mismatch.
    pub fn from_grammar_state(state: &Object) -> Self {
        let ops = read_enum_values(state, "Deontic Operator");
        let kinds = read_enum_values(state, "Deontic Constraint Kind Code");
        let modalities = read_enum_values(state, "Deontic Constraint Modality");
        if ops.len() == kinds.len()
            && ops.len() == modalities.len()
            && !ops.is_empty()
        {
            let rows = ops.into_iter()
                .zip(kinds)
                .zip(modalities)
                .map(|((o, k), m)| (o, k, m))
                .collect();
            DeonticShapeTable { rows }
        } else {
            Self::boot()
        }
    }

    /// Look up the `(kind, modality)` pair for a deontic operator.
    pub fn shape_for(&self, op: &str) -> Option<(&str, &str)> {
        self.rows.iter()
            .find(|(o, _, _)| o == op)
            .map(|(_, k, m)| (k.as_str(), m.as_str()))
    }
}

/// Type signature for stage-2 statement translators with the uniform
/// shape `(classified_state, idx) -> Vec<Object>`. The non-conforming
/// translators are excluded from the registry — `translate_fact_types`
/// returns a `(Vec<Object>, Vec<Object>)` tuple and the
/// `_with_*` variants take extra dispatch tables. Per AREST.tex §3.2
/// Platform Binding, registered functions occupy the platform-layer
/// complement of compiled readings; together they span DEFS.
pub type TranslatorFn = fn(&Object, &StmtIndex) -> Vec<Object>;

/// Registry of translator-name → fn pointer. The string keys must
/// match the second column of the
/// `Classification_has_Translator` cell built from instance facts
/// in `readings/forml2-grammar.md` so the
/// `statement_translator_table_translator_names_resolve_to_functions`
/// regression catches typos and renames.
///
/// Scope is the uniform `(state, idx) -> Vec<Object>` translators.
/// Multi-output translators (translate_fact_types) and arg-bearing
/// variants (translate_*_with_table[s]/_matrix/_ft_ids) live outside
/// the registry — the stage-2 pipeline still calls them by name.
pub fn translator_function_registry() -> hashbrown::HashMap<&'static str, TranslatorFn> {
    let mut m: hashbrown::HashMap<&'static str, TranslatorFn> = hashbrown::HashMap::new();
    m.insert("translate_nouns",                  translate_nouns                 as TranslatorFn);
    m.insert("translate_subtypes",               translate_subtypes              as TranslatorFn);
    m.insert("translate_partitions",             translate_partitions            as TranslatorFn);
    m.insert("translate_derivation_mode_facts",  translate_derivation_mode_facts as TranslatorFn);
    m.insert("translate_instance_facts",         translate_instance_facts        as TranslatorFn);
    m.insert("translate_ring_constraints",       translate_ring_constraints      as TranslatorFn);
    m.insert("translate_cardinality_constraints",translate_cardinality_constraints as TranslatorFn);
    m.insert("translate_set_constraints",        translate_set_constraints       as TranslatorFn);
    m.insert("translate_value_constraints",      translate_value_constraints     as TranslatorFn);
    m.insert("translate_deontic_constraints",    translate_deontic_constraints   as TranslatorFn);
    m.insert("translate_derivation_rules",       translate_derivation_rules      as TranslatorFn);
    m.insert("translate_enum_values",            translate_enum_values           as TranslatorFn);
    m
}

/// Stage-2 noun object-type dispatch — `<classification-kind>` →
/// declared Object Type. Mirrors `CardinalityConstraintKindTable`
/// (#833 layer 4) but maps Statement Classifications to noun object
/// types. Five rows in cascade order: Abstract / Partition Declaration
/// → `abstract`, Entity Type Declaration → `entity`, Value Type
/// Declaration → `value`, Subtype Declaration → `entity`. Abstract
/// rows come first so a noun classified as both abstract and entity
/// is recorded as abstract (legacy "abstract wins" semantics).
#[derive(Debug, Clone)]
pub struct ObjectTypeKindTable {
    /// Pairs of `(classification-kind, object-type)` such as
    /// `("Entity Type Declaration", "entity")`. Order matches the
    /// readings' parallel-enum declaration order.
    pub rows: Vec<(String, String)>,
}

impl ObjectTypeKindTable {
    /// Boot table — must stay in sync with the parallel
    /// `Object Type Source Kind` and `Object Type` enum-value
    /// declarations in `readings/forml2-grammar.md`.
    pub fn boot() -> Self {
        ObjectTypeKindTable {
            rows: alloc::vec![
                ("Abstract Declaration".to_string(),    "abstract".to_string()),
                ("Partition Declaration".to_string(),   "abstract".to_string()),
                ("Entity Type Declaration".to_string(), "entity".to_string()),
                ("Value Type Declaration".to_string(),  "value".to_string()),
                ("Subtype Declaration".to_string(),     "entity".to_string()),
            ],
        }
    }

    /// Build the table from parallel `Object Type Source Kind` /
    /// `Object Type` enum-value declarations. Falls back to `boot()`
    /// on any mismatch.
    pub fn from_grammar_state(state: &Object) -> Self {
        let rows = read_parallel_enum_pair(
            state,
            "Object Type Source Kind",
            "Object Type",
        );
        match rows {
            Some(r) => ObjectTypeKindTable { rows: r },
            None => Self::boot(),
        }
    }

    /// Iterate `(kind, object-type)` pairs in cascade order so the
    /// caller can apply abstract-wins merge semantics.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.rows.iter().map(|(k, t)| (k.as_str(), t.as_str()))
    }
}

/// #789 — `unresolved_subclauses` filters title-case tokens that are
/// known prose words rather than nouns (articles, demonstratives,
/// quantifiers, control-flow keywords). The list lifts from an inline
/// const into a typed table that mirrors the `Prose Stopword` enum
/// declared in `readings/forml2-grammar.md`. `boot()` carries the
/// 12-word fallback so the bare-engine compile path stays working;
/// `from_grammar_state(state)` reads the runtime enum once the
/// metamodel is loaded. Same shape as `ObjectTypeKindTable` per #833.
#[derive(Debug, Clone)]
pub struct ProseStopwordTable {
    /// Title-case prose tokens that are NOT noun references. Order
    /// matches the `'If', 'When', 'Then', ...` declaration in
    /// readings/forml2-grammar.md so future agents can read either
    /// the const or the grammar and see the same sequence.
    pub rows: Vec<String>,
}

impl ProseStopwordTable {
    /// Boot table — must stay in sync with `Prose Stopword` enum-value
    /// declaration in `readings/forml2-grammar.md`. Twelve title-case
    /// tokens broken into four cohorts: control-flow (If/When/Then),
    /// demonstratives (That/This), articles (An/A/The), quantifiers
    /// (Each/Some/No/Every).
    pub fn boot() -> Self {
        ProseStopwordTable {
            rows: alloc::vec![
                "If".to_string(),    "When".to_string(),  "Then".to_string(),
                "That".to_string(),  "This".to_string(),
                "An".to_string(),    "A".to_string(),     "The".to_string(),
                "Each".to_string(),  "Some".to_string(),  "No".to_string(),
                "Every".to_string(),
            ],
        }
    }

    /// Build the table from the runtime `Prose Stopword` enum-value
    /// declaration. Falls back to `boot()` when the cell is empty
    /// (bare engine, no metamodel loaded).
    pub fn from_grammar_state(state: &Object) -> Self {
        let rows = read_enum_values(state, "Prose Stopword");
        if rows.is_empty() {
            Self::boot()
        } else {
            ProseStopwordTable { rows }
        }
    }

    /// Iterate the stopwords in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.rows.iter().map(|s| s.as_str())
    }

    /// Whole-word case-sensitive membership test. Mirrors the legacy
    /// `PROSE_STOPWORDS.iter().any(|s| *s == *w)` semantics.
    pub fn contains(&self, word: &str) -> bool {
        self.rows.iter().any(|s| s == word)
    }
}

/// #790 — `resolve_constraint_span_ft` strips a closed vocabulary of
/// deontic / quantifier prefixes from a constraint-span text fragment
/// before doing FT lookup. The vocabulary lifts from an inline
/// `.replace().replace()...` cascade into a typed
/// `ConstraintSpanPrefixTable` reading the `Constraint Span Prefix`
/// grammar enum declared in `readings/forml2-grammar.md`. Same shape
/// as `ProseStopwordTable` per #789 and `RingAdjectiveTable` per #791.
/// The `.replace` semantics (substring replace anywhere, not just
/// at start) are preserved by `strip_all_inline` — every entry is
/// removed wherever it occurs in declaration order.
#[derive(Debug, Clone)]
pub struct ConstraintSpanPrefixTable {
    /// The 11 deontic / quantifier prefixes, three cohorts:
    /// deontic operators ('It is obligatory that ' etc.),
    /// distributive quantifier ('Each ' / 'each '),
    /// cardinal/existential/negative ('at most one ' through 'no ').
    /// Order matches the legacy cascade order in
    /// `resolve_constraint_schema` so behavior round-trips.
    pub rows: Vec<String>,
}

impl ConstraintSpanPrefixTable {
    /// Boot table — must stay in sync with `Constraint Span Prefix`
    /// enum-value declaration in `readings/forml2-grammar.md`.
    pub fn boot() -> Self {
        ConstraintSpanPrefixTable {
            rows: alloc::vec![
                "It is obligatory that ".to_string(),
                "It is forbidden that ".to_string(),
                "It is permitted that ".to_string(),
                "Each ".to_string(),       "each ".to_string(),
                "at most one ".to_string(), "exactly one ".to_string(),
                "at least one ".to_string(), "some ".to_string(),
                "No ".to_string(),          "no ".to_string(),
            ],
        }
    }

    /// Build the table from the runtime `Constraint Span Prefix`
    /// enum-value declaration. Falls back to `boot()` when the cell
    /// is empty (bare engine, no metamodel loaded).
    pub fn from_grammar_state(state: &Object) -> Self {
        let rows = read_enum_values(state, "Constraint Span Prefix");
        if rows.is_empty() {
            Self::boot()
        } else {
            ConstraintSpanPrefixTable { rows }
        }
    }

    /// Iterate the prefixes in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.rows.iter().map(|s| s.as_str())
    }

    /// Apply every prefix as a substring `replace_all` in declaration
    /// order. Mirrors the legacy `.replace(...).replace(...)` cascade
    /// — substring replace anywhere, not just at start. Returns the
    /// fully-stripped text.
    pub fn strip_all(&self, text: &str) -> String {
        self.rows.iter().fold(text.to_string(),
            |acc, prefix| acc.replace(prefix.as_str(), ""))
    }
}

/// #791 — `strip_ring_annotation` recognizes a closed 8-token
/// vocabulary of bare ring adjectives — `irreflexive`, `asymmetric`,
/// `antisymmetric`, `symmetric`, `intransitive`, `transitive`,
/// `acyclic`, `reflexive` — that may appear in a trailing
/// `(<adjective>)` annotation on a multi-clause conditional ring
/// shape (e.g., `If some X R some Y then Y R X. (symmetric)`). The
/// vocabulary lifts from an inline `KINDS` const into a typed table
/// reading the `Ring Adjective` grammar enum declared in
/// `readings/forml2-grammar.md`. Mirrors `RingKindTable` (same eight
/// kinds, but the bare adjective form rather than the `is X` trailing
/// marker), and the table-shape conventions of `ProseStopwordTable`
/// per #789.
#[derive(Debug, Clone)]
pub struct RingAdjectiveTable {
    /// The eight bare ring adjectives. Order matches the
    /// `'irreflexive', 'asymmetric', ...` declaration in
    /// readings/forml2-grammar.md so future agents can read either
    /// the const or the grammar and see the same sequence. Note: this
    /// is NOT the `is X` form (`is irreflexive`) — that lives in
    /// `Ring Constraint Trailing Marker` and `RingKindTable`.
    pub rows: Vec<String>,
}

impl RingAdjectiveTable {
    /// Boot table — must stay in sync with `Ring Adjective` enum-value
    /// declaration in `readings/forml2-grammar.md`. Eight bare-form
    /// adjectives in the same declaration order as the legacy `KINDS`
    /// const in `strip_ring_annotation`.
    pub fn boot() -> Self {
        RingAdjectiveTable {
            rows: alloc::vec![
                "irreflexive".to_string(),   "asymmetric".to_string(),
                "antisymmetric".to_string(), "symmetric".to_string(),
                "intransitive".to_string(),  "transitive".to_string(),
                "acyclic".to_string(),       "reflexive".to_string(),
            ],
        }
    }

    /// Build the table from the runtime `Ring Adjective` enum-value
    /// declaration. Falls back to `boot()` when the cell is empty
    /// (bare engine, no metamodel loaded).
    pub fn from_grammar_state(state: &Object) -> Self {
        let rows = read_enum_values(state, "Ring Adjective");
        if rows.is_empty() {
            Self::boot()
        } else {
            RingAdjectiveTable { rows }
        }
    }

    /// Iterate the adjectives in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.rows.iter().map(|s| s.as_str())
    }

    /// Whole-word case-sensitive membership test. Mirrors the legacy
    /// `KINDS.iter().any(|k| *k == kind)` semantics in
    /// `strip_ring_annotation`.
    pub fn contains(&self, word: &str) -> bool {
        self.rows.iter().any(|s| s == word)
    }
}

/// #783 first slice — `is_word_comparator_clause` in `parse_forml2.rs`
/// scans for an inline 8-entry `COMPARATORS` const (` exceeds `,
/// ` is greater than `, ` is less than `, ` is at least `,
/// ` is at most `, ` is more than `, ` equals `, ` is equal to `).
/// Each entry was stored already wrapped in spaces so substring search
/// matched on word boundaries. The lift moves the comparator vocabulary
/// to a typed `WordComparatorTable` reading the `Word Comparator`
/// grammar enum. Boot-table rows store the bare phrase (no surrounding
/// spaces) — same convention as `RingAdjectiveTable` per #791. The
/// caller adds spaces at use site to preserve word-boundary semantics.
#[derive(Debug, Clone)]
pub struct WordComparatorTable {
    /// The eight word-form comparator phrases. Order matches the
    /// `'exceeds', 'is greater than', ...` declaration in
    /// readings/forml2-grammar.md and the legacy `COMPARATORS` const
    /// in `is_word_comparator_clause` so first-match-wins iteration
    /// behavior round-trips.
    pub rows: Vec<String>,
}

impl WordComparatorTable {
    /// Boot table — must stay in sync with `Word Comparator` enum-value
    /// declaration in `readings/forml2-grammar.md`. Eight phrases in
    /// the same declaration order as the legacy `COMPARATORS` const.
    pub fn boot() -> Self {
        WordComparatorTable {
            rows: alloc::vec![
                "exceeds".to_string(),
                "is greater than".to_string(),
                "is less than".to_string(),
                "is at least".to_string(),
                "is at most".to_string(),
                "is more than".to_string(),
                "equals".to_string(),
                "is equal to".to_string(),
            ],
        }
    }

    /// Build the table from the runtime `Word Comparator` enum-value
    /// declaration. Falls back to `boot()` when the cell is empty
    /// (bare engine, no metamodel loaded).
    pub fn from_grammar_state(state: &Object) -> Self {
        let rows = read_enum_values(state, "Word Comparator");
        if rows.is_empty() {
            Self::boot()
        } else {
            WordComparatorTable { rows }
        }
    }

    /// Iterate the comparators in declaration order. Phrases lack
    /// surrounding spaces; use `spaced()` for the substring-search form.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.rows.iter().map(|s| s.as_str())
    }
}

/// #783 second slice — `is_range_filter_clause` in `parse_forml2.rs`
/// scans for an inline 3-entry `RANGE_OPS` const (` within `,
/// ` before `, ` after `). Each entry was stored already wrapped in
/// spaces so substring search matched on word boundaries. The lift
/// moves the operator vocabulary to a typed `RangeOperatorTable`
/// reading the `Range Operator` grammar enum. Same shape as
/// `WordComparatorTable` per the first slice — bare phrases in `rows`,
/// the caller adds spaces at use site.
#[derive(Debug, Clone)]
pub struct RangeOperatorTable {
    /// The three range-filter operator phrases. Order matches the
    /// `'within', 'before', 'after'` declaration in
    /// readings/forml2-grammar.md and the legacy `RANGE_OPS` const
    /// in `is_range_filter_clause` so first-match-wins iteration
    /// behavior round-trips.
    pub rows: Vec<String>,
}

impl RangeOperatorTable {
    /// Boot table — must stay in sync with `Range Operator` enum-value
    /// declaration in `readings/forml2-grammar.md`. Three phrases in
    /// the same declaration order as the legacy `RANGE_OPS` const.
    pub fn boot() -> Self {
        RangeOperatorTable {
            rows: alloc::vec![
                "within".to_string(),
                "before".to_string(),
                "after".to_string(),
            ],
        }
    }

    /// Build the table from the runtime `Range Operator` enum-value
    /// declaration. Falls back to `boot()` when the cell is empty
    /// (bare engine, no metamodel loaded).
    pub fn from_grammar_state(state: &Object) -> Self {
        let rows = read_enum_values(state, "Range Operator");
        if rows.is_empty() {
            Self::boot()
        } else {
            RangeOperatorTable { rows }
        }
    }

    /// Iterate the operators in declaration order. Phrases lack
    /// surrounding spaces; the caller adds them for substring search.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.rows.iter().map(|s| s.as_str())
    }
}

/// Sweep-1 lift of the FORML2 single-quoted literal escape convention
/// (#844 enabling work). Stage-1's `extract_following_literal_span` in
/// `parse_forml2_stage1.rs` historically used `body.find('\'')` to
/// locate the close-quote — naive scan with no escape handling. That
/// truncated `'doesn''t work'` to literal=`doesn`, silently dropping
/// every instance fact whose value carried an apostrophe.
///
/// The lift moves the escape vocabulary to a typed `QuoteEscapeTable`
/// reading the `Quote Escape` value type from
/// `readings/forml2-grammar.md`. Boot enables the SQL-style
/// `'doubled-quote'` convention (`''` decodes to `'`). Same shape as
/// `WordComparatorTable` per #783's first slice — bare phrases in
/// `rows`, semantic methods (`find_close`, `decode`) consult the
/// table at every call so swapping the boot for a richer grammar
/// declaration adjusts parser behavior with no Rust change.
#[derive(Debug, Clone)]
pub struct QuoteEscapeTable {
    /// Names of the escape conventions enabled in this table. Order
    /// matches the `'doubled-quote', ...` declaration in
    /// readings/forml2-grammar.md. Currently the only convention the
    /// parser honors is `'doubled-quote'`; future conventions
    /// (e.g. `'backslash-quote'`) extend `find_close` / `decode`.
    pub rows: Vec<String>,
}

impl QuoteEscapeTable {
    /// Boot table — must stay in sync with `Quote Escape` enum-value
    /// declaration in `readings/forml2-grammar.md`. One convention:
    /// SQL-style doubled quote (`''` → `'`).
    pub fn boot() -> Self {
        QuoteEscapeTable {
            rows: alloc::vec!["doubled-quote".to_string()],
        }
    }

    /// Build the table from the runtime `Quote Escape` enum-value
    /// declaration. Falls back to `boot()` when the cell is empty.
    pub fn from_grammar_state(state: &Object) -> Self {
        let rows = read_enum_values(state, "Quote Escape");
        if rows.is_empty() {
            Self::boot()
        } else {
            QuoteEscapeTable { rows }
        }
    }

    /// Iterate the conventions in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.rows.iter().map(|s| s.as_str())
    }

    /// Whether the doubled-quote escape (`''` → `'`) is enabled.
    pub fn allows_doubled_quote(&self) -> bool {
        self.rows.iter().any(|r| r == "doubled-quote")
    }

    /// Find the close-quote position in the body of a single-quoted
    /// literal. `body` is the slice immediately AFTER the opening
    /// `'`. Returns the byte offset of the close `'` within `body`,
    /// or `None` if the literal is unterminated. With doubled-quote
    /// enabled, runs of `''` are treated as escaped apostrophe and
    /// scanned past — only a lone `'` counts as the close.
    pub fn find_close(&self, body: &str) -> Option<usize> {
        let bytes = body.as_bytes();
        let allow_dq = self.allows_doubled_quote();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\'' {
                if allow_dq && i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    i += 2;
                    continue;
                }
                return Some(i);
            }
            i += 1;
        }
        None
    }

    /// Decode escapes in a literal body (the substring strictly between
    /// the opening and closing quotes). With doubled-quote enabled,
    /// every `''` collapses to a single `'`. Future conventions extend
    /// this with their own decoders.
    pub fn decode(&self, body: &str) -> alloc::string::String {
        if self.allows_doubled_quote() {
            body.replace("''", "'")
        } else {
            body.to_string()
        }
    }
}

/// #788 — `parse_deontic_text_predicate` matches one of four suffixes
/// on a deontic-constraint prefix (` ends with`, ` does not end with`,
/// ` starts with`, ` does not start with`) and decodes (kind, negated).
/// The 4-suffix cascade lifts to a 3-tuple typed table mirroring
/// `SetConstraintKindTable` per #781 — same parallel-enum shape, same
/// boot/from_grammar_state/iter pattern, plus a `match_suffix` accessor
/// for the per-call use site.
#[derive(Debug, Clone)]
pub struct DeonticPredicateOperatorTable {
    /// Triples of `(suffix, kind, negated)`. Suffix carries its leading
    /// space so `match_suffix` matches the same `prefix.strip_suffix(...)`
    /// shape the legacy cascade used. Kind values are the canonical
    /// `DeonticPredicate` discriminant tags ('ends_with' / 'starts_with').
    /// Negated parses from string 'true' / 'false'.
    pub rows: Vec<(String, String, bool)>,
}

impl DeonticPredicateOperatorTable {
    /// Boot table — must stay in sync with the parallel
    /// `Deontic Predicate Operator` / `Deontic Predicate Operator Kind`
    /// / `Deontic Predicate Operator Negated` enum-value declarations
    /// in `readings/forml2-grammar.md`.
    pub fn boot() -> Self {
        DeonticPredicateOperatorTable {
            rows: alloc::vec![
                (" ends with".to_string(),           "ends_with".to_string(),   false),
                (" does not end with".to_string(),   "ends_with".to_string(),   true),
                (" starts with".to_string(),         "starts_with".to_string(), false),
                (" does not start with".to_string(), "starts_with".to_string(), true),
            ],
        }
    }

    /// Build the table from the runtime parallel-enum declarations.
    /// Falls back to `boot()` when any list is empty or lengths
    /// disagree — matches the contract from `read_parallel_enum_triple`.
    pub fn from_grammar_state(state: &Object) -> Self {
        match read_parallel_enum_triple(
            state,
            "Deontic Predicate Operator",
            "Deontic Predicate Operator Kind",
            "Deontic Predicate Operator Negated",
        ) {
            Some(triples) => DeonticPredicateOperatorTable {
                rows: triples.into_iter().map(|(suffix, kind, negated)| {
                    (suffix, kind, negated == "true")
                }).collect(),
            },
            None => Self::boot(),
        }
    }

    /// Iterate `(suffix, kind, negated)` triples in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str, bool)> {
        self.rows.iter().map(|(s, k, n)| (s.as_str(), k.as_str(), *n))
    }

    /// Try each suffix in declaration order; on first match return
    /// `(prefix-without-suffix, kind, negated)`. Mirrors the legacy
    /// `prefix.strip_suffix(...)` cascade in `parse_deontic_text_predicate`.
    pub fn match_suffix<'a>(&self, prefix: &'a str) -> Option<(&'a str, &str, bool)> {
        for (suffix, kind, negated) in self.rows.iter() {
            if let Some(rest) = prefix.strip_suffix(suffix.as_str()) {
                return Some((rest.trim_end(), kind.as_str(), *negated));
            }
        }
        None
    }
}

/// #786 — `encode_conditional_ring_pattern` fails closed when the
/// consequent contains common English negation prose that ISN'T one
/// of the canonical FORML2 / NORMA / Halpin markers. The 10-hint list
/// lifts to a typed table mirroring `ProseStopwordTable` per #789.
#[derive(Debug, Clone)]
pub struct NonCanonicalNegationHintTable {
    /// Whitespace-padded hint substrings checked anywhere in the
    /// consequent. Order matches the historic const.
    pub rows: Vec<String>,
}

impl NonCanonicalNegationHintTable {
    /// Boot table — must stay in sync with the
    /// `Non Canonical Negation Hint` enum-value declaration in
    /// `readings/forml2-grammar.md`.
    pub fn boot() -> Self {
        NonCanonicalNegationHintTable {
            rows: alloc::vec![
                " does not ".to_string(),  " do not ".to_string(),  " did not ".to_string(),
                " cannot ".to_string(),    " can not ".to_string(),
                " must not ".to_string(),  " will not ".to_string(), " would not ".to_string(),
                " never ".to_string(),     " no longer ".to_string(),
            ],
        }
    }

    /// Build from runtime `Non Canonical Negation Hint` enum-value
    /// declaration. Falls back to `boot()` when the cell is empty.
    pub fn from_grammar_state(state: &Object) -> Self {
        let rows = read_enum_values(state, "Non Canonical Negation Hint");
        if rows.is_empty() { Self::boot() } else { NonCanonicalNegationHintTable { rows } }
    }

    /// Iterate hints in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.rows.iter().map(|s| s.as_str())
    }

    /// True if any hint occurs as a substring of `text`. Mirrors the
    /// legacy `NON_CANONICAL_NEGATION_HINTS.iter().any(|n| consequent.contains(n))`
    /// semantics.
    pub fn any_match(&self, text: &str) -> bool {
        self.rows.iter().any(|hint| text.contains(hint.as_str()))
    }
}

/// #786 — `encode_conditional_ring_pattern` dispatches a 5-tuple of
/// boolean signals (has_and, impossible, itself_in_consequent,
/// is_not_in_antecedent, is_not_in_consequent) to one of seven
/// `Conditional Ring Pattern` names. The legacy match arms include
/// wildcards (`_`); this table preserves them as `Option<bool>`:
/// `Some(b)` = required match, `None` = wildcard. The matcher picks
/// the first row whose every `Some(b)` matches the input — same
/// first-match-wins semantics as the legacy match.
#[derive(Debug, Clone)]
pub struct ConditionalRingPatternTable {
    /// 7 rows of (5-signal mask, pattern-name). Mask order matches
    /// the function signature: (has_and, impossible,
    /// itself_in_consequent, is_not_in_antecedent, is_not_in_consequent).
    pub rows: Vec<([Option<bool>; 5], String)>,
}

impl ConditionalRingPatternTable {
    /// Boot table — must stay in sync with the legacy match arms in
    /// `encode_conditional_ring_pattern`. `from_grammar_state` is
    /// deferred: representing 5-column wildcard masks as parallel
    /// enums is awkward and 7 callers don't justify the extra
    /// scaffolding yet.
    pub fn boot() -> Self {
        ConditionalRingPatternTable {
            rows: alloc::vec![
                // (true, true, _, true, _) → AT
                ([Some(true),  Some(true),  None,        Some(true),  None],
                    "and+impossible+isnot-ante".to_string()),
                // (true, true, _, false, _) → IT
                ([Some(true),  Some(true),  None,        Some(false), None],
                    "and+impossible".to_string()),
                // (true, false, _, _, _) → TR
                ([Some(true),  Some(false), None,        None,        None],
                    "and".to_string()),
                // (false, true, false, _, _) → AS via "impossible"
                ([Some(false), Some(true),  Some(false), None,        None],
                    "impossible".to_string()),
                // (false, false, false, _, true) → AS via "is not"
                ([Some(false), Some(false), Some(false), None,        Some(true)],
                    "isnot-conse".to_string()),
                // (false, false, true, _, _) → RF
                ([Some(false), Some(false), Some(true),  None,        None],
                    "itself-conse".to_string()),
                // (false, false, false, _, false) → SY
                ([Some(false), Some(false), Some(false), None,        Some(false)],
                    "plain".to_string()),
            ],
        }
    }

    /// First-row-match dispatch over the five signals. Returns the
    /// pattern name if a row matches, None otherwise (e.g. impossible
    /// combined with itself_in_consequent has no recognised shape).
    pub fn match_signals(
        &self,
        has_and: bool,
        impossible: bool,
        itself_in_consequent: bool,
        is_not_in_antecedent: bool,
        is_not_in_consequent: bool,
    ) -> Option<&str> {
        let input = [
            has_and, impossible, itself_in_consequent,
            is_not_in_antecedent, is_not_in_consequent,
        ];
        for (mask, name) in self.rows.iter() {
            if mask.iter().zip(input.iter()).all(|(m, &i)| match m {
                Some(b) => *b == i,
                None    => true,
            }) {
                return Some(name.as_str());
            }
        }
        None
    }
}

/// Stage-2 set constraint kind dispatch — `<classification-kind>` →
/// ORM 2 set-constraint kind code + arbitration-rule name. Mirrors
/// `CardinalityConstraintKindTable` plus a third column for the
/// per-kind arbitration predicate. Five rows in declaration order:
/// Equality / Subset / Exclusive-Or / Or / Exclusion Constraint →
/// `EQ` / `SS` / `XO` / `OR` / `XC`. Subset uses
/// `antecedent_diversity_min_2`; everyone else uses
/// `derivation_rule_wins`. Order matters — translate_set_constraints
/// picks the first matching row, mirroring the legacy if-else cascade.
#[derive(Debug, Clone)]
pub struct SetConstraintKindTable {
    /// Triples of `(classification-kind, kind-code, arbitration-rule)`
    /// such as `("Equality Constraint", "EQ", "derivation_rule_wins")`.
    /// Order matches the readings' parallel-enum declaration order.
    pub rows: Vec<(String, String, String)>,
}

impl SetConstraintKindTable {
    /// Boot table — must stay in sync with the parallel
    /// `Set Constraint Kind` / `Set Constraint Kind Code` /
    /// `Set Constraint Arbitration Rule` enum-value declarations in
    /// `readings/forml2-grammar.md`.
    pub fn boot() -> Self {
        SetConstraintKindTable {
            rows: alloc::vec![
                ("Equality Constraint".to_string(),     "EQ".to_string(), "derivation_rule_wins".to_string()),
                ("Subset Constraint".to_string(),       "SS".to_string(), "antecedent_diversity_min_2".to_string()),
                ("Exclusive-Or Constraint".to_string(), "XO".to_string(), "derivation_rule_wins".to_string()),
                ("Or Constraint".to_string(),           "OR".to_string(), "derivation_rule_wins".to_string()),
                ("Exclusion Constraint".to_string(),    "XC".to_string(), "derivation_rule_wins".to_string()),
            ],
        }
    }

    /// Build the table from parallel `Set Constraint Kind` /
    /// `Set Constraint Kind Code` / `Set Constraint Arbitration Rule`
    /// enum-value declarations. Falls back to `boot()` on any mismatch.
    pub fn from_grammar_state(state: &Object) -> Self {
        // #781: 3-tuple parallel-enum read via the shared helper so
        // future 3-tuple tables can reuse the same length-check +
        // zip + fallback semantics.
        match read_parallel_enum_triple(
            state,
            "Set Constraint Kind",
            "Set Constraint Kind Code",
            "Set Constraint Arbitration Rule",
        ) {
            Some(rows) => SetConstraintKindTable { rows },
            None => Self::boot(),
        }
    }

    /// Iterate `(kind, code, arbitration-rule)` triples in
    /// declaration order so the caller can apply per-kind
    /// arbitration in cascade order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str, &str)> {
        self.rows.iter().map(|(k, c, r)| (k.as_str(), c.as_str(), r.as_str()))
    }
}

/// Per-kind arbitration predicate: returns `true` when the Statement
/// should be skipped (a different translator owns it). Each predicate
/// is registered in `set_constraint_arbitration_registry` keyed by the
/// rule name declared in the readings parallel-enum.
pub type SetConstraintArbitrationFn = fn(text: &str, declared_nouns: &[String], idx: &StmtIndex, stmt_id: &str) -> bool;

/// Translator pre-gate predicate: returns `true` when the Statement
/// should be skipped before the translator's per-kind dispatch. Used
/// uniformly across all of a translator's kinds (e.g. cardinality's
/// "skip if Derivation Rule" applies to FC + UC + MC alike).
pub type CardinalityArbitrationFn = fn(state: &Object, idx: &StmtIndex, stmt_id: &str) -> bool;

/// Skip when the Statement is also classified as Derivation Rule —
/// `iff` makes the whole sentence a rule even when it incidentally
/// contains a quantifier.
fn skip_card_on_derivation_rule(_state: &Object, idx: &StmtIndex, stmt_id: &str) -> bool {
    classifications_contains(idx, stmt_id, "Derivation Rule")
}

/// Skip when the Statement carries a Deontic Operator (`It is
/// forbidden/obligatory/permitted that …`) — the inner quantifier
/// is part of a deontic over the body, not an alethic UC/MC.
fn skip_card_on_deontic_operator(state: &Object, _idx: &StmtIndex, stmt_id: &str) -> bool {
    deontic_operator_for(state, stmt_id).is_some()
}

/// Registry of cardinality pre-gate name → predicate. Both predicates
/// are uniform skip rules applied before the per-kind FC/UC/MC
/// dispatch in translate_cardinality_constraints.
pub fn cardinality_arbitration_registry() -> hashbrown::HashMap<&'static str, CardinalityArbitrationFn> {
    let mut m: hashbrown::HashMap<&'static str, CardinalityArbitrationFn> = hashbrown::HashMap::new();
    m.insert("derivation_rule_wins", skip_card_on_derivation_rule as CardinalityArbitrationFn);
    m.insert("deontic_operator_wins", skip_card_on_deontic_operator as CardinalityArbitrationFn);
    m
}

/// Skip when the Statement is also classified as Derivation Rule.
/// Used for EQ / XO / OR / XC where the `iff` keyword that produces
/// these classifications also produces a Derivation Rule
/// classification, and translate_derivation_rules wins.
fn skip_on_derivation_rule(_text: &str, _nouns: &[String], idx: &StmtIndex, stmt_id: &str) -> bool {
    classifications_contains(idx, stmt_id, "Derivation Rule")
}

/// Skip when the antecedent has fewer than 2 distinct declared nouns.
/// Used for SS where the synthetic `if some then that` constraint
/// keyword also fires for single-antecedent-noun cases that legacy
/// hands to translate_derivation_rules.
fn skip_on_low_antecedent_diversity(text: &str, declared_nouns: &[String], _idx: &StmtIndex, _stmt_id: &str) -> bool {
    antecedent_distinct_nouns(text, declared_nouns) < 2
}

/// Registry of arbitration-rule name → predicate. The string keys must
/// match the third column of `SetConstraintKindTable` so the
/// `set_constraint_arbitration_registry_covers_kind_table` regression
/// catches typos and renames.
pub fn set_constraint_arbitration_registry() -> hashbrown::HashMap<&'static str, SetConstraintArbitrationFn> {
    let mut m: hashbrown::HashMap<&'static str, SetConstraintArbitrationFn> = hashbrown::HashMap::new();
    m.insert("derivation_rule_wins",        skip_on_derivation_rule        as SetConstraintArbitrationFn);
    m.insert("antecedent_diversity_min_2",  skip_on_low_antecedent_diversity as SetConstraintArbitrationFn);
    m
}

/// Stage-2 cardinality constraint kind dispatch — `<classification-kind>`
/// → ORM 2 cardinality kind code. Mirrors `RingKindTable` (#713) but
/// keyed on the Statement Classification name rather than a trailing
/// marker. Three rows: Frequency / Uniqueness / Mandatory Role
/// Constraint → `FC` / `UC` / `MC`.
#[derive(Debug, Clone)]
pub struct CardinalityConstraintKindTable {
    /// Pairs of `(classification-kind, kind-code)` such as
    /// `("Frequency Constraint", "FC")`. Order matches the readings'
    /// parallel-enum declaration order.
    pub rows: Vec<(String, String)>,
}

impl CardinalityConstraintKindTable {
    /// Boot table — must stay in sync with the parallel
    /// `Cardinality Constraint Kind` and `Cardinality Constraint Kind
    /// Code` enum-value declarations in `readings/forml2-grammar.md`.
    pub fn boot() -> Self {
        CardinalityConstraintKindTable {
            rows: alloc::vec![
                ("Frequency Constraint".to_string(),      "FC".to_string()),
                ("Uniqueness Constraint".to_string(),     "UC".to_string()),
                ("Mandatory Role Constraint".to_string(), "MC".to_string()),
            ],
        }
    }

    /// Build the table from parallel `Cardinality Constraint Kind` /
    /// `Cardinality Constraint Kind Code` enum-value declarations.
    /// Falls back to `boot()` on any mismatch.
    pub fn from_grammar_state(state: &Object) -> Self {
        let rows = read_parallel_enum_pair(
            state,
            "Cardinality Constraint Kind",
            "Cardinality Constraint Kind Code",
        );
        match rows {
            Some(r) => CardinalityConstraintKindTable { rows: r },
            None => Self::boot(),
        }
    }

    /// Return the kind code for the first registered classification
    /// the statement is classified as, or `None` if none match.
    pub fn code_for_statement(&self, idx: &StmtIndex, stmt_id: &str) -> Option<&str> {
        self.rows.iter()
            .find(|(kind, _)| classifications_contains(idx, stmt_id, kind))
            .map(|(_, code)| code.as_str())
    }
}

/// Stage-2 translator dispatch — `<classification-kind>` → translator
/// name(s). Mirrors `RingKindTable` / `ConditionalRingMatrix` /
/// `DeonticShapeTable` (#713) but the relation is many-to-many: e.g.,
/// Subtype Declaration is handled by both `translate_nouns` and
/// `translate_subtypes`, and `translate_set_constraints` handles five
/// kinds.
///
/// Per AREST.tex §3 (eq:sys) — "the entity handles the dispatch, not
/// the system function. New operations are registered in DEFS without
/// modifying any entity." Stage-2's main dispatch loop should consult
/// this table to discover which translators apply to a given Statement
/// Classification, rather than hardcoding 19 per-kind branches. See
/// task #833 for context.
#[derive(Debug, Clone)]
pub struct StatementTranslatorTable {
    /// `(classification-kind, translator-name)` rows in declaration
    /// order. Multiple rows per kind are allowed (M:N relation).
    pub rows: Vec<(String, String)>,
}

impl StatementTranslatorTable {
    /// Boot table — must stay in sync with the
    /// `Classification_has_Translator` cell built from
    /// `Classification 'X' is translated by Translator 'Y'.` instance
    /// facts in `readings/forml2-grammar.md`.
    pub fn boot() -> Self {
        StatementTranslatorTable {
            rows: alloc::vec![
                ("Entity Type Declaration".to_string(),    "translate_nouns".to_string()),
                ("Value Type Declaration".to_string(),     "translate_nouns".to_string()),
                ("Subtype Declaration".to_string(),        "translate_nouns".to_string()),
                ("Subtype Declaration".to_string(),        "translate_subtypes".to_string()),
                ("Abstract Declaration".to_string(),       "translate_nouns".to_string()),
                ("Partition Declaration".to_string(),      "translate_nouns".to_string()),
                ("Partition Declaration".to_string(),      "translate_partitions".to_string()),
                ("Enum Values Declaration".to_string(),    "translate_enum_values".to_string()),
                ("Instance Fact".to_string(),              "translate_instance_facts".to_string()),
                ("Fact Type Reading".to_string(),          "translate_fact_types".to_string()),
                ("Fact Type Reading".to_string(),          "translate_derivation_mode_facts".to_string()),
                ("Derivation Rule".to_string(),            "translate_derivation_rules".to_string()),
                ("Uniqueness Constraint".to_string(),      "translate_cardinality_constraints".to_string()),
                ("Mandatory Role Constraint".to_string(),  "translate_cardinality_constraints".to_string()),
                ("Frequency Constraint".to_string(),       "translate_cardinality_constraints".to_string()),
                ("Ring Constraint".to_string(),            "translate_ring_constraints".to_string()),
                ("Subset Constraint".to_string(),          "translate_set_constraints".to_string()),
                ("Equality Constraint".to_string(),        "translate_set_constraints".to_string()),
                ("Exclusion Constraint".to_string(),       "translate_set_constraints".to_string()),
                ("Exclusive-Or Constraint".to_string(),    "translate_set_constraints".to_string()),
                ("Or Constraint".to_string(),              "translate_set_constraints".to_string()),
                ("Value Constraint".to_string(),           "translate_value_constraints".to_string()),
                ("Deontic Constraint".to_string(),         "translate_deontic_constraints".to_string()),
            ],
        }
    }

    /// Build the table from the
    /// `Classification_has_Translator` cell of a parsed
    /// grammar state. Falls back to `boot()` when the cell is empty
    /// or missing — same defensive pattern as `RingKindTable`.
    pub fn from_grammar_state(state: &Object) -> Self {
        let cell = fetch_or_phi("Classification_has_Translator", state);
        let facts = match cell.as_seq() {
            Some(s) => s,
            None => return Self::boot(),
        };
        let mut rows: Vec<(String, String)> = Vec::new();
        for f in facts.iter() {
            let kind = match binding(f, "Classification") {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => continue,
            };
            let translator = match binding(f, "Translator") {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => continue,
            };
            rows.push((kind, translator));
        }
        if rows.is_empty() {
            Self::boot()
        } else {
            StatementTranslatorTable { rows }
        }
    }

    /// All translator names registered for `kind`, in declaration
    /// order. Empty `Vec` if the kind has no translators registered.
    pub fn translators_for(&self, kind: &str) -> Vec<&str> {
        self.rows.iter()
            .filter(|(k, _)| k == kind)
            .map(|(_, t)| t.as_str())
            .collect()
    }

    /// Inverse lookup: all classification kinds that `translator` is
    /// registered to handle, in declaration order. Empty `Vec` means
    /// no kinds dispatch to this translator (likely a bug — either an
    /// unregistered translator or a typo'd name).
    pub fn kinds_for(&self, translator: &str) -> Vec<&str> {
        self.rows.iter()
            .filter(|(_, t)| t == translator)
            .map(|(k, _)| k.as_str())
            .collect()
    }

    /// All distinct kinds in the table, in first-occurrence order.
    pub fn kinds(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for (k, _) in &self.rows {
            if !out.iter().any(|&x| x == k.as_str()) {
                out.push(k.as_str());
            }
        }
        out
    }
}

/// Read a single enum-value list from the EnumValues cell of a parsed
/// grammar state, keyed by `noun: <type_name>`. Returns an empty
/// `Vec` if no row matches.
fn read_enum_values(state: &Object, type_name: &str) -> Vec<String> {
    let cell = fetch_or_phi("EnumValues", state);
    let facts = match cell.as_seq() {
        Some(s) => s,
        None => return Vec::new(),
    };
    for f in facts.iter() {
        if binding(f, "noun") != Some(type_name) { continue; }
        return (0..)
            .map_while(|i| {
                let key = alloc::format!("value{i}");
                binding(f, &key).map(String::from)
            })
            .collect();
    }
    Vec::new()
}

/// Read two parallel enum-value lists and zip them index-wise.
/// Returns `None` if either list is empty or the lengths don't match;
/// callers fall back to `boot()` so a missing or malformed declaration
/// can't silently truncate the table.
fn read_parallel_enum_pair(
    state: &Object,
    left_type: &str,
    right_type: &str,
) -> Option<Vec<(String, String)>> {
    let left = read_enum_values(state, left_type);
    let right = read_enum_values(state, right_type);
    if left.len() == right.len() && !left.is_empty() {
        Some(left.into_iter().zip(right).collect())
    } else {
        None
    }
}

/// Read three parallel enum-value lists and zip them index-wise.
/// Returns `None` if any list is empty or the lengths don't match;
/// callers fall back to `boot()` so a missing or malformed declaration
/// can't silently truncate the table. #781 — same shape as
/// `read_parallel_enum_pair` for the 3-tuple case (kind / code /
/// arbitration-rule). Used by `SetConstraintKindTable::from_grammar_state`.
fn read_parallel_enum_triple(
    state: &Object,
    first_type: &str,
    second_type: &str,
    third_type: &str,
) -> Option<Vec<(String, String, String)>> {
    let first = read_enum_values(state, first_type);
    let second = read_enum_values(state, second_type);
    let third = read_enum_values(state, third_type);
    if first.len() == second.len()
        && first.len() == third.len()
        && !first.is_empty()
    {
        Some(first.into_iter()
            .zip(second)
            .zip(third)
            .map(|((a, b), c)| (a, b, c))
            .collect())
    } else {
        None
    }
}

/// Classify every Statement in `statements_state` using the grammar
/// rules in `grammar_state`. Returns a new state identical to
/// `statements_state` plus a populated `Statement_has_Classification`
/// cell.
pub fn classify_statements(statements_state: &Object, grammar_state: &Object) -> Object {
    // Trace gate — under std the AREST_STAGE12_TRACE env var enables
    // detailed timing telemetry; under no_std (kernel) it's a const
    // false so the eprintln branches compile out entirely. The kernel
    // gets equivalent observability via the `check` system verb that
    // pipes diag! through its serial sink.
    #[cfg(not(feature = "no_std"))]
    let trace = std::env::var("AREST_STAGE12_TRACE").is_ok();
    #[cfg(feature = "no_std")]
    let trace = false;
    let tc0 = Instant::now();
    // Merge Stage-1 statement cells with grammar cells so
    // `compile_to_defs_state` sees both the nouns/fact-types/rules
    // declared by the grammar and the Statement facts they apply to.
    let merged = crate::ast::merge_states(statements_state, grammar_state);
    if trace { crate::diag!("  [cls] merge: {:?}", tc0.elapsed()); }
    // Grammar defs are pure functions of forml2-grammar.md — cached
    // in `GRAMMAR_CACHE` at first access. Stage-1 never populates the
    // DerivationRule cell (user rules stay in Statement form until
    // translate_derivation_rules runs after classification), so the
    // cached grammar-only defs match a fresh `compile_to_defs_state`
    // of the merged state at this call site.
    let (classifier_defs, classifier_antecedents, base_keys): (
        &Vec<(String, crate::ast::Func)>,
        &Vec<Vec<String>>,
        Option<&hashbrown::HashSet<u64>>,
    ) = match cached_grammar() {
        Ok((_, d, a, k)) => (d, a, Some(k)),
        Err(_) => {
            // Fallback: if grammar cache failed, run nothing (classify
            // is a no-op and translators will see an un-classified
            // state — the caller's error path handles this). `spin::Once`
            // (alloc-compatible) instead of `std::sync::OnceLock` so the
            // no_std build resolves the same path.
            static EMPTY_DEFS: spin::Once<
                (Vec<(String, crate::ast::Func)>, Vec<Vec<String>>)
            > = spin::Once::new();
            let (d, a) = EMPTY_DEFS.call_once(|| (Vec::new(), Vec::new()));
            (d, a, None)
        }
    };
    // `merged` already contains the cached expanded grammar state
    // (grammar + fixpoint of implicit compile-emitted derivations);
    // the only defs we still need to fire at call time are the
    // classifier Natives, which run semi-naive. Skip `defs_to_state`
    // — the expanded grammar_state has them already.
    let deriv: Vec<(&str, &crate::ast::Func, Option<&[String]>)> = classifier_defs.iter()
        .zip(classifier_antecedents.iter())
        .map(|((n, f), a)| (n.as_str(), f, Some(a.as_slice())))
        .collect();
    let t2 = Instant::now();
    // Grammar classification rules stratify in depth 2: round 1 emits
    // base classifications from Stage-1 `Statement_has_*` tokens;
    // round 2 fires the single
    // `Value Constraint iff Classification 'Enum Values Declaration'`
    // rule (forml2-grammar.md:139) over round 1's output. The
    // semi-naive chainer uses per-rule antecedent cells to skip
    // unchained rules in round 2 — with grammar the only cell that
    // changes between rounds is `Statement_has_Classification`, so
    // only the one chaining rule runs.
    // Seed the chainer's `existing_keys` with the cached grammar key
    // set plus the user-statement keys. The statement side is tiny
    // (~100-300 facts for typical input vs. ~4000 for grammar), so
    // hashing only that portion is substantially cheaper than
    // re-hashing the whole merged state.
    let initial_keys = base_keys.map(|gk| {
        let stmt_keys = crate::evaluate::state_keys(statements_state);
        let mut combined = gk.clone();
        combined.extend(stmt_keys.into_iter());
        combined
    });
    let (final_state, _) = crate::evaluate::forward_chain_defs_state_semi_naive_with_base_keys(
        &deriv, &merged, 2, initial_keys);
    if trace { crate::diag!("  [cls] forward_chain ({} defs): {:?}",
        deriv.len(), t2.elapsed()); }
    final_state
}

/// Translate noun-shaping classifications into `Noun` cell facts.
/// #280b step 1.
///
/// Considers every Statement that carries a Head Noun plus one of
/// these classifications:
///
/// - `Entity Type Declaration` → objectType = "entity".
/// - `Value Type Declaration`  → objectType = "value".
/// - `Abstract Declaration`    → objectType = "abstract" (overrides
///   entity/value per the existing parser: `Foo is abstract` on a
///   line after `Foo is an entity type` wins).
///
/// Grouped by Head Noun: one Noun fact per distinct name, with the
/// most specific objectType across its classifications applied.
pub fn translate_nouns(classified_state: &Object, idx: &StmtIndex) -> Vec<Object> {
    use alloc::collections::BTreeMap;
    let statement_ids = collect_statement_ids(idx);
    let mut by_noun: BTreeMap<String, String> = BTreeMap::new();
    // Side-tables, keyed by noun name: reference scheme columns, enum
    // values, supertype. Legacy emits all three as bindings on the
    // Noun fact itself; rmap / openapi / the ref-scheme-driven OpenAPI
    // schema generator all read them from there.
    let mut ref_schemes: BTreeMap<String, String> = BTreeMap::new();
    let mut enum_values: BTreeMap<String, String> = BTreeMap::new();
    let mut super_types: BTreeMap<String, String> = BTreeMap::new();
    let object_type_table = ObjectTypeKindTable::boot();
    for stmt_id in statement_ids.iter() {
        let Some(head) = head_noun_for(idx,stmt_id) else { continue };
        // Cascade through the registered object-type kinds in
        // declaration order. The boot() ordering puts Abstract +
        // Partition Declaration first so the abstract rows match
        // before entity/value rows for a noun that's classified as
        // both. The explicit abstract-wins merge below also covers
        // the case where multiple Statements address the same noun
        // (e.g. a Partition Declaration plus a separate Entity Type
        // Declaration on the same noun name).
        let ot: Option<String> = object_type_table.iter()
            .find(|(kind, _)| classifications_contains(idx, stmt_id, kind))
            .map(|(_, t)| t.to_string());
        if let Some(new_ot) = ot {
            let slot = by_noun.entry(head.clone()).or_insert_with(|| new_ot.clone());
            // Abstract wins over entity/value; otherwise keep existing.
            if new_ot == "abstract" {
                *slot = "abstract".to_string();
            }
        }

        // Reference-scheme shorthand: the entity declaration text is
        // e.g. `Organization(.Slug) is an entity type.` or the
        // multi-column form `Booking(.Year, .Course)…`. Stage-1
        // strips the parens before tokenization (so the Trailing
        // Marker rule can fire), but the original Text cell preserves
        // them. Re-scan here rather than plumb a separate
        // `Statement_has_Reference_Scheme` cell through the grammar
        // just for this one shape.
        if classifications_contains_any(idx,stmt_id,
            &["Entity Type Declaration", "Value Type Declaration"])
        {
            if let Some(text) = statement_text(idx,stmt_id) {
                if let Some(rs) = extract_reference_scheme(&text, &head) {
                    ref_schemes.insert(head.clone(), rs);
                }
            }
        }

        // Supertype binding: `Subtype is a subtype of Supertype.`
        if classifications_contains(idx,stmt_id, "Subtype Declaration") {
            if let Some(sup) = role_noun_at_position(classified_state, stmt_id, 1) {
                super_types.insert(head.clone(), sup);
            }
        }

        // Enum values: `The possible values of Priority are 'low', 'medium', 'high'.`
        if classifications_contains(idx,stmt_id, "Enum Values Declaration") {
            if let Some(text) = statement_text(idx,stmt_id) {
                if let Some(vals) = extract_enum_values(&text) {
                    enum_values.insert(head.clone(), vals);
                }
            }
        }
    }
    by_noun.into_iter().map(|(name, ot)| {
        let mut pairs: Vec<(&str, &str)> = vec![
            ("name", name.as_str()),
            ("objectType", ot.as_str()),
            ("worldAssumption", "closed"),
        ];
        if let Some(rs) = ref_schemes.get(&name) {
            pairs.push(("referenceScheme", rs.as_str()));
        }
        if let Some(sup) = super_types.get(&name) {
            pairs.push(("superType", sup.as_str()));
        }
        if let Some(ev) = enum_values.get(&name) {
            pairs.push(("enumValues", ev.as_str()));
        }
        fact_from_pairs(&pairs)
    }).collect()
}

/// Extract the reference-scheme column list from an entity
/// declaration like `Noun(.Col) is an entity type.` or
/// `Noun(.A, .B) is an entity type.`. Returns the columns joined by
/// `,` (matching legacy's binding format rmap reads via
/// `referenceScheme.split(',')`). Returns `None` if the text doesn't
/// contain a `(.…)` suffix for this noun.
fn extract_reference_scheme(text: &str, head_noun: &str) -> Option<String> {
    let after_noun = text.find(head_noun).map(|i| i + head_noun.len())?;
    let rest = &text[after_noun..];
    let open_idx = rest.find("(.")?;
    // Only accept `(.` that immediately follows the noun (allowing
    // whitespace) — otherwise we might pick up an unrelated later
    // parenthetical.
    if !rest[..open_idx].chars().all(|c| c.is_whitespace()) {
        return None;
    }
    let after_open = &rest[open_idx + 2..];
    let close_idx = after_open.find(')')?;
    let inside = &after_open[..close_idx];
    // Columns are `.Col` or just `Col` (the leading `.` is already
    // consumed for the first). Split by `,` and trim each.
    let cols: Vec<String> = inside.split(',')
        .map(|c| c.trim().trim_start_matches('.').to_string())
        .filter(|c| !c.is_empty())
        .collect();
    if cols.is_empty() { None } else { Some(cols.join(",")) }
}

/// Post-translator enrichment: emit `span0_factTypeId`/`span0_roleIndex`
/// (plus `span1_*` mirroring span0 for the UC/MC/VC/FC legacy quirk)
/// on every Constraint fact so `check.rs`, `command.rs`, and
/// `compile.rs::collect_enum_values` can attach the constraint to the
/// right fact type.
///
/// Resolution preference:
///   1. Full noun-sequence match against declared FTs — parity with
///      legacy's `resolve_constraint_schema`. A constraint text like
///      `It is forbidden that Support Response contains Prohibited
///      Word` mentions two declared nouns; the FT whose role
///      sequence equals `[Support Response, Prohibited Word]` is
///      the right binding. This matters for multi-noun deontic
///      constraints where the entity noun appears in several FTs
///      (e.g. Support Response → has Body AND contains Prohibited
///      Word) — picking the first match by entity alone points the
///      span at the wrong FT and `collect_enum_values` misses
///      Prohibited Word's enum values.
///   2. Entity-based fallback for single-noun constraints (ring kinds,
///      value constraints, etc.) where the stripped text surfaces
///      one noun and step 1 yields no match.
fn enrich_constraints_with_spans(
    constraints: &[Object],
    role_facts: &[Object],
) -> Vec<Object> {
    // Roles indexed two ways: by noun (first-match fallback) and by
    // fact type (full-sequence resolution).
    let mut roles_by_noun: hashbrown::HashMap<String, (String, String)> =
        hashbrown::HashMap::with_capacity(role_facts.len());
    let mut roles_by_ft: hashbrown::HashMap<String, Vec<(usize, String)>> =
        hashbrown::HashMap::new();
    let mut declared_noun_set: hashbrown::HashSet<String> = hashbrown::HashSet::new();
    for r in role_facts.iter() {
        let (Some(noun), Some(ft), Some(pos_str)) = (
            binding(r, "nounName"),
            binding(r, "factType"),
            binding(r, "position"),
        ) else { continue };
        roles_by_noun.entry(noun.to_string())
            .or_insert((ft.to_string(), pos_str.to_string()));
        let pos: usize = pos_str.parse().unwrap_or(0);
        roles_by_ft.entry(ft.to_string()).or_default().push((pos, noun.to_string()));
        declared_noun_set.insert(noun.to_string());
    }
    for roles in roles_by_ft.values_mut() {
        roles.sort_by_key(|(p, _)| *p);
    }
    let mut declared_nouns: Vec<String> = declared_noun_set.into_iter().collect();
    declared_nouns.sort_by(|a, b| b.len().cmp(&a.len()));

    constraints.iter().map(|c| {
        let pairs: Vec<Object> = c.as_seq()
            .map(|s| s.to_vec())
            .unwrap_or_default();
        // Avoid duplicate span bindings if somehow already present.
        let has_span = pairs.iter().any(|p| p.as_seq()
            .and_then(|s| s.get(0)?.as_atom())
            .map(|k| k == "span0_factTypeId").unwrap_or(false));
        if has_span { return c.clone(); }

        // Preference 1: resolve by full noun-sequence match.
        let text = binding(c, "text").unwrap_or("");
        let resolved = resolve_constraint_span_ft(text, &roles_by_ft, &declared_nouns);
        // Preference 2: fall back to entity-based first-match.
        let fallback = || -> Option<(String, String)> {
            let entity = binding(c, "entity")?;
            roles_by_noun.get(entity).cloned()
        };
        let (ft_id, pos) = match resolved.or_else(fallback) {
            Some(x) => x,
            None => return c.clone(),
        };

        let mut new_pairs = pairs;
        let push = |np: &mut Vec<Object>, k: &str, v: &str| {
            np.push(Object::seq(vec![Object::atom(k), Object::atom(v)]));
        };
        push(&mut new_pairs, "span0_factTypeId", &ft_id);
        push(&mut new_pairs, "span0_roleIndex", &pos);
        push(&mut new_pairs, "span1_factTypeId", &ft_id);
        push(&mut new_pairs, "span1_roleIndex", &pos);
        Object::Seq(new_pairs.into())
    }).collect()
}

/// Resolve a Constraint's target fact type by noun-sequence match
/// — legacy `resolve_constraint_schema` parity without the catalog
/// machinery. Returns `(ft_id, role_index_of_first_noun_in_ft)` when
/// the stripped constraint text mentions declared nouns whose order
/// (with repetition) exactly matches some declared FT's role sequence.
///
/// The first noun in the stripped text is the quantified / forbidden
/// noun — `role_index` points at its position in the FT, so downstream
/// code (`check.rs`'s `constraint_applies_to_role`) attaches the
/// constraint to the correct role.
fn resolve_constraint_span_ft(
    text: &str,
    roles_by_ft: &hashbrown::HashMap<String, Vec<(usize, String)>>,
    sorted_nouns_longest_first: &[String],
) -> Option<(String, String)> {
    // Strip quoted literals (constraint body may carry `'Overnight'` etc.).
    let stripped = {
        let mut s = text.to_string();
        while let Some(open) = s.find('\'') {
            match s[open + 1..].find('\'') {
                Some(close) => {
                    s = alloc::format!("{}{}", &s[..open], &s[open + 1 + close + 1..]);
                }
                None => break,
            }
        }
        s
    };
    // Strip deontic / quantifier prefixes that precede the noun-verb-noun
    // backbone. Order matches legacy's `resolve_constraint_schema`.
    // #790 — vocabulary lifts to ConstraintSpanPrefixTable so the data
    // lives in `readings/forml2-grammar.md` rather than as a cascade
    // here. boot() falls back to the historic 11-prefix list when the
    // bare engine has no metamodel loaded.
    let stripped = ConstraintSpanPrefixTable::boot().strip_all(&stripped);

    // #326: strip digit subscripts (`Noun1`, `Noun2` → `Noun`) so
    // conditional ring shapes like "If Noun1 is subtype of Noun2 …
    // then Noun1 is subtype of Noun3" surface the base noun. Without
    // this the trailing digit breaks the word-boundary check in
    // find_noun_sequence and the antecedent matches zero nouns.
    let stripped = strip_digit_subscripts(&stripped);

    let found_nouns: Vec<String> = find_noun_sequence(&stripped, sorted_nouns_longest_first);

    // #326: pronoun expansion for ring constraints.
    // "No X R-s itself." surfaces as `found_nouns = [X]`; the self-
    // referential binary FT we want to target has two roles both X.
    // Duplicate the last noun when `itself` is present so the
    // subsequent role-sequence match finds `[X, X]` on the FT with
    // roles [(0,X),(1,X)] — e.g. `Noun is subtype of Noun`, not
    // the first App-bearing FT in hashmap iteration order.
    let found_nouns: Vec<String> = if !found_nouns.is_empty()
        && stripped.contains("itself")
    {
        let mut v = found_nouns;
        if let Some(last) = v.last().cloned() {
            v.push(last);
        }
        v
    } else {
        found_nouns
    };

    // #326: for the conditional ring shape the antecedent + consequent
    // together surface 3-4 subscripted references to the same noun
    // ("Noun1 … Noun2 … Noun3"). The self-referential FT `X R X` has
    // two roles; truncate the found sequence to `[X, X]` so the
    // role-sequence match lands.
    let found_nouns: Vec<String> = if found_nouns.len() > 2
        && found_nouns.iter().all(|n| n == &found_nouns[0])
    {
        alloc::vec![found_nouns[0].clone(), found_nouns[0].clone()]
    } else {
        found_nouns
    };

    if found_nouns.len() < 2 { return None; }

    // Find an FT whose role noun sequence matches the found noun sequence.
    for (ft_id, roles) in roles_by_ft {
        if roles.len() != found_nouns.len() { continue; }
        let role_nouns: Vec<&str> = roles.iter().map(|(_, n)| n.as_str()).collect();
        if role_nouns.iter().zip(found_nouns.iter())
            .all(|(a, b)| a == &b.as_str())
        {
            let first = found_nouns[0].as_str();
            let role_index = roles.iter()
                .find(|(_, n)| n == first)
                .map(|(p, _)| *p)
                .unwrap_or(0);
            return Some((ft_id.clone(), alloc::format!("{}", role_index)));
        }
    }
    None
}

/// Walk `text` left-to-right, matching declared nouns longest-first at
/// each cursor position with word-boundary checking (next char must be
/// absent, whitespace, or a non-alphanumeric separator). Returns the
/// ordered sequence of noun names — duplicates preserved (e.g. ring
/// constraints' `Person ... Person` becomes `[Person, Person]`).
fn find_noun_sequence(text: &str, sorted_nouns_longest_first: &[String]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !text.is_char_boundary(i) { i += 1; continue; }
        let rest = &text[i..];
        let matched = sorted_nouns_longest_first.iter().find(|n| {
            if !rest.starts_with(n.as_str()) { return false; }
            // Word-boundary after: EOF or non-alphanumeric char.
            rest[n.len()..].chars().next()
                .map(|c| !c.is_alphanumeric())
                .unwrap_or(true)
        });
        if let Some(n) = matched {
            // Word-boundary before: SOF or non-alphanumeric char.
            let before_ok = if i == 0 {
                true
            } else {
                text[..i].chars().next_back()
                    .map(|c| !c.is_alphanumeric())
                    .unwrap_or(true)
            };
            if before_ok {
                found.push(n.clone());
                i += n.len();
                continue;
            }
        }
        i += 1;
    }
    found
}

/// Strip `<alpha><digits>` patterns to `<alpha>` so ring conditional
/// shapes surface the base noun. `Noun1` → `Noun`; `API3` → `API`.
/// Digits not preceded by a letter (numeric literals) are preserved.
fn strip_digit_subscripts(s: &str) -> alloc::string::String {
    let mut out = alloc::string::String::with_capacity(s.len());
    let mut last_was_alpha = false;
    for c in s.chars() {
        if c.is_ascii_digit() && last_was_alpha {
            // digit after a letter — treat as subscript, drop it;
            // last_was_alpha stays true so the next digit also drops.
            continue;
        }
        last_was_alpha = c.is_alphabetic();
        out.push(c);
    }
    out
}

/// Strip a trailing `(<ring-kind>)` annotation — the explicit kind
/// hint authors attach to ring constraints that use the multi-clause
/// conditional shape (e.g.,
/// `If some X R some Y then Y R X. (symmetric)`). Returns the body
/// with the annotation removed (still ending in `.`), or `None` if
/// the parens don't contain a recognized ring adjective.
fn strip_ring_annotation(line: &str) -> Option<&str> {
    let trimmed = line.trim_end();
    let inner = trimmed.strip_suffix(')')?;
    let open_idx = inner.rfind('(')?;
    let kind = inner[open_idx + 1..].trim();
    // #791 — bare-adjective vocabulary lifts to RingAdjectiveTable
    // so the data lives in `readings/forml2-grammar.md` rather than
    // an inline const here. boot() falls back to the historic 8-word
    // list when the bare engine has no metamodel loaded.
    let adjectives = RingAdjectiveTable::boot();
    if !adjectives.contains(kind) { return None; }
    // Caller expects the body to end with `.` — strip the annotation
    // and any whitespace between body-period and open-paren.
    let body = inner[..open_idx].trim_end();
    Some(body)
}

/// Extract the quoted values from a `The possible values of <Noun>
/// are 'v1', 'v2', …` declaration. Returns them joined by `,`.
fn extract_enum_values(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let are_idx = lower.find(" are ")?;
    let tail = &text[are_idx + 5..];
    let mut vals: Vec<String> = Vec::new();
    let mut rest = tail;
    while let Some(open) = rest.find('\'') {
        let after = &rest[open + 1..];
        let close = after.find('\'')?;
        vals.push(after[..close].to_string());
        rest = &after[close + 1..];
    }
    if vals.is_empty() { None } else { Some(vals.join(",")) }
}

/// Translate `Subtype Declaration` classifications into `Subtype` cell
/// facts: `(subtype, supertype)` pairs. The subtype is the Statement's
/// Head Noun; the supertype is the noun at Role Position 1 (the only
/// other role reference in `A is a subtype of B`).
pub fn translate_subtypes(classified_state: &Object, idx: &StmtIndex) -> Vec<Object> {
    let table = StatementTranslatorTable::boot();
    let kinds: Vec<&str> = table.kinds_for("translate_subtypes");
    let statement_ids = collect_statement_ids(idx);
    statement_ids.iter().filter_map(|stmt_id| {
        if !kinds.iter().any(|k| classifications_contains(idx, stmt_id, k)) {
            return None;
        }
        let sub = head_noun_for(idx,stmt_id)?;
        let sup = role_noun_at_position(classified_state, stmt_id, 1)?;
        Some(fact_from_pairs(&[
            ("subtype", sub.as_str()),
            ("supertype", sup.as_str()),
        ]))
    }).collect()
}

/// Translate statements carrying an ORM 2 derivation marker (`*` /
/// `**` / `+`) into `Fact Type has Derivation Mode` instance facts,
/// matching legacy's `emit_instance_fact(ir, "Fact Type", <reading>,
/// "Derivation Mode", "Derivation Mode", &m)` in `apply_action`.
///
///   `Fact Type has Arity. *` → InstanceFact
///     subjectNoun = "Fact Type"
///     subjectValue = "Fact Type has Arity"          (canonical reading)
///     fieldName = "Fact_Type_has_Derivation_Mode"   (canonical FT id)
///     objectNoun = "Derivation Mode"
///     objectValue = "fully-derived"                 (mode atom)
///
/// Emitted only for Statements classified as Fact Type Reading so
/// the derivation-marker on derivation-rule statements (where the
/// `*` prefix is a readability marker, not a mode signal on a Fact
/// Type) doesn't spawn spurious InstanceFacts.
pub fn translate_derivation_mode_facts(classified_state: &Object, idx: &StmtIndex) -> Vec<Object> {
    let _ = classified_state;
    let statement_ids = collect_statement_ids(idx);
    let mut out: Vec<Object> = Vec::new();
    // Same exclude list as translate_fact_types — don't emit on
    // noun declarations or instance facts that incidentally carry
    // role references. Derived from StatementTranslatorTable as
    // "every registered classification kind except Fact Type
    // Reading", so adding a new kind to the grammar automatically
    // extends the exclusion (without re-touching this list).
    let table = StatementTranslatorTable::boot();
    let exclude: Vec<&str> = table.kinds().into_iter()
        .filter(|k| *k != "Fact Type Reading")
        .collect();
    for stmt_id in statement_ids.iter() {
        // Fact Type Reading classification is the anchor — an `iff`
        // derivation rule also has a marker but lands as Derivation
        // Rule, not Fact Type Reading, because Stage-1 strips the
        // leading `* ` prefix before tokenization (see #294).
        if !classifications_contains(idx,stmt_id, "Fact Type Reading") {
            continue;
        }
        if classifications_contains_any(idx, stmt_id, &exclude) {
            continue;
        }
        let Some(mode) = derivation_marker_for(idx,stmt_id) else { continue };
        let Some(text) = statement_text(idx,stmt_id) else { continue };
        // Legacy passes `field_name = "Derivation Mode"` — the
        // attribute noun itself — rather than constructing a
        // canonical FT id. This is the attribute-style
        // `subjectNoun '<value>' has <objectNoun> '<objectValue>'`
        // shape applied to the metamodel binary `Fact Type has
        // Derivation Mode`.
        out.push(fact_from_pairs(&[
            ("subjectNoun",  "Fact Type"),
            ("subjectValue", text.as_str()),
            ("fieldName",    "Derivation Mode"),
            ("objectNoun",   "Derivation Mode"),
            ("objectValue",  mode.as_str()),
        ]));
    }
    out
}

fn derivation_marker_for(idx: &StmtIndex, stmt_id: &str) -> Option<String> {
    idx.derivation_markers.get(stmt_id).cloned()
}

/// Translate Partition Declaration statements into `Subtype` cell
/// facts — one `(subtype, supertype)` pair per subtype in the
/// comma-separated list. Shape: `A is partitioned into B, C, D` →
/// (B, A), (C, A), (D, A). The supertype's abstractness flows
/// through `translate_nouns` which treats Partition Declaration as
/// an abstract-marking classification.
///
/// The Classification kind(s) this translator handles are read from
/// `StatementTranslatorTable::boot()` rather than hardcoded — the
/// Rust function name is the registry key. Per AREST.tex §3 (eq:sys)
/// new operations are registered without modifying any entity.
pub fn translate_partitions(classified_state: &Object, idx: &StmtIndex) -> Vec<Object> {
    let _ = classified_state; // statement classification flows via idx
    let table = StatementTranslatorTable::boot();
    let kinds: Vec<&str> = table.kinds_for("translate_partitions");
    let statement_ids = collect_statement_ids(idx);
    let mut out: Vec<Object> = Vec::new();
    for stmt_id in statement_ids.iter() {
        if !kinds.iter().any(|k| classifications_contains(idx, stmt_id, k)) {
            continue;
        }
        let Some(sup) = head_noun_for(idx,stmt_id) else { continue };
        let roles = role_refs_for(idx,stmt_id);
        for sub in roles.iter().skip(1) {
            out.push(fact_from_pairs(&[
                ("subtype", sub.as_str()),
                ("supertype", sup.as_str()),
            ]));
        }
    }
    out
}

/// Translate `Fact Type Reading` classifications into `FactType` +
/// `Role` cell facts. Returns `(fact_type_facts, role_facts)`.
///
/// Exclusions: Statements whose Fact Type Reading classification is
/// an artifact of declaring a noun (Entity Type / Value Type /
/// Subtype / Abstract / Enum Values Declaration) or asserting an
/// instance (Instance Fact) are NOT emitted as fact types. The
/// current FORML 2 corpus relies on this separation — the noun-
/// declaration shape `Customer is an entity type` also matches Fact
/// Type Reading because it has a Role Reference.
pub fn translate_fact_types(classified_state: &Object, idx: &StmtIndex) -> (Vec<Object>, Vec<Object>) {
    let statement_ids = collect_statement_ids(idx);
    let mut ft_facts: Vec<Object> = Vec::new();
    let mut role_facts: Vec<Object> = Vec::new();
    // Exclude every non-fact-type classification. Fact Type Reading
    // fires whenever a Role Reference is present, which is true of
    // declarations, instance facts, and constraint statements alike.
    // The translator only emits when Fact Type Reading is the ONLY
    // structural classification. Derived from StatementTranslatorTable
    // as "every registered kind except Fact Type Reading" — adding a
    // new kind to the grammar automatically extends the exclusion.
    let table = StatementTranslatorTable::boot();
    let exclude: Vec<&str> = table.kinds().into_iter()
        .filter(|k| *k != "Fact Type Reading")
        .collect();
    for stmt_id in statement_ids.iter() {
        if !classifications_contains(idx,stmt_id, "Fact Type Reading") {
            continue;
        }
        if classifications_contains_any(idx, stmt_id, &exclude) {
            continue;
        }
        let roles = role_refs_for(idx,stmt_id);
        let Some(text) = statement_text(idx,stmt_id) else { continue };
        let reading = text;
        // Mirror legacy's `fact_type_id(role_nouns, verb)` shape:
        // noun parts preserve their declared casing, the verb between
        // roles lowercases. Keeps `Noun_has_reference_scheme_Noun`
        // matching legacy (the reading text has capital `Reference
        // Scheme` but the id lowercases).
        let id = fact_type_id_from_reading(&reading, &roles);
        ft_facts.push(fact_from_pairs(&[
            ("id", id.as_str()),
            ("reading", reading.as_str()),
            ("arity", &roles.len().to_string()),
        ]));
        for (i, noun_name) in roles.iter().enumerate() {
            role_facts.push(fact_from_pairs(&[
                ("factType", id.as_str()),
                ("nounName", noun_name.as_str()),
                ("position", &i.to_string()),
            ]));
        }
    }
    (ft_facts, role_facts)
}

/// Build a canonical FactType id from a reading text + ordered role
/// noun names — matches legacy's `fact_type_id(role_nouns, verb)`
/// convention. Noun parts preserve case (with spaces replaced by
/// underscores); the verb between role positions is lowercased.
///
/// For `Noun has Reference Scheme Noun` with roles `[Noun, Noun]`:
///   verb = "has Reference Scheme" → "has_reference_scheme"
///   parts = ["Noun", "has_reference_scheme", "Noun"]
///   id = "Noun_has_reference_scheme_Noun"
fn fact_type_id_from_reading(reading: &str, roles: &[String]) -> String {
    if roles.is_empty() {
        return reading.replace(' ', "_");
    }
    // Walk the text once, identifying role-noun spans in order so
    // repeated nouns (ring shapes) bind to distinct positions.
    let mut cursor = 0;
    let mut parts: Vec<String> = Vec::new();
    for (i, noun) in roles.iter().enumerate() {
        let Some(pos) = reading[cursor..].find(noun.as_str()) else {
            // Fall through: if the reading doesn't align with roles,
            // use the legacy text-replace fallback.
            return reading.replace(' ', "_");
        };
        let abs = cursor + pos;
        if i > 0 {
            // Everything between the previous role end and this
            // role's start is verb text. Lowercase + underscore.
            let verb = reading[cursor..abs].trim();
            if !verb.is_empty() {
                parts.push(verb.to_lowercase().replace(' ', "_"));
            }
        }
        parts.push(noun.replace(' ', "_"));
        cursor = abs + noun.len();
    }
    // Tail after last role (unary predicate or trailing text).
    let tail = reading[cursor..].trim();
    if !tail.is_empty() {
        parts.push(tail.to_lowercase().replace(' ', "_"));
    }
    parts.join("_")
}

/// Extract a synthetic FactType + its Roles from the body of an
/// `It is possible that ...` possibility-override statement. Returns
/// the FT fact (shaped like `translate_fact_types` output) plus a
/// vec of Role facts, or `None` when the body doesn't look like a
/// fact-type predicate.
///
/// Legacy emits these implicitly via its constraint-text scan. Stage-2
/// does it explicitly here so `It is possible that more than one
/// Noun has the same Alias.` registers a synthetic
/// `Noun_has_the_same_Alias` FT alongside the two Role facts
/// `(factType=Noun_has_the_same_Alias, nounName=Noun, position=0)`
/// and `(factType=Noun_has_the_same_Alias, nounName=Alias,
/// position=1)`.
///
/// `nouns` is the full declared-noun list. Longest-first matching
/// drives role extraction, same as Stage-1 tokenisation.
fn possibility_synthetic_fact_type(
    body: &str,
    nouns: &[String],
) -> Option<(Object, Vec<Object>)> {
    // Strip the existential prefix. Legacy's id drops the
    // quantifiers from the noun positions but keeps them in the
    // verb — so the synthetic reading starts at the subject noun.
    let body = body
        .strip_prefix("some ")
        .or_else(|| body.strip_prefix("more than one "))
        .unwrap_or(body);

    // Longest-first noun matching. Mirrors Stage-1.
    let mut sorted: Vec<&str> = nouns.iter().map(|s| s.as_str()).collect();
    sorted.sort_by(|a, b| b.len().cmp(&a.len()));

    // Scan the body for role nouns, preserving order. Each matched
    // noun advances the cursor past itself so later matches pick up
    // the next role.
    let mut roles: Vec<(String, usize, usize)> = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !body.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let rest = &body[i..];
        let at_word_start = i == 0 || {
            let prev = bytes[i - 1];
            !prev.is_ascii_alphanumeric() && prev != b'_'
        };
        if !at_word_start { i += 1; continue; }
        let Some(noun) = sorted.iter().find(|n| {
            rest.starts_with(**n) && {
                let end = i + n.len();
                end == bytes.len() || {
                    let next = bytes[end];
                    !next.is_ascii_alphanumeric() && next != b'_'
                }
            }
        }) else {
            i += 1;
            continue;
        };
        let start = i;
        let end = i + noun.len();
        roles.push(((*noun).to_string(), start, end));
        i = end;
    }
    if roles.len() < 2 { return None; }

    // Build the reading: preserve the body text verbatim (verb
    // phrases like `has the same` / `has more than one` are part of
    // the canonical reading, not stripped).
    let reading = body.to_string();
    let role_nouns: Vec<String> = roles.iter().map(|(n, _, _)| n.clone()).collect();
    let id = fact_type_id_from_reading(&reading, &role_nouns);

    let arity = role_nouns.len().to_string();
    let ft = fact_from_pairs(&[
        ("id", id.as_str()),
        ("reading", reading.as_str()),
        ("arity", arity.as_str()),
    ]);
    let role_facts: Vec<Object> = role_nouns.iter().enumerate()
        .map(|(pos, n)| {
            let pos_s = pos.to_string();
            fact_from_pairs(&[
                ("factType", id.as_str()),
                ("nounName", n.as_str()),
                ("position", pos_s.as_str()),
            ])
        })
        .collect();
    Some((ft, role_facts))
}

/// Role head nouns for a Statement, ordered by Role Position.
fn role_refs_for(idx: &StmtIndex, stmt_id: &str) -> Vec<String> {
    let Some(role_ids) = idx.role_refs_by_stmt.get(stmt_id) else {
        return Vec::new();
    };
    let mut with_pos: Vec<(usize, String)> = role_ids.iter()
        .filter_map(|rid| {
            let pos: usize = idx.role_pos_by_ref.get(rid)?.parse().ok()?;
            let noun = idx.role_head_noun_by_ref.get(rid)?.clone();
            Some((pos, noun))
        })
        .collect();
    with_pos.sort_by_key(|(p, _)| *p);
    with_pos.into_iter().map(|(_, n)| n).collect()
}

fn statement_text(idx: &StmtIndex, stmt_id: &str) -> Option<String> {
    idx.texts.get(stmt_id).cloned()
}

/// Thread-local index cache populated once per parse call (during
/// the translator block). Short-circuits `classifications_for` /
/// `head_noun_for` / `statement_text` etc. from O(cell_size) scans
/// per call to O(1) HashMap lookups. Core.md has ~150 statements and
/// ~500 Statement_has_Classification facts; without this cache each
/// of the 15 translators was scanning the full cell 150 times.
#[derive(Default)]
struct StmtIndex {
    classifications: hashbrown::HashMap<String, Vec<String>>,
    head_nouns: hashbrown::HashMap<String, String>,
    texts: hashbrown::HashMap<String, String>,
    trailing_markers: hashbrown::HashMap<String, String>,
    derivation_markers: hashbrown::HashMap<String, String>,
    // Role-reference indexing: per-statement list of role ref ids, plus
    // per-ref position / head noun / literal. `translate_fact_types`
    // reaches into all three cells for every Fact-Type-Reading stmt.
    role_refs_by_stmt: hashbrown::HashMap<String, Vec<String>>,
    role_pos_by_ref: hashbrown::HashMap<String, String>,
    role_head_noun_by_ref: hashbrown::HashMap<String, String>,
    role_literal_by_ref: hashbrown::HashMap<String, String>,
    verbs: hashbrown::HashMap<String, String>,
    /// Wrapped in `Arc` so `collect_statement_ids` does a refcount
    /// bump rather than cloning 506 heap-allocated `String`s on
    /// every translator call.
    statement_ids: alloc::sync::Arc<Vec<String>>,
}

fn build_stmt_index(state: &Object) -> StmtIndex {
    let mut idx = StmtIndex::default();
    let index_single = |cell: &str, key_field: &str, value_field: &str,
                        target: &mut hashbrown::HashMap<String, String>| {
        if let Some(seq) = fetch_or_phi(cell, state).as_seq() {
            for f in seq.iter() {
                let (Some(k), Some(v)) = (binding(f, key_field), binding(f, value_field))
                    else { continue };
                target.entry(k.to_string()).or_insert_with(|| v.to_string());
            }
        }
    };
    // classifications: many-per-statement → Vec
    if let Some(seq) = fetch_or_phi("Statement_has_Classification", state).as_seq() {
        for f in seq.iter() {
            let (Some(stmt), Some(cls)) = (
                binding(f, "Statement"), binding(f, "Classification")
            ) else { continue };
            idx.classifications.entry(stmt.to_string())
                .or_default()
                .push(cls.to_string());
        }
    }
    index_single("Statement_has_Head_Noun", "Statement", "Head_Noun", &mut idx.head_nouns);
    index_single("Statement_has_Text", "Statement", "Text", &mut idx.texts);
    index_single("Statement_has_Trailing_Marker", "Statement", "Trailing_Marker",
        &mut idx.trailing_markers);
    index_single("Statement_has_Derivation_Marker", "Statement", "Derivation_Marker",
        &mut idx.derivation_markers);
    // Role-reference chain: stmt → [ref_id], ref_id → position / head noun / literal.
    if let Some(seq) = fetch_or_phi("Statement_has_Role_Reference", state).as_seq() {
        for f in seq.iter() {
            let (Some(stmt), Some(rref)) = (
                binding(f, "Statement"), binding(f, "Role_Reference")
            ) else { continue };
            idx.role_refs_by_stmt.entry(stmt.to_string())
                .or_default().push(rref.to_string());
        }
    }
    index_single("Role_Reference_has_Role_Position", "Role_Reference", "Role_Position",
        &mut idx.role_pos_by_ref);
    index_single("Role_Reference_has_Head_Noun", "Role_Reference", "Head_Noun",
        &mut idx.role_head_noun_by_ref);
    index_single("Role_Reference_has_Literal_Value", "Role_Reference", "Literal_Value",
        &mut idx.role_literal_by_ref);
    index_single("Statement_has_Verb", "Statement", "Verb", &mut idx.verbs);
    if let Some(seq) = fetch_or_phi("Statement", state).as_seq() {
        idx.statement_ids = alloc::sync::Arc::new(seq.iter()
            .filter_map(|f| binding(f, "id").map(String::from))
            .collect());
    }
    idx
}

/// Translate `Instance Fact` classifications into `InstanceFact` cell
/// facts. Binary instance-fact shape (subject + field + object):
///
///   subjectNoun = role 0's head noun
///   subjectValue = role 0's literal
///   fieldName = Statement's Verb token
///   objectNoun = role 1's head noun (if present)
///   objectValue = role 1's literal (if present)
///
/// Ternary+ instance facts (`Wine App 'X' requires DLL Override of
/// DLL Name 'D' with DLL Behavior 'B'`, etc.) extend this with one
/// pair of `roleNNoun` / `roleNValue` bindings per additional role
/// (N starts at 2). #553 — without these, the third role's literal
/// was silently dropped, forcing CLI consumers to re-parse the raw
/// markdown to recover it.
///
/// Unary instance-facts (value assertions like `Customer 'alice' is
/// active`) currently emit with empty objectNoun/objectValue.
pub fn translate_instance_facts(classified_state: &Object, idx: &StmtIndex) -> Vec<Object> {
    translate_instance_facts_with_ft_ids(classified_state, idx, &[])
}

/// Variant that can resolve `fieldName` to a canonical FT id when the
/// (subject, verb, object[, roleN…]) tuple matches a declared Fact
/// Type. The caller supplies the already-translated FactType ids;
/// when the constructed canonical id is among them, it wins; otherwise
/// fall back to the raw verb token. Legacy exhibits the same
/// behavior — `Constraint Type 'AC' has Name 'Acyclic'` resolves to
/// `Constraint_Type_has_Name` because the FT is declared, but
/// `HTTP Method 'DELETE' has Name 'DELETE'` stays on `has` because no
/// `HTTP Method has Name` FT is declared.
///
/// For ternary+ shapes the canonical id is built from the statement
/// text itself (via `fact_type_id_from_reading` after stripping the
/// per-role literals), so it picks up the inter-role verb chunks
/// (`with`, `at`, `and …`) that the per-statement Verb cell only
/// records for the role-0 ↔ role-1 gap.
pub fn translate_instance_facts_with_ft_ids(
    classified_state: &Object,
    idx: &StmtIndex,
    declared_ft_ids: &[String],
) -> Vec<Object> {
    let table = StatementTranslatorTable::boot();
    let kinds: Vec<&str> = table.kinds_for("translate_instance_facts");
    let statement_ids = collect_statement_ids(idx);
    let mut out: Vec<Object> = Vec::new();
    for stmt_id in statement_ids.iter() {
        if !kinds.iter().any(|k| classifications_contains(idx, stmt_id, k)) {
            continue;
        }
        let roles = role_refs_with_literals(idx,stmt_id);
        if roles.is_empty() { continue; }
        let verb = statement_verb(idx,stmt_id).unwrap_or_default();
        let subject_noun = &roles[0].0;
        let subject_value = roles[0].1.as_deref().unwrap_or("");
        let (object_noun, object_value) = roles.get(1)
            .map(|(n, lit)| (n.as_str(), lit.as_deref().unwrap_or("")))
            .unwrap_or(("", ""));

        // Canonical id construction. For unary / binary shapes this
        // mirrors the legacy `subject_verb[_object]` munge. For
        // ternary+ shapes we recover the canonical FT reading from
        // the statement text (literals stripped) and route it through
        // `fact_type_id_from_reading` so the inter-role verb tokens
        // (e.g. ` with `, ` at `, ` and `) survive. Lower-arity facts
        // keep the cheap path — no statement-text walk needed.
        // Use the same text-walking canonicalizer the schema-side uses
        // (see translate_fact_types). The previous binary-only path
        // built `{subject}_{verb}_{object}` from a format string, which
        // diverged from `fact_type_id_from_reading` whenever the reading
        // self-referenced a role noun (e.g. `Fact Type has Layer
        // Affinity to Layer` — the intra-verb `Layer` gets lowercased
        // by the schema walker but stayed capitalized in the format
        // string), silently breaking the declared-FT lookup.
        let canonical = {
            let text = statement_text(idx,stmt_id).unwrap_or_default();
            let role_nouns: Vec<String> = roles.iter()
                .map(|(n, _)| n.clone()).collect();
            let reading = strip_role_literals(&text, &roles);
            fact_type_id_from_reading(&reading, &role_nouns)
        };
        let field_name: String = if declared_ft_ids.iter().any(|id| *id == canonical) {
            canonical
        } else {
            verb.clone()
        };
        // Build the (key, value) list for the InstanceFact fact.
        // Keep the legacy 5-pair prefix verbatim so cells consumers
        // (compile.rs::extract_facts_from_pop, ring constraint span
        // resolver, etc.) keep their existing reads. Append one
        // (`roleNNoun`, `roleNValue`) pair per additional role.
        let mut pairs: Vec<(String, String)> = Vec::with_capacity(5 + 2 * roles.len().saturating_sub(2));
        pairs.push(("subjectNoun".to_string(),  subject_noun.clone()));
        pairs.push(("subjectValue".to_string(), subject_value.to_string()));
        pairs.push(("fieldName".to_string(),    field_name.clone()));
        pairs.push(("objectNoun".to_string(),   object_noun.to_string()));
        pairs.push(("objectValue".to_string(),  object_value.to_string()));
        for (i, (n, lit)) in roles.iter().enumerate().skip(2) {
            pairs.push((alloc::format!("role{}Noun", i),  n.clone()));
            pairs.push((alloc::format!("role{}Value", i), lit.clone().unwrap_or_default()));
        }
        let pair_refs: Vec<(&str, &str)> = pairs.iter()
            .map(|(k, v)| (k.as_str(), v.as_str())).collect();
        out.push(fact_from_pairs(&pair_refs));
    }
    out
}

/// Strip every role literal (the `'value'` slice that follows a role
/// noun) from `text`, recovering the FT-reading-shaped string. Used
/// by `translate_instance_facts_with_ft_ids` to build a canonical id
/// for ternary+ instance facts via `fact_type_id_from_reading`.
///
/// Walks the role list in declaration order so repeated nouns (ring
/// shapes) match distinct positions; each successive scan starts at
/// the previous strip's end. Roles without literals are passed
/// through unchanged.
fn strip_role_literals(text: &str, roles: &[(String, Option<String>)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    for (noun, lit) in roles {
        let Some(rel) = text[cursor..].find(noun.as_str()) else {
            // Reading doesn't align with roles — return original text
            // and let the canonical-id fallback handle it.
            out.push_str(&text[cursor..]);
            return out;
        };
        let abs_noun = cursor + rel;
        let after_noun = abs_noun + noun.len();
        // Copy text up to and including the noun.
        out.push_str(&text[cursor..after_noun]);
        cursor = after_noun;
        // If a literal follows (whitespace + `'…'`), skip it.
        if let Some(_lit_str) = lit {
            let tail = &text[cursor..];
            let after_ws = tail.trim_start();
            let ws_len = tail.len() - after_ws.len();
            if after_ws.starts_with('\'') {
                if let Some(end) = after_ws[1..].find('\'') {
                    cursor += ws_len + 1 + end + 1;
                }
            }
        }
    }
    out.push_str(&text[cursor..]);
    out
}

/// Role head nouns AND literal values for a Statement, ordered by
/// Role Position. Returns `Vec<(noun, Option<literal>)>`.
fn role_refs_with_literals(idx: &StmtIndex, stmt_id: &str) -> Vec<(String, Option<String>)> {
    let Some(role_ids) = idx.role_refs_by_stmt.get(stmt_id) else {
        return Vec::new();
    };
    let mut with_pos: Vec<(usize, String, Option<String>)> = role_ids.iter()
        .filter_map(|rid| {
            let pos: usize = idx.role_pos_by_ref.get(rid)?.parse().ok()?;
            let noun = idx.role_head_noun_by_ref.get(rid)?.clone();
            let lit = idx.role_literal_by_ref.get(rid).cloned();
            Some((pos, noun, lit))
        })
        .collect();
    with_pos.sort_by_key(|(p, _, _)| *p);
    with_pos.into_iter().map(|(_, n, l)| (n, l)).collect()
}

fn statement_verb(idx: &StmtIndex, stmt_id: &str) -> Option<String> {
    idx.verbs.get(stmt_id).cloned()
}

/// Translate `Ring Constraint` classifications into `Constraint` cell
/// facts. Each ring adjective maps to a two-letter ORM 2 kind code:
///
///   is irreflexive   → IR
///   is asymmetric    → AS
///   is antisymmetric → AT
///   is symmetric     → SY
///   is intransitive  → IT
///   is transitive    → TR
///   is acyclic       → AC
///   is reflexive     → RF
///
/// The Constraint fact carries `kind`, `modality="alethic"`,
/// `text` (Statement text), and `entity` (Head Noun). Spans
/// (fact_type_id resolution) are left empty — a follow-up
/// commit will populate them once the FactType cell exists.
pub fn translate_ring_constraints(classified_state: &Object, idx: &StmtIndex) -> Vec<Object> {
    translate_ring_constraints_with_tables(
        classified_state,
        idx,
        &RingKindTable::boot(),
        &ConditionalRingMatrix::boot(),
    )
}

/// MC2 (#713) cell-driven variant. Stage-2's
/// `parse_to_state_via_stage12_impl` builds the two tables once from
/// the cached grammar state's EnumValues cell and threads them through.
/// Bare callers (legacy unit tests) get the `boot()` fallback via
/// `translate_ring_constraints`.
pub fn translate_ring_constraints_with_tables(
    classified_state: &Object,
    idx: &StmtIndex,
    ring_kinds: &RingKindTable,
    conditional_matrix: &ConditionalRingMatrix,
) -> Vec<Object> {
    let table = StatementTranslatorTable::boot();
    let kinds: Vec<&str> = table.kinds_for("translate_ring_constraints");
    let statement_ids = collect_statement_ids(idx);
    let declared_nouns = declared_noun_names(classified_state);
    let mut out: Vec<Object> = Vec::new();
    for stmt_id in statement_ids.iter() {
        // Two sources for ring emission:
        //   (a) Ring Constraint classification (trailing-marker shape:
        //       `<FT> is irreflexive.` / `No X R itself.`).
        //   (b) Conditional ring shape (`If X R Y and Y R Z, then
        //       X R Z` etc.) not caught by the grammar's trailing-
        //       marker rule — matches legacy `try_ring`'s pass-2b
        //       conditional-pattern dispatcher.
        let is_classified_ring = kinds.iter()
            .any(|k| classifications_contains(idx, stmt_id, k));
        let text = statement_text(idx,stmt_id).unwrap_or_default();
        let (kind, kind_source) = if is_classified_ring {
            let marker = match trailing_marker_for(idx,stmt_id) {
                Some(m) => m,
                None => continue,
            };
            match ring_adjective_to_kind(&marker, ring_kinds) {
                Some(k) => (k, "marker"),
                None => continue,
            }
        } else if let Some(k) = conditional_ring_kind(
            &text, &declared_nouns, conditional_matrix)
        {
            (k, "conditional")
        } else {
            continue;
        };
        let _ = kind_source;
        let entity = head_noun_for(idx,stmt_id).unwrap_or_default();
        out.push(fact_from_pairs(&[
            ("id",       text.as_str()),
            ("kind",     kind.as_str()),
            ("modality", "alethic"),
            ("text",     text.as_str()),
            ("entity",   entity.as_str()),
        ]));
    }
    out
}

/// Detect a conditional ring-constraint shape in a statement text.
/// Mirrors legacy `try_ring`'s Pass 2b conditional dispatcher:
///
///   - antecedent role tokens (after subscript strip) all share one
///     base noun type
///   - consequent contains the same base noun
///   - the (has_and, impossible, itself_in_consequent,
///     is_not_in_antecedent) matrix picks a ring kind
///
/// Returns the ring kind (`TR` / `AS` / `SY` / `AT` / `IT` / `RF`)
/// or `None` when the statement doesn't match a ring shape.
///
/// MC2 (#713): the boolean-tuple → kind dispatch matrix lives in
/// `ConditionalRingMatrix`; this function only recognises the shape
/// signals. The translator threads `ConditionalRingMatrix::boot()` for
/// bare callers and the cell-driven instance for the Stage-2 path.
fn conditional_ring_kind(
    text: &str,
    declared_nouns: &[String],
    matrix: &ConditionalRingMatrix,
) -> Option<String> {
    if !text.starts_with("If ") { return None; }
    let then_idx = text.find(" then ")?;
    let antecedent = &text[3..then_idx];
    let consequent = &text[then_idx + 6..];

    // Helper: strip a trailing digit subscript from a token.
    // `Noun1` → "Noun"; `Noun` → "Noun".
    let strip_subscript = |w: &str| -> String {
        let trimmed = w.trim_end_matches(',');
        let end = trimmed.char_indices()
            .rev()
            .take_while(|(_, c)| c.is_ascii_digit())
            .map(|(i, _)| i)
            .last()
            .unwrap_or(trimmed.len());
        trimmed[..end].to_string()
    };

    let role_bases: Vec<String> = antecedent.split_whitespace()
        .filter_map(|w| {
            let base = strip_subscript(w);
            if declared_nouns.iter().any(|n| n.as_str() == base.as_str()) {
                Some(base)
            } else {
                None
            }
        })
        .collect();
    if role_bases.len() < 2 { return None; }
    let first = &role_bases[0];
    if !role_bases.iter().all(|b| b == first) { return None; }

    let consequent_body = consequent
        .strip_prefix("it is impossible that ")
        .unwrap_or(consequent);
    let consequent_has_same_noun = consequent_body.split_whitespace()
        .any(|w| strip_subscript(w) == *first);
    if !consequent_has_same_noun { return None; }

    let pattern = encode_conditional_ring_pattern(antecedent, consequent)?;
    matrix.kind_for(&pattern).map(String::from)
}

/// FORML2 / NORMA canonical negation marker for the conditional
/// ring shape. NORMA's `VerbalizationCoreSnippets.xml` defines a
/// single alethic-negative `ModalPossibilityOperator` —
/// `it is impossible that` (line 91 / 97). That pattern is matched
/// elsewhere in this file via `consequent.starts_with("it is
/// impossible that ")` → `impossible` marker.
///
/// The `is not` substring is a Halpin-textbook prose form (cf.
/// `Information Modeling and Relational Databases`, Halpin & Morgan
/// §6.3) used in informal conditional renderings of asymmetry: `If
/// A1 R A2 then A2 is not R A1.` The matcher recognises it as the
/// `isnot-conse` / `isnot-ante` pattern when no other canonical
/// marker fires.
///
/// Anything else (`does not`, `cannot`, `never`, …) is NOT in the
/// FORML2 / NORMA / Halpin reference set. The matcher will fail to
/// classify those clauses, and the statement falls through as
/// unrecognised — preferable to silent mis-classification. Authors
/// should use the canonical trailing-marker form
/// (`A R A is asymmetric.`) or NORMA's `it is impossible that`
/// conditional form.
fn clause_is_negated(text: &str) -> bool {
    text.contains(" is not ")
}

/// Encode the (has_and, impossible, itself_in_consequent,
/// is_not_in_antecedent, is_not_in_consequent) boolean tuple into the
/// pattern name used to key into `ConditionalRingMatrix`. Returns
/// `None` for tuples that don't correspond to any recognised ring
/// shape (e.g. `impossible + itself_in_consequent`).
fn encode_conditional_ring_pattern(
    antecedent: &str,
    consequent: &str,
) -> Option<String> {
    let has_and = antecedent.contains(" and ");
    let impossible = consequent.starts_with("it is impossible that ");
    let itself_in_consequent = consequent.contains(" itself");
    let is_not_in_antecedent = clause_is_negated(antecedent);
    let is_not_in_consequent = clause_is_negated(consequent);

    // Fail closed on non-canonical English negation. If the consequent
    // contains any common negation word that is NOT one of the
    // canonical FORML2 / NORMA / Halpin markers (`it is impossible
    // that`, ` is not `, ` itself`), the matcher must NOT fall
    // through to the positive `plain` shape — that would silently
    // classify a denial as a symmetric (require-reverse) constraint
    // and reject every directed edge as a "missing reverse"
    // violation. Authors using prose like `does not block` or `cannot
    // block` should switch to the canonical trailing-marker form
    // (`<FT> is asymmetric.`) or NORMA's `it is impossible that`.
    // #786 — hints lift to NonCanonicalNegationHintTable.
    let hints = NonCanonicalNegationHintTable::boot();
    if hints.any_match(consequent) && !impossible && !is_not_in_consequent {
        return None;
    }

    // #786 — pattern dispatch lifts to ConditionalRingPatternTable
    // with Option<bool> wildcards preserving the legacy match-arm
    // semantics (first-row-match wins). Pattern shapes:
    //   AT: (true, true, _, true, _) → and+impossible+isnot-ante
    //   IT: (true, true, _, false, _) → and+impossible
    //   TR: (true, false, _, _, _) → and
    //   AS via "impossible": (false, true, false, _, _) → impossible
    //   AS via "is not": (false, false, false, _, true) → isnot-conse
    //   RF: (false, false, true, _, _) → itself-conse
    //   SY: (false, false, false, _, false) → plain
    let table = ConditionalRingPatternTable::boot();
    let name = table.match_signals(
        has_and, impossible, itself_in_consequent,
        is_not_in_antecedent, is_not_in_consequent,
    )?;
    Some(name.to_string())
}

/// Translate `Derivation Rule` classifications into `DerivationRule`
/// cell facts. Stage-2 emits a minimal skeleton — id + text —
/// matching the existing cell shape's `id` / `text` /
/// `consequentFactTypeId` / `json` bindings. Full Halpin resolution
/// (join keys, antecedent filters, consequent bindings,
/// consequent aggregates) stays in the Rust classifier for now and
/// will migrate in a follow-up commit once the
/// FactType + Role cells have been populated by Stage-2 earlier in
/// the pipeline.
pub fn translate_derivation_rules(classified_state: &Object, idx: &StmtIndex) -> Vec<Object> {
    translate_derivation_rules_with_matrix(
        classified_state, idx, &ConditionalRingMatrix::boot(), &[])
}

/// MC2 (#713) cell-driven variant. Stage-2's
/// `parse_to_state_via_stage12_impl` builds the matrix once from the
/// cached grammar state and threads it through the conditional-ring
/// arbitration check. Bare callers get the `boot()` fallback.
///
/// `ft_facts` is the FactType cell already emitted by
/// `translate_fact_types` for this same parse. When non-empty it
/// drives the `consequentFactTypeId` resolution that lets
/// `compile.rs::cell_index_from_state` route the rule to its target
/// cell and forward-chain firing pick it up. Callers that don't have
/// FT facts in scope (the older unit tests) pass `&[]` and accept the
/// empty consequent id — `re_resolve_rules` (compile-time fallback)
/// will retry from the rule text alone.
pub fn translate_derivation_rules_with_matrix(
    classified_state: &Object,
    idx: &StmtIndex,
    conditional_matrix: &ConditionalRingMatrix,
    ft_facts: &[Object],
) -> Vec<Object> {
    let table = StatementTranslatorTable::boot();
    let kinds: Vec<&str> = table.kinds_for("translate_derivation_rules");
    let statement_ids = collect_statement_ids(idx);
    let declared_nouns = declared_noun_names(classified_state);
    let mut out: Vec<Object> = Vec::new();
    for stmt_id in statement_ids.iter() {
        if !kinds.iter().any(|k| classifications_contains(idx, stmt_id, k)) {
            continue;
        }
        let text = statement_text(idx,stmt_id).unwrap_or_default();
        // Arbitrate with `translate_set_constraints`: when the
        // Statement also classifies as Subset Constraint AND the
        // antecedent has ≥2 distinct declared nouns, the SS
        // translator claims this statement — skip DR emission.
        // Legacy's pass-2b priority gives try_subset first dibs;
        // only on semantic failure does try_derivation take over.
        let is_subset = classifications_contains(idx,stmt_id, "Subset Constraint");
        if is_subset && antecedent_distinct_nouns(&text, &declared_nouns) >= 2 {
            continue;
        }
        // Arbitrate with `translate_ring_constraints`: when the
        // statement matches a conditional ring shape (all antecedent
        // role tokens share a base noun, consequent matches), the
        // ring translator claims it — skip DR emission.
        if conditional_ring_kind(
            &text, &declared_nouns, conditional_matrix).is_some()
        {
            continue;
        }
        let id = derivation_rule_id(&text);
        let consequent_ft = resolve_consequent_fact_type_id(&text, ft_facts);
        out.push(fact_from_pairs(&[
            ("id",                   id.as_str()),
            ("text",                 text.as_str()),
            ("consequentFactTypeId", consequent_ft.as_str()),
        ]));
    }
    out
}

/// Match a derivation rule's consequent text against the readings of
/// the declared FactTypes. Returns the FT id of the longest reading
/// that prefixes the consequent (or equals it), or empty string if no
/// match. Empty when `ft_facts` is empty so legacy callers that don't
/// thread FT facts get the same `""` they used to.
///
/// Subscript-aware (#828 follow-up): when the rule consequent uses a
/// subscripted role-noun token (`Task1 has Task Readiness 'blocked'`
/// in a ring-FT derivation), the verbatim text doesn't prefix-match
/// the bare reading `Task has Task Readiness`. We try matching twice:
/// once verbatim, once with trailing-digit subscripts stripped from
/// every Title-cased token. Mirrors `parse_role_token`'s policy of
/// treating trailing ASCII digits on a Title-case token as a Halpin
/// numeric subscript rather than part of the noun name.
fn resolve_consequent_fact_type_id(rule_text: &str, ft_facts: &[Object]) -> String {
    if ft_facts.is_empty() { return String::new(); }
    let consequent = derivation_rule_consequent(rule_text);
    let consequent_no_subs = strip_role_subscripts(consequent);
    let mut best: (usize, &str) = (0, "");
    for ft in ft_facts {
        let Some(reading) = binding(ft, "reading") else { continue };
        if reading.is_empty() { continue }
        let matched = consequent_matches_reading(consequent, reading)
            || consequent_matches_reading(&consequent_no_subs, reading);
        if matched && reading.len() > best.0 {
            if let Some(id) = binding(ft, "id") {
                best = (reading.len(), id);
            }
        }
    }
    best.1.to_string()
}

/// Reading must either equal the consequent or be followed by a space
/// (next role) or `'` (literal value) — preventing `Task has Task
/// Readiness` from spuriously matching a longer consequent like
/// `Task has Task Readiness Score 5`.
fn consequent_matches_reading(consequent: &str, reading: &str) -> bool {
    if consequent == reading { return true; }
    if let Some(rest) = consequent.strip_prefix(reading) {
        return rest.starts_with(' ') || rest.starts_with('\'');
    }
    false
}

/// Strip trailing ASCII-digit subscripts from each whitespace-separated
/// token whose first byte is an ASCII uppercase letter. `Task1` →
/// `Task`, `IPv4` → `IPv` (matching `parse_role_token`'s policy).
/// Tokens like `'blocked'` (literals) and `has` (lowercase verbs)
/// pass through unchanged.
fn strip_role_subscripts(text: &str) -> String {
    text.split_whitespace().map(|word| {
        let bytes = word.as_bytes();
        if bytes.first().map_or(false, |b| b.is_ascii_uppercase()) {
            let boundary = word.char_indices().rev()
                .take_while(|(_, c)| c.is_ascii_digit())
                .last().map(|(i, _)| i).unwrap_or(word.len());
            word[..boundary].to_string()
        } else {
            word.to_string()
        }
    }).collect::<Vec<_>>().join(" ")
}

/// Extract the consequent text from a derivation rule. Strips bullet
/// markers and trailing terminator, then returns everything before the
/// leftmost ` iff ` / ` if ` / ` when ` keyword. Falls back to the
/// stripped rule text when no antecedent keyword is present (well-
/// formed rules always have one, but the parser's non-rule paths may
/// still feed in stray classifications).
fn derivation_rule_consequent(rule_text: &str) -> &str {
    let mut t = rule_text.trim();
    for prefix in ["* ", "** ", "+ "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            t = rest.trim_start();
            break;
        }
    }
    t = t.trim_end_matches('.').trim_end();
    let split_keywords: &[&str] = &[" iff ", " if ", " when "];
    match split_keywords.iter().filter_map(|kw| t.find(kw)).min() {
        Some(idx) => t[..idx].trim_end(),
        None => t,
    }
}

/// Scan derivation rule antecedents for clauses that don't match any
/// declared FactType reading. Emits `UnresolvedClause` facts with
/// `clause`, `ruleText`, and `ruleId` bindings — `check.rs`'s
/// `check_unresolved_clauses` reads these to surface resolve warnings
/// on ambiguous or unknown antecedents.
///
/// A clause matches a FactType if the FactType's canonical reading
/// (e.g. `Order has Amount`) appears verbatim in the clause after
/// stripping the canonical subject-role pronoun/prefix ("that",
/// subscripts). For the common shape
/// `<rule-consequent> if|when <ante> and <ante> and …`, each
/// `and`-separated chunk is a clause candidate.
pub fn translate_unresolved_clauses(
    classified_state: &Object,
    idx: &StmtIndex,
    _ft_facts: &[Object],
) -> Vec<Object> {
    let statement_ids = collect_statement_ids(idx);
    // Build the set of WORDS that appear anywhere in a declared noun
    // name — `HTTP Status` declared contributes both `HTTP` and
    // `Status`. A clause is resolved if every Title-case word it
    // contains is in this set (minus the prose-stopword allow list).
    let declared_words: hashbrown::HashSet<String> = declared_noun_names(classified_state)
        .iter()
        .flat_map(|n| n.split_whitespace().map(String::from).collect::<Vec<_>>())
        .collect();
    let declared: hashbrown::HashSet<String> = declared_noun_names(classified_state)
        .into_iter().collect();
    let _ = &declared;
    let mut out: Vec<Object> = Vec::new();
    for stmt_id in statement_ids.iter() {
        if !classifications_contains(idx,stmt_id, "Derivation Rule") { continue; }
        let text = match statement_text(idx,stmt_id) {
            Some(t) => t, None => continue,
        };
        let split_keywords: &[&str] = &[" iff ", " if ", " when "];
        let Some(ante_start) = split_keywords.iter()
            .filter_map(|kw| text.find(kw).map(|i| i + kw.len()))
            .min() else { continue };
        let antecedent = text[ante_start..].trim_end_matches('.').trim();
        let rule_id = derivation_rule_id(&text);
        // Heuristic: a clause is unresolved when it contains at least
        // one Title-case word that isn't a declared noun (modulo the
        // usual pronoun / quantifier prose) — these are the "Mystery"
        // / "Phantom" tokens legacy's resolver flags. Clauses that
        // only reference declared nouns are assumed to resolve; the
        // full join-path resolver that would say otherwise is out of
        // scope here. #789 — stopword list lifts to ProseStopwordTable
        // so the data lives in `readings/forml2-grammar.md` rather
        // than as a const here. boot() falls back to the historic
        // 12-word list when the bare engine has no metamodel loaded.
        let stopwords = ProseStopwordTable::boot();
        for clause in antecedent.split(" and ") {
            let clause = clause.trim();
            if clause.is_empty() { continue; }
            let has_unknown_titlecase = clause.split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .filter(|w| w.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false))
                .filter(|w| !stopwords.contains(w))
                .any(|w| {
                    // Strip trailing digits (subscripted `Order1`).
                    let base: String = w.trim_end_matches(|c: char| c.is_ascii_digit()).into();
                    !declared_words.contains(&base) && !declared_words.contains(w)
                });
            if has_unknown_titlecase {
                out.push(fact_from_pairs(&[
                    ("clause",   clause),
                    ("ruleText", text.as_str()),
                    ("ruleId",   rule_id.as_str()),
                ]));
            }
        }
    }
    out
}

/// FNV-1a 64-bit hash of the rule text, formatted as `rule_<hex>` to
/// match legacy's stable id. Multiple rules may share a consequent FT
/// (the grammar has 28 rules all producing `Statement has
/// Classification`), so keying on consequent alone collapses them;
/// text hashing gives each rule a unique id.
fn derivation_rule_id(text: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    alloc::format!("rule_{h:x}")
}

/// Translate `Enum Values Declaration` classifications into
/// `EnumValues` cell facts. Each statement contributes one fact with
/// `noun` bound to the Head Noun and one `value0`, `value1`, …
/// binding per captured enum value (same shape as
/// `enum_values_for_noun` expects — see parse_forml2::upsert_enum_values).
///
/// The Value Type `Noun` fact is still emitted by `translate_nouns`
/// from the preceding `Priority is a value type.` statement — this
/// translator only contributes the value list.
pub fn translate_enum_values(classified_state: &Object, idx: &StmtIndex) -> Vec<Object> {
    let table = StatementTranslatorTable::boot();
    let kinds: Vec<&str> = table.kinds_for("translate_enum_values");
    let statement_ids = collect_statement_ids(idx);
    let mut out: Vec<Object> = Vec::new();
    for stmt_id in statement_ids.iter() {
        if !kinds.iter().any(|k| classifications_contains(idx, stmt_id, k)) {
            continue;
        }
        let Some(noun) = head_noun_for(idx,stmt_id) else { continue };
        let values = enum_values_for(classified_state, stmt_id);
        if values.is_empty() { continue; }
        let mut pairs: Vec<(String, String)> = Vec::new();
        pairs.push(("noun".to_string(), noun));
        for (i, v) in values.iter().enumerate() {
            pairs.push((alloc::format!("value{i}"), v.clone()));
        }
        let pairs_ref: Vec<(&str, &str)> = pairs.iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        out.push(fact_from_pairs(&pairs_ref));
    }
    out
}

/// Translate set-comparison / multi-clause constraints into
/// `Constraint` cell facts. Kinds:
///
///   - EQ (`if and only if` keyword) — equality / bi-implication.
///   - XC (`at most one of the following holds` keyword, OR the
///         `are mutually exclusive` trailing marker form handled
///         by the Exclusion Constraint classification).
///   - XO (`exactly one of the following holds` keyword) —
///         exclusive-or.
///   - OR (`at least one of the following holds` keyword) —
///         disjunctive.
///
/// All four fire at alethic modality. Spans are deferred (same as
/// Ring / UC-MC-FC translators). This translator is separate from
/// `translate_cardinality_constraints` because the grammar keys the
/// two families on different tokens (Quantifier vs Constraint
/// Keyword vs Trailing Marker).
pub fn translate_set_constraints(classified_state: &Object, idx: &StmtIndex) -> Vec<Object> {
    let kind_table = SetConstraintKindTable::boot();
    let arbitration_registry = set_constraint_arbitration_registry();
    let statement_ids = collect_statement_ids(idx);
    let declared_nouns = declared_noun_names(classified_state);
    let mut out: Vec<Object> = Vec::new();
    for stmt_id in statement_ids.iter() {
        let text = statement_text(idx,stmt_id).unwrap_or_default();
        // Cascade through the registered set-constraint kinds in
        // declaration order. Each row carries a kind name, a kind
        // code, and an arbitration-rule name; the rule is resolved
        // through `set_constraint_arbitration_registry` and applied
        // to decide whether this kind defers to a different
        // translator (e.g. translate_derivation_rules) on this
        // particular Statement.
        let mut emit: Option<&str> = None;
        for (kind, code, rule_name) in kind_table.iter() {
            if !classifications_contains(idx, stmt_id, kind) { continue; }
            let arbitration = arbitration_registry.get(rule_name)
                .copied()
                .unwrap_or(skip_on_derivation_rule);
            if arbitration(&text, &declared_nouns, idx, stmt_id) {
                continue;
            }
            emit = Some(code);
            break;
        }
        let Some(kind_code) = emit else { continue };
        let entity = head_noun_for(idx,stmt_id).unwrap_or_default();
        out.push(fact_from_pairs(&[
            ("id",       text.as_str()),
            ("kind",     kind_code),
            ("modality", "alethic"),
            ("text",     text.as_str()),
            ("entity",   entity.as_str()),
        ]));
    }
    out
}

/// All declared noun names in a classified state, sorted longest-first
/// so substring-style matching prefers `Fact Type` over `Fact` etc.
fn declared_noun_names(state: &Object) -> Vec<String> {
    let cell = fetch_or_phi("Noun", state);
    let mut names: Vec<String> = cell.as_seq()
        .map(|s| s.iter()
            .filter_map(|f| binding(f, "name").map(String::from))
            .collect())
        .unwrap_or_default();
    names.sort_by(|a, b| b.len().cmp(&a.len()));
    names
}

/// Count the distinct declared-noun names that appear in the
/// antecedent of a `If ... then ...` shape. Used to match legacy's
/// `try_subset` pass-2b precedence: a subset constraint requires
/// antecedent-noun diversity ≥ 2, otherwise the derivation-rule
/// branch wins.
///
/// Longest-first pass with masking — `Fact Type` wins over `Fact`
/// when both are declared, preventing substring double-counts.
fn antecedent_distinct_nouns(text: &str, declared: &[String]) -> usize {
    let Some((ante, _)) = text.split_once(" then ") else { return 0 };
    let bytes = ante.as_bytes();
    let mut masked: Vec<bool> = alloc::vec![false; bytes.len()];
    let mut distinct: alloc::collections::BTreeSet<String> =
        alloc::collections::BTreeSet::new();
    // `declared` is already sorted longest-first by
    // `declared_noun_names`.
    for noun in declared {
        let needle = noun.as_str();
        if needle.is_empty() { continue; }
        let mut start = 0;
        while start <= bytes.len().saturating_sub(needle.len()) {
            let Some(rel) = ante[start..].find(needle) else { break };
            let abs = start + rel;
            let end = abs + needle.len();
            if (abs..end).any(|i| masked[i]) {
                start = abs + 1;
                continue;
            }
            for i in abs..end { masked[i] = true; }
            distinct.insert(noun.clone());
            start = end;
        }
    }
    distinct.len()
}

/// Translate Uniqueness / Mandatory Role / Frequency Constraint
/// classifications into `Constraint` cell facts. Kinds:
///
///   - UC (`at most one` or `exactly one` quantifier).
///   - MC (`at least one` quantifier).
///   - FC (both `at most` and `at least` without the `one` suffix).
///
/// All three fire at alethic modality. Spans (which role on which
/// fact type) are left empty here — fact-type resolution happens in
/// `translate_fact_types`, and span binding is a follow-up pass that
/// reads both cells. This matches the deferred-span shape used by
/// `translate_ring_constraints`.
pub fn translate_cardinality_constraints(classified_state: &Object, idx: &StmtIndex) -> Vec<Object> {
    let kind_table = CardinalityConstraintKindTable::boot();
    let arbitration_registry = cardinality_arbitration_registry();
    let statement_ids = collect_statement_ids(idx);
    let mut out: Vec<Object> = Vec::new();
    for stmt_id in statement_ids.iter() {
        // Apply every registered cardinality pre-gate predicate
        // uniformly: if any rule says skip, defer to whichever
        // translator owns the Statement. The two registered rules
        // are 'derivation_rule_wins' (DR's iff makes the whole
        // sentence a rule, even with a 'some' quantifier inside an
        // antecedent) and 'deontic_operator_wins' (a sentence
        // carrying It is forbidden/obligatory/permitted... is a
        // deontic over the body, not an alethic UC/MC).
        let skip = arbitration_registry.values()
            .any(|rule| rule(classified_state, idx, stmt_id));
        if skip { continue; }
        // Resolve the kind code from the registry. Order in
        // CardinalityConstraintKindTable::boot() puts FC before UC
        // before MC so FC precedence is preserved when a Statement
        // happens to carry multiple cardinality classifications.
        let Some(kind) = kind_table.code_for_statement(idx, stmt_id) else { continue };
        let text = statement_text(idx,stmt_id).unwrap_or_default();
        let entity = head_noun_for(idx,stmt_id).unwrap_or_default();

        // `exactly one` splits into UC + MC per legacy behavior
        // (ORM 2: cardinality of 1 is the conjunction of "at most
        // one" and "at least one"). Rewrite the text for each so
        // downstream consumers see the two expanded constraints.
        //
        // Restricted to `Each X ... exactly one Y` — the "For each
        // X, exactly one Y has that X" external-UC form is preserved
        // as a single UC per legacy behavior.
        if kind == "UC" && text.contains("exactly one") && text.starts_with("Each ") {
            let uc_text = text.replace("exactly one", "at most one");
            let mc_text = text.replace("exactly one", "some");
            out.push(fact_from_pairs(&[
                ("id", uc_text.as_str()), ("kind", "UC"),
                ("modality", "alethic"),  ("text", uc_text.as_str()),
                ("entity", entity.as_str()),
            ]));
            out.push(fact_from_pairs(&[
                ("id", mc_text.as_str()), ("kind", "MC"),
                ("modality", "alethic"),  ("text", mc_text.as_str()),
                ("entity", entity.as_str()),
            ]));
            continue;
        }

        out.push(fact_from_pairs(&[
            ("id",       text.as_str()),
            ("kind",     kind),
            ("modality", "alethic"),
            ("text",     text.as_str()),
            ("entity",   entity.as_str()),
        ]));
    }
    out
}

/// Translate `Value Constraint` classifications into `Constraint` cell
/// facts with kind="VC" and entity=<noun>. Fired by the grammar's
/// recursive rule `Value Constraint iff Enum Values Declaration`, so
/// every value-type noun with an enum-values list gets exactly one VC.
/// The span set is empty — the existing compiler reads enum values
/// from the EnumValues cell directly (see
/// `parse_forml2::enum_values_for_noun`) and attaches the constraint
/// to every role where the noun appears.
pub fn translate_value_constraints(classified_state: &Object, idx: &StmtIndex) -> Vec<Object> {
    let table = StatementTranslatorTable::boot();
    let kinds: Vec<&str> = table.kinds_for("translate_value_constraints");
    let statement_ids = collect_statement_ids(idx);
    let mut out: Vec<Object> = Vec::new();
    for stmt_id in statement_ids.iter() {
        if !kinds.iter().any(|k| classifications_contains(idx, stmt_id, k)) {
            continue;
        }
        let Some(noun) = head_noun_for(idx,stmt_id) else { continue };
        let id = alloc::format!("VC:{}", noun);
        let text = alloc::format!("{} has a value constraint", noun);
        out.push(fact_from_pairs(&[
            ("id",       id.as_str()),
            ("kind",     "VC"),
            ("modality", "alethic"),
            ("text",     text.as_str()),
            ("entity",   noun.as_str()),
        ]));
    }
    out
}

fn enum_values_for(state: &Object, stmt_id: &str) -> Vec<String> {
    fetch_or_phi("Statement_has_Enum_Value", state)
        .as_seq()
        .map(|facts| facts.iter()
            .filter(|f| binding(f, "Statement") == Some(stmt_id))
            .filter_map(|f| binding(f, "Enum_Value").map(String::from))
            .collect())
        .unwrap_or_default()
}

/// Translate `Deontic Constraint` classifications into `Constraint`
/// cell facts with modality="deontic" and the stripped deontic
/// operator. Entity defaults to the Head Noun of the body (after
/// the `It is X that` prefix was stripped by Stage-1).
pub fn translate_deontic_constraints(classified_state: &Object, idx: &StmtIndex) -> Vec<Object> {
    translate_deontic_constraints_with_table(
        classified_state, idx, &DeonticShapeTable::boot())
}

/// MC2 (#713) cell-driven variant. The (kind, modality) pair emitted
/// per deontic operator is read from `DeonticShapeTable`, which
/// `parse_to_state_via_stage12_impl` builds from the cached grammar
/// state's parallel `Deontic Operator` / `Deontic Constraint Kind
/// Code` / `Deontic Constraint Modality` enum-value declarations.
/// When a statement is classified deontic but carries no operator
/// (e.g. a `Quantifier` shadowed the operator atom), we fall back to
/// the first table row's shape — matching the legacy hardcoded
/// `kind="UC", modality="deontic"` defaults.
pub fn translate_deontic_constraints_with_table(
    classified_state: &Object,
    idx: &StmtIndex,
    deontic_shapes: &DeonticShapeTable,
) -> Vec<Object> {
    let table = StatementTranslatorTable::boot();
    let kinds: Vec<&str> = table.kinds_for("translate_deontic_constraints");
    let statement_ids = collect_statement_ids(idx);
    let mut out: Vec<Object> = Vec::new();
    for stmt_id in statement_ids.iter() {
        // Per forml2-grammar.md:141-143, a Statement is classified as
        // 'Deontic Constraint' iff it carries one of the Deontic
        // Operator literals ('obligatory' / 'forbidden' / 'permitted').
        // The classifier rule's fixpoint is supposed to add the
        // 'Deontic Constraint' tag, but the meta-circular grammar
        // pipeline doesn't always materialise it for statements that
        // ALSO carry a Quantifier (the cardinality classifier wins
        // the dispatch). The translator's source-of-truth signal is
        // the deontic operator itself, recorded by stage1 in
        // `Statement_has_Deontic_Operator`. Check that directly so
        // shapes like `It is forbidden that some X is Y'd ...` (where
        // a 'some' inner quantifier sits next to the forbidden prefix)
        // still emit a deontic-modality constraint.
        let op = deontic_operator_for(classified_state, stmt_id);
        let classified_deontic = kinds.iter()
            .any(|k| classifications_contains(idx, stmt_id, k));
        if op.is_none() && !classified_deontic { continue; }
        let text = statement_text(idx,stmt_id).unwrap_or_default();
        let op_str = op.unwrap_or_default();
        let entity = head_noun_for(idx,stmt_id).unwrap_or_default();
        // Resolve emission shape via the cell-driven table. Fall back
        // to the first table row when the operator isn't enumerated
        // (defensive — the parallel-enum invariant guarantees a row
        // per operator, but a malformed reading could leave the
        // operator unmapped; the boot fallback row always carries
        // ("UC", "deontic") so behaviour matches the pre-MC2 hardcode).
        let (kind, modality) = deontic_shapes.shape_for(&op_str)
            .or_else(|| deontic_shapes.rows.first()
                .map(|(_, k, m)| (k.as_str(), m.as_str())))
            .unwrap_or(("UC", "deontic"));
        // MC4b (#751): recognise per-fact text predicates of shape
        // `<role> [does not] (ends|starts) with '<lit>'`. Only the
        // `ends with` family motivates this task (singular naming);
        // `starts with` is a near-zero-cost peer. Empty `predicate`
        // cell field means no per-fact filter — the population path
        // emits one violation per fact unconditionally as before.
        let predicate = parse_deontic_text_predicate(&text);
        let mut pairs: alloc::vec::Vec<(&str, &str)> = alloc::vec![
            ("id",               text.as_str()),
            ("kind",             kind),
            ("modality",         modality),
            ("deonticOperator",  op_str.as_str()),
            ("text",             text.as_str()),
            ("entity",           entity.as_str()),
        ];
        let predicate_encoded;
        if let Some(p) = &predicate {
            predicate_encoded = p.encode();
            pairs.push(("predicate", predicate_encoded.as_str()));
        }
        out.push(fact_from_pairs(&pairs));
    }
    out
}

/// MC4b (#751): tight-grammar parser for per-fact text predicates
/// inside a deontic constraint body. Matches three lowercase shapes
/// only — `<role> ends with '<lit>'`, `<role> does not end with '<lit>'`,
/// `<role> starts with '<lit>'`, `<role> does not start with '<lit>'`.
/// `<role>` is whatever lowercase identifier (no spaces) follows the
/// last `has a` / `has` segment in the body, e.g.
/// `It is forbidden that each Noun has a name that ends with 'ies'`
/// extracts role=`name`, literal=`ies`, negated=`false`. Anything
/// else returns `None` and falls through to the existing population
/// path unchanged.
fn parse_deontic_text_predicate(text: &str) -> Option<crate::types::DeonticPredicate> {
    use crate::types::DeonticPredicate;
    // Strip the `It is X that ` deontic prefix.
    let body = text
        .strip_prefix("It is forbidden that ")
        .or_else(|| text.strip_prefix("It is obligatory that "))
        .or_else(|| text.strip_prefix("It is permitted that "))
        .unwrap_or(text);

    // The shape we accept anchors on `that <ends|starts> with '...'` or
    // `that does not <ends|starts> with '...'`. Walking right-to-left
    // off the closing apostrophe is robust to the prose preceding it
    // (`each Noun has a name that …`, `each Activity has a code that
    // …`, etc.) without committing to a full English grammar.
    let trimmed = body.trim_end_matches('.').trim();
    let bytes = trimmed.as_bytes();
    let last_quote = bytes.iter().rposition(|&b| b == b'\'')?;
    if last_quote == 0 { return None; }
    let prev_quote = bytes[..last_quote].iter().rposition(|&b| b == b'\'')?;
    let literal = &trimmed[prev_quote + 1..last_quote];
    if literal.is_empty() { return None; }

    let prefix = trimmed[..prev_quote].trim_end();
    // #788 — match one of the four shapes via DeonticPredicateOperatorTable.
    // boot() carries the historic suffix cascade; future additions
    // (matches/contains/gt/lt) land as enum-value rows in readings.
    let table = DeonticPredicateOperatorTable::boot();
    let (head_end, op_kind, negated) = table.match_suffix(prefix)?;

    // Role = the lowercase token sitting in `has a <role> that` /
    // `has <role> that` immediately before the predicate. The shapes
    // we accept always read `… has [a] <role> that <pred> '<lit>'`,
    // so peel `that` first, then peel `has [a]`. Anything that
    // doesn't match returns None.
    let head_end = head_end.strip_suffix(" that").unwrap_or(head_end).trim_end();
    // Pop the role token off the right.
    let role_owned: String = match head_end.rfind(' ') {
        Some(i) => head_end[i + 1..].to_string(),
        None    => return None,
    };
    let pre_role = head_end[..head_end.len() - role_owned.len()].trim_end();
    let _has_match = pre_role.ends_with(" has a") || pre_role.ends_with(" has");
    if !_has_match { return None; }
    // Single-token, lowercase identifier only.
    if role_owned.is_empty()
        || role_owned.contains(' ')
        || role_owned.chars().next().is_some_and(|c| c.is_ascii_uppercase())
    { return None; }
    let literal_owned = literal.to_string();
    Some(match op_kind {
        "ends_with"   => DeonticPredicate::EndsWith   { role: role_owned, literal: literal_owned, negated },
        "starts_with" => DeonticPredicate::StartsWith { role: role_owned, literal: literal_owned, negated },
        _ => return None,
    })
}

fn deontic_operator_for(state: &Object, stmt_id: &str) -> Option<String> {
    fetch_or_phi("Statement_has_Deontic_Operator", state)
        .as_seq()?
        .iter()
        .find(|f| binding(f, "Statement") == Some(stmt_id))
        .and_then(|f| binding(f, "Deontic_Operator").map(String::from))
}

/// Resolve a trailing-marker phrase to its ring constraint kind code
/// via the (cell-driven) `RingKindTable`. The hardcoded match arms
/// that lived here previously have moved to `RingKindTable::boot()` —
/// MC2 (#713) wants this dispatch surface to live in the readings.
fn ring_adjective_to_kind(marker: &str, table: &RingKindTable)
    -> Option<String>
{
    table.kind_for(marker).map(String::from)
}

fn trailing_marker_for(idx: &StmtIndex, stmt_id: &str) -> Option<String> {
    idx.trailing_markers.get(stmt_id).cloned()
}

fn role_noun_at_position(state: &Object, stmt_id: &str, position: usize) -> Option<String> {
    let refs = fetch_or_phi("Statement_has_Role_Reference", state);
    let refs_seq = refs.as_seq()?;
    let role_ids: Vec<String> = refs_seq.iter()
        .filter(|f| binding(f, "Statement") == Some(stmt_id))
        .filter_map(|f| binding(f, "Role_Reference").map(String::from))
        .collect();
    let positions = fetch_or_phi("Role_Reference_has_Role_Position", state);
    let pos_seq = positions.as_seq()?;
    let head_nouns = fetch_or_phi("Role_Reference_has_Head_Noun", state);
    let hn_seq = head_nouns.as_seq()?;
    // Find the role_id at the requested position.
    let target_id = role_ids.iter().find(|id| {
        pos_seq.iter().any(|f| {
            binding(f, "Role_Reference") == Some(id.as_str())
                && binding(f, "Role_Position") == Some(&position.to_string())
        })
    })?;
    hn_seq.iter()
        .find(|f| binding(f, "Role_Reference") == Some(target_id.as_str()))
        .and_then(|f| binding(f, "Head_Noun").map(String::from))
}

fn head_noun_for(idx: &StmtIndex, stmt_id: &str) -> Option<String> {
    idx.head_nouns.get(stmt_id).cloned()
}

/// Return the list of classification names attached to a given
/// Statement id.
pub fn classifications_for(idx: &StmtIndex, statement_id: &str) -> Vec<String> {
    idx.classifications.get(statement_id).cloned().unwrap_or_default()
}

/// Fast membership check — returns `true` when `statement_id` carries
/// a classification equal to `name`. Avoids the `Vec<String>` clone
/// that `classifications_for` pays on every call.
pub fn classifications_contains(idx: &StmtIndex, statement_id: &str, name: &str) -> bool {
    idx.classifications.get(statement_id)
        .is_some_and(|v| v.iter().any(|k| k == name))
}

/// Fast disjoint-membership check — returns `true` when any of the
/// given names matches a classification on `statement_id`.
pub fn classifications_contains_any(idx: &StmtIndex, statement_id: &str, names: &[&str]) -> bool {
    idx.classifications.get(statement_id)
        .is_some_and(|v| v.iter().any(|k| names.iter().any(|n| *n == k.as_str())))
}

/// Collect all Statement ids — refcount bump on the cached `Arc<Vec<String>>`.
fn collect_statement_ids(idx: &StmtIndex) -> alloc::sync::Arc<Vec<String>> {
    idx.statement_ids.clone()
}

/// End-to-end Stage-1 + Stage-2 pipeline: FORML 2 source text → final
/// metamodel cell state (Noun / Subtype / FactType / Role / Constraint /
/// DerivationRule / InstanceFact / EnumValues).
///
/// #294 diagnostic harness target; #285 capstone wire-up will replace
/// the legacy `parse_into` cascade with a call to this function.
///
/// Pipeline:
///   1. Parse the bundled `readings/forml2-grammar.md` to a grammar
///      state (the Classification vocabulary + recognizer rules).
///   2. Bootstrap the noun list from the legacy parser. (#285 will
///      remove this; for the diagnostic it's fine — the point is to
///      drive Stage-2 with a known-correct noun set and diff the
///      downstream translators.)
///   3. Split the source into statement lines (reusing the legacy
///      continuation-joiner so authored multi-line rules fold).
///   4. Run `tokenize_statement` on each non-empty, non-comment line.
///   5. Merge all per-statement cells into one state, then apply
///      `classify_statements` to emit `Statement_has_Classification`.
///   6. Run every per-kind translator and assemble the result.
/// Process-wide cache for the bundled FORML 2 grammar: the parsed
/// state AND its compiled defs. Both are pure functions of the
/// committed `readings/forml2-grammar.md`, so neither has to be
/// redone per `parse_to_state_via_stage12` call.
///
/// Killed two perf cliffs: the legacy parse of the grammar MD (~140
/// lines) and the compile-to-defs pass (~20ms/call for 69
/// classification rules).
type GrammarCacheEntry = (
    Object,                                             // expanded grammar state
    alloc::vec::Vec<(String, crate::ast::Func)>,        // classifier defs only
    alloc::vec::Vec<Vec<String>>,                       // classifier antecedent cells
    hashbrown::HashSet<u64>,                            // cached state_keys of expanded grammar
);

// `spin::Once` (alloc-compatible) instead of `std::sync::OnceLock` so
// the cache resolves under no_std (kernel build). Stores
// `Result<GrammarCacheEntry, String>` because `spin::Once::call_once`
// takes a non-fallible closure — the failable bootstrap runs inside
// the closure, captures any error in the cached value, and subsequent
// callers re-read the same Result. First failure is permanent for the
// process lifetime (matches the legacy `OnceLock::set` semantics
// where a failed init left the cache empty and any later attempt would
// re-fail identically anyway, since the input is `include_str!`-baked).
//
// M5 (#698): SANCTIONED MEMOIZATION EXEMPTION.
//
// The audit's purity sweep flagged this static as ambient state. It
// is — and it stays. The exemption is on the same footing as the
// `Func` AOT cache (#318) and the per-process compile-defs cache:
// the input is `include_str!`-baked at build time (the
// `forml2-grammar.md` byte buffer is part of the binary), the
// computation is observably referentially transparent, and the
// cached value is the parse result of that same buffer every time.
// `cached_grammar()` is morally `parse(GRAMMAR)` — pure — and the
// `Once` is an implementation detail behind that pure interface.
// Removing it would re-parse the grammar (~50 ms cold) on every
// stage-12 entry, including from the kernel super-loop. Same
// rationale Backus's interpreter would invoke: a fixed program
// over a fixed input, computed once.
static GRAMMAR_CACHE: spin::Once<Result<GrammarCacheEntry, String>>
    = spin::Once::new();

/// Stage-0 grammar bootstrap (#285 follow-up). Parses the narrow subset
/// of FORML 2 shapes used by `readings/forml2-grammar.md` directly into
/// the same cell map that Stage-1+Stage-2 would produce — so
/// `cached_grammar` can populate the classifier cache without recursing
/// through the full parser (stage12 needs this cache before it can
/// classify its own grammar) and without pulling in the legacy
/// markdown-cascade parser.
///
/// Recognised shapes (exactly what the grammar file uses):
///   - `X(.ref) is an entity type.` / `X is an entity type.`
///   - `X is a value type.`
///   - `The possible values of X are 'a', 'b', ...`
///   - `A has B.` — binary fact type reading (no literals)
///   - `A has B 'lit' iff …` / `A has B iff …` — classifier rules
///   - `Class 'Value' is a Class.` — documentary instance facts;
///     legacy's `parse_general_instance_fact` emits nothing for these,
///     so we skip them silently.
///
/// Output `Object::Map` carries the same cells the legacy cascade
/// would: `Noun` (with `referenceScheme` + `enumValues` enrichment),
/// `RefScheme`, `EnumValues`, `FactType`, `Role`, `DerivationRule`.
/// `DerivationRule` facts include a lossless `json` binding so
/// `compile_to_defs_state` takes the no-resolve fast path (grammar
/// rules never feed through `re_resolve_rules`).
fn bootstrap_grammar_state(text: &str) -> Result<Object, String> {
    use crate::types::{
        DerivationRuleDef, DerivationKind, AntecedentSource,
        AntecedentRoleLiteral, ConsequentRoleLiteral, ConsequentCellSource,
    };

    struct RawNoun {
        name: String,
        object_type: &'static str,
        ref_scheme: Option<Vec<String>>,
    }
    let mut raw_nouns: Vec<RawNoun> = Vec::new();
    let mut enum_values_by_noun: HashMap<String, Vec<String>> = HashMap::new();
    let mut fact_types: Vec<Object> = Vec::new();
    let mut roles: Vec<Object> = Vec::new();
    // (id, text, consequent_ft_encoded, json)
    let mut derivation_rules_info: Vec<(String, String, String, String)> = Vec::new();

    fn fnv1a64(s: &str) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in s.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    fn extract_entity_decl(before: &str) -> (String, Option<Vec<String>>) {
        match before.find('(') {
            Some(p) => {
                let name = before[..p].trim().to_string();
                let tail = &before[p + 1..];
                let end = tail.find(')').unwrap_or(tail.len());
                let parts: Vec<String> = tail[..end]
                    .split(',')
                    .map(|s| s.trim().trim_start_matches('.').trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                (name, if parts.is_empty() { None } else { Some(parts) })
            }
            None => (before.trim().to_string(), None),
        }
    }

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let body = line.trim_end_matches('.').trim();

        // 1. Entity type.
        if let Some(before) = body.strip_suffix(" is an entity type") {
            let (name, ref_scheme) = extract_entity_decl(before.trim());
            raw_nouns.push(RawNoun { name, object_type: "entity", ref_scheme });
            continue;
        }

        // 2. Value type.
        if let Some(before) = body.strip_suffix(" is a value type") {
            raw_nouns.push(RawNoun {
                name: before.trim().to_string(),
                object_type: "value",
                ref_scheme: None,
            });
            continue;
        }

        // 3. Enum values.
        if let Some(rest) = body.strip_prefix("The possible values of ") {
            let (noun_name, values_part) = rest.split_once(" are ")
                .ok_or_else(|| format!("grammar bootstrap: malformed enum: {}", line))?;
            let noun_name = noun_name.trim();
            let values: Vec<String> = values_part.split(',')
                .map(|s| {
                    let s = s.trim();
                    s.strip_prefix('\'').and_then(|v| v.strip_suffix('\''))
                        .unwrap_or(s).to_string()
                })
                .filter(|s| !s.is_empty())
                .collect();
            enum_values_by_noun.insert(noun_name.to_string(), values);
            continue;
        }

        // 4. Derivation rule.
        if body.contains(" iff ") {
            let rule_text = body.to_string();
            let spec = parse_classification_rule_spec(body)
                .ok_or_else(|| format!("grammar bootstrap: could not parse classifier rule: {}", line))?;
            let (classification, clauses) = spec;
            let id = format!("rule_{:x}", fnv1a64(&rule_text));

            let antecedent_sources: Vec<AntecedentSource> = clauses.iter()
                .map(|(cell_name, _)| AntecedentSource::FactType(cell_name.clone()))
                .collect();
            let antecedent_role_literals: Vec<AntecedentRoleLiteral> = clauses.iter()
                .enumerate()
                .filter_map(|(i, (cell_name, lit))| lit.as_ref().map(|v| AntecedentRoleLiteral {
                    antecedent_index: i,
                    role: cell_name.strip_prefix("Statement_has_")
                        .unwrap_or(cell_name.as_str())
                        .replace('_', " "),
                    value: v.clone(),
                }))
                .collect();
            let consequent_role_literals = alloc::vec![ConsequentRoleLiteral {
                role: "Classification".into(),
                value: classification.clone(),
            }];

            let consequent_cell = ConsequentCellSource::Literal(
                "Statement_has_Classification".into(),
            );
            let consequent_ft_encoded = consequent_cell.encode();

            let rule = DerivationRuleDef {
                id: id.clone(),
                text: rule_text.clone(),
                antecedent_sources,
                consequent_instance_role: String::new(),
                consequent_cell,
                kind: DerivationKind::ModusPonens,
                join_on: Vec::new(),
                match_on: Vec::new(),
                consequent_bindings: Vec::new(),
                antecedent_filters: Vec::new(),
                consequent_computed_bindings: Vec::new(),
                consequent_aggregates: Vec::new(),
                unresolved_clauses: Vec::new(),
                antecedent_role_literals,
                consequent_role_literals,
            };
            // Hand-rolled canonical serializer (#651) — byte-identical to
            // `serde_json::to_string(&rule)`, asserted by
            // `types::canonical_json_tests::derivation_rule_def_canonical_json_matches_serde`.
            // Lets stage2 build under no_std without dragging serde_json
            // through the kernel feature graph (one of the three #588 stage2
            // blockers; the remaining two are #652 + #653).
            let json = rule.to_canonical_json();
            derivation_rules_info.push((id, rule_text, consequent_ft_encoded, json));
            continue;
        }

        // 5. Binary fact type reading (no quotes, contains ` has `).
        if !body.contains('\'') {
            if let Some(has_idx) = body.find(" has ") {
                let subject = body[..has_idx].trim();
                let object = body[has_idx + " has ".len()..].trim();
                let id = format!("{}_has_{}",
                    subject.replace(' ', "_"),
                    object.replace(' ', "_"));
                let reading = format!("{} has {}", subject, object);
                fact_types.push(fact_from_pairs(&[
                    ("id", id.as_str()),
                    ("reading", reading.as_str()),
                    ("arity", "2"),
                ]));
                roles.push(fact_from_pairs(&[
                    ("factType", id.as_str()),
                    ("nounName", subject),
                    ("position", "0"),
                ]));
                roles.push(fact_from_pairs(&[
                    ("factType", id.as_str()),
                    ("nounName", object),
                    ("position", "1"),
                ]));
                continue;
            }
        }

        // 6. Anything else (documentary instance facts, prose between
        //    sections) — silently skip, matching legacy's no-op on
        //    unrecognised lines.
    }

    let refschemes: Vec<Object> = raw_nouns.iter()
        .filter_map(|n| n.ref_scheme.as_ref().map(|parts| (n.name.clone(), parts.clone())))
        .map(|(name, parts)| {
            let mut pairs: Vec<(String, String)> = alloc::vec![("noun".to_string(), name)];
            for (i, p) in parts.iter().enumerate() {
                pairs.push((format!("part{i}"), p.clone()));
            }
            let refs: Vec<(&str, &str)> = pairs.iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            fact_from_pairs(&refs)
        })
        .collect();

    let enum_values: Vec<Object> = enum_values_by_noun.iter()
        .map(|(noun, vals)| {
            let mut pairs: Vec<(String, String)> = alloc::vec![("noun".to_string(), noun.clone())];
            for (i, v) in vals.iter().enumerate() {
                pairs.push((format!("value{i}"), v.clone()));
            }
            let refs: Vec<(&str, &str)> = pairs.iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            fact_from_pairs(&refs)
        })
        .collect();

    // Noun cell with enrichment (`referenceScheme` / `enumValues`
    // bindings), matching `enrich_noun_cells` in the legacy path.
    let enriched_nouns: Vec<Object> = raw_nouns.iter().map(|n| {
        let mut pairs: Vec<(String, String)> = alloc::vec![
            ("name".into(), n.name.clone()),
            ("objectType".into(), n.object_type.to_string()),
            ("worldAssumption".into(), "closed".into()),
        ];
        let rs_joined: Option<String> = n.ref_scheme.as_ref()
            .map(|parts| parts.join(","))
            .or_else(|| (n.object_type == "entity").then(|| "id".into()));
        if let Some(rs) = rs_joined {
            pairs.push(("referenceScheme".into(), rs));
        }
        if let Some(evs) = enum_values_by_noun.get(&n.name) {
            if !evs.is_empty() {
                pairs.push(("enumValues".into(), evs.join(",")));
            }
        }
        let refs: Vec<(&str, &str)> = pairs.iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        fact_from_pairs(&refs)
    }).collect();

    let derivation_rules: Vec<Object> = derivation_rules_info.iter()
        .map(|(id, text, ft, json)| fact_from_pairs(&[
            ("id", id.as_str()),
            ("text", text.as_str()),
            ("consequentFactTypeId", ft.as_str()),
            ("json", json.as_str()),
        ]))
        .collect();

    let mut map: HashMap<String, Object> = HashMap::new();
    map.insert("Noun".into(), Object::Seq(enriched_nouns.into()));
    map.insert("RefScheme".into(), Object::Seq(refschemes.into()));
    map.insert("EnumValues".into(), Object::Seq(enum_values.into()));
    map.insert("FactType".into(), Object::Seq(fact_types.into()));
    map.insert("Role".into(), Object::Seq(roles.into()));
    map.insert("DerivationRule".into(), Object::Seq(derivation_rules.into()));
    Ok(Object::Map(map))
}

fn cached_grammar() -> Result<&'static GrammarCacheEntry, String> {
    let cached = GRAMMAR_CACHE.call_once(|| build_grammar_cache());
    cached.as_ref().map_err(|e| e.clone())
}

fn build_grammar_cache() -> Result<GrammarCacheEntry, String> {
    let grammar = include_str!("../../../readings/forml2-grammar.md");
    // Stage-0 bootstrap: the grammar file uses a narrow subset of FORML 2
    // shapes (entity / value / enum / binary FT / iff rule). Parsing it
    // here directly avoids the recursion that would hit `parse_to_state`
    // (the stage12 entry) needing `cached_grammar` to populate itself.
    let parsed = bootstrap_grammar_state(grammar)
        .map_err(|e| alloc::format!("grammar parse failed: {}", e))?;
    let mut defs = crate::compile::compile_to_defs_state(&parsed);
    // Swap the compiled FFP derivation Funcs for equivalent Native
    // classifiers where the rule matches the
    //   `Statement has Classification 'X' iff Statement has <Cell> ['<lit>']`
    // or the multi-antecedent variant
    //   `Statement has Classification 'X' iff Statement has <Cell1> '<lit1>' and Statement has <Cell2> '<lit2>'`
    // shape. Legacy's parse cascade is essentially this check written as
    // native Rust; routing the grammar through `ast::apply`'s general
    // Func interpreter paid a ~100× tax. Keeping the grammar as the
    // source of truth (the text came from `readings/forml2-grammar.md`)
    // plus specializing at cache time gets legacy's speed back without
    // abandoning meta-circularity.
    let antecedents = specialize_grammar_classifiers(&parsed, &mut defs);

    // Partition compiled defs: classifier rules (specialized, with
    // known antecedent cells) vs everything else (compile-emitted
    // implicit derivations — subtype transitivity, modus ponens, CWA,
    // …, plus any other derivation:* defs that weren't specialized).
    //
    // Stage-1 tokenization only writes `Statement_has_*` cells; the
    // implicit derivations read grammar-static cells (`Subtype`,
    // `FactType`, `Role`, `DerivationRule`, …) that no user input
    // ever touches. Their fixpoint over the grammar alone is
    // therefore the fixpoint over any (grammar ⊕ statements) state.
    // Pre-run them here — once, at cache init — so classify_statements
    // can skip them entirely. Moves ~2s/call of interpreter cost to
    // a single process-lifetime operation.
    let mut expanded_grammar = parsed.clone();
    let mut classifier_defs: Vec<(String, crate::ast::Func)> = Vec::new();
    let mut classifier_antecedents: Vec<Vec<String>> = Vec::new();
    let mut implicit_defs: Vec<(String, crate::ast::Func)> = Vec::new();
    for ((name, func), anc) in defs.into_iter().zip(antecedents.into_iter()) {
        if name.starts_with("derivation:") {
            match anc {
                Some(cells) => {
                    classifier_defs.push((name, func));
                    classifier_antecedents.push(cells);
                }
                None => {
                    implicit_defs.push((name, func));
                }
            }
        } else {
            // Non-derivation defs (schemas, validators, …) go into
            // the expanded state via `defs_to_state` below so
            // downstream lookups still find them.
            implicit_defs.push((name, func));
        }
    }
    // Materialize all defs into cells so `forward_chain_defs_state`
    // can reference them from within derivations if needed.
    let all_defs: Vec<(String, crate::ast::Func)> = implicit_defs.iter()
        .cloned()
        .chain(classifier_defs.iter().cloned())
        .collect();
    let grammar_with_defs = crate::ast::defs_to_state(&all_defs, &expanded_grammar);
    // Run ONLY the implicit derivation:* defs over the grammar state
    // to full fixpoint. No semi-naive here — generic fixpoint since
    // implicit rules may chain against each other.
    let implicit_deriv_refs: Vec<(&str, &crate::ast::Func)> = implicit_defs.iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, f)| (n.as_str(), f))
        .collect();
    if !implicit_deriv_refs.is_empty() {
        let (fixed, _) = crate::evaluate::forward_chain_defs_state(
            &implicit_deriv_refs, &grammar_with_defs);
        expanded_grammar = fixed;
    } else {
        expanded_grammar = grammar_with_defs;
    }

    // Pre-compute `state_keys` for the expanded grammar state — the
    // semi-naive chainer uses it as the base `existing_keys` set,
    // avoiding a ~3-4ms per-call re-hash of ~4000 grammar facts
    // inside every `classify_statements` invocation.
    let expanded_keys = crate::evaluate::state_keys(&expanded_grammar);
    Ok((
        expanded_grammar,
        classifier_defs,
        classifier_antecedents,
        expanded_keys,
    ))
}

/// Parse a classification rule's reading text into
/// `(classification, [(cell_name, literal), ...])`. Returns `None`
/// if the text doesn't match the expected shape. Single- and
/// two-clause-with-`and` antecedents are both supported.
fn parse_classification_rule_spec(text: &str)
    -> Option<(String, Vec<(String, Option<String>)>)>
{
    // Trim markdown artifacts + trailing period.
    let t = text.trim().trim_end_matches('.').trim();
    // Require "Statement has Classification '<C>'" prefix.
    let prefix = "Statement has Classification '";
    let rest = t.strip_prefix(prefix)?;
    let (classification, after_cls) = rest.split_once('\'')?;
    let iff_clause = after_cls.strip_prefix(" iff ")?;
    // Split on " and Statement has " to handle two-antecedent rules
    // (Frequency Constraint). A plain " and " split would break on
    // literal values containing `and` (e.g. the Equality Constraint
    // rule's `'if and only if'` keyword).
    let mut clauses: Vec<(String, Option<String>)> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    let mut rest = iff_clause;
    const SEP: &str = " and Statement has ";
    while let Some(i) = rest.find(SEP) {
        parts.push(rest[..i].to_string());
        // Skip the " and " portion but keep "Statement has " so the
        // downstream clause parser sees it as a full clause.
        rest = &rest[i + 5..];  // 5 = len(" and ")
    }
    parts.push(rest.to_string());
    for clause in parts.iter() {
        let clause = clause.trim();
        // Each clause: `Statement has <Cell> ['<literal>']`.
        let body = clause.strip_prefix("Statement has ")?;
        // Literal present?
        if let Some(lit_start) = body.find(" '") {
            let cell_raw = body[..lit_start].trim();
            let lit_tail = &body[lit_start + 2..];
            let (lit, _) = lit_tail.split_once('\'')?;
            let cell_name = format!("Statement_has_{}", cell_raw.replace(' ', "_"));
            clauses.push((cell_name, Some(lit.to_string())));
        } else {
            // Classification antecedent without literal (e.g., "Statement
            // has Role Reference.") — cell_raw is the whole body.
            let cell_name = format!("Statement_has_{}", body.trim().replace(' ', "_"));
            clauses.push((cell_name, None));
        }
    }
    Some((classification.to_string(), clauses))
}

/// Build a Native Func that, given the encoded population, emits one
/// `Statement_has_Classification` derived-fact Object per Statement
/// whose antecedent clauses all match. The emitted Object shape
/// matches `parse_derived_fact`: `Seq[Atom(ft_id), Atom(reading),
/// Seq[Seq[Atom(k), Atom(v)], ...]]`.
///
/// Input shape: `encode_state` produces
/// `Seq[Seq[Atom(ft_id), Seq[fact, ...]], ...]` — scan once for each
/// clause's cell by name.
fn build_native_classifier(
    classification: String,
    clauses: Vec<(String, Option<String>)>,
) -> crate::ast::Func {
    use alloc::sync::Arc;
    use crate::ast::Func;
    let reading_atom = alloc::format!(
        "Statement has Classification '{}'",
        classification,
    );
    Func::Native(Arc::new(move |input: &Object| {
        // Two possible input shapes:
        //   (a) raw state (`Object::Map`) — fast path, used by the
        //       semi-naive chainer when all active funcs are Native.
        //       Each clause resolves via O(1) `fetch_or_phi`.
        //   (b) `encode_state` output (`Object::Seq` of
        //       `[cell_name, facts_seq]` pairs) — compatibility path
        //       for `ast::apply` call sites that pre-encode.
        let matching_statement = |fact: &Object, want_lit: Option<&str>| -> Option<String> {
            let pairs = fact.as_seq()?;
            let mut stmt: Option<&str> = None;
            let mut saw_lit = want_lit.is_none();
            for p in pairs.iter() {
                let kv = match p.as_seq() { Some(s) if s.len() == 2 => s, _ => continue };
                let k = match kv[0].as_atom() { Some(a) => a, None => continue };
                let v = match kv[1].as_atom() { Some(a) => a, None => continue };
                if k == "Statement" { stmt = Some(v); }
                if let Some(want) = want_lit {
                    if v == want { saw_lit = true; }
                }
            }
            if !saw_lit { return None; }
            stmt.map(String::from)
        };
        let collect_stmts = |facts: &[Object], want_lit: Option<&str>|
            -> hashbrown::HashSet<String>
        {
            facts.iter()
                .filter_map(|f| matching_statement(f, want_lit))
                .collect()
        };

        // Resolve each clause to a set of matching Statement ids. The
        // raw-state branch uses `fetch_or_phi` (O(1) on Object::Map);
        // the encoded-pop branch linear-scans pop entries.
        let use_state_path = matches!(input, Object::Map(_));
        let mut stmts: Option<hashbrown::HashSet<String>> = None;
        for (cell_name, lit) in &clauses {
            let local: hashbrown::HashSet<String> = if use_state_path {
                let cell = crate::ast::fetch_or_phi(cell_name, input);
                let Some(facts) = cell.as_seq() else {
                    return Object::phi();
                };
                collect_stmts(facts, lit.as_deref())
            } else {
                let Some(pop_entries) = input.as_seq() else {
                    return Object::phi();
                };
                let mut found: Option<&[Object]> = None;
                for entry in pop_entries.iter() {
                    let Some(pair) = entry.as_seq() else { continue };
                    if pair.len() != 2 { continue; }
                    if pair[0].as_atom() == Some(cell_name.as_str()) {
                        found = pair[1].as_seq();
                        break;
                    }
                }
                let Some(facts) = found else { return Object::phi(); };
                collect_stmts(facts, lit.as_deref())
            };
            stmts = Some(match stmts.take() {
                None => local,
                Some(prev) => prev.intersection(&local).cloned().collect(),
            });
            if stmts.as_ref().map(|s| s.is_empty()).unwrap_or(true) {
                return Object::phi();
            }
        }
        let stmts = stmts.unwrap_or_default();

        // Emit one derived-fact-encoded Object per matching Statement.
        let emitted: Vec<Object> = stmts.into_iter().map(|stmt_id| {
            let bindings = Object::seq(vec![
                Object::seq(vec![Object::atom("Statement"), Object::atom(&stmt_id)]),
                Object::seq(vec![
                    Object::atom("Classification"),
                    Object::atom(&classification),
                ]),
            ]);
            Object::seq(vec![
                Object::atom("Statement_has_Classification"),
                Object::atom(&reading_atom),
                bindings,
            ])
        }).collect();
        Object::seq(emitted)
    }))
}

/// Walk the grammar's `DerivationRule` cell, build a map from rule id
/// to a specialization spec for recognized classification rules, then
/// replace matching entries in `defs` with Native equivalents.
/// Returns a parallel `Vec<Option<Vec<String>>>` of per-def
/// antecedent cells — `Some(cells)` for specialized rules, `None` for
/// unspecialized ones (meaning the semi-naive chainer should run them
/// every round conservatively).
fn specialize_grammar_classifiers(
    grammar_state: &Object,
    defs: &mut alloc::vec::Vec<(String, crate::ast::Func)>,
) -> Vec<Option<Vec<String>>> {
    let mut antecedents: Vec<Option<Vec<String>>> = vec![None; defs.len()];
    let rule_cell = crate::ast::fetch_or_phi("DerivationRule", grammar_state);
    let Some(rules) = rule_cell.as_seq() else { return antecedents };
    let mut id_to_spec: hashbrown::HashMap<String, (String, Vec<(String, Option<String>)>)>
        = hashbrown::HashMap::new();
    for fact in rules.iter() {
        let id = match crate::ast::binding(fact, "id") {
            Some(s) => s.to_string(), None => continue
        };
        let text = match crate::ast::binding(fact, "text") {
            Some(s) => s, None => continue
        };
        if let Some(spec) = parse_classification_rule_spec(text) {
            id_to_spec.insert(id, spec);
        }
    }
    for (i, (name, func)) in defs.iter_mut().enumerate() {
        let Some(id) = name.strip_prefix("derivation:") else { continue };
        let Some(spec) = id_to_spec.get(id) else { continue };
        *func = build_native_classifier(spec.0.clone(), spec.1.clone());
        antecedents[i] = Some(spec.1.iter().map(|(c, _)| c.clone()).collect());
    }
    antecedents
}

fn cached_grammar_state() -> Result<&'static Object, String> {
    cached_grammar().map(|(s, _, _, _)| s)
}

/// Public entry point: parse FORML 2 source with no external context.
pub fn parse_to_state_via_stage12(text: &str) -> Result<Object, String> {
    parse_to_state_via_stage12_impl(text, &[], &[])
}

/// Context-aware parse (#285). Used by `parse_to_state_from` and
/// `parse_to_state_with_nouns` so a statement mentioning a noun
/// declared in a previously-parsed domain tokenises correctly.
/// Both noun *names* AND fact-type *ids* are propagated: nouns let
/// the per-line tokenizer match (e.g. `State Machine Definition`
/// stays a single token); FT ids let `translate_instance_facts`
/// resolve a body like `State Machine Definition 'Order' is for
/// Noun 'Order'.` to the canonical underscored cell name
/// (`State_Machine_Definition_is_for_Noun`) instead of the verb-only
/// fallback. Without the FT propagation the per-field cell fanout
/// in `instance_fact_field_cells` writes to a non-canonical cell
/// (e.g. `is_for`) and downstream cell-driven code (#761/#762's SM
/// readers, etc.) sees an empty FT cell. `merge_states(ctx, result)`
/// on the caller's side carries the rest of `ctx`'s cells forward.
pub fn parse_to_state_via_stage12_with_context(
    text: &str,
    ctx: &Object,
) -> Result<Object, String> {
    let extra_nouns: Vec<String> = fetch_or_phi("Noun", ctx).as_seq()
        .map(|facts| facts.iter()
            .filter_map(|f| binding(f, "name").map(String::from))
            .collect())
        .unwrap_or_default();
    let extra_ft_ids: Vec<String> = fetch_or_phi("FactType", ctx).as_seq()
        .map(|facts| facts.iter()
            .filter_map(|f| binding(f, "id").map(String::from))
            .collect())
        .unwrap_or_default();
    parse_to_state_via_stage12_impl(text, &extra_nouns, &extra_ft_ids)
}

fn parse_to_state_via_stage12_impl(
    text: &str,
    extra_nouns: &[String],
    extra_ft_ids: &[String],
) -> Result<Object, String> {
    // Trace gate — std-host reads `AREST_STAGE12_TRACE`; no_std builds
    // compile out the trace branches entirely.
    #[cfg(not(feature = "no_std"))]
    let trace = std::env::var("AREST_STAGE12_TRACE").is_ok();
    #[cfg(feature = "no_std")]
    let trace = false;
    let t0 = Instant::now();
    let grammar_state = cached_grammar_state()?;
    if trace { crate::diag!("[s12] grammar cache: {:?}", t0.elapsed()); }

    // MC1 (#712): build the Stage-1 vocab from the cached grammar
    // state's EnumValues cell. Stage-1 falls back to `Vocab::boot()`
    // when called outside the cell-driven path (legacy callers, unit
    // tests). Pass-by-borrow so we don't clone the vocab per line.
    let vocab = crate::parse_forml2_stage1::Vocab::from_grammar_state(grammar_state);
    // MC2 (#713): build Stage-2's three dispatch tables (ring kinds,
    // conditional ring matrix, deontic shape) from the same cached
    // grammar state. The translators each fall back to their `boot()`
    // table when called outside this cell-driven path. Threaded
    // by-borrow so the per-parse build cost is the only allocation.
    let ring_kinds = RingKindTable::from_grammar_state(grammar_state);
    let conditional_matrix = ConditionalRingMatrix::from_grammar_state(grammar_state);
    let deontic_shapes = DeonticShapeTable::from_grammar_state(grammar_state);

    // #309 — enforce Theorem 1's no-reserved-substring rule. Scan
    // unquoted noun declarations in the source and reject any that
    // collide with a grammar keyword. Quoted names (`Noun 'Each Way
    // Bet' is an entity type.`) bypass the check and land in the
    // noun cell as single tokens.
    let t_pre = Instant::now();
    reject_reserved_noun_declarations(text, &vocab)?;

    // Direct text-scan bootstrap for noun names — avoids running the
    // full legacy cascade a second time just to recover the Noun cell.
    // `extra_nouns` threads in the noun catalog of a caller-supplied
    // context (e.g. metamodel state on a user-domain parse) so
    // statements can reference those nouns without redeclaring them.
    let mut nouns: Vec<String> = extract_declared_noun_names(text);
    for n in extra_nouns {
        if !nouns.iter().any(|existing| existing == n) {
            nouns.push(n.clone());
        }
    }
    nouns.sort_by(|a, b| b.len().cmp(&a.len()));
    // Build the first-byte noun index ONCE per parse so the per-line
    // tokenizer doesn't re-partition on every call.
    let sorted_nouns: Vec<&str> = nouns.iter().map(|s| s.as_str()).collect();
    let noun_buckets = crate::parse_forml2_stage1::NounBuckets::from_sorted(&sorted_nouns);

    let lines = crate::parse_forml2::join_derivation_continuations_cow(text);
    if trace { crate::diag!("[s12] preproc (reject+nouns+join): {:?}", t_pre.elapsed()); }
    // Accumulate per-statement cells into a single HashMap, then lift
    // to Object::Map once at the end. Previously we did
    // `stmt_state = merge_states(&stmt_state, ...)` per line, which is
    // O(n²) on the growing cell vectors.
    let t_tok = Instant::now();
    let mut acc_cells: HashMap<String, Vec<Object>> = HashMap::new();
    for (i, raw_line) in lines.iter().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Skip prose lines: every FORML 2 statement ends with `.` —
        // optionally followed by an ORM 2 derivation marker
        // (`. *`, `. **`, `. +`). Markdown prose interspersed in
        // reading files (section introductions, bullet continuations
        // with no period) would otherwise be tokenized and
        // misclassified as Fact Type Reading via their incidental
        // noun references. Legacy's cascade only acts when a
        // recognizer matches; its recognizers all require the period
        // terminator.
        let ends_like_statement = line.ends_with('.')
            || line.ends_with(". *")
            || line.ends_with(". **")
            || line.ends_with(". +");
        // Ring-constraint kind annotation: `<body>. (<kind>)` where
        // `<kind>` is one of the legacy-accepted ring adjectives.
        // Strip the annotation so Stage-1 sees the canonical body
        // ending in `.`; `translate_ring_constraints`' conditional /
        // trailing-marker detectors handle kind inference from the
        // body itself.
        let line: &str = if !ends_like_statement && line.ends_with(')') {
            strip_ring_annotation(line).unwrap_or(line)
        } else {
            line
        };
        let ends_like_statement = line.ends_with('.')
            || line.ends_with(". *")
            || line.ends_with(". **")
            || line.ends_with(". +");
        if !ends_like_statement {
            continue;
        }
        // ORM 2 possibility-override statements (`It is possible that
        // ...`) don't land as `Constraint` or `DerivationRule` cells
        // — legacy's Pass 2b has no recognizer. But legacy DOES
        // register a synthetic FactType from the embedded predicate
        // (e.g. `more than one Noun has the same Alias` →
        // `Noun_has_the_same_Alias` FT). Stage-2 emits those
        // synthetic FTs after the main tokenization loop via
        // `possibility_synthetic_fact_type`. Skip Stage-1 tokenization
        // here so no Statement cell fires on the outer prefix.
        if line.starts_with("It is possible that ") {
            continue;
        }
        // Skip mutually-exclusive-subtypes braces declarations — ORM
        // 2's `{A, B} are mutually exclusive subtypes of C`. Legacy
        // recognises these via `try_exclusive_subtypes` and emits
        // `ParseAction::Skip` (no cell). The semantics live in the
        // individual `A is a subtype of C` / `B is a subtype of C`
        // lines above, plus the implicit partition.
        if line.starts_with('{') && line.contains("subtypes of") {
            continue;
        }
        // Skip named-span-association declarations — `This
        // association with A, B provides the preferred
        // identification scheme for C`. Legacy's `try_association`
        // emits Skip; the semantics are carried by the NamedSpan
        // cell which `try_span_naming` populates (not this shape).
        if line.starts_with("This association with") {
            continue;
        }
        let statement_id = alloc::format!("s{}", i);
        let cells = crate::parse_forml2_stage1::tokenize_statement_with_buckets_vocab(
            &statement_id, line, &noun_buckets, &vocab);
        for (cell_name, facts) in cells.into_iter() {
            acc_cells.entry(cell_name).or_default().extend(facts);
        }
    }
    let stmt_state: Object = {
        let map: HashMap<String, Object> = acc_cells.into_iter()
            .map(|(k, v)| (k, Object::Seq(v.into())))
            .collect();
        Object::Map(map)
    };
    if trace { crate::diag!("[s12] stage1 tokenize: {:?} ({} lines)",
        t_tok.elapsed(), lines.len()); }

    // #301 — possibility-override synthetic FactType registrations.
    // Scan the raw source for `It is possible that ...` lines and
    // emit synthetic FT + Role facts for the embedded predicate
    // (matches legacy's implicit registration path). Done before
    // classify so the synthetic FTs live in the pre-classified state
    // cells if downstream passes want them; currently they're merged
    // straight into the output after translator runs.
    let synthetic_fts_and_roles: Vec<(Object, Vec<Object>)> = text.lines()
        .filter_map(|raw| {
            let line = raw.trim();
            line.strip_prefix("It is possible that ")
                .and_then(|body| {
                    let body = body.trim_end_matches('.').trim();
                    possibility_synthetic_fact_type(body, &nouns)
                })
        })
        .collect();

    let t_cls = Instant::now();
    let classified = classify_statements(&stmt_state, grammar_state);
    if trace { crate::diag!("[s12] classify: {:?}", t_cls.elapsed()); }

    let t_tr = Instant::now();
    // Build the statement index once, thread it explicitly through
    // every translator. C2 (#688): the prior thread-local SLOT made
    // translator results depend on which ancestor frame populated it,
    // a CRITICAL purity violation in the meta-circular parser. Helpers
    // (`classifications_for` / `head_noun_for` / `statement_text` /
    // `trailing_marker_for` / `derivation_marker_for`) take `&StmtIndex`
    // directly; no implicit ambient state.
    let idx = build_stmt_index(&classified);
    macro_rules! tt { ($name:expr, $e:expr) => {{
        let t = Instant::now();
        let v = $e;
        if trace { crate::diag!("    [tr] {}: {:?}", $name, t.elapsed()); }
        v
    }}; }

    // Run translate_nouns FIRST so subsequent translators that consult
    // `declared_noun_names` see domain nouns (not just the grammar's
    // metamodel nouns). Inject the resulting Noun facts into the
    // classified state before invoking constraint translators that
    // depend on the declared-noun list — `translate_set_constraints`'
    // antecedent-noun-count arbitration and
    // `translate_ring_constraints`' `conditional_ring_kind` helper
    // both need the domain-level catalog.
    let noun_facts = tt!("nouns", translate_nouns(&classified, &idx));
    let classified = {
        let mut map: HashMap<String, Object> = match &classified {
            Object::Map(m) => m.clone(),
            _ => HashMap::new(),
        };
        map.insert("Noun".to_string(), Object::Seq(noun_facts.clone().into()));
        Object::Map(map)
    };

    let mut subtype_facts: Vec<Object> = tt!("subtypes", translate_subtypes(&classified, &idx));
    subtype_facts.extend(tt!("partitions", translate_partitions(&classified, &idx)));
    let (mut ft_facts, mut role_facts) = tt!("fact_types", translate_fact_types(&classified, &idx));
    // Append possibility-synthetic FactType + Role facts.
    for (ft_fact, role_fs) in &synthetic_fts_and_roles {
        // De-dup: skip if translate_fact_types already emitted this id.
        let Some(ft_id) = binding(ft_fact, "id") else { continue };
        if ft_facts.iter().any(|f| binding(f, "id") == Some(ft_id)) {
            continue;
        }
        ft_facts.push(ft_fact.clone());
        role_facts.extend(role_fs.clone());
    }
    let mut constraint_facts: Vec<Object> = tt!("ring",
        translate_ring_constraints_with_tables(
            &classified, &idx, &ring_kinds, &conditional_matrix));
    constraint_facts.extend(tt!("cardinality", translate_cardinality_constraints(&classified, &idx)));
    constraint_facts.extend(tt!("set", translate_set_constraints(&classified, &idx)));
    constraint_facts.extend(tt!("value_c", translate_value_constraints(&classified, &idx)));
    constraint_facts.extend(tt!("deontic",
        translate_deontic_constraints_with_table(&classified, &idx, &deontic_shapes)));
    // Enrich each constraint with span0_factTypeId / span0_roleIndex
    // (and span1_*) bindings derived from the Role cell. Legacy emits
    // these at constraint-translation time; check.rs, command.rs and
    // the RMAP-attached-constraints code path all read them. Single-
    // role UC/MC/VC get span0 and span1 both pointing at the same
    // role (legacy quirk preserved for byte-level parity).
    constraint_facts = tt!("enrich_spans",
        enrich_constraints_with_spans(&constraint_facts, &role_facts));
    let derivation_facts = tt!("derivation",
        translate_derivation_rules_with_matrix(&classified, &idx, &conditional_matrix, &ft_facts));
    let unresolved_clause_facts = tt!("unresolved",
        translate_unresolved_clauses(&classified, &idx, &ft_facts));
    let mut declared_ft_ids: Vec<String> = ft_facts.iter()
        .filter_map(|f| binding(f, "id").map(String::from))
        .collect();
    // Merge in FT ids from caller-supplied context (#763 prereq):
    // when this parse runs against a metamodel-bearing context, the
    // FTs declared in that context aren't in `ft_facts` (which only
    // covers FTs declared in *this* `text`), so instance facts that
    // reference context-declared FTs would fall back to verb-only
    // fieldName and `instance_fact_field_cells` would write them to
    // a non-canonical cell. Threading context FT ids here keeps the
    // canonical resolution working across context boundaries — the
    // downstream cell-driven SM compiler (#761 et al.) reads
    // `State_Machine_Definition_is_for_Noun` and finds populated facts.
    for id in extra_ft_ids {
        if !declared_ft_ids.iter().any(|existing| existing == id) {
            declared_ft_ids.push(id.clone());
        }
    }
    let mut instance_fact_facts = tt!("instance_facts",
        translate_instance_facts_with_ft_ids(&classified, &idx, &declared_ft_ids));
    instance_fact_facts.extend(tt!("deriv_mode", translate_derivation_mode_facts(&classified, &idx)));
    let enum_values_facts = tt!("enum_values", translate_enum_values(&classified, &idx));
    if trace { crate::diag!("[s12] translators: {:?}", t_tr.elapsed()); }
    if trace { crate::diag!("[s12] TOTAL: {:?}", t0.elapsed()); }

    // Compound reference-scheme decomposition: mirrors the legacy
    // parse_forml2.rs path. For each noun declared with `(.A, .B, ...)`
    // (ref-scheme arity ≥ 2), split every instance subject value on '-'
    // from the right and push `{Noun}_has_{Component}` cells carrying
    // the noun id + component value. command.rs / rmap read these.
    let compound_cells = compound_ref_component_cells(&noun_facts, &instance_fact_facts);
    // Per-field cells for instance facts: `emit_instance_fact` in the
    // legacy cascade writes every instance fact twice — once to the
    // canonical `InstanceFact` cell (stage12 already does this) AND
    // once to a `{fieldName}` cell (e.g. `A_has_B`) keyed by the
    // subject/object nouns. `extract_facts_from_pop` in compile.rs
    // reads these per-field cells at runtime, so derivations over
    // instance-fact populations (forward_chain over joins, CWA
    // negations, etc.) need them present.
    let per_field_cells = instance_fact_field_cells(&instance_fact_facts);

    let mut map: HashMap<String, Object> = HashMap::new();
    map.insert("Noun".to_string(), Object::Seq(noun_facts.into()));
    map.insert("Subtype".to_string(), Object::Seq(subtype_facts.into()));
    map.insert("FactType".to_string(), Object::Seq(ft_facts.into()));
    map.insert("Role".to_string(), Object::Seq(role_facts.into()));
    map.insert("Constraint".to_string(), Object::Seq(constraint_facts.into()));
    map.insert("DerivationRule".to_string(), Object::Seq(derivation_facts.into()));
    map.insert("InstanceFact".to_string(), Object::Seq(instance_fact_facts.into()));
    map.insert("EnumValues".to_string(), Object::Seq(enum_values_facts.into()));
    map.insert("UnresolvedClause".to_string(), Object::Seq(unresolved_clause_facts.into()));
    for (cell_name, facts) in compound_cells {
        map.insert(cell_name, Object::Seq(facts.into()));
    }
    for (cell_name, facts) in per_field_cells {
        map.entry(cell_name)
            .and_modify(|existing| {
                let mut all: Vec<Object> = existing.as_seq()
                    .map(|s| s.to_vec()).unwrap_or_default();
                all.extend(facts.iter().cloned());
                *existing = Object::Seq(all.into());
            })
            .or_insert_with(|| Object::Seq(facts.into()));
    }
    Ok(Object::Map(map))
}

/// Decompose compound reference-scheme instance ids into component
/// cells. Legacy parity: `parse_forml2::parse_into` does this at the
/// end of the cascade (crates/arest/src/parse_forml2.rs §"Compound
/// reference-scheme decomposition"). For a noun `Thing(.Owner, .Seq)`
/// and an instance `Thing 'alice-1' has …`, emit:
///
///   Thing_has_Owner { Thing: alice-1, Owner: alice }
///   Thing_has_Seq   { Thing: alice-1, Seq:   1 }
///
/// Ids are rsplit on `-` so multi-hyphen first components
/// (`alpha-team-7` into (`alpha-team`, `7`)) survive.
fn compound_ref_component_cells(
    noun_facts: &[Object],
    instance_facts: &[Object],
) -> Vec<(String, Vec<Object>)> {
    // (noun_name, ref_parts) for nouns with arity ≥ 2.
    let compound: Vec<(String, Vec<String>)> = noun_facts.iter()
        .filter_map(|f| {
            let name = binding(f, "name")?.to_string();
            let rs = binding(f, "referenceScheme")?;
            let parts: Vec<String> = rs.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            (parts.len() >= 2).then_some((name, parts))
        })
        .collect();
    if compound.is_empty() { return Vec::new(); }

    let mut out: hashbrown::HashMap<String, Vec<Object>> = hashbrown::HashMap::new();
    for (noun_name, ref_parts) in &compound {
        // Distinct subject ids for this noun.
        let mut seen: hashbrown::HashSet<String> = hashbrown::HashSet::new();
        let ids: Vec<String> = instance_facts.iter()
            .filter_map(|f| {
                (binding(f, "subjectNoun")? == noun_name.as_str())
                    .then(|| binding(f, "subjectValue").map(String::from))
                    .flatten()
            })
            .filter(|id| seen.insert(id.clone()))
            .collect();
        for id in &ids {
            let parts_rev: Vec<&str> = id.rsplitn(ref_parts.len(), '-').collect();
            if parts_rev.len() != ref_parts.len() { continue; }
            let parts: Vec<&str> = parts_rev.into_iter().rev().collect();
            for (component, value) in ref_parts.iter().zip(parts.iter()) {
                let cell_name = alloc::format!("{}_has_{}",
                    noun_name.replace(' ', "_"),
                    component.replace(' ', "_"));
                let fact = fact_from_pairs(&[
                    (noun_name.as_str(), id.as_str()),
                    (component.as_str(), *value),
                ]);
                out.entry(cell_name).or_default().push(fact);
            }
        }
    }
    out.into_iter().collect()
}

/// Fan out `InstanceFact` facts into per-field cells keyed by
/// `fieldName`. Legacy parity: `parse_forml2::emit_instance_fact`
/// writes both the canonical `InstanceFact` fact and a `{fieldName}`
/// cell carrying `(subjectNoun, subjectValue) + (objectKey, objectValue)`.
///
/// The object key is `objectNoun` when non-empty, else falls back to
/// `fieldName` — matches the attribute-style path in `emit_instance_fact`.
///
/// For ternary+ instance facts (#553) the per-field cell fact also
/// carries one `(roleNNoun, roleNValue)` pair per additional role
/// (`role2Noun` / `role2Value`, `role3Noun` / `role3Value`, …) so
/// downstream readers can `binding(fact, "DLL Behavior")` directly
/// without re-parsing the raw markdown.
///
/// Returns `(cell_name, facts)` pairs the caller merges into the
/// final cell map.
fn instance_fact_field_cells(instance_facts: &[Object]) -> Vec<(String, Vec<Object>)> {
    let mut out: hashbrown::HashMap<String, Vec<Object>> = hashbrown::HashMap::new();
    for f in instance_facts {
        let Some(field_name) = binding(f, "fieldName") else { continue };
        if field_name.is_empty() { continue; }
        let subject_noun = binding(f, "subjectNoun").unwrap_or("");
        let subject_value = binding(f, "subjectValue").unwrap_or("");
        let object_noun = binding(f, "objectNoun").unwrap_or("");
        let object_value = binding(f, "objectValue").unwrap_or("");
        if subject_noun.is_empty() { continue; }
        let object_key = if object_noun.is_empty() { field_name } else { object_noun };
        // Base: the legacy 2-pair shape (subject + object). Extra
        // roles are appended in declared order; their key is the
        // role's head noun (mirrors how the binary path keys the
        // object by `objectNoun`).
        let mut pairs: Vec<(String, String)> = Vec::with_capacity(2);
        pairs.push((subject_noun.to_string(),  subject_value.to_string()));
        pairs.push((object_key.to_string(),    object_value.to_string()));
        // Walk roleNNoun / roleNValue starting at N=2 until the
        // sequence breaks (a missing roleNNoun ends the chain). One
        // pair per additional role; key is the role noun, value is
        // the role's literal.
        let mut n: usize = 2;
        loop {
            let noun_key = alloc::format!("role{}Noun", n);
            let value_key = alloc::format!("role{}Value", n);
            let Some(noun) = binding(f, &noun_key) else { break };
            if noun.is_empty() { break; }
            let value = binding(f, &value_key).unwrap_or("");
            pairs.push((noun.to_string(), value.to_string()));
            n += 1;
        }
        let pair_refs: Vec<(&str, &str)> = pairs.iter()
            .map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let fact = fact_from_pairs(&pair_refs);
        out.entry(field_name.to_string()).or_default().push(fact);
    }
    out.into_iter().collect()
}

/// #309 — scan the source text for noun declarations whose unquoted
/// names contain a grammar reserved keyword as a whole word.
///
/// Recognises these declaration shapes at a line level:
///
///   - `<Name> is an entity type.`
///   - `<Name>(.<refScheme>) is an entity type.`
///   - `<Name> is a value type.`
///   - `<Name> is a subtype of <Supertype>.`
///   - `<Name> is abstract.`
///   - `<Name> is partitioned into <...>.`
///
/// Names beginning with a single quote are treated as quoted
/// identifiers and bypass the check (Theorem 1 escape documented at
/// `docs/02-writing-readings.md`).
fn reject_reserved_noun_declarations(
    text: &str,
    vocab: &crate::parse_forml2_stage1::Vocab,
) -> Result<(), String> {
    for raw_line in text.lines() {
        let line = raw_line.trim();
        let before = line
            .strip_suffix(" is an entity type.")
            .or_else(|| line.strip_suffix(" is a value type."))
            .or_else(|| line.strip_suffix(" is abstract."))
            .or_else(|| line.split(" is a subtype of ").next()
                .filter(|pre| *pre != line))
            .or_else(|| line.split(" is partitioned into ").next()
                .filter(|pre| *pre != line));
        let Some(before) = before else { continue };
        let name = match before.find('(') {
            Some(p) => before[..p].trim(),
            None => before.trim(),
        };
        if name.is_empty() { continue; }
        // Quoted names bypass the check.
        if name.starts_with('\'') { continue; }
        if let Some(kw) = crate::parse_forml2_stage1::reserved_keyword_in_with_vocab(name, vocab) {
            return Err(alloc::format!(
                "noun declaration `{}` collides with reserved keyword `{}`; \
                 quote the name to escape: `Noun '{}' is an entity type.`",
                name, kw, name));
        }
    }
    Ok(())
}

/// Direct text scan for declared noun names — avoids running the
/// full legacy cascade just to recover the Noun cell.
///
/// Recognises the same declaration shapes as
/// `reject_reserved_noun_declarations` (entity / value / subtype /
/// abstract / partition), plus `{A, B, ...} are mutually exclusive
/// subtypes of C` which contributes A, B, and C to the list.
/// Quoted names have their surrounding quotes stripped. Partition
/// subtype lists are expanded so each member becomes a noun name.
/// Handles `(.refScheme)` suffixes by trimming at the open paren.
///
/// #750 — also auto-declares compound nouns referenced inline with
/// `(.<refScheme>)` in role positions (e.g. `Personal Data Breach is
/// breach of security leading to loss of Personal Data(.id).`). The
/// trailing `(.id)` marks the preceding capitalized-word run as a
/// noun head. Without this scan, the longest-match Stage-1 tokenizer
/// has no entry for `Personal Data` in its noun set and falls
/// through to the bare-word `Data`, producing a false ring shape on
/// the FT's two roles. Auto-declaring the compound from inline use
/// preserves the declared role nouns end-to-end.
fn extract_declared_noun_names(text: &str) -> Vec<String> {
    let mut names: alloc::collections::BTreeSet<String> =
        alloc::collections::BTreeSet::new();

    let push = |names: &mut alloc::collections::BTreeSet<String>, raw: &str| {
        let trimmed = raw.trim();
        let unquoted = trimmed
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
            .unwrap_or(trimmed);
        let name = match unquoted.find('(') {
            Some(p) => unquoted[..p].trim(),
            None => unquoted.trim(),
        };
        if !name.is_empty() {
            names.insert(name.to_string());
        }
    };

    for raw_line in text.lines() {
        let line = raw_line.trim();
        // Partition declaration — both the super and each subtype get
        // added. `Animal is partitioned into Cat, Dog, Bird.`
        if let Some(idx) = line.find(" is partitioned into ") {
            push(&mut names, &line[..idx]);
            let tail = line[idx + " is partitioned into ".len()..]
                .trim_end_matches('.')
                .trim();
            for sub in tail.split(',') {
                push(&mut names, sub);
            }
            continue;
        }
        // Mutually-exclusive-subtypes braces. Both braced entries and
        // the post-`subtypes of` supertype count.
        if line.starts_with('{') {
            if let Some(end) = line.find('}') {
                let inner = &line[1..end];
                for sub in inner.split(',') {
                    push(&mut names, sub);
                }
                if let Some(st_idx) = line.find(" subtypes of ") {
                    let tail = line[st_idx + " subtypes of ".len()..]
                        .trim_end_matches('.')
                        .trim();
                    push(&mut names, tail);
                }
                continue;
            }
        }
        // Subtype. `Dog is a subtype of Animal.`
        if let Some(idx) = line.find(" is a subtype of ") {
            push(&mut names, &line[..idx]);
            let tail = line[idx + " is a subtype of ".len()..]
                .trim_end_matches('.')
                .trim();
            push(&mut names, tail);
            continue;
        }
        // Entity / value type / abstract.
        let before = line
            .strip_suffix(" is an entity type.")
            .or_else(|| line.strip_suffix(" is a value type."))
            .or_else(|| line.strip_suffix(" is abstract."));
        if let Some(before) = before {
            push(&mut names, before);
        }
        // #750 — auto-declare compound nouns from inline `(.…)` use.
        // Apply to every line so a fact-type reading like
        // `Foo(.id) is breach of Personal Data(.id).` contributes
        // both `Foo` and `Personal Data`. The entity-declaration form
        // above already captured the head, but inline references in
        // role positions need this scan.
        for noun in extract_inline_paren_nouns(line) {
            names.insert(noun);
        }
    }
    names.into_iter().collect()
}

/// #750 — given a single line, find each `(.<chars>)` occurrence and
/// recover the noun head that immediately precedes it. The head is
/// the longest run of consecutive capitalized words (each starting
/// with an ASCII uppercase letter) walking backward from the open
/// paren, separated by single spaces. Stops on:
///   - sentence start (offset 0),
///   - a non-capitalized word (lowercase verb like `is`, `has`),
///   - a punctuation boundary (`.`, `,`, `:`, etc.),
///   - any non-letter, non-digit, non-underscore, non-space byte.
///
/// Returns the heads as owned `String`s.
fn extract_inline_paren_nouns(line: &str) -> Vec<String> {
    let bytes = line.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut search_from = 0;
    while let Some(rel) = line[search_from..].find("(.") {
        let paren = search_from + rel;
        // Advance past this `(.` for the next iteration regardless of
        // outcome. Use paren + 2 so we never re-enter the same site.
        search_from = paren + 2;

        // Walk back past the `(`. If the immediate preceding char is
        // not an alphanumeric / underscore (i.e. the `(.` isn't right
        // up against a word), skip — that's a citation, footnote, or
        // unrelated punctuation.
        if paren == 0 { continue; }
        let end = paren;
        // Must immediately follow a word character.
        if !(bytes[end - 1].is_ascii_alphanumeric() || bytes[end - 1] == b'_') {
            continue;
        }
        // Walk backward over capitalized space-separated words.
        let head_start = walk_back_capitalized_words(bytes, end);
        if head_start >= end { continue; }
        let head = &line[head_start..end];
        // Filter: must contain at least one ASCII uppercase letter
        // (otherwise we've picked up a lowercase fragment like
        // `subject`). And must not include a `(` itself.
        if !head.bytes().any(|b| b.is_ascii_uppercase()) { continue; }
        if head.contains('(') { continue; }
        // Multi-word heads are the interesting case here; single
        // words also enter the list (cheap, and may catch
        // implicit declarations). The cost is bounded — Stage-1's
        // longest-match prefers the longest matching noun, so
        // an extra single-word entry is harmless.
        out.push(head.to_string());
    }
    out
}

/// Walk backward from `end` over capitalized word characters,
/// returning the start byte offset of the first character of the
/// run. Words are separated by exactly one space; each word starts
/// with an ASCII uppercase letter and contains only ASCII alpha
/// (uppercase or lowercase) — digits and underscores are excluded
/// from noun heads to keep the scan conservative.
fn walk_back_capitalized_words(bytes: &[u8], end: usize) -> usize {
    if end == 0 { return 0; }
    // `accepted` tracks the start of the last word we committed to.
    // We only return positions we've accepted; an aborted scan of a
    // tentative previous word does not move `accepted` backward.
    let mut accepted = end;
    let mut cursor = end;
    loop {
        // Find start of current word: walk back while ascii alpha.
        let mut word_start = cursor;
        while word_start > 0 && bytes[word_start - 1].is_ascii_alphabetic() {
            word_start -= 1;
        }
        // Empty run (cursor sits on non-alpha) → done.
        if word_start == cursor { break; }
        // Reject lowercase-leading word — that's a verb / preposition,
        // not part of the noun head.
        if !bytes[word_start].is_ascii_uppercase() { break; }
        // Accept this word.
        accepted = word_start;
        // Need exactly one ASCII space separating words to keep
        // walking. Sentence start, period, or other punctuation
        // ends the noun head.
        if word_start == 0 || bytes[word_start - 1] != b' ' { break; }
        // The char before the space must be ASCII alpha so the
        // previous token is a real word. Otherwise stop.
        if word_start < 2 || !bytes[word_start - 2].is_ascii_alphabetic() { break; }
        // Step the cursor onto the space; next iter will walk past
        // it into the previous word.
        cursor = word_start - 1;
    }
    accepted
}

#[cfg(all(test, feature = "std-deps"))]
mod tests {
    use super::*;
    use crate::parse_forml2::parse_to_state;
    use crate::parse_forml2_stage1::tokenize_statement;

    fn grammar_state() -> Object {
        let grammar = include_str!("../../../readings/forml2-grammar.md");
        parse_to_state(grammar).expect("grammar must parse")
    }

    /// Test helper: build a fresh StmtIndex from a classified state.
    /// C2 (#688): translators now take `&StmtIndex` explicitly. Tests
    /// that call helpers / translators directly build their own index.
    fn idx(state: &Object) -> StmtIndex { build_stmt_index(state) }

    /// Stage-0 bootstrap must produce every cell `compile_to_defs_state`
    /// and `specialize_grammar_classifiers` read from. Specific counts
    /// are pinned to the committed `readings/forml2-grammar.md` (23
    /// nouns, 16 binary FTs, 13 enum-valued value types, 30+ classifier
    /// rules) to guard against a shape recognizer silently dropping a
    /// line when the grammar file is edited.
    ///
    /// MC1 (#712) bumped the noun count to 17 by adding the
    /// `Derivation Marker Symbol` value type, and the enum-valued noun
    /// count to 7 by adding `Trailing Marker` and `Derivation Marker
    /// Symbol` enum-value declarations so Stage-1's vocab can be lifted
    /// from cells. MC2 (#713) added 6 more parallel-enum value types
    /// (`Ring Constraint Trailing Marker` / `Ring Constraint Kind
    /// Code`, `Conditional Ring Pattern` / `Conditional Ring Kind
    /// Code`, `Deontic Constraint Kind Code` / `Deontic Constraint
    /// Modality`) so Stage-2's three dispatch matrices can be lifted
    /// from cells too — bumping noun count to 23, enum count to 13.
    /// #833 added the `Translator` entity type and the
    /// `Classification is translated by Translator` binary fact type
    /// so the per-classification translator dispatch is fact-based
    /// (per AREST.tex §3 eq:sys) — bumping noun=24, FT=17, role=34.
    /// #833 layer 4 added the parallel-enum
    /// `Cardinality Constraint Kind` ↔ `Cardinality Constraint Kind
    /// Code` so translate_cardinality_constraints reads its kind
    /// codes from a registry — bumping noun=26, enum=15. Layer 5
    /// did the same for set constraints (`Set Constraint Kind` ↔
    /// `Set Constraint Kind Code`) — bumping noun=28, enum=17.
    /// Layer 6 added `Object Type Source Kind` ↔ `Object Type` so
    /// translate_nouns reads its kind→object_type mapping from the
    /// registry — bumping noun=30, enum=19. Layer 8 added a third
    /// parallel-enum `Set Constraint Arbitration Rule` so each set
    /// constraint kind names the predicate that decides whether
    /// translate_set_constraints emits or defers — bumping noun=31,
    /// enum=20. #789 added `Prose Stopword` enum (12 values) — bumping
    /// noun=32, enum=21. #791 added `Ring Adjective` enum (8 values)
    /// — bumping noun=33, enum=22.
    #[test]
    fn bootstrap_grammar_covers_expected_shapes() {
        let grammar = include_str!("../../../readings/forml2-grammar.md");
        let state = super::bootstrap_grammar_state(grammar).expect("bootstrap");

        let noun_count = fetch_or_phi("Noun", &state)
            .as_seq().map(|s| s.len()).unwrap_or(0);
        assert_eq!(noun_count, 41, "noun count");

        let ft_count = fetch_or_phi("FactType", &state)
            .as_seq().map(|s| s.len()).unwrap_or(0);
        assert_eq!(ft_count, 17, "fact type count");

        let role_count = fetch_or_phi("Role", &state)
            .as_seq().map(|s| s.len()).unwrap_or(0);
        assert_eq!(role_count, 34, "role count (2 per FT)");

        let enum_count = fetch_or_phi("EnumValues", &state)
            .as_seq().map(|s| s.len()).unwrap_or(0);
        assert_eq!(enum_count, 30, "enum-valued noun count");

        let dr_count = fetch_or_phi("DerivationRule", &state)
            .as_seq().map(|s| s.len()).unwrap_or(0);
        assert!(dr_count >= 30, "classifier rule count, got {}", dr_count);

        // Every rule must carry a parseable `json` binding so
        // `compile_to_defs_state`'s lossless path activates and
        // `re_resolve_rules` (legacy-dependent) is never called.
        let rules = fetch_or_phi("DerivationRule", &state);
        for f in rules.as_seq().expect("rules").iter() {
            let json = binding(f, "json").expect("rule carries json");
            let _parsed: crate::types::DerivationRuleDef =
                serde_json::from_str(json).expect("rule json round-trips");
        }
    }

    fn stage1_state(statement_id: &str, text: &str, nouns: &[&str]) -> Object {
        let cells = tokenize_statement(
            statement_id, text,
            &nouns.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        );
        let mut map: HashMap<String, Object> = cells.into_iter()
            .map(|(k, v)| (k, Object::Seq(v.into())))
            .collect();
        // Seed the `Noun` cell so Stage-2 translators that consult
        // the declared-noun catalog (e.g. `translate_set_constraints`'
        // antecedent-noun-count arbitration) see the same nouns that
        // Stage-1 was told about.
        let noun_facts: Vec<Object> = nouns.iter().map(|n| {
            fact_from_pairs(&[("name", *n), ("objectType", "entity")])
        }).collect();
        map.insert("Noun".to_string(), Object::Seq(noun_facts.into()));
        Object::Map(map)
    }

    #[test]
    fn entity_type_declaration_is_classified() {
        let stmt = stage1_state(
            "s1", "Customer is an entity type.", &["Customer"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Entity Type Declaration"),
            "expected Entity Type Declaration classification; got {:?}", kinds);
    }

    #[test]
    fn value_type_declaration_is_classified() {
        let stmt = stage1_state(
            "s1", "Priority is a value type.", &["Priority"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Value Type Declaration"),
            "expected Value Type Declaration classification; got {:?}", kinds);
    }

    #[test]
    fn abstract_declaration_is_classified() {
        let stmt = stage1_state("s1", "Request is abstract.", &["Request"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Abstract Declaration"),
            "expected Abstract Declaration; got {:?}", kinds);
    }

    #[test]
    fn ring_constraint_is_classified_per_adjective() {
        let cases: &[(&str, &[&str])] = &[
            ("Category has parent Category is acyclic.",  &["Category"]),
            ("Person is parent of Person is irreflexive.", &["Person"]),
            ("Person loves Person is symmetric.",          &["Person"]),
        ];
        for (text, nouns) in cases {
            let stmt = stage1_state("s1", text, nouns);
            let classified = classify_statements(&stmt, &grammar_state());
            let kinds = classifications_for(&idx(&classified),"s1");
            assert!(kinds.iter().any(|k| k == "Ring Constraint"),
                "expected Ring Constraint for {:?}; got {:?}", text, kinds);
        }
    }

    #[test]
    fn subtype_declaration_is_classified() {
        let stmt = stage1_state(
            "s1", "Support Request is a subtype of Request.",
            &["Support Request", "Request"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Subtype Declaration"),
            "expected Subtype Declaration; got {:?}", kinds);
    }

    #[test]
    fn fact_type_reading_classified_from_existential_role_reference() {
        let stmt = stage1_state(
            "s1", "Customer places Order.", &["Customer", "Order"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Fact Type Reading"),
            "expected Fact Type Reading; got {:?}", kinds);
    }

    #[test]
    fn translate_nouns_emits_entity_type_fact() {
        let stmt = stage1_state(
            "s1", "Customer is an entity type.", &["Customer"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let noun_facts = super::translate_nouns(&classified, &idx(&classified));
        assert_eq!(noun_facts.len(), 1);
        assert_eq!(binding(&noun_facts[0], "name"), Some("Customer"));
        assert_eq!(binding(&noun_facts[0], "objectType"), Some("entity"));
    }

    #[test]
    fn translate_nouns_emits_value_type_fact() {
        let stmt = stage1_state(
            "s1", "Priority is a value type.", &["Priority"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let noun_facts = super::translate_nouns(&classified, &idx(&classified));
        assert_eq!(noun_facts.len(), 1);
        assert_eq!(binding(&noun_facts[0], "name"), Some("Priority"));
        assert_eq!(binding(&noun_facts[0], "objectType"), Some("value"));
    }

    #[test]
    fn translate_nouns_skips_fact_type_reading_statements() {
        // Fact type readings have Head Noun but no entity/value declaration.
        let stmt = stage1_state(
            "s1", "Customer places Order.", &["Customer", "Order"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let noun_facts = super::translate_nouns(&classified, &idx(&classified));
        assert!(noun_facts.is_empty(),
            "fact-type readings must not produce Noun facts; got {:?}", noun_facts);
    }

    #[test]
    fn translate_nouns_handles_multiple_statements() {
        // Run each declaration through its own Stage-1 pass, then merge
        // the cells before classify — a tiny end-to-end check.
        let mut merged_cells: HashMap<String, Object> = HashMap::new();
        for (i, (text, nouns)) in [
            ("Customer is an entity type.", vec!["Customer"]),
            ("Priority is a value type.", vec!["Priority"]),
        ].into_iter().enumerate() {
            let stmt_id = format!("s{}", i);
            let cells = tokenize_statement(
                &stmt_id, text,
                &nouns.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            );
            for (name, facts) in cells {
                let entry = merged_cells.entry(name).or_insert_with(|| Object::Seq(Vec::new().into()));
                let existing = entry.as_seq().map(|s| s.to_vec()).unwrap_or_default();
                let mut combined = existing;
                combined.extend(facts);
                *entry = Object::Seq(combined.into());
            }
        }
        let stmt = Object::Map(merged_cells);
        let classified = classify_statements(&stmt, &grammar_state());
        let noun_facts = super::translate_nouns(&classified, &idx(&classified));
        assert_eq!(noun_facts.len(), 2);
        let by_name: HashMap<String, String> = noun_facts.iter()
            .filter_map(|f| {
                let name = binding(f, "name")?.to_string();
                let ot = binding(f, "objectType")?.to_string();
                Some((name, ot))
            })
            .collect();
        assert_eq!(by_name.get("Customer").map(String::as_str), Some("entity"));
        assert_eq!(by_name.get("Priority").map(String::as_str), Some("value"));
    }

    #[test]
    fn translate_nouns_abstract_wins_over_entity() {
        // Simulate two Statements on the same Head Noun: one Entity
        // Type Declaration + one Abstract Declaration. The merged
        // Noun fact must have objectType="abstract".
        let mut merged: HashMap<String, Object> = HashMap::new();
        for (i, (text, nouns)) in [
            ("Request is an entity type.", vec!["Request"]),
            ("Request is abstract.",       vec!["Request"]),
        ].into_iter().enumerate() {
            let stmt_id = format!("s{}", i);
            let cells = tokenize_statement(
                &stmt_id, text,
                &nouns.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            );
            for (name, facts) in cells {
                let entry = merged.entry(name).or_insert_with(|| Object::Seq(Vec::new().into()));
                let existing = entry.as_seq().map(|s| s.to_vec()).unwrap_or_default();
                let mut combined = existing;
                combined.extend(facts);
                *entry = Object::Seq(combined.into());
            }
        }
        let stmt = Object::Map(merged);
        let classified = classify_statements(&stmt, &grammar_state());
        let noun_facts = super::translate_nouns(&classified, &idx(&classified));
        assert_eq!(noun_facts.len(), 1);
        assert_eq!(binding(&noun_facts[0], "name"), Some("Request"));
        assert_eq!(binding(&noun_facts[0], "objectType"), Some("abstract"));
    }

    #[test]
    fn translate_subtypes_emits_subtype_fact() {
        let stmt = stage1_state(
            "s1", "Support Request is a subtype of Request.",
            &["Support Request", "Request"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let subtype_facts = super::translate_subtypes(&classified, &idx(&classified));
        assert_eq!(subtype_facts.len(), 1);
        assert_eq!(binding(&subtype_facts[0], "subtype"), Some("Support Request"));
        assert_eq!(binding(&subtype_facts[0], "supertype"), Some("Request"));
    }

    #[test]
    fn translate_subtypes_skips_non_subtype_statements() {
        let stmt = stage1_state(
            "s1", "Customer is an entity type.", &["Customer"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let subtype_facts = super::translate_subtypes(&classified, &idx(&classified));
        assert!(subtype_facts.is_empty());
    }

    #[test]
    fn translate_fact_types_emits_ft_and_role_facts_for_binary() {
        let stmt = stage1_state(
            "s1", "Customer places Order.", &["Customer", "Order"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let (ft, roles) = super::translate_fact_types(&classified, &idx(&classified));
        assert_eq!(ft.len(), 1);
        assert_eq!(binding(&ft[0], "id"), Some("Customer_places_Order"));
        assert_eq!(binding(&ft[0], "reading"), Some("Customer places Order"));
        assert_eq!(binding(&ft[0], "arity"), Some("2"));
        assert_eq!(roles.len(), 2);
        let positions: Vec<String> = roles.iter()
            .filter_map(|r| Some(format!("{}@{}",
                binding(r, "nounName")?,
                binding(r, "position")?)))
            .collect();
        assert!(positions.contains(&"Customer@0".to_string()), "got {:?}", positions);
        assert!(positions.contains(&"Order@1".to_string()), "got {:?}", positions);
    }

    #[test]
    fn translate_fact_types_skips_entity_type_declaration() {
        // `Customer is an entity type` matches Fact Type Reading
        // (has a Role Reference) but is excluded from FT emission.
        let stmt = stage1_state(
            "s1", "Customer is an entity type.", &["Customer"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let (ft, roles) = super::translate_fact_types(&classified, &idx(&classified));
        assert!(ft.is_empty(), "got FT facts: {:?}", ft);
        assert!(roles.is_empty());
    }

    #[test]
    fn translate_fact_types_skips_subtype_declaration() {
        let stmt = stage1_state(
            "s1", "Support Request is a subtype of Request.",
            &["Support Request", "Request"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let (ft, _) = super::translate_fact_types(&classified, &idx(&classified));
        assert!(ft.is_empty());
    }

    #[test]
    fn instance_fact_is_classified() {
        let stmt = stage1_state(
            "s1", "Customer 'alice' places Order 'o-7'.",
            &["Customer", "Order"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Instance Fact"),
            "expected Instance Fact; got {:?}", kinds);
    }

    #[test]
    fn translate_instance_facts_emits_subject_field_object() {
        let stmt = stage1_state(
            "s1", "Customer 'alice' places Order 'o-7'.",
            &["Customer", "Order"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let facts = super::translate_instance_facts(&classified, &idx(&classified));
        assert_eq!(facts.len(), 1);
        let f = &facts[0];
        assert_eq!(binding(f, "subjectNoun"),  Some("Customer"));
        assert_eq!(binding(f, "subjectValue"), Some("alice"));
        // translate_instance_facts (no FT context) falls back to the
        // raw verb — the pipeline passes declared FT ids via
        // translate_instance_facts_with_ft_ids to resolve canonically.
        assert_eq!(binding(f, "fieldName"),    Some("places"));
        assert_eq!(binding(f, "objectNoun"),   Some("Order"));
        assert_eq!(binding(f, "objectValue"),  Some("o-7"));
    }

    #[test]
    fn translate_instance_facts_with_ft_ids_resolves_canonical() {
        // When the canonical `subject_verb_object` FT id is declared,
        // the fieldName resolves to it. Same statement, with FT list.
        let stmt = stage1_state(
            "s1", "Customer 'alice' places Order 'o-7'.",
            &["Customer", "Order"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let facts = super::translate_instance_facts_with_ft_ids(
            &classified, &idx(&classified), &["Customer_places_Order".to_string()]);
        assert_eq!(facts.len(), 1);
        assert_eq!(binding(&facts[0], "fieldName"),
            Some("Customer_places_Order"));
    }

    /// Regression: instance-fact-side FT id construction must match the
    /// schema-side `fact_type_id_from_reading` even when the fact-type
    /// reading self-references one of its role nouns. For the reading
    /// `Fact Type has Layer Affinity to Layer`, the role noun `Layer`
    /// also appears inside the verb phrase (`Layer Affinity`); the
    /// schema-side walks the reading and lowercases that intra-verb
    /// occurrence, yielding `Fact_Type_has_layer_affinity_to_layer`.
    /// The previous binary-case code path used a format string that
    /// preserved the object noun's casing, producing
    /// `Fact_Type_has_layer_affinity_to_Layer` and breaking the
    /// declared-FT lookup so metamodel-targeting instance facts
    /// silently dropped on the floor.
    #[test]
    fn translate_instance_facts_resolves_canonical_for_self_referential_reading() {
        // Drive both schema-side and instance-side through the SAME stage1
        // input so the role-detection and walking are symmetric. The FT
        // declaration `Fact Type has Layer Affinity to Layer.` is parsed
        // first; its id from fact_type_id_from_reading IS the declared
        // FT id we expect translate_instance_facts to resolve to.
        let decl_stmt = stage1_state(
            "s_decl",
            "Fact Type has Layer Affinity to Layer.",
            &["Fact Type", "Layer"]);
        let decl_classified = classify_statements(&decl_stmt, &grammar_state());
        let (ft_facts, _role_facts) = super::translate_fact_types(
            &decl_classified, &idx(&decl_classified));
        assert_eq!(ft_facts.len(), 1, "expected one FactType from declaration");
        let canonical_id = binding(&ft_facts[0], "id")
            .expect("FactType must have id binding").to_string();

        let stmt = stage1_state(
            "s_inst",
            "Fact Type 'Decision_was_made_by_Agent' has Layer Affinity to Layer 'SPD1-6'.",
            &["Fact Type", "Layer"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let facts = super::translate_instance_facts_with_ft_ids(
            &classified, &idx(&classified), &[canonical_id.clone()]);
        assert_eq!(facts.len(), 1);
        assert_eq!(
            binding(&facts[0], "fieldName"),
            Some(canonical_id.as_str()),
            "instance-fact FT id must match the schema-side canonical id even \
             when the reading self-references a role noun (canonical was {})",
            canonical_id);
    }

    #[test]
    fn translate_instance_facts_skips_non_instance_statements() {
        let stmt = stage1_state(
            "s1", "Customer places Order.", &["Customer", "Order"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let facts = super::translate_instance_facts(&classified, &idx(&classified));
        assert!(facts.is_empty(), "got {:?}", facts);
    }

    /// #553 — ternary instance facts must preserve the third role's
    /// noun + literal in the InstanceFact cell. Mirrors wine.md's
    /// `Wine App requires DLL Override of DLL Name 'D' with DLL
    /// Behavior 'B'` shape: three roles, all three with literals,
    /// three matched declared nouns.
    #[test]
    fn translate_instance_facts_emits_third_role_for_ternary() {
        let stmt = stage1_state(
            "s1",
            "Wine App 'office-2016-word' requires DLL Override of \
             DLL Name 'riched20.dll' with DLL Behavior 'native'.",
            &["Wine App", "DLL Name", "DLL Behavior"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let facts = super::translate_instance_facts(&classified, &idx(&classified));
        assert_eq!(facts.len(), 1, "expected 1 instance fact; got {:?}", facts);
        let f = &facts[0];
        assert_eq!(binding(f, "subjectNoun"),  Some("Wine App"));
        assert_eq!(binding(f, "subjectValue"), Some("office-2016-word"));
        assert_eq!(binding(f, "objectNoun"),   Some("DLL Name"));
        assert_eq!(binding(f, "objectValue"),  Some("riched20.dll"));
        // The third role: noun + literal must survive the parse.
        assert_eq!(binding(f, "role2Noun"),    Some("DLL Behavior"));
        assert_eq!(binding(f, "role2Value"),   Some("native"));
    }

    /// #553 — ternary instance facts whose canonical 3-role FT id is
    /// declared resolve `fieldName` to that id (not the bare verb).
    /// Confirms the FT-resolution path now considers all three roles
    /// when constructing the canonical id to match against.
    #[test]
    fn translate_instance_facts_with_ft_ids_resolves_canonical_for_ternary() {
        let stmt = stage1_state(
            "s1",
            "Wine App 'office-2016-word' requires DLL Override of \
             DLL Name 'riched20.dll' with DLL Behavior 'native'.",
            &["Wine App", "DLL Name", "DLL Behavior"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let canonical_id =
            "Wine_App_requires_dll_override_of_DLL_Name_with_DLL_Behavior".to_string();
        let facts = super::translate_instance_facts_with_ft_ids(
            &classified, &idx(&classified), &[canonical_id.clone()]);
        assert_eq!(facts.len(), 1);
        assert_eq!(binding(&facts[0], "fieldName"), Some(canonical_id.as_str()));
    }

    /// #553 end-to-end: parse the real `readings/compat/wine.md`
    /// (with filesystem.md preloaded so `Directory` is in scope) and
    /// confirm the canonical 3-role cells are emitted with all three
    /// role bindings. The bundled steam-windows / spotify / notion-
    /// desktop instance facts cover DLL Override, Registry Key, and
    /// Environment Variable shapes respectively.
    #[cfg(feature = "compat-readings")]
    #[test]
    fn ternary_instance_facts_land_in_canonical_cells_via_real_parse() {
        let filesystem_md = include_str!("../../../readings/os/filesystem.md");
        let wine_md = include_str!("../../../readings/compat/wine.md");
        let fs_state = crate::parse_forml2::parse_to_state(filesystem_md)
            .expect("filesystem.md must parse");
        let state = crate::parse_forml2::parse_to_state_from(wine_md, &fs_state)
            .expect("wine.md must parse");

        // DLL Override: steam-windows requires dwrite.dll = disabled.
        let dll_cell = crate::ast::fetch_or_phi(
            "Wine_App_requires_dll_override_of_DLL_Name_with_DLL_Behavior",
            &state);
        let dll_seq = dll_cell.as_seq()
            .expect("ternary DLL Override cell must be populated");
        let dwrite = dll_seq.iter().find(|f| {
            crate::ast::binding(f, "Wine App") == Some("steam-windows")
                && crate::ast::binding(f, "DLL Name") == Some("dwrite.dll")
        }).expect("dwrite override fact must be present");
        assert_eq!(crate::ast::binding(dwrite, "DLL Behavior"), Some("disabled"),
            "third role must survive in the canonical cell");

        // Registry Key: spotify CrashReporter = disabled.
        let reg_cell = crate::ast::fetch_or_phi(
            "Wine_App_requires_registry_key_at_Registry_Path_with_Registry_Value",
            &state);
        let reg_seq = reg_cell.as_seq()
            .expect("ternary Registry Key cell must be populated");
        let spot = reg_seq.iter().find(|f| {
            crate::ast::binding(f, "Wine App") == Some("spotify")
        }).expect("spotify registry fact must be present");
        assert_eq!(crate::ast::binding(spot, "Registry Path"),
            Some("HKCU\\\\Software\\\\Spotify\\\\CrashReporter"));
        assert_eq!(crate::ast::binding(spot, "Registry Value"), Some("disabled"));

        // Environment Variable: notion-desktop WINEDLLOVERRIDES = libglesv2=b.
        let env_cell = crate::ast::fetch_or_phi(
            "Wine_App_requires_environment_variable_with_Env_Var_Name_and_Env_Var_Value",
            &state);
        let env_seq = env_cell.as_seq()
            .expect("ternary Environment Variable cell must be populated");
        let nv = env_seq.iter().find(|f| {
            crate::ast::binding(f, "Wine App") == Some("notion-desktop")
                && crate::ast::binding(f, "Env Var Name") == Some("WINEDLLOVERRIDES")
        }).expect("notion-desktop WINEDLLOVERRIDES fact must be present");
        assert_eq!(crate::ast::binding(nv, "Env Var Value"), Some("libglesv2=b"));
    }

    /// #553 — `instance_fact_field_cells` must propagate the third
    /// role into the per-field cell so downstream readers (CLI / .reg
    /// writers) can `binding(fact, "DLL Behavior")` instead of
    /// re-parsing the raw markdown.
    #[test]
    fn instance_fact_field_cells_includes_third_role_binding() {
        // Build a single InstanceFact that carries all three roles
        // (mirrors what translate_instance_facts now emits).
        let inst = fact_from_pairs(&[
            ("subjectNoun",  "Wine App"),
            ("subjectValue", "office-2016-word"),
            ("fieldName",    "Wine_App_requires_dll_override_of_DLL_Name_with_DLL_Behavior"),
            ("objectNoun",   "DLL Name"),
            ("objectValue",  "riched20.dll"),
            ("role2Noun",    "DLL Behavior"),
            ("role2Value",   "native"),
        ]);
        let cells = super::instance_fact_field_cells(&[inst]);
        let (_name, facts) = cells.iter()
            .find(|(n, _)| n ==
                "Wine_App_requires_dll_override_of_DLL_Name_with_DLL_Behavior")
            .expect("expected per-field cell for the canonical FT id");
        assert_eq!(facts.len(), 1);
        let f = &facts[0];
        assert_eq!(binding(f, "Wine App"),     Some("office-2016-word"));
        assert_eq!(binding(f, "DLL Name"),     Some("riched20.dll"));
        assert_eq!(binding(f, "DLL Behavior"), Some("native"));
    }

    #[test]
    fn translate_ring_constraints_covers_all_eight_adjectives() {
        for (text, nouns, expected_kind) in [
            ("Category has parent Category is acyclic.",    vec!["Category"], "AC"),
            ("Person is parent of Person is irreflexive.",  vec!["Person"],   "IR"),
            ("Person loves Person is symmetric.",           vec!["Person"],   "SY"),
            ("Thing owns Thing is asymmetric.",             vec!["Thing"],    "AS"),
            ("Thing owns Thing is antisymmetric.",          vec!["Thing"],    "AT"),
            ("Thing owns Thing is transitive.",             vec!["Thing"],    "TR"),
            ("Thing owns Thing is intransitive.",           vec!["Thing"],    "IT"),
            ("Thing owns Thing is reflexive.",              vec!["Thing"],    "RF"),
        ] {
            let stmt = stage1_state("s1", text, &nouns);
            let classified = classify_statements(&stmt, &grammar_state());
            let constraints = super::translate_ring_constraints(&classified, &idx(&classified));
            assert_eq!(constraints.len(), 1, "text={:?}", text);
            assert_eq!(binding(&constraints[0], "kind"), Some(expected_kind),
                "text={:?}", text);
            assert_eq!(binding(&constraints[0], "modality"), Some("alethic"));
        }
    }

    /// #326: a ring constraint using the "No X R-s itself." shape
    /// must attach its spans to the self-referential binary FT
    /// `X R X`, not to whichever other X-bearing FT happens to be
    /// enumerated first. Previously the span lookup fell back to
    /// `roles_by_noun`, which on `find_noun_sequence` returning
    /// `["App"]` picked the first App FT in hashmap order — for the
    /// bundled metamodel that was `App has navigable Domain`, not
    /// `App extends App`, and the validator flagged the ring as
    /// landing on `{App, Domain}` with "expected matched pair".
    #[test]
    fn ring_constraint_on_itself_resolves_to_self_referential_ft() {
        let src = "\
            App is an entity type.
            Domain is an entity type.
            App has navigable Domain.
            App extends App.
            No App extends itself.
        ";
        let state = super::parse_to_state_via_stage12(src)
            .expect("parse_to_state_via_stage12");
        let constraints = crate::ast::fetch_or_phi("Constraint", &state);
        let ring = constraints.as_seq()
            .expect("Constraint cell Seq")
            .iter()
            .find(|c| binding(c, "kind") == Some("IR"))
            .cloned()
            .expect("one IR ring constraint");
        let span = binding(&ring, "span0_factTypeId")
            .expect("span0_factTypeId on ring");
        assert!(span.contains("App") && span.contains("extend"),
            "ring span attached to {span}; expected self-referential `App extends App`");
        assert!(!span.contains("Domain"),
            "ring span must not land on `App has navigable Domain`; got {span}");
    }

    /// Same fix across the three Metamodel shapes that were failing:
    /// Noun / Derivation Rule / App.
    #[test]
    fn ring_on_itself_resolves_across_metamodel_noun_kinds() {
        let src = "\
            Noun is an entity type.
            Object Type is an entity type.
            Derivation Rule is an entity type.
            Text is an entity type.
            Noun has Object Type.
            Noun is subtype of Noun.
            Derivation Rule has Text.
            Derivation Rule depends on Derivation Rule.
            No Noun is subtype of itself.
            No Derivation Rule depends on itself.
        ";
        let state = super::parse_to_state_via_stage12(src)
            .expect("parse_to_state_via_stage12");
        let constraints = crate::ast::fetch_or_phi("Constraint", &state);
        let rings: Vec<_> = constraints.as_seq()
            .expect("Constraint cell Seq")
            .iter()
            .filter(|c| binding(c, "kind") == Some("IR"))
            .cloned()
            .collect();
        assert_eq!(rings.len(), 2,
            "expected 2 IR ring constraints; got {}", rings.len());
        for r in &rings {
            let span = binding(r, "span0_factTypeId")
                .expect("span0_factTypeId on ring");
            // Must not land on a mixed-noun FT.
            assert!(!span.contains("Object") && !span.contains("_has_Text"),
                "ring span attached to non-self-referential {span}");
        }
    }

    #[test]
    fn translate_ring_constraints_skips_non_ring_statements() {
        let stmt = stage1_state(
            "s1", "Customer is an entity type.", &["Customer"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let constraints = super::translate_ring_constraints(&classified, &idx(&classified));
        assert!(constraints.is_empty());
    }

    #[test]
    fn translate_derivation_rules_captures_text() {
        let stmt = stage1_state(
            "s1",
            "Customer has Full Name iff Customer has First Name.",
            &["Customer", "Full Name", "First Name"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let rules = super::translate_derivation_rules(&classified, &idx(&classified));
        assert_eq!(rules.len(), 1);
        assert!(binding(&rules[0], "text").unwrap()
                .contains("Customer has Full Name iff"));
    }

    #[test]
    fn translate_derivation_rules_skips_non_derivations() {
        let stmt = stage1_state("s1", "Customer is an entity type.", &["Customer"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let rules = super::translate_derivation_rules(&classified, &idx(&classified));
        assert!(rules.is_empty());
    }

    #[test]
    fn deontic_constraint_is_classified_for_obligatory() {
        let stmt = stage1_state(
            "s1", "It is obligatory that Customer has Email.",
            &["Customer", "Email"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Deontic Constraint"),
            "expected Deontic Constraint; got {:?}", kinds);
    }

    #[test]
    fn translate_deontic_constraints_emits_with_operator_and_entity() {
        let stmt = stage1_state(
            "s1", "It is forbidden that Support Response uses Dash.",
            &["Support Response", "Dash"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let constraints = super::translate_deontic_constraints(&classified, &idx(&classified));
        assert_eq!(constraints.len(), 1);
        assert_eq!(binding(&constraints[0], "modality"), Some("deontic"));
        assert_eq!(binding(&constraints[0], "deonticOperator"), Some("forbidden"));
        assert_eq!(binding(&constraints[0], "entity"), Some("Support Response"));
    }

    /// MC4b (#751): the deontic translator recognises the tight
    /// `<role> ends with '<lit>'` clause-shape inside a forbidden
    /// constraint body and emits an encoded `predicate` field on the
    /// Constraint cell. The compiler reads it back via
    /// `DeonticPredicate::decode` and uses `Func::filter` to gate
    /// per-fact violations on the population path.
    #[test]
    fn translate_deontic_constraints_captures_ends_with_predicate() {
        let stmt = stage1_state(
            "s1",
            "It is forbidden that each Noun has a name that ends with 'ies'.",
            &["Noun"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let constraints = super::translate_deontic_constraints(&classified, &idx(&classified));
        assert_eq!(constraints.len(), 1, "one deontic constraint");
        let predicate = binding(&constraints[0], "predicate")
            .expect("predicate field must be present on the Constraint cell");
        let decoded = crate::types::DeonticPredicate::decode(predicate)
            .expect("predicate field must round-trip through DeonticPredicate::decode");
        match decoded {
            crate::types::DeonticPredicate::EndsWith { role, literal, negated } => {
                assert_eq!(role, "name");
                assert_eq!(literal, "ies");
                assert!(!negated);
            }
            other => panic!("expected EndsWith, got {:?}", other),
        }
    }

    #[test]
    fn enum_values_declaration_is_classified() {
        let stmt = stage1_state(
            "s1",
            "The possible values of Priority are 'low', 'medium', 'high'.",
            &["Priority"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Enum Values Declaration"),
            "expected Enum Values Declaration; got {:?}", kinds);
    }

    #[test]
    fn translate_enum_values_emits_value_list_for_noun() {
        let stmt = stage1_state(
            "s1",
            "The possible values of Priority are 'low', 'medium', 'high'.",
            &["Priority"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let facts = super::translate_enum_values(&classified, &idx(&classified));
        assert_eq!(facts.len(), 1);
        let f = &facts[0];
        assert_eq!(binding(f, "noun"), Some("Priority"));
        assert_eq!(binding(f, "value0"), Some("low"));
        assert_eq!(binding(f, "value1"), Some("medium"));
        assert_eq!(binding(f, "value2"), Some("high"));
    }

    #[test]
    fn partition_declaration_is_classified() {
        let stmt = stage1_state(
            "s1", "Animal is partitioned into Cat, Dog, Bird.",
            &["Animal", "Cat", "Dog", "Bird"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Partition Declaration"),
            "expected Partition Declaration; got {:?}", kinds);
    }

    #[test]
    fn translate_partitions_emits_subtype_facts_and_marks_supertype_abstract() {
        let stmt = stage1_state(
            "s1", "Animal is partitioned into Cat, Dog, Bird.",
            &["Animal", "Cat", "Dog", "Bird"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let subtypes = super::translate_partitions(&classified, &idx(&classified));
        let subs: Vec<_> = subtypes.iter()
            .filter_map(|f| binding(f, "subtype").map(String::from))
            .collect();
        assert_eq!(subs, vec!["Cat", "Dog", "Bird"]);
        for s in &subtypes {
            assert_eq!(binding(s, "supertype"), Some("Animal"));
        }
        // translate_nouns must see Partition Declaration as a signal
        // to mark Animal as abstract.
        let nouns = super::translate_nouns(&classified, &idx(&classified));
        let animal = nouns.iter()
            .find(|f| binding(f, "name") == Some("Animal"))
            .expect("Animal noun fact");
        assert_eq!(binding(animal, "objectType"), Some("abstract"));
    }

    #[test]
    fn value_constraint_is_classified_via_enum_values_recursive_rule() {
        // The grammar rule `Statement has Classification 'Value
        // Constraint' iff Statement has Classification 'Enum Values
        // Declaration'` fires after the Enum Values Declaration rule,
        // giving every enum-values statement a Value Constraint
        // classification too.
        let stmt = stage1_state(
            "s1",
            "The possible values of Priority are 'low', 'medium', 'high'.",
            &["Priority"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Value Constraint"),
            "expected Value Constraint; got {:?}", kinds);
    }

    #[test]
    fn uniqueness_constraint_is_classified_on_exactly_one() {
        let stmt = stage1_state(
            "s1",
            "Each Order was placed by exactly one Customer.",
            &["Order", "Customer"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Uniqueness Constraint"),
            "expected Uniqueness Constraint; got {:?}", kinds);
    }

    #[test]
    fn mandatory_role_constraint_is_classified_on_at_least_one() {
        let stmt = stage1_state(
            "s1",
            "Each Customer has at least one Email.",
            &["Customer", "Email"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Mandatory Role Constraint"),
            "expected Mandatory Role Constraint; got {:?}", kinds);
    }

    #[test]
    fn frequency_constraint_is_classified_on_at_most_and_at_least() {
        let stmt = stage1_state(
            "s1",
            "Each Order has at most 5 and at least 2 Line Items.",
            &["Order", "Line Item"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Frequency Constraint"),
            "expected Frequency Constraint; got {:?}", kinds);
    }

    #[test]
    fn equality_constraint_is_classified_on_if_and_only_if() {
        let stmt = stage1_state(
            "s1",
            "Each Employee is paid if and only if Employee has Salary.",
            &["Employee", "Salary"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Equality Constraint"),
            "expected Equality Constraint; got {:?}", kinds);
    }

    #[test]
    fn exclusion_constraint_is_classified_on_at_most_one_of_the_following() {
        let stmt = stage1_state(
            "s1",
            "For each Account at most one of the following holds: Account is open; Account is closed.",
            &["Account"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Exclusion Constraint"),
            "expected Exclusion Constraint (multi-clause form); got {:?}", kinds);
    }

    #[test]
    fn exclusive_or_constraint_is_classified() {
        let stmt = stage1_state(
            "s1",
            "For each Order exactly one of the following holds: Order is draft; Order is placed.",
            &["Order"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Exclusive-Or Constraint"),
            "expected Exclusive-Or Constraint; got {:?}", kinds);
    }

    #[test]
    fn or_constraint_is_classified() {
        let stmt = stage1_state(
            "s1",
            "For each User at least one of the following holds: User has Email; User has Phone.",
            &["User", "Email", "Phone"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Or Constraint"),
            "expected Or Constraint; got {:?}", kinds);
    }

    #[test]
    fn subset_constraint_is_classified_on_if_some_then_that() {
        let stmt = stage1_state(
            "s1",
            "If some User owns some Organization then that User has some Email.",
            &["User", "Organization", "Email"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Subset Constraint"),
            "expected Subset Constraint; got {:?}", kinds);
    }

    #[test]
    fn translate_set_constraints_includes_subset() {
        // `If some X then that Y` with ≥2 distinct declared antecedent
        // nouns is a subset constraint (ORM 2 shape). Stage-1 emits
        // Keyword 'if' unconditionally, so BOTH SS and Derivation
        // Rule classifications fire; Stage-2 translators arbitrate by
        // counting distinct declared nouns in the antecedent.
        // Here antecedent has `User` + `Organization` (2 distinct) →
        // SS wins; translate_derivation_rules defers.
        let stmt = stage1_state(
            "s1",
            "If some User owns some Organization then that User has some Email.",
            &["User", "Organization", "Email"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Subset Constraint"),
            "expected Subset Constraint; got {:?}", kinds);
        assert!(kinds.iter().any(|k| k == "Derivation Rule"),
            "expected Derivation Rule classification (arbitrated below); \
             got {:?}", kinds);
        let constraints = super::translate_set_constraints(&classified, &idx(&classified));
        let ss: Vec<_> = constraints.iter()
            .filter(|f| binding(f, "kind") == Some("SS"))
            .collect();
        assert_eq!(ss.len(), 1, "expected 1 SS, got {:?}", constraints);
        assert_eq!(binding(ss[0], "modality"), Some("alethic"));
        let rules = super::translate_derivation_rules(&classified, &idx(&classified));
        assert!(rules.is_empty(),
            "expected no Derivation Rule emission (SS wins); got {:?}",
            rules);
    }

    #[test]
    fn translate_derivation_rules_wins_when_subset_has_under_two_nouns() {
        // Same `If some ... then that ...` shape but only ONE distinct
        // declared noun in the antecedent — legacy's `try_subset`
        // would fail the multi-noun check, and `try_derivation` picks
        // up the slack. Match that precedence.
        //
        // "some Stuff" — "Stuff" is not a declared noun. antecedent
        // distinct count = 0 < 2. DR wins, SS defers.
        let stmt = stage1_state(
            "s1",
            "If some Stuff matches some Thing then that Stuff is Thing.",
            &["Stuff", "Thing"]);
        // Override the Noun cell to force only one of the referenced
        // nouns to actually be declared, matching the legacy "nouns
        // in the antecedent are mostly unknown" shape.
        let stmt_only_thing = {
            let mut map = match stmt {
                Object::Map(m) => m,
                _ => unreachable!(),
            };
            let noun = fact_from_pairs(&[("name", "Thing"), ("objectType", "entity")]);
            map.insert("Noun".to_string(), Object::Seq(alloc::vec![noun].into()));
            Object::Map(map)
        };
        let classified = classify_statements(&stmt_only_thing, &grammar_state());
        let ss = super::translate_set_constraints(&classified, &idx(&classified));
        assert!(ss.is_empty(), "SS defers when antecedent nouns < 2; got {:?}", ss);
        let rules = super::translate_derivation_rules(&classified, &idx(&classified));
        assert_eq!(rules.len(), 1,
            "DR picks up the statement when SS defers; got {:?}", rules);
    }

    #[test]
    fn translate_set_constraints_emits_eq_xc_xo_or() {
        let nouns_all = &["Employee", "Salary", "Account", "Order", "User", "Email", "Phone"];
        let eq = stage1_state("s-eq",
            "Each Employee is paid if and only if Employee has Salary.", nouns_all);
        let xc = stage1_state("s-xc",
            "For each Account at most one of the following holds: Account is open; Account is closed.", nouns_all);
        let xo = stage1_state("s-xo",
            "For each Order exactly one of the following holds: Order is draft; Order is placed.", nouns_all);
        let or_stmt = stage1_state("s-or",
            "For each User at least one of the following holds: User has Email; User has Phone.", nouns_all);
        let merged = crate::ast::merge_states(&eq, &xc);
        let merged = crate::ast::merge_states(&merged, &xo);
        let merged = crate::ast::merge_states(&merged, &or_stmt);
        let classified = classify_statements(&merged, &grammar_state());

        let constraints = super::translate_set_constraints(&classified, &idx(&classified));
        let by_kind = |k: &str| -> Vec<&Object> {
            constraints.iter().filter(|f| binding(f, "kind") == Some(k)).collect()
        };
        assert_eq!(by_kind("EQ").len(), 1, "expected 1 EQ, got {:?}", constraints);
        assert_eq!(by_kind("XC").len(), 1, "expected 1 XC, got {:?}", constraints);
        assert_eq!(by_kind("XO").len(), 1, "expected 1 XO, got {:?}", constraints);
        assert_eq!(by_kind("OR").len(), 1, "expected 1 OR, got {:?}", constraints);
        for c in &constraints {
            assert_eq!(binding(c, "modality"), Some("alethic"));
        }
    }

    #[test]
    fn translate_cardinality_constraints_emits_uc_mc_fc() {
        // `exactly one` splits into UC + MC (1+1), `at least one`
        // gives a second MC (0+1), and `at most N and at least M`
        // gives FC (0+0+1). Expected totals: UC=1, MC=2, FC=1.
        let nouns_list = &["Order", "Customer", "Email", "Line Item"];
        let uc = stage1_state("s-uc",
            "Each Order was placed by exactly one Customer.", nouns_list);
        let mc = stage1_state("s-mc",
            "Each Customer has at least one Email.", nouns_list);
        let fc = stage1_state("s-fc",
            "Each Order has at most 5 and at least 2 Line Items.", nouns_list);
        let merged = crate::ast::merge_states(&uc, &mc);
        let merged = crate::ast::merge_states(&merged, &fc);
        let classified = classify_statements(&merged, &grammar_state());

        let constraints = super::translate_cardinality_constraints(&classified, &idx(&classified));
        let by_kind = |k: &str| -> Vec<&Object> {
            constraints.iter().filter(|f| binding(f, "kind") == Some(k)).collect()
        };
        assert_eq!(by_kind("UC").len(), 1, "expected 1 UC, got {:?}", constraints);
        assert_eq!(by_kind("MC").len(), 2, "expected 2 MC, got {:?}", constraints);
        assert_eq!(by_kind("FC").len(), 1, "expected 1 FC, got {:?}", constraints);
        for c in &constraints {
            assert_eq!(binding(c, "modality"), Some("alethic"));
        }
    }

    #[test]
    fn mandatory_role_constraint_fires_on_some_quantifier() {
        // ORM 2 plural `some` = "at least one" — `Each X has some Y`
        // is MC. Stage-1 emits `Statement has Quantifier 'some'`; the
        // grammar routes it to Mandatory Role Constraint.
        let stmt = stage1_state(
            "s1", "Each Noun plays some Role.", &["Noun", "Role"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let kinds = classifications_for(&idx(&classified),"s1");
        assert!(kinds.iter().any(|k| k == "Mandatory Role Constraint"),
            "expected MC classification for 'some' quantifier; got {:?}", kinds);
    }

    #[test]
    fn translate_value_constraints_emits_vc_per_enum_noun() {
        let stmt = stage1_state(
            "s1",
            "The possible values of Priority are 'low', 'medium', 'high'.",
            &["Priority"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let vcs = super::translate_value_constraints(&classified, &idx(&classified));
        assert_eq!(vcs.len(), 1);
        let f = &vcs[0];
        assert_eq!(binding(f, "kind"), Some("VC"));
        assert_eq!(binding(f, "modality"), Some("alethic"));
        assert_eq!(binding(f, "entity"), Some("Priority"));
    }

    // ------------------------------------------------------------------
    // #294 — Diagnostic parse-and-diff harness.
    //
    // `parse_to_state_via_stage12` is the capstone pipeline (#285 will
    // replace `parse_into`'s legacy cascade with a call to it). Before
    // the wire-up, run both pipelines on every bundled reading file and
    // diff the key metamodel cells. Any divergence is a real gap.
    // ------------------------------------------------------------------

    // ─── #309 reserved-substring rejection ───────────────────────────

    #[test]
    fn stage12_pipeline_rejects_reserved_substring_entity_name() {
        let err = super::parse_to_state_via_stage12(
            "# Demo\n\nEach Way Bet(.id) is an entity type.\n"
        ).expect_err("expected rejection");
        assert!(err.contains("Each Way Bet"),
            "diagnostic must name the offending noun; got: {}", err);
        assert!(err.contains("each"),
            "diagnostic must name the offending keyword; got: {}", err);
    }

    #[test]
    fn stage12_pipeline_rejects_reserved_substring_value_type() {
        let err = super::parse_to_state_via_stage12(
            "No Show Fee is a value type.\n"
        ).expect_err("expected rejection");
        assert!(err.contains("No Show Fee"));
        assert!(err.contains("no"));
    }

    #[test]
    fn stage12_pipeline_rejects_reserved_substring_subtype() {
        let err = super::parse_to_state_via_stage12(
            "Animal is an entity type.\n\
             At Most One Hop is a subtype of Animal.\n"
        ).expect_err("expected rejection");
        assert!(err.contains("At Most One Hop"));
        assert!(err.contains("at most one"));
    }

    #[test]
    fn stage12_pipeline_accepts_quoted_reserved_substring() {
        // Quoted identifiers bypass the reserved-word check.
        // `Noun 'Each Way Bet'` treats the whole quoted span as a
        // single token; legacy-parse still needs to accept it, so
        // pair with a plain declaration it already understands.
        // If legacy rejects the quoted form, the test will fail with
        // a legacy-side error rather than a #309 rejection.
        let result = super::parse_to_state_via_stage12(
            "Customer is an entity type.\n"
        );
        assert!(result.is_ok(),
            "plain entity declaration must pass: {:?}", result.err());
    }

    #[test]
    fn stage12_pipeline_smoke_entity_type() {
        let state = super::parse_to_state_via_stage12(
            "# Smoke\n\nCustomer is an entity type.\n"
        ).expect("pipeline ran");
        let nouns = fetch_or_phi("Noun", &state);
        let names: Vec<String> = nouns.as_seq()
            .map(|s| s.iter().filter_map(|f| binding(f, "name").map(String::from)).collect())
            .unwrap_or_default();
        assert!(names.iter().any(|n| n == "Customer"),
            "expected Customer in Noun cell; got {:?}", names);
    }

    #[test]
    fn stage12_pipeline_smoke_subtype() {
        let text = "Animal is an entity type.\nDog is a subtype of Animal.\n";
        let state = super::parse_to_state_via_stage12(text).expect("ran");
        let subs = fetch_or_phi("Subtype", &state);
        let pairs: Vec<(String, String)> = subs.as_seq()
            .map(|s| s.iter().filter_map(|f| {
                Some((binding(f, "subtype")?.to_string(),
                     binding(f, "supertype")?.to_string()))
            }).collect())
            .unwrap_or_default();
        assert!(pairs.contains(&("Dog".to_string(), "Animal".to_string())),
            "expected (Dog, Animal) in Subtype cell; got {:?}", pairs);
    }

    // `diff_organization_fixture` was a legacy-vs-stage12 parity check
    // calling `parse_to_state_legacy`; retired with the legacy cascade
    // in #285 (stage12 is the parser now).

    /// Stage-1/2 must preserve all instance facts even when their
    /// quoted values contain shell/tokenizer-significant characters
    /// like `<`. apps/tasks bug (#821): a single `<` inside one
    /// Subject value silently truncated every subsequent Task Subject
    /// from the parsed cell — observed boundary at Task 622/623 because
    /// 623's subject was the first to contain `<`. Apps/tasks
    /// instance-fact migrations carry user-typed prose; characters
    /// like `<` in `< 30 s` must round-trip cleanly.
    #[test]
    fn instance_fact_value_containing_lessthan_does_not_truncate_subsequent_facts() {
        // Stage-2 itself preserves all facts even at file scale (540
        // facts × one with `<` in its value: all land). The empirical
        // apps/tasks symptom — boundary at Task 622 in the WASM-side
        // cell — must therefore live downstream of Stage-2 (validate,
        // merge_states, or handle-cache invalidation in
        // src/mcp/server.ts). This test pins the parser-side
        // robustness; the downstream investigation is tracked
        // separately.
        let mut src = String::new();
        src.push_str("Task(.id) is an entity type.\n");
        src.push_str("Task Subject is a value type.\n");
        src.push_str("Task has Task Subject.\n");
        for n in 100..640 {
            if n == 510 {
                src.push_str(&alloc::format!(
                    "Task '{}' has Task Subject 'reachable in < 30 s'.\n", n));
            } else {
                src.push_str(&alloc::format!(
                    "Task '{}' has Task Subject 'subject {}'.\n", n, n));
            }
        }
        let state = super::parse_to_state_via_stage12(&src)
            .expect("parse_to_state_via_stage12");
        let cell = fetch_or_phi("Task_has_Task_Subject", &state);
        let entries: Vec<&Object> = cell.as_seq()
            .map(|s| s.iter().collect())
            .unwrap_or_default();
        let task_ids: Vec<&str> = entries.iter()
            .filter_map(|f| f.as_seq())
            .filter_map(|pairs| pairs.iter().find_map(|p| {
                let kv = p.as_seq()?;
                let role = kv.first()?.as_atom()?;
                if role == "Task" { kv.get(1)?.as_atom() } else { None }
            }))
            .collect();
        let expected = 540;  // 100..640 inclusive of start, exclusive of end
        assert!(task_ids.contains(&"509"), "Task 509 (pre-<) must parse; got {} entries", task_ids.len());
        assert!(task_ids.contains(&"510"), "Task 510 (with <) must parse; got {} entries", task_ids.len());
        assert!(task_ids.contains(&"511"), "Task 511 (after <) must parse; got {} entries", task_ids.len());
        assert!(task_ids.contains(&"639"), "Task 639 (last) must parse; got {} entries", task_ids.len());
        assert_eq!(entries.len(), expected,
            "all {} instance facts must land in the cell; got {} entries — likely truncated past the `<`",
            expected, entries.len());
    }

    /// Stage-2 must populate `consequentFactTypeId` on emitted
    /// DerivationRule cell facts when the rule's consequent matches a
    /// declared FactType reading. Without this, downstream
    /// `re_resolve_rules` has nothing to resolve, the compiler can't
    /// route the rule to its target cell, and forward-chain firing
    /// drops the rule on the floor — which is why apps/tasks's
    /// `Task has Task Readiness 'ready' iff Task has Task Status 'pending'.`
    /// never materialises a Task_has_Task_Readiness cell entry.
    /// See task #822 in apps/tasks for the full engine-gap diagnosis.
    #[test]
    fn stage12_pipeline_populates_consequent_fact_type_id_for_literal_iff_rule() {
        let src = "\
            Task is an entity type.\n\
            Task Status is a value type.\n\
            Task Readiness is a value type.\n\
            Task has Task Status.\n\
            Task has Task Readiness.\n\
            Task has Task Readiness 'ready' iff Task has Task Status 'pending'.\n\
        ";
        let state = super::parse_to_state_via_stage12(src)
            .expect("parse_to_state_via_stage12");
        let rules = fetch_or_phi("DerivationRule", &state);
        let rule_seq = rules.as_seq()
            .expect("DerivationRule cell must be a Seq");
        assert_eq!(rule_seq.len(), 1,
            "exactly one derivation rule expected; got {}", rule_seq.len());
        let consequent_ft = binding(&rule_seq[0], "consequentFactTypeId")
            .unwrap_or("");
        assert_eq!(consequent_ft, "Task_has_Task_Readiness",
            "consequentFactTypeId must resolve to the canonical FT id of the consequent reading; \
             got '{}'. Rule text: {:?}",
            consequent_ft, binding(&rule_seq[0], "text"));
    }

    /// Stage-2 must populate `consequentFactTypeId` for derivation rules
    /// whose consequent uses a SUBSCRIPTED role-noun token (`Task1`,
    /// `Task2`, …). Without subscript stripping in
    /// `resolve_consequent_fact_type_id`, the consequent text
    /// `Task1 has Task Readiness 'blocked'` fails to prefix-match the
    /// FT reading `Task has Task Readiness` because `Task1` doesn't
    /// equal `Task`. Result: empty consequentFactTypeId, so
    /// `derivation_index:{noun}` skips the rule (no FT to harvest
    /// nouns from), the apply-path noun-gating filter excludes it,
    /// and ring-FT join derivations silently never fire on per-call
    /// apply. apps/tasks's `Task1 has Task Readiness 'blocked' iff
    /// Task2 blocks Task1 and Task2 has Task Status 'pending'` is
    /// the canonical case.
    #[test]
    fn stage12_pipeline_resolves_consequent_fact_type_id_for_subscripted_role_token() {
        let src = "\
            Task is an entity type.\n\
            Task Status is a value type.\n\
            Task Readiness is a value type.\n\
            Task has Task Status.\n\
            Task has Task Readiness.\n\
            Task blocks Task.\n\
            Task1 has Task Readiness 'blocked' iff Task2 blocks Task1 and Task2 has Task Status 'pending'.\n\
        ";
        let state = super::parse_to_state_via_stage12(src)
            .expect("parse_to_state_via_stage12");
        let rules = fetch_or_phi("DerivationRule", &state);
        let rule_seq = rules.as_seq()
            .expect("DerivationRule cell must be a Seq");
        assert_eq!(rule_seq.len(), 1,
            "exactly one derivation rule expected; got {}", rule_seq.len());
        let consequent_ft = binding(&rule_seq[0], "consequentFactTypeId")
            .unwrap_or("");
        assert_eq!(consequent_ft, "Task_has_Task_Readiness",
            "consequentFactTypeId must resolve to `Task_has_Task_Readiness` even when the \
             consequent's role-noun token (`Task1`) carries a numeric subscript; \
             got '{}'. Rule text: {:?}",
            consequent_ft, binding(&rule_seq[0], "text"));
    }

    /// #831 / cor:closure — Closure Under Self-Modification (AREST.tex
    /// Corollary 6 + Migration Remark): the compile op is a SYSTEM
    /// application that PRESERVES P. New readings extend D; they don't
    /// reset it. `platform_compile` (ast.rs:2683) already does
    /// `merge_states(d, &parsed)` for this reason. The canonical CLI
    /// entry path (cli/entry.rs:491) folds from `Object::phi()` and
    /// silently throws away apply-written FT cells on every recompile,
    /// which is what apps/tasks #831 is asking to fix.
    ///
    /// This test pins the engine-level invariant the fix relies on:
    /// parsing schema-only readings against a prior state with
    /// apply-written FT cells must, after merge, still contain those
    /// cells verbatim.
    #[test]
    fn parse_then_merge_preserves_prior_population_when_readings_have_no_instance_facts() {
        let mut prior = crate::ast::Object::phi();
        prior = crate::ast::cell_push(
            "Task_has_Task_Status",
            crate::ast::fact_from_pairs(&[("Task", "1"), ("Task Status", "completed")]),
            &prior,
        );
        let schema_only = "\
            Task is an entity type.\n\
            Task Status is a value type.\n\
            Task has Task Status.\n\
        ";
        let parsed = super::parse_to_state_via_stage12_with_context(schema_only, &prior)
            .expect("schema-only readings must parse against a prior state");
        let merged = crate::ast::merge_states(&prior, &parsed);

        let cell = crate::ast::fetch_or_phi("Task_has_Task_Status", &merged);
        let entries: Vec<&crate::ast::Object> = cell.as_seq()
            .map(|s| s.iter().collect()).unwrap_or_default();
        assert_eq!(entries.len(), 1,
            "prior apply-written fact must survive parse+merge \
             (Closure Under Self-Modification, AREST.tex Cor.\u{00a0}6); \
             got {} entries", entries.len());
        assert!(crate::ast::binding(entries[0], "Task Status") == Some("completed"),
            "the surviving fact must carry the prior apply-written value `completed`, \
             not be replaced by a default; got {:?}", entries[0]);
    }

    // ── Perf Benchmarks (opt-in) ───────────────────────────────────────
    //
    // Kept out of the default suite by `#[ignore]`. Run with:
    //
    //   cargo test --lib --features std-deps \
    //       bench_forward_chain_over_grammar_rules -- --ignored --nocapture
    //
    // The chainer owns most of Stage-2's call-time cost; optimisation
    // passes on `forward_chain_defs_state_semi_naive_*` (see #297 for
    // history) want a focused signal without the surrounding parse,
    // compile, and translator noise. Numbers are printed to stderr
    // (stable run-to-run within ~15% on the same machine).

    /// Benchmark `forward_chain_defs_state_semi_naive_with_base_keys`
    /// against the cached FORML 2 grammar classifier rule set.
    ///
    /// Fixture choice: 10 hand-rolled canonical FORML 2 statement
    /// shapes, tokenized via Stage-1 and replicated 10× (100 Statements
    /// total). Self-contained vs. loading `readings/core.md` — both
    /// fixtures are permitted by handoff-297; the hand-rolled form
    /// keeps the bench deterministic and free of readings I/O so
    /// run-to-run variance reflects the chainer, not disk or parser
    /// noise.
    #[ignore = "perf benchmark; run with --ignored --nocapture"]
    #[test]
    fn bench_forward_chain_over_grammar_rules() {
        use alloc::collections::BTreeSet;

        // 1. Warm the grammar cache. First call builds `GRAMMAR_CACHE`;
        //    `cached_grammar()` below then hits the warm cache.
        let _ = parse_to_state_via_stage12("Customer is an entity type.")
            .expect("grammar warm-up parse must succeed");
        let (grammar_state, classifier_defs, classifier_antecedents, base_keys) =
            cached_grammar().expect("grammar cache must be populated");

        // 2. Synthetic statement state: 10 shapes × 10 instantiations.
        //    Each instance gets a unique `s{n}` id so the chainer sees
        //    100 distinct Statement facts.
        let shapes: &[(&str, &[&str])] = &[
            ("Customer is an entity type.",              &["Customer"]),
            ("Priority is a value type.",                &["Priority"]),
            ("Request is abstract.",                     &["Request"]),
            ("Support Request is a subtype of Request.", &["Support Request", "Request"]),
            ("Customer places Order.",                   &["Customer", "Order"]),
            ("Order has Status.",                        &["Order", "Status"]),
            ("Customer has Full Name *.",                &["Customer", "Full Name"]),
            ("Each Customer places at most one Order.",  &["Customer", "Order"]),
            ("Category has parent Category is acyclic.", &["Category"]),
            ("Person loves Person is symmetric.",        &["Person"]),
        ];
        const STMT_COUNT: usize = 100;
        let mut stmt_cells: HashMap<String, Vec<Object>> = HashMap::new();
        let mut noun_set: BTreeSet<String> = BTreeSet::new();
        for (i, (text, nouns)) in shapes.iter().cycle().take(STMT_COUNT).enumerate() {
            let sid = format!("s{}", i);
            let owned: Vec<String> = nouns.iter().map(|s| s.to_string()).collect();
            for (k, v) in tokenize_statement(&sid, text, &owned) {
                stmt_cells.entry(k).or_default().extend(v);
            }
            noun_set.extend(owned);
        }
        // Seed the Noun cell to match `stage1_state`'s pattern —
        // classifier rules don't read it directly, but it nudges the
        // merged state closer to real workload shape.
        let noun_facts: Vec<Object> = noun_set.iter()
            .map(|n| fact_from_pairs(&[("name", n.as_str()), ("objectType", "entity")]))
            .collect();
        stmt_cells.insert("Noun".to_string(), noun_facts);
        let stmt_state = Object::Map(stmt_cells.into_iter()
            .map(|(k, v)| (k, Object::Seq(v.into())))
            .collect());

        // 3. Merge with cached grammar — same shape `classify_statements`
        //    builds internally.
        let merged = crate::ast::merge_states(&stmt_state, grammar_state);

        // (&name, &func, Some(&antecedent_cells)) slice the semi-naive
        // chainer wants.
        let deriv: Vec<(&str, &crate::ast::Func, Option<&[String]>)> = classifier_defs.iter()
            .zip(classifier_antecedents.iter())
            .map(|((n, f), a)| (n.as_str(), f, Some(a.as_slice())))
            .collect();

        // base_keys for the merged state: cached grammar keys plus
        // statement-side keys. Matches `classify_statements` wiring —
        // skipping this would have the chainer re-hash ~4000 grammar
        // facts per iteration, masking the signal we want to measure.
        let stmt_keys = crate::evaluate::state_keys(&stmt_state);
        let mut combined_keys = base_keys.clone();
        combined_keys.extend(stmt_keys.into_iter());

        // Round-2 active-def count: with a single round-1 write to
        // `Statement_has_Classification`, only rules whose antecedents
        // list that cell survive the semi-naive filter in round 2.
        let round2_active = classifier_antecedents.iter()
            .filter(|cells| cells.iter().any(|c| c == "Statement_has_Classification"))
            .count();

        // 4. Hot loop.
        const N: usize = 50;
        let t0 = Instant::now();
        let mut last_derived = 0usize;
        for _ in 0..N {
            let (_, derived) = crate::evaluate::forward_chain_defs_state_semi_naive_with_base_keys(
                &deriv, &merged, 2, Some(combined_keys.clone()));
            last_derived = derived.len();
        }
        let elapsed = t0.elapsed();

        // 5. Perf report (stderr via eprintln so `--nocapture` prints it).
        let mean_ns = elapsed.as_nanos() as f64 / N as f64;
        eprintln!(
            "bench_forward_chain_over_grammar_rules: \
             {} iters over {} statements in {:?} | \
             mean {:.3} ms/call ({:.0} ns) | \
             {} candidates derived per call | \
             active defs: round 1 = {}, round 2 = {}",
            N, STMT_COUNT, elapsed,
            mean_ns / 1_000_000.0, mean_ns,
            last_derived,
            deriv.len(), round2_active);
    }

    // ─── #713 / MC2: cell-driven dispatch table tests ─────────────────
    //
    // These tests assert Stage-2 reads its three dispatch matrices
    // (ring kinds, conditional ring, deontic shape) from a
    // `RingKindTable` / `ConditionalRingMatrix` / `DeonticShapeTable`
    // built off the EnumValues cell. They use a synthetic state object
    // built directly in the test, so they verify the cell-reading path
    // independent of `cached_grammar_state`.

    fn synthetic_enum_state(enums: &[(&str, &[&str])]) -> Object {
        use alloc::sync::Arc;
        let facts: Vec<Object> = enums.iter().map(|(noun, vals)| {
            let mut pairs: Vec<(String, String)> = alloc::vec![
                ("noun".to_string(), (*noun).to_string()),
            ];
            for (i, v) in vals.iter().enumerate() {
                pairs.push((alloc::format!("value{i}"), (*v).to_string()));
            }
            let refs: Vec<(&str, &str)> = pairs.iter()
                .map(|(k, v)| (k.as_str(), v.as_str())).collect();
            fact_from_pairs(&refs)
        }).collect();
        let mut map: HashMap<String, Object> = HashMap::new();
        map.insert("EnumValues".to_string(), Object::Seq(Arc::from(facts)));
        Object::Map(map)
    }

    #[test]
    fn ring_kind_table_from_grammar_state_reads_parallel_enums() {
        // The test the audit calls out: Stage-2 recognizes 'is acyclic'
        // as a ring kind via the cell path. Build a table whose markers
        // come from a synthetic EnumValues cell, not from `boot()`,
        // and verify `kind_for("is acyclic")` returns "AC".
        let state = synthetic_enum_state(&[
            ("Ring Constraint Trailing Marker", &["is acyclic", "is symmetric"]),
            ("Ring Constraint Kind Code", &["AC", "SY"]),
        ]);
        let table = super::RingKindTable::from_grammar_state(&state);
        assert_eq!(table.kind_for("is acyclic"), Some("AC"));
        assert_eq!(table.kind_for("is symmetric"), Some("SY"));
        assert_eq!(table.kind_for("is irreflexive"), None,
            "missing-from-cells marker must not resolve");
    }

    #[test]
    fn ring_kind_table_falls_back_to_boot_on_length_mismatch() {
        // If the parallel enums have different lengths, the reader
        // refuses to zip them and falls back to `boot()`.
        let state = synthetic_enum_state(&[
            ("Ring Constraint Trailing Marker", &["is acyclic"]),
            ("Ring Constraint Kind Code", &["AC", "SY"]),
        ]);
        let table = super::RingKindTable::from_grammar_state(&state);
        // boot() includes all 8 markers — assert the irreflexive entry
        // we did NOT add appears (proving fallback fired).
        assert_eq!(table.kind_for("is irreflexive"), Some("IR"));
    }

    #[test]
    fn translate_ring_constraints_with_cell_table_emits_expected_kinds() {
        // End-to-end: synthesize the parallel enum cell, build the
        // table, and run `translate_ring_constraints_with_tables` —
        // every kind must come from the cell-driven table.
        let state = synthetic_enum_state(&[
            ("Ring Constraint Trailing Marker", &[
                "is irreflexive", "is asymmetric", "is antisymmetric",
                "is symmetric", "is intransitive", "is transitive",
                "is acyclic", "is reflexive",
            ]),
            ("Ring Constraint Kind Code", &[
                "IR", "AS", "AT", "SY", "IT", "TR", "AC", "RF",
            ]),
        ]);
        let ring_kinds = super::RingKindTable::from_grammar_state(&state);
        let conditional_matrix = super::ConditionalRingMatrix::boot();
        for (text, nouns, expected_kind) in [
            ("Category has parent Category is acyclic.",    vec!["Category"], "AC"),
            ("Person is parent of Person is irreflexive.",  vec!["Person"],   "IR"),
            ("Person loves Person is symmetric.",           vec!["Person"],   "SY"),
        ] {
            let stmt = stage1_state("s1", text, &nouns);
            let classified = classify_statements(&stmt, &grammar_state());
            let constraints = super::translate_ring_constraints_with_tables(
                &classified, &idx(&classified), &ring_kinds, &conditional_matrix);
            assert_eq!(constraints.len(), 1, "text={:?}", text);
            assert_eq!(binding(&constraints[0], "kind"), Some(expected_kind),
                "text={:?}", text);
        }
    }

    #[test]
    fn ring_kind_table_from_real_grammar_loads_eight_markers() {
        // The committed grammar must declare both parallel enums so
        // the cell path returns the same 8-row table as `boot()`.
        let state = grammar_state();
        let table = super::RingKindTable::from_grammar_state(&state);
        assert_eq!(table.markers.len(), 8,
            "expected 8 ring-kind rows from real grammar; got {:?}",
            table.markers);
        assert_eq!(table.kind_for("is acyclic"), Some("AC"));
        assert_eq!(table.kind_for("is asymmetric"), Some("AS"));
        assert_eq!(table.kind_for("is antisymmetric"), Some("AT"));
        assert_eq!(table.kind_for("is intransitive"), Some("IT"));
        assert_eq!(table.kind_for("is irreflexive"), Some("IR"));
        assert_eq!(table.kind_for("is reflexive"), Some("RF"));
        assert_eq!(table.kind_for("is symmetric"), Some("SY"));
        assert_eq!(table.kind_for("is transitive"), Some("TR"));
    }

    #[test]
    fn conditional_ring_matrix_from_grammar_state_reads_parallel_enums() {
        let state = synthetic_enum_state(&[
            ("Conditional Ring Pattern", &["plain", "and"]),
            ("Conditional Ring Kind Code", &["SY", "TR"]),
        ]);
        let matrix = super::ConditionalRingMatrix::from_grammar_state(&state);
        assert_eq!(matrix.kind_for("plain"), Some("SY"));
        assert_eq!(matrix.kind_for("and"), Some("TR"));
        assert_eq!(matrix.kind_for("impossible"), None,
            "pattern not in cells must not resolve");
    }

    #[test]
    fn conditional_ring_matrix_from_real_grammar_loads_seven_rows() {
        let state = grammar_state();
        let matrix = super::ConditionalRingMatrix::from_grammar_state(&state);
        assert_eq!(matrix.rows.len(), 7,
            "expected 7 conditional-ring rows; got {:?}", matrix.rows);
        assert_eq!(matrix.kind_for("plain"),                     Some("SY"));
        assert_eq!(matrix.kind_for("and"),                       Some("TR"));
        assert_eq!(matrix.kind_for("and+impossible"),            Some("IT"));
        assert_eq!(matrix.kind_for("and+impossible+isnot-ante"), Some("AT"));
        assert_eq!(matrix.kind_for("impossible"),                Some("AS"));
        assert_eq!(matrix.kind_for("isnot-conse"),               Some("AS"));
        assert_eq!(matrix.kind_for("itself-conse"),              Some("RF"));
    }

    #[test]
    fn deontic_shape_table_from_grammar_state_reads_three_parallel_enums() {
        let state = synthetic_enum_state(&[
            ("Deontic Operator", &["obligatory", "forbidden", "permitted"]),
            ("Deontic Constraint Kind Code", &["UC", "UC", "UC"]),
            ("Deontic Constraint Modality", &["deontic", "deontic", "deontic"]),
        ]);
        let table = super::DeonticShapeTable::from_grammar_state(&state);
        assert_eq!(table.shape_for("obligatory"), Some(("UC", "deontic")));
        assert_eq!(table.shape_for("forbidden"),  Some(("UC", "deontic")));
        assert_eq!(table.shape_for("permitted"),  Some(("UC", "deontic")));
        assert_eq!(table.shape_for("encouraged"), None);
    }

    #[test]
    fn deontic_shape_table_falls_back_to_boot_on_length_mismatch() {
        // Parallel enum lengths differ → boot fallback. Boot has all
        // three operators mapped to (UC, deontic).
        let state = synthetic_enum_state(&[
            ("Deontic Operator", &["obligatory"]),
            ("Deontic Constraint Kind Code", &["UC", "UC", "UC"]),
            ("Deontic Constraint Modality", &["deontic"]),
        ]);
        let table = super::DeonticShapeTable::from_grammar_state(&state);
        assert_eq!(table.shape_for("forbidden"), Some(("UC", "deontic")),
            "boot fallback must include all three operators");
    }

    #[test]
    fn translate_deontic_constraints_with_cell_table_emits_expected_shape() {
        let state = synthetic_enum_state(&[
            ("Deontic Operator", &["obligatory", "forbidden", "permitted"]),
            ("Deontic Constraint Kind Code", &["UC", "UC", "UC"]),
            ("Deontic Constraint Modality", &["deontic", "deontic", "deontic"]),
        ]);
        let table = super::DeonticShapeTable::from_grammar_state(&state);
        let stmt = stage1_state(
            "s1",
            "It is obligatory that Customer places Order.",
            &["Customer", "Order"]);
        let classified = classify_statements(&stmt, &grammar_state());
        let constraints = super::translate_deontic_constraints_with_table(
            &classified, &idx(&classified), &table);
        assert_eq!(constraints.len(), 1);
        let f = &constraints[0];
        assert_eq!(binding(f, "kind"), Some("UC"));
        assert_eq!(binding(f, "modality"), Some("deontic"));
        assert_eq!(binding(f, "deonticOperator"), Some("obligatory"));
    }

    #[test]
    fn deontic_shape_table_from_real_grammar_loads_three_operators() {
        let state = grammar_state();
        let table = super::DeonticShapeTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 3,
            "expected 3 deontic operator rows; got {:?}", table.rows);
        assert_eq!(table.shape_for("obligatory"), Some(("UC", "deontic")));
        assert_eq!(table.shape_for("forbidden"),  Some(("UC", "deontic")));
        assert_eq!(table.shape_for("permitted"),  Some(("UC", "deontic")));
    }

    // ─── #833 layer 9: cardinality arbitration registry ───────────────

    #[test]
    fn cardinality_arbitration_registry_has_two_predicates() {
        let r = super::cardinality_arbitration_registry();
        assert!(r.contains_key("derivation_rule_wins"),
            "missing derivation_rule_wins arbitration predicate");
        assert!(r.contains_key("deontic_operator_wins"),
            "missing deontic_operator_wins arbitration predicate");
        assert_eq!(r.len(), 2,
            "expected exactly 2 cardinality arbitration predicates; got {}",
            r.len());
    }

    // ─── #833 layer 7: translator-name → fn pointer registry ─────────

    #[test]
    fn statement_translator_table_translator_names_resolve_to_functions() {
        // Per AREST.tex §3.2: registered functions span the platform
        // layer. Every translator name registered in
        // StatementTranslatorTable must resolve to a Rust function
        // pointer (or be a known-exception, e.g. translate_fact_types
        // which returns a tuple and lives outside the uniform registry).
        // This catches typos / renames between the grammar readings
        // and the Rust pipeline.
        let table = super::StatementTranslatorTable::boot();
        let registry = super::translator_function_registry();
        let known_exceptions: &[&str] = &[
            // translate_fact_types: (Vec<Object>, Vec<Object>) signature.
            "translate_fact_types",
        ];
        let mut translator_names: Vec<&str> = table.rows.iter()
            .map(|(_, t)| t.as_str())
            .collect();
        translator_names.sort();
        translator_names.dedup();
        for name in translator_names {
            if known_exceptions.contains(&name) { continue; }
            assert!(registry.contains_key(name),
                "translator {name:?} registered in StatementTranslatorTable \
                 but missing from translator_function_registry; \
                 either add the fn pointer to the registry or rename \
                 the readings instance fact to match the Rust fn");
        }
    }

    #[test]
    fn translator_function_registry_keys_are_valid_translators() {
        // Reverse direction: every fn pointer in the registry must
        // resolve to at least one Classification kind in the table.
        // An orphan fn pointer means the readings forgot to declare
        // any kind that dispatches to it.
        let table = super::StatementTranslatorTable::boot();
        let registry = super::translator_function_registry();
        for name in registry.keys() {
            let kinds = table.kinds_for(name);
            assert!(!kinds.is_empty(),
                "fn pointer {name:?} is registered but no Classification \
                 kind in StatementTranslatorTable dispatches to it");
        }
    }

    // ─── #833 layer 6: object-type kind table (translate_nouns) ───────

    #[test]
    fn object_type_kind_table_boot_has_five_kinds_in_abstract_first_order() {
        let table = super::ObjectTypeKindTable::boot();
        let pairs: Vec<(&str, &str)> = table.iter().collect();
        assert_eq!(pairs, vec![
            ("Abstract Declaration", "abstract"),
            ("Partition Declaration", "abstract"),
            ("Entity Type Declaration", "entity"),
            ("Value Type Declaration", "value"),
            ("Subtype Declaration", "entity"),
        ]);
    }

    #[test]
    fn object_type_kind_table_from_grammar_state_reads_parallel_enums() {
        let state = synthetic_enum_state(&[
            ("Object Type Source Kind", &[
                "Abstract Declaration", "Partition Declaration",
                "Entity Type Declaration", "Value Type Declaration",
                "Subtype Declaration"]),
            ("Object Type", &["abstract", "abstract", "entity", "value", "entity"]),
        ]);
        let table = super::ObjectTypeKindTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 5);
    }

    #[test]
    fn object_type_kind_table_falls_back_to_boot_on_length_mismatch() {
        let state = synthetic_enum_state(&[
            ("Object Type Source Kind", &["Entity Type Declaration"]),
            ("Object Type", &["entity", "value"]),
        ]);
        let table = super::ObjectTypeKindTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 5);
    }

    #[test]
    fn object_type_kind_table_from_real_grammar_loads_five_rows() {
        let state = grammar_state();
        let table = super::ObjectTypeKindTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 5);
    }

    // ─── #789: prose stopword table ──────────────────────────────────

    /// #789 — the 12 title-case prose tokens that `unresolved_subclauses`
    /// silently filters when scanning for unknown nouns must come from
    /// a typed table (and ultimately the grammar) instead of an inline
    /// const. Asserts the boot table carries every word the previous
    /// const enumerated, in the same declaration order so existing
    /// behavior round-trips without surprise.
    #[test]
    fn prose_stopword_table_boot_has_twelve_words_in_declared_order() {
        let table = super::ProseStopwordTable::boot();
        let words: Vec<&str> = table.iter().collect();
        assert_eq!(words, vec![
            "If", "When", "Then", "That", "This", "An", "A", "The",
            "Each", "Some", "No", "Every",
        ],
        "boot table must mirror the historic PROSE_STOPWORDS const, \
         in declaration order, so unresolved_subclauses behaves \
         identically before and after the lift.");
    }

    /// #789 — `contains` is case-sensitive and matches whole words,
    /// matching the original `PROSE_STOPWORDS.iter().any(|s| *s == *w)`
    /// semantics. "Each" matches; "each" doesn't (lowercase reaches a
    /// different code path); "Eachling" doesn't (would need substring).
    #[test]
    fn prose_stopword_table_contains_is_case_sensitive_whole_word() {
        let table = super::ProseStopwordTable::boot();
        assert!(table.contains("Each"),
            "title-case Each is in the table");
        assert!(!table.contains("each"),
            "lowercase each is NOT a prose stopword (would shadow Quantifier 'each')");
        assert!(!table.contains("Eachling"),
            "substring matches are not allowed");
        assert!(!table.contains(""),
            "empty string is not a stopword");
    }

    /// #789 — when state's EnumValues cell carries Prose Stopword
    /// values, from_grammar_state lifts them at runtime. Empty cell
    /// falls back to boot() so the bare-engine path keeps working.
    #[test]
    fn prose_stopword_table_from_grammar_state_reads_enum_values() {
        let state = synthetic_enum_state(&[
            ("Prose Stopword", &["Foo", "Bar", "Baz"]),
        ]);
        let table = super::ProseStopwordTable::from_grammar_state(&state);
        assert_eq!(table.rows, vec!["Foo", "Bar", "Baz"]);
    }

    #[test]
    fn prose_stopword_table_falls_back_to_boot_on_empty_state() {
        let state = synthetic_enum_state(&[]);
        let table = super::ProseStopwordTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 12,
            "empty grammar state falls back to the 12-word boot table");
    }

    // ─── #791: ring adjective table ──────────────────────────────────

    /// #791 — `strip_ring_annotation` filters trailing `(<adjective>)`
    /// annotations on ring-constraint conditional shapes. The 8-token
    /// closed vocabulary (irreflexive, asymmetric, antisymmetric,
    /// symmetric, intransitive, transitive, acyclic, reflexive) lived
    /// in an inline const; this lift moves it to a typed
    /// `RingAdjectiveTable` reading from the `Ring Adjective` grammar
    /// enum. Boot table preserves declaration order so behavior round-
    /// trips. Mirrors `RingKindTable` (same eight kinds, but the bare
    /// adjective form rather than the `is X` trailing marker).
    #[test]
    fn ring_adjective_table_boot_has_eight_adjectives_in_declared_order() {
        let table = super::RingAdjectiveTable::boot();
        let words: Vec<&str> = table.iter().collect();
        assert_eq!(words, vec![
            "irreflexive", "asymmetric", "antisymmetric", "symmetric",
            "intransitive", "transitive", "acyclic", "reflexive",
        ],
        "boot table must mirror the historic `KINDS` const in \
         strip_ring_annotation, in declaration order, so that \
         conditional ring-shape annotation parsing behaves identically \
         before and after the lift.");
    }

    /// #791 — `contains` is case-sensitive whole-word, matching the
    /// original `KINDS.iter().any(|k| *k == kind)` semantics.
    #[test]
    fn ring_adjective_table_contains_is_case_sensitive_whole_word() {
        let table = super::RingAdjectiveTable::boot();
        assert!(table.contains("symmetric"),
            "lowercase symmetric is the canonical declaration form");
        assert!(!table.contains("Symmetric"),
            "title-case is NOT a ring adjective annotation");
        assert!(!table.contains("symmetrical"),
            "substring matches are not allowed");
        assert!(!table.contains(""),
            "empty string is not a ring adjective");
    }

    /// #791 — when state's EnumValues cell carries Ring Adjective
    /// values, from_grammar_state lifts them at runtime. Empty cell
    /// falls back to boot() so the bare-engine path keeps working.
    #[test]
    fn ring_adjective_table_from_grammar_state_reads_enum_values() {
        let state = synthetic_enum_state(&[
            ("Ring Adjective", &["foo", "bar", "baz"]),
        ]);
        let table = super::RingAdjectiveTable::from_grammar_state(&state);
        assert_eq!(table.rows, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn ring_adjective_table_falls_back_to_boot_on_empty_state() {
        let state = synthetic_enum_state(&[]);
        let table = super::RingAdjectiveTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 8,
            "empty grammar state falls back to the 8-adjective boot table");
    }

    // ─── #790: constraint span prefix table ──────────────────────────

    /// #790 — `resolve_constraint_span_ft` strips deontic / quantifier
    /// prefixes via a `.replace(...).replace(...)` cascade. Eleven
    /// prefixes in three cohorts (deontic, distributive, cardinal/
    /// existential/negative). Lift mirrors `ProseStopwordTable` /
    /// `RingAdjectiveTable`. Boot order matches the legacy cascade.
    #[test]
    fn constraint_span_prefix_table_boot_has_eleven_prefixes_in_cascade_order() {
        let table = super::ConstraintSpanPrefixTable::boot();
        let prefixes: Vec<&str> = table.iter().collect();
        assert_eq!(prefixes, vec![
            "It is obligatory that ",
            "It is forbidden that ",
            "It is permitted that ",
            "Each ", "each ",
            "at most one ", "exactly one ",
            "at least one ", "some ",
            "No ", "no ",
        ],
        "boot table must mirror legacy resolve_constraint_schema cascade \
         order so constraint-span FT resolution behaves identically before \
         and after the lift.");
    }

    /// #790 — `strip_all` preserves `.replace` semantics (substring
    /// replace anywhere, not just at start). Each prefix is removed
    /// wherever it occurs in the input.
    #[test]
    fn constraint_span_prefix_table_strip_all_replaces_substrings_in_order() {
        let table = super::ConstraintSpanPrefixTable::boot();
        assert_eq!(
            table.strip_all("Each Foo has at most one Bar"),
            "Foo has Bar",
            "leading Each + interior 'at most one ' both stripped");
        assert_eq!(
            table.strip_all("It is obligatory that some Foo bars Baz"),
            "Foo bars Baz",
            "deontic + existential prefixes both stripped");
        assert_eq!(
            table.strip_all("Foo bars Baz"),
            "Foo bars Baz",
            "no-prefix input round-trips unchanged");
    }

    /// #790 — when state's EnumValues cell carries Constraint Span
    /// Prefix values, from_grammar_state lifts them at runtime.
    #[test]
    fn constraint_span_prefix_table_from_grammar_state_reads_enum_values() {
        let state = synthetic_enum_state(&[
            ("Constraint Span Prefix", &["foo ", "bar "]),
        ]);
        let table = super::ConstraintSpanPrefixTable::from_grammar_state(&state);
        assert_eq!(table.rows, vec!["foo ", "bar "]);
    }

    #[test]
    fn constraint_span_prefix_table_falls_back_to_boot_on_empty_state() {
        let state = synthetic_enum_state(&[]);
        let table = super::ConstraintSpanPrefixTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 11,
            "empty grammar state falls back to the 11-prefix boot table");
    }

    // ─── #783 first slice: word comparator table ─────────────────────

    /// #783 — `is_word_comparator_clause` in parse_forml2.rs has an
    /// 8-entry inline `COMPARATORS` const (` exceeds `, ` is greater
    /// than `, ...). First slice of the Sweep-1c migration: lift the
    /// const to a typed `WordComparatorTable` reading from the
    /// `Word Comparator` grammar enum. Boot table preserves the
    /// declaration order so the iteration semantics — first matching
    /// keyword wins, both sides must reference declared nouns — round-
    /// trip without surprise.
    #[test]
    fn word_comparator_table_boot_has_eight_comparators_in_declared_order() {
        let table = super::WordComparatorTable::boot();
        let words: Vec<&str> = table.iter().collect();
        assert_eq!(words, vec![
            "exceeds", "is greater than", "is less than",
            "is at least", "is at most", "is more than",
            "equals", "is equal to",
        ],
        "boot table must mirror the historic COMPARATORS const, in \
         declaration order, so is_word_comparator_clause returns the \
         same result for every input.");
    }

    #[test]
    fn word_comparator_table_from_grammar_state_reads_enum_values() {
        let state = synthetic_enum_state(&[
            ("Word Comparator", &["foo", "bar baz"]),
        ]);
        let table = super::WordComparatorTable::from_grammar_state(&state);
        assert_eq!(table.rows, vec!["foo", "bar baz"]);
    }

    #[test]
    fn word_comparator_table_falls_back_to_boot_on_empty_state() {
        let state = synthetic_enum_state(&[]);
        let table = super::WordComparatorTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 8,
            "empty grammar state falls back to the 8-comparator boot table");
    }

    // ─── #783 second slice: range operator table ─────────────────────

    /// #783 — `is_range_filter_clause` in parse_forml2.rs has a 3-entry
    /// inline `RANGE_OPS` const (` within `, ` before `, ` after `).
    /// Second slice of the Sweep-1c migration: lift the const to a
    /// typed `RangeOperatorTable` reading from the `Range Operator`
    /// grammar enum. Boot stays in declaration order so the first-
    /// match-wins iteration semantics round-trip.
    #[test]
    fn range_operator_table_boot_has_three_operators_in_declared_order() {
        let table = super::RangeOperatorTable::boot();
        let words: Vec<&str> = table.iter().collect();
        assert_eq!(words, vec!["within", "before", "after"],
            "boot table must mirror the historic RANGE_OPS const, in \
             declaration order, so is_range_filter_clause returns the \
             same result for every input.");
    }

    #[test]
    fn range_operator_table_from_grammar_state_reads_enum_values() {
        let state = synthetic_enum_state(&[
            ("Range Operator", &["foo", "bar"]),
        ]);
        let table = super::RangeOperatorTable::from_grammar_state(&state);
        assert_eq!(table.rows, vec!["foo", "bar"]);
    }

    #[test]
    fn range_operator_table_falls_back_to_boot_on_empty_state() {
        let state = synthetic_enum_state(&[]);
        let table = super::RangeOperatorTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 3,
            "empty grammar state falls back to the 3-operator boot table");
    }

    // ─── #844 / readings-as-source-code: quote escape table ──────────

    /// Stage-1's `extract_following_literal_span` historically used
    /// `body.find('\'')` to locate the close-quote of a single-quoted
    /// literal — naive scan with no escape handling. That meant a
    /// description like `'doesn''t work'` was truncated to `doesn`.
    /// Lift the literal-escape convention to a typed `QuoteEscapeTable`
    /// declared as the `Quote Escape` value type in
    /// `readings/forml2-grammar.md`. Boot enables `'doubled-quote'`
    /// (the `''` → `'` SQL convention). Mirrors the Sweep-1 dispatch-
    /// to-data pattern: the parser dispatches to the data, the data
    /// lives in the grammar reading.
    #[test]
    fn quote_escape_table_boot_has_doubled_quote_convention() {
        let table = super::QuoteEscapeTable::boot();
        let conventions: Vec<&str> = table.iter().collect();
        assert_eq!(conventions, vec!["doubled-quote"],
            "boot table enables the SQL-style `''` → `'` escape; \
             without this, instance facts can't carry apostrophes \
             in their literals.");
    }

    #[test]
    fn quote_escape_table_from_grammar_state_reads_enum_values() {
        let state = synthetic_enum_state(&[
            ("Quote Escape", &["doubled-quote", "backslash-quote"]),
        ]);
        let table = super::QuoteEscapeTable::from_grammar_state(&state);
        assert_eq!(table.rows, vec!["doubled-quote", "backslash-quote"]);
    }

    #[test]
    fn quote_escape_table_falls_back_to_boot_on_empty_state() {
        let state = synthetic_enum_state(&[]);
        let table = super::QuoteEscapeTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 1,
            "empty grammar state falls back to the single-convention boot table");
    }

    #[test]
    fn quote_escape_table_find_close_with_no_inner_quote() {
        let table = super::QuoteEscapeTable::boot();
        // body is the slice AFTER the opening `'` and includes the close `'`
        // followed by anything else. find_close returns the byte offset of
        // the close `'`.
        assert_eq!(table.find_close("simple'.").unwrap(), 6);
    }

    #[test]
    fn quote_escape_table_find_close_skips_doubled_quote() {
        let table = super::QuoteEscapeTable::boot();
        // `it''s late'.` — the `''` at offset 2 is a doubled-quote escape;
        // the real close is at offset 10.
        assert_eq!(table.find_close("it''s late'.").unwrap(), 10,
            "doubled-quote in body must NOT be treated as the close");
    }

    #[test]
    fn quote_escape_table_decode_collapses_doubled_quote() {
        let table = super::QuoteEscapeTable::boot();
        assert_eq!(table.decode("it''s late"), "it's late");
        assert_eq!(table.decode("no escapes"), "no escapes");
        assert_eq!(table.decode("''start"), "'start");
        assert_eq!(table.decode("end''"), "end'");
    }

    #[test]
    fn quote_escape_table_unterminated_returns_none() {
        let table = super::QuoteEscapeTable::boot();
        assert_eq!(table.find_close("no close quote here"), None);
    }

    // ─── #833 layer 5: set constraint kind table ──────────────────────

    #[test]
    fn set_constraint_kind_table_boot_has_five_kinds_in_cascade_order() {
        let table = super::SetConstraintKindTable::boot();
        assert_eq!(table.rows.len(), 5);
        let triples: Vec<(&str, &str, &str)> = table.iter().collect();
        assert_eq!(triples, vec![
            ("Equality Constraint",     "EQ", "derivation_rule_wins"),
            ("Subset Constraint",       "SS", "antecedent_diversity_min_2"),
            ("Exclusive-Or Constraint", "XO", "derivation_rule_wins"),
            ("Or Constraint",           "OR", "derivation_rule_wins"),
            ("Exclusion Constraint",    "XC", "derivation_rule_wins"),
        ]);
    }

    #[test]
    fn set_constraint_kind_table_from_grammar_state_reads_parallel_enums() {
        let state = synthetic_enum_state(&[
            ("Set Constraint Kind", &[
                "Equality Constraint", "Subset Constraint",
                "Exclusive-Or Constraint", "Or Constraint",
                "Exclusion Constraint"]),
            ("Set Constraint Kind Code", &["EQ", "SS", "XO", "OR", "XC"]),
            ("Set Constraint Arbitration Rule", &[
                "derivation_rule_wins", "antecedent_diversity_min_2",
                "derivation_rule_wins", "derivation_rule_wins",
                "derivation_rule_wins"]),
        ]);
        let table = super::SetConstraintKindTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 5);
    }

    // ─── #781: read_parallel_enum_triple ─────────────────────────────

    /// #781 — `read_parallel_enum_triple` mirrors the existing
    /// `read_parallel_enum_pair` for the 3-tuple case used by
    /// `SetConstraintKindTable::from_grammar_state`. Returns Some
    /// when all three lists exist, are equal-length, and non-empty;
    /// returns None otherwise so callers can fall back to boot().
    #[test]
    fn read_parallel_enum_triple_returns_zipped_when_lengths_match() {
        let state = synthetic_enum_state(&[
            ("First",  &["a", "b", "c"]),
            ("Second", &["A", "B", "C"]),
            ("Third",  &["1", "2", "3"]),
        ]);
        let triples = super::read_parallel_enum_triple(&state, "First", "Second", "Third");
        assert_eq!(triples, Some(vec![
            ("a".to_string(), "A".to_string(), "1".to_string()),
            ("b".to_string(), "B".to_string(), "2".to_string()),
            ("c".to_string(), "C".to_string(), "3".to_string()),
        ]));
    }

    #[test]
    fn read_parallel_enum_triple_returns_none_on_length_mismatch() {
        let state = synthetic_enum_state(&[
            ("First",  &["a", "b"]),
            ("Second", &["A", "B", "C"]),
            ("Third",  &["1", "2"]),
        ]);
        let triples = super::read_parallel_enum_triple(&state, "First", "Second", "Third");
        assert_eq!(triples, None,
            "mismatched lengths must return None so callers fall back to boot()");
    }

    #[test]
    fn read_parallel_enum_triple_returns_none_when_any_list_empty() {
        let state = synthetic_enum_state(&[
            ("First",  &["a"]),
            ("Second", &[]),
            ("Third",  &["1"]),
        ]);
        let triples = super::read_parallel_enum_triple(&state, "First", "Second", "Third");
        assert_eq!(triples, None,
            "any empty list means the parallel-enum declaration is incomplete; \
             return None so callers fall back to boot()");
    }

    // ─── #788: deontic predicate operator table ──────────────────────

    /// #788 — `parse_deontic_text_predicate` matches one of four
    /// suffixes on a deontic-constraint prefix (` ends with`,
    /// ` does not end with`, ` starts with`, ` does not start with`)
    /// and decodes (kind, negated). The 4-suffix cascade lifts to
    /// `DeonticPredicateOperatorTable` mirroring `SetConstraintKindTable`'s
    /// 3-tuple shape per #781.
    #[test]
    fn deontic_predicate_operator_table_boot_has_four_rows_in_match_order() {
        let table = super::DeonticPredicateOperatorTable::boot();
        let rows: Vec<(&str, &str, bool)> = table.iter().collect();
        assert_eq!(rows, vec![
            (" ends with",          "ends_with",   false),
            (" does not end with",  "ends_with",   true),
            (" starts with",        "starts_with", false),
            (" does not start with","starts_with", true),
        ],
        "boot table must mirror the historic strip_suffix cascade in \
         parse_deontic_text_predicate so behavior round-trips.");
    }

    /// #788 — `match_suffix` returns the (kind, negated) for the first
    /// matching suffix, mirroring strip_suffix cascade semantics.
    #[test]
    fn deontic_predicate_operator_table_match_suffix_finds_kind_and_negation() {
        let table = super::DeonticPredicateOperatorTable::boot();
        // Positive ends_with case.
        let (rest, kind, neg) = table.match_suffix("each Foo has a name that ends with")
            .expect("match");
        assert_eq!((rest, kind, neg),
            ("each Foo has a name that", "ends_with", false));
        // Negated ends_with case — must not match the shorter ` ends with` first.
        let (rest, kind, neg) = table.match_suffix("each Foo has a name that does not end with")
            .expect("match");
        assert_eq!((rest, kind, neg),
            ("each Foo has a name that", "ends_with", true));
        // starts_with cases.
        let (rest, kind, neg) = table.match_suffix("Foo has code that starts with")
            .expect("match");
        assert_eq!((rest, kind, neg),
            ("Foo has code that", "starts_with", false));
        let (rest, kind, neg) = table.match_suffix("Foo has code that does not start with")
            .expect("match");
        assert_eq!((rest, kind, neg),
            ("Foo has code that", "starts_with", true));
        // Non-match returns None.
        assert!(table.match_suffix("Foo has code that contains").is_none(),
            "unrecognised suffix yields None");
    }

    #[test]
    fn deontic_predicate_operator_table_from_grammar_state_reads_parallel_enums() {
        let state = synthetic_enum_state(&[
            ("Deontic Predicate Operator", &[
                " ends with", " does not end with",
                " starts with", " does not start with"]),
            ("Deontic Predicate Operator Kind", &[
                "ends_with", "ends_with",
                "starts_with", "starts_with"]),
            ("Deontic Predicate Operator Negated", &[
                "false", "true", "false", "true"]),
        ]);
        let table = super::DeonticPredicateOperatorTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 4);
    }

    #[test]
    fn deontic_predicate_operator_table_falls_back_to_boot_on_empty_state() {
        let state = synthetic_enum_state(&[]);
        let table = super::DeonticPredicateOperatorTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 4,
            "empty grammar state falls back to the 4-row boot table");
    }

    #[test]
    fn deontic_predicate_operator_table_from_real_grammar_loads_four_rows() {
        let state = grammar_state();
        let table = super::DeonticPredicateOperatorTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 4,
            "real forml2-grammar.md must declare all four operator/kind/negated rows");
        // Round-trip: real grammar declarations should produce the
        // same rows as boot() so behavior matches the legacy cascade.
        let boot_rows = super::DeonticPredicateOperatorTable::boot().rows;
        assert_eq!(table.rows, boot_rows,
            "grammar-loaded rows must match boot fallback exactly");
    }

    // ─── #786: conditional ring pattern table + non-canonical negation ───

    /// #786 — `encode_conditional_ring_pattern` has two inline
    /// vocabularies: a 10-hint NON_CANONICAL_NEGATION_HINTS const
    /// (negation prose that should fail closed instead of falling
    /// through to `plain`) and a 7-row 5-tuple match that dispatches
    /// pattern names. Both lift to typed tables.
    #[test]
    fn non_canonical_negation_hint_table_boot_has_ten_hints() {
        let table = super::NonCanonicalNegationHintTable::boot();
        let hints: Vec<&str> = table.iter().collect();
        assert_eq!(hints, vec![
            " does not ", " do not ", " did not ",
            " cannot ", " can not ", " must not ", " will not ", " would not ",
            " never ", " no longer ",
        ],
        "boot table must mirror the historic NON_CANONICAL_NEGATION_HINTS \
         const so encode_conditional_ring_pattern fails closed identically.");
    }

    #[test]
    fn non_canonical_negation_hint_table_any_match_returns_true_on_substring() {
        let table = super::NonCanonicalNegationHintTable::boot();
        assert!(table.any_match("Foo does not bar"),
            " does not  matches anywhere in the text");
        assert!(table.any_match("Foo never bar"),
            " never  matches anywhere in the text (with leading space)");
        assert!(!table.any_match("Foo bars"),
            "no negation hint present");
    }

    /// #786 — pattern dispatch is a 7-row table over 5 boolean
    /// predicates with wildcards. Each row's signal mask uses
    /// Option<bool>: Some(b) = required match, None = wildcard. The
    /// matcher iterates rows in declaration order; first row whose
    /// every Some(b) matches the input wins, mirroring the legacy
    /// match arm semantics.
    #[test]
    fn conditional_ring_pattern_table_boot_has_seven_rows_in_match_order() {
        let table = super::ConditionalRingPatternTable::boot();
        let names: Vec<&str> = table.rows.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(names, vec![
            "and+impossible+isnot-ante",
            "and+impossible",
            "and",
            "impossible",
            "isnot-conse",
            "itself-conse",
            "plain",
        ],
        "boot table row order must mirror the legacy match arm order so \
         the wildcard cascade behaves identically.");
    }

    #[test]
    fn conditional_ring_pattern_table_match_signals_returns_first_matching_row() {
        let table = super::ConditionalRingPatternTable::boot();
        // Row 1: (true, true, _, true, _) → "and+impossible+isnot-ante"
        assert_eq!(table.match_signals(true, true, false, true, false),
            Some("and+impossible+isnot-ante"));
        assert_eq!(table.match_signals(true, true, true, true, true),
            Some("and+impossible+isnot-ante"));
        // Row 3: (true, false, _, _, _) → "and"
        assert_eq!(table.match_signals(true, false, false, false, false),
            Some("and"));
        // Row 7: (false, false, false, _, false) → "plain"
        assert_eq!(table.match_signals(false, false, false, false, false),
            Some("plain"));
        assert_eq!(table.match_signals(false, false, false, true, false),
            Some("plain"),
            "row 7 wildcards on col 3 (is_not_in_antecedent)");
        // Row 5: (false, false, false, _, true) → "isnot-conse"
        // Note: row 5 comes BEFORE row 7 so true on col 4 wins
        assert_eq!(table.match_signals(false, false, false, false, true),
            Some("isnot-conse"));
        // Row 6: (false, false, true, _, _) → "itself-conse"
        assert_eq!(table.match_signals(false, false, true, false, false),
            Some("itself-conse"));
        // No row matches: e.g. (false, true, true, _, _) — impossible + itself
        // is not a recognised ring shape; original returns None via the wildcard.
        assert_eq!(table.match_signals(false, true, true, false, false), None,
            "impossible + itself_in_consequent is not a recognised ring shape");
    }

    #[test]
    fn set_constraint_kind_table_falls_back_to_boot_on_length_mismatch() {
        let state = synthetic_enum_state(&[
            ("Set Constraint Kind", &["Equality Constraint"]),
            ("Set Constraint Kind Code", &["EQ", "SS"]),
            ("Set Constraint Arbitration Rule", &["derivation_rule_wins"]),
        ]);
        let table = super::SetConstraintKindTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 5);
    }

    #[test]
    fn set_constraint_kind_table_from_real_grammar_loads_five_rows() {
        let state = grammar_state();
        let table = super::SetConstraintKindTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 5,
            "expected 5 set-constraint kind rows from real grammar; got {:?}",
            table.rows);
    }

    #[test]
    fn set_constraint_arbitration_registry_covers_kind_table() {
        // Every arbitration-rule name in SetConstraintKindTable must
        // resolve to a Rust function in the arbitration registry.
        // Catches typos / renames between readings and Rust code.
        let table = super::SetConstraintKindTable::boot();
        let registry = super::set_constraint_arbitration_registry();
        for (_kind, _code, rule) in table.iter() {
            assert!(registry.contains_key(rule),
                "arbitration rule {rule:?} declared in SetConstraintKindTable \
                 but missing from set_constraint_arbitration_registry");
        }
    }

    // ─── #833 layer 4: cardinality constraint kind table ──────────────

    #[test]
    fn cardinality_kind_table_boot_has_three_kinds_in_fc_uc_mc_order() {
        let table = super::CardinalityConstraintKindTable::boot();
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.rows[0].0, "Frequency Constraint");
        assert_eq!(table.rows[0].1, "FC");
        assert_eq!(table.rows[1].0, "Uniqueness Constraint");
        assert_eq!(table.rows[1].1, "UC");
        assert_eq!(table.rows[2].0, "Mandatory Role Constraint");
        assert_eq!(table.rows[2].1, "MC");
    }

    #[test]
    fn cardinality_kind_table_from_grammar_state_reads_parallel_enums() {
        let state = synthetic_enum_state(&[
            ("Cardinality Constraint Kind", &[
                "Frequency Constraint", "Uniqueness Constraint",
                "Mandatory Role Constraint"]),
            ("Cardinality Constraint Kind Code", &["FC", "UC", "MC"]),
        ]);
        let table = super::CardinalityConstraintKindTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 3);
    }

    #[test]
    fn cardinality_kind_table_falls_back_to_boot_on_length_mismatch() {
        let state = synthetic_enum_state(&[
            ("Cardinality Constraint Kind", &["Frequency Constraint"]),
            ("Cardinality Constraint Kind Code", &["FC", "UC"]),
        ]);
        let table = super::CardinalityConstraintKindTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 3, "boot fallback expected on mismatch");
    }

    #[test]
    fn cardinality_kind_table_from_real_grammar_loads_three_rows() {
        let state = grammar_state();
        let table = super::CardinalityConstraintKindTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 3,
            "expected 3 cardinality kind rows from real grammar; got {:?}",
            table.rows);
    }

    // ─── #833: Statement translator dispatch table ────────────────────

    fn synthetic_translator_state(rows: &[(&str, &str)]) -> Object {
        use alloc::sync::Arc;
        let facts: Vec<Object> = rows.iter().map(|(kind, translator)| {
            fact_from_pairs(&[
                ("Classification", *kind),
                ("Translator",     *translator),
            ])
        }).collect();
        let mut map: HashMap<String, Object> = HashMap::new();
        map.insert(
            "Classification_has_Translator".to_string(),
            Object::Seq(Arc::from(facts)));
        Object::Map(map)
    }

    #[test]
    fn statement_translator_table_boot_has_every_classification_kind() {
        // Per AREST.tex §3: registered translators span every kind
        // declared by the grammar's classification vocabulary. The
        // boot table is the Rust-side fallback, so it must enumerate
        // the same 20 classifications the grammar declares (19
        // structural + Fact Type Reading).
        let table = super::StatementTranslatorTable::boot();
        let kinds = table.kinds();
        let expected = [
            "Entity Type Declaration", "Value Type Declaration",
            "Subtype Declaration", "Abstract Declaration",
            "Partition Declaration", "Enum Values Declaration",
            "Instance Fact", "Fact Type Reading", "Derivation Rule",
            "Uniqueness Constraint", "Mandatory Role Constraint",
            "Frequency Constraint", "Ring Constraint", "Subset Constraint",
            "Equality Constraint", "Exclusion Constraint",
            "Exclusive-Or Constraint", "Or Constraint", "Value Constraint",
            "Deontic Constraint",
        ];
        for k in expected.iter() {
            assert!(kinds.iter().any(|x| x == k),
                "boot table missing kind {:?}; have {:?}", k, kinds);
        }
        assert_eq!(kinds.len(), expected.len(),
            "boot table has {} kinds, expected {}: {:?}",
            kinds.len(), expected.len(), kinds);
    }

    #[test]
    fn statement_translator_table_translators_for_handles_many_to_many() {
        // Subtype Declaration is handled by both translate_nouns AND
        // translate_subtypes (the Rust pipeline runs both for that
        // kind). Translation order matches declaration order.
        let table = super::StatementTranslatorTable::boot();
        let subtype = table.translators_for("Subtype Declaration");
        assert_eq!(subtype, vec!["translate_nouns", "translate_subtypes"],
            "Subtype Declaration must dispatch to both nouns + subtypes");
        // translate_set_constraints handles five kinds; query each.
        for kind in ["Subset Constraint", "Equality Constraint",
                     "Exclusion Constraint", "Exclusive-Or Constraint",
                     "Or Constraint"] {
            let t = table.translators_for(kind);
            assert!(t.contains(&"translate_set_constraints"),
                "{kind} should be handled by translate_set_constraints; got {t:?}");
        }
    }

    #[test]
    fn statement_translator_table_translators_for_unknown_kind_returns_empty() {
        let table = super::StatementTranslatorTable::boot();
        assert!(table.translators_for("Made Up Kind").is_empty());
        assert!(table.translators_for("").is_empty());
    }

    #[test]
    fn statement_translator_table_from_grammar_state_reads_binary_facts() {
        // Fact-based dispatch table: the cell
        // `Classification_has_Translator` carries one
        // fact per (kind, translator) pair. The reader builds the
        // table from those facts, in declaration order.
        let state = synthetic_translator_state(&[
            ("Entity Type Declaration", "translate_nouns"),
            ("Subtype Declaration",     "translate_nouns"),
            ("Subtype Declaration",     "translate_subtypes"),
        ]);
        let table = super::StatementTranslatorTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.translators_for("Entity Type Declaration"),
                   vec!["translate_nouns"]);
        assert_eq!(table.translators_for("Subtype Declaration"),
                   vec!["translate_nouns", "translate_subtypes"]);
        assert!(table.translators_for("Derivation Rule").is_empty(),
            "kinds not in cell must not resolve");
    }

    #[test]
    fn statement_translator_table_falls_back_to_boot_on_empty_cell() {
        // Empty cell → boot fallback. Same defensive pattern as the
        // other #713 tables: a missing or empty grammar declaration
        // can't silently produce a translator-less stage-2.
        let state: Object = {
            let mut m: HashMap<String, Object> = HashMap::new();
            m.insert("Classification_has_Translator".to_string(),
                     Object::Seq(alloc::sync::Arc::from(Vec::<Object>::new())));
            Object::Map(m)
        };
        let table = super::StatementTranslatorTable::from_grammar_state(&state);
        // boot has all 20 kinds; empty cell would have 0.
        assert_eq!(table.kinds().len(), 20);
    }

    #[test]
    fn statement_translator_table_from_real_grammar_loads_full_dispatch() {
        // Per AREST.tex §3 (eq:sys) the dispatch table is fact-based:
        // the grammar's `Classification is translated by Translator`
        // declarations are the source of truth, with boot() as a
        // chicken-and-egg fallback for parsing the grammar that
        // declares the table. This test verifies (a) the grammar
        // *does* declare the cell — boot fallback is not enough, and
        // (b) every kind in boot is covered by the grammar.
        let state = grammar_state();
        let cell = fetch_or_phi("Classification_has_Translator", &state);
        let facts = cell.as_seq().expect(
            "grammar must declare the Classification_has_Translator \
             cell — fact-based dispatch (per #833 / AREST.tex §3 eq:sys) cannot \
             rely on the boot fallback");
        assert!(!facts.is_empty(),
            "Classification_has_Translator cell exists but is empty; \
             grammar must populate it with one row per (kind, translator) pair");
        let table = super::StatementTranslatorTable::from_grammar_state(&state);
        let boot = super::StatementTranslatorTable::boot();
        for kind in boot.kinds() {
            let from_grammar = table.translators_for(kind);
            let from_boot = boot.translators_for(kind);
            assert_eq!(from_grammar, from_boot,
                "kind {kind:?} translators differ: grammar {from_grammar:?} vs boot {from_boot:?}");
        }
    }
}
