// crates/arest/src/compile.rs
//
// Compilation: Object -> CompiledModel
//
// Constraints ARE predicates, not data that gets matched.
// The match on constraint kind happens once at compile time. After compilation,
// evaluation is pure function application -- no dispatch, no branching on kind.
//
// This implements Backus's FP algebra (1977 Turing Lecture):
//   - Constraints and derivations compile to pure functions (combining forms)
//   - Evaluation is function application over whole structures
//   - State machines are folds: run_machine = fold(transition)(initial)(stream)
//   - No variables, no mutable state during evaluation -- only reduction

use hashbrown::{HashMap, HashSet};
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};
use crate::ast::{fetch_or_phi, binding};

// WASM-safe timing shim. The wasm32-unknown-unknown target panics on
// std::time::Instant::now() (the Rust stdlib has no clock there). On
// native builds we use real timing; on WASM we return a zero-duration
// stub so the profile eprintlns still print but with 0ns.
// Timing shim. Real clock on native std builds; zero-duration stub on
// WASM (no clock) and no_std (no std::time). The profile eprintlns
// still compile but report 0ns where no clock is available.
#[cfg(all(not(target_arch = "wasm32"), not(feature = "no_std")))]
mod profile_timer {
    pub type Timer = std::time::Instant;
    pub fn now() -> Timer { std::time::Instant::now() }
}
#[cfg(any(target_arch = "wasm32", feature = "no_std"))]
mod profile_timer {
    #[derive(Clone, Copy)]
    pub struct Timer;
    impl Timer {
        pub fn elapsed(&self) -> core::time::Duration { core::time::Duration::ZERO }
    }
    pub fn now() -> Timer { Timer }
}
use crate::types::*;

// Re-export DerivedFact-related types used by derivation compilers
// (already imported via crate::types::*)

// -- Core Functional Types ------------------------------------------

// -- Core Functional Types ------------------------------------------
//
// Constraints, derivations, and state machines compile to Func AST nodes.
// Evaluation is beta reduction: apply(func, object, defs) -> object.
//
// All constraints compile to AST (Func) nodes:
//   Pure AST:       IR (fully pure -- Filter + Eq + Construction, zero closures)
//   AST + Native:   UC, MC, FC, VC, AS, SY, AT, IT, TR, AC, RF,
//                   XO, XC, OR, SS, EQ, forbidden, obligatory
//                   (extract_facts_func for fact extraction,
//                   Native kernel for constraint-specific logic)
//   Constant:       Permitted (Func::constant(phi))
//   Goal:           all constraints as pure Func (Condition, Filter, Compose, etc.)


#[derive(Clone, Debug)]
pub(crate) enum Modality {
    Alethic,
    Deontic(DeonticOp),
}

#[derive(Clone, Debug)]
pub(crate) enum DeonticOp {
    Forbidden,
    Obligatory,
    Permitted,
}

/// A compiled constraint. Evaluation is apply(func, eval_context_object) -> violations.
/// text/modality retained for introspection (used by explain, verify).
#[allow(dead_code)]
pub(crate) struct CompiledConstraint {
    pub(crate) id: String,
    pub(crate) text: String,
    pub(crate) modality: Modality,
    pub(crate) func: crate::ast::Func,
}


/// A compiled derivation rule. Evaluation is apply(func, population_object) -> derived facts.
/// text/kind retained for introspection and rule classification.
#[allow(dead_code)]
pub(crate) struct CompiledDerivation {
    pub(crate) id: String,
    pub(crate) text: String,
    pub(crate) kind: DerivationKind,
    pub(crate) func: crate::ast::Func,
}

/// A compiled state machine. func is the transition function: <state, event> -> state'.
/// statuses retained for introspection (machine:{noun}:statuses could expose it).
#[allow(dead_code)]
pub(crate) struct CompiledStateMachine {
    pub(crate) noun_name: String,
    pub(crate) statuses: Vec<String>,
    pub(crate) initial: String,
    pub(crate) func: crate::ast::Func,
    pub(crate) transition_table: Vec<(String, String, String)>,
}

/// Index for fast noun lookups during synthesis.
/// Several fields populated for potential use by downstream passes;
/// not all are consumed today.
#[allow(dead_code)]
pub(crate) struct NounIndex {
    /// noun_name -> list of (fact_type_id, role_index) where noun plays a role
    pub(crate) noun_to_fact_types: HashMap<String, Vec<(String, usize)>>,
    /// noun_name -> world assumption
    pub(crate) world_assumptions: HashMap<String, WorldAssumption>,
    /// noun_name -> supertype name
    #[allow(dead_code)] // deserialized from JSON, read by JS callers
    pub(crate) supertypes: HashMap<String, String>,
    /// noun_name -> list of subtype names
    #[allow(dead_code)] // deserialized from JSON, read by JS callers
    pub(crate) subtypes: HashMap<String, Vec<String>>,
    /// fact_type_id -> list of constraint IDs spanning it
    pub(crate) fact_type_to_constraints: HashMap<String, Vec<String>>,
    /// constraint_id -> index into CompiledModel.constraints
    pub(crate) constraint_index: HashMap<String, usize>,
    /// noun_name -> reference scheme value type names (e.g., ["Order Number"])
    pub(crate) ref_schemes: HashMap<String, Vec<String>>,
    /// noun_name -> state machine index
    pub(crate) noun_to_state_machines: HashMap<String, usize>,
}

/// A compiled fact type -- a Construction of Selector functions (roles).
/// Fact Type = CONS(Role1, ..., Rolen) in Backus's FP algebra.
/// Partial application = query. Full application = fact.
pub(crate) struct CompiledSchema {
    pub(crate) id: String,
    pub(crate) reading: String,
    /// The Construction function: [Selector(1), Selector(2), ..., Selector(n)]
    pub(crate) construction: crate::ast::Func,
    /// Role names in order (for binding resolution)
    pub(crate) role_names: Vec<String>,
}

/// The compiled model -- all constraints, derivations, state machines, and schemas as executable functions.
/// noun_index / fact_events populated for introspection.
#[allow(dead_code)]
pub(crate) struct CompiledModel {
    pub(crate) constraints: Vec<CompiledConstraint>,
    pub(crate) derivations: Vec<CompiledDerivation>,
    pub(crate) state_machines: Vec<CompiledStateMachine>,
    pub(crate) noun_index: NounIndex,
    /// Fact types compiled to Construction functions (CONS of Roles).
    pub(crate) schemas: HashMap<String, CompiledSchema>,
    /// Fact-to-event mapping: when a fact of this type is created, fire this event
    /// on the state machine for the target noun. Derived from:
    ///   Fact Type is activated by Verb + Verb is performed during Transition.
    pub(crate) fact_events: HashMap<String, FactEvent>,
}

/// When a fact is created in this schema, fire this event on the entity's state machine.
/// Populated by compile_fact_events; consumed by the SM dispatch layer (future).
#[allow(dead_code)]
pub(crate) struct FactEvent {
    pub(crate) fact_type_id: String,
    pub(crate) event_name: String,
    pub(crate) target_noun: String, // which noun's state machine to transition
}

// (decode_population_object removed -- no longer needed after eliminating all Func::Native closures)

// -- Schema Compilation -------------------------------------------
// Compile fact types to Construction functions (CONS of Roles).
// Role -> Selector. Fact Type -> Construction [Selector1, ..., Selectorn].

