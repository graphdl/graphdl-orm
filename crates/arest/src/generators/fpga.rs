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

    // Constraint check modules (UC / MC / FC). Each constraint in state
    // becomes one Verilog module with a `violation` output. v0 emits the
    // module skeleton — the actual hardware logic (UC: N² comparator
    // tree across the spanning fact type's stored rows; MC: sentinel-
    // value comparator on the reference-scheme column; FC: bounded
    // population counter) lands as a follow-up. The pipeline-level
    // contract — Constraint cell → per-constraint module → violation
    // output threaded into top — is what's exercised here.
    let (constraint_modules, constraint_names) = emit_constraint_modules(state);
    modules.extend(constraint_modules);

    // State-machine modules. Each SM declared in state gets a module
    // with (clk, rst_n, event_code, status) ports and a case-dispatch
    // transition table keyed on (current status, event code).
    let (sm_modules, sm_specs) = emit_sm_modules(&domain.state_machines);
    modules.extend(sm_modules);

    let top = emit_top_module(&entities, &constraint_names, &sm_specs);

    format!("{}{}\n{}", header, modules.join("\n"), top)
}

/// Emit one synthesizable Verilog module per state machine in the
/// compiled domain. Each SM module takes (clk, rst_n, event_code)
/// and outputs `status` (a register wide enough to hold every declared
/// status code). On reset, status falls back to the initial status's
/// code; otherwise a two-level case dispatch on `(current_status,
/// event_code)` either advances to the destination status or holds.
///
/// Status codes are assigned by position in `StateMachineDef::statuses`
/// (compile-time deterministic per SM). Event codes are FNV-1a 32-bit
/// hashes of the event string, so external drivers encode events the
/// same way when pushing into `event_code`.
///
/// Unknown events from the current status are held (default arm) —
/// matching the AREST machine fold's "invalid events are no-ops"
/// semantic (paper §"machine fold", transition function's fallthrough
/// to `s`).
///
/// Returns (module_text, (name, status_width)) pairs so the top
/// emitter can size the status wire it declares for each SM.
fn emit_sm_modules(sms: &std::collections::HashMap<String, crate::types::StateMachineDef>) -> (Vec<String>, Vec<(String, usize)>) {
    let mut entries: Vec<(&String, &crate::types::StateMachineDef)> = sms.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut modules: Vec<String> = Vec::new();
    let mut specs: Vec<(String, usize)> = Vec::new();
    for (_, sm) in entries {
        if sm.statuses.is_empty() { continue; }
        let module_name = sanitize(&format!("sm_{}", sm.noun_name));
        let status_count = sm.statuses.len();
        // Width = ceil(log2(max(count, 2))). Needs at least 1 bit.
        let status_width = std::cmp::max(
            1,
            (status_count.max(2) as f64).log2().ceil() as usize,
        );
        let status_codes: std::collections::HashMap<&str, usize> = sm.statuses.iter()
            .enumerate().map(|(i, s)| (s.as_str(), i)).collect();
        let initial_code = status_codes.get(sm.statuses[0].as_str()).copied().unwrap_or(0);

        let mut m = String::new();
        m.push_str(&format!("module {module_name} (\n"));
        m.push_str("    input wire clk,\n");
        m.push_str("    input wire rst_n,\n");
        m.push_str("    input wire [31:0] event_code,\n");
        m.push_str(&format!("    output reg [{}:0] status\n", status_width.saturating_sub(1)));
        m.push_str(");\n");
        m.push_str("    // Status codes (position in SM definition):\n");
        for (i, s) in sm.statuses.iter().enumerate() {
            m.push_str(&format!("    //   {status_width}'d{i} = {s}\n"));
        }
        m.push_str("    always @(posedge clk) begin\n");
        m.push_str("        if (!rst_n) begin\n");
        m.push_str(&format!("            status <= {status_width}'d{initial_code};\n"));
        m.push_str("        end else begin\n");
        if sm.transitions.is_empty() {
            m.push_str("            status <= status;\n");
        } else {
            // Group transitions by from-status.
            let mut by_from: std::collections::BTreeMap<&str, Vec<&crate::types::TransitionDef>>
                = std::collections::BTreeMap::new();
            for t in &sm.transitions {
                by_from.entry(t.from.as_str()).or_default().push(t);
            }
            m.push_str("            case (status)\n");
            for (from, ts) in &by_from {
                let from_code = status_codes.get(from).copied().unwrap_or(0);
                m.push_str(&format!("                {status_width}'d{from_code}: case (event_code)\n"));
                for t in ts {
                    let to_code = status_codes.get(t.to.as_str()).copied().unwrap_or(0);
                    let event_code = fnv1a_32(&t.event);
                    m.push_str(&format!(
                        "                    32'd{event_code}: status <= {status_width}'d{to_code};  // {}\n",
                        t.event,
                    ));
                }
                m.push_str("                    default: status <= status;\n");
                m.push_str("                endcase\n");
            }
            m.push_str("                default: status <= status;\n");
            m.push_str("            endcase\n");
        }
        m.push_str("        end\n");
        m.push_str("    end\n");
        m.push_str("endmodule\n");
        modules.push(m);
        specs.push((module_name, status_width));
    }
    (modules, specs)
}

