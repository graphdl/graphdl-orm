// crates/fol-engine/src/lib.rs
//
// WASM interface. Exports:
//   load_ir            — parse JSON IR, compile into predicates (once)
//   evaluate_response  — apply compiled predicates to response + population (per request)
//   synthesize_noun    — collect all knowledge about a noun from the compiled model
//   forward_chain      — apply derivation rules to population until fixed point
//   query_population   — filter a population by predicate, return matching entities

use std::collections::HashMap;

pub mod ast;
pub mod types;
pub mod compile;
pub mod evaluate;
mod query;
mod induce;
pub mod rmap;
pub mod naming;
pub mod validate;
pub mod conceptual_query;
pub mod parse_rule;
pub mod parse_forml2;
pub mod verbalize;
pub mod arest;

use wasm_bindgen::prelude::*;
use std::sync::Mutex;
use std::sync::OnceLock;

use types::{ConstraintIR, ResponseContext, Population, Violation, DerivedFact, SynthesisResult, WorldAssumption};

/// Convert a JsValue to a Rust type via serde-wasm-bindgen (no JSON string roundtrip).
fn from_js<T: serde::de::DeserializeOwned>(val: &JsValue) -> Result<T, JsValue> {
    serde_wasm_bindgen::from_value(val.clone())
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Convert a Rust type to JsValue via serde-wasm-bindgen (no JSON string roundtrip).
fn to_js<T: serde::Serialize>(val: &T) -> JsValue {
    serde_wasm_bindgen::to_value(val).unwrap_or(JsValue::NULL)
}
use compile::CompiledModel;

/// Lightweight transition record for JsValue serialization (get_transitions_wasm).
#[derive(serde::Serialize)]
struct Transition {
    from: String,
    to: String,
    event: String,
}

/// Lightweight fact-event record for JsValue serialization (resolve_fact_event).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FactEventRecord {
    fact_type_id: String,
    event_name: String,
    target_noun: String,
}

/// Debug state machine info for JsValue serialization.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DebugSmInfo {
    noun_name: String,
    initial: String,
    transitions: usize,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DebugCompiledState {
    loaded: bool,
    state_machines: Vec<DebugSmInfo>,
    noun_to_state_machines: std::collections::HashMap<String, usize>,
}

#[derive(serde::Serialize)]
struct DebugCompiledStateEmpty {
    loaded: bool,
}

/// Result of prepare_entity for JsValue serialization.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PrepareEntityResult {
    initial_state: Option<String>,
    violations: Vec<Violation>,
    derived_facts: Vec<DerivedFact>,
    fact_event: Option<FactEventRecord>,
}

/// A fact projected from an entity row using compiled graph schema references.
/// This is the output of α(project) applied to the 3NF row.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProjectedFact {
    /// The compiled graph schema ID (from the engine, not string concatenation)
    schema_id: String,
    /// The natural language reading (e.g., "Customer has name")
    reading: String,
    /// Role bindings: [(role_name, value), ...] in schema role order
    bindings: Vec<(String, String)>,
}

/// Schema mapping entry: maps a field name to its compiled graph schema.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldSchemaMapping {
    field_name: String,
    schema_id: String,
    reading: String,
    role_names: Vec<String>,
}

struct CompiledState {
    ir: ConstraintIR,
    model: CompiledModel,
}

/// Compiled validation model (from core.md + validation.md).
/// Stored separately from the domain model so it persists across domain loads.
static VALIDATION_MODEL: OnceLock<Mutex<Option<CompiledModel>>> = OnceLock::new();

fn validation_store() -> &'static Mutex<Option<CompiledModel>> {
    VALIDATION_MODEL.get_or_init(|| Mutex::new(None))
}

static STATE: OnceLock<Mutex<Option<CompiledState>>> = OnceLock::new();

fn state_store() -> &'static Mutex<Option<CompiledState>> {
    STATE.get_or_init(|| Mutex::new(None))
}