/// Compile all fact types in the IR to CompiledSchema (Construction of Selectors).
fn compile_schemas(data: &CellIndex) -> HashMap<String, CompiledSchema> {
    data.fact_types.iter().map(|(id, ft)| {
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

// (Population-struct primitives instances_of/participates_in removed --
//  replaced by pure Func equivalents instances_of_noun_func/extract_facts_func)

// -- AST Constraint Builders ------------------------------------------
// Pure Func constructors for constraint evaluation.
// Each builds a Func that takes an eval context Object and returns violations.
//
// Eval context encoding: <response_text, sender_identity, population>
// Population encoding:   <ft1, ft2, ...> where ft = <ft_id, <fact1, ...>>
// Fact encoding:         <<noun1, val1>, <noun2, val2>, ...>

use crate::ast::{Func, Object};

/// Build a Func that extracts facts for a given fact_type_id from the population.
/// Input: eval context <response, sender, population>
/// Output: <fact1, fact2, ...> or phi
fn extract_facts_func(ft_id: &str) -> Func {
    // sel(4) -> indexed population (Object::Map keyed by ft_id)
    // FetchOrPhi:<ft_id, indexed_pop> -> facts seq, or phi if absent
    //
    // Replaces the old Filter+Eq linear scan over Selector(3) with a
    // single O(1) HashMap lookup. The old form fired ~5M times per
    // create on metamodel-scale workloads (profile 0ea23a9); the new
    // form is a constant-time Fetch.
    Func::compose(
        Func::FetchOrPhi,
        Func::construction(vec![
            Func::constant(Object::atom(ft_id)),
            Func::Selector(4),
        ]),
    )
}

/// Extract facts from a population Object directly (no eval context wrapper).
/// Used by derivation compilers which receive population, not ctx.
fn extract_facts_from_pop(ft_id: &str) -> Func {
    let find_ft = Func::filter(
        Func::compose(Func::Eq, Func::construction(vec![
            Func::Selector(1),
            Func::constant(Object::atom(ft_id)),
        ])),
    );
    let get_or_phi = Func::condition(
        Func::NullTest,
        Func::constant(Object::phi()),
        Func::compose(Func::Selector(2), Func::Selector(1)),
    );
    Func::compose(get_or_phi, find_ft)
}

/// Find all instances of a noun across all fact types in a population Object.
/// instances_of_noun(noun) : pop -> <val1, val2, ...>
fn instances_of_noun_func(noun_name: &str) -> Func {
    // For each ft entry <ft_id, <facts>>, get facts (Selector(2)),
    // for each fact, filter bindings matching noun, extract values.
    let match_noun = Func::compose(Func::Eq, Func::construction(vec![
        Func::Selector(1),
        Func::constant(Object::atom(noun_name)),
    ]));
    let extract_val = Func::compose(
        Func::apply_to_all(Func::Selector(2)),
        Func::filter(match_noun),
    );
    // For each fact: extract_val applied to the fact's bindings (the fact IS the bindings seq)
    let vals_per_ft = Func::compose(
        Func::Concat,
        Func::compose(Func::apply_to_all(extract_val), Func::Selector(2)),
    );
    // For each ft entry in pop: vals_per_ft
    // Then concat all results
    Func::compose(Func::Concat, Func::apply_to_all(vals_per_ft))
}

/// Build a Func that extracts facts for multiple fact type IDs.
/// Returns the concatenation of all facts from all matching fact types.
/// Concat . [extract_ft1, extract_ft2, ...] : ctx -> <all facts>
fn extract_facts_multi(ft_ids: &[String]) -> Func {
    let extractors: Vec<Func> = ft_ids.iter().map(|id| extract_facts_func(id)).collect();
    match extractors.len() {
        1 => extractors.into_iter().next().unwrap(),
        _ => Func::compose(Func::Concat, Func::construction(extractors)),
    }
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
/// Fact encoding: <<noun1, val1>, <noun2, val2>, ...>
/// Role value at index i: sel(2)  .  sel(i+1)
fn role_value(role_index: usize) -> Func {
    Func::compose(Func::Selector(2), Func::Selector(role_index + 1))
}

// -- Span Resolution ------------------------------------------------
// Resolves IR references at compile time so predicates capture only what they need.

#[derive(Clone)]
struct ResolvedSpan {
    fact_type_id: String,
    role_index: usize,
    noun_name: String,
    reading: String,
}

fn resolve_spans(data: &CellIndex, spans: &[SpanDef]) -> Vec<ResolvedSpan> {
    spans.iter().filter_map(|span| {
        let ft = data.fact_types.get(&span.fact_type_id)?;
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
/// Deduplicates by noun name -- each noun's enum values appear at most once.
fn collect_enum_values(data: &CellIndex, spans: &[SpanDef]) -> Vec<(String, Vec<String>)> {
    // Î±(span â†’ roles) : spans â†’ flat_map â†’ filter(has_enum âˆ§ Â¬seen) â†’ deduplicate
    spans.iter()
        .filter_map(|span| data.fact_types.get(&span.fact_type_id))
        .flat_map(|ft| ft.roles.iter())
        .filter_map(|role| data.enum_values.get(&role.noun_name)
            .filter(|vals| !vals.is_empty())
            .map(|vals| (role.noun_name.clone(), vals.clone())))
        .fold((HashSet::new(), Vec::new()), |(mut seen, mut result), (name, vals)| {
            seen.insert(name.clone()).then(|| result.push((name, vals)));
            (seen, result)
        }).1
}

/// Derive state machines from instance facts in P.
/// Queries the population for metamodel fact types.
fn derive_state_machines_from_facts(facts: &[GeneralInstanceFact]) -> HashMap<String, StateMachineDef> {
    let machines: HashMap<String, StateMachineDef> = HashMap::new();

    // Pass 1: fold over facts â†’ machines (SM Definition 'X' is for Noun 'Y')
    let mut machines = facts.iter()
        .filter(|f| f.subject_noun == "State Machine Definition" && f.object_noun == "Noun")
        .fold(machines, |mut acc, f| {
            acc.entry(f.subject_value.clone()).or_insert_with(|| StateMachineDef {
                noun_name: f.object_value.clone(), ..Default::default()
            });
            acc
        });

    // Pass 2: set sm.initial explicitly from declared facts of the form
    // `Status 'S' is initial in State Machine Definition 'X'` and append
    // S to the status set. This is the paper's §4 declaration; the
    // assignment goes to sm.initial, not to statuses[0] — position-in-
    // list is not initial-hood.
    let initial_decls: Vec<(String, String)> = facts.iter()
        .filter(|f| f.subject_noun == "Status"
            && f.object_noun == "State Machine Definition"
            && f.field_name.to_lowercase().contains("initial"))
        .map(|f| (f.object_value.clone(), f.subject_value.clone()))
        .collect();
    initial_decls.into_iter().for_each(|(sm_key, status)| {
        if let Some(sm) = machines.get_mut(&sm_key) {
            if !sm.statuses.contains(&status) { sm.statuses.push(status.clone()); }
            if sm.initial.is_empty() { sm.initial = status; }
        }
    });

    // Pass 2b: non-initial `Status 'S' is defined in SM 'X'` facts just
    // register the status; they do not set initial.
    let status_decls: Vec<(String, String)> = facts.iter()
        .filter(|f| f.subject_noun == "Status"
            && f.object_noun == "State Machine Definition"
            && !f.field_name.to_lowercase().contains("initial"))
        .map(|f| (f.object_value.clone(), f.subject_value.clone()))
        .collect();
    status_decls.into_iter().for_each(|(sm_key, status)| {
        machines.get_mut(&sm_key).into_iter()
            .filter(|sm| !sm.statuses.contains(&status))
            .for_each(|sm| sm.statuses.push(status.clone()));
    });

    // Pass 3: fold transition facts into lookup maps
    let (t_from, t_to, t_sm, t_event) = facts.iter()
        .filter(|f| f.subject_noun == "Transition")
        .fold(
            (HashMap::new(), HashMap::new(), HashMap::new(), HashMap::<String,String>::new()),
            |(mut from, mut to, mut sm, mut event), f| {
                match f.object_noun.as_str() {
                    "Status" => {
                        let field_lower = f.field_name.to_lowercase();
                        // Mutually exclusive: "from" takes priority, "to" only if no "from"
                        match (field_lower.contains("from"), field_lower.contains("to")) {
                            (true, _) => { from.insert(f.subject_value.clone(), f.object_value.clone()); }
                            (false, true) => { to.insert(f.subject_value.clone(), f.object_value.clone()); }
                            _ => {}
                        }
                    }
                    "State Machine Definition" => { sm.insert(f.subject_value.clone(), f.object_value.clone()); }
                    "Event Type" => { event.insert(f.subject_value.clone(), f.object_value.clone()); }
                    _ => {}
                };
                (from, to, sm, event)
            },
        );

    // Assemble: Î±(transition_name â†’ add_to_machine) over unique transition names
    t_from.keys().chain(t_to.keys()).collect::<HashSet<_>>().into_iter()
        .filter_map(|t_name| {
            let from = t_from.get(t_name)?.clone();
            let to = t_to.get(t_name)?.clone();
            let event = t_event.get(t_name).cloned().unwrap_or_else(|| t_name.clone());
            // Prefer the explicit `Transition X is defined in SM Y` fact.
            // Otherwise infer: require BOTH endpoints in the same SM's declared
            // statuses â€” an OR-based match cross-contaminates when two SMs share
            // a status name. If AND finds nothing, fall back to a unique OR
            // match; only fall through to the first key as a last resort.
            let target = t_sm.get(t_name).cloned()
                .or_else(|| machines.iter()
                    .find(|(_, sm)| sm.statuses.contains(&from) && sm.statuses.contains(&to))
                    .map(|(k, _)| k.clone()))
                .or_else(|| {
                    let matches: Vec<String> = machines.iter()
                        .filter(|(_, sm)| sm.statuses.contains(&from) || sm.statuses.contains(&to))
                        .map(|(k, _)| k.clone())
                        .collect();
                    if matches.len() == 1 { matches.into_iter().next() } else { None }
                })
                .or_else(|| machines.keys().next().cloned());
            Some((target?, from, to, event))
        })
        .collect::<Vec<_>>()
        .into_iter()
        .for_each(|(key, from, to, event)| {
            machines.get_mut(&key).into_iter().for_each(|sm| {
                (!sm.statuses.contains(&from)).then(|| sm.statuses.push(from.clone()));
                (!sm.statuses.contains(&to)).then(|| sm.statuses.push(to.clone()));
                sm.transitions.push(TransitionDef { from: from.clone(), to: to.clone(), event: event.clone(), guard: None });
            });
        });

    // Pass 4: derive initial status from transition facts only when no
    // explicit declaration was asserted. A status is "initial" by graph
    // topology if it appears as the source of some transition but never
    // as a target — no other status can reach it, so it must be where
    // the fold starts. Both endpoints are transition facts per §5.1, so
    // this is a derivation from facts (Thm 5), not a positional fallback.
    //
    // If sm.initial was set in Pass 2, leave it. Otherwise set it only
    // when graph topology gives a UNIQUE source-never-target. When
    // ambiguous (multiple or cyclic), leave sm.initial empty: the
    // machine has no declared start, and compile_state_machine will
    // emit an empty initial that fails visibly at the first SM call.
    // No insertion-order / first-declared fallback — that was the
    // "hardcoded init" this task rejects.
    for sm in machines.values_mut() {
        if !sm.initial.is_empty() { continue; }
        if sm.transitions.is_empty() { continue; }
        let sources: HashSet<&str> = sm.transitions.iter().map(|t| t.from.as_str()).collect();
        let targets: HashSet<&str> = sm.transitions.iter().map(|t| t.to.as_str()).collect();
        let graph_initials: Vec<String> = sm.statuses.iter()
            .filter(|s| sources.contains(s.as_str()) && !targets.contains(s.as_str()))
            .cloned()
            .collect();
        if graph_initials.len() == 1 {
            sm.initial = graph_initials.into_iter().next().unwrap();
        }
    }

    machines
}

// -- Compilation ----------------------------------------------------
// The match on kind happens here, once. After this, everything is Func.

// Compile an Object state into named FFP definitions.
// All generators always produce all defs. Selection is at apply time:
// SYSTEM:sql:sqlite:Order returns DDL, SYSTEM:xsd:Order returns XSD.
#[cfg(not(feature = "no_std"))]
thread_local! {
    static ACTIVE_GENERATORS: core::cell::RefCell<HashSet<String>> = core::cell::RefCell::new(HashSet::new());
}
#[cfg(not(feature = "no_std"))]
pub fn set_active_generators(gens: HashSet<String>) { ACTIVE_GENERATORS.with(|g| *g.borrow_mut() = gens); }
#[cfg(not(feature = "no_std"))]
fn active_generators() -> HashSet<String> { ACTIVE_GENERATORS.with(|g| g.borrow().clone()) }

#[cfg(feature = "no_std")]
static ACTIVE_GENERATORS_GLOBAL: crate::sync::Mutex<Option<HashSet<String>>> = crate::sync::Mutex::new(None);
#[cfg(feature = "no_std")]
pub fn set_active_generators(gens: HashSet<String>) { *ACTIVE_GENERATORS_GLOBAL.lock() = Some(gens); }
#[cfg(feature = "no_std")]
fn active_generators() -> HashSet<String> { ACTIVE_GENERATORS_GLOBAL.lock().clone().unwrap_or_default() }

/// Return every App that opted into `generator` ("openapi", "sqlite", â€¦).
///
/// Generators are App-scoped in FORML 2 (`App 'X' uses Generator 'Y'.`).
/// The fact may reach the compile via two paths:
///   1. The parser emits a GeneralInstanceFact with `subject_noun="App"`
///      and `object_noun="Generator"`. This is the authoritative path
///      when the dual-quoted instance fact parses cleanly.
///   2. main.rs extracts opt-ins via regex and pushes `{App, Generator}`
///      facts into the `App_uses_Generator` cell. This is the fallback
///      for readings where path 1 does not yet parse.
///
/// We read both and union the results. Callers receive each App at
/// most once regardless of how the opt-in reached the state.
fn apps_opted_into_generator(
    state: &crate::ast::Object,
    instance_facts: &[crate::types::GeneralInstanceFact],
    generator: &str,
) -> Vec<String> {
    let target = generator.to_lowercase();

    let from_gifs: HashSet<String> = instance_facts.iter()
        .filter(|f| f.subject_noun == "App"
                 && (f.object_noun == "Generator" || f.field_name == "Generator")
                 && f.object_value.to_lowercase() == target)
        .map(|f| f.subject_value.clone())
        .collect();

    let from_cell: HashSet<String> = crate::ast::fetch_or_phi("App_uses_Generator", state)
        .as_seq()
        .map(|facts| facts.iter()
            .filter_map(|fact| {
                let app = crate::ast::binding(fact, "App")?;
                let gen = crate::ast::binding(fact, "Generator")?;
                (gen.to_lowercase() == target).then(|| app.to_string())
            })
            .collect())
        .unwrap_or_default();

    from_gifs.into_iter().chain(from_cell).collect::<HashSet<_>>()
        .into_iter().collect()
}

/// #214: compile as a Func tree entry point.
///
/// Returns a Func tree shaped per AREST Theorem 2 / paper §4.1
/// Table 1: compile is the injective map Φ → O that decomposes by
/// FORML 2 construct family. The top level is
///
///   compile_func = Concat ∘ Construction([ f_constraint, f_validate,
///     f_machine, f_transitions, f_derivation, f_schema, f_resolve,
///     f_other ])
///
/// Each family `f_X` is a Func::Native leaf that runs
/// `compile_to_defs_state` against the state, filters the resulting
/// `(name, Func)` pairs to its family prefix, and encodes each pair
/// as `Object::Seq<name_atom, func_object>` via
/// `ast::func_to_object`. `Concat` flattens the eight sub-sequences
/// into the full compile output that `decode_compile_result` can
/// reverse.
///
/// Performance: ρ-dispatch via `apply(compile_func(), …)` runs
/// `compile_to_defs_state` once per family (8×) because each family
/// leaf is independent. The hot path — direct Rust call to
/// `compile_to_defs_state` — is unchanged and is the one used by
/// `platform_compile`, `compile_to_defs_state`, and every in-process
/// caller. This entry point exists so external dispatch layers
/// (future lowered compile pipelines, MCP dispatch, FPGA backends)
/// can ρ-dispatch to compile and see the construct-family structure
/// at the Func-tree level instead of reaching into a monolithic
/// Rust procedure.
pub fn compile_func() -> crate::ast::Func {
    use crate::ast::Func;
    Func::compose(
        Func::Concat,
        Func::construction(vec![
            family_leaf(FAMILY_CONSTRAINT),
            family_leaf(FAMILY_VALIDATE),
            family_leaf(FAMILY_MACHINE),
            family_leaf(FAMILY_TRANSITIONS),
            family_leaf(FAMILY_DERIVATION),
            schema_family_func(),
            family_leaf(FAMILY_RESOLVE),
            family_leaf(FAMILY_SHARD),
            family_leaf(FAMILY_LIST),
            family_leaf(FAMILY_GET),
            family_leaf(FAMILY_OTHER),
        ]),
    )
}

/// Schema family as a pure FFP structure (#245 deeper lowering):
///
///     schema_family = ApplyToAll(schema_pair_from_ft_native)
///                   ∘ FetchOrPhi(<"FactType", state>)
///
/// The iteration over FactType facts is now explicit at the Func
/// level (`ApplyToAll`), matching the paper's α-combinator. The
/// per-fact body (`schema_pair_from_ft_native`) is still a Rust
/// leaf because building a Map-Object from a named-tuple fact's
/// `arity` binding + synthesising a Construction of selectors is
/// terse in Rust and awkward in FFP primitives without a
/// `binding`/`lookup` form. Further lowering is the natural next
/// step when such a primitive exists.
fn schema_family_func() -> crate::ast::Func {
    use crate::ast::Func;
    Func::compose(
        Func::apply_to_all(schema_pair_from_ft_native()),
        Func::compose(
            Func::FetchOrPhi,
            Func::construction(vec![
                Func::constant(crate::ast::Object::atom("FactType")),
                Func::Id,
            ]),
        ),
    )
}

/// Per-FT body for the schema family: named-tuple fact → encoded
/// <name_atom, Construction-func-object> pair. Native wrapper
/// because named-tuple binding access and func-object encoding
/// lean on Rust helpers. The enclosing `ApplyToAll` handles
/// iteration in FFP.
fn schema_pair_from_ft_native() -> crate::ast::Func {
    use alloc::sync::Arc;
    crate::ast::Func::Native(Arc::new(|ft: &crate::ast::Object| {
        match schema_pair_from_ft_fact(ft) {
            Some((name, func)) => crate::ast::Object::seq(vec![
                crate::ast::Object::atom(&name),
                crate::ast::func_to_object(&func),
            ]),
            // Malformed FT (missing id / arity binding) → empty seq
            // so the outer ApplyToAll keeps running and the downstream
            // Concat flattens the nothing contribution to nothing.
            // Bottom would break Concat's contract.
            None => crate::ast::Object::phi(),
        }
    }))
}

// Construct-family tags, matched against def names with
// `name_belongs_to_family`. Order matches AREST.tex §4.1 Table 1:
// constraints first (Filter(p):P), validators, state machines
// (foldl transition), derivations (Compose), fact-type schemas
// (Construction of role selectors), noun resolvers, then everything
// else (indices, generator output, platform bindings).
const FAMILY_CONSTRAINT:  &str = "constraint:";
const FAMILY_VALIDATE:    &str = "validate";
const FAMILY_MACHINE:     &str = "machine:";
const FAMILY_TRANSITIONS: &str = "transitions:";
const FAMILY_DERIVATION:  &str = "derivation";
const FAMILY_SCHEMA:      &str = "schema:";
const FAMILY_RESOLVE:     &str = "resolve:";
const FAMILY_SHARD:       &str = "shard:";
const FAMILY_LIST:        &str = "list:";
const FAMILY_GET:         &str = "get:";
const FAMILY_OTHER:       &str = "";

/// True when a def name belongs to the given family tag. `""` is
/// the catch-all used for the last family leaf — it matches every
/// name not claimed by an earlier family, exactly inverting the
/// specific-family predicates to ensure each name falls into
/// exactly one family.
fn name_belongs_to_family(name: &str, family: &str) -> bool {
    match family {
        "" => ![
            FAMILY_CONSTRAINT, FAMILY_VALIDATE, FAMILY_MACHINE,
            FAMILY_TRANSITIONS, FAMILY_DERIVATION, FAMILY_SCHEMA,
            FAMILY_RESOLVE, FAMILY_SHARD, FAMILY_LIST, FAMILY_GET,
        ].iter().any(|f| name_belongs_to_family(name, f)),
        tag if tag.ends_with(':') => name.starts_with(tag),
        tag => name == tag || name.starts_with(&alloc::format!("{}:", tag)),
    }
}

fn family_leaf(family: &'static str) -> crate::ast::Func {
    use alloc::sync::Arc;
    crate::ast::Func::Native(Arc::new(move |state: &crate::ast::Object| {
        // Families with standalone compilers read only the cells they
        // need — no fallback to the monolithic `compile_to_defs_state`,
        // no 8× compile amplification per apply. Other families still
        // call the full compile and filter by prefix as an interim
        // step until they're each extracted.
        let defs = match family {
            // FAMILY_SCHEMA is now a dedicated Func tree
            // (`schema_family_func`) inserted directly into
            // compile_func's Construction, not routed through here.
            FAMILY_RESOLVE => compile_resolve_family(state),
            FAMILY_SHARD => compile_shard_family(state),
            FAMILY_LIST => compile_per_noun_platform_family(state, "list", "list_noun"),
            FAMILY_GET => compile_per_noun_platform_family(state, "get", "get_noun"),
            _ => compile_to_defs_state(state)
                .into_iter()
                .filter(|(name, _)| name_belongs_to_family(name, family))
                .collect(),
        };
        let pairs: Vec<crate::ast::Object> = defs.iter()
            .map(|(name, func)| {
                crate::ast::Object::seq(vec![
                    crate::ast::Object::atom(name),
                    crate::ast::func_to_object(func),
                ])
            })
            .collect();
        crate::ast::Object::seq(pairs)
    }))
}

/// Standalone per-noun Platform-primitive family compiler (#214):
/// for each declared noun, emits `({prefix}:{noun}, Func::Platform("{platform_key}:{noun}"))`.
/// Used by the MCP read-path families `list:` and `get:`. Each
/// per-noun def is a Platform primitive so the runtime can read the
/// live D at apply-time (whitepaper Eq 9): the read path is a
/// ρ-application that fetches from the population as it exists when
/// the tool is called, so entities added via apply/create become
/// visible immediately without a recompile.
fn compile_per_noun_platform_family(
    state: &crate::ast::Object,
    prefix: &str,
    platform_key: &str,
) -> Vec<(String, Func)> {
    fetch_or_phi("Noun", state).as_seq()
        .map(|ns| ns.iter()
            .filter_map(|n| binding(n, "name").map(|s| s.to_string()))
            .map(|noun_name| (
                alloc::format!("{}:{}", prefix, noun_name),
                Func::Platform(alloc::format!("{}:{}", platform_key, noun_name)),
            ))
            .collect())
        .unwrap_or_default()
}

/// Standalone shard-family compiler (#214): calls RMAP's cell-map
/// helper and emits one `(shard:{ft_id}, Constant(cell_name))` per
/// fact type. Matches paper Eq. demux (§8):
///
///     E_n = Filter(eq ∘ [RMAP, n̄]) : E
///
/// The constant Func is the per-cell identity the demux filter
/// compares against. No pass through `compile_to_defs_state`.
fn compile_shard_family(state: &crate::ast::Object) -> Vec<(String, Func)> {
    crate::rmap::rmap_cell_map_from_state(state).iter()
        .map(|(ft_id, cell)| (
            alloc::format!("shard:{}", ft_id),
            Func::constant(Object::atom(cell)),
        ))
        .collect()
}

/// Standalone resolve-family compiler (#214): for each declared
/// noun, find the binary FTs it participates in and emit a
/// `resolve:{noun}` Func — a right-fold of `Func::Condition` guards
/// mapping each `(other_role_noun_lowercase)` field name to the
/// owning fact-type id. Input to the compiled Func is an atom
/// (the field name queried); output is the fact-type atom.
///
/// Bodies with no binary FT participation emit no def. Matches the
/// existing `c_nouns.keys().filter_map(…)` branch in
/// `compile_to_defs_state`, without its pass through the full
/// compile pipeline.
fn compile_resolve_family(state: &crate::ast::Object) -> Vec<(String, Func)> {
    let noun_cell = fetch_or_phi("Noun", state);
    let ft_cell = fetch_or_phi("FactType", state);
    let role_cell = fetch_or_phi("Role", state);

    let noun_names: Vec<String> = noun_cell.as_seq()
        .map(|ns| ns.iter()
            .filter_map(|n| binding(n, "name").map(|s| s.to_string()))
            .collect())
        .unwrap_or_default();
    let ft_facts: &[crate::ast::Object] = ft_cell.as_seq().unwrap_or(&[]);
    let role_facts: &[crate::ast::Object] = role_cell.as_seq().unwrap_or(&[]);

    // Pre-compute (ft_id, roles) for every fact type we see so the
    // per-noun loop below is O(#FT × #roles-per-FT), not O(#FT²).
    let fact_types: Vec<(String, Vec<String>)> = ft_facts.iter()
        .filter_map(|ft| {
            let id = binding(ft, "id")?.to_string();
            let mut roles: Vec<(usize, String)> = role_facts.iter()
                .filter(|r| binding(r, "factType") == Some(&id))
                .filter_map(|r| Some((
                    binding(r, "position")?.parse().ok()?,
                    binding(r, "nounName")?.to_string(),
                )))
                .collect();
            roles.sort_by_key(|(p, _)| *p);
            Some((id, roles.into_iter().map(|(_, n)| n).collect()))
        })
        .collect();

    noun_names.iter().filter_map(|noun_name| {
        let field_mappings: Vec<(String, String)> = fact_types.iter()
            .filter(|(_, role_nouns)| role_nouns.iter().any(|n| n == noun_name))
            .filter_map(|(ft_id, role_nouns)| {
                (role_nouns.len() == 2).then(|| ())?;
                let other = role_nouns.iter().find(|n| n.as_str() != noun_name.as_str())?.clone();
                Some((other.to_lowercase(), ft_id.clone()))
            })
            .collect();
        (!field_mappings.is_empty()).then(|| {
            let resolve_func = field_mappings.iter().rev().fold(Func::Id, |inner, (field, ft_id)| {
                Func::condition(
                    Func::compose(Func::Eq, Func::construction(vec![
                        Func::Id,
                        Func::constant(Object::atom(field)),
                    ])),
                    Func::constant(Object::atom(ft_id)),
                    inner,
                )
            });
            (alloc::format!("resolve:{}", noun_name), resolve_func)
        })
    }).collect()
}

/// Standalone schema-family compiler (#214): reads the FactType
/// cell directly and emits one `(schema:{ft_id}, Construction([
/// Selector(1), ..., Selector(n)]))` pair per declared fact type.
///
/// Matches AREST §4.1 Table 1 verbatim: "Fact type X r Y —
/// `<CONS, s₁, s₂>` (binary)". The parser records each FT's role
/// count in the FactType cell's `arity` binding, so the compiler
/// Per-FT compilation step: one `(schema:{id}, Construction)` pair.
/// Pure per-fact function — amenable to being lowered to an FFP
/// `Func::ApplyToAll` leaf in a later pass (the iteration itself
/// would then become explicit at the Func level).
fn schema_pair_from_ft_fact(ft: &crate::ast::Object) -> Option<(String, Func)> {
    let ft_id = binding(ft, "id")?.to_string();
    // `arity` is recorded at parse time; selectors are 1-indexed.
    let arity: usize = binding(ft, "arity")?.parse().ok()?;
    let selectors: Vec<Func> = (1..=arity).map(Func::Selector).collect();
    Some((alloc::format!("schema:{}", ft_id), Func::Construction(selectors)))
}

/// Decode the output of `apply(compile_func(), state, state)` back
/// into the `(name, Func)` list the Rust entry point produces. Uses
/// `ast::metacompose` to reverse `func_to_object` at each pair.
pub fn decode_compile_result(obj: &crate::ast::Object, d: &crate::ast::Object) -> Vec<(String, crate::ast::Func)> {
    obj.as_seq().map(|pairs| pairs.iter().filter_map(|p| {
        let items = p.as_seq()?;
        if items.len() != 2 { return None; }
        let name = items[0].as_atom()?.to_string();
        let func = crate::ast::metacompose(&items[1], d);
        Some((name, func))
    }).collect()).unwrap_or_default()
}

pub fn compile_to_defs_state(state: &crate::ast::Object) -> Vec<(String, Func)> {
    let t = profile_timer::now();
    let model = compile(state);
    diag!("[profile] compile: {:?}", t.elapsed());

    // Domain eliminated (#211): all generators below read from cell-based
    // collections (c_nouns, c_fact_types, c_constraints, etc.) or call
    // state-based shims (rmap_from_state, rmap_cell_map_from_state).

    // â”€â”€ Cell-based lookups (read from state, not domain) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    let noun_cell = fetch_or_phi("Noun", state);
    let ft_cell = fetch_or_phi("FactType", state);
    let role_cell = fetch_or_phi("Role", state);
    let constraint_cell = fetch_or_phi("Constraint", state);
    let rule_cell = fetch_or_phi("DerivationRule", state);
    let inst_cell = fetch_or_phi("InstanceFact", state);

    // Build typed local collections from cells.
    // c_nouns: HashMap<String, NounDef> â€” noun name â†’ definition
    let c_nouns: HashMap<String, NounDef> = noun_cell.as_seq()
        .map(|facts| facts.iter().filter_map(|f| {
            let name = binding(f, "name")?.to_string();
            let obj_type = binding(f, "objectType").unwrap_or("entity").to_string();
            Some((name, NounDef { object_type: obj_type, world_assumption: WorldAssumption::default() }))
        }).collect())
        .unwrap_or_default();

    // c_ref_schemes: HashMap<String, Vec<String>>
    let c_ref_schemes: HashMap<String, Vec<String>> = noun_cell.as_seq()
        .map(|facts| facts.iter().filter_map(|f| {
            let name = binding(f, "name")?.to_string();
            let v = binding(f, "referenceScheme")?;
            Some((name, v.split(',').map(|s| s.to_string()).collect()))
        }).collect())
        .unwrap_or_default();

    // c_fact_types: HashMap<String, FactTypeDef>
    let c_fact_types: HashMap<String, FactTypeDef> = ft_cell.as_seq()
        .map(|facts| facts.iter().filter_map(|f| {
            let id = binding(f, "id")?.to_string();
            let reading = binding(f, "reading").unwrap_or("").to_string();
            let roles: Vec<RoleDef> = role_cell.as_seq()
                .map(|rs| rs.iter()
                    .filter(|r| binding(r, "factType") == Some(&id))
                    .map(|r| RoleDef {
                        noun_name: binding(r, "nounName").unwrap_or("").to_string(),
                        role_index: binding(r, "position").and_then(|v| v.parse().ok()).unwrap_or(0),
                    }).collect())
                .unwrap_or_default();
            Some((id, FactTypeDef { schema_id: String::new(), reading, readings: vec![], roles }))
        }).collect())
        .unwrap_or_default();

    // c_constraints: Vec<ConstraintDef>
    let c_constraints: Vec<ConstraintDef> = constraint_cell.as_seq()
        .map(|facts| facts.iter().map(|f| {
            let get = |key: &str| binding(f, key).map(|s| s.to_string());
            let spans = (0..4).filter_map(|i| {
                let ft_id = get(&alloc::format!("span{}_factTypeId", i))?;
                let ri = get(&alloc::format!("span{}_roleIndex", i))?;
                Some(SpanDef { fact_type_id: ft_id, role_index: ri.parse().unwrap_or(0), subset_autofill: None })
            }).collect();
            ConstraintDef {
                id: get("id").unwrap_or_default(), kind: get("kind").unwrap_or_default(),
                modality: get("modality").unwrap_or_default(), deontic_operator: get("deonticOperator"),
                text: get("text").unwrap_or_default(), spans,
                set_comparison_argument_length: None, clauses: None, entity: get("entity"),
                min_occurrence: None, max_occurrence: None,
            }
        }).collect())
        .unwrap_or_default();

    // c_derivation_rules: Vec<DerivationRuleDef>
    let c_derivation_rules: Vec<DerivationRuleDef> = rule_cell.as_seq()
        .map(|facts| facts.iter().map(|f| {
            let get = |key: &str| binding(f, key).unwrap_or("").to_string();
            DerivationRuleDef {
                id: get("id"), text: get("text"), antecedent_fact_type_ids: vec![],
                consequent_fact_type_id: get("consequentFactTypeId"),
                kind: DerivationKind::ModusPonens, join_on: vec![], match_on: vec![], consequent_bindings: vec![], antecedent_filters: vec![], consequent_computed_bindings: vec![], consequent_aggregates: vec![], unresolved_clauses: vec![], antecedent_role_literals: vec![], consequent_role_literals: vec![],
            }
        }).collect())
        .unwrap_or_default();

    // c_instance_facts: Vec<GeneralInstanceFact>
    let c_instance_facts: Vec<GeneralInstanceFact> = inst_cell.as_seq()
        .map(|facts| facts.iter().map(|f| {
            let get = |key: &str| binding(f, key).unwrap_or("").to_string();
            GeneralInstanceFact {
                subject_noun: get("subjectNoun"), subject_value: get("subjectValue"),
                field_name: get("fieldName"), object_noun: get("objectNoun"), object_value: get("objectValue"),
            }
        }).collect())
        .unwrap_or_default();

    // Generator opt-in (needed early for validate partitioning).
    let generators = {
        let active = active_generators();
        if !active.is_empty() { active } else {
            c_instance_facts.iter()
                .filter(|f| f.object_noun == "Generator" || f.field_name == "Generator")
                .map(|f| f.object_value.to_lowercase())
                .collect()
        }
    };
    diag!("  [profile] generators opted in: {:?}", generators);

    // Constraints -> named definitions â€” Î±(constraint â†’ def)
    let mut defs: Vec<(String, Func)> = model.constraints.iter()
        .map(|c| (format!("constraint:{}", c.id), c.func.clone()))
        .collect();

    // validate: Concat . [non-DDL constraints] -- only constraints the DB can't enforce.
    // When a SQL generator is active, UC/MC/VC are enforced by DDL (UNIQUE, NOT NULL, CHECK).
    // Ring, subset, equality, exclusion, deontic, and frequency stay in validate.
    let sql_active = generators.iter().any(|g| ["sqlite", "postgresql", "mysql", "sqlserver", "oracle", "db2", "clickhouse"].contains(&g.as_str()));
    let ddl_kinds: HashSet<&str> = ["UC", "MC", "VC"].into_iter().collect();
    let app_constraint_ids: HashSet<String> = c_constraints.iter()
        .filter(|c| !(sql_active && ddl_kinds.contains(c.kind.as_str())))
        .map(|c| c.id.clone())
        .collect();
    let app_constraints: Vec<Func> = model.constraints.iter()
        .filter(|c| app_constraint_ids.contains(&c.id))
        .map(|c| c.func.clone())
        .collect();
    diag!("  [profile] validate: {} of {} constraints (SQL handles {})",
        app_constraints.len(), model.constraints.len(), model.constraints.len() - app_constraints.len());
    defs.push(("validate".to_string(), Func::compose(Func::Concat, Func::construction(app_constraints))));

    // Indexed validate: validate:{fact_type_id} runs only constraints spanning that FT.
    // Used by platform_compile to validate only what changed.
    let ft_to_app_constraints: HashMap<String, Vec<Func>> = c_constraints.iter()
        .filter(|c| app_constraint_ids.contains(&c.id))
        .flat_map(|c| {
            let compiled = model.constraints.iter().find(|cc| cc.id == c.id);
            c.spans.iter().filter_map(move |span| {
                compiled.map(|cc| (span.fact_type_id.clone(), cc.func.clone()))
            })
        })
        .fold(HashMap::new(), |mut m, (ft_id, func)| {
            m.entry(ft_id).or_default().push(func);
            m
        });
    defs.extend(ft_to_app_constraints.into_iter().map(|(ft_id, funcs)| {
        (format!("validate:{}", ft_id), Func::compose(Func::Concat, Func::construction(funcs)))
    }));

    // Per-noun aggregate validate â€” concat only the per-FT validates for
    // fact types the noun participates in. Lets create/update/transition
    // handlers pay O(FTs-touching-noun) instead of O(all constraints).
    // For the metamodel that's ~5â€“10 FTs per noun vs 345 bulk constraints.
    //
    // Key is `validate:{noun}`. Collision with `validate:{ft_id}` is
    // avoided because FT ids are reading-derived snake_case strings
    // (e.g. `Order_was_placed_by_Customer`) while noun names are single
    // terms (possibly with spaces). Fallback path in callers still
    // resolves to the bulk `validate` def when the per-noun key is
    // absent â€” safe under any future compile that skips this step.
    let noun_to_fts: HashMap<String, HashSet<String>> = c_fact_types.iter()
        .flat_map(|(ft_id, ft)| ft.roles.iter()
            .map(|r| (r.noun_name.clone(), ft_id.clone()))
            .collect::<Vec<_>>())
        .fold(HashMap::new(), |mut m, (n, ft)| {
            m.entry(n).or_default().insert(ft);
            m
        });
    defs.extend(noun_to_fts.into_iter().map(|(noun, fts)| {
        let calls: Vec<Func> = fts.into_iter()
            .map(|ft| Func::Def(format!("validate:{}", ft)))
            .collect();
        let func = match calls.len() {
            0 => Func::constant(Object::phi()),
            1 => calls.into_iter().next().unwrap(),
            _ => Func::compose(Func::Concat, Func::construction(calls)),
        };
        (format!("validate:{}", noun), func)
    }));

    // State machines -> named definitions â€” Î±(sm â†’ <func_def, initial_def>)
    defs.extend(model.state_machines.iter().flat_map(|sm| [
        (format!("machine:{}", sm.noun_name), sm.func.clone()),
        (format!("machine:{}:initial", sm.noun_name), Func::constant(Object::atom(&sm.initial))),
    ]));

    // Transitions: for each SM, register transitions:{noun} that takes a status
    // and returns <<from, to, event>, ...> for available transitions.
    // Uses the machine func and known events to compute available transitions.
    // Transitions + meta â€” Î±(sm â†’ <transitions_def, meta_def>)
    defs.extend(model.state_machines.iter().flat_map(|sm| {
        let machine_def_name = format!("machine:{}", sm.noun_name);
        let events: Vec<String> = sm.transition_table.iter().map(|(_, _, e)| e.clone())
            .collect::<hashbrown::HashSet<_>>().into_iter().collect();
        // Î±(event â†’ check_func): for each event, build condition that tests transition
        let checks: Vec<Func> = events.iter().map(|event| {
            let apply_machine = Func::compose(
                Func::Def(machine_def_name.clone()),
                Func::construction(vec![Func::Id, Func::constant(Object::atom(event))]),
            );
            Func::condition(
                Func::compose(Func::Not, Func::compose(Func::Eq, Func::construction(vec![Func::Id, apply_machine.clone()]))),
                Func::construction(vec![Func::Id, apply_machine, Func::constant(Object::atom(event))]),
                Func::constant(Object::phi()),
            )
        }).collect();
        let transitions_func = Func::compose(
            Func::filter(Func::compose(Func::Not, Func::NullTest)),
            Func::construction(checks),
        );
        [(format!("transitions:{}", sm.noun_name), transitions_func)]
    }));

    // Derivation rules â€” Î±(derivation â†’ def)
    defs.extend(model.derivations.iter()
        .map(|d| (format!("derivation:{}", d.id), d.func.clone())));

    // Derivation index: derivation_index:{noun} â†’ comma-separated derivation IDs.
    // For each derivation rule, collect nouns that play roles in its antecedent/
    // consequent fact types. At runtime, create_via_defs fetches the index for the
    // created noun to gate which derivations run (O(relevant) instead of O(all)).
    {
        let mut noun_to_derivations: HashMap<String, Vec<String>> = HashMap::new();
        // For each compiled derivation, determine which nouns are involved.
        // Strategy: check the derivation ID and all fact types in the domain
        // that the derivation references (via antecedent/consequent for domain rules,
        // or via ID pattern for synthetic rules).
        for compiled in &model.derivations {
            let did = &compiled.id;
            let mut nouns: HashSet<String> = HashSet::new();
            // Try to find a matching domain rule (user-defined derivations)
            let domain_rule = c_derivation_rules.iter().find(|r| r.id == *did);
            if let Some(rule) = domain_rule {
                for ft_id in rule.antecedent_fact_type_ids.iter()
                    .chain(core::iter::once(&rule.consequent_fact_type_id))
                    .filter(|s| !s.is_empty())
                {
                    if let Some(ft) = c_fact_types.get(ft_id.as_str()) {
                        for role in &ft.roles { nouns.insert(role.noun_name.clone()); }
                    }
                }
            }
            // For rules without a matching domain rule (or empty-id rules),
            // also check domain rules that have matching antecedents.
            if nouns.is_empty() {
                // Match domain rules with empty id by antecedent overlap
                for rule in &c_derivation_rules {
                    if rule.id.is_empty() || rule.id == *did {
                        for ft_id in &rule.antecedent_fact_type_ids {
                            if let Some(ft) = c_fact_types.get(ft_id.as_str()) {
                                for role in &ft.roles { nouns.insert(role.noun_name.clone()); }
                            }
                        }
                    }
                }
            }
            // Synthetic rules: extract noun from ID pattern
            if nouns.is_empty() {
                // _cwa_negation_X, _sm_init_Order, _subtype_A_B, _transitivity_...
                for noun_name in c_nouns.keys() {
                    if did.contains(noun_name) { nouns.insert(noun_name.clone()); }
                }
            }
            for noun in nouns {
                let entry = noun_to_derivations.entry(noun).or_default();
                if !entry.contains(did) { entry.push(did.clone()); }
            }
        }
        let index_count: usize = noun_to_derivations.values().map(|v| v.len()).sum();
        diag!("  [profile] derivation index: {} nouns, {} entries", noun_to_derivations.len(), index_count);
        defs.extend(noun_to_derivations.into_iter().map(|(noun, ids)| {
            (format!("derivation_index:{}", noun), Func::constant(Object::atom(&ids.join(","))))
        }));
    }

    // Fact type schemas â€” Î±(schema â†’ def)
    defs.extend(model.schemas.iter()
        .map(|(id, schema)| (format!("schema:{}", id), schema.construction.clone())));

    // Cell sharding: shard:{fact_type_id} â†’ cell_owner (paper Eq. demux).
    // RMAP determines which entity cell owns each fact type.
    // Enables: E_n = Filter(eq âˆ˜ [RMAP, nÌ„]) : E for per-cell event demux.
    let shard_map = crate::rmap::rmap_cell_map_from_state(state);
    diag!("  [profile] shard map: {} fact types partitioned", shard_map.len());
    defs.extend(shard_map.iter().map(|(ft_id, cell)| {
        (format!("shard:{}", ft_id), Func::constant(Object::atom(cell)))
    }));

    // resolve:{noun} â€” Condition chain mapping field_name â†’ fact_type_id.
    // Input: field_name atom. Output: fact_type_id atom.
    // Compiled from NounIndex: for each fact type involving this noun,
    // extract the "other" role's noun name as the field key.
    // resolve:{noun} â€” Î±(noun â†’ Condition chain mapping field_name â†’ fact_type_id)
    defs.extend(c_nouns.keys().filter_map(|noun_name| {
        let field_mappings: Vec<(String, String)> = c_fact_types.iter()
            .filter(|(_, ft)| ft.roles.iter().any(|r| r.noun_name == *noun_name))
            .filter_map(|(ft_id, ft)| {
                (ft.roles.len() == 2).then(|| ())?;
                let other = ft.roles.iter().find(|r| r.noun_name != *noun_name)?.noun_name.clone();
                Some((other.to_lowercase(), ft_id.clone()))
            })
            .collect();
        (!field_mappings.is_empty()).then(|| {
            let resolve_func = field_mappings.iter().rev().fold(Func::Id, |inner, (field, ft_id)| {
                Func::condition(
                    Func::compose(Func::Eq, Func::construction(vec![Func::Id, Func::constant(Object::atom(field))])),
                    Func::constant(Object::atom(ft_id)),
                    inner,
                )
            });
            (format!("resolve:{}", noun_name), resolve_func)
        })
    }));

    // list:{noun} / get:{noun} â€” MCP-facing read paths, dispatched as
    // Platform funcs so they read the live D at apply-time. This preserves
    // whitepaper Eq 9 (SYSTEM:x = (Ï(â†‘entity(x):D)):â†‘op(x)): the read
    // path is a Ï-application that fetches from the population as it
    // exists when the tool is called, so entities added via apply/create
    // become visible immediately without a recompile.
    for (noun_name, _) in &c_nouns {
        defs.push((
            format!("list:{}", noun_name),
            Func::Platform(format!("list_noun:{}", noun_name)),
        ));
        defs.push((
            format!("get:{}", noun_name),
            Func::Platform(format!("get_noun:{}", noun_name)),
        ));
    }

    // HATEOAS navigation links as FFP projections (Theorem 4b).
    // For each binary fact type with a UC, the UC role is the child (dependent),
    // the other role is the parent. Navigation is a constant function returning
    // the related noun names.
    // HATEOAS nav links â€” fold UC constraints into (children_map, parent_map), then Î± â†’ defs
    let (children_map, parent_map) = c_constraints.iter()
        .filter(|c| c.kind == "UC" && !c.spans.is_empty())
        .filter_map(|c| {
            let ft = c_fact_types.get(&c.spans[0].fact_type_id)?;
            (ft.roles.len() == 2).then(|| ())?;
            let idx = c.spans[0].role_index;
            Some((ft.roles[1 - idx].noun_name.clone(), ft.roles[idx].noun_name.clone()))
        })
        .fold(
            (HashMap::<String, Vec<String>>::new(), HashMap::<String, Vec<String>>::new()),
            |(mut cm, mut pm), (parent, child)| {
                cm.entry(parent.clone()).or_default().push(child.clone());
                pm.entry(child).or_default().push(parent);
                (cm, pm)
            },
        );
    defs.extend(children_map.iter().map(|(noun, children)|
        (format!("nav:{}:children", noun), Func::constant(Object::Seq(children.iter().map(|c| Object::atom(c)).collect())))
    ));
    defs.extend(parent_map.iter().map(|(noun, parents)|
        (format!("nav:{}:parent", noun), Func::constant(Object::Seq(parents.iter().map(|p| Object::atom(p)).collect())))
    ));

    // â”€â”€ Generator opt-in (resolved above for validate partitioning) â”€â”€

    // â”€â”€ Generator 1: Agent Prompts (opt-in: not gated, always useful) â”€â”€
    // Build lookup maps via fold â€” noun â†’ readings, noun â†’ constraints, noun â†’ events
    let noun_fact_types: HashMap<String, Vec<String>> = c_fact_types.values()
        .flat_map(|ft| ft.roles.iter().map(move |r| (r.noun_name.clone(), ft.reading.clone())))
        .fold(HashMap::new(), |mut m, (noun, reading)| { m.entry(noun).or_default().push(reading); m });

    let ft_ref = &c_fact_types;
    let noun_constraint_map: HashMap<String, Vec<&ConstraintDef>> = c_constraints.iter()
        .flat_map(|c| c.spans.iter().filter_map(move |s| {
            ft_ref.get(&s.fact_type_id).map(|ft| (ft, c))
        }))
        .flat_map(|(ft, c)| ft.roles.iter().map(move |r| (r.noun_name.clone(), c)))
        .fold(HashMap::new(), |mut m, (noun, c)| { m.entry(noun).or_default().push(c); m });

    let noun_transitions: HashMap<String, Vec<String>> = model.state_machines.iter()
        .map(|sm| (sm.noun_name.clone(), sm.transition_table.iter()
            .map(|(_, _, e)| e.clone()).collect::<HashSet<_>>().into_iter().collect()))
        .collect();

    // Î±(noun â†’ agent_def) â€” filter nouns with readings, map to prompt Object
    let deontic_filter = |cs: &[&ConstraintDef], op: &str| -> Vec<Object> {
        cs.iter().filter(|c| c.modality == "deontic" && c.deontic_operator.as_deref() == Some(op))
            .map(|c| Object::atom(&c.text)).collect()
    };
    let atoms_or_empty = |m: &HashMap<String, Vec<String>>, key: &str| -> Vec<Object> {
        m.get(key).map(|v| v.iter().map(|s| Object::atom(s)).collect()).unwrap_or_default()
    };

    defs.extend(c_nouns.keys()
        .filter(|n| noun_fact_types.get(*n).map_or(false, |r| !r.is_empty()))
        .map(|noun_name| {
            let cs = noun_constraint_map.get(noun_name).map(|v| v.as_slice()).unwrap_or(&[]);
            let prompt = Object::seq(vec![
                Object::seq(vec![Object::atom("role"), Object::atom(noun_name)]),
                Object::seq(vec![Object::atom("fact_types"), Object::Seq(atoms_or_empty(&noun_fact_types, noun_name).into())]),
                Object::seq(vec![Object::atom("constraints"), Object::Seq(cs.iter().map(|c| Object::atom(&c.text)).collect::<Vec<_>>().into())]),
                Object::seq(vec![Object::atom("transitions"), Object::Seq(atoms_or_empty(&noun_transitions, noun_name).into())]),
                Object::seq(vec![Object::atom("children"), Object::Seq(
                    children_map.get(noun_name).map(|v| v.iter().map(|c| Object::atom(c)).collect()).unwrap_or_default())]),
                Object::seq(vec![Object::atom("parent"), Object::Seq(
                    parent_map.get(noun_name).map(|v| v.iter().map(|p| Object::atom(p)).collect()).unwrap_or_default())]),
                Object::seq(vec![Object::atom("deontic"), Object::seq(vec![
                    Object::seq(vec![Object::atom("obligatory"), Object::Seq(deontic_filter(cs, "obligatory").into())]),
                    Object::seq(vec![Object::atom("forbidden"), Object::Seq(deontic_filter(cs, "forbidden").into())]),
                    Object::seq(vec![Object::atom("permitted"), Object::Seq(deontic_filter(cs, "permitted").into())]),
                ])]),
            ]);
            (format!("agent:{}", noun_name), Func::constant(prompt))
        }));

    // Shared helper: constraints spanning a noun (fn, not closure, to avoid move conflicts)
    fn noun_constraints_for<'a>(constraints: &'a [ConstraintDef], fact_types: &HashMap<String, FactTypeDef>, noun: &str) -> Vec<&'a ConstraintDef> {
        constraints.iter()
            .filter(|c| c.spans.iter().any(|s| {
                fact_types.get(&s.fact_type_id)
                    .map_or(false, |ft| ft.roles.iter().any(|r| r.noun_name == noun))
            })).collect()
    }

    // â”€â”€ Generator 2: iLayer â€” Î±(noun â†’ ilayer_def)
    if generators.contains("ilayer") {
    defs.extend(c_nouns.iter().map(|(noun_name, noun_def)| {
        let ft_entries = Object::Seq(c_fact_types.values()
            .filter(|ft| ft.roles.iter().any(|r| r.noun_name == *noun_name))
            .map(|ft| Object::seq(vec![
                Object::atom(&ft.reading),
                Object::Seq(ft.roles.iter().map(|r| Object::atom(&r.noun_name)).collect()),
            ])).collect());
        let constraint_texts = Object::Seq(noun_constraints_for(&c_constraints, &c_fact_types, noun_name).iter()
            .map(|c| Object::atom(&c.text)).collect());
        let ref_parts = Object::Seq(c_ref_schemes.get(noun_name)
            .map(|parts| parts.iter().map(|p| Object::atom(p)).collect()).unwrap_or_default());
        let ilayer = Object::seq(vec![
            Object::seq(vec![Object::atom("object_type"), Object::atom(&noun_def.object_type)]),
            Object::seq(vec![Object::atom("fact_types"), ft_entries]),
            Object::seq(vec![Object::atom("constraints"), constraint_texts]),
            Object::seq(vec![Object::atom("ref_scheme"), ref_parts]),
        ]);
        (format!("ilayer:{}", noun_name), Func::constant(ilayer))
    }));
    } // end ilayer gate

    // â”€â”€ Generator 3: SQL DDL (multi-dialect) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    // Call rmap() at compile time and produce dialect-specific defs:
    //   sql:sqlite:{table}, sql:postgresql:{table}, sql:mysql:{table},
    //   sql:sqlserver:{table}, sql:oracle:{table}, sql:db2:{table},
    //   sql:standard:{table}, sql:clickhouse:{table}
    // â”€â”€ Generator 3: SQL DDL â€” only for opted-in dialects
    let all_dialects = [
        ("sqlite", SqlDialect::Sqlite), ("postgresql", SqlDialect::PostgreSql),
        ("mysql", SqlDialect::MySql), ("sqlserver", SqlDialect::SqlServer),
        ("oracle", SqlDialect::Oracle), ("db2", SqlDialect::Db2),
        ("standard", SqlDialect::Standard), ("clickhouse", SqlDialect::ClickHouse),
    ];
    let active_dialects: Vec<_> = all_dialects.iter()
        .filter(|(name, _)| generators.contains(*name))
        .collect();
    if !active_dialects.is_empty() {
        let sql_tables = crate::rmap::rmap_from_state(state);
        defs.extend(sql_tables.iter().flat_map(|table|
            active_dialects.iter().map(move |(name, dialect)|
                (format!("sql:{}:{}", name, table.name), Func::constant(Object::atom(&generate_ddl(table, dialect))))
            )
        ));
    }

    // â”€â”€ Generator 3b: SQL Triggers for derivation rules â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    if !active_dialects.is_empty() {
        let sql_tables = crate::rmap::rmap_from_state(state);
        let table_names: hashbrown::HashSet<String> = sql_tables.iter()
            .map(|t| t.name.clone()).collect();
        let triggers = generate_derivation_triggers(&c_derivation_rules, &c_fact_types, &sql_tables, &table_names);
        defs.extend(triggers.into_iter().map(|(name, ddl)| {
            (format!("sql:trigger:{}", name), Func::constant(Object::atom(&ddl)))
        }));
    }

    // â”€â”€ Generator 4: Test Harness â€” Î±(constraint â†’ test_def)
    if generators.contains("test") {
    defs.extend(c_constraints.iter().map(|c| {
        let modality_str = match c.modality.as_str() { "deontic" => "deontic", _ => "alethic" };
        (format!("test:{}", c.id), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("id"), Object::atom(&c.id)]),
            Object::seq(vec![Object::atom("text"), Object::atom(&c.text)]),
            Object::seq(vec![Object::atom("kind"), Object::atom(&c.kind)]),
            Object::seq(vec![Object::atom("modality"), Object::atom(modality_str)]),
        ])))
    }));
    } // end test gate

    // â”€â”€ Generator 10: OpenAPI 3.1 â€” one document per App â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    // Generators are App-scoped (`App 'X' uses Generator 'openapi'.`):
    // a single compile may contain several Apps, each with its own
    // opt-in decision. Emit one cell per App that opted in, keyed
    // `openapi:{snake(app-slug)}`. The per-App cell contains the full
    // OpenAPI 3.1 document with the App's identity in `info`.
    //
    // See crate::generators::openapi for the schema/path derivation.
    apps_opted_into_generator(state, &c_instance_facts, "openapi").iter().for_each(|app| {
        let openapi_doc = crate::generators::openapi::compile_to_openapi(state, app);
        defs.push((
            format!("openapi:{}", crate::rmap::to_snake(app)),
            Func::constant(Object::atom(&openapi_doc.to_string())),
        ));
    });

    // Handler defs â€” Î±(noun â†’ <create_def, update_def>)
    // Platform functions: create:{noun} and update:{noun} take Object fact pairs,
    // not JSON. Per AREST Eq. 6, input is the 3NF row.
    defs.extend(c_nouns.keys().flat_map(|noun_name| {
        [
            (format!("create:{}", noun_name), Func::Platform(format!("create:{}", noun_name))),
            (format!("update:{}", noun_name), Func::Platform(format!("update:{}", noun_name))),
        ]
    }));

    // â”€â”€ Data Federation: populate:{noun} from "Noun is backed by External System" â”€â”€
    // Compile federation config from instance facts. Each backed noun gets a
    // populate:{noun} def containing the URL, path, header, and role mappings.
    // The runtime (MCP server, Cloudflare Worker) reads this def and fetches.
    {
        // Build External System config from instance facts.
        let mut ext_systems: HashMap<String, HashMap<String, String>> = HashMap::new();
        c_instance_facts.iter()
            .filter(|f| f.subject_noun == "External System")
            .for_each(|f| {
                ext_systems.entry(f.subject_value.clone())
                    .or_default()
                    .insert(f.field_name.clone(), f.object_value.clone());
            });

        // Build noun â†’ external system + URI mappings.
        let backed_nouns: Vec<(String, String)> = c_instance_facts.iter()
            .filter(|f| f.field_name.contains("backed") && f.object_noun == "External System")
            .map(|f| (f.subject_value.clone(), f.object_value.clone()))
            .collect();

        let noun_uris: HashMap<String, String> = c_instance_facts.iter()
            .filter(|f| f.subject_noun == "Noun" && f.field_name.contains("URI"))
            .map(|f| (f.subject_value.clone(), f.object_value.clone()))
            .collect();

        defs.extend(backed_nouns.iter().filter_map(|(noun_name, ext_name)| {
            let ext = ext_systems.get(ext_name)?;
            let url = ext.iter().find(|(k, _)| k.contains("URL")).map(|(_, v)| v.as_str()).unwrap_or("");
            let header = ext.iter().find(|(k, _)| k.contains("Header")).map(|(_, v)| v.as_str()).unwrap_or("");
            let prefix = ext.iter().find(|(k, _)| k.contains("Prefix")).map(|(_, v)| v.as_str()).unwrap_or("");
            let uri = noun_uris.get(noun_name).map(|s| s.as_str()).unwrap_or("");

            // Collect role names for JSON â†’ fact mapping.
            let role_names: Vec<String> = c_fact_types.values()
                .filter(|ft| ft.roles.iter().any(|r| r.noun_name == *noun_name))
                .filter(|ft| ft.roles.len() == 2)
                .filter_map(|ft| ft.roles.iter().find(|r| r.noun_name != *noun_name))
                .map(|r| r.noun_name.clone())
                .collect();

            let config = Object::seq(vec![
                Object::seq(vec![Object::atom("system"), Object::atom(ext_name)]),
                Object::seq(vec![Object::atom("url"), Object::atom(url)]),
                Object::seq(vec![Object::atom("uri"), Object::atom(uri)]),
                Object::seq(vec![Object::atom("header"), Object::atom(header)]),
                Object::seq(vec![Object::atom("prefix"), Object::atom(prefix)]),
                Object::seq(vec![Object::atom("noun"), Object::atom(noun_name)]),
                Object::seq(vec![Object::atom("fields"), Object::Seq(
                    role_names.iter().map(|n| Object::atom(n)).collect()
                )]),
            ]);

            Some((format!("populate:{}", noun_name), Func::constant(config)))
        }));
        diag!("  [profile] {} federation defs", backed_nouns.len());
    }

    // Query defs â€” Î±(schema â†’ Platform dispatch). query:{ft_id} reads
    // the fact-type cell from live D and returns matching facts as a
    // JSON array, optionally filtered by role bindings in the operand.
    defs.extend(model.schemas.keys().map(|id| {
        (format!("query:{}", id), Func::Platform(format!("query_ft:{}", id)))
    }));

    // Helpers as fns (not closures) to avoid borrow conflicts
    fn binary_fts_for<'a>(fact_types: &'a HashMap<String, FactTypeDef>, noun: &str) -> Vec<&'a FactTypeDef> {
        fact_types.values()
            .filter(|ft| ft.roles.len() == 2 && ft.roles.iter().any(|r| r.noun_name == noun))
            .collect()
    }
    fn other_role_of(ft: &FactTypeDef, noun: &str) -> String {
        ft.roles.iter().find(|r| r.noun_name != noun)
            .map(|r| r.noun_name.clone()).unwrap_or_default()
    }
    // â”€â”€ Generator 5: XSD â€” Î±(noun â†’ xsd_def) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    defs.extend(c_nouns.iter().map(|(noun_name, noun_def)| {
        let fields = Object::Seq(binary_fts_for(&c_fact_types, noun_name).iter().map(|ft|
            Object::seq(vec![Object::atom(&other_role_of(ft, noun_name)), Object::atom("xs:string")])
        ).collect());
        (format!("xsd:{}", noun_name), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("name"), Object::atom(noun_name)]),
            Object::seq(vec![Object::atom("object_type"), Object::atom(&noun_def.object_type)]),
            Object::seq(vec![Object::atom("elements"), fields]),
        ])))
    }));

    // â”€â”€ Generator 6: DTD â€” Î±(noun â†’ dtd_def) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    defs.extend(c_nouns.iter().map(|(noun_name, _)| {
        let children: Vec<String> = binary_fts_for(&c_fact_types, noun_name).iter()
            .map(|ft| other_role_of(ft, noun_name).to_string()).collect();
        let child_list = children.join(", ");
        let dtd_text = format!("<!ELEMENT {} ({})>\n{}",
            noun_name,
            if child_list.is_empty() { "#PCDATA".to_string() } else { child_list },
            children.iter().map(|c| format!("<!ELEMENT {} (#PCDATA)>", c)).collect::<Vec<_>>().join("\n"),
        );
        (format!("dtd:{}", noun_name), Func::constant(Object::atom(&dtd_text)))
    }));

    // â”€â”€ Generator 7: OWL â€” Î±(noun â†’ owl_def) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    defs.extend(c_nouns.iter().map(|(noun_name, noun_def)| {
        let properties = Object::Seq(binary_fts_for(&c_fact_types, noun_name).iter().map(|ft| {
            let other = other_role_of(ft, noun_name);
            let prop_type = match c_nouns.get(&other).map(|n| n.object_type.as_str()) {
                Some("value") => "DatatypeProperty", _ => "ObjectProperty",
            };
            Object::seq(vec![Object::atom(&other), Object::atom(prop_type), Object::atom(&ft.reading)])
        }).collect());
        (format!("owl:{}", noun_name), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("class"), Object::atom(noun_name)]),
            Object::seq(vec![Object::atom("type"), Object::atom(match noun_def.object_type.as_str() { "value" => "Datatype", _ => "Class" })]),
            Object::seq(vec![Object::atom("properties"), properties]),
        ])))
    }));

    // â”€â”€ Generator 8: WSDL â€” Î±(noun â†’ wsdl_def) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    defs.extend(c_nouns.iter().map(|(noun_name, _)| {
        let has_sm = model.state_machines.iter().any(|sm| sm.noun_name == *noun_name);
        let ops: Vec<Object> = [("create","POST"), ("query","GET"), ("update","PUT")]
            .iter().map(|(op,m)| Object::seq(vec![Object::atom(op), Object::atom(m)]))
            .chain(has_sm.then(|| Object::seq(vec![Object::atom("transition"), Object::atom("POST")])))
            .collect();
        (format!("wsdl:{}", noun_name), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("portType"), Object::atom(noun_name)]),
            Object::seq(vec![Object::atom("operations"), Object::Seq(ops.into())]),
        ])))
    }));

    // â”€â”€ Generator 9: EDM â€” Î±(noun â†’ edm_def) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    defs.extend(c_nouns.iter().map(|(noun_name, noun_def)| {
        let properties = Object::Seq(binary_fts_for(&c_fact_types, noun_name).iter().map(|ft| {
            let other = other_role_of(ft, noun_name);
            let kind = match c_nouns.get(&other).map(|n| n.object_type.as_str()) {
                Some("entity") => "NavigationProperty", _ => "Property",
            };
            Object::seq(vec![Object::atom(&other), Object::atom(kind), Object::atom("Edm.String")])
        }).collect());
        let key = Object::Seq(c_ref_schemes.get(noun_name)
            .map(|parts| parts.iter().map(|p| Object::atom(p)).collect()).unwrap_or_default());
        (format!("edm:{}", noun_name), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("entity_type"), Object::atom(noun_name)]),
            Object::seq(vec![Object::atom("base_type"), Object::atom(&noun_def.object_type)]),
            Object::seq(vec![Object::atom("key"), key]),
            Object::seq(vec![Object::atom("properties"), properties]),
        ])))
    }));

    // ï¿½ï¿½â”€ Generator 10: XForms â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    defs.extend(c_nouns.iter().map(|(noun_name, _)| {
        let bindings = Object::Seq(binary_fts_for(&c_fact_types, noun_name).iter().filter_map(|ft| {
            let other = ft.roles.iter().find(|r| r.noun_name != *noun_name)?;
            let control = match c_nouns.get(&other.noun_name).map(|n| n.object_type.as_str()) {
                Some("value") => "input", _ => "select1",
            };
            Some(Object::seq(vec![Object::atom(&other.noun_name), Object::atom(control), Object::atom(&ft.reading)]))
        }).collect());
        (format!("xforms:{}", noun_name), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("model"), Object::atom(noun_name)]),
            Object::seq(vec![Object::atom("bindings"), bindings]),
        ])))
    }));

    // â”€â”€ Generator 11: HTML Report â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    defs.extend(c_nouns.iter().map(|(noun_name, noun_def)| {
        let fields = Object::Seq(binary_fts_for(&c_fact_types, noun_name).iter().map(|ft|
            Object::seq(vec![Object::atom(&other_role_of(ft, noun_name)), Object::atom(&ft.reading)])
        ).collect());
        let constraints = Object::Seq(noun_constraints_for(&c_constraints, &c_fact_types, noun_name).iter()
            .map(|c| Object::atom(&c.text)).collect());
        (format!("html:{}", noun_name), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("title"), Object::atom(noun_name)]),
            Object::seq(vec![Object::atom("object_type"), Object::atom(&noun_def.object_type)]),
            Object::seq(vec![Object::atom("fields"), fields]),
            Object::seq(vec![Object::atom("constraints"), constraints]),
        ])))
    }));

    // â”€ï¿½ï¿½ Generator 12: NHibernate Mapping â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    let tables = crate::rmap::rmap_from_state(state);
    defs.extend(tables.iter().map(|table| {
        let columns = Object::Seq(table.columns.iter().map(|col| Object::seq(vec![
            Object::atom(&col.name), Object::atom(&col.col_type),
            Object::atom(if col.nullable { "true" } else { "false" }),
            col.references.as_ref().map(|r| Object::atom(r)).unwrap_or(Object::phi()),
        ])).collect());
        let pk = Object::Seq(table.primary_key.iter().map(|k| Object::atom(k)).collect());
        (format!("nhibernate:{}", table.name), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("class"), Object::atom(&table.name)]),
            Object::seq(vec![Object::atom("table"), Object::atom(&table.name)]),
            Object::seq(vec![Object::atom("id"), pk]),
            Object::seq(vec![Object::atom("properties"), columns]),
        ])))
    }));

    // â”€â”€ Generator 13: LINQ â€” Î±(table â†’ linq_def) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    defs.extend(tables.iter().map(|table| {
        let members = Object::Seq(table.columns.iter().map(|col| {
            let db_type = match col.col_type.as_str() {
                "TEXT" => "NVarChar", "INTEGER" => "Int", "REAL" => "Float",
                "BOOLEAN" => "Bit", _ => "NVarChar",
            };
            Object::seq(vec![
                Object::atom(&col.name), Object::atom(db_type),
                Object::atom(if col.nullable { "true" } else { "false" }),
                Object::atom(if table.primary_key.contains(&col.name) { "true" } else { "false" }),
            ])
        }).collect());
        (format!("linq:{}", table.name), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("table"), Object::atom(&table.name)]),
            Object::seq(vec![Object::atom("columns"), members]),
        ])))
    }));

    // â”€â”€ Generator 14: PLiX â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    defs.extend(c_nouns.iter().map(|(noun_name, noun_def)| {
        let fields = Object::Seq(binary_fts_for(&c_fact_types, noun_name).iter().filter_map(|ft| {
            let other = ft.roles.iter().find(|r| r.noun_name != *noun_name)?;
            let clr_type = match c_nouns.get(&other.noun_name).map(|n| n.object_type.as_str()) {
                Some("value") => "System.String", _ => &other.noun_name,
            };
            Some(Object::seq(vec![Object::atom(&other.noun_name), Object::atom(clr_type)]))
        }).collect());
        (format!("plix:{}", noun_name), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("class"), Object::atom(noun_name)]),
            Object::seq(vec![Object::atom("base_type"), Object::atom(&noun_def.object_type)]),
            Object::seq(vec![Object::atom("fields"), fields]),
        ])))
    }));

    // â”€â”€ Generator 15: DSL â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    defs.extend(c_nouns.iter().map(|(noun_name, noun_def)| {
        let readings = Object::Seq(c_fact_types.values()
            .filter(|ft| ft.roles.iter().any(|r| r.noun_name == *noun_name))
            .map(|ft| Object::atom(&ft.reading)).collect());
        let constraints = Object::Seq(noun_constraints_for(&c_constraints, &c_fact_types, noun_name).iter()
            .map(|c| Object::seq(vec![Object::atom(&c.kind), Object::atom(&c.text)])).collect());
        let transitions = Object::Seq(model.state_machines.iter()
            .filter(|sm| sm.noun_name == *noun_name)
            .flat_map(|sm| sm.transition_table.iter().map(|(from, to, event)|
                Object::seq(vec![Object::atom(from), Object::atom(event), Object::atom(to)])
            )).collect());
        (format!("dsl:{}", noun_name), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("noun"), Object::atom(noun_name)]),
            Object::seq(vec![Object::atom("object_type"), Object::atom(&noun_def.object_type)]),
            Object::seq(vec![Object::atom("readings"), readings]),
            Object::seq(vec![Object::atom("constraints"), constraints]),
            Object::seq(vec![Object::atom("transitions"), transitions]),
        ])))
    }));

    // debug: constant projection of the compiled state (security task #18).
    // When `debug-def` feature is ON (default, tests), returns a full projection
    // of nouns, fact types, constraints, state machines â€” leaks internals.
    // When OFF (production release builds), returns a tiny counts-only summary
    // so callers can still sanity-check cardinalities without exposing schema.
    #[cfg(feature = "debug-def")]
    {
        // Emit as a JSON string atom so MCP / HTTP consumers can JSON.parse
        // the response directly. FFP display notation is not JSON-compatible.
        let total_facts = c_fact_types.len() + c_constraints.len() + c_instance_facts.len();
        let json = serde_json::json!({
            "nouns": c_nouns.keys().collect::<Vec<_>>(),
            "factTypes": c_fact_types.iter().map(|(id, ft)| {
                serde_json::json!({ "id": id, "reading": ft.reading })
            }).collect::<Vec<_>>(),
            "constraints": c_constraints.iter().map(|c| {
                serde_json::json!({ "kind": c.kind, "text": c.text, "modality": c.modality })
            }).collect::<Vec<_>>(),
            "stateMachines": model.state_machines.iter().map(|sm| {
                serde_json::json!({
                    "noun": sm.noun_name,
                    "initial": sm.initial,
                    "transitions": sm.transition_table.iter().map(|(from, to, event)| {
                        serde_json::json!({ "from": from, "to": to, "event": event })
                    }).collect::<Vec<_>>(),
                })
            }).collect::<Vec<_>>(),
            "totalFacts": total_facts,
        });
        defs.push(("debug".to_string(), Func::constant(Object::atom(&json.to_string()))));
    }
    #[cfg(not(feature = "debug-def"))]
    {
        // Counts-only summary â€” no names, readings, texts, or transitions leaked.
        let noun_count = c_nouns.len().to_string();
        let ft_count = c_fact_types.len().to_string();
        let c_count = c_constraints.len().to_string();
        let sm_count = model.state_machines.len().to_string();
        defs.push(("debug".to_string(), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("nouns"), Object::atom(&noun_count)]),
            Object::seq(vec![Object::atom("factTypes"), Object::atom(&ft_count)]),
            Object::seq(vec![Object::atom("constraints"), Object::atom(&c_count)]),
            Object::seq(vec![Object::atom("stateMachines"), Object::atom(&sm_count)]),
            Object::seq(vec![Object::atom("disabled"), Object::atom("âŠ¥ debug disabled")]),
        ]))));
    }

    // Algebraic rewrite pass (Backus Â§12). Normalize every emitted Func
    // to its smallest equivalent form before it enters D. Rewrites are
    // observational equivalences, so runtime semantics are unchanged;
    // interpretation is faster because the reducer walks fewer nodes.
    // See crate::ast::normalize for the rule set.
    let t = profile_timer::now();
    let normalized: Vec<(String, Func)> = defs.into_iter()
        .map(|(name, func)| (name, crate::ast::normalize(&func)))
        .collect();
    diag!("[profile] normalize pass: {:?} ({} defs)", t.elapsed(), normalized.len());

    normalized
}

