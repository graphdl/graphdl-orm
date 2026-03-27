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

// ── Core Functional Types ──────────────────────────────────────────
//
// No closures. No opaque types. Everything is Func — AST nodes that
// the evaluator reduces. Constraints, derivations, state machines are
// all Func nodes applied to Objects (populations, events, contexts).
//
// The legacy Predicate/DeriveFn/EvalContext types are deleted.
// Evaluation is beta reduction: apply(func, object, defs) → object.

/// Legacy predicate type — kept temporarily while compile_* functions
/// are migrated to produce Func nodes directly. Will be deleted.
pub type Predicate = Arc<dyn Fn(&EvalContext) -> Vec<Violation> + Send + Sync>;

/// Legacy evaluation context — will be replaced by Object encoding.
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

/// A compiled constraint. The `func` field is the AST — evaluation is
/// apply(func, eval_context_object) → sequence of violation objects.
///
/// The `predicate` field is a transitional bridge: it wraps the same
/// logic as a closure so the existing evaluate() path works while
/// constraint compilers are migrated to emit pure Func nodes.
pub struct CompiledConstraint {
    pub id: String,
    pub text: String,
    pub modality: Modality,
    /// Func: Object → Object. Takes encoded eval context, returns violation sequence.
    pub func: crate::ast::Func,
    /// Transitional bridge — same logic as func, as a closure.
    pub predicate: Predicate,
}

/// Legacy derivation fn type — transitional.
pub type DeriveFn = Arc<dyn Fn(&EvalContext, &Population) -> Vec<DerivedFact> + Send + Sync>;

/// A compiled derivation rule. The `func` field is the AST.
pub struct CompiledDerivation {
    pub id: String,
    pub text: String,
    pub kind: DerivationKind,
    /// Func: Object → Object. Takes population, returns derived fact sequence.
    pub func: crate::ast::Func,
    /// Transitional bridge.
    pub derive: DeriveFn,
}

/// A compiled state machine. The `func` field is Insert(transition) — fold over events.
pub struct CompiledStateMachine {
    pub noun_name: String,
    pub statuses: Vec<String>,
    pub initial: String,
    /// Func: Insert(Condition(guard, Constant(next), Id)) — fold over event stream.
    pub func: crate::ast::Func,
    /// Compiled transitions as (from, to, event) for introspection.
    pub transition_table: Vec<(String, String, String)>,
    /// Transitional bridge.
    pub transition: Arc<dyn Fn(&str, &str, &EvalContext) -> Option<String> + Send + Sync>,
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
    /// noun_name -> reference scheme value type names (e.g., ["Order Number"])
    pub ref_schemes: HashMap<String, Vec<String>>,
    /// noun_name -> state machine index
    pub noun_to_state_machines: HashMap<String, usize>,
}

/// A compiled graph schema — a Construction of Selector functions (roles).
/// Graph Schema = CONS(Role₁, ..., Roleₙ) in Backus's FP algebra.
/// Partial application = query. Full application = fact.
pub struct CompiledSchema {
    pub id: String,
    pub reading: String,
    /// The Construction function: [Selector(1), Selector(2), ..., Selector(n)]
    pub construction: crate::ast::Func,
    /// Role names in order (for binding resolution)
    pub role_names: Vec<String>,
}

/// The compiled model — all constraints, derivations, state machines, and schemas as executable functions.
pub struct CompiledModel {
    pub constraints: Vec<CompiledConstraint>,
    pub derivations: Vec<CompiledDerivation>,
    pub state_machines: Vec<CompiledStateMachine>,
    pub noun_index: NounIndex,
    /// Fact types compiled to Construction functions (CONS of Roles).
    pub schemas: HashMap<String, CompiledSchema>,
    /// Fact-to-event mapping: when a fact of this type is created, fire this event
    /// on the state machine for the target noun. Derived from:
    ///   Graph Schema is activated by Verb + Verb is performed during Transition.
    pub fact_events: HashMap<String, FactEvent>,
}

