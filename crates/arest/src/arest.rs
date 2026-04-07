// crates/arest/src/arest.rs
//
// AREST -- Applicative REpresentational State Transfer
//
// Command : State -> (State', Representation)
//
// The command is compiled from readings. The engine applies it.
// The result is the new state and a hypermedia representation
// with HATEOAS links showing valid state transitions.

use serde::{Serialize, Deserialize};
use crate::types::*;
use crate::ast;

// -- Commands ---------------------------------------------------------

/// The five input classes from Backus Section 14.4.2.
/// Each corresponds to an AREST operation.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Command {
    /// is-cmd: execute with validation (create entity with SM, constraints)
    CreateEntity {
        noun: String,
        domain: String,
        id: Option<String>,
        fields: std::collections::HashMap<String, String>,
    },
    /// is-cmd: state machine transition
    Transition {
        #[serde(alias = "entityId")]
        entity_id: String,
        event: String,
        domain: String,
        #[serde(alias = "currentStatus", default)]
        current_status: Option<String>,
    },
    /// is-qry: query the population (partial application of graph schema)
    Query {
        #[serde(alias = "schemaId")]
        schema_id: String,
        domain: String,
        target: String,
        bindings: std::collections::HashMap<String, String>,
    },
    /// is-upd: update entity fields (<->F  .  [upd, defs])
    UpdateEntity {
        noun: String,
        domain: String,
        #[serde(alias = "entityId")]
        entity_id: String,
        fields: std::collections::HashMap<String, String>,
    },
    /// is-chg: install or update readings (modify definitions D)
    LoadReadings {
        markdown: String,
        domain: String,
    },
}