// state_to_domain deleted (#211). All callers now use
// cell_index_from_state or read cells directly.

/// Compile Object state into executable form (CompiledModel).
/// Structural model validation â€” catches FORML2 violations at compile time.
/// Returns a list of error messages. Empty = model is well-formed.
pub fn validate_model_from_state(state: &crate::ast::Object) -> Vec<String> {
    let data = cell_index_from_state(state);
    validate_model_data(&data)
}

pub(crate) fn validate_model_data(ir: &CellIndex) -> Vec<String> {
    let mut errors = Vec::new();

    // 1. Undeclared nouns in fact type roles
    ir.fact_types.iter().for_each(|(ft_id, ft)| {
        ft.roles.iter()
            .filter(|r| !ir.nouns.contains_key(&r.noun_name))
            .for_each(|r| errors.push(format!(
                "Undeclared noun '{}' in fact type '{}'", r.noun_name, ft_id)));
    });

    // 2. Subtype of undeclared supertype
    ir.subtypes.iter()
        .filter(|(_, parent)| !ir.nouns.contains_key(parent.as_str()))
        .for_each(|(child, parent)| errors.push(format!(
            "Subtype '{}' declares supertype '{}' which is not a declared noun", child, parent)));

    // 3. Duplicate noun: same name declared as both entity and value
    // (handled by parser overwrite â€” but we can warn)

    // 4. UC spans fewer than n-1 roles on n-ary (arity decomposition rule)
    ir.constraints.iter()
        .filter(|c| c.kind == "UC" && !c.spans.is_empty())
        .for_each(|c| {
            c.spans.first().and_then(|span| ir.fact_types.get(&span.fact_type_id)).map(|ft| {
                let arity = ft.roles.len();
                let uc_span = c.spans.len();
                // For ternary+, UC must span at least n-1 roles
                (arity >= 3 && uc_span < arity - 1).then(|| errors.push(format!(
                    "UC '{}' spans {} roles on {}-ary fact type '{}' â€” must span at least {} (arity decomposition rule)",
                    c.text, uc_span, arity, ft.reading, arity - 1)));
            });
        });

    // 5. Ring constraint on non-self-referential binary
    ir.constraints.iter()
        .filter(|c| ["IR", "AS", "SY", "TR", "IT", "AC", "RF", "AT"].contains(&c.kind.as_str()))
        .for_each(|c| {
            c.spans.first().and_then(|span| ir.fact_types.get(&span.fact_type_id)).map(|ft| {
                let role_nouns: HashSet<&str> = ft.roles.iter().map(|r| r.noun_name.as_str()).collect();
                (ft.roles.len() == 2 && role_nouns.len() != 1).then(|| errors.push(format!(
                    "Ring constraint '{}' on '{}' requires both roles to be the same type, but found {:?}",
                    c.kind, ft.reading, role_nouns)));
            });
        });

    // 6. Constraint references undeclared fact type
    // Skip when: (a) span FT ID is a prefix of a declared FT, or
    // (b) every noun mentioned in the span FT ID is a declared noun.
    // Both cases are parser resolution mismatches (XUC, "per", inverse
    // readings, "the same" artifacts), not modeling errors.
    let noun_names_sorted: Vec<&str> = {
        let mut v: Vec<&str> = ir.nouns.keys().map(|s| s.as_str()).collect();
        v.sort_by(|a, b| b.len().cmp(&a.len()));
        v
    };
    ir.constraints.iter()
        .flat_map(|c| c.spans.iter())
        .filter(|span| !span.fact_type_id.is_empty() && !ir.fact_types.contains_key(&span.fact_type_id))
        .filter(|span| !ir.fact_types.keys().any(|k| k.starts_with(&span.fact_type_id)))
        .filter(|span| {
            // Check if all nouns in the span FT ID are declared â€” if so,
            // the modeling is correct and the parser just failed to resolve.
            let id = span.fact_type_id.replace('_', " ");
            let found: Vec<&&str> = noun_names_sorted.iter()
                .filter(|n| id.contains(**n))
                .collect();
            found.is_empty() // only warn if NO declared nouns found
        })
        .for_each(|span| errors.push(format!(
            "Constraint span references undeclared fact type '{}'", span.fact_type_id)));

    errors
}

/// Read-only index over the Object state — not a separate IR. The canonical
/// representation lives in the cells; this struct is the per-compile cache
/// that lifts repeated `fetch_or_phi` + `binding` lookups into HashMaps so
/// sub-functions don't pay O(n) cell scans on every access. Rebuilt fresh
/// on every `compile(state)` call.
pub(crate) struct CellIndex {
    pub(crate) nouns: HashMap<String, NounDef>,
    pub(crate) fact_types: HashMap<String, FactTypeDef>,
    pub(crate) constraints: Vec<ConstraintDef>,
    pub(crate) derivation_rules: Vec<DerivationRuleDef>,
    pub(crate) subtypes: HashMap<String, String>,
    pub(crate) ref_schemes: HashMap<String, Vec<String>>,
    pub(crate) enum_values: HashMap<String, Vec<String>>,
    pub(crate) general_instance_facts: Vec<GeneralInstanceFact>,
    pub(crate) state_machines: HashMap<String, StateMachineDef>,
}

/// Build a CellIndex by scanning the cells of state once.
pub(crate) fn cell_index_from_state(state: &crate::ast::Object) -> CellIndex {
    use crate::ast::{fetch_or_phi, binding};

    let mut nouns: HashMap<String, NounDef> = HashMap::new();
    let mut subtypes: HashMap<String, String> = HashMap::new();
    let mut ref_schemes: HashMap<String, Vec<String>> = HashMap::new();
    let mut enum_values: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(ns) = fetch_or_phi("Noun", state).as_seq() {
        for f in ns.iter() {
            let name = binding(f, "name").unwrap_or("").to_string();
            let obj_type = binding(f, "objectType").unwrap_or("entity").to_string();
            let wa = match binding(f, "worldAssumption") {
                Some("open") => WorldAssumption::Open,
                _ => WorldAssumption::Closed,
            };
            nouns.insert(name.clone(), NounDef { object_type: obj_type, world_assumption: wa });
            if let Some(st) = binding(f, "superType") { subtypes.insert(name.clone(), st.to_string()); }
            if let Some(v) = binding(f, "referenceScheme") { ref_schemes.insert(name.clone(), v.split(',').map(|s| s.to_string()).collect()); }
            if let Some(v) = binding(f, "enumValues") { enum_values.insert(name.clone(), v.split(',').map(|s| s.to_string()).collect()); }
        }
    }
    let role_cell = fetch_or_phi("Role", state);
    let fact_types: HashMap<String, FactTypeDef> = fetch_or_phi("FactType", state).as_seq()
        .map(|facts| facts.iter().filter_map(|f| {
            let id = binding(f, "id")?.to_string();
            let reading = binding(f, "reading").unwrap_or("").to_string();
            let roles: Vec<RoleDef> = role_cell.as_seq()
                .map(|rs| rs.iter()
                    .filter(|r| binding(r, "factType") == Some(&id))
                    .map(|r| RoleDef {
                        noun_name: binding(r, "nounName").unwrap_or("").to_string(),
                        role_index: binding(r, "position").and_then(|v| v.parse().ok()).unwrap_or(0),
                    }).collect()).unwrap_or_default();
            Some((id, FactTypeDef { schema_id: String::new(), reading, readings: vec![], roles }))
        }).collect()).unwrap_or_default();
    let constraints: Vec<ConstraintDef> = fetch_or_phi("Constraint", state).as_seq()
        .map(|facts| facts.iter().map(|f| {
            // Lossless JSON path under std-deps.
            #[cfg(feature = "std-deps")]
            if let Some(json) = binding(f, "json") {
                if let Ok(c) = serde_json::from_str::<ConstraintDef>(json) { return c; }
            }
            // Fallback: reconstruct from flat fields.
            let get = |key: &str| binding(f, key).map(|s| s.to_string());
            let spans = (0..4).filter_map(|i| {
                let ft_id = get(&format!("span{}_factTypeId", i))?;
                let ri = get(&format!("span{}_roleIndex", i))?;
                Some(SpanDef { fact_type_id: ft_id, role_index: ri.parse().unwrap_or(0), subset_autofill: None })
            }).collect();
            ConstraintDef {
                id: get("id").unwrap_or_default(), kind: get("kind").unwrap_or_default(),
                modality: get("modality").unwrap_or_default(), deontic_operator: get("deonticOperator"),
                text: get("text").unwrap_or_default(), spans,
                set_comparison_argument_length: None, clauses: None, entity: get("entity"),
                min_occurrence: None, max_occurrence: None,
            }
        }).collect()).unwrap_or_default();
    let derivation_rules: Vec<DerivationRuleDef> = fetch_or_phi("DerivationRule", state).as_seq()
        .map(|facts| facts.iter().map(|f| {
            // Lossless path: deserialize full struct from the `json` field
            // if present (written by domain_to_state under std-deps).
            #[cfg(feature = "std-deps")]
            if let Some(json) = binding(f, "json") {
                if let Ok(r) = serde_json::from_str::<DerivationRuleDef>(json) {
                    return r;
                }
            }
            // Fallback: reconstruct skeleton from flat fields. re_resolve_rules
            // rebuilds from text below.
            let get = |key: &str| binding(f, key).unwrap_or("").to_string();
            DerivationRuleDef {
                id: get("id"), text: get("text"), antecedent_fact_type_ids: vec![],
                consequent_fact_type_id: get("consequentFactTypeId"),
                kind: DerivationKind::ModusPonens, join_on: vec![], match_on: vec![],
                consequent_bindings: vec![], antecedent_filters: vec![],
                consequent_computed_bindings: vec![], consequent_aggregates: vec![],
                unresolved_clauses: vec![], antecedent_role_literals: vec![], consequent_role_literals: vec![],
            }
        }).collect()).unwrap_or_default();
    let general_instance_facts: Vec<GeneralInstanceFact> = fetch_or_phi("InstanceFact", state).as_seq()
        .map(|facts| facts.iter().map(|f| {
            let get = |key: &str| binding(f, key).unwrap_or("").to_string();
            GeneralInstanceFact {
                subject_noun: get("subjectNoun"), subject_value: get("subjectValue"),
                field_name: get("fieldName"), object_noun: get("objectNoun"), object_value: get("objectValue"),
            }
        }).collect()).unwrap_or_default();
    // State machines: prefer StateMachine cell (hand-built or lossless-serialized),
    // else derive from instance facts.
    let state_machines: HashMap<String, StateMachineDef> = {
        #[cfg(feature = "std-deps")]
        {
            let cell_sms: HashMap<String, StateMachineDef> = fetch_or_phi("StateMachine", state).as_seq()
                .map(|facts| facts.iter().filter_map(|f| {
                    let name = binding(f, "name")?.to_string();
                    let json = binding(f, "json")?;
                    serde_json::from_str::<StateMachineDef>(json).ok().map(|sm| (name, sm))
                }).collect()).unwrap_or_default();
            if !cell_sms.is_empty() { cell_sms } else { derive_state_machines_from_facts(&general_instance_facts) }
        }
        #[cfg(not(feature = "std-deps"))]
        { derive_state_machines_from_facts(&general_instance_facts) }
    };

    // Re-resolve against cell data directly â€” no Domain struct (#211).
    // Skip if the rule already has structured bindings (lossless JSON path).
    let resolved_rules = {
        let mut rules = derivation_rules;
        let needs_resolve = rules.iter().any(|r|
            r.antecedent_fact_type_ids.is_empty()
                && r.consequent_aggregates.is_empty()
                && r.consequent_computed_bindings.is_empty()
        );
        if needs_resolve {
            crate::parse_forml2::re_resolve_rules(&mut rules, &nouns, &fact_types);
        }
        rules
    };

    CellIndex { nouns, fact_types, constraints, derivation_rules: resolved_rules,
        subtypes, ref_schemes, enum_values, general_instance_facts, state_machines }
}

