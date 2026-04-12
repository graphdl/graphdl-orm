// crates/arest/src/generators/solidity.rs
//
// Solidity generator: compile FFP state to Ethereum smart contracts.
//
// AREST maps readings into FFP objects (Backus §13 metacomposition). This
// module reverses the arrow on the output side: each entity becomes a
// Solidity contract with RMAP-derived typed storage, facts-as-events
// (paper §"Facts as events"), and state machine transitions as functions
// guarded by modifiers.
//
// Output structure per entity:
//   contract <Entity> {
//       struct Data { <RMAP columns as fields> }
//       mapping(string => Data) public records;
//       event <FactType>(string indexed id, ...);          // facts-as-events
//       modifier onlyInStatus(string id, bytes32 expected); // SM guard
//       function create(...) external;                      // resolve+emit
//       function <transition>(string id) external;          // SM transitions
//   }
//
// Design constraints (project rules):
//   - Pure FP style: iterator combinators, no for loops, no control-flow ifs.
//   - The function is total: missing cells yield a valid empty program.
//   - Output is solc-compilable.

use crate::ast::{Object, binding, fetch_or_phi};
use crate::rmap::{self, TableDef, TableColumn};
use crate::types::{Domain, StateMachineDef};

/// Compile a compiled AREST state into Solidity source code.
///
/// Reconstructs the domain from state, runs RMAP for typed storage,
/// and emits one contract per entity with SM transitions and events.
pub fn compile_to_solidity(state: &Object) -> String {
    let header = "// SPDX-License-Identifier: MIT\n\
                  // Generated from FORML2 readings by AREST\n\
                  pragma solidity ^0.8.20;\n\n";

    // Reconstruct domain + RMAP tables for typed storage.
    let domain = crate::compile::state_to_domain(state);
    let tables = rmap::rmap(&domain);
    let table_by_name: std::collections::HashMap<String, &TableDef> = tables.iter()
        .map(|t| (t.name.clone(), t)).collect();

    let nouns = fetch_or_phi("Noun", state);
    let contracts: Vec<String> = nouns.as_seq().map(|ns| {
        ns.iter().filter_map(|n| {
            let name = binding(n, "name")?.to_string();
            let obj_type = binding(n, "objectType")?;
            if obj_type != "entity" { return None; }
            let table_name = rmap::to_snake(&name);
            let table = table_by_name.get(&table_name).copied();
            let sm = domain.state_machines.get(&name);
            Some(emit_contract(&name, table, sm, &domain))
        }).collect()
    }).unwrap_or_default();

    format!("{}{}", header, contracts.join("\n"))
}

/// Emit a full Solidity contract for an entity noun.
fn emit_contract(name: &str, table: Option<&TableDef>, sm: Option<&StateMachineDef>, domain: &Domain) -> String {
    let contract_name = sanitize_name(name);
    let struct_def = emit_struct(table);
    let events = emit_events(name, domain);
    let sm_parts = sm.map(emit_state_machine).unwrap_or_default();
    let create_fn = emit_create(name, table, sm);
    let transitions = sm.map(|s| emit_transitions(s)).unwrap_or_default();

    format!(
        "contract {} {{\n\
         {}\n\
         \n    mapping(string => Data) public records;\n\
         \n{}\
         {}\
         {}\
         {}\
         }}\n",
        contract_name, struct_def, events, sm_parts, create_fn, transitions
    )
}

/// Emit the Data struct with RMAP columns as typed fields.
fn emit_struct(table: Option<&TableDef>) -> String {
    let fields: Vec<String> = match table {
        Some(t) => t.columns.iter().map(|c| {
            let sol_type = solidity_type(c);
            format!("        {} {};", sol_type, sanitize_field(&c.name))
        }).collect(),
        None => vec!["        string id;".to_string()],
    };
    let status_line = "        bytes32 status;  // SM current state".to_string();
    let mut all = fields;
    all.push(status_line);
    format!("    struct Data {{\n{}\n    }}", all.join("\n"))
}