// -- Result -----------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    pub entities: Vec<EntityResult>,
    pub status: Option<String>,
    pub transitions: Vec<TransitionAction>,
    pub violations: Vec<Violation>,
    pub derived_count: usize,
    pub rejected: bool,
    /// The transformed state -- the authoritative state after this command.
    #[serde(skip)]
    pub state: ast::Object,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityResult {
    pub id: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    pub data: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionAction {
    pub event: String,
    pub target_status: String,
    pub method: String,
    pub href: String,
}

// -- Encode/decode bridge (Object ↔ CommandResult) --------------------

/// Encode command input as Object for compiled handler Func.
/// create: <entity_id, <<field_name, value>, ...>, domain, state>
pub fn encode_create_input(
    entity_id: &str, fields: &std::collections::HashMap<String, String>,
    domain: &str, state: &ast::Object,
) -> ast::Object {
    let field_seq = ast::Object::Seq(
        fields.iter().map(|(k, v)| ast::Object::seq(vec![ast::Object::atom(k), ast::Object::atom(v)])).collect()
    );
    ast::Object::seq(vec![ast::Object::atom(entity_id), field_seq, ast::Object::atom(domain), state.clone()])
}

/// Encode transition input: <entity_id, event, current_status_or_phi, state>
pub fn encode_transition_input(
    entity_id: &str, event: &str, current_status: Option<&str>, state: &ast::Object,
) -> ast::Object {
    let status_obj = current_status.map(ast::Object::atom).unwrap_or(ast::Object::phi());
    ast::Object::seq(vec![ast::Object::atom(entity_id), ast::Object::atom(event), status_obj, state.clone()])
}

/// Encode update input: <entity_id, <<field_name, value>, ...>, noun, domain, state>
pub fn encode_update_input(
    entity_id: &str, fields: &std::collections::HashMap<String, String>,
    noun: &str, domain: &str, state: &ast::Object,
) -> ast::Object {
    let field_seq = ast::Object::Seq(
        fields.iter().map(|(k, v)| ast::Object::seq(vec![ast::Object::atom(k), ast::Object::atom(v)])).collect()
    );
    ast::Object::seq(vec![
        ast::Object::atom(entity_id), field_seq,
        ast::Object::atom(noun), ast::Object::atom(domain), state.clone(),
    ])
}

/// Decode a compiled handler's Object result into CommandResult.
/// Expected: <entities, status, transitions, violations, derived_count, rejected, new_state>
pub fn decode_command_result(obj: &ast::Object) -> CommandResult {
    let items = obj.as_seq().unwrap_or(&[]);
    let sel = |i: usize| items.get(i);

    let entities = sel(0).and_then(|o| o.as_seq()).map(|es| {
        es.iter().filter_map(|e| {
            let parts = e.as_seq()?;
            let id = parts.get(0)?.as_atom()?.to_string();
            let entity_type = parts.get(1)?.as_atom()?.to_string();
            let data = parts.get(2)?.as_seq().map(|pairs| {
                pairs.iter().filter_map(|p| {
                    let kv = p.as_seq()?;
                    Some((kv.get(0)?.as_atom()?.to_string(), kv.get(1)?.as_atom()?.to_string()))
                }).collect()
            }).unwrap_or_default();
            Some(EntityResult { id, entity_type, data })
        }).collect()
    }).unwrap_or_default();

    let status = sel(1).and_then(|o| o.as_atom()).map(|s| s.to_string());

    let transitions = sel(2).and_then(|o| o.as_seq()).map(|ts| {
        ts.iter().filter_map(|t| {
            let parts = t.as_seq()?;
            Some(TransitionAction {
                event: parts.get(0)?.as_atom()?.to_string(),
                target_status: parts.get(1)?.as_atom()?.to_string(),
                method: parts.get(2)?.as_atom()?.to_string(),
                href: parts.get(3)?.as_atom()?.to_string(),
            })
        }).collect()
    }).unwrap_or_default();

    let violations = sel(3).and_then(|o| o.as_seq()).map(|vs| {
        vs.iter().filter_map(|v| ast::decode_violation(v)).collect()
    }).unwrap_or_default();

    let derived_count = sel(4).and_then(|o| o.as_atom())
        .and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
    let rejected = sel(5).and_then(|o| o.as_atom()) == Some("T");
    let new_state = sel(6).cloned().unwrap_or(ast::Object::phi());

    CommandResult { entities, status, transitions, violations, derived_count, rejected, state: new_state }
}

/// Encode a CommandResult as an Object (inverse of decode_command_result).
pub fn encode_command_result(result: &CommandResult) -> ast::Object {
    let entities = ast::Object::Seq(result.entities.iter().map(|e| {
        let data = ast::Object::Seq(e.data.iter().map(|(k, v)| {
            ast::Object::seq(vec![ast::Object::atom(k), ast::Object::atom(v)])
        }).collect());
        ast::Object::seq(vec![ast::Object::atom(&e.id), ast::Object::atom(&e.entity_type), data])
    }).collect());

    let status = result.status.as_ref().map(|s| ast::Object::atom(s)).unwrap_or(ast::Object::phi());

    let transitions = ast::Object::Seq(result.transitions.iter().map(|t| {
        ast::Object::seq(vec![
            ast::Object::atom(&t.event), ast::Object::atom(&t.target_status),
            ast::Object::atom(&t.method), ast::Object::atom(&t.href),
        ])
    }).collect());

    let violations = ast::Object::Seq(result.violations.iter().map(|v| {
        ast::Object::seq(vec![
            ast::Object::atom(&v.constraint_id), ast::Object::atom(&v.constraint_text),
            ast::Object::atom(&v.detail), ast::Object::atom(if v.alethic { "T" } else { "F" }),
        ])
    }).collect());

    ast::Object::seq(vec![
        entities, status, transitions, violations,
        ast::Object::atom(&result.derived_count.to_string()),
        if result.rejected { ast::Object::t() } else { ast::Object::f() },
        result.state.clone(),
    ])
}

// -- Apply ------------------------------------------------------------

pub fn apply_command_defs(
    defs: &std::collections::HashMap<String, ast::Func>,
    command: &Command,
    state: &ast::Object,
) -> CommandResult {
    match command {
        Command::CreateEntity { noun, domain, id, fields } => {
            create_via_defs(defs, noun, domain, id.as_deref(), fields, state)
        }
        Command::Transition { entity_id, event, domain, current_status } => {
            transition_via_defs(defs, entity_id, event, domain, current_status.as_deref(), state)
        }
        Command::Query { schema_id, domain: _, target, bindings } => {
            query_via_defs(defs, schema_id, target, bindings, state)
        }
        Command::UpdateEntity { noun, domain, entity_id, fields } => {
            update_via_defs(defs, noun, domain, entity_id, fields, state)
        }
        Command::LoadReadings { markdown, domain } => {
            apply_load_readings(markdown, domain, state)
        }
        #[allow(unreachable_patterns)]
        _ => CommandResult {
            entities: vec![],
            status: None,
            transitions: vec![],
            violations: vec![],
            derived_count: 0,
            rejected: false,
            state: state.clone(),
        },
    }
}

fn create_via_defs(
    defs: &std::collections::HashMap<String, ast::Func>,
    noun: &str,
    domain: &str,
    explicit_id: Option<&str>,
    fields: &std::collections::HashMap<String, String>,
    state: &ast::Object,
) -> CommandResult {
    let entity_id = explicit_id.unwrap_or("").to_string();

    let mut new_state = state.clone();
    let mut entity_data = fields.clone();
    entity_data.insert("domain".to_string(), domain.to_string());

    let resolve_key = format!("resolve:{}", noun);
    for (field_name, value) in &entity_data {
        let ft_id = defs.get(&resolve_key)
            .map(|f| ast::apply(f, &ast::Object::atom(&field_name.to_lowercase()), defs))
            .and_then(|o| o.as_atom().map(|s| s.to_string()))
            .unwrap_or_else(|| resolve_fact_type_id_defs(defs, noun, field_name));
        new_state = ast::cell_push(&ft_id, ast::fact_from_pairs(&[
            (noun, &entity_id),
            (field_name.as_str(), value.as_str()),
        ]), &new_state);
    }

    // -- derive --
    let derivation_defs: Vec<(&str, &ast::Func)> = defs.iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, f)| (n.as_str(), f))
        .collect();
    let (new_state, derived) = crate::evaluate::forward_chain_defs_state(&derivation_defs, &new_state);

    // Build entity result
    let mut entities = vec![EntityResult {
        id: entity_id.clone(),
        entity_type: noun.to_string(),
        data: entity_data,
    }];

    let sm_id = entity_id.clone();
    let status = extract_sm_status(&new_state, &sm_id);

    if let Some(ref st) = status {
        let mut sm_data = std::collections::HashMap::new();
        sm_data.insert("forResource".to_string(), entity_id.clone());
        sm_data.insert("currentlyInStatus".to_string(), st.clone());
        sm_data.insert("domain".to_string(), domain.to_string());
        entities.push(EntityResult {
            id: sm_id,
            entity_type: "State Machine".to_string(),
            data: sm_data,
        });
    }

    // Inject transition facts from compiled transitions_meta:{noun} (Theorem 3: T is in P)
    let new_state = defs.get(&format!("transitions_meta:{}", noun))
        .map(|f| {
            let triples = ast::apply(f, &ast::Object::phi(), defs);
            triples.as_seq().map(|facts| {
                facts.iter().fold(new_state.clone(), |s, fact| ast::cell_push("Transition", fact.clone(), &s))
            }).unwrap_or(new_state.clone())
        })
        .unwrap_or_else(|| inject_transition_facts(state, new_state));

    // -- validate --
    let ctx_obj = ast::encode_eval_context_state("", None, &new_state);
    let validate_func = defs.get("validate").cloned().unwrap_or(ast::Func::constant(ast::Object::phi()));
    let violation_obj = ast::apply(&validate_func, &ctx_obj, defs);
    let violations = ast::decode_violations(&violation_obj);
    let rejected = violations.iter().any(|v| v.alethic);

    // -- emit --
    let transitions = hateoas_from_state(&new_state, noun, &entity_id, status.as_deref());

    CommandResult {
        entities,
        status,
        transitions,
        violations,
        derived_count: derived.len(),
        rejected,
        state: if rejected { state.clone() } else { new_state },
    }
}

