// crates/arest/tests/integration.rs
//
// Integration tests exercise the compile + evaluate pipeline directly,
// bypassing the wasm_bindgen layer (which requires JsValue).
//
// These tests use the DEFS path (compile_to_defs) instead of the
// CompiledModel path (compile + evaluate_via_ast).
use std::collections::HashMap;
use arest::types::{Domain, Population, Violation};
use arest::compile;
use arest::evaluate;
use arest::ast;
use arest::parse_forml2;

/// Helper: build a def_map from compile_to_defs output.
fn build_def_map(defs: &[(String, ast::Func)]) -> HashMap<String, ast::Func> {
    defs.iter().map(|(n, f)| (n.clone(), f.clone())).collect()
}

/// Helper: evaluate all constraint defs against a context, returning violations.
fn evaluate_constraints(
    defs: &[(String, ast::Func)],
    def_map: &HashMap<String, ast::Func>,
    text: &str,
    sender: Option<&str>,
    population: &Population,
) -> Vec<Violation> {
    let ctx_obj = ast::encode_eval_context(text, sender, population);
    defs.iter()
        .filter(|(n, _)| n.starts_with("constraint:"))
        .flat_map(|(name, func)| {
            let result = ast::apply(func, &ctx_obj, def_map);
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
    let pop = parse_forml2::domain_to_population(&ir);
    let defs = compile::compile_to_defs(&pop);
    let def_map = build_def_map(&defs);

    let population: Population = serde_json::from_str(r#"{"facts": {}}"#).unwrap();
    let emdash = core::char::from_u32(0x2014).unwrap();
    let text = format!("Hello {} how are you?", emdash);
    let violations = evaluate_constraints(&defs, &def_map, &text, None, &population);
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
    let pop = parse_forml2::domain_to_population(&ir);
    let defs = compile::compile_to_defs(&pop);
    let def_map = build_def_map(&defs);

    let population: Population = serde_json::from_str(r#"{"facts": {}}"#).unwrap();

    let violations = evaluate_constraints(&defs, &def_map, "", None, &population);
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
    let pop = parse_forml2::domain_to_population(&ir);
    let defs = compile::compile_to_defs(&pop);
    let def_map = build_def_map(&defs);

    // Customer c1 has two names -> UC violation
    let population: Population = serde_json::from_str(r#"{"facts": {
        "ft1": [
            { "factTypeId": "ft1", "bindings": [["Customer", "c1"], ["Name", "Alice"]] },
            { "factTypeId": "ft1", "bindings": [["Customer", "c1"], ["Name", "Bob"]] }
        ]
    }}"#).unwrap();

    let violations = evaluate_constraints(&defs, &def_map, "", None, &population);
    assert_eq!(violations.len(), 1);
    assert!(violations[0].detail.contains("Uniqueness violation"));
    assert_eq!(violations[0].constraint_id, "c1");
}

// --- Dual-instance convergence tests (Definition 2) ---

/// Two def_maps compiled from the same IR produce identical evaluation results.
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

    // Two independent def_maps (server and client)
    let server_pop = parse_forml2::domain_to_population(&ir);
    let server_defs = compile::compile_to_defs(&server_pop);
    let server_def_map = build_def_map(&server_defs);

    let client_pop = parse_forml2::domain_to_population(&ir);
    let client_defs = compile::compile_to_defs(&client_pop);
    let client_def_map = build_def_map(&client_defs);

    // Valid population: each order has one customer
    let population: Population = serde_json::from_str(r#"{"facts": {
        "ft1": [
            { "factTypeId": "ft1", "bindings": [["Order", "ord-1"], ["Customer", "acme"]] },
            { "factTypeId": "ft1", "bindings": [["Order", "ord-2"], ["Customer", "beta"]] }
        ]
    }}"#).unwrap();

    let server_violations = evaluate_constraints(&server_defs, &server_def_map, "", None, &population);
    let client_violations = evaluate_constraints(&client_defs, &client_def_map, "", None, &population);

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

    let ir: Domain = serde_json::from_str(ir_json).unwrap();
    let pop = parse_forml2::domain_to_population(&ir);
    let defs = compile::compile_to_defs(&pop);
    let def_map = build_def_map(&defs);

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
    let a_violations = evaluate_constraints(&defs, &def_map, "", None, &client_a_pop);
    let b_violations = evaluate_constraints(&defs, &def_map, "", None, &client_b_pop);
    assert!(a_violations.is_empty(), "Client A's local view is valid");
    assert!(b_violations.is_empty(), "Client B's local view is valid");

    // Server merges both writes: ord-1 has TWO customers -> UC violation
    let merged_pop: Population = serde_json::from_str(r#"{"facts": {
        "ft1": [
            { "factTypeId": "ft1", "bindings": [["Order", "ord-1"], ["Customer", "acme"]] },
            { "factTypeId": "ft1", "bindings": [["Order", "ord-1"], ["Customer", "beta"]] }
        ]
    }}"#).unwrap();

    let server_violations = evaluate_constraints(&defs, &def_map, "", None, &merged_pop);
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

    let ir: Domain = serde_json::from_str(ir_json).unwrap();

    // Two independent def_maps (server and client)
    let server_pop = parse_forml2::domain_to_population(&ir);
    let server_defs = compile::compile_to_defs(&server_pop);

    let client_pop = parse_forml2::domain_to_population(&ir);
    let client_defs = compile::compile_to_defs(&client_pop);

    let population: Population = serde_json::from_str(r#"{"facts": {
        "ft_heads": [
            { "factTypeId": "ft_heads", "bindings": [["Person", "alice"], ["Department", "eng"]] }
        ],
        "ft_works": []
    }}"#).unwrap();

    let server_derivation_defs: Vec<(&str, &ast::Func)> = server_defs.iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, f)| (n.as_str(), f))
        .collect();
    let client_derivation_defs: Vec<(&str, &ast::Func)> = client_defs.iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, f)| (n.as_str(), f))
        .collect();

    let mut server_pop = population.clone();
    let mut client_pop = population.clone();
    let server_derived = evaluate::forward_chain_defs(&server_derivation_defs, &mut server_pop);
    let client_derived = evaluate::forward_chain_defs(&client_derivation_defs, &mut client_pop);

    // Both should derive the same fact: alice works in eng
    assert_eq!(server_derived.len(), client_derived.len(),
        "Server and client derive the same number of facts");

    if !server_derived.is_empty() {
        // Compare by fact content, not debug string (HashMap iteration order may differ)
        let mut s_facts: Vec<String> = server_derived.iter().map(|d| format!("{}:{:?}", d.fact_type_id, d.bindings)).collect();
        let mut c_facts: Vec<String> = client_derived.iter().map(|d| format!("{}:{:?}", d.fact_type_id, d.bindings)).collect();
        s_facts.sort();
        c_facts.sort();
        assert_eq!(s_facts, c_facts,
            "Derived facts are identical"
        );
    }
}
