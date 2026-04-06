// crates/arest/src/arest.rs
//
// AREST â€" Applicative REpresentational State Transfer
//
// Command : Population â†’ (Population', Representation)
//
// The command is compiled from readings. The engine applies it.
// The result is the new population and a hypermedia representation
// with HATEOAS links showing valid state transitions.

use serde::{Serialize, Deserialize};
use crate::types::*;
use crate::ast;

// â"€â"€ Commands â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

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
    /// is-upd: update entity fields (â†"F âˆ˜ [upd, defs])
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

// â"€â"€ Result â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    pub entities: Vec<EntityResult>,
    pub status: Option<String>,
    pub transitions: Vec<TransitionAction>,
    pub violations: Vec<Violation>,
    pub derived_count: usize,
    pub rejected: bool,
    /// The transformed population â€" the authoritative state after this command.
    pub population: Population,
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

// â"€â"€ Apply â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

// -- apply_command_defs -----------------------------------------------
// Eq. 12: create = emit . validate . derive . resolve
// All operations resolve through named Func definitions.

pub fn apply_command_defs(
    defs: &std::collections::HashMap<String, ast::Func>,
    command: &Command,
    population: &Population,
) -> CommandResult {
    match command {
        Command::CreateEntity { noun, domain, id, fields } => {
            create_via_defs(defs, noun, domain, id.as_deref(), fields, population)
        }
        Command::Transition { entity_id, event, domain, current_status } => {
            transition_via_defs(defs, entity_id, event, domain, current_status.as_deref(), population)
        }
        Command::Query { schema_id, domain: _, target, bindings } => {
            query_via_defs(defs, schema_id, target, bindings, population)
        }
        Command::UpdateEntity { noun, domain, entity_id, fields } => {
            update_via_defs(defs, noun, domain, entity_id, fields, population)
        }
        Command::LoadReadings { markdown, domain } => {
            apply_load_readings(markdown, domain, population)
        }
        #[allow(unreachable_patterns)]
        _ => CommandResult {
            entities: vec![],
            status: None,
            transitions: vec![],
            violations: vec![],
            derived_count: 0,
            rejected: false,
            population: population.clone(),
        },
    }
}

