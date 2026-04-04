// crates/arest/src/lib.rs â€” AREST WIT Component

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
    model: compile::CompiledModel,
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

// All WIT types are referenced as exports::graphdl::arest::engine::T
// and graphdl::arest::types::T. No abbreviations possible due to
// macro expansion ordering.

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

    fn apply_command(handle: u32, noun: String, id: String, op: String,
        fields: Vec<exports::graphdl::arest::engine::Binding>,
        pop: exports::graphdl::arest::engine::Population,
    ) -> Result<exports::graphdl::arest::engine::CommandResult, String> {
        let s = ds().lock().unwrap();
        let st = s.get(handle as usize).and_then(|x| x.as_ref()).ok_or("no domain")?;
        let p = ipop(&pop);
        let fm: HashMap<String, String> = fields.iter().map(|b| (b.noun.clone(), b.value.clone())).collect();
        let cmd = match op.as_str() {
            "create" => arest::Command::CreateEntity { noun, domain: String::new(), id: Some(id), fields: fm },
            "transition" => arest::Command::Transition { entity_id: id, event: fm.get("event").cloned().unwrap_or_default(), domain: String::new(), current_status: fm.get("status").cloned() },
            "update" => arest::Command::UpdateEntity { noun, domain: String::new(), entity_id: id, fields: fm },
            _ => return Err(format!("unknown op: {}", op)),
        };
        let r = arest::apply_command(&st.model, &cmd, &p);
        Ok(exports::graphdl::arest::engine::CommandResult {
            entity_id: r.entities.first().map(|e| e.id.clone()).unwrap_or_default(),
            status: r.status.clone(),
            violations: r.violations.iter().map(ovio).collect(),
            derived_facts: vec![],
            available_transitions: r.transitions.iter().map(|t| exports::graphdl::arest::engine::Transition { from_status: String::new(), to_status: t.target_status.clone(), event: t.event.clone() }).collect(),
        })
    }

    fn run_machine(handle: u32, noun: String, events: Vec<String>) -> Result<String, String> {
        let s = ds().lock().unwrap();
        let st = s.get(handle as usize).and_then(|x| x.as_ref()).ok_or("no domain")?;
        let sm_name = format!("machine:{}", noun);
        let init_name = format!("machine:{}:initial", noun);
        let transition = st.def(&sm_name).ok_or(format!("no sm for '{}'", noun))?;
        let initial_func = st.def(&init_name).ok_or(format!("no initial for '{}'", noun))?;
        let def_map: HashMap<String, ast::Func> = st.defs.iter().map(|(n, f)| (n.clone(), f.clone())).collect();
        // Get initial state
        let mut state = ast::apply(initial_func, &ast::Object::phi(), &def_map);
        // Fold transition function over events
        for event in &events {
            let input = ast::Object::seq(vec![state, ast::Object::atom(event)]);
            state = ast::apply(transition, &input, &def_map);
        }
        Ok(state.as_atom().unwrap_or("").to_string())
    }

    fn get_transitions(handle: u32, noun: String, status: String) -> Vec<exports::graphdl::arest::engine::Transition> {
        // Fall back to CompiledModel for now. The transition table is needed
        // to enumerate all possible events. With pure FFP, this would query
        // the transition facts in P and apply the transition function to each.
        let s = ds().lock().unwrap();
        let st = match s.get(handle as usize).and_then(|x| x.as_ref()) { Some(x) => x, None => return vec![] };
        let idx = st.model.noun_index.noun_to_state_machines.get(&noun);
        match idx.and_then(|&i| st.model.state_machines.get(i)) {
            Some(sm) => sm.transition_table.iter().filter(|(f, _, _)| f == &status)
                .map(|(f, t, ev)| exports::graphdl::arest::engine::Transition { from_status: f.clone(), to_status: t.clone(), event: ev.clone() }).collect(),
            None => vec![],
        }
    }

    fn evaluate_response(handle: u32, text: String, sender: Option<String>, pop: exports::graphdl::arest::engine::Population) -> Vec<exports::graphdl::arest::engine::Violation> {
        let s = ds().lock().unwrap();
        let st = match s.get(handle as usize).and_then(|x| x.as_ref()) { Some(x) => x, None => return vec![] };
        let p = ipop(&pop);
        let ctx = ast::encode_eval_context(&text, sender.as_deref(), &p);
        let def_map: HashMap<String, ast::Func> = st.defs.iter().map(|(n, f)| (n.clone(), f.clone())).collect();
        // Apply each constraint func to the eval context
        st.defs_matching("constraint:").iter().flat_map(|(_, func)| {
            let result = ast::apply(func, &ctx, &def_map);
            ast::decode_violations(&result)
        }).map(|v| ovio(&v)).collect()
    }

    fn get_deontic_constraints(handle: u32, noun: String) -> Vec<exports::graphdl::arest::engine::DeonticConstraint> {
        let s = ds().lock().unwrap();
        let st = match s.get(handle as usize).and_then(|x| x.as_ref()) { Some(x) => x, None => return vec![] };
        // Query Constraint facts from Population for deontic constraints on this noun
        let d = st.domain();
        let mut out = Vec::new();
        for c in &d.constraints {
            if c.modality != "deontic" { continue; }
            // Check if constraint's entity matches the requested noun
            let entity = c.entity.as_deref().unwrap_or("");
            if !entity.is_empty() && entity != noun { continue; }
            // Check if any span references a fact type involving this noun
            if entity.is_empty() {
                let involves_noun = c.spans.iter().any(|s| {
                    d.fact_types.get(&s.fact_type_id).map_or(false, |ft| ft.roles.iter().any(|r| r.noun_name == noun))
                });
                if !involves_noun { continue; }
            }
            let ev: Vec<String> = compile::collect_enum_values_pub(&d, &c.spans).into_iter().flat_map(|(_, v)| v).collect();
            let wa = if ev.is_empty() { graphdl::arest::types::WorldAssumption::Open } else { graphdl::arest::types::WorldAssumption::Closed };
            out.push(exports::graphdl::arest::engine::DeonticConstraint {
                id: c.id.clone(), text: c.text.clone(),
                entity: c.entity.clone(),
                operator: c.deontic_operator.clone().unwrap_or_default(),
                enum_values: ev,
                fact_type_id: c.spans.first().map(|s| s.fact_type_id.clone()),
                assumption: wa,
            });
        }
        out
    }

    fn evaluate_constraint(handle: u32, cid: String, text: String, sender: Option<String>, pop: exports::graphdl::arest::engine::Population) -> exports::graphdl::arest::engine::ConstraintResult {
        let none = exports::graphdl::arest::engine::ConstraintResult { violated: false, violation: None };
        let s = ds().lock().unwrap();
        let st = match s.get(handle as usize).and_then(|x| x.as_ref()) { Some(x) => x, None => return none };
        let p = ipop(&pop);
        let ctx = ast::encode_eval_context(&text, sender.as_deref(), &p);
        let def_map: HashMap<String, ast::Func> = st.defs.iter().map(|(n, f)| (n.clone(), f.clone())).collect();
        let constraint_key = format!("constraint:{}", cid);
        let func = match st.def(&constraint_key) { Some(f) => f, None => return none };
        let result = ast::apply(func, &ctx, &def_map);
        let vv = ast::decode_violations(&result);
        if vv.is_empty() { none } else { exports::graphdl::arest::engine::ConstraintResult { violated: true, violation: Some(ovio(&vv[0])) } }
    }

    fn forward_chain(handle: u32, pop: exports::graphdl::arest::engine::Population) -> Vec<exports::graphdl::arest::engine::DerivedFact> {
        // Forward chaining requires iteration to fixed point.
        // Keep using CompiledModel for now. The derivation funcs in defs
        // are individual rules. The fixed-point loop is a system-level concern.
        let s = ds().lock().unwrap();
        let st = match s.get(handle as usize).and_then(|x| x.as_ref()) { Some(x) => x, None => return vec![] };
        let mut p = ipop(&pop);
        evaluate::forward_chain_ast(&st.model, &mut p).iter().map(|d| exports::graphdl::arest::engine::DerivedFact {
            fact_type_id: d.fact_type_id.clone(), reading: d.reading.clone(),
            bindings: d.bindings.iter().map(|(n, v)| exports::graphdl::arest::engine::Binding { noun: n.clone(), value: v.clone() }).collect(),
            derived_by: d.derived_by.clone(),
        }).collect()
    }

    fn query(_handle: u32, fact_type_id: String, target_noun: String, filter: Vec<exports::graphdl::arest::engine::Binding>, pop: exports::graphdl::arest::engine::Population) -> Vec<String> {
        let p = ipop(&pop);
        let pred = query::QueryPredicate { fact_type_id, target_noun, filter_bindings: filter.iter().map(|b| (b.noun.clone(), b.value.clone())).collect() };
        query::query_population(&p, &pred).matches
    }

    fn induce(handle: u32, pop: exports::graphdl::arest::engine::Population) -> String {
        let s = ds().lock().unwrap();
        let st = match s.get(handle as usize).and_then(|x| x.as_ref()) { Some(x) => x, None => return "{}".into() };
        let d = st.domain();
        serde_json::to_string(&induce::induce(&d, &ipop(&pop))).unwrap_or_else(|_| "{}".into())
    }

    fn synthesize(handle: u32, noun: String, depth: u32) -> String {
        let s = ds().lock().unwrap();
        let st = match s.get(handle as usize).and_then(|x| x.as_ref()) { Some(x) => x, None => return "{}".into() };
        let d = st.domain();
        serde_json::to_string(&evaluate::synthesize(&st.model, &d, &noun, depth as usize)).unwrap_or_else(|_| "{}".into())
    }

    fn prove(handle: u32, goal: String, pop: exports::graphdl::arest::engine::Population, wa: graphdl::arest::types::WorldAssumption) -> String {
        let s = ds().lock().unwrap();
        let st = match s.get(handle as usize).and_then(|x| x.as_ref()) { Some(x) => x, None => return "{}".into() };
        let w = match wa { graphdl::arest::types::WorldAssumption::Open => types::WorldAssumption::Open, graphdl::arest::types::WorldAssumption::Closed => types::WorldAssumption::Closed };
        let d = st.domain();
        serde_json::to_string(&evaluate::prove(&d, &ipop(&pop), &goal, &w)).unwrap_or_else(|_| "{}".into())
    }

    fn rmap(handle: u32) -> String {
        let s = ds().lock().unwrap();
        let st = match s.get(handle as usize).and_then(|x| x.as_ref()) { Some(x) => x, None => return "[]".into() };
        let d = st.domain();
        serde_json::to_string(&rmap::rmap(&d)).unwrap_or_else(|_| "[]".into())
    }
}

fn ipop(p: &exports::graphdl::arest::engine::Population) -> types::Population {
    let mut f: HashMap<String, Vec<types::FactInstance>> = HashMap::new();
    for fact in &p.facts {
        f.entry(fact.fact_type_id.clone()).or_default().push(types::FactInstance {
            fact_type_id: fact.fact_type_id.clone(),
            bindings: fact.bindings.iter().map(|b| (b.noun.clone(), b.value.clone())).collect(),
        });
    }
    types::Population { facts: f }
}

fn ovio(v: &types::Violation) -> exports::graphdl::arest::engine::Violation {
    exports::graphdl::arest::engine::Violation {
        constraint_id: v.constraint_id.clone(), constraint_text: v.constraint_text.clone(),
        detail: v.detail.clone(), alethic: v.alethic,
    }
}