fn resolve_fact_type_id_defs(
    defs: &std::collections::HashMap<String, ast::Func>,
    noun: &str,
    field: &str,
) -> String {
    for (name, _) in defs {
        if !name.starts_with("schema:") { continue; }
        let schema_id = &name["schema:".len()..];
        if schema_id.contains(noun) && schema_id.contains(field) {
            return schema_id.to_string();
        }
    }
    format!("{}_has_{}", noun, field)
}

fn transition_via_defs(
    defs: &std::collections::HashMap<String, ast::Func>,
    entity_id: &str,
    event: &str,
    _domain: &str,
    current_status: Option<&str>,
    state: &ast::Object,
) -> CommandResult {
    let mut new_state = state.clone();
    let mut new_status = None;

    for (name, func) in defs {
        if !name.starts_with("machine:") || name.contains(":initial") { continue; }

        let initial_key = format!("{}:initial", name);
        let from_status = match current_status {
            Some(s) => s.to_string(),
            None => {
                if let Some(init_func) = defs.get(&initial_key) {
                    ast::apply(init_func, &ast::Object::phi(), defs)
                        .as_atom().unwrap_or("").to_string()
                } else { continue; }
            }
        };

        let input = ast::Object::seq(vec![
            ast::Object::atom(&from_status),
            ast::Object::atom(event),
        ]);
        let result = ast::apply(func, &input, defs);
        if let Some(next) = result.as_atom() {
            if next != from_status {
                new_status = Some(next.to_string());
                break;
            }
        }
    }

    if let Some(ref status) = new_status {
        // Update SM status fact in state
        let status_key = "StateMachine_has_currentlyInStatus";
        // Remove old status fact for this entity, add new one
        new_state = ast::cell_filter(status_key, |f| {
            !ast::binding_matches(f, "State Machine", entity_id)
        }, &new_state);
        new_state = ast::cell_push(status_key, ast::fact_from_pairs(&[
            ("State Machine", entity_id),
            ("currentlyInStatus", status.as_str()),
        ]), &new_state);
    }

    let status = new_status.or_else(|| current_status.map(|s| s.to_string()));

    // Inject transition facts for HATEOAS
    let new_state = inject_transition_facts(state, new_state);

    let noun = "";
    let transitions = hateoas_from_state(&new_state, noun, entity_id, status.as_deref());

    CommandResult {
        entities: vec![],
        status,
        transitions,
        violations: vec![],
        derived_count: 0,
        rejected: false,
        state: new_state,
    }
}

