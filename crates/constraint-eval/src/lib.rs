// crates/constraint-eval/src/lib.rs
mod types;
mod evaluate;

use wasm_bindgen::prelude::*;
use std::sync::Mutex;
use std::sync::OnceLock;

use types::{ConstraintIR, ResponseContext, Population, Violation};

static IR: OnceLock<Mutex<Option<ConstraintIR>>> = OnceLock::new();

fn ir_store() -> &'static Mutex<Option<ConstraintIR>> {
    IR.get_or_init(|| Mutex::new(None))
}

#[wasm_bindgen]
pub fn load_ir(ir_json: &str) -> Result<(), JsValue> {
    let ir: ConstraintIR = serde_json::from_str(ir_json)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse IR: {}", e)))?;
    let mut store = ir_store().lock().unwrap();
    *store = Some(ir);
    Ok(())
}

#[wasm_bindgen]
pub fn evaluate_response(response_json: &str, population_json: &str) -> String {
    let store = ir_store().lock().unwrap();
    let ir = match store.as_ref() {
        Some(ir) => ir,
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

    let violations = evaluate::evaluate(ir, &response, &population);
    serde_json::to_string(&violations).unwrap()
}
