// crates/fol-engine/src/types.rs
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── IR Types (deserialized from generator JSON) ──────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintIR {
    pub domain: String,
    pub nouns: HashMap<String, NounDef>,
    pub fact_types: HashMap<String, FactTypeDef>,
    pub constraints: Vec<ConstraintDef>,
    pub state_machines: HashMap<String, StateMachineDef>,
    #[serde(default)]
    pub derivation_rules: Vec<DerivationRuleDef>,
}

/// World assumption for a noun — determines how absence of facts is interpreted
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum WorldAssumption {
    Closed, // not stated = false (government powers, corporate authority)
    Open,   // not stated = unknown (individual rights, unenumerated freedoms)
}

impl Default for WorldAssumption {
    fn default() -> Self {
        WorldAssumption::Closed
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NounDef {
    pub object_type: String,
    pub enum_values: Option<Vec<String>>,
    pub value_type: Option<String>,
    pub super_type: Option<String>,
    #[serde(default)]
    pub world_assumption: WorldAssumption,
}

/// A derivation rule in the IR — compiled to a DeriveFn at compile time.
#[derive(Debug, Clone, Deserialize)]
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
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DerivationKind {
    SubtypeInheritance, // X is subtype of Y -> X inherits Y's constraints
    ModusPonens,        // If A then B, A holds -> B holds
    Transitivity,       // A->B, B->C -> A->C
    ClosedWorldNegation, // Not derivable under CWA -> false
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactTypeDef {
    pub reading: String,
    pub roles: Vec<RoleDef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDef {
    pub noun_name: String,
    pub role_index: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintDef {
    pub id: String,
    pub kind: String,
    pub modality: String,
    pub deontic_operator: Option<String>,
    pub text: String,
    pub spans: Vec<SpanDef>,
    pub set_comparison_argument_length: Option<usize>,
    pub clauses: Option<Vec<String>>,
    pub entity: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanDef {
    pub fact_type_id: String,
    pub role_index: usize,
    pub subset_autofill: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateMachineDef {
    pub noun_name: String,
    pub statuses: Vec<String>,
    pub transitions: Vec<TransitionDef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionDef {
    pub from: String,
    pub to: String,
    pub event: String,
    pub guard: Option<GuardDef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardDef {
    pub graph_schema_id: String,
    pub constraint_ids: Vec<String>,
}

// ── Evaluation Types ─────────────────────────────────────────────────

/// A snapshot of facts for evaluation. Keys are fact type IDs.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Population {
    pub facts: HashMap<String, Vec<FactInstance>>,
}

/// A single fact instance — binds references to roles in a fact type.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactInstance {
    pub fact_type_id: String,
    /// Vec of (role_noun_name, reference_value)
    pub bindings: Vec<(String, String)>,
}

/// The response being evaluated (for deontic text constraints).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseContext {
    pub text: String,
    pub sender_identity: Option<String>,
    pub fields: Option<HashMap<String, String>>,
}

/// A constraint violation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Violation {
    pub constraint_id: String,
    pub constraint_text: String,
    pub detail: String,
}

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
    Incomplete, // derived under OWA — absence doesn't mean false
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
