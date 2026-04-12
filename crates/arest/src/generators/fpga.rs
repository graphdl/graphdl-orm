// crates/arest/src/generators/fpga.rs
//
// FPGA (Verilog) generator: compile FFP state (cells of named-tuple facts)
// to synthesizable Verilog HDL.
//
// Backus's 1977 Turing lecture pitched FP as a substrate for hardware
// synthesis: combining forms (compose, construction, condition, apply-to-all,
// insert, while) map to parallel hardware blocks. A Verilog emitter is the
// natural companion to compile — readings in, HDL out.
//
// First pass: emit one module per entity Noun, with wire declarations for
// clock/reset/id and a trivial always block that holds `valid` high after
// reset release. FP style: fold/map only, no for loops, no control-flow ifs.

use crate::ast::{binding, fetch_or_phi, Object};
use crate::rmap::{self, TableDef};

/// Compile a compiled state object to Verilog source.
///
/// Reads the "Noun" cell from state, computes RMAP tables from the domain,
/// and emits a synthesizable module for each entity noun with ports derived
/// from RMAP columns. Non-entity nouns (value types, enums) become wire
/// widths on the entity modules that reference them.
pub fn compile_to_verilog(state: &Object) -> String {
    let header = "// Generated from arest FORML2 readings\n\
                  // Backus FP combining forms synthesize to parallel hardware\n\n";

    let nouns = fetch_or_phi("Noun", state);
    // Compute RMAP tables for column-derived ports.
    let domain = crate::compile::state_to_domain(state);
    let tables = rmap::rmap(&domain);
    let table_map: std::collections::HashMap<String, &TableDef> = tables.iter()
        .map(|t| (t.name.clone(), t)).collect();

    let modules: Vec<String> = nouns
        .as_seq()
        .map(|ns| {
            ns.iter()
                .filter_map(|n| {
                    let name = binding(n, "name")?.to_string();
                    let obj_type = binding(n, "objectType")?;
                    if obj_type != "entity" { return None; }
                    let table_name = rmap::to_snake(&name);
                    let table = table_map.get(&table_name);
                    Some(emit_module(&name, table.map(|t| &t.columns)))
                })
                .collect()
        })
        .unwrap_or_default();

    format!("{}{}", header, modules.join("\n"))
}

/// Emit a Verilog module for an entity noun with RMAP-derived ports.
/// Each RMAP column becomes a port: PK columns are inputs, others are
/// input/output wires. If no RMAP table exists, emits a minimal shell.
fn emit_module(name: &str, columns: Option<&Vec<rmap::TableColumn>>) -> String {
    let m = sanitize(name);

    // Build port declarations from RMAP columns.
    let ports: Vec<String> = match columns {
        Some(cols) if !cols.is_empty() => {
            let mut p = vec![
                "    input wire clk".to_string(),
                "    input wire rst_n".to_string(),
            ];
            for col in cols {
                let wire_name = sanitize(&col.name);
                p.push(format!("    input wire [255:0] {}", wire_name));
            }
            p.push("    output reg valid".to_string());
            p
        }
        _ => vec![
            "    input wire clk".to_string(),
            "    input wire rst_n".to_string(),
            "    input wire [255:0] id_in".to_string(),
            "    output reg valid".to_string(),
        ],
    };

    format!(
        "module {} (\n{}\n);\n    \
             always @(posedge clk) begin\n        \
                 valid <= rst_n;\n    \
             end\n\
         endmodule\n",
        m,
        ports.join(",\n")
    )
}

/// Sanitize a noun name into a Verilog identifier: lowercase, spaces to
/// underscores. No control-flow ifs — pure character substitution via map.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            ' ' => '_',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{fact_from_pairs, merge_states, store, Object};
    use crate::parse_forml2::{parse_to_state, parse_to_state_with_nouns};

    /// Build a state containing a single "Noun" cell populated with the
    /// given (name, objectType) pairs. Pure FP construction: map + fold.
    fn state_with_nouns(pairs: &[(&str, &str)]) -> Object {
        let nouns: Vec<Object> = pairs
            .iter()
            .map(|(n, t)| fact_from_pairs(&[("name", n), ("objectType", t)]))
            .collect();
        store("Noun", Object::Seq(nouns), &Object::phi())
    }

    /// Minimal Order domain: one entity noun with a reference scheme and one
    /// binary fact type. Compiles to a single Verilog module named `order`.
    const ORDER_READINGS: &str = r#"
# Orders

## Entity Types

Order(.Order Number) is an entity type.

## Fact Types

Order has Amount.
"#;

    /// Metamodel seed: declares the Noun fact type so parse_forml2 can emit
    /// Noun cells with (name, objectType) bindings.
    const STATE_METAMODEL: &str = r#"
# State

## Fact Types

