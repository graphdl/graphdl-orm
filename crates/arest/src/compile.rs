// crates/arest/src/compile.rs
//
// Compilation: Domain -> CompiledModel
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

use std::collections::{HashMap, HashSet};
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
pub(crate) struct CompiledConstraint {
    pub(crate) id: String,
    pub(crate) text: String,
    pub(crate) modality: Modality,
    pub(crate) func: crate::ast::Func,
}


/// A compiled derivation rule. Evaluation is apply(func, population_object) -> derived facts.
pub(crate) struct CompiledDerivation {
    pub(crate) id: String,
    pub(crate) text: String,
    pub(crate) kind: DerivationKind,
    pub(crate) func: crate::ast::Func,
}

/// A compiled state machine. func is the transition function: <state, event> -> state'.
pub(crate) struct CompiledStateMachine {
    pub(crate) noun_name: String,
    pub(crate) statuses: Vec<String>,
    pub(crate) initial: String,
    pub(crate) func: crate::ast::Func,
    pub(crate) transition_table: Vec<(String, String, String)>,
}

/// Index for fast noun lookups during synthesis.
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

/// A compiled graph schema -- a Construction of Selector functions (roles).
/// Graph Schema = CONS(Role1, ..., Rolen) in Backus's FP algebra.
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
pub(crate) struct CompiledModel {
    pub(crate) constraints: Vec<CompiledConstraint>,
    pub(crate) derivations: Vec<CompiledDerivation>,
    pub(crate) state_machines: Vec<CompiledStateMachine>,
    pub(crate) noun_index: NounIndex,
    /// Fact types compiled to Construction functions (CONS of Roles).
    pub(crate) schemas: HashMap<String, CompiledSchema>,
    /// Fact-to-event mapping: when a fact of this type is created, fire this event
    /// on the state machine for the target noun. Derived from:
    ///   Graph Schema is activated by Verb + Verb is performed during Transition.
    pub(crate) fact_events: HashMap<String, FactEvent>,
}

/// When a fact is created in this schema, fire this event on the entity's state machine.
pub(crate) struct FactEvent {
    pub(crate) fact_type_id: String,
    pub(crate) event_name: String,
    pub(crate) target_noun: String, // which noun's state machine to transition
}

// (decode_population_object removed -- no longer needed after eliminating all Func::Native closures)

