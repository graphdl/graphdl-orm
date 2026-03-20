// crates/fol-engine/src/lib.rs
//
// WASM interface. Exports:
//   load_ir            — parse JSON IR, compile into predicates (once)
//   evaluate_response  — apply compiled predicates to response + population (per request)
//   synthesize_noun    — collect all knowledge about a noun from the compiled model
//   forward_chain      — apply derivation rules to population until fixed point

mod types;
mod compile;
mod evaluate;

use wasm_bindgen::prelude::*;
use std::sync::Mutex;
use std::sync::OnceLock;

use types::{ConstraintIR, ResponseContext, Population, Violation, SynthesisResult};
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
