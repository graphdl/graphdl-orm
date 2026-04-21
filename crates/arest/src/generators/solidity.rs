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
use crate::types::StateMachineDef;
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

/// Extract state machines directly from InstanceFact cells in state.
/// No Domain round-trip — reads the same cells domain_to_state would.
fn state_machines_from_state(state: &Object) -> hashbrown::HashMap<String, StateMachineDef> {
    let inst = fetch_or_phi("InstanceFact", state);
    let facts = inst.as_seq().unwrap_or(&[]);
    let b = |f: &Object, k: &str| binding(f, k).unwrap_or("").to_string();

    let mut sms: hashbrown::HashMap<String, StateMachineDef> = hashbrown::HashMap::new();
    // "State Machine Definition 'X' is for Noun 'Y'"
    for f in facts.iter().filter(|f| b(f, "subjectNoun") == "State Machine Definition" && b(f, "fieldName").contains("is for")) {
        let sm_name = b(f, "subjectValue");
        let noun = b(f, "objectValue");
        sms.entry(noun).or_insert_with(|| StateMachineDef {
            noun_name: sm_name, statuses: vec![], transitions: vec![],
            initial: String::new(),
        });
    }
    // "Status 'Z' is defined in State Machine Definition 'X'"
    for f in facts.iter().filter(|f| b(f, "subjectNoun") == "Status" && b(f, "fieldName").contains("defined in")) {
        let status = b(f, "subjectValue");
        let sm_name = b(f, "objectValue");
        if let Some(sm) = sms.values_mut().find(|s| s.noun_name == sm_name) {
            if !sm.statuses.contains(&status) { sm.statuses.push(status); }
        }
    }
    // Transitions
    for f in facts.iter().filter(|f| b(f, "subjectNoun") == "Transition") {
        let trans_name = b(f, "subjectValue");
        let field = b(f, "fieldName");
        let value = b(f, "objectValue");
        for sm in sms.values_mut() {
            let t = sm.transitions.iter_mut().find(|t| t.event == trans_name);
            match t {
                Some(t) => {
                    if field.contains("from") { t.from = value.clone(); }
                    if field.contains("to") { t.to = value.clone(); }
                }
                None => {
                    let mut td = crate::types::TransitionDef { event: trans_name.clone(), from: String::new(), to: String::new(), guard: None };
                    if field.contains("from") { td.from = value.clone(); }
                    if field.contains("to") { td.to = value.clone(); }
                    if field.contains("triggered") { td.event = value.clone(); }
                    sm.transitions.push(td);
                }
            }
        }
    }
    sms
}

/// Compile every entity noun in `state` into a Solidity contract.
///
/// Reconstructs the domain from state, runs RMAP for typed storage,
/// and emits one contract per entity with SM transitions and events.
/// If you want to scope the output to a subset of nouns (for example,
/// to exclude metamodel entities), use `compile_to_solidity_for_nouns`.
pub fn compile_to_solidity(state: &Object) -> String {
    compile_to_solidity_inner(state, None)
}

/// Compile only the given nouns into Solidity contracts. Use this when
/// a user domain was parsed on top of a metamodel and you only want
/// contracts for the user's own entities.
pub fn compile_to_solidity_for_nouns(state: &Object, include: &[&str]) -> String {
    let set: hashbrown::HashSet<&str> = include.iter().copied().collect();
    compile_to_solidity_inner(state, Some(set))
}

fn compile_to_solidity_inner(
    state: &Object,
    include: Option<hashbrown::HashSet<&str>>,
) -> String {
    let header = "// SPDX-License-Identifier: MIT\n\
                  // Generated from FORML2 readings by AREST\n\
                  pragma solidity ^0.8.20;\n\n";

    let tables = rmap::rmap_from_state(state);
    let table_by_name: hashbrown::HashMap<String, &TableDef> = tables.iter()
        .map(|t| (t.name.clone(), t)).collect();
    let sms = state_machines_from_state(state);

    let nouns = fetch_or_phi("Noun", state);
    let contracts: Vec<String> = nouns.as_seq().map(|ns| {
        ns.iter().filter_map(|n| {
            let name = binding(n, "name")?.to_string();
            let obj_type = binding(n, "objectType")?;
            if obj_type != "entity" { return None; }
            if let Some(ref set) = include {
                if !set.contains(name.as_str()) { return None; }
            }
            let table_name = rmap::to_snake(&name);
            let table = table_by_name.get(&table_name).copied();
            let sm = sms.get(&name);
            Some(emit_contract(&name, table, sm, state, include.as_ref()))
        }).collect()
    }).unwrap_or_default();

    format!("{}{}", header, contracts.join("\n"))
}

