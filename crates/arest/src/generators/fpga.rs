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

/// Compile a compiled state object to Verilog source.
///
/// Reads the "Noun" cell from state, filters to entity-typed nouns, and emits
/// a synthesizable module shell for each. Non-entity nouns (value types,
/// enums) become wire widths in a future pass.
pub fn compile_to_verilog(state: &Object) -> String {
    let header = "// Generated from graphdl-orm FORML2 readings\n\
                  // Backus FP combining forms synthesize to parallel hardware\n\n";

    let nouns = fetch_or_phi("Noun", state);
    let modules: Vec<String> = nouns
        .as_seq()
        .map(|ns| {
            ns.iter()
                .filter_map(|n| {
                    let name = binding(n, "name")?.to_string();
                    let obj_type = binding(n, "objectType")?;
                    (obj_type == "entity").then(|| emit_module(&name))
                })
                .collect()
        })
        .unwrap_or_default();

    format!("{}{}", header, modules.join("\n"))
}

/// Emit a single Verilog module shell for an entity noun.
/// Pure function of the noun name — no state lookups, no side effects.
fn emit_module(name: &str) -> String {
    let m = sanitize(name);
    format!(
        "module {} (\n    \
             input wire clk,\n    \
             input wire rst_n,\n    \
             input wire [255:0] id_in,\n    \
             output reg valid\n\
         );\n    \
             always @(posedge clk) begin\n        \
                 valid <= rst_n;\n    \
             end\n\
         endmodule\n",
        m
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
    use crate::ast::merge_states;
    use crate::parse_forml2::{parse_to_state, parse_to_state_with_nouns};

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
        assert!(verilog.contains("// Generated from graphdl-orm"));
    }
}
