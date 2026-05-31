// crates/arest/tests/integration.rs
//
// Integration tests exercise the compile + evaluate pipeline directly,
// bypassing the wasm_bindgen layer (which requires JsValue).
//
// These tests use the DEFS path (compile_to_defs_state) with Object state.
//
// Test-local compat layer: these tests were authored against an
// older IR-shaped API (`types::Domain`, `domain_to_state`)
// that the engine since retired in favour of state-first pipeline.
// The `compat::Domain` struct below restores the serde-deserializable
// shape the JSON fixtures expect, and `compat::domain_to_state`
// lowers it into the cell-shaped state the current
// `compile_to_defs_state` consumes.
use arest::types::Violation;
use arest::compile;
use arest::evaluate;
use arest::ast;
use compat::{Domain, domain_to_state};

mod compat {
    use arest::ast;
    use arest::types::{NounDef, FactTypeDef, ConstraintDef, RoleDef, SpanDef, TransitionDef, WorldAssumption};
    use hashbrown::HashMap;
    use serde::{Deserialize, Serialize};

    #[derive(Default, Clone, Deserialize, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Domain {
        #[serde(default)] pub domain: String,
        #[serde(default)] pub nouns: HashMap<String, NounDefShim>,
        #[serde(default)] pub fact_types: HashMap<String, FactTypeShim>,
        #[serde(default)] pub constraints: Vec<ConstraintShim>,
        #[serde(default)] pub state_machines: HashMap<String, StateMachineShim>,
        #[serde(default)] pub enum_values: HashMap<String, Vec<String>>,
        #[serde(default)] pub subtypes: HashMap<String, String>,
    }

    #[derive(Default, Clone, Deserialize, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct NounDefShim {
        pub object_type: String,
        #[serde(default)] pub world_assumption: Option<String>,
    }

    #[derive(Default, Clone, Deserialize, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct FactTypeShim {
        pub reading: String,
        pub roles: Vec<RoleShim>,
    }

    #[derive(Default, Clone, Deserialize, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RoleShim {
        pub noun_name: String,
        pub role_index: usize,
    }

    #[derive(Default, Clone, Deserialize, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ConstraintShim {
        #[serde(default)] pub id: String,
        pub kind: String,
        #[serde(default)] pub modality: String,
        #[serde(default)] pub deontic_operator: Option<String>,
        pub text: String,
        pub spans: Vec<SpanShim>,
    }

    #[derive(Default, Clone, Deserialize, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SpanShim {
        pub fact_type_id: String,
        pub role_index: usize,
    }

    #[derive(Default, Clone, Deserialize, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct StateMachineShim {
        pub noun: String,
        pub initial_status: String,
        pub transitions: Vec<TransitionShim>,
    }

    #[derive(Default, Clone, Deserialize, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct TransitionShim {
        pub id: String,
        pub from_status: String,
        pub to_status: String,
        #[serde(default)] pub event_type: Option<String>,
    }

    /// Serialise the shim Domain into a cell-shaped state Object that
    /// compile_to_defs_state consumes. Only the cells that the tests
    /// exercise are written; anything else is left at φ.
    pub fn domain_to_state(ir: &Domain) -> ast::Object {
        let mut state = ast::Object::phi();

        // Noun cell.
        for (name, def) in &ir.nouns {
            let wa = def.world_assumption.as_deref().unwrap_or("closed");
            let mut pairs: Vec<(&str, &str)> = vec![
                ("name", name.as_str()),
                ("objectType", def.object_type.as_str()),
                ("worldAssumption", wa),
            ];
            let sup;
            if let Some(s) = ir.subtypes.get(name) {
                sup = s.clone();
                pairs.push(("superType", sup.as_str()));
            }
            let enum_joined;
            if let Some(vals) = ir.enum_values.get(name) {
                enum_joined = vals.join(",");
                pairs.push(("enumValues", enum_joined.as_str()));
            }
            state = ast::cell_push("Noun", ast::fact_from_pairs(&pairs), &state);
        }

        // FactType + Role cells.
        for (id, ft) in &ir.fact_types {
            let pairs = vec![("id", id.as_str()), ("reading", ft.reading.as_str())];
            state = ast::cell_push("FactType", ast::fact_from_pairs(&pairs), &state);
            for role in &ft.roles {
                let position = role.role_index.to_string();
                let role_pairs: Vec<(&str, &str)> = vec![
                    ("factType", id.as_str()),
                    ("nounName", role.noun_name.as_str()),
                    ("position", position.as_str()),
                ];
                state = ast::cell_push("Role", ast::fact_from_pairs(&role_pairs), &state);
            }
        }

        // Constraint cell — serialise to JSON for lossless round-trip.
        for c in &ir.constraints {
            let spans: Vec<SpanDef> = c.spans.iter().map(|s| SpanDef {
                fact_type_id: s.fact_type_id.clone(),
                role_index: s.role_index,
                subset_autofill: None,
            }).collect();
            let real = ConstraintDef {
                id: c.id.clone(),
                kind: c.kind.clone(),
                modality: c.modality.clone(),
                deontic_operator: c.deontic_operator.clone(),
                text: c.text.clone(),
                spans,
                set_comparison_argument_length: None,
                clauses: None,
                entity: None,
                min_occurrence: None,
                max_occurrence: None,
                ..Default::default()
            };
            let json = serde_json::to_string(&real).unwrap_or_default();
            let pairs = vec![
                ("id", c.id.as_str()),
                ("kind", c.kind.as_str()),
                ("modality", c.modality.as_str()),
                ("text", c.text.as_str()),
                ("json", json.as_str()),
            ];
            state = ast::cell_push("Constraint", ast::fact_from_pairs(&pairs), &state);
        }

        // StateMachine cell — serialise each transition directly as
        // JSON without round-tripping through the real engine types
        // (whose field names differ from these fixtures). The engine
        // reads these cells back via serde_json, so any shape the
        // engine's StateMachineDef/TransitionDef accepts is fine.
        for (id, sm) in &ir.state_machines {
            // Build the real-engine shape: StateMachineDef { nounName,
            // statuses, transitions: [{ from, to, event, guard }],
            // initial }.
            let trans_json: Vec<serde_json::Value> = sm.transitions.iter().map(|t| {
                serde_json::json!({
                    "from": t.from_status,
                    "to": t.to_status,
                    "event": t.event_type.clone().unwrap_or_else(|| t.id.clone()),
                    "guard": serde_json::Value::Null,
                })
            }).collect();
            let real = serde_json::json!({
                "nounName": sm.noun,
                "statuses": [],
                "transitions": trans_json,
                "initial": sm.initial_status,
            });
            let json = serde_json::to_string(&real).unwrap_or_default();
            let pairs = vec![
                ("id", id.as_str()),
                ("noun", sm.noun.as_str()),
                ("json", json.as_str()),
            ];
            state = ast::cell_push("StateMachine", ast::fact_from_pairs(&pairs), &state);
        }

        // Silence unused imports when no branch exercises them.
        let _ = std::marker::PhantomData::<(NounDef, FactTypeDef, RoleDef, WorldAssumption, TransitionDef)>;

        state
    }
}

