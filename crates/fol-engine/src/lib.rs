// crates/fol-engine/src/lib.rs
//
// WASM interface. Exports:
//   load_ir            — parse JSON IR, compile into predicates (once)
//   evaluate_response  — apply compiled predicates to response + population (per request)
//   synthesize_noun    — collect all knowledge about a noun from the compiled model
//   forward_chain      — apply derivation rules to population until fixed point
//   query_population   — filter a population by predicate, return matching entities

mod types;
mod compile;
mod evaluate;
mod query;
mod induce;

use wasm_bindgen::prelude::*;
use std::sync::Mutex;
use std::sync::OnceLock;

use types::{ConstraintIR, ResponseContext, Population, Violation, SynthesisResult, WorldAssumption};
use compile::{CompiledModel, EvalContext};

struct CompiledState {
    ir: ConstraintIR,
    model: CompiledModel,
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

    let ctx = EvalContext {
        response: &response,
        population: &population,
    };

    let violations = evaluate::evaluate(&state.model, &ctx);
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

    let response = ResponseContext {
        text: String::new(),
        sender_identity: None,
        fields: None,
    };

    let derived = evaluate::forward_chain(&state.model, &response, &mut population);
    serde_json::to_string(&derived).unwrap()
}

#[wasm_bindgen]
pub fn query_population_wasm(population_json: &str, predicate_json: &str) -> String {
    let population: Population = serde_json::from_str(population_json).unwrap_or_default();
    let predicate: query::QueryPredicate = serde_json::from_str(predicate_json).unwrap_or_default();
    let result = query::query_population(&population, &predicate);
    serde_json::to_string(&result).unwrap_or_else(|_| r#"{"matches":[],"count":0}"#.to_string())
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

    let population: Population = serde_json::from_str(population_json).unwrap_or_default();
    let response = ResponseContext {
        text: String::new(),
        sender_identity: None,
        fields: None,
    };
    let ctx = EvalContext { response: &response, population: &population };

    let events: Vec<(String, String)> = serde_json::from_str(events_json).unwrap_or_default();

    // Find the state machine for this noun
    let machine_idx = state.model.noun_index.noun_to_state_machines.get(noun_name);
    match machine_idx.and_then(|&idx| state.model.state_machines.get(idx)) {
        Some(machine) => {
            let final_state = evaluate::run_machine(machine, &events, &ctx);
            serde_json::to_string(&final_state).unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
        }
        None => format!(r#"{{"error":"no state machine for noun '{}'"}}"#, noun_name),
    }
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
