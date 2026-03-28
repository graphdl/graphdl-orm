// crates/fol-engine/src/lib.rs
//
// WASM interface. Exports:
//   load_ir            — parse JSON IR, compile into predicates (once)
//   evaluate_response  — apply compiled predicates to response + population (per request)
//   synthesize_noun    — collect all knowledge about a noun from the compiled model
//   forward_chain      — apply derivation rules to population until fixed point
//   query_population   — filter a population by predicate, return matching entities

pub mod ast;
mod types;
mod compile;
mod evaluate;
mod query;
mod induce;
pub mod rmap;
pub mod naming;
pub mod validate;
pub mod conceptual_query;
pub mod parse_rule;
pub mod arest;

use wasm_bindgen::prelude::*;
use std::sync::Mutex;
use std::sync::OnceLock;

use types::{ConstraintIR, ResponseContext, Population, Violation, SynthesisResult, WorldAssumption};
use compile::CompiledModel;

struct CompiledState {
    ir: ConstraintIR,
    model: CompiledModel,
}

/// Compiled validation model (from core.md + validation.md).
/// Stored separately from the domain model so it persists across domain loads.
static VALIDATION_MODEL: OnceLock<Mutex<Option<CompiledModel>>> = OnceLock::new();

fn validation_store() -> &'static Mutex<Option<CompiledModel>> {
    VALIDATION_MODEL.get_or_init(|| Mutex::new(None))
}

static STATE: OnceLock<Mutex<Option<CompiledState>>> = OnceLock::new();

fn state_store() -> &'static Mutex<Option<CompiledState>> {
    STATE.get_or_init(|| Mutex::new(None))
}

#[wasm_bindgen]
pub fn load_ir(ir_json: &str) -> Result<(), JsValue> {
    let ir: ConstraintIR = serde_json::from_str(ir_json)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse IR: {}", e)))?;
    let model = compile::compile(&ir);
    let mut store = state_store().lock().unwrap();
    *store = Some(CompiledState { ir, model });
    Ok(())
}

#[wasm_bindgen]
pub fn evaluate_response(response_json: &str, population_json: &str) -> String {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return serde_json::to_string(&Vec::<Violation>::new()).unwrap(),
    };

    let response: ResponseContext = match serde_json::from_str(response_json) {
        Ok(r) => r,
        Err(e) => {
            let v = vec![Violation {
                constraint_id: "PARSE_ERROR".to_string(),
                constraint_text: String::new(),
                detail: format!("Failed to parse response: {}", e),
            }];
            return serde_json::to_string(&v).unwrap();
        }
    };

    let population: Population = match serde_json::from_str(population_json) {
        Ok(p) => p,
        Err(e) => {
            let v = vec![Violation {
                constraint_id: "PARSE_ERROR".to_string(),
                constraint_text: String::new(),
                detail: format!("Failed to parse population: {}", e),
            }];
            return serde_json::to_string(&v).unwrap();
        }
    };

    // Evaluate via AST reduction
    let violations = evaluate::evaluate_via_ast(&state.model, &response, &population);
    serde_json::to_string(&violations).unwrap()
}

#[wasm_bindgen]
pub fn synthesize_noun(noun_name: &str, depth: usize) -> String {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return serde_json::to_string(&SynthesisResult::empty(noun_name)).unwrap(),
    };

    let result = evaluate::synthesize(&state.model, &state.ir, noun_name, depth);
    serde_json::to_string(&result).unwrap()
}

#[wasm_bindgen]
pub fn forward_chain_population(population_json: &str) -> String {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return "[]".to_string(),
    };

    let mut population: Population = match serde_json::from_str(population_json) {
        Ok(p) => p,
        Err(e) => return format!("{{\"error\":\"{}\"}}", e),
    };

    // Forward chain via AST reduction
    let derived = evaluate::forward_chain_ast(&state.model, &mut population);
    serde_json::to_string(&derived).unwrap()
}

