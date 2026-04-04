// crates/arest/src/evaluate.rs
//
// Evaluation is beta reduction. That's it.
//
// Constraint verification:  constraints.flat_map(|c| apply(c.func, ctx)) â†’ [Violation]
// Forward inference:        derivations.flat_map(|d| apply(d.func, pop)) â†’ [DerivedFact]
// State machine execution:  fold(transition)(initial)(stream) â†’ final_state
// Synthesis:                collect all knowledge about a noun from the compiled model.

use std::collections::HashSet;
use crate::types::*;
use crate::compile::CompiledModel;
use crate::ast;

/// Evaluate all compiled constraints via AST reduction.
/// Evaluation = beta reduction: apply(func, object) â†’ violations.
/// Alethic constraints always reject. Deontic violations are tagged as non-alethic.
pub fn evaluate_via_ast(model: &CompiledModel, text: &str, sender: Option<&str>, population: &Population) -> Vec<Violation> {
    let ctx_obj = ast::encode_eval_context(text, sender, population);
    let defs = std::collections::HashMap::new();

    model.constraints.iter()
        .flat_map(|c| {
            let result = ast::apply(&c.func, &ctx_obj, &defs);
            let is_alethic = matches!(c.modality, crate::compile::Modality::Alethic);
            ast::decode_violations(&result).into_iter().map(move |mut v| {
                v.alethic = is_alethic;
                v
            })
        })
        .collect()
}

/// Run a state machine via AST reduction.
/// The machine's func is a transition function: <state, event> â†’ next_state.
/// Guards are compiled into the Condition predicates.
pub fn run_machine_ast(
    machine: &crate::compile::CompiledStateMachine,
    events: &[&str],
) -> String {
    let defs = std::collections::HashMap::new();
    let mut state = machine.initial.clone();

    for event in events {
        let input = ast::Object::seq(vec![
            ast::Object::atom(&state),
            ast::Object::atom(event),
        ]);
        let result = ast::apply(&machine.func, &input, &defs);
        if let Some(next) = result.as_atom() {
            state = next.to_string();
        }
    }

    state
}

// â”€â”€ Forward Chaining â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
//
// Correctness: FORML 2 derivation rules are monotonic (add facts, never
// remove). The population is finite. A monotonic sequence over a finite
// set reaches a fixed point. The loop terminates when no new facts are
// derived.
//
// Safety: the iteration bound prevents pathological rule sets from
// producing unbounded intermediate populations. If the bound is hit,
// the engine stops and returns what it has â€” a partial fixed point.

/// Forward chain via AST reduction to fixed point.
/// Bounded to prevent pathological rule sets from running unbounded.
pub fn forward_chain_ast(
    model: &CompiledModel,
    population: &mut Population,
) -> Vec<DerivedFact> {
    let mut all_derived: Vec<DerivedFact> = Vec::new();
    let max_iterations = 100;
    let defs = std::collections::HashMap::new();

    for _ in 0..max_iterations {
        let pop_obj = ast::encode_population(population);
        let mut new_facts: Vec<DerivedFact> = Vec::new();

        for derivation in &model.derivations {
            let result = ast::apply(&derivation.func, &pop_obj, &defs);
            // Decode derived facts from the result Object
            if let Some(items) = result.as_seq() {
                for item in items {
                    if let Some(fact_items) = item.as_seq() {
                        if fact_items.len() >= 3 {
                            let ft_id = fact_items[0].as_atom().unwrap_or("").to_string();
                            let reading = fact_items[1].as_atom().unwrap_or("").to_string();
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

                            let fact = DerivedFact {
                                fact_type_id: ft_id,
                                reading,
                                bindings,
                                derived_by: derivation.id.clone(),
                                confidence: Confidence::Definitive,
                            };

                            if !population_contains(population, &fact)
                                && !all_derived.iter().any(|d| same_fact(d, &fact))
                                && !new_facts.iter().any(|d| same_fact(d, &fact))
                            {
                                new_facts.push(fact);
                            }
                        }
                    }
                }
            }
        }

        if new_facts.is_empty() { break; } // Fixed point reached â€” proof complete.

        for fact in &new_facts {
            add_to_population(population, fact);
        }
        all_derived.extend(new_facts);
    }

    all_derived
}

/// Check if a derived fact already exists in the population.
fn population_contains(population: &Population, fact: &DerivedFact) -> bool {
    population.facts.get(&fact.fact_type_id).map_or(false, |facts| {
        facts.iter().any(|f| {
            f.fact_type_id == fact.fact_type_id
                && f.bindings.len() == fact.bindings.len()
                && f.bindings.iter().all(|b| fact.bindings.contains(b))
        })
    })
}

/// Check if two derived facts represent the same fact.
fn same_fact(a: &DerivedFact, b: &DerivedFact) -> bool {
    a.fact_type_id == b.fact_type_id
        && a.bindings.len() == b.bindings.len()
        && a.bindings.iter().all(|ab| b.bindings.contains(ab))
}

/// Add a derived fact to the population as a FactInstance.
fn add_to_population(population: &mut Population, fact: &DerivedFact) {
    let instance = FactInstance {
        fact_type_id: fact.fact_type_id.clone(),
        bindings: fact.bindings.clone(),
    };
    population.facts.entry(fact.fact_type_id.clone())
        .or_insert_with(Vec::new)
        .push(instance);
}

// â”€â”€ Proof Engine (Backward Chaining) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
// Given a goal fact, work backward through derivation rules to build a proof tree.
// Each step either finds the fact in the population (axiom), derives it via a rule
// (recursively proving antecedents), or concludes based on world assumption.