#[wasm_bindgen]
pub fn load_ir(ir_json: &str) -> Result<(), JsValue> {
    let ir: ConstraintIR = serde_json::from_str(ir_json)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse IR: {}", e)))?;
    let model = compile::compile(&ir);
    let mut store = state_store().lock().unwrap();
    *store = Some(CompiledState { ir, model });
    Ok(())
}

#[wasm_bindgen]
pub fn evaluate_response(response_val: JsValue, population_val: JsValue) -> Result<JsValue, JsValue> {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return Ok(to_js(&Vec::<Violation>::new())),
    };

    let response: ResponseContext = from_js(&response_val)?;
    let population: Population = from_js(&population_val)?;

    // Evaluate via AST reduction
    let violations = evaluate::evaluate_via_ast(&state.model, &response, &population);
    Ok(to_js(&violations))
}

#[wasm_bindgen]
pub fn synthesize_noun(noun_name: &str, depth: usize) -> JsValue {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return to_js(&SynthesisResult::empty(noun_name)),
    };

    let result = evaluate::synthesize(&state.model, &state.ir, noun_name, depth);
    to_js(&result)
}

#[wasm_bindgen]
pub fn forward_chain_population(population_val: JsValue) -> Result<JsValue, JsValue> {
    let store = state_store().lock().unwrap();
    let state = store.as_ref().ok_or_else(|| JsValue::from_str("no IR loaded"))?;

    let mut population: Population = from_js(&population_val)?;

    // Forward chain via AST reduction
    let derived = evaluate::forward_chain_ast(&state.model, &mut population);
    Ok(to_js(&derived))
}

#[wasm_bindgen]
pub fn query_population_wasm(population_val: JsValue, predicate_val: JsValue) -> Result<JsValue, JsValue> {
    let population: Population = from_js(&population_val)?;
    let predicate: query::QueryPredicate = from_js(&predicate_val)?;
    let result = query::query_population(&population, &predicate);
    Ok(to_js(&result))
}

/// Query a population using the AST-based partial application model.
///
/// schema_id: the fact type ID to query
/// target_role: 1-indexed role to extract from matching facts
/// filter_json: array of [role_index, value] pairs to filter by
/// population_json: the population to query
///
/// Returns JSON: { "matches": ["value1", "value2", ...], "count": N }
#[wasm_bindgen]
pub fn query_schema_wasm(
    schema_id: &str,
    target_role: usize,
    filter_val: JsValue,
    population_val: JsValue,
) -> Result<JsValue, JsValue> {
    let store = state_store().lock().unwrap();
    let state = store.as_ref().ok_or_else(|| JsValue::from_str("no IR loaded"))?;

    let schema = state.model.schemas.get(schema_id)
        .ok_or_else(|| JsValue::from_str("schema not found"))?;

    let population: Population = from_js(&population_val)?;
    let filters: Vec<(usize, String)> = from_js(&filter_val)?;
    let filter_refs: Vec<(usize, &str)> = filters.iter().map(|(i, v)| (*i, v.as_str())).collect();

    let matches = query::query_with_ast(&population, schema, target_role, &filter_refs);
    let count = matches.len();

    Ok(to_js(&query::QueryResult { matches, count }))
}

/// Induce constraints and rules from a population.
/// Given observed facts, discover the UC, MC, FC, SS constraints and
/// derivation rules that govern the data. This is the inverse of evaluation.
#[wasm_bindgen]
pub fn induce_from_population(population_val: JsValue) -> Result<JsValue, JsValue> {
    let store = state_store().lock().unwrap();
    let state = store.as_ref().ok_or_else(|| JsValue::from_str("no IR loaded"))?;

    let population: Population = from_js(&population_val)?;
    let result = induce::induce(&state.ir, &population);
    Ok(to_js(&result))
}

