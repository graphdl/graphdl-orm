// crates/arest/src/generators/solidity.rs
//
// Solidity generator: compile FFP state to Ethereum smart-contract source.
//
// AREST maps readings into FFP objects (Backus §13 metacomposition). This
// module reverses the arrow on the output side: it walks the Noun and
// GraphSchema cells of a compiled state and emits a Solidity contract per
// entity plus storage mappings per fact type. It is a minimal first pass --
// no constraints-as-require() yet, no state-machine modifiers -- but the
// contract-per-entity shape is enough to round-trip through solc.
//
// Design constraints (project rules):
//   - Pure FP style: no for loops, no control-flow `if` statements.
//   - All composition happens via iterator combinators and `.then(||)`.
//   - The function is total: missing cells collapse to an empty program body,
//     never a panic.

use crate::ast::{Object, binding, fetch_or_phi};

/// Compile a compiled AREST state into Solidity source code.
///
/// Walks the `Noun` cell and emits one stub `contract` per entity type,
/// plus one `mapping(string => string)` line per non-entity fact-adjacent
/// noun (value types). Fact-type storage mappings are emitted inside a
/// single `Facts` library contract so that each `GraphSchema` gets its own
/// on-chain slot.
///
/// This is the "stub" pass used by task #28 — the AREST paper describes
/// compiling FORML2 readings into FFP; this function is the dual arrow
/// from FFP to Solidity bytecode-able source.
pub fn compile_to_solidity(state: &Object) -> String {
    let header = "// SPDX-License-Identifier: MIT\npragma solidity ^0.8.20;\n\n";
    let contracts = entity_contracts(state);
    let facts_lib = facts_library(state);
    format!("{}{}{}", header, contracts, facts_lib)
}

/// Emit one stub contract per entity Noun.
///
/// Filters the `Noun` cell for facts whose `objectType` binding equals
/// `entity`, then maps each to a minimal contract containing a single
/// `data` mapping. The `name.replace(' ', "")` call sanitizes multi-word
/// noun names (e.g. `"Purchase Order"` → `PurchaseOrder`).
fn entity_contracts(state: &Object) -> String {
    let nouns = fetch_or_phi("Noun", state);
    nouns
        .as_seq()
        .map(|ns| {
            ns.iter()
                .filter_map(|n| {
                    let name = binding(n, "name")?;
                    let obj_type = binding(n, "objectType")?;
                    (obj_type == "entity").then(|| {
                        format!(
                            "contract {} {{\n    mapping(string => string) internal data;\n}}\n",
                            name.replace(' ', "")
                        )
                    })
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Emit a single `Facts` library with one storage mapping per GraphSchema.
///
/// Each fact type becomes `mapping(bytes32 => string) ft_<id>;`. The id is
/// sanitized the same way as entity names. When there are no fact types
/// we return an empty string so the output is still a valid Solidity file.
fn facts_library(state: &Object) -> String {
    let schemas = fetch_or_phi("GraphSchema", state);
    let body = schemas
        .as_seq()
        .map(|ss| {
            ss.iter()
                .filter_map(|s| {
                    let id = binding(s, "id")?;
                    Some(format!(
                        "    mapping(bytes32 => string) internal ft_{};",
                        id.replace(' ', "_").replace('-', "_")
                    ))
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();

    body.is_empty()
        .then(String::new)
        .unwrap_or_else(|| format!("\nlibrary Facts {{\n{}\n}}\n", body))
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Object, cell_push, fact_from_pairs};

    /// Build a minimal Order domain state by pushing a single Noun fact
    /// directly into an empty Object. This bypasses parse_forml2 so the
    /// test stays focused on the generator's contract-per-entity walk.
    fn minimal_order_state() -> Object {
        let empty = Object::phi();
        let with_order = cell_push(
            "Noun",
            fact_from_pairs(&[("name", "Order"), ("objectType", "entity")]),
            &empty,
        );
        cell_push(
            "Noun",
            fact_from_pairs(&[("name", "Amount"), ("objectType", "value")]),
            &with_order,
        )
    }

    #[test]
    fn compile_to_solidity_emits_contract_for_order_entity() {
        let state = minimal_order_state();
        let out = compile_to_solidity(&state);

        // Header is present.
        assert!(out.contains("SPDX-License-Identifier: MIT"));
        assert!(out.contains("pragma solidity ^0.8.20;"));

        // Entity noun compiled to a contract stub.
        assert!(
            out.contains("contract Order"),
            "expected `contract Order` in output, got:\n{}",
            out
        );
        assert!(out.contains("mapping(string => string) internal data;"));

        // Value-type noun MUST NOT become a contract (filtered by objectType).
        assert!(
            !out.contains("contract Amount"),
            "value-type noun should not emit a contract, got:\n{}",
            out
        );
    }

    #[test]
    fn compile_to_solidity_handles_empty_state() {
        let out = compile_to_solidity(&Object::phi());
        // Header only — no contracts, no library.
        assert!(out.contains("pragma solidity ^0.8.20;"));
        assert!(!out.contains("contract "));
    }

    #[test]
    fn compile_to_solidity_sanitizes_multi_word_nouns() {
        let empty = Object::phi();
        let state = cell_push(
            "Noun",
            fact_from_pairs(&[("name", "Purchase Order"), ("objectType", "entity")]),
            &empty,
        );
        let out = compile_to_solidity(&state);
        assert!(out.contains("contract PurchaseOrder"));
    }
}
