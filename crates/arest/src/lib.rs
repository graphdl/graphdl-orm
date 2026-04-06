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

struct CompiledState {
    pop: types::Population,
    defs: Vec<(String, ast::Func)>,
}

impl CompiledState {
    fn def(&self, name: &str) -> Option<&ast::Func> {
        self.defs.iter().find(|(n, _)| n == name).map(|(_, f)| f)
    }
    fn defs_matching(&self, prefix: &str) -> Vec<(&str, &ast::Func)> {
        self.defs.iter().filter(|(n, _)| n.starts_with(prefix)).map(|(n, f)| (n.as_str(), f)).collect()
    }
    fn def_map(&self) -> HashMap<String, ast::Func> {
        self.defs.iter().map(|(n, f)| (n.clone(), f.clone())).collect()
    }
}

static DOMAINS: OnceLock<Mutex<Vec<Option<CompiledState>>>> = OnceLock::new();
fn ds() -> &'static Mutex<Vec<Option<CompiledState>>> {
    DOMAINS.get_or_init(|| Mutex::new(Vec::new()))
}

fn allocate(pop: types::Population, defs: Vec<(String, ast::Func)>) -> u32 {
    let mut s = ds().lock().unwrap();
    let h = s.iter().position(|x| x.is_none()).unwrap_or_else(|| { s.push(None); s.len() - 1 });
    s[h] = Some(CompiledState { pop, defs });
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
    let pop = parse_forml2::domain_to_population(&m);
    let defs = compile::compile_to_defs(&pop);
    Ok(allocate(pop, defs))
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
    let def_map = st.def_map();
    let obj = ast::Object::parse(input);

    // Resolve key in DEFS. Apply. Return.
    if let Some(func) = def_map.get(key) {
        return ast::apply(func, &obj, &def_map).to_string();
    }

    // Two remaining operations that need Rust-level access to P:
    // rho: metacomposition (one line, used by federation tests)
    // forward_chain: iterative fixed-point loop over derivation defs
    match key {
        "rho" => {
            let operation = obj.as_seq().and_then(|s| s.get(1)).and_then(|o| o.as_atom()).unwrap_or("");
            let func = ast::metacompose(&obj, &def_map);
            ast::apply(&func, &ast::Object::atom(operation), &def_map).to_string()
        }
        "forward_chain" => {
            let mut pop = st.pop.clone();
            let derivation_defs = st.defs_matching("derivation:");
            let derived = evaluate::forward_chain_defs(&derivation_defs, &mut pop);
            let items: Vec<ast::Object> = derived.iter().map(|d| {
                ast::Object::seq(vec![
                    ast::Object::atom(&d.fact_type_id),
                    ast::Object::atom(&d.reading),
                    ast::Object::Seq(d.bindings.iter().map(|(k, v)| ast::Object::seq(vec![ast::Object::atom(k), ast::Object::atom(v)])).collect()),
                ])
            }).collect();
            ast::Object::Seq(items).to_string()
        }
        _ => format!("⊥ unknown: {}", key),
    }
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