/// Run a compiled state machine by folding events through the transition function.
/// Events are [(event_name, payload)] pairs. Returns the final state.
#[wasm_bindgen]
pub fn run_machine_wasm(noun_name: &str, events_val: JsValue, _population_val: JsValue) -> Result<JsValue, JsValue> {
    let store = state_store().lock().unwrap();
    let state = store.as_ref().ok_or_else(|| JsValue::from_str("no IR loaded"))?;

    let events: Vec<(String, String)> = from_js(&events_val)?;
    let event_names: Vec<&str> = events.iter().map(|(e, _)| e.as_str()).collect();

    // Find the state machine for this noun and run via AST reduction
    let machine_idx = state.model.noun_index.noun_to_state_machines.get(noun_name);
    match machine_idx.and_then(|&idx| state.model.state_machines.get(idx)) {
        Some(machine) => {
            let final_state = evaluate::run_machine_ast(machine, &event_names);
            Ok(to_js(&final_state))
        }
        None => Err(JsValue::from_str(&format!("no state machine for noun '{}'", noun_name))),
    }
}

/// Get valid transitions from a given status in a compiled state machine.
/// Returns JSON: [{ "from": "status", "to": "target", "event": "eventName" }]
#[wasm_bindgen]
pub fn get_transitions_wasm(noun_name: &str, current_status: &str) -> JsValue {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return to_js(&Vec::<Transition>::new()),
    };

    let machine_idx = state.model.noun_index.noun_to_state_machines.get(noun_name);
    match machine_idx.and_then(|&idx| state.model.state_machines.get(idx)) {
        Some(machine) => {
            let valid: Vec<Transition> = machine.transition_table.iter()
                .filter(|(from, _, _)| from == current_status)
                .map(|(from, to, event)| Transition {
                    from: from.clone(),
                    to: to.clone(),
                    event: event.clone(),
                })
                .collect();
            to_js(&valid)
        }
        None => to_js(&Vec::<Transition>::new()),
    }
}

/// Given a fact type ID, resolve what event should fire on which state machine.
/// Returns JSON: { "factTypeId": "...", "eventName": "...", "targetNoun": "..." } or null.
#[wasm_bindgen]
pub fn resolve_fact_event(fact_type_id: &str) -> JsValue {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return JsValue::NULL,
    };
    match state.model.fact_events.get(fact_type_id) {
        Some(fe) => {
            let record = FactEventRecord {
                fact_type_id: fe.fact_type_id.clone(),
                event_name: fe.event_name.clone(),
                target_noun: fe.target_noun.clone(),
            };
            to_js(&record)
        }
        None => JsValue::NULL,
    }
}

/// Debug: return the compiled model state (noun-to-SM mapping)
#[wasm_bindgen]
pub fn debug_compiled_state() -> JsValue {
    let store = state_store().lock().unwrap();
    match store.as_ref() {
        Some(s) => {
            let sm_info: Vec<DebugSmInfo> = s.model.state_machines.iter()
                .map(|sm| DebugSmInfo {
                    noun_name: sm.noun_name.clone(),
                    initial: sm.initial.clone(),
                    transitions: sm.transition_table.len(),
                })
                .collect();
            let noun_map: std::collections::HashMap<&str, usize> = s.model.noun_index.noun_to_state_machines.iter()
                .map(|(k, v)| (k.as_str(), *v))
                .collect();
            to_js(&DebugCompiledState {
                loaded: true,
                state_machines: sm_info,
                noun_to_state_machines: noun_map.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            })
        }
        None => to_js(&DebugCompiledStateEmpty { loaded: false }),
    }
}

/// AREST: Apply a command to the current population.
/// One function application. One state transfer.
/// Returns the complete result: entities, status, transitions, violations, derived facts.
#[wasm_bindgen]
pub fn apply_command_wasm(command_val: JsValue, population_val: JsValue) -> Result<JsValue, JsValue> {
    let store = state_store().lock().unwrap();
    let state = store.as_ref().ok_or_else(|| JsValue::from_str("no IR loaded"))?;

    let command: arest::Command = from_js(&command_val)?;
    let population: Population = from_js(&population_val)?;
    let result = arest::apply_command(&state.model, &command, &population);
    Ok(to_js(&result))
}

