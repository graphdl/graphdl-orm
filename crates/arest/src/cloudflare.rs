// Platform adapter for Cloudflare Workers.
// CF Workers do not support WASM Component Model.
// SYSTEM is the only function. create() bootstraps D with compile ∘ parse.

use wasm_bindgen::prelude::*;

/// Allocate an empty D with compile ∘ parse pre-registered in DEFS.
#[wasm_bindgen]
pub fn create() -> u32 { crate::create_impl() }

/// SYSTEM:x = ⟨o, D'⟩. The only function.
/// Ingesting readings: system(handle, "compile", readings_text)
/// All other operations: system(handle, key, input)
#[wasm_bindgen]
pub fn system(handle: u32, key: &str, input: &str) -> String {
    crate::system_impl(handle, key, input)
}

/// Release a compiled domain handle.
#[wasm_bindgen]
pub fn release(handle: u32) { crate::release_impl(handle); }

/// Legacy: parse_and_compile as create + system(h, "compile", readings).
/// Kept for backward compatibility during migration.
#[wasm_bindgen]
pub fn parse_and_compile(readings_json: &str) -> Result<u32, JsError> {
    let readings: Vec<(String, String)> = serde_json::from_str(readings_json)
        .map_err(|e| JsError::new(&e.to_string()))?;
    crate::parse_and_compile_impl(readings).map_err(|e| JsError::new(&e))
}