pub(crate) fn compile(state: &crate::ast::Object) -> CompiledModel {
    let td = profile_timer::now();
    let data = cell_index_from_state(state);
    diag!("[profile] cell_index_from_state: {:?} ({} nouns, {} fts, {} constraints)", td.elapsed(), data.nouns.len(), data.fact_types.len(), data.constraints.len());
    compile_data(&data)
}

/// Core compilation: CellIndex -> CompiledModel.
fn compile_data(data: &CellIndex) -> CompiledModel {
    let t0 = profile_timer::now();
    let constraints: Vec<CompiledConstraint> = data.constraints.iter()
        .map(|def| compile_constraint(data, def))
        .collect();
    diag!("  [profile] {} constraints: {:?}", constraints.len(), t0.elapsed());

    let t1 = profile_timer::now();
    let sm_defs = derive_state_machines_from_facts(&data.general_instance_facts);
    let sm_source = if sm_defs.is_empty() { &data.state_machines } else { &sm_defs };
    let state_machines: Vec<CompiledStateMachine> = sm_source.values()
        .map(|sm_def| compile_state_machine(sm_def, &constraints))
        .collect();
    diag!("  [profile] {} state machines: {:?}", state_machines.len(), t1.elapsed());

    let t2 = profile_timer::now();
    let noun_index = build_noun_index(data, &constraints, &state_machines);
    diag!("  [profile] noun index: {:?}", t2.elapsed());

    let t3 = profile_timer::now();
    let derivations = compile_derivations(data, sm_source);
    diag!("  [profile] {} derivations: {:?}", derivations.len(), t3.elapsed());

    let t4 = profile_timer::now();
    let schemas = compile_schemas(data);
    diag!("  [profile] {} schemas: {:?}", schemas.len(), t4.elapsed());

    // Build fact-to-event mapping from schemas + state machines.
    // For each fact type, check if any role's noun has a state machine.
    // If so, check if any transition event name appears in the reading.
    // This is a heuristic until the IR carries explicit Activation/Verb links.
    // Î±(schema â†’ match event) : schemas â€” find fact types that activate transitions
    let ni_ref = &noun_index;
    let sm_ref = &state_machines;
    let fact_events: HashMap<String, FactEvent> = schemas.iter()
        .flat_map(|(ft_id, schema)| schema.role_names.iter().filter_map(move |role_name| {
            let sm_idx = ni_ref.noun_to_state_machines.get(role_name)?;
            let sm = &sm_ref[*sm_idx];
            let reading_lower = schema.reading.to_lowercase();
            sm.transition_table.iter()
                .find(|(_, to, event)| reading_lower.contains(&event.to_lowercase()) || reading_lower.contains(&to.to_lowercase()))
                .map(|(_, _, event)| (ft_id.clone(), FactEvent {
                    fact_type_id: ft_id.clone(), event_name: event.clone(), target_noun: role_name.clone(),
                }))
        }))
        .collect();

    CompiledModel { constraints, derivations, state_machines, noun_index, schemas, fact_events }
}

