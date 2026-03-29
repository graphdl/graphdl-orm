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
            "ProhibitedText": { "objectType": "value", "enumValues": ["—", "–"], "valueType": "string" }
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
        "stateMachines": {}
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
            "ProhibitedText": { "objectType": "value", "enumValues": ["—"], "valueType": "string" }
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