fn query_via_defs(
    defs: &std::collections::HashMap<String, ast::Func>,
    schema_id: &str,
    target: &str,
    bindings: &std::collections::HashMap<String, String>,
    state: &ast::Object,
) -> CommandResult {
    // Look up schema role names from state metadata
    let role_cell = ast::fetch_or_phi("Role", state);
    let role_names: Vec<String> = role_cell.as_seq()
        .map(|roles| {
            let mut matched: Vec<(usize, String)> = roles.iter()
                .filter(|r| ast::binding_matches(r, "graphSchema", schema_id))
                .filter_map(|r| {
                    let name = ast::binding(r, "nounName")?.to_string();
                    let pos: usize = ast::binding(r, "position").and_then(|v| v.parse().ok()).unwrap_or(0);
                    Some((pos, name))
                })
                .collect();
            matched.sort_by_key(|(p, _)| *p);
            matched.into_iter().map(|(_, n)| n).collect()
        })
        .unwrap_or_default();

    let mut filter_pairs: Vec<(usize, String)> = Vec::new();
    let mut target_role: usize = 0;
    for (i, name) in role_names.iter().enumerate() {
        if name == target { target_role = i + 1; }
        if let Some(value) = bindings.get(name) {
            filter_pairs.push((i + 1, value.clone()));
        }
    }

    let filter_refs: Vec<(usize, &str)> = filter_pairs.iter().map(|(i, v)| (*i, v.as_str())).collect();
    let schema = crate::compile::CompiledSchema {
        id: schema_id.to_string(),
        reading: String::new(),
        construction: defs.get(&format!("schema:{}", schema_id)).cloned().unwrap_or(ast::Func::Id),
        role_names: role_names.clone(),
    };
    let results = crate::query::query_with_ast(state, &schema, target_role, &filter_refs);

    let mut data = std::collections::HashMap::new();
    data.insert(String::from("matches"), results.join(","));
    data.insert(String::from("count"), results.len().to_string());

    CommandResult {
        entities: vec![EntityResult {
            id: format!("query:{}", schema_id),
            entity_type: String::from("QueryResult"),
            data,
        }],
        status: None,
        transitions: vec![],
        violations: vec![],
        derived_count: 0,
        rejected: false,
        state: state.clone(),
    }
}

