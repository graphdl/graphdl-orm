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

// ── The three operations ────────────────────────────────────────────

fn parse_and_compile_impl(readings: Vec<(String, String)>) -> Result<u32, String> {
    let mut m = types::Domain::default();
    for (name, text) in &readings {
        let ir = if m.nouns.is_empty() {
            parse_forml2::parse_markdown(text)
        } else {
            parse_forml2::parse_markdown_with_nouns(text, &m.nouns)
        }.map_err(|e| format!("{}: {}", name, e))?;
        m.nouns.extend(ir.nouns);
        m.fact_types.extend(ir.fact_types);
        m.constraints.extend(ir.constraints);
        m.state_machines.extend(ir.state_machines);
        m.derivation_rules.extend(ir.derivation_rules);
        m.general_instance_facts.extend(ir.general_instance_facts);
        m.subtypes.extend(ir.subtypes);
        m.enum_values.extend(ir.enum_values);
        m.ref_schemes.extend(ir.ref_schemes);
        m.objectifications.extend(ir.objectifications);
        m.named_spans.extend(ir.named_spans);
        m.autofill_spans.extend(ir.autofill_spans);
    }
    let state = parse_forml2::domain_to_state(&m);
    let defs = compile::compile_to_defs_state(&state);
    Ok(allocate(state, defs))
}

fn release_impl(handle: u32) {
    let mut s = ds().lock().unwrap();
    if let Some(slot) = s.get_mut(handle as usize) { *slot = None; }
}

/// SYSTEM:x = ⟨o, D'⟩. One function. Input classification inside.
///
/// The key is a function name in DEFS. The input is an FFP object.
/// apply(defs[key], parse(input), defs) → result.
///
/// Keys not yet in DEFS fall through to legacy dispatch.
fn system_impl(handle: u32, key: &str, input: &str) -> String {
    // ── Handle-free operations ──────────────────────────────────
    match key {
        "parse" => {
            // input: <markdown, domain>
            let obj = ast::Object::parse(input);
            let items = obj.as_seq().unwrap_or(&[]);
            let markdown = items.first().and_then(|o| o.as_atom()).unwrap_or(input);
            let domain = items.get(1).and_then(|o| o.as_atom()).unwrap_or("");
            return match parse_forml2::parse_markdown(markdown) {
                Ok(d) => parse_forml2::domain_to_entities(&d, domain),
                Err(e) => format!("⊥ {}", e),
            };
        }
        "parse_with_nouns" => {
            // input: <markdown, domain, <⟨name, objectType⟩, ...>>
            let obj = ast::Object::parse(input);
            let items = obj.as_seq().unwrap_or(&[]);
            let markdown = items.first().and_then(|o| o.as_atom()).unwrap_or("");
            let domain = items.get(1).and_then(|o| o.as_atom()).unwrap_or("");
            let mut existing = HashMap::new();
            if let Some(nouns_obj) = items.get(2).and_then(|o| o.as_seq()) {
                for noun_obj in nouns_obj {
                    if let Some(pair) = noun_obj.as_seq() {
                        let name = pair.first().and_then(|o| o.as_atom()).unwrap_or("");
                        let ot = pair.get(1).and_then(|o| o.as_atom()).unwrap_or("entity");
                        existing.insert(name.to_string(), types::NounDef {
                            object_type: ot.to_string(),
                            world_assumption: types::WorldAssumption::Closed,
                        });
                    }
                }
            }
            return match parse_forml2::parse_markdown_with_nouns(markdown, &existing) {
                Ok(d) => parse_forml2::domain_to_entities(&d, domain),
                Err(e) => format!("⊥ {}", e),
            };
        }
        _ => {}
    }

    // ── SYSTEM:x = (ρ(↑entity(x):D)) ↑ op(x) ────────────────────
    let s = ds().lock().unwrap();
    let st = match s.get(handle as usize).and_then(|x| x.as_ref()) {
        Some(x) => x,
        None => return "⊥".into(),
    };
    let obj = ast::Object::parse(input);

    // SYSTEM:x = (ρ(↑entity(x):D)):↑op(x)  (Eq. 9)
    // Single ρ-dispatch. No Rust fallback. Every key resolves from D.
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
