// crates/arest/src/lib.rs
//
// AREST: Applicative REpresentational State Transfer
//
// SYSTEM:x = <o, D'>
// One function. Readings in, applications out.

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

struct CompiledState {
    pop: types::Population,
    defs: Vec<(String, ast::Func)>,
    model: compile::CompiledModel, // legacy, used by forward_chain/synthesize/apply_command
}

impl CompiledState {
    fn domain(&self) -> types::Domain {
        compile::population_to_domain(&self.pop)
    }
    fn def(&self, name: &str) -> Option<&ast::Func> {
        self.defs.iter().find(|(n, _)| n == name).map(|(_, f)| f)
    }
    fn defs_matching(&self, prefix: &str) -> Vec<(&str, &ast::Func)> {
        self.defs.iter().filter(|(n, _)| n.starts_with(prefix)).map(|(n, f)| (n.as_str(), f)).collect()
    }
}

static DOMAINS: OnceLock<Mutex<Vec<Option<CompiledState>>>> = OnceLock::new();
fn ds() -> &'static Mutex<Vec<Option<CompiledState>>> {
    DOMAINS.get_or_init(|| Mutex::new(Vec::new()))
}

wit_bindgen::generate!({ world: "arest", path: "wit" });
struct E; export!(E);

impl exports::graphdl::arest::engine::Guest for E {
    fn parse_and_compile(readings: Vec<(String, String)>) -> Result<u32, String> {
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
        let model = compile::compile(&m);
        let defs = compile::compile_to_defs(&pop);
        let mut s = ds().lock().unwrap();
        let h = s.iter().position(|x| x.is_none()).unwrap_or_else(|| { s.push(None); s.len() - 1 });
        s[h] = Some(CompiledState { pop, defs, model });
        Ok(h as u32)
    }

    fn release(handle: u32) {
        let mut s = ds().lock().unwrap();
        if let Some(slot) = s.get_mut(handle as usize) { *slot = None; }
    }

