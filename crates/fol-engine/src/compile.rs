// crates/fol-engine/src/compile.rs
//
// Compilation: ConstraintIR → CompiledModel
//
// Constraints ARE predicates, not data that gets matched.
// The match on constraint kind happens once at compile time. After compilation,
// evaluation is pure function application — no dispatch, no branching on kind.
//
// This implements Backus's FP algebra (1977 Turing Lecture):
//   - Constraints and derivations compile to pure functions (combining forms)
//   - Evaluation is function application over whole structures
//   - State machines are folds: run_machine = fold(transition)(initial)(stream)
//   - No variables, no mutable state during evaluation — only reduction

use std::sync::Arc;
use std::collections::{HashMap, HashSet};
use crate::types::*;

// Re-export DerivedFact-related types used by derivation compilers
// (already imported via crate::types::*)

// ── Core Functional Types ──────────────────────────────────────────

/// A predicate is a pure function from evaluation context to violations.
/// This is the fundamental type. Constraints ARE predicates.
pub type Predicate = Arc<dyn Fn(&EvalContext) -> Vec<Violation> + Send + Sync>;

/// Immutable evaluation context — the only input predicates receive.
pub struct EvalContext<'a> {
    pub response: &'a ResponseContext,
    pub population: &'a Population,
}

#[derive(Clone, Debug)]
pub enum Modality {
    Alethic,
    Deontic(DeonticOp),
}

#[derive(Clone, Debug)]
pub enum DeonticOp {
    Forbidden,
    Obligatory,
    Permitted,
}

/// A compiled constraint: identity + modality + predicate.
pub struct CompiledConstraint {
    pub id: String,
    pub text: String,
    pub modality: Modality,
    pub predicate: Predicate,
}

/// A compiled derivation rule — produces derived facts instead of violations.
/// Same architecture as CompiledConstraint but output type differs.
/// Derivation: (context, population) → [DerivedFact]
pub type DeriveFn = Arc<dyn Fn(&EvalContext, &Population) -> Vec<DerivedFact> + Send + Sync>;

pub struct CompiledDerivation {
    #[allow(dead_code)] // deserialized from JSON, read by JS callers
    pub id: String,
    #[allow(dead_code)] // deserialized from JSON, read by JS callers
    pub text: String,
    #[allow(dead_code)] // deserialized from JSON, read by JS callers
    pub kind: DerivationKind,
    pub derive: DeriveFn,
}

/// A compiled state machine: transition function + initial state.
/// Evaluation: fold(transition)(initial)(event_stream)
pub struct CompiledStateMachine {
    pub noun_name: String,
    pub statuses: Vec<String>,
    pub initial: String,
    /// Transition: (current_state, event, ctx) → Option<next_state>
    /// Guard passes iff guard predicate produces zero violations.
    #[allow(dead_code)] // used by run_machine WASM export
    pub transition: Arc<dyn Fn(&str, &str, &EvalContext) -> Option<String> + Send + Sync>,
    /// Compiled transitions as (from, to, event) for introspection
    pub transition_table: Vec<(String, String, String)>,
}

/// Index for fast noun lookups during synthesis.
pub struct NounIndex {
    /// noun_name -> list of (fact_type_id, role_index) where noun plays a role
    pub noun_to_fact_types: HashMap<String, Vec<(String, usize)>>,
    /// noun_name -> world assumption
    pub world_assumptions: HashMap<String, WorldAssumption>,
    /// noun_name -> supertype name
    #[allow(dead_code)] // deserialized from JSON, read by JS callers
    pub supertypes: HashMap<String, String>,
    /// noun_name -> list of subtype names
    #[allow(dead_code)] // deserialized from JSON, read by JS callers
    pub subtypes: HashMap<String, Vec<String>>,
    /// fact_type_id -> list of constraint IDs spanning it
    pub fact_type_to_constraints: HashMap<String, Vec<String>>,
    /// constraint_id -> index into CompiledModel.constraints
    pub constraint_index: HashMap<String, usize>,
    /// noun_name -> state machine index
    pub noun_to_state_machines: HashMap<String, usize>,
}

/// The compiled model — all constraints, derivations, and state machines as executable functions.
pub struct CompiledModel {
    pub constraints: Vec<CompiledConstraint>,
    pub derivations: Vec<CompiledDerivation>,
    pub state_machines: Vec<CompiledStateMachine>,
    pub noun_index: NounIndex,
}

// ── Population Primitives ──────────────────────────────────────────
// Composable building blocks. Each is a pure function over Population.

/// All instances of a noun across the entire population.
fn instances_of(noun_name: &str, population: &Population) -> HashSet<String> {
    population.facts.values()
        .flat_map(|facts| facts.iter())
        .flat_map(|f| &f.bindings)
        .filter(|(name, _)| name == noun_name)
        .map(|(_, val)| val.clone())
        .collect()
}

/// Whether an entity instance participates in a specific fact type.
fn participates_in(entity: &str, noun_name: &str, fact_type_id: &str, population: &Population) -> bool {
    population.facts.get(fact_type_id).map_or(false, |facts| {
        facts.iter().any(|f| {
            f.bindings.iter().any(|(name, val)| name == noun_name && val == entity)
        })
    })
}

// ── Span Resolution ────────────────────────────────────────────────
// Resolves IR references at compile time so predicates capture only what they need.

#[derive(Clone)]
struct ResolvedSpan {
    fact_type_id: String,
    role_index: usize,
    noun_name: String,
    reading: String,
}

fn resolve_spans(ir: &ConstraintIR, spans: &[SpanDef]) -> Vec<ResolvedSpan> {
    spans.iter().filter_map(|span| {
        let ft = ir.fact_types.get(&span.fact_type_id)?;
        let role = ft.roles.get(span.role_index)?;
        Some(ResolvedSpan {
            fact_type_id: span.fact_type_id.clone(),
            role_index: span.role_index,
            noun_name: role.noun_name.clone(),
            reading: ft.reading.clone(),
        })
    }).collect()
}

/// Collect (noun_name, enum_values) for value-type nouns in spanned fact types.
/// Deduplicates by noun name — each noun's enum values appear at most once.
fn collect_enum_values(ir: &ConstraintIR, spans: &[SpanDef]) -> Vec<(String, Vec<String>)> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for span in spans {
        if let Some(ft) = ir.fact_types.get(&span.fact_type_id) {
            for role in &ft.roles {
                if seen.contains(&role.noun_name) { continue; }
                if let Some(noun_def) = ir.nouns.get(&role.noun_name) {
                    if noun_def.object_type == "value" {
                        if let Some(vals) = &noun_def.enum_values {
                            if !vals.is_empty() {
                                seen.insert(role.noun_name.clone());
                                result.push((role.noun_name.clone(), vals.clone()));
                            }
                        }
                    }
                }
            }
        }
    }
    result
}