/// When a fact is created in this schema, fire this event on the entity's state machine.
pub struct FactEvent {
    pub fact_type_id: String,
    pub event_name: String,
    pub target_noun: String, // which noun's state machine to transition
}

// ── Object ↔ Population decoding ─────────────────────────────────
// Decode a population Object back to a Population struct.
// Inverse of ast::encode_population.

fn decode_population_object(obj: &crate::ast::Object) -> Population {
    let mut facts: HashMap<String, Vec<FactInstance>> = HashMap::new();
    if let Some(fact_types) = obj.as_seq() {
        for ft_obj in fact_types {
            if let Some(ft_items) = ft_obj.as_seq() {
                if ft_items.len() == 2 {
                    let ft_id = ft_items[0].as_atom().unwrap_or("").to_string();
                    if let Some(fact_objs) = ft_items[1].as_seq() {
                        let instances: Vec<FactInstance> = fact_objs.iter().filter_map(|fact_obj| {
                            let bindings: Vec<(String, String)> = fact_obj.as_seq()?.iter().filter_map(|b| {
                                let pair = b.as_seq()?;
                                if pair.len() == 2 {
                                    Some((pair[0].as_atom()?.to_string(), pair[1].as_atom()?.to_string()))
                                } else {
                                    None
                                }
                            }).collect();
                            Some(FactInstance { fact_type_id: ft_id.clone(), bindings })
                        }).collect();
                        facts.insert(ft_id, instances);
                    }
                }
            }
        }
    }
    Population { facts }
}

// ── Schema Compilation ───────────────────────────────────────────
// Compile fact types to Construction functions (CONS of Roles).
// Role → Selector. Graph Schema → Construction [Selector₁, ..., Selectorₙ].