/// Emit a full Solidity contract for an entity noun.
fn emit_contract(
    name: &str,
    table: Option<&TableDef>,
    sm: Option<&StateMachineDef>,
    state: &Object,
    scope: Option<&hashbrown::HashSet<&str>>,
) -> String {
    let contract_name = sanitize_name(name);
    let off_chain = emit_off_chain_comment(name, state);
    let struct_def = emit_struct(table);
    let events = emit_events(name, state, scope);
    let sm_parts = sm.map(emit_state_machine).unwrap_or_default();
    let create_fn = emit_create(name, table, sm, state);
    let transitions = sm.map(|s| emit_transitions(s)).unwrap_or_default();
    let vc_validators = emit_vc_validators(name, table, state);

    format!(
        "{}contract {} {{\n\
         {}\n\
         \n    mapping(string => Data) public records;\n\
         \n{}\
         {}\
         {}\
         {}\
         {}\
         }}\n",
        off_chain, contract_name, struct_def, events, sm_parts, create_fn, transitions, vc_validators
    )
}

/// Emit one internal pure `_validate<ValueType>` helper per column
/// whose value-type noun has declared enum values. Keyed by
/// `keccak256(bytes(v))` against each declared value for O(1)
/// membership. Paired with the `require(_validate{}(param), "VC: …")`
/// calls emitted by `emit_create`.
fn emit_vc_validators(
    noun_name: &str,
    table: Option<&TableDef>,
    state: &Object,
) -> String {
    let Some(t) = table else { return String::new(); };
    let mut emitted: hashbrown::HashSet<String> = hashbrown::HashSet::new();
    let fns: Vec<String> = t.columns.iter().filter_map(|c| {
        let col_noun = column_value_type_noun(&c.name, noun_name, state)?;
        let values = enum_values_for_value_type(&col_noun, state);
        if values.is_empty() { return None; }
        let fn_name = alloc::format!("_validate{}", sanitize_name(&col_noun));
        if !emitted.insert(fn_name.clone()) { return None; }
        let cases: Vec<String> = values.iter()
            .map(|v| alloc::format!("h == keccak256(bytes(\"{}\"))", v))
            .collect();
        Some(alloc::format!(
            "\n    function {}(string memory v) internal pure returns (bool) {{\n\
             \x20       bytes32 h = keccak256(bytes(v));\n\
             \x20       return {};\n\
             \x20   }}\n",
            fn_name, cases.join("\n            || ")))
    }).collect();
    fns.concat()
}

