// crates/fol-engine/src/arest.rs
//
// AREST — Applicative REpresentational State Transfer
//
// Command : Population → (Population', Representation)
//
// The command is compiled from readings. The engine applies it.
// The result is the new population and a hypermedia representation
// with HATEOAS links showing valid state transitions.

use serde::{Serialize, Deserialize};
use crate::types::*;
use crate::compile::CompiledModel;
use crate::ast;

// ── Commands ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Command {
    CreateEntity {
        noun: String,
        domain: String,
        id: Option<String>,
        fields: std::collections::HashMap<String, String>,
    },
    Transition {
        #[serde(alias = "entityId")]
        entity_id: String,
        event: String,
        domain: String,
        #[serde(alias = "currentStatus", default)]
        current_status: Option<String>,
    },
}

// ── Result ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    pub entities: Vec<EntityResult>,
    pub status: Option<String>,
    pub transitions: Vec<TransitionAction>,
    pub violations: Vec<Violation>,
    pub derived_count: usize,
    pub rejected: bool,
    /// The transformed population — the authoritative state after this command.
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

// ── Apply ────────────────────────────────────────────────────────────

pub fn apply_command(
    model: &CompiledModel,
    command: &Command,
    population: &Population,
) -> CommandResult {
    match command {
        Command::CreateEntity { noun, domain, id, fields } => {
            apply_create_entity(model, noun, domain, id.as_deref(), fields, population)
        }
        Command::Transition { entity_id, event, domain, current_status } => {
            apply_transition(model, entity_id, event, domain, current_status.as_deref(), population)
        }
    }
}

// ── create = emit ∘ derive ∘ validate ∘ resolve ─────────────────────

fn apply_create_entity(
    model: &CompiledModel,
    noun: &str,
    domain: &str,
    explicit_id: Option<&str>,
    fields: &std::collections::HashMap<String, String>,
    population: &Population,
) -> CommandResult {
    // resolve: reference scheme selector → entity identity
    let entity_id = resolve_entity_id(model, noun, explicit_id, fields);

    // Start building the new population
    let mut new_pop = population.clone();

    // The Resource entity
    let mut entity_data = fields.clone();
    entity_data.insert("domain".to_string(), domain.to_string());
    let mut entities = vec![EntityResult {
        id: entity_id.clone(),
        entity_type: noun.to_string(),
        data: entity_data,
    }];

    // State Machine initialization via AST:
    // apply sm.func to empty event stream → initial state
    let mut status = None;
    if let Some(&sm_idx) = model.noun_index.noun_to_state_machines.get(noun) {
        let sm = &model.state_machines[sm_idx];
        let initial = crate::evaluate::run_machine_ast(sm, &[]);
        status = Some(initial.clone());

        let mut sm_data = std::collections::HashMap::new();
        sm_data.insert("instanceOf".to_string(), sm.noun_name.clone());
        sm_data.insert("currentlyInStatus".to_string(), initial);
        sm_data.insert("forResource".to_string(), entity_id.clone());
        sm_data.insert("domain".to_string(), domain.to_string());
        entities.push(EntityResult {
            id: format!("sm:{}", entity_id),
            entity_type: "State Machine".to_string(),
            data: sm_data,
        });
    }

    // derive: forward chain to fixed point on the new population
    let derived = crate::evaluate::forward_chain_ast(model, &mut new_pop);

    // validate: evaluate constraints against the new population
    let response = ResponseContext { text: String::new(), sender_identity: None, fields: None };
    let violations = crate::evaluate::evaluate_via_ast(model, &response, &new_pop);
    let rejected = violations.iter().any(|v| v.detail.contains("forbidden"));

    // emit: HATEOAS links from compiled transition table
    let transitions = hateoas_from_model(model, noun, &entity_id, status.as_deref());

    CommandResult {
        entities,
        status,
        transitions,
        violations,
        derived_count: derived.len(),
        rejected,
        population: new_pop,
    }
}

// ── transition = sm.func : <status, event> → status' ────────────────

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
    // Guards are compiled into the Condition predicates —
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
    if new_status.is_some() {
        let mut event_data = std::collections::HashMap::new();
        event_data.insert("eventType".to_string(), event.to_string());
        event_data.insert("domain".to_string(), domain.to_string());
        entities.push(EntityResult {
            id: format!("evt:{}:{}", entity_id, event),
            entity_type: "Event".to_string(),
            data: event_data,
        });
    }

    let transitions = if let Some(ref status) = new_status {
        hateoas_from_model(model, &sm_noun, entity_id, Some(status))
    } else {
        vec![]
    };

    let rejected = new_status.is_none() && !model.state_machines.is_empty();

    CommandResult {
        entities,
        status: new_status,
        transitions,
        violations: vec![],
        derived_count: 0,
        rejected,
        population: new_pop,
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

/// HATEOAS: project valid transitions from the compiled model.
fn hateoas_from_model(
    model: &CompiledModel,
    noun: &str,
    entity_id: &str,
    status: Option<&str>,
) -> Vec<TransitionAction> {
    let Some(status) = status else { return vec![] };
    let Some(&sm_idx) = model.noun_index.noun_to_state_machines.get(noun) else { return vec![] };
    let sm = &model.state_machines[sm_idx];
    let encoded = noun.replace(' ', "%20");
    sm.transition_table.iter()
        .filter(|(from, _, _)| from == status)
        .map(|(_, to, evt)| TransitionAction {
            event: evt.clone(),
            target_status: to.clone(),
            method: "POST".to_string(),
            href: format!("/api/entities/{}/{}/transition", encoded, entity_id),
        })
        .collect()
}

/// Resolve entity ID from Halpin's reference scheme.
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

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_order_ir() -> ConstraintIR {
        let mut ir = ConstraintIR {
            domain: "orders".to_string(),
            nouns: HashMap::new(),
            fact_types: HashMap::new(),
            constraints: vec![],
            state_machines: HashMap::new(),
            derivation_rules: vec![],
        };
        ir.nouns.insert("Order".to_string(), NounDef {
            object_type: "entity".to_string(),
            enum_values: None, value_type: None, super_type: None,
            world_assumption: WorldAssumption::default(), ref_scheme: Some(vec!["Order Number".to_string()]),
        });
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
    fn command_returns_population() {
        let ir = make_order_ir();
        let model = crate::compile::compile(&ir);
        let pop = Population { facts: HashMap::new() };

        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-1".to_string()),
            fields: HashMap::new(),
        };

        let result = apply_command(&model, &cmd, &pop);
        // CommandResult includes the transformed population
        let _ = &result.population;
    }
}