/// Prepare entity creation: given a noun name, return the initial state
/// and any constraint violations. This is a single function application —
/// the engine evaluates state machine initialization, deontic checks, and
/// derivation rules in one call.
///
/// Returns JSON: { initialState: "Draft" | null, violations: [...], derivedFacts: [...] }
#[wasm_bindgen]
pub fn prepare_entity(noun_name: &str, _fields_val: JsValue, population_val: JsValue) -> Result<JsValue, JsValue> {
    let store = state_store().lock().unwrap();
    let state = store.as_ref().ok_or_else(|| JsValue::from_str("no IR loaded"))?;

    // 1. State machine initialization — find initial state for this noun
    let initial_state = state.model.noun_index.noun_to_state_machines.get(noun_name)
        .and_then(|&idx| state.model.state_machines.get(idx))
        .map(|sm| sm.initial.clone());

    // 2. Deontic constraint evaluation
    let response = types::ResponseContext { text: String::new(), sender_identity: None, fields: None };
    let population: types::Population = from_js(&population_val)?;
    let violations = evaluate::evaluate_via_ast(&state.model, &response, &population);

    // 3. Forward chain derivation rules
    let mut pop_clone = population.clone();
    let derived = evaluate::forward_chain_ast(&state.model, &mut pop_clone);

    // 4. Fact-to-event resolution — does creating this entity trigger a transition?
    let fact_event = state.model.fact_events.values()
        .find(|fe| fe.target_noun == noun_name)
        .map(|fe| FactEventRecord {
            fact_type_id: fe.fact_type_id.clone(),
            event_name: fe.event_name.clone(),
            target_noun: fe.target_noun.clone(),
        });

    Ok(to_js(&PrepareEntityResult {
        initial_state,
        violations,
        derived_facts: derived,
        fact_event,
    }))
}

/// Project an entity's fields into facts using compiled graph schema references.
///
/// This is α(project) applied to the 3NF row: for each field, find the compiled
/// schema where this noun plays role 0 and the field name matches role 1's noun name,
/// then produce a fact with the compiled schema ID and proper bindings.
///
/// Fields that don't match a compiled schema are included with provisional IDs
/// (the reading format: "Noun has field"). System fields (starting with _) are excluded.
#[wasm_bindgen]
pub fn project_entity_wasm(noun_name: &str, entity_id: &str, fields_val: JsValue) -> Result<JsValue, JsValue> {
    let store = state_store().lock().unwrap();
    let fields: std::collections::HashMap<String, String> = from_js(&fields_val)?;

    let state = match store.as_ref() {
        Some(s) => s,
        None => {
            // No schema loaded — produce provisional facts (same as entity-do getFacts)
            let facts: Vec<ProjectedFact> = fields.iter()
                .filter(|(k, v)| !k.starts_with('_') && !v.is_empty())
                .map(|(field, value)| ProjectedFact {
                    schema_id: format!("{} has {}", noun_name, field),
                    reading: format!("{} has {}", noun_name, field),
                    bindings: vec![(noun_name.to_string(), entity_id.to_string()), (field.clone(), value.clone())],
                })
                .collect();
            return Ok(to_js(&facts));
        }
    };

    // Build a field_name → (schema_id, reading, role_names) map for this noun
    let noun_fts = state.model.noun_index.noun_to_fact_types.get(noun_name);
    let mut field_to_schema: std::collections::HashMap<&str, (&str, &str, &[String])> = std::collections::HashMap::new();

    if let Some(fts) = noun_fts {
        for (ft_id, role_idx) in fts {
            // Only schemas where this noun plays role 0 (the entity/subject role)
            if *role_idx != 0 { continue; }
            if let Some(schema) = state.model.schemas.get(ft_id) {
                // Binary fact type: role 0 = entity, role 1 = field
                if schema.role_names.len() >= 2 {
                    let field_name = &schema.role_names[1];
                    field_to_schema.insert(field_name.as_str(), (ft_id.as_str(), schema.reading.as_str(), &schema.role_names));
                }
            }
        }
    }

    let mut facts: Vec<ProjectedFact> = Vec::new();

    for (field, value) in &fields {
        if field.starts_with('_') || value.is_empty() { continue; }

        if let Some(&(schema_id, reading, _role_names)) = field_to_schema.get(field.as_str()) {
            // Compiled schema match — use the engine's schema ID
            facts.push(ProjectedFact {
                schema_id: schema_id.to_string(),
                reading: reading.to_string(),
                bindings: vec![(noun_name.to_string(), entity_id.to_string()), (field.clone(), value.clone())],
            });
        } else {
            // No compiled schema — provisional fact with reading-format ID
            facts.push(ProjectedFact {
                schema_id: format!("{} has {}", noun_name, field),
                reading: format!("{} has {}", noun_name, field),
                bindings: vec![(noun_name.to_string(), entity_id.to_string()), (field.clone(), value.clone())],
            });
        }
    }

    // Sort by schema_id for deterministic output
    facts.sort_by(|a, b| a.schema_id.cmp(&b.schema_id));

    Ok(to_js(&facts))
}