    fn system(handle: u32, key: String, input: String) -> String {
        let s = ds().lock().unwrap();
        let st = match s.get(handle as usize).and_then(|x| x.as_ref()) {
            Some(x) => x,
            None => return r#"{"error":"no domain loaded"}"#.into(),
        };
        let def_map: HashMap<String, ast::Func> = st.defs.iter().map(|(n, f)| (n.clone(), f.clone())).collect();

        match key.as_str() {
            "evaluate" => {
                let args: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
                let text = args["text"].as_str().unwrap_or("");
                let sender = args["sender"].as_str();
                let ctx = ast::encode_eval_context(text, sender, &types::Population::default());
                let violations: Vec<types::Violation> = st.defs_matching("constraint:").iter().flat_map(|(_, func)| {
                    let result = ast::apply(func, &ctx, &def_map);
                    ast::decode_violations(&result)
                }).collect();
                serde_json::to_string(&violations).unwrap_or_else(|_| "[]".into())
            }
            "machine" => {
                let args: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
                let noun = args["noun"].as_str().unwrap_or("");
                let events: Vec<&str> = args["events"].as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();
                let sm_name = format!("machine:{}", noun);
                let init_name = format!("machine:{}:initial", noun);
                let transition = match st.def(&sm_name) { Some(f) => f, None => return r#"{"error":"no machine"}"#.into() };
                let initial_func = match st.def(&init_name) { Some(f) => f, None => return r#"{"error":"no initial"}"#.into() };
                let mut state = ast::apply(initial_func, &ast::Object::phi(), &def_map);
                for event in &events {
                    let inp = ast::Object::seq(vec![state, ast::Object::atom(event)]);
                    state = ast::apply(transition, &inp, &def_map);
                }
                let current = state.as_atom().unwrap_or("").to_string();
                // HATEOAS: available transitions from current state
                let events_all: Vec<String> = st.pop.facts.get("InstanceFact")
                    .map(|facts| facts.iter()
                        .filter(|f| f.bindings.iter().any(|(k, v)| k == "subjectNoun" && v == "Transition"))
                        .filter_map(|f| {
                            let obj_noun = f.bindings.iter().find(|(k, _)| k == "objectNoun").map(|(_, v)| v.as_str())?;
                            if obj_noun == "Event Type" { f.bindings.iter().find(|(k, _)| k == "objectValue").map(|(_, v)| v.clone()) } else { None }
                        })
                        .collect::<std::collections::HashSet<_>>().into_iter().collect()
                    ).unwrap_or_default();
                let mut available = vec![];
                for ev in &events_all {
                    let inp = ast::Object::seq(vec![ast::Object::atom(&current), ast::Object::atom(ev)]);
                    let next = ast::apply(transition, &inp, &def_map);
                    if let Some(ns) = next.as_atom() {
                        if ns != current { available.push(serde_json::json!({"event": ev, "to": ns})); }
                    }
                }
                serde_json::to_string(&serde_json::json!({
                    "output": { "state": current, "transitions": available },
                    "newstate": { "status": current }
                })).unwrap_or_else(|_| "{}".into())
            }
            "transitions" => {
                let args: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
                let noun = args["noun"].as_str().unwrap_or("");
                let status = args["status"].as_str().unwrap_or("");
                let sm_name = format!("machine:{}", noun);
                let transition = match st.def(&sm_name) { Some(f) => f, None => return "[]".into() };
                let events: Vec<String> = st.pop.facts.get("InstanceFact")
                    .map(|facts| facts.iter()
                        .filter(|f| f.bindings.iter().any(|(k, v)| k == "subjectNoun" && v == "Transition"))
                        .filter_map(|f| {
                            let obj_noun = f.bindings.iter().find(|(k, _)| k == "objectNoun").map(|(_, v)| v.as_str())?;
                            if obj_noun == "Event Type" { f.bindings.iter().find(|(k, _)| k == "objectValue").map(|(_, v)| v.clone()) } else { None }
                        })
                        .collect::<std::collections::HashSet<_>>().into_iter().collect()
                    ).unwrap_or_default();
                let mut result = vec![];
                for event in &events {
                    let inp = ast::Object::seq(vec![ast::Object::atom(status), ast::Object::atom(event)]);
                    let next = ast::apply(transition, &inp, &def_map);
                    if let Some(ns) = next.as_atom() {
                        if ns != status { result.push(serde_json::json!({"from": status, "to": ns, "event": event})); }
                    }
                }
                serde_json::to_string(&result).unwrap_or_else(|_| "[]".into())
            }
            "rho" => {
                let args: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
                let fact_type = args["factType"].as_str().unwrap_or("");
                let operation = args["operation"].as_str().unwrap_or("");
                let mut elements = vec![ast::Object::atom(fact_type)];
                if let Some(bindings) = args["bindings"].as_array() {
                    for b in bindings {
                        elements.push(ast::Object::seq(vec![
                            ast::Object::atom(b["noun"].as_str().unwrap_or("")),
                            ast::Object::atom(b["value"].as_str().unwrap_or("")),
                        ]));
                    }
                }
                let fact = ast::Object::Seq(elements);
                let func = ast::metacompose(&fact, &def_map);
                let result = ast::apply(&func, &ast::Object::atom(operation), &def_map);
                match result.as_atom() { Some(s) => s.to_string(), None => format!("{:?}", result) }
            }
            "nav" => {
                let args: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
                let noun = args["noun"].as_str().unwrap_or("");
                let children = st.def(&format!("nav:{}:children", noun))
                    .map(|f| ast::apply(f, &ast::Object::phi(), &def_map))
                    .and_then(|r| r.as_seq().map(|s| s.iter().filter_map(|o| o.as_atom().map(|a| a.to_string())).collect::<Vec<_>>()))
                    .unwrap_or_default();
                let parents = st.def(&format!("nav:{}:parent", noun))
                    .map(|f| ast::apply(f, &ast::Object::phi(), &def_map))
                    .and_then(|r| r.as_seq().map(|s| s.iter().filter_map(|o| o.as_atom().map(|a| a.to_string())).collect::<Vec<_>>()))
                    .unwrap_or_default();
                serde_json::to_string(&serde_json::json!({"children": children, "parent": parents})).unwrap_or_else(|_| "{}".into())
            }
            "defs" => {
                let names: Vec<&str> = st.defs.iter().map(|(n, _)| n.as_str()).collect();
                serde_json::to_string(&names).unwrap_or_else(|_| "[]".into())
            }
            "rmap" => {
                let d = st.domain();
                serde_json::to_string(&rmap::rmap(&d)).unwrap_or_else(|_| "[]".into())
            }
            "synthesize" => {
                let args: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
                let noun = args["noun"].as_str().unwrap_or("");
                let depth = args["depth"].as_u64().unwrap_or(2) as usize;
                let d = st.domain();
                serde_json::to_string(&evaluate::synthesize(&st.model, &d, noun, depth)).unwrap_or_else(|_| "{}".into())
            }
            "forward_chain" => {
                let mut pop = st.pop.clone();
                let derived = evaluate::forward_chain_ast(&st.model, &mut pop);
                serde_json::to_string(&derived).unwrap_or_else(|_| "[]".into())
            }
            _ => format!(r#"{{"error":"unknown key: {}"}}"#, key),
        }
    }
}
