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
use crate::types::StateMachineDef;
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

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
    let tables = rmap::rmap_from_state(state);
    let table_map: hashbrown::HashMap<String, &TableDef> = tables.iter()
        .map(|t| (t.name.clone(), t)).collect();

    // Single pass: build each entity module and capture its top-
    // module wiring spec at the same time. The RMAP key is
    // `to_snake(name)` (which inserts underscores at camelCase
    // boundaries); the Verilog identifier is `sanitize(name)` (pure
    // lowercase + spaces→underscores). Preserving that distinction
    // keeps `emit_module`'s behaviour unchanged.
    // Entity specs carry (sanitised-name, [(col-name, col-width-bits)]) so
    // the top emitter can tie each per-column input to a matching-width
    // zero literal instead of the pre-#187 blanket {256{1'b0}} default.
    let mut modules: Vec<String> = Vec::new();
    let mut entities: Vec<(String, Vec<(String, usize)>)> = Vec::new();
    if let Some(ns) = nouns.as_seq() {
        for n in ns.iter() {
            let Some(name_str) = binding(n, "name") else { continue };
            let Some(obj_type) = binding(n, "objectType") else { continue };
            if obj_type != "entity" { continue; }
            let name = name_str.to_string();
            let table = table_map.get(&rmap::to_snake(&name));
            let columns: Vec<(String, usize)> = table
                .map(|t| t.columns.iter()
                    .map(|c| (sanitize(&c.name), verilog_width_for(&c.col_type)))
                    .collect())
                .unwrap_or_else(|| vec![("id_in".to_string(), 256)]);
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
    let (sm_modules, sm_specs) = emit_sm_modules(&state_machines_from_state(state));
    modules.extend(sm_modules);

    // Per-noun BRAM cell memory map (#166). One dual-port bank per
    // entity, width = sum of per-column Verilog widths from #187.
    // Emitted before the top module so top can instantiate with
    // matching-width address / data ports.
    let (bram_modules, bram_specs) = emit_bram_modules(&entities);
    modules.extend(bram_modules);

    // Audit-log ring buffer. Mirrors the CPU-side crate::ring::RingBuffer
    // (#188) in synthesizable form: fixed-depth BRAM with write-pointer
    // + wrap-around overflow flag. Emitted only when the state has any
    // entities / constraints / SMs — empty states produce just the
    // header line (preserving the "empty compile = empty output" rule).
    let has_any = !entities.is_empty() || !constraint_names.is_empty() || !sm_specs.is_empty();
    if has_any {
        modules.push(emit_audit_log_module());
        // Fact ingress / egress ports (#168). Streaming I/O using the
        // ASCII-atom-ID convention (#189): fact-type name is a fixed
        // 32-byte = 256-bit wire, payload is 256 bits. One module per
        // direction so the integrator wires each independently.
        modules.push(emit_fact_ingress_module());
        modules.push(emit_fact_egress_module());
        // WASM reducer loader / dispatcher stub (#169). Interface-only
        // module documenting the ports a future hardware WASM
        // interpreter will drive. Body is empty — dispatch logic is
        // post-1.0 work (1-2 weeks of HDL + verification). Emitting
        // the stub keeps the bundle structurally complete so `top`
        // can wire BRAM + host-import ports even before the real
        // interpreter lands.
        modules.push(emit_wasm_reducer_stub());
        // Boot-sequence FSM (#170). Sequences post-reset bring-up:
        // LOAD_ROM -> INIT_BRAM -> READY. Back-pressures ingress
        // until BRAMs are warm.
        modules.push(emit_boot_fsm_module());
        // SYSTEM kernel (#154). The ρ-dispatch FSM that orchestrates
        // the other modules: command in → def lookup → reducer run →
        // audit → response out. This is SYSTEM(x, D) = ⟨o, D'⟩ in
        // synthesizable form; it's always emitted alongside the boot
        // FSM because the kernel's IDLE phase gates on boot_ready.
        modules.push(emit_system_kernel_module());
    }
    let _ = bram_specs;  // reserved for later top-level wiring work

    let top = emit_top_module(&entities, &constraint_names, &sm_specs);

    format!("{}{}\n{}", header, modules.join("\n"), top)
}

/// Emit the audit-log ring-buffer Verilog module. Depth and entry width
/// are exposed as `parameter`s so downstream integrators override the
/// defaults via instantiation-time values.
///
/// Mirrors the CPU-side `crate::ring::RingBuffer` (#188):
///   - append-only, bounded depth, oldest-out-on-overflow
///   - `overflow` flag latches high after the first wrap so the commit
///     path can react (SM transition, secondary-storage drain)
///
/// The storage vector `mem[]` is inferred as BRAM on most synthesis
/// tools because the access pattern is one-port read + one-port write
/// with a sequential address. Tools that need an explicit dual-port
/// BRAM swap can replace `mem` with a vendor macro.
fn emit_audit_log_module() -> String {
    r#"module audit_log #(
    parameter DEPTH = 1024,
    parameter ENTRY_WIDTH = 256
) (
    input wire clk,
    input wire rst_n,
    input wire wr_en,
    input wire [ENTRY_WIDTH-1:0] wr_data,
    input wire [$clog2(DEPTH)-1:0] rd_addr,
    output reg [ENTRY_WIDTH-1:0] rd_data,
    output reg [$clog2(DEPTH):0] count,
    output reg overflow
);
    // Ring storage — BRAM-inferred by synth tools.
    reg [ENTRY_WIDTH-1:0] mem [0:DEPTH-1];
    reg [$clog2(DEPTH)-1:0] head;

    always @(posedge clk) begin
        if (!rst_n) begin
            head     <= 0;
            count    <= 0;
            overflow <= 1'b0;
        end else if (wr_en) begin
            mem[head] <= wr_data;
            head      <= (head == DEPTH-1) ? 0 : head + 1;
            // Saturating count + overflow latch on first wrap.
            if (count < DEPTH) begin
                count <= count + 1;
            end else begin
                overflow <= 1'b1;
            end
        end
    end

    // Read port: registered for BRAM inference.
    always @(posedge clk) begin
        rd_data <= mem[rd_addr];
    end
endmodule
"#.to_string()
}