use arest::parse_forml2;

/// Helper: build D from compile_to_defs_state output.
fn build_d(defs: &[(String, ast::Func)], state: &ast::Object) -> ast::Object {
    ast::defs_to_state(defs, state)
}

/// Helper: evaluate all constraint defs against a context, returning violations.
fn evaluate_constraints(
    defs: &[(String, ast::Func)],
    d: &ast::Object,
    text: &str,
    sender: Option<&str>,
    state: &ast::Object,
) -> Vec<Violation> {
    let ctx_obj = ast::encode_eval_context_state(text, sender, state);
    defs.iter()
        .filter(|(n, _)| n.starts_with("constraint:"))
        .flat_map(|(name, func)| {
            let result = ast::apply(func, &ctx_obj, d);
            let is_deontic = name.contains("obligatory") || name.contains("forbidden");
            ast::decode_violations(&result).into_iter().map(move |mut v| {
                v.alethic = !is_deontic;
                v
            })
        })
        .collect()
}

#[test]
fn test_full_pipeline_forbidden_text() {
    let ir_json = r#"{
        "domain": "test",
        "nouns": {
            "SupportResponse": { "objectType": "entity" },
            "ProhibitedText": { "objectType": "value" }
        },
        "factTypes": {
            "ft1": {
                "reading": "SupportResponse contains ProhibitedText",
                "roles": [
                    { "nounName": "SupportResponse", "roleIndex": 0 },
                    { "nounName": "ProhibitedText", "roleIndex": 1 }
                ]
            }
        },
        "constraints": [{
            "id": "c1",
            "kind": "UC",
            "modality": "Deontic",
            "deonticOperator": "forbidden",
            "text": "It is forbidden that SupportResponse contains ProhibitedText",
            "spans": [{ "factTypeId": "ft1", "roleIndex": 0 }]
        }],
        "stateMachines": {},
        "enumValues": { "ProhibitedText": ["\u2013", "\u2014"] }
    }"#;

    let ir: Domain = serde_json::from_str(ir_json).unwrap();
    let state = domain_to_state(&ir);
    let defs = compile::compile_to_defs_state(&state);
    let d = build_d(&defs, &ast::Object::phi());

    let empty_state = ast::Object::phi();
    let emdash = core::char::from_u32(0x2014).unwrap();
    let text = format!("Hello {} how are you?", emdash);
    let violations = evaluate_constraints(&defs, &d, &text, None, &empty_state);
    assert!(!violations.is_empty());
}

