// crates/arest/tests/integration.rs
//
// Integration tests exercise the compile + evaluate pipeline directly,
// bypassing the wasm_bindgen layer (which requires JsValue).
//
// These tests use the DEFS path (compile_to_defs_state) with Object state.
use std::collections::HashMap;
use arest::types::{Domain, Violation};
use arest::compile;
use arest::evaluate;
use arest::ast;
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
    let state = parse_forml2::domain_to_state(&ir);
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
    let state = parse_forml2::domain_to_state(&ir);
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
    let state = parse_forml2::domain_to_state(&ir);
    let defs = compile::compile_to_defs_state(&state);
    let d = build_d(&defs, &ast::Object::phi());

    // Customer c1 has two names -> UC violation
    let mut pop_state = ast::Object::phi();
    pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Customer", "c1"), ("Name", "Alice")]), &pop_state);
    pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Customer", "c1"), ("Name", "Bob")]), &pop_state);

    let violations = evaluate_constraints(&defs, &d, "", None, &pop_state);
    assert_eq!(violations.len(), 1);
    assert!(violations[0].detail.contains("Uniqueness violation"));
    assert_eq!(violations[0].constraint_id, "c1");
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

    let server_state = parse_forml2::domain_to_state(&ir);
    let server_defs = compile::compile_to_defs_state(&server_state);
    let server_d = build_d(&server_defs, &ast::Object::phi());

    let client_state = parse_forml2::domain_to_state(&ir);
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
    let ir_state = parse_forml2::domain_to_state(&ir);
    let defs = compile::compile_to_defs_state(&ir_state);
    let d = build_d(&defs, &ast::Object::phi());

    // Client A's local view
    let mut client_a_state = ast::Object::phi();
    client_a_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Order", "ord-1"), ("Customer", "acme")]), &client_a_state);

    // Client B's local view
    let mut client_b_state = ast::Object::phi();
    client_b_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Order", "ord-1"), ("Customer", "beta")]), &client_b_state);

    let a_violations = evaluate_constraints(&defs, &d, "", None, &client_a_state);
    let b_violations = evaluate_constraints(&defs, &d, "", None, &client_b_state);
    assert!(a_violations.is_empty(), "Client A's local view is valid");
    assert!(b_violations.is_empty(), "Client B's local view is valid");

    // Server merges both writes
    let mut merged_state = ast::Object::phi();
    merged_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Order", "ord-1"), ("Customer", "acme")]), &merged_state);
    merged_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Order", "ord-1"), ("Customer", "beta")]), &merged_state);

    let server_violations = evaluate_constraints(&defs, &d, "", None, &merged_state);
    assert_eq!(server_violations.len(), 1, "Server detects the conflict");
    assert!(server_violations[0].detail.contains("Uniqueness violation"));
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

    let server_state = parse_forml2::domain_to_state(&ir);
    let server_defs = compile::compile_to_defs_state(&server_state);

    let client_state = parse_forml2::domain_to_state(&ir);
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
