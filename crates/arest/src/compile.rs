// crates/arest/src/compile.rs
//
// Compilation: Domain â†’ CompiledModel
//
// Constraints ARE predicates, not data that gets matched.
// The match on constraint kind happens once at compile time. After compilation,
// evaluation is pure function application â€” no dispatch, no branching on kind.
//
// This implements Backus's FP algebra (1977 Turing Lecture):
//   - Constraints and derivations compile to pure functions (combining forms)
//   - Evaluation is function application over whole structures
//   - State machines are folds: run_machine = fold(transition)(initial)(stream)
//   - No variables, no mutable state during evaluation â€” only reduction

use std::sync::Arc;
use std::collections::{HashMap, HashSet};
use crate::types::*;

// Re-export DerivedFact-related types used by derivation compilers
// (already imported via crate::types::*)

// â”€â”€ Core Functional Types â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

// â”€â”€ Core Functional Types â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
//
// Constraints, derivations, and state machines compile to Func AST nodes.
// Evaluation is beta reduction: apply(func, object, defs) â†’ object.
//
// All constraints compile to AST (Func) nodes:
//   Pure AST:       IR (fully pure â€” Filter + Eq + Construction, zero closures)
//   AST + Native:   UC, MC, FC, VC, AS, SY, AT, IT, TR, AC, RF,
//                   XO, XC, OR, SS, EQ, forbidden, obligatory
//                   (extract_facts_func for fact extraction,
//                   Native kernel for constraint-specific logic)
//   Constant:       Permitted (Func::constant(phi))
//   Goal:           all constraints as pure Func (Condition, Filter, Compose, etc.)


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

/// A compiled constraint. Evaluation is apply(func, eval_context_object) â†’ violations.
pub struct CompiledConstraint {
    pub id: String,
    pub text: String,
    pub modality: Modality,
    pub func: crate::ast::Func,
}


/// A compiled derivation rule. Evaluation is apply(func, population_object) â†’ derived facts.
pub struct CompiledDerivation {
    pub id: String,
    pub text: String,
    pub kind: DerivationKind,
    pub func: crate::ast::Func,
}

/// A compiled state machine. func is the transition function: <state, event> â†’ state'.
pub struct CompiledStateMachine {
    pub noun_name: String,
    pub statuses: Vec<String>,
    pub initial: String,
    pub func: crate::ast::Func,
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
    /// noun_name -> reference scheme value type names (e.g., ["Order Number"])
    pub ref_schemes: HashMap<String, Vec<String>>,
    /// noun_name -> state machine index
    pub noun_to_state_machines: HashMap<String, usize>,
}

/// A compiled graph schema â€” a Construction of Selector functions (roles).
/// Graph Schema = CONS(Roleâ‚, ..., Roleâ‚™) in Backus's FP algebra.
/// Partial application = query. Full application = fact.
pub struct CompiledSchema {
    pub id: String,
    pub reading: String,
    /// The Construction function: [Selector(1), Selector(2), ..., Selector(n)]
    pub construction: crate::ast::Func,
    /// Role names in order (for binding resolution)
    pub role_names: Vec<String>,
}

/// The compiled model â€” all constraints, derivations, state machines, and schemas as executable functions.
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

// â”€â”€ Object â†” Population decoding â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
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

// â”€â”€ Schema Compilation â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
// Compile fact types to Construction functions (CONS of Roles).
// Role â†’ Selector. Graph Schema â†’ Construction [Selectorâ‚, ..., Selectorâ‚™].

/// Compile all fact types in the IR to CompiledSchema (Construction of Selectors).
fn compile_schemas(ir: &Domain) -> HashMap<String, CompiledSchema> {
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

// â”€â”€ Population Primitives â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
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

// â”€â”€ AST Constraint Builders â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
// Pure Func constructors for constraint evaluation.
// Each builds a Func that takes an eval context Object and returns violations.
//
// Eval context encoding: <response_text, sender_identity, population>
// Population encoding:   <ftâ‚, ftâ‚‚, ...> where ft = <ft_id, <factâ‚, ...>>
// Fact encoding:         <<nounâ‚, valâ‚>, <nounâ‚‚, valâ‚‚>, ...>

use crate::ast::{Func, Object};

/// Build a Func that extracts facts for a given fact_type_id from the population.
/// Input: eval context <response, sender, population>
/// Output: <factâ‚, factâ‚‚, ...> or Ï†
fn extract_facts_func(ft_id: &str) -> Func {
    // sel(3) â†’ population
    // Filter(eq âˆ˜ [sel(1), ft_idÌ„]) â†’ matching fact type entries
    // (null â†’ Ï†Ì„; sel(2) âˆ˜ sel(1)) â†’ get facts from first match, or Ï†
    let find_ft = Func::filter(
        Func::compose(
            Func::Eq,
            Func::construction(vec![
                Func::Selector(1),
                Func::constant(Object::atom(ft_id)),
            ]),
        ),
    );

    let get_facts_or_phi = Func::condition(
        Func::NullTest,
        Func::constant(Object::phi()),
        Func::compose(Func::Selector(2), Func::Selector(1)),
    );

    Func::compose(get_facts_or_phi, Func::compose(find_ft, Func::Selector(3)))
}

/// Build a Func that extracts facts for multiple fact type IDs.
/// Returns the concatenation of all facts from all matching fact types.
fn extract_facts_multi(ft_ids: &[String]) -> Func {
    if ft_ids.len() == 1 {
        return extract_facts_func(&ft_ids[0]);
    }
    // For multiple fact type IDs: get facts from each, concatenate.
    // Use a Native for the concatenation since we don't have a built-in flatten.
    let ft_ids_owned: Vec<String> = ft_ids.to_vec();
    Func::Native(Arc::new(move |ctx: &crate::ast::Object| {
        let defs = HashMap::new();
        let mut all_facts = Vec::new();
        for ft_id in &ft_ids_owned {
            let extractor = extract_facts_func(ft_id);
            let result = crate::ast::apply(&extractor, ctx, &defs);
            if let Some(items) = result.as_seq() {
                all_facts.extend_from_slice(items);
            }
        }
        Object::Seq(all_facts)
    }))
}

/// Build a violation Object from constants and a detail Func.
/// The detail Func is applied to the violating fact to produce detail parts.
fn make_violation_func(id: &str, text: &str, detail: Func) -> Func {
    Func::construction(vec![
        Func::constant(Object::atom(id)),
        Func::constant(Object::atom(text)),
        detail,
    ])
}

/// Extract the value of a role from an encoded fact.
/// Fact encoding: <<nounâ‚, valâ‚>, <nounâ‚‚, valâ‚‚>, ...>
/// Role value at index i: sel(2) âˆ˜ sel(i+1)
fn role_value(role_index: usize) -> Func {
    Func::compose(Func::Selector(2), Func::Selector(role_index + 1))
}

// â”€â”€ Span Resolution â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
// Resolves IR references at compile time so predicates capture only what they need.

#[derive(Clone)]
struct ResolvedSpan {
    fact_type_id: String,
    role_index: usize,
    noun_name: String,
    reading: String,
}

fn resolve_spans(ir: &Domain, spans: &[SpanDef]) -> Vec<ResolvedSpan> {
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
/// Deduplicates by noun name â€” each noun's enum values appear at most once.
pub fn collect_enum_values_pub(ir: &Domain, spans: &[SpanDef]) -> Vec<(String, Vec<String>)> {
    collect_enum_values(ir, spans)
}

fn collect_enum_values(ir: &Domain, spans: &[SpanDef]) -> Vec<(String, Vec<String>)> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for span in spans {
        if let Some(ft) = ir.fact_types.get(&span.fact_type_id) {
            for role in &ft.roles {
                if seen.contains(&role.noun_name) { continue; }
                if let Some(vals) = ir.enum_values.get(&role.noun_name) {
                    if !vals.is_empty() {
                        seen.insert(role.noun_name.clone());
                        result.push((role.noun_name.clone(), vals.clone()));
                    }
                }
            }
        }
    }
    result
}

/// Derive state machines from instance facts in P.
/// Queries the population for metamodel fact types.
fn derive_state_machines_from_facts(facts: &[GeneralInstanceFact]) -> HashMap<String, StateMachineDef> {
    let mut machines: HashMap<String, StateMachineDef> = HashMap::new();

    // Pass 1: State Machine Definition 'X' is for Noun 'Y'
    // Field name is the graph schema ID from the declared fact type.
    for f in facts {
        if f.subject_noun == "State Machine Definition" && f.object_noun == "Noun" {
            machines.entry(f.subject_value.clone()).or_insert(StateMachineDef {
                noun_name: f.object_value.clone(),
                statuses: vec![],
                transitions: vec![],
            });
        }
    }

    // Pass 2: Status 'S' is initial in State Machine Definition 'X'
    for f in facts {
        if f.subject_noun == "Status" && f.object_noun == "State Machine Definition" {
            if let Some(sm) = machines.get_mut(&f.object_value) {
                if !sm.statuses.contains(&f.subject_value) {
                    sm.statuses.insert(0, f.subject_value.clone());
                }
            }
        }
    }

    // Pass 3: Build transition lookup from instance facts.
    // Match on object noun type, not field name strings.
    let mut t_from: HashMap<String, String> = HashMap::new();
    let mut t_to: HashMap<String, String> = HashMap::new();
    let mut t_sm: HashMap<String, String> = HashMap::new();
    let mut t_event: HashMap<String, String> = HashMap::new();

    for f in facts {
        if f.subject_noun == "Transition" {
            if f.object_noun == "Status" {
                // Distinguish "is from" vs "is to" by the field name (graph schema ID)
                let field_lower = f.field_name.to_lowercase();
                if field_lower.contains("from") {
                    t_from.insert(f.subject_value.clone(), f.object_value.clone());
                } else if field_lower.contains("to") {
                    t_to.insert(f.subject_value.clone(), f.object_value.clone());
                }
            } else if f.object_noun == "State Machine Definition" {
                t_sm.insert(f.subject_value.clone(), f.object_value.clone());
            } else if f.object_noun == "Event Type" {
                t_event.insert(f.subject_value.clone(), f.object_value.clone());
            }
        }
    }

    // Assemble transitions
    let all_transitions: HashSet<&String> = t_from.keys().chain(t_to.keys()).collect();
    for t_name in all_transitions {
        let from = match t_from.get(t_name) { Some(s) => s.clone(), None => continue };
        let to = match t_to.get(t_name) { Some(s) => s.clone(), None => continue };
        let event = t_event.get(t_name).cloned().unwrap_or_else(|| t_name.clone());

        // Find which SM this transition belongs to
        let target_key = if let Some(name) = t_sm.get(t_name) {
            Some(name.clone())
        } else {
            machines.iter()
                .find(|(_, sm)| sm.statuses.contains(&from) || sm.statuses.contains(&to))
                .map(|(k, _)| k.clone())
                .or_else(|| machines.keys().next().cloned())
        };

        if let Some(sm) = target_key.and_then(|k| machines.get_mut(&k)) {
            if !sm.statuses.contains(&from) { sm.statuses.push(from.clone()); }
            if !sm.statuses.contains(&to) { sm.statuses.push(to.clone()); }
            sm.transitions.push(TransitionDef { from, to, event, guard: None });
        }
    }

    machines
}

// â”€â”€ Compilation â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
// The match on kind happens here, once. After this, everything is Func.

/// Compile a Population into named FFP definitions.
/// Readings in, Def name = func out. Nothing else.
pub fn compile_to_defs(pop: &Population) -> Vec<(String, Func)> {
    let domain = population_to_domain(pop);
    let model = compile(&domain);
    let mut defs: Vec<(String, Func)> = Vec::new();

    // Constraints -> named definitions
    for c in &model.constraints {
        defs.push((format!("constraint:{}", c.id), c.func.clone()));
    }

    // State machines -> named definitions
    // The func is the transition function: <state, event> -> state'.
    // The complete machine is foldl(transition, initial, events).
    // We store the transition function and the initial state as separate cells.
    for sm in &model.state_machines {
        defs.push((format!("machine:{}", sm.noun_name), sm.func.clone()));
        defs.push((format!("machine:{}:initial", sm.noun_name), Func::constant(Object::atom(&sm.initial))));
    }

    // Derivation rules -> named definitions
    for d in &model.derivations {
        defs.push((format!("derivation:{}", d.id), d.func.clone()));
    }

    // Fact type schemas -> named definitions (CONS of roles)
    for (id, schema) in &model.schemas {
        defs.push((format!("schema:{}", id), schema.construction.clone()));
    }

    // HATEOAS navigation links as FFP projections (Theorem 4b).
    // For each binary fact type with a UC, the UC role is the child (dependent),
    // the other role is the parent. Navigation is a constant function returning
    // the related noun names.
    let mut children_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut parent_map: HashMap<String, Vec<String>> = HashMap::new();
    for c in &domain.constraints {
        if c.kind != "UC" || c.spans.is_empty() { continue; }
        let span = &c.spans[0];
        if let Some(ft) = domain.fact_types.get(&span.fact_type_id) {
            if ft.roles.len() != 2 { continue; }
            let constrained_role = span.role_index;
            let child_noun = &ft.roles[constrained_role].noun_name;
            let parent_noun = &ft.roles[1 - constrained_role].noun_name;
            children_map.entry(parent_noun.clone()).or_default().push(child_noun.clone());
            parent_map.entry(child_noun.clone()).or_default().push(parent_noun.clone());
        }
    }
    for (noun, children) in &children_map {
        let child_atoms: Vec<Object> = children.iter().map(|c| Object::atom(c)).collect();
        defs.push((format!("nav:{}:children", noun), Func::constant(Object::Seq(child_atoms))));
    }
    for (noun, parents) in &parent_map {
        let parent_atoms: Vec<Object> = parents.iter().map(|p| Object::atom(p)).collect();
        defs.push((format!("nav:{}:parent", noun), Func::constant(Object::Seq(parent_atoms))));
    }

    defs
}

/// Compile from a Population of facts. Reconstructs the Domain internally
/// by querying P for metamodel fact types, then delegates to compile().
/// This is the target API. The Domain struct is an implementation detail
/// that will be eliminated.
pub fn compile_from_population(pop: &Population) -> CompiledModel {
    let domain = population_to_domain(pop);
    compile(&domain)
}

/// Reconstruct a Domain from a Population by querying metamodel fact types.
pub fn population_to_domain(pop: &Population) -> Domain {
    let mut domain = Domain::default();

    // Query Noun facts
    if let Some(noun_facts) = pop.facts.get("Noun") {
        for f in noun_facts {
            let name = f.bindings.iter().find(|(k, _)| k == "name").map(|(_, v)| v.clone()).unwrap_or_default();
            let obj_type = f.bindings.iter().find(|(k, _)| k == "objectType").map(|(_, v)| v.clone()).unwrap_or("entity".to_string());
            let super_type = f.bindings.iter().find(|(k, _)| k == "superType").map(|(_, v)| v.clone());
            let ref_scheme = f.bindings.iter().find(|(k, _)| k == "referenceScheme").map(|(_, v)| v.split(',').map(|s| s.to_string()).collect::<Vec<_>>());
            let enum_vals = f.bindings.iter().find(|(k, _)| k == "enumValues").map(|(_, v)| v.split(',').map(|s| s.to_string()).collect::<Vec<_>>());

            domain.nouns.insert(name.clone(), NounDef { object_type: obj_type, world_assumption: WorldAssumption::default() });
            if let Some(st) = super_type { domain.subtypes.insert(name.clone(), st); }
            if let Some(rs) = ref_scheme { domain.ref_schemes.insert(name.clone(), rs); }
            if let Some(ev) = enum_vals { domain.enum_values.insert(name.clone(), ev); }
        }
    }

    // Query GraphSchema + Role facts
    if let Some(schema_facts) = pop.facts.get("GraphSchema") {
        for f in schema_facts {
            let id = f.bindings.iter().find(|(k, _)| k == "id").map(|(_, v)| v.clone()).unwrap_or_default();
            let reading = f.bindings.iter().find(|(k, _)| k == "reading").map(|(_, v)| v.clone()).unwrap_or_default();

            // Find roles for this schema
            let roles: Vec<RoleDef> = pop.facts.get("Role")
                .map(|role_facts| role_facts.iter()
                    .filter(|r| r.bindings.iter().any(|(k, v)| k == "graphSchema" && v == &id))
                    .map(|r| {
                        let noun_name = r.bindings.iter().find(|(k, _)| k == "nounName").map(|(_, v)| v.clone()).unwrap_or_default();
                        let position = r.bindings.iter().find(|(k, _)| k == "position").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
                        RoleDef { noun_name, role_index: position }
                    })
                    .collect()
                )
                .unwrap_or_default();

            domain.fact_types.insert(id, FactTypeDef {
                schema_id: String::new(),
                reading,
                readings: vec![],
                roles,
            });
        }
    }

    // Query Constraint facts
    if let Some(constraint_facts) = pop.facts.get("Constraint") {
        for f in constraint_facts {
            let get = |key: &str| f.bindings.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
            let mut spans = vec![];
            for i in 0..4 {
                let ft_key = format!("span{}_factTypeId", i);
                let ri_key = format!("span{}_roleIndex", i);
                if let (Some(ft_id), Some(ri)) = (get(&ft_key), get(&ri_key)) {
                    spans.push(SpanDef {
                        fact_type_id: ft_id,
                        role_index: ri.parse().unwrap_or(0),
                        subset_autofill: None,
                    });
                }
            }
            domain.constraints.push(ConstraintDef {
                id: get("id").unwrap_or_default(),
                kind: get("kind").unwrap_or_default(),
                modality: get("modality").unwrap_or_default(),
                deontic_operator: get("deonticOperator"),
                text: get("text").unwrap_or_default(),
                spans,
                set_comparison_argument_length: None,
                clauses: None,
                entity: get("entity"),
                min_occurrence: None,
                max_occurrence: None,
            });
        }
    }

    // Query DerivationRule facts
    if let Some(rule_facts) = pop.facts.get("DerivationRule") {
        for f in rule_facts {
            let get = |key: &str| f.bindings.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()).unwrap_or_default();
            domain.derivation_rules.push(DerivationRuleDef {
                id: get("id"),
                text: get("text"),
                antecedent_fact_type_ids: vec![],
                consequent_fact_type_id: get("consequentFactTypeId"),
                kind: DerivationKind::ModusPonens,
                join_on: vec![],
                match_on: vec![],
                consequent_bindings: vec![],
            });
        }
    }

    // Query InstanceFact facts
    if let Some(inst_facts) = pop.facts.get("InstanceFact") {
        for f in inst_facts {
            let get = |key: &str| f.bindings.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()).unwrap_or_default();
            domain.general_instance_facts.push(GeneralInstanceFact {
                subject_noun: get("subjectNoun"),
                subject_value: get("subjectValue"),
                field_name: get("fieldName"),
                object_noun: get("objectNoun"),
                object_value: get("objectValue"),
            });
        }
    }

    domain
}