// ── Compilation ────────────────────────────────────────────────────
// The match on kind happens here, once. After this, everything is Predicate.

/// Compile an entire ConstraintIR into executable form.
pub fn compile(ir: &ConstraintIR) -> CompiledModel {
    let constraints: Vec<CompiledConstraint> = ir.constraints.iter()
        .map(|def| compile_constraint(ir, def))
        .collect();

    let constraint_predicates: HashMap<String, Predicate> = constraints.iter()
        .map(|c| (c.id.clone(), c.predicate.clone()))
        .collect();

    let state_machines: Vec<CompiledStateMachine> = ir.state_machines.values()
        .map(|sm_def| compile_state_machine(sm_def, &constraint_predicates))
        .collect();

    // Build NounIndex for synthesis queries
    let noun_index = build_noun_index(ir, &constraints, &state_machines);

    // Compile derivation rules — both explicit from IR and implicit from structure
    let derivations = compile_derivations(ir);

    CompiledModel { constraints, derivations, state_machines, noun_index }
}

/// Build the NounIndex by iterating the IR.
fn build_noun_index(
    ir: &ConstraintIR,
    constraints: &[CompiledConstraint],
    state_machines: &[CompiledStateMachine],
) -> NounIndex {
    // noun_name -> list of (fact_type_id, role_index)
    let mut noun_to_fact_types: HashMap<String, Vec<(String, usize)>> = HashMap::new();
    for (ft_id, ft) in &ir.fact_types {
        for role in &ft.roles {
            noun_to_fact_types.entry(role.noun_name.clone())
                .or_default()
                .push((ft_id.clone(), role.role_index));
        }
    }

    // noun_name -> world assumption
    let world_assumptions: HashMap<String, WorldAssumption> = ir.nouns.iter()
        .map(|(name, def)| (name.clone(), def.world_assumption.clone()))
        .collect();

    // noun_name -> supertype
    let mut supertypes: HashMap<String, String> = HashMap::new();
    let mut subtypes: HashMap<String, Vec<String>> = HashMap::new();
    for (name, def) in &ir.nouns {
        if let Some(ref st) = def.super_type {
            supertypes.insert(name.clone(), st.clone());
            subtypes.entry(st.clone()).or_default().push(name.clone());
        }
    }

    // fact_type_id -> list of constraint IDs spanning it
    let mut fact_type_to_constraints: HashMap<String, Vec<String>> = HashMap::new();
    for cdef in &ir.constraints {
        for span in &cdef.spans {
            fact_type_to_constraints.entry(span.fact_type_id.clone())
                .or_default()
                .push(cdef.id.clone());
        }
    }

    // constraint_id -> index
    let constraint_index: HashMap<String, usize> = constraints.iter()
        .enumerate()
        .map(|(i, c)| (c.id.clone(), i))
        .collect();

    // noun_name -> state machine index
    let noun_to_state_machines: HashMap<String, usize> = state_machines.iter()
        .enumerate()
        .map(|(i, sm)| (sm.noun_name.clone(), i))
        .collect();

    NounIndex {
        noun_to_fact_types,
        world_assumptions,
        supertypes,
        subtypes,
        fact_type_to_constraints,
        constraint_index,
        noun_to_state_machines,
    }
}

/// Compile all derivation rules: explicit from IR + implicit structural rules.
fn compile_derivations(ir: &ConstraintIR) -> Vec<CompiledDerivation> {
    let mut derivations = Vec::new();

    // Compile explicit derivation rules from IR
    for rule in &ir.derivation_rules {
        let compiled = match rule.kind {
            DerivationKind::SubtypeInheritance => compile_explicit_derivation(ir, rule),
            DerivationKind::ModusPonens => compile_explicit_derivation(ir, rule),
            DerivationKind::Transitivity => compile_explicit_derivation(ir, rule),
            DerivationKind::ClosedWorldNegation => compile_explicit_derivation(ir, rule),
        };
        derivations.push(compiled);
    }

    // Implicit: subtype inheritance from noun definitions
    derivations.extend(compile_subtype_inheritance(ir));

    // Implicit: modus ponens from subset constraints
    derivations.extend(compile_modus_ponens(ir));

    // Implicit: transitivity from shared roles
    derivations.extend(compile_transitivity(ir));

    // Implicit: CWA negation from world assumptions
    derivations.extend(compile_cwa_negation(ir));

    derivations
}

/// Compile an explicit derivation rule from the IR.
fn compile_explicit_derivation(ir: &ConstraintIR, rule: &DerivationRuleDef) -> CompiledDerivation {
    let id = rule.id.clone();
    let text = rule.text.clone();
    let kind = rule.kind.clone();
    let antecedent_ids = rule.antecedent_fact_type_ids.clone();
    let consequent_id = rule.consequent_fact_type_id.clone();
    let consequent_reading = ir.fact_types.get(&consequent_id)
        .map(|ft| ft.reading.clone())
        .unwrap_or_default();

    let derive_id = id.clone();
    let derive: DeriveFn = Arc::new(move |_ctx: &EvalContext, population: &Population| {
        // Check if all antecedent fact types have instances
        let all_hold = antecedent_ids.iter().all(|ft_id| {
            population.facts.get(ft_id).map_or(false, |facts| !facts.is_empty())
        });

        if all_hold {
            // Collect all entity bindings from antecedent facts
            let mut bindings = Vec::new();
            for ft_id in &antecedent_ids {
                if let Some(facts) = population.facts.get(ft_id) {
                    for fact in facts {
                        for binding in &fact.bindings {
                            if !bindings.contains(binding) {
                                bindings.push(binding.clone());
                            }
                        }
                    }
                }
            }

            vec![DerivedFact {
                fact_type_id: consequent_id.clone(),
                reading: consequent_reading.clone(),
                bindings,
                derived_by: derive_id.clone(),
                confidence: Confidence::Definitive,
            }]
        } else {
            Vec::new()
        }
    });

    CompiledDerivation { id, text, kind, derive }
}