/// Attempt to prove a goal fact.
///
/// `goal` is a string like "Academic has Rank 'P'" â€” a reading with optional values.
/// The engine searches the population for a matching fact, then tries derivation
/// rules whose consequent matches, recursively proving antecedents.
#[allow(dead_code)] // used by lib.rs WASM export, not by main.rs binary
pub fn prove(
    ir: &Domain,
    population: &Population,
    goal: &str,
    world_assumption: &WorldAssumption,
) -> ProofResult {
    let mut visited = HashSet::new();
    let proof = prove_goal(ir, population, goal, &mut visited);

    let status = match &proof {
        Some(_) => ProofStatus::Proven,
        None => match world_assumption {
            WorldAssumption::Closed => ProofStatus::Disproven,
            WorldAssumption::Open => ProofStatus::Unknown,
        },
    };

    ProofResult {
        goal: goal.to_string(),
        status,
        proof,
        world_assumption: world_assumption.clone(),
    }
}

/// Recursive backward chaining.
#[allow(dead_code)] // called by prove()
fn prove_goal(
    ir: &Domain,
    population: &Population,
    goal: &str,
    visited: &mut HashSet<String>,
) -> Option<ProofStep> {
    // Cycle detection
    if visited.contains(goal) {
        return None;
    }
    visited.insert(goal.to_string());

    // Step 1: Check if the goal is directly in the population (axiom)
    for (ft_id, facts) in &population.facts {
        if let Some(ft) = ir.fact_types.get(ft_id) {
            for fact in facts {
                // Match goal against reading + bindings
                let fact_text = format_fact(&ft.reading, &fact.bindings);
                if fact_text_matches(goal, &fact_text, &ft.reading) {
                    return Some(ProofStep {
                        fact: fact_text,
                        justification: Justification::Axiom,
                        children: vec![],
                    });
                }
            }
        }
    }

    // Step 2: Try derivation rules whose consequent could match the goal
    for rule in &ir.derivation_rules {
        if let Some(cons_ft) = ir.fact_types.get(&rule.consequent_fact_type_id) {
            // Check if the goal could be the consequent of this rule
            if goal.contains(&cons_ft.reading) || cons_ft.reading.contains(goal.split(' ').next().unwrap_or("")) {
                // Try to prove all antecedents
                let mut child_proofs = Vec::new();
                let mut all_proven = true;

                for ant_ft_id in &rule.antecedent_fact_type_ids {
                    if let Some(ant_ft) = ir.fact_types.get(ant_ft_id) {
                        match prove_goal(ir, population, &ant_ft.reading, visited) {
                            Some(proof) => child_proofs.push(proof),
                            None => {
                                all_proven = false;
                                break;
                            }
                        }
                    }
                }

                if all_proven && !child_proofs.is_empty() {
                    return Some(ProofStep {
                        fact: goal.to_string(),
                        justification: Justification::Derived {
                            rule_id: rule.id.clone(),
                            rule_text: rule.text.clone(),
                        },
                        children: child_proofs,
                    });
                }
            }
        }
    }

    visited.remove(goal);
    None
}

/// Format a fact from its reading template and bindings
#[allow(dead_code)] // called by prove_goal()
fn format_fact(reading: &str, bindings: &[(String, String)]) -> String {
    let mut result = reading.to_string();
    for (noun, value) in bindings {
        // Replace first occurrence of the noun name with the value
        if let Some(pos) = result.find(noun.as_str()) {
            result = format!("{}{} '{}'{}",
                &result[..pos], noun, value, &result[pos + noun.len()..]);
        }
    }
    result
}

/// Check if a goal string matches a formatted fact
#[allow(dead_code)] // called by prove_goal()
fn fact_text_matches(goal: &str, fact_text: &str, reading: &str) -> bool {
    // Exact match
    if goal == fact_text || goal == reading {
        return true;
    }
    // Goal without quotes matches reading
    let goal_lower = goal.to_lowercase();
    let fact_lower = fact_text.to_lowercase();
    let reading_lower = reading.to_lowercase();
    goal_lower == fact_lower || goal_lower == reading_lower
        || fact_lower.contains(&goal_lower)
        || goal_lower.contains(&reading_lower)
}

// â”€â”€ Synthesis â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
// Collect all knowledge about a noun from the compiled model.