/// Compile an entire Domain into executable form.
pub fn compile(ir: &Domain) -> CompiledModel {
    let constraints: Vec<CompiledConstraint> = ir.constraints.iter()
        .map(|def| compile_constraint(ir, def))
        .collect();

    // Derive state machines from instance facts in P.
    // Query the population for metamodel fact types:
    //   State Machine Definition 'X' is for Noun 'Y'
    //   Status 'S' is initial in State Machine Definition 'X'
    //   Transition 'T' is from Status 'A'
    //   Transition 'T' is to Status 'B'
    //   Transition 'T' is triggered by Event Type 'E'
    //   Transition 'T' is defined in State Machine Definition 'X'
    let sm_defs = derive_state_machines_from_facts(&ir.general_instance_facts);
    // Fall back to ir.state_machines if instance facts produced nothing
    // (supports old-style readings that were parsed before this change).
    let sm_source = if sm_defs.is_empty() { &ir.state_machines } else { &sm_defs };
    let state_machines: Vec<CompiledStateMachine> = sm_source.values()
        .map(|sm_def| compile_state_machine(sm_def, &constraints))
        .collect();

    // Build NounIndex for synthesis queries
    let noun_index = build_noun_index(ir, &constraints, &state_machines);

    // Compile derivation rules â€” both explicit from IR and implicit from structure
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
    ir: &Domain,
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

    // noun_name -> supertype (from IR maps)
    let supertypes: HashMap<String, String> = ir.subtypes.clone();
    let mut subtypes: HashMap<String, Vec<String>> = HashMap::new();
    for (child, parent) in &ir.subtypes {
        subtypes.entry(parent.clone()).or_default().push(child.clone());
    }
    let ref_schemes: HashMap<String, Vec<String>> = ir.ref_schemes.clone();

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

// â”€â”€ AST Derivation Chains â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
// Compile derivation rules to Func::Compose chains.
// "User can access Domain iff A and B and C" becomes f âˆ˜ g âˆ˜ h
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
    ir: &Domain,
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

        // Find the other role (output) â€” the noun we're traversing TO
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
fn compile_derivations(ir: &Domain) -> Vec<CompiledDerivation> {
    let mut derivations = Vec::new();

    // Compile explicit derivation rules from IR
    for rule in &ir.derivation_rules {
        let compiled = match rule.kind {
            DerivationKind::SubtypeInheritance => compile_explicit_derivation(ir, rule),
            DerivationKind::ModusPonens => compile_explicit_derivation(ir, rule),
            DerivationKind::Transitivity => compile_explicit_derivation(ir, rule),
            DerivationKind::ClosedWorldNegation => compile_explicit_derivation(ir, rule),
            DerivationKind::Join => compile_join_derivation(ir, rule),
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

    // Implicit: state machine initialization from SM definitions
    derivations.extend(compile_sm_initialization(ir));

    derivations
}

// â”€â”€ Object-level population helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
// These operate directly on the population Object (Seq of <ft_id, <facts>>)
// without decoding to Population structs. Used by derivation compilers.

/// Find the facts Seq for a specific fact type ID in a population Object.
/// Population: <E1, E2, ...> where Ei = <ft_id, <facts...>>
/// Returns the <facts...> Seq if found, or empty Seq.
fn obj_find_ft<'a>(pop_obj: &'a crate::ast::Object, target_ft_id: &str) -> &'a [crate::ast::Object] {
    static EMPTY: Vec<crate::ast::Object> = Vec::new();
    if let Some(entries) = pop_obj.as_seq() {
        for entry in entries {
            if let Some(pair) = entry.as_seq() {
                if pair.len() == 2 {
                    if let Some(ft_id) = pair[0].as_atom() {
                        if ft_id == target_ft_id {
                            return pair[1].as_seq().unwrap_or(&EMPTY);
                        }
                    }
                }
            }
        }
    }
    &EMPTY
}

/// Collect all unique values for a noun name across the entire population Object.
fn obj_instances_of(pop_obj: &crate::ast::Object, noun_name: &str) -> HashSet<String> {
    let mut result = HashSet::new();
    if let Some(entries) = pop_obj.as_seq() {
        for entry in entries {
            if let Some(pair) = entry.as_seq() {
                if pair.len() == 2 {
                    if let Some(facts) = pair[1].as_seq() {
                        for fact in facts {
                            if let Some(bindings) = fact.as_seq() {
                                for binding in bindings {
                                    if let Some(bpair) = binding.as_seq() {
                                        if bpair.len() == 2 {
                                            if let (Some(n), Some(v)) = (bpair[0].as_atom(), bpair[1].as_atom()) {
                                                if n == noun_name {
                                                    result.insert(v.to_string());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    result
}

/// Check if an entity participates in a specific fact type in the population Object.
fn obj_participates_in(pop_obj: &crate::ast::Object, entity: &str, noun_name: &str, ft_id: &str) -> bool {
    let facts = obj_find_ft(pop_obj, ft_id);
    for fact in facts {
        if let Some(bindings) = fact.as_seq() {
            for binding in bindings {
                if let Some(bpair) = binding.as_seq() {
                    if bpair.len() == 2 {
                        if let (Some(n), Some(v)) = (bpair[0].as_atom(), bpair[1].as_atom()) {
                            if n == noun_name && v == entity {
                                return true;
                            }
                        }
                    }
                }
            }
        }
    }
    false
}

/// Build a derived fact Object: <fact_type_id, reading, <bindings>>
/// where each binding is <noun_name, value>.
fn obj_derived_fact(ft_id: &str, reading: &str, bindings: &[(String, String)]) -> crate::ast::Object {
    let binding_objs: Vec<crate::ast::Object> = bindings.iter().map(|(n, v)| {
        crate::ast::Object::seq(vec![crate::ast::Object::atom(n), crate::ast::Object::atom(v)])
    }).collect();
    crate::ast::Object::seq(vec![
        crate::ast::Object::atom(ft_id),
        crate::ast::Object::atom(reading),
        crate::ast::Object::Seq(binding_objs),
    ])
}

/// Compile an explicit derivation rule from the IR.
///
/// Pure AST form would be:
///   Condition(
///     /And âˆ˜ Î±(Compose(Not âˆ˜ NullTest, find_ft)) : <antecedent_ids>,
///     Construction of collected bindings,
///     Constant(Ï†)
///   )
/// Blocked on: no Filter/Find primitive to locate a fact type by ID in the
/// population Seq. Requires a fold-based search (Insert + Condition) that
/// would be more complex than the direct Object traversal below.
fn compile_explicit_derivation(ir: &Domain, rule: &DerivationRuleDef) -> CompiledDerivation {
    let id = rule.id.clone();
    let text = rule.text.clone();
    let kind = rule.kind.clone();
    let antecedent_ids = rule.antecedent_fact_type_ids.clone();
    let consequent_id = rule.consequent_fact_type_id.clone();
    let consequent_reading = ir.fact_types.get(&consequent_id)
        .map(|ft| ft.reading.clone())
        .unwrap_or_default();

    // Operate directly on the population Object â€” no decode/re-encode.
    let func = crate::ast::Func::Native(Arc::new(move |pop_obj: &crate::ast::Object| {
        // Check if all antecedent fact types have non-empty fact lists
        let all_hold = antecedent_ids.iter().all(|ft_id| {
            !obj_find_ft(pop_obj, ft_id).is_empty()
        });

        if !all_hold {
            return crate::ast::Object::phi();
        }

        // Collect all unique bindings from antecedent facts
        let mut bindings: Vec<(String, String)> = Vec::new();
        for ft_id in &antecedent_ids {
            for fact in obj_find_ft(pop_obj, ft_id) {
                if let Some(fact_bindings) = fact.as_seq() {
                    for binding in fact_bindings {
                        if let Some(bpair) = binding.as_seq() {
                            if bpair.len() == 2 {
                                if let (Some(n), Some(v)) = (bpair[0].as_atom(), bpair[1].as_atom()) {
                                    let pair = (n.to_string(), v.to_string());
                                    if !bindings.contains(&pair) {
                                        bindings.push(pair);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        crate::ast::Object::Seq(vec![
            obj_derived_fact(&consequent_id, &consequent_reading, &bindings),
        ])
    }));
    CompiledDerivation { id, text, kind, func }
}


/// Compile a Join derivation rule â€” cross-fact-type equi-join on shared noun names.
///
/// For each combination of facts from the antecedent fact types, if all join keys
/// (noun names in `rule.join_on`) have matching values across the facts, emit a
/// consequent fact with the combined bindings.
///
/// This implements the relational equi-join needed for rules like:
///   Vehicle is resolved to Chrome Style Candidate
///     := Vehicle has Squish VIN
///     and Chrome Style Candidate has that Squish VIN
///     and some Listing has that VIN
///     and that Listing has Listing Trim
///     and Chrome Style Candidate has Chrome Trim
///     and that Chrome Trim contains that Listing Trim.
///
/// The join_on field specifies which noun names must match across antecedents.
/// The consequent_bindings field specifies which nouns appear in the output.
fn compile_join_derivation(ir: &Domain, rule: &DerivationRuleDef) -> CompiledDerivation {
    let id = rule.id.clone();
    let text = rule.text.clone();
    let kind = rule.kind.clone();
    let antecedent_ids = rule.antecedent_fact_type_ids.clone();
    let consequent_id = rule.consequent_fact_type_id.clone();
    let join_keys = rule.join_on.clone();
    let match_pairs = rule.match_on.clone();
    let consequent_binding_names = rule.consequent_bindings.clone();
    let consequent_reading = ir.fact_types.get(&consequent_id)
        .map(|ft| ft.reading.clone())
        .unwrap_or_default();

    let func = crate::ast::Func::Native(Arc::new(move |pop_obj: &crate::ast::Object| {
        // Collect facts per antecedent fact type
        let mut fact_sets: Vec<Vec<Vec<(String, String)>>> = Vec::new();
        for ft_id in &antecedent_ids {
            let raw_facts = obj_find_ft(pop_obj, ft_id);
            let mut parsed: Vec<Vec<(String, String)>> = Vec::new();
            for fact in raw_facts {
                if let Some(bindings) = fact.as_seq() {
                    let pairs: Vec<(String, String)> = bindings.iter()
                        .filter_map(|b| {
                            let bpair = b.as_seq()?;
                            if bpair.len() == 2 {
                                Some((bpair[0].as_atom()?.to_string(), bpair[1].as_atom()?.to_string()))
                            } else { None }
                        })
                        .collect();
                    if !pairs.is_empty() {
                        parsed.push(pairs);
                    }
                }
            }
            if parsed.is_empty() {
                // If any antecedent has no facts, the join produces nothing
                return crate::ast::Object::phi();
            }
            fact_sets.push(parsed);
        }

        // Perform nested-loop join across all antecedent fact sets.
        // For each combination, check that join keys have matching values.
        let mut results: Vec<crate::ast::Object> = Vec::new();
        join_recursive(
            &fact_sets,
            &join_keys,
            &match_pairs,
            &consequent_binding_names,
            &consequent_id,
            &consequent_reading,
            &mut Vec::new(),
            0,
            &mut results,
        );

        crate::ast::Object::Seq(results)
    }));

    CompiledDerivation { id, text, kind, func }
}

/// Recursive helper for nested-loop join across N fact sets.
fn join_recursive(
    fact_sets: &[Vec<Vec<(String, String)>>],
    join_keys: &[String],
    match_pairs: &[(String, String)],
    consequent_bindings: &[String],
    consequent_id: &str,
    consequent_reading: &str,
    current_combo: &mut Vec<Vec<(String, String)>>,
    depth: usize,
    results: &mut Vec<crate::ast::Object>,
) {
    if depth == fact_sets.len() {
        // All fact sets joined â€” check join keys and match predicates
        if check_join_keys(current_combo, join_keys)
            && check_match_predicates(current_combo, match_pairs)
        {
            // Collect bindings for the consequent
            let mut merged: Vec<(String, String)> = Vec::new();
            for fact_bindings in current_combo.iter() {
                for (n, v) in fact_bindings {
                    if !merged.iter().any(|(mn, mv)| mn == n && mv == v) {
                        merged.push((n.clone(), v.clone()));
                    }
                }
            }

            // Filter to consequent_bindings if specified
            let output = if consequent_bindings.is_empty() {
                merged
            } else {
                merged.into_iter()
                    .filter(|(n, _)| consequent_bindings.contains(n))
                    .collect()
            };

            if !output.is_empty() {
                results.push(obj_derived_fact(consequent_id, consequent_reading, &output));
            }
        }
        return;
    }

    for fact in &fact_sets[depth] {
        current_combo.push(fact.clone());
        join_recursive(fact_sets, join_keys, match_pairs, consequent_bindings, consequent_id, consequent_reading, current_combo, depth + 1, results);
        current_combo.pop();
    }
}

/// Check that all join keys have consistent values across the fact combination.
/// A join key matches if every fact that contains a binding for that noun name
/// has the same value (exact equality).
fn check_join_keys(combo: &[Vec<(String, String)>], join_keys: &[String]) -> bool {
    for key in join_keys {
        let mut values: Vec<&str> = Vec::new();
        for fact_bindings in combo {
            for (n, v) in fact_bindings {
                if n == key {
                    values.push(v.as_str());
                }
            }
        }
        if values.len() >= 2 && !values.windows(2).all(|w| w[0] == w[1]) {
            return false;
        }
    }
    true
}

/// Check cross-noun match predicates.
/// Each pair (left, right) requires: the value of noun `left` contains
/// the value of noun `right` (case-insensitive).
fn check_match_predicates(combo: &[Vec<(String, String)>], match_pairs: &[(String, String)]) -> bool {
    for (left_noun, right_noun) in match_pairs {
        let left_val = combo.iter()
            .flat_map(|b| b.iter())
            .find(|(n, _)| n == left_noun)
            .map(|(_, v)| v.as_str());
        let right_val = combo.iter()
            .flat_map(|b| b.iter())
            .find(|(n, _)| n == right_noun)
            .map(|(_, v)| v.as_str());

        match (left_val, right_val) {
            (Some(l), Some(r)) => {
                if !l.to_lowercase().contains(&r.to_lowercase()) {
                    return false;
                }
            }
            _ => {} // If either value is missing, predicate is vacuously true
        }
    }
    true
}

/// Subtype inheritance: for each noun with a supertype,
/// instances of the subtype inherit participation in the supertype's fact types.
///
/// Pure AST form would be:
///   For each supertype fact type:
///     Î±(Condition(Not âˆ˜ participates, construct_derived, Constant(Ï†))) âˆ˜ instances
///   Blocked on: instances_of requires a global scan (fold over all fact types
///   extracting bindings), and participates_in requires a find-by-ID lookup.
///   Both need Filter/Find primitives not yet in the AST.
fn compile_subtype_inheritance(ir: &Domain) -> Vec<CompiledDerivation> {
    let mut derivations = Vec::new();

    for (sub_name, super_name) in &ir.subtypes {
        {
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
            let text = format!("{} is a subtype of {} â€” inherits fact types", sub, sup);

            // Operate directly on the population Object â€” no decode/re-encode.
            let func = crate::ast::Func::Native(Arc::new(move |pop_obj: &crate::ast::Object| {
                let mut derived = Vec::new();

                // Find all instances of the subtype in the population
                let sub_instances = obj_instances_of(pop_obj, &sub);

                for (ft_id, reading, _role_idx) in &sft {
                    for instance in &sub_instances {
                        // Check if this instance already participates in this fact type
                        if !obj_participates_in(pop_obj, instance, &sup, ft_id) {
                            derived.push(obj_derived_fact(
                                ft_id,
                                reading,
                                &[(sup.clone(), instance.clone())],
                            ));
                        }
                    }
                }

                if derived.is_empty() {
                    crate::ast::Object::phi()
                } else {
                    crate::ast::Object::Seq(derived)
                }
            }));
            derivations.push(CompiledDerivation {
                id,
                text,
                kind: DerivationKind::SubtypeInheritance,
                func,
            });
        }
    }

    derivations
}

/// Modus ponens on subset constraints: if A subset B (SS constraint),
/// when we find an instance in A, derive its presence in B.
///
/// Pure AST form would be:
///   Î±(Condition(Not âˆ˜ exists_in_B, construct_B_fact, Constant(Ï†)))
///     âˆ˜ Î±(project_to_B_nouns)
///     âˆ˜ find_ft(A)
/// Blocked on: find_ft requires searching the population Seq by atom ID,
/// and exists_in_B needs a nested membership check. Both need a fold-based
/// search primitive (Insert + Condition) not yet ergonomic in the AST.
fn compile_modus_ponens(ir: &Domain) -> Vec<CompiledDerivation> {
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
        let b_role_names: Vec<String> = ir.fact_types.get(&b_ft_id)
            .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
            .unwrap_or_default();

        let b_reading = ir.fact_types.get(&b_ft_id)
            .map(|ft| ft.reading.clone())
            .unwrap_or_default();

        let id = format!("_modus_ponens_{}", cdef.id);
        let text = format!("Modus ponens from SS constraint: {}", cdef.text);

        // Operate directly on the population Object â€” no decode/re-encode.
        let func = crate::ast::Func::Native(Arc::new(move |pop_obj: &crate::ast::Object| {
            let a_facts = obj_find_ft(pop_obj, &a_ft_id);
            let b_facts = obj_find_ft(pop_obj, &b_ft_id);

            let mut derived = Vec::new();

            for a_fact in a_facts {
                if let Some(a_bindings) = a_fact.as_seq() {
                    // Build the consequent tuple by mapping bindings by noun name
                    let mut b_bindings: Vec<(String, String)> = Vec::new();
                    for b_noun in &b_role_names {
                        for ab in a_bindings {
                            if let Some(abpair) = ab.as_seq() {
                                if abpair.len() == 2 {
                                    if let (Some(n), Some(v)) = (abpair[0].as_atom(), abpair[1].as_atom()) {
                                        if n == b_noun {
                                            b_bindings.push((b_noun.clone(), v.to_string()));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if b_bindings.is_empty() { continue; }

                    // Check if this tuple already exists in the consequent population
                    let already_exists = b_facts.iter().any(|bf| {
                        if let Some(bf_bindings) = bf.as_seq() {
                            b_bindings.iter().all(|(name, val)| {
                                bf_bindings.iter().any(|bb| {
                                    if let Some(bbpair) = bb.as_seq() {
                                        bbpair.len() == 2
                                            && bbpair[0].as_atom() == Some(name.as_str())
                                            && bbpair[1].as_atom() == Some(val.as_str())
                                    } else {
                                        false
                                    }
                                })
                            })
                        } else {
                            false
                        }
                    });

                    if !already_exists {
                        derived.push(obj_derived_fact(&b_ft_id, &b_reading, &b_bindings));
                    }
                }
            }

            if derived.is_empty() {
                crate::ast::Object::phi()
            } else {
                crate::ast::Object::Seq(derived)
            }
        }));
        derivations.push(CompiledDerivation {
            id,
            text,
            kind: DerivationKind::ModusPonens,
            func,
        });
    }

    derivations
}

/// Transitivity: for fact types that share a noun in different roles (A->B, B->C),
/// derive the transitive closure A->C. Limited depth to prevent infinite chains.
///
/// Pure AST form would be:
///   Î±(Î±(construct_pair) âˆ˜ DistL) âˆ˜ Trans âˆ˜ [join_key_matches, src_vals, dst_vals]
///   where join_key_matches filters the cross-product of ft1 x ft2 on shared noun.
/// Blocked on: the equi-join (nested loop matching shared noun values) requires
/// a cross-product (DistL/DistR) followed by a filter, which needs Filter or
/// a fold-based select. The AST lacks these as first-class primitives.
fn compile_transitivity(ir: &Domain) -> Vec<CompiledDerivation> {
    let mut derivations = Vec::new();

    // Find binary fact types (exactly 2 roles) that share a noun
    let binary_fts: Vec<(&String, &FactTypeDef)> = ir.fact_types.iter()
        .filter(|(_, ft)| ft.roles.len() == 2)
        .collect();

    for (i, (ft1_id, ft1)) in binary_fts.iter().enumerate() {
        for (j, (ft2_id, ft2)) in binary_fts.iter().enumerate() {
            if i == j { continue; } // skip self-pairing
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
            let reading_c = reading.clone();
            let src_noun_c = src_noun.clone();
            let dst_noun_c = dst_noun.clone();
            let shared_noun_c = shared_noun.clone();
            let transitive_ft_id = format!("_transitive_{}_{}", ft1_id_c, ft2_id_c);

            // Operate directly on the population Object â€” no decode/re-encode.
            let func = crate::ast::Func::Native(Arc::new(move |pop_obj: &crate::ast::Object| {
                let ft1_facts = obj_find_ft(pop_obj, &ft1_id_c);
                let ft2_facts = obj_find_ft(pop_obj, &ft2_id_c);

                let mut derived = Vec::new();

                for f1 in ft1_facts {
                    if let Some(f1_bindings) = f1.as_seq() {
                        // Extract shared and src values from f1
                        let mut shared_val: Option<&str> = None;
                        let mut src_val: Option<&str> = None;
                        for b in f1_bindings {
                            if let Some(bp) = b.as_seq() {
                                if bp.len() == 2 {
                                    if let (Some(n), Some(v)) = (bp[0].as_atom(), bp[1].as_atom()) {
                                        if n == shared_noun_c { shared_val = Some(v); }
                                        if n == src_noun_c { src_val = Some(v); }
                                    }
                                }
                            }
                        }

                        if let (Some(sv), Some(srcv)) = (shared_val, src_val) {
                            // Find matching ft2 facts where shared noun == sv
                            for f2 in ft2_facts {
                                if let Some(f2_bindings) = f2.as_seq() {
                                    let mut f2_shared: Option<&str> = None;
                                    let mut dst_val: Option<&str> = None;
                                    for b in f2_bindings {
                                        if let Some(bp) = b.as_seq() {
                                            if bp.len() == 2 {
                                                if let (Some(n), Some(v)) = (bp[0].as_atom(), bp[1].as_atom()) {
                                                    if n == shared_noun_c { f2_shared = Some(v); }
                                                    if n == dst_noun_c { dst_val = Some(v); }
                                                }
                                            }
                                        }
                                    }

                                    if f2_shared == Some(sv) {
                                        if let Some(dv) = dst_val {
                                            derived.push(obj_derived_fact(
                                                &transitive_ft_id,
                                                &reading_c,
                                                &[
                                                    (src_noun_c.clone(), srcv.to_string()),
                                                    (dst_noun_c.clone(), dv.to_string()),
                                                ],
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if derived.is_empty() {
                    crate::ast::Object::phi()
                } else {
                    crate::ast::Object::Seq(derived)
                }
            }));
            derivations.push(CompiledDerivation {
                id,
                text: reading,
                kind: DerivationKind::Transitivity,
                func,
            });
        }
    }

    derivations
}

/// CWA negation: for nouns with WorldAssumption::Closed,
/// if a fact type involving this noun has no instances for a given entity,
/// derive the negation. For OWA nouns, absence is unknown, not false.
///
/// Pure AST form would be:
///   For each relevant fact type:
///     Î±(Condition(Not âˆ˜ participates_in_ft, construct_negation, Constant(Ï†)))
///       âˆ˜ all_instances(noun)
///   Blocked on: all_instances requires a global fold over every fact type in
///   the population extracting noun bindings, and participates_in_ft requires
///   a find-by-ID search. Both need Filter/Find primitives not yet in the AST.
fn compile_cwa_negation(ir: &Domain) -> Vec<CompiledDerivation> {
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

        // Operate directly on the population Object â€” no decode/re-encode.
        let func = crate::ast::Func::Native(Arc::new(move |pop_obj: &crate::ast::Object| {
            let all_instances = obj_instances_of(pop_obj, &noun);
            let mut derived = Vec::new();

            for (ft_id, reading, _role_idx) in &rft {
                for instance in &all_instances {
                    if !obj_participates_in(pop_obj, instance, &noun, ft_id) {
                        derived.push(obj_derived_fact(
                            ft_id,
                            &format!("NOT: {} (CWA negation for {} '{}')", reading, noun, instance),
                            &[(noun.clone(), instance.clone())],
                        ));
                    }
                }
            }

            if derived.is_empty() {
                crate::ast::Object::phi()
            } else {
                crate::ast::Object::Seq(derived)
            }
        }));
        derivations.push(CompiledDerivation {
            id,
            text,
            kind: DerivationKind::ClosedWorldNegation,
            func,
        });
    }

    derivations
}

/// State machine initialization as a derivation rule.
///
/// Paper: "State machine initialization is not a separate step. The derivation
/// rules produce the State Machine instance and its initial Status as derived facts."
///
/// For each noun with a state machine definition, when an entity of that noun
/// exists in the population but no State Machine is for that entity, derive:
///   - State Machine instance (instanceOf, forResource, currentlyInStatus = initial)
fn compile_sm_initialization(ir: &Domain) -> Vec<CompiledDerivation> {
    let mut derivations = Vec::new();

    for (noun_name, sm_def) in &ir.state_machines {
        let noun = noun_name.clone();
        let sm_noun = sm_def.noun_name.clone();
        let initial_status = sm_def.statuses.first().cloned().unwrap_or_default();
        let id = format!("_sm_init_{}", noun);
        let text = format!("State Machine initialization for {}: initial status = {}", noun, initial_status);

        let func = Func::Native(Arc::new(move |pop_obj: &Object| {
            // Find all instances of this noun in the population
            let instances = obj_instances_of(pop_obj, &noun);
            if instances.is_empty() {
                return Object::phi();
            }

            // Find existing SM-for-resource facts
            let sm_for_facts = obj_find_ft(pop_obj, "StateMachine_has_forResource");
            let existing_resources: std::collections::HashSet<&str> = sm_for_facts.iter()
                .filter_map(|fact| {
                    let bindings = fact.as_seq()?;
                    for b in bindings {
                        if let Some(pair) = b.as_seq() {
                            if pair.len() == 2 && pair[0].as_atom() == Some("forResource") {
                                return pair[1].as_atom();
                            }
                        }
                    }
                    None
                })
                .collect();

            let mut derived = Vec::new();
            for instance in &instances {
                if existing_resources.contains(instance.as_str()) {
                    continue; // SM already exists for this entity
                }

                let sm_id = format!("sm:{}", instance);
                // Derive SM facts: instanceOf, currentlyInStatus, forResource
                derived.push(obj_derived_fact(
                    "StateMachine_has_instanceOf",
                    &format!("State Machine for {} is instance of {}", instance, sm_noun),
                    &[
                        ("State Machine".to_string(), sm_id.clone()),
                        ("instanceOf".to_string(), sm_noun.clone()),
                    ],
                ));
                derived.push(obj_derived_fact(
                    "StateMachine_has_currentlyInStatus",
                    &format!("State Machine for {} initially in {}", instance, initial_status),
                    &[
                        ("State Machine".to_string(), sm_id.clone()),
                        ("currentlyInStatus".to_string(), initial_status.clone()),
                    ],
                ));
                derived.push(obj_derived_fact(
                    "StateMachine_has_forResource",
                    &format!("State Machine is for {}", instance),
                    &[
                        ("State Machine".to_string(), sm_id.clone()),
                        ("forResource".to_string(), instance.clone()),
                    ],
                ));
            }

            if derived.is_empty() { Object::phi() } else { Object::Seq(derived) }
        }));

        derivations.push(CompiledDerivation {
            id,
            text,
            kind: DerivationKind::SubtypeInheritance, // reuse kind â€” SM init is structural
            func,
        });
    }

    derivations
}

fn compile_constraint(ir: &Domain, def: &ConstraintDef) -> CompiledConstraint {
    let modality = match def.modality.to_lowercase().as_str() {
        "deontic" => {
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

    // Compile constraint to a Func.
    // AST compilers return Func directly.
    let func = match &modality {
        Modality::Deontic(DeonticOp::Permitted) => {
            // Permitted constraints never violate
            Func::constant(Object::phi())
        }
        Modality::Deontic(DeonticOp::Forbidden) => {
            compile_forbidden_ast(ir, def)
        }
        Modality::Deontic(DeonticOp::Obligatory) => {
            compile_obligatory_ast(ir, def)
        }
        Modality::Alethic => match def.kind.as_str() {
            // â”€â”€ Pure AST constraints â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
            "IR" => compile_ring_irreflexive_ast(def),
            "AS" => compile_ring_asymmetric_ast(def),
            "SY" => compile_ring_symmetric_ast(def),
            "AT" | "ANS" => compile_ring_antisymmetric_ast(def),

            // â”€â”€ AST with Native evaluation kernel â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
            "UC" => compile_uniqueness_ast(ir, def),
            "MC" => compile_mandatory_ast(ir, def),

            // â”€â”€ AST with Native evaluation kernel (continued) â”€â”€â”€â”€â”€â”€â”€â”€
            "FC" => compile_frequency_ast(ir, def),
            "VC" => compile_value_constraint_ast(ir, def),
            "IT" => compile_ring_intransitive_ast(def),
            "TR" => compile_ring_transitive_ast(def),
            "AC" => compile_ring_acyclic_ast(def),
            "RF" => compile_ring_reflexive_ast(ir, def),
            "XO" => compile_set_comparison_ast(ir, def, |n| n != 1, "exactly one"),
            "XC" => compile_set_comparison_ast(ir, def, |n| n > 1, "at most one"),
            "OR" => compile_set_comparison_ast(ir, def, |n| n < 1, "at least one"),
            "SS" => compile_subset_ast(ir, def),
            "EQ" => compile_equality_ast(ir, def),
            _ => Func::constant(Object::phi()),
        },
    };

    CompiledConstraint {
        id: def.id.clone(),
        text: def.text.clone(),
        modality,
        func,
    }
}

// â”€â”€ Ring Constraints â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
// Ring constraints on binary self-referential fact types.
// Each returns a Func that takes an eval context Object â†’ violations.

/// IR: Â¬âˆƒ(x,x) â€” no fact where both roles reference the same entity.
/// Î±(make_violation) âˆ˜ Filter(eq âˆ˜ [roleâ‚_val, roleâ‚‚_val]) âˆ˜ facts
fn compile_ring_irreflexive_ast(def: &ConstraintDef) -> Func {
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();
    let facts = extract_facts_multi(&ft_ids);

    // Predicate: role 0 value = role 1 value (self-reference)
    let is_self_ref = Func::compose(
        Func::Eq,
        Func::construction(vec![role_value(0), role_value(1)]),
    );

    // Violation detail: <"Irreflexive violation:", value, "references itself">
    let detail = Func::construction(vec![
        Func::constant(Object::atom("Irreflexive violation:")),
        role_value(0),
        Func::constant(Object::atom("references itself")),
    ]);

    let viol = make_violation_func(&def.id, &def.text, detail);

    // Î±(make_viol) âˆ˜ Filter(is_self_ref) âˆ˜ extract_facts
    Func::compose(
        Func::apply_to_all(viol),
        Func::compose(Func::filter(is_self_ref), facts),
    )
}

/// AS: xRy â†’ Â¬yRx â€” if (x,y) exists and (y,x) exists, violation.
/// Uses DistL + Filter to check for reverse pairs.
fn compile_ring_asymmetric_ast(def: &ConstraintDef) -> Func {
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();
    let facts = extract_facts_multi(&ft_ids);

    // For each pair (x,y) where xâ‰ y, check if (y,x) also exists.
    // This is O(nÂ²) but populations are entity-scoped (bounded).
    let id = def.id.clone();
    let text = def.text.clone();

    // AS: xRy → ¬yRx. Violation when both ⟨x,y⟩ and ⟨y,x⟩ exist (and x≠y).
    //
    // Pure Func using distl for membership test:
    //   distr ∘ [facts, facts] : ctx → ⟨⟨f₁, all⟩, ⟨f₂, all⟩, ...⟩
    //   For each ⟨fact, all⟩:
    //     distl : ⟨fact, all⟩ → ⟨⟨fact,f₁⟩, ⟨fact,f₂⟩, ...⟩
    //     Filter(match_reversed) → candidates where role₀(candidate)=role₁(fact) ∧ role₁(candidate)=role₀(fact)
    //     ¬null → has_reverse
    //   Filter facts where has_reverse ∧ x≠y, wrap in violations.

    // match_reversed: ⟨fact, candidate⟩ → role₀(cand) = role₁(fact) ∧ role₁(cand) = role₀(fact)
    let match_reversed = Func::compose(Func::And, Func::construction(vec![
        Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(role_value(0), Func::Selector(2)), // role₀(candidate)
            Func::compose(role_value(1), Func::Selector(1)), // role₁(fact)
        ])),
        Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(role_value(1), Func::Selector(2)), // role₁(candidate)
            Func::compose(role_value(0), Func::Selector(1)), // role₀(fact)
        ])),
    ]));

    // check_one: ⟨fact, all_facts⟩ → T if reverse exists, else F
    let check_one = Func::compose(
        Func::compose(Func::Not, Func::NullTest),
        Func::compose(Func::filter(match_reversed), Func::DistL),
    );

    // not_self on original fact: role₀ ≠ role₁
    let not_self = Func::compose(Func::Not, Func::compose(Func::Eq, Func::construction(vec![
        Func::compose(role_value(0), Func::Selector(1)),
        Func::compose(role_value(1), Func::Selector(1)),
    ])));

    // combined: has_reverse ∧ not_self
    let pred = Func::compose(Func::And, Func::construction(vec![check_one, not_self]));

    // violation detail from ⟨fact, all_facts⟩ — uses fact (sel₁)
    let detail = Func::construction(vec![
        Func::constant(Object::atom("Asymmetric violation:")),
        Func::compose(role_value(0), Func::Selector(1)),
        Func::constant(Object::atom("relates to")),
        Func::compose(role_value(1), Func::Selector(1)),
        Func::constant(Object::atom("and vice versa")),
    ]);
    let viol = make_violation_func(&def.id, &def.text, detail);

    // α(make_viol) ∘ Filter(pred) ∘ distr ∘ [facts, facts] : ctx
    Func::compose(
        Func::apply_to_all(viol),
        Func::compose(
            Func::filter(pred),
            Func::compose(Func::DistR, Func::construction(vec![facts.clone(), facts])),
        ),
    )
}

/// SY: xRy â†’ yRx â€” violation when reverse is missing.
fn compile_ring_symmetric_ast(def: &ConstraintDef) -> Func {
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();
    let facts = extract_facts_multi(&ft_ids);

    let id = def.id.clone();
    let text = def.text.clone();

    Func::Native(Arc::new(move |ctx: &Object| {
        let defs = HashMap::new();
        let all_facts = crate::ast::apply(&facts, ctx, &defs);
        let items = match all_facts.as_seq() {
            Some(items) => items,
            None => return Object::phi(),
        };

        let pairs: Vec<(String, String)> = items.iter().filter_map(|fact| {
            let v0 = crate::ast::apply(&role_value(0), fact, &defs);
            let v1 = crate::ast::apply(&role_value(1), fact, &defs);
            Some((v0.as_atom()?.to_string(), v1.as_atom()?.to_string()))
        }).collect();

        let set: HashSet<(String, String)> = pairs.iter().cloned().collect();

        let violations: Vec<Object> = pairs.iter()
            .filter(|(x, y)| x != y && !set.contains(&(y.clone(), x.clone())))
            .map(|(x, y)| {
                Object::seq(vec![
                    Object::atom(&id),
                    Object::atom(&text),
                    Object::seq(vec![
                        Object::atom("Symmetric violation:"),
                        Object::atom(x),
                        Object::atom("relates to"),
                        Object::atom(y),
                        Object::atom("but not the reverse"),
                    ]),
                ])
            })
            .collect();

        if violations.is_empty() { Object::phi() } else { Object::Seq(violations) }
    }))
}

/// AT/ANS: xRy âˆ§ yRx â†’ x = y â€” violation when both directions exist for distinct entities.
fn compile_ring_antisymmetric_ast(def: &ConstraintDef) -> Func {
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();
    let facts = extract_facts_multi(&ft_ids);

    let id = def.id.clone();
    let text = def.text.clone();

    Func::Native(Arc::new(move |ctx: &Object| {
        let defs = HashMap::new();
        let all_facts = crate::ast::apply(&facts, ctx, &defs);
        let items = match all_facts.as_seq() {
            Some(items) => items,
            None => return Object::phi(),
        };

        let pairs: Vec<(String, String)> = items.iter().filter_map(|fact| {
            let v0 = crate::ast::apply(&role_value(0), fact, &defs);
            let v1 = crate::ast::apply(&role_value(1), fact, &defs);
            Some((v0.as_atom()?.to_string(), v1.as_atom()?.to_string()))
        }).collect();

        let set: HashSet<(String, String)> = pairs.iter().cloned().collect();

        let violations: Vec<Object> = pairs.iter()
            .filter(|(x, y)| x != y && set.contains(&(y.clone(), x.clone())))
            .map(|(x, y)| {
                Object::seq(vec![
                    Object::atom(&id),
                    Object::atom(&text),
                    Object::seq(vec![
                        Object::atom("Antisymmetric violation:"),
                        Object::atom(x),
                        Object::atom("and"),
                        Object::atom(y),
                        Object::atom("relate to each other but are not the same"),
                    ]),
                ])
            })
            .collect();

        if violations.is_empty() { Object::phi() } else { Object::Seq(violations) }
    }))
}

/// IT: xRy âˆ§ yRz â†’ Â¬xRz â€” violation when transitive shortcut exists.
fn compile_ring_intransitive_ast(def: &ConstraintDef) -> Func {
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();
    let facts = extract_facts_multi(&ft_ids);

    let id = def.id.clone();
    let text = def.text.clone();

    Func::Native(Arc::new(move |ctx: &Object| {
        let defs = HashMap::new();
        let all_facts = crate::ast::apply(&facts, ctx, &defs);
        let items = match all_facts.as_seq() {
            Some(items) => items,
            None => return Object::phi(),
        };

        let pairs: Vec<(String, String)> = items.iter().filter_map(|fact| {
            let v0 = crate::ast::apply(&role_value(0), fact, &defs);
            let v1 = crate::ast::apply(&role_value(1), fact, &defs);
            Some((v0.as_atom()?.to_string(), v1.as_atom()?.to_string()))
        }).collect();

        let set: HashSet<(String, String)> = pairs.iter().cloned().collect();

        // For each xRy, look for yRz, check if xRz exists (violation)
        let mut violations = Vec::new();
        for (x, y) in &pairs {
            for (y2, z) in &pairs {
                if y == y2 && x != z && set.contains(&(x.clone(), z.clone())) {
                    violations.push(Object::seq(vec![
                        Object::atom(&id),
                        Object::atom(&text),
                        Object::seq(vec![
                            Object::atom("Intransitive violation:"),
                            Object::atom(x),
                            Object::atom("relates to"),
                            Object::atom(y),
                            Object::atom("relates to"),
                            Object::atom(z),
                            Object::atom("but shortcut also exists"),
                        ]),
                    ]));
                }
            }
        }

        if violations.is_empty() { Object::phi() } else { Object::Seq(violations) }
    }))
}

/// TR: xRy âˆ§ yRz â†’ xRz â€” violation when transitive chain completion is missing.
fn compile_ring_transitive_ast(def: &ConstraintDef) -> Func {
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();
    let facts = extract_facts_multi(&ft_ids);

    let id = def.id.clone();
    let text = def.text.clone();

    Func::Native(Arc::new(move |ctx: &Object| {
        let defs = HashMap::new();
        let all_facts = crate::ast::apply(&facts, ctx, &defs);
        let items = match all_facts.as_seq() {
            Some(items) => items,
            None => return Object::phi(),
        };

        let pairs: Vec<(String, String)> = items.iter().filter_map(|fact| {
            let v0 = crate::ast::apply(&role_value(0), fact, &defs);
            let v1 = crate::ast::apply(&role_value(1), fact, &defs);
            Some((v0.as_atom()?.to_string(), v1.as_atom()?.to_string()))
        }).collect();

        let set: HashSet<(String, String)> = pairs.iter().cloned().collect();

        // For each xRy, look for yRz, check if xRz is missing (violation)
        let mut violations = Vec::new();
        for (x, y) in &pairs {
            for (y2, z) in &pairs {
                if y == y2 && x != z && !set.contains(&(x.clone(), z.clone())) {
                    violations.push(Object::seq(vec![
                        Object::atom(&id),
                        Object::atom(&text),
                        Object::seq(vec![
                            Object::atom("Transitive violation:"),
                            Object::atom(x),
                            Object::atom("relates to"),
                            Object::atom(y),
                            Object::atom("relates to"),
                            Object::atom(z),
                            Object::atom("but shortcut is missing"),
                        ]),
                    ]));
                }
            }
        }

        if violations.is_empty() { Object::phi() } else { Object::Seq(violations) }
    }))
}

/// AC: no cycle xâ‚Rxâ‚‚...xâ‚™Rxâ‚ â€” DFS cycle detection.
fn compile_ring_acyclic_ast(def: &ConstraintDef) -> Func {
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();
    let facts = extract_facts_multi(&ft_ids);

    let id = def.id.clone();
    let text = def.text.clone();

    Func::Native(Arc::new(move |ctx: &Object| {
        let defs = HashMap::new();
        let all_facts = crate::ast::apply(&facts, ctx, &defs);
        let items = match all_facts.as_seq() {
            Some(items) => items,
            None => return Object::phi(),
        };

        let pairs: Vec<(String, String)> = items.iter().filter_map(|fact| {
            let v0 = crate::ast::apply(&role_value(0), fact, &defs);
            let v1 = crate::ast::apply(&role_value(1), fact, &defs);
            Some((v0.as_atom()?.to_string(), v1.as_atom()?.to_string()))
        }).collect();

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
                    let cycle_text = cycle_nodes.join(" -> ");
                    violations.push(Object::seq(vec![
                        Object::atom(&id),
                        Object::atom(&text),
                        Object::seq(vec![
                            Object::atom("Acyclic violation:"),
                            Object::atom("cycle detected through"),
                            Object::atom(&cycle_text),
                        ]),
                    ]));
                }
            }
        }

        if violations.is_empty() { Object::phi() } else { Object::Seq(violations) }
    }))
}

/// RF: for each entity x, xRx must exist â€” violation when self-reference is missing.
fn compile_ring_reflexive_ast(ir: &Domain, def: &ConstraintDef) -> Func {
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();
    let facts = extract_facts_multi(&ft_ids);

    let id = def.id.clone();
    let text = def.text.clone();

    // Find the noun name from spans to know which instances to check
    let noun_name: String = def.spans.first()
        .and_then(|s| ir.fact_types.get(&s.fact_type_id))
        .and_then(|ft| ft.roles.first())
        .map(|r| r.noun_name.clone())
        .unwrap_or_default();

    Func::Native(Arc::new(move |ctx: &Object| {
        let defs = HashMap::new();
        let all_facts = crate::ast::apply(&facts, ctx, &defs);

        // Collect self-referencing instances from the ring facts
        let self_refs: HashSet<String> = match all_facts.as_seq() {
            Some(items) => items.iter().filter_map(|fact| {
                let v0 = crate::ast::apply(&role_value(0), fact, &defs);
                let v1 = crate::ast::apply(&role_value(1), fact, &defs);
                let s0 = v0.as_atom()?.to_string();
                let s1 = v1.as_atom()?.to_string();
                if s0 == s1 { Some(s0) } else { None }
            }).collect(),
            None => HashSet::new(),
        };

        // Decode population to find all instances of the noun
        let population = decode_population_object(&crate::ast::apply(&Func::Selector(3), ctx, &defs));
        let all_instances = instances_of(&noun_name, &population);

        let violations: Vec<Object> = all_instances.iter()
            .filter(|inst| !self_refs.contains(*inst))
            .map(|inst| {
                Object::seq(vec![
                    Object::atom(&id),
                    Object::atom(&text),
                    Object::seq(vec![
                        Object::atom("Reflexive violation:"),
                        Object::atom(inst),
                        Object::atom("does not reference itself"),
                    ]),
                ])
            })
            .collect();

        if violations.is_empty() { Object::phi() } else { Object::Seq(violations) }
    }))
}

// â”€â”€ Alethic Constraint Compilers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
// Each returns a Func that takes an eval context Object â†’ violations.
// Fact extraction uses extract_facts_func (pure AST).
// Constraint-specific evaluation uses Native where point-free FP
// would be impractical (grouping, counting, set operations).

/// UC: Uniqueness constraint.
/// Compose(check_uniqueness, extract_facts) â€” fact type visible in AST.
fn compile_uniqueness_ast(_ir: &Domain, def: &ConstraintDef) -> Func {
    let id = def.id.clone();
    let text = def.text.clone();
    let spans = resolve_spans(_ir, &def.spans);

    let mut groups: HashMap<String, Vec<ResolvedSpan>> = HashMap::new();
    for span in &spans {
        groups.entry(span.fact_type_id.clone()).or_default().push(span.clone());
    }
    let span_groups: Vec<(String, Vec<ResolvedSpan>)> = groups.into_iter().collect();

    // Build extractors for each fact type (pure AST)
    let extractors: Vec<(Func, Vec<ResolvedSpan>)> = span_groups.iter()
        .map(|(ft_id, spans)| (extract_facts_func(ft_id), spans.clone()))
        .collect();

    Func::Native(Arc::new(move |ctx: &Object| {
        let defs = HashMap::new();
        let mut violations = Vec::new();

        for (extractor, group_spans) in &extractors {
            let facts_obj = crate::ast::apply(extractor, ctx, &defs);
            let facts = match facts_obj.as_seq() {
                Some(f) => f,
                None => continue,
            };

            if group_spans.len() == 1 {
                let span = &group_spans[0];
                let role_sel = role_value(span.role_index);
                let mut seen: HashMap<String, usize> = HashMap::new();
                for fact in facts {
                    if let Some(val) = crate::ast::apply(&role_sel, fact, &defs).as_atom() {
                        *seen.entry(val.to_string()).or_insert(0) += 1;
                    }
                }
                for (val, count) in seen {
                    if count > 1 {
                        violations.push(Object::seq(vec![
                            Object::atom(&id),
                            Object::atom(&text),
                            Object::seq(vec![
                                Object::atom("Uniqueness violation:"),
                                Object::atom(&span.noun_name),
                                Object::atom(&format!("'{}'", val)),
                                Object::atom(&format!("appears {} times in", count)),
                                Object::atom(&span.reading),
                            ]),
                        ]));
                    }
                }
            } else {
                let role_sels: Vec<Func> = group_spans.iter()
                    .map(|s| role_value(s.role_index))
                    .collect();
                let role_names: Vec<&str> = group_spans.iter()
                    .map(|s| s.noun_name.as_str())
                    .collect();
                let reading = &group_spans[0].reading;

                let mut seen: HashMap<String, usize> = HashMap::new();
                for fact in facts {
                    let tuple_key: String = role_sels.iter()
                        .map(|sel| crate::ast::apply(sel, fact, &defs)
                            .as_atom().unwrap_or("").to_string())
                        .collect::<Vec<_>>()
                        .join("|");
                    *seen.entry(tuple_key).or_insert(0) += 1;
                }

                let label = role_names.join(", ");
                for (tuple, count) in seen {
                    if count > 1 {
                        violations.push(Object::seq(vec![
                            Object::atom(&id),
                            Object::atom(&text),
                            Object::seq(vec![
                                Object::atom("Uniqueness violation:"),
                                Object::atom(&format!("({})", label)),
                                Object::atom(&format!("combination '{}' appears {} times in", tuple.replace('|', ", "), count)),
                                Object::atom(reading),
                            ]),
                        ]));
                    }
                }
            }
        }

        if violations.is_empty() { Object::phi() } else { Object::Seq(violations) }
    }))
}

/// MC: Mandatory constraint.
/// For each entity instance of the constrained noun, check it participates
/// in the required fact type.
fn compile_mandatory_ast(_ir: &Domain, def: &ConstraintDef) -> Func {
    let id = def.id.clone();
    let text = def.text.clone();
    let spans = resolve_spans(_ir, &def.spans);

    let span_data: Vec<(Func, String, String)> = spans.iter()
        .map(|s| (extract_facts_func(&s.fact_type_id), s.noun_name.clone(), s.reading.clone()))
        .collect();

    Func::Native(Arc::new(move |ctx: &Object| {
        let defs = HashMap::new();

        // Get the full population to find all instances of each noun
        let pop_obj = crate::ast::apply(&Func::Selector(3), ctx, &defs);
        let population = decode_population_object(&pop_obj);

        let mut violations = Vec::new();
        for (extractor, noun_name, reading) in &span_data {
            let facts_obj = crate::ast::apply(extractor, ctx, &defs);

            let all_instances = instances_of(noun_name, &population);

            for instance in &all_instances {
                let has_fact = match facts_obj.as_seq() {
                    Some(facts) => facts.iter().any(|fact| {
                        match fact.as_seq() {
                            Some(bindings) => bindings.iter().any(|b| {
                                match b.as_seq() {
                                    Some(pair) if pair.len() == 2 =>
                                        pair[0].as_atom() == Some(noun_name) &&
                                        pair[1].as_atom() == Some(instance),
                                    _ => false,
                                }
                            }),
                            None => false,
                        }
                    }),
                    None => false,
                };
                if !has_fact {
                    violations.push(Object::seq(vec![
                        Object::atom(&id),
                        Object::atom(&text),
                        Object::seq(vec![
                            Object::atom("Mandatory violation:"),
                            Object::atom(noun_name),
                            Object::atom(&format!("'{}'", instance)),
                            Object::atom("does not participate in"),
                            Object::atom(reading),
                        ]),
                    ]));
                }
            }
        }

        if violations.is_empty() { Object::phi() } else { Object::Seq(violations) }
    }))
}

/// FC: Frequency constraint â€” each value in the constrained role must occur
/// within [min_occurrence, max_occurrence] times in the fact type's population.
/// Per Halpin Ch 7.2: generalizes UC (FC with max=1 is a UC).
fn compile_frequency_ast(_ir: &Domain, def: &ConstraintDef) -> Func {
    let id = def.id.clone();
    let text = def.text.clone();
    let spans = resolve_spans(_ir, &def.spans);
    let min_occ = def.min_occurrence.unwrap_or(1);
    let max_occ = def.max_occurrence;

    let extractors: Vec<(Func, ResolvedSpan)> = spans.iter()
        .map(|s| (extract_facts_func(&s.fact_type_id), s.clone()))
        .collect();

    Func::Native(Arc::new(move |ctx: &Object| {
        let defs = HashMap::new();
        let mut violations = Vec::new();

        for (extractor, span) in &extractors {
            let facts_obj = crate::ast::apply(extractor, ctx, &defs);
            let facts = match facts_obj.as_seq() {
                Some(f) => f,
                None => continue,
            };

            let role_sel = role_value(span.role_index);
            let mut counts: HashMap<String, usize> = HashMap::new();
            for fact in facts {
                if let Some(val) = crate::ast::apply(&role_sel, fact, &defs).as_atom() {
                    *counts.entry(val.to_string()).or_insert(0) += 1;
                }
            }

            for (val, count) in counts {
                if count < min_occ || max_occ.map_or(false, |max| count > max) {
                    let range = match max_occ {
                        Some(max) if max == min_occ => format!("exactly {}", min_occ),
                        Some(max) => format!("between {} and {}", min_occ, max),
                        None => format!("at least {}", min_occ),
                    };
                    violations.push(Object::seq(vec![
                        Object::atom(&id),
                        Object::atom(&text),
                        Object::seq(vec![
                            Object::atom("Frequency violation:"),
                            Object::atom(&span.noun_name),
                            Object::atom(&format!("'{}'", val)),
                            Object::atom(&format!("occurs {} times in", count)),
                            Object::atom(&span.reading),
                            Object::atom(&format!("expected {}", range)),
                        ]),
                    ]));
                }
            }
        }

        if violations.is_empty() { Object::phi() } else { Object::Seq(violations) }
    }))
}

/// VC: Value constraint â€” each value in the constrained role must be in the
/// noun's allowed value set (enum_values). Per Halpin Ch 6.3.
fn compile_value_constraint_ast(ir: &Domain, def: &ConstraintDef) -> Func {
    let id = def.id.clone();
    let text = def.text.clone();

    // Collect allowed values from the nouns in the spanned fact types
    let spans = resolve_spans(ir, &def.spans);
    let allowed: Vec<(String, HashSet<String>)> = spans.iter().filter_map(|span| {
        let vals = ir.enum_values.get(&span.noun_name)?;
        if vals.is_empty() { return None; }
        Some((span.noun_name.clone(), vals.iter().cloned().collect::<HashSet<_>>()))
    }).collect();

    // If no enum values found on spanned nouns, check all nouns with enum_values
    let all_enum_nouns: Vec<(String, HashSet<String>)> = if allowed.is_empty() {
        ir.enum_values.iter().filter_map(|(name, vals)| {
            if vals.is_empty() { return None; }
            Some((name.clone(), vals.iter().cloned().collect::<HashSet<_>>()))
        }).collect()
    } else {
        Vec::new()
    };

    Func::Native(Arc::new(move |ctx: &Object| {
        let defs = HashMap::new();
        let check_nouns = if !allowed.is_empty() { &allowed } else { &all_enum_nouns };
        let mut violations = Vec::new();

        // Decode population to scan all facts
        let pop_obj = crate::ast::apply(&Func::Selector(3), ctx, &defs);
        let population = decode_population_object(&pop_obj);

        for (noun_name, valid_values) in check_nouns {
            for facts in population.facts.values() {
                for fact in facts {
                    for (name, val) in &fact.bindings {
                        if name == noun_name && !valid_values.contains(val) {
                            let valid_str = valid_values.iter().cloned().collect::<Vec<_>>().join(", ");
                            violations.push(Object::seq(vec![
                                Object::atom(&id),
                                Object::atom(&text),
                                Object::seq(vec![
                                    Object::atom("Value constraint violation:"),
                                    Object::atom(noun_name),
                                    Object::atom(&format!("'{}'", val)),
                                    Object::atom("is not in"),
                                    Object::atom(&format!("{{{}}}", valid_str)),
                                ]),
                            ]));
                        }
                    }
                }
            }
        }

        if violations.is_empty() { Object::phi() } else { Object::Seq(violations) }
    }))
}

/// XO/XC/OR: Set-comparison constraint â€” for each entity instance, count how many
/// of the clause fact types it participates in, and check against the requirement.
fn compile_set_comparison_ast(
    _ir: &Domain,
    def: &ConstraintDef,
    violates: fn(usize) -> bool,
    requirement: &'static str,
) -> Func {
    let id = def.id.clone();
    let text = def.text.clone();
    let entity_name = def.entity.clone().unwrap_or_default();
    let clause_ft_ids: Vec<String> = def.spans.iter()
        .map(|s| s.fact_type_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    Func::Native(Arc::new(move |ctx: &Object| {
        let defs = HashMap::new();
        let pop_obj = crate::ast::apply(&Func::Selector(3), ctx, &defs);
        let population = decode_population_object(&pop_obj);

        let all_instances = instances_of(&entity_name, &population);
        let clause_count = clause_ft_ids.len();

        let violations: Vec<Object> = all_instances.into_iter()
            .filter_map(|instance| {
                let holding = clause_ft_ids.iter()
                    .filter(|ft_id| participates_in(&instance, &entity_name, ft_id, &population))
                    .count();

                if violates(holding) {
                    Some(Object::seq(vec![
                        Object::atom(&id),
                        Object::atom(&text),
                        Object::seq(vec![
                            Object::atom("Set-comparison violation:"),
                            Object::atom(&entity_name),
                            Object::atom(&format!("'{}'", instance)),
                            Object::atom(&format!("has {} of {} clause fact types holding", holding, clause_count)),
                            Object::atom(&format!("expected {}", requirement)),
                        ]),
                    ]))
                } else {
                    None
                }
            })
            .collect();

        if violations.is_empty() { Object::phi() } else { Object::Seq(violations) }
    }))
}

/// SS: Subset constraint â€” pop(rs1) âŠ† pop(rs2).
/// For join-path subsets, checks that every tuple in fact type A
/// also exists in fact type B, matching by common noun names.
fn compile_subset_ast(ir: &Domain, def: &ConstraintDef) -> Func {
    let id = def.id.clone();
    let text = def.text.clone();

    if def.spans.len() < 2 {
        return Func::constant(Object::phi());
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

    let a_extractor = extract_facts_func(&a_ft_id);
    let b_extractor = extract_facts_func(&b_ft_id);

    Func::Native(Arc::new(move |ctx: &Object| {
        let defs = HashMap::new();
        let a_facts_obj = crate::ast::apply(&a_extractor, ctx, &defs);
        let b_facts_obj = crate::ast::apply(&b_extractor, ctx, &defs);

        let a_facts = a_facts_obj.as_seq().map(|s| s.as_ref()).unwrap_or(&[]);
        let b_facts = b_facts_obj.as_seq().map(|s| s.as_ref()).unwrap_or(&[]);

        // Build a set of common-noun value tuples from B for fast lookup
        let b_tuples: HashSet<Vec<String>> = b_facts.iter().map(|bf| {
            common_nouns.iter().map(|noun| {
                // Each fact is <<noun1, val1>, <noun2, val2>, ...>
                bf.as_seq().and_then(|bindings| {
                    bindings.iter().find_map(|b| {
                        let pair = b.as_seq()?;
                        if pair.len() == 2 && pair[0].as_atom() == Some(noun) {
                            pair[1].as_atom().map(|v| v.to_string())
                        } else {
                            None
                        }
                    })
                }).unwrap_or_default()
            }).collect()
        }).collect();

        // Check each fact in A â€” its common-noun tuple must exist in B
        let violations: Vec<Object> = a_facts.iter().filter_map(|a_fact| {
            let a_tuple: Vec<String> = common_nouns.iter().map(|noun| {
                a_fact.as_seq().and_then(|bindings| {
                    bindings.iter().find_map(|b| {
                        let pair = b.as_seq()?;
                        if pair.len() == 2 && pair[0].as_atom() == Some(noun) {
                            pair[1].as_atom().map(|v| v.to_string())
                        } else {
                            None
                        }
                    })
                }).unwrap_or_default()
            }).collect();

            if !b_tuples.contains(&a_tuple) {
                let pair_desc = common_nouns.iter().zip(a_tuple.iter())
                    .map(|(n, v)| format!("{} '{}'", n, v))
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(Object::seq(vec![
                    Object::atom(&id),
                    Object::atom(&text),
                    Object::seq(vec![
                        Object::atom("Subset violation:"),
                        Object::atom(&format!("({})", pair_desc)),
                        Object::atom("participates in"),
                        Object::atom(&a_ft_id),
                        Object::atom("but not in"),
                        Object::atom(&b_ft_id),
                    ]),
                ]))
            } else {
                None
            }
        }).collect();

        if violations.is_empty() { Object::phi() } else { Object::Seq(violations) }
    }))
}

/// EQ: Equality constraint â€” pop(rs1) = pop(rs2) (bidirectional subset).
/// Uses tuple-based comparison same as compile_subset_ast.
fn compile_equality_ast(ir: &Domain, def: &ConstraintDef) -> Func {
    let id = def.id.clone();
    let text = def.text.clone();

    if def.spans.len() < 2 {
        return Func::constant(Object::phi());
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

    let a_extractor = extract_facts_func(&a_ft_id);
    let b_extractor = extract_facts_func(&b_ft_id);

    Func::Native(Arc::new(move |ctx: &Object| {
        let defs = HashMap::new();
        let a_facts_obj = crate::ast::apply(&a_extractor, ctx, &defs);
        let b_facts_obj = crate::ast::apply(&b_extractor, ctx, &defs);

        let a_facts = a_facts_obj.as_seq().map(|s| s.as_ref()).unwrap_or(&[]);
        let b_facts = b_facts_obj.as_seq().map(|s| s.as_ref()).unwrap_or(&[]);

        // Helper to extract common-noun value tuples from fact Objects
        let extract_tuples = |facts: &[Object]| -> HashSet<Vec<String>> {
            facts.iter().map(|f| {
                common_nouns.iter().map(|noun| {
                    f.as_seq().and_then(|bindings| {
                        bindings.iter().find_map(|b| {
                            let pair = b.as_seq()?;
                            if pair.len() == 2 && pair[0].as_atom() == Some(noun) {
                                pair[1].as_atom().map(|v| v.to_string())
                            } else {
                                None
                            }
                        })
                    }).unwrap_or_default()
                }).collect()
            }).collect()
        };

        let a_tuples = extract_tuples(a_facts);
        let b_tuples = extract_tuples(b_facts);

        let mut violations = Vec::new();

        for tuple in a_tuples.difference(&b_tuples) {
            let desc = common_nouns.iter().zip(tuple.iter())
                .map(|(n, v)| format!("{} '{}'", n, v))
                .collect::<Vec<_>>().join(", ");
            violations.push(Object::seq(vec![
                Object::atom(&id),
                Object::atom(&text),
                Object::seq(vec![
                    Object::atom("Equality violation:"),
                    Object::atom(&format!("({})", desc)),
                    Object::atom("in"),
                    Object::atom(&a_ft_id),
                    Object::atom("but not in"),
                    Object::atom(&b_ft_id),
                ]),
            ]));
        }

        for tuple in b_tuples.difference(&a_tuples) {
            let desc = common_nouns.iter().zip(tuple.iter())
                .map(|(n, v)| format!("{} '{}'", n, v))
                .collect::<Vec<_>>().join(", ");
            violations.push(Object::seq(vec![
                Object::atom(&id),
                Object::atom(&text),
                Object::seq(vec![
                    Object::atom("Equality violation:"),
                    Object::atom(&format!("({})", desc)),
                    Object::atom("in"),
                    Object::atom(&b_ft_id),
                    Object::atom("but not in"),
                    Object::atom(&a_ft_id),
                ]),
            ]));
        }

        if violations.is_empty() { Object::phi() } else { Object::Seq(violations) }
    }))
}

/// Deontic: Forbidden constraint.
/// Uses Func::Selector(1) for response_text and Func::Selector(2) for sender_identity
/// from the eval context <response_text, sender_identity, population>.
fn compile_forbidden_ast(ir: &Domain, def: &ConstraintDef) -> Func {
    let id = def.id.clone();
    let text = def.text.clone();
    let forbidden_values = collect_enum_values(ir, &def.spans);
    let entity_scope = def.entity.clone();
    let text_keywords = extract_constraint_keywords(&text);

    // Deontic response constraints (entity = "Support Response" etc.) always apply
    // to the response being evaluated. Entity scoping only applies when the constraint
    // is about a domain entity that may or may not be referenced in the text.
    let is_response_constraint = def.entity.as_ref()
        .map_or(false, |e| e.to_lowercase().contains("response"));

    Func::Native(Arc::new(move |ctx: &Object| {
        let defs = HashMap::new();
        let response_text = crate::ast::apply(&Func::Selector(1), ctx, &defs);
        let response_str = response_text.as_atom().unwrap_or("").to_string();
        let lower_text = response_str.to_lowercase();

        // Entity scoping: skip for response constraints (they always apply).
        // For other constraints, only evaluate when the response references the entity.
        if !is_response_constraint {
            if let Some(ref entity) = entity_scope {
                let entity_lower = entity.to_lowercase();
                let entity_compact: String = entity.chars().filter(|c| !c.is_whitespace()).collect();
                let entity_compact_lower = entity_compact.to_lowercase();
                if !lower_text.contains(&entity_lower) && !lower_text.contains(&entity_compact_lower) {
                    return Object::phi();
                }
            }
        }

        let mut violations = Vec::new();
        let mut seen = HashSet::new();

        // Enum-based check (exact value match)
        for (noun_name, enum_vals) in &forbidden_values {
            for val in enum_vals {
                let lower_val = val.to_lowercase();
                if lower_text.contains(&lower_val) {
                    let detail_str = format!(
                        "Response contains forbidden {}: '{}'",
                        noun_name, val
                    );
                    if seen.insert(detail_str.clone()) {
                        violations.push(Object::seq(vec![
                            Object::atom(&id),
                            Object::atom(&text),
                            Object::seq(vec![
                                Object::atom("Response contains forbidden"),
                                Object::atom(noun_name),
                                Object::atom(&format!("'{}'", val)),
                            ]),
                        ]));
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
            if matched.len() > text_keywords.len() / 2 && matched.len() >= 2 {
                violations.push(Object::seq(vec![
                    Object::atom(&id),
                    Object::atom(&text),
                    Object::seq(vec![
                        Object::atom("Response may violate:"),
                        Object::atom(&text),
                        Object::atom(&format!("(matched keywords: {})", matched.join(", "))),
                    ]),
                ]));
            }
        }

        if violations.is_empty() { Object::phi() } else { Object::Seq(violations) }
    }))
}

/// Deontic: Obligatory constraint.
/// Uses Func::Selector(1) for response_text and Func::Selector(2) for sender_identity
/// from the eval context <response_text, sender_identity, population>.
fn compile_obligatory_ast(ir: &Domain, def: &ConstraintDef) -> Func {
    let id = def.id.clone();
    let text = def.text.clone();
    let obligatory_values = collect_enum_values(ir, &def.spans);
    let checks_sender = def.text.to_lowercase().contains("senderidentity");
    let entity_scope = def.entity.clone();

    let text_keywords = if obligatory_values.is_empty() {
        extract_constraint_keywords(&text)
    } else {
        Vec::new()
    };

    let is_response_constraint = def.entity.as_ref()
        .map_or(false, |e| e.to_lowercase().contains("response"));

    Func::Native(Arc::new(move |ctx: &Object| {
        let defs = HashMap::new();
        let response_text = crate::ast::apply(&Func::Selector(1), ctx, &defs);
        let response_str = response_text.as_atom().unwrap_or("").to_string();
        let lower_text = response_str.to_lowercase();

        // Entity scoping: skip for response constraints (they always apply)
        if !is_response_constraint {
            if let Some(ref entity) = entity_scope {
                let entity_lower = entity.to_lowercase();
                let entity_compact: String = entity.chars().filter(|c| !c.is_whitespace()).collect();
                let entity_compact_lower = entity_compact.to_lowercase();
                if !lower_text.contains(&entity_lower) && !lower_text.contains(&entity_compact_lower) {
                    return Object::phi();
                }
            }
        }

        let mut violations = Vec::new();

        // Enum-based check
        for (noun_name, enum_vals) in &obligatory_values {
            let found = enum_vals.iter().any(|val| lower_text.contains(&val.to_lowercase()));
            if !found {
                violations.push(Object::seq(vec![
                    Object::atom(&id),
                    Object::atom(&text),
                    Object::seq(vec![
                        Object::atom("Response missing obligatory"),
                        Object::atom(noun_name),
                        Object::atom(&format!("expected one of {:?}", enum_vals)),
                    ]),
                ]));
            }
        }

        // Sender identity check
        if checks_sender {
            let sender_obj = crate::ast::apply(&Func::Selector(2), ctx, &defs);
            let sender_empty = match sender_obj.as_atom() {
                Some(s) => s.is_empty(),
                None => true,
            };
            if sender_empty {
                violations.push(Object::seq(vec![
                    Object::atom(&id),
                    Object::atom(&text),
                    Object::seq(vec![
                        Object::atom("Response missing obligatory SenderIdentity"),
                    ]),
                ]));
            }
        }

        // Text-based: include the obligation as metadata
        let _ = &text_keywords;

        if violations.is_empty() { Object::phi() } else { Object::Seq(violations) }
    }))
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
        // Split PascalCase: AutomotiveData â†’ automotive, data
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

// â”€â”€ State Machine Compilation â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
// State machines compile to transition functions.
// run_machine = fold(transition)(initial)(stream)

fn compile_state_machine(
    def: &StateMachineDef,
    constraints: &[CompiledConstraint],
) -> CompiledStateMachine {
    // Build constraint ID â†’ func index for guard lookup
    let constraint_by_id: HashMap<&str, &crate::ast::Func> = constraints.iter()
        .map(|c| (c.id.as_str(), &c.func))
        .collect();

    let initial = def.statuses.first().cloned().unwrap_or_default();

    // â”€â”€ Hierarchical composition (Harel statecharts) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    // If a transition's source is the SM Definition name (which IS a Status
    // per the subtype relationship), expand it to all statuses in this machine.
    // A transition from the parent state exits all children.
    let sm_name = &def.noun_name;
    let defined_statuses: Vec<&str> = def.statuses.iter()
        .map(|s| s.as_str())
        .filter(|s| *s != sm_name.as_str())
        .collect();

    // Start with explicit transitions (owned, with guards)
    struct ExpandedTransition {
        from: String,
        to: String,
        event: String,
        guard: Option<GuardDef>,
    }

    let mut expanded: Vec<ExpandedTransition> = def.transitions.iter()
        .map(|t| ExpandedTransition {
            from: t.from.clone(),
            to: t.to.clone(),
            event: t.event.clone(),
            guard: t.guard.clone(),
        })
        .collect();

    // Collect parent-state transitions for expansion
    let parent_transitions: Vec<(String, String, Option<GuardDef>)> = expanded.iter()
        .filter(|t| t.from == sm_name.as_str())
        .map(|t| (t.to.clone(), t.event.clone(), t.guard.clone()))
        .collect();

    for (to, event, guard) in &parent_transitions {
        for status in &defined_statuses {
            let already_exists = expanded.iter()
                .any(|t| t.from == *status && t.event == *event);
            if !already_exists {
                expanded.push(ExpandedTransition {
                    from: status.to_string(),
                    to: to.clone(),
                    event: event.clone(),
                    guard: guard.clone(),
                });
            }
        }
    }

    let transition_table: Vec<(String, String, String)> = expanded.iter()
        .map(|t| (t.from.clone(), t.to.clone(), t.event.clone()))
        .collect();

    // AST: transition function <current_state, event> â†’ next_state.
    //
    // Without guards:
    //   (eq âˆ˜ [id, <from, event>]) â†’ target; next
    //
    // With guards (guard_passes âˆ§ match):
    //   (null âˆ˜ guard_func âˆ˜ ... âˆ§ eq âˆ˜ [id, <from, event>]) â†’ target; next
    //
    // Guard passes iff the constraint func returns Ï† (empty = no violations).
    let mut sm_func = crate::ast::Func::Selector(1); // fallback: return current state

    for t in expanded.iter().rev() {
        // Match predicate: <current_state, event> == <from, event>
        let match_pred = crate::ast::Func::compose(
            crate::ast::Func::Eq,
            crate::ast::Func::construction(vec![
                crate::ast::Func::Id,
                crate::ast::Func::constant(crate::ast::Object::seq(vec![
                    crate::ast::Object::atom(&t.from),
                    crate::ast::Object::atom(&t.event),
                ])),
            ]),
        );

        // If transition has guards, compose them with the match predicate.
        // Guard passes iff all constraint funcs produce Ï† (no violations).
        let pred = if let Some(ref guard) = t.guard {
            let guard_funcs: Vec<&crate::ast::Func> = guard.constraint_ids.iter()
                .filter_map(|cid| constraint_by_id.get(cid.as_str()).copied())
                .collect();

            if guard_funcs.is_empty() {
                match_pred
            } else {
                // Build: null_test âˆ˜ guard_func (returns T if guard produces Ï†)
                // For multiple guards: all must pass
                let mut guard_check = crate::ast::Func::compose(
                    crate::ast::Func::NullTest,
                    guard_funcs[0].clone(),
                );
                for gf in &guard_funcs[1..] {
                    // AND: both must be true (NullTest returns T/F)
                    let next_check = crate::ast::Func::compose(
                        crate::ast::Func::NullTest,
                        (*gf).clone(),
                    );
                    // Compose as: if guard1_passes then check guard2+match else id
                    guard_check = crate::ast::Func::condition(
                        guard_check,
                        next_check,
                        crate::ast::Func::constant(crate::ast::Object::atom("F")),
                    );
                }
                // Final: if guards pass AND state+event match â†’ fire
                crate::ast::Func::condition(
                    guard_check,
                    match_pred,
                    crate::ast::Func::constant(crate::ast::Object::atom("F")),
                )
            }
        } else {
            match_pred
        };

        sm_func = crate::ast::Func::condition(
            pred,
            crate::ast::Func::constant(crate::ast::Object::atom(&t.to)),
            sm_func,
        );
    }

    CompiledStateMachine {
        noun_name: def.noun_name.clone(),
        statuses: def.statuses.clone(),
        initial,
        transition_table,
        func: sm_func,
    }
}

// â”€â”€ Schema Compilation Tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

#[cfg(test)]
mod schema_tests {
    use super::*;
    use crate::ast::{self, Object};

    fn make_ir_with_fact_type(id: &str, reading: &str, roles: Vec<(&str, usize)>) -> Domain {
        let mut fact_types = HashMap::new();
        fact_types.insert(id.to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: reading.to_string(),
            readings: vec![],
            roles: roles.iter().map(|(name, idx)| RoleDef {
                noun_name: name.to_string(),
                role_index: *idx,
            }).collect(),
        });
        Domain {
            domain: "test".to_string(),
            nouns: HashMap::new(),
            fact_types,
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![], general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(), objectifications: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
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

        // Apply construction to a fact â€” identity (selects each role)
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
        let _schema = model.schemas.get("ft1").unwrap();
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
        // Î±(Selector(2)) over a population extracts role 2 from each fact
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

        // Extract all users: Î±(2):population
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
            schema_id: String::new(),
            reading: "Person has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        let ir = Domain {
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
            derivation_rules: vec![], general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(), objectifications: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
        };

        let model = compile(&ir);
        let constraint = &model.constraints[0];

        // Create a population WITH a UC violation: Alice has two names
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
        let ctx_obj = crate::ast::encode_eval_context("", None, &population);
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
            schema_id: String::new(),
            reading: "Person has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        let ir = Domain {
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
            derivation_rules: vec![], general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(), objectifications: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
        };

        let model = compile(&ir);
        let constraint = &model.constraints[0];

        // No violation: each person has exactly one name
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

        let ctx_obj = crate::ast::encode_eval_context("", None, &population);
        let defs = HashMap::new();
        let result = crate::ast::apply(&constraint.func, &ctx_obj, &defs);

        // No violations â€” should be phi (empty sequence)
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
        // This is a 3-step chain: User â†’ OrgMembership â†’ Organization â†’ Domain
        let mut fact_types = HashMap::new();
        let mut nouns = HashMap::new();

        // ft1: OrgMembership is for User (roles: OrgMembership[0], User[1])
        fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "OrgMembership is for User".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "OrgMembership".to_string(), role_index: 0 },
                RoleDef { noun_name: "User".to_string(), role_index: 1 },
            ],
        });

        // ft2: OrgMembership is in Organization (roles: OrgMembership[0], Organization[1])
        fact_types.insert("ft2".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "OrgMembership is in Organization".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "OrgMembership".to_string(), role_index: 0 },
                RoleDef { noun_name: "Organization".to_string(), role_index: 1 },
            ],
        });

        // ft3: Domain belongs to Organization (roles: Domain[0], Organization[1])
        fact_types.insert("ft3".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Domain belongs to Organization".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Domain".to_string(), role_index: 0 },
                RoleDef { noun_name: "Organization".to_string(), role_index: 1 },
            ],
        });

        for name in &["User", "OrgMembership", "Organization", "Domain"] {
            nouns.insert(name.to_string(), NounDef {
                object_type: "entity".to_string(),
                world_assumption: WorldAssumption::Closed,
            });
        }

        let ir = Domain {
            domain: "test".to_string(),
            nouns,
            fact_types,
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![], general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(), objectifications: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
        };

        // Chain: User â†’ (ft1) â†’ OrgMembership â†’ (ft2) â†’ Organization
        // Step 1: User is role 2 in ft1, output is OrgMembership (role 1)
        // Step 2: OrgMembership is role 1 in ft2, output is Organization (role 2)
        let chain = compile_derivation_chain(
            &ir,
            &["ft1".to_string(), "ft2".to_string()],
            "User",
            "Organization",
        );
        assert!(chain.is_some(), "chain should compile");

        // Chain: Organization â†’ (ft3, reversed) â†’ Domain
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

    #[test]
    fn project_entity_maps_fields_to_schemas() {
        // Simulate an entity with fields that correspond to compiled schemas.
        // The entity "Customer" has fields "name" and "plan".
        let mut fact_types = HashMap::new();
        fact_types.insert("schema-uuid-1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Customer has name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "name".to_string(), role_index: 1 },
            ],
        });
        fact_types.insert("schema-uuid-2".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Customer has plan".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "plan".to_string(), role_index: 1 },
            ],
        });
        let ir = Domain {
            domain: "test".to_string(),
            nouns: HashMap::new(),
            fact_types,
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![], general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(), objectifications: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
        };

        let model = compile(&ir);

        // Verify noun_to_fact_types has both schemas for "Customer" at role 0
        let customer_fts = model.noun_index.noun_to_fact_types.get("Customer").unwrap();
        let role0_fts: Vec<&str> = customer_fts.iter()
            .filter(|(_, idx)| *idx == 0)
            .map(|(id, _)| id.as_str())
            .collect();
        assert_eq!(role0_fts.len(), 2, "Customer plays role 0 in 2 schemas");

        // Verify we can map fields to schemas via role_names[1]
        for (ft_id, role_idx) in customer_fts {
            if *role_idx != 0 { continue; }
            let schema = model.schemas.get(ft_id).unwrap();
            assert_eq!(schema.role_names[0], "Customer");
            // role_names[1] should be "name" or "plan"
            assert!(
                schema.role_names[1] == "name" || schema.role_names[1] == "plan",
                "unexpected role_names[1]: {}", schema.role_names[1]
            );
        }
    }

    #[test]
    fn project_entity_unmatched_fields_remain_provisional() {
        // An entity with a field that has no compiled schema should still
        // be projectable (with a provisional string-concatenated ID).
        let ir = make_ir_with_fact_type(
            "ft1", "Order has total",
            vec![("Order", 0), ("total", 1)],
        );
        let model = compile(&ir);

        // "total" matches schema ft1. "notes" has no schema.
        let order_fts = model.noun_index.noun_to_fact_types.get("Order").unwrap();
        let field_map: HashMap<&str, &str> = order_fts.iter()
            .filter(|(_, idx)| *idx == 0)
            .filter_map(|(id, _)| {
                model.schemas.get(id).and_then(|s| {
                    if s.role_names.len() >= 2 {
                        Some((s.role_names[1].as_str(), id.as_str()))
                    } else { None }
                })
            })
            .collect();

        assert!(field_map.contains_key("total"), "total should map to ft1");
        assert!(!field_map.contains_key("notes"), "notes should not have a schema mapping");
    }
}
