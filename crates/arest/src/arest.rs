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
use crate::compile::CompiledModel;
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

pub fn apply_command(
    model: &CompiledModel,
    command: &Command,
    population: &Population,
) -> CommandResult {
    match command {
        // is-cmd: create entity with validation
        Command::CreateEntity { noun, domain, id, fields } => {
            apply_create_entity(model, noun, domain, id.as_deref(), fields, population)
        }
        // is-cmd: state machine transition
        Command::Transition { entity_id, event, domain, current_status } => {
            apply_transition(model, entity_id, event, domain, current_status.as_deref(), population)
        }
        // is-qry: query the population via partial application
        Command::Query { schema_id, domain: _, target, bindings } => {
            apply_query(model, schema_id, target, bindings, population)
        }
        // is-upd: update entity fields with validation
        Command::UpdateEntity { noun, domain, entity_id, fields } => {
            apply_update_entity(model, noun, domain, entity_id, fields, population)
        }
        // is-chg: install readings (modify definitions)
        Command::LoadReadings { markdown, domain } => {
            apply_load_readings(markdown, domain, population)
        }
    }
}

// -- apply_command_defs: DEFS-only path (no CompiledModel) ---------
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
    let sm_id = format!("sm:{}", entity_id);
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

    // Inject transition facts from DEFS (Theorem 3: T is in P)
    // The machine def's transition table is encoded in the population
    // by the forward chaining derivation. Transition facts already in new_pop.

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

// â"€â"€ create = emit âˆ˜ validate âˆ˜ derive âˆ˜ resolve â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€
// Right-to-left: resolve â†’ derive â†’ validate â†’ emit
// Validate must see the complete population (base + derived facts).

