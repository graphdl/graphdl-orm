// crates/fol-engine/src/arest.rs
//
// AREST — Applicative REpresentational State Transfer
//
// The composition of Backus's AST (1977) with Fielding's REST (2000).
//
// AST layer: a Command applied to a Population produces a new Population.
// REST layer: the new Population is rendered as a Representation with
//             HATEOAS links showing valid state transitions.
//
// One function application. One state transfer. The command is compiled
// from readings. The engine applies it. The result includes everything:
// the entity, its state machine, derived facts, violations, and the
// hypermedia representation.

use serde::{Serialize, Deserialize};
use crate::types::*;
use crate::compile::CompiledModel;
use crate::ast;

// ── Commands ─────────────────────────────────────────────────────────
// A Command is a function: Population → Population.
// Commands are the AST layer — they transform state.

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Command {
    /// Create an entity. If the noun has a state machine, also creates the
    /// State Machine instance in its initial status.
    CreateEntity {
        noun: String,
        domain: String,
        id: Option<String>,
        fields: std::collections::HashMap<String, String>,
    },

    /// Fire a transition on an entity's state machine.
    /// Creates an Event entity and the ternary fact
    /// "Event triggered Transition in State Machine".
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
// The result of applying a Command. Contains both the AST output
// (new facts, state changes) and the REST output (representation + links).

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    /// Entities created by this command (Resource, State Machine, Event, etc.)
    pub entities: Vec<EntityResult>,

    /// The current status (if entity has a state machine)
    pub status: Option<String>,

    /// Valid transitions from the current status (HATEOAS actions)
    pub transitions: Vec<TransitionAction>,

    /// Constraint violations (deontic warnings or rejections)
    pub violations: Vec<Violation>,

    /// Facts derived by forward chaining
    pub derived_count: usize,

    /// Whether the command was rejected (deontic forbidden)
    pub rejected: bool,
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
// The single operation: apply a Command to produce a CommandResult.

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

fn apply_create_entity(
    model: &CompiledModel,
    noun: &str,
    domain: &str,
    explicit_id: Option<&str>,
    fields: &std::collections::HashMap<String, String>,
    population: &Population,
) -> CommandResult {
    // Resolve entity ID from reference scheme (Halpin's reference mode).
    // The NounDef's ref_scheme names the value type(s) that identify the entity.
    // "Order(.Order Number)" → ref_scheme: ["Order Number"] → field "orderNumber"
    let entity_id = explicit_id.map(|s| s.to_string()).unwrap_or_else(|| {
        if let Some(ref_scheme) = model.noun_index.ref_schemes.get(noun) {
            if ref_scheme.len() == 1 && ref_scheme[0] != "id" {
                // Single value reference scheme — derive field name and look up
                let ref_name = &ref_scheme[0];
                // Try: exact, camelCase, last word lowercase
                let camel = ref_name.split(' ')
                    .enumerate()
                    .map(|(i, w)| if i == 0 { w.to_lowercase() } else { let mut c = w.chars(); match c.next() { Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(), None => String::new() } })
                    .collect::<String>();
                let last_word = ref_name.split(' ').last().unwrap_or("").to_lowercase();

                fields.get(ref_name.as_str())
                    .or_else(|| fields.get(&camel))
                    .or_else(|| fields.get(&last_word))
                    .cloned()
                    .unwrap_or_default()
            } else if ref_scheme.len() > 1 {
                // Composite reference scheme — concatenate values
                ref_scheme.iter()
                    .filter_map(|r| {
                        let camel = r.split(' ').enumerate()
                            .map(|(i, w)| if i == 0 { w.to_lowercase() } else { let mut c = w.chars(); match c.next() { Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(), None => String::new() } })
                            .collect::<String>();
                        fields.get(r.as_str()).or_else(|| fields.get(&camel)).cloned()
                    })
                    .collect::<Vec<_>>()
                    .join(":")
            } else {
                String::new()
            }
        } else {
            String::new()
        }
    });
    let mut entities = Vec::new();
    let mut status: Option<String> = None;
    let mut transitions = Vec::new();

    // 1. The Resource entity
    let mut entity_data = fields.clone();
    entity_data.insert("domain".to_string(), domain.to_string());

    entities.push(EntityResult {
        id: entity_id.clone(),
        entity_type: noun.to_string(),
        data: entity_data,
    });

    // 2. State Machine instance (if noun has one)
    if let Some(&sm_idx) = model.noun_index.noun_to_state_machines.get(noun) {
        let sm = &model.state_machines[sm_idx];
        status = Some(sm.initial.clone());

        // Create State Machine entity
        let mut sm_data = std::collections::HashMap::new();
        sm_data.insert("instanceOf".to_string(), sm.noun_name.clone());
        sm_data.insert("currentlyInStatus".to_string(), sm.initial.clone());
        sm_data.insert("forResource".to_string(), entity_id.clone());
        sm_data.insert("domain".to_string(), domain.to_string());

        entities.push(EntityResult {
            id: format!("sm:{}", entity_id),
            entity_type: "State Machine".to_string(),
            data: sm_data,
        });

        // Valid transitions from initial status (HATEOAS)
        let encoded = urlencoding_noun(noun);
        for (from, to, event) in &sm.transition_table {
            if from == &sm.initial {
                transitions.push(TransitionAction {
                    event: event.clone(),
                    target_status: to.clone(),
                    method: "POST".to_string(),
                    href: format!("/api/entities/{}/{}/transition", encoded, entity_id),
                });
            }
        }
    }

    // 3. Constraint evaluation
    let response = ResponseContext { text: String::new(), sender_identity: None, fields: None };
    let violations = crate::evaluate::evaluate_via_ast(model, &response, population);
    let rejected = violations.iter().any(|v| v.detail.contains("forbidden"));

    // 4. Forward chain
    let mut pop_clone = population.clone();
    let derived = crate::evaluate::forward_chain_ast(model, &mut pop_clone);

    CommandResult {
        entities,
        status,
        transitions,
        violations,
        derived_count: derived.len(),
        rejected,
    }
}

fn apply_transition(
    model: &CompiledModel,
    entity_id: &str,
    event: &str,
    domain: &str,
    current_status: Option<&str>,
    population: &Population,
) -> CommandResult {
    let mut new_status: Option<String> = None;
    let mut transitions = Vec::new();
    let mut sm_noun = String::new();

    for sm in &model.state_machines {
        // Use the current status if provided, otherwise start from initial
        let from_status = current_status.unwrap_or(&sm.initial);

        // Check if this SM has a transition from the current status with this event
        let matching = sm.transition_table.iter()
            .find(|(from, _, evt)| from.as_str() == from_status && evt.as_str() == event);

        if let Some((_, to, _)) = matching {
            new_status = Some(to.clone());
            sm_noun = sm.noun_name.clone();
        } else {
            continue;
        }

        if let Some(ref result) = new_status {
            // Valid transitions from the NEW status
            let encoded = urlencoding_noun(&sm_noun);
            for (from, to, evt) in &sm.transition_table {
                if from == result {
                    transitions.push(TransitionAction {
                        event: evt.clone(),
                        target_status: to.clone(),
                        method: "POST".to_string(),
                        href: format!("/api/entities/{}/{}/transition", encoded, entity_id),
                    });
                }
            }
            break;
        }
    }

    // Create Event entity
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

    // Evaluate guards
    let response = ResponseContext { text: String::new(), sender_identity: None, fields: None };
    let violations = crate::evaluate::evaluate_via_ast(model, &response, population);
    let rejected = !violations.is_empty();

    CommandResult {
        entities,
        status: if rejected { None } else { new_status },
        transitions: if rejected { vec![] } else { transitions },
        violations,
        derived_count: 0,
        rejected,
    }
}

fn urlencoding_noun(noun: &str) -> String {
    noun.replace(' ', "%20")
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

        // Resource entity created — ID from reference scheme "Order Number" → field "orderNumber"
        assert_eq!(result.entities[0].id, "ORD-100");
        assert_eq!(result.entities[0].entity_type, "Order");

        // State Machine instance created
        assert_eq!(result.entities[1].entity_type, "State Machine");
        assert_eq!(result.entities[1].data["currentlyInStatus"], "Draft");
        assert_eq!(result.entities[1].data["forResource"], "ORD-100");

        // Initial status
        assert_eq!(result.status.as_deref(), Some("Draft"));

        // HATEOAS: valid transitions from Draft
        assert_eq!(result.transitions.len(), 2); // place, cancel
        assert!(result.transitions.iter().any(|t| t.event == "place"));
        assert!(result.transitions.iter().any(|t| t.event == "cancel"));

        // Not rejected
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

        // No explicit ID — should resolve from ref_scheme ["Order Number"] → field "orderNumber"
        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: None,
            fields,
        };

        let result = apply_command(&model, &cmd, &pop);
        assert_eq!(result.entities[0].id, "ORD-REF", "ID should come from reference scheme");
    }

    #[test]
    fn create_entity_without_state_machine() {
        let ir = make_order_ir();
        let model = crate::compile::compile(&ir);
        let pop = Population { facts: HashMap::new() };

        let mut fields = HashMap::new();
        fields.insert("name".to_string(), "Electronics".to_string());

        let cmd = Command::CreateEntity {
            noun: "Category".to_string(), // no SM for Category
            domain: "catalog".to_string(),
            id: Some("electronics".to_string()),
            fields,
        };

        let result = apply_command(&model, &cmd, &pop);

        assert_eq!(result.entities.len(), 1); // just the entity, no SM
        assert!(result.status.is_none());
        assert!(result.transitions.is_empty());
    }

    #[test]
    fn transition_changes_state_and_returns_next_actions() {
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

        // Status changed to Placed
        assert_eq!(result.status.as_deref(), Some("Placed"));

        // Event entity created
        assert!(result.entities.iter().any(|e| e.entity_type == "Event"));

        // HATEOAS: valid transitions from Placed (pay)
        assert!(result.transitions.iter().any(|t| t.event == "pay"));
    }
}