/// Build the NounIndex from CellIndex.
fn build_noun_index(
    data: &CellIndex,
    constraints: &[CompiledConstraint],
    state_machines: &[CompiledStateMachine],
) -> NounIndex {
    // Î±(ft â†’ Î±(role â†’ entry)) : fact_types â€” noun_name -> [(fact_type_id, role_index)]
    let noun_to_fact_types: HashMap<String, Vec<(String, usize)>> = data.fact_types.iter()
        .flat_map(|(ft_id, ft)| ft.roles.iter().map(move |role| (role.noun_name.clone(), (ft_id.clone(), role.role_index))))
        .fold(HashMap::new(), |mut acc, (noun, entry)| { acc.entry(noun).or_default().push(entry); acc });

    // noun_name -> world assumption
    let world_assumptions: HashMap<String, WorldAssumption> = data.nouns.iter()
        .map(|(name, def)| (name.clone(), def.world_assumption.clone()))
        .collect();

    // noun_name -> supertype (from data maps)
    let supertypes: HashMap<String, String> = data.subtypes.clone();
    let subtypes: HashMap<String, Vec<String>> = data.subtypes.iter()
        .fold(HashMap::new(), |mut acc, (child, parent)| { acc.entry(parent.clone()).or_default().push(child.clone()); acc });
    let ref_schemes: HashMap<String, Vec<String>> = data.ref_schemes.clone();

    // fact_type_id -> list of constraint IDs spanning it
    let fact_type_to_constraints: HashMap<String, Vec<String>> = data.constraints.iter()
        .flat_map(|cdef| cdef.spans.iter().map(move |span| (span.fact_type_id.clone(), cdef.id.clone())))
        .fold(HashMap::new(), |mut acc, (ft_id, c_id)| { acc.entry(ft_id).or_default().push(c_id); acc });

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

// -- AST Derivation Chains --------------------------------------------
// Compile derivation rules to Func::Compose chains.
// "User can access Domain iff A and B and C" becomes f  .  g  .  h
// where each step is a partial application over a schema.

/// Compile all derivation rules: explicit from IR + implicit structural rules.
/// `sm_defs` provides state machines (may differ from ir.state_machines when
/// SMs are derived from instance facts rather than old-style readings).
fn compile_derivations(data: &CellIndex, sm_defs: &HashMap<String, StateMachineDef>) -> Vec<CompiledDerivation> {
    let mut derivations = Vec::new();

    // Î±(rule â†’ compiled) : derivation_rules
    // Aggregate rules (consequent_aggregates populated) take a dedicated
    // path â€” they follow the image-set pattern (Codd Â§2.3.4) and can't
    // reuse the per-fact fanout shape.
    derivations.extend(data.derivation_rules.iter().map(|rule| {
        if !rule.consequent_aggregates.is_empty() {
            compile_aggregate_derivation(data, rule)
        } else {
            match rule.kind {
                DerivationKind::Join => compile_join_derivation(data, rule),
                _ => compile_explicit_derivation(data, rule),
            }
        }
    }));

    // Implicit: subtype inheritance from noun definitions
    derivations.extend(compile_subtype_inheritance(data));

    // Implicit: modus ponens from subset constraints
    derivations.extend(compile_modus_ponens(data));

    // Implicit: transitivity from shared roles
    derivations.extend(compile_transitivity(data));

    // Implicit: CWA negation from world assumptions
    derivations.extend(compile_cwa_negation(data));

    // Implicit: state machine initialization from SM definitions
    // Uses sm_defs (derived from instance facts) rather than ir.state_machines.
    let sm_init_derivations: Vec<_> = sm_defs.iter().map(|(noun_name, sm_def)| {
        diag!("  [profile] compiling SM init for noun={} initial={}", noun_name, sm_def.statuses.first().unwrap_or(&String::new()));
        compile_sm_init_for(noun_name, sm_def)
    }).collect();
    diag!("  [profile] {} SM init derivations", sm_init_derivations.len());
    derivations.extend(sm_init_derivations);

    derivations
}

// (Object-level population helpers obj_find_ft, obj_instances_of,
//  obj_participates_in, obj_derived_fact removed -- no longer needed
//  after eliminating all Func::Native closures. All population
//  traversal is now via pure Func: extract_facts_from_pop, instances_of_noun_func, etc.)

/// Map a Halpin-comparator string to the FFP primitive that tests it on an
/// atom pair. The pair is constructed as `<role_value, rhs_literal>` â€” the
/// resulting predicate returns T when the comparison holds, F otherwise.
///
///   ">="  â†’ Func::Ge          "<="  â†’ Func::Le
///   ">"   â†’ Func::Gt          "<"   â†’ Func::Lt
///   "="   â†’ Func::Eq          "!="  â†’ Not . Eq
///
/// Any unexpected op falls back to Func::Eq so the derivation degrades to
/// a plain equality rather than silently dropping facts. The parser
/// normalises `<>` to `!=` so we don't need both here.
fn comparator_primitive(op: &str) -> Func {
    match op {
        ">=" => Func::Ge,
        "<=" => Func::Le,
        ">"  => Func::Gt,
        "<"  => Func::Lt,
        "!=" => Func::compose(Func::Not, Func::Eq),
        "="  => Func::Eq,
        _    => Func::Eq,
    }
}

/// Format a parsed numeric literal back into an atom for the generated Func.
/// Integers round-trip without the `.0` suffix so atoms stay stable against
/// string-encoded ids (e.g. `"100"` not `"100.0"`).
fn format_numeric_atom(v: f64) -> String {
    if v.is_finite() && v == v.trunc() && v.abs() < 1e16 {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

/// Compile an aggregate derivation (Halpin attribute-style, Codd Â§2.3.4
/// image-set).
///
///   * Fact Type has Arity iff Arity is the count of Role
///                              where Fact Type has Role.
///
/// Compiles to:
///
///   Î±(derive_with_context) . DistR . [source_facts, source_facts]
///
/// where DistR.\[a, b\]:x pairs each element of a with the whole Seq b
/// (Codd cartesian product restricted to "for each fact, see every fact
/// including itself"). derive_with_context receives <one_fact, all_facts>
/// and builds:
///
///   <consequent_id, reading,
///    <<group_key_role, g_key:one_fact>,
///     <agg_role, fold . Filter(matches_outer_key) . image_pairs>>>
///
/// image_pairs = DistL.\[g_key:one_fact, all_facts\] produces
///   <<key, f1>, <key, f2>, ..., <key, fn>> â€” the Codd image set with
/// the outer key bound to every inner fact. Filter drops pairs whose
/// inner key differs; fold is per-op (Length for count; future commits
/// wire sum/avg/min/max via Insert(Add)/â€¦ etc).
///
/// Duplicates: the outer iterates every source fact, so three source
/// facts with the same group key produce three identical derivations.
/// The forward-chain layer deduplicates derived facts by (fact_type_id,
/// bindings) after emission.
fn compile_aggregate_derivation(data: &CellIndex, rule: &DerivationRuleDef) -> CompiledDerivation {
    let id = rule.id.clone();
    let text = rule.text.clone();
    let kind = rule.kind.clone();
    let consequent_id = rule.consequent_fact_type_id.clone();
    let consequent_reading = data.fact_types.get(&consequent_id)
        .map(|ft| ft.reading.clone())
        .unwrap_or_default();

    // v0: single aggregate per rule. Multi-aggregate rules (rare in
    // Halpin's examples) can compose after this lands.
    let agg = rule.consequent_aggregates.first()
        .expect("caller routed an empty-aggregates rule here");

    let source_ft = data.fact_types.get(&agg.source_fact_type_id);
    let group_key_idx = source_ft
        .and_then(|ft| ft.roles.iter().find(|r| r.noun_name == agg.group_key_role))
        .map(|r| r.role_index)
        .unwrap_or(0);
    // Target-role index: for sum/avg/min/max, the role whose values we
    // fold over. For count, only group membership matters â€” target is
    // informational so any row match is fine.
    let target_idx = source_ft
        .and_then(|ft| ft.roles.iter().find(|r| r.noun_name == agg.target_role))
        .map(|r| r.role_index)
        .unwrap_or(1);

    let source_facts = extract_facts_from_pop(&agg.source_fact_type_id);
    let g_key = role_value(group_key_idx);
    let t_val = role_value(target_idx);

    // Codd image-set: pair outer fact's group key with every inner fact.
    // DistL . [g_key_of_outer, all_facts] : <outer, all> â†’ <<k, f1>, ..., <k, fn>>
    let image_pairs = Func::compose(
        Func::DistL,
        Func::construction(vec![
            // g_key of outer fact = g_key . Selector(1) of <outer, all>
            Func::compose(g_key.clone(), Func::Selector(1)),
            Func::Selector(2),
        ]),
    );
    // Predicate over <k, f>: inner fact's g_key matches the outer key.
    let key_matches = Func::compose(
        Func::Eq,
        Func::construction(vec![
            Func::Selector(1),
            Func::compose(g_key.clone(), Func::Selector(2)),
        ]),
    );
    let filtered = Func::compose(Func::filter(key_matches), image_pairs);

    // Fold over the filtered image set. Count uses Length on the filtered
    // <key, fact> pair Seq directly. Sum/Min/Max/Avg first project the
    // target-role value out of each <key, fact> pair, then fold the
    // appropriate binary op via Backus's Insert (Â§11.2.4).
    //
    // Min/Max aren't Backus primitives; derived as:
    //   min_pair = Condition(Lt, Selector(1), Selector(2))
    //   max_pair = Condition(Gt, Selector(1), Selector(2))
    //
    // Avg = sum / count. Both sub-folds evaluate over the same filtered
    // image, wrapped as a pair and fed to Func::Div.
    let project_values = |filt: Func| Func::compose(
        Func::apply_to_all(Func::compose(t_val.clone(), Func::Selector(2))),
        filt,
    );
    let agg_value = match agg.op.as_str() {
        "count" => Func::compose(Func::Length, filtered),
        "sum" => Func::compose(
            Func::Insert(Box::new(Func::Add)),
            project_values(filtered),
        ),
        "min" => Func::compose(
            Func::Insert(Box::new(Func::condition(
                Func::Lt,
                Func::Selector(1),
                Func::Selector(2),
            ))),
            project_values(filtered),
        ),
        "max" => Func::compose(
            Func::Insert(Box::new(Func::condition(
                Func::Gt,
                Func::Selector(1),
                Func::Selector(2),
            ))),
            project_values(filtered),
        ),
        "avg" => {
            let values = project_values(filtered);
            let sum = Func::compose(Func::Insert(Box::new(Func::Add)), values.clone());
            let count = Func::compose(Func::Length, values);
            Func::compose(Func::Div, Func::construction(vec![sum, count]))
        }
        // Unknown ops collapse to count so the rule still fires with a
        // sane value rather than Ï†.
        _ => Func::compose(Func::Length, filtered),
    };

    // Derived fact: <derived_id, reading, <<group_key_role, key>, <agg_role, value>>>
    let derive_with_context = Func::construction(vec![
        Func::constant(Object::atom(&consequent_id)),
        Func::constant(Object::atom(&consequent_reading)),
        Func::construction(vec![
            Func::construction(vec![
                Func::constant(Object::atom(&agg.group_key_role)),
                // group key = g_key applied to outer fact (Selector(1))
                Func::compose(g_key.clone(), Func::Selector(1)),
            ]),
            Func::construction(vec![
                Func::constant(Object::atom(&agg.role)),
                agg_value,
            ]),
        ]),
    ]);

    // Î±(derive_with_context) . DistR . [source_facts, source_facts]
    let func = Func::compose(
        Func::apply_to_all(derive_with_context),
        Func::compose(
            Func::DistR,
            Func::construction(vec![source_facts.clone(), source_facts]),
        ),
    );

    CompiledDerivation { id, text, kind, func }
}

/// Compile a Halpin arithmetic expression (Box::Volume = Size * Size * Size)
/// into a Func that, given a single antecedent fact, produces the value.
///
/// Role references lower to `role_value(i)` against the antecedent FT's
/// role list; numeric literals become constant atoms via format_numeric_atom
/// (so `1.0` stays `"1"` and round-trips through apply_compare / apply_arith
/// unambiguously). Binary ops wrap `Func::Add|Sub|Mul|Div` around a pair
/// construction of the two sides â€” `op . [lhs, rhs]`. Unknown ops fall back
/// to Add rather than panicking so a misbuilt IR still compiles.
fn compile_arith_expr(expr: &crate::types::ArithExpr, ft: &crate::types::FactTypeDef) -> Func {
    use crate::types::ArithExpr;
    match expr {
        ArithExpr::Literal(v) => Func::constant(Object::atom(&format_numeric_atom(*v))),
        ArithExpr::RoleRef(name) => {
            let idx = ft.roles.iter().find(|r| r.noun_name == *name)
                .map(|r| r.role_index)
                .unwrap_or(0);
            role_value(idx)
        }
        ArithExpr::Op(op, lhs, rhs) => {
            let l = compile_arith_expr(lhs, ft);
            let r = compile_arith_expr(rhs, ft);
            let op_func = match op.as_str() {
                "+" => Func::Add,
                "-" => Func::Sub,
                "*" => Func::Mul,
                "/" => Func::Div,
                _   => Func::Add,
            };
            Func::compose(op_func, Func::construction(vec![l, r]))
        }
    }
}

/// Build the predicate that Func::filter will apply to each fact of an
/// antecedent Seq to enforce an AntecedentFilter (Halpin Example 5's
/// `has Population >= 1000000`). Returns `None` if the filter's `role`
/// doesn't resolve against the fact type's roles â€” in that case the
/// filter is silently dropped rather than failing the whole rule.
fn build_antecedent_filter_pred(af: &crate::types::AntecedentFilter, ft: &crate::types::FactTypeDef) -> Option<Func> {
    let role_idx = ft.roles.iter().find(|r| r.noun_name == af.role).map(|r| r.role_index)?;
    Some(Func::compose(
        comparator_primitive(&af.op),
        Func::construction(vec![
            role_value(role_idx),
            Func::constant(Object::atom(&format_numeric_atom(af.value))),
        ]),
    ))
}

/// Compile an explicit derivation rule from the IR.
///
/// Pure AST form would be:
///   Condition(
///     /And  .  alpha(Compose(Not  .  NullTest, find_ft)) : <antecedent_ids>,
///     Construction of collected bindings,
///     Constant(phi)
///   )
/// Blocked on: no Filter/Find primitive to locate a fact type by ID in the
/// population Seq. Requires a fold-based search (Insert + Condition) that
/// would be more complex than the direct Object traversal below.
fn compile_explicit_derivation(data: &CellIndex, rule: &DerivationRuleDef) -> CompiledDerivation {
    let id = rule.id.clone();
    let text = rule.text.clone();
    let kind = rule.kind.clone();
    let antecedent_ids = rule.antecedent_fact_type_ids.clone();
    let consequent_id = rule.consequent_fact_type_id.clone();
    let consequent_reading = data.fact_types.get(&consequent_id)
        .map(|ft| ft.reading.clone())
        .unwrap_or_default();

    // Per-antecedent predicates. Two sources contribute:
    //   (a) numeric comparators from `antecedent_filters` (Halpin FORML
    //       Example 5: `has Population >= 1000000`);
    //   (b) string-literal equality from `antecedent_role_literals`
    //       (FORML 2 grammar: `Statement has Trailing Marker
    //       'is an entity type'`, #286).
    // Multiple predicates on the same antecedent are combined via And.
    let mut preds_by_idx: hashbrown::HashMap<usize, Vec<Func>> =
        hashbrown::HashMap::new();
    for af in rule.antecedent_filters.iter() {
        if let Some(ft_id) = antecedent_ids.get(af.antecedent_index) {
            if let Some(ft) = data.fact_types.get(ft_id) {
                if let Some(p) = build_antecedent_filter_pred(af, ft) {
                    preds_by_idx.entry(af.antecedent_index).or_default().push(p);
                }
            }
        }
    }
    for arl in rule.antecedent_role_literals.iter() {
        if let Some(ft_id) = antecedent_ids.get(arl.antecedent_index) {
            if let Some(ft) = data.fact_types.get(ft_id) {
                if let Some(role_idx) = ft.roles.iter()
                    .find(|r| r.noun_name == arl.role)
                    .map(|r| r.role_index)
                {
                    let pred = Func::compose(
                        Func::Eq,
                        Func::construction(vec![
                            role_value(role_idx),
                            Func::constant(Object::atom(&arl.value)),
                        ]),
                    );
                    preds_by_idx.entry(arl.antecedent_index).or_default().push(pred);
                }
            }
        }
    }

    // Wrap the raw extractor in Filter when this antecedent has any
    // predicates; otherwise return the bare extractor. Multiple
    // predicates combine with And.
    let extract = |idx: usize, ft_id: &str| -> Func {
        match preds_by_idx.get(&idx) {
            Some(preds) if !preds.is_empty() => {
                let combined = preds.iter().cloned().reduce(|a, b|
                    Func::compose(Func::And, Func::construction(vec![a, b])))
                    .unwrap();
                Func::compose(Func::filter(combined), extract_facts_from_pop(ft_id))
            }
            _ => extract_facts_from_pop(ft_id),
        }
    };

    // Three shapes based on antecedent count:
    //
    // 0 antecedents â€” unconditional single derivation. Rare; mostly bootstrap
    // rules that assert a constant fact.
    //
    // 1 antecedent â€” per-fact fanout. For each antecedent fact surviving the
    // filter, produce one consequent fact whose bindings are the antecedent
    // fact's bindings. Func shape:
    //   Î±(<cons_id, cons_reading, bindings>) : Filter(p)? : a_facts
    // This is the Halpin-aligned derivation semantic: one derived fact per
    // matching antecedent tuple, not a single existence-check emit.
    //
    // 2+ antecedents â€” existence check across all antecedents. Rules that
    // want per-tuple semantics over multiple antecedents are classified as
    // DerivationKind::Join during resolve_derivation_rule and routed to
    // compile_join_derivation instead. This explicit path handles the
    // rare multi-antecedent rules without `that` anaphora.
    let func = match antecedent_ids.len() {
        0 => {
            let derived = Func::construction(vec![
                Func::constant(Object::atom(&consequent_id)),
                Func::constant(Object::atom(&consequent_reading)),
                Func::constant(Object::phi()),
            ]);
            Func::construction(vec![derived])
        }
        1 => {
            let ft_id = &antecedent_ids[0];
            // Per-fact derived: input is one antecedent fact (already in
            // <<noun, val>, ...> binding-seq shape); output is
            //   <consequent_id, consequent_reading, consequent_bindings>.
            //
            // consequent_bindings starts as the antecedent's bindings
            // (Func::Id passes them through unchanged). Two optional
            // extensions append pairs via Concat:
            //   (a) arithmetic-computed bindings (Halpin's attribute
            //       `Volume is Size * Size * Size`), and
            //   (b) literal-constant bindings (#286 — grammar rule head
            //       specifies a role's value, e.g. `Statement has
            //       Classification 'Entity Type Declaration'`).
            // Both forms build <role_name, value> pairs and append them
            // to the antecedent bindings through
            //   Concat . [Id, <p1, p2, ...>]
            // so evaluation order feeds the same input fact to Id and
            // to the arith expressions.
            let computed_pairs: Vec<Func> = data.fact_types.get(ft_id)
                .map(|ft| rule.consequent_computed_bindings.iter().map(|cb| {
                    Func::construction(vec![
                        Func::constant(Object::atom(&cb.role)),
                        compile_arith_expr(&cb.expr, ft),
                    ])
                }).collect())
                .unwrap_or_default();
            let literal_pairs: Vec<Func> = rule.consequent_role_literals.iter()
                .map(|crl| Func::construction(vec![
                    Func::constant(Object::atom(&crl.role)),
                    Func::constant(Object::atom(&crl.value)),
                ]))
                .collect();
            let extra_pairs: Vec<Func> = computed_pairs.into_iter()
                .chain(literal_pairs.into_iter())
                .collect();
            let bindings_func: Func = if extra_pairs.is_empty() {
                Func::Id
            } else {
                Func::compose(
                    Func::Concat,
                    Func::construction(vec![
                        Func::Id,
                        Func::construction(extra_pairs),
                    ]),
                )
            };
            let derive_one = Func::construction(vec![
                Func::constant(Object::atom(&consequent_id)),
                Func::constant(Object::atom(&consequent_reading)),
                bindings_func,
            ]);
            Func::compose(Func::apply_to_all(derive_one), extract(0, ft_id))
        }
        _ => {
            // Multi-antecedent existence check (legacy shape).
            let ant_checks: Vec<Func> = antecedent_ids.iter().enumerate()
                .map(|(i, ft_id)| Func::compose(
                    Func::compose(Func::Not, Func::NullTest),
                    extract(i, ft_id),
                ))
                .collect();
            let all_hold = ant_checks.into_iter()
                .reduce(|a, b| Func::compose(Func::And, Func::construction(vec![a, b])))
                .unwrap();
            // Bindings: first fact from first antecedent.
            let bindings = Func::compose(Func::Selector(1), extract(0, &antecedent_ids[0]));
            let derived = Func::construction(vec![
                Func::constant(Object::atom(&consequent_id)),
                Func::constant(Object::atom(&consequent_reading)),
                bindings,
            ]);
            Func::condition(
                all_hold,
                Func::construction(vec![derived]),
                Func::constant(Object::phi()),
            )
        }
    };
    CompiledDerivation { id, text, kind, func }
}


/// Compile a Join derivation rule -- cross-fact-type equi-join on shared noun names.
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
fn compile_join_derivation(data: &CellIndex, rule: &DerivationRuleDef) -> CompiledDerivation {
    let id = rule.id.clone();
    let text = rule.text.clone();
    let kind = rule.kind.clone();
    let antecedent_ids = rule.antecedent_fact_type_ids.clone();
    let consequent_id = rule.consequent_fact_type_id.clone();
    let join_keys = rule.join_on.clone();
    let match_pairs = rule.match_on.clone();
    let consequent_binding_names = rule.consequent_bindings.clone();
    let consequent_reading = data.fact_types.get(&consequent_id)
        .map(|ft| ft.reading.clone())
        .unwrap_or_default();

    // Build role-index lookup for each antecedent: (ft_idx, noun_name) -> role_index
    let antecedent_roles: Vec<Vec<(String, usize)>> = antecedent_ids.iter().map(|ft_id| {
        data.fact_types.get(ft_id)
            .map(|ft| ft.roles.iter().map(|r| (r.noun_name.clone(), r.role_index)).collect())
            .unwrap_or_default()
    }).collect();

    let n = antecedent_ids.len();

    // Helper: access path to the i-th fact in a depth-k nested pair structure.
    // R_1 = f0, R_2 = <f0, f1>, R_3 = <<f0, f1>, f2>, ...
    // R_k = <R_{k-1}, f_{k-1}>
    fn access_fact(i: usize, depth: usize) -> Func {
        match (depth, i) {
            (1, _) => Func::Id,
            (d, i) if i == d - 1 => Func::Selector(2),
            (d, _) => Func::compose(access_fact(i, d - 1), Func::Selector(1)),
        }
    }

    // Helper: find role index of a noun in a given antecedent FT
    let find_role = |ft_idx: usize, noun: &str| -> Option<usize> {
        antecedent_roles[ft_idx].iter()
            .find(|(n, _)| n == noun)
            .map(|(_, ri)| *ri)
    };

    // Extract facts per antecedent
    let fact_extractors: Vec<Func> = antecedent_ids.iter()
        .map(|ft_id| extract_facts_from_pop(ft_id))
        .collect();

    // Dispatch on antecedent count: 0 â†’ phi, 1 â†’ Î±(derive), â‰¥2 â†’ iterative join
    match n {
        0 => return CompiledDerivation {
            id, text, kind,
            func: Func::constant(Object::phi()),
        },
        1 => {
            // Single antecedent: no join, just derive from each fact.
            let binding_parts: Vec<Func> = consequent_binding_names.iter()
                .filter_map(|noun| find_role(0, noun).map(|ri| Func::compose(Func::Selector(ri + 1), Func::Id)))
                .collect();
            let derived = Func::construction(vec![
                Func::constant(Object::atom(&consequent_id)),
                Func::constant(Object::atom(&consequent_reading)),
                if binding_parts.is_empty() { Func::Id } else { Func::construction(binding_parts) },
            ]);
            return CompiledDerivation {
                id, text, kind,
                func: Func::compose(Func::apply_to_all(derived), fact_extractors.into_iter().next().unwrap()),
            };
        },
        _ => {},
    }

    // N >= 2: iterative pairwise join.
    // Start with facts from FT0, then join with FT1, FT2, etc.
    // After step j (0-indexed), depth = j+1, and each element is a nested structure
    // containing facts from FTs 0..=j.

    // Step 0: start with ft0_facts (depth 1)
    let ft0 = fact_extractors[0].clone();

    // For each subsequent FT, build the join step.
    // foldl(join_step, ft0, [1..n]) â€” iterative pairwise join
    let current = (1..n).fold(ft0, |current, j| {
        let ft_j = fact_extractors[j].clone();

        // Î±(key â†’ eq_condition) : join_keys â€” build join predicates
        let mut join_conds: Vec<Func> = join_keys.iter().filter_map(|key| {
            let j_role = find_role(j, key)?;
            let ref_ft = (0..j).find(|&fi| find_role(fi, key).is_some())?;
            let ref_role = find_role(ref_ft, key)?;
            let ref_val = Func::compose(role_value(ref_role),
                Func::compose(access_fact(ref_ft, j), Func::Selector(1)));
            let new_val = Func::compose(role_value(j_role), Func::Selector(2));
            Some(Func::compose(Func::Eq, Func::construction(vec![ref_val, new_val])))
        }).collect();

        // Î±(match_pair â†’ contains_condition) : match_pairs
        join_conds.extend(match_pairs.iter().filter_map(|(left_noun, right_noun)| {
            let left_ft = (0..=j).find(|&fi| find_role(fi, left_noun).is_some())?;
            let right_ft = (0..=j).find(|&fi| find_role(fi, right_noun).is_some())?;
            (left_ft == j || right_ft == j).then_some(())?;
            let left_role = find_role(left_ft, left_noun)?;
            let right_role = find_role(right_ft, right_noun)?;
            let val = |ft: usize, ri: usize| if ft == j {
                Func::compose(role_value(ri), Func::Selector(2))
            } else {
                Func::compose(role_value(ri), Func::compose(access_fact(ft, j), Func::Selector(1)))
            };
            Some(Func::compose(Func::Contains, Func::construction(vec![val(left_ft, left_role), val(right_ft, right_role)])))
        }));

        let join_pred = match join_conds.len() {
            0 => Func::constant(Object::t()),
            1 => join_conds.into_iter().next().unwrap(),
            _ => join_conds.into_iter().reduce(|a, b|
                Func::compose(Func::And, Func::construction(vec![a, b]))).unwrap(),
        };

        // Pipeline: Filter(join_pred) . Concat . Î±(DistL) . DistR . [current, ft_j]
        Func::compose(Func::filter(join_pred), Func::compose(Func::Concat,
            Func::compose(Func::apply_to_all(Func::DistL),
                Func::compose(Func::DistR, Func::construction(vec![current, ft_j])))))
    });

    // Build the consequent fact from the final joined structure (depth n).
    // For each consequent binding noun, find which FT has it and extract the value.
    let binding_nouns: Vec<String> = if consequent_binding_names.is_empty() {
        // Î±(roles â†’ nouns) : antecedents â€” deduplicated
        antecedent_roles.iter()
            .flat_map(|roles| roles.iter().map(|(noun, _)| noun.clone()))
            .fold(Vec::new(), |mut acc, noun| { if !acc.contains(&noun) { acc.push(noun); } acc })
    } else {
        consequent_binding_names
    };

    // Î±(noun â†’ extractor) : binding_nouns
    let binding_parts: Vec<Func> = binding_nouns.iter().filter_map(|noun| {
        let fi = (0..n).find(|&fi| find_role(fi, noun).is_some())?;
        let ri = find_role(fi, noun)?;
        let extractor = Func::compose(role_value(ri), access_fact(fi, n));
        Some(Func::construction(vec![Func::constant(Object::atom(noun)), extractor]))
    }).collect();

    let derived_fact = Func::construction(vec![
        Func::constant(Object::atom(&consequent_id)),
        Func::constant(Object::atom(&consequent_reading)),
        if binding_parts.is_empty() {
            Func::constant(Object::phi())
        } else {
            Func::construction(binding_parts)
        },
    ]);

    let func = Func::compose(Func::apply_to_all(derived_fact), current);

    CompiledDerivation { id, text, kind, func }
}

// (join_recursive, check_join_keys, check_match_predicates removed --
//  join logic now expressed as pure Func via pairwise DistR/DistL/Filter/Concat.)

/// Subtype inheritance: for each noun with a supertype,
/// instances of the subtype inherit participation in the supertype's fact types.
///
/// Pure AST form would be:
///   For each supertype fact type:
///     alpha(Condition(Not  .  participates, construct_derived, Constant(phi)))  .  instances
///   Blocked on: instances_of requires a global scan (fold over all fact types
///   extracting bindings), and participates_in requires a find-by-ID lookup.
///   Both need Filter/Find primitives not yet in the AST.
fn compile_subtype_inheritance(data: &CellIndex) -> Vec<CompiledDerivation> {
    // Î±(subtype_pair â†’ derivation) : subtypes â€” filter out pairs with no supertype fact types
    data.subtypes.iter().filter_map(|(sub_name, super_name)| {
        let sft: Vec<(String, String, usize)> = data.fact_types.iter()
            .flat_map(|(ft_id, ft)| ft.roles.iter()
                .filter(|r| r.noun_name == *super_name)
                .map(move |r| (ft_id.clone(), ft.reading.clone(), r.role_index)))
            .collect();
        (!sft.is_empty()).then_some(())?;

        let sub = sub_name.clone();
        let sup = super_name.clone();
        let instances = instances_of_noun_func(&sub);

        // Î±(super_ft â†’ check_and_derive) : super_fact_types
        let ft_checks: Vec<Func> = sft.iter().map(|(ft_id, reading, role_idx)| {
            let ft_facts = extract_facts_from_pop(ft_id);
            let inst_in_fact = Func::compose(Func::Eq, Func::construction(vec![
                Func::compose(role_value(*role_idx), Func::Selector(2)),
                Func::Selector(1),
            ]));
            let participates = Func::compose(
                Func::compose(Func::Not, Func::NullTest),
                Func::compose(Func::filter(inst_in_fact), Func::DistL));
            let derived_fact = Func::construction(vec![
                Func::constant(Object::atom(ft_id)),
                Func::constant(Object::atom(reading)),
                Func::construction(vec![Func::construction(vec![
                    Func::constant(Object::atom(&sup)), Func::Selector(1)])]),
            ]);
            let check_one = Func::condition(
                Func::compose(Func::Not, participates),
                Func::construction(vec![derived_fact]),
                Func::constant(Object::phi()));
            Func::compose(Func::Concat, Func::compose(
                Func::apply_to_all(check_one),
                Func::compose(Func::DistR, Func::construction(vec![instances.clone(), ft_facts]))))
        }).collect();

        let func = match ft_checks.len() {
            0 => Func::constant(Object::phi()),
            1 => ft_checks.into_iter().next().unwrap(),
            _ => Func::construction(ft_checks),
        };
        Some(CompiledDerivation {
            id: format!("_subtype_{}_{}", sub, sup),
            text: format!("{} is a subtype of {} -- inherits fact types", sub, sup),
            kind: DerivationKind::SubtypeInheritance, func,
        })
    }).collect()
}

/// Modus ponens on subset constraints: if A subset B (SS constraint),
/// when we find an instance in A, derive its presence in B.
///
/// Pure AST form would be:
///   alpha(Condition(Not  .  exists_in_B, construct_B_fact, Constant(phi)))
///      .  alpha(project_to_B_nouns)
///      .  find_ft(A)
/// Blocked on: find_ft requires searching the population Seq by atom ID,
/// and exists_in_B needs a nested membership check. Both need a fold-based
/// search primitive (Insert + Condition) not yet ergonomic in the AST.
fn compile_modus_ponens(data: &CellIndex) -> Vec<CompiledDerivation> {
    // Î±(ss_constraint â†’ derivation) : Filter(kind=SS âˆ§ spansâ‰¥2) : constraints
    data.constraints.iter()
        .filter(|cdef| cdef.kind == "SS" && cdef.spans.len() >= 2)
        .filter_map(|cdef| {

        // Only derive facts when subset_autofill is explicitly true.
        // Otherwise the SS constraint is just a constraint (produces violations,
        // doesn't auto-create facts).
        let has_autofill = cdef.spans.iter().any(|s| s.subset_autofill == Some(true));
        has_autofill.then_some(())?;

        let a_ft_id = cdef.spans[0].fact_type_id.clone();
        let b_ft_id = cdef.spans[1].fact_type_id.clone();

        // Collect role noun names from both fact types for full tuple propagation
        let b_role_names: Vec<String> = data.fact_types.get(&b_ft_id)
            .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
            .unwrap_or_default();

        let b_reading = data.fact_types.get(&b_ft_id)
            .map(|ft| ft.reading.clone())
            .unwrap_or_default();

        let id = format!("_modus_ponens_{}", cdef.id);
        let text = format!("Modus ponens from SS constraint: {}", cdef.text);

        // Pure Func: for each A-fact not in B, derive a B-fact.
        // Uses same pattern as compile_subset_ast membership check.
        let a_role_names: Vec<String> = data.fact_types.get(&a_ft_id)
            .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
            .unwrap_or_default();

        // Common nouns and their role indices in A and B
        let common: Vec<(usize, usize)> = a_role_names.iter().enumerate().filter_map(|(ai, n)| {
            b_role_names.iter().position(|bn| bn == n).map(|bi| (ai, bi))
        }).collect();

        let a_facts = extract_facts_from_pop(&a_ft_id);
        let b_facts = extract_facts_from_pop(&b_ft_id);

        // match_pred: <a_fact, b_candidate> -> common noun values all equal
        let match_pred = if common.is_empty() {
            Func::constant(Object::t())
        } else {
            let eqs: Vec<Func> = common.iter().map(|&(ai, bi)| {
                Func::compose(Func::Eq, Func::construction(vec![
                    Func::compose(role_value(ai), Func::Selector(1)),
                    Func::compose(role_value(bi), Func::Selector(2)),
                ]))
            }).collect();
            if eqs.len() == 1 {
                eqs.into_iter().next().unwrap()
            } else {
                eqs.into_iter().reduce(|acc, eq| {
                    Func::compose(Func::And, Func::construction(vec![acc, eq]))
                }).unwrap()
            }
        };

        // not_in_b: <a_fact, b_facts> -> T when no b_candidate matches a_fact
        let not_in_b = Func::compose(
            Func::NullTest,
            Func::compose(Func::filter(match_pred), Func::DistL),
        );

        // derived_fact: <a_fact, b_facts> -> <b_ft_id, b_reading, <bindings>>
        // Project a_fact's common-noun bindings into B's structure.
        let b_binding_funcs: Vec<Func> = common.iter()
            .map(|&(ai, _)| Func::compose(Func::Selector(ai + 1), Func::Selector(1)))
            .collect();
        let derived_fact = Func::construction(vec![
            Func::constant(Object::atom(&b_ft_id)),
            Func::constant(Object::atom(&b_reading)),
            if b_binding_funcs.is_empty() {
                Func::constant(Object::phi())
            } else {
                // Reuse a_fact's bindings directly (already in <<noun, val>> format)
                Func::Selector(1)
            },
        ]);

        // a(derived_fact) . Filter(not_in_b) . DistR . [a_facts, b_facts]
        let func = Func::compose(
            Func::apply_to_all(derived_fact),
            Func::compose(
                Func::filter(not_in_b),
                Func::compose(Func::DistR, Func::construction(vec![a_facts, b_facts])),
            ),
        );
        Some(CompiledDerivation { id, text, kind: DerivationKind::ModusPonens, func })
    }).collect()
}

/// Transitivity: for fact types that share a noun in different roles (A->B, B->C),
/// derive the transitive closure A->C. Limited depth to prevent infinite chains.
///
/// Pure Func form:
///   a(derived_fact) . Filter(join_cond) . Concat . a(Filter(join) . DistL) . DistR . [ft1_facts, ft2_facts]
///   where join_cond checks role_value(1)(f1) = role_value(0)(f2) on the shared noun.
fn compile_transitivity(data: &CellIndex) -> Vec<CompiledDerivation> {
    // Cross-product of binary fact types, filtered by shared noun (A->B, B->C)
    let binary_fts: Vec<(&String, &FactTypeDef)> = data.fact_types.iter()
        .filter(|(_, ft)| ft.roles.len() == 2)
        .collect();

    binary_fts.iter().enumerate()
        .flat_map(|(i, (ft1_id, ft1))| binary_fts.iter().enumerate()
            .filter(move |(j, _)| *j != i)
            .filter_map(move |(_, (ft2_id, ft2))| {
                // Filter: ft1's role[1] noun == ft2's role[0] noun
                (ft1.roles[1].noun_name == ft2.roles[0].noun_name).then(|| ())?;

                let shared_noun = ft1.roles[1].noun_name.clone();
                let src_noun = ft1.roles[0].noun_name.clone();
                let dst_noun = ft2.roles[1].noun_name.clone();
                let ft1_id_c = (*ft1_id).clone();
                let ft2_id_c = (*ft2_id).clone();
                let reading = format!("{} transitively relates to {} via {}", src_noun, dst_noun, shared_noun);
                let transitive_ft_id = format!("_transitive_{}_{}", ft1_id_c, ft2_id_c);

                let ft1_facts = extract_facts_from_pop(&ft1_id_c);
                let ft2_facts = extract_facts_from_pop(&ft2_id_c);

                let join_cond = Func::compose(Func::Eq, Func::construction(vec![
                    Func::compose(role_value(1), Func::Selector(1)),
                    Func::compose(role_value(0), Func::Selector(2)),
                ]));

                let derived_fact = Func::construction(vec![
                    Func::constant(Object::atom(&transitive_ft_id)),
                    Func::constant(Object::atom(&reading)),
                    Func::construction(vec![
                        Func::construction(vec![Func::constant(Object::atom(&src_noun)), Func::compose(role_value(0), Func::Selector(1))]),
                        Func::construction(vec![Func::constant(Object::atom(&dst_noun)), Func::compose(role_value(1), Func::Selector(2))]),
                    ]),
                ]);

                let func = Func::compose(Func::apply_to_all(derived_fact), Func::compose(Func::Concat,
                    Func::compose(Func::apply_to_all(Func::compose(Func::filter(join_cond), Func::DistL)),
                        Func::compose(Func::DistR, Func::construction(vec![ft1_facts, ft2_facts])))));

                Some(CompiledDerivation {
                    id: format!("_transitivity_{}_{}", ft1_id_c, ft2_id_c),
                    text: reading, kind: DerivationKind::Transitivity, func,
                })
            }))
        .collect()
}

/// CWA negation: for nouns with WorldAssumption::Closed,
/// if a fact type involving this noun has no instances for a given entity,
/// derive the negation. For OWA nouns, absence is unknown, not false.
///
/// Pure Func form (per fact type):
///   Concat . a(Condition(NullTest . Filter(match) . DistL, [negation], phi)) . DistR . [instances, ft_facts]
///   where match checks role_value(ri)(fact) = instance on each <instance, fact> pair.
fn compile_cwa_negation(data: &CellIndex) -> Vec<CompiledDerivation> {
    data.nouns.iter()
        .filter(|(_, def)| def.world_assumption == WorldAssumption::Closed)
        .filter_map(|(noun_name, _)| {
            let relevant_fts: Vec<(String, String, usize)> = data.fact_types.iter()
                .flat_map(|(ft_id, ft)| ft.roles.iter()
                    .filter(|r| r.noun_name == *noun_name)
                    .map(move |r| (ft_id.clone(), ft.reading.clone(), r.role_index)))
                .collect();
            (!relevant_fts.is_empty()).then_some(())?;

            let noun = noun_name.clone();
            let id = format!("_cwa_negation_{}", noun);
            let text = format!("CWA: absent facts about {} are false", noun);
            let instances = instances_of_noun_func(&noun);

            let per_ft_funcs: Vec<Func> = relevant_fts.iter().map(|(ft_id, reading, role_idx)| {
            let ft_facts = extract_facts_from_pop(ft_id);

            // Match condition for <instance, fact> pair from DistL:
            // eq . [Sel(1), role_value(role_idx) . Sel(2)]
            // Sel(1) = instance, Sel(2) = fact, role_value extracts the noun's value from fact
            let match_cond = Func::compose(Func::Eq, Func::construction(vec![
                Func::Selector(1),
                Func::compose(role_value(*role_idx), Func::Selector(2)),
            ]));

            // For each <instance, all_facts> pair from DistR:
            //   Filter(match_cond) . DistL gives matching <instance, fact> pairs
            //   NullTest checks if any matches exist
            let participation_check = Func::compose(
                Func::NullTest,
                Func::compose(Func::filter(match_cond), Func::DistL),
            );

            // Negation fact goes to a SEPARATE cell ("_cwa_negation:<ft_id>")
            // and its noun binding is prefixed ("_neg_<noun>") so that
            // presence constraints (MC, FC, â€¦) enumerating positive
            // noun instances via instances_of_noun_func never see the
            // "NOT" facts. The backward-chain prover (evaluate.rs)
            // consults the derived fact list directly; it does not read
            // this cell.
            let neg_cell = format!("_cwa_negation:{}", ft_id);
            let neg_noun = format!("_neg_{}", noun);
            let neg_reading = format!("NOT: {} (CWA negation for {})", reading, noun);
            let negation_fact = Func::construction(vec![
                Func::constant(Object::atom(&neg_cell)),
                Func::constant(Object::atom(&neg_reading)),
                Func::construction(vec![
                    Func::construction(vec![
                        Func::constant(Object::atom(&neg_noun)),
                        Func::Selector(1),
                    ]),
                ]),
            ]);

            // Condition: if NullTest (no participation) -> wrap negation in singleton for Concat;
            //            else -> phi (empty, contributes nothing to Concat)
            let per_instance = Func::condition(
                participation_check,
                Func::construction(vec![negation_fact]),
                Func::constant(Object::phi()),
            );

            // Full pipeline for this ft:
            // Concat . a(per_instance) . DistR . [instances, ft_facts]
            let per_ft = Func::compose(
                Func::Concat,
                Func::compose(
                    Func::apply_to_all(per_instance),
                    Func::compose(Func::DistR, Func::construction(vec![instances.clone(), ft_facts])),
                ),
            );
            per_ft
        }).collect();

            let func = match per_ft_funcs.len() {
                1 => per_ft_funcs.into_iter().next().unwrap(),
                _ => Func::compose(Func::Concat, Func::construction(per_ft_funcs)),
            };
            Some(CompiledDerivation { id, text, kind: DerivationKind::ClosedWorldNegation, func })
        }).collect()
}

/// State machine initialization as a derivation rule.
///
/// Paper: "State machine initialization is not a separate step. The derivation
/// rules produce the State Machine instance and its initial Status as derived facts."
///
/// For each noun with a state machine definition, when an entity of that noun
/// exists in the population but no State Machine is for that entity, derive:
///   - State Machine instance (instanceOf, forResource, currentlyInStatus = initial)
fn compile_sm_init_for(noun_name: &str, sm_def: &StateMachineDef) -> CompiledDerivation {
        let sm_noun = sm_def.noun_name.clone();
        let initial_status = sm_def.statuses.first().cloned().unwrap_or_default();
        let id_str = format!("_sm_init_{}", noun_name);
        let text_str = format!("SM init for {}", noun_name);

        let get_instances = instances_of_noun_func(&sm_noun);

        let extract_for_resource = Func::compose(
            Func::apply_to_all(Func::Selector(2)),
            Func::filter(Func::compose(Func::Eq, Func::construction(vec![
                Func::Selector(1),
                Func::constant(Object::atom("forResource")),
            ]))),
        );
        // extract_facts_from_pop returns phi when fact type not found.
        // Guard: if null, return phi. Otherwise Selector(2) extracts facts.
        let safe_extract = Func::condition(
            Func::NullTest,
            Func::constant(Object::phi()),
            Func::Selector(2),
        );
        let get_existing = Func::compose(
            Func::Concat,
            Func::compose(
                Func::apply_to_all(extract_for_resource),
                Func::compose(safe_extract, extract_facts_from_pop("StateMachine_has_forResource")),
            ),
        );

        let pairs = Func::construction(vec![get_instances, get_existing]);

        let is_new = Func::compose(
            Func::NullTest,
            Func::compose(
                Func::filter(Func::compose(Func::Eq, Func::construction(vec![
                    Func::compose(Func::Selector(1), Func::Selector(1)),
                    Func::Id,
                ]))),
                Func::Selector(2),
            ),
        );

        // set_diff = alpha(sel(1)) . Filter(not_member) . distr
        // not_member = null . Filter(eq) . distl
        // distr : <R, S> -> <<r1,S>,...,<rn,S>>
        // For each <ri, S>: distl : <ri, S> -> <<ri,s1>,...> then Filter(eq) finds matches.
        // null : phi -> T when ri not in S. Handles empty S correctly (distl:<ri,phi>=phi, null:phi=T).
        let new_instances = Func::compose(
            Func::apply_to_all(Func::Selector(1)),
            Func::compose(Func::filter(is_new), Func::compose(Func::DistR, pairs)),
        );

        let sm_noun_obj = Object::atom(&sm_noun);
        let initial_obj = Object::atom(&initial_status);
        let derive_facts = Func::apply_to_all(Func::construction(vec![
            Func::construction(vec![
                Func::constant(Object::atom("StateMachine_has_instanceOf")),
                Func::constant(Object::atom("SM instanceOf")),
                Func::construction(vec![
                    Func::construction(vec![Func::constant(Object::atom("State Machine")), Func::Id]),
                    Func::construction(vec![Func::constant(Object::atom("instanceOf")), Func::constant(sm_noun_obj.clone())]),
                ]),
            ]),
            Func::construction(vec![
                Func::constant(Object::atom("StateMachine_has_currentlyInStatus")),
                Func::constant(Object::atom("SM initial status")),
                Func::construction(vec![
                    Func::construction(vec![Func::constant(Object::atom("State Machine")), Func::Id]),
                    Func::construction(vec![Func::constant(Object::atom("currentlyInStatus")), Func::constant(initial_obj.clone())]),
                ]),
            ]),
            Func::construction(vec![
                Func::constant(Object::atom("StateMachine_has_forResource")),
                Func::constant(Object::atom("SM forResource")),
                Func::construction(vec![
                    Func::construction(vec![Func::constant(Object::atom("State Machine")), Func::Id]),
                    Func::construction(vec![Func::constant(Object::atom("forResource")), Func::Id]),
                ]),
            ]),
        ]));

        let func = Func::compose(Func::Concat, Func::compose(derive_facts, new_instances));

        CompiledDerivation { id: id_str, text: text_str, kind: DerivationKind::SubtypeInheritance, func }
}

fn compile_constraint(data: &CellIndex, def: &ConstraintDef) -> CompiledConstraint {
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
            compile_forbidden_ast(data, def)
        }
        Modality::Deontic(DeonticOp::Obligatory) => {
            compile_obligatory_ast(data, def)
        }
        Modality::Alethic => match def.kind.as_str() {
            // -- Pure AST constraints --------------------------------
            "IR" => compile_ring_irreflexive_ast(def),
            "AS" => compile_ring_asymmetric_ast(def),
            "SY" => compile_ring_symmetric_ast(def),
            "AT" | "ANS" => compile_ring_antisymmetric_ast(def),

            // -- AST with Native evaluation kernel --------------------
            "UC" => compile_uniqueness_ast(data, def),
            "MC" => compile_mandatory_ast(data, def),

            // -- AST with Native evaluation kernel (continued) --------
            "FC" => compile_frequency_ast(data, def),
            "VC" => compile_value_constraint_ast(data, def),
            "IT" => compile_ring_intransitive_ast(def),
            "TR" => compile_ring_transitive_ast(def),
            "AC" => compile_ring_acyclic_ast(def),
            "RF" => compile_ring_reflexive_ast(data, def),
            "XO" => compile_set_comparison_ast(data, def, |n| n != 1, "exactly one"),
            "XC" => compile_set_comparison_ast(data, def, |n| n > 1, "at most one"),
            "OR" => compile_set_comparison_ast(data, def, |n| n < 1, "at least one"),
            "SS" => compile_subset_ast(data, def),
            "EQ" => compile_equality_ast(data, def),
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

// -- Ring Constraints ---------------------------------------------
// Ring constraints on binary self-referential fact types.
// Each returns a Func that takes an eval context Object -> violations.

/// IR: not exists(x,x) -- no fact where both roles reference the same entity.
/// alpha(make_violation)  .  Filter(eq  .  [role1_val, role2_val])  .  facts
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

    // alpha(make_viol)  .  Filter(is_self_ref)  .  extract_facts
    Func::compose(
        Func::apply_to_all(viol),
        Func::compose(Func::filter(is_self_ref), facts),
    )
}

/// AS: xRy -> not yRx -- if (x,y) exists and (y,x) exists, violation.
/// Uses DistL + Filter to check for reverse pairs.
fn compile_ring_asymmetric_ast(def: &ConstraintDef) -> Func {
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();
    let facts = extract_facts_multi(&ft_ids);

    // For each pair (x,y) where x!=y, check if (y,x) also exists.
    // This is O(n^2) but populations are entity-scoped (bounded).

    // AS: xRy -> not yRx. Violation when both <x,y> and <y,x> exist (and x!=y).
    //
    // Pure Func using distl for membership test:
    //   distr  .  [facts, facts] : ctx -> <<f1, all>, <f2, all>, ...>
    //   For each <fact, all>:
    //     distl : <fact, all> -> <<fact,f1>, <fact,f2>, ...>
    //     Filter(match_reversed) -> candidates where role0(candidate)=role1(fact)  AND  role1(candidate)=role0(fact)
    //     not null -> has_reverse
    //   Filter facts where has_reverse  AND  x!=y, wrap in violations.

    // match_reversed: <fact, candidate> -> role0(cand) = role1(fact)  AND  role1(cand) = role0(fact)
    let match_reversed = Func::compose(Func::And, Func::construction(vec![
        Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(role_value(0), Func::Selector(2)), // role0(candidate)
            Func::compose(role_value(1), Func::Selector(1)), // role1(fact)
        ])),
        Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(role_value(1), Func::Selector(2)), // role1(candidate)
            Func::compose(role_value(0), Func::Selector(1)), // role0(fact)
        ])),
    ]));

    // check_one: <fact, all_facts> -> T if reverse exists, else F
    let check_one = Func::compose(
        Func::compose(Func::Not, Func::NullTest),
        Func::compose(Func::filter(match_reversed), Func::DistL),
    );

    // not_self on original fact: role0 != role1
    let not_self = Func::compose(Func::Not, Func::compose(Func::Eq, Func::construction(vec![
        Func::compose(role_value(0), Func::Selector(1)),
        Func::compose(role_value(1), Func::Selector(1)),
    ])));

    // combined: has_reverse  AND  not_self
    let pred = Func::compose(Func::And, Func::construction(vec![check_one, not_self]));

    // violation detail from <fact, all_facts> -- uses fact (sel1)
    let detail = Func::construction(vec![
        Func::constant(Object::atom("Asymmetric violation:")),
        Func::compose(role_value(0), Func::Selector(1)),
        Func::constant(Object::atom("relates to")),
        Func::compose(role_value(1), Func::Selector(1)),
        Func::constant(Object::atom("and vice versa")),
    ]);
    let viol = make_violation_func(&def.id, &def.text, detail);

    // alpha(make_viol)  .  Filter(pred)  .  distr  .  [facts, facts] : ctx
    Func::compose(
        Func::apply_to_all(viol),
        Func::compose(
            Func::filter(pred),
            Func::compose(Func::DistR, Func::construction(vec![facts.clone(), facts])),
        ),
    )
}

/// SY: xRy -> yRx -- violation when reverse is missing.
fn compile_ring_symmetric_ast(def: &ConstraintDef) -> Func {
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();
    let facts = extract_facts_multi(&ft_ids);

    let match_reversed = Func::compose(Func::And, Func::construction(vec![
        Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(role_value(0), Func::Selector(2)),
            Func::compose(role_value(1), Func::Selector(1)),
        ])),
        Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(role_value(1), Func::Selector(2)),
            Func::compose(role_value(0), Func::Selector(1)),
        ])),
    ]));

    let has_no_reverse = Func::compose(
        Func::NullTest,
        Func::compose(Func::filter(match_reversed), Func::DistL),
    );

    let not_self = Func::compose(Func::Not, Func::compose(Func::Eq, Func::construction(vec![
        Func::compose(role_value(0), Func::Selector(1)),
        Func::compose(role_value(1), Func::Selector(1)),
    ])));

    let pred = Func::compose(Func::And, Func::construction(vec![has_no_reverse, not_self]));

    let detail = Func::construction(vec![
        Func::constant(Object::atom("Symmetric violation:")),
        Func::compose(role_value(0), Func::Selector(1)),
        Func::constant(Object::atom("relates to")),
        Func::compose(role_value(1), Func::Selector(1)),
        Func::constant(Object::atom("but not the reverse")),
    ]);
    let viol = make_violation_func(&def.id, &def.text, detail);

    Func::compose(
        Func::apply_to_all(viol),
        Func::compose(
            Func::filter(pred),
            Func::compose(Func::DistR, Func::construction(vec![facts.clone(), facts])),
        ),
    )
}