/// Get the field-to-schema mapping for a noun.
/// Returns all compiled graph schemas where this noun plays role 0 (entity role),
/// mapped by the role 1 noun name (the field name).
///
/// This is the schema metadata needed by the TypeScript layer to understand
/// how entity fields map to compiled constructions.
#[wasm_bindgen]
pub fn get_noun_schemas_wasm(noun_name: &str) -> JsValue {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return to_js(&Vec::<FieldSchemaMapping>::new()),
    };

    let noun_fts = state.model.noun_index.noun_to_fact_types.get(noun_name);
    let mut mappings: Vec<FieldSchemaMapping> = Vec::new();

    if let Some(fts) = noun_fts {
        for (ft_id, role_idx) in fts {
            if *role_idx != 0 { continue; }
            if let Some(schema) = state.model.schemas.get(ft_id) {
                if schema.role_names.len() >= 2 {
                    mappings.push(FieldSchemaMapping {
                        field_name: schema.role_names[1].clone(),
                        schema_id: schema.id.clone(),
                        reading: schema.reading.clone(),
                        role_names: schema.role_names.clone(),
                    });
                }
            }
        }
    }

    mappings.sort_by(|a, b| a.field_name.cmp(&b.field_name));
    to_js(&mappings)
}

/// Load the validation model (compiled from core.md + validation.md).
/// Called once at startup. The validation model persists across domain loads.
#[wasm_bindgen]
pub fn load_validation_model(ir_json: &str) -> Result<(), JsValue> {
    let ir: ConstraintIR = serde_json::from_str(ir_json)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse validation IR: {}", e)))?;
    let model = compile::compile(&ir);
    let mut store = validation_store().lock().unwrap();
    *store = Some(model);
    Ok(())
}

/// Validate a domain IR against the validation model.
/// Takes domain IR as JSON string, converts to metamodel population,
/// evaluates validation constraints. Returns JS array of violations.
#[wasm_bindgen]
pub fn validate_schema_wasm(domain_ir_json: &str) -> Result<JsValue, JsValue> {
    let val_store = validation_store().lock().unwrap();
    let validation_model = match val_store.as_ref() {
        Some(m) => m,
        None => return Ok(to_js(&Vec::<Violation>::new())),
    };
    let domain_ir: ConstraintIR = serde_json::from_str(domain_ir_json)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse domain IR: {}", e)))?;
    let violations = validate::validate_schema(validation_model, &domain_ir);
    Ok(to_js(&violations))
}

/// Run RMAP (Relational Mapping Procedure) on the loaded IR.
/// Returns table definitions as JSON.
#[wasm_bindgen]
pub fn rmap_wasm() -> JsValue {
    let store = state_store().lock().unwrap();
    let state = match store.as_ref() {
        Some(s) => s,
        None => return to_js(&Vec::<()>::new()),
    };
    let tables = rmap::rmap(&state.ir);
    to_js(&tables)
}