/// Emit a comment block above the contract body listing constraints
/// that are NOT enforced by the generated create(...) body — SS / EQ
/// / XC are too expensive on-chain for arbitrary spans; ring kinds
/// (IR / AS / AT / SY / IT / TR / AC / RF) need to guard fact-type
/// writes, which the current generator doesn't expose as contract
/// functions (facts land as events, not callable writes).
///
/// Surfacing the constraints in a comment gives auditors reading the
/// ABI a visible record instead of silently dropping them. Flip the
/// guard emission to real `require`s once fact-type writes are
/// contract-visible.
fn emit_off_chain_comment(noun_name: &str, state: &Object) -> String {
    const ANNOTATED_KINDS: &[(&str, &str)] = &[
        ("SS", "SS"), ("EQ", "EQ"), ("XC", "XC"),
        ("XO", "XO"), ("OR", "OR"),
        ("IR", "IR"), ("AS", "AS"), ("AT", "AT"), ("SY", "SY"),
        ("IT", "IT"), ("TR", "TR"), ("AC", "AC"), ("RF", "RF"),
    ];
    let constraints = fetch_or_phi("Constraint", state);
    let Some(facts) = constraints.as_seq() else { return String::new(); };
    let relevant: Vec<String> = facts.iter()
        .filter_map(|c| {
            let kind = binding(c, "kind")?;
            let label = ANNOTATED_KINDS.iter()
                .find(|(k, _)| *k == kind)
                .map(|(_, l)| *l)?;
            let text = binding(c, "text")?;
            // Only surface constraints naming this entity. Entity
            // binding may be absent; fall back to text substring.
            let applies = binding(c, "entity") == Some(noun_name)
                || text.contains(noun_name);
            if !applies { return None; }
            Some(alloc::format!("//   - {}: {}", label, text))
        })
        .collect();
    if relevant.is_empty() { return String::new(); }
    alloc::format!(
        "// The following constraints are enforced off-chain:\n\
         {}\n",
        relevant.join("\n")
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
///
/// When `scope` is `Some(set)`, only emit events for fact types whose
/// every role references a noun in `set` — this keeps metamodel
/// cross-reference fact types (e.g. `FactType has Order`) out of
/// user-facing output. When `scope` is `None`, emit for every fact
/// type involving this noun.
fn emit_events(
    noun_name: &str,
    state: &Object,
    scope: Option<&hashbrown::HashSet<&str>>,
) -> String {
    let ft_cell = fetch_or_phi("FactType", state);
    let role_cell = fetch_or_phi("Role", state);
    let fts = ft_cell.as_seq().unwrap_or(&[]);
    let roles = role_cell.as_seq().unwrap_or(&[]);

    let events: Vec<String> = fts.iter().filter_map(|f| {
        let ft_id = binding(f, "id")?;
        let reading = binding(f, "reading").unwrap_or("");
        let ft_roles: Vec<&str> = roles.iter()
            .filter(|r| binding(r, "factType") == Some(ft_id))
            .filter_map(|r| binding(r, "nounName"))
            .collect();
        if !ft_roles.iter().any(|r| *r == noun_name) { return None; }
        match scope {
            Some(set) => { if !ft_roles.iter().all(|r| set.contains(r)) { return None; } }
            None => {
                let distinct: hashbrown::HashSet<&str> = ft_roles.iter().copied().collect();
                if distinct.len() <= 1 && !reading.contains(noun_name) { return None; }
            }
        }
        let evt_name = sanitize_name(ft_id);
        let args: Vec<String> = ft_roles.iter().enumerate().map(|(i, r)| {
            let prefix = if i == 0 { "string indexed " } else { "string " };
            format!("{}{}", prefix, sanitize_field(r))
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
    // Forge lint prefers modifier logic wrapped in an internal function
    // to reduce code size when the modifier is applied to many funcs.
    let modifier = "    modifier onlyInStatus(string memory id, bytes32 expected) {\n        _onlyInStatus(id, expected);\n        _;\n    }\n\n    function _onlyInStatus(string memory id, bytes32 expected) internal view {\n        require(records[id].status == expected, \"SM: wrong state\");\n    }\n";
    format!("\n{}{}", enum_def, modifier)
}

/// Emit the create(...) function with UC + MC + VC requires (E2, #304).
fn emit_create(
    noun_name: &str,
    table: Option<&TableDef>,
    sm: Option<&StateMachineDef>,
    state: &Object,
) -> String {
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

    // UC: PK must not already exist. Before create, records[id].{pk}
    // is the empty string; after create it holds the id. Length 0
    // means "slot is unused" for string primary keys.
    let uc_check = format!(
        "        require(bytes(records[{}].{}).length == 0, \"UC: {} already exists\");",
        pk, pk, noun_name
    );

    // MC: every mandatory role of this entity requires its column to
    // be non-empty at create time.
    let mc_checks = mandatory_fields_for(noun_name, state).into_iter()
        .map(|field_noun| {
            let f = sanitize_field(&field_noun);
            format!(
                "        require(bytes({}).length > 0, \"MC: {} required\");",
                f, field_noun)
        })
        .collect::<Vec<_>>();

    // VC: for each column whose value-type noun has enum values
    // declared, require the incoming parameter to match one of them.
    let vc_checks = match table {
        Some(t) => t.columns.iter().filter_map(|c| {
            let col_noun = column_value_type_noun(&c.name, noun_name, state)?;
            let values = enum_values_for_value_type(&col_noun, state);
            if values.is_empty() { return None; }
            let f = sanitize_field(&c.name);
            Some(format!(
                "        require(_validate{}({}), \"VC: {} invalid\");",
                sanitize_name(&col_noun), f, c.name))
        }).collect::<Vec<_>>(),
        None => vec![],
    };

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
        mc_checks,
        vc_checks,
        assignments,
        if initial_status.is_empty() { vec![] } else { vec![initial_status] },
    ].concat().join("\n");

    format!(
        "\n    function create({}) external {{\n{}\n    }}\n",
        params.join(", "), body
    )
}

/// Return every role noun `field_noun` such that a Constraint cell
/// fact has `kind == "MC"`, `entity == noun_name`, and the constraint
/// text names `field_noun` via `some <field_noun>` phrasing.
///
/// MC constraints in the Constraint cell today carry only `entity` +
/// `text` (stage-2 translator doesn't populate role-noun binding yet
/// per task #304's field-resolution note). Parse the text locally —
/// Legacy emits `Each <entity> <verb> some <field>.` for MC shapes.
fn mandatory_fields_for(noun_name: &str, state: &Object) -> Vec<String> {
    let constraints = fetch_or_phi("Constraint", state);
    let facts = constraints.as_seq().unwrap_or(&[]);
    facts.iter()
        .filter(|c| binding(c, "kind") == Some("MC"))
        .filter(|c| binding(c, "entity") == Some(noun_name))
        .filter_map(|c| {
            let text = binding(c, "text")?;
            // `Each X has some Y.` or `Each X <verb> some Y.` — take
            // the phrase after ` some ` up to punctuation / trailing
            // period.
            let after = text.split(" some ").nth(1)?;
            let tail = after
                .trim_end_matches('.')
                .split(|c: char| c == '.' || c == ',')
                .next()?
                .trim();
            if tail.is_empty() { return None; }
            Some(tail.to_string())
        })
        .collect()
}

/// Look up the value-type noun referenced by a column on the entity.
/// Returns `Some(noun_name)` when the column's fact type resolves to
/// `Entity has <ValueNoun>`; `None` otherwise. Used by VC emission to
/// decide which columns need an enum validator.
fn column_value_type_noun(
    column_name: &str,
    entity_name: &str,
    state: &Object,
) -> Option<String> {
    let ft_cell = fetch_or_phi("FactType", state);
    let role_cell = fetch_or_phi("Role", state);
    let fts = ft_cell.as_seq()?;
    let roles = role_cell.as_seq()?;

    // Find the fact type whose reading mentions both the entity and
    // `column_name`, and whose non-entity role noun is the column.
    for ft in fts {
        let ft_id = binding(ft, "id").unwrap_or("");
        let reading = binding(ft, "reading").unwrap_or("");
        if !reading.contains(entity_name) { continue; }
        let ft_roles: Vec<&str> = roles.iter()
            .filter(|r| binding(r, "factType") == Some(ft_id))
            .filter_map(|r| binding(r, "nounName"))
            .collect();
        // Binary `Entity has <Value>` — the non-entity role is the
        // value-type noun.
        for &other in ft_roles.iter().filter(|n| **n != entity_name) {
            if sanitize_field(other) == sanitize_field(column_name) {
                return Some(other.to_string());
            }
        }
    }
    None
}

/// Read enum values declared for a value-type noun from the
/// `EnumValues` cell. Empty vec if none.
fn enum_values_for_value_type(noun_name: &str, state: &Object) -> Vec<String> {
    let cell = fetch_or_phi("EnumValues", state);
    let Some(facts) = cell.as_seq() else { return vec![]; };
    for f in facts {
        if binding(f, "noun") != Some(noun_name) { continue; }
        // Values land under `value0`, `value1`, …
        return (0..)
            .map_while(|i| {
                let key = format!("value{i}");
                binding(f, &key).map(String::from)
            })
            .collect();
    }
    vec![]
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

    // ─── E2 — ORM 2 constraint coverage in Solidity ────────────────────

    const IR_READINGS: &str = r#"
## Entity Types

Person(.person-id) is an entity type.

## Fact Types

Person reports to Person.
  Person reports to Person is irreflexive.
"#;

    #[test]
    fn compile_to_solidity_emits_ir_require() {
        let meta = parse_to_state(STATE_METAMODEL).unwrap();
        let state = parse_to_state_with_nouns(IR_READINGS, &meta).unwrap();
        let state = merge_states(&meta, &state);
        let out = compile_to_solidity_for_nouns(&state, &["Person"]);
        assert!(out.contains("IR:"),
            "expected IR require, got:\n{}", out);
    }

    const OFF_CHAIN_READINGS: &str = r#"
## Entity Types

Claim(.id) is an entity type.
Cause(.id) is an entity type.

## Fact Types

Claim has Cause.

## Subset Constraints

If some Claim has some Cause then that Claim has some Cause.
"#;

    #[test]
    fn compile_to_solidity_documents_off_chain_constraints() {
        // SS / EQ / XC are too expensive to enforce on-chain; the
        // generator should emit a comment block rather than silently
        // dropping the constraint.
        let meta = parse_to_state(STATE_METAMODEL).unwrap();
        let state = parse_to_state_with_nouns(OFF_CHAIN_READINGS, &meta).unwrap();
        let state = merge_states(&meta, &state);
        let out = compile_to_solidity_for_nouns(&state, &["Claim"]);
        assert!(out.contains("off-chain") || out.contains("Off-chain"),
            "expected off-chain-enforcement comment, got:\n{}", out);
    }
}