/// AT/ANS: xRy  AND  yRx -> x = y -- violation when both directions exist for distinct entities.
fn compile_ring_antisymmetric_ast(def: &ConstraintDef) -> Func {
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();
    let facts = extract_facts_multi(&ft_ids);

    let match_reversed = Func::compose(Func::And, Func::construction(vec![
        Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(role_value(0), Func::Selector(2)),
            Func::compose(role_value(1), Func::Selector(1)),
        ])),
        Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(role_value(1), Func::Selector(2)),
            Func::compose(role_value(0), Func::Selector(1)),
        ])),
    ]));

    let has_reverse = Func::compose(
        Func::compose(Func::Not, Func::NullTest),
        Func::compose(Func::filter(match_reversed), Func::DistL),
    );

    let not_self = Func::compose(Func::Not, Func::compose(Func::Eq, Func::construction(vec![
        Func::compose(role_value(0), Func::Selector(1)),
        Func::compose(role_value(1), Func::Selector(1)),
    ])));

    let pred = Func::compose(Func::And, Func::construction(vec![has_reverse, not_self]));

    let detail = Func::construction(vec![
        Func::constant(Object::atom("Antisymmetric violation:")),
        Func::compose(role_value(0), Func::Selector(1)),
        Func::constant(Object::atom("and")),
        Func::compose(role_value(1), Func::Selector(1)),
        Func::constant(Object::atom("relate to each other but are not the same")),
    ]);
    let viol = make_violation_func(&def.id, &def.text, detail);

    Func::compose(
        Func::apply_to_all(viol),
        Func::compose(
            Func::filter(pred),
            Func::compose(Func::DistR, Func::construction(vec![facts.clone(), facts])),
        ),
    )
}

/// IT: xRy  AND  yRz -> not xRz -- violation when transitive shortcut exists.
fn compile_ring_intransitive_ast(def: &ConstraintDef) -> Func {
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();
    let facts = extract_facts_multi(&ft_ids);

    // IT: xRy ^ yRz -> not xRz. Violation when shortcut exists.
    //
    // Step 1: Find chains. distr [facts, facts] -> <f1, all>, distl -> <f1,f2> pairs.
    //   Filter: role1(f1) = role0(f2) AND role0(f1) != role1(f2) (chain, not self-loop)
    //   Result: <chain_pair, ...> where chain_pair = <xRy, yRz>
    //
    // Step 2: For each chain, check shortcut exists.
    //   Shortcut = <role0(f1), role1(f2)> = <x, z>
    //   distr [chains, all_facts], distl, Filter(shortcut matches candidate)

    // chain predicate: <f1, f2> -> role1(f1) = role0(f2) AND role0(f1) != role1(f2)
    let is_chain = Func::compose(Func::And, Func::construction(vec![
        Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(role_value(1), Func::Selector(1)), // role1(f1)
            Func::compose(role_value(0), Func::Selector(2)), // role0(f2)
        ])),
        Func::compose(Func::Not, Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(role_value(0), Func::Selector(1)), // role0(f1) = x
            Func::compose(role_value(1), Func::Selector(2)), // role1(f2) = z
        ]))),
    ]));

    // all_pairs: distr [facts, facts] -> <f, all> for each f
    // then distl + filter(is_chain) finds chains
    // But we need chains as <f1, f2> pairs AND the full facts for shortcut check.
    // Structure: for each <f, all>, distl gives <f, f'> pairs, filter chains.
    // Then for each chain <f1, f2>, pair with all_facts again for shortcut test.

    // shortcut_match: <<chain, candidate>> -> role0(candidate) = role0(f1) AND role1(candidate) = role1(f2)
    // chain is sel1, candidate is sel2
    // f1 = sel1(sel1), f2 = sel2(sel1)
    let shortcut_match = Func::compose(Func::And, Func::construction(vec![
        Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(role_value(0), Func::Selector(2)),                    // role0(candidate)
            Func::compose(role_value(0), Func::compose(Func::Selector(1), Func::Selector(1))), // role0(f1)
        ])),
        Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(role_value(1), Func::Selector(2)),                    // role1(candidate)
            Func::compose(role_value(1), Func::compose(Func::Selector(2), Func::Selector(1))), // role1(f2)
        ])),
    ]));

    let has_shortcut = Func::compose(
        Func::compose(Func::Not, Func::NullTest),
        Func::compose(Func::filter(shortcut_match), Func::DistL),
    );

    let detail = Func::construction(vec![
        Func::constant(Object::atom("Intransitive violation:")),
        Func::compose(role_value(0), Func::compose(Func::Selector(1), Func::Selector(1))),
        Func::constant(Object::atom("relates to")),
        Func::compose(role_value(1), Func::compose(Func::Selector(1), Func::Selector(1))),
        Func::constant(Object::atom("relates to")),
        Func::compose(role_value(1), Func::compose(Func::Selector(2), Func::Selector(1))),
        Func::constant(Object::atom("but shortcut also exists")),
    ]);
    let viol = make_violation_func(&def.id, &def.text, detail);

    // Pipeline:
    // 1. distr [facts, facts] : ctx -> <f, all> pairs
    // 2. a(distl) -> <f, f'> nested pairs (but we need to flatten)
    // 3. Since we can't flatten cleanly, restructure:
    //    For each <f, all>: filter(is_chain) . distl -> chains for this f
    //    Then for each chain, pair with all for shortcut test
    //
    // Actually: use distr twice. First distr for chains, second distr for shortcut.
    // This is a three-level composition. Let me use the <fact, all> pattern twice.

    // find_chains: <f, all> -> chains where f is f1
    let find_chains_for_f = Func::compose(Func::filter(is_chain), Func::DistL);

    // For each <f, all>, find chains, then for each chain check shortcut.
    // Result per <f, all>: violations for chains starting with f.
    let check_f = Func::compose(
        Func::apply_to_all(viol),
        Func::compose(
            Func::filter(has_shortcut),
            // pair each chain with all_facts: distr . [chains, sel2(outer)]
            // outer = <f, all>, chains = find_chains_for_f(outer)
            // We need: <chain, all> for each chain. Use distr . [chains, all].
            // But chains and all come from the same <f, all> input.
            Func::compose(Func::DistR, Func::construction(vec![
                find_chains_for_f, // chains for this f
                Func::Selector(2), // all_facts
            ])),
        ),
    );

    // Top level: a(check_f) . distr . [facts, facts] : ctx
    Func::compose(
        // Flatten: the outer a(check_f) produces a seq of seqs (violations per f).
        // We need Insert(ApndR) or similar. But apndl/apndr don't concat.
        // For now, nested violations are acceptable (each f produces its own seq).
        // The decode_violations function handles nested seqs.
        Func::apply_to_all(check_f),
        Func::compose(Func::DistR, Func::construction(vec![facts.clone(), facts])),
    )
}

/// TR: xRy  AND  yRz -> xRz -- violation when transitive chain completion is missing.
fn compile_ring_transitive_ast(def: &ConstraintDef) -> Func {
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();
    let facts = extract_facts_multi(&ft_ids);

    // TR: same chain pattern as IT, but violation when shortcut is MISSING.
    let is_chain = Func::compose(Func::And, Func::construction(vec![
        Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(role_value(1), Func::Selector(1)),
            Func::compose(role_value(0), Func::Selector(2)),
        ])),
        Func::compose(Func::Not, Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(role_value(0), Func::Selector(1)),
            Func::compose(role_value(1), Func::Selector(2)),
        ]))),
    ]));

    let shortcut_match = Func::compose(Func::And, Func::construction(vec![
        Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(role_value(0), Func::Selector(2)),
            Func::compose(role_value(0), Func::compose(Func::Selector(1), Func::Selector(1))),
        ])),
        Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(role_value(1), Func::Selector(2)),
            Func::compose(role_value(1), Func::compose(Func::Selector(2), Func::Selector(1))),
        ])),
    ]));

    // NullTest = shortcut missing = violation (opposite of IT)
    let no_shortcut = Func::compose(
        Func::NullTest,
        Func::compose(Func::filter(shortcut_match), Func::DistL),
    );

    let find_chains_for_f = Func::compose(Func::filter(is_chain), Func::DistL);

    let detail = Func::construction(vec![
        Func::constant(Object::atom("Transitive violation:")),
        Func::compose(role_value(0), Func::compose(Func::Selector(1), Func::Selector(1))),
        Func::constant(Object::atom("relates to")),
        Func::compose(role_value(1), Func::compose(Func::Selector(1), Func::Selector(1))),
        Func::constant(Object::atom("relates to")),
        Func::compose(role_value(1), Func::compose(Func::Selector(2), Func::Selector(1))),
        Func::constant(Object::atom("but shortcut is missing")),
    ]);
    let viol = make_violation_func(&def.id, &def.text, detail);

    let check_f = Func::compose(
        Func::apply_to_all(viol),
        Func::compose(
            Func::filter(no_shortcut),
            Func::compose(Func::DistR, Func::construction(vec![
                find_chains_for_f,
                Func::Selector(2),
            ])),
        ),
    );

    Func::compose(
        Func::apply_to_all(check_f),
        Func::compose(Func::DistR, Func::construction(vec![facts.clone(), facts])),
    )
}

/// AC: no cycle x1Rx2...xnRx1 -- DFS cycle detection.
fn compile_ring_acyclic_ast(def: &ConstraintDef) -> Func {
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();
    let facts = extract_facts_multi(&ft_ids);

    // AC: no cycles in the graph. Detect cycles of ANY depth via
    // transitive closure with the While combinator.
    //
    // Algorithm: starting from the edge set E, repeatedly compute
    // E' = E union {<x,z> | <x,y> in E, <y,z> in E_original}.
    // When E stops growing (no new edges), check for self-loops.
    //
    // State for While: <current_edges, original_edges, prev_count>
    //   - sel(1) = current transitive closure (grows each iteration)
    //   - sel(2) = original edges (constant, used for one-hop extension)
    //   - sel(3) = edge count from previous iteration (for convergence check)
    //
    // Predicate: length(current_edges) != prev_count
    //   (i.e., new edges were added in the last iteration)
    //
    // Body: extend current_edges by one hop, update prev_count.
    //
    // The While combinator has a 1000-iteration safety bound, which is
    // sufficient for any practical population (transitive closure of N
    // nodes stabilizes in at most N-1 iterations).

    // Expressing the full transitive closure fixed-point as pure Func
    // requires While with state = <edges, original, count>. The one-hop
    // extension step needs:
    //   1. Cross product of current_edges x original_edges (via DistR)
    //   2. Filter for chains: role1(e1) = role0(e2)
    //   3. Extract new edges: <role0(e1), role1(e2)>
    //   4. Concat with current_edges (dedup not strictly needed for cycle check)
    //
    // This is expressible but deeply nested. For clarity and maintainability,
    // we use a Native that implements the full algorithm but documents the
    // pure Func equivalent. The Native computes the transitive closure by
    // iterating until convergence, bounded by 1000 iterations.

    let detail = Func::construction(vec![
        Func::constant(Object::atom("Acyclic violation:")),
        Func::constant(Object::atom("cycle detected through")),
        role_value(0),
    ]);
    let viol = make_violation_func(&def.id, &def.text, detail);

    // The transitive closure step is a Native because the pure Func
    // equivalent (While over <edges, originals, count> with DistR-based
    // cross product, Filter for chains, alpha for new edges, Concat for
    // union, and Length for convergence check) is O(n^2) in Func tree
    // depth per iteration. A Native closure is clearer and matches the
    // same semantics:
    //
    //   Pure Func equivalent (pseudocode):
    //     extend_one_hop = concat . [sel(1),
    //       alpha(new_edge) . Filter(is_chain) . concat . alpha(distl) .
    //       distr . [sel(1), sel(2)]]
    //     body = [extend_one_hop, sel(2), length . sel(1)]
    //     pred = not . eq . [length . sel(1), sel(3)]
    //     tc = sel(1) . While(pred, body) . [facts, facts, const(0)]
    //
    // Transitive closure + cycle extraction routed through Platform.
    // Implementation lives in ast::platform_tc_cycles so FPGA and Solidity
    // runtimes can provide their own synthesized/contract implementations.
    let tc_func = Func::Platform("tc_cycles".to_string());

    // Pipeline: extract facts -> compute transitive closure -> report violations
    Func::compose(
        Func::apply_to_all(viol),
        Func::compose(tc_func, facts),
    )
}

/// RF: for each entity x, xRx must exist -- violation when self-reference is missing.
/// Pure Func: set_diff(all_instances, self_refs) then make_violation for each.
fn compile_ring_reflexive_ast(data: &CellIndex, def: &ConstraintDef) -> Func {
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();
    let facts = extract_facts_multi(&ft_ids);

    let id_obj = Object::atom(&def.id);
    let text_obj = Object::atom(&def.text);

    let noun_name: String = def.spans.first()
        .and_then(|s| data.fact_types.get(&s.fact_type_id))
        .and_then(|ft| ft.roles.first())
        .map(|r| r.noun_name.clone())
        .unwrap_or_default();

    // self_refs: instances that DO reference themselves in the ring facts.
    // Filter(eq . [role(0), role(1)]) : facts -> self-referencing facts
    // alpha(role(0)) -> just the values
    let self_refs = Func::compose(
        Func::apply_to_all(role_value(0)),
        Func::compose(
            Func::filter(Func::compose(Func::Eq, Func::construction(vec![role_value(0), role_value(1)]))),
            facts,
        ),
    );

    // all_instances: instances_of_noun_func applied to population (Selector(3) from eval ctx)
    let all_instances = Func::compose(instances_of_noun_func(&noun_name), Func::Selector(3));

    // set_diff: instances NOT in self_refs
    // not_member : <inst, self_refs> -> T if inst not in self_refs
    let not_member = Func::compose(
        Func::NullTest,
        Func::compose(
            Func::filter(Func::compose(Func::Eq, Func::construction(vec![
                Func::compose(Func::Selector(1), Func::Selector(1)),
                Func::Id,
            ]))),
            Func::Selector(2),
        ),
    );
    // distr : <instances, self_refs> -> <<inst1, self_refs>, ...>
    // Filter(not_member) keeps instances not in self_refs
    // alpha(sel(1)) extracts just the instance values
    let missing = Func::compose(
        Func::apply_to_all(Func::Selector(1)),
        Func::compose(
            Func::filter(not_member),
            Func::compose(Func::DistR, Func::construction(vec![all_instances, self_refs])),
        ),
    );

    // For each missing instance, produce violation
    let make_viol = Func::apply_to_all(Func::construction(vec![
        Func::constant(id_obj),
        Func::constant(text_obj),
        Func::construction(vec![
            Func::constant(Object::atom("Reflexive violation:")),
            Func::Id,
            Func::constant(Object::atom("does not reference itself")),
        ]),
    ]));

    Func::compose(make_viol, missing)
}

// -- Alethic Constraint Compilers ----------------------------------
// Each returns a Func that takes an eval context Object -> violations.
// Fact extraction uses extract_facts_func (pure AST).
// Constraint-specific evaluation uses Native where point-free FP
// would be impractical (grouping, counting, set operations).

/// UC: |bu(fact_type, scope_value) : P| <= 1. Violation when > 1.
fn compile_uniqueness_ast(data: &CellIndex, def: &ConstraintDef) -> Func {
    let spans = resolve_spans(data, &def.spans);

    let groups: HashMap<String, Vec<ResolvedSpan>> = spans.iter().fold(HashMap::new(), |mut acc, span| {
        acc.entry(span.fact_type_id.clone()).or_default().push(span.clone());
        acc
    });
    let span_groups: Vec<(String, Vec<ResolvedSpan>)> = groups.into_iter().collect();

    // Pure Func UC: single fact type, any number of spans.
    // Scope = first span's role (the "Each" side). Uniqueness on scope means
    // for each scope value, at most one distinct tuple across the other roles.
    match span_groups.len() {
        1 => {
        let spans_in_group = &span_groups[0].1;
        let facts = extract_facts_func(&span_groups[0].0);
        let scope_idx = spans_in_group[0].role_index;
        // "Other" role: any role not in the scope. For binary, it's the other one.
        let other_idx = spans_in_group.iter()
            .map(|s| s.role_index)
            .find(|&i| i != scope_idx)
            .unwrap_or(if scope_idx == 0 { 1 } else { 0 });

        // same_scope_diff_other on <fact, candidate>
        let dup_check = Func::compose(Func::And, Func::construction(vec![
            Func::compose(Func::Eq, Func::construction(vec![
                Func::compose(role_value(scope_idx), Func::Selector(1)),
                Func::compose(role_value(scope_idx), Func::Selector(2)),
            ])),
            Func::compose(Func::Not, Func::compose(Func::Eq, Func::construction(vec![
                Func::compose(role_value(other_idx), Func::Selector(1)),
                Func::compose(role_value(other_idx), Func::Selector(2)),
            ]))),
        ]));

        // has_any_dup: <fact, all> -> T if scope is duplicated
        let has_any_dup = Func::compose(
            Func::compose(Func::Not, Func::NullTest),
            Func::compose(Func::filter(dup_check), Func::DistL),
        );

        // violating_facts = Filter(has_any_dup) . distr . [facts, facts]
        let violating = Func::compose(
            Func::filter(has_any_dup),
            Func::compose(Func::DistR, Func::construction(vec![facts.clone(), facts])),
        );

        // ONE violation if non-empty (Corollary 2: one per constraint).
        // Detail uses first violating <fact, all> pair.
        let noun = spans_in_group[0].noun_name.clone();
        let reading = spans_in_group[0].reading.clone();
        let detail = Func::construction(vec![
            Func::constant(Object::atom("Uniqueness violation:")),
            Func::constant(Object::atom(&noun)),
            Func::compose(role_value(scope_idx), Func::Selector(1)),
            Func::constant(Object::atom("is not unique in")),
            Func::constant(Object::atom(&reading)),
        ]);
        let viol = make_violation_func(&def.id, &def.text, detail);

        // cond(not.null, viol.sel1, phi) . violating
        return Func::compose(
            Func::condition(
                Func::compose(Func::Not, Func::NullTest),
                Func::construction(vec![Func::compose(viol, Func::Selector(1))]),
                Func::constant(Object::phi()),
            ),
            violating,
        );
        },
        _ => {},
    }

    // Multi-span UC: pure Func per group, then Concat.
    // Each group is checked independently for uniqueness, same as single-FT case.
    let group_checks: Vec<Func> = span_groups.iter().map(|(ft_id, group_spans)| {
        let facts = extract_facts_func(ft_id);

        if group_spans.len() == 1 {
            // Single span in this group: same logic as single-FT UC.
            let scope_idx = group_spans[0].role_index;
            let other_idx = group_spans.iter()
                .map(|s| s.role_index)
                .find(|&i| i != scope_idx)
                .unwrap_or(if scope_idx == 0 { 1 } else { 0 });

            let dup_check = Func::compose(Func::And, Func::construction(vec![
                Func::compose(Func::Eq, Func::construction(vec![
                    Func::compose(role_value(scope_idx), Func::Selector(1)),
                    Func::compose(role_value(scope_idx), Func::Selector(2)),
                ])),
                Func::compose(Func::Not, Func::compose(Func::Eq, Func::construction(vec![
                    Func::compose(role_value(other_idx), Func::Selector(1)),
                    Func::compose(role_value(other_idx), Func::Selector(2)),
                ]))),
            ]));

            let has_any_dup = Func::compose(
                Func::compose(Func::Not, Func::NullTest),
                Func::compose(Func::filter(dup_check), Func::DistL),
            );

            let violating = Func::compose(
                Func::filter(has_any_dup),
                Func::compose(Func::DistR, Func::construction(vec![facts.clone(), facts])),
            );

            let noun = group_spans[0].noun_name.clone();
            let reading = group_spans[0].reading.clone();
            let detail = Func::construction(vec![
                Func::constant(Object::atom("Uniqueness violation:")),
                Func::constant(Object::atom(&noun)),
                Func::compose(role_value(scope_idx), Func::Selector(1)),
                Func::constant(Object::atom("is not unique in")),
                Func::constant(Object::atom(&reading)),
            ]);
            let viol = make_violation_func(&def.id, &def.text, detail);

            Func::compose(
                Func::condition(
                    Func::compose(Func::Not, Func::NullTest),
                    Func::construction(vec![Func::compose(viol, Func::Selector(1))]),
                    Func::constant(Object::phi()),
                ),
                violating,
            )
        } else {
            // Multi-span in one FT group: composite scope key.
            // Two facts are "same scope" if ALL constrained roles match,
            // and are "duplicates" if they are not fully identical.
            // scope_eq: <f1, f2> -> all constrained roles equal
            let scope_eqs: Vec<Func> = group_spans.iter().map(|s| {
                Func::compose(Func::Eq, Func::construction(vec![
                    Func::compose(role_value(s.role_index), Func::Selector(1)),
                    Func::compose(role_value(s.role_index), Func::Selector(2)),
                ]))
            }).collect();
            let scope_eq = if scope_eqs.len() == 1 {
                scope_eqs.into_iter().next().unwrap()
            } else {
                scope_eqs.into_iter().reduce(|a, b| {
                    Func::compose(Func::And, Func::construction(vec![a, b]))
                }).unwrap()
            };

            // not_identical: facts differ in at least one role (entire fact differs)
            let not_identical = Func::compose(Func::Not, Func::compose(Func::Eq, Func::construction(vec![
                Func::Selector(1),
                Func::Selector(2),
            ])));

            let dup_check = Func::compose(Func::And, Func::construction(vec![
                scope_eq,
                not_identical,
            ]));

            let has_any_dup = Func::compose(
                Func::compose(Func::Not, Func::NullTest),
                Func::compose(Func::filter(dup_check), Func::DistL),
            );

            let violating = Func::compose(
                Func::filter(has_any_dup),
                Func::compose(Func::DistR, Func::construction(vec![facts.clone(), facts])),
            );

            let label = group_spans.iter().map(|s| s.noun_name.as_str()).collect::<Vec<_>>().join(", ");
            let reading = group_spans[0].reading.clone();
            // Detail: extract the composite scope values from the first violating fact.
            let mut detail_parts: Vec<Func> = vec![
                Func::constant(Object::atom("Uniqueness violation:")),
                Func::constant(Object::atom(&format!("({})", label))),
            ];
            // Show each scope role value from the violating fact (Sel(1) of <fact, all> pair)
            detail_parts.extend(group_spans.iter().map(|s| Func::compose(role_value(s.role_index), Func::Selector(1))));
            detail_parts.push(Func::constant(Object::atom("is not unique in")));
            detail_parts.push(Func::constant(Object::atom(&reading)));
            let detail = Func::construction(detail_parts);
            let viol = make_violation_func(&def.id, &def.text, detail);

            Func::compose(
                Func::condition(
                    Func::compose(Func::Not, Func::NullTest),
                    Func::construction(vec![Func::compose(viol, Func::Selector(1))]),
                    Func::constant(Object::phi()),
                ),
                violating,
            )
        }
    }).collect();

    match group_checks.len() {
        0 => Func::constant(Object::phi()),
        1 => group_checks.into_iter().next().unwrap(),
        _ => Func::compose(Func::Concat, Func::construction(group_checks)),
    }
}

/// MC: Mandatory constraint.
/// For each entity instance of the constrained noun, check it participates
/// in the required fact type.
fn compile_mandatory_ast(data: &CellIndex, def: &ConstraintDef) -> Func {
    let spans = resolve_spans(data, &def.spans);

    // Build a pure Func check per span, then Concat to flatten.
    let span_checks: Vec<Func> = spans.iter().map(|span| {
        let noun_name = &span.noun_name;
        let reading = &span.reading;

        // instances of this noun from the eval context population
        // instances_of_noun_func(noun) : pop -> <val1, val2, ...>
        // Compose with Selector(3) to extract population from ctx.
        let instances = Func::compose(instances_of_noun_func(noun_name), Func::Selector(3));

        // facts of the constrained fact type from eval context
        let ft_facts = extract_facts_func(&span.fact_type_id);

        // binding_match: <instance, <noun, val>> -> T if noun == noun_name AND val == instance
        let noun_const = Func::constant(Object::atom(noun_name));
        let binding_match = Func::compose(Func::And, Func::construction(vec![
            Func::compose(Func::Eq, Func::construction(vec![
                Func::compose(Func::Selector(1), Func::Selector(2)),  // noun from binding
                noun_const,
            ])),
            Func::compose(Func::Eq, Func::construction(vec![
                Func::compose(Func::Selector(2), Func::Selector(2)),  // val from binding
                Func::Selector(1),                                     // instance from outer pair
            ])),
        ]));

        // fact_mentions: <instance, fact> -> T if fact has binding <noun, instance>
        // DistL on <instance, fact_bindings> -> <<instance, binding1>, ...>
        // Filter(binding_match) keeps matches
        // not . NullTest -> T if any match found
        let fact_mentions = Func::compose(
            Func::compose(Func::Not, Func::NullTest),
            Func::compose(Func::filter(binding_match), Func::DistL),
        );

        // not_participating: <instance, all_facts> -> T when NO fact mentions instance
        // DistL on <instance, all_facts> -> <<instance, fact1>, <instance, fact2>, ...>
        // Filter(fact_mentions) keeps facts that mention the instance
        // NullTest -> T if empty (no fact mentions instance)
        let not_participating = Func::compose(
            Func::NullTest,
            Func::compose(Func::filter(fact_mentions), Func::DistL),
        );

        // detail: <instance, all_facts> -> violation detail sequence
        let mc_label = String::from("Mandatory violation:");
        let not_in_msg = String::from("does not participate in");
        let detail = Func::construction(vec![
            Func::constant(Object::atom(&mc_label)),
            Func::constant(Object::atom(noun_name)),
            Func::Selector(1),  // the instance value
            Func::constant(Object::atom(&not_in_msg)),
            Func::constant(Object::atom(reading)),
        ]);
        let viol = make_violation_func(&def.id, &def.text, detail);

        // apply_to_all(viol) . Filter(not_participating) . DistR . [instances, ft_facts]
        // DistR on <instances, ft_facts> -> <<inst1, ft_facts>, <inst2, ft_facts>, ...>
        Func::compose(
            Func::apply_to_all(viol),
            Func::compose(
                Func::filter(not_participating),
                Func::compose(Func::DistR, Func::construction(vec![instances, ft_facts])),
            ),
        )
    }).collect();

    match span_checks.len() {
        0 => Func::constant(Object::phi()),
        1 => span_checks.into_iter().next().unwrap(),
        _ => Func::compose(Func::Concat, Func::construction(span_checks)),
    }
}

/// FC: Frequency constraint -- each value in the constrained role must occur
/// within [min_occurrence, max_occurrence] times in the fact type's population.
/// Per Halpin Ch 7.2: generalizes UC (FC with max=1 is a UC).
fn compile_frequency_ast(data: &CellIndex, def: &ConstraintDef) -> Func {
    let spans = resolve_spans(data, &def.spans);
    let min_occ = def.min_occurrence.unwrap_or(1);
    let max_occ = def.max_occurrence;

    let range_str = match max_occ {
        Some(max) if max == min_occ => format!("exactly {}", min_occ),
        Some(max) => format!("between {} and {}", min_occ, max),
        None => format!("at least {}", min_occ),
    };

    // Build a pure Func check per span, then Concat to flatten.
    let span_checks: Vec<Func> = spans.iter().map(|span| {
        let facts = extract_facts_func(&span.fact_type_id);
        let scope_idx = span.role_index;

        // same_scope: <fact, candidate> -> T if scope values match
        let same_scope = Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(role_value(scope_idx), Func::Selector(1)),
            Func::compose(role_value(scope_idx), Func::Selector(2)),
        ]));

        // For <fact, all_facts>: DistL gives <<fact, f1>, <fact, f2>, ...>
        // Filter(same_scope) keeps pairs where scope matches
        let same_scope_facts = Func::compose(
            Func::filter(same_scope),
            Func::DistL,
        );

        // Build tl^n: compose Tail n times. tl^0 = Id.
        // too_few: count < min  =>  null . tl^(min-1) . same_scope_facts
        // too_many: count > max =>  not . null . tl^max . same_scope_facts
        let tl_n = |n: usize| -> Func {
            (0..n).fold(Func::Id, |acc, _| Func::compose(Func::Tail, acc))
        };

        let too_few = if min_occ <= 1 {
            // count < 1 means count == 0 => NullTest . same_scope_facts
            Func::compose(Func::NullTest, same_scope_facts.clone())
        } else {
            // null . tl^(min-1) . same_scope_facts
            Func::compose(
                Func::NullTest,
                Func::compose(tl_n(min_occ - 1), same_scope_facts.clone()),
            )
        };

        let violates = match max_occ {
            Some(max) => {
                // too_many: not . null . tl^max . same_scope_facts
                let too_many = Func::compose(
                    Func::compose(Func::Not, Func::NullTest),
                    Func::compose(tl_n(max), same_scope_facts),
                );
                Func::compose(Func::Or, Func::construction(vec![too_few, too_many]))
            }
            None => too_few, // only min bound
        };

        // violating_facts = Filter(violates) . DistR . [facts, facts]
        let violating = Func::compose(
            Func::filter(violates),
            Func::compose(Func::DistR, Func::construction(vec![facts.clone(), facts])),
        );

        let detail = Func::construction(vec![
            Func::constant(Object::atom("Frequency violation:")),
            Func::constant(Object::atom(&span.noun_name)),
            Func::compose(role_value(scope_idx), Func::Selector(1)),
            Func::constant(Object::atom("in")),
            Func::constant(Object::atom(&span.reading)),
            Func::constant(Object::atom(&format!("expected {}", range_str))),
        ]);
        let viol = make_violation_func(&def.id, &def.text, detail);

        // ONE violation per violating scope value (take first).
        Func::compose(
            Func::condition(
                Func::compose(Func::Not, Func::NullTest),
                Func::construction(vec![Func::compose(viol, Func::Selector(1))]),
                Func::constant(Object::phi()),
            ),
            violating,
        )
    }).collect();

    match span_checks.len() {
        0 => Func::constant(Object::phi()),
        1 => span_checks.into_iter().next().unwrap(),
        _ => Func::compose(Func::Concat, Func::construction(span_checks)),
    }
}

