// crates/arest/src/lib.rs
//
// AREST: Applicative REpresentational State Transfer
//
// SYSTEM:x = <o, D'>
// One function. Readings in, applications out.
// State = P (facts) + DEFS (named Func).

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::OnceLock;

pub mod ast;
pub mod types;
pub mod compile;
pub mod evaluate;
pub mod query;
pub mod induce;
pub mod rmap;
pub mod naming;
pub mod validate;
pub mod conceptual_query;
pub mod parse_rule;
pub mod parse_forml2;
pub mod verbalize;
pub mod arest;
pub mod generators;

#[cfg(feature = "cloudflare")]
pub mod cloudflare;

/// D: the unified state — population cells + def cells in one Object.
/// Backus Sec. 14.3: "the state D of an AST system."
struct CompiledState {
    d: ast::Object,
}

static DOMAINS: OnceLock<Mutex<Vec<Option<CompiledState>>>> = OnceLock::new();
fn ds() -> &'static Mutex<Vec<Option<CompiledState>>> {
    DOMAINS.get_or_init(|| Mutex::new(Vec::new()))
}

fn allocate(state: ast::Object, defs: Vec<(String, ast::Func)>) -> u32 {
    let d = ast::defs_to_state(&defs, &state);
    let mut s = ds().lock().unwrap();
    let h = s.iter().position(|x| x.is_none()).unwrap_or_else(|| { s.push(None); s.len() - 1 });
    s[h] = Some(CompiledState { d });
    h as u32
}

// ── SYSTEM is the only function ─────────────────────────────────────

/// create: allocate empty D with platform primitives registered in DEFS.
fn create_impl() -> u32 {
    let state = ast::Object::phi();
    let defs = vec![
        ("compile".to_string(), ast::Func::Platform("compile".to_string())),
        ("apply".to_string(), ast::Func::Platform("apply_command".to_string())),
    ];
    allocate(state, defs)
}

/// Legacy: parse_and_compile as create + compile for each readings pair.
fn parse_and_compile_impl(readings: Vec<(String, String)>) -> Result<u32, String> {
    let h = create_impl();
    readings.iter().try_fold(h, |h, (_name, text)| {
        let result = system_impl(h, "compile", text);
        if result.starts_with("⊥") { Err(result) } else { Ok(h) }
    })
}

fn release_impl(handle: u32) {
    let mut s = ds().lock().unwrap();
    s.get_mut(handle as usize).into_iter().for_each(|slot| *slot = None);
}

/// SYSTEM:x = ⟨o, D'⟩. Pure ρ-dispatch + state transition.
///
/// The FPGA core: look up key in D via ρ, beta-reduce, update state.
/// No match arms. No if-branches. Every operation is a def in D.
fn system_impl(handle: u32, key: &str, input: &str) -> String {
    let mut s = ds().lock().unwrap();
    let st = match s.get(handle as usize).and_then(|x| x.as_ref()) {
        Some(x) => x,
        None => return "⊥".into(),
    };
    let obj = ast::Object::parse(input);

    // Single ρ-dispatch (Eq. 9)
    let result = ast::apply(&ast::Func::Def(key.to_string()), &obj, &st.d);

    // AST state transition: when result is a new D, store it (⟨o, D'⟩).
    // Platform primitives (compile, apply) return D' as their result.
    // Pure query defs return display notation (no state change).
    // Result contains cells (Noun, GraphSchema, etc.) iff it's a new D.
    let is_new_d = result.as_seq().is_some() && ast::fetch("Noun", &result) != ast::Object::Bottom;
    is_new_d.then(|| s[handle as usize] = Some(CompiledState { d: result.clone() }));

    result.to_string()
}

// ── WIT Component exports ───────────────────────────────────────────

#[cfg(feature = "wit")]
wit_bindgen::generate!({ world: "arest", path: "wit" });

#[cfg(feature = "wit")]
struct E;

#[cfg(feature = "wit")]
export!(E);

#[cfg(feature = "wit")]
impl exports::graphdl::arest::engine::Guest for E {
    fn parse_and_compile(readings: Vec<(String, String)>) -> Result<u32, String> {
        parse_and_compile_impl(readings)
    }
    fn release(handle: u32) { release_impl(handle) }
    fn system(handle: u32, key: String, input: String) -> String {
        system_impl(handle, &key, &input)
    }
}