// -- Schema Compilation -------------------------------------------
// Compile fact types to Construction functions (CONS of Roles).
// Role -> Selector. Graph Schema -> Construction [Selector1, ..., Selectorn].

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
    // sel(3) -> population
    // Filter(eq  .  [sel(1), ft_id]) -> matching fact type entries
    // (null -> phi; sel(2)  .  sel(1)) -> get facts from first match, or phi
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
/// Deduplicates by noun name -- each noun's enum values appear at most once.
fn collect_enum_values(ir: &Domain, spans: &[SpanDef]) -> Vec<(String, Vec<String>)> {
    // α(span → roles) : spans → flat_map → filter(has_enum ∧ ¬seen) → deduplicate
    spans.iter()
        .filter_map(|span| ir.fact_types.get(&span.fact_type_id))
        .flat_map(|ft| ft.roles.iter())
        .filter_map(|role| ir.enum_values.get(&role.noun_name)
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
    let mut machines: HashMap<String, StateMachineDef> = HashMap::new();

    // Pass 1: fold over facts → machines (SM Definition 'X' is for Noun 'Y')
    let mut machines = facts.iter()
        .filter(|f| f.subject_noun == "State Machine Definition" && f.object_noun == "Noun")
        .fold(machines, |mut acc, f| {
            acc.entry(f.subject_value.clone()).or_insert(StateMachineDef {
                noun_name: f.object_value.clone(), statuses: vec![], transitions: vec![],
            });
            acc
        });

    // Pass 2: fold initial statuses (Status 'S' is initial in SM Definition 'X')
    // α(fact → (sm_key, status)) : filtered_facts → for_each(insert_if_absent)
    let status_inserts: Vec<(String, String)> = facts.iter()
        .filter(|f| f.subject_noun == "Status" && f.object_noun == "State Machine Definition")
        .map(|f| (f.object_value.clone(), f.subject_value.clone()))
        .collect();
    status_inserts.into_iter().for_each(|(sm_key, status)| {
        machines.get_mut(&sm_key).into_iter()
            .filter(|sm| !sm.statuses.contains(&status))
            .for_each(|sm| sm.statuses.insert(0, status.clone()));
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

    // Assemble: α(transition_name → add_to_machine) over unique transition names
    t_from.keys().chain(t_to.keys()).collect::<HashSet<_>>().into_iter()
        .filter_map(|t_name| {
            let from = t_from.get(t_name)?.clone();
            let to = t_to.get(t_name)?.clone();
            let event = t_event.get(t_name).cloned().unwrap_or_else(|| t_name.clone());
            let target = t_sm.get(t_name).cloned()
                .or_else(|| machines.iter()
                    .find(|(_, sm)| sm.statuses.contains(&from) || sm.statuses.contains(&to))
                    .map(|(k, _)| k.clone()))
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

    machines
}

// -- Compilation ----------------------------------------------------
// The match on kind happens here, once. After this, everything is Func.

/// Compile an Object state into named FFP definitions.
/// All generators always produce all defs. Selection is at apply time:
/// SYSTEM:sql:sqlite:Order returns DDL, SYSTEM:xsd:Order returns XSD.
pub fn compile_to_defs_state(state: &crate::ast::Object) -> Vec<(String, Func)> {
    let domain = state_to_domain(state);
    let model = compile(&domain);

    // Constraints -> named definitions — α(constraint → def)
    let mut defs: Vec<(String, Func)> = model.constraints.iter()
        .map(|c| (format!("constraint:{}", c.id), c.func.clone()))
        .collect();

    // validate: Concat . [all constraints] -- single Func that returns all violations.
    // Empty constraint set produces phi (no violations). The algebra handles it.
    let all_constraints: Vec<Func> = model.constraints.iter().map(|c| c.func.clone()).collect();
    defs.push(("validate".to_string(), Func::compose(Func::Concat, Func::construction(all_constraints))));

    // State machines -> named definitions — α(sm → <func_def, initial_def>)
    defs.extend(model.state_machines.iter().flat_map(|sm| [
        (format!("machine:{}", sm.noun_name), sm.func.clone()),
        (format!("machine:{}:initial", sm.noun_name), Func::constant(Object::atom(&sm.initial))),
    ]));

    // Transitions: for each SM, register transitions:{noun} that takes a status
    // and returns <<from, to, event>, ...> for available transitions.
    // Uses the machine func and known events to compute available transitions.
    // Transitions + meta — α(sm → <transitions_def, meta_def>)
    defs.extend(model.state_machines.iter().flat_map(|sm| {
        let machine_def_name = format!("machine:{}", sm.noun_name);
        let events: Vec<String> = sm.transition_table.iter().map(|(_, _, e)| e.clone())
            .collect::<std::collections::HashSet<_>>().into_iter().collect();
        // α(event → check_func): for each event, build condition that tests transition
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

    // Derivation rules — α(derivation → def)
    defs.extend(model.derivations.iter()
        .map(|d| (format!("derivation:{}", d.id), d.func.clone())));

    // Fact type schemas — α(schema → def)
    defs.extend(model.schemas.iter()
        .map(|(id, schema)| (format!("schema:{}", id), schema.construction.clone())));

    // resolve:{noun} — Condition chain mapping field_name → fact_type_id.
    // Input: field_name atom. Output: fact_type_id atom.
    // Compiled from NounIndex: for each fact type involving this noun,
    // extract the "other" role's noun name as the field key.
    // resolve:{noun} — α(noun → Condition chain mapping field_name → fact_type_id)
    defs.extend(domain.nouns.keys().filter_map(|noun_name| {
        let field_mappings: Vec<(String, String)> = domain.fact_types.iter()
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

    // HATEOAS navigation links as FFP projections (Theorem 4b).
    // For each binary fact type with a UC, the UC role is the child (dependent),
    // the other role is the parent. Navigation is a constant function returning
    // the related noun names.
    // HATEOAS nav links — fold UC constraints into (children_map, parent_map), then α → defs
    let (children_map, parent_map) = domain.constraints.iter()
        .filter(|c| c.kind == "UC" && !c.spans.is_empty())
        .filter_map(|c| {
            let ft = domain.fact_types.get(&c.spans[0].fact_type_id)?;
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

    // ── Generator 1: Agent Prompts ──────────────────────────────────
    // Build lookup maps via fold — noun → readings, noun → constraints, noun → events
    let noun_fact_types: HashMap<String, Vec<String>> = domain.fact_types.values()
        .flat_map(|ft| ft.roles.iter().map(move |r| (r.noun_name.clone(), ft.reading.clone())))
        .fold(HashMap::new(), |mut m, (noun, reading)| { m.entry(noun).or_default().push(reading); m });

    let ft_ref = &domain.fact_types;
    let noun_constraint_map: HashMap<String, Vec<&ConstraintDef>> = domain.constraints.iter()
        .flat_map(|c| c.spans.iter().filter_map(move |s| {
            ft_ref.get(&s.fact_type_id).map(|ft| (ft, c))
        }))
        .flat_map(|(ft, c)| ft.roles.iter().map(move |r| (r.noun_name.clone(), c)))
        .fold(HashMap::new(), |mut m, (noun, c)| { m.entry(noun).or_default().push(c); m });

    let noun_transitions: HashMap<String, Vec<String>> = model.state_machines.iter()
        .map(|sm| (sm.noun_name.clone(), sm.transition_table.iter()
            .map(|(_, _, e)| e.clone()).collect::<HashSet<_>>().into_iter().collect()))
        .collect();

    // α(noun → agent_def) — filter nouns with readings, map to prompt Object
    let deontic_filter = |cs: &[&ConstraintDef], op: &str| -> Vec<Object> {
        cs.iter().filter(|c| c.modality == "deontic" && c.deontic_operator.as_deref() == Some(op))
            .map(|c| Object::atom(&c.text)).collect()
    };
    let atoms_or_empty = |m: &HashMap<String, Vec<String>>, key: &str| -> Vec<Object> {
        m.get(key).map(|v| v.iter().map(|s| Object::atom(s)).collect()).unwrap_or_default()
    };

    defs.extend(domain.nouns.keys()
        .filter(|n| noun_fact_types.get(*n).map_or(false, |r| !r.is_empty()))
        .map(|noun_name| {
            let cs = noun_constraint_map.get(noun_name).map(|v| v.as_slice()).unwrap_or(&[]);
            let prompt = Object::Seq(vec![
                Object::seq(vec![Object::atom("role"), Object::atom(noun_name)]),
                Object::seq(vec![Object::atom("fact_types"), Object::Seq(atoms_or_empty(&noun_fact_types, noun_name))]),
                Object::seq(vec![Object::atom("constraints"), Object::Seq(cs.iter().map(|c| Object::atom(&c.text)).collect())]),
                Object::seq(vec![Object::atom("transitions"), Object::Seq(atoms_or_empty(&noun_transitions, noun_name))]),
                Object::seq(vec![Object::atom("children"), Object::Seq(
                    children_map.get(noun_name).map(|v| v.iter().map(|c| Object::atom(c)).collect()).unwrap_or_default())]),
                Object::seq(vec![Object::atom("parent"), Object::Seq(
                    parent_map.get(noun_name).map(|v| v.iter().map(|p| Object::atom(p)).collect()).unwrap_or_default())]),
                Object::seq(vec![Object::atom("deontic"), Object::Seq(vec![
                    Object::seq(vec![Object::atom("obligatory"), Object::Seq(deontic_filter(cs, "obligatory"))]),
                    Object::seq(vec![Object::atom("forbidden"), Object::Seq(deontic_filter(cs, "forbidden"))]),
                    Object::seq(vec![Object::atom("permitted"), Object::Seq(deontic_filter(cs, "permitted"))]),
                ])]),
            ]);
            (format!("agent:{}", noun_name), Func::constant(prompt))
        }));

    // Shared helper: constraints spanning a noun (fn, not closure, to avoid move conflicts)
    fn noun_constraints_for<'a>(domain: &'a Domain, noun: &str) -> Vec<&'a ConstraintDef> {
        domain.constraints.iter()
            .filter(|c| c.spans.iter().any(|s| {
                domain.fact_types.get(&s.fact_type_id)
                    .map_or(false, |ft| ft.roles.iter().any(|r| r.noun_name == noun))
            })).collect()
    }

    // ── Generator 2: iLayer — α(noun → ilayer_def)
    defs.extend(domain.nouns.iter().map(|(noun_name, noun_def)| {
        let ft_entries = Object::Seq(domain.fact_types.values()
            .filter(|ft| ft.roles.iter().any(|r| r.noun_name == *noun_name))
            .map(|ft| Object::seq(vec![
                Object::atom(&ft.reading),
                Object::Seq(ft.roles.iter().map(|r| Object::atom(&r.noun_name)).collect()),
            ])).collect());
        let constraint_texts = Object::Seq(noun_constraints_for(&domain, noun_name).iter()
            .map(|c| Object::atom(&c.text)).collect());
        let ref_parts = Object::Seq(domain.ref_schemes.get(noun_name)
            .map(|parts| parts.iter().map(|p| Object::atom(p)).collect()).unwrap_or_default());
        let ilayer = Object::seq(vec![
            Object::seq(vec![Object::atom("object_type"), Object::atom(&noun_def.object_type)]),
            Object::seq(vec![Object::atom("fact_types"), ft_entries]),
            Object::seq(vec![Object::atom("constraints"), constraint_texts]),
            Object::seq(vec![Object::atom("ref_scheme"), ref_parts]),
        ]);
        (format!("ilayer:{}", noun_name), Func::constant(ilayer))
    }));

    // ── Generator 3: SQL DDL (multi-dialect) ─────────────────────────
    // Call rmap() at compile time and produce dialect-specific defs:
    //   sql:sqlite:{table}, sql:postgresql:{table}, sql:mysql:{table},
    //   sql:sqlserver:{table}, sql:oracle:{table}, sql:db2:{table},
    //   sql:standard:{table}, sql:clickhouse:{table}
    // ── Generator 3: SQL DDL — α(table × dialect → def)
    let sql_tables = crate::rmap::rmap(&domain);
    let dialects = [
        ("sqlite", SqlDialect::Sqlite), ("postgresql", SqlDialect::PostgreSql),
        ("mysql", SqlDialect::MySql), ("sqlserver", SqlDialect::SqlServer),
        ("oracle", SqlDialect::Oracle), ("db2", SqlDialect::Db2),
        ("standard", SqlDialect::Standard), ("clickhouse", SqlDialect::ClickHouse),
    ];
    defs.extend(sql_tables.iter().flat_map(|table|
        dialects.iter().map(move |(name, dialect)|
            (format!("sql:{}:{}", name, table.name), Func::constant(Object::atom(&generate_ddl(table, dialect))))
        )
    ));

    // ── Generator 4: Test Harness — α(constraint → test_def)
    defs.extend(domain.constraints.iter().map(|c| {
        let modality_str = match c.modality.as_str() { "deontic" => "deontic", _ => "alethic" };
        (format!("test:{}", c.id), Func::constant(Object::Seq(vec![
            Object::seq(vec![Object::atom("id"), Object::atom(&c.id)]),
            Object::seq(vec![Object::atom("text"), Object::atom(&c.text)]),
            Object::seq(vec![Object::atom("kind"), Object::atom(&c.kind)]),
            Object::seq(vec![Object::atom("modality"), Object::atom(modality_str)]),
        ])))
    }));

    // Handler defs — α(noun → <create_def, update_def>)
    defs.extend(domain.nouns.keys().flat_map(|noun_name| {
        let noun_obj = Object::atom(noun_name);
        [
            (format!("create:{}", noun_name), Func::constant(noun_obj.clone())),
            (format!("update:{}", noun_name), Func::constant(noun_obj)),
        ]
    }));

    // Query defs — α(schema → role_map_def)
    defs.extend(model.schemas.iter().map(|(id, schema)| {
        let role_map = Object::Seq(schema.role_names.iter().enumerate()
            .map(|(i, name)| Object::seq(vec![Object::atom(name), Object::atom(&(i + 1).to_string())]))
            .collect());
        (format!("query:{}", id), Func::constant(role_map))
    }));

    // Helpers as fns (not closures) to avoid borrow conflicts with domain
    fn binary_fts_for<'a>(domain: &'a Domain, noun: &str) -> Vec<&'a FactTypeDef> {
        domain.fact_types.values()
            .filter(|ft| ft.roles.len() == 2 && ft.roles.iter().any(|r| r.noun_name == noun))
            .collect()
    }
    fn other_role_of(ft: &FactTypeDef, noun: &str) -> String {
        ft.roles.iter().find(|r| r.noun_name != noun)
            .map(|r| r.noun_name.clone()).unwrap_or_default()
    }
    // ── Generator 5: XSD — α(noun → xsd_def) ────────────────────────
    defs.extend(domain.nouns.iter().map(|(noun_name, noun_def)| {
        let fields = Object::Seq(binary_fts_for(&domain, noun_name).iter().map(|ft|
            Object::seq(vec![Object::atom(&other_role_of(ft, noun_name)), Object::atom("xs:string")])
        ).collect());
        (format!("xsd:{}", noun_name), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("name"), Object::atom(noun_name)]),
            Object::seq(vec![Object::atom("object_type"), Object::atom(&noun_def.object_type)]),
            Object::seq(vec![Object::atom("elements"), fields]),
        ])))
    }));

    // ── Generator 6: DTD — α(noun → dtd_def) ────────────────────────
    defs.extend(domain.nouns.iter().map(|(noun_name, _)| {
        let children: Vec<String> = binary_fts_for(&domain, noun_name).iter()
            .map(|ft| other_role_of(ft, noun_name).to_string()).collect();
        let child_list = children.join(", ");
        let dtd_text = format!("<!ELEMENT {} ({})>\n{}",
            noun_name,
            if child_list.is_empty() { "#PCDATA".to_string() } else { child_list },
            children.iter().map(|c| format!("<!ELEMENT {} (#PCDATA)>", c)).collect::<Vec<_>>().join("\n"),
        );
        (format!("dtd:{}", noun_name), Func::constant(Object::atom(&dtd_text)))
    }));

    // ── Generator 7: OWL — α(noun → owl_def) ──────────────────────────
    defs.extend(domain.nouns.iter().map(|(noun_name, noun_def)| {
        let properties = Object::Seq(binary_fts_for(&domain, noun_name).iter().map(|ft| {
            let other = other_role_of(ft, noun_name);
            let prop_type = match domain.nouns.get(&other).map(|n| n.object_type.as_str()) {
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

    // ── Generator 8: WSDL — α(noun → wsdl_def) ─────────────────────
    defs.extend(domain.nouns.iter().map(|(noun_name, _)| {
        let has_sm = model.state_machines.iter().any(|sm| sm.noun_name == *noun_name);
        let ops: Vec<Object> = [("create","POST"), ("query","GET"), ("update","PUT")]
            .iter().map(|(op,m)| Object::seq(vec![Object::atom(op), Object::atom(m)]))
            .chain(has_sm.then(|| Object::seq(vec![Object::atom("transition"), Object::atom("POST")])))
            .collect();
        (format!("wsdl:{}", noun_name), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("portType"), Object::atom(noun_name)]),
            Object::seq(vec![Object::atom("operations"), Object::Seq(ops)]),
        ])))
    }));

    // ── Generator 9: EDM — α(noun → edm_def) ──────────────────────────
    defs.extend(domain.nouns.iter().map(|(noun_name, noun_def)| {
        let properties = Object::Seq(binary_fts_for(&domain, noun_name).iter().map(|ft| {
            let other = other_role_of(ft, noun_name);
            let kind = match domain.nouns.get(&other).map(|n| n.object_type.as_str()) {
                Some("entity") => "NavigationProperty", _ => "Property",
            };
            Object::seq(vec![Object::atom(&other), Object::atom(kind), Object::atom("Edm.String")])
        }).collect());
        let key = Object::Seq(domain.ref_schemes.get(noun_name)
            .map(|parts| parts.iter().map(|p| Object::atom(p)).collect()).unwrap_or_default());
        (format!("edm:{}", noun_name), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("entity_type"), Object::atom(noun_name)]),
            Object::seq(vec![Object::atom("base_type"), Object::atom(&noun_def.object_type)]),
            Object::seq(vec![Object::atom("key"), key]),
            Object::seq(vec![Object::atom("properties"), properties]),
        ])))
    }));

    // ��─ Generator 10: XForms ──────────────────────────────────────────
    defs.extend(domain.nouns.iter().map(|(noun_name, _)| {
        let bindings = Object::Seq(binary_fts_for(&domain, noun_name).iter().filter_map(|ft| {
            let other = ft.roles.iter().find(|r| r.noun_name != *noun_name)?;
            let control = match domain.nouns.get(&other.noun_name).map(|n| n.object_type.as_str()) {
                Some("value") => "input", _ => "select1",
            };
            Some(Object::seq(vec![Object::atom(&other.noun_name), Object::atom(control), Object::atom(&ft.reading)]))
        }).collect());
        (format!("xforms:{}", noun_name), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("model"), Object::atom(noun_name)]),
            Object::seq(vec![Object::atom("bindings"), bindings]),
        ])))
    }));

    // ── Generator 11: HTML Report ─────────────────────────────────────
    defs.extend(domain.nouns.iter().map(|(noun_name, noun_def)| {
        let fields = Object::Seq(binary_fts_for(&domain, noun_name).iter().map(|ft|
            Object::seq(vec![Object::atom(&other_role_of(ft, noun_name)), Object::atom(&ft.reading)])
        ).collect());
        let constraints = Object::Seq(noun_constraints_for(&domain, noun_name).iter()
            .map(|c| Object::atom(&c.text)).collect());
        (format!("html:{}", noun_name), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("title"), Object::atom(noun_name)]),
            Object::seq(vec![Object::atom("object_type"), Object::atom(&noun_def.object_type)]),
            Object::seq(vec![Object::atom("fields"), fields]),
            Object::seq(vec![Object::atom("constraints"), constraints]),
        ])))
    }));

    // ─�� Generator 12: NHibernate Mapping ──────────────────────────────
    let tables = crate::rmap::rmap(&domain);
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

    // ── Generator 13: LINQ — α(table → linq_def) ────────────────────
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

    // ── Generator 14: PLiX ────────────────────────────────────────────
    defs.extend(domain.nouns.iter().map(|(noun_name, noun_def)| {
        let fields = Object::Seq(binary_fts_for(&domain, noun_name).iter().filter_map(|ft| {
            let other = ft.roles.iter().find(|r| r.noun_name != *noun_name)?;
            let clr_type = match domain.nouns.get(&other.noun_name).map(|n| n.object_type.as_str()) {
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

    // ── Generator 15: DSL ─────────────────────────────────────────────
    defs.extend(domain.nouns.iter().map(|(noun_name, noun_def)| {
        let readings = Object::Seq(domain.fact_types.values()
            .filter(|ft| ft.roles.iter().any(|r| r.noun_name == *noun_name))
            .map(|ft| Object::atom(&ft.reading)).collect());
        let constraints = Object::Seq(noun_constraints_for(&domain, noun_name).iter()
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
    // of nouns, fact types, constraints, state machines — leaks internals.
    // When OFF (production release builds), returns a tiny counts-only summary
    // so callers can still sanity-check cardinalities without exposing schema.
    #[cfg(feature = "debug-def")]
    {
        let noun_atoms: Vec<Object> = domain.nouns.keys().map(|n| Object::atom(n)).collect();
        let ft_atoms: Vec<Object> = domain.fact_types.iter()
            .map(|(id, ft)| Object::seq(vec![Object::atom(id), Object::atom(&ft.reading)]))
            .collect();
        let constraint_atoms: Vec<Object> = domain.constraints.iter()
            .map(|c| Object::seq(vec![Object::atom(&c.kind), Object::atom(&c.text)]))
            .collect();
        let sm_atoms: Vec<Object> = model.state_machines.iter()
            .map(|sm| Object::seq(vec![
                Object::atom(&sm.noun_name),
                Object::atom(&sm.initial),
                Object::Seq(sm.transition_table.iter()
                    .map(|(from, to, event)| Object::seq(vec![Object::atom(from), Object::atom(to), Object::atom(event)]))
                    .collect()),
            ]))
            .collect();
        let total_facts = domain.fact_types.len() + domain.constraints.len() + domain.general_instance_facts.len();
        defs.push(("debug".to_string(), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("nouns"), Object::Seq(noun_atoms)]),
            Object::seq(vec![Object::atom("factTypes"), Object::Seq(ft_atoms)]),
            Object::seq(vec![Object::atom("constraints"), Object::Seq(constraint_atoms)]),
            Object::seq(vec![Object::atom("stateMachines"), Object::Seq(sm_atoms)]),
            Object::seq(vec![Object::atom("totalFacts"), Object::atom(&total_facts.to_string())]),
        ]))));
    }
    #[cfg(not(feature = "debug-def"))]
    {
        // Counts-only summary — no names, readings, texts, or transitions leaked.
        let noun_count = domain.nouns.len().to_string();
        let ft_count = domain.fact_types.len().to_string();
        let c_count = domain.constraints.len().to_string();
        let sm_count = model.state_machines.len().to_string();
        defs.push(("debug".to_string(), Func::constant(Object::seq(vec![
            Object::seq(vec![Object::atom("nouns"), Object::atom(&noun_count)]),
            Object::seq(vec![Object::atom("factTypes"), Object::atom(&ft_count)]),
            Object::seq(vec![Object::atom("constraints"), Object::atom(&c_count)]),
            Object::seq(vec![Object::atom("stateMachines"), Object::atom(&sm_count)]),
            Object::seq(vec![Object::atom("disabled"), Object::atom("⊥ debug disabled")]),
        ]))));
    }

    defs
}

/// Reconstruct a Domain from an Object state by querying metamodel cells.
pub fn state_to_domain(state: &crate::ast::Object) -> Domain {
    use crate::ast::{fetch_or_phi, binding, cells_iter};
    let mut domain = Domain::default();

    // α(noun_fact → insert into domain maps) : Noun cell
    fetch_or_phi("Noun", state).as_seq().into_iter().flat_map(|facts| facts.iter()).for_each(|f| {
        let name = binding(f, "name").unwrap_or("").to_string();
        let obj_type = binding(f, "objectType").unwrap_or("entity").to_string();
        domain.nouns.insert(name.clone(), NounDef { object_type: obj_type, world_assumption: WorldAssumption::default() });
        binding(f, "superType").map(|st| domain.subtypes.insert(name.clone(), st.to_string()));
        binding(f, "referenceScheme").map(|v| domain.ref_schemes.insert(name.clone(), v.split(',').map(|s| s.to_string()).collect()));
        binding(f, "enumValues").map(|v| domain.enum_values.insert(name.clone(), v.split(',').map(|s| s.to_string()).collect()));
    });

    // α(schema_fact → fact_type) : GraphSchema cell
    let role_cell = fetch_or_phi("Role", state);
    domain.fact_types = fetch_or_phi("GraphSchema", state).as_seq()
        .map(|facts| facts.iter().filter_map(|f| {
            let id = binding(f, "id")?.to_string();
            let reading = binding(f, "reading").unwrap_or("").to_string();
            let roles: Vec<RoleDef> = role_cell.as_seq()
                .map(|rs| rs.iter()
                    .filter(|r| binding(r, "graphSchema") == Some(&id))
                    .map(|r| RoleDef {
                        noun_name: binding(r, "nounName").unwrap_or("").to_string(),
                        role_index: binding(r, "position").and_then(|v| v.parse().ok()).unwrap_or(0),
                    }).collect())
                .unwrap_or_default();
            Some((id, FactTypeDef { schema_id: String::new(), reading, readings: vec![], roles }))
        }).collect())
        .unwrap_or_default();

    // α(constraint_fact → constraint_def) : Constraint cell
    domain.constraints = fetch_or_phi("Constraint", state).as_seq()
        .map(|facts| facts.iter().map(|f| {
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
        }).collect())
        .unwrap_or_default();

    // α(rule_fact → derivation_rule) : DerivationRule cell
    domain.derivation_rules = fetch_or_phi("DerivationRule", state).as_seq()
        .map(|facts| facts.iter().map(|f| {
            let get = |key: &str| binding(f, key).unwrap_or("").to_string();
            DerivationRuleDef {
                id: get("id"), text: get("text"), antecedent_fact_type_ids: vec![],
                consequent_fact_type_id: get("consequentFactTypeId"),
                kind: DerivationKind::ModusPonens, join_on: vec![], match_on: vec![], consequent_bindings: vec![],
            }
        }).collect())
        .unwrap_or_default();

    // α(inst_fact → general_instance_fact) : InstanceFact cell
    domain.general_instance_facts = fetch_or_phi("InstanceFact", state).as_seq()
        .map(|facts| facts.iter().map(|f| {
            let get = |key: &str| binding(f, key).unwrap_or("").to_string();
            GeneralInstanceFact {
                subject_noun: get("subjectNoun"), subject_value: get("subjectValue"),
                field_name: get("fieldName"), object_noun: get("objectNoun"), object_value: get("objectValue"),
            }
        }).collect())
        .unwrap_or_default();

    domain.state_machines = derive_state_machines_from_facts(&domain.general_instance_facts);

    domain
}

/// Compile an entire Domain into executable form.
pub(crate) fn compile(ir: &Domain) -> CompiledModel {
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

    // Compile derivation rules -- both explicit from IR and implicit from structure
    let derivations = compile_derivations(ir);

    // Compile fact types to Construction functions (CONS of Roles)
    let schemas = compile_schemas(ir);

    // Build fact-to-event mapping from schemas + state machines.
    // For each fact type, check if any role's noun has a state machine.
    // If so, check if any transition event name appears in the reading.
    // This is a heuristic until the IR carries explicit Activation/Verb links.
    // α(schema → match event) : schemas — find fact types that activate transitions
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

/// Build the NounIndex by iterating the IR.
fn build_noun_index(
    ir: &Domain,
    constraints: &[CompiledConstraint],
    state_machines: &[CompiledStateMachine],
) -> NounIndex {
    // α(ft → α(role → entry)) : fact_types — noun_name -> [(fact_type_id, role_index)]
    let noun_to_fact_types: HashMap<String, Vec<(String, usize)>> = ir.fact_types.iter()
        .flat_map(|(ft_id, ft)| ft.roles.iter().map(move |role| (role.noun_name.clone(), (ft_id.clone(), role.role_index))))
        .fold(HashMap::new(), |mut acc, (noun, entry)| { acc.entry(noun).or_default().push(entry); acc });

    // noun_name -> world assumption
    let world_assumptions: HashMap<String, WorldAssumption> = ir.nouns.iter()
        .map(|(name, def)| (name.clone(), def.world_assumption.clone()))
        .collect();

    // noun_name -> supertype (from IR maps)
    let supertypes: HashMap<String, String> = ir.subtypes.clone();
    let subtypes: HashMap<String, Vec<String>> = ir.subtypes.iter()
        .fold(HashMap::new(), |mut acc, (child, parent)| { acc.entry(parent.clone()).or_default().push(child.clone()); acc });
    let ref_schemes: HashMap<String, Vec<String>> = ir.ref_schemes.clone();

    // fact_type_id -> list of constraint IDs spanning it
    let fact_type_to_constraints: HashMap<String, Vec<String>> = ir.constraints.iter()
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
fn compile_derivations(ir: &Domain) -> Vec<CompiledDerivation> {
    let mut derivations = Vec::new();

    // α(rule → compiled) : derivation_rules
    derivations.extend(ir.derivation_rules.iter().map(|rule| match rule.kind {
        DerivationKind::Join => compile_join_derivation(ir, rule),
        _ => compile_explicit_derivation(ir, rule),
    }));

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

// (Object-level population helpers obj_find_ft, obj_instances_of,
//  obj_participates_in, obj_derived_fact removed -- no longer needed
//  after eliminating all Func::Native closures. All population
//  traversal is now via pure Func: extract_facts_from_pop, instances_of_noun_func, etc.)

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
fn compile_explicit_derivation(ir: &Domain, rule: &DerivationRuleDef) -> CompiledDerivation {
    let id = rule.id.clone();
    let text = rule.text.clone();
    let kind = rule.kind.clone();
    let antecedent_ids = rule.antecedent_fact_type_ids.clone();
    let consequent_id = rule.consequent_fact_type_id.clone();
    let consequent_reading = ir.fact_types.get(&consequent_id)
        .map(|ft| ft.reading.clone())
        .unwrap_or_default();

    // Pure Func: check all antecedent FTs non-empty, derive consequent.
    // For each antecedent: not(null(extract_facts_from_pop(ft_id)))
    let ant_checks: Vec<Func> = antecedent_ids.iter()
        .map(|ft_id| Func::compose(Func::compose(Func::Not, Func::NullTest), extract_facts_from_pop(ft_id)))
        .collect();

    let all_hold = match ant_checks.len() {
        0 => Func::constant(Object::t()),
        1 => ant_checks.into_iter().next().unwrap(),
        _ => ant_checks.into_iter().reduce(|a, b| Func::compose(Func::And, Func::construction(vec![a, b]))).unwrap(),
    };

    // When all antecedents hold, produce a derived fact.
    // Collect first fact from each antecedent to gather bindings.
    let binding_extractors: Vec<Func> = antecedent_ids.iter()
        .map(|ft_id| Func::compose(Func::Selector(1), extract_facts_from_pop(ft_id)))
        .collect();

    // Derived fact = <ft_id, reading, <bindings from first antecedent fact>>
    let derived = Func::construction(vec![
        Func::constant(Object::atom(&consequent_id)),
        Func::constant(Object::atom(&consequent_reading)),
        if binding_extractors.is_empty() {
            Func::constant(Object::phi())
        } else {
            binding_extractors.into_iter().next().unwrap() // bindings from first antecedent
        },
    ]);

    let func = Func::condition(
        all_hold,
        Func::construction(vec![derived]),
        Func::constant(Object::phi()),
    );
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

    // Build role-index lookup for each antecedent: (ft_idx, noun_name) -> role_index
    let antecedent_roles: Vec<Vec<(String, usize)>> = antecedent_ids.iter().map(|ft_id| {
        ir.fact_types.get(ft_id)
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

    // Dispatch on antecedent count: 0 → phi, 1 → α(derive), ≥2 → iterative join
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
    // foldl(join_step, ft0, [1..n]) — iterative pairwise join
    let current = (1..n).fold(ft0, |current, j| {
        let ft_j = fact_extractors[j].clone();

        // α(key → eq_condition) : join_keys — build join predicates
        let mut join_conds: Vec<Func> = join_keys.iter().filter_map(|key| {
            let j_role = find_role(j, key)?;
            let ref_ft = (0..j).find(|&fi| find_role(fi, key).is_some())?;
            let ref_role = find_role(ref_ft, key)?;
            let ref_val = Func::compose(role_value(ref_role),
                Func::compose(access_fact(ref_ft, j), Func::Selector(1)));
            let new_val = Func::compose(role_value(j_role), Func::Selector(2));
            Some(Func::compose(Func::Eq, Func::construction(vec![ref_val, new_val])))
        }).collect();

        // α(match_pair → contains_condition) : match_pairs
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

        // Pipeline: Filter(join_pred) . Concat . α(DistL) . DistR . [current, ft_j]
        Func::compose(Func::filter(join_pred), Func::compose(Func::Concat,
            Func::compose(Func::apply_to_all(Func::DistL),
                Func::compose(Func::DistR, Func::construction(vec![current, ft_j])))))
    });

    // Build the consequent fact from the final joined structure (depth n).
    // For each consequent binding noun, find which FT has it and extract the value.
    let binding_nouns: Vec<String> = if consequent_binding_names.is_empty() {
        // α(roles → nouns) : antecedents — deduplicated
        antecedent_roles.iter()
            .flat_map(|roles| roles.iter().map(|(noun, _)| noun.clone()))
            .fold(Vec::new(), |mut acc, noun| { if !acc.contains(&noun) { acc.push(noun); } acc })
    } else {
        consequent_binding_names
    };

    // α(noun → extractor) : binding_nouns
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
fn compile_subtype_inheritance(ir: &Domain) -> Vec<CompiledDerivation> {
    // α(subtype_pair → derivation) : subtypes — filter out pairs with no supertype fact types
    ir.subtypes.iter().filter_map(|(sub_name, super_name)| {
        let sft: Vec<(String, String, usize)> = ir.fact_types.iter()
            .flat_map(|(ft_id, ft)| ft.roles.iter()
                .filter(|r| r.noun_name == *super_name)
                .map(move |r| (ft_id.clone(), ft.reading.clone(), r.role_index)))
            .collect();
        (!sft.is_empty()).then_some(())?;

        let sub = sub_name.clone();
        let sup = super_name.clone();
        let instances = instances_of_noun_func(&sub);

        // α(super_ft → check_and_derive) : super_fact_types
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
fn compile_modus_ponens(ir: &Domain) -> Vec<CompiledDerivation> {
    // α(ss_constraint → derivation) : Filter(kind=SS ∧ spans≥2) : constraints
    ir.constraints.iter()
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
        let b_role_names: Vec<String> = ir.fact_types.get(&b_ft_id)
            .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
            .unwrap_or_default();

        let b_reading = ir.fact_types.get(&b_ft_id)
            .map(|ft| ft.reading.clone())
            .unwrap_or_default();

        let id = format!("_modus_ponens_{}", cdef.id);
        let text = format!("Modus ponens from SS constraint: {}", cdef.text);

        // Pure Func: for each A-fact not in B, derive a B-fact.
        // Uses same pattern as compile_subset_ast membership check.
        let a_role_names: Vec<String> = ir.fact_types.get(&a_ft_id)
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
fn compile_transitivity(ir: &Domain) -> Vec<CompiledDerivation> {
    // Cross-product of binary fact types, filtered by shared noun (A->B, B->C)
    let binary_fts: Vec<(&String, &FactTypeDef)> = ir.fact_types.iter()
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
fn compile_cwa_negation(ir: &Domain) -> Vec<CompiledDerivation> {
    ir.nouns.iter()
        .filter(|(_, def)| def.world_assumption == WorldAssumption::Closed)
        .filter_map(|(noun_name, _)| {
            let relevant_fts: Vec<(String, String, usize)> = ir.fact_types.iter()
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
            // presence constraints (MC, FC, …) enumerating positive
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
fn compile_sm_initialization(ir: &Domain) -> Vec<CompiledDerivation> {
    ir.state_machines.iter().map(|(noun_name, sm_def)| {
        let sm_noun = sm_def.noun_name.clone();
        let initial_status = sm_def.statuses.first().cloned().unwrap_or_default();
        let id_str = format!("_sm_init_{}", noun_name);
        let text_str = format!("SM init for {}", noun_name);

        let get_instances = instances_of_noun_func(noun_name);

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
    }).collect()
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
            // -- Pure AST constraints --------------------------------
            "IR" => compile_ring_irreflexive_ast(def),
            "AS" => compile_ring_asymmetric_ast(def),
            "SY" => compile_ring_symmetric_ast(def),
            "AT" | "ANS" => compile_ring_antisymmetric_ast(def),

            // -- AST with Native evaluation kernel --------------------
            "UC" => compile_uniqueness_ast(ir, def),
            "MC" => compile_mandatory_ast(ir, def),

            // -- AST with Native evaluation kernel (continued) --------
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
    let id = def.id.clone();
    let text = def.text.clone();

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
    let tc_func = Func::Native(std::sync::Arc::new(move |edges: &Object| {
        // edges is the initial edge set (sequence of facts)
        let initial = match edges.as_seq() {
            Some(e) => e.to_vec(),
            None => return Object::Bottom,
        };

        // Extract <role0_val, role1_val> pairs from encoded facts.
        // Fact encoding: <<noun0, val0>, <noun1, val1>, ...>
        fn edge_pair(fact: &Object) -> Option<(String, String)> {
            let items = fact.as_seq().filter(|i| i.len() >= 2)?;
            let v0 = items[0].as_seq().and_then(|p| p.get(1)).and_then(|v| v.as_atom())?;
            let v1 = items[1].as_seq().and_then(|p| p.get(1)).and_then(|v| v.as_atom())?;
            Some((v0.to_string(), v1.to_string()))
        }

        let original_pairs: Vec<(String, String)> = initial.iter()
            .filter_map(|f| edge_pair(f))
            .collect();

        // Build transitive closure via iter::successors — Backus's while form.
        // Termination encoded as None (no new edges); max 1001 iterations.
        let tc: std::collections::HashSet<(String, String)> = std::iter::successors(
            Some(original_pairs.iter().cloned().collect::<std::collections::HashSet<(String, String)>>()),
            |tc| {
                let new_edges: Vec<(String, String)> = tc.iter()
                    .flat_map(|(a, b)| original_pairs.iter()
                        .filter(|(c, _)| b == c)
                        .filter_map(|(_, d)| {
                            (!tc.contains(&(a.clone(), d.clone())))
                                .then(|| (a.clone(), d.clone()))
                        })
                        .collect::<Vec<_>>())
                    .collect();
                (!new_edges.is_empty()).then(|| {
                    let mut next = tc.clone();
                    next.extend(new_edges);
                    next
                })
            },
        ).take(1001).last().unwrap();

        // Find self-loops (cycles): (x, x) in tc
        let cycle_nodes: Vec<Object> = tc.iter()
            .filter(|(a, b)| a == b)
            .map(|(a, _)| {
                // Reconstruct as a fact-like object for the violation formatter
                Object::Seq(vec![
                    Object::Seq(vec![Object::atom("_"), Object::atom(a)]),
                    Object::Seq(vec![Object::atom("_"), Object::atom(a)]),
                ])
            })
            .collect();

        Object::Seq(cycle_nodes)
    }));

    // Pipeline: extract facts -> compute transitive closure -> report violations
    Func::compose(
        Func::apply_to_all(viol),
        Func::compose(tc_func, facts),
    )
}

/// RF: for each entity x, xRx must exist -- violation when self-reference is missing.
/// Pure Func: set_diff(all_instances, self_refs) then make_violation for each.
fn compile_ring_reflexive_ast(ir: &Domain, def: &ConstraintDef) -> Func {
    let ft_ids: Vec<String> = def.spans.iter().map(|s| s.fact_type_id.clone()).collect();
    let facts = extract_facts_multi(&ft_ids);

    let id_obj = Object::atom(&def.id);
    let text_obj = Object::atom(&def.text);

    let noun_name: String = def.spans.first()
        .and_then(|s| ir.fact_types.get(&s.fact_type_id))
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
fn compile_uniqueness_ast(_ir: &Domain, def: &ConstraintDef) -> Func {
    let spans = resolve_spans(_ir, &def.spans);

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
fn compile_mandatory_ast(_ir: &Domain, def: &ConstraintDef) -> Func {
    let spans = resolve_spans(_ir, &def.spans);

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
fn compile_frequency_ast(_ir: &Domain, def: &ConstraintDef) -> Func {
    let spans = resolve_spans(_ir, &def.spans);
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
fn compile_value_constraint_ast(ir: &Domain, def: &ConstraintDef) -> Func {
    // Collect allowed values from the nouns in the spanned fact types
    let spans = resolve_spans(ir, &def.spans);
    let allowed: Vec<(String, HashSet<String>)> = spans.iter().filter_map(|span| {
        let vals = ir.enum_values.get(&span.noun_name).filter(|v| !v.is_empty())?;
        Some((span.noun_name.clone(), vals.iter().cloned().collect::<HashSet<_>>()))
    }).collect();

    // If no enum values found on spanned nouns, check all nouns with enum_values
    let check_nouns: Vec<(String, HashSet<String>)> = if !allowed.is_empty() {
        allowed
    } else {
        ir.enum_values.iter().filter_map(|(name, vals)| {
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
        let allowed_const = Func::constant(Object::Seq(allowed_atoms));

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
    _ir: &Domain,
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
fn compile_subset_ast(ir: &Domain, def: &ConstraintDef) -> Func {
    match def.spans.len() {
        0 | 1 => return Func::constant(Object::phi()),
        _ => {},
    }

    let a_ft_id = def.spans[0].fact_type_id.clone();
    let b_ft_id = def.spans[1].fact_type_id.clone();

    let a_nouns: Vec<String> = ir.fact_types.get(&a_ft_id)
        .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
        .unwrap_or_default();
    let b_nouns: Vec<String> = ir.fact_types.get(&b_ft_id)
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
    let mut detail_parts: Vec<Func> = std::iter::once(Func::constant(Object::atom("Subset violation:")))
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
fn compile_equality_ast(ir: &Domain, def: &ConstraintDef) -> Func {
    match def.spans.len() {
        0 | 1 => return Func::constant(Object::phi()),
        _ => {},
    }

    // EQ = SS(A,B) union SS(B,A). Build both subset checks.
    let a_ft_id = def.spans[0].fact_type_id.clone();
    let b_ft_id = def.spans[1].fact_type_id.clone();

    let a_roles: Vec<(usize, String)> = ir.fact_types.get(&a_ft_id)
        .map(|ft| ft.roles.iter().enumerate().map(|(i, r)| (i, r.noun_name.clone())).collect())
        .unwrap_or_default();
    let b_roles: Vec<(usize, String)> = ir.fact_types.get(&b_ft_id)
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
    let detail_ab: Vec<Func> = std::iter::once(Func::constant(Object::atom("Equality violation:")))
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
    let detail_ba: Vec<Func> = std::iter::once(Func::constant(Object::atom("Equality violation:")))
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
fn compile_forbidden_ast(ir: &Domain, def: &ConstraintDef) -> Func {
    let forbidden_values = collect_enum_values(ir, &def.spans);
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
        let values_const = Func::constant(Object::Seq(value_atoms));

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
        let kws_const = Func::constant(Object::Seq(kw_atoms));

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

        // Condition: length(matched) > threshold -> violation
        let threshold_str = threshold.to_string();
        Func::condition(
            Func::compose(Func::Not, Func::compose(Func::Eq, Func::construction(vec![
                Func::compose(Func::Length, matched_kws.clone()),
                Func::constant(Object::atom("0")),
            ]))),
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
fn compile_obligatory_ast(ir: &Domain, def: &ConstraintDef) -> Func {
    let obligatory_values = collect_enum_values(ir, &def.spans);
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
    // α(noun_values → condition) : obligatory_values
    let noun_checks: Vec<Func> = obligatory_values.iter().map(|(noun_name, enum_vals)| {
        let val_atoms: Vec<Object> = enum_vals.iter().map(|v| Object::atom(v)).collect();
        let vals_const = Func::constant(Object::Seq(val_atoms));

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
    // Use .then() to conditionally produce a check — pure Backus cond without side effects.
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

    // α(word → pascal_split → filter(len>2)) : words
    let mut keywords: Vec<String> = stripped.split_whitespace()
        .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|clean| !clean.is_empty())
        .flat_map(|clean| {
            // PascalCase split via fold: accumulate chars, emit on uppercase boundary
            let (parts, last) = clean.chars().fold((Vec::new(), String::new()), |(mut parts, mut cur), ch| {
                // At uppercase boundary (with non-empty accumulator), flush cur into parts
                (ch.is_uppercase() && !cur.is_empty())
                    .then(|| parts.push(std::mem::take(&mut cur)));
                cur.push(ch);
                (parts, cur)
            });
            parts.into_iter().chain(std::iter::once(last))
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

    let initial = def.statuses.first().cloned().unwrap_or_default();

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
                    // For multiple guards: all must pass — fold over tail
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

// -- Schema Compilation Tests -----------------------------------------

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
        let ir = make_ir_with_fact_type(
            "ft1", "Organization has Name",
            vec![("Organization", 0), ("Name", 1)],
        );
        let model = compile(&ir);
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
        let ir = make_ir_with_fact_type(
            "ft1", "OrgMembership is for User",
            vec![("OrgMembership", 0), ("User", 1)],
        );
        let model = compile(&ir);
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
        let mut state = crate::ast::Object::phi();
        state = crate::ast::cell_push("ft1", crate::ast::fact_from_pairs(&[("Person", "Alice"), ("Name", "Alice Smith")]), &state);
        state = crate::ast::cell_push("ft1", crate::ast::fact_from_pairs(&[("Person", "Bob"), ("Name", "Bob Jones")]), &state);

        let ctx_obj = crate::ast::encode_eval_context_state("", None, &state);
        let defs = crate::ast::Object::phi();
        let result = crate::ast::apply(&constraint.func, &ctx_obj, &defs);

        // No violations -- should be phi (empty sequence)
        let violations = crate::ast::decode_violations(&result);
        assert_eq!(violations.len(), 0);
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