#[wasm_bindgen]
pub fn query_population_wasm(population_json: &str, predicate_json: &str) -> String {
    let population: Population = serde_json::from_str(population_json).unwrap_or_default();
    let predicate: query::QueryPredicate = serde_json::from_str(predicate_json).unwrap_or_default();
    let result = query::query_population(&population, &predicate);
    serde_json::to_string(&result).unwrap_or_else(|_| r#"{"matches":[],"count":0}"#.to_string())
}

/// Query a population using the AST-based partial application model.
///
/// schema_id: the fact type ID to query
/// target_role: 1-indexed role to extract from matching facts
/// filter_json: array of [role_index, value] pairs to filter by
/// population_json: the population to query
///
/// Returns JSON: { "matches": ["value1", "value2", ...], "count": N }
#[wasm_bindgen]
pub fn query_schema_wasm(
    schema_id: &str,
    target_role: usize,
    filter_json: &str,
    population_json: &str,
) -> String {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return r#"{"matches":[],"count":0}"#.to_string(),
    };

    let schema = match state.model.schemas.get(schema_id) {
        Some(s) => s,
        None => return r#"{"matches":[],"count":0,"error":"schema not found"}"#.to_string(),
    };

    let population: Population = serde_json::from_str(population_json).unwrap_or_default();
    let filters: Vec<(usize, String)> = serde_json::from_str(filter_json).unwrap_or_default();
    let filter_refs: Vec<(usize, &str)> = filters.iter().map(|(i, v)| (*i, v.as_str())).collect();

    let matches = query::query_with_ast(&population, schema, target_role, &filter_refs);
    let count = matches.len();

    serde_json::to_string(&query::QueryResult { matches, count })
        .unwrap_or_else(|_| r#"{"matches":[],"count":0}"#.to_string())
}

/// Induce constraints and rules from a population.
/// Given observed facts, discover the UC, MC, FC, SS constraints and
/// derivation rules that govern the data. This is the inverse of evaluation.
#[wasm_bindgen]
pub fn induce_from_population(population_json: &str) -> String {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return r#"{"constraints":[],"rules":[],"populationStats":{"factTypeCount":0,"totalFacts":0,"entityCount":0}}"#.to_string(),
    };

    let population: Population = serde_json::from_str(population_json).unwrap_or_default();
    let result = induce::induce(&state.ir, &population);
    serde_json::to_string(&result).unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
}

/// Run a compiled state machine by folding events through the transition function.
/// Events are [(event_name, payload)] pairs. Returns the final state.
#[wasm_bindgen]
pub fn run_machine_wasm(noun_name: &str, events_json: &str, population_json: &str) -> String {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return r#"{"error":"no IR loaded"}"#.to_string(),
    };

    let events: Vec<(String, String)> = serde_json::from_str(events_json).unwrap_or_default();
    let event_names: Vec<&str> = events.iter().map(|(e, _)| e.as_str()).collect();

    // Find the state machine for this noun and run via AST reduction
    let machine_idx = state.model.noun_index.noun_to_state_machines.get(noun_name);
    match machine_idx.and_then(|&idx| state.model.state_machines.get(idx)) {
        Some(machine) => {
            let final_state = evaluate::run_machine_ast(machine, &event_names);
            serde_json::to_string(&final_state).unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
        }
        None => format!(r#"{{"error":"no state machine for noun '{}'"}}"#, noun_name),
    }
}

/// Get valid transitions from a given status in a compiled state machine.
/// Returns JSON: [{ "from": "status", "to": "target", "event": "eventName" }]
#[wasm_bindgen]
pub fn get_transitions_wasm(noun_name: &str, current_status: &str) -> String {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return "[]".to_string(),
    };

    let machine_idx = state.model.noun_index.noun_to_state_machines.get(noun_name);
    match machine_idx.and_then(|&idx| state.model.state_machines.get(idx)) {
        Some(machine) => {
            let valid: Vec<_> = machine.transition_table.iter()
                .filter(|(from, _, _)| from == current_status)
                .map(|(from, to, event)| {
                    serde_json::json!({ "from": from, "to": to, "event": event })
                })
                .collect();
            serde_json::to_string(&valid).unwrap_or_else(|_| "[]".to_string())
        }
        None => "[]".to_string(),
    }
}

/// Given a fact type ID, resolve what event should fire on which state machine.
/// Returns JSON: { "factTypeId": "...", "eventName": "...", "targetNoun": "..." } or null.
#[wasm_bindgen]
pub fn resolve_fact_event(fact_type_id: &str) -> String {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return "null".to_string(),
    };
    match state.model.fact_events.get(fact_type_id) {
        Some(fe) => serde_json::to_string(&serde_json::json!({
            "factTypeId": fe.fact_type_id,
            "eventName": fe.event_name,
            "targetNoun": fe.target_noun,
        })).unwrap_or_else(|_| "null".to_string()),
        None => "null".to_string(),
    }
}