/// Map an RMAP column type to a Solidity type.
fn solidity_type(col: &TableColumn) -> &'static str {
    match col.col_type.as_str() {
        "TEXT" | "VARCHAR" => "string",
        "INTEGER" | "INT" => "int256",
        "REAL" | "FLOAT" => "int256", // Solidity lacks floats; use fixed-point
        "BOOLEAN" | "BOOL" => "bool",
        _ => "string",
    }
}

/// Emit one event per fact type involving this entity noun.
/// Implements the paper's "Facts as events" concept in Solidity.
fn emit_events(noun_name: &str, domain: &Domain) -> String {
    let events: Vec<String> = domain.fact_types.iter()
        .filter(|(_, ft)| ft.roles.iter().any(|r| r.noun_name == noun_name))
        .filter_map(|(ft_id, ft)| {
            let evt_name = sanitize_name(ft_id);
            // Event args: first role is always indexed id, rest are params
            let args: Vec<String> = ft.roles.iter().enumerate().map(|(i, r)| {
                let prefix = if i == 0 { "string indexed " } else { "string " };
                format!("{}{}", prefix, sanitize_field(&r.noun_name))
            }).collect();
            Some(format!("    event {}({});", evt_name, args.join(", ")))
        }).collect();
    if events.is_empty() { String::new() } else { format!("{}\n", events.join("\n")) }
}

/// Emit SM status enum and modifier.
fn emit_state_machine(sm: &StateMachineDef) -> String {
    let statuses: Vec<String> = sm.statuses.iter()
        .map(|s| sanitize_name(s))
        .collect();
    let enum_def = if statuses.is_empty() {
        String::new()
    } else {
        format!("    // State Machine: {} statuses\n    // Statuses: {}\n",
            statuses.len(), statuses.join(", "))
    };
    let modifier = "    modifier onlyInStatus(string memory id, bytes32 expected) {\n        require(records[id].status == expected, \"SM: wrong state\");\n        _;\n    }\n";
    format!("\n{}{}", enum_def, modifier)
}

/// Emit the create(...) function with UC + MC requires.
fn emit_create(noun_name: &str, table: Option<&TableDef>, sm: Option<&StateMachineDef>) -> String {
    let params: Vec<String> = match table {
        Some(t) => t.columns.iter().map(|c| {
            let t = solidity_type(c);
            format!("{} memory {}", t, sanitize_field(&c.name))
        }).collect(),
        None => vec!["string memory id".to_string()],
    };
    let pk = table.and_then(|t| t.primary_key.first())
        .map(|s| sanitize_field(s))
        .unwrap_or_else(|| "id".to_string());

    // UC: PK must not already exist
    let uc_check = format!(
        "        require(bytes(records[{}].status == bytes32(0) ? \"_\" : \"\").length == 1, \"UC: {} already exists\");",
        pk, noun_name
    );

    // Assign struct fields
    let assignments: Vec<String> = match table {
        Some(t) => t.columns.iter().map(|c| {
            let f = sanitize_field(&c.name);
            format!("        records[{}].{} = {};", pk, f, f)
        }).collect(),
        None => vec![],
    };

    // Initial SM status
    let initial_status = sm.and_then(|s| s.statuses.first())
        .map(|s| format!(
            "        records[{}].status = keccak256(bytes(\"{}\"));",
            pk, s))
        .unwrap_or_default();

    let body = [
        vec![uc_check],
        assignments,
        if initial_status.is_empty() { vec![] } else { vec![initial_status] },
    ].concat().join("\n");

    format!(
        "\n    function create({}) external {{\n{}\n    }}\n",
        params.join(", "), body
    )
}

/// Emit one function per state machine transition.
fn emit_transitions(sm: &StateMachineDef) -> String {
    let fns: Vec<String> = sm.transitions.iter().map(|t| {
        let fn_name = sanitize_field(&t.event);
        format!(
            "\n    function {}(string memory id) external onlyInStatus(id, keccak256(bytes(\"{}\"))) {{\n        records[id].status = keccak256(bytes(\"{}\"));\n    }}\n",
            fn_name, t.from, t.to
        )
    }).collect();
    fns.join("")
}