/// Subtype inheritance: for each noun with a supertype,
/// instances of the subtype inherit participation in the supertype's fact types.
fn compile_subtype_inheritance(ir: &ConstraintIR) -> Vec<CompiledDerivation> {
    let mut derivations = Vec::new();

    for (sub_name, noun_def) in &ir.nouns {
        if let Some(ref super_name) = noun_def.super_type {
            // Find all fact types where the supertype plays a role
            let super_fact_types: Vec<(String, String, usize)> = ir.fact_types.iter()
                .flat_map(|(ft_id, ft)| {
                    ft.roles.iter()
                        .filter(|r| r.noun_name == *super_name)
                        .map(move |r| (ft_id.clone(), ft.reading.clone(), r.role_index))
                })
                .collect();

            if super_fact_types.is_empty() {
                continue;
            }

            let sub = sub_name.clone();
            let sup = super_name.clone();
            let sft = super_fact_types.clone();
            let id = format!("_subtype_{}_{}", sub, sup);
            let text = format!("{} is a subtype of {} — inherits fact types", sub, sup);

            let derive: DeriveFn = Arc::new(move |_ctx: &EvalContext, population: &Population| {
                let mut derived = Vec::new();

                // Find all instances of the subtype in the population
                let sub_instances = instances_of(&sub, population);

                for (ft_id, reading, _role_idx) in &sft {
                    for instance in &sub_instances {
                        // Check if this instance already participates in this fact type
                        if !participates_in(instance, &sup, ft_id, population) {
                            derived.push(DerivedFact {
                                fact_type_id: ft_id.clone(),
                                reading: reading.clone(),
                                bindings: vec![(sup.clone(), instance.clone())],
                                derived_by: format!("_subtype_{}_{}", sub, sup),
                                confidence: Confidence::Definitive,
                            });
                        }
                    }
                }

                derived
            });

            derivations.push(CompiledDerivation {
                id,
                text,
                kind: DerivationKind::SubtypeInheritance,
                derive,
            });
        }
    }

    derivations
}

