// crates/constraint-eval/tests/integration.rs
use constraint_eval::{load_ir, evaluate_response};

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
