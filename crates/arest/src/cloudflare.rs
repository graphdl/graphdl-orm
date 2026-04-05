// Platform adapter for Cloudflare Workers.
// CF Workers do not support WASM Component Model.
// The system function is canonical. These re-export it via wasm-bindgen.

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn parse_and_compile(readings_json: &str) -> Result<u32, JsError> {
    let readings: Vec<(String, String)> = serde_json::from_str(readings_json)
        .map_err(|e| JsError::new(&e.to_string()))?;
    crate::parse_and_compile_impl(readings).map_err(|e| JsError::new(&e))
}

#[wasm_bindgen]
pub fn release(handle: u32) { crate::release_impl(handle); }

#[wasm_bindgen]
pub fn system(handle: u32, key: &str, input: &str) -> String {
    crate::system_impl(handle, key, input)
}