/// Debug: return the compiled model state (noun-to-SM mapping)
#[wasm_bindgen]
pub fn debug_compiled_state() -> String {
    let store = state_store().lock().unwrap();
    match store.as_ref() {
        Some(s) => {
            let sm_info: Vec<serde_json::Value> = s.model.state_machines.iter()
                .map(|sm| serde_json::json!({
                    "nounName": sm.noun_name,
                    "initial": sm.initial,
                    "transitions": sm.transition_table.len(),
                }))
                .collect();
            let noun_map: std::collections::HashMap<&str, usize> = s.model.noun_index.noun_to_state_machines.iter()
                .map(|(k, v)| (k.as_str(), *v))
                .collect();
            serde_json::to_string(&serde_json::json!({
                "loaded": true,
                "stateMachines": sm_info,
                "nounToStateMachines": noun_map,
            })).unwrap_or_else(|_| r#"{"error":"serialization"}"#.to_string())
        }
        None => r#"{"loaded":false}"#.to_string(),
    }
}

/// AREST: Apply a command to the current population.
/// One function application. One state transfer.
/// Returns the complete result: entities, status, transitions, violations, derived facts.
#[wasm_bindgen]
pub fn apply_command_wasm(command_json: &str, population_json: &str) -> String {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return r#"{"entities":[],"status":null,"transitions":[],"violations":[],"derivedCount":0,"rejected":false}"#.to_string(),
    };

    let command: arest::Command = match serde_json::from_str(command_json) {
        Ok(c) => c,
        Err(e) => return format!(r#"{{"error":"Invalid command: {}"}}"#, e),
    };

    let population: Population = serde_json::from_str(population_json).unwrap_or_default();
    let result = arest::apply_command(&state.model, &command, &population);
    serde_json::to_string(&result).unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
}

/// Prepare entity creation: given a noun name, return the initial state
/// and any constraint violations. This is a single function application —
/// the engine evaluates state machine initialization, deontic checks, and
/// derivation rules in one call.
///
/// Returns JSON: { initialState: "Draft" | null, violations: [...], derivedFacts: [...] }
#[wasm_bindgen]
pub fn prepare_entity(noun_name: &str, fields_json: &str, population_json: &str) -> String {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return r#"{"initialState":null,"violations":[],"derivedFacts":[]}"#.to_string(),
    };

    // 1. State machine initialization — find initial state for this noun
    let initial_state = state.model.noun_index.noun_to_state_machines.get(noun_name)
        .and_then(|&idx| state.model.state_machines.get(idx))
        .map(|sm| sm.initial.clone());

    // 2. Deontic constraint evaluation
    let response = types::ResponseContext { text: String::new(), sender_identity: None, fields: None };
    let population: types::Population = serde_json::from_str(population_json).unwrap_or_default();
    let violations = evaluate::evaluate_via_ast(&state.model, &response, &population);

    // 3. Forward chain derivation rules
    let mut pop_clone = population.clone();
    let derived = evaluate::forward_chain_ast(&state.model, &mut pop_clone);

    // 4. Fact-to-event resolution — does creating this entity trigger a transition?
    let fact_event = state.model.fact_events.values()
        .find(|fe| fe.target_noun == noun_name)
        .map(|fe| serde_json::json!({ "eventName": fe.event_name, "factTypeId": fe.fact_type_id }));

    serde_json::to_string(&serde_json::json!({
        "initialState": initial_state,
        "violations": violations,
        "derivedFacts": derived,
        "factEvent": fact_event,
    })).unwrap_or_else(|_| r#"{"initialState":null,"violations":[],"derivedFacts":[]}"#.to_string())
}

/// Load the validation model (compiled from core.md + validation.md).
/// Called once at startup. The validation model persists across domain loads.
#[wasm_bindgen]
pub fn load_validation_model(ir_json: &str) -> Result<(), JsValue> {
    let ir: ConstraintIR = serde_json::from_str(ir_json)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse validation IR: {}", e)))?;
    let model = compile::compile(&ir);
    let mut store = validation_store().lock().unwrap();
    *store = Some(model);
    Ok(())
}

/// Validate a domain IR against the validation model.
/// Takes domain IR as JSON, converts to metamodel population,
/// evaluates validation constraints. Returns JSON violations array.
#[wasm_bindgen]
pub fn validate_schema_wasm(domain_ir_json: &str) -> String {
    let val_store = validation_store().lock().unwrap();
    let validation_model = match val_store.as_ref() {
        Some(m) => m,
        None => return "[]".to_string(),
    };
    let domain_ir: ConstraintIR = match serde_json::from_str(domain_ir_json) {
        Ok(ir) => ir,
        Err(e) => return format!(r#"[{{"constraint_id":"parse_error","constraint_text":"","detail":"{}"}}]"#, e),
    };
    let violations = validate::validate_schema(validation_model, &domain_ir);
    serde_json::to_string(&violations).unwrap_or_else(|_| "[]".to_string())
}

/// Run RMAP (Relational Mapping Procedure) on the loaded IR.
/// Returns table definitions as JSON.
#[wasm_bindgen]
pub fn rmap_wasm() -> String {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return "[]".to_string(),
    };
    let tables = rmap::rmap(&state.ir);
    serde_json::to_string(&tables).unwrap_or_else(|_| "[]".to_string())
}

/// Prove a goal fact via backward chaining.
/// Returns a ProofResult with status (Proven/Disproven/Unknown) and proof tree.
#[wasm_bindgen]
pub fn prove_goal(goal: &str, population_json: &str, world_assumption: &str) -> String {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return r#"{"goal":"","status":"unknown","proof":null,"worldAssumption":"closed"}"#.to_string(),
    };

    let population: Population = serde_json::from_str(population_json).unwrap_or_default();
    let wa = match world_assumption {
        "open" => WorldAssumption::Open,
        _ => WorldAssumption::Closed,
    };

    let result = evaluate::prove(&state.ir, &population, goal, &wa);
    serde_json::to_string(&result).unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
}
