// crates/constraint-eval/tests/integration.rs
use constraint_eval::{load_ir, evaluate_response};
use serde_json::Value;

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

    load_ir(ir_json).unwrap();

    let response = r#"{"text": "Hello — how are you?", "senderIdentity": null, "fields": null}"#;
    let population = r#"{"facts": {}}"#;

    let result = evaluate_response(response, population);
    let violations: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
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

    load_ir(ir_json).unwrap();

    let response = r#"{"text": "Hello, how are you today?", "senderIdentity": null, "fields": null}"#;
    let population = r#"{"facts": {}}"#;

    let result = evaluate_response(response, population);
    let violations: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
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

    load_ir(ir_json).unwrap();

    // Customer c1 has two names → UC violation
    let response = r#"{"text": "", "senderIdentity": null, "fields": null}"#;
    let population = r#"{"facts": {
        "ft1": [
            { "factTypeId": "ft1", "bindings": [["Customer", "c1"], ["Name", "Alice"]] },
            { "factTypeId": "ft1", "bindings": [["Customer", "c1"], ["Name", "Bob"]] }
        ]
    }}"#;

    let result = evaluate_response(response, population);
    let violations: Vec<Value> = serde_json::from_str(&result).unwrap();
    assert_eq!(violations.len(), 1);
    assert!(violations[0]["detail"].as_str().unwrap().contains("Uniqueness violation"));
    assert!(violations[0]["constraintId"].as_str().unwrap() == "c1");
}