/// Modus ponens on subset constraints: if A subset B (SS constraint),
/// when we find an instance in A, derive its presence in B.
fn compile_modus_ponens(ir: &ConstraintIR) -> Vec<CompiledDerivation> {
    let mut derivations = Vec::new();

    for cdef in &ir.constraints {
        if cdef.kind != "SS" || cdef.spans.len() < 2 {
            continue;
        }

        let a_ft_id = cdef.spans[0].fact_type_id.clone();
        let b_ft_id = cdef.spans[1].fact_type_id.clone();

        let entity_name = ir.fact_types.get(&a_ft_id)
            .and_then(|ft| ft.roles.get(cdef.spans[0].role_index))
            .map(|r| r.noun_name.clone())
            .unwrap_or_default();

        let b_reading = ir.fact_types.get(&b_ft_id)
            .map(|ft| ft.reading.clone())
            .unwrap_or_default();

        let id = format!("_modus_ponens_{}", cdef.id);
        let text = format!("Modus ponens from SS constraint: {}", cdef.text);
        let derive_id = id.clone();

        let derive: DeriveFn = Arc::new(move |_ctx: &EvalContext, population: &Population| {
            let a_facts = population.facts.get(&a_ft_id).cloned().unwrap_or_default();
            let b_facts = population.facts.get(&b_ft_id).cloned().unwrap_or_default();

            a_facts.iter().filter_map(|a_fact| {
                if let Some((_, entity_val)) = a_fact.bindings.iter()
                    .find(|(name, _)| *name == entity_name)
                {
                    let b_holds = b_facts.iter().any(|bf| {
                        bf.bindings.iter().any(|(_, val)| val == entity_val)
                    });
                    if !b_holds {
                        Some(DerivedFact {
                            fact_type_id: b_ft_id.clone(),
                            reading: b_reading.clone(),
                            bindings: vec![(entity_name.clone(), entity_val.clone())],
                            derived_by: derive_id.clone(),
                            confidence: Confidence::Definitive,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            }).collect()
        });

        derivations.push(CompiledDerivation {
            id,
            text,
            kind: DerivationKind::ModusPonens,
            derive,
        });
    }

    derivations
}

/// Transitivity: for fact types that share a noun in different roles (A->B, B->C),
/// derive the transitive closure A->C. Limited depth to prevent infinite chains.
fn compile_transitivity(ir: &ConstraintIR) -> Vec<CompiledDerivation> {
    let mut derivations = Vec::new();

    // Find binary fact types (exactly 2 roles) that share a noun
    let binary_fts: Vec<(&String, &FactTypeDef)> = ir.fact_types.iter()
        .filter(|(_, ft)| ft.roles.len() == 2)
        .collect();

    for (i, (ft1_id, ft1)) in binary_fts.iter().enumerate() {
        for (j, (ft2_id, ft2)) in binary_fts.iter().enumerate() {
            if i == j { continue; } // skip self-pairing
            // Skip same fact type (self-transitivity handled separately)
            // Check if ft1's role[1] noun == ft2's role[0] noun (A->B, B->C)
            let ft1_r1 = &ft1.roles[1].noun_name;
            let ft2_r0 = &ft2.roles[0].noun_name;

            if ft1_r1 != ft2_r0 {
                continue;
            }

            let shared_noun = ft1_r1.clone();
            let src_noun = ft1.roles[0].noun_name.clone();
            let dst_noun = ft2.roles[1].noun_name.clone();

            let ft1_id_c = (*ft1_id).clone();
            let ft2_id_c = (*ft2_id).clone();
            let reading = format!("{} transitively relates to {} via {}",
                src_noun, dst_noun, shared_noun);

            let id = format!("_transitivity_{}_{}",  ft1_id_c, ft2_id_c);
            let derive_id = id.clone();
            let reading_c = reading.clone();
            let src_noun_c = src_noun.clone();
            let dst_noun_c = dst_noun.clone();
            let shared_noun_c = shared_noun.clone();

            let derive: DeriveFn = Arc::new(move |_ctx: &EvalContext, population: &Population| {
                let ft1_facts = population.facts.get(&ft1_id_c).cloned().unwrap_or_default();
                let ft2_facts = population.facts.get(&ft2_id_c).cloned().unwrap_or_default();

                let mut derived = Vec::new();

                for f1 in &ft1_facts {
                    // Get the value of the shared noun from ft1's role[1]
                    let shared_val = f1.bindings.iter()
                        .find(|(name, _)| *name == shared_noun_c)
                        .map(|(_, v)| v.clone());

                    let src_val = f1.bindings.iter()
                        .find(|(name, _)| *name == src_noun_c)
                        .map(|(_, v)| v.clone());

                    if let (Some(shared_v), Some(src_v)) = (shared_val, src_val) {
                        // Find matching ft2 facts where role[0] == shared_val
                        for f2 in &ft2_facts {
                            let f2_shared = f2.bindings.iter()
                                .find(|(name, _)| *name == shared_noun_c)
                                .map(|(_, v)| v.clone());

                            if f2_shared.as_deref() == Some(&shared_v) {
                                if let Some((_, dst_v)) = f2.bindings.iter()
                                    .find(|(name, _)| *name == dst_noun_c)
                                {
                                    derived.push(DerivedFact {
                                        fact_type_id: format!("_transitive_{}_{}",
                                            ft1_id_c, ft2_id_c),
                                        reading: reading_c.clone(),
                                        bindings: vec![
                                            (src_noun_c.clone(), src_v.clone()),
                                            (dst_noun_c.clone(), dst_v.clone()),
                                        ],
                                        derived_by: derive_id.clone(),
                                        confidence: Confidence::Definitive,
                                    });
                                }
                            }
                        }
                    }
                }

                derived
            });

            derivations.push(CompiledDerivation {
                id,
                text: reading,
                kind: DerivationKind::Transitivity,
                derive,
            });
        }
    }

    derivations
}

/// CWA negation: for nouns with WorldAssumption::Closed,
/// if a fact type involving this noun has no instances for a given entity,
/// derive the negation. For OWA nouns, absence is unknown, not false.
fn compile_cwa_negation(ir: &ConstraintIR) -> Vec<CompiledDerivation> {
    let mut derivations = Vec::new();

    for (noun_name, noun_def) in &ir.nouns {
        if noun_def.world_assumption != WorldAssumption::Closed {
            continue;
        }

        // Find all fact types where this CWA noun plays a role
        let relevant_fts: Vec<(String, String, usize)> = ir.fact_types.iter()
            .flat_map(|(ft_id, ft)| {
                ft.roles.iter()
                    .filter(|r| r.noun_name == *noun_name)
                    .map(move |r| (ft_id.clone(), ft.reading.clone(), r.role_index))
            })
            .collect();

        if relevant_fts.is_empty() {
            continue;
        }

        let noun = noun_name.clone();
        let rft = relevant_fts.clone();
        let id = format!("_cwa_negation_{}", noun);
        let text = format!("CWA: absent facts about {} are false", noun);
        let derive_id = id.clone();

        let derive: DeriveFn = Arc::new(move |_ctx: &EvalContext, population: &Population| {
            let all_instances = instances_of(&noun, population);
            let mut derived = Vec::new();

            for (ft_id, reading, _role_idx) in &rft {
                for instance in &all_instances {
                    if !participates_in(instance, &noun, ft_id, population) {
                        derived.push(DerivedFact {
                            fact_type_id: ft_id.clone(),
                            reading: format!("NOT: {} (CWA negation for {} '{}')",
                                reading, noun, instance),
                            bindings: vec![(noun.clone(), instance.clone())],
                            derived_by: derive_id.clone(),
                            confidence: Confidence::Definitive,
                        });
                    }
                }
            }

            derived
        });

        derivations.push(CompiledDerivation {
            id,
            text,
            kind: DerivationKind::ClosedWorldNegation,
            derive,
        });
    }

    derivations
}

fn compile_constraint(ir: &ConstraintIR, def: &ConstraintDef) -> CompiledConstraint {
    let modality = match def.modality.as_str() {
        "Deontic" => {
            let op = match def.deontic_operator.as_deref() {
                Some("forbidden") => DeonticOp::Forbidden,
                Some("obligatory") => DeonticOp::Obligatory,
                Some("permitted") => DeonticOp::Permitted,
                _ => DeonticOp::Obligatory,
            };
            Modality::Deontic(op)
        }
        _ => Modality::Alethic,
    };

    let predicate = match &modality {
        Modality::Deontic(DeonticOp::Permitted) => {
            Arc::new(|_: &EvalContext| Vec::new()) as Predicate
        }
        Modality::Deontic(DeonticOp::Forbidden) => compile_forbidden(ir, def),
        Modality::Deontic(DeonticOp::Obligatory) => compile_obligatory(ir, def),
        Modality::Alethic => match def.kind.as_str() {
            "UC" => compile_uniqueness(ir, def),
            "MC" => compile_mandatory(ir, def),
            "FC" => compile_frequency(ir, def),
            "VC" => compile_value_constraint(ir, def),
            // Ring constraints — each property has its own evaluator
            "IR" => compile_ring_irreflexive(def),
            "AS" => compile_ring_asymmetric(def),
            "SY" => compile_ring_symmetric(def),
            "AT" | "ANS" => compile_ring_antisymmetric(def),
            "IT" => compile_ring_intransitive(def),
            "TR" => compile_ring_transitive(def),
            "AC" => compile_ring_acyclic(def),
            "RF" => compile_ring_reflexive(ir, def),
            "XO" => compile_set_comparison(ir, def, |n| n != 1, "exactly one"),
            "XC" => compile_set_comparison(ir, def, |n| n > 1, "at most one"),
            "OR" => compile_set_comparison(ir, def, |n| n < 1, "at least one"),
            "SS" => compile_subset(ir, def),
            "EQ" => compile_equality(ir, def),
            _ => Arc::new(|_: &EvalContext| Vec::new()) as Predicate,
        },
    };

    CompiledConstraint {
        id: def.id.clone(),
        text: def.text.clone(),
        modality,
        predicate,
    }
}

// ── Alethic Predicates ─────────────────────────────────────────────
// Each returns a Predicate that captures all needed data from the IR.

fn compile_uniqueness(_ir: &ConstraintIR, def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let spans = resolve_spans(_ir, &def.spans);

    // Group spans by fact_type_id to handle compound (multi-role) UC correctly.
    // Per Halpin Ch 4: a UC spanning multiple roles means the TUPLE of values
    // at those roles must be unique, not each role independently.
    let mut groups: HashMap<String, Vec<ResolvedSpan>> = HashMap::new();
    for span in &spans {
        groups.entry(span.fact_type_id.clone()).or_default().push(span.clone());
    }
    let span_groups: Vec<(String, Vec<ResolvedSpan>)> = groups.into_iter().collect();

    Arc::new(move |ctx: &EvalContext| {
        span_groups.iter().flat_map(|(ft_id, group_spans)| {
            let facts = ctx.population.facts.get(ft_id)
                .map(|f| f.as_slice()).unwrap_or(&[]);

            if group_spans.len() == 1 {
                // Simple UC: single role — check that role's values are unique
                let span = &group_spans[0];
                let mut seen: HashMap<String, usize> = HashMap::new();
                for fact in facts {
                    if let Some((_, val)) = fact.bindings.get(span.role_index) {
                        *seen.entry(val.clone()).or_insert(0) += 1;
                    }
                }
                seen.into_iter()
                    .filter(|(_, count)| *count > 1)
                    .map(|(val, count)| Violation {
                        constraint_id: id.clone(),
                        constraint_text: text.clone(),
                        detail: format!(
                            "Uniqueness violation: {} '{}' appears {} times in {}",
                            span.noun_name, val, count, span.reading
                        ),
                    })
                    .collect::<Vec<_>>()
            } else {
                // Compound UC: multiple roles — check that the TUPLE of values is unique.
                // Per Halpin p.119 Figure 4.17(d): "No duplicate (a,b) rows are allowed"
                let role_indices: Vec<usize> = group_spans.iter().map(|s| s.role_index).collect();
                let role_names: Vec<&str> = group_spans.iter().map(|s| s.noun_name.as_str()).collect();
                let reading = &group_spans[0].reading;

                let mut seen: HashMap<String, usize> = HashMap::new();
                for fact in facts {
                    let tuple_key: String = role_indices.iter()
                        .map(|&idx| fact.bindings.get(idx).map(|(_, v)| v.as_str()).unwrap_or(""))
                        .collect::<Vec<_>>()
                        .join("|");
                    *seen.entry(tuple_key).or_insert(0) += 1;
                }

                seen.into_iter()
                    .filter(|(_, count)| *count > 1)
                    .map(|(tuple, count)| {
                        let label = role_names.join(", ");
                        Violation {
                            constraint_id: id.clone(),
                            constraint_text: text.clone(),
                            detail: format!(
                                "Uniqueness violation: ({}) combination '{}' appears {} times in {}",
                                label, tuple.replace('|', ", "), count, reading
                            ),
                        }
                    })
                    .collect::<Vec<_>>()
            }
        }).collect()
    })
}

fn compile_mandatory(_ir: &ConstraintIR, def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let spans = resolve_spans(_ir, &def.spans);

    Arc::new(move |ctx: &EvalContext| {
        spans.iter().flat_map(|span| {
            let facts = ctx.population.facts.get(&span.fact_type_id)
                .cloned().unwrap_or_default();

            // Collect all instances of this noun from ALL fact types
            let all_instances = instances_of(&span.noun_name, ctx.population);

            all_instances.into_iter()
                .filter(|instance| {
                    !facts.iter().any(|f| {
                        f.bindings.iter().any(|(name, val)| *name == span.noun_name && val == instance)
                    })
                })
                .map(|instance| Violation {
                    constraint_id: id.clone(),
                    constraint_text: text.clone(),
                    detail: format!(
                        "Mandatory violation: {} '{}' does not participate in {}",
                        span.noun_name, instance, span.reading
                    ),
                })
                .collect::<Vec<_>>()
        }).collect()
    })
}

/// VC: Value constraint — each value in the constrained role must be in the
/// noun's allowed value set (enum_values). Per Halpin Ch 6.3.
fn compile_value_constraint(ir: &ConstraintIR, def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let spans = resolve_spans(ir, &def.spans);

    // Collect allowed values from the nouns in the spanned fact types
    let allowed: Vec<(String, HashSet<String>)> = spans.iter().filter_map(|span| {
        let noun_def = ir.nouns.get(&span.noun_name)?;
        let vals = noun_def.enum_values.as_ref()?;
        if vals.is_empty() { return None; }
        Some((span.noun_name.clone(), vals.iter().cloned().collect::<HashSet<_>>()))
    }).collect();

    // If no enum values found on spanned nouns, check all nouns with enum_values
    // against their occurrences in the population (population-wide VC)
    let all_enum_nouns: Vec<(String, HashSet<String>)> = if allowed.is_empty() {
        ir.nouns.iter().filter_map(|(name, noun)| {
            let vals = noun.enum_values.as_ref()?;
            if vals.is_empty() { return None; }
            Some((name.clone(), vals.iter().cloned().collect::<HashSet<_>>()))
        }).collect()
    } else {
        Vec::new()
    };

    Arc::new(move |ctx: &EvalContext| {
        let check_nouns = if !allowed.is_empty() { &allowed } else { &all_enum_nouns };
        let mut violations = Vec::new();

        for (noun_name, valid_values) in check_nouns {
            // Scan all facts for this noun's values
            for facts in ctx.population.facts.values() {
                for fact in facts {
                    for (name, val) in &fact.bindings {
                        if name == noun_name && !valid_values.contains(val) {
                            violations.push(Violation {
                                constraint_id: id.clone(),
                                constraint_text: text.clone(),
                                detail: format!(
                                    "Value constraint violation: {} '{}' is not in {{{}}}",
                                    noun_name, val,
                                    valid_values.iter().cloned().collect::<Vec<_>>().join(", ")
                                ),
                            });
                        }
                    }
                }
            }
        }
        violations
    })
}

/// FC: Frequency constraint — each value in the constrained role must occur
/// within [min_occurrence, max_occurrence] times in the fact type's population.
/// Per Halpin Ch 7.2: generalizes UC (FC with max=1 is a UC).
fn compile_frequency(_ir: &ConstraintIR, def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let spans = resolve_spans(_ir, &def.spans);
    let min_occ = def.min_occurrence.unwrap_or(1);
    let max_occ = def.max_occurrence;

    Arc::new(move |ctx: &EvalContext| {
        spans.iter().flat_map(|span| {
            let facts = ctx.population.facts.get(&span.fact_type_id)
                .map(|f| f.as_slice()).unwrap_or(&[]);

            // Count occurrences of each value at the constrained role
            let mut counts: HashMap<String, usize> = HashMap::new();
            for fact in facts {
                if let Some((_, val)) = fact.bindings.get(span.role_index) {
                    *counts.entry(val.clone()).or_insert(0) += 1;
                }
            }

            counts.into_iter()
                .filter(|(_, count)| {
                    *count < min_occ || max_occ.map_or(false, |max| *count > max)
                })
                .map(|(val, count)| {
                    let range = match max_occ {
                        Some(max) if max == min_occ => format!("exactly {}", min_occ),
                        Some(max) => format!("between {} and {}", min_occ, max),
                        None => format!("at least {}", min_occ),
                    };
                    Violation {
                        constraint_id: id.clone(),
                        constraint_text: text.clone(),
                        detail: format!(
                            "Frequency violation: {} '{}' occurs {} times in {}, expected {}",
                            span.noun_name, val, count, span.reading, range
                        ),
                    }
                })
                .collect::<Vec<_>>()
        }).collect()
    })
}

/// Helper: collect all binary facts (pairs) from the constrained fact types.
fn ring_facts(ctx: &EvalContext, fact_type_ids: &[String]) -> Vec<(String, String)> {
    fact_type_ids.iter().flat_map(|ft_id| {
        ctx.population.facts.get(ft_id)
            .map(|f| f.as_slice()).unwrap_or(&[])
            .iter()
            .filter(|f| f.bindings.len() >= 2)
            .map(|f| (f.bindings[0].1.clone(), f.bindings[1].1.clone()))
    }).collect()
}

/// Helper: build a set of (x, y) pairs for fast lookup.
fn ring_pair_set(pairs: &[(String, String)]) -> HashSet<(String, String)> {
    pairs.iter().cloned().collect()
}

/// IR: No x such that xRx (irreflexive)
fn compile_ring_irreflexive(def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();

    Arc::new(move |ctx: &EvalContext| {
        ring_facts(ctx, &ft_ids).iter()
            .filter(|(x, y)| x == y)
            .map(|(x, _)| Violation {
                constraint_id: id.clone(),
                constraint_text: text.clone(),
                detail: format!("Irreflexive violation: '{}' references itself", x),
            })
            .collect()
    })
}

/// AS: xRy → ¬yRx (asymmetric)
fn compile_ring_asymmetric(def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();

    Arc::new(move |ctx: &EvalContext| {
        let pairs = ring_facts(ctx, &ft_ids);
        let set = ring_pair_set(&pairs);
        pairs.iter()
            .filter(|(x, y)| x != y && set.contains(&(y.clone(), x.clone())))
            .map(|(x, y)| Violation {
                constraint_id: id.clone(),
                constraint_text: text.clone(),
                detail: format!("Asymmetric violation: '{}' relates to '{}' and vice versa", x, y),
            })
            .collect()
    })
}

/// SY: xRy → yRx (symmetric) — violation when reverse is missing
fn compile_ring_symmetric(def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();

    Arc::new(move |ctx: &EvalContext| {
        let pairs = ring_facts(ctx, &ft_ids);
        let set = ring_pair_set(&pairs);
        pairs.iter()
            .filter(|(x, y)| x != y && !set.contains(&(y.clone(), x.clone())))
            .map(|(x, y)| Violation {
                constraint_id: id.clone(),
                constraint_text: text.clone(),
                detail: format!("Symmetric violation: '{}' relates to '{}' but not the reverse", x, y),
            })
            .collect()
    })
}

/// AT/ANS: xRy ∧ yRx → x = y (antisymmetric)
fn compile_ring_antisymmetric(def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();

    Arc::new(move |ctx: &EvalContext| {
        let pairs = ring_facts(ctx, &ft_ids);
        let set = ring_pair_set(&pairs);
        pairs.iter()
            .filter(|(x, y)| x != y && set.contains(&(y.clone(), x.clone())))
            .map(|(x, y)| Violation {
                constraint_id: id.clone(),
                constraint_text: text.clone(),
                detail: format!("Antisymmetric violation: '{}' and '{}' relate to each other but are not the same", x, y),
            })
            .collect()
    })
}

/// IT: xRy ∧ yRz → ¬xRz (intransitive)
fn compile_ring_intransitive(def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();

    Arc::new(move |ctx: &EvalContext| {
        let pairs = ring_facts(ctx, &ft_ids);
        let set = ring_pair_set(&pairs);
        // For each xRy, look for yRz, check if xRz exists (violation)
        let mut violations = Vec::new();
        for (x, y) in &pairs {
            for (y2, z) in &pairs {
                if y == y2 && x != z && set.contains(&(x.clone(), z.clone())) {
                    violations.push(Violation {
                        constraint_id: id.clone(),
                        constraint_text: text.clone(),
                        detail: format!(
                            "Intransitive violation: '{}'→'{}'→'{}' but '{}'→'{}' also exists",
                            x, y, z, x, z
                        ),
                    });
                }
            }
        }
        violations
    })
}

/// TR: xRy ∧ yRz → xRz (transitive) — violation when chain completion is missing
fn compile_ring_transitive(def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();

    Arc::new(move |ctx: &EvalContext| {
        let pairs = ring_facts(ctx, &ft_ids);
        let set = ring_pair_set(&pairs);
        let mut violations = Vec::new();
        for (x, y) in &pairs {
            for (y2, z) in &pairs {
                if y == y2 && x != z && !set.contains(&(x.clone(), z.clone())) {
                    violations.push(Violation {
                        constraint_id: id.clone(),
                        constraint_text: text.clone(),
                        detail: format!(
                            "Transitive violation: '{}'→'{}' and '{}'→'{}' but '{}'→'{}' is missing",
                            x, y, y, z, x, z
                        ),
                    });
                }
            }
        }
        violations
    })
}

/// AC: No cycle x₁Rx₂...xₙRx₁ (acyclic) — DFS cycle detection
fn compile_ring_acyclic(def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();

    Arc::new(move |ctx: &EvalContext| {
        let pairs = ring_facts(ctx, &ft_ids);
        // Build adjacency list
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();
        for (x, y) in &pairs {
            if x != y {
                adj.entry(x.clone()).or_default().push(y.clone());
            }
        }
        // DFS cycle detection
        let mut violations = Vec::new();
        let mut visited = HashSet::new();
        let mut in_stack = HashSet::new();

        fn dfs(
            node: &str,
            adj: &HashMap<String, Vec<String>>,
            visited: &mut HashSet<String>,
            in_stack: &mut HashSet<String>,
            cycle_nodes: &mut Vec<String>,
        ) -> bool {
            visited.insert(node.to_string());
            in_stack.insert(node.to_string());
            if let Some(neighbors) = adj.get(node) {
                for next in neighbors {
                    if !visited.contains(next.as_str()) {
                        if dfs(next, adj, visited, in_stack, cycle_nodes) {
                            cycle_nodes.push(node.to_string());
                            return true;
                        }
                    } else if in_stack.contains(next.as_str()) {
                        cycle_nodes.push(next.to_string());
                        cycle_nodes.push(node.to_string());
                        return true;
                    }
                }
            }
            in_stack.remove(node);
            false
        }

        let nodes: Vec<String> = adj.keys().cloned().collect();
        for node in &nodes {
            if !visited.contains(node.as_str()) {
                let mut cycle_nodes = Vec::new();
                if dfs(node, &adj, &mut visited, &mut in_stack, &mut cycle_nodes) {
                    cycle_nodes.reverse();
                    violations.push(Violation {
                        constraint_id: id.clone(),
                        constraint_text: text.clone(),
                        detail: format!(
                            "Acyclic violation: cycle detected through {}",
                            cycle_nodes.join(" → ")
                        ),
                    });
                }
            }
        }
        violations
    })
}

/// RF: Each x must xRx (purely reflexive) — violation when self-reference is missing
fn compile_ring_reflexive(ir: &ConstraintIR, def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();
    // Find the noun name from spans to know which instances to check
    let noun_name: String = def.spans.first()
        .and_then(|s| ir.fact_types.get(&s.fact_type_id))
        .and_then(|ft| ft.roles.first())
        .map(|r| r.noun_name.clone())
        .unwrap_or_default();

    Arc::new(move |ctx: &EvalContext| {
        let pairs = ring_facts(ctx, &ft_ids);
        let self_refs: HashSet<String> = pairs.iter()
            .filter(|(x, y)| x == y)
            .map(|(x, _)| x.clone())
            .collect();
        // Find all instances of this noun across the population
        let mut all_instances = HashSet::new();
        for facts in ctx.population.facts.values() {
            for fact in facts {
                for (noun, val) in &fact.bindings {
                    if noun == &noun_name {
                        all_instances.insert(val.clone());
                    }
                }
            }
        }
        all_instances.iter()
            .filter(|inst| !self_refs.contains(*inst))
            .map(|inst| Violation {
                constraint_id: id.clone(),
                constraint_text: text.clone(),
                detail: format!("Reflexive violation: '{}' does not reference itself", inst),
            })
            .collect()
    })
}

fn compile_set_comparison(
    _ir: &ConstraintIR,
    def: &ConstraintDef,
    violates: fn(usize) -> bool,
    requirement: &'static str,
) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let entity_name = def.entity.clone().unwrap_or_default();
    let clause_ft_ids: Vec<String> = def.spans.iter()
        .map(|s| s.fact_type_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    Arc::new(move |ctx: &EvalContext| {
        let all_instances = instances_of(&entity_name, ctx.population);

        all_instances.into_iter()
            .filter_map(|instance| {
                let holding = clause_ft_ids.iter()
                    .filter(|ft_id| participates_in(&instance, &entity_name, ft_id, ctx.population))
                    .count();

                if violates(holding) {
                    Some(Violation {
                        constraint_id: id.clone(),
                        constraint_text: text.clone(),
                        detail: format!(
                            "Set-comparison violation: {} '{}' has {} of {} clause fact types holding, expected {}",
                            entity_name, instance, holding, clause_ft_ids.len(), requirement
                        ),
                    })
                } else {
                    None
                }
            })
            .collect()
    })
}

/// SS: Subset constraint — pop(rs1) ⊆ pop(rs2).
/// For join-path subsets like "If Academic heads Department then that Academic
/// works for that Department", checks that every (Academic, Department) pair
/// in fact type A also exists in fact type B, matching by noun name.
fn compile_subset(ir: &ConstraintIR, def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();

    if def.spans.len() < 2 {
        return Arc::new(|_: &EvalContext| Vec::new());
    }

    let a_ft_id = def.spans[0].fact_type_id.clone();
    let b_ft_id = def.spans[1].fact_type_id.clone();

    // Find common noun names between the two fact types — these are the join keys
    let a_nouns: Vec<String> = ir.fact_types.get(&a_ft_id)
        .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
        .unwrap_or_default();
    let b_nouns: Vec<String> = ir.fact_types.get(&b_ft_id)
        .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
        .unwrap_or_default();
    let common_nouns: Vec<String> = a_nouns.iter()
        .filter(|n| b_nouns.contains(n))
        .cloned()
        .collect();

    Arc::new(move |ctx: &EvalContext| {
        let a_facts = ctx.population.facts.get(&a_ft_id).cloned().unwrap_or_default();
        let b_facts = ctx.population.facts.get(&b_ft_id).cloned().unwrap_or_default();

        // Build a set of value tuples from B for fast lookup
        let b_tuples: HashSet<Vec<String>> = b_facts.iter().map(|bf| {
            common_nouns.iter().map(|noun| {
                bf.bindings.iter()
                    .find(|(name, _)| name == noun)
                    .map(|(_, val)| val.clone())
                    .unwrap_or_default()
            }).collect()
        }).collect();

        // Check each fact in A — its common-noun tuple must exist in B
        a_facts.iter().filter_map(|a_fact| {
            let a_tuple: Vec<String> = common_nouns.iter().map(|noun| {
                a_fact.bindings.iter()
                    .find(|(name, _)| name == noun)
                    .map(|(_, val)| val.clone())
                    .unwrap_or_default()
            }).collect();

            if !b_tuples.contains(&a_tuple) {
                let pair_desc = common_nouns.iter().zip(a_tuple.iter())
                    .map(|(n, v)| format!("{} '{}'", n, v))
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(Violation {
                    constraint_id: id.clone(),
                    constraint_text: text.clone(),
                    detail: format!(
                        "Subset violation: ({}) participates in '{}' but not in '{}'",
                        pair_desc, a_ft_id, b_ft_id
                    ),
                })
            } else {
                None
            }
        }).collect()
    })
}

/// EQ: Equality constraint — pop(rs1) = pop(rs2) (bidirectional subset).
/// Uses tuple-based comparison same as compile_subset.
fn compile_equality(ir: &ConstraintIR, def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();

    if def.spans.len() < 2 {
        return Arc::new(|_: &EvalContext| Vec::new());
    }

    let a_ft_id = def.spans[0].fact_type_id.clone();
    let b_ft_id = def.spans[1].fact_type_id.clone();

    let a_nouns: Vec<String> = ir.fact_types.get(&a_ft_id)
        .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
        .unwrap_or_default();
    let b_nouns: Vec<String> = ir.fact_types.get(&b_ft_id)
        .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
        .unwrap_or_default();
    let common_nouns: Vec<String> = a_nouns.iter()
        .filter(|n| b_nouns.contains(n))
        .cloned()
        .collect();

    Arc::new(move |ctx: &EvalContext| {
        let a_facts = ctx.population.facts.get(&a_ft_id).cloned().unwrap_or_default();
        let b_facts = ctx.population.facts.get(&b_ft_id).cloned().unwrap_or_default();

        let extract_tuples = |facts: &[FactInstance]| -> HashSet<Vec<String>> {
            facts.iter().map(|f| {
                common_nouns.iter().map(|noun| {
                    f.bindings.iter()
                        .find(|(name, _)| name == noun)
                        .map(|(_, val)| val.clone())
                        .unwrap_or_default()
                }).collect()
            }).collect()
        };

        let a_tuples = extract_tuples(&a_facts);
        let b_tuples = extract_tuples(&b_facts);

        let mut violations = Vec::new();

        for tuple in a_tuples.difference(&b_tuples) {
            let desc = common_nouns.iter().zip(tuple.iter())
                .map(|(n, v)| format!("{} '{}'", n, v))
                .collect::<Vec<_>>().join(", ");
            violations.push(Violation {
                constraint_id: id.clone(),
                constraint_text: text.clone(),
                detail: format!("Equality violation: ({}) in '{}' but not in '{}'", desc, a_ft_id, b_ft_id),
            });
        }

        for tuple in b_tuples.difference(&a_tuples) {
            let desc = common_nouns.iter().zip(tuple.iter())
                .map(|(n, v)| format!("{} '{}'", n, v))
                .collect::<Vec<_>>().join(", ");
            violations.push(Violation {
                constraint_id: id.clone(),
                constraint_text: text.clone(),
                detail: format!("Equality violation: ({}) in '{}' but not in '{}'", desc, b_ft_id, a_ft_id),
            });
        }

        violations
    })
}

// ── Deontic Predicates ─────────────────────────────────────────────

fn compile_forbidden(ir: &ConstraintIR, def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let forbidden_values = collect_enum_values(ir, &def.spans);
    let entity_scope = def.entity.clone();

    // Extract key phrases from the constraint text for text-based matching.
    // "It is forbidden that Customer resells AutomotiveData to ThirdParty"
    // → extract nouns and verbs as keywords to detect violations.
    let text_keywords = extract_constraint_keywords(&text);

    Arc::new(move |ctx: &EvalContext| {
        let mut violations = Vec::new();
        let mut seen = HashSet::new();
        let lower_text = ctx.response.text.to_lowercase();

        // Entity scoping: only evaluate when the response references the scoped entity
        if let Some(ref entity) = entity_scope {
            let entity_lower = entity.to_lowercase();
            let entity_compact: String = entity.chars().filter(|c| !c.is_whitespace()).collect();
            let entity_compact_lower = entity_compact.to_lowercase();
            if !lower_text.contains(&entity_lower) && !lower_text.contains(&entity_compact_lower) {
                return violations;
            }
        }

        // Enum-based check (exact value match)
        for (noun_name, enum_vals) in &forbidden_values {
            for val in enum_vals {
                let lower_val = val.to_lowercase();
                if lower_text.contains(&lower_val) {
                    let detail = format!(
                        "Response contains forbidden {}: '{}'",
                        noun_name, val
                    );
                    if seen.insert(detail.clone()) {
                        violations.push(Violation {
                            constraint_id: id.clone(),
                            constraint_text: text.clone(),
                            detail,
                        });
                    }
                }
            }
        }

        // Text-based check: if no enum values, check keyword co-occurrence
        if forbidden_values.is_empty() && !text_keywords.is_empty() {
            let matched: Vec<&str> = text_keywords.iter()
                .filter(|kw| lower_text.contains(kw.as_str()))
                .map(|s| s.as_str())
                .collect();
            // Trigger if majority of keywords found (suggests the response discusses the forbidden topic)
            if matched.len() > text_keywords.len() / 2 && matched.len() >= 2 {
                violations.push(Violation {
                    constraint_id: id.clone(),
                    constraint_text: text.clone(),
                    detail: format!(
                        "Response may violate: '{}' (matched keywords: {})",
                        text, matched.join(", ")
                    ),
                });
            }
        }

        violations
    })
}

/// Extract lowercase keywords from a deontic constraint text.
/// Strips the "It is forbidden/obligatory/permitted that" prefix,
/// then extracts PascalCase and multi-word noun phrases.
fn extract_constraint_keywords(text: &str) -> Vec<String> {
    let stripped = text
        .replace("It is forbidden that ", "")
        .replace("It is obligatory that ", "")
        .replace("It is permitted that ", "");

    let mut keywords = Vec::new();
    for word in stripped.split_whitespace() {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric());
        if clean.is_empty() { continue; }
        // Split PascalCase: AutomotiveData → automotive, data
        let mut current = String::new();
        for ch in clean.chars() {
            if ch.is_uppercase() && !current.is_empty() {
                let lower = current.to_lowercase();
                if lower.len() > 2 { keywords.push(lower); }
                current.clear();
            }
            current.push(ch);
        }
        if !current.is_empty() {
            let lower = current.to_lowercase();
            if lower.len() > 2 { keywords.push(lower); }
        }
    }

    // Deduplicate
    keywords.sort();
    keywords.dedup();
    // Filter out common stop words
    keywords.retain(|w| !matches!(w.as_str(), "the" | "that" | "for" | "and" | "with" | "without" | "using" | "has" | "have" | "into" | "from"));
    keywords
}

fn compile_obligatory(ir: &ConstraintIR, def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let obligatory_values = collect_enum_values(ir, &def.spans);
    let checks_sender = def.text.to_lowercase().contains("senderidentity");
    let entity_scope = def.entity.clone();

    // For text-based obligatory constraints, the constraint text itself is included
    // in the compiled model for semantic evaluation by the LLM layer.
    // WASM flags it as a rule the response should acknowledge.
    let text_keywords = if obligatory_values.is_empty() {
        extract_constraint_keywords(&text)
    } else {
        Vec::new()
    };

    Arc::new(move |ctx: &EvalContext| {
        let mut violations = Vec::new();
        let lower_text = ctx.response.text.to_lowercase();

        // Entity scoping: if this constraint is scoped to an entity, only evaluate
        // when the response text mentions that entity. A constraint about User/Organization
        // should not fire on text about the weather.
        if let Some(ref entity) = entity_scope {
            let entity_lower = entity.to_lowercase();
            // Also check for multi-word entity names with spaces removed (e.g. "OrgRole")
            let entity_compact: String = entity.chars().filter(|c| !c.is_whitespace()).collect();
            let entity_compact_lower = entity_compact.to_lowercase();
            if !lower_text.contains(&entity_lower) && !lower_text.contains(&entity_compact_lower) {
                return violations; // entity not relevant to this text
            }
        }

        // Enum-based check
        for (noun_name, enum_vals) in &obligatory_values {
            let found = enum_vals.iter().any(|val| lower_text.contains(&val.to_lowercase()));
            if !found {
                violations.push(Violation {
                    constraint_id: id.clone(),
                    constraint_text: text.clone(),
                    detail: format!(
                        "Response missing obligatory {}: expected one of {:?}",
                        noun_name, enum_vals
                    ),
                });
            }
        }

        // Sender identity check
        if checks_sender {
            if let Some(sender) = &ctx.response.sender_identity {
                if sender.is_empty() {
                    violations.push(Violation {
                        constraint_id: id.clone(),
                        constraint_text: text.clone(),
                        detail: "Response missing obligatory SenderIdentity".to_string(),
                    });
                }
            }
        }

        // Text-based: include the obligation as metadata so the LLM layer can evaluate
        // (WASM can't determine semantic compliance, but it flags the rule exists)
        let _ = &text_keywords; // available for future keyword checks

        violations
    })
}

// ── State Machine Compilation ──────────────────────────────────────
// State machines compile to transition functions.
// run_machine = fold(transition)(initial)(stream)

struct CompiledTransition {
    from: String,
    to: String,
    event: String,
    guard: Predicate,
}

fn compile_state_machine(
    def: &StateMachineDef,
    constraint_predicates: &HashMap<String, Predicate>,
) -> CompiledStateMachine {
    // Build transition table for introspection
    let transition_table: Vec<(String, String, String)> = def.transitions.iter()
        .map(|t| (t.from.clone(), t.to.clone(), t.event.clone()))
        .collect();

    let transitions: Vec<CompiledTransition> = def.transitions.iter()
        .map(|t| {
            let guard_preds: Vec<Predicate> = t.guard.as_ref()
                .map(|g| g.constraint_ids.iter()
                    .filter_map(|cid| constraint_predicates.get(cid).cloned())
                    .collect())
                .unwrap_or_default();

            // Guard passes iff all constraint predicates produce zero violations
            let guard: Predicate = Arc::new(move |ctx: &EvalContext| {
                guard_preds.iter()
                    .flat_map(|p| p(ctx))
                    .collect()
            });

            CompiledTransition {
                from: t.from.clone(),
                to: t.to.clone(),
                event: t.event.clone(),
                guard,
            }
        })
        .collect();

    let initial = def.statuses.first().cloned().unwrap_or_default();

    // Transition function: find first matching (from, event) where guard passes
    let transition_fn: Arc<dyn Fn(&str, &str, &EvalContext) -> Option<String> + Send + Sync> =
        Arc::new(move |state: &str, event: &str, ctx: &EvalContext| {
            transitions.iter()
                .find(|t| t.from == state && t.event == event && (t.guard)(ctx).is_empty())
                .map(|t| t.to.clone())
        });

    CompiledStateMachine {
        noun_name: def.noun_name.clone(),
        statuses: def.statuses.clone(),
        initial,
        transition: transition_fn,
        transition_table,
    }
}