/// Prove a goal fact via backward chaining.
/// Returns a ProofResult with status (Proven/Disproven/Unknown) and proof tree.
#[wasm_bindgen]
pub fn prove_goal(goal: &str, population_val: JsValue, world_assumption: &str) -> Result<JsValue, JsValue> {
    let store = state_store().lock().unwrap();
    let state = store.as_ref().ok_or_else(|| JsValue::from_str("no IR loaded"))?;

    let population: Population = from_js(&population_val)?;
    let wa = match world_assumption {
        "open" => WorldAssumption::Open,
        _ => WorldAssumption::Closed,
    };

    let result = evaluate::prove(&state.ir, &population, goal, &wa);
    Ok(to_js(&result))
}

/// Parse FORML 2 markdown readings → entities ready for materialization.
/// This is the ONLY path from readings to entities. No TS parser.
///
/// Per the paper: parse: R → Φ (Theorem 2).
#[wasm_bindgen]
pub fn parse_readings_wasm(markdown: &str, domain: &str) -> Result<JsValue, JsValue> {
    let ir = parse_forml2::parse_markdown(markdown)
        .map_err(|e| JsValue::from_str(&e))?;

    let mut entities: Vec<serde_json::Value> = Vec::new();

    // Domains are NORMA tabs — not partitions. Fact types are idempotent.
    // A noun "Customer" declared in multiple domains is ONE cell.
    // Domain is metadata (which tab), not identity.

    // Nouns → Noun entities (id = noun name, globally unique in the UoD)
    for (name, noun) in &ir.nouns {
        let mut data = serde_json::Map::new();
        data.insert("name".into(), serde_json::Value::String(name.clone()));
        data.insert("domain".into(), serde_json::Value::String(domain.into()));
        data.insert("objectType".into(), serde_json::Value::String(noun.object_type.clone()));
        if let Some(ref st) = noun.super_type {
            data.insert("superType".into(), serde_json::Value::String(st.clone()));
        }
        if let Some(ref rs) = noun.ref_scheme {
            data.insert("referenceScheme".into(),
                serde_json::Value::String(serde_json::to_string(rs).unwrap_or_default()));
        }
        if let Some(ref obj) = noun.objectifies {
            data.insert("objectifies".into(), serde_json::Value::String(obj.clone()));
        }
        if let Some(ref backed) = noun.backed_by {
            data.insert("backedBy".into(), serde_json::Value::String(backed.clone()));
        }
        if let Some(ref evs) = noun.enum_values {
            if !evs.is_empty() {
                data.insert("enumValues".into(),
                    serde_json::Value::String(serde_json::to_string(evs).unwrap_or_default()));
            }
        }

        // ID is the noun name — idempotent across domains
        entities.push(serde_json::json!({
            "id": name,
            "type": "Noun",
            "domain": domain,
            "data": serde_json::Value::Object(data),
        }));
    }

    // Fact types → Reading + Graph Schema + Role entities
    // Reading ID = fact type ID (the predicate reading text is the identity)
    for (ft_id, ft) in &ir.fact_types {
        entities.push(serde_json::json!({
            "id": ft_id,
            "type": "Reading",
            "domain": domain,
            "data": {
                "text": ft.reading,
                "domain": domain,
                "graphSchema": ft_id,
            },
        }));

        entities.push(serde_json::json!({
            "id": ft_id,
            "type": "Graph Schema",
            "domain": domain,
            "data": {
                "name": ft_id,
                "domain": domain,
                "reading": ft.reading,
                "arity": ft.roles.len(),
            },
        }));

        for (i, role) in ft.roles.iter().enumerate() {
            entities.push(serde_json::json!({
                "id": format!("{}:role:{}", ft_id, i),
                "type": "Role",
                "domain": domain,
                "data": {
                    "nounName": role.noun_name,
                    "position": i,
                    "graphSchema": ft_id,
                    "domain": domain,
                },
            }));
        }
    }

    // Constraints → Constraint entities (id = constraint text, idempotent)
    for (i, c) in ir.constraints.iter().enumerate() {
        let reading_ref = c.spans.first().map(|s| s.fact_type_id.clone()).unwrap_or_default();
        // Use constraint text as ID when available, fall back to index
        let constraint_id = if !c.text.is_empty() {
            c.text.clone()
        } else {
            format!("constraint:{}", i)
        };
        entities.push(serde_json::json!({
            "id": constraint_id,
            "type": "Constraint",
            "domain": domain,
            "data": {
                "text": c.text,
                "kind": c.kind,
                "modality": c.modality,
                "reading": reading_ref,
                "domain": domain,
            },
        }));
    }

    // State machines → SM Definition, Status, Transition entities
    for (sm_name, sm) in &ir.state_machines {
        let sm_id = format!("sm:{}", sm_name);
        entities.push(serde_json::json!({
            "id": &sm_id,
            "type": "State Machine Definition",
            "domain": domain,
            "data": { "name": sm_name, "forNoun": &sm.noun_name, "domain": domain },
        }));

        for status_name in &sm.statuses {
            entities.push(serde_json::json!({
                "id": format!("{}:{}", sm_id, status_name),
                "type": "Status",
                "domain": domain,
                "data": {
                    "name": status_name,
                    "stateMachineDefinition": &sm_id,
                    "domain": domain,
                },
            }));
        }

        for transition in &sm.transitions {
            entities.push(serde_json::json!({
                "id": format!("{}:{}:{}", sm_id, transition.from, transition.to),
                "type": "Transition",
                "domain": domain,
                "data": {
                    "from": transition.from,
                    "to": transition.to,
                    "event": transition.event,
                    "stateMachineDefinition": &sm_id,
                    "domain": domain,
                },
            }));
        }
    }

    // Derivation rules
    for (i, rule) in ir.derivation_rules.iter().enumerate() {
        let rule_id = if !rule.text.is_empty() {
            rule.text.clone()
        } else {
            format!("derivation:{}", i)
        };
        entities.push(serde_json::json!({
            "id": rule_id,
            "type": "Derivation Rule",
            "domain": domain,
            "data": { "text": rule.text, "domain": domain },
        }));
    }

    // Instance facts — x̄ asserted into P.
    // /merge : α key_by : instance_facts  (fold groups, then map to entities)
    let instance_entities = ir.general_instance_facts.iter()
        .map(|fact| (format!("{}:{}", fact.subject_noun, fact.subject_value), fact))
        .fold(HashMap::<String, serde_json::Map<String, serde_json::Value>>::new(), |mut acc, (id, fact)| {
            let data = acc.entry(id).or_insert_with(|| {
                let mut m = serde_json::Map::new();
                m.insert("domain".into(), serde_json::Value::String(domain.into()));
                m
            });
            data.insert(fact.field_name.clone(), serde_json::Value::String(fact.object_value.clone()));
            acc
        });

    entities.extend(instance_entities.iter().map(|(entity_id, data)| {
        let noun_name = entity_id.split(':').next().unwrap_or("");
        serde_json::json!({
            "id": entity_id,
            "type": noun_name,
            "domain": domain,
            "data": serde_json::Value::Object(data.clone()),
        })
    }));

    // Store the compiled IR as a cell — compile(parse(readings)) is a derived fact.
    // loadDomainSchema reads this one cell instead of reconstructing from parts.
    let ir_json = serde_json::to_string(&ir).map_err(|e| JsValue::from_str(&format!("{}", e)))?;
    entities.push(serde_json::json!({
        "id": format!("ir:{}", domain),
        "type": "CompiledSchema",
        "domain": domain,
        "data": { "domain": domain, "ir": ir_json },
    }));

    let json_str = serde_json::to_string(&entities).map_err(|e| JsValue::from_str(&format!("{}", e)))?;
    Ok(JsValue::from_str(&json_str))
}
