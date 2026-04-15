// crates/arest/src/types.rs
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Domain: the parsed result of FORML2 readings ────────────────────

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Domain {
    #[allow(dead_code)] // deserialized from JSON, read by JS callers
    pub domain: String,
    pub nouns: HashMap<String, NounDef>,
    pub fact_types: HashMap<String, FactTypeDef>,
    pub constraints: Vec<ConstraintDef>,
    pub state_machines: HashMap<String, StateMachineDef>,
    #[serde(default)]
    pub derivation_rules: Vec<DerivationRuleDef>,
    #[serde(default)]
    pub general_instance_facts: Vec<GeneralInstanceFact>,
    /// child noun → parent noun (subtype relationships)
    #[serde(default)]
    pub subtypes: HashMap<String, String>,
    /// noun name → enum values
    #[serde(default)]
    pub enum_values: HashMap<String, Vec<String>>,
    /// noun name → reference scheme parts
    #[serde(default)]
    pub ref_schemes: HashMap<String, Vec<String>>,
    /// objectified noun → fact type reading
    #[serde(default)]
    pub objectifications: HashMap<String, String>,
    /// Named constraint spans: span_name → role nouns
    #[serde(default)]
    pub named_spans: HashMap<String, Vec<String>>,
    /// Span names with autofill enabled
    #[serde(default)]
    pub autofill_spans: Vec<String>,
}

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

/// A derivation rule in the IR — compiled to a DeriveFn at compile time.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivationRuleDef {
    pub id: String,
    pub text: String,
    /// The reading/condition that must hold
    pub antecedent_fact_type_ids: Vec<String>,
    /// What is derived when antecedent holds
    pub consequent_fact_type_id: String,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateMachineDef {
    pub noun_name: String,
    pub statuses: Vec<String>,
    pub transitions: Vec<TransitionDef>,
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
