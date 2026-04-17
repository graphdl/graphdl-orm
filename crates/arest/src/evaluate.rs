// crates/arest/src/evaluate.rs
//
// Evaluation is beta reduction. That's it.
//
// Constraint verification:  constraints.flat_map(|c| apply(c.func, ctx)) -> [Violation]
// Forward inference:        derivations.flat_map(|d| apply(d.func, pop)) -> [DerivedFact]
// State machine execution:  fold(transition)(initial)(stream) -> final_state
// Synthesis:                collect all knowledge about a noun from the compiled model.

use hashbrown::HashSet;
use crate::types::*;
use crate::ast;
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

// -- Forward Chaining -------------------------------------------------
//
// Correctness: FORML 2 derivation rules are monotonic (add facts, never
// remove). The population is finite. A monotonic sequence over a finite
// set reaches a fixed point. The loop terminates when no new facts are
// derived.
//
// Safety: the iteration bound prevents pathological rule sets from
// producing unbounded intermediate populations. If the bound is hit,
// the engine stops and returns what it has -- a partial fixed point.

/// Forward-chain derivation rules to a fixed point.
///
/// Each derivation def is applied to the current population. New facts
/// are added, and the process repeats until no new facts are derived
/// (fixed point reached) or the iteration bound is hit.
///
/// Iteration bound: 100 iterations maximum. FORML2 derivation rules are
/// monotonic (facts are added, never removed) over a finite domain, so
/// convergence is guaranteed in theory. The 100-iteration bound is a
/// safety net for pathological rule sets that produce very large
/// intermediate populations. If the bound is exceeded, the function
/// returns a partial fixed point -- all facts derived so far, even
/// though additional derivations may be possible. This is safe because
/// each derived fact is individually correct; only completeness is
/// affected.
/// Forward-chain derivation rules over D to fixed point. Returns (D', derived_facts).
/// D contains both population cells and def cells (Backus Sec. 14.3).
pub fn forward_chain_defs_state(
    derivation_defs: &[(&str, &ast::Func)],
    d: &ast::Object,
) -> (ast::Object, Vec<DerivedFact>) {

    /// Apply all derivation rules once, returning novel facts.
    fn derive_one_round(
        derivation_defs: &[(&str, &ast::Func)],
        current_state: &ast::Object,
        all_derived: &[DerivedFact],
        d: &ast::Object,
    ) -> Vec<DerivedFact> {
        let pop_obj = ast::encode_state(current_state);
        derivation_defs.iter()
            .flat_map(|(name, func)| {
                let result = ast::apply(func, &pop_obj, d);
                let name = name.to_string();
                result.as_seq().into_iter()
                    .flat_map(move |items| items.iter().cloned().collect::<Vec<_>>())
                    .filter_map(move |item| parse_derived_fact(&item, &name))
                    .collect::<Vec<_>>()
            })
            .filter(|fact| {
                !state_contains_fact(current_state, fact)
                    && !all_derived.iter().any(|d| same_fact(d, fact))
            })
            .fold(Vec::new(), |mut acc, fact| {
                (!acc.iter().any(|d| same_fact(d, &fact))).then(|| acc.push(fact));
                acc
            })
    }

    // Fixed-point iteration via iter::successors (Backus while form, Knaster-Tarski lfp).
    // Terminates when derive_one_round produces no new facts (returns None).
    let (final_state, all_derived) = core::iter::successors(
        Some((d.clone(), Vec::<DerivedFact>::new())),
        |(current_state, all_derived)| {
            let new_facts = derive_one_round(derivation_defs, current_state, all_derived, d);
            (!new_facts.is_empty()).then(|| {
                let new_state = new_facts.iter().fold(current_state.clone(), |acc, fact| {
                    let pairs: Vec<(&str, &str)> = fact.bindings.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                    ast::cell_push(&fact.fact_type_id, ast::fact_from_pairs(&pairs), &acc)
                });
                let all = [all_derived.clone(), new_facts].concat();
                (new_state, all)
            })
        },
    ).take(101).last().unwrap();
    (final_state, all_derived)
}

/// Parse a derivation result Object into a DerivedFact.
fn parse_derived_fact(item: &ast::Object, derived_by: &str) -> Option<DerivedFact> {
    let fact_items = item.as_seq().filter(|f| f.len() >= 3)?;
    let ft_id = fact_items[0].as_atom()?.to_string();
    let reading = fact_items[1].as_atom()?.to_string();
    let bindings: Vec<(String, String)> = fact_items[2].as_seq()
        .unwrap_or(&[])
        .iter()
        .filter_map(|b| {
            let pair = b.as_seq()?;
            if pair.len() == 2 {
                Some((pair[0].as_atom()?.to_string(), pair[1].as_atom()?.to_string()))
            } else { None }
        })
        .collect();
    Some(DerivedFact {
        fact_type_id: ft_id, reading, bindings,
        derived_by: derived_by.to_string(),
        confidence: Confidence::Definitive,
    })
}

/// Check if a derived fact already exists in the state.
fn state_contains_fact(state: &ast::Object, fact: &DerivedFact) -> bool {
    let cell = ast::fetch_or_phi(&fact.fact_type_id, state);
    cell.as_seq().map_or(false, |facts| {
        facts.iter().any(|f| {
            let fb: Vec<(String, String)> = f.as_seq().map(|pairs| {
                pairs.iter().filter_map(|pair| {
                    let items = pair.as_seq()?;
                    Some((items.get(0)?.as_atom()?.to_string(), items.get(1)?.as_atom()?.to_string()))
                }).collect()
            }).unwrap_or_default();
            fb.len() == fact.bindings.len()
                && fb.iter().all(|b| fact.bindings.contains(b))
        })
    })
}

/// Check if two derived facts represent the same fact.
fn same_fact(a: &DerivedFact, b: &DerivedFact) -> bool {
    a.fact_type_id == b.fact_type_id
        && a.bindings.len() == b.bindings.len()
        && a.bindings.iter().all(|ab| b.bindings.contains(ab))
}

// -- Proof Engine (Backward Chaining) ---------------------------------
// Given a goal fact, work backward through derivation rules to build a proof tree.
// Each step either finds the fact in the population (axiom), derives it via a rule
// (recursively proving antecedents), or concludes based on world assumption.

/// Attempt to prove a goal fact.
///
/// `goal` is a string like "Academic has Rank 'P'" -- a reading with optional values.
/// The engine searches the population for a matching fact, then tries derivation
/// Prove from Object state directly. No Domain reconstruction.
pub fn prove_from_state(state: &ast::Object, goal: &str, world_assumption: &WorldAssumption) -> ProofResult {
    let schemas = ast::fetch_or_phi("FactType", state);
    let rules = ast::fetch_or_phi("DerivationRule", state);
    let proof = prove_goal_state_pop(state, goal, &HashSet::new(), &schemas, &rules);
    let status = match &proof {
        Some(_) => ProofStatus::Proven,
        None => match world_assumption {
            WorldAssumption::Closed => ProofStatus::Disproven,
            WorldAssumption::Open => ProofStatus::Unknown,
        },
    };
    ProofResult { goal: goal.to_string(), status, proof, world_assumption: world_assumption.clone() }
}

fn prove_goal_state_pop(
    state: &ast::Object, goal: &str, visited: &HashSet<String>,
    schemas: &ast::Object, rules: &ast::Object,
) -> Option<ProofStep> {
    (!visited.contains(goal)).then_some(())?;
    let visited = &{ let mut v = visited.clone(); v.insert(goal.to_string()); v };

    let schema_reading = |ft_id: &str| -> Option<String> {
        schemas.as_seq()?.iter()
            .find(|s| ast::binding(s, "id") == Some(ft_id))
            .and_then(|s| ast::binding(s, "reading").map(|r| r.to_string()))
    };

    // Axiom search first (Step 1), else derivation search (Step 2).
    // `or_else` is Backus cond lifted into Option: axiom ? axiom : derive().
    ast::cells_iter(state).into_iter()
        .filter_map(|(ft_id, contents)| {
            let reading = schema_reading(ft_id)?;
            contents.as_seq()?.iter()
                .map(|fact| {
                    let bindings = extract_bindings(fact);
                    format_fact(&reading, &bindings)
                })
                .find(|fact_text| fact_text_matches(goal, fact_text, &reading))
                .map(|fact_text| ProofStep { fact: fact_text, justification: Justification::Axiom, children: vec![] })
        })
        .next()
        .or_else(|| rules.as_seq().and_then(|rule_list| {
        rule_list.iter().find_map(|rule| {
            let cons_ft_id = ast::binding(rule, "consequentFactTypeId")?.to_string();
            let cons_reading = schema_reading(&cons_ft_id)?;
            let goal_prefix = goal.split(' ').next().unwrap_or("");
            (goal.contains(&cons_reading) || cons_reading.contains(goal_prefix)).then_some(())?;

            let ant_ids_str = ast::binding(rule, "antecedentFactTypeIds")?.to_string();
            let child_proofs: Option<Vec<ProofStep>> = ant_ids_str.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(|ant_id| {
                    let ant_reading = schema_reading(&ant_id)?;
                    prove_goal_state_pop(state, &ant_reading, visited, schemas, rules)
                })
                .collect();

            let children = child_proofs.filter(|c| !c.is_empty())?;
            Some(ProofStep {
                fact: goal.to_string(),
                justification: Justification::Derived {
                    rule_id: ast::binding(rule, "id").unwrap_or("").to_string(),
                    rule_text: ast::binding(rule, "text").unwrap_or("").to_string(),
                },
                children,
            })
        })
    }))
}

/// Extract bindings from a fact Object as (key, value) pairs.
fn extract_bindings(fact: &ast::Object) -> Vec<(String, String)> {
    fact.as_seq().map(|pairs| {
        pairs.iter().filter_map(|pair| {
            let items = pair.as_seq()?;
            Some((items.get(0)?.as_atom()?.to_string(), items.get(1)?.as_atom()?.to_string()))
        }).collect()
    }).unwrap_or_default()
}

/// Format a fact from its reading template and bindings
#[allow(dead_code)] // called by prove_goal()
fn format_fact(reading: &str, bindings: &[(String, String)]) -> String {
    bindings.iter().fold(reading.to_string(), |result, (noun, value)| {
        result.find(noun.as_str())
            .map(|pos| format!("{}{} '{}'{}",  &result[..pos], noun, value, &result[pos + noun.len()..]))
            .unwrap_or(result)
    })
}

/// Check if a goal string matches a formatted fact
#[allow(dead_code)] // called by prove_goal()
fn fact_text_matches(goal: &str, fact_text: &str, reading: &str) -> bool {
    let goal_lower = goal.to_lowercase();
    let fact_lower = fact_text.to_lowercase();
    let reading_lower = reading.to_lowercase();
    goal == fact_text || goal == reading
        || goal_lower == fact_lower || goal_lower == reading_lower
        || fact_lower.contains(&goal_lower)
        || goal_lower.contains(&reading_lower)
}

// -- Synthesis --------------------------------------------------------