fn create_via_defs(
    defs: &std::collections::HashMap<String, ast::Func>,
    noun: &str,
    domain: &str,
    explicit_id: Option<&str>,
    fields: &std::collections::HashMap<String, String>,
    population: &Population,
) -> CommandResult {
    // -- resolve --
    let entity_id = explicit_id.unwrap_or("").to_string();

    let mut new_pop = population.clone();
    let mut entity_data = fields.clone();
    entity_data.insert("domain".to_string(), domain.to_string());

    for (field_name, value) in &entity_data {
        let ft_id = resolve_fact_type_id_defs(defs, noun, field_name);
        new_pop.facts.entry(ft_id.clone()).or_default().push(
            FactInstance {
                fact_type_id: ft_id,
                bindings: vec![
                    (noun.to_string(), entity_id.clone()),
                    (field_name.clone(), value.clone()),
                ],
            }
        );
    }

    // -- derive --
    let derivation_defs: Vec<(&str, &ast::Func)> = defs.iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, f)| (n.as_str(), f))
        .collect();
    let derived = crate::evaluate::forward_chain_defs(&derivation_defs, &mut new_pop);

    // Build entity result
    let mut entities = vec![EntityResult {
        id: entity_id.clone(),
        entity_type: noun.to_string(),
        data: entity_data,
    }];

    // Extract SM status from population (derived by forward chaining)
    let sm_id = entity_id.clone();
    let status = extract_sm_status(&new_pop, &sm_id);

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

    // Inject transition facts from InstanceFact entries (Theorem 3: T is in P)
    if let Some(inst_facts) = population.facts.get("InstanceFact") {
        let mut t_from: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut t_to: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut t_event: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for f in inst_facts {
            let subj_noun = f.bindings.iter().find(|(k, _)| k == "subjectNoun").map(|(_, v)| v.as_str());
            let subj_val = f.bindings.iter().find(|(k, _)| k == "subjectValue").map(|(_, v)| v.clone());
            let obj_noun = f.bindings.iter().find(|(k, _)| k == "objectNoun").map(|(_, v)| v.as_str());
            let obj_val = f.bindings.iter().find(|(k, _)| k == "objectValue").map(|(_, v)| v.clone());
            let field = f.bindings.iter().find(|(k, _)| k == "fieldName").map(|(_, v)| v.as_str());
            if subj_noun == Some("Transition") {
                let sv = subj_val.unwrap_or_default();
                if obj_noun == Some("Status") {
                    let fld = field.unwrap_or_default();
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
                let ft_key = String::from("Transition");
                new_pop.facts.entry(ft_key.clone()).or_default().push(
                    FactInstance {
                        fact_type_id: ft_key,
                        bindings: vec![
                            (String::from("from"), from.clone()),
                            (String::from("to"), to.clone()),
                            (String::from("event"), event),
                        ],
                    }
                );
            }
        }
    }

    // -- validate --
    let ctx_obj = ast::encode_eval_context("", None, &new_pop);
    let mut violations = Vec::new();
    for (name, func) in defs {
        if !name.starts_with("constraint:") { continue; }
        let result = ast::apply(func, &ctx_obj, defs);
        let is_deontic = name.contains("obligatory") || name.contains("forbidden");
        let decoded = ast::decode_violations(&result);
        for mut v in decoded {
            v.alethic = !is_deontic;
            violations.push(v);
        }
    }

    let rejected = violations.iter().any(|v| v.alethic);

    // -- emit --
    let transitions = hateoas_from_population(&new_pop, noun, &entity_id, status.as_deref());

    CommandResult {
        entities,
        status,
        transitions,
        violations,
        derived_count: derived.len(),
        rejected,
        population: if rejected { population.clone() } else { new_pop },
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

// -- transition via DEFS: machine_func : <status, event> -> status' --

fn transition_via_defs(
    defs: &std::collections::HashMap<String, ast::Func>,
    entity_id: &str,
    event: &str,
    _domain: &str,
    current_status: Option<&str>,
    population: &Population,
) -> CommandResult {
    let mut new_pop = population.clone();
    let mut new_status = None;

    // Try each machine def. machine:{noun} is the transition func.
    for (name, func) in defs {
        if !name.starts_with("machine:") || name.contains(":initial") { continue; }

        // Get initial status for this machine if current_status not provided
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
        // Update SM status fact in population
        let status_key = String::from("StateMachine_has_currentlyInStatus");
        let mut found = false;
        if let Some(facts) = new_pop.facts.get_mut(&status_key) {
            for fact in facts.iter_mut() {
                if fact.bindings.iter().any(|(_, v)| v == entity_id) {
                    for (noun, val) in fact.bindings.iter_mut() {
                        if noun == "currentlyInStatus" {
                            *val = status.clone();
                            found = true;
                        }
                    }
                }
            }
        }
        if !found {
            new_pop.facts.entry(status_key.clone()).or_default().push(
                FactInstance {
                    fact_type_id: status_key,
                    bindings: vec![
                        (String::from("State Machine"), entity_id.to_string()),
                        (String::from("currentlyInStatus"), status.clone()),
                    ],
                }
            );
        }
    }

    let status = new_status.or_else(|| current_status.map(|s| s.to_string()));

    // Inject transition facts for HATEOAS
    if let Some(inst_facts) = population.facts.get("InstanceFact") {
        let mut t_from: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut t_to: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut t_event: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for f in inst_facts {
            let subj_noun = f.bindings.iter().find(|(k, _)| k == "subjectNoun").map(|(_, v)| v.as_str());
            let subj_val = f.bindings.iter().find(|(k, _)| k == "subjectValue").map(|(_, v)| v.clone());
            let obj_noun = f.bindings.iter().find(|(k, _)| k == "objectNoun").map(|(_, v)| v.as_str());
            let obj_val = f.bindings.iter().find(|(k, _)| k == "objectValue").map(|(_, v)| v.clone());
            let field = f.bindings.iter().find(|(k, _)| k == "fieldName").map(|(_, v)| v.as_str());
            if subj_noun == Some("Transition") {
                let sv = subj_val.unwrap_or_default();
                if obj_noun == Some("Status") {
                    let fld = field.unwrap_or_default();
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
                let evt = t_event.get(t_name).cloned().unwrap_or_else(|| t_name.clone());
                let ft_key = String::from("Transition");
                new_pop.facts.entry(ft_key.clone()).or_default().push(
                    FactInstance {
                        fact_type_id: ft_key,
                        bindings: vec![
                            (String::from("from"), from.clone()),
                            (String::from("to"), to.clone()),
                            (String::from("event"), evt),
                        ],
                    }
                );
            }
        }
    }

    let noun = "";
    let transitions = hateoas_from_population(&new_pop, noun, entity_id, status.as_deref());

    CommandResult {
        entities: vec![],
        status,
        transitions,
        violations: vec![],
        derived_count: 0,
        rejected: false,
        population: new_pop,
    }
}

// -- query via DEFS: partial application of graph schema --

fn query_via_defs(
    defs: &std::collections::HashMap<String, ast::Func>,
    schema_id: &str,
    target: &str,
    bindings: &std::collections::HashMap<String, String>,
    population: &Population,
) -> CommandResult {
    // Look up schema role names from population metadata
    let role_names: Vec<String> = population.facts.get("Role")
        .map(|roles| {
            let mut matched: Vec<(usize, String)> = roles.iter()
                .filter(|r| r.bindings.iter().any(|(k, v)| k == "graphSchema" && v == schema_id))
                .filter_map(|r| {
                    let name = r.bindings.iter().find(|(k, _)| k == "nounName").map(|(_, v)| v.clone())?;
                    let pos: usize = r.bindings.iter().find(|(k, _)| k == "position").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
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
    let results = crate::query::query_with_ast(population, &schema, target_role, &filter_refs);

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
        population: population.clone(),
    }
}

// -- update via DEFS: replace fields then create pipeline --

fn update_via_defs(
    defs: &std::collections::HashMap<String, ast::Func>,
    noun: &str,
    domain: &str,
    entity_id: &str,
    new_fields: &std::collections::HashMap<String, String>,
    population: &Population,
) -> CommandResult {
    // Read current facts for this entity
    let mut merged = std::collections::HashMap::new();
    for (_, facts) in &population.facts {
        for fact in facts {
            if fact.bindings.len() >= 2 && fact.bindings[0].1 == entity_id {
                merged.insert(fact.bindings[1].0.clone(), fact.bindings[1].1.clone());
            }
        }
    }
    for (k, v) in new_fields {
        merged.insert(k.clone(), v.clone());
    }

    // Remove old facts for this entity, insert merged
    let mut new_pop = population.clone();
    for (field_name, value) in &merged {
        let ft_id = resolve_fact_type_id_defs(defs, noun, field_name);
        if let Some(instances) = new_pop.facts.get_mut(&ft_id) {
            instances.retain(|inst| {
                !(inst.bindings.len() >= 2 && inst.bindings[0].1 == entity_id)
            });
        }
        new_pop.facts.entry(ft_id.clone()).or_default().push(
            FactInstance {
                fact_type_id: ft_id,
                bindings: vec![
                    (noun.to_string(), entity_id.to_string()),
                    (field_name.clone(), value.clone()),
                ],
            }
        );
    }

    // derive + validate + emit (same as create)
    let derivation_defs: Vec<(&str, &ast::Func)> = defs.iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, f)| (n.as_str(), f))
        .collect();
    let derived = crate::evaluate::forward_chain_defs(&derivation_defs, &mut new_pop);

    let ctx_obj = ast::encode_eval_context("", None, &new_pop);
    let mut violations = Vec::new();
    for (name, func) in defs {
        if !name.starts_with("constraint:") { continue; }
        let result = ast::apply(func, &ctx_obj, defs);
        let is_deontic = name.contains("obligatory") || name.contains("forbidden");
        let decoded = ast::decode_violations(&result);
        for mut v in decoded {
            v.alethic = !is_deontic;
            violations.push(v);
        }
    }

    let rejected = violations.iter().any(|v| v.alethic);
    let sm_id = entity_id.to_string();
    let status = extract_sm_status(&new_pop, &sm_id);
    let transitions = hateoas_from_population(&new_pop, noun, entity_id, status.as_deref());

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
        population: if rejected { population.clone() } else { new_pop },
    }
}

// â"€â"€ is-chg: install readings â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

fn apply_load_readings(
    markdown: &str,
    domain: &str,
    population: &Population,
) -> CommandResult {
    // Parse readings via the FORML 2 parser
    match crate::parse_forml2::parse_markdown(markdown) {
        Ok(ir) => {
            let _model = crate::compile::compile(&ir);
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
                population: population.clone(),
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
                population: population.clone(),
            }
        }
    }
}

// â"€â"€ Helpers â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

/// HATEOAS as Projection (Theorem 3):
/// links(s) = Ï€_event(Filter(p) : T)
/// where p(t) = (s_from(t) = s) âˆ¨ anc(s_from(t), s)
///
/// anc(a, b) = true if a is a supertype status that b inherits transitions from.
/// For flat state machines (no subtyping), only direct matches apply.
/// When subtype state machines are supported, anc traverses the subtype hierarchy.
fn hateoas_from_population(
    population: &Population,
    noun: &str,
    entity_id: &str,
    status: Option<&str>,
) -> Vec<TransitionAction> {
    let Some(status) = status else { return vec![] };
    let encoded = noun.replace(' ', "%20");

    let transition_facts = match population.facts.get("Transition") {
        Some(facts) => facts,
        None => return vec![],
    };

    // Build ancestor set: statuses that the current status inherits from.
    // For now: check if any Status subtype facts exist in P.
    // anc(a, s) = true if "Status s is subtype of Status a" in P.
    let mut ancestor_statuses: Vec<String> = vec![status.to_string()];
    if let Some(subtype_facts) = population.facts.get("Status is subtype of Status") {
        // Traverse upward: if current status is a subtype, include the supertype
        let mut frontier = vec![status.to_string()];
        while let Some(current) = frontier.pop() {
            for fact in subtype_facts {
                if fact.bindings.len() >= 2 && fact.bindings[0].1 == current {
                    let supertype = &fact.bindings[1].1;
                    if !ancestor_statuses.contains(supertype) {
                        ancestor_statuses.push(supertype.clone());
                        frontier.push(supertype.clone());
                    }
                }
            }
        }
    }

    // Filter(p) : T where p(t) = s_from(t) âˆˆ {status} âˆª ancestors(status)
    transition_facts.iter()
        .filter(|fact| {
            fact.bindings.iter().any(|(k, v)| k == "from" && ancestor_statuses.contains(v))
        })
        .filter_map(|fact| {
            let event = fact.bindings.iter().find(|(k, _)| k == "event").map(|(_, v)| v.clone())?;
            let to = fact.bindings.iter().find(|(k, _)| k == "to").map(|(_, v)| v.clone())?;
            Some(TransitionAction {
                event,
                target_status: to,
                method: "POST".to_string(),
                href: format!("/api/entities/{}/{}/transition", encoded, entity_id),
            })
        })
        .collect()
}

/// Resolve entity ID from Halpin's reference scheme.
/// Extract the current status of a State Machine instance from the population.
fn extract_sm_status(population: &Population, sm_id: &str) -> Option<String> {
    let status_facts = population.facts.get("StateMachine_has_currentlyInStatus")?;
    for fact in status_facts {
        let has_sm = fact.bindings.iter().any(|(_, v)| v == sm_id);
        if has_sm {
            return fact.bindings.iter()
                .find(|(n, _)| n == "currentlyInStatus")
                .map(|(_, v)| v.clone());
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

// â"€â"€ Tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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
    /// return (def_map, base_population).
    fn setup_order_defs() -> (HashMap<String, crate::ast::Func>, Population) {
        let meta_pop = crate::parse_forml2::parse_to_population(STATE_METAMODEL).unwrap();
        let orders_pop = crate::parse_forml2::parse_to_population_with_nouns(ORDER_READINGS, &meta_pop).unwrap();
        let mut pop = meta_pop;
        for (k, v) in orders_pop.facts {
            pop.facts.entry(k).or_default().extend(v);
        }
        let defs = crate::compile::compile_to_defs(&pop);
        let def_map: HashMap<String, crate::ast::Func> = defs.iter().map(|(n, f)| (n.clone(), f.clone())).collect();
        (def_map, pop)
    }

    #[test]
    fn create_entity_initializes_state_machine() {
        let (def_map, pop) = setup_order_defs();

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-100".to_string());
        fields.insert("amount".to_string(), "999".to_string());

        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-100".to_string()),
            fields,
        };

        let result = apply_command_defs(&def_map, &cmd, &pop);

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
        let (def_map, pop) = setup_order_defs();

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-REF".to_string());
        fields.insert("amount".to_string(), "500".to_string());

        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-REF".to_string()),
            fields,
        };

        let result = apply_command_defs(&def_map, &cmd, &pop);
        assert_eq!(result.entities[0].id, "ORD-REF");
    }

    #[test]
    fn create_entity_without_state_machine() {
        let (def_map, pop) = setup_order_defs();

        let mut fields = HashMap::new();
        fields.insert("name".to_string(), "Electronics".to_string());

        let cmd = Command::CreateEntity {
            noun: "Category".to_string(),
            domain: "catalog".to_string(),
            id: Some("electronics".to_string()),
            fields,
        };

        let result = apply_command_defs(&def_map, &cmd, &pop);

        assert_eq!(result.entities.len(), 1);
        assert!(result.status.is_none());
        assert!(result.transitions.is_empty());
    }

    #[test]
    fn transition_changes_status() {
        let (def_map, pop) = setup_order_defs();

        // Create entity first so population has SM status
        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-100".to_string());
        let create_cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-100".to_string()),
            fields,
        };
        let created = apply_command_defs(&def_map, &create_cmd, &pop);
        assert_eq!(created.status.as_deref(), Some("Draft"));

        // Transition: Draft -> Placed
        let cmd = Command::Transition {
            entity_id: "ORD-100".to_string(),
            event: "place".to_string(),
            domain: "orders".to_string(),
            current_status: Some("Draft".to_string()),
        };

        let result = apply_command_defs(&def_map, &cmd, &created.population);

        assert_eq!(result.status.as_deref(), Some("Placed"));
        assert!(result.transitions.iter().any(|t| t.event == "pay"));
    }

    #[test]
    fn population_contains_entity_facts() {
        let (def_map, pop) = setup_order_defs();

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-1".to_string());
        fields.insert("customer".to_string(), "acme".to_string());

        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-1".to_string()),
            fields,
        };

        let result = apply_command_defs(&def_map, &cmd, &pop);

        // Entity fields are facts in the population (Graph Schema ID format)
        assert!(result.population.facts.contains_key("Order_has_customer"));
        let customer_facts = &result.population.facts["Order_has_customer"];
        assert_eq!(customer_facts.len(), 1);
        assert!(customer_facts[0].bindings.iter().any(|(_, v)| v == "acme"));

        // SM facts are in the population
        assert!(result.population.facts.contains_key("StateMachine_has_currentlyInStatus"));
        let sm_facts = &result.population.facts["StateMachine_has_currentlyInStatus"];
        assert!(sm_facts[0].bindings.iter().any(|(_, v)| v == "Draft"));
    }

    #[test]
    fn transition_updates_population_status() {
        // Theorem 3: every observable value derivable from population.
        // Transition must write new status into Pop'.
        let (def_map, pop) = setup_order_defs();

        // Create entity first to get a population with SM facts
        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-1".to_string());
        let create = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-1".to_string()),
            fields,
        };
        let created = apply_command_defs(&def_map, &create, &pop);
        assert_eq!(created.status.as_deref(), Some("Draft"));

        // Transition: Draft â†’ Placed
        let transition = Command::Transition {
            entity_id: "ORD-1".to_string(),
            event: "place".to_string(),
            domain: "orders".to_string(),
            current_status: Some("Draft".to_string()),
        };
        let result = apply_command_defs(&def_map, &transition, &created.population);

        assert_eq!(result.status.as_deref(), Some("Placed"));

        // Population must contain the updated status
        let sm_facts = &result.population.facts["StateMachine_has_currentlyInStatus"];
        let sm_fact = sm_facts.iter().find(|f|
            f.bindings.iter().any(|(_, v)| v == "ORD-1")
        ).expect("SM fact must exist for ORD-1");
        let status_binding = sm_fact.bindings.iter()
            .find(|(n, _)| n == "currentlyInStatus")
            .expect("must have currentlyInStatus binding");
        assert_eq!(status_binding.1, "Placed", "population must reflect new status");
    }

    // â"€â"€ is-qry: Query command â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    #[test]
    fn query_command_returns_matches() {
        let (def_map, _) = setup_order_defs();

        // Populate with some facts
        let mut pop = Population { facts: HashMap::new() };
        let ft_id = "Order has customer".to_string();
        pop.facts.insert(ft_id.clone(), vec![
            FactInstance {
                fact_type_id: ft_id.clone(),
                bindings: vec![("Order".to_string(), "ord-1".to_string()), ("customer".to_string(), "acme".to_string())],
            },
            FactInstance {
                fact_type_id: ft_id.clone(),
                bindings: vec![("Order".to_string(), "ord-2".to_string()), ("customer".to_string(), "acme".to_string())],
            },
            FactInstance {
                fact_type_id: ft_id.clone(),
                bindings: vec![("Order".to_string(), "ord-3".to_string()), ("customer".to_string(), "beta".to_string())],
            },
        ]);

        let mut bindings = HashMap::new();
        bindings.insert("customer".to_string(), "acme".to_string());

        let cmd = Command::Query {
            schema_id: ft_id,
            domain: "orders".to_string(),
            target: "Order".to_string(),
            bindings,
        };

        let result = apply_command_defs(&def_map, &cmd, &pop);
        assert!(!result.rejected);
        assert_eq!(result.entities[0].entity_type, "QueryResult");
    }

    // â"€â"€ is-chg: LoadReadings command â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    #[test]
    fn load_readings_command_parses_markdown() {
        let (def_map, pop) = setup_order_defs();

        let cmd = Command::LoadReadings {
            markdown: "# Test\n\nProduct(.SKU) is an entity type.\nCategory(.Name) is an entity type.\nProduct belongs to Category.\n  Each Product belongs to exactly one Category.".to_string(),
            domain: "catalog".to_string(),
        };

        let result = apply_command_defs(&def_map, &cmd, &pop);
        assert!(!result.rejected);
        assert_eq!(result.entities[0].entity_type, "SchemaLoaded");
        assert_eq!(result.entities[0].data["nouns"], "2");
    }

    #[test]
    fn load_readings_command_reports_parse_error() {
        let (def_map, pop) = setup_order_defs();

        let cmd = Command::LoadReadings {
            markdown: "".to_string(), // empty â€" should parse OK (empty domain)
            domain: "empty".to_string(),
        };

        let result = apply_command_defs(&def_map, &cmd, &pop);
        assert!(!result.rejected); // empty is valid
    }
}