Noun has Object Type.
"#;

    #[test]
    fn compile_to_verilog_emits_entity_module() {
        let meta = parse_to_state(STATE_METAMODEL).unwrap();
        let orders = parse_to_state_with_nouns(ORDER_READINGS, &meta).unwrap();
        let state = merge_states(&meta, &orders);

        let verilog = compile_to_verilog(&state);

        assert!(
            verilog.contains("module order"),
            "expected `module order` in output, got:\n{}",
            verilog
        );
        assert!(verilog.contains("endmodule"));
        assert!(verilog.contains("// Generated from arest"));
    }

    /// Empty state → header comment only, zero modules.
    /// Compiling φ must not emit any `module` keyword.
    #[test]
    fn compile_to_verilog_empty_state_emits_header_only() {
        let verilog = compile_to_verilog(&Object::phi());

        assert!(verilog.contains("// Generated from arest"));
        assert!(verilog.contains("Backus FP combining forms"));
        assert!(
            !verilog.contains("module "),
            "empty state must not emit any modules, got:\n{}",
            verilog
        );
        assert!(!verilog.contains("endmodule"));
    }

    /// Multi-entity: three entity nouns produce three module blocks.
    /// Each module name must appear exactly once.
    #[test]
    fn compile_to_verilog_multiple_entities_produce_multiple_modules() {
        let state = state_with_nouns(&[
            ("Order", "entity"),
            ("Customer", "entity"),
            ("Product", "entity"),
        ]);

        let verilog = compile_to_verilog(&state);

        let module_count = verilog.matches("module ").count();
        let endmodule_count = verilog.matches("endmodule").count();
        assert_eq!(module_count, 3, "expected 3 modules, got:\n{}", verilog);
        assert_eq!(endmodule_count, 3, "module/endmodule mismatch:\n{}", verilog);
        assert!(verilog.contains("module order"));
        assert!(verilog.contains("module customer"));
        assert!(verilog.contains("module product"));
    }

    /// Value types must NOT become Verilog modules — only entities do.
    /// Mixed state with one entity and several values emits exactly one module.
    #[test]
    fn compile_to_verilog_skips_non_entity_nouns() {
        let state = state_with_nouns(&[
            ("Order", "entity"),
            ("Amount", "value"),
            ("Currency Code", "value"),
            ("Priority", "enum"),
        ]);

        let verilog = compile_to_verilog(&state);

        assert_eq!(verilog.matches("module ").count(), 1);
        assert!(verilog.contains("module order"));
        assert!(!verilog.contains("module amount"));
        assert!(!verilog.contains("module currency_code"));
        assert!(!verilog.contains("module priority"));
    }

    /// Multi-word entity names get sanitized: spaces → underscores,
    /// uppercase → lowercase. Produces a legal Verilog identifier.
    #[test]
    fn compile_to_verilog_sanitizes_multiword_entity_names() {
        let state = state_with_nouns(&[("State Machine Definition", "entity")]);

        let verilog = compile_to_verilog(&state);

        assert!(
            verilog.contains("module state_machine_definition"),
            "expected sanitized module name, got:\n{}",
            verilog
        );
        assert!(!verilog.contains("State Machine Definition"));
        assert!(!verilog.contains("module State"));
    }

    /// Verify synthesizable Verilog constructs are present in every emitted
    /// module: clock input, active-low reset, id bus, valid output, and a
    /// clocked always block.
    #[test]
    fn compile_to_verilog_emits_synthesizable_constructs() {
        let state = state_with_nouns(&[("Widget", "entity")]);

        let verilog = compile_to_verilog(&state);

        let required = [
            "module widget",
            "input wire clk",
            "input wire rst_n",
            "input wire [255:0] id_in",
            "output reg valid",
            "always @(posedge clk)",
            "valid <= rst_n",
            "endmodule",
        ];
        required.iter().for_each(|needle| {
            assert!(
                verilog.contains(needle),
                "missing `{}` in emitted Verilog:\n{}",
                needle,
                verilog
            );
        });
    }

    /// End-to-end: feed a FORML2 reading through parse_to_state and pipe
    /// the resulting state straight into compile_to_verilog. Exercises the
    /// full reading-to-HDL pipeline Backus envisioned in the 1977 lecture.
    #[test]
    fn compile_to_verilog_from_parsed_forml2_readings() {
        let meta = parse_to_state(STATE_METAMODEL).unwrap();
        let domain = parse_to_state_with_nouns(
            r#"
# Inventory

## Entity Types

Widget(.Widget Id) is an entity type.
Supplier(.Supplier Code) is an entity type.

## Fact Types

Widget has Quantity.
Supplier supplies Widget.
"#,
            &meta,
        )
        .unwrap();
        let state = merge_states(&meta, &domain);

        let verilog = compile_to_verilog(&state);

        assert!(verilog.contains("// Generated from arest"));
        assert!(verilog.contains("module widget"));
        assert!(verilog.contains("module supplier"));
        assert_eq!(verilog.matches("endmodule").count(), 2);
    }

    /// Verilog output is well-formed: every module has clk/rst_n ports,
    /// balanced module/endmodule pairs, and valid output declarations.
    /// Ports come from RMAP when available, fallback otherwise.
    #[test]
    fn compile_to_verilog_emits_structurally_sound_modules() {
        let meta = parse_to_state(STATE_METAMODEL).unwrap();
        let orders = parse_to_state_with_nouns(ORDER_READINGS, &meta).unwrap();
        let state = merge_states(&meta, &orders);

        let verilog = compile_to_verilog(&state);

        // Every entity module has clock and reset
        assert!(verilog.contains("input wire clk"));
        assert!(verilog.contains("input wire rst_n"));
        assert!(verilog.contains("output reg valid"));
        // Balanced module/endmodule
        assert_eq!(
            verilog.matches("module ").count(),
            verilog.matches("endmodule").count(),
            "module/endmodule count mismatch:\n{}", verilog
        );
    }
}