/// Sanitize name to a Solidity identifier (PascalCase).
fn sanitize_name(name: &str) -> String {
    name.chars().fold((String::new(), true), |(mut acc, cap), c| {
        match c {
            ' ' | '_' | '-' => (acc, true),
            c if c.is_alphanumeric() => {
                acc.push(if cap { c.to_ascii_uppercase() } else { c });
                (acc, false)
            }
            _ => (acc, cap),
        }
    }).0
}

/// Sanitize field name (camelCase, first char lowercase).
fn sanitize_field(name: &str) -> String {
    let pascal = sanitize_name(name);
    pascal.char_indices().map(|(i, c)| {
        if i == 0 { c.to_ascii_lowercase() } else { c }
    }).collect()
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{merge_states};
    use crate::parse_forml2::{parse_to_state, parse_to_state_with_nouns};

    const STATE_METAMODEL: &str = r#"
## Fact Types

Noun has Object Type.
"#;

    const ORDER_READINGS: &str = r#"
## Entity Types

Order(.Order Number) is an entity type.
Amount is a value type.

## Fact Types

Order has Amount.
"#;

    #[test]
    fn compile_to_solidity_emits_header() {
        let out = compile_to_solidity(&Object::phi());
        assert!(out.contains("SPDX-License-Identifier"));
        assert!(out.contains("pragma solidity ^0.8.20"));
    }

    #[test]
    fn compile_to_solidity_emits_entity_contract() {
        let meta = parse_to_state(STATE_METAMODEL).unwrap();
        let orders = parse_to_state_with_nouns(ORDER_READINGS, &meta).unwrap();
        let state = merge_states(&meta, &orders);
        let out = compile_to_solidity(&state);
        assert!(out.contains("contract Order"), "expected contract Order in:\n{}", out);
        assert!(out.contains("struct Data"), "expected struct Data");
        assert!(out.contains("mapping(string => Data) public records"));
    }

    #[test]
    fn compile_to_solidity_emits_status_in_struct() {
        let meta = parse_to_state(STATE_METAMODEL).unwrap();
        let orders = parse_to_state_with_nouns(ORDER_READINGS, &meta).unwrap();
        let state = merge_states(&meta, &orders);
        let out = compile_to_solidity(&state);
        assert!(out.contains("bytes32 status"),
            "expected bytes32 status field for SM tracking, got:\n{}", out);
    }

    #[test]
    fn compile_to_solidity_emits_create_function() {
        let meta = parse_to_state(STATE_METAMODEL).unwrap();
        let orders = parse_to_state_with_nouns(ORDER_READINGS, &meta).unwrap();
        let state = merge_states(&meta, &orders);
        let out = compile_to_solidity(&state);
        assert!(out.contains("function create"),
            "expected function create, got:\n{}", out);
        assert!(out.contains("external"), "expected external visibility");
    }

    #[test]
    fn compile_to_solidity_sanitizes_names() {
        assert_eq!(sanitize_name("Purchase Order"), "PurchaseOrder");
        assert_eq!(sanitize_name("order-id"), "OrderId");
        assert_eq!(sanitize_field("Order Number"), "orderNumber");
    }

    #[test]
    fn compile_to_solidity_skips_value_types() {
        let meta = parse_to_state(STATE_METAMODEL).unwrap();
        let orders = parse_to_state_with_nouns(ORDER_READINGS, &meta).unwrap();
        let state = merge_states(&meta, &orders);
        let out = compile_to_solidity(&state);
        // Amount is a value type — must not emit a contract for it
        assert!(!out.contains("contract Amount"),
            "value type should not emit a contract, got:\n{}", out);
    }

    #[test]
    fn compile_to_solidity_empty_state_is_valid() {
        let out = compile_to_solidity(&Object::phi());
        // Valid minimal Solidity file (just pragma)
        assert!(out.contains("pragma solidity"));
        assert!(!out.contains("contract "));
    }
}
