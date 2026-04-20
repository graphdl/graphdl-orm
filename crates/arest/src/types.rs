// crates/arest/src/types.rs
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

// Domain struct moved to parse_forml2.rs (#211) — it is a parse-time
// accumulator, not a public type. Re-exported as pub(crate) from there.

/// x̄ — a constant asserted into P.
/// Subject noun 'value' predicate object 'value'.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralInstanceFact {
    pub subject_noun: String,
    pub subject_value: String,
    pub field_name: String,
    pub object_noun: String,
    pub object_value: String,
}

/// World assumption for a noun — determines how absence of facts is interpreted
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum WorldAssumption {
    Closed, // not stated = false (permissions, corporate authority)
    Open,   // not stated = unknown (capabilities, unenumerated abilities)
}

impl Default for WorldAssumption {
    fn default() -> Self {
        WorldAssumption::Closed
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NounDef {
    pub object_type: String,
    #[serde(default)]
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
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
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
    /// sentinel-prefixed strings.
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
                out.push_str(role);
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
            if let Some((idx_str, role)) = rest.split_once(':') {
                if let Ok(antecedent_index) = idx_str.parse::<usize>() {
                    return Self::AntecedentRole {
                        antecedent_index,
                        role: role.to_string(),
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

/// A derivation rule in the IR — compiled to a DeriveFn at compile time.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivationRuleDef {
    pub id: String,
    pub text: String,
    /// The reading/condition that must hold
    pub antecedent_fact_type_ids: Vec<String>,
    /// Where the consequent's cell key comes from. `Literal(..)` is the
    /// common case (every user-authored rule) and preserves the
    /// pre-enum wire format. `AntecedentRole { .. }` lets a single rule
    /// fan out across all user fact types matching the antecedent,
    /// which the four implicit-derivation shapes (subtype inheritance,
    /// CWA negation, …) expand into readings (`#287`).
    #[serde(default)]
    pub consequent_cell: ConsequentCellSource,
    /// Derivation kind for compile dispatch
    pub kind: DerivationKind,
    /// For Join rules: noun names that must have equal values across all
    /// antecedent facts that mention them. Enforces both:
    /// - Value join keys (e.g., "Squish VIN" matches Vehicle↔Candidate↔Listing)
    /// - Entity consistency (e.g., "Chrome Style Candidate" is the same entity
    ///   across candidate_squishvin and candidate_trim facts)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub join_on: Vec<String>,
    /// For Join rules: cross-noun match predicates. Each pair (left, right)
    /// requires that the value of left contains the value of right (case-insensitive).
    /// Used for fuzzy matching like "Chrome Trim" contains "Listing Trim".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub match_on: Vec<(String, String)>,
    /// For Join rules: noun names to include in the consequent bindings.
    /// If empty, all bindings from the joined facts are included.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consequent_bindings: Vec<String>,
    /// Inline numeric comparisons attached to individual antecedents.
    ///
    /// Halpin FORML Example 5 (W3C position paper):
    ///   Each LargeUSCity is a City that is in Country 'US'
    ///                    and has Population >= 1000000.
    /// The `>= 1000000` is recorded as an AntecedentFilter pinned to the
    /// `has Population` antecedent. At compile time the rule becomes
    /// Filter(p) : P over the filtered antecedent facts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub antecedent_filters: Vec<AntecedentFilter>,
    /// Consequent roles whose values are computed from arithmetic over
    /// antecedent role values. Halpin FORML attribute-style definitions:
    ///   * Box has Volume iff Box has Size and Volume is Size * Size * Size.
    /// The `Volume is Size * Size * Size` clause populates this with one
    /// ConsequentComputedBinding { role: "Volume", expr: ... }.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consequent_computed_bindings: Vec<ConsequentComputedBinding>,
    /// Consequent roles whose values come from aggregating an image set
    /// (Codd §2.3.4 + Backus Insert). Halpin FORML:
    ///   * Fact Type has Arity iff Arity is the count of Role where Fact
    ///     Type has Role.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consequent_aggregates: Vec<ConsequentAggregate>,
    /// Clauses the parser saw in the derivation body but could not
    /// classify into any known form (FT reference, comparison,
    /// aggregate, computed binding, anaphora, negation). The checker
    /// reports these directly — no parallel heuristic needed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_clauses: Vec<String>,
    /// Per-antecedent string-literal role equality filters. FORML 2 grammar
    /// readings use this for token matching, e.g.
    ///   `Statement has Classification 'Entity Type Declaration' iff
    ///    Statement has Trailing Marker 'is an entity type'`
    /// where the antecedent fact type `Statement has Trailing Marker`
    /// must have its `Trailing Marker` role equal to the literal
    /// `'is an entity type'`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub antecedent_role_literals: Vec<AntecedentRoleLiteral>,
    /// Consequent roles bound to fixed string literals. Used by grammar
    /// readings whose consequent specifies a role's value, e.g.
    ///   `Statement has Classification 'Entity Type Declaration'`.
    /// The `Classification` role is bound to the literal; the remaining
    /// consequent roles inherit bindings from the antecedent fact.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consequent_role_literals: Vec<ConsequentRoleLiteral>,
}

/// Numeric comparison that further restricts a derivation antecedent.
/// See `DerivationRuleDef::antecedent_filters`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AntecedentFilter {
    /// Index into `antecedent_fact_type_ids` that this filter restricts.
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
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AntecedentRoleLiteral {
    /// Index into `antecedent_fact_type_ids` that this filter restricts.
    pub antecedent_index: usize,
    /// Role name (noun name) whose value must equal `value`.
    pub role: String,
    /// Required literal value (string equality).
    pub value: String,
}

/// Fixed string literal bound to a consequent role.
/// See `DerivationRuleDef::consequent_role_literals`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DerivationKind {
    SubtypeInheritance, // X is subtype of Y -> X inherits Y's constraints
    ModusPonens,        // If A then B, A holds -> B holds
    Transitivity,       // A->B, B->C -> A->C
    ClosedWorldNegation, // Not derivable under CWA -> false
    Join,               // Cross-fact-type equi-join on shared noun names
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FactTypeDef {
    #[serde(default)]
    pub schema_id: String,
    pub reading: String,
    #[serde(default)]
    pub readings: Vec<ReadingDef>,
    pub roles: Vec<RoleDef>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingDef {
    pub text: String,
    pub role_order: Vec<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDef {
    pub noun_name: String,
    pub role_index: usize,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanDef {
    pub fact_type_id: String,
    pub role_index: usize,
    #[allow(dead_code)] // deserialized from JSON, read by JS callers
    pub subset_autofill: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateMachineDef {
    pub noun_name: String,
    pub statuses: Vec<String>,
    pub transitions: Vec<TransitionDef>,
    /// Explicitly declared initial status. Empty when neither
    /// `Status 'X' is initial in SM Definition 'Y'` was asserted nor
    /// graph-topology inference (source-never-target) gave a unique
    /// answer. The compiled machine fails visibly at first SM call
    /// when empty — per Thm 3, the fold needs s_0.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub initial: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionDef {
    pub from: String,
    pub to: String,
    pub event: String,
    pub guard: Option<GuardDef>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Violation {
    pub constraint_id: String,
    pub constraint_text: String,
    pub detail: String,
    /// Alethic violations are structural impossibilities (always reject).
    /// Deontic violations are reportable but may not reject.
    #[serde(default = "default_alethic")]
    pub alethic: bool,
}

#[allow(dead_code)] // Used by serde default attribute
fn default_alethic() -> bool { true }

// ── Forward Inference & Synthesis Types ──────────────────────────────

/// A fact derived by forward inference
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivedFact {
    pub fact_type_id: String,
    pub reading: String,
    pub bindings: Vec<(String, String)>,
    pub derived_by: String, // ID of the derivation rule that produced this
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Definitive, // derived under CWA — fact is definitively true/false
    #[allow(dead_code)] // reserved: used when OWA derivation is implemented
    Incomplete, // derived under OWA — absence doesn't mean false
}

// ── Proof Engine Types ──────────────────────────────────────────────

/// Result of attempting to prove a goal
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // used by lib.rs WASM export, not by main.rs binary
pub struct ProofResult {
    pub goal: String,
    pub status: ProofStatus,
    pub proof: Option<ProofStep>,
    pub world_assumption: WorldAssumption,
}

/// Result of synthesizing knowledge about a noun
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FactTypeSummary {
    pub id: String,
    pub reading: String,
    pub role_index: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintSummary {
    pub id: String,
    pub text: String,
    pub kind: String,
    pub modality: String,
    pub deontic_operator: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateMachineSummary {
    pub noun_name: String,
    pub statuses: Vec<String>,
    pub current_status: Option<String>,
    pub valid_transitions: Vec<String>, // events that can fire from current state
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedNoun {
    pub name: String,
    pub via_fact_type: String,
    pub via_reading: String,
    pub world_assumption: WorldAssumption,
}