#[test]
fn test_full_pipeline_clean_response() {
    let ir_json = r#"{
        "domain": "test",
        "nouns": {
            "SupportResponse": { "objectType": "entity" },
            "ProhibitedText": { "objectType": "value" }
        },
        "enumValues": { "ProhibitedText": ["\u2013"] },
        "factTypes": {
            "ft1": {
                "reading": "SupportResponse contains ProhibitedText",
                "roles": [
                    { "nounName": "SupportResponse", "roleIndex": 0 },
                    { "nounName": "ProhibitedText", "roleIndex": 1 }
                ]
            }
        },
        "constraints": [{
            "id": "c1",
            "kind": "UC",
            "modality": "Deontic",
            "deonticOperator": "forbidden",
            "text": "It is forbidden that SupportResponse contains ProhibitedText",
            "spans": [{ "factTypeId": "ft1", "roleIndex": 0 }]
        }],
        "stateMachines": {}
    }"#;

    let ir: Domain = serde_json::from_str(ir_json).unwrap();
    let state = domain_to_state(&ir);
    let defs = compile::compile_to_defs_state(&state);
    let d = build_d(&defs, &ast::Object::phi());

    let empty_state = ast::Object::phi();
    let violations = evaluate_constraints(&defs, &d, "", None, &empty_state);
    assert!(violations.is_empty());
}

#[test]
fn test_full_pipeline_uniqueness_violation() {
    let ir_json = r#"{
        "domain": "test",
        "nouns": {
            "Customer": { "objectType": "entity" },
            "Name": { "objectType": "value", "valueType": "string" }
        },
        "factTypes": {
            "ft1": {
                "reading": "Customer has Name",
                "roles": [
                    { "nounName": "Customer", "roleIndex": 0 },
                    { "nounName": "Name", "roleIndex": 1 }
                ]
            }
        },
        "constraints": [{
            "id": "c1",
            "kind": "UC",
            "modality": "Alethic",
            "text": "Each Customer has at most one Name",
            "spans": [{ "factTypeId": "ft1", "roleIndex": 0 }]
        }],
        "stateMachines": {}
    }"#;

    let ir: Domain = serde_json::from_str(ir_json).unwrap();
    let state = domain_to_state(&ir);
    let defs = compile::compile_to_defs_state(&state);
    let _d = build_d(&defs, &ast::Object::phi());

    // task-744 / task-820 / task-822: alethic non-spanning UC enforcement
    // moved from constraint-evaluation time INTO the cell write. The FT
    // cell `ft1` is registered Map-keyed by its scope role `Customer`
    // (see `_CellKeyRoles`), so `compile_uniqueness_ast` lowers the UC to
    // a structural no-op (`Func::constant(φ)`) — evaluating the compiled
    // constraint over a Seq population built with `cell_push` will NEVER
    // report a violation (that path is deliberately retired). The UC is
    // now enforced when a second tuple lands at the same scope key:
    // `cell_put_keyed` returns `Err(KeyConflict)`, which the apply path
    // renders as "Uniqueness violation: key '…' is not unique in …".
    let key = ["Customer"];
    let s0 = ast::cell_put_keyed("ft1", &key,
        ast::fact_from_pairs(&[("Customer", "c1"), ("Name", "Alice")]),
        &ast::Object::phi())
        .expect("first write at a fresh scope key must succeed");
    // Customer c1 gets a SECOND, different Name -> UC conflict at key 'c1'.
    let conflict = ast::cell_put_keyed("ft1", &key,
        ast::fact_from_pairs(&[("Customer", "c1"), ("Name", "Bob")]), &s0)
        .expect_err("second tuple at the same scope key must raise a UC conflict");
    assert_eq!(conflict.key, "c1", "conflict key is the duplicated Customer scope value");
    assert_eq!(conflict.name, "ft1", "conflict is on the ft1 cell");
}