/// Emit one synthesizable Verilog module per state machine in the
/// compiled domain. Each SM module takes (clk, rst_n, event_code)
/// and outputs `status` (a register wide enough to hold every declared
/// status code). On reset, status falls back to the initial status's
/// code; otherwise a two-level case dispatch on `(current_status,
/// event_code)` either advances to the destination status or holds.
///
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
fn emit_sm_modules(sms: &hashbrown::HashMap<String, crate::types::StateMachineDef>) -> (Vec<String>, Vec<(String, usize)>) {
    let mut entries: Vec<(&String, &crate::types::StateMachineDef)> = sms.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut modules: Vec<String> = Vec::new();
    let mut specs: Vec<(String, usize)> = Vec::new();
    for (_, sm) in entries {
        if sm.statuses.is_empty() { continue; }
        let module_name = sanitize(&format!("sm_{}", sm.noun_name));
        let status_count = sm.statuses.len();
        // Width = ceil(log2(max(count, 2))). Needs at least 1 bit.
        let status_width = core::cmp::max(
            1,
            (status_count.max(2) as f64).log2().ceil() as usize,
        );
        let status_codes: hashbrown::HashMap<&str, usize> = sm.statuses.iter()
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
            let mut by_from: alloc::collections::BTreeMap<&str, Vec<&crate::types::TransitionDef>>
                = alloc::collections::BTreeMap::new();
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

/// Bundle produced by `compile_to_bundle` — everything a downstream
/// synthesis pipeline needs to build an AREST FPGA image.
#[derive(Debug, Clone)]
pub struct FpgaBundle {
    /// Generated Verilog source (entity modules + constraint/SM/
    /// audit/fact-io/boot_fsm + top).
    pub verilog: String,
    /// Freeze-image ROM bytes (metamodel baked via crate::freeze).
    /// Empty until a real metamodel state is supplied.
    pub rom: Vec<u8>,
    /// Manifest — JSON with bundle metadata: entity list, Verilog
    /// module list, ROM size, build timestamp placeholder.
    pub manifest: String,
}

/// Produce a complete FPGA deliverable bundle from compiled state:
/// Verilog source + freeze-image ROM + manifest. Downstream
/// integrators (Vivado / Yosys) consume the Verilog for synthesis and
/// burn the ROM bytes via their toolchain's BRAM-init hooks.
///
/// The manifest is plain JSON so it's readable by any downstream
/// toolchain without bringing in an AREST dependency.
pub fn compile_to_bundle(state: &Object) -> FpgaBundle {
    let verilog = compile_to_verilog(state);
    let rom = crate::freeze::freeze(state);
    let manifest = build_bundle_manifest(state, &verilog, &rom);
    FpgaBundle { verilog, rom, manifest }
}

/// Build the JSON manifest — minimal shape to keep downstream
/// consumers simple. No serde dep outside what the crate already uses.
fn build_bundle_manifest(state: &Object, verilog: &str, rom: &[u8]) -> String {
    let nouns = fetch_or_phi("Noun", state);
    let entity_names: Vec<String> = nouns.as_seq()
        .map(|ns| ns.iter()
            .filter_map(|n| {
                let obj_type = binding(n, "objectType")?;
                if obj_type != "entity" { return None; }
                binding(n, "name").map(|s| s.to_string())
            })
            .collect())
        .unwrap_or_default();
    let module_count = verilog.matches("module ").count();
    // Hand-rolled JSON to avoid pulling in serde_json in every bundle
    // caller. The manifest is small and well-known; the syntax is
    // deterministic for reproducible bundle hashing.
    let entities_json = entity_names.iter()
        .map(|n| format!("\"{}\"", n.replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{{\n  \
            \"arest_bundle_version\": 1,\n  \
            \"entities\": [{}],\n  \
            \"verilog_module_count\": {},\n  \
            \"rom_bytes\": {}\n\
        }}\n",
        entities_json, module_count, rom.len(),
    )
}

/// Compile state to Verilog AND append a self-contained `tb_top`
/// testbench module so the output can feed Icarus Verilog or Verilator
/// without external stimulus. Identical to `compile_to_verilog` for
/// non-simulation use.
///
/// The testbench:
///   - Generates a 100 MHz clock (period = 10 time-units).
///   - Asserts reset for 2 cycles, then releases.
///   - Runs for 20 clock cycles observing `all_valid` and
///     `constraint_ok`.
///   - Emits a `$display` line on every clock edge.
///   - Calls `$finish` at cycle 20 so `iverilog + vvp` terminate.
///
/// Usage:
///   iverilog -o sim.vvp generated.v
///   vvp sim.vvp
///
/// The testbench is a module by convention — adding it doesn't break
/// downstream synth tools (they reject `tb_*` modules by convention or
/// the integrator strips them at packaging time).
pub fn compile_to_verilog_with_testbench(state: &Object) -> String {
    let core = compile_to_verilog(state);
    // No testbench when there's nothing to simulate.
    if !core.contains("module top") {
        return core;
    }
    format!("{}\n{}", core, emit_testbench_module())
}

/// The canonical testbench wrapping the emitted `top` module.
fn emit_testbench_module() -> String {
    r#"// Simulation testbench — consumed by Icarus Verilog or Verilator.
// Drives clk + rst_n, observes top-level valid / constraint signals,
// exits after a bounded cycle count so the simulator terminates.
module tb_top;
    reg clk;
    reg rst_n;
    wire all_valid;
    wire constraint_ok;

    top dut (
        .clk(clk),
        .rst_n(rst_n),
        .all_valid(all_valid),
        .constraint_ok(constraint_ok)
    );

    // 100 MHz clock (period 10 time-units).
    initial clk = 1'b0;
    always #5 clk = ~clk;

    integer cycle;
    initial begin
        rst_n = 1'b0;
        cycle = 0;
        #20;          // hold reset for 2 full cycles
        rst_n = 1'b1;
    end

    always @(posedge clk) begin
        cycle = cycle + 1;
        $display("t=%0t cycle=%0d rst_n=%b all_valid=%b constraint_ok=%b",
                 $time, cycle, rst_n, all_valid, constraint_ok);
        if (cycle >= 20) $finish;
    end
endmodule
"#.to_string()
}

/// Emit the WASM reducer loader / dispatcher stub (#169).
///
/// Post-1.0 placeholder. A real implementation interprets the lowered
/// AREST-Func WASM emitted by `wasm_lower.rs` (#152) against on-chip
/// BRAM via the host-import bridge (#161) and the per-noun BRAM map
/// (#166). Because AREST's lowered WASM is structurally bounded
/// (every `Func::While` emits a max-iteration counter at compile
/// time), the dispatcher needs no gas metering — just decode +
/// dispatch + host-import routing.
///
/// Shipped interface pins the ABI future work must honour:
///
///   Inputs:
///     clk, rst_n                  — standard clock / active-low reset
///     start                       — caller raises for one cycle to
///                                    request evaluation of the
///                                    top-level `apply`
///     wasm_rom_data [7:0]         — one byte from ROM at
///                                    wasm_rom_addr; ROM is the
///                                    image baked via #171
///   Outputs:
///     wasm_rom_addr [15:0]        — byte-addressed ROM read port
///     cell_fetch_name / _data /   — host-import passthrough to #166
///       _req / _ack
///     cell_store_name / _data /   — same direction for writes
///       _we
///     result [255:0]              — atom encoding of top-level return
///     done                        — latches high when evaluation
///                                    completes; caller resets via
///                                    rst_n to dispatch the next op
///
/// Body: zero-driver stubs so the module is synthesizable but inert.
/// A real dispatcher replaces the always blocks with a decoded
/// microcontroller state machine.
fn emit_wasm_reducer_stub() -> String {
    r#"module wasm_reducer (
    input  wire clk,
    input  wire rst_n,
    input  wire start,
    // WASM ROM read port (byte-addressed; ROM is the bundle image #171).
    output reg  [15:0] wasm_rom_addr,
    input  wire [7:0]  wasm_rom_data,
    // Host-import: cell_fetch (read from per-noun BRAM #166).
    output reg  [255:0] cell_fetch_name,
    input  wire [255:0] cell_fetch_data,
    output reg          cell_fetch_req,
    input  wire         cell_fetch_ack,
    // Host-import: cell_store (write into per-noun BRAM).
    output reg  [255:0] cell_store_name,
    output reg  [255:0] cell_store_data,
    output reg          cell_store_we,
    // Evaluation result — atom encoding of the top-level return.
    output reg  [255:0] result,
    output reg          done
);
    // STUB (#169). Real dispatch logic is post-1.0:
    //   decoder  — maps WASM opcode bytes to dispatch-state encoding
    //   stack    — operand + control stack machine per WASM spec
    //   branches — routes `call` of host-imports to the fetch/store
    //              ports above
    //   halt     — on return-from-top-level, raise `done`
    //
    // Until then, the shell stays inert: every output holds its
    // reset value so synthesis leaves no unconnected-port warnings
    // and downstream integrators can wire the top file without the
    // real interpreter in place.
    always @(posedge clk) begin
        if (!rst_n) begin
            wasm_rom_addr    <= 16'd0;
            cell_fetch_name  <= {256{1'b0}};
            cell_fetch_req   <= 1'b0;
            cell_store_name  <= {256{1'b0}};
            cell_store_data  <= {256{1'b0}};
            cell_store_we    <= 1'b0;
            result           <= {256{1'b0}};
            done             <= 1'b0;
        end
        // Intentionally no else branch — stubs hold their reset state.
    end
endmodule
"#.to_string()
}

/// Emit the SYSTEM kernel — the top-level ρ-dispatch FSM (#154).
///
/// This is the hardware form of `SYSTEM(x, D) = ⟨o, D'⟩` (paper Eq. 1):
/// one incoming command x with key `k` and input `i`, state D in the
/// per-noun BRAMs (#166), and output o routed to the egress port
/// (#168). The FSM is the synthesizable form of the Rust `system_impl`
/// loop: lookup_def(k) → apply(def, i, D) → commit(new_D) → audit →
/// respond, ρ-dispatched through the def ROM address — not through
/// verb keywords.
///
/// State encoding is 3 bits so synthesis collapses to three FFs plus
/// the small combinatorial chain between phases. Phases:
///
///   IDLE     wait for boot_ready + cmd_valid; latch (k, i) and begin.
///   LOOKUP   drive def_lookup_name = k; stall until def_rom_valid.
///   EXECUTE  raise reducer_start and stall until reducer_done.
///   COMMIT   capture reducer_result as the output (BRAM writes the
///            reducer already did while it ran are the new D).
///   AUDIT    append the result entry into audit_log (#167).
///   RESPOND  drop result on egress for one cycle, return to IDLE.
///
/// Back-pressure: the module never advances past IDLE while boot_ready
/// is low, so it naturally waits for boot_fsm to finish LOAD_ROM /
/// INIT_BRAM. Downstream consumers of `result_valid` are responsible
/// for catching it on the single-cycle pulse in RESPOND.
fn emit_system_kernel_module() -> String {
    r#"module system_kernel (
    input  wire         clk,
    input  wire         rst_n,
    // Boot-FSM gate — stays low through LOAD_ROM / INIT_BRAM.
    input  wire         boot_ready,
    // Command ingress: one k/i pair per transaction. cmd_accepted
    // pulses high for one cycle when the kernel latches the command.
    input  wire         cmd_valid,
    output reg          cmd_accepted,
    input  wire [255:0] cmd_key,
    input  wire [255:0] cmd_input,
    // Def ROM lookup: k -> def-function handle. The ROM is bundle-
    // derived (#171) — integrators back it with BRAM / LUT-ROM.
    output reg  [255:0] def_lookup_name,
    output reg          def_lookup_req,
    input  wire [15:0]  def_rom_addr,
    input  wire         def_rom_valid,
    // WASM reducer handshake (#169). reducer_result is the top-level
    // ρ-application's atom-encoded return.
    output reg          reducer_start,
    input  wire [255:0] reducer_result,
    input  wire         reducer_done,
    // Result egress — one-cycle pulse on result_valid.
    output reg  [255:0] result_key,
    output reg  [255:0] result_value,
    output reg          result_valid,
    // Audit log write port (#167).
    output reg          audit_wr_en,
    output reg  [255:0] audit_entry,
    // Observable phase encoding (testbench hook).
    output reg  [2:0]   phase
);
    localparam [2:0] P_IDLE    = 3'd0;
    localparam [2:0] P_LOOKUP  = 3'd1;
    localparam [2:0] P_EXECUTE = 3'd2;
    localparam [2:0] P_COMMIT  = 3'd3;
    localparam [2:0] P_AUDIT   = 3'd4;
    localparam [2:0] P_RESPOND = 3'd5;

    // Hold the decoded (k, i) for the duration of the transaction so
    // the def ROM / reducer can pulse their request signals without
    // racing the external driver's cmd_valid.
    reg [255:0] latched_key;
    reg [255:0] latched_input;

    always @(posedge clk) begin
        if (!rst_n) begin
            phase           <= P_IDLE;
            cmd_accepted    <= 1'b0;
            def_lookup_name <= {256{1'b0}};
            def_lookup_req  <= 1'b0;
            reducer_start   <= 1'b0;
            result_key      <= {256{1'b0}};
            result_value    <= {256{1'b0}};
            result_valid    <= 1'b0;
            audit_wr_en     <= 1'b0;
            audit_entry     <= {256{1'b0}};
            latched_key     <= {256{1'b0}};
            latched_input   <= {256{1'b0}};
        end else begin
            case (phase)
                P_IDLE: begin
                    result_valid   <= 1'b0;
                    audit_wr_en    <= 1'b0;
                    cmd_accepted   <= 1'b0;
                    if (boot_ready && cmd_valid) begin
                        latched_key     <= cmd_key;
                        latched_input   <= cmd_input;
                        def_lookup_name <= cmd_key;
                        def_lookup_req  <= 1'b1;
                        cmd_accepted    <= 1'b1;
                        phase           <= P_LOOKUP;
                    end
                end
                P_LOOKUP: begin
                    def_lookup_req <= 1'b0;
                    cmd_accepted   <= 1'b0;
                    if (def_rom_valid) begin
                        // def_rom_addr is the compiled handle. In a fuller
                        // integration it drives reducer's code-pointer
                        // register; the atom-level contract here is that
                        // reducer_start triggers the interpreter pass.
                        reducer_start <= 1'b1;
                        phase         <= P_EXECUTE;
                    end
                end
                P_EXECUTE: begin
                    reducer_start <= 1'b0;
                    if (reducer_done) begin
                        phase <= P_COMMIT;
                    end
                end
                P_COMMIT: begin
                    // D writes happened inside the reducer via
                    // cell_store (#166) — here we only capture the
                    // output atom for egress + audit.
                    result_value <= reducer_result;
                    result_key   <= latched_key;
                    phase        <= P_AUDIT;
                end
                P_AUDIT: begin
                    audit_wr_en <= 1'b1;
                    audit_entry <= reducer_result;
                    phase       <= P_RESPOND;
                end
                P_RESPOND: begin
                    audit_wr_en  <= 1'b0;
                    result_valid <= 1'b1;
                    phase        <= P_IDLE;
                end
                default: phase <= P_IDLE;
            endcase
        end
    end
endmodule
"#.to_string()
}

/// Emit the top-level boot-sequence state machine (#170). Sequences
/// post-reset bring-up across every other FPGA-generator module:
///
///   RESET (rst_n low)
///     → LOAD_ROM (stream the freeze/thaw metamodel image from ROM)
///       → INIT_BRAM (zero / preload each per-noun BRAM bank)
///         → READY (reducer + constraint pipeline accepting events)
///
/// The FSM holds `ready` low through LOAD_ROM and INIT_BRAM so the
/// fact-ingress port (#168) back-pressures until BRAMs are warm. Once
/// READY, `ready` latches high for the life of the tenant.
///
/// Per-phase cycle counts are conservative defaults (16 cycles per
/// phase) — downstream integrators replace with real
/// ROM-size-derived values. The state encoding is 2 bits so the FSM
/// synthesizes down to two FFs + a few comparators.
fn emit_boot_fsm_module() -> String {
    r#"module boot_fsm (
    input wire clk,
    input wire rst_n,
    output reg ready,
    output reg [1:0] phase,
    // Phase progress counter — flips LOAD_ROM -> INIT_BRAM -> READY
    // after a conservative default cycle budget. Integrators override.
    input wire [7:0] phase_cycles
);
    // Phase codes — 2 bits so the synthesiser collapses to two FFs.
    localparam [1:0] PHASE_RESET    = 2'd0;
    localparam [1:0] PHASE_LOAD_ROM = 2'd1;
    localparam [1:0] PHASE_INIT_BRAM = 2'd2;
    localparam [1:0] PHASE_READY    = 2'd3;

    reg [7:0] counter;

    always @(posedge clk) begin
        if (!rst_n) begin
            phase   <= PHASE_RESET;
            counter <= 8'd0;
            ready   <= 1'b0;
        end else begin
            case (phase)
                PHASE_RESET: begin
                    phase   <= PHASE_LOAD_ROM;
                    counter <= 8'd0;
                end
                PHASE_LOAD_ROM: begin
                    if (counter >= phase_cycles) begin
                        phase   <= PHASE_INIT_BRAM;
                        counter <= 8'd0;
                    end else begin
                        counter <= counter + 1;
                    end
                end
                PHASE_INIT_BRAM: begin
                    if (counter >= phase_cycles) begin
                        phase   <= PHASE_READY;
                        ready   <= 1'b1;
                    end else begin
                        counter <= counter + 1;
                    end
                end
                PHASE_READY: begin
                    // Latched — stays high for the tenant's lifetime.
                    ready <= 1'b1;
                end
                default: phase <= PHASE_RESET;
            endcase
        end
    end
endmodule
"#.to_string()
}

/// Emit per-noun BRAM cell memory map (#166). One dual-port BRAM bank
/// per entity, row width = sum of the entity's column widths (from
/// #187), depth defaulted to 1024 rows (configurable per instance via
/// the `DEPTH` parameter).
///
/// Dual-port layout: port A is the read side (lookups / queries via
/// `cell_fetch`), port B is the write side (create / update). On
/// silicon the per-cell locks from #163 translate to per-bank
/// write-enable gating — disjoint-cell writes are trivially parallel.
///
/// Returns (module_text, (module_name, row_width_bits, depth)) so
/// downstream steps can address-decode + route.
fn emit_bram_modules(
    entities: &[(String, Vec<(String, usize)>)],
) -> (Vec<String>, Vec<(String, usize, usize)>) {
    let mut modules: Vec<String> = Vec::new();
    let mut specs: Vec<(String, usize, usize)> = Vec::new();
    for (name, cols) in entities {
        let row_width: usize = cols.iter().map(|(_, w)| *w).sum::<usize>().max(1);
        let depth: usize = 1024;
        let module_name = format!("{}_bram", name);
        let addr_bits = (depth as f64).log2().ceil() as usize;
        let addr_bits = addr_bits.max(1);
        let mut m = String::new();
        m.push_str(&format!(
            "module {module_name} #(\n    \
                parameter DEPTH = {depth},\n    \
                parameter ROW_WIDTH = {row_width}\n\
            ) (\n    \
                input  wire clk,\n    \
                input  wire rst_n,\n    \
                // Port A (read).\n    \
                input  wire [{addr_bits_minus_one}:0] addr_a,\n    \
                output reg  [ROW_WIDTH-1:0] rdata_a,\n    \
                // Port B (write).\n    \
                input  wire [{addr_bits_minus_one}:0] addr_b,\n    \
                input  wire [ROW_WIDTH-1:0] wdata_b,\n    \
                input  wire we_b,\n    \
                // Monotonic row counter — the 3NF cardinality.\n    \
                output reg  [{addr_bits}:0] row_count\n\
            );\n    \
                reg [ROW_WIDTH-1:0] mem [0:DEPTH-1];\n\n    \
                // Port A: registered read (BRAM-inferrable).\n    \
                always @(posedge clk) begin\n        \
                    rdata_a <= mem[addr_a];\n    \
                end\n\n    \
                // Port B: conditional write + row counter.\n    \
                always @(posedge clk) begin\n        \
                    if (!rst_n) begin\n            \
                        row_count <= 0;\n        \
                    end else if (we_b) begin\n            \
                        mem[addr_b] <= wdata_b;\n            \
                        if (row_count < DEPTH) row_count <= row_count + 1;\n        \
                    end\n    \
                end\n\
            endmodule\n",
            module_name = module_name,
            depth = depth,
            row_width = row_width,
            addr_bits = addr_bits,
            addr_bits_minus_one = addr_bits.saturating_sub(1),
        ));
        modules.push(m);
        specs.push((module_name, row_width, depth));
    }
    (modules, specs)
}

/// Emit the fact-ingress Verilog module — the on-chip entry point for
/// external fact assertions (webhook → FPGA, peer-to-peer message
/// stream, Kafka topic fanned to fabric). Single-fact valid/accepted
/// handshake; the real commit path downstream drives `accepted` based
/// on constraint-check outputs.
///
/// Wire widths follow the ASCII-atom-ID convention (#189): fact-type
/// name is fixed-width 32 bytes = 256 bits. Payload is 256 bits to
/// match the existing entity-module column width.
fn emit_fact_ingress_module() -> String {
    r#"module fact_ingress (
    input wire clk,
    input wire rst_n,
    input wire valid_in,
    input wire [255:0] name,
    input wire [255:0] payload,
    output reg accepted
);
    // Stub commit path: latch accepted on valid_in after reset release.
    // Real hardware replaces this with constraint-gated commit logic
    // (see constraint_ok from top-level constraint aggregation).
    always @(posedge clk) begin
        if (!rst_n) begin
            accepted <= 1'b0;
        end else begin
            accepted <= valid_in;
        end
    end
endmodule
"#.to_string()
}

/// Emit the fact-egress Verilog module — the on-chip exit point for
/// fact streams leaving the fabric (audit sync, replicate-to-peer,
/// downstream SQL writer). Single-fact valid/ready handshake matching
/// AXI-Stream discipline.
fn emit_fact_egress_module() -> String {
    r#"module fact_egress (
    input wire clk,
    input wire rst_n,
    input wire ready_in,
    output reg valid_out,
    output reg [255:0] name,
    output reg [255:0] payload
);
    // Stub: holds valid_out low until a real derivation-output source
    // is wired. Integrators replace with the publish-side driver (for
    // example the audit_log read port for log streaming).
    always @(posedge clk) begin
        if (!rst_n) begin
            valid_out <= 1'b0;
            name      <= {256{1'b0}};
            payload   <= {256{1'b0}};
        end
    end
endmodule
"#.to_string()
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
    entities: &[(String, Vec<(String, usize)>)],
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
    // Per-column zero width matches the entity module's declared port
    // width (#187), so synthesis doesn't warn about width mismatches.
    for (name, cols) in entities {
        out.push_str(&format!("    {} {}_inst (\n", name, name));
        out.push_str("        .clk(clk),\n");
        out.push_str("        .rst_n(rst_n),\n");
        for (col, width) in cols {
            out.push_str(&format!("        .{}({{{}{{1'b0}}}}),\n", col, width));
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
    // Audit-log singleton. Write port tied-off to zero by default;
    // downstream integrators replace with real commit-path drivers.
    // Count width is $clog2(DEPTH)+1 for the saturating-count form,
    // so DEPTH=1024 → 11-bit count. rd_addr width matches DEPTH.
    out.push_str("    wire [10:0] audit_count;\n");
    out.push_str("    wire audit_overflow;\n");
    out.push_str("    wire [255:0] audit_rd_data;\n");
    out.push_str("    audit_log #(.DEPTH(1024), .ENTRY_WIDTH(256)) audit_log_inst (\n");
    out.push_str("        .clk(clk),\n");
    out.push_str("        .rst_n(rst_n),\n");
    out.push_str("        .wr_en(1'b0),\n");
    out.push_str("        .wr_data({256{1'b0}}),\n");
    out.push_str("        .rd_addr({10{1'b0}}),\n");
    out.push_str("        .rd_data(audit_rd_data),\n");
    out.push_str("        .count(audit_count),\n");
    out.push_str("        .overflow(audit_overflow)\n");
    out.push_str("    );\n");
    // Fact-ingress / egress ports. Ingress write-enabled off by
    // default; egress ready held high so downstream sinks don't stall.
    out.push_str("    wire ingress_accepted;\n");
    out.push_str("    fact_ingress fact_ingress_inst (\n");
    out.push_str("        .clk(clk),\n");
    out.push_str("        .rst_n(rst_n),\n");
    out.push_str("        .valid_in(1'b0),\n");
    out.push_str("        .name({256{1'b0}}),\n");
    out.push_str("        .payload({256{1'b0}}),\n");
    out.push_str("        .accepted(ingress_accepted)\n");
    out.push_str("    );\n");
    out.push_str("    wire egress_valid_out;\n");
    out.push_str("    wire [255:0] egress_name;\n");
    out.push_str("    wire [255:0] egress_payload;\n");
    out.push_str("    fact_egress fact_egress_inst (\n");
    out.push_str("        .clk(clk),\n");
    out.push_str("        .rst_n(rst_n),\n");
    out.push_str("        .ready_in(1'b1),\n");
    out.push_str("        .valid_out(egress_valid_out),\n");
    out.push_str("        .name(egress_name),\n");
    out.push_str("        .payload(egress_payload)\n");
    out.push_str("    );\n");
    // Boot FSM + SYSTEM kernel wiring (#154 + #170). The kernel's
    // IDLE phase depends on `boot_ready`, which the boot FSM latches
    // after LOAD_ROM + INIT_BRAM complete.
    out.push_str("    wire boot_ready_sig;\n");
    out.push_str("    wire [1:0] boot_phase_sig;\n");
    out.push_str("    boot_fsm boot_fsm_inst (\n");
    out.push_str("        .clk(clk),\n");
    out.push_str("        .rst_n(rst_n),\n");
    out.push_str("        .ready(boot_ready_sig),\n");
    out.push_str("        .phase(boot_phase_sig),\n");
    out.push_str("        .phase_cycles(8'd16)\n");
    out.push_str("    );\n");
    // SYSTEM kernel. All cmd_* / reducer_* / def_rom_* ports tied
    // off at top — an integrator wires a real command source, def
    // ROM, and WASM reducer when the bundle deploys.
    out.push_str("    wire kernel_cmd_accepted;\n");
    out.push_str("    wire [255:0] kernel_def_lookup_name;\n");
    out.push_str("    wire kernel_def_lookup_req;\n");
    out.push_str("    wire kernel_reducer_start;\n");
    out.push_str("    wire [255:0] kernel_result_key;\n");
    out.push_str("    wire [255:0] kernel_result_value;\n");
    out.push_str("    wire kernel_result_valid;\n");
    out.push_str("    wire kernel_audit_wr_en;\n");
    out.push_str("    wire [255:0] kernel_audit_entry;\n");
    out.push_str("    wire [2:0] kernel_phase;\n");
    out.push_str("    system_kernel system_kernel_inst (\n");
    out.push_str("        .clk(clk),\n");
    out.push_str("        .rst_n(rst_n),\n");
    out.push_str("        .boot_ready(boot_ready_sig),\n");
    out.push_str("        .cmd_valid(1'b0),\n");
    out.push_str("        .cmd_accepted(kernel_cmd_accepted),\n");
    out.push_str("        .cmd_key({256{1'b0}}),\n");
    out.push_str("        .cmd_input({256{1'b0}}),\n");
    out.push_str("        .def_lookup_name(kernel_def_lookup_name),\n");
    out.push_str("        .def_lookup_req(kernel_def_lookup_req),\n");
    out.push_str("        .def_rom_addr({16{1'b0}}),\n");
    out.push_str("        .def_rom_valid(1'b0),\n");
    out.push_str("        .reducer_start(kernel_reducer_start),\n");
    out.push_str("        .reducer_result({256{1'b0}}),\n");
    out.push_str("        .reducer_done(1'b0),\n");
    out.push_str("        .result_key(kernel_result_key),\n");
    out.push_str("        .result_value(kernel_result_value),\n");
    out.push_str("        .result_valid(kernel_result_valid),\n");
    out.push_str("        .audit_wr_en(kernel_audit_wr_en),\n");
    out.push_str("        .audit_entry(kernel_audit_entry),\n");
    out.push_str("        .phase(kernel_phase)\n");
    out.push_str("    );\n");
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
                // Per-column width from the RMAP type (#187) — INTEGER
                // → 32 bits, BIGINT → 64, TEXT → 256, etc. Replaces the
                // pre-#187 blanket 256-bit default.
                let width = verilog_width_for(&col.col_type);
                p.push(format!("    input wire [{}:0] {}", width.saturating_sub(1), wire_name));
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

/// Map a SQL column type to its Verilog bit width (#187 — typed row
/// shape on Seq). Drives both the entity module's per-column port
/// width and top's tied-zero instantiation.
///
/// Width choices follow the narrowest SQL family that round-trips the
/// value without truncation; callers that need wider columns supply
/// `VARCHAR(N)` / `CHAR(N)` forms and the mapper returns `8 * N`.
/// Unknown types fall back to 256 bits (the pre-#187 default) so the
/// generator stays backwards-compatible with any schema the SQL
/// emitter produces today.
fn verilog_width_for(col_type: &str) -> usize {
    let up = col_type.to_ascii_uppercase();
    // VARCHAR(N) / CHAR(N) → 8*N bits, capped at 256 so the silicon
    // footprint stays reasonable for long identifiers.
    if let Some(start) = up.find('(') {
        if up.starts_with("VARCHAR") || up.starts_with("CHAR") {
            let rest = &up[start + 1..];
            if let Some(end) = rest.find(')') {
                if let Ok(n) = rest[..end].trim().parse::<usize>() {
                    return core::cmp::min(8 * n, 256);
                }
            }
        }
    }
    match up.as_str() {
        "BOOLEAN" | "BOOL" => 1,
        "TINYINT" => 8,
        "SMALLINT" => 16,
        "INTEGER" | "INT" => 32,
        "BIGINT" => 64,
        "REAL" => 32,
        "DOUBLE" | "NUMERIC" | "DECIMAL" => 64,
        // TEXT / VARCHAR-without-length / unknown → 256 (backwards
        // compatible). Narrows as per-type schemas land.
        _ => 256,
    }
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
        // 3 entity + 3 bram + audit + ingress + egress + wasm_reducer + boot_fsm + top = 12.
        assert_eq!(module_count, 12, "expected 12 module decls, got:\n{}", verilog);
        assert_eq!(endmodule_count, 12, "module/endmodule mismatch:\n{}", verilog);
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

        // 1 entity + 1 bram + audit + ingress + egress + wasm_reducer + boot_fsm + top = 8.
        assert_eq!(verilog.matches("module ").count(), 8);
        assert!(verilog.contains("module order"));
        assert!(verilog.contains("module order_bram"));
        assert!(verilog.contains("module top"));
        assert!(verilog.contains("module audit_log"));
        assert!(verilog.contains("module fact_ingress"));
        assert!(verilog.contains("module fact_egress"));
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
        // 2 entity + 2 bram + audit + ingress + egress + wasm_reducer + boot_fsm + top = 10.
        assert_eq!(verilog.matches("endmodule").count(), 10);
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

    // ── Audit-log Verilog ring buffer (#167) ──

    // ── Counter domain end-to-end (#173) ──
    //
    // A single entity "Counter" with one integer "Count" column,
    // walked through the whole FPGA pipeline: parse readings →
    // compile_to_bundle → verify every family of emitted module
    // shows up in the Verilog + every pipeline artefact
    // (bundle.rom, manifest entries) lines up. Synthesizable
    // under Icarus Verilog / Verilator via the testbench from #172.

    #[test]
    fn counter_domain_produces_all_fpga_module_families() {
        let meta = parse_to_state(STATE_METAMODEL).unwrap();
        let counter_readings = r#"
# Counter

## Entity Types

Counter(.Counter Id) is an entity type.

## Fact Types

Counter has Count.
"#;
        let domain = parse_to_state_with_nouns(counter_readings, &meta).unwrap();
        let state = merge_states(&meta, &domain);

        let bundle = compile_to_bundle(&state);
        let v = &bundle.verilog;

        // Entity module for the Counter noun.
        assert!(v.contains("module counter ("),
            "counter entity module missing:\n{}", v);
        // Per-noun BRAM bank (#166).
        assert!(v.contains("module counter_bram "),
            "counter_bram module missing");
        // Audit log (#167).
        assert!(v.contains("module audit_log "),
            "audit_log module missing");
        // Fact ingress / egress (#168).
        assert!(v.contains("module fact_ingress ("));
        assert!(v.contains("module fact_egress ("));
        // Boot FSM (#170).
        assert!(v.contains("module boot_fsm ("));
        // Top aggregation (base + #164/#165 constraint + SM folds).
        assert!(v.contains("module top ("));

        // Bundle ROM carries the metamodel freeze image.
        assert!(bundle.rom.starts_with(b"AREST"),
            "freeze-image ROM magic must be present");

        // Manifest lists the entity.
        assert!(bundle.manifest.contains("\"Counter\""),
            "manifest entities must name Counter:\n{}", bundle.manifest);
    }

    #[test]
    fn counter_domain_with_testbench_is_self_simulating() {
        let meta = parse_to_state(STATE_METAMODEL).unwrap();
        let counter_readings = r#"
# Counter

## Entity Types

Counter(.Counter Id) is an entity type.

## Fact Types

Counter has Count.
"#;
        let domain = parse_to_state_with_nouns(counter_readings, &meta).unwrap();
        let state = merge_states(&meta, &domain);

        let verilog = compile_to_verilog_with_testbench(&state);
        // tb_top wraps top; iverilog + vvp terminate via $finish at
        // cycle 20 so the full Counter pipeline simulates without
        // external stimulus.
        assert!(verilog.contains("module tb_top"));
        assert!(verilog.contains("top dut ("));
        assert!(verilog.contains("$finish"));
    }

    // ── compile_to_bundle packaging (#171) ──

    #[test]
    fn bundle_packs_verilog_rom_and_manifest() {
        let state = state_with_nouns(&[("Widget", "entity"), ("Gadget", "entity")]);
        let bundle = compile_to_bundle(&state);
        assert!(bundle.verilog.contains("module widget"),
            "bundle.verilog must carry entity modules");
        assert!(!bundle.rom.is_empty(),
            "bundle.rom must contain the freeze-image bytes");
        assert!(bundle.rom.starts_with(b"AREST"),
            "rom must carry the freeze magic header");
        assert!(bundle.manifest.contains("\"arest_bundle_version\": 1"));
        assert!(bundle.manifest.contains("\"Widget\""));
        assert!(bundle.manifest.contains("\"Gadget\""));
    }

    #[test]
    fn bundle_manifest_reports_rom_size_and_module_count() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let bundle = compile_to_bundle(&state);
        // rom_bytes in manifest matches actual rom length.
        let expected = format!("\"rom_bytes\": {}", bundle.rom.len());
        assert!(bundle.manifest.contains(&expected),
            "manifest must report rom size ({}):\n{}", bundle.rom.len(), bundle.manifest);
        // module_count matches actual Verilog module count.
        let actual_count = bundle.verilog.matches("module ").count();
        let expected_mc = format!("\"verilog_module_count\": {}", actual_count);
        assert!(bundle.manifest.contains(&expected_mc));
    }

    #[test]
    fn bundle_empty_state_still_produces_valid_manifest() {
        let bundle = compile_to_bundle(&Object::phi());
        // No entities, no modules, but a valid manifest.
        assert!(bundle.manifest.contains("\"arest_bundle_version\": 1"));
        assert!(bundle.manifest.contains("\"entities\": []"));
        // ROM still has the magic header even for empty state.
        assert!(bundle.rom.starts_with(b"AREST"));
    }

    // ── WASM reducer stub (#169) ──

    #[test]
    fn wasm_reducer_stub_emitted_with_state() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        assert!(verilog.contains("module wasm_reducer "),
            "wasm_reducer stub must be emitted with state:\n{}", verilog);
    }

    #[test]
    fn wasm_reducer_stub_pins_the_host_import_abi() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        // ROM read port the future dispatcher consumes.
        assert!(verilog.contains("output reg  [15:0] wasm_rom_addr"));
        assert!(verilog.contains("input  wire [7:0]  wasm_rom_data"));
        // Host-import fetch / store passthrough to #166 BRAMs.
        assert!(verilog.contains("output reg  [255:0] cell_fetch_name"));
        assert!(verilog.contains("output reg  [255:0] cell_store_name"));
        assert!(verilog.contains("input  wire         cell_fetch_ack"));
        assert!(verilog.contains("output reg          cell_store_we"));
        // Top-level result + done latch.
        assert!(verilog.contains("output reg  [255:0] result"));
        assert!(verilog.contains("output reg          done"));
    }

    // ── Boot-sequence FSM (#170) ──

    #[test]
    fn boot_fsm_emitted_with_state() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        assert!(verilog.contains("module boot_fsm"),
            "boot_fsm module missing:\n{}", verilog);
    }

    #[test]
    fn boot_fsm_has_four_phase_state_machine() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        // Four canonical phases as 2-bit codes.
        assert!(verilog.contains("PHASE_RESET    = 2'd0"));
        assert!(verilog.contains("PHASE_LOAD_ROM = 2'd1"));
        assert!(verilog.contains("PHASE_INIT_BRAM = 2'd2"));
        assert!(verilog.contains("PHASE_READY    = 2'd3"));
    }

    #[test]
    fn boot_fsm_holds_ready_low_until_init_complete() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        // Reset clears ready; transition to READY latches it high.
        assert!(verilog.contains("ready   <= 1'b0"),
            "reset must clear ready:\n{}", verilog);
        assert!(verilog.contains("ready   <= 1'b1"));
        // READY phase is terminal — ready stays high.
        assert!(verilog.contains("PHASE_READY: begin"));
    }

    // ── SYSTEM kernel dispatch FSM (#154) ──

    #[test]
    fn system_kernel_module_emitted_alongside_boot_fsm() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        assert!(verilog.contains("module system_kernel"),
            "system_kernel must accompany the other FPGA modules:\n{}", verilog);
        // The kernel gates on boot_ready — integrity of the dependency.
        assert!(verilog.contains("boot_ready"));
    }

    #[test]
    fn system_kernel_has_six_phase_dispatch_fsm() {
        // IDLE → LOOKUP → EXECUTE → COMMIT → AUDIT → RESPOND → IDLE
        // — the ρ-dispatch loop. All six phases must land in the
        // encoding so SYSTEM(x, D) = ⟨o, D'⟩ runs as written.
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        for phase in ["P_IDLE", "P_LOOKUP", "P_EXECUTE", "P_COMMIT", "P_AUDIT", "P_RESPOND"] {
            assert!(verilog.contains(phase),
                "kernel must expose {} phase encoding:\n{}", phase, verilog);
        }
    }

    #[test]
    fn system_kernel_exposes_def_rom_and_reducer_handshake_ports() {
        // The kernel's two external dependencies are the def ROM
        // (key → compiled handle) and the WASM reducer (handle →
        // result). Both must be visible at the port boundary so a
        // downstream integrator can wire them without surgery.
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        // Def ROM side.
        assert!(verilog.contains("def_lookup_name"));
        assert!(verilog.contains("def_lookup_req"));
        assert!(verilog.contains("def_rom_addr"));
        assert!(verilog.contains("def_rom_valid"));
        // Reducer side.
        assert!(verilog.contains("reducer_start"));
        assert!(verilog.contains("reducer_result"));
        assert!(verilog.contains("reducer_done"));
        // Result egress.
        assert!(verilog.contains("result_valid"));
    }

    #[test]
    fn top_instantiates_system_kernel_gated_on_boot_ready() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        // The top wires boot_fsm's `ready` into system_kernel's
        // `boot_ready` — that's the entire Definition-2 gate.
        assert!(verilog.contains("system_kernel system_kernel_inst"),
            "top must instantiate the kernel:\n{}", verilog);
        assert!(verilog.contains(".boot_ready(boot_ready_sig)"),
            "kernel's boot_ready must be wired to boot_fsm.ready:\n{}", verilog);
    }

    // ── Per-noun BRAM cell memory map (#166) ──

    #[test]
    fn bram_module_emitted_per_entity() {
        let state = state_with_nouns(&[("Widget", "entity"), ("Gadget", "entity")]);
        let verilog = compile_to_verilog(&state);
        assert!(verilog.contains("module widget_bram"), "widget_bram module absent:\n{}", verilog);
        assert!(verilog.contains("module gadget_bram"), "gadget_bram module absent:\n{}", verilog);
    }

    #[test]
    fn bram_module_has_dual_port_layout() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        // Port A: read side.
        assert!(verilog.contains("input  wire [9:0] addr_a"));
        assert!(verilog.contains("output reg  [ROW_WIDTH-1:0] rdata_a"));
        // Port B: write side with write-enable.
        assert!(verilog.contains("input  wire [9:0] addr_b"));
        assert!(verilog.contains("input  wire [ROW_WIDTH-1:0] wdata_b"));
        assert!(verilog.contains("input  wire we_b"));
        // Row counter exposing the 3NF cardinality.
        assert!(verilog.contains("output reg  [10:0] row_count"));
    }

    #[test]
    fn bram_module_parameterises_depth_and_row_width() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        assert!(verilog.contains("parameter DEPTH = 1024"),
            "depth parameter missing:\n{}", verilog);
        // Without an RMAP table the noun falls back to the single-
        // id_in column of width 256.
        assert!(verilog.contains("parameter ROW_WIDTH = 256"));
    }

    #[test]
    fn bram_module_has_registered_read_and_conditional_write() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        // Port A read is a registered clocked assignment (BRAM-inferrable).
        assert!(verilog.contains("rdata_a <= mem[addr_a]"));
        // Port B write is guarded on we_b AND reset, with counter bump.
        assert!(verilog.contains("if (we_b)"));
        assert!(verilog.contains("mem[addr_b] <= wdata_b"));
        assert!(verilog.contains("row_count <= row_count + 1"));
        // Reset clears row_count to 0.
        assert!(verilog.contains("row_count <= 0"));
    }

    // ── Typed-row column widths (#187) ──

    #[test]
    fn verilog_width_for_sql_types() {
        // Narrow integer family
        assert_eq!(verilog_width_for("BOOLEAN"), 1);
        assert_eq!(verilog_width_for("BOOL"), 1);
        assert_eq!(verilog_width_for("TINYINT"), 8);
        assert_eq!(verilog_width_for("SMALLINT"), 16);
        assert_eq!(verilog_width_for("INTEGER"), 32);
        assert_eq!(verilog_width_for("INT"), 32);
        assert_eq!(verilog_width_for("BIGINT"), 64);
        // Floating / numeric
        assert_eq!(verilog_width_for("REAL"), 32);
        assert_eq!(verilog_width_for("DOUBLE"), 64);
        assert_eq!(verilog_width_for("NUMERIC"), 64);
        assert_eq!(verilog_width_for("DECIMAL"), 64);
        // Character types
        assert_eq!(verilog_width_for("VARCHAR(4)"), 32);
        assert_eq!(verilog_width_for("CHAR(8)"), 64);
        assert_eq!(verilog_width_for("VARCHAR(100)"), 256, "caps at 256");
        // Case insensitive
        assert_eq!(verilog_width_for("integer"), 32);
        assert_eq!(verilog_width_for("Integer"), 32);
        // Unknown / bare TEXT falls back to 256
        assert_eq!(verilog_width_for("TEXT"), 256);
        assert_eq!(verilog_width_for("VARCHAR"), 256);
        assert_eq!(verilog_width_for("JSON"), 256);
        assert_eq!(verilog_width_for(""), 256);
    }

    // ── Testbench harness (#172) ──

    #[test]
    fn testbench_appended_only_when_top_is_emitted() {
        let with_state = compile_to_verilog_with_testbench(&state_with_nouns(&[("W", "entity")]));
        assert!(with_state.contains("module tb_top"),
            "testbench must be present when state is non-empty");
        // Empty state emits only the header line — no top, no tb_top.
        let empty = compile_to_verilog_with_testbench(&Object::phi());
        assert!(!empty.contains("tb_top"),
            "empty state must not carry a testbench:\n{}", empty);
    }

    #[test]
    fn testbench_instantiates_top_as_dut() {
        let verilog = compile_to_verilog_with_testbench(&state_with_nouns(&[("Widget", "entity")]));
        assert!(verilog.contains("top dut ("),
            "testbench must instantiate top as dut:\n{}", verilog);
        assert!(verilog.contains(".all_valid(all_valid)"));
        assert!(verilog.contains(".constraint_ok(constraint_ok)"));
    }

    #[test]
    fn testbench_generates_clock_and_resets() {
        let verilog = compile_to_verilog_with_testbench(&state_with_nouns(&[("Widget", "entity")]));
        // Clock: toggled every 5 time-units (100 MHz with period 10).
        assert!(verilog.contains("always #5 clk = ~clk;"));
        // Reset held low, then raised after 20 time-units.
        assert!(verilog.contains("rst_n = 1'b0;"));
        assert!(verilog.contains("rst_n = 1'b1;"));
        // Bounded simulation: $finish after cycle count.
        assert!(verilog.contains("$finish"));
        assert!(verilog.contains("cycle >= 20"));
    }

    #[test]
    fn plain_compile_to_verilog_omits_testbench() {
        // The non-testbench entry point must not accidentally include
        // tb_top — synth tools reject unknown top-level modules.
        let verilog = compile_to_verilog(&state_with_nouns(&[("Widget", "entity")]));
        assert!(!verilog.contains("tb_top"),
            "compile_to_verilog must not emit testbench:\n{}", verilog);
    }

    // ── Fact ingress / egress ports (#168) ──

    #[test]
    fn fact_ingress_and_egress_emitted_with_state() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        assert!(verilog.contains("module fact_ingress"),
            "missing fact_ingress module:\n{}", verilog);
        assert!(verilog.contains("module fact_egress"),
            "missing fact_egress module:\n{}", verilog);
    }

    #[test]
    fn fact_ingress_uses_fixed_width_ascii_name_port() {
        // ASCII atom ID convention (#189): name is a fixed-width 256-bit
        // (32-byte) port, matching the atom_id_is_valid runtime guard.
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        assert!(verilog.contains("input wire [255:0] name"),
            "fact_ingress must expose a 256-bit name port:\n{}", verilog);
        assert!(verilog.contains("input wire valid_in"));
        assert!(verilog.contains("output reg accepted"));
    }

    #[test]
    fn fact_egress_provides_axis_style_streaming() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        // Valid/ready handshake + name/payload data bus.
        assert!(verilog.contains("input wire ready_in"));
        assert!(verilog.contains("output reg valid_out"));
        assert!(verilog.contains("output reg [255:0] name"));
        assert!(verilog.contains("output reg [255:0] payload"));
    }

    #[test]
    fn top_instantiates_fact_io_modules() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        assert!(verilog.contains("fact_ingress fact_ingress_inst ("),
            "top must instantiate fact_ingress:\n{}", verilog);
        assert!(verilog.contains("fact_egress fact_egress_inst ("),
            "top must instantiate fact_egress:\n{}", verilog);
        // Integrator-replaceable defaults: ingress valid tied low,
        // egress ready tied high.
        assert!(verilog.contains(".valid_in(1'b0)"));
        assert!(verilog.contains(".ready_in(1'b1)"));
    }

    #[test]
    fn audit_log_emitted_when_state_has_entities() {
        // audit_log rides along with any non-empty compile. Empty state
        // still produces just the header, preserving the "empty compile
        // = empty output" rule.
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        assert!(verilog.contains("module audit_log"),
            "audit_log module must accompany non-empty compile:\n{}", verilog);
    }

    #[test]
    fn audit_log_absent_for_empty_state() {
        let verilog = compile_to_verilog(&Object::phi());
        assert!(!verilog.contains("audit_log"),
            "empty state must emit no modules, got:\n{}", verilog);
    }

    #[test]
    fn audit_log_has_parameterised_depth_and_entry_width() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        assert!(verilog.contains("parameter DEPTH = 1024"));
        assert!(verilog.contains("parameter ENTRY_WIDTH = 256"));
    }

    #[test]
    fn audit_log_shape_matches_cpu_ring_semantics() {
        // The FPGA audit log mirrors crate::ring::RingBuffer:
        // append-only, saturating count, latched overflow flag.
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        assert!(verilog.contains("input wire wr_en"));
        assert!(verilog.contains("input wire [ENTRY_WIDTH-1:0] wr_data"));
        assert!(verilog.contains("output reg [ENTRY_WIDTH-1:0] rd_data"));
        assert!(verilog.contains("output reg overflow"));
        // Overflow latches on first wrap (count saturates).
        assert!(verilog.contains("overflow <= 1'b1"));
        // Head wraps at DEPTH-1.
        assert!(verilog.contains("head == DEPTH-1"));
    }

    #[test]
    fn top_instantiates_audit_log_with_tied_off_write_port() {
        let state = state_with_nouns(&[("Widget", "entity")]);
        let verilog = compile_to_verilog(&state);
        assert!(verilog.contains("audit_log #(.DEPTH(1024), .ENTRY_WIDTH(256)) audit_log_inst"),
            "audit_log must be instantiated with explicit params in top:\n{}", verilog);
        assert!(verilog.contains(".wr_en(1'b0)"),
            "write-enable tied off by default:\n{}", verilog);
        assert!(verilog.contains(".wr_data({256{1'b0}})"),
            "write data tied to zero:\n{}", verilog);
    }

    #[test]
    fn sm_module_emits_case_dispatch_with_status_codes() {
        use crate::types::{StateMachineDef, TransitionDef};
        let mut sms = hashbrown::HashMap::new();
        sms.insert("Order".to_string(), StateMachineDef {
            noun_name: "Order".to_string(),
            statuses: vec!["Draft".to_string(), "Placed".to_string(), "Shipped".to_string()],
            transitions: vec![
                TransitionDef { from: "Draft".to_string(), to: "Placed".to_string(), event: "place".to_string(), guard: None },
                TransitionDef { from: "Placed".to_string(), to: "Shipped".to_string(), event: "ship".to_string(), guard: None },
            ],
            initial: String::new(),
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
        let mut sms = hashbrown::HashMap::new();
        sms.insert("Door".to_string(), StateMachineDef {
            noun_name: "Door".to_string(),
            statuses: vec!["Closed".to_string(), "Open".to_string()],
            transitions: vec![
                TransitionDef { from: "Closed".to_string(), to: "Open".to_string(), event: "open".to_string(), guard: None },
            ],
            initial: String::new(),
        });
        let (_modules, specs) = emit_sm_modules(&sms);
        assert_eq!(specs[0].1, 1, "2 statuses → 1-bit width");
    }

    #[test]
    fn sm_module_empty_sms_map_produces_nothing() {
        let (modules, specs) = emit_sm_modules(&hashbrown::HashMap::new());
        assert!(modules.is_empty());
        assert!(specs.is_empty());
    }

    #[test]
    fn sm_module_without_transitions_holds_initial() {
        use crate::types::StateMachineDef;
        let mut sms = hashbrown::HashMap::new();
        sms.insert("Frozen".to_string(), StateMachineDef {
            noun_name: "Frozen".to_string(),
            statuses: vec!["Only".to_string()],
            transitions: vec![],
            initial: String::new(),
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
        // 4 entity + 4 bram + audit + ingress + egress + wasm_reducer + boot_fsm + top = 14.
        assert_eq!(modules, 14);
        assert_eq!(endmodules, 14);
        assert_eq!(modules, endmodules);
    }
}
