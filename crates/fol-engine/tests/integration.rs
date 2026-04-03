// crates/fol-engine/tests/integration.rs
//
// Integration tests exercise the compile + evaluate pipeline directly,
// bypassing the wasm_bindgen layer (which requires JsValue).
use fol_engine::types::{ConstraintIR, ResponseContext, Population};
use fol_engine::compile;
use fol_engine::evaluate;

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
        "enumValues": { "ProhibitedText": ["—", "–"] }
    }"#;

    let ir: ConstraintIR = serde_json::from_str(ir_json).unwrap();
    let model = compile::compile(&ir);

    let response: ResponseContext = serde_json::from_str(
        r#"{"text": "Hello — how are you?", "senderIdentity": null, "fields": null}"#
    ).unwrap();
    let population: Population = serde_json::from_str(r#"{"facts": {}}"#).unwrap();

    let violations = evaluate::evaluate_via_ast(&model, &response, &population);
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
        "enumValues": { "ProhibitedText": ["—"] },
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

    let ir: ConstraintIR = serde_json::from_str(ir_json).unwrap();
    let model = compile::compile(&ir);

    let response: ResponseContext = serde_json::from_str(
        r#"{"text": "Hello, how are you today?", "senderIdentity": null, "fields": null}"#
    ).unwrap();
    let population: Population = serde_json::from_str(r#"{"facts": {}}"#).unwrap();

    let violations = evaluate::evaluate_via_ast(&model, &response, &population);
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

    let ir: ConstraintIR = serde_json::from_str(ir_json).unwrap();
    let model = compile::compile(&ir);

    // Customer c1 has two names -> UC violation
    let response: ResponseContext = serde_json::from_str(
        r#"{"text": "", "senderIdentity": null, "fields": null}"#
    ).unwrap();
    let population: Population = serde_json::from_str(r#"{"facts": {
        "ft1": [
            { "factTypeId": "ft1", "bindings": [["Customer", "c1"], ["Name", "Alice"]] },
            { "factTypeId": "ft1", "bindings": [["Customer", "c1"], ["Name", "Bob"]] }
        ]
    }}"#).unwrap();

    let violations = evaluate::evaluate_via_ast(&model, &response, &population);
    assert_eq!(violations.len(), 1);
    assert!(violations[0].detail.contains("Uniqueness violation"));
    assert_eq!(violations[0].constraint_id, "c1");
}

// ── Dual-instance convergence tests (Definition 2) ──────────────────

/// Two compiled models from the same IR produce identical evaluation results.
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

    let ir: ConstraintIR = serde_json::from_str(ir_json).unwrap();

    // Two independent compiled models (server and client)
    let server_model = compile::compile(&ir);
    let client_model = compile::compile(&ir);

    let response: ResponseContext = serde_json::from_str(
        r#"{"text": "", "senderIdentity": null, "fields": null}"#
    ).unwrap();

    // Valid population: each order has one customer
    let population: Population = serde_json::from_str(r#"{"facts": {
        "ft1": [
            { "factTypeId": "ft1", "bindings": [["Order", "ord-1"], ["Customer", "acme"]] },
            { "factTypeId": "ft1", "bindings": [["Order", "ord-2"], ["Customer", "beta"]] }
        ]
    }}"#).unwrap();

    let server_violations = evaluate::evaluate_via_ast(&server_model, &response, &population);
    let client_violations = evaluate::evaluate_via_ast(&client_model, &response, &population);

    // Both produce zero violations
    assert!(server_violations.is_empty(), "Server should see no violations");
    assert!(client_violations.is_empty(), "Client should see no violations");
}

/// Server detects violation from concurrent client writes.
/// Client A assigns ord-1 to acme. Client B assigns ord-1 to beta.
/// Each client's local view is valid. The merged population violates UC.
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

    let ir: ConstraintIR = serde_json::from_str(ir_json).unwrap();
    let server_model = compile::compile(&ir);

    let response: ResponseContext = serde_json::from_str(
        r#"{"text": "", "senderIdentity": null, "fields": null}"#
    ).unwrap();

    // Client A's local view: ord-1 placed by acme (valid locally)
    let client_a_pop: Population = serde_json::from_str(r#"{"facts": {
        "ft1": [
            { "factTypeId": "ft1", "bindings": [["Order", "ord-1"], ["Customer", "acme"]] }
        ]
    }}"#).unwrap();

    // Client B's local view: ord-1 placed by beta (valid locally)
    let client_b_pop: Population = serde_json::from_str(r#"{"facts": {
        "ft1": [
            { "factTypeId": "ft1", "bindings": [["Order", "ord-1"], ["Customer", "beta"]] }
        ]
    }}"#).unwrap();

    // Both local views are valid
    let a_violations = evaluate::evaluate_via_ast(&server_model, &response, &client_a_pop);
    let b_violations = evaluate::evaluate_via_ast(&server_model, &response, &client_b_pop);
    assert!(a_violations.is_empty(), "Client A's local view is valid");
    assert!(b_violations.is_empty(), "Client B's local view is valid");

    // Server merges both writes: ord-1 has TWO customers -> UC violation
    let merged_pop: Population = serde_json::from_str(r#"{"facts": {
        "ft1": [
            { "factTypeId": "ft1", "bindings": [["Order", "ord-1"], ["Customer", "acme"]] },
            { "factTypeId": "ft1", "bindings": [["Order", "ord-1"], ["Customer", "beta"]] }
        ]
    }}"#).unwrap();

    let server_violations = evaluate::evaluate_via_ast(&server_model, &response, &merged_pop);
    assert_eq!(server_violations.len(), 1, "Server detects the conflict");
    assert!(server_violations[0].detail.contains("Uniqueness violation"));
}

/// Server and client produce identical forward chain results.
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

    let ir: ConstraintIR = serde_json::from_str(ir_json).unwrap();
    let server_model = compile::compile(&ir);
    let client_model = compile::compile(&ir);

    let population: Population = serde_json::from_str(r#"{"facts": {
        "ft_heads": [
            { "factTypeId": "ft_heads", "bindings": [["Person", "alice"], ["Department", "eng"]] }
        ],
        "ft_works": []
    }}"#).unwrap();

    let mut server_pop = population.clone();
    let mut client_pop = population.clone();
    let server_derived = evaluate::forward_chain_ast(&server_model, &mut server_pop);
    let client_derived = evaluate::forward_chain_ast(&client_model, &mut client_pop);

    // Both should derive the same fact: alice works in eng
    assert_eq!(server_derived.len(), client_derived.len(),
        "Server and client derive the same number of facts");

    if !server_derived.is_empty() {
        assert_eq!(
            format!("{:?}", server_derived),
            format!("{:?}", client_derived),
            "Derived facts are identical"
        );
    }
}