/// VC: Value constraint -- each value in the constrained role must be in the
/// noun's allowed value set (enum_values). Per Halpin Ch 6.3.
fn compile_value_constraint_ast(data: &CellIndex, def: &ConstraintDef) -> Func {
    // Collect allowed values from the nouns in the spanned fact types
    let spans = resolve_spans(data, &def.spans);
    let allowed: Vec<(String, HashSet<String>)> = spans.iter().filter_map(|span| {
        let vals = data.enum_values.get(&span.noun_name).filter(|v| !v.is_empty())?;
        Some((span.noun_name.clone(), vals.iter().cloned().collect::<HashSet<_>>()))
    }).collect();

    // If no enum values found on spanned nouns, check all nouns with enum_values
    let check_nouns: Vec<(String, HashSet<String>)> = if !allowed.is_empty() {
        allowed
    } else {
        data.enum_values.iter().filter_map(|(name, vals)| {
            (!vals.is_empty()).then_some(())?;
            Some((name.clone(), vals.iter().cloned().collect::<HashSet<_>>()))
        }).collect()
    };

    // Build a pure Func check per noun, then Concat to flatten.
    let mut noun_checks: Vec<Func> = Vec::new();

    for (noun_name, valid_values) in &check_nouns {
        // All instances of this noun from eval context population
        let instances = Func::compose(instances_of_noun_func(noun_name), Func::Selector(3));

        // Allowed values as a constant sequence (compile-time known)
        let allowed_atoms: Vec<Object> = valid_values.iter()
            .map(|v| Object::atom(v))
            .collect();
        let allowed_const = Func::constant(Object::Seq(allowed_atoms.into()));

        // is_allowed: <instance, allowed_seq> -> T if instance is in allowed_seq
        // DistL on <instance, <v1, v2, ...>> -> <<instance, v1>, <instance, v2>, ...>
        // Filter(Eq) keeps pairs where instance == vi
        // NullTest -> T if no match (instance NOT in allowed set)
        let not_allowed = Func::compose(
            Func::NullTest,
            Func::compose(Func::filter(Func::Eq), Func::DistL),
        );

        // detail: <instance, allowed_seq> -> violation detail
        let valid_str = valid_values.iter().cloned().collect::<Vec<_>>().join(", ");
        let vc_label = String::from("Value constraint violation:");
        let not_in_msg = String::from("is not in");
        let valid_set_str = format!("{{{}}}", valid_str);
        let detail = Func::construction(vec![
            Func::constant(Object::atom(&vc_label)),
            Func::constant(Object::atom(noun_name)),
            Func::Selector(1),  // the instance value
            Func::constant(Object::atom(&not_in_msg)),
            Func::constant(Object::atom(&valid_set_str)),
        ]);
        let viol = make_violation_func(&def.id, &def.text, detail);

        // apply_to_all(viol) . Filter(not_allowed) . DistR . [instances, allowed_const]
        // DistR on <instances, allowed_const> -> <<inst1, allowed>, <inst2, allowed>, ...>
        let check = Func::compose(
            Func::apply_to_all(viol),
            Func::compose(
                Func::filter(not_allowed),
                Func::compose(Func::DistR, Func::construction(vec![instances, allowed_const])),
            ),
        );

        noun_checks.push(check);
    }

    match noun_checks.len() {
        0 => Func::constant(Object::phi()),
        1 => noun_checks.into_iter().next().unwrap(),
        _ => Func::compose(Func::Concat, Func::construction(noun_checks)),
    }
}

/// XO/XC/OR: Set-comparison constraint -- for each entity instance, count how many
/// of the clause fact types it participates in, and check against the requirement.
fn compile_set_comparison_ast(
    _data: &CellIndex,
    def: &ConstraintDef,
    _violates: fn(usize) -> bool,
    requirement: &'static str,
) -> Func {
    let entity_name = def.entity.clone().unwrap_or_default();
    let clause_ft_ids: Vec<String> = def.spans.iter()
        .map(|s| s.fact_type_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    // All instances of the entity noun from eval context population
    let instances = Func::compose(instances_of_noun_func(&entity_name), Func::Selector(3));

    // For each clause FT, build a participation check: <instance, ctx> -> T/F
    // A fact mentions the entity if any binding has noun == entity_name AND val == instance.
    let entity_const = Func::constant(Object::atom(&entity_name));

    // binding_match : <instance, <noun, val>> -> T if noun == entity AND val == instance
    let binding_match = Func::compose(Func::And, Func::construction(vec![
        Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(Func::Selector(1), Func::Selector(2)),  // noun from binding
            entity_const,
        ])),
        Func::compose(Func::Eq, Func::construction(vec![
            Func::compose(Func::Selector(2), Func::Selector(2)),  // val from binding
            Func::Selector(1),                                     // instance
        ])),
    ]));

    // fact_mentions : <instance, fact> -> T if fact has matching binding
    let fact_mentions = Func::compose(
        Func::compose(Func::Not, Func::NullTest),
        Func::compose(Func::filter(binding_match), Func::DistL),
    );

    // participates_in_ft(ft_id) : <instance, ctx> -> T/F
    let per_ft_checks: Vec<Func> = clause_ft_ids.iter().map(|ft_id| {
        let ft_facts = Func::compose(extract_facts_func(ft_id), Func::Selector(2));
        Func::compose(
            Func::compose(Func::Not, Func::NullTest),
            Func::compose(
                Func::filter(fact_mentions.clone()),
                Func::compose(Func::DistL, Func::construction(vec![Func::Selector(1), ft_facts])),
            ),
        )
    }).collect();

    // [check1, check2, ...] : <inst, ctx> -> <T/F, T/F, ...>
    let all_checks = Func::construction(per_ft_checks);
    // Filter(Id) keeps T values (truthy)
    let participating_seq = Func::compose(Func::filter(Func::Id), all_checks);

    // Violation predicate based on requirement:
    //   "exactly one"  -> n != 1 => null(seq) OR not.null.tl(seq)
    //   "at most one"  -> n > 1  => not.null.tl(seq)
    //   "at least one" -> n < 1  => null(seq)
    let violation_pred = match requirement {
        "exactly one" => Func::compose(Func::Or, Func::construction(vec![
            Func::compose(Func::NullTest, participating_seq.clone()),
            Func::compose(
                Func::compose(Func::Not, Func::NullTest),
                Func::compose(Func::Tail, participating_seq),
            ),
        ])),
        "at most one" => Func::compose(
            Func::compose(Func::Not, Func::NullTest),
            Func::compose(Func::Tail, participating_seq),
        ),
        "at least one" => Func::compose(Func::NullTest, participating_seq),
        _ => Func::constant(Object::f()),
    };

    let clause_count = clause_ft_ids.len();
    let detail = Func::construction(vec![
        Func::constant(Object::atom("Set-comparison violation:")),
        Func::constant(Object::atom(&entity_name)),
        Func::Selector(1), // instance value
        Func::constant(Object::atom(&format!("expected {}", requirement))),
        Func::constant(Object::atom(&format!("of {} clause fact types", clause_count))),
    ]);
    let viol = make_violation_func(&def.id, &def.text, detail);

    // DistR . [instances, Id] : ctx -> <<inst1, ctx>, <inst2, ctx>, ...>
    let inst_ctx_pairs = Func::compose(
        Func::DistR,
        Func::construction(vec![instances, Func::Id]),
    );

    Func::compose(
        Func::apply_to_all(viol),
        Func::compose(Func::filter(violation_pred), inst_ctx_pairs),
    )
}

/// SS: Subset constraint -- pop(rs1) subset_of pop(rs2).
/// For join-path subsets, checks that every tuple in fact type A
/// also exists in fact type B, matching by common noun names.
fn compile_subset_ast(data: &CellIndex, def: &ConstraintDef) -> Func {
    match def.spans.len() {
        0 | 1 => return Func::constant(Object::phi()),
        _ => {},
    }

    let a_ft_id = def.spans[0].fact_type_id.clone();
    let b_ft_id = def.spans[1].fact_type_id.clone();

    let a_nouns: Vec<String> = data.fact_types.get(&a_ft_id)
        .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
        .unwrap_or_default();
    let b_nouns: Vec<String> = data.fact_types.get(&b_ft_id)
        .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
        .unwrap_or_default();

    // Compile-time: find common nouns and their role indices in A and B.
    let common: Vec<(usize, usize)> = a_nouns.iter().enumerate().filter_map(|(ai, n)| {
        b_nouns.iter().position(|bn| bn == n).map(|bi| (ai, bi))
    }).collect();

    let a_facts = extract_facts_func(&a_ft_id);
    let b_facts = extract_facts_func(&b_ft_id);

    // match_pred: <a_fact, b_candidate> -> common noun values all equal
    // For each common noun: Eq . [role_value(a_idx) . Sel(1), role_value(b_idx) . Sel(2)]
    let match_pred = if common.is_empty() {
        // No common nouns: every a_fact trivially matches (no violation possible)
        Func::constant(Object::t())
    } else {
        let eqs: Vec<Func> = common.iter().map(|&(ai, bi)| {
            Func::compose(Func::Eq, Func::construction(vec![
                Func::compose(role_value(ai), Func::Selector(1)),
                Func::compose(role_value(bi), Func::Selector(2)),
            ]))
        }).collect();
        if eqs.len() == 1 {
            eqs.into_iter().next().unwrap()
        } else {
            eqs.into_iter().reduce(|acc, eq| {
                Func::compose(Func::And, Func::construction(vec![acc, eq]))
            }).unwrap()
        }
    };

    // not_in_b: <a_fact, b_facts> -> T when no b_candidate matches a_fact
    // NullTest . Filter(match_pred) . DistL
    let not_in_b = Func::compose(
        Func::NullTest,
        Func::compose(Func::filter(match_pred), Func::DistL),
    );

    // detail: <a_fact, b_facts> -> violation description sequence
    // Include each common noun name and its value from a_fact.
    let mut detail_parts: Vec<Func> = core::iter::once(Func::constant(Object::atom("Subset violation:")))
        .chain(common.iter().flat_map(|&(ai, _)| [
            Func::constant(Object::atom(&a_nouns[ai])),
            Func::compose(role_value(ai), Func::Selector(1)),
        ]))
        .collect();
    detail_parts.push(Func::constant(Object::atom("participates in")));
    detail_parts.push(Func::constant(Object::atom(&a_ft_id)));
    detail_parts.push(Func::constant(Object::atom("but not in")));
    detail_parts.push(Func::constant(Object::atom(&b_ft_id)));
    let detail = Func::construction(detail_parts);

    let viol = make_violation_func(&def.id, &def.text, detail);

    // apply_to_all(viol) . Filter(not_in_b) . DistR . [a_facts, b_facts]
    Func::compose(
        Func::apply_to_all(viol),
        Func::compose(
            Func::filter(not_in_b),
            Func::compose(Func::DistR, Func::construction(vec![a_facts, b_facts])),
        ),
    )
}

/// EQ: Equality constraint -- pop(rs1) = pop(rs2) (bidirectional subset).
/// Uses tuple-based comparison same as compile_subset_ast.
fn compile_equality_ast(data: &CellIndex, def: &ConstraintDef) -> Func {
    match def.spans.len() {
        0 | 1 => return Func::constant(Object::phi()),
        _ => {},
    }

    // EQ = SS(A,B) union SS(B,A). Build both subset checks.
    let a_ft_id = def.spans[0].fact_type_id.clone();
    let b_ft_id = def.spans[1].fact_type_id.clone();

    let a_roles: Vec<(usize, String)> = data.fact_types.get(&a_ft_id)
        .map(|ft| ft.roles.iter().enumerate().map(|(i, r)| (i, r.noun_name.clone())).collect())
        .unwrap_or_default();
    let b_roles: Vec<(usize, String)> = data.fact_types.get(&b_ft_id)
        .map(|ft| ft.roles.iter().enumerate().map(|(i, r)| (i, r.noun_name.clone())).collect())
        .unwrap_or_default();

    let common: Vec<(usize, usize)> = a_roles.iter().filter_map(|(ai, n)| {
        b_roles.iter().find(|(_, bn)| bn == n).map(|(bi, _)| (*ai, *bi))
    }).collect();

    match common.is_empty() {
        true => return Func::constant(Object::phi()),
        false => {},
    }

    // Build match predicate for <left_fact, right_candidate>
    let build_match = |left_indices: &[(usize, usize)], swap: bool| -> Func {
        let eqs: Vec<Func> = left_indices.iter().map(|&(ai, bi)| {
            let (li, ri) = if swap { (bi, ai) } else { (ai, bi) };
            Func::compose(Func::Eq, Func::construction(vec![
                Func::compose(role_value(li), Func::Selector(1)),
                Func::compose(role_value(ri), Func::Selector(2)),
            ]))
        }).collect();
        if eqs.len() == 1 { eqs.into_iter().next().unwrap() }
        else { eqs.into_iter().reduce(|a, b| Func::compose(Func::And, Func::construction(vec![a, b]))).unwrap() }
    };

    let a_facts = extract_facts_func(&a_ft_id);
    let b_facts = extract_facts_func(&b_ft_id);

    // A not in B
    let match_ab = build_match(&common, false);
    let not_in_b = Func::compose(Func::NullTest, Func::compose(Func::filter(match_ab), Func::DistL));
    let detail_ab: Vec<Func> = core::iter::once(Func::constant(Object::atom("Equality violation:")))
        .chain(common.iter().flat_map(|&(ai, _)| [
            Func::constant(Object::atom(&a_roles[ai].1)),
            Func::compose(role_value(ai), Func::Selector(1)),
        ]))
        .chain([Func::constant(Object::atom("in")), Func::constant(Object::atom(&a_ft_id)),
                Func::constant(Object::atom("but not in")), Func::constant(Object::atom(&b_ft_id))])
        .collect();
    let viol_ab = make_violation_func(&def.id, &def.text, Func::construction(detail_ab));
    let check_ab = Func::compose(
        Func::apply_to_all(viol_ab),
        Func::compose(Func::filter(not_in_b), Func::compose(Func::DistR, Func::construction(vec![a_facts.clone(), b_facts.clone()]))),
    );

    // B not in A
    let match_ba = build_match(&common, true);
    let not_in_a = Func::compose(Func::NullTest, Func::compose(Func::filter(match_ba), Func::DistL));
    let detail_ba: Vec<Func> = core::iter::once(Func::constant(Object::atom("Equality violation:")))
        .chain(common.iter().flat_map(|&(_, bi)| [
            Func::constant(Object::atom(&b_roles[bi].1)),
            Func::compose(role_value(bi), Func::Selector(1)),
        ]))
        .chain([Func::constant(Object::atom("in")), Func::constant(Object::atom(&b_ft_id)),
                Func::constant(Object::atom("but not in")), Func::constant(Object::atom(&a_ft_id))])
        .collect();
    let viol_ba = make_violation_func(&def.id, &def.text, Func::construction(detail_ba));
    let check_ba = Func::compose(
        Func::apply_to_all(viol_ba),
        Func::compose(Func::filter(not_in_a), Func::compose(Func::DistR, Func::construction(vec![b_facts, a_facts]))),
    );

    // V = union of both directions (Theorem 3 step 3).
    // Construction produces nested seqs; decode_violations recurses.
    Func::construction(vec![check_ab, check_ba])
}

/// Deontic: Forbidden constraint.
/// Uses Func::Selector(1) for response_text and Func::Selector(2) for sender_identity
/// from the eval context <response_text, sender_identity, population>.
fn compile_forbidden_ast(data: &CellIndex, def: &ConstraintDef) -> Func {
    let forbidden_values = collect_enum_values(data, &def.spans);
    let text_keywords = extract_constraint_keywords(&def.text);
    let is_response_constraint = def.entity.as_ref()
        .map_or(false, |e| e.to_lowercase().contains("response"));

    // response_text = Selector(1) : ctx
    let response = Func::Selector(1);

    // Entity scoping: if not a response constraint and entity is specified,
    // only evaluate when response text contains the entity name.
    let entity_gate = match (&def.entity, is_response_constraint) {
        (Some(entity), false) => {
            // Contains . [response, entity_name] -> T if entity mentioned
            Some(Func::compose(Func::Contains, Func::construction(vec![
                response.clone(),
                Func::constant(Object::atom(entity)),
            ])))
        }
        _ => None,
    };

    // CWA path: forbidden enum values as constant sequence, filter for matches.
    let core = if !forbidden_values.is_empty() {
        // Build constant seq of <noun, value> pairs
        let value_atoms: Vec<Object> = forbidden_values.iter()
            .flat_map(|(noun, vals)| vals.iter().map(move |v| Object::seq(vec![Object::atom(noun), Object::atom(v)])))
            .collect();
        let values_const = Func::constant(Object::Seq(value_atoms.into()));

        // For each <noun, value>, check contains(response, value)
        // distr . [values, response] -> <<noun_val, response>, ...>
        // Filter: contains . [sel2, sel2(sel1)] -- response contains value
        let text_contains_val = Func::compose(Func::Contains, Func::construction(vec![
            Func::Selector(2),                                    // response text
            Func::compose(Func::Selector(2), Func::Selector(1)), // value from pair
        ]));

        let detail = Func::construction(vec![
            Func::constant(Object::atom("Response contains forbidden")),
            Func::compose(Func::Selector(1), Func::Selector(1)), // noun name
            Func::compose(Func::Selector(2), Func::Selector(1)), // value
        ]);
        let viol = make_violation_func(&def.id, &def.text, detail);

        Func::compose(
            Func::apply_to_all(viol),
            Func::compose(
                Func::filter(text_contains_val),
                Func::compose(Func::DistR, Func::construction(vec![values_const, response.clone()])),
            ),
        )
    } else if !text_keywords.is_empty() {
        // OWA path: keyword co-occurrence. Build constant keyword seq,
        // filter for matches, check count threshold.
        let kw_atoms: Vec<Object> = text_keywords.iter().map(|k| Object::atom(k)).collect();
        let threshold = text_keywords.len() / 2;
        let kws_const = Func::constant(Object::Seq(kw_atoms.into()));

        // Filter keywords that appear in response
        let kw_in_response = Func::compose(Func::Contains, Func::construction(vec![
            Func::Selector(2), // response text
            Func::Selector(1), // keyword
        ]));

        let matched_kws = Func::compose(
            Func::filter(kw_in_response),
            Func::compose(Func::DistR, Func::construction(vec![kws_const, response.clone()])),
        );

        // Violation if matched count > threshold and >= 2
        let detail = Func::construction(vec![
            Func::constant(Object::atom("Response may violate:")),
            Func::constant(Object::atom(&def.text)),
        ]);
        let viol = make_violation_func(&def.id, &def.text, detail);

        // Condition: length(matched) > threshold -> violation.
        // "Majority of keywords present" interpretation: threshold = n/2, so
        // more than n/2 matches is a probable violation. Uses Func::Gt
        // (added after the original Eq-against-0 placeholder).
        Func::condition(
            Func::compose(Func::Gt, Func::construction(vec![
                Func::compose(Func::Length, matched_kws.clone()),
                Func::constant(Object::atom(&threshold.to_string())),
            ])),
            Func::construction(vec![Func::compose(viol, Func::constant(Object::phi()))]),
            Func::constant(Object::phi()),
        )
    } else {
        Func::constant(Object::phi())
    };

    // Apply entity gate if present
    match entity_gate {
        Some(gate) => Func::condition(gate, core, Func::constant(Object::phi())),
        None => core,
    }
}

/// Deontic: Obligatory constraint.
/// Uses Func::Selector(1) for response_text and Func::Selector(2) for sender_identity
/// from the eval context <response_text, sender_identity, population>.
fn compile_obligatory_ast(data: &CellIndex, def: &ConstraintDef) -> Func {
    let obligatory_values = collect_enum_values(data, &def.spans);
    let checks_sender = def.text.to_lowercase().contains("senderidentity");
    let is_response_constraint = def.entity.as_ref()
        .map_or(false, |e| e.to_lowercase().contains("response"));

    let response = Func::Selector(1);

    // Entity gate (same as forbidden)
    let entity_gate = match (&def.entity, is_response_constraint) {
        (Some(entity), false) => Some(Func::compose(Func::Contains, Func::construction(vec![
            response.clone(), Func::constant(Object::atom(entity)),
        ]))),
        _ => None,
    };

    // Build checks for each noun's obligatory values.
    // For each (noun, values): filter values contained in response.
    // If filter result is empty (NullTest), violation.
    // Î±(noun_values â†’ condition) : obligatory_values
    let noun_checks: Vec<Func> = obligatory_values.iter().map(|(noun_name, enum_vals)| {
        let val_atoms: Vec<Object> = enum_vals.iter().map(|v| Object::atom(v)).collect();
        let vals_const = Func::constant(Object::Seq(val_atoms.into()));

        // Filter values found in response: contains . [response, value]
        let val_in_response = Func::compose(Func::Contains, Func::construction(vec![
            Func::Selector(2), // response text (from distr pair)
            Func::Selector(1), // value atom
        ]));

        let found_any = Func::compose(
            Func::compose(Func::Not, Func::NullTest),
            Func::compose(
                Func::filter(val_in_response),
                Func::compose(Func::DistR, Func::construction(vec![vals_const, response.clone()])),
            ),
        );

        // Condition: if NOT found_any -> violation
        let detail = Func::construction(vec![
            Func::constant(Object::atom("Response missing obligatory")),
            Func::constant(Object::atom(noun_name)),
        ]);
        let viol = make_violation_func(&def.id, &def.text, detail);

        Func::condition(
            found_any,
            Func::constant(Object::phi()),
            Func::construction(vec![Func::compose(viol, Func::constant(Object::phi()))]),
        )
    }).collect();

    // Sender identity check: NullTest . Selector(2)
    // Use .then() to conditionally produce a check â€” pure Backus cond without side effects.
    let sender_check: Option<Func> = checks_sender.then(|| {
        let sender_detail = Func::construction(vec![
            Func::constant(Object::atom("Response missing obligatory SenderIdentity")),
        ]);
        let sender_viol = make_violation_func(&def.id, &def.text, sender_detail);

        // Sender empty = Eq . [Selector(2), ""] OR NullTest . Selector(2)
        let sender_empty = Func::compose(Func::Or, Func::construction(vec![
            Func::compose(Func::Eq, Func::construction(vec![
                Func::Selector(2),
                Func::constant(Object::atom("")),
            ])),
            Func::compose(Func::NullTest, Func::Selector(2)),
        ]));

        Func::condition(
            sender_empty,
            Func::construction(vec![Func::compose(sender_viol, Func::constant(Object::phi()))]),
            Func::constant(Object::phi()),
        )
    });

    let checks: Vec<Func> = noun_checks.into_iter().chain(sender_check).collect();

    let core = match checks.len() {
        0 => Func::constant(Object::phi()),
        1 => checks.into_iter().next().unwrap(),
        _ => Func::construction(checks), // nested, decode_violations handles
    };

    match entity_gate {
        Some(gate) => Func::condition(gate, core, Func::constant(Object::phi())),
        None => core,
    }
}

/// Extract lowercase keywords from a deontic constraint text.
/// Strips the "It is forbidden/obligatory/permitted that" prefix,
/// then extracts PascalCase and multi-word noun phrases.
fn extract_constraint_keywords(text: &str) -> Vec<String> {
    let stripped = text
        .replace("It is forbidden that ", "")
        .replace("It is obligatory that ", "")
        .replace("It is permitted that ", "");

    // Î±(word â†’ pascal_split â†’ filter(len>2)) : words
    let mut keywords: Vec<String> = stripped.split_whitespace()
        .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|clean| !clean.is_empty())
        .flat_map(|clean| {
            // PascalCase split via fold: accumulate chars, emit on uppercase boundary
            let (parts, last) = clean.chars().fold((Vec::new(), String::new()), |(mut parts, mut cur), ch| {
                // At uppercase boundary (with non-empty accumulator), flush cur into parts
                (ch.is_uppercase() && !cur.is_empty())
                    .then(|| parts.push(core::mem::take(&mut cur)));
                cur.push(ch);
                (parts, cur)
            });
            parts.into_iter().chain(core::iter::once(last))
                .map(|s| s.to_lowercase())
                .filter(|s| s.len() > 2)
        })
        .collect();

    // Deduplicate
    keywords.sort();
    keywords.dedup();
    // Filter out common stop words
    keywords.retain(|w| !matches!(w.as_str(), "the" | "that" | "for" | "and" | "with" | "without" | "using" | "has" | "have" | "into" | "from"));
    keywords
}

// -- State Machine Compilation --------------------------------------
// State machines compile to transition functions.
// run_machine = fold(transition)(initial)(stream)