fn update_via_defs(
    defs: &std::collections::HashMap<String, ast::Func>,
    noun: &str,
    domain: &str,
    entity_id: &str,
    new_fields: &std::collections::HashMap<String, String>,
    state: &ast::Object,
) -> CommandResult {
    // Read current facts for this entity
    let mut merged = std::collections::HashMap::new();
    for (_, contents) in ast::cells_iter(state) {
        if let Some(facts) = contents.as_seq() {
            for fact in facts {
                if let Some(pairs) = fact.as_seq() {
                    if pairs.len() >= 2 {
                        let v0 = pairs[0].as_seq().and_then(|p| p.get(1)?.as_atom().map(|s| s.to_string()));
                        let k1 = pairs[1].as_seq().and_then(|p| p.get(0)?.as_atom().map(|s| s.to_string()));
                        let v1 = pairs[1].as_seq().and_then(|p| p.get(1)?.as_atom().map(|s| s.to_string()));
                        if v0.as_deref() == Some(entity_id) {
                            if let (Some(k), Some(v)) = (k1, v1) {
                                merged.insert(k, v);
                            }
                        }
                    }
                }
            }
        }
    }
    for (k, v) in new_fields {
        merged.insert(k.clone(), v.clone());
    }

    // Remove old facts for this entity, insert merged
    let mut new_state = state.clone();
    for (field_name, value) in &merged {
        let ft_id = resolve_fact_type_id_defs(defs, noun, field_name);
        // Remove old facts for this entity in this fact type
        new_state = ast::cell_filter(&ft_id, |f| {
            // Keep facts that are NOT for this entity
            if let Some(pairs) = f.as_seq() {
                if pairs.len() >= 2 {
                    let v0 = pairs[0].as_seq().and_then(|p| p.get(1)?.as_atom());
                    return v0 != Some(entity_id);
                }
            }
            true
        }, &new_state);
        new_state = ast::cell_push(&ft_id, ast::fact_from_pairs(&[
            (noun, entity_id),
            (field_name.as_str(), value.as_str()),
        ]), &new_state);
    }

    // derive + validate + emit
    let derivation_defs: Vec<(&str, &ast::Func)> = defs.iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, f)| (n.as_str(), f))
        .collect();
    let (new_state, derived) = crate::evaluate::forward_chain_defs_state(&derivation_defs, &new_state);

    let ctx_obj = ast::encode_eval_context_state("", None, &new_state);
    let validate_func = defs.get("validate").cloned().unwrap_or(ast::Func::constant(ast::Object::phi()));
    let violation_obj = ast::apply(&validate_func, &ctx_obj, defs);
    let violations = ast::decode_violations(&violation_obj);
    let rejected = violations.iter().any(|v| v.alethic);
    let sm_id = entity_id.to_string();
    let status = extract_sm_status(&new_state, &sm_id);
    let transitions = hateoas_from_state(&new_state, noun, entity_id, status.as_deref());

    CommandResult {
        entities: vec![EntityResult {
            id: entity_id.to_string(),
            entity_type: noun.to_string(),
            data: merged,
        }],
        status,
        transitions,
        violations,
        derived_count: derived.len(),
        rejected,
        state: if rejected { state.clone() } else { new_state },
    }
}

fn apply_load_readings(
    markdown: &str,
    domain: &str,
    state: &ast::Object,
) -> CommandResult {
    match crate::parse_forml2::parse_markdown(markdown) {
        Ok(ir) => {
            let mut data = std::collections::HashMap::new();
            data.insert("domain".to_string(), domain.to_string());
            data.insert("nouns".to_string(), ir.nouns.len().to_string());
            data.insert("factTypes".to_string(), ir.fact_types.len().to_string());
            data.insert("constraints".to_string(), ir.constraints.len().to_string());
            data.insert("stateMachines".to_string(), ir.state_machines.len().to_string());

            CommandResult {
                entities: vec![EntityResult {
                    id: format!("schema:{}", domain),
                    entity_type: "SchemaLoaded".to_string(),
                    data,
                }],
                status: None,
                transitions: vec![],
                violations: vec![],
                derived_count: 0,
                rejected: false,
                state: state.clone(),
            }
        }
        Err(e) => {
            CommandResult {
                entities: vec![],
                status: None,
                transitions: vec![],
                violations: vec![crate::types::Violation {
                    constraint_id: "parse_error".to_string(),
                    constraint_text: "FORML 2 parse error".to_string(),
                    detail: e,
                    alethic: true,
                }],
                derived_count: 0,
                rejected: true,
                state: state.clone(),
            }
        }
    }
}

// -- Helpers ----------------------------------------------------------

/// Inject transition facts from InstanceFact entries into state.
fn inject_transition_facts(source_state: &ast::Object, mut target_state: ast::Object) -> ast::Object {
    let inst_cell = ast::fetch_or_phi("InstanceFact", source_state);
    if let Some(inst_facts) = inst_cell.as_seq() {
        let mut t_from: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut t_to: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut t_event: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for f in inst_facts {
            let subj_noun = ast::binding(f, "subjectNoun");
            let subj_val = ast::binding(f, "subjectValue").map(|s| s.to_string());
            let obj_noun = ast::binding(f, "objectNoun");
            let obj_val = ast::binding(f, "objectValue").map(|s| s.to_string());
            let field = ast::binding(f, "fieldName");
            if subj_noun == Some("Transition") {
                let sv = subj_val.unwrap_or_default();
                if obj_noun == Some("Status") {
                    let fld = field.unwrap_or("");
                    if fld.to_lowercase().contains("from") {
                        t_from.insert(sv, obj_val.unwrap_or_default());
                    } else if fld.to_lowercase().contains("to") {
                        t_to.insert(sv, obj_val.unwrap_or_default());
                    }
                } else if obj_noun == Some("Event Type") {
                    t_event.insert(sv, obj_val.unwrap_or_default());
                }
            }
        }
        for (t_name, from) in &t_from {
            if let Some(to) = t_to.get(t_name) {
                let event = t_event.get(t_name).cloned().unwrap_or_else(|| t_name.clone());
                target_state = ast::cell_push("Transition", ast::fact_from_pairs(&[
                    ("from", from.as_str()),
                    ("to", to.as_str()),
                    ("event", event.as_str()),
                ]), &target_state);
            }
        }
    }
    target_state
}