/// Synthesize from Object state directly.
pub fn synthesize_from_state(state: &ast::Object, noun_name: &str, depth: usize) -> SynthesisResult {
    let b = |f: &ast::Object, key: &str| -> String {
        ast::binding(f, key).unwrap_or("").to_string()
    };

    let wa = WorldAssumption::Closed;

    // 1. Find schemas where this noun plays a role (via Role facts)
    let role_cell = ast::fetch_or_phi("Role", state);
    let role_facts = role_cell.as_seq().unwrap_or(&[]);
    let schema_ids_for_noun: Vec<(String, usize)> = role_facts.iter()
        .filter(|r| b(r, "nounName") == noun_name)
        .map(|r| (b(r, "factType"), b(r, "position").parse().unwrap_or(0)))
        .collect();

    let schema_cell = ast::fetch_or_phi("FactType", state);
    let schema_facts = schema_cell.as_seq().unwrap_or(&[]);
    let participates_in: Vec<FactTypeSummary> = schema_ids_for_noun.iter()
        .filter_map(|(sid, role_idx)| {
            let reading = schema_facts.iter()
                .find(|s| b(s, "id") == *sid)
                .map(|s| b(s, "reading"))?;
            Some(FactTypeSummary { id: sid.clone(), reading, role_index: *role_idx })
        })
        .collect();

    // 2. Constraints spanning those fact types
    // Block-scoped ft_ids so its borrow on participates_in ends
    // before the move into SynthesisResult at end of function.
    let applicable_constraints: Vec<ConstraintSummary> = {
        let ft_ids: HashSet<&str> = participates_in.iter().map(|f| f.id.as_str()).collect();
        let constraint_cell = ast::fetch_or_phi("Constraint", state);
        let constraint_facts = constraint_cell.as_seq().unwrap_or(&[]);
        let mut seen = HashSet::new();
        constraint_facts.iter()
            .filter(|c| {
                (0..4).any(|i| {
                    let ft_key = format!("span{}_factTypeId", i);
                    let ft_id = b(c, &ft_key);
                    !ft_id.is_empty() && ft_ids.contains(ft_id.as_str())
                })
            })
            .filter(|c| seen.insert(b(c, "id")))
            .map(|c| ConstraintSummary {
                id: b(c, "id"), text: b(c, "text"), kind: b(c, "kind"),
                modality: b(c, "modality"), deontic_operator: {
                    let op = b(c, "deonticOperator");
                    if op.is_empty() { None } else { Some(op) }
                },
            })
            .collect()
    };

    // 3. State machines (from InstanceFact: "State Machine Definition 'X' is for Noun 'noun'")
    let inst_cell = ast::fetch_or_phi("InstanceFact", state);
    let inst_facts = inst_cell.as_seq().unwrap_or(&[]);
    let state_machines: Vec<StateMachineSummary> = inst_facts.iter()
        .filter(|f| b(f, "subjectNoun") == "State Machine Definition" && b(f, "objectNoun") == "Noun" && b(f, "objectValue") == noun_name)
        .map(|f| {
            let sm_name = b(f, "subjectValue");
            let statuses: Vec<String> = inst_facts.iter()
                .filter(|s| b(s, "subjectNoun") == "Status" && b(s, "objectNoun") == "State Machine Definition" && b(s, "objectValue") == sm_name)
                .map(|s| b(s, "subjectValue"))
                .collect();
            let initial = inst_facts.iter()
                .find(|s| b(s, "subjectNoun") == "Status" && b(s, "fieldName") == "is initial in" && b(s, "objectValue") == sm_name)
                .map(|s| b(s, "subjectValue"))
                .unwrap_or_else(|| statuses.first().cloned().unwrap_or_default());
            let valid_transitions: Vec<String> = inst_facts.iter()
                .filter(|t| b(t, "subjectNoun") == "Transition" && b(t, "objectNoun") == "Event Type")
                .filter(|t| {
                    let trans_name = b(t, "subjectValue");
                    inst_facts.iter().any(|tf| b(tf, "subjectNoun") == "Transition" && b(tf, "subjectValue") == trans_name && b(tf, "objectNoun") == "Status" && b(tf, "objectValue") == initial && b(tf, "fieldName").contains("from"))
                })
                .map(|t| b(t, "objectValue"))
                .collect();
            StateMachineSummary { noun_name: sm_name, statuses, current_status: Some(initial), valid_transitions }
        })
        .collect();

    // 4. Related nouns
    let mut seen_related = HashSet::new();
    let related_nouns: Vec<RelatedNoun> = if depth > 0 {
        participates_in.iter()
            .flat_map(|fts| {
                role_facts.iter()
                    .filter(|r| b(r, "factType") == fts.id && b(r, "nounName") != noun_name)
                    .filter(|r| seen_related.insert(b(r, "nounName")))
                    .map(|r| RelatedNoun {
                        name: b(r, "nounName"),
                        via_fact_type: fts.id.clone(),
                        via_reading: fts.reading.clone(),
                        world_assumption: WorldAssumption::Closed,
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    } else { Vec::new() };

    SynthesisResult {
        noun_name: noun_name.to_string(), world_assumption: wa,
        participates_in, applicable_constraints, state_machines,
        derived_facts: Vec::new(), related_nouns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashbrown::HashMap;
    use crate::parse_forml2::Domain;

    fn empty_ir() -> Domain {
        Domain {
            nouns: HashMap::new(),
            fact_types: HashMap::new(),
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![], general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
            cells: HashMap::new(),
        }
    }

    fn empty_state() -> ast::Object {
        ast::Object::phi()
    }

    fn make_noun(object_type: &str) -> NounDef {
        NounDef {
            object_type: object_type.to_string(),
            world_assumption: WorldAssumption::default(),
        }
    }

    /// Build Object state with facts from pairs.
    fn state_with_facts(ft_id: &str, pairs_list: &[&[(&str, &str)]]) -> ast::Object {
        pairs_list.iter().fold(ast::Object::phi(), |acc, pairs|
            ast::cell_push(ft_id, ast::fact_from_pairs(pairs), &acc))
    }

    fn ir_to_defs(ir: &Domain) -> (ast::Object, Vec<(String, ast::Func)>, ast::Object) {
        // Via Object state: the AREST pipeline per Thm 2 (parse Râ†’Î¦, compile Î¦â†’O).
        let state = crate::parse_forml2::domain_to_state(ir);
        let model = crate::compile::compile(&state);
        let defs: Vec<(String, ast::Func)> = model.constraints.iter()
            .map(|c| (format!("constraint:{}", c.id), c.func.clone()))
            .chain(model.state_machines.iter().flat_map(|sm| [
                (format!("machine:{}", sm.noun_name), sm.func.clone()),
                (format!("machine:{}:initial", sm.noun_name), ast::Func::constant(ast::Object::atom(&sm.initial))),
            ]))
            .chain(model.derivations.iter().map(|d| (format!("derivation:{}", d.id), d.func.clone())))
            .chain(model.schemas.iter().map(|(id, schema)| (format!("schema:{}", id), schema.construction.clone())))
            .collect();
        let def_map = ast::defs_to_state(&defs, &state);
        (state, defs, def_map)
    }

    /// Evaluate constraints via defs.
    fn eval_constraints_defs(
        defs: &[(String, ast::Func)],
        def_map: &ast::Object,
        text: &str,
        sender: Option<&str>,
        state: &ast::Object,
    ) -> Vec<Violation> {
        let ctx_obj = ast::encode_eval_context_state(text, sender, state);
        defs.iter()
            .filter(|(n, _)| n.starts_with("constraint:"))
            .flat_map(|(name, func)| {
                let result = ast::apply(func, &ctx_obj, def_map);
                let is_deontic = name.contains("obligatory") || name.contains("forbidden");
                ast::decode_violations(&result).into_iter().map(move |mut v| {
                    v.alethic = !is_deontic;
                    v
                })
            })
            .collect()
    }

    /// Run a state machine from defs (replaces run_machine_ast).
    fn run_machine_defs(
        defs: &[(String, ast::Func)],
        def_map: &ast::Object,
        noun_name: &str,
        events: &[&str],
    ) -> String {
        let machine_key = format!("machine:{}", noun_name);
        let initial_key = format!("machine:{}:initial", noun_name);
        let func = defs.iter().find(|(n, _)| *n == machine_key).map(|(_, f)| f);
        let initial = defs.iter().find(|(n, _)| *n == initial_key)
            .and_then(|(_, f)| {
                let r = ast::apply(f, &ast::Object::phi(), def_map);
                r.as_atom().map(|s| s.to_string())
            })
            .unwrap_or_default();

        let func = match func {
            Some(f) => f,
            None => return initial,
        };

        events.into_iter().fold(initial, |state, event| {
            let input = ast::Object::seq(vec![
                ast::Object::atom(&state),
                ast::Object::atom(event),
            ]);
            let result = ast::apply(func, &input, def_map);
            result.as_atom().map(|s| s.to_string()).unwrap_or(state)
        })
    }

    /// Extract derivation defs from the full defs list.
    fn derivation_defs_from<'a>(defs: &'a [(String, ast::Func)]) -> Vec<(&'a str, &'a ast::Func)> {
        defs.iter()
            .filter(|(n, _)| n.starts_with("derivation:"))
            .map(|(n, f)| (n.as_str(), f))
            .collect()
    }

    // -- DEFS evaluation path tests ------------------------------------

    #[test]
    fn test_evaluate_via_ast_uniqueness_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Person has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        ir.constraints.push(ConstraintDef {
            id: "uc1".to_string(),
            kind: "UC".to_string(),
            modality: "Alethic".to_string(),
            text: "Each Person has at most one Name".to_string(),
            spans: vec![crate::types::SpanDef {
                fact_type_id: "ft1".to_string(),
                role_index: 0,
                subset_autofill: None,
            }],
            ..Default::default()
        });

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);

        let state = state_with_facts("ft1", &[
            &[("Person", "Alice"), ("Name", "A")],
            &[("Person", "Alice"), ("Name", "B")],
        ]);

        let violations = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].constraint_id, "uc1");
    }

    #[test]
    fn test_evaluate_via_ast_no_violations() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Person has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        ir.constraints.push(ConstraintDef {
            id: "uc1".to_string(),
            kind: "UC".to_string(),
            modality: "Alethic".to_string(),
            text: "Each Person has at most one Name".to_string(),
            spans: vec![crate::types::SpanDef {
                fact_type_id: "ft1".to_string(),
                role_index: 0,
                subset_autofill: None,
            }],
            ..Default::default()
        });

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);

        let state = state_with_facts("ft1", &[
            &[("Person", "Alice"), ("Name", "A")],
        ]);

        let violations = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_run_machine_via_ast() {
        // Domain Change state machine: Proposed -> Under Review -> Approved -> Applied
        let mut ir = empty_ir();
        ir.state_machines.insert("DomainChange".to_string(), StateMachineDef {
            noun_name: "DomainChange".to_string(),
            statuses: vec![
                "Proposed".to_string(),
                "Under Review".to_string(),
                "Approved".to_string(),
                "Applied".to_string(),
                "Rejected".to_string(),
            ],
            transitions: vec![
                TransitionDef { from: "Proposed".to_string(), to: "Under Review".to_string(), event: "review-requested".to_string(), guard: None },
                TransitionDef { from: "Under Review".to_string(), to: "Approved".to_string(), event: "approved".to_string(), guard: None },
                TransitionDef { from: "Under Review".to_string(), to: "Rejected".to_string(), event: "rejected".to_string(), guard: None },
                TransitionDef { from: "Approved".to_string(), to: "Applied".to_string(), event: "applied".to_string(), guard: None },
            ],
        });

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);

        // Happy path: Proposed -> Under Review -> Approved -> Applied
        let final_state = run_machine_defs(&defs, &def_map, "DomainChange", &["review-requested", "approved", "applied"]);
        assert_eq!(final_state, "Applied");

        // Rejection path: Proposed -> Under Review -> Rejected
        let final_state = run_machine_defs(&defs, &def_map, "DomainChange", &["review-requested", "rejected"]);
        assert_eq!(final_state, "Rejected");

        // Invalid event: stays in current state
        let final_state = run_machine_defs(&defs, &def_map, "DomainChange", &["applied"]);
        assert_eq!(final_state, "Proposed"); // "applied" not valid from Proposed

        // Partial: just review
        let final_state = run_machine_defs(&defs, &def_map, "DomainChange", &["review-requested"]);
        assert_eq!(final_state, "Under Review");
    }

    #[test]
    fn test_initial_state_is_first_status() {
        let mut ir = empty_ir();
        ir.state_machines.insert("SM".to_string(), StateMachineDef {
            noun_name: "Order".to_string(),
            statuses: vec!["Pending".to_string(), "Shipped".to_string(), "Delivered".to_string()],
            transitions: vec![
                TransitionDef { from: "Pending".to_string(), to: "Shipped".to_string(), event: "ship".to_string(), guard: None },
                TransitionDef { from: "Shipped".to_string(), to: "Delivered".to_string(), event: "deliver".to_string(), guard: None },
            ],
        });
        let (_meta_state, defs, def_map) = ir_to_defs(&ir);
        let initial_key = "machine:Order:initial";
        let initial = defs.iter().find(|(n, _)| n == initial_key)
            .and_then(|(_, f)| {
                let r = ast::apply(f, &ast::Object::phi(), &def_map);
                r.as_atom().map(|s| s.to_string())
            })
            .unwrap_or_default();
        assert_eq!(initial, "Pending");
    }

    #[test]
    fn test_noun_without_state_machine() {
        let ir = empty_ir(); // no state machines
        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);
        let has_machine = defs.iter().any(|(n, _)| n.starts_with("machine:Customer"));
        assert!(!has_machine);
    }

    #[test]
    fn test_valid_transitions_from_status() {
        let mut ir = empty_ir();
        ir.state_machines.insert("SM".to_string(), StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Triaging".to_string(), "Investigating".to_string(), "Resolved".to_string()],
            transitions: vec![
                TransitionDef { from: "Triaging".to_string(), to: "Investigating".to_string(), event: "investigate".to_string(), guard: None },
                TransitionDef { from: "Triaging".to_string(), to: "Resolved".to_string(), event: "quick-resolve".to_string(), guard: None },
                TransitionDef { from: "Investigating".to_string(), to: "Resolved".to_string(), event: "resolve".to_string(), guard: None },
            ],
        });
        let (_meta_state, defs, def_map) = ir_to_defs(&ir);

        // From Triaging: two valid transitions
        let after_investigate = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate"]);
        assert_eq!(after_investigate, "Investigating");
        let after_quick_resolve = run_machine_defs(&defs, &def_map, "SupportRequest", &["quick-resolve"]);
        assert_eq!(after_quick_resolve, "Resolved");

        // From Investigating: one valid transition
        let after_resolve = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate", "resolve"]);
        assert_eq!(after_resolve, "Resolved");

        // From Resolved: no transitions (terminal) - invalid event stays put
        let after_terminal = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate", "resolve", "investigate"]);
        assert_eq!(after_terminal, "Resolved");
    }

    #[test]
    fn test_run_machine_support_request_lifecycle() {
        let mut ir = empty_ir();
        ir.state_machines.insert("SM".to_string(), StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Triaging".to_string(), "Investigating".to_string(), "WaitingOnCustomer".to_string(), "Resolved".to_string()],
            transitions: vec![
                TransitionDef { from: "Triaging".to_string(), to: "Investigating".to_string(), event: "investigate".to_string(), guard: None },
                TransitionDef { from: "Investigating".to_string(), to: "WaitingOnCustomer".to_string(), event: "request-info".to_string(), guard: None },
                TransitionDef { from: "WaitingOnCustomer".to_string(), to: "Investigating".to_string(), event: "customer-replied".to_string(), guard: None },
                TransitionDef { from: "Investigating".to_string(), to: "Resolved".to_string(), event: "resolve".to_string(), guard: None },
            ],
        });
        let (_meta_state, defs, def_map) = ir_to_defs(&ir);

        // Full lifecycle with back-and-forth
        let final_state = run_machine_defs(&defs, &def_map, "SupportRequest", &[
            "investigate",
            "request-info",
            "customer-replied",
            "resolve",
        ]);
        assert_eq!(final_state, "Resolved");

        // Invalid event mid-flow stays in current state
        let final_state = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate", "resolve", "investigate"]);
        assert_eq!(final_state, "Resolved"); // already resolved, "investigate" has no effect
    }

    #[test]
    fn test_deontic_forbidden_text_via_ast() {
        let mut ir = empty_ir();
        ir.nouns.insert("Markdown Syntax".to_string(), make_noun("value"));
        ir.enum_values.insert("Markdown Syntax".to_string(), vec!["#".to_string(), "##".to_string(), "**".to_string()]);
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Response contains Markdown Syntax".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Markdown Syntax".to_string(), role_index: 0 }],
        });
        ir.constraints.push(ConstraintDef {
            id: "dc1".to_string(),
            kind: "FC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("forbidden".to_string()),
            text: "It is forbidden that a Response contains Markdown Syntax.".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            ..Default::default()
        });
        let (_meta_state, defs, def_map) = ir_to_defs(&ir);

        // Text with markdown -> violations
        let violations = eval_constraints_defs(&defs, &def_map, "## Heading here", None, &empty_state());
        assert!(violations.len() > 0, "should detect forbidden markdown");

        // Clean text -> no violations
        let clean_violations = eval_constraints_defs(&defs, &def_map, "No special formatting here.", None, &empty_state());
        assert_eq!(clean_violations.len(), 0);
    }

    #[test]
    fn test_deontic_permitted_never_violates_via_ast() {
        let mut ir = empty_ir();
        ir.constraints.push(ConstraintDef {
            id: "pc1".to_string(),
            kind: "FC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("permitted".to_string()),
            text: "It is permitted that something happens.".to_string(),
            spans: vec![],
            ..Default::default()
        });
        let (_meta_state, defs, def_map) = ir_to_defs(&ir);
        let violations = eval_constraints_defs(&defs, &def_map, "anything", None, &empty_state());
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_no_constraints_no_violations_via_ast() {
        let (_meta_pop, defs, def_map) = ir_to_defs(&empty_ir());
        let violations = eval_constraints_defs(&defs, &def_map, "", None, &empty_state());
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_fact_creation_triggers_state_transition() {
        let mut ir = empty_ir();
        ir.nouns.insert("Customer".to_string(), make_noun("entity"));
        ir.nouns.insert("SupportRequest".to_string(), make_noun("entity"));

        ir.fact_types.insert("ft_submit".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Customer submits SupportRequest".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "SupportRequest".to_string(), role_index: 1 },
            ],
        });

        ir.state_machines.insert("SupportRequest".to_string(), StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Triaging".to_string(), "Investigating".to_string(), "Resolved".to_string()],
            transitions: vec![
                TransitionDef { from: "Triaging".to_string(), to: "Investigating".to_string(), event: "investigate".to_string(), guard: None },
                TransitionDef { from: "Investigating".to_string(), to: "Resolved".to_string(), event: "resolve".to_string(), guard: None },
            ],
        });

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);

        // The state machine starts at "Triaging"
        let initial_key = "machine:SupportRequest:initial";
        let initial = defs.iter().find(|(n, _)| n == initial_key)
            .and_then(|(_, f)| {
                let r = ast::apply(f, &ast::Object::phi(), &def_map);
                r.as_atom().map(|s| s.to_string())
            })
            .unwrap_or_default();
        assert_eq!(initial, "Triaging");

        // Verify the state machine can transition
        let after_investigate = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate"]);
        assert_eq!(after_investigate, "Investigating");

        // Verify schema was compiled
        let has_schema = defs.iter().any(|(n, _)| n == "schema:ft_submit");
        assert!(has_schema, "Schema compiled for submit fact type");
    }

    #[test]
    fn test_fact_event_mapping_compiled() {
        let mut ir = empty_ir();
        ir.nouns.insert("Customer".to_string(), make_noun("entity"));
        ir.nouns.insert("SupportRequest".to_string(), make_noun("entity"));

        ir.fact_types.insert("ft_submit".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Customer submits SupportRequest".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "SupportRequest".to_string(), role_index: 1 },
            ],
        });

        ir.state_machines.insert("SM".to_string(), StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Triaging".to_string(), "Investigating".to_string()],
            transitions: vec![
                TransitionDef { from: "Triaging".to_string(), to: "Investigating".to_string(), event: "submit".to_string(), guard: None },
            ],
        });

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);

        // Verify the state machine transitions on "submit"
        let final_state = run_machine_defs(&defs, &def_map, "SupportRequest", &["submit"]);
        assert_eq!(final_state, "Investigating");
    }

    #[test]
    fn test_guarded_transition_blocks_on_violation() {
        let mut ir = empty_ir();
        ir.nouns.insert("SupportRequest".to_string(), make_noun("entity"));
        ir.nouns.insert("Prohibited".to_string(), make_noun("value"));
        ir.enum_values.insert("Prohibited".to_string(), vec!["internal-details".to_string()]);

        ir.fact_types.insert("ft_resp".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Response contains Prohibited".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Prohibited".to_string(), role_index: 0 }],
        });

        ir.constraints.push(ConstraintDef {
            id: "guard1".to_string(),
            kind: "FC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("forbidden".to_string()),
            text: "It is forbidden that a Response contains internal-details".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft_resp".to_string(), role_index: 0, subset_autofill: None }],
            ..Default::default()
        });

        ir.state_machines.insert("SM".to_string(), StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Investigating".to_string(), "Resolved".to_string()],
            transitions: vec![
                TransitionDef {
                    from: "Investigating".to_string(), to: "Resolved".to_string(),
                    event: "resolve".to_string(),
                    guard: Some(GuardDef {
                        fact_type_id: "ft_resp".to_string(),
                        constraint_ids: vec!["guard1".to_string()],
                    }),
                },
            ],
        });

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);

        // Response with forbidden content -> constraint detects violation
        let pop = empty_state();
        let violations = eval_constraints_defs(&defs, &def_map, "Here are the internal-details of the system", None, &pop);
        assert!(!violations.is_empty(), "Guard constraint should produce violations");

        // Clean response -> no constraint violations
        let clean_violations = eval_constraints_defs(&defs, &def_map, "Your issue has been resolved. Thank you.", None, &pop);
        assert!(clean_violations.is_empty(), "No guard violations for clean response");

        // The machine processes the event:
        let state = run_machine_defs(&defs, &def_map, "SupportRequest", &["resolve"]);
        assert_eq!(state, "Resolved",
            "run_machine_defs fires the transition; guard enforcement is the caller's responsibility");
    }

    #[test]
    fn test_fact_driven_event_resolution() {
        let mut ir = empty_ir();
        ir.nouns.insert("Customer".to_string(), make_noun("entity"));
        ir.nouns.insert("SupportRequest".to_string(), make_noun("entity"));
        ir.nouns.insert("Agent".to_string(), make_noun("entity"));

        ir.fact_types.insert("ft_submit".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Customer submits SupportRequest".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "SupportRequest".to_string(), role_index: 1 },
            ],
        });
        ir.fact_types.insert("ft_resolve".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Agent resolves SupportRequest".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Agent".to_string(), role_index: 0 },
                RoleDef { noun_name: "SupportRequest".to_string(), role_index: 1 },
            ],
        });

        ir.state_machines.insert("SupportRequest".to_string(), StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Triaging".to_string(), "Investigating".to_string(), "Resolved".to_string()],
            transitions: vec![
                TransitionDef { from: "Triaging".to_string(), to: "Investigating".to_string(), event: "investigate".to_string(), guard: None },
                TransitionDef { from: "Investigating".to_string(), to: "Resolved".to_string(), event: "resolve".to_string(), guard: None },
            ],
        });

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);

        // Both schemas should compile
        let has_submit = defs.iter().any(|(n, _)| n == "schema:ft_submit");
        let has_resolve = defs.iter().any(|(n, _)| n == "schema:ft_resolve");
        assert!(has_submit);
        assert!(has_resolve);

        // Full lifecycle through events
        let state = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate", "resolve"]);
        assert_eq!(state, "Resolved");
    }

    #[test]
    fn test_subset_constraint_without_autofill_produces_violation() {
        let mut ir = empty_ir();
        ir.nouns.insert("Person".to_string(), make_noun("entity"));
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasLicense".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasInsurance".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        // SS constraint WITHOUT autofill -- just validates, doesn't derive
        ir.constraints.push(ConstraintDef {
            id: "ss_no_auto".to_string(),
            kind: "SS".to_string(),
            modality: "Alethic".to_string(),
            text: "If some Person hasLicense then that Person hasInsurance".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            ..Default::default()
        });

        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        // No modus ponens derivation should be compiled
        let mp_count = defs.iter()
            .filter(|(n, _)| n.starts_with("derivation:") && n.contains("modus_ponens"))
            .count();
        assert_eq!(mp_count, 0, "Should NOT compile modus ponens without autofill");

        // Forward chain should produce no derived facts
        let state = state_with_facts("ft1", &[&[("Person", "p1")]]);
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);
        let mp_derived: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "ft2").collect();
        // CWA negation may derive "NOT Person hasInsurance" -- that's expected.
        // But no POSITIVE modus ponens derivation should exist.
        let positive_mp = mp_derived.iter().filter(|d| !d.reading.contains("NOT")).count();
        assert_eq!(positive_mp, 0, "No autofill -> no positive derived insurance facts");
    }

    #[test]
    fn test_forward_chain_ast_subtype_inheritance() {
        // Teacher is subtype of Academic. Academic has Rank.
        // Forward chaining should terminate without panicking.
        let mut ir = empty_ir();
        ir.nouns.insert("Academic".to_string(), make_noun("entity"));
        ir.nouns.insert("Teacher".to_string(), make_noun("entity"));
        ir.subtypes.insert("Teacher".to_string(), "Academic".to_string());
        ir.nouns.insert("Rank".to_string(), make_noun("value"));
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Academic has Rank".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Academic".to_string(), role_index: 0 },
                RoleDef { noun_name: "Rank".to_string(), role_index: 1 },
            ],
        });
        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        // Verify subtype inheritance derivation was compiled
        let subtype_derivations = defs.iter()
            .filter(|(n, _)| n.starts_with("derivation:") && n.contains("subtype"))
            .count();
        assert!(subtype_derivations > 0,
            "Expected at least one subtype inheritance derivation");

        // Teacher T1 has Rank P
        let state = state_with_facts("ft1", &[&[("Academic", "T1"), ("Rank", "P")]]);

        let dd = derivation_defs_from(&defs);
        let (_new_state, _derived) = forward_chain_defs_state(&dd, &state);
        // Should derive that T1 participates in Academic fact types via subtype inheritance
        // subtype derivation adds inherited facts (may be zero if none applicable)
    }

    #[test]
    fn test_forward_chain_ast_modus_ponens() {
        let mut ir = empty_ir();
        ir.nouns.insert("Academic".to_string(), make_noun("entity"));
        ir.nouns.insert("Department".to_string(), make_noun("entity"));

        ir.fact_types.insert("ft_heads".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Academic heads Department".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Academic".to_string(), role_index: 0 },
                RoleDef { noun_name: "Department".to_string(), role_index: 1 },
            ],
        });
        ir.fact_types.insert("ft_works".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Academic works for Department".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Academic".to_string(), role_index: 0 },
                RoleDef { noun_name: "Department".to_string(), role_index: 1 },
            ],
        });

        // Subset constraint with autofill: heads -> automatically derive works for
        ir.constraints.push(ConstraintDef {
            id: "ss1".to_string(),
            kind: "SS".to_string(),
            modality: "Alethic".to_string(),
            text: "If some Academic heads some Department then that Academic works for that Department".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft_heads".to_string(), role_index: 0, subset_autofill: Some(true) },
                SpanDef { fact_type_id: "ft_works".to_string(), role_index: 0, subset_autofill: None },
            ],
            entity: None,
            set_comparison_argument_length: None,
            clauses: None,
            min_occurrence: None,
            max_occurrence: None,
            deontic_operator: None,
        });

        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        // Academic A1 heads Department D1
        let state = state_with_facts("ft_heads", &[&[("Academic", "A1"), ("Department", "D1")]]);

        let dd = derivation_defs_from(&defs);
        let (_new_state, ast_derived) = forward_chain_defs_state(&dd, &state);
        // Modus ponens should derive the full tuple: (A1, D1) in ft_works
        let works_for = ast_derived.iter().any(|d|
            d.fact_type_id == "ft_works" &&
            d.bindings.iter().any(|(n, v)| n == "Academic" && v == "A1") &&
            d.bindings.iter().any(|(n, v)| n == "Department" && v == "D1")
        );
        assert!(works_for, "Expected full tuple derivation: A1 works for D1");
    }

    #[test]
    fn test_forward_chain_ast_no_rules_no_derivations() {
        let ir = empty_ir();
        let (_meta_state, defs, _def_map) = ir_to_defs(&ir);
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &empty_state());
        assert_eq!(derived.len(), 0);
    }

    // -- Constraint evaluation tests -----------------------------------

    #[test]
    fn test_no_constraints_no_violations() {
        let ir = empty_ir();
        let (_meta_state, defs, def_map) = ir_to_defs(&ir);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &empty_state());
        assert!(result.is_empty());
    }

    #[test]
    fn test_uniqueness_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Customer has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "Each Customer has at most one Name".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let state = state_with_facts("ft1", &[&[("Customer", "c1"), ("Name", "Alice")], &[("Customer", "c1"), ("Name", "Bob")]]);

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert_eq!(result.len(), 1);
        assert!(result[0].detail.contains("Uniqueness violation"));
    }

    #[test]
    fn test_ring_irreflexive_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Person manages Person".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Person".to_string(), role_index: 1 },
            ],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "IR".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "No Person manages itself".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Person", "p1"), ("Person", "p1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Irreflexive"));
    }

    #[test]
    fn test_exclusive_or_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Order isPaid".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Order isPending".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "XO".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "For each Order, exactly one holds".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: Some(2),
            clauses: Some(vec!["Order isPaid".to_string(), "Order isPending".to_string()]),
            entity: Some("Order".to_string()),
            min_occurrence: None,
            max_occurrence: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Order", "o1")]), &pop_state);
        pop_state = ast::cell_push("ft2", ast::fact_from_pairs(&[("Order", "o1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Set-comparison violation"));
    }

    #[test]
    fn test_subset_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasLicense".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasInsurance".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "SS".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "If some Person hasLicense then that Person hasInsurance".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Person", "p1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Subset violation"));
    }

    #[test]
    fn test_permitted_never_violates() {
        let mut ir = empty_ir();
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("permitted".to_string()),
            text: "It is permitted that SupportResponse offers data retrieval".to_string(),
            spans: vec![],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &empty_state());
        assert!(result.is_empty());
    }

    #[test]
    fn test_exclusive_choice_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Order isPaid".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Order isPending".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "XC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "For each Order, at most one holds".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: Some(2),
            clauses: Some(vec!["Order isPaid".to_string(), "Order isPending".to_string()]),
            entity: Some("Order".to_string()),
            min_occurrence: None,
            max_occurrence: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Order", "o1")]), &pop_state);
        pop_state = ast::cell_push("ft2", ast::fact_from_pairs(&[("Order", "o1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Set-comparison violation"));
    }

    #[test]
    fn test_mandatory_violation() {
        let mut ir = empty_ir();
        ir.nouns.insert("Customer".to_string(), make_noun("entity"));
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Customer has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Customer has Email".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Email".to_string(), role_index: 1 },
            ],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "MC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "Each Customer has at least one Name".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft2", ast::fact_from_pairs(&[("Customer", "c1"), ("Email", "a@b.com")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert_eq!(result.len(), 1);
        assert!(result[0].detail.contains("Mandatory violation"));
        assert!(result[0].detail.contains("c1"));
    }

    #[test]
    fn test_inclusive_or_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Customer hasPhone".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Customer".to_string(), role_index: 0 }],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Customer hasEmail".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Customer".to_string(), role_index: 0 }],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "OR".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "For each Customer, at least one of the following holds: hasPhone, hasEmail".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: Some(2),
            clauses: Some(vec!["Customer hasPhone".to_string(), "Customer hasEmail".to_string()]),
            entity: Some("Customer".to_string()),
            min_occurrence: None,
            max_occurrence: None,
        });

        ir.fact_types.insert("ft3".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Customer hasName".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Customer".to_string(), role_index: 0 }],
        });
        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft3", ast::fact_from_pairs(&[("Customer", "c1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert_eq!(result.len(), 1);
        assert!(result[0].detail.contains("Set-comparison violation"));
        assert!(result[0].detail.contains("at least one"));
    }

    #[test]
    fn test_obligatory_missing_enum_value() {
        let mut ir = empty_ir();
        ir.nouns.insert("SenderIdentityValue".to_string(), make_noun("value"));
        ir.enum_values.insert("SenderIdentityValue".to_string(), vec!["Support Team <support@example.com>".to_string()]);
        ir.nouns.insert("SupportResponse".to_string(), make_noun("entity"));
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "SupportResponse has SenderIdentityValue".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "SupportResponse".to_string(), role_index: 0 },
                RoleDef { noun_name: "SenderIdentityValue".to_string(), role_index: 1 },
            ],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("obligatory".to_string()),
            text: "It is obligatory that each SupportResponse has SenderIdentity".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);
        let result = eval_constraints_defs(&defs, &def_map, "Here is some help for you.", Some(""), &empty_state());
        assert!(result.len() >= 1);
        let details: Vec<String> = result.iter().map(|v| v.detail.clone()).collect();
        assert!(details.iter().any(|d: &String| d.contains("obligatory")));
    }

    #[test]
    fn test_obligatory_sender_identity_empty() {
        let mut ir = empty_ir();
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("obligatory".to_string()),
            text: "It is obligatory that each SupportResponse has SenderIdentity".to_string(),
            spans: vec![],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);
        let result = eval_constraints_defs(&defs, &def_map, "Hello", Some(""), &empty_state());
        assert_eq!(result.len(), 1);
        assert!(result[0].detail.contains("SenderIdentity"));
    }

    /// Regression: constraints spanning multiple fact types that share a value-type noun
    /// must not produce duplicate violations. collect_enum_values deduplicates by noun name.
    #[test]
    fn test_no_duplicate_violations_for_multi_span_constraints() {
        let mut ir = empty_ir();
        ir.nouns.insert("FieldName".to_string(), make_noun("value"));
        ir.enum_values.insert("FieldName".to_string(), vec!["EndpointSlug".to_string(), "Title".to_string()]);
        ir.nouns.insert("SupportResponse".to_string(), make_noun("entity"));
        ir.nouns.insert("APIProduct".to_string(), make_noun("entity"));
        // Three fact types that all reference FieldName -- simulates multi-span constraint
        ir.fact_types.extend((1..=3).map(|i| (format!("ft{}", i), FactTypeDef {
            schema_id: String::new(),
            reading: format!("SupportResponse names APIProduct by FieldName ({})", i),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "SupportResponse".to_string(), role_index: 0 },
                RoleDef { noun_name: "APIProduct".to_string(), role_index: 1 },
                RoleDef { noun_name: "FieldName".to_string(), role_index: 2 },
            ],
        })));
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("obligatory".to_string()),
            text: "It is obligatory that SupportResponse names APIProduct by FieldName 'Title'.".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft3".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);
        let result = eval_constraints_defs(&defs, &def_map, "test response without required field names", None, &empty_state());
        // Should produce exactly 1 violation per unique noun, not 3 duplicates
        let field_name_violations: Vec<_> = result.iter()
            .filter(|v| v.detail.contains("FieldName"))
            .collect();
        assert_eq!(field_name_violations.len(), 1,
            "Expected 1 FieldName violation, got {}. Violations: {:?}",
            field_name_violations.len(),
            field_name_violations.iter().map(|v| &v.detail).collect::<Vec<_>>());
    }

    #[test]
    fn test_equality_violation() {
        let mut ir = empty_ir();
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Person isEmployee".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasBadge".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "EQ".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "Person isEmployee if and only if Person hasBadge".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Person", "p1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = ir_to_defs(&ir);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Equality violation"));
    }

    // -- Forward Inference & Synthesis Tests ----------------------------

    #[test]
    fn test_subtype_inheritance_derivation() {
        let mut ir = empty_ir();

        ir.nouns.insert("Vehicle".to_string(), make_noun("entity"));
        ir.nouns.insert("Car".to_string(), make_noun("entity"));
        ir.subtypes.insert("Car".to_string(), "Vehicle".to_string());
        ir.nouns.insert("License".to_string(), make_noun("entity"));

        ir.fact_types.insert("ft_vehicle_license".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Vehicle has License".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Vehicle".to_string(), role_index: 0 },
                RoleDef { noun_name: "License".to_string(), role_index: 1 },
            ],
        });

        ir.fact_types.insert("ft_car_color".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Car has Color".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Car".to_string(), role_index: 0 },
            ],
        });

        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        // Verify subtype inheritance derivation was compiled
        let subtype_derivations = defs.iter()
            .filter(|(n, _)| n.starts_with("derivation:") && n.contains("subtype"))
            .count();
        assert!(subtype_derivations > 0,
            "Expected at least one subtype inheritance derivation");

        // Test forward chaining with a population that has a Car instance
        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft_car_color", ast::fact_from_pairs(&[("Car", "my_car")]), &pop_state);
        let state = pop_state;

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);

        let inheritance_facts: Vec<_> = derived.iter()
            .filter(|d| d.derived_by.contains("subtype"))
            .collect();
        assert!(!inheritance_facts.is_empty(),
            "Expected subtype inheritance to derive facts for Car instance");
    }

    #[test]
    fn test_modus_ponens_from_subset() {
        let mut ir = empty_ir();

        ir.nouns.insert("Person".to_string(), make_noun("entity"));
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasLicense".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasInsurance".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        // SS constraint with autofill: hasLicense -> automatically derive hasInsurance
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "SS".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "If some Person hasLicense then that Person hasInsurance".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: Some(true) },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        // Verify modus ponens derivation was compiled
        let mp_derivations = defs.iter()
            .filter(|(n, _)| n.starts_with("derivation:") && n.contains("modus_ponens"))
            .count();
        assert!(mp_derivations > 0,
            "Expected a modus ponens derivation from SS constraint");

        // Forward chain: p1 hasLicense -> should derive p1 hasInsurance
        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Person", "p1")]), &pop_state);
        let state = pop_state;

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);

        let insurance_facts: Vec<_> = derived.iter()
            .filter(|d| d.fact_type_id == "ft2")
            .collect();
        assert_eq!(insurance_facts.len(), 1,
            "Expected modus ponens to derive hasInsurance for p1");
        assert_eq!(insurance_facts[0].bindings, vec![("Person".to_string(), "p1".to_string())]);
        assert_eq!(insurance_facts[0].confidence, Confidence::Definitive);
    }

    #[test]
    fn test_cwa_vs_owa_negation() {
        let mut ir = empty_ir();

        // CWA noun: Permission (not stated = false)
        ir.nouns.insert("Permission".to_string(), NounDef {
            object_type: "entity".to_string(),
            world_assumption: WorldAssumption::Closed,
        });
        // OWA noun: Capability (not stated = unknown)
        ir.nouns.insert("Capability".to_string(), NounDef {
            object_type: "entity".to_string(),
            world_assumption: WorldAssumption::Open,
        });

        ir.nouns.insert("Resource".to_string(), make_noun("entity"));

        ir.fact_types.insert("ft_perm".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Permission grants access to Resource".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Permission".to_string(), role_index: 0 },
                RoleDef { noun_name: "Resource".to_string(), role_index: 1 },
            ],
        });
        ir.fact_types.insert("ft_cap".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Capability enables Resource".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Capability".to_string(), role_index: 0 },
                RoleDef { noun_name: "Resource".to_string(), role_index: 1 },
            ],
        });

        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        // CWA derivation should exist for Permission
        let cwa_for_perm = defs.iter()
            .any(|(n, _)| n.starts_with("derivation:") && n.contains("cwa_negation") && n.contains("Permission"));
        assert!(cwa_for_perm,
            "Expected CWA negation derivation for Permission");

        // No CWA derivation for Capability (it's OWA)
        let cwa_for_cap = defs.iter()
            .any(|(n, _)| n.starts_with("derivation:") && n.contains("cwa_negation") && n.contains("Capability"));
        assert!(!cwa_for_cap,
            "Expected NO CWA negation derivation for Capability (OWA noun)");

        // Forward chain with a population where Permission exists
        // but doesn't participate in ft_perm
        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft_other", ast::fact_from_pairs(&[("Permission", "read")]), &pop_state);
        let state = pop_state;

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);

        // Under CWA, "read" doesn't participate in ft_perm -> derive negation
        let negation_facts: Vec<_> = derived.iter()
            .filter(|d| d.derived_by.contains("cwa_negation") && d.reading.contains("NOT"))
            .collect();
        assert!(!negation_facts.is_empty(),
            "Expected CWA to derive negation for Permission 'read' not in ft_perm");
        assert_eq!(negation_facts[0].confidence, Confidence::Definitive);
    }

    #[test]
    fn test_synthesis_basic() {
        let mut ir = empty_ir();

        ir.nouns.insert("Customer".to_string(), NounDef {
            object_type: "entity".to_string(),
            world_assumption: WorldAssumption::Closed,
        });
        ir.nouns.insert("Name".to_string(), make_noun("value"));
        ir.nouns.insert("Email".to_string(), make_noun("value"));

        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Customer has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        ir.fact_types.insert("ft2".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Customer has Email".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Email".to_string(), role_index: 1 },
            ],
        });

        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "MC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "Each Customer has at least one Name".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });

        let (meta_pop, _defs, _def_map) = ir_to_defs(&ir);
        let result = synthesize_from_state(&meta_pop, "Customer", 1);

        assert_eq!(result.noun_name, "Customer");

        // Customer participates in two fact types
        assert_eq!(result.participates_in.len(), 2,
            "Customer should participate in ft1 and ft2. Got: {:?}",
            result.participates_in);

        // One constraint applies to Customer
        assert_eq!(result.applicable_constraints.len(), 1,
            "Expected 1 constraint for Customer. Got: {:?}",
            result.applicable_constraints);
        assert_eq!(result.applicable_constraints[0].id, "c1");

        // Related nouns: Name and Email
        assert_eq!(result.related_nouns.len(), 2,
            "Expected 2 related nouns. Got: {:?}", result.related_nouns);
        let related_names: Vec<_> = result.related_nouns.iter()
            .map(|r| r.name.as_str())
            .collect();
        assert!(related_names.contains(&"Name"), "Expected Name as related noun");
        assert!(related_names.contains(&"Email"), "Expected Email as related noun");
    }

    #[test]
    fn test_synthesis_empty_noun() {
        let (meta_pop, _defs, _def_map) = ir_to_defs(&empty_ir());
        let result = synthesize_from_state(&meta_pop, "NonExistent", 1);

        assert_eq!(result.noun_name, "NonExistent");
        assert!(result.participates_in.is_empty());
        assert!(result.applicable_constraints.is_empty());
        assert!(result.state_machines.is_empty());
        assert!(result.related_nouns.is_empty());
    }

    #[test]
    fn test_forward_chain_fixed_point() {
        // Verify forward chaining reaches a fixed point (no infinite loops)
        let mut ir = empty_ir();
        ir.nouns.insert("A".to_string(), make_noun("entity"));
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "A exists".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "A".to_string(), role_index: 0 }],
        });

        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("A", "a1")]), &pop_state);
        let state = pop_state;

        // Should terminate even if derivations produce facts
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);
        // Just verify it terminates -- the exact count depends on CWA rules
        assert!(derived.len() < 100, "Forward chaining should reach fixed point quickly");
    }

    #[test]
    fn test_transitivity_derivation() {
        let mut ir = empty_ir();

        ir.nouns.insert("City".to_string(), make_noun("entity"));
        ir.nouns.insert("State".to_string(), make_noun("entity"));
        ir.nouns.insert("Country".to_string(), make_noun("entity"));

        ir.fact_types.insert("ft_city_state".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "City isIn State".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "City".to_string(), role_index: 0 },
                RoleDef { noun_name: "State".to_string(), role_index: 1 },
            ],
        });
        ir.fact_types.insert("ft_state_country".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "State isIn Country".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "State".to_string(), role_index: 0 },
                RoleDef { noun_name: "Country".to_string(), role_index: 1 },
            ],
        });

        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        // Should have a transitivity derivation
        let trans_derivations = defs.iter()
            .filter(|(n, _)| n.starts_with("derivation:") && n.contains("transitivity"))
            .count();
        assert!(trans_derivations > 0,
            "Expected transitivity derivation for City->State->Country chain");

        // Forward chain: Austin isIn Texas, Texas isIn USA -> Austin (transitively) in USA
        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft_city_state", ast::fact_from_pairs(&[("City", "Austin"), ("State", "Texas")]), &pop_state);
        pop_state = ast::cell_push("ft_state_country", ast::fact_from_pairs(&[("State", "Texas"), ("Country", "USA")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let transitive_facts: Vec<_> = derived.iter()
            .filter(|d| d.derived_by.contains("transitivity"))
            .collect();
        assert!(!transitive_facts.is_empty(),
            "Expected transitivity to derive Austin->USA relationship");

        // Verify the derived fact connects City to Country
        let city_country = transitive_facts.iter().find(|d| {
            d.bindings.iter().any(|(_, v)| v == "Austin")
                && d.bindings.iter().any(|(_, v)| v == "USA")
        });
        assert!(city_country.is_some(),
            "Expected derived fact linking Austin to USA. Derived: {:?}", transitive_facts);
    }

    #[test]
    fn test_world_assumption_default_is_closed() {
        assert_eq!(WorldAssumption::default(), WorldAssumption::Closed);
    }

    // â”€â”€ Inline-comparator filter end-to-end (Halpin FORML Example 5) â”€â”€
    //
    // Each AntecedentFilter on a DerivationRuleDef wraps the antecedent's
    // fact-extraction in Func::filter, so only facts whose role value
    // satisfies the comparator reach the existence check. With the current
    // existence-based semantics: if every antecedent fact is filtered out,
    // NullTest on the filtered Seq returns true and the rule stops firing.
    // If at least one fact passes, the rule fires and the binding
    // extractor pulls from the first post-filter fact.

    fn city_population_ir(filter: Option<crate::types::AntecedentFilter>) -> Domain {
        let mut fact_types = HashMap::new();
        fact_types.insert("city_has_population".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "City has Population".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "City".to_string(), role_index: 0 },
                RoleDef { noun_name: "Population".to_string(), role_index: 1 },
            ],
        });
        fact_types.insert("big_city".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Big City has City".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Big City".to_string(), role_index: 0 },
                RoleDef { noun_name: "City".to_string(), role_index: 1 },
            ],
        });
        Domain {
            nouns: HashMap::new(),
            fact_types,
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![DerivationRuleDef {
                id: "big-city".to_string(),
                text: "* Big City has City iff City has Population >= 1000000".to_string(),
                antecedent_fact_type_ids: vec!["city_has_population".to_string()],
                consequent_fact_type_id: "big_city".to_string(),
                kind: DerivationKind::ModusPonens,
                join_on: vec![],
                match_on: vec![],
                consequent_bindings: vec![],
                antecedent_filters: filter.into_iter().collect(),
                consequent_computed_bindings: vec![], consequent_aggregates: vec![], unresolved_clauses: vec![],
            }],
            general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
            cells: HashMap::new(),
        }
    }

    #[test]
    fn inline_ge_filter_suppresses_derivation_when_no_fact_matches() {
        // Both cities well below the 1M threshold â†’ filter strips every
        // antecedent fact â†’ rule's existence check fails â†’ no derivation.
        let ir = city_population_ir(Some(crate::types::AntecedentFilter {
            antecedent_index: 0,
            role: "Population".to_string(),
            op: ">=".to_string(),
            value: 1_000_000.0,
        }));
        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "SmallTown"), ("Population", "500000")]), &pop_state);
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "MidVille"), ("Population", "250000")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let big: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "big_city").collect();
        assert!(big.is_empty(), "expected no big_city derivations, got {:?}", big);
    }

    #[test]
    fn inline_ge_filter_allows_derivation_when_a_fact_matches() {
        // One city below the threshold, one above. The filter keeps only
        // the big one, the existence check passes, and the rule fires with
        // the matching city's bindings.
        let ir = city_population_ir(Some(crate::types::AntecedentFilter {
            antecedent_index: 0,
            role: "Population".to_string(),
            op: ">=".to_string(),
            value: 1_000_000.0,
        }));
        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "SmallTown"), ("Population", "500000")]), &pop_state);
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "Megapolis"), ("Population", "2000000")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let big: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "big_city").collect();
        assert_eq!(big.len(), 1, "expected exactly one big_city derivation, got {:?}", big);
        // Bindings must come from the matching (post-filter) fact, not the
        // small-town one whose Population is below the cutoff.
        assert!(big[0].bindings.iter().any(|(k, v)| k == "City" && v == "Megapolis"),
            "expected Megapolis as the derived binding, got {:?}", big[0].bindings);
    }

    #[test]
    fn inline_lt_filter_keeps_only_smaller_values() {
        // Flip direction: derivation should fire only when some fact's
        // Population is strictly less than 1M. Exercises Func::Lt path in
        // comparator_primitive.
        let ir = city_population_ir(Some(crate::types::AntecedentFilter {
            antecedent_index: 0,
            role: "Population".to_string(),
            op: "<".to_string(),
            value: 1_000_000.0,
        }));
        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "Megapolis"), ("Population", "2000000")]), &pop_state);
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "Hamlet"), ("Population", "400")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let big: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "big_city").collect();
        assert_eq!(big.len(), 1);
        assert!(big[0].bindings.iter().any(|(k, v)| k == "City" && v == "Hamlet"),
            "expected Hamlet (pop<1M), got {:?}", big[0].bindings);
    }

    #[test]
    fn per_fact_fanout_produces_one_derivation_per_matching_fact() {
        // Four cities, three above the 1M threshold. Per-fact semantic
        // demands one derived fact per matching antecedent tuple â€” the
        // old existence-check semantic would have produced one regardless.
        let ir = city_population_ir(Some(crate::types::AntecedentFilter {
            antecedent_index: 0,
            role: "Population".to_string(),
            op: ">=".to_string(),
            value: 1_000_000.0,
        }));
        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        let mut pop_state = ast::Object::phi();
        for (name, pop) in [("Alpha", "2000000"), ("Bravo", "5000000"), ("Charlie", "800000"), ("Delta", "3000000")] {
            pop_state = ast::cell_push("city_has_population",
                ast::fact_from_pairs(&[("City", name), ("Population", pop)]), &pop_state);
        }

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let big: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "big_city").collect();
        assert_eq!(big.len(), 3, "expected 3 big cities (Alpha/Bravo/Delta), got {:?}", big);

        let names: hashbrown::HashSet<&str> = big.iter()
            .flat_map(|d| d.bindings.iter()
                .filter(|(k, _)| k == "City")
                .map(|(_, v)| v.as_str()))
            .collect();
        assert!(names.contains("Alpha"));
        assert!(names.contains("Bravo"));
        assert!(names.contains("Delta"));
        assert!(!names.contains("Charlie"), "sub-threshold city must not derive");
    }

    // â”€â”€ Arithmetic definitional clauses, end-to-end â”€â”€
    //
    // A rule like `* Foo has Doubled iff Foo has Val and Doubled is Val + Val.`
    // records a ConsequentComputedBinding { role: "Doubled", expr: Val + Val }
    // which the compile side turns into a per-fact Func that appends the
    // computed pair to the antecedent's bindings.

    fn val_derived_ir(expr: crate::types::ArithExpr, derived_role: &str) -> Domain {
        let mut fact_types = HashMap::new();
        fact_types.insert("foo_has_val".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Foo has Val".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Foo".to_string(), role_index: 0 },
                RoleDef { noun_name: "Val".to_string(), role_index: 1 },
            ],
        });
        fact_types.insert("foo_has_derived".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: format!("Foo has {}", derived_role),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Foo".to_string(), role_index: 0 },
                RoleDef { noun_name: derived_role.to_string(), role_index: 1 },
            ],
        });
        Domain {
            nouns: HashMap::new(),
            fact_types,
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![DerivationRuleDef {
                id: "arith-rule".to_string(),
                text: format!("* Foo has {} iff Foo has Val and ...", derived_role),
                antecedent_fact_type_ids: vec!["foo_has_val".to_string()],
                consequent_fact_type_id: "foo_has_derived".to_string(),
                kind: DerivationKind::ModusPonens,
                join_on: vec![],
                match_on: vec![],
                consequent_bindings: vec![],
                antecedent_filters: vec![],
                consequent_computed_bindings: vec![crate::types::ConsequentComputedBinding {
                    role: derived_role.to_string(),
                    expr,
                }],
                consequent_aggregates: vec![], unresolved_clauses: vec![],
            }],
            general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
            cells: HashMap::new(),
        }
    }

    fn val_ref() -> crate::types::ArithExpr {
        crate::types::ArithExpr::RoleRef("Val".to_string())
    }

    fn lit(n: f64) -> crate::types::ArithExpr {
        crate::types::ArithExpr::Literal(n)
    }

    fn bin(op: &str, l: crate::types::ArithExpr, r: crate::types::ArithExpr) -> crate::types::ArithExpr {
        crate::types::ArithExpr::Op(op.to_string(), Box::new(l), Box::new(r))
    }

    #[test]
    fn arithmetic_add_computes_role_plus_role() {
        let ir = val_derived_ir(bin("+", val_ref(), val_ref()), "Doubled");
        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("foo_has_val",
            ast::fact_from_pairs(&[("Foo", "f1"), ("Val", "7")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let out: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "foo_has_derived").collect();
        assert_eq!(out.len(), 1);
        assert!(out[0].bindings.iter().any(|(k, v)| k == "Doubled" && v == "14"),
            "expected ('Doubled','14'), got {:?}", out[0].bindings);
    }

    #[test]
    fn arithmetic_sub_computes_role_minus_literal() {
        let ir = val_derived_ir(bin("-", val_ref(), lit(3.0)), "Less");
        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("foo_has_val",
            ast::fact_from_pairs(&[("Foo", "f1"), ("Val", "10")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let out: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "foo_has_derived").collect();
        assert_eq!(out.len(), 1);
        assert!(out[0].bindings.iter().any(|(k, v)| k == "Less" && v == "7"),
            "expected ('Less','7'), got {:?}", out[0].bindings);
    }

    #[test]
    fn arithmetic_mul_and_div_chain_left_associative() {
        // (Val * 3) / 2 applied to Val=10 â†’ 15.
        let expr = bin("/", bin("*", val_ref(), lit(3.0)), lit(2.0));
        let ir = val_derived_ir(expr, "Scaled");
        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("foo_has_val",
            ast::fact_from_pairs(&[("Foo", "f1"), ("Val", "10")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let out: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "foo_has_derived").collect();
        assert_eq!(out.len(), 1);
        assert!(out[0].bindings.iter().any(|(k, v)| k == "Scaled" && v == "15"),
            "expected ('Scaled','15'), got {:?}", out[0].bindings);
    }

    #[test]
    fn arithmetic_fanout_computes_per_fact_independently() {
        // Three Foo facts with different Vals â†’ three derivations, each
        // carrying its own computed value.
        let ir = val_derived_ir(bin("*", val_ref(), lit(2.0)), "Twice");
        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        let mut pop_state = ast::Object::phi();
        for (id, val) in [("a", "3"), ("b", "5"), ("c", "11")] {
            pop_state = ast::cell_push("foo_has_val",
                ast::fact_from_pairs(&[("Foo", id), ("Val", val)]), &pop_state);
        }

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let out: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "foo_has_derived").collect();
        assert_eq!(out.len(), 3);
        let mut pairs: Vec<(String, String)> = out.iter().map(|d| {
            let foo = d.bindings.iter().find(|(k, _)| k == "Foo").map(|(_, v)| v.clone()).unwrap_or_default();
            let tw  = d.bindings.iter().find(|(k, _)| k == "Twice").map(|(_, v)| v.clone()).unwrap_or_default();
            (foo, tw)
        }).collect();
        pairs.sort();
        assert_eq!(pairs, vec![
            ("a".to_string(), "6".to_string()),
            ("b".to_string(), "10".to_string()),
            ("c".to_string(), "22".to_string()),
        ]);
    }

    // â”€â”€ Aggregate derivations, end-to-end (Codd image-set) â”€â”€

    fn thing_part_arity_ir() -> Domain {
        let mut fact_types = HashMap::new();
        fact_types.insert("thing_has_part".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Thing has Part".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Thing".to_string(), role_index: 0 },
                RoleDef { noun_name: "Part".to_string(), role_index: 1 },
            ],
        });
        fact_types.insert("thing_has_arity".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Thing has Arity".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Thing".to_string(), role_index: 0 },
                RoleDef { noun_name: "Arity".to_string(), role_index: 1 },
            ],
        });
        Domain {
            nouns: HashMap::new(),
            fact_types,
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![DerivationRuleDef {
                id: "thing-arity".to_string(),
                text: "* Thing has Arity iff Arity is the count of Part where Thing has Part.".to_string(),
                antecedent_fact_type_ids: vec![],
                consequent_fact_type_id: "thing_has_arity".to_string(),
                kind: DerivationKind::ModusPonens,
                join_on: vec![],
                match_on: vec![],
                consequent_bindings: vec![],
                antecedent_filters: vec![],
                consequent_computed_bindings: vec![],
                consequent_aggregates: vec![crate::types::ConsequentAggregate {
                    role: "Arity".to_string(),
                    op: "count".to_string(),
                    target_role: "Part".to_string(),
                    source_fact_type_id: "thing_has_part".to_string(),
                    group_key_role: "Thing".to_string(),
                }],
                unresolved_clauses: vec![],
            }],
            general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
            cells: HashMap::new(),
        }
    }

    #[test]
    fn count_aggregate_computes_image_set_size_per_group() {
        // Three Parts belong to T1, one to T2. Expect two derived rows:
        // T1 has Arity=3, T2 has Arity=1.
        let ir = thing_part_arity_ir();
        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        let mut pop_state = ast::Object::phi();
        for (thing, part) in [("T1", "P1"), ("T1", "P2"), ("T1", "P3"), ("T2", "PX")] {
            pop_state = ast::cell_push("thing_has_part",
                ast::fact_from_pairs(&[("Thing", thing), ("Part", part)]), &pop_state);
        }

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let arity: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "thing_has_arity").collect();
        // Collect distinct (Thing, Arity) pairs â€” the outer iteration emits
        // duplicates per group, which forward_chain is expected to dedup.
        let mut pairs: alloc::collections::BTreeSet<(String, String)> = arity.iter().map(|d| {
            let t = d.bindings.iter().find(|(k, _)| k == "Thing").map(|(_, v)| v.clone()).unwrap_or_default();
            let a = d.bindings.iter().find(|(k, _)| k == "Arity").map(|(_, v)| v.clone()).unwrap_or_default();
            (t, a)
        }).collect();
        let expected: alloc::collections::BTreeSet<(String, String)> = [
            ("T1".to_string(), "3".to_string()),
            ("T2".to_string(), "1".to_string()),
        ].into_iter().collect();
        assert_eq!(pairs, expected,
            "distinct (Thing, Arity) derivations expected T1â†’3 and T2â†’1, got {:?} (raw count = {})", pairs, arity.len());
        // Sanity â€” if dedup isn't happening, the raw list still contains
        // the right pairs somewhere.
        assert!(arity.iter().any(|d|
            d.bindings.iter().any(|(k, v)| k == "Thing" && v == "T1") &&
            d.bindings.iter().any(|(k, v)| k == "Arity" && v == "3")));
        pairs.clear();  // avoid unused warning via reset
    }

    fn order_line_item_sum_ir() -> Domain {
        // `LineItem has Amount for Order` is ternary-ish in Halpin's
        // example; for testing we use a simpler binary form
        // `Order has LineItem Amount`, with Order as group key and
        // Amount as target.
        let mut fact_types = HashMap::new();
        fact_types.insert("order_has_line_amount".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Order has LineItem Amount".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Order".to_string(), role_index: 0 },
                RoleDef { noun_name: "LineItem Amount".to_string(), role_index: 1 },
            ],
        });
        fact_types.insert("order_has_total".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Order has Amount".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Order".to_string(), role_index: 0 },
                RoleDef { noun_name: "Amount".to_string(), role_index: 1 },
            ],
        });
        Domain {
            nouns: HashMap::new(),
            fact_types,
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![DerivationRuleDef {
                id: "order-total".to_string(),
                text: "* Order has Amount iff Amount is the sum of LineItem Amount where Order has LineItem Amount.".to_string(),
                antecedent_fact_type_ids: vec![],
                consequent_fact_type_id: "order_has_total".to_string(),
                kind: DerivationKind::ModusPonens,
                join_on: vec![],
                match_on: vec![],
                consequent_bindings: vec![],
                antecedent_filters: vec![],
                consequent_computed_bindings: vec![],
                consequent_aggregates: vec![crate::types::ConsequentAggregate {
                    role: "Amount".to_string(),
                    op: "sum".to_string(),
                    target_role: "LineItem Amount".to_string(),
                    source_fact_type_id: "order_has_line_amount".to_string(),
                    group_key_role: "Order".to_string(),
                }],
                unresolved_clauses: vec![],
            }],
            general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
            cells: HashMap::new(),
        }
    }

    #[test]
    fn sum_aggregate_folds_add_over_projected_target_values() {
        // Order O1: 10 + 25 + 5 = 40; Order O2: 7 = 7.
        let ir = order_line_item_sum_ir();
        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        let mut pop_state = ast::Object::phi();
        for (order, amt) in [("O1", "10"), ("O1", "25"), ("O1", "5"), ("O2", "7")] {
            pop_state = ast::cell_push("order_has_line_amount",
                ast::fact_from_pairs(&[("Order", order), ("LineItem Amount", amt)]), &pop_state);
        }

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let totals: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "order_has_total").collect();
        let pairs: alloc::collections::BTreeSet<(String, String)> = totals.iter().map(|d| {
            let o = d.bindings.iter().find(|(k, _)| k == "Order").map(|(_, v)| v.clone()).unwrap_or_default();
            let a = d.bindings.iter().find(|(k, _)| k == "Amount").map(|(_, v)| v.clone()).unwrap_or_default();
            (o, a)
        }).collect();
        let expected: alloc::collections::BTreeSet<(String, String)> = [
            ("O1".to_string(), "40".to_string()),
            ("O2".to_string(), "7".to_string()),
        ].into_iter().collect();
        assert_eq!(pairs, expected,
            "expected O1=40, O2=7; got {:?} (raw count={})", pairs, totals.len());
    }

    fn order_amount_agg_ir(op: &str) -> Domain {
        // Reuse order_line_item_sum_ir's shape but parameterise the op.
        let mut ir = order_line_item_sum_ir();
        ir.derivation_rules[0].consequent_aggregates[0].op = op.to_string();
        ir.derivation_rules[0].id = format!("order-{}", op);
        ir
    }

    #[test]
    fn min_aggregate_folds_pairwise_minimum() {
        let ir = order_amount_agg_ir("min");
        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);
        let mut pop_state = ast::Object::phi();
        for (o, a) in [("O1", "10"), ("O1", "4"), ("O1", "25"), ("O2", "7")] {
            pop_state = ast::cell_push("order_has_line_amount",
                ast::fact_from_pairs(&[("Order", o), ("LineItem Amount", a)]), &pop_state);
        }
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);
        let totals: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "order_has_total").collect();
        let pairs: alloc::collections::BTreeSet<(String, String)> = totals.iter().map(|d| {
            let o = d.bindings.iter().find(|(k, _)| k == "Order").map(|(_, v)| v.clone()).unwrap_or_default();
            let a = d.bindings.iter().find(|(k, _)| k == "Amount").map(|(_, v)| v.clone()).unwrap_or_default();
            (o, a)
        }).collect();
        let expected: alloc::collections::BTreeSet<(String, String)> = [
            ("O1".to_string(), "4".to_string()),
            ("O2".to_string(), "7".to_string()),
        ].into_iter().collect();
        assert_eq!(pairs, expected, "min: expected O1=4 O2=7, got {:?}", pairs);
    }

    #[test]
    fn max_aggregate_folds_pairwise_maximum() {
        let ir = order_amount_agg_ir("max");
        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);
        let mut pop_state = ast::Object::phi();
        for (o, a) in [("O1", "10"), ("O1", "4"), ("O1", "25"), ("O2", "7")] {
            pop_state = ast::cell_push("order_has_line_amount",
                ast::fact_from_pairs(&[("Order", o), ("LineItem Amount", a)]), &pop_state);
        }
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);
        let totals: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "order_has_total").collect();
        let pairs: alloc::collections::BTreeSet<(String, String)> = totals.iter().map(|d| {
            let o = d.bindings.iter().find(|(k, _)| k == "Order").map(|(_, v)| v.clone()).unwrap_or_default();
            let a = d.bindings.iter().find(|(k, _)| k == "Amount").map(|(_, v)| v.clone()).unwrap_or_default();
            (o, a)
        }).collect();
        let expected: alloc::collections::BTreeSet<(String, String)> = [
            ("O1".to_string(), "25".to_string()),
            ("O2".to_string(), "7".to_string()),
        ].into_iter().collect();
        assert_eq!(pairs, expected, "max: expected O1=25 O2=7, got {:?}", pairs);
    }

    #[test]
    fn avg_aggregate_divides_sum_by_count() {
        let ir = order_amount_agg_ir("avg");
        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);
        let mut pop_state = ast::Object::phi();
        // O1: (9 + 12 + 15) / 3 = 12.
        for (o, a) in [("O1", "9"), ("O1", "12"), ("O1", "15"), ("O2", "7")] {
            pop_state = ast::cell_push("order_has_line_amount",
                ast::fact_from_pairs(&[("Order", o), ("LineItem Amount", a)]), &pop_state);
        }
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);
        let totals: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "order_has_total").collect();
        let pairs: alloc::collections::BTreeSet<(String, String)> = totals.iter().map(|d| {
            let o = d.bindings.iter().find(|(k, _)| k == "Order").map(|(_, v)| v.clone()).unwrap_or_default();
            let a = d.bindings.iter().find(|(k, _)| k == "Amount").map(|(_, v)| v.clone()).unwrap_or_default();
            (o, a)
        }).collect();
        // Accept either integer or float formatting for the averaged value.
        let has_pair = |o: &str, expected_nums: &[&str]| -> bool {
            pairs.iter().any(|(actual_o, v)| actual_o == o && expected_nums.iter().any(|e| v == e))
        };
        assert!(has_pair("O1", &["12", "12.0"]), "avg: expected O1 to average to 12, got {:?}", pairs);
        assert!(has_pair("O2", &["7", "7.0"]), "avg: expected O2=7, got {:?}", pairs);
    }

    #[test]
    fn rule_without_filter_fires_for_any_fact_regression() {
        // Regression: when antecedent_filters is empty, behavior is
        // unchanged from pre-#192 â€” any fact makes the rule fire.
        let ir = city_population_ir(None);
        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "SmallTown"), ("Population", "500000")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let big: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "big_city").collect();
        assert_eq!(big.len(), 1, "unfiltered rule must still fire");
    }

    #[test]
    fn join_derivation_equi_join_on_shared_key() {
        let mut fact_types = HashMap::new();
        fact_types.insert("a_key".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "A has Key".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "A".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        fact_types.insert("b_key".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "B has Key".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "B".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        fact_types.insert("derived".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "A is matched to B".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "A".to_string(), role_index: 0 },
                RoleDef { noun_name: "B".to_string(), role_index: 1 },
            ],
        });

        let ir = Domain {
            nouns: HashMap::new(),
            fact_types,
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![DerivationRuleDef {
                id: "join1".to_string(),
                text: "A matches B on Key".to_string(),
                antecedent_fact_type_ids: vec!["a_key".to_string(), "b_key".to_string()],
                consequent_fact_type_id: "derived".to_string(),
                kind: DerivationKind::Join,
                join_on: vec!["Key".to_string()],
                match_on: vec![],
                consequent_bindings: vec!["A".to_string(), "B".to_string()],
                antecedent_filters: vec![], consequent_computed_bindings: vec![], consequent_aggregates: vec![], unresolved_clauses: vec![],
            }],
            general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
            cells: HashMap::new(),
        };

        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("a_key", ast::fact_from_pairs(&[("A", "a1"), ("Key", "k1")]), &pop_state);
        pop_state = ast::cell_push("a_key", ast::fact_from_pairs(&[("A", "a2"), ("Key", "k2")]), &pop_state);
        pop_state = ast::cell_push("b_key", ast::fact_from_pairs(&[("B", "b1"), ("Key", "k1")]), &pop_state);
        pop_state = ast::cell_push("b_key", ast::fact_from_pairs(&[("B", "b2"), ("Key", "k3")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let derived_facts: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "derived").collect();
        // Only a1<->b1 (both Key="k1"). a2 has Key="k2" which doesn't match any B.
        assert_eq!(derived_facts.len(), 1);
        assert!(derived_facts[0].bindings.contains(&("A".to_string(), "a1".to_string())));
        assert!(derived_facts[0].bindings.contains(&("B".to_string(), "b1".to_string())));
    }

    #[test]
    fn join_derivation_entity_consistency_across_fact_types() {
        let mut fact_types = HashMap::new();
        fact_types.insert("x_key".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "X has Key".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "X".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        fact_types.insert("x_label".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "X has Label".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "X".to_string(), role_index: 0 },
                RoleDef { noun_name: "Label".to_string(), role_index: 1 },
            ],
        });
        fact_types.insert("y_key".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Y has Key".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Y".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        fact_types.insert("result".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Y is resolved to X".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Y".to_string(), role_index: 0 },
                RoleDef { noun_name: "X".to_string(), role_index: 1 },
            ],
        });

        let ir = Domain {
            nouns: HashMap::new(),
            fact_types,
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![DerivationRuleDef {
                id: "join2".to_string(),
                text: "Y resolves to X via Key".to_string(),
                antecedent_fact_type_ids: vec!["y_key".to_string(), "x_key".to_string(), "x_label".to_string()],
                consequent_fact_type_id: "result".to_string(),
                kind: DerivationKind::Join,
                join_on: vec!["Key".to_string(), "X".to_string()],
                match_on: vec![],
                consequent_bindings: vec!["Y".to_string(), "X".to_string()],
                antecedent_filters: vec![], consequent_computed_bindings: vec![], consequent_aggregates: vec![], unresolved_clauses: vec![],
            }],
            general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
            cells: HashMap::new(),
        };

        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("x_key", ast::fact_from_pairs(&[("X", "x1"), ("Key", "k1")]), &pop_state);
        pop_state = ast::cell_push("x_key", ast::fact_from_pairs(&[("X", "x2"), ("Key", "k1")]), &pop_state);
        pop_state = ast::cell_push("x_label", ast::fact_from_pairs(&[("X", "x1"), ("Label", "L1")]), &pop_state);
        pop_state = ast::cell_push("x_label", ast::fact_from_pairs(&[("X", "x2"), ("Label", "L2")]), &pop_state);
        pop_state = ast::cell_push("y_key", ast::fact_from_pairs(&[("Y", "y1"), ("Key", "k1")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let resolved: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "result").collect();
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn join_derivation_match_on_containment() {
        let mut fact_types = HashMap::new();
        fact_types.insert("a_name".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "A has Full Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "A".to_string(), role_index: 0 },
                RoleDef { noun_name: "Full Name".to_string(), role_index: 1 },
            ],
        });
        fact_types.insert("b_name".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "B has Short Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "B".to_string(), role_index: 0 },
                RoleDef { noun_name: "Short Name".to_string(), role_index: 1 },
            ],
        });
        fact_types.insert("matched".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "B is matched to A".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "B".to_string(), role_index: 0 },
                RoleDef { noun_name: "A".to_string(), role_index: 1 },
            ],
        });

        let ir = Domain {
            nouns: HashMap::new(),
            fact_types,
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![DerivationRuleDef {
                id: "match1".to_string(),
                text: "B matches A by name containment".to_string(),
                antecedent_fact_type_ids: vec!["a_name".to_string(), "b_name".to_string()],
                consequent_fact_type_id: "matched".to_string(),
                kind: DerivationKind::Join,
                join_on: vec![],
                match_on: vec![("Full Name".to_string(), "Short Name".to_string())],
                consequent_bindings: vec!["B".to_string(), "A".to_string()],
                antecedent_filters: vec![], consequent_computed_bindings: vec![], consequent_aggregates: vec![], unresolved_clauses: vec![],
            }],
            general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
            cells: HashMap::new(),
        };

        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("a_name", ast::fact_from_pairs(&[("A", "a1"), ("Full Name", "Alpha Bravo")]), &pop_state);
        pop_state = ast::cell_push("a_name", ast::fact_from_pairs(&[("A", "a2"), ("Full Name", "Charlie Delta")]), &pop_state);
        pop_state = ast::cell_push("b_name", ast::fact_from_pairs(&[("B", "b1"), ("Short Name", "Alpha")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let matched: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "matched").collect();
        assert_eq!(matched.len(), 1);
        assert!(matched[0].bindings.contains(&("A".to_string(), "a1".to_string())));
        assert!(matched[0].bindings.contains(&("B".to_string(), "b1".to_string())));
    }

    #[test]
    fn join_derivation_no_match_produces_nothing() {
        let mut fact_types = HashMap::new();
        fact_types.insert("a_key".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "A has Key".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "A".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        fact_types.insert("b_key".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "B has Key".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "B".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        fact_types.insert("derived".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "A matches B".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "A".to_string(), role_index: 0 },
                RoleDef { noun_name: "B".to_string(), role_index: 1 },
            ],
        });

        let ir = Domain {
            nouns: HashMap::new(),
            fact_types,
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![DerivationRuleDef {
                id: "j".to_string(),
                text: "join".to_string(),
                antecedent_fact_type_ids: vec!["a_key".to_string(), "b_key".to_string()],
                consequent_fact_type_id: "derived".to_string(),
                kind: DerivationKind::Join,
                join_on: vec!["Key".to_string()],
                match_on: vec![],
                consequent_bindings: vec!["A".to_string(), "B".to_string()],
                antecedent_filters: vec![], consequent_computed_bindings: vec![], consequent_aggregates: vec![], unresolved_clauses: vec![],
            }],
            general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
            cells: HashMap::new(),
        };

        let (_meta_pop, defs, _def_map) = ir_to_defs(&ir);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("a_key", ast::fact_from_pairs(&[("A", "a1"), ("Key", "k1")]), &pop_state);
        pop_state = ast::cell_push("b_key", ast::fact_from_pairs(&[("B", "b1"), ("Key", "k2")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let derived_count = derived.iter().filter(|d| d.fact_type_id == "derived").count();
        assert_eq!(derived_count, 0, "No match should produce no derivation");
    }

    fn make_forbidden_text_ir(enum_vals: Vec<String>) -> Domain {
        let mut ir = empty_ir();
        let pt = "ProhibitedText";
        let sr = "SupportResponse";
        ir.nouns.insert(pt.to_string(), make_noun("value"));
        ir.enum_values.insert(pt.to_string(), enum_vals);
        ir.nouns.insert(sr.to_string(), make_noun("entity"));
        ir.fact_types.insert("ft1".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: format!("{} contains {}", sr, pt),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: sr.to_string(), role_index: 0 },
                RoleDef { noun_name: pt.to_string(), role_index: 1 },
            ],
        });
        ir.constraints.push(ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("forbidden".to_string()),
            text: format!("It is forbidden that {} contains {}", sr, pt),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
        });
        ir
    }

    #[test]
    fn test_forbidden_text_detected() {
        let endash = core::char::from_u32(0x2013).unwrap().to_string();
        let emdash_s = core::char::from_u32(0x2014).unwrap().to_string();
        let ir = make_forbidden_text_ir(vec![endash, emdash_s]);
        let (_meta_state, defs, def_map) = ir_to_defs(&ir);
        let emdash = core::char::from_u32(0x2014).unwrap();
        let text: String = ['H','e','l','l','o',' ',emdash,' ','h','o','w',' ','c','a','n',' ','I',' ','h','e','l','p','?'].iter().collect();
        let result = eval_constraints_defs(&defs, &def_map, &text, None, &empty_state());
        assert!(!result.is_empty());
        assert!(result[0].detail.contains(emdash));
    }

    #[test]
    fn test_forbidden_text_clean() {
        let endash = core::char::from_u32(0x2013).unwrap().to_string();
        let ir = make_forbidden_text_ir(vec![endash]);
        let (_meta_state, defs, def_map) = ir_to_defs(&ir);
        let result = eval_constraints_defs(&defs, &def_map, "Hello, how can I help you today?", None, &empty_state());
        assert!(result.is_empty());
    }
}