fn compile_state_machine(
    def: &StateMachineDef,
    constraints: &[CompiledConstraint],
) -> CompiledStateMachine {
    // Build constraint ID -> func index for guard lookup
    let constraint_by_id: HashMap<&str, &crate::ast::Func> = constraints.iter()
        .map(|c| (c.id.as_str(), &c.func))
        .collect();

    // s_0 for the fold (Thm 3, §5.1). Comes from the explicit
    // `Status 'X' is initial in SM 'Y'` declaration, or from unique
    // source-never-target graph inference when no declaration exists.
    // Empty if neither path resolved — compile emits the empty string
    // and the runtime fails explicitly at first SM call. No insertion-
    // order fallback: position in the status list does not make a
    // status initial.
    let initial = if !def.initial.is_empty() {
        def.initial.clone()
    } else {
        let sources: HashSet<&str> = def.transitions.iter().map(|t| t.from.as_str()).collect();
        let targets: HashSet<&str> = def.transitions.iter().map(|t| t.to.as_str()).collect();
        let graph_initials: Vec<&String> = def.statuses.iter()
            .filter(|s| sources.contains(s.as_str()) && !targets.contains(s.as_str()))
            .collect();
        if graph_initials.len() == 1 { graph_initials[0].clone() } else { String::new() }
    };

    // -- Hierarchical composition (Harel statecharts) ----------------
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

    let new_transitions: Vec<ExpandedTransition> = parent_transitions.iter()
        .flat_map(|(to, event, guard)| {
            defined_statuses.iter()
                .filter(|status| !expanded.iter().any(|t| t.from == **status && t.event == *event))
                .map(|status| ExpandedTransition {
                    from: status.to_string(),
                    to: to.clone(),
                    event: event.clone(),
                    guard: guard.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    expanded.extend(new_transitions);

    let transition_table: Vec<(String, String, String)> = expanded.iter()
        .map(|t| (t.from.clone(), t.to.clone(), t.event.clone()))
        .collect();

    // AST: transition function <current_state, event> -> next_state.
    //
    // Without guards:
    //   (eq  .  [id, <from, event>]) -> target; next
    //
    // With guards (guard_passes  AND  match):
    //   (null  .  guard_func  .  ...  AND  eq  .  [id, <from, event>]) -> target; next
    //
    // Guard passes iff the constraint func returns phi (empty = no violations).
    let sm_func = expanded.iter().rev().fold(
        crate::ast::Func::Selector(1), // fallback: return current state
        |sm_func, t| {
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
            // Guard passes iff all constraint funcs produce phi (no violations).
            let pred = if let Some(ref guard) = t.guard {
                let guard_funcs: Vec<&crate::ast::Func> = guard.constraint_ids.iter()
                    .filter_map(|cid| constraint_by_id.get(cid.as_str()).copied())
                    .collect();

                if guard_funcs.is_empty() {
                    match_pred
                } else {
                    // Build: null_test  .  guard_func (returns T if guard produces phi)
                    // For multiple guards: all must pass â€” fold over tail
                    let first_check = crate::ast::Func::compose(
                        crate::ast::Func::NullTest,
                        guard_funcs[0].clone(),
                    );
                    let guard_check = guard_funcs[1..].iter().fold(first_check, |acc, gf| {
                        // AND: both must be true (NullTest returns T/F)
                        let next_check = crate::ast::Func::compose(
                            crate::ast::Func::NullTest,
                            (*gf).clone(),
                        );
                        // Compose as: if guard_passes then check next else F
                        crate::ast::Func::condition(
                            acc,
                            next_check,
                            crate::ast::Func::constant(crate::ast::Object::atom("F")),
                        )
                    });
                    // Final: if guards pass AND state+event match -> fire
                    crate::ast::Func::condition(
                        guard_check,
                        match_pred,
                        crate::ast::Func::constant(crate::ast::Object::atom("F")),
                    )
                }
            } else {
                match_pred
            };

            crate::ast::Func::condition(
                pred,
                crate::ast::Func::constant(crate::ast::Object::atom(&t.to)),
                sm_func,
            )
        },
    );

    CompiledStateMachine {
        noun_name: def.noun_name.clone(),
        statuses: def.statuses.clone(),
        initial,
        transition_table,
        func: sm_func,
    }
}

// -- SQL Dialect DDL Generation ---------------------------------------

#[derive(Clone, Copy)]
enum SqlDialect { Sqlite, PostgreSql, MySql, SqlServer, Oracle, Db2, Standard, ClickHouse }

/// Generate DDL for a table in the given SQL dialect.
fn generate_ddl(table: &crate::rmap::TableDef, dialect: &SqlDialect) -> String {
    let q = |s: &str| match dialect {
        SqlDialect::MySql => format!("`{}`", s),
        SqlDialect::SqlServer => format!("[{}]", s),
        _ => format!("\"{}\"", s),
    };

    let map_type = |base: &str| -> &str {
        match dialect {
            SqlDialect::Sqlite => match base {
                "TEXT" => "TEXT", "INTEGER" => "INTEGER", "REAL" => "REAL",
                "BOOLEAN" => "INTEGER", _ => "TEXT",
            },
            SqlDialect::PostgreSql => match base {
                "TEXT" => "TEXT", "INTEGER" => "INTEGER", "REAL" => "DOUBLE PRECISION",
                "BOOLEAN" => "BOOLEAN", _ => "TEXT",
            },
            SqlDialect::MySql => match base {
                "TEXT" => "VARCHAR(255)", "INTEGER" => "INT", "REAL" => "DOUBLE",
                "BOOLEAN" => "TINYINT(1)", _ => "VARCHAR(255)",
            },
            SqlDialect::SqlServer => match base {
                "TEXT" => "NVARCHAR(255)", "INTEGER" => "INT", "REAL" => "FLOAT",
                "BOOLEAN" => "BIT", _ => "NVARCHAR(255)",
            },
            SqlDialect::Oracle => match base {
                "TEXT" => "VARCHAR2(255)", "INTEGER" => "NUMBER(10)", "REAL" => "NUMBER",
                "BOOLEAN" => "NUMBER(1)", _ => "VARCHAR2(255)",
            },
            SqlDialect::Db2 => match base {
                "TEXT" => "VARCHAR(255)", "INTEGER" => "INTEGER", "REAL" => "DOUBLE",
                "BOOLEAN" => "SMALLINT", _ => "VARCHAR(255)",
            },
            SqlDialect::ClickHouse => match base {
                "TEXT" => "String", "INTEGER" => "Int64", "REAL" => "Float64",
                "BOOLEAN" => "UInt8", _ => "String",
            },
            SqlDialect::Standard => match base {
                "TEXT" => "CHARACTER VARYING(255)", "INTEGER" => "INTEGER", "REAL" => "DOUBLE PRECISION",
                "BOOLEAN" => "BOOLEAN", _ => "CHARACTER VARYING(255)",
            },
        }
    };

    let create_kw = match dialect {
        SqlDialect::ClickHouse => format!("CREATE TABLE IF NOT EXISTS {} (\n", q(&table.name)),
        SqlDialect::Oracle => format!("CREATE TABLE {} (\n", q(&table.name)),
        _ => format!("CREATE TABLE IF NOT EXISTS {} (\n", q(&table.name)),
    };

    let has_pk = !table.primary_key.is_empty();
    let has_checks = table.checks.as_ref().map_or(false, |c| !c.is_empty());
    let has_ucs = table.unique_constraints.as_ref().map_or(false, |u| !u.is_empty());

    let columns: String = table.columns.iter().enumerate().map(|(i, col)| {
        let col_type = map_type(&col.col_type);
        let nullable = if col.nullable {
            match dialect { SqlDialect::ClickHouse => " Nullable", _ => "" }
        } else {
            " NOT NULL"
        };
        let refs = col.references.as_ref()
            .map(|r| match dialect {
                SqlDialect::ClickHouse => String::new(), // no FK in ClickHouse
                _ => format!(" REFERENCES {}", q(r)),
            })
            .unwrap_or_default();
        let trailing = if i < table.columns.len() - 1 || has_pk || has_checks || has_ucs { "," } else { "" };
        format!("  {} {}{}{}{}\n", q(&col.name), col_type, nullable, refs, trailing)
    }).collect();

    let pk = if has_pk {
        let pk_cols: Vec<String> = table.primary_key.iter().map(|c| q(c)).collect();
        let trailing = if has_checks || has_ucs { "," } else { "" };
        format!("  PRIMARY KEY ({}){}\n", pk_cols.join(", "), trailing)
    } else { String::new() };

    let checks: String = match (dialect, &table.checks) {
        (SqlDialect::ClickHouse, _) => String::new(), // no CHECK in ClickHouse
        (_, Some(checks)) => checks.iter().enumerate().map(|(i, check)| {
            let trailing = if i < checks.len() - 1 || has_ucs { "," } else { "" };
            format!("  CHECK ({}){}\n", check, trailing)
        }).collect(),
        _ => String::new(),
    };

    let ucs: String = table.unique_constraints.as_ref().map(|ucs| {
        ucs.iter().enumerate().map(|(i, uc)| {
            let uc_cols: Vec<String> = uc.iter().map(|c| q(c)).collect();
            let trailing = if i < ucs.len() - 1 { "," } else { "" };
            format!("  UNIQUE ({}){}\n", uc_cols.join(", "), trailing)
        }).collect::<String>()
    }).unwrap_or_default();

    let engine = match dialect {
        SqlDialect::ClickHouse => "\nENGINE = MergeTree()\nORDER BY tuple()",
        _ => "",
    };

    format!("{}{}{}{}{});{}", create_kw, columns, pk, checks, ucs, engine)
}

// -- SQL Trigger Generation -------------------------------------------

/// Generate SQL triggers for derivation rules.
/// Each rule with resolved antecedents/consequent compiles to a trigger
/// on each antecedent table that INSERTs into the consequent table.
/// Returns Vec<(trigger_group_name, ddl_string)>.
pub fn generate_derivation_triggers(
    derivation_rules: &[DerivationRuleDef],
    fact_types: &HashMap<String, FactTypeDef>,
    sql_tables: &[crate::rmap::TableDef],
    table_names: &HashSet<String>,
) -> Vec<(String, String)> {
    let mut result = Vec::new();

    for rule in derivation_rules {
        let consequent = &rule.consequent_fact_type_id;
        if consequent.is_empty() || rule.antecedent_fact_type_ids.is_empty() { continue; }

        let consequent_table = crate::rmap::to_snake(consequent);

        // Get consequent columns from RMAP table, or derive from fact type roles.
        let consequent_cols: Vec<String> = sql_tables.iter()
            .find(|t| t.name == consequent_table)
            .map(|t| t.columns.iter().map(|c| c.name.clone()).collect())
            .or_else(|| fact_types.get(consequent).map(|ft|
                ft.roles.iter().map(|r| crate::rmap::to_snake(&r.noun_name)).collect()))
            .unwrap_or_default();
        if consequent_cols.is_empty() { continue; }

        // If RMAP didn't create the table, generate a CREATE TABLE for it.
        if !table_names.contains(&consequent_table) {
            let cols_ddl = consequent_cols.iter()
                .map(|c| format!("\"{}\" TEXT", c))
                .collect::<Vec<_>>()
                .join(", ");
            let unique = consequent_cols.iter()
                .map(|c| format!("\"{}\"", c))
                .collect::<Vec<_>>()
                .join(", ");
            result.push((
                format!("table_{}", consequent_table),
                format!("CREATE TABLE IF NOT EXISTS \"{}\" ({}, UNIQUE({}));", consequent_table, cols_ddl, unique),
            ));
        }

        let mut triggers = Vec::new();
        for (i, ant_ft_id) in rule.antecedent_fact_type_ids.iter().enumerate() {
            let ant_table = crate::rmap::to_snake(ant_ft_id);
            if !table_names.contains(&ant_table) { continue; }

            let ant_ft = match fact_types.get(ant_ft_id) { Some(f) => f, None => continue };
            let cons_ft = match fact_types.get(consequent) { Some(f) => f, None => continue };
            let ant_nouns: Vec<&str> = ant_ft.roles.iter().map(|r| r.noun_name.as_str()).collect();
            let cons_nouns: Vec<&str> = cons_ft.roles.iter().map(|r| r.noun_name.as_str()).collect();

            let other_ants: Vec<&str> = rule.antecedent_fact_type_ids.iter()
                .filter(|id| *id != ant_ft_id)
                .map(|id| id.as_str())
                .collect();

            let mut select_cols = Vec::new();
            let mut join_clauses = Vec::new();
            let mut ok = true;

            for cons_noun in &cons_nouns {
                let col = crate::rmap::to_snake(cons_noun);
                if ant_nouns.contains(cons_noun) {
                    select_cols.push(format!("NEW.\"{}\"", col));
                } else {
                    let joined_ant = other_ants.iter().find(|other_id| {
                        fact_types.get(**other_id)
                            .map_or(false, |ft| ft.roles.iter().any(|r| r.noun_name == *cons_noun))
                    });
                    if let Some(joined_id) = joined_ant {
                        let joined_table = crate::rmap::to_snake(joined_id);
                        select_cols.push(format!("\"{}\".\"{}\"", joined_table, col));
                        if let Some(joined_ft) = fact_types.get(*joined_id) {
                            if let Some(shared) = ant_nouns.iter()
                                .find(|n| joined_ft.roles.iter().any(|r| r.noun_name == **n))
                            {
                                let shared_col = crate::rmap::to_snake(shared);
                                join_clauses.push(format!(
                                    "INNER JOIN \"{}\" ON \"{}\".\"{}\" = NEW.\"{}\"",
                                    joined_table, joined_table, shared_col, shared_col
                                ));
                            } else { ok = false; break; }
                        } else { ok = false; break; }
                    } else { ok = false; break; }
                }
            }
            if !ok { continue; }

            let trigger_name = format!("derive_{}_from_{}_{}", consequent_table, ant_table, i);
            let joins = join_clauses.join(" ");
            let select = select_cols.join(", ");
            let cols = consequent_cols.iter()
                .filter(|c| *c != "id")
                .cloned()
                .collect::<Vec<_>>()
                .join("\", \"");

            let from_clause = if joins.is_empty() {
                String::new()
            } else {
                format!(" FROM (SELECT 1) {}", joins)
            };

            triggers.push(format!(
                "CREATE TRIGGER IF NOT EXISTS \"{}\" AFTER INSERT ON \"{}\" BEGIN INSERT OR IGNORE INTO \"{}\" (\"{}\") SELECT {}{} WHERE 1; END;",
                trigger_name, ant_table, consequent_table, cols, select, from_clause
            ));
        }

        if !triggers.is_empty() {
            result.push((crate::rmap::to_snake(consequent), triggers.join("\n")));
        }
    }

    diag!("  [trigger] {} SQL triggers from {} derivation rules", result.len(), derivation_rules.len());
    result
}

// -- Schema Compilation Tests -----------------------------------------

#[cfg(test)]
mod schema_tests {
    use super::*;
    use crate::ast::{self, Object, fact_from_pairs};

    /// Build Object state with a single fact type + its roles. Emits
    /// FactType + Role cells directly (no Domain intermediate).
    fn make_state_with_fact_type(id: &str, reading: &str, roles: Vec<(&str, usize)>) -> Object {
        let mut cells: HashMap<String, Vec<Object>> = HashMap::new();
        let arity = roles.len().to_string();
        cells.entry("FactType".into()).or_default().push(fact_from_pairs(&[
            ("id", id), ("reading", reading), ("arity", arity.as_str()),
        ]));
        for (name, idx) in &roles {
            let pos = idx.to_string();
            cells.entry("Role".into()).or_default().push(fact_from_pairs(&[
                ("factType", id), ("nounName", *name), ("position", pos.as_str()),
            ]));
        }
        Object::Map(cells.into_iter().map(|(k, v)| (k, Object::Seq(v.into()))).collect())
    }

    #[test]
    fn role_compiles_to_selector() {
        let state = make_state_with_fact_type(
            "ft1", "User has Org Role in Organization",
            vec![("User", 0), ("Org Role", 1), ("Organization", 2)],
        );
        let model = compile(&state);
        let schema = model.schemas.get("ft1").unwrap();

        // Each role becomes a Selector (1-indexed)
        assert_eq!(schema.role_names, vec!["User", "Org Role", "Organization"]);

        // The construction is [Selector(1), Selector(2), Selector(3)]
        let fact = Object::seq(vec![
            Object::atom("alice@example.com"),
            Object::atom("owner"),
            Object::atom("org-123"),
        ]);

        let defs = ast::Object::phi();

        // Apply construction to a fact -- identity (selects each role)
        let result = ast::apply(&schema.construction, &fact, &defs);
        assert_eq!(result, Object::seq(vec![
            Object::atom("alice@example.com"),
            Object::atom("owner"),
            Object::atom("org-123"),
        ]));
    }

    #[test]
    fn selector_extracts_individual_role() {
        let state = make_state_with_fact_type(
            "ft1", "Organization has Name",
            vec![("Organization", 0), ("Name", 1)],
        );
        let model = compile(&state);
        let _schema = model.schemas.get("ft1").unwrap();
        let defs = ast::Object::phi();

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
        // alpha(Selector(2)) over a population extracts role 2 from each fact
        let state = make_state_with_fact_type(
            "ft1", "OrgMembership is for User",
            vec![("OrgMembership", 0), ("User", 1)],
        );
        let model = compile(&state);
        let _schema = model.schemas.get("ft1").unwrap();
        let defs = ast::Object::phi();

        let population = Object::seq(vec![
            Object::seq(vec![Object::atom("mem-1"), Object::atom("alice@example.com")]),
            Object::seq(vec![Object::atom("mem-2"), Object::atom("bob@example.com")]),
            Object::seq(vec![Object::atom("mem-3"), Object::atom("alice@example.com")]),
        ]);

        // Extract all users: alpha(2):population
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
        let defs = ast::Object::phi();

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

    /// Build Object state with one fact type + one UC constraint on role 0.
    fn person_has_name_with_uc() -> Object {
        let mut state = make_state_with_fact_type(
            "ft1", "Person has Name",
            vec![("Person", 0), ("Name", 1)],
        );
        let uc = ConstraintDef {
            id: "uc1".into(), kind: "UC".into(), modality: "Alethic".into(),
            text: "Each Person has at most one Name".into(),
            spans: vec![SpanDef { fact_type_id: "ft1".into(), role_index: 0, subset_autofill: None }],
            ..Default::default()
        };
        if let Object::Map(ref mut m) = state {
            m.insert("Constraint".into(), Object::Seq(vec![
                crate::parse_forml2::constraint_to_fact_test(&uc),
            ].into()));
        }
        state
    }

    #[test]
    fn constraint_func_evaluates_via_ast_apply() {
        // Compile a UC constraint and verify the func field works via ast::apply
        let compile_state = person_has_name_with_uc();
        let model = compile(&compile_state);
        let constraint = &model.constraints[0];

        // Create state WITH a UC violation: Alice has two names
        let mut state = crate::ast::Object::phi();
        state = crate::ast::cell_push("ft1", crate::ast::fact_from_pairs(&[("Person", "Alice"), ("Name", "Alice Smith")]), &state);
        state = crate::ast::cell_push("ft1", crate::ast::fact_from_pairs(&[("Person", "Alice"), ("Name", "Alice Jones")]), &state);

        // Evaluate via AST: apply(func, encoded_context)
        let ctx_obj = crate::ast::encode_eval_context_state("", None, &state);
        let defs = crate::ast::Object::phi();
        let result = crate::ast::apply(&constraint.func, &ctx_obj, &defs);

        // Should return a sequence of violation Objects (not phi)
        let violations = crate::ast::decode_violations(&result);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].constraint_id, "uc1");
        assert!(violations[0].detail.contains("Alice"));
    }

    #[test]
    fn constraint_func_no_violation_returns_phi() {
        let compile_state = person_has_name_with_uc();
        let model = compile(&compile_state);
        let constraint = &model.constraints[0];

        // No violation: each person has exactly one name
        let mut state = crate::ast::Object::phi();
        state = crate::ast::cell_push("ft1", crate::ast::fact_from_pairs(&[("Person", "Alice"), ("Name", "Alice Smith")]), &state);
        state = crate::ast::cell_push("ft1", crate::ast::fact_from_pairs(&[("Person", "Bob"), ("Name", "Bob Jones")]), &state);

        let ctx_obj = crate::ast::encode_eval_context_state("", None, &state);
        let defs = crate::ast::Object::phi();
        let result = crate::ast::apply(&constraint.func, &ctx_obj, &defs);

        let violations = crate::ast::decode_violations(&result);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn schema_reading_preserved() {
        let state = make_state_with_fact_type(
            "ft1", "Domain Change proposes Reading",
            vec![("Domain Change", 0), ("Reading", 1)],
        );
        let model = compile(&state);
        let schema = model.schemas.get("ft1").unwrap();
        assert_eq!(schema.reading, "Domain Change proposes Reading");
    }

    #[test]
    fn project_entity_maps_fields_to_schemas() {
        // Simulate an entity with fields that correspond to compiled schemas.
        // The entity "Customer" has fields "name" and "plan".
        let mut cells: HashMap<String, Vec<Object>> = HashMap::new();
        cells.insert("FactType".into(), vec![
            fact_from_pairs(&[("id", "schema-uuid-1"), ("reading", "Customer has name"), ("arity", "2")]),
            fact_from_pairs(&[("id", "schema-uuid-2"), ("reading", "Customer has plan"), ("arity", "2")]),
        ]);
        cells.insert("Role".into(), vec![
            fact_from_pairs(&[("factType", "schema-uuid-1"), ("nounName", "Customer"), ("position", "0")]),
            fact_from_pairs(&[("factType", "schema-uuid-1"), ("nounName", "name"), ("position", "1")]),
            fact_from_pairs(&[("factType", "schema-uuid-2"), ("nounName", "Customer"), ("position", "0")]),
            fact_from_pairs(&[("factType", "schema-uuid-2"), ("nounName", "plan"), ("position", "1")]),
        ]);
        let state = Object::Map(cells.into_iter().map(|(k, v)| (k, Object::Seq(v.into()))).collect());

        let model = compile(&state);

        // Verify noun_to_fact_types has both schemas for "Customer" at role 0
        let customer_fts = model.noun_index.noun_to_fact_types.get("Customer").unwrap();
        let role0_fts: Vec<&str> = customer_fts.iter()
            .filter(|(_, idx)| *idx == 0)
            .map(|(id, _)| id.as_str())
            .collect();
        assert_eq!(role0_fts.len(), 2, "Customer plays role 0 in 2 schemas");

        // Verify we can map fields to schemas via role_names[1]
        customer_fts.iter()
            .filter(|(_, role_idx)| *role_idx == 0)
            .for_each(|(ft_id, _)| {
                let schema = model.schemas.get(ft_id).unwrap();
                assert_eq!(schema.role_names[0], "Customer");
                // role_names[1] should be "name" or "plan"
                assert!(
                    schema.role_names[1] == "name" || schema.role_names[1] == "plan",
                    "unexpected role_names[1]: {}", schema.role_names[1]
                );
            });
    }

    #[test]
    fn project_entity_unmatched_fields_remain_provisional() {
        // An entity with a field that has no compiled schema should still
        // be projectable (with a provisional string-concatenated ID).
        let state = make_state_with_fact_type(
            "ft1", "Order has total",
            vec![("Order", 0), ("total", 1)],
        );
        let model = compile(&state);

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

    /// When two SMs share a status name (e.g. both Order and
    /// Notification declare "Delivered"), the old Pass 3 heuristic
    /// (`from OR to` in sm.statuses) would misassign the Notification
    /// transition `confirm-delivery (Sent â†’ Delivered)` to the Order SM
    /// because Delivered is in both, pulling `Sent` into Order's statuses
    /// and eventually surfacing Sent as Order's initial status.
    ///
    /// The fix is to require BOTH endpoints in the same SM's declared
    /// (Pass 2) statuses â€” if only one endpoint matches, the heuristic
    /// abstains and looks elsewhere.
    #[test]
    fn sm_transitions_do_not_leak_across_domains_sharing_a_status() {
        use crate::types::GeneralInstanceFact;

        let fact = |subject_noun: &str, subject_value: &str, field_name: &str,
                    object_noun: &str, object_value: &str| GeneralInstanceFact {
            subject_noun: subject_noun.to_string(),
            subject_value: subject_value.to_string(),
            field_name: field_name.to_string(),
            object_noun: object_noun.to_string(),
            object_value: object_value.to_string(),
        };

        let facts = vec![
            // Two SMs, each for its own noun
            fact("State Machine Definition", "Order", "is for", "Noun", "Order"),
            fact("State Machine Definition", "Notification", "is for", "Noun", "Notification"),

            // Order SM statuses (declared)
            fact("Status", "Draft", "is defined in", "State Machine Definition", "Order"),
            fact("Status", "Placed", "is defined in", "State Machine Definition", "Order"),
            fact("Status", "Delivered", "is defined in", "State Machine Definition", "Order"),

            // Notification SM statuses (declared) â€” shares "Delivered"
            fact("Status", "Sent", "is defined in", "State Machine Definition", "Notification"),
            fact("Status", "Delivered", "is defined in", "State Machine Definition", "Notification"),

            // Order transitions
            fact("Transition", "place", "is from", "Status", "Draft"),
            fact("Transition", "place", "is to", "Status", "Placed"),
            fact("Transition", "deliver", "is from", "Status", "Placed"),
            fact("Transition", "deliver", "is to", "Status", "Delivered"),

            // Notification transitions
            fact("Transition", "confirm-delivery", "is from", "Status", "Sent"),
            fact("Transition", "confirm-delivery", "is to", "Status", "Delivered"),
        ];

        let machines = derive_state_machines_from_facts(&facts);

        let order = machines.get("Order").expect("Order SM present");
        assert!(!order.statuses.contains(&"Sent".to_string()),
            "Order SM must not contain Notification's 'Sent' status; got {:?}", order.statuses);
        assert!(!order.transitions.iter().any(|t| t.event == "confirm-delivery"),
            "Order SM must not contain Notification's 'confirm-delivery' transition");
        assert_eq!(order.statuses.first().map(String::as_str), Some("Draft"),
            "Order initial must be Draft; got {:?}", order.statuses.first());

        let notif = machines.get("Notification").expect("Notification SM present");
        assert!(notif.transitions.iter().any(|t| t.event == "confirm-delivery"),
            "Notification SM must contain its own 'confirm-delivery' transition");
        assert_eq!(notif.statuses.first().map(String::as_str), Some("Sent"),
            "Notification initial must be Sent; got {:?}", notif.statuses.first());
    }

    /// #214: compile_func applied via ast::apply produces the same
    /// (name, Func) list as the direct Rust call. Pins the FFP entry
    /// point so downstream callers can ρ-dispatch to compile.
    #[test]
    fn compile_func_round_trip_matches_direct_call() {
        let state = make_state_with_fact_type(
            "ft1", "User has Org Role in Organization",
            vec![("User", 0), ("Org Role", 1), ("Organization", 2)],
        );
        let direct = compile_to_defs_state(&state);
        let encoded = crate::ast::apply(&compile_func(), &state, &state);
        let decoded = decode_compile_result(&encoded, &state);

        assert_eq!(direct.len(), decoded.len(),
            "Func-apply must emit the same number of defs as the direct call");
        // Order-independent: family decomposition groups defs by
        // prefix (constraint → validate → machine → … → other), so
        // direct-emission order need not match after partition. What
        // must match is the SET of emitted names.
        let direct_names: std::collections::HashSet<&str> = direct.iter().map(|(n, _)| n.as_str()).collect();
        let decoded_names: std::collections::HashSet<&str> = decoded.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(direct_names, decoded_names,
            "Func-apply must preserve the set of def names");
    }

    /// #214: pin the top-level Func-tree shape so a future refactor
    /// can't quietly regress compile back to a single Native leaf.
    /// The shape must remain `Compose(Concat, Construction([8 family Funcs]))`
    /// matching AREST Table 1 (constraints, validators, machines,
    /// transitions, derivations, schemas, resolvers, other).
    #[test]
    fn compile_func_top_level_is_concat_of_family_construction() {
        let func = compile_func();
        match &func {
            crate::ast::Func::Compose(outer, inner) => {
                assert!(matches!(**outer, crate::ast::Func::Concat),
                    "top level must compose Concat onto the family construction");
                match &**inner {
                    crate::ast::Func::Construction(families) => {
                        assert_eq!(families.len(), 11,
                            "compile_func must expose 11 family leaves (Table 1 + shard + list + get); got {}",
                            families.len());
                    }
                    other => panic!("inner must be Construction of family leaves; got {:?}", other),
                }
            }
            other => panic!("top-level compile_func shape broke: {:?}", other),
        }
    }

    /// #214 deeper lowering: the shard-family leaf calls RMAP's
    /// cell-map helper and emits one `(shard:{ft_id}, Constant(cell))`
    /// per fact type. Verify the standalone compiler matches the
    /// direct call's shard: subset exactly.
    #[test]
    fn compile_shard_family_matches_direct_shard_defs() {
        let state = make_state_with_fact_type(
            "User has Org Role in Organization",
            "User has Org Role in Organization",
            vec![("User", 0), ("Org Role", 1), ("Organization", 2)],
        );
        let via_direct: Vec<_> = compile_to_defs_state(&state).into_iter()
            .filter(|(n, _)| n.starts_with("shard:"))
            .collect();
        let via_family = super::compile_shard_family(&state);
        let direct_names: std::collections::HashSet<&str> =
            via_direct.iter().map(|(n, _)| n.as_str()).collect();
        let family_names: std::collections::HashSet<&str> =
            via_family.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(direct_names, family_names,
            "shard-family compiler must emit the same shard:{{ft_id}} names as the monolithic call");
    }

    /// #214 deeper lowering: the resolve-family leaf reads
    /// Noun + FactType + Role cells directly and emits a
    /// `resolve:{noun}` Condition chain per noun with binary FT
    /// participation. Verify it matches the direct call's resolve
    /// subset exactly.
    #[test]
    fn compile_resolve_family_matches_direct_resolve_defs() {
        // Two nouns, one FT — should yield resolve:User (binary FT
        // with Org Role) but no resolve:Org Role because only one
        // binary FT mentions it and the filter would yield just
        // one entry (which still builds a valid chain). Actually
        // both nouns should get resolve defs since each participates
        // in at least one binary FT. We assert the names match.
        let state = make_state_with_fact_type(
            "User_has_Org Role", "User has Org Role",
            vec![("User", 0), ("Org Role", 1)],
        );
        let via_direct: Vec<_> = compile_to_defs_state(&state).into_iter()
            .filter(|(n, _)| n.starts_with("resolve:"))
            .collect();
        let via_family = super::compile_resolve_family(&state);
        let direct_names: std::collections::HashSet<&str> =
            via_direct.iter().map(|(n, _)| n.as_str()).collect();
        let family_names: std::collections::HashSet<&str> =
            via_family.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(direct_names, family_names,
            "resolve-family compiler must emit the same resolve:{{noun}} names as the monolithic call");
    }

    /// #214 deeper lowering: the schema family is a pure FFP
    /// expression `ApplyToAll(schema_pair_from_ft_native) ∘
    /// FetchOrPhi(<"FactType", state>)`. Verify that running it via
    /// `apply(schema_family_func(), …)` produces the same
    /// `schema:{ft_id}` names the monolithic `compile_to_defs_state`
    /// call would emit for the same state.
    #[test]
    fn schema_family_func_matches_direct_schema_defs() {
        let state = make_state_with_fact_type(
            "User has Org Role in Organization",
            "User has Org Role in Organization",
            vec![("User", 0), ("Org Role", 1), ("Organization", 2)],
        );
        let via_direct: Vec<_> = compile_to_defs_state(&state).into_iter()
            .filter(|(n, _)| n.starts_with("schema:"))
            .collect();

        // Apply the FFP schema family directly, decode pairs.
        let encoded = crate::ast::apply(&super::schema_family_func(), &state, &state);
        let via_family: Vec<(String, crate::ast::Func)> = encoded.as_seq()
            .map(|pairs| pairs.iter().filter_map(|p| {
                let items = p.as_seq()?;
                if items.len() != 2 { return None; }
                let name = items[0].as_atom()?.to_string();
                let func = crate::ast::metacompose(&items[1], &state);
                Some((name, func))
            }).collect())
            .unwrap_or_default();

        let direct_names: std::collections::HashSet<&str> =
            via_direct.iter().map(|(n, _)| n.as_str()).collect();
        let family_names: std::collections::HashSet<&str> =
            via_family.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(direct_names, family_names,
            "schema-family Func must emit the same schema:{{ft_id}} names as the monolithic call");

        // And each Construction must have the same role count.
        for (name, func) in &via_family {
            let direct_func = &via_direct.iter()
                .find(|(n, _)| n == name)
                .expect("schema present in direct call").1;
            match (func, direct_func) {
                (crate::ast::Func::Construction(a), crate::ast::Func::Construction(b)) => {
                    assert_eq!(a.len(), b.len(),
                        "Construction arity must match for {}: family={}, direct={}",
                        name, a.len(), b.len());
                }
                _ => panic!("both sides must be Construction Funcs for {}", name),
            }
        }
    }

    /// Sanity: every family predicate is disjoint except for the
    /// catch-all. A name must belong to exactly ONE family so the
    /// Concat doesn't emit duplicates.
    #[test]
    fn family_tags_partition_def_names() {
        let families = [
            FAMILY_CONSTRAINT, FAMILY_VALIDATE, FAMILY_MACHINE,
            FAMILY_TRANSITIONS, FAMILY_DERIVATION, FAMILY_SCHEMA,
            FAMILY_RESOLVE, FAMILY_SHARD, FAMILY_LIST, FAMILY_GET,
        ];
        let samples = [
            "constraint:uc_1", "validate", "validate:Order_has_Status",
            "machine:Order", "machine:Order:initial",
            "transitions:Order", "derivation:uncle_rule",
            "derivation_index:Order", "schema:ft_23",
            "resolve:Customer", "shard:Order_has_Status",
            "list:Customer", "get:Customer",
            "sql:sqlite:orders", "openapi:my-app",
            "populate:ExternalNoun", "compile", "apply",
        ];
        for name in samples {
            let claimed: Vec<&&str> = families.iter()
                .filter(|f| name_belongs_to_family(name, f))
                .collect();
            assert!(claimed.len() <= 1,
                "def name `{}` matches multiple families: {:?}", name, claimed);
            // Either one specific family claims it, or the catch-all does.
            let other_claims = name_belongs_to_family(name, FAMILY_OTHER);
            assert_eq!(claimed.len() == 0, other_claims,
                "name `{}` must belong to exactly one family (specific or _other)", name);
        }
    }
}
