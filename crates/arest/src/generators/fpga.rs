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
///
/// A `top` module wires all entity modules together with shared clock /
/// reset and an AND-reduction of their `valid` outputs. The per-entity
/// column inputs are tied to zero in `top`; an external synthesis
/// driver replaces those constants with real wires when integrating
/// with storage (BRAM / LUT-RAM per Backus §14.3 cell layout). Emitting
/// `top` turns the output from a loose bag of modules into a single
/// buildable unit.
pub fn compile_to_verilog(state: &Object) -> String {
    let header = "// Generated from arest FORML2 readings\n\
                  // Backus FP combining forms synthesize to parallel hardware\n\n";

    let nouns = fetch_or_phi("Noun", state);
    // Compute RMAP tables for column-derived ports.
    let domain = crate::compile::state_to_domain(state);
    let tables = rmap::rmap(&domain);
    let table_map: std::collections::HashMap<String, &TableDef> = tables.iter()
        .map(|t| (t.name.clone(), t)).collect();

    // Single pass: build each entity module and capture its top-
    // module wiring spec at the same time. The RMAP key is
    // `to_snake(name)` (which inserts underscores at camelCase
    // boundaries); the Verilog identifier is `sanitize(name)` (pure
    // lowercase + spaces→underscores). Preserving that distinction
    // keeps `emit_module`'s behaviour unchanged.
    let mut modules: Vec<String> = Vec::new();
    let mut entities: Vec<(String, Vec<String>)> = Vec::new();
    if let Some(ns) = nouns.as_seq() {
        for n in ns.iter() {
            let Some(name_str) = binding(n, "name") else { continue };
            let Some(obj_type) = binding(n, "objectType") else { continue };
            if obj_type != "entity" { continue; }
            let name = name_str.to_string();
            let table = table_map.get(&rmap::to_snake(&name));
            let columns: Vec<String> = table
                .map(|t| t.columns.iter().map(|c| sanitize(&c.name)).collect())
                .unwrap_or_else(|| vec!["id_in".to_string()]);
            modules.push(emit_module(&name, table.map(|t| &t.columns)));
            entities.push((sanitize(&name), columns));
        }
    }

    let top = emit_top_module(&entities);

    format!("{}{}\n{}", header, modules.join("\n"), top)
}