// --- Dual-instance convergence tests (Definition 2) ---

#[test]
fn test_dual_instance_convergence_valid() {
    let ir_json = r#"{
        "domain": "test",
        "nouns": {
            "Order": { "objectType": "entity" },
            "Customer": { "objectType": "entity" }
        },
        "factTypes": {
            "ft1": {
                "reading": "Order was placed by Customer",
                "roles": [
                    { "nounName": "Order", "roleIndex": 0 },
                    { "nounName": "Customer", "roleIndex": 1 }
                ]
            }
        },
        "constraints": [{
            "id": "uc1",
            "kind": "UC",
            "modality": "Alethic",
            "text": "Each Order was placed by at most one Customer",
            "spans": [{ "factTypeId": "ft1", "roleIndex": 0 }]
        }],
        "stateMachines": {}
    }"#;

    let ir: Domain = serde_json::from_str(ir_json).unwrap();

    let server_state = domain_to_state(&ir);
    let server_defs = compile::compile_to_defs_state(&server_state);
    let server_d = build_d(&server_defs, &ast::Object::phi());

    let client_state = domain_to_state(&ir);
    let client_defs = compile::compile_to_defs_state(&client_state);
    let client_d = build_d(&client_defs, &ast::Object::phi());

    // Valid state: each order has one customer
    let mut pop_state = ast::Object::phi();
    pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Order", "ord-1"), ("Customer", "acme")]), &pop_state);
    pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Order", "ord-2"), ("Customer", "beta")]), &pop_state);

    let server_violations = evaluate_constraints(&server_defs, &server_d, "", None, &pop_state);
    let client_violations = evaluate_constraints(&client_defs, &client_d, "", None, &pop_state);

    assert!(server_violations.is_empty(), "Server should see no violations");
    assert!(client_violations.is_empty(), "Client should see no violations");
}