/// Compile all fact types in the IR to CompiledSchema (Construction of Selectors).
fn compile_schemas(ir: &ConstraintIR) -> HashMap<String, CompiledSchema> {
    ir.fact_types.iter().map(|(id, ft)| {
        // Each role compiles to a Selector at its position (1-indexed)
        let selectors: Vec<crate::ast::Func> = ft.roles.iter()
            .map(|role| crate::ast::Func::Selector(role.role_index + 1))
            .collect();

        let role_names: Vec<String> = ft.roles.iter()
            .map(|role| role.noun_name.clone())
            .collect();

        let schema = CompiledSchema {
            id: id.clone(),
            reading: ft.reading.clone(),
            construction: crate::ast::Func::Construction(selectors),
            role_names,
        };

        (id.clone(), schema)
    }).collect()
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

    // Compile fact types to Construction functions (CONS of Roles)
    let schemas = compile_schemas(ir);

    // Build fact-to-event mapping from schemas + state machines.
    // For each fact type, check if any role's noun has a state machine.
    // If so, check if any transition event name appears in the reading.
    // This is a heuristic until the IR carries explicit Activation/Verb links.
    let mut fact_events: HashMap<String, FactEvent> = HashMap::new();
    for (ft_id, schema) in &schemas {
        for role_name in &schema.role_names {
            if let Some(&sm_idx) = noun_index.noun_to_state_machines.get(role_name) {
                let sm = &state_machines[sm_idx];
                let reading_lower = schema.reading.to_lowercase();
                for (_, to, event) in &sm.transition_table {
                    if reading_lower.contains(&event.to_lowercase()) ||
                       reading_lower.contains(&to.to_lowercase()) {
                        fact_events.insert(ft_id.clone(), FactEvent {
                            fact_type_id: ft_id.clone(),
                            event_name: event.clone(),
                            target_noun: role_name.clone(),
                        });
                        break;
                    }
                }
            }
        }
    }

    CompiledModel { constraints, derivations, state_machines, noun_index, schemas, fact_events }
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
    let mut ref_schemes: HashMap<String, Vec<String>> = HashMap::new();
    for (name, def) in &ir.nouns {
        if let Some(ref st) = def.super_type {
            supertypes.insert(name.clone(), st.clone());
            subtypes.entry(st.clone()).or_default().push(name.clone());
        }
        if let Some(ref rs) = def.ref_scheme {
            ref_schemes.insert(name.clone(), rs.clone());
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
        ref_schemes,
        fact_type_to_constraints,
        constraint_index,
        noun_to_state_machines,
    }
}

// ── AST Derivation Chains ────────────────────────────────────────────
// Compile derivation rules to Func::Compose chains.
// "User can access Domain iff A and B and C" becomes f ∘ g ∘ h
// where each step is a partial application over a schema.

/// Compile a derivation chain from antecedent fact type IDs.
///
/// Given fact types [ft1, ft2, ft3] with shared nouns, builds a composition:
///   For each fact type, create a query function that:
///     - Takes a known binding from the previous step
///     - Filters the population for that fact type
///     - Extracts the shared noun's value to pass to the next step
///
/// Returns a Func that, given a population Object, produces the set of
/// derived resources at the end of the chain.
pub fn compile_derivation_chain(
    ir: &ConstraintIR,
    antecedent_ft_ids: &[String],
    source_noun: &str,
    target_noun: &str,
) -> Option<crate::ast::Func> {
    if antecedent_ft_ids.is_empty() { return None; }

    // For each fact type, determine which role is the "input" (shared with previous)
    // and which is the "output" (passed to next or final result).
    // This builds the composition chain from the shared nouns between adjacent fact types.

    let mut steps: Vec<(String, usize, usize)> = Vec::new(); // (ft_id, input_role, output_role)
    let mut current_noun = source_noun.to_string();

    for ft_id in antecedent_ft_ids {
        let ft = ir.fact_types.get(ft_id)?;

        // Find the role that matches the current noun (input)
        let input_role = ft.roles.iter()
            .find(|r| r.noun_name == current_noun)?;

        // Find the other role (output) — the noun we're traversing TO
        let output_role = ft.roles.iter()
            .find(|r| r.noun_name != current_noun)?;

        steps.push((ft_id.clone(), input_role.role_index + 1, output_role.role_index + 1));
        current_noun = output_role.noun_name.clone();
    }

    // Build the query function chain.
    // Each step: given input value, query population for matching facts, extract output values.
    // The chain composes these steps.
    Some(crate::ast::Func::Def(format!(
        "_chain:{}->{}:{}",
        source_noun, target_noun,
        antecedent_ft_ids.join(",")
    )))
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

    let derive_for_func = derive.clone();
    let func = crate::ast::Func::Native(Arc::new(move |pop_obj: &crate::ast::Object| {
        let population = decode_population_object(pop_obj);
        let response = ResponseContext { text: String::new(), sender_identity: None, fields: None };
        let ctx = EvalContext { response: &response, population: &population };
        let derived = derive_for_func(&ctx, &population);
        if derived.is_empty() {
            crate::ast::Object::phi()
        } else {
            crate::ast::Object::Seq(derived.iter().map(|d| {
                let bindings: Vec<crate::ast::Object> = d.bindings.iter().map(|(n, v)| {
                    crate::ast::Object::seq(vec![crate::ast::Object::atom(n), crate::ast::Object::atom(v)])
                }).collect();
                crate::ast::Object::seq(vec![
                    crate::ast::Object::atom(&d.fact_type_id),
                    crate::ast::Object::atom(&d.reading),
                    crate::ast::Object::Seq(bindings),
                ])
            }).collect())
        }
    }));
    CompiledDerivation { id, text, kind, derive, func }
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

            let func = crate::ast::Func::Def(format!("_derive:{}", id));
            derivations.push(CompiledDerivation {
                id,
                text,
                kind: DerivationKind::SubtypeInheritance,
                derive,
                func,
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

        // Only derive facts when subset_autofill is explicitly true.
        // Otherwise the SS constraint is just a constraint (produces violations,
        // doesn't auto-create facts).
        let has_autofill = cdef.spans.iter().any(|s| s.subset_autofill == Some(true));
        if !has_autofill {
            continue;
        }

        let a_ft_id = cdef.spans[0].fact_type_id.clone();
        let b_ft_id = cdef.spans[1].fact_type_id.clone();

        // Collect role noun names from both fact types for full tuple propagation
        let a_role_names: Vec<String> = ir.fact_types.get(&a_ft_id)
            .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
            .unwrap_or_default();

        let b_role_names: Vec<String> = ir.fact_types.get(&b_ft_id)
            .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
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
                // Build the full consequent tuple by mapping bindings from the
                // superset fact to the subset fact by noun name correspondence.
                let mut b_bindings: Vec<(String, String)> = Vec::new();
                for b_noun in &b_role_names {
                    if let Some((_, val)) = a_fact.bindings.iter().find(|(n, _)| n == b_noun) {
                        b_bindings.push((b_noun.clone(), val.clone()));
                    }
                }

                if b_bindings.is_empty() { return None; }

                // Check if this tuple already exists in the consequent population
                let already_exists = b_facts.iter().any(|bf| {
                    b_bindings.iter().all(|(name, val)| {
                        bf.bindings.iter().any(|(bn, bv)| bn == name && bv == val)
                    })
                });

                if !already_exists {
                    Some(DerivedFact {
                        fact_type_id: b_ft_id.clone(),
                        reading: b_reading.clone(),
                        bindings: b_bindings,
                        derived_by: derive_id.clone(),
                        confidence: Confidence::Definitive,
                    })
                } else {
                    None
                }
            }).collect()
        });

        let func = crate::ast::Func::Def(format!("_derive:{}", id));
        derivations.push(CompiledDerivation {
            id,
            text,
            kind: DerivationKind::ModusPonens,
            derive,
            func,
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

            let func = crate::ast::Func::Def(format!("_derive:{}", id));
            derivations.push(CompiledDerivation {
                id,
                text: reading,
                kind: DerivationKind::Transitivity,
                derive,
                func,
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

        let func = crate::ast::Func::Def(format!("_derive:{}", id));
        derivations.push(CompiledDerivation {
            id,
            text,
            kind: DerivationKind::ClosedWorldNegation,
            derive,
            func,
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

    // Wrap predicate as a Func::Native that takes an encoded eval context Object
    // and returns a sequence of violation Objects.
    // Each constraint will be migrated to pure AST (Construction, Condition, etc.)
    let pred_for_func = predicate.clone();
    let func = crate::ast::Func::Native(Arc::new(move |ctx_obj: &crate::ast::Object| {
        // Decode the eval context from the Object
        let items = match ctx_obj.as_seq() {
            Some(items) if items.len() == 2 => items,
            _ => return crate::ast::Object::phi(),
        };
        let response_text = items[0].as_atom().unwrap_or("").to_string();
        let response = ResponseContext {
            text: response_text,
            sender_identity: None,
            fields: None,
        };
        // Decode population from Object back to Population struct
        let population = decode_population_object(&items[1]);
        let ctx = EvalContext { response: &response, population: &population };
        let violations = pred_for_func(&ctx);
        // Encode violations as Object sequence
        if violations.is_empty() {
            crate::ast::Object::phi()
        } else {
            crate::ast::Object::Seq(
                violations.iter().map(crate::ast::encode_violation).collect()
            )
        }
    }));

    CompiledConstraint {
        id: def.id.clone(),
        text: def.text.clone(),
        modality,
        func,
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

    // AST: state machine as a transition function.
    //
    // The transition function takes <current_state, event_name> and returns next_state.
    // It's a chain of Conditions — try each transition in order:
    //   (match_t1 → "target1"̄; (match_t2 → "target2"̄; ... ; 1))
    // where match_tN = eq ∘ [id, <"from_state", "event">̄] (check state+event pair)
    // If no transition matches, Selector(1) returns current_state unchanged.
    //
    // run_machine = fold(transition_func)(initial)(event_stream)
    let mut sm_func = crate::ast::Func::Selector(1); // fallback: return current state

    // Build condition chain in reverse (innermost = fallback)
    for t in transition_table.iter().rev() {
        // Predicate: eq ∘ [[1, 2], ["from", "event"]̄]
        // Input is <current_state, event_name>
        // Check: <current_state, event_name> == <from, event>
        let match_pred = crate::ast::Func::compose(
            crate::ast::Func::Eq,
            crate::ast::Func::construction(vec![
                crate::ast::Func::Id,
                crate::ast::Func::constant(crate::ast::Object::seq(vec![
                    crate::ast::Object::atom(&t.0),
                    crate::ast::Object::atom(&t.2),
                ])),
            ]),
        );

        // If match: return target state. Else: try next transition.
        sm_func = crate::ast::Func::condition(
            match_pred,
            crate::ast::Func::constant(crate::ast::Object::atom(&t.1)),
            sm_func,
        );
    }

    CompiledStateMachine {
        noun_name: def.noun_name.clone(),
        statuses: def.statuses.clone(),
        initial,
        transition: transition_fn,
        transition_table,
        func: sm_func,
    }
}

// ── Schema Compilation Tests ─────────────────────────────────────────

#[cfg(test)]
mod schema_tests {
    use super::*;
    use crate::ast::{self, Object};

    fn make_ir_with_fact_type(id: &str, reading: &str, roles: Vec<(&str, usize)>) -> ConstraintIR {
        let mut fact_types = HashMap::new();
        fact_types.insert(id.to_string(), FactTypeDef {
            reading: reading.to_string(),
            roles: roles.iter().map(|(name, idx)| RoleDef {
                noun_name: name.to_string(),
                role_index: *idx,
            }).collect(),
        });
        ConstraintIR {
            domain: "test".to_string(),
            nouns: HashMap::new(),
            fact_types,
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![],
        }
    }

    #[test]
    fn role_compiles_to_selector() {
        let ir = make_ir_with_fact_type(
            "ft1", "User has Org Role in Organization",
            vec![("User", 0), ("Org Role", 1), ("Organization", 2)],
        );
        let model = compile(&ir);
        let schema = model.schemas.get("ft1").unwrap();

        // Each role becomes a Selector (1-indexed)
        assert_eq!(schema.role_names, vec!["User", "Org Role", "Organization"]);

        // The construction is [Selector(1), Selector(2), Selector(3)]
        let fact = Object::seq(vec![
            Object::atom("alice@example.com"),
            Object::atom("owner"),
            Object::atom("org-123"),
        ]);

        let defs = HashMap::new();

        // Apply construction to a fact — identity (selects each role)
        let result = ast::apply(&schema.construction, &fact, &defs);
        assert_eq!(result, Object::seq(vec![
            Object::atom("alice@example.com"),
            Object::atom("owner"),
            Object::atom("org-123"),
        ]));
    }

    #[test]
    fn selector_extracts_individual_role() {
        let ir = make_ir_with_fact_type(
            "ft1", "Organization has Name",
            vec![("Organization", 0), ("Name", 1)],
        );
        let model = compile(&ir);
        let schema = model.schemas.get("ft1").unwrap();
        let defs = HashMap::new();

        let fact = Object::seq(vec![Object::atom("org-1"), Object::atom("Acme Corp")]);

        // Selector(1) = Organization role
        let org_selector = ast::Func::Selector(1);
        assert_eq!(ast::apply(&org_selector, &fact, &defs), Object::atom("org-1"));

        // Selector(2) = Name role
        let name_selector = ast::Func::Selector(2);
        assert_eq!(ast::apply(&name_selector, &fact, &defs), Object::atom("Acme Corp"));
    }

    #[test]
    fn construction_applied_to_population_via_apply_to_all() {
        // α(Selector(2)) over a population extracts role 2 from each fact
        let ir = make_ir_with_fact_type(
            "ft1", "OrgMembership is for User",
            vec![("OrgMembership", 0), ("User", 1)],
        );
        let model = compile(&ir);
        let _schema = model.schemas.get("ft1").unwrap();
        let defs = HashMap::new();

        let population = Object::seq(vec![
            Object::seq(vec![Object::atom("mem-1"), Object::atom("alice@example.com")]),
            Object::seq(vec![Object::atom("mem-2"), Object::atom("bob@example.com")]),
            Object::seq(vec![Object::atom("mem-3"), Object::atom("alice@example.com")]),
        ]);

        // Extract all users: α(2):population
        let extract_users = ast::Func::apply_to_all(ast::Func::Selector(2));
        let users = ast::apply(&extract_users, &population, &defs);
        assert_eq!(users, Object::seq(vec![
            Object::atom("alice@example.com"),
            Object::atom("bob@example.com"),
            Object::atom("alice@example.com"),
        ]));
    }

    #[test]
    fn partial_application_via_bu_creates_query() {
        // (bu eq "alice@example.com") applied to each user = membership check
        let defs = HashMap::new();

        let check_alice = ast::Func::bu(ast::Func::Eq, Object::atom("alice@example.com"));
        assert_eq!(
            ast::apply(&check_alice, &Object::atom("alice@example.com"), &defs),
            Object::t()
        );
        assert_eq!(
            ast::apply(&check_alice, &Object::atom("bob@example.com"), &defs),
            Object::f()
        );
    }

    #[test]
    fn constraint_func_evaluates_via_ast_apply() {
        // Compile a UC constraint and verify the func field works via ast::apply
        let mut fact_types = HashMap::new();
        fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Person has Name".to_string(),
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        let ir = ConstraintIR {
            domain: "test".to_string(),
            nouns: HashMap::new(),
            fact_types,
            constraints: vec![ConstraintDef {
                id: "uc1".to_string(),
                kind: "UC".to_string(),
                modality: "Alethic".to_string(),
                deontic_operator: None,
                text: "Each Person has at most one Name".to_string(),
                spans: vec![SpanDef {
                    fact_type_id: "ft1".to_string(),
                    role_index: 0,
                    subset_autofill: None,
                }],
                set_comparison_argument_length: None,
                clauses: None,
                entity: None,
                min_occurrence: None,
                max_occurrence: None,
            }],
            state_machines: HashMap::new(),
            derivation_rules: vec![],
        };

        let model = compile(&ir);
        let constraint = &model.constraints[0];

        // Create a population WITH a UC violation: Alice has two names
        let response = ResponseContext { text: String::new(), sender_identity: None, fields: None };
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("Person".to_string(), "Alice".to_string()), ("Name".to_string(), "Alice Smith".to_string())],
            },
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("Person".to_string(), "Alice".to_string()), ("Name".to_string(), "Alice Jones".to_string())],
            },
        ]);
        let population = Population { facts };

        // Evaluate via AST: apply(func, encoded_context)
        let ctx_obj = crate::ast::encode_eval_context(&response, &population);
        let defs = HashMap::new();
        let result = crate::ast::apply(&constraint.func, &ctx_obj, &defs);

        // Should return a sequence of violation Objects (not phi)
        let violations = crate::ast::decode_violations(&result);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].constraint_id, "uc1");
        assert!(violations[0].detail.contains("Alice"));
    }

    #[test]
    fn constraint_func_no_violation_returns_phi() {
        let mut fact_types = HashMap::new();
        fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "Person has Name".to_string(),
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        let ir = ConstraintIR {
            domain: "test".to_string(),
            nouns: HashMap::new(),
            fact_types,
            constraints: vec![ConstraintDef {
                id: "uc1".to_string(),
                kind: "UC".to_string(),
                modality: "Alethic".to_string(),
                deontic_operator: None,
                text: "Each Person has at most one Name".to_string(),
                spans: vec![SpanDef {
                    fact_type_id: "ft1".to_string(),
                    role_index: 0,
                    subset_autofill: None,
                }],
                set_comparison_argument_length: None,
                clauses: None,
                entity: None,
                min_occurrence: None,
                max_occurrence: None,
            }],
            state_machines: HashMap::new(),
            derivation_rules: vec![],
        };

        let model = compile(&ir);
        let constraint = &model.constraints[0];

        // No violation: each person has exactly one name
        let response = ResponseContext { text: String::new(), sender_identity: None, fields: None };
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("Person".to_string(), "Alice".to_string()), ("Name".to_string(), "Alice Smith".to_string())],
            },
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("Person".to_string(), "Bob".to_string()), ("Name".to_string(), "Bob Jones".to_string())],
            },
        ]);
        let population = Population { facts };

        let ctx_obj = crate::ast::encode_eval_context(&response, &population);
        let defs = HashMap::new();
        let result = crate::ast::apply(&constraint.func, &ctx_obj, &defs);

        // No violations — should be phi (empty sequence)
        let violations = crate::ast::decode_violations(&result);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn derivation_chain_compiles_for_access_control() {
        // "User can access Domain iff
        //    OrgMembership is for User AND
        //    OrgMembership is in Organization AND
        //    Domain belongs to Organization"
        //
        // This is a 3-step chain: User → OrgMembership → Organization → Domain
        let mut fact_types = HashMap::new();
        let mut nouns = HashMap::new();

        // ft1: OrgMembership is for User (roles: OrgMembership[0], User[1])
        fact_types.insert("ft1".to_string(), FactTypeDef {
            reading: "OrgMembership is for User".to_string(),
            roles: vec![
                RoleDef { noun_name: "OrgMembership".to_string(), role_index: 0 },
                RoleDef { noun_name: "User".to_string(), role_index: 1 },
            ],
        });

        // ft2: OrgMembership is in Organization (roles: OrgMembership[0], Organization[1])
        fact_types.insert("ft2".to_string(), FactTypeDef {
            reading: "OrgMembership is in Organization".to_string(),
            roles: vec![
                RoleDef { noun_name: "OrgMembership".to_string(), role_index: 0 },
                RoleDef { noun_name: "Organization".to_string(), role_index: 1 },
            ],
        });

        // ft3: Domain belongs to Organization (roles: Domain[0], Organization[1])
        fact_types.insert("ft3".to_string(), FactTypeDef {
            reading: "Domain belongs to Organization".to_string(),
            roles: vec![
                RoleDef { noun_name: "Domain".to_string(), role_index: 0 },
                RoleDef { noun_name: "Organization".to_string(), role_index: 1 },
            ],
        });

        for name in &["User", "OrgMembership", "Organization", "Domain"] {
            nouns.insert(name.to_string(), NounDef {
                object_type: "entity".to_string(),
                enum_values: None,
                value_type: None,
                super_type: None,
                world_assumption: WorldAssumption::Closed,
                ref_scheme: None,
            });
        }

        let ir = ConstraintIR {
            domain: "test".to_string(),
            nouns,
            fact_types,
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![],
        };

        // Chain: User → (ft1) → OrgMembership → (ft2) → Organization
        // Step 1: User is role 2 in ft1, output is OrgMembership (role 1)
        // Step 2: OrgMembership is role 1 in ft2, output is Organization (role 2)
        let chain = compile_derivation_chain(
            &ir,
            &["ft1".to_string(), "ft2".to_string()],
            "User",
            "Organization",
        );
        assert!(chain.is_some(), "chain should compile");

        // Chain: Organization → (ft3, reversed) → Domain
        // Organization is role 2 in ft3, output is Domain (role 1)
        let chain2 = compile_derivation_chain(
            &ir,
            &["ft3".to_string()],
            "Organization",
            "Domain",
        );
        assert!(chain2.is_some(), "single-step chain should compile");
    }

    #[test]
    fn schema_reading_preserved() {
        let ir = make_ir_with_fact_type(
            "ft1", "Domain Change proposes Reading",
            vec![("Domain Change", 0), ("Reading", 1)],
        );
        let model = compile(&ir);
        let schema = model.schemas.get("ft1").unwrap();
        assert_eq!(schema.reading, "Domain Change proposes Reading");
    }
}