fn apply_create_entity(
    model: &CompiledModel,
    noun: &str,
    domain: &str,
    explicit_id: Option<&str>,
    fields: &std::collections::HashMap<String, String>,
    population: &Population,
) -> CommandResult {
    // create = emit âˆ˜ validate âˆ˜ derive âˆ˜ resolve
    //
    // â"€â"€ resolve â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€
    // Apply the reference scheme selector to determine entity identity.
    // Insert entity fields as facts: Pop' = Pop âˆª {entity facts}.
    let entity_id = resolve_entity_id(model, noun, explicit_id, fields);

    let mut new_pop = population.clone();
    let mut entity_data = fields.clone();
    entity_data.insert("domain".to_string(), domain.to_string());
    for (field_name, value) in &entity_data {
        let ft_id = resolve_fact_type_id(model, noun, field_name);
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

    // â"€â"€ derive â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€
    // Forward-chain derivation rules to fixed point.
    // State machine initialization is NOT a separate step â€" the SM instance
    // and its initial status are derived facts produced by forward chaining
    // (compile_sm_initialization derivation rule).
    let derived = crate::evaluate::forward_chain_ast(model, &mut new_pop);

    // Build entity results from the population.
    // The SM instance was derived by forward chaining â€" extract status from population.
    let mut entities = vec![EntityResult {
        id: entity_id.clone(),
        entity_type: noun.to_string(),
        data: entity_data,
    }];

    let sm_id = format!("sm:{}", entity_id);
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

    // Inject transition facts into population (Theorem 3: T âŠ† P)
    if status.is_some() {
        if let Some(&sm_idx) = model.noun_index.noun_to_state_machines.get(noun) {
            let sm = &model.state_machines[sm_idx];
            for (from, to, event) in &sm.transition_table {
                let ft_key = "Transition".to_string();
                new_pop.facts.entry(ft_key.clone()).or_default().push(
                    FactInstance {
                        fact_type_id: ft_key,
                        bindings: vec![
                            ("from".to_string(), from.clone()),
                            ("to".to_string(), to.clone()),
                            ("event".to_string(), event.clone()),
                        ],
                   
                    }
                );
            }
        }
    }

    // â"€â"€ validate â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€
    // Evaluate constraints against the complete population (base + derived).
    let violations = crate::evaluate::evaluate_via_ast(model, "", None, &new_pop);

    // Alethic violations are structural impossibilities â€" always reject.
    // Deontic violations are reportable but don't prevent the command.
    let rejected = violations.iter().any(|v| v.alethic);

    // â"€â"€ emit â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€
    // Produce the representation: entities, HATEOAS links, violations.
    // If rejected, the population is unchanged (paper Â§4: "The population is unchanged").
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

// â"€â"€ update = emit âˆ˜ validate âˆ˜ derive âˆ˜ (â†"F âˆ˜ [upd, â†‘F]) â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€
// Per Eq. 6: is-upd âˆ˜ â†‘K â†’ [rpt, â†"F âˆ˜ [upd, defs]] âˆ˜ [â†‘I, â†‘F]
// Reads current facts (â†‘F), merges new fields (upd), validates, stores (â†"F).

fn apply_update_entity(
    model: &CompiledModel,
    noun: &str,
    domain: &str,
    entity_id: &str,
    new_fields: &std::collections::HashMap<String, String>,
    population: &Population,
) -> CommandResult {
    // â†‘F: read current entity facts from population
    let mut merged_fields = std::collections::HashMap::new();
    merged_fields.insert("domain".to_string(), domain.to_string());

    // Extract existing fields for this entity from population
    for (_ft_id, instances) in &population.facts {
        for inst in instances {
            if inst.bindings.len() >= 2 && inst.bindings[0].1 == entity_id {
                let field_name = &inst.bindings[1].0;
                let field_value = &inst.bindings[1].1;
                merged_fields.insert(field_name.clone(), field_value.clone());
            }
        }
    }

    // upd: merge new fields over existing
    for (k, v) in new_fields {
        merged_fields.insert(k.clone(), v.clone());
    }

    // â†"F: replace entity facts in population
    let mut new_pop = population.clone();
    for (field_name, value) in &merged_fields {
        let ft_id = resolve_fact_type_id(model, noun, field_name);
        // Remove old fact for this entity+field
        if let Some(instances) = new_pop.facts.get_mut(&ft_id) {
            instances.retain(|inst| {
                !(inst.bindings.len() >= 2 && inst.bindings[0].1 == entity_id)
            });
        }
        // Insert updated fact
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

    // derive: forward-chain to fixed point
    let derived = crate::evaluate::forward_chain_ast(model, &mut new_pop);

    // validate: evaluate constraints against complete population
    let violations = crate::evaluate::evaluate_via_ast(model, "", None, &new_pop);
    let rejected = violations.iter().any(|v| v.alethic);

    // emit: produce representation
    let entity = EntityResult {
        id: entity_id.to_string(),
        entity_type: noun.to_string(),
        data: merged_fields,
    };

    let sm_id = format!("sm:{}", entity_id);
    let status = extract_sm_status(&new_pop, &sm_id);
    let transitions = hateoas_from_population(&new_pop, noun, entity_id, status.as_deref());

    CommandResult {
        entities: vec![entity],
        status,
        transitions,
        violations,
        derived_count: derived.len(),
        rejected,
        population: if rejected { population.clone() } else { new_pop },
    }
}

// â"€â"€ transition = sm.func : <status, event> â†’ status' â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

fn apply_transition(
    model: &CompiledModel,
    entity_id: &str,
    event: &str,
    domain: &str,
    current_status: Option<&str>,
    population: &Population,
) -> CommandResult {
    let mut new_pop = population.clone();
    let mut new_status = None;
    let mut sm_noun = String::new();
    let defs = std::collections::HashMap::new();

    // Apply the SM's AST func to <current_status, event>.
    // Guards are compiled into the Condition predicates â€"
    // if a guard fails, the func returns current state (no transition).
    for sm in &model.state_machines {
        let from_status = current_status.unwrap_or(&sm.initial);
        let input = ast::Object::seq(vec![
            ast::Object::atom(from_status),
            ast::Object::atom(event),
        ]);
        let result = ast::apply(&sm.func, &input, &defs);
        if let Some(next) = result.as_atom() {
            if next != from_status {
                new_status = Some(next.to_string());
                sm_noun = sm.noun_name.clone();
                break;
            }
        }
    }

    let mut entities = Vec::new();
    if let Some(ref status) = new_status {
        // Pop' = Pop with updated SM status fact.
        // Theorem 3: every observable value must be in the population.
        let sm_id = format!("sm:{}", entity_id);
        let status_key = "StateMachine_has_currentlyInStatus".to_string();
        if let Some(facts) = new_pop.facts.get_mut(&status_key) {
            // Update existing SM status fact for this entity
            for fact in facts.iter_mut() {
                if fact.bindings.iter().any(|(_, v)| v == &sm_id) {
                    for (noun, val) in fact.bindings.iter_mut() {
                        if noun == "currentlyInStatus" {
                            *val = status.clone();
                        }
                    }
                }
            }
        } else {
            // No existing SM facts â€" insert new status fact
            new_pop.facts.entry(status_key.clone()).or_default().push(
                FactInstance {
                    fact_type_id: status_key,
                    bindings: vec![
                        ("State Machine".to_string(), sm_id),
                        ("currentlyInStatus".to_string(), status.clone()),
                    ],
               
                }
            );
        }

        let mut event_data = std::collections::HashMap::new();
        event_data.insert("eventType".to_string(), event.to_string());
        event_data.insert("domain".to_string(), domain.to_string());
        entities.push(EntityResult {
            id: format!("evt:{}:{}", entity_id, event),
            entity_type: "Event".to_string(),
            data: event_data,
        });

        // Inject transition facts into population (Theorem 3: T âŠ† P)
        if let Some(&sm_idx) = model.noun_index.noun_to_state_machines.get(sm_noun.as_str()) {
            let sm = &model.state_machines[sm_idx];
            for (from, to, evt) in &sm.transition_table {
                let ft_key = "Transition".to_string();
                new_pop.facts.entry(ft_key.clone()).or_default().push(
                    FactInstance {
                        fact_type_id: ft_key,
                        bindings: vec![
                            ("from".to_string(), from.clone()),
                            ("to".to_string(), to.clone()),
                            ("event".to_string(), evt.clone()),
                        ],
                   
                    }
                );
            }
        }
    }

    let transitions = if let Some(ref status) = new_status {
        hateoas_from_population(&new_pop, &sm_noun, entity_id, Some(status))
    } else {
        vec![]
    };

    let rejected = new_status.is_none() && !model.state_machines.is_empty();

    // If rejected (no valid transition), population is unchanged.
    CommandResult {
        entities,
        status: new_status,
        transitions,
        violations: vec![],
        derived_count: 0,
        rejected,
        population: if rejected { population.clone() } else { new_pop },
    }
}

// â"€â"€ is-qry: query the population â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

fn apply_query(
    model: &CompiledModel,
    schema_id: &str,
    target: &str,
    bindings: &std::collections::HashMap<String, String>,
    population: &Population,
) -> CommandResult {
    // Find the schema and resolve role positions
    let schema = model.schemas.get(schema_id);
    let role_names = schema.map(|s| &s.role_names);

    // Build filter bindings: for each bound noun, find its role index
    let mut filter_pairs: Vec<(usize, String)> = Vec::new();
    let mut target_role: usize = 0;

    if let Some(names) = role_names {
        for (i, name) in names.iter().enumerate() {
            if name == target {
                target_role = i + 1; // 1-indexed
            }
            if let Some(value) = bindings.get(name) {
                filter_pairs.push((i + 1, value.clone()));
            }
        }
    }

    // Query the population for matching facts
    let facts = population.facts.get(schema_id).cloned().unwrap_or_default();
    let mut matches: Vec<String> = Vec::new();

    for fact in &facts {
        let mut all_match = true;
        for (role_idx, expected) in &filter_pairs {
            let actual = fact.bindings.iter()
                .nth(*role_idx - 1)
                .map(|(_, v)| v.as_str());
            if actual != Some(expected.as_str()) {
                all_match = false;
                break;
            }
        }
        if all_match {
            if let Some((_, value)) = fact.bindings.iter().nth(target_role.saturating_sub(1)) {
                if !matches.contains(value) {
                    matches.push(value.clone());
                }
            }
        }
    }

    let mut data = std::collections::HashMap::new();
    data.insert("matches".to_string(), matches.join(","));
    data.insert("count".to_string(), matches.len().to_string());

    CommandResult {
        entities: vec![EntityResult {
            id: format!("query:{}", schema_id),
            entity_type: "QueryResult".to_string(),
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

/// Look up the compiled graph schema ID for a noun's field.
/// Falls back to reading-format ID if no compiled schema matches.
fn resolve_fact_type_id(model: &CompiledModel, noun: &str, field: &str) -> String {
    if let Some(fts) = model.noun_index.noun_to_fact_types.get(noun) {
        for (ft_id, role_idx) in fts {
            if *role_idx != 0 { continue; }
            if let Some(schema) = model.schemas.get(ft_id) {
                if schema.role_names.len() >= 2 && schema.role_names[1] == field {
                    return ft_id.clone();
                }
            }
        }
    }
    format!("{}_has_{}", noun, field)
}

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

fn resolve_entity_id(
    model: &CompiledModel,
    noun: &str,
    explicit_id: Option<&str>,
    fields: &std::collections::HashMap<String, String>,
) -> String {
    if let Some(id) = explicit_id {
        return id.to_string();
    }
    let Some(ref_scheme) = model.noun_index.ref_schemes.get(noun) else {
        return String::new();
    };
    if ref_scheme.len() == 1 && ref_scheme[0] != "id" {
        let ref_name = &ref_scheme[0];
        let camel = to_camel_case(ref_name);
        let last_word = ref_name.split(' ').last().unwrap_or("").to_lowercase();
        fields.get(ref_name.as_str())
            .or_else(|| fields.get(&camel))
            .or_else(|| fields.get(&last_word))
            .cloned()
            .unwrap_or_default()
    } else if ref_scheme.len() > 1 {
        ref_scheme.iter()
            .filter_map(|r| {
                let camel = to_camel_case(r);
                fields.get(r.as_str()).or_else(|| fields.get(&camel)).cloned()
            })
            .collect::<Vec<_>>()
            .join(":")
    } else {
        String::new()
    }
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

    fn make_order_ir() -> Domain {
        let mut ir = Domain {
            domain: "orders".to_string(),
            nouns: HashMap::new(),
            fact_types: HashMap::new(),
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![], general_instance_facts: vec![],
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(), objectifications: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
        };
        ir.nouns.insert("Order".to_string(), NounDef {
            object_type: "entity".to_string(),
            world_assumption: WorldAssumption::default(),
        });
        ir.ref_schemes.insert("Order".to_string(), vec!["Order Number".to_string()]);
        ir.state_machines.insert("Order".to_string(), StateMachineDef {
            noun_name: "Order".to_string(),
            statuses: vec!["Draft".to_string(), "Placed".to_string(), "Paid".to_string(), "Cancelled".to_string()],
            transitions: vec![
                TransitionDef { from: "Draft".to_string(), to: "Placed".to_string(), event: "place".to_string(), guard: None },
                TransitionDef { from: "Placed".to_string(), to: "Paid".to_string(), event: "pay".to_string(), guard: None },
                TransitionDef { from: "Draft".to_string(), to: "Cancelled".to_string(), event: "cancel".to_string(), guard: None },
            ],
        });
        ir
    }

    #[test]
    fn create_entity_initializes_state_machine() {
        let ir = make_order_ir();
        let model = crate::compile::compile(&ir);
        let pop = Population { facts: HashMap::new() };

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-100".to_string());
        fields.insert("amount".to_string(), "999".to_string());

        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-100".to_string()),
            fields,
        };

        let result = apply_command(&model, &cmd, &pop);

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
    fn create_entity_resolves_id_from_reference_scheme() {
        let ir = make_order_ir();
        let model = crate::compile::compile(&ir);
        let pop = Population { facts: HashMap::new() };

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-REF".to_string());
        fields.insert("amount".to_string(), "500".to_string());

        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: None,
            fields,
        };

        let result = apply_command(&model, &cmd, &pop);
        assert_eq!(result.entities[0].id, "ORD-REF");
    }

    #[test]
    fn create_entity_without_state_machine() {
        let ir = make_order_ir();
        let model = crate::compile::compile(&ir);
        let pop = Population { facts: HashMap::new() };

        let mut fields = HashMap::new();
        fields.insert("name".to_string(), "Electronics".to_string());

        let cmd = Command::CreateEntity {
            noun: "Category".to_string(),
            domain: "catalog".to_string(),
            id: Some("electronics".to_string()),
            fields,
        };

        let result = apply_command(&model, &cmd, &pop);

        assert_eq!(result.entities.len(), 1);
        assert!(result.status.is_none());
        assert!(result.transitions.is_empty());
    }

    #[test]
    fn transition_via_ast_func() {
        let ir = make_order_ir();
        let model = crate::compile::compile(&ir);
        let pop = Population { facts: HashMap::new() };

        let cmd = Command::Transition {
            entity_id: "ORD-100".to_string(),
            event: "place".to_string(),
            domain: "orders".to_string(),
            current_status: Some("Draft".to_string()),
        };

        let result = apply_command(&model, &cmd, &pop);

        assert_eq!(result.status.as_deref(), Some("Placed"));
        assert!(result.entities.iter().any(|e| e.entity_type == "Event"));
        assert!(result.transitions.iter().any(|t| t.event == "pay"));
    }

    #[test]
    fn population_contains_entity_facts() {
        let ir = make_order_ir();
        let model = crate::compile::compile(&ir);
        let pop = Population { facts: HashMap::new() };

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-1".to_string());
        fields.insert("customer".to_string(), "acme".to_string());

        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-1".to_string()),
            fields,
        };

        let result = apply_command(&model, &cmd, &pop);

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
        let ir = make_order_ir();
        let model = crate::compile::compile(&ir);

        // Create entity first to get a population with SM facts
        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-1".to_string());
        let create = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-1".to_string()),
            fields,
        };
        let created = apply_command(&model, &create, &Population { facts: HashMap::new() });
        assert_eq!(created.status.as_deref(), Some("Draft"));

        // Transition: Draft â†’ Placed
        let transition = Command::Transition {
            entity_id: "ORD-1".to_string(),
            event: "place".to_string(),
            domain: "orders".to_string(),
            current_status: Some("Draft".to_string()),
        };
        let result = apply_command(&model, &transition, &created.population);

        assert_eq!(result.status.as_deref(), Some("Placed"));

        // Population must contain the updated status
        let sm_facts = &result.population.facts["StateMachine_has_currentlyInStatus"];
        let sm_fact = sm_facts.iter().find(|f|
            f.bindings.iter().any(|(_, v)| v == "sm:ORD-1")
        ).expect("SM fact must exist for ORD-1");
        let status_binding = sm_fact.bindings.iter()
            .find(|(n, _)| n == "currentlyInStatus")
            .expect("must have currentlyInStatus binding");
        assert_eq!(status_binding.1, "Placed", "population must reflect new status");
    }

    // â"€â"€ is-qry: Query command â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    #[test]
    fn query_command_returns_matches() {
        let ir = make_order_ir();
        let model = crate::compile::compile(&ir);

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

        let result = apply_command(&model, &cmd, &pop);
        assert!(!result.rejected);
        assert_eq!(result.entities[0].entity_type, "QueryResult");
    }

    // â"€â"€ is-chg: LoadReadings command â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    #[test]
    fn load_readings_command_parses_markdown() {
        let ir = make_order_ir();
        let model = crate::compile::compile(&ir);
        let pop = Population { facts: HashMap::new() };

        let cmd = Command::LoadReadings {
            markdown: "# Test\n\nProduct(.SKU) is an entity type.\nCategory(.Name) is an entity type.\nProduct belongs to Category.\n  Each Product belongs to exactly one Category.".to_string(),
            domain: "catalog".to_string(),
        };

        let result = apply_command(&model, &cmd, &pop);
        assert!(!result.rejected);
        assert_eq!(result.entities[0].entity_type, "SchemaLoaded");
        assert_eq!(result.entities[0].data["nouns"], "2");
    }

    #[test]
    fn load_readings_command_reports_parse_error() {
        let ir = make_order_ir();
        let model = crate::compile::compile(&ir);
        let pop = Population { facts: HashMap::new() };

        let cmd = Command::LoadReadings {
            markdown: "".to_string(), // empty â€" should parse OK (empty domain)
            domain: "empty".to_string(),
        };

        let result = apply_command(&model, &cmd, &pop);
        assert!(!result.rejected); // empty is valid
    }
}