#[test]
fn test_dual_instance_concurrent_write_conflict() {
    let ir_json = r#"{
        "domain": "test",
        "nouns": {
            "Order": { "objectType": "entity" },
            "Customer": { "objectType": "entity" }
        },
        "factTypes": {
            "ft1": {
                "reading": "Order was placed by Customer",
                "roles": [
                    { "nounName": "Order", "roleIndex": 0 },
                    { "nounName": "Customer", "roleIndex": 1 }
                ]
            }
        },
        "constraints": [{
            "id": "uc1",
            "kind": "UC",
            "modality": "Alethic",
            "text": "Each Order was placed by at most one Customer",
            "spans": [{ "factTypeId": "ft1", "roleIndex": 0 }]
        }],
        "stateMachines": {}
    }"#;

    let ir: Domain = serde_json::from_str(ir_json).unwrap();
    let ir_state = domain_to_state(&ir);
    let defs = compile::compile_to_defs_state(&ir_state);
    let _d = build_d(&defs, &ast::Object::phi());

    // task-744 / task-820 / task-822: the UC on `ft1` ("Order was placed
    // by exactly one Customer") is enforced at the cell write, keyed by
    // the scope role `Order`, not at constraint-evaluation time (which is
    // a lowered no-op for Map-keyed UCs). Each client's local single-fact
    // write succeeds; the server's merge of two different Customers for
    // the same Order surfaces the conflict via `cell_put_keyed`.
    let key = ["Order"];

    // Client A's local view: one Order→Customer fact. Valid.
    let client_a = ast::cell_put_keyed("ft1", &key,
        ast::fact_from_pairs(&[("Order", "ord-1"), ("Customer", "acme")]),
        &ast::Object::phi())
        .expect("Client A's local view is valid (single fact at key)");

    // Client B's local view: a different Customer for the same Order, but
    // in B's OWN state it is the only fact, so it is locally valid.
    let _client_b = ast::cell_put_keyed("ft1", &key,
        ast::fact_from_pairs(&[("Order", "ord-1"), ("Customer", "beta")]),
        &ast::Object::phi())
        .expect("Client B's local view is valid (single fact at key)");

    // Server merges both writes: writing B's fact on top of A's state
    // collides at key 'ord-1' — the concurrent-write conflict is detected.
    let conflict = ast::cell_put_keyed("ft1", &key,
        ast::fact_from_pairs(&[("Order", "ord-1"), ("Customer", "beta")]), &client_a)
        .expect_err("Server detects the conflict at the merged key");
    assert_eq!(conflict.key, "ord-1", "conflict key is the shared Order scope value");
    assert_eq!(conflict.name, "ft1", "conflict is on the ft1 cell");
}

#[test]
fn test_dual_instance_forward_chain_convergence() {
    let ir_json = r#"{
        "domain": "test",
        "nouns": {
            "Person": { "objectType": "entity" },
            "Department": { "objectType": "entity" }
        },
        "factTypes": {
            "ft_heads": {
                "reading": "Person heads Department",
                "roles": [
                    { "nounName": "Person", "roleIndex": 0 },
                    { "nounName": "Department", "roleIndex": 1 }
                ]
            },
            "ft_works": {
                "reading": "Person works in Department",
                "roles": [
                    { "nounName": "Person", "roleIndex": 0 },
                    { "nounName": "Department", "roleIndex": 1 }
                ]
            }
        },
        "constraints": [{
            "id": "ss1",
            "kind": "SS",
            "modality": "Alethic",
            "text": "If some Person heads some Department then that Person works in that Department",
            "spans": [
                { "factTypeId": "ft_heads", "roleIndex": 0, "subsetAutofill": true },
                { "factTypeId": "ft_works", "roleIndex": 0 }
            ]
        }],
        "stateMachines": {}
    }"#;

    let ir: Domain = serde_json::from_str(ir_json).unwrap();

    let server_state = domain_to_state(&ir);
    let server_defs = compile::compile_to_defs_state(&server_state);

    let client_state = domain_to_state(&ir);
    let client_defs = compile::compile_to_defs_state(&client_state);

    let mut pop_state = ast::Object::phi();
    pop_state = ast::cell_push("ft_heads", ast::fact_from_pairs(&[("Person", "alice"), ("Department", "eng")]), &pop_state);

    let server_derivation_defs: Vec<(&str, &ast::Func)> = server_defs.iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, f)| (n.as_str(), f))
        .collect();
    let client_derivation_defs: Vec<(&str, &ast::Func)> = client_defs.iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, f)| (n.as_str(), f))
        .collect();

    let (_server_new, server_derived) = evaluate::forward_chain_defs_state(&server_derivation_defs, &pop_state);
    let (_client_new, client_derived) = evaluate::forward_chain_defs_state(&client_derivation_defs, &pop_state);

    assert_eq!(server_derived.len(), client_derived.len(),
        "Server and client derive the same number of facts");

    if !server_derived.is_empty() {
        let mut s_facts: Vec<String> = server_derived.iter().map(|d| format!("{}:{:?}", d.fact_type_id, d.bindings)).collect();
        let mut c_facts: Vec<String> = client_derived.iter().map(|d| format!("{}:{:?}", d.fact_type_id, d.bindings)).collect();
        s_facts.sort();
        c_facts.sort();
        assert_eq!(s_facts, c_facts, "Derived facts are identical");
    }
}
