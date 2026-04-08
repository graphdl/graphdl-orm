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

/// create: allocate empty D. compile ∘ parse is pre-registered in DEFS.
fn create_impl() -> u32 {
    let state = ast::Object::phi();
    let defs = vec![]; // empty — compile is handled by system_impl
    allocate(state, defs)
}

/// Legacy: parse_and_compile as create + compile for each readings pair.
fn parse_and_compile_impl(readings: Vec<(String, String)>) -> Result<u32, String> {
    let h = create_impl();
    for (_name, text) in &readings {
        let result = system_impl(h, "compile", text);
        if result.starts_with("⊥") {
            return Err(result);
        }
    }
    Ok(h)
}

fn release_impl(handle: u32) {
    let mut s = ds().lock().unwrap();
    if let Some(slot) = s.get_mut(handle as usize) { *slot = None; }
}

/// SYSTEM:x = ⟨o, D'⟩. The only function.
///
/// Per Eq. 9: SYSTEM:x = (ρ(↑entity(x):D)):↑op(x).
/// Self-modification: system(h, "compile", readings_text) ingests readings.
/// All other operations: ρ-dispatch via defs in D.
fn system_impl(handle: u32, key: &str, input: &str) -> String {
    // ── Self-modification: compile ∘ parse (Sec. 5.1) ─────────────
    // The addressed entity is DEFS. The operation is compile ∘ parse.
    // The input is readings text. D' = D with new defs.
    if key == "compile" {
        let mut s = ds().lock().unwrap();
        let existing_d = s.get(handle as usize)
            .and_then(|x| x.as_ref())
            .map(|st| &st.d);

        // Extract existing nouns and fact types from D for cross-domain resolution
        let (existing_nouns, existing_fact_types) = match existing_d {
            Some(d) => {
                let domain = compile::state_to_domain(d);
                (domain.nouns, domain.fact_types)
            }
            None => (HashMap::new(), HashMap::new()),
        };

        // compile ∘ parse: readings → domain → state → defs → D'
        let ir = if existing_nouns.is_empty() {
            parse_forml2::parse_markdown(input)
        } else {
            parse_forml2::parse_markdown_with_context(input, &existing_nouns, &existing_fact_types)
        };
        let domain = match ir {
            Ok(d) => d,
            Err(e) => return format!("⊥ {}", e),
        };

        // Merge with existing domain from D
        let mut merged = match existing_d {
            Some(d) => compile::state_to_domain(d),
            None => types::Domain::default(),
        };
        merged.nouns.extend(domain.nouns);
        merged.fact_types.extend(domain.fact_types);
        merged.constraints.extend(domain.constraints);
        merged.state_machines.extend(domain.state_machines);
        merged.derivation_rules.extend(domain.derivation_rules);
        merged.general_instance_facts.extend(domain.general_instance_facts);
        merged.subtypes.extend(domain.subtypes);
        merged.enum_values.extend(domain.enum_values);
        merged.ref_schemes.extend(domain.ref_schemes);
        merged.objectifications.extend(domain.objectifications);
        merged.named_spans.extend(domain.named_spans);
        merged.autofill_spans.extend(domain.autofill_spans);

        // ↓DEFS: store new compiled state
        let state = parse_forml2::domain_to_state(&merged);
        let defs = compile::compile_to_defs_state(&state);
        let d = ast::defs_to_state(&defs, &state);
        s[handle as usize] = Some(CompiledState { d });

        return format!("{}", defs.len());
    }

    // ── SYSTEM:x = (ρ(↑entity(x):D)):↑op(x)  (Eq. 9) ────────────
    let s = ds().lock().unwrap();
    let st = match s.get(handle as usize).and_then(|x| x.as_ref()) {
        Some(x) => x,
        None => return "⊥".into(),
    };
    let obj = ast::Object::parse(input);

    // Single ρ-dispatch. Every key resolves from D.
    ast::apply(&ast::Func::Def(key.to_string()), &obj, &st.d).to_string()
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