/// Emit a `top` Verilog module that instantiates every entity module,
/// wires the shared clock / reset fan-out, ties per-entity column
/// inputs to zero, and AND-reduces the `valid` outputs into a single
/// `all_valid` system signal.
///
/// Tying inputs to `{N{1'b0}}` keeps the output synthesizable as-is —
/// a downstream integrator replaces those constants with real drivers
/// (memory ports, pipeline registers) when wiring the module into a
/// larger design.
///
/// Returns an empty string if no entities are present (so the
/// caller's `format!` produces just the header).
fn emit_top_module(entities: &[(String, Vec<String>)]) -> String {
    if entities.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str("module top (\n");
    out.push_str("    input wire clk,\n");
    out.push_str("    input wire rst_n,\n");
    out.push_str("    output reg all_valid\n");
    out.push_str(");\n");
    // Per-entity valid wires.
    for (name, _) in entities {
        out.push_str(&format!("    wire {}_valid;\n", name));
    }
    out.push('\n');
    // Instantiate each entity with clk/rst_n + zero inputs + valid out.
    for (name, cols) in entities {
        out.push_str(&format!("    {} {}_inst (\n", name, name));
        out.push_str("        .clk(clk),\n");
        out.push_str("        .rst_n(rst_n),\n");
        for col in cols {
            out.push_str(&format!("        .{}({{256{{1'b0}}}}),\n", col));
        }
        out.push_str(&format!("        .valid({}_valid)\n", name));
        out.push_str("    );\n");
    }
    // AND-reduce valids after reset release.
    out.push_str("\n    always @(posedge clk) begin\n");
    out.push_str("        all_valid <= rst_n");
    for (name, _) in entities {
        out.push_str(&format!(" & {}_valid", name));
    }
    out.push_str(";\n    end\n");
    out.push_str("endmodule\n");
    out
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
        store("Noun", Object::Seq(nouns.into()), &Object::phi())
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

    /// Multi-entity: three entity nouns produce three module blocks
    /// plus one `top` module that instantiates and AND-reduces them.
    /// Each module declaration must appear exactly once.
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
        // 3 entity modules + 1 top module = 4.
        assert_eq!(module_count, 4, "expected 4 module decls, got:\n{}", verilog);
        assert_eq!(endmodule_count, 4, "module/endmodule mismatch:\n{}", verilog);
        assert!(verilog.contains("module order"));
        assert!(verilog.contains("module customer"));
        assert!(verilog.contains("module product"));
        assert!(verilog.contains("module top"));
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

        // 1 entity module + 1 top module = 2.
        assert_eq!(verilog.matches("module ").count(), 2);
        assert!(verilog.contains("module order"));
        assert!(verilog.contains("module top"));
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
        assert!(verilog.contains("module top"));
        // 2 entity modules + 1 top module = 3 endmodule markers.
        assert_eq!(verilog.matches("endmodule").count(), 3);
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

    // ── Top-level module wiring ────────────────────────────────────

    /// Single-entity compile produces a `top` module that instantiates
    /// the entity with clk/rst_n fan-out and names the per-entity
    /// `valid` wire.
    #[test]
    fn top_module_instantiates_single_entity_with_clock_reset_fanout() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        assert!(verilog.contains("module top ("),
            "top module header missing:\n{}", verilog);
        // Top declares clk / rst_n inputs and all_valid output.
        assert!(verilog.contains("module top (\n    input wire clk,\n    input wire rst_n,\n    output reg all_valid\n);"));
        // Declares the widget_valid wire.
        assert!(verilog.contains("wire widget_valid;"));
        // Instantiates the widget module.
        assert!(verilog.contains("widget widget_inst ("),
            "top must instantiate widget:\n{}", verilog);
        // Wires clk / rst_n through.
        assert!(verilog.contains(".clk(clk)"));
        assert!(verilog.contains(".rst_n(rst_n)"));
        // The AND-reduction includes this entity's valid.
        assert!(verilog.contains("all_valid <= rst_n & widget_valid;"));
    }

    /// Multi-entity: top instantiates every entity, ANDs all their
    /// valids, and uses sanitized names consistently.
    #[test]
    fn top_module_and_reduces_valid_across_entities() {
        let state = state_with_nouns(&[
            ("Alpha", "entity"),
            ("Beta", "entity"),
            ("Gamma", "entity"),
        ]);
        let verilog = compile_to_verilog(&state);
        assert!(verilog.contains("module top ("));
        // All three valid wires declared.
        for name in ["alpha", "beta", "gamma"] {
            assert!(verilog.contains(&format!("wire {}_valid;", name)),
                "missing `wire {}_valid;` in:\n{}", name, verilog);
            assert!(verilog.contains(&format!("{} {}_inst (", name, name)),
                "missing instantiation for {}", name);
        }
        // AND-reduce includes each entity's valid.
        assert!(verilog.contains("all_valid <= rst_n & alpha_valid & beta_valid & gamma_valid;"),
            "AND-reduction shape wrong:\n{}", verilog);
    }

    /// Column inputs on instantiated entities are tied to zero in
    /// `top` — the default driver. A downstream integrator replaces
    /// these with real wires (memory ports, pipeline registers) when
    /// integrating with storage. Without the zero-ties, unconnected
    /// input ports would generate synthesis warnings.
    #[test]
    fn top_module_ties_entity_column_inputs_to_zero() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        // Widget has no RMAP table in this synthetic fixture, so the
        // default port list is [id_in]. The top must wire it to zero.
        assert!(verilog.contains(".id_in({256{1'b0}})"),
            "missing zero-tie for widget.id_in:\n{}", verilog);
    }

    /// Empty state: no entities to wire, so top must be elided. The
    /// output is just the header — nothing that looks like a module
    /// declaration at all.
    #[test]
    fn top_module_absent_on_empty_state() {
        let verilog = compile_to_verilog(&Object::phi());
        assert!(!verilog.contains("module top"),
            "top must be elided when no entities present:\n{}", verilog);
        assert_eq!(verilog.matches("module ").count(), 0);
    }

    /// Sanity: counts stay balanced even with the top module in the
    /// mix. An N-entity state produces N+1 `module` declarations and
    /// N+1 `endmodule`s.
    #[test]
    fn top_module_preserves_module_endmodule_balance() {
        let state = state_with_nouns(&[
            ("One", "entity"),
            ("Two", "entity"),
            ("Three", "entity"),
            ("Four", "entity"),
        ]);
        let verilog = compile_to_verilog(&state);
        let modules = verilog.matches("module ").count();
        let endmodules = verilog.matches("endmodule").count();
        assert_eq!(modules, 5, "4 entities + 1 top = 5 module decls");
        assert_eq!(endmodules, 5);
        assert_eq!(modules, endmodules);
    }
}