/// FNV-1a 32-bit hash. Used for event-code encoding in SM Verilog
/// modules so external drivers and the SM case statement agree on
/// the numeric code for every declared event name.
fn fnv1a_32(s: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Emit one stub Verilog module per alethic cardinality constraint
/// (UC / MC / FC) found in the `Constraint` cell. Each module has the
/// canonical port shape (`clk`, `rst_n`, `output reg violation`) so the
/// `top` module can instantiate it and AND-reduce its violation signal
/// into an aggregate `constraint_ok` line. Body is a placeholder
/// (`violation <= 1'b0`) — the comparator-tree / sentinel / counter
/// logic is the next deliverable on this path.
///
/// Other constraint kinds (SS, EQ, IR/AS/AT/SY/IT/TR/AC, VC) are skipped
/// here; they need their own emitters covering set-comparison, ring,
/// and value-domain shapes.
///
/// Returns (module_text, module_name) pairs so the top emitter can
/// instantiate each by name without re-parsing the constraint list.
fn emit_constraint_modules(state: &Object) -> (Vec<String>, Vec<String>) {
    let constraints = fetch_or_phi("Constraint", state);
    let mut modules: Vec<String> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    if let Some(cs) = constraints.as_seq() {
        for c in cs.iter() {
            let Some(id) = binding(c, "id") else { continue };
            let Some(kind) = binding(c, "kind") else { continue };
            if !matches!(kind, "UC" | "MC" | "FC") { continue; }
            let module_name = sanitize(&format!("constraint_{}_{}", kind.to_ascii_lowercase(), id));
            modules.push(format!(
                "module {name} (\n    \
                    input wire clk,\n    \
                    input wire rst_n,\n    \
                    output reg violation\n\
                );\n    \
                // {kind} stub — comparator/sentinel/counter logic is future work.\n    \
                always @(posedge clk) begin\n        \
                    violation <= 1'b0;\n    \
                end\n\
                endmodule\n",
                name = module_name,
                kind = kind,
            ));
            names.push(module_name);
        }
    }
    (modules, names)
}

/// Emit a `top` Verilog module that instantiates every entity module,
/// wires the shared clock / reset fan-out, ties per-entity column
/// inputs to zero, and AND-reduces the `valid` outputs into a single
/// `all_valid` system signal. Constraint check modules are
/// instantiated alongside; their `violation` outputs AND-reduce
/// (after inversion) into an aggregate `constraint_ok` signal so the
/// commit path can gate on a single line.
///
/// Tying entity inputs to `{N{1'b0}}` keeps the output synthesizable
/// as-is — a downstream integrator replaces those constants with real
/// drivers (memory ports, pipeline registers) when wiring the module
/// into a larger design.
///
/// Returns an empty string if no entities AND no constraints are
/// present. With at least one entity OR constraint, `top` is emitted
/// with the relevant ports and instantiations.
fn emit_top_module(
    entities: &[(String, Vec<String>)],
    constraints: &[String],
    sms: &[(String, usize)],
) -> String {
    if entities.is_empty() && constraints.is_empty() && sms.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str("module top (\n");
    out.push_str("    input wire clk,\n");
    out.push_str("    input wire rst_n,\n");
    out.push_str("    output reg all_valid,\n");
    out.push_str("    output reg constraint_ok\n");
    out.push_str(");\n");
    // Per-entity valid wires.
    for (name, _) in entities {
        out.push_str(&format!("    wire {}_valid;\n", name));
    }
    // Per-constraint violation wires. `_v` suffix to keep them visually
    // distinct from entity `_valid` lines.
    for cname in constraints {
        out.push_str(&format!("    wire {}_v;\n", cname));
    }
    // Per-SM status wires, each sized to its declared status code width.
    for (sm_name, width) in sms {
        out.push_str(&format!("    wire [{}:0] {}_status;\n", width.saturating_sub(1), sm_name));
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
    // Instantiate each constraint module — clk/rst_n + violation out.
    for cname in constraints {
        out.push_str(&format!("    {} {}_inst (\n", cname, cname));
        out.push_str("        .clk(clk),\n");
        out.push_str("        .rst_n(rst_n),\n");
        out.push_str(&format!("        .violation({}_v)\n", cname));
        out.push_str("    );\n");
    }
    // Instantiate each SM — clk/rst_n + tied-low event_code + status out.
    // The integrator replaces `{32{1'b0}}` with the real event driver
    // when wiring events into the system.
    for (sm_name, _width) in sms {
        out.push_str(&format!("    {} {}_inst (\n", sm_name, sm_name));
        out.push_str("        .clk(clk),\n");
        out.push_str("        .rst_n(rst_n),\n");
        out.push_str("        .event_code({32{1'b0}}),\n");
        out.push_str(&format!("        .status({}_status)\n", sm_name));
        out.push_str("    );\n");
    }
    // AND-reduce valids after reset release. Empty-entity edge case:
    // emit `1'b1` as the identity so the expression stays well-formed.
    out.push_str("\n    always @(posedge clk) begin\n");
    out.push_str("        all_valid <= rst_n");
    if entities.is_empty() {
        out.push_str(" & 1'b1");
    } else {
        for (name, _) in entities {
            out.push_str(&format!(" & {}_valid", name));
        }
    }
    out.push_str(";\n");
    // constraint_ok = !any_violation. AND-reduce inverted violations.
    out.push_str("        constraint_ok <= rst_n");
    if constraints.is_empty() {
        out.push_str(" & 1'b1");
    } else {
        for cname in constraints {
            out.push_str(&format!(" & ~{}_v", cname));
        }
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
        // Top declares clk / rst_n inputs and all_valid + constraint_ok outputs.
        assert!(verilog.contains("module top (\n    input wire clk,\n    input wire rst_n,\n    output reg all_valid,\n    output reg constraint_ok\n);"),
            "top module header missing expected ports:\n{}", verilog);
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

    /// State with explicit Constraint cell entries: each UC/MC/FC
    /// constraint produces one stub Verilog module wired into top's
    /// `constraint_ok` aggregate. SS / EQ / ring / VC are intentionally
    /// not emitted yet — separate emitters per family.
    fn state_with_constraints(specs: &[(&str, &str)]) -> Object {
        let constraints: Vec<Object> = specs.iter().map(|(id, kind)| {
            fact_from_pairs(&[
                ("id", id),
                ("kind", kind),
                ("modality", "alethic"),
                ("text", "test constraint"),
            ])
        }).collect();
        store("Constraint", Object::Seq(constraints.into()), &Object::phi())
    }

    #[test]
    fn constraint_modules_emitted_per_alethic_cardinality_kind() {
        let state = state_with_constraints(&[
            ("c1", "UC"),
            ("c2", "MC"),
            ("c3", "FC"),
            ("c4", "SS"),  // skipped — set-comparison emitter is future work
            ("c5", "IR"),  // skipped — ring emitter is future work
        ]);
        let verilog = compile_to_verilog(&state);

        assert!(verilog.contains("module constraint_uc_c1"),
            "expected UC stub module:\n{}", verilog);
        assert!(verilog.contains("module constraint_mc_c2"),
            "expected MC stub module:\n{}", verilog);
        assert!(verilog.contains("module constraint_fc_c3"),
            "expected FC stub module:\n{}", verilog);
        // SS and IR are NOT emitted yet — separate task per family.
        assert!(!verilog.contains("constraint_ss_c4"),
            "SS not yet supported, should not emit");
        assert!(!verilog.contains("constraint_ir_c5"),
            "ring not yet supported, should not emit");
    }

    #[test]
    fn constraint_modules_have_canonical_port_shape() {
        let state = state_with_constraints(&[("c1", "UC")]);
        let verilog = compile_to_verilog(&state);
        assert!(verilog.contains("input wire clk"));
        assert!(verilog.contains("input wire rst_n"));
        assert!(verilog.contains("output reg violation"));
    }

    #[test]
    fn top_aggregates_constraint_violations_into_constraint_ok() {
        let state = state_with_constraints(&[("c_a", "UC"), ("c_b", "MC")]);
        let verilog = compile_to_verilog(&state);
        // top has constraint wires + instantiations + AND-reduction of
        // inverted violations.
        assert!(verilog.contains("wire constraint_uc_c_a_v;"));
        assert!(verilog.contains("wire constraint_mc_c_b_v;"));
        assert!(verilog.contains("constraint_uc_c_a constraint_uc_c_a_inst ("));
        assert!(verilog.contains("constraint_ok <= rst_n"));
        assert!(verilog.contains("& ~constraint_uc_c_a_v"));
        assert!(verilog.contains("& ~constraint_mc_c_b_v"));
    }

    /// Constraint-only state (no entities) must still emit a top with
    /// constraint plumbing — the all_valid AND-reduce uses 1'b1 as
    /// identity to keep the expression well-formed.
    #[test]
    fn constraint_only_state_emits_top_without_entities() {
        let state = state_with_constraints(&[("c1", "UC")]);
        let verilog = compile_to_verilog(&state);
        assert!(verilog.contains("module top ("),
            "top must be emitted when constraints exist:\n{}", verilog);
        assert!(verilog.contains("all_valid <= rst_n & 1'b1"),
            "no entities → all_valid identity 1'b1:\n{}", verilog);
        assert!(verilog.contains("constraint_ok <= rst_n & ~constraint_uc_c1_v"));
    }

    #[test]
    fn sm_module_emits_case_dispatch_with_status_codes() {
        use crate::types::{StateMachineDef, TransitionDef};
        let mut sms = std::collections::HashMap::new();
        sms.insert("Order".to_string(), StateMachineDef {
            noun_name: "Order".to_string(),
            statuses: vec!["Draft".to_string(), "Placed".to_string(), "Shipped".to_string()],
            transitions: vec![
                TransitionDef { from: "Draft".to_string(), to: "Placed".to_string(), event: "place".to_string(), guard: None },
                TransitionDef { from: "Placed".to_string(), to: "Shipped".to_string(), event: "ship".to_string(), guard: None },
            ],
        });
        let (modules, specs) = emit_sm_modules(&sms);
        assert_eq!(modules.len(), 1);
        assert_eq!(specs[0].0, "sm_order");
        assert_eq!(specs[0].1, 2, "3 statuses → 2-bit width");

        let m = &modules[0];
        assert!(m.contains("module sm_order ("), "module header:\n{}", m);
        assert!(m.contains("input wire [31:0] event_code"));
        assert!(m.contains("output reg [1:0] status"));
        // Status code comments
        assert!(m.contains("2'd0 = Draft"));
        assert!(m.contains("2'd1 = Placed"));
        assert!(m.contains("2'd2 = Shipped"));
        // Transition case dispatch
        assert!(m.contains("2'd0: case (event_code)"),
            "missing from-Draft case:\n{}", m);
        assert!(m.contains("2'd1: case (event_code)"),
            "missing from-Placed case:\n{}", m);
        // FNV-1a hashes for specific events encode correctly
        let place_hash = fnv1a_32("place");
        let ship_hash = fnv1a_32("ship");
        assert!(m.contains(&format!("32'd{place_hash}: status <= 2'd1")),
            "place → Placed transition missing:\n{}", m);
        assert!(m.contains(&format!("32'd{ship_hash}: status <= 2'd2")),
            "ship → Shipped transition missing:\n{}", m);
        // Reset holds initial
        assert!(m.contains("status <= 2'd0;"), "reset to initial missing:\n{}", m);
    }

    #[test]
    fn sm_module_with_two_statuses_uses_1bit_width() {
        use crate::types::{StateMachineDef, TransitionDef};
        let mut sms = std::collections::HashMap::new();
        sms.insert("Door".to_string(), StateMachineDef {
            noun_name: "Door".to_string(),
            statuses: vec!["Closed".to_string(), "Open".to_string()],
            transitions: vec![
                TransitionDef { from: "Closed".to_string(), to: "Open".to_string(), event: "open".to_string(), guard: None },
            ],
        });
        let (_modules, specs) = emit_sm_modules(&sms);
        assert_eq!(specs[0].1, 1, "2 statuses → 1-bit width");
    }

    #[test]
    fn sm_module_empty_sms_map_produces_nothing() {
        let (modules, specs) = emit_sm_modules(&std::collections::HashMap::new());
        assert!(modules.is_empty());
        assert!(specs.is_empty());
    }

    #[test]
    fn sm_module_without_transitions_holds_initial() {
        use crate::types::StateMachineDef;
        let mut sms = std::collections::HashMap::new();
        sms.insert("Frozen".to_string(), StateMachineDef {
            noun_name: "Frozen".to_string(),
            statuses: vec!["Only".to_string()],
            transitions: vec![],
        });
        let (modules, _specs) = emit_sm_modules(&sms);
        // Transition table absent → always holds status.
        assert!(modules[0].contains("status <= status;"),
            "no-transition SM must hold status:\n{}", modules[0]);
        assert!(!modules[0].contains("case (status)"));
    }

    #[test]
    fn fnv1a_32_is_stable_for_common_events() {
        // Regression: the hash encoding is part of the SM ABI — events
        // drive the FPGA at the same codes the emitter assigns. These
        // constants pin the exact FNV-1a 32-bit output for strings the
        // SM emitter sees in the tutor domain.
        assert_eq!(fnv1a_32("place"), 0xc8d632fc);
        assert_eq!(fnv1a_32("ship"), 0xac56f17f);
        assert_eq!(fnv1a_32(""), 0x811c9dc5);
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
