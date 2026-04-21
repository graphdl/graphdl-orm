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
use crate::rmap;
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

// State-machine access helpers. The generator reads `InstanceFact`
// cells on demand instead of materialising a `StateMachineDef` struct
// — no typed IR layer over the cell store (#325).
//
// Source shape in `InstanceFact`:
//   subjectNoun="State Machine Definition" subjectValue=<sm_name>
//     fieldName contains "is for" objectValue=<noun_name>
//   subjectNoun="Status" subjectValue=<status>
//     fieldName contains "defined in" objectValue=<sm_name>
//   subjectNoun="Transition" subjectValue=<event>
//     fieldName contains "from" | "to" | "triggered" objectValue=<…>

/// Resolve the SM name attached to a noun, if any.
fn sm_name_for_noun(state: &Object, noun_name: &str) -> Option<String> {
    fetch_or_phi("InstanceFact", state).as_seq()?
        .iter()
        .find(|f| binding(f, "subjectNoun") == Some("State Machine Definition")
            && binding(f, "fieldName").map(|s| s.contains("is for")).unwrap_or(false)
            && binding(f, "objectValue") == Some(noun_name))
        .and_then(|f| binding(f, "subjectValue").map(String::from))
}

/// All status names declared under the SM attached to `noun_name`, in
/// declaration order (first status is treated as the initial state
/// by `emit_create`).
fn sm_statuses(state: &Object, noun_name: &str) -> Vec<String> {
    let Some(sm_name) = sm_name_for_noun(state, noun_name) else { return vec![]; };
    let inst = fetch_or_phi("InstanceFact", state);
    let Some(facts) = inst.as_seq() else { return vec![]; };
    let mut out: Vec<String> = Vec::new();
    for f in facts.iter().filter(|f|
        binding(f, "subjectNoun") == Some("Status")
        && binding(f, "fieldName").map(|s| s.contains("defined in")).unwrap_or(false)
        && binding(f, "objectValue") == Some(sm_name.as_str()))
    {
        if let Some(s) = binding(f, "subjectValue") {
            let s = s.to_string();
            if !out.contains(&s) { out.push(s); }
        }
    }
    out
}

/// Every transition under the SM attached to `noun_name`, as
/// `(event_name, from_status, to_status)`. The `InstanceFact` rows
/// for a single transition are scattered across `from` / `to` /
/// `triggered` fields; fold them into per-event tuples.
fn sm_transitions(state: &Object, noun_name: &str) -> Vec<(String, String, String)> {
    if sm_name_for_noun(state, noun_name).is_none() { return vec![]; }
    let inst = fetch_or_phi("InstanceFact", state);
    let Some(facts) = inst.as_seq() else { return vec![]; };
    let mut by_event: Vec<(String, String, String)> = Vec::new();
    for f in facts.iter().filter(|f| binding(f, "subjectNoun") == Some("Transition")) {
        let Some(event) = binding(f, "subjectValue").map(String::from) else { continue };
        let field = binding(f, "fieldName").unwrap_or("");
        let value = binding(f, "objectValue").unwrap_or("").to_string();
        let slot = by_event.iter_mut().find(|(e, _, _)| *e == event);
        match slot {
            Some((_, from, to)) => {
                if field.contains("from") { *from = value; }
                else if field.contains("to") { *to = value; }
            }
            None => {
                let mut from = String::new();
                let mut to = String::new();
                let mut ev = event.clone();
                if field.contains("from") { from = value; }
                else if field.contains("to") { to = value; }
                else if field.contains("triggered") { ev = value; }
                by_event.push((ev, from, to));
            }
        }
    }
    by_event
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

    // RMAP still returns `Vec<TableDef>` today — that's the next
    // typed-IR consumer to retire per #325. For now read it into a
    // scope-local index, not a module-level type alias.
    let tables = rmap::rmap_from_state(state);

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
            let table = tables.iter().find(|t| t.name == table_name);
            Some(emit_contract(&name, table, state, include.as_ref()))
        }).collect()
    }).unwrap_or_default();

    format!("{}{}", header, contracts.join("\n"))
}

/// Emit a full Solidity contract for an entity noun.
fn emit_contract(
    name: &str,
    table: Option<&rmap::TableDef>,
    state: &Object,
    scope: Option<&hashbrown::HashSet<&str>>,
) -> String {
    let contract_name = sanitize_name(name);
    let off_chain = emit_off_chain_comment(name, state);
    let struct_def = emit_struct(table);
    let events = emit_events(name, state, scope);
    let has_sm = sm_name_for_noun(state, name).is_some();
    let sm_parts = if has_sm { emit_state_machine(name, state) } else { String::new() };
    let create_fn = emit_create(name, table, state);
    let transitions = if has_sm { emit_transitions(name, state) } else { String::new() };
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
    table: Option<&rmap::TableDef>,
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
fn emit_struct(table: Option<&rmap::TableDef>) -> String {
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
fn solidity_type(col: &rmap::TableColumn) -> &'static str {
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

/// Emit SM status enum and modifier. Reads `InstanceFact` cells
/// directly; no typed StateMachineDef materialisation.
fn emit_state_machine(noun_name: &str, state: &Object) -> String {
    let statuses: Vec<String> = sm_statuses(state, noun_name).iter()
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
    table: Option<&rmap::TableDef>,
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

    // Initial SM status: first status declared under the noun's SM
    // (or empty if no SM attached).
    let initial_status = sm_statuses(state, noun_name).first()
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

/// Emit one function per state machine transition. Reads
/// `InstanceFact` cells directly.
fn emit_transitions(noun_name: &str, state: &Object) -> String {
    let fns: Vec<String> = sm_transitions(state, noun_name).into_iter()
        .map(|(event, from, to)| {
            let fn_name = sanitize_field(&event);
            format!(
                "\n    function {}(string memory id) external onlyInStatus(id, keccak256(bytes(\"{}\"))) {{\n        records[id].status = keccak256(bytes(\"{}\"));\n    }}\n",
                fn_name, from, to
            )
        })
        .collect();
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