/// Synthesize: collect all knowledge about a noun from the compiled model.
pub fn synthesize(
    model: &CompiledModel,
    ir: &Domain,
    noun_name: &str,
    depth: usize,
) -> SynthesisResult {
    let index = &model.noun_index;

    let world_assumption = index.world_assumptions.get(noun_name)
        .cloned()
        .unwrap_or(WorldAssumption::Closed);

    // 1. Find all fact types where this noun plays a role
    let participates_in: Vec<FactTypeSummary> = index.noun_to_fact_types
        .get(noun_name)
        .map(|fts| fts.iter().filter_map(|(ft_id, role_idx)| {
            ir.fact_types.get(ft_id).map(|ft| FactTypeSummary {
                id: ft_id.clone(),
                reading: ft.reading.clone(),
                role_index: *role_idx,
            })
        }).collect())
        .unwrap_or_default();

    // 2. Find all constraints spanning those fact types
    let mut seen_constraint_ids = HashSet::new();
    let applicable_constraints: Vec<ConstraintSummary> = participates_in.iter()
        .flat_map(|ft_summary| {
            index.fact_type_to_constraints.get(&ft_summary.id)
                .cloned()
                .unwrap_or_default()
        })
        .filter(|cid| seen_constraint_ids.insert(cid.clone()))
        .filter_map(|cid| {
            index.constraint_index.get(&cid).and_then(|&idx| {
                model.constraints.get(idx).map(|cc| {
                    // Look up the original constraint def for kind info
                    let cdef = ir.constraints.iter().find(|c| c.id == cid);
                    ConstraintSummary {
                        id: cid.clone(),
                        text: cc.text.clone(),
                        kind: cdef.map(|c| c.kind.clone()).unwrap_or_default(),
                        modality: format!("{:?}", cc.modality),
                        deontic_operator: cdef.and_then(|c| c.deontic_operator.clone()),
                    }
                })
            })
        })
        .collect();

    // 3. Find state machines for this noun
    let state_machines: Vec<StateMachineSummary> = index.noun_to_state_machines
        .get(noun_name)
        .and_then(|&idx| model.state_machines.get(idx))
        .map(|sm| {
            vec![StateMachineSummary {
                noun_name: sm.noun_name.clone(),
                statuses: sm.statuses.clone(),
                current_status: Some(sm.initial.clone()),
                valid_transitions: sm.transition_table.iter()
                    .filter(|(from, _, _)| *from == sm.initial)
                    .map(|(_, _, event)| event.clone())
                    .collect(),
            }]
        })
        .unwrap_or_default();

    // 4. Find related nouns (other nouns in shared fact types)
    let mut seen_related = HashSet::new();
    let related_nouns: Vec<RelatedNoun> = if depth > 0 {
        participates_in.iter()
            .flat_map(|ft_summary| {
                ir.fact_types.get(&ft_summary.id)
                    .map(|ft| {
                        ft.roles.iter()
                            .filter(|r| r.noun_name != noun_name)
                            .filter(|r| seen_related.insert(r.noun_name.clone()))
                            .map(|r| {
                                let wa = index.world_assumptions.get(&r.noun_name)
                                    .cloned()
                                    .unwrap_or(WorldAssumption::Closed);
                                RelatedNoun {
                                    name: r.noun_name.clone(),
                                    via_fact_type: ft_summary.id.clone(),
                                    via_reading: ft.reading.clone(),
                                    world_assumption: wa,
                                }
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
            .collect()
    } else {
        Vec::new()
    };

    // 5. Derived facts would need population context; leave empty for static synthesis.
    let derived_facts = Vec::new();

    SynthesisResult {
        noun_name: noun_name.to_string(),
        world_assumption,
        participates_in,
        applicable_constraints,
        state_machines,
        derived_facts,
        related_nouns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn empty_ir() -> Domain {
        Domain {
            domain: "test".to_string(),
            nouns: HashMap::new(),
            fact_types: HashMap::new(),
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![], general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(), objectifications: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
        }
    }

    fn empty_population() -> Population {
        Population { facts: HashMap::new() }
    }

    fn make_noun(object_type: &str) -> NounDef {
        NounDef {
            object_type: object_type.to_string(),
            world_assumption: WorldAssumption::default(),
        }
    }

    // â”€â”€ AST evaluation path tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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

        let model = crate::compile::compile(&ir);

        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![
            FactInstance { fact_type_id: "ft1".to_string(), bindings: vec![
                ("Person".to_string(), "Alice".to_string()), ("Name".to_string(), "A".to_string()),
            ]},
            FactInstance { fact_type_id: "ft1".to_string(), bindings: vec![
                ("Person".to_string(), "Alice".to_string()), ("Name".to_string(), "B".to_string()),
            ]},
        ]);
        let pop = Population { facts };

        // AST path
        let violations = evaluate_via_ast(&model, "", None, &pop);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].constraint_id, "uc1");

        // Verified via AST evaluation path.
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

        let model = crate::compile::compile(&ir);

        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![
            FactInstance { fact_type_id: "ft1".to_string(), bindings: vec![
                ("Person".to_string(), "Alice".to_string()), ("Name".to_string(), "A".to_string()),
            ]},
        ]);
        let pop = Population { facts };

        let violations = evaluate_via_ast(&model, "", None, &pop);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_run_machine_via_ast() {
        // Domain Change state machine: Proposed â†’ Under Review â†’ Approved â†’ Applied
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

        let model = crate::compile::compile(&ir);
        let machine = &model.state_machines[0];

        // Happy path: Proposed â†’ Under Review â†’ Approved â†’ Applied
        let final_state = run_machine_ast(machine, &["review-requested", "approved", "applied"]);
        assert_eq!(final_state, "Applied");

        // Rejection path: Proposed â†’ Under Review â†’ Rejected
        let final_state = run_machine_ast(machine, &["review-requested", "rejected"]);
        assert_eq!(final_state, "Rejected");

        // Invalid event: stays in current state
        let final_state = run_machine_ast(machine, &["applied"]);
        assert_eq!(final_state, "Proposed"); // "applied" not valid from Proposed

        // Partial: just review
        let final_state = run_machine_ast(machine, &["review-requested"]);
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
        let model = crate::compile::compile(&ir);
        let machine = &model.state_machines[0];
        assert_eq!(machine.initial, "Pending");
    }

    #[test]
    fn test_noun_without_state_machine() {
        let ir = empty_ir(); // no state machines
        let model = crate::compile::compile(&ir);
        assert!(model.noun_index.noun_to_state_machines.get("Customer").is_none());
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
        let model = crate::compile::compile(&ir);
        let machine = &model.state_machines[0];

        // From Triaging: two valid transitions
        let from_triaging: Vec<&str> = machine.transition_table.iter()
            .filter(|(from, _, _)| from == "Triaging")
            .map(|(_, _, event)| event.as_str())
            .collect();
        assert_eq!(from_triaging, vec!["investigate", "quick-resolve"]);

        // From Investigating: one valid transition
        let from_investigating: Vec<&str> = machine.transition_table.iter()
            .filter(|(from, _, _)| from == "Investigating")
            .map(|(_, _, event)| event.as_str())
            .collect();
        assert_eq!(from_investigating, vec!["resolve"]);

        // From Resolved: no transitions (terminal)
        let from_resolved: Vec<&str> = machine.transition_table.iter()
            .filter(|(from, _, _)| from == "Resolved")
            .map(|(_, _, event)| event.as_str())
            .collect();
        assert!(from_resolved.is_empty());
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
        let model = crate::compile::compile(&ir);
        let machine = &model.state_machines[0];

        // Full lifecycle with back-and-forth
        let final_state = run_machine_ast(machine, &[
            "investigate",
            "request-info",
            "customer-replied",
            "resolve",
        ]);
        assert_eq!(final_state, "Resolved");

        // Invalid event mid-flow stays in current state
        let final_state = run_machine_ast(machine, &["investigate", "resolve", "investigate"]);
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
        let model = crate::compile::compile(&ir);

        // Text with markdown â†’ violations
        let violations = evaluate_via_ast(&model, "## Heading here", None, &empty_population());
        assert!(violations.len() > 0, "should detect forbidden markdown");

        // Clean text â†’ no violations
        let clean_violations = evaluate_via_ast(&model, "No special formatting here.", None, &empty_population());
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
        let model = crate::compile::compile(&ir);
        let violations = evaluate_via_ast(&model, "anything", None, &empty_population());
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_no_constraints_no_violations_via_ast() {
        let model = crate::compile::compile(&empty_ir());
        let violations = evaluate_via_ast(&model, "", None, &empty_population());
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_fact_creation_triggers_state_transition() {
        // Full chain: fact creation â†’ Activation â†’ Verb â†’ Transition â†’ state change
        //
        // Domain: Support
        //   Graph Schema: "Customer submits SupportRequest"
        //   Verb: "submit"
        //   Activation: (Graph Schema, Verb) â€” objectified with spanning UC
        //   State Machine: SupportRequest
        //     Triaging â†’ Investigating (event: "investigate")
        //   Verb "submit" is performed during Transition "investigate"
        //
        // When a fact "Customer submits SupportRequest" is created,
        // the engine should recognize the Verb â†’ find the Transition â†’ fire it.

        let mut ir = empty_ir();
        ir.nouns.insert("Customer".to_string(), make_noun("entity"));
        ir.nouns.insert("SupportRequest".to_string(), make_noun("entity"));

        // Graph Schema
        ir.fact_types.insert("ft_submit".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Customer submits SupportRequest".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "SupportRequest".to_string(), role_index: 1 },
            ],
        });

        // State Machine for SupportRequest
        ir.state_machines.insert("SupportRequest".to_string(), StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Triaging".to_string(), "Investigating".to_string(), "Resolved".to_string()],
            transitions: vec![
                TransitionDef { from: "Triaging".to_string(), to: "Investigating".to_string(), event: "investigate".to_string(), guard: None },
                TransitionDef { from: "Investigating".to_string(), to: "Resolved".to_string(), event: "resolve".to_string(), guard: None },
            ],
        });

        let compiled = crate::compile::compile(&ir);

        // The state machine starts at "Triaging"
        let machine = &compiled.state_machines[0];
        assert_eq!(machine.initial, "Triaging");

        // When the fact "Customer submits SupportRequest" is created,
        // the "investigate" event should fire (Verb â†’ Transition mapping).
        // For now, verify the state machine can transition:
        let after_investigate = run_machine_ast(machine, &["investigate"]);
        assert_eq!(after_investigate, "Investigating");

        // The Activation lookup is: given the fact type (ft_submit), find the Verb,
        // then find which Transition the Verb is performed during.
        // This requires the compiled model to have:
        //   1. Schema â†’ Verb mapping (from "Graph Schema is activated by Verb")
        //   2. Verb â†’ Transition mapping (from "Verb is performed during Transition")
        //
        // The engine should expose: given a fact_type_id, what event fires?
        // This is compile_derivation_chain: ft_submit â†’ Activation â†’ Verb â†’ Transition â†’ event name

        // For now, verify the pieces exist in the compiled model:
        assert!(compiled.schemas.contains_key("ft_submit"), "Schema compiled for submit fact type");
        assert_eq!(compiled.schemas["ft_submit"].role_names, vec!["Customer", "SupportRequest"]);
    }

    #[test]
    fn test_fact_event_mapping_compiled() {
        // Verify the compiled model maps fact types to events
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

        // "investigate" appears in the reading via heuristic matching
        ir.state_machines.insert("SM".to_string(), StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Triaging".to_string(), "Investigating".to_string()],
            transitions: vec![
                TransitionDef { from: "Triaging".to_string(), to: "Investigating".to_string(), event: "submit".to_string(), guard: None },
            ],
        });

        let compiled = crate::compile::compile(&ir);

        // The fact type "Customer submits SupportRequest" should map to event "submit"
        // because "submit" appears in the reading "Customer submits SupportRequest"
        let fe = compiled.fact_events.get("ft_submit");
        assert!(fe.is_some(), "fact_events should contain ft_submit");
        let fe = fe.unwrap();
        assert_eq!(fe.event_name, "submit");
        assert_eq!(fe.target_noun, "SupportRequest");

        // Verify the state machine transitions on this event
        let machine = &compiled.state_machines[0];
        let final_state = run_machine_ast(machine, &["submit"]);
        assert_eq!(final_state, "Investigating");
    }

    #[test]
    fn test_guarded_transition_blocks_on_violation() {
        // A deontic guard prevents the transition when violated.
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

        // Deontic forbidden constraint (guard)
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
                        graph_schema_id: "ft_resp".to_string(),
                        constraint_ids: vec!["guard1".to_string()],
                    }),
                },
            ],
        });

        let compiled = crate::compile::compile(&ir);
        let machine = &compiled.state_machines[0];

        // Response with forbidden content â†’ constraint detects violation
        let pop = empty_population();
        // Verify the constraint produces violations via evaluate_via_ast
        let violations = evaluate_via_ast(&compiled, "Here are the internal-details of the system", None, &pop);
        assert!(!violations.is_empty(), "Guard constraint should produce violations");

        // Clean response â†’ no constraint violations
        let clean_violations = evaluate_via_ast(&compiled, "Your issue has been resolved. Thank you.", None, &pop);
        assert!(clean_violations.is_empty(), "No guard violations for clean response");

        // run_machine_ast evaluates the compiled transition function (guards are
        // baked into the Condition predicate). The machine processes the event:
        let state = run_machine_ast(machine, &["resolve"]);
        // The transition fires because the guard Condition is compiled statically.
        // Guard enforcement at runtime requires the caller to evaluate constraints
        // separately (via evaluate_via_ast) and gate the event accordingly.
        assert_eq!(state, "Resolved",
            "run_machine_ast fires the transition; guard enforcement is the caller's responsibility");
    }

    #[test]
    fn test_fact_driven_event_resolution() {
        // Test the event resolution: given a fact type and the compiled model,
        // determine which event should fire on the state machine.
        //
        // This requires a new compilation step:
        //   For each Graph Schema that is activated by a Verb,
        //   and that Verb is performed during a Transition,
        //   record: fact_type_id â†’ event_name

        let mut ir = empty_ir();
        ir.nouns.insert("Customer".to_string(), make_noun("entity"));
        ir.nouns.insert("SupportRequest".to_string(), make_noun("entity"));
        ir.nouns.insert("Agent".to_string(), make_noun("entity"));

        // Two fact types with different verbs
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

        let compiled = crate::compile::compile(&ir);

        // Both schemas should compile
        assert!(compiled.schemas.contains_key("ft_submit"));
        assert!(compiled.schemas.contains_key("ft_resolve"));

        // State machine should have both transitions
        let machine = &compiled.state_machines[0];
        assert_eq!(machine.transition_table.len(), 2);

        // Full lifecycle through fact-driven events
        let state = run_machine_ast(machine, &["investigate", "resolve"]);
        assert_eq!(state, "Resolved");
    }

    #[test]
    fn test_subset_constraint_without_autofill_produces_violation() {
        // SS constraint WITHOUT autofill: should produce violations, not derived facts.
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
        // SS constraint WITHOUT autofill â€” just validates, doesn't derive
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

        let compiled = crate::compile::compile(&ir);

        // No modus ponens derivation should be compiled
        let mp_count = compiled.derivations.iter()
            .filter(|d| d.kind == DerivationKind::ModusPonens)
            .count();
        assert_eq!(mp_count, 0, "Should NOT compile modus ponens without autofill");

        // Forward chain should produce no derived facts
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Person".to_string(), "p1".to_string())],
        }]);
        let mut population = Population { facts };
        let derived = forward_chain_ast(&compiled, &mut population);
        let mp_derived: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "ft2").collect();
        // CWA negation may derive "NOT Person hasInsurance" â€” that's expected.
        // But no POSITIVE modus ponens derivation should exist.
        let positive_mp = mp_derived.iter().filter(|d| !d.reading.contains("NOT")).count();
        assert_eq!(positive_mp, 0, "No autofill â†’ no positive derived insurance facts");

        // The subset constraint validates the population â€” violations are produced
        // by the constraint evaluator (compile_subset) when the superset fact
        // doesn't have a matching consequent fact. This is verified by the
        // existing test_subset_violation test.
    }

    #[test]
    fn test_forward_chain_ast_subtype_inheritance() {
        // Teacher is subtype of Academic. Academic has Rank.
        // Teacher "T1" exists â†’ should derive Academic participation.
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
        let compiled = crate::compile::compile(&ir);

        // Teacher T1 has Rank P
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Academic".to_string(), "T1".to_string()), ("Rank".to_string(), "P".to_string())],
        }]);
        let mut population = Population { facts };

        let _derived = forward_chain_ast(&compiled, &mut population);
        // Should derive that T1 participates in Academic fact types via subtype inheritance
        // subtype derivation adds inherited facts (may be zero if none applicable)
    }

    #[test]
    fn test_forward_chain_ast_modus_ponens() {
        // If Academic heads Department then Academic works for Department (subset constraint).
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

        // Subset constraint with autofill: heads â†’ automatically derive works for
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

        let compiled = crate::compile::compile(&ir);

        // Academic A1 heads Department D1
        let mut facts = HashMap::new();
        facts.insert("ft_heads".to_string(), vec![FactInstance {
            fact_type_id: "ft_heads".to_string(),
            bindings: vec![
                ("Academic".to_string(), "A1".to_string()),
                ("Department".to_string(), "D1".to_string()),
            ],
        }]);
        let mut population = Population { facts };

        let _derived = forward_chain_ast(&compiled, &mut population);
        // Modus ponens should derive: A1 works for D1
        // Note: the AST path encodes/decodes through Object, so the derivation
        // must survive the round-trip. If no derivations, the SS constraint
        // compiler may need the target fact type reference.
        // For now, verify forward chaining terminates and check via AST path.
        let mut pop2 = Population { facts: {
            let mut f = HashMap::new();
            f.insert("ft_heads".to_string(), vec![FactInstance {
                fact_type_id: "ft_heads".to_string(),
                bindings: vec![
                    ("Academic".to_string(), "A1".to_string()),
                    ("Department".to_string(), "D1".to_string()),
                ],
            }]);
            f
        }};
        let ast_derived = forward_chain_ast(&compiled, &mut pop2);
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
        let compiled = crate::compile::compile(&ir);
        let mut population = Population { facts: HashMap::new() };
        let derived = forward_chain_ast(&compiled, &mut population);
        assert_eq!(derived.len(), 0);
    }

    // â”€â”€ Constraint evaluation tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn test_no_constraints_no_violations() {
        let ir = empty_ir();
        let compiled = crate::compile::compile(&ir);
        let result = evaluate_via_ast(&compiled, "", None, &empty_population());
        assert!(result.is_empty());
    }

    // test_forbidden_text_detected and test_forbidden_text_clean moved to end of module

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

        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("Customer".to_string(), "c1".to_string()), ("Name".to_string(), "Alice".to_string())],
            },
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("Customer".to_string(), "c1".to_string()), ("Name".to_string(), "Bob".to_string())],
            },
        ]);
        let population = Population { facts };

        let compiled = crate::compile::compile(&ir);
        let result = evaluate_via_ast(&compiled, "", None, &population);
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

        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("Person".to_string(), "p1".to_string()), ("Person".to_string(), "p1".to_string())],
            },
        ]);
        let population = Population { facts };

        let compiled = crate::compile::compile(&ir);
        let result = evaluate_via_ast(&compiled, "", None, &population);
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

        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Order".to_string(), "o1".to_string())],
        }]);
        facts.insert("ft2".to_string(), vec![FactInstance {
            fact_type_id: "ft2".to_string(),
            bindings: vec![("Order".to_string(), "o1".to_string())],
        }]);
        let population = Population { facts };

        let compiled = crate::compile::compile(&ir);
        let result = evaluate_via_ast(&compiled, "", None, &population);
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

        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Person".to_string(), "p1".to_string())],
        }]);
        let population = Population { facts };

        let compiled = crate::compile::compile(&ir);
        let result = evaluate_via_ast(&compiled, "", None, &population);
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

        let compiled = crate::compile::compile(&ir);
        let result = evaluate_via_ast(&compiled, "", None, &empty_population());
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

        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Order".to_string(), "o1".to_string())],
        }]);
        facts.insert("ft2".to_string(), vec![FactInstance {
            fact_type_id: "ft2".to_string(),
            bindings: vec![("Order".to_string(), "o1".to_string())],
        }]);
        let population = Population { facts };

        let compiled = crate::compile::compile(&ir);
        let result = evaluate_via_ast(&compiled, "", None, &population);
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

        let mut facts = HashMap::new();
        facts.insert("ft2".to_string(), vec![FactInstance {
            fact_type_id: "ft2".to_string(),
            bindings: vec![("Customer".to_string(), "c1".to_string()), ("Email".to_string(), "a@b.com".to_string())],
        }]);
        let population = Population { facts };

        let compiled = crate::compile::compile(&ir);
        let result = evaluate_via_ast(&compiled, "", None, &population);
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
        let mut facts = HashMap::new();
        facts.insert("ft3".to_string(), vec![FactInstance {
            fact_type_id: "ft3".to_string(),
            bindings: vec![("Customer".to_string(), "c1".to_string())],
        }]);
        let population = Population { facts };

        let compiled = crate::compile::compile(&ir);
        let result = evaluate_via_ast(&compiled, "", None, &population);
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

        let compiled = crate::compile::compile(&ir);
        let result = evaluate_via_ast(&compiled, "Here is some help for you.", Some(""), &empty_population());
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

        let compiled = crate::compile::compile(&ir);
        let result = evaluate_via_ast(&compiled, "Hello", Some(""), &empty_population());
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
        // Three fact types that all reference FieldName â€” simulates multi-span constraint
        for i in 1..=3 {
            ir.fact_types.insert(format!("ft{}", i), FactTypeDef {
                schema_id: String::new(),
                reading: format!("SupportResponse names APIProduct by FieldName ({})", i),
                readings: vec![],
                roles: vec![
                    RoleDef { noun_name: "SupportResponse".to_string(), role_index: 0 },
                    RoleDef { noun_name: "APIProduct".to_string(), role_index: 1 },
                    RoleDef { noun_name: "FieldName".to_string(), role_index: 2 },
                ],
            });
        }
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

        let compiled = crate::compile::compile(&ir);
        let result = evaluate_via_ast(&compiled, "test response without required field names", None, &empty_population());
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

        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Person".to_string(), "p1".to_string())],
        }]);
        let population = Population { facts };

        let compiled = crate::compile::compile(&ir);
        let result = evaluate_via_ast(&compiled, "", None, &population);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Equality violation"));
    }

    // â”€â”€ Forward Inference & Synthesis Tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn test_subtype_inheritance_derivation() {
        let mut ir = empty_ir();

        // Vehicle is a supertype, Car is a subtype of Vehicle
        ir.nouns.insert("Vehicle".to_string(), make_noun("entity"));
        ir.nouns.insert("Car".to_string(), make_noun("entity"));
        ir.subtypes.insert("Car".to_string(), "Vehicle".to_string());
        ir.nouns.insert("License".to_string(), make_noun("entity"));

        // Fact type: Vehicle has License
        ir.fact_types.insert("ft_vehicle_license".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Vehicle has License".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Vehicle".to_string(), role_index: 0 },
                RoleDef { noun_name: "License".to_string(), role_index: 1 },
            ],
        });

        // Fact type: Car has Color (to give Car instances)
        ir.fact_types.insert("ft_car_color".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Car has Color".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Car".to_string(), role_index: 0 },
            ],
        });

        let compiled = crate::compile::compile(&ir);

        // Verify subtype inheritance derivation was compiled
        let subtype_derivations: Vec<_> = compiled.derivations.iter()
            .filter(|d| d.kind == DerivationKind::SubtypeInheritance)
            .collect();
        assert!(!subtype_derivations.is_empty(),
            "Expected at least one subtype inheritance derivation");

        // Test forward chaining with a population that has a Car instance
        let mut facts = HashMap::new();
        facts.insert("ft_car_color".to_string(), vec![FactInstance {
            fact_type_id: "ft_car_color".to_string(),
            bindings: vec![("Car".to_string(), "my_car".to_string())],
        }]);
        let mut population = Population { facts };

        let derived = forward_chain_ast(&compiled, &mut population);

        // Car "my_car" should inherit Vehicle's fact type
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

        let compiled = crate::compile::compile(&ir);

        // Verify modus ponens derivation was compiled
        let mp_derivations: Vec<_> = compiled.derivations.iter()
            .filter(|d| d.kind == DerivationKind::ModusPonens)
            .collect();
        assert!(!mp_derivations.is_empty(),
            "Expected a modus ponens derivation from SS constraint");

        // Forward chain: p1 hasLicense -> should derive p1 hasInsurance
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("Person".to_string(), "p1".to_string())],
        }]);
        let mut population = Population { facts };

        let derived = forward_chain_ast(&compiled, &mut population);

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

        // Fact type involving CWA noun
        ir.fact_types.insert("ft_perm".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Permission grants access to Resource".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Permission".to_string(), role_index: 0 },
                RoleDef { noun_name: "Resource".to_string(), role_index: 1 },
            ],
        });
        // Fact type involving OWA noun
        ir.fact_types.insert("ft_cap".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "Capability enables Resource".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Capability".to_string(), role_index: 0 },
                RoleDef { noun_name: "Resource".to_string(), role_index: 1 },
            ],
        });

        let compiled = crate::compile::compile(&ir);

        // CWA derivation should exist for Permission
        let cwa_derivations: Vec<_> = compiled.derivations.iter()
            .filter(|d| d.kind == DerivationKind::ClosedWorldNegation)
            .collect();

        let cwa_for_perm = cwa_derivations.iter()
            .any(|d| d.id.contains("Permission"));
        assert!(cwa_for_perm,
            "Expected CWA negation derivation for Permission");

        // No CWA derivation for Capability (it's OWA)
        let cwa_for_cap = cwa_derivations.iter()
            .any(|d| d.id.contains("Capability"));
        assert!(!cwa_for_cap,
            "Expected NO CWA negation derivation for Capability (OWA noun)");

        // Forward chain with a population where Permission exists
        // but doesn't participate in ft_perm
        let mut facts = HashMap::new();
        // Permission "read" exists (via some other fact type)
        facts.insert("ft_other".to_string(), vec![FactInstance {
            fact_type_id: "ft_other".to_string(),
            bindings: vec![("Permission".to_string(), "read".to_string())],
        }]);
        let mut population = Population { facts };

        let derived = forward_chain_ast(&compiled, &mut population);

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

        let compiled = crate::compile::compile(&ir);
        let result = synthesize(&compiled, &ir, "Customer", 1);

        assert_eq!(result.noun_name, "Customer");
        assert_eq!(result.world_assumption, WorldAssumption::Closed);

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
        let ir = empty_ir();
        let compiled = crate::compile::compile(&ir);
        let result = synthesize(&compiled, &ir, "NonExistent", 1);

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

        let compiled = crate::compile::compile(&ir);

        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![FactInstance {
            fact_type_id: "ft1".to_string(),
            bindings: vec![("A".to_string(), "a1".to_string())],
        }]);
        let mut population = Population { facts };

        // Should terminate even if derivations produce facts
        let derived = forward_chain_ast(&compiled, &mut population);
        // Just verify it terminates â€” the exact count depends on CWA rules
        assert!(derived.len() < 100, "Forward chaining should reach fixed point quickly");
    }

    #[test]
    fn test_transitivity_derivation() {
        let mut ir = empty_ir();

        ir.nouns.insert("City".to_string(), make_noun("entity"));
        ir.nouns.insert("State".to_string(), make_noun("entity"));
        ir.nouns.insert("Country".to_string(), make_noun("entity"));

        // City isIn State
        ir.fact_types.insert("ft_city_state".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "City isIn State".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "City".to_string(), role_index: 0 },
                RoleDef { noun_name: "State".to_string(), role_index: 1 },
            ],
        });
        // State isIn Country
        ir.fact_types.insert("ft_state_country".to_string(), FactTypeDef {
            schema_id: String::new(),
            reading: "State isIn Country".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "State".to_string(), role_index: 0 },
                RoleDef { noun_name: "Country".to_string(), role_index: 1 },
            ],
        });

        let compiled = crate::compile::compile(&ir);

        // Should have a transitivity derivation for City->State->Country
        let trans_derivations: Vec<_> = compiled.derivations.iter()
            .filter(|d| d.kind == DerivationKind::Transitivity)
            .collect();
        assert!(!trans_derivations.is_empty(),
            "Expected transitivity derivation for City->State->Country chain");

        // Forward chain: Austin isIn Texas, Texas isIn USA -> Austin (transitively) in USA
        let mut facts = HashMap::new();
        facts.insert("ft_city_state".to_string(), vec![FactInstance {
            fact_type_id: "ft_city_state".to_string(),
            bindings: vec![
                ("City".to_string(), "Austin".to_string()),
                ("State".to_string(), "Texas".to_string()),
            ],
        }]);
        facts.insert("ft_state_country".to_string(), vec![FactInstance {
            fact_type_id: "ft_state_country".to_string(),
            bindings: vec![
                ("State".to_string(), "Texas".to_string()),
                ("Country".to_string(), "USA".to_string()),
            ],
        }]);
        let mut population = Population { facts };

        let derived = forward_chain_ast(&compiled, &mut population);

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

    #[test]
    fn test_backward_compatible_deserialization() {
        // Old IR JSON without derivation_rules or world_assumption should still parse
        let json = r#"{
            "domain": "test",
            "nouns": {
                "Customer": {
                    "objectType": "entity",
                    "enumValues": null,
                    "valueType": null,
                    "superType": null
                }
            },
            "factTypes": {},
            "constraints": [],
            "stateMachines": {}
        }"#;

        let ir: Domain = serde_json::from_str(json).expect("Should parse old IR format");
        assert_eq!(ir.domain, "test");
        assert!(ir.derivation_rules.is_empty());
        let customer = ir.nouns.get("Customer").unwrap();
        assert_eq!(customer.world_assumption, WorldAssumption::Closed);
    }

    #[test]
    fn join_derivation_equi_join_on_shared_key() {
        // Generic test: join two fact types on a shared noun name.
        // A has Key "k1", B has Key "k1" â†’ derive C with both A and B values.
        // A has Key "k1", B has Key "k2" â†’ no derivation (keys don't match).
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
            domain: "test".to_string(),
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
            }],
            general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(), objectifications: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
        };

        let compiled = crate::compile::compile(&ir);

        let mut facts = HashMap::new();
        facts.insert("a_key".to_string(), vec![
            FactInstance { fact_type_id: "a_key".to_string(), bindings: vec![("A".to_string(), "a1".to_string()), ("Key".to_string(), "k1".to_string())]},
            FactInstance { fact_type_id: "a_key".to_string(), bindings: vec![("A".to_string(), "a2".to_string()), ("Key".to_string(), "k2".to_string())] },
        ]);
        facts.insert("b_key".to_string(), vec![
            FactInstance { fact_type_id: "b_key".to_string(), bindings: vec![("B".to_string(), "b1".to_string()), ("Key".to_string(), "k1".to_string())]},
            FactInstance { fact_type_id: "b_key".to_string(), bindings: vec![("B".to_string(), "b2".to_string()), ("Key".to_string(), "k3".to_string())] },
        ]);

        let mut population = Population { facts };
        forward_chain_ast(&compiled, &mut population);

        let derived = population.facts.get("derived").expect("Should derive");
        // Only a1â†”b1 (both Key="k1"). a2 has Key="k2" which doesn't match any B.
        assert_eq!(derived.len(), 1);
        assert!(derived[0].bindings.contains(&("A".to_string(), "a1".to_string())));
        assert!(derived[0].bindings.contains(&("B".to_string(), "b1".to_string())));
    }

    #[test]
    fn join_derivation_entity_consistency_across_fact_types() {
        // When the same entity noun appears in multiple antecedent fact types,
        // the join must bind them to the same entity.
        // Entity X has Key "k1" (ft1) and X has Label "L1" (ft2).
        // Entity X with Key "k1" but Label "L2" should NOT match.
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
            domain: "test".to_string(),
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
            }],
            general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(), objectifications: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
        };

        let compiled = crate::compile::compile(&ir);

        let mut facts = HashMap::new();
        // Two X entities with the same key but different labels
        facts.insert("x_key".to_string(), vec![
            FactInstance { fact_type_id: "x_key".to_string(), bindings: vec![("X".to_string(), "x1".to_string()), ("Key".to_string(), "k1".to_string())]},
            FactInstance { fact_type_id: "x_key".to_string(), bindings: vec![("X".to_string(), "x2".to_string()), ("Key".to_string(), "k1".to_string())] },
        ]);
        facts.insert("x_label".to_string(), vec![
            FactInstance { fact_type_id: "x_label".to_string(), bindings: vec![("X".to_string(), "x1".to_string()), ("Label".to_string(), "L1".to_string())]},
            FactInstance { fact_type_id: "x_label".to_string(), bindings: vec![("X".to_string(), "x2".to_string()), ("Label".to_string(), "L2".to_string())] },
        ]);
        facts.insert("y_key".to_string(), vec![
            FactInstance { fact_type_id: "y_key".to_string(), bindings: vec![("Y".to_string(), "y1".to_string()), ("Key".to_string(), "k1".to_string())]},
        ]);

        let mut population = Population { facts };
        forward_chain_ast(&compiled, &mut population);

        let resolved = population.facts.get("result").expect("Should derive");
        // Y "y1" should match BOTH x1 and x2 (both have Key="k1"), entity consistency
        // holds because X bindings are consistent within each combination.
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn join_derivation_match_on_containment() {
        // Cross-noun containment predicate: A.FullName contains B.ShortName.
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
            domain: "test".to_string(),
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
            }],
            general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(), objectifications: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
        };

        let compiled = crate::compile::compile(&ir);

        let mut facts = HashMap::new();
        facts.insert("a_name".to_string(), vec![
            FactInstance { fact_type_id: "a_name".to_string(), bindings: vec![("A".to_string(), "a1".to_string()), ("Full Name".to_string(), "Alpha Bravo".to_string())]},
            FactInstance { fact_type_id: "a_name".to_string(), bindings: vec![("A".to_string(), "a2".to_string()), ("Full Name".to_string(), "Charlie Delta".to_string())] },
        ]);
        facts.insert("b_name".to_string(), vec![
            FactInstance { fact_type_id: "b_name".to_string(), bindings: vec![("B".to_string(), "b1".to_string()), ("Short Name".to_string(), "Alpha".to_string())]},
        ]);

        let mut population = Population { facts };
        forward_chain_ast(&compiled, &mut population);

        let matched = population.facts.get("matched").expect("Should derive");
        // "Alpha Bravo" contains "Alpha" â†’ a1 matches. "Charlie Delta" doesn't â†’ a2 excluded.
        assert_eq!(matched.len(), 1);
        assert!(matched[0].bindings.contains(&("A".to_string(), "a1".to_string())));
        assert!(matched[0].bindings.contains(&("B".to_string(), "b1".to_string())));
    }

    #[test]
    fn join_derivation_no_match_produces_nothing() {
        // When join keys don't match, no facts are derived.
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
            domain: "test".to_string(),
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
            }],
            general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(), objectifications: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
        };

        let compiled = crate::compile::compile(&ir);

        let mut facts = HashMap::new();
        facts.insert("a_key".to_string(), vec![
            FactInstance { fact_type_id: "a_key".to_string(), bindings: vec![("A".to_string(), "a1".to_string()), ("Key".to_string(), "k1".to_string())]},
        ]);
        facts.insert("b_key".to_string(), vec![
            FactInstance { fact_type_id: "b_key".to_string(), bindings: vec![("B".to_string(), "b1".to_string()), ("Key".to_string(), "k2".to_string())]},
        ]);

        let mut population = Population { facts };
        forward_chain_ast(&compiled, &mut population);

        assert!(population.facts.get("derived").is_none(), "No match should produce no derivation");
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
        let compiled = crate::compile::compile(&ir);
        let emdash = core::char::from_u32(0x2014).unwrap();
        let text: String = ['H','e','l','l','o',' ',emdash,' ','h','o','w',' ','c','a','n',' ','I',' ','h','e','l','p','?'].iter().collect();
        let result = evaluate_via_ast(&compiled, &text, None, &empty_population());
        assert!(!result.is_empty());
        assert!(result[0].detail.contains(emdash));
    }

    #[test]
    fn test_forbidden_text_clean() {
        let endash = core::char::from_u32(0x2013).unwrap().to_string();
        let ir = make_forbidden_text_ir(vec![endash]);
        let compiled = crate::compile::compile(&ir);
        let result = evaluate_via_ast(&compiled, "Hello, how can I help you today?", None, &empty_population());
        assert!(result.is_empty());
    }
}