/// HATEOAS as Projection (Theorem 4a)
fn hateoas_from_state(
    state: &ast::Object,
    noun: &str,
    entity_id: &str,
    status: Option<&str>,
) -> Vec<TransitionAction> {
    let Some(status) = status else { return vec![] };
    let encoded = noun.replace(' ', "%20");

    let transition_cell = ast::fetch_or_phi("Transition", state);
    let transition_facts = match transition_cell.as_seq() {
        Some(facts) => facts,
        None => return vec![],
    };

    // Build ancestor set
    let mut ancestor_statuses: Vec<String> = vec![status.to_string()];
    let subtype_cell = ast::fetch_or_phi("Status is subtype of Status", state);
    if let Some(subtype_facts) = subtype_cell.as_seq() {
        let mut frontier = vec![status.to_string()];
        while let Some(current) = frontier.pop() {
            for fact in subtype_facts {
                if let Some(pairs) = fact.as_seq() {
                    if pairs.len() >= 2 {
                        let v0 = pairs[0].as_seq().and_then(|p| p.get(1)?.as_atom());
                        let v1 = pairs[1].as_seq().and_then(|p| p.get(1)?.as_atom());
                        if v0 == Some(&current) {
                            if let Some(supertype) = v1 {
                                if !ancestor_statuses.contains(&supertype.to_string()) {
                                    ancestor_statuses.push(supertype.to_string());
                                    frontier.push(supertype.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    transition_facts.iter()
        .filter(|fact| {
            ast::binding(fact, "from").map_or(false, |v| ancestor_statuses.iter().any(|a| a == v))
        })
        .filter_map(|fact| {
            let event = ast::binding(fact, "event")?.to_string();
            let to = ast::binding(fact, "to")?.to_string();
            Some(TransitionAction {
                event,
                target_status: to,
                method: "POST".to_string(),
                href: format!("/api/entities/{}/{}/transition", encoded, entity_id),
            })
        })
        .collect()
}

fn extract_sm_status(state: &ast::Object, sm_id: &str) -> Option<String> {
    let cell = ast::fetch_or_phi("StateMachine_has_currentlyInStatus", state);
    let facts = cell.as_seq()?;
    for fact in facts {
        if ast::binding_matches(fact, "State Machine", sm_id) {
            return ast::binding(fact, "currentlyInStatus").map(|s| s.to_string());
        }
        // Also check positional: first binding value == sm_id
        if let Some(pairs) = fact.as_seq() {
            let has_sm = pairs.iter().any(|pair| {
                pair.as_seq().and_then(|p| p.get(1)?.as_atom()).map_or(false, |v| v == sm_id)
            });
            if has_sm {
                return ast::binding(fact, "currentlyInStatus").map(|s| s.to_string());
            }
        }
    }
    None
}

fn to_camel_case(s: &str) -> String {
    s.split(' ')
        .enumerate()
        .map(|(i, w)| {
            if i == 0 {
                w.to_lowercase()
            } else {
                let mut c = w.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                    None => String::new(),
                }
            }
        })
        .collect()
}

// -- Tests ------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn command_result_round_trips_through_object() {
        let mut data = HashMap::new();
        data.insert("customer".to_string(), "acme".to_string());
        let result = CommandResult {
            entities: vec![EntityResult { id: "ord-1".into(), entity_type: "Order".into(), data }],
            status: Some("Draft".into()),
            transitions: vec![TransitionAction {
                event: "place".into(), target_status: "Placed".into(),
                method: "POST".into(), href: "/orders/ord-1/transition".into(),
            }],
            violations: vec![],
            derived_count: 2,
            rejected: false,
            state: ast::Object::phi(),
        };
        let obj = encode_command_result(&result);
        let decoded = decode_command_result(&obj);
        assert_eq!(decoded.entities.len(), 1);
        assert_eq!(decoded.entities[0].id, "ord-1");
        assert_eq!(decoded.entities[0].entity_type, "Order");
        assert_eq!(decoded.status, Some("Draft".into()));
        assert_eq!(decoded.transitions.len(), 1);
        assert_eq!(decoded.transitions[0].event, "place");
        assert_eq!(decoded.derived_count, 2);
        assert!(!decoded.rejected);
    }

    const STATE_METAMODEL: &str = r#"
# State

## Entity Types

Status(.Name) is an entity type.
State Machine Definition is a subtype of Status.
Transition(.id) is an entity type.
Event Type(.id) is an entity type.
Noun is an entity type.
Name is a value type.

## Fact Types

State Machine Definition is for Noun.
Status is initial in State Machine Definition.
Transition is defined in State Machine Definition.
Transition is from Status.
Transition is to Status.
Transition is triggered by Event Type.
"#;

    const ORDER_READINGS: &str = r#"
# Orders

## Entity Types

Order(.Order Number) is an entity type.

## Fact Types

Order has Amount.

## Instance Facts

State Machine Definition 'Order' is for Noun 'Order'.
Status 'Draft' is initial in State Machine Definition 'Order'.

Transition 'place' is defined in State Machine Definition 'Order'.
  Transition 'place' is from Status 'Draft'.
  Transition 'place' is to Status 'Placed'.
  Transition 'place' is triggered by Event Type 'place'.

Transition 'pay' is defined in State Machine Definition 'Order'.
  Transition 'pay' is from Status 'Placed'.
  Transition 'pay' is to Status 'Paid'.
  Transition 'pay' is triggered by Event Type 'pay'.

Transition 'cancel' is defined in State Machine Definition 'Order'.
  Transition 'cancel' is from Status 'Draft'.
  Transition 'cancel' is to Status 'Cancelled'.
  Transition 'cancel' is triggered by Event Type 'cancel'.
"#;

    /// Parse state metamodel + order domain readings, compile to defs,
    /// return (def_map, base_state).
    fn setup_order_defs() -> (HashMap<String, crate::ast::Func>, ast::Object) {
        let meta_state = crate::parse_forml2::parse_to_state(STATE_METAMODEL).unwrap();
        let orders_state = crate::parse_forml2::parse_to_state_with_nouns(ORDER_READINGS, &meta_state).unwrap();
        // Merge: push all cells from orders_state into meta_state
        let mut state = meta_state;
        for (name, contents) in ast::cells_iter(&orders_state) {
            if let Some(facts) = contents.as_seq() {
                for fact in facts {
                    state = ast::cell_push(name, fact.clone(), &state);
                }
            }
        }
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_map: HashMap<String, crate::ast::Func> = defs.iter().map(|(n, f)| (n.clone(), f.clone())).collect();
        (def_map, state)
    }

    #[test]
    fn create_entity_initializes_state_machine() {
        let (def_map, state) = setup_order_defs();

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-100".to_string());
        fields.insert("amount".to_string(), "999".to_string());

        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-100".to_string()),
            fields,
        };

        let result = apply_command_defs(&def_map, &cmd, &state);

        assert_eq!(result.entities[0].id, "ORD-100");
        assert_eq!(result.entities[0].entity_type, "Order");
        assert_eq!(result.entities[1].entity_type, "State Machine");
        assert_eq!(result.entities[1].data["currentlyInStatus"], "Draft");
        assert_eq!(result.entities[1].data["forResource"], "ORD-100");
        assert_eq!(result.status.as_deref(), Some("Draft"));
        assert_eq!(result.transitions.len(), 2); // place, cancel
        assert!(result.transitions.iter().any(|t| t.event == "place"));
        assert!(result.transitions.iter().any(|t| t.event == "cancel"));
        assert!(!result.rejected);
    }

    #[test]
    fn create_entity_with_explicit_id() {
        let (def_map, state) = setup_order_defs();

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-REF".to_string());
        fields.insert("amount".to_string(), "500".to_string());

        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-REF".to_string()),
            fields,
        };

        let result = apply_command_defs(&def_map, &cmd, &state);
        assert_eq!(result.entities[0].id, "ORD-REF");
    }

    #[test]
    fn create_entity_without_state_machine() {
        let (def_map, state) = setup_order_defs();

        let mut fields = HashMap::new();
        fields.insert("name".to_string(), "Electronics".to_string());

        let cmd = Command::CreateEntity {
            noun: "Category".to_string(),
            domain: "catalog".to_string(),
            id: Some("electronics".to_string()),
            fields,
        };

        let result = apply_command_defs(&def_map, &cmd, &state);

        assert_eq!(result.entities.len(), 1);
        assert!(result.status.is_none());
        assert!(result.transitions.is_empty());
    }

    #[test]
    fn transition_changes_status() {
        let (def_map, state) = setup_order_defs();

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-100".to_string());
        let create_cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-100".to_string()),
            fields,
        };
        let created = apply_command_defs(&def_map, &create_cmd, &state);
        assert_eq!(created.status.as_deref(), Some("Draft"));

        let cmd = Command::Transition {
            entity_id: "ORD-100".to_string(),
            event: "place".to_string(),
            domain: "orders".to_string(),
            current_status: Some("Draft".to_string()),
        };

        let result = apply_command_defs(&def_map, &cmd, &created.state);

        assert_eq!(result.status.as_deref(), Some("Placed"));
        assert!(result.transitions.iter().any(|t| t.event == "pay"));
    }

    #[test]
    fn state_contains_entity_facts() {
        let (def_map, state) = setup_order_defs();

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-1".to_string());
        fields.insert("customer".to_string(), "acme".to_string());

        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-1".to_string()),
            fields,
        };

        let result = apply_command_defs(&def_map, &cmd, &state);

        // Entity fields are facts in the state
        let customer_cell = ast::fetch_or_phi("Order_has_customer", &result.state);
        let customer_facts = customer_cell.as_seq().unwrap();
        assert_eq!(customer_facts.len(), 1);
        assert!(ast::binding(&customer_facts[0], "customer") == Some("acme"));

        // SM facts are in the state
        let sm_cell = ast::fetch_or_phi("StateMachine_has_currentlyInStatus", &result.state);
        let sm_facts = sm_cell.as_seq().unwrap();
        assert!(ast::binding(&sm_facts[0], "currentlyInStatus") == Some("Draft"));
    }

    #[test]
    fn transition_updates_state_status() {
        let (def_map, state) = setup_order_defs();

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-1".to_string());
        let create = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-1".to_string()),
            fields,
        };
        let created = apply_command_defs(&def_map, &create, &state);
        assert_eq!(created.status.as_deref(), Some("Draft"));

        let transition = Command::Transition {
            entity_id: "ORD-1".to_string(),
            event: "place".to_string(),
            domain: "orders".to_string(),
            current_status: Some("Draft".to_string()),
        };
        let result = apply_command_defs(&def_map, &transition, &created.state);

        assert_eq!(result.status.as_deref(), Some("Placed"));

        // State must contain the updated status
        let sm_cell = ast::fetch_or_phi("StateMachine_has_currentlyInStatus", &result.state);
        let sm_facts = sm_cell.as_seq().unwrap();
        let sm_fact = sm_facts.iter().find(|f|
            ast::binding_matches(f, "State Machine", "ORD-1")
        ).expect("SM fact must exist for ORD-1");
        assert_eq!(ast::binding(sm_fact, "currentlyInStatus"), Some("Placed"), "state must reflect new status");
    }

    #[test]
    fn query_command_returns_matches() {
        let (def_map, _) = setup_order_defs();

        let ft_id = "Order has customer";
        let mut state = ast::Object::phi();
        state = ast::cell_push(ft_id, ast::fact_from_pairs(&[("Order", "ord-1"), ("customer", "acme")]), &state);
        state = ast::cell_push(ft_id, ast::fact_from_pairs(&[("Order", "ord-2"), ("customer", "acme")]), &state);
        state = ast::cell_push(ft_id, ast::fact_from_pairs(&[("Order", "ord-3"), ("customer", "beta")]), &state);

        let mut bindings = HashMap::new();
        bindings.insert("customer".to_string(), "acme".to_string());

        let cmd = Command::Query {
            schema_id: ft_id.to_string(),
            domain: "orders".to_string(),
            target: "Order".to_string(),
            bindings,
        };

        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(!result.rejected);
        assert_eq!(result.entities[0].entity_type, "QueryResult");
    }

    #[test]
    fn load_readings_command_parses_markdown() {
        let (def_map, state) = setup_order_defs();

        let cmd = Command::LoadReadings {
            markdown: "# Test\n\nProduct(.SKU) is an entity type.\nCategory(.Name) is an entity type.\nProduct belongs to Category.\n  Each Product belongs to exactly one Category.".to_string(),
            domain: "catalog".to_string(),
        };

        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(!result.rejected);
        assert_eq!(result.entities[0].entity_type, "SchemaLoaded");
        assert_eq!(result.entities[0].data["nouns"], "2");
    }

    #[test]
    fn load_readings_command_reports_parse_error() {
        let (def_map, state) = setup_order_defs();

        let cmd = Command::LoadReadings {
            markdown: "".to_string(),
            domain: "empty".to_string(),
        };

        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(!result.rejected); // empty is valid
    }
}
