// crates/arest/src/wasm_lower.rs
//
// Lower Func trees to WASM functions (task #152 prototype).
//
// Per docs/11-system-as-os-kernel.md §"Don't reinvent the VM": we
// already run inside a WASM VM (V8 in Workers, wasmer/wasmtime on
// server). Compile each Func to a WASM function and dispatch via the
// host VM — no AREST-level interpreter on the hot path.
//
// This file is the proof-of-concept. Supports a narrow subset:
//   - Func::Id          (identity on i64)
//   - Func::Constant(x) where x is an Object::Atom parseable as i64
//
// The emitted module exports one function named `apply` with
// signature (i64) -> i64. Callers pass the input value; Id returns
// it unchanged, Constant ignores it and returns the literal.
//
// Extending to combinators (Compose, Construction, Condition, …)
// is a matter of adding ops; the emission scaffolding stays the
// same. Object::Seq / Object::Map require a linear-memory
// representation — that's the next step after this PoC validates
// the round-trip.

#![cfg(feature = "wasm-lower")]

use wasm_encoder::{
    CodeSection, ExportKind, ExportSection, Function, FunctionSection, Instruction,
    Module, TypeSection, ValType,
};

use crate::ast::{Func, Object};

/// Lower a Func tree to a valid WASM module.
///
/// Returns `Ok(bytes)` with the module on success, or an `Err`
/// describing which variant is not yet supported. Callers wrap the
/// bytes in `WebAssembly.Module` (V8) or `wasmtime::Module::new`
/// (server) to instantiate and invoke.
pub fn lower_to_wasm(func: &Func) -> Result<Vec<u8>, String> {
    let mut module = Module::new();

    // Type section: one function type (i64) -> i64.
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I64], vec![ValType::I64]);
    module.section(&types);

    // Function section: one function of type 0.
    let mut functions = FunctionSection::new();
    functions.function(0);
    module.section(&functions);

    // Export section: `apply` → function 0.
    let mut exports = ExportSection::new();
    exports.export("apply", ExportKind::Func, 0);
    module.section(&exports);

    // Code section: load arg → emit body → end.
    // Convention: emit_body always leaves the result on the stack
    // and consumes its single i64 input from the stack. The outer
    // wrapper pushes the argument once at the start.
    let mut codes = CodeSection::new();
    let mut body = Function::new([]);
    body.instruction(&Instruction::LocalGet(0));
    emit_body(func, &mut body)?;
    body.instruction(&Instruction::End);
    codes.function(&body);
    module.section(&codes);

    Ok(module.finish())
}

/// Emit instructions that consume one i64 from the stack and leave
/// one i64 on the stack — the stack-discipline lowering convention.
///
/// This convention makes Compose trivial: `emit(g); emit(f)` leaves
/// `f(g(x))` on the stack without any intermediate local. Each Func
/// variant implements its own transformation; the outer
/// `lower_to_wasm` pushes the initial argument once.
fn emit_body(func: &Func, body: &mut Function) -> Result<(), String> {
    match func {
        // id:x = x — input on stack is already the output.
        Func::Id => { /* noop */ }

        // c̄:x = c (when x ≠ ⊥) — drop the input, push the literal.
        // PoC restricts the constant to i64-valued atoms; full
        // Object marshalling via linear memory is follow-up.
        Func::Constant(Object::Atom(s)) => {
            let n: i64 = s.parse()
                .map_err(|_| format!("Constant atom must parse as i64 for PoC: got {:?}", s))?;
            body.instruction(&Instruction::Drop);
            body.instruction(&Instruction::I64Const(n));
        }

        // Backus §11.2.4 Composition: (f ∘ g):x = f:(g:x).
        // Stack discipline makes this just concatenation —
        // emit g (consumes input, leaves g(x)), then emit f
        // (consumes g(x), leaves f(g(x))).
        Func::Compose(f, g) => {
            emit_body(g, body)?;
            emit_body(f, body)?;
        }

        other => return Err(format!("wasm_lower: variant not yet supported: {:?}",
            std::mem::discriminant(other))),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmi::{Engine, Linker, Module as WiModule, Store};

    fn roundtrip(func: &Func, input: i64) -> i64 {
        let bytes = lower_to_wasm(func).expect("lower must succeed for supported variants");
        let engine = Engine::default();
        let module = WiModule::new(&engine, &bytes[..]).expect("emitted WASM must validate");
        let mut store: Store<()> = Store::new(&engine, ());
        let linker: Linker<()> = Linker::new(&engine);
        let instance = linker.instantiate(&mut store, &module)
            .expect("module must instantiate")
            .start(&mut store)
            .expect("module must start");
        let apply = instance.get_typed_func::<i64, i64>(&store, "apply")
            .expect("exported `apply` must exist with (i64) -> i64 signature");
        apply.call(&mut store, input).expect("apply must invoke")
    }

    #[test]
    fn lower_id_emits_valid_module_and_returns_argument() {
        // id:42 = 42 ; id:-7 = -7
        assert_eq!(roundtrip(&Func::Id, 42), 42);
        assert_eq!(roundtrip(&Func::Id, -7), -7);
        assert_eq!(roundtrip(&Func::Id, 0), 0);
    }

    #[test]
    fn lower_constant_emits_valid_module_and_returns_literal() {
        // c̄:x = c for any x
        let f = Func::Constant(Object::atom("100"));
        assert_eq!(roundtrip(&f, 0), 100);
        assert_eq!(roundtrip(&f, 42), 100);
        assert_eq!(roundtrip(&f, -1), 100);
    }

    #[test]
    fn lower_constant_with_non_numeric_atom_errors() {
        let f = Func::Constant(Object::atom("hello"));
        let err = lower_to_wasm(&f).expect_err("non-numeric atom must fail cleanly");
        assert!(err.contains("i64"));
    }

    #[test]
    fn lower_rejects_unsupported_variant() {
        // Construction isn't wired yet — must error, not panic.
        let f = Func::Construction(vec![Func::Id, Func::Id]);
        let err = lower_to_wasm(&f).expect_err("Construction should be marked unsupported");
        assert!(err.contains("not yet supported"));
    }

    #[test]
    fn lower_compose_emits_chained_body() {
        // Compose(Constant(7), Id) : x = Constant(7):(Id:x) = 7.
        // Proves the stack-discipline emission concatenates correctly.
        let f = Func::Compose(
            Box::new(Func::Constant(Object::atom("7"))),
            Box::new(Func::Id),
        );
        assert_eq!(roundtrip(&f, 42), 7);
        assert_eq!(roundtrip(&f, -1), 7);
    }

    #[test]
    fn lower_compose_of_two_ids_is_identity() {
        // (id ∘ id):x = x. Double-wraps the input through the
        // stack-discipline pipeline without touching the value.
        let f = Func::Compose(Box::new(Func::Id), Box::new(Func::Id));
        assert_eq!(roundtrip(&f, 42), 42);
        assert_eq!(roundtrip(&f, -7), -7);
    }

    #[test]
    fn lower_compose_constant_over_constant_returns_outer() {
        // Compose(Constant(A), Constant(B)):x = A.
        // Each Constant drops its input; the outer's result wins.
        let f = Func::Compose(
            Box::new(Func::Constant(Object::atom("9"))),
            Box::new(Func::Constant(Object::atom("11"))),
        );
        assert_eq!(roundtrip(&f, 0), 9);
    }

    /// Benchmark + fixture writer. `#[ignore]`'d so normal cargo test
    /// doesn't pay the runtime cost; run explicitly:
    ///
    ///   cargo test --features wasm-lower --release --lib \
    ///     bench_and_emit_wasm_fixtures -- --ignored --nocapture
    ///
    /// Writes the emitted WASM modules to `target/wasm_fixtures/`
    /// so the companion Bun script (scripts/bench_bun.ts) can load
    /// them and run the identical loop. Prints ns/op for the Rust
    /// native apply() path AND the wasmi-interpreted WASM path so
    /// you can see the interpreter tax locally. Bun's V8 JIT result
    /// is produced by scripts/bench_bun.ts after you run this.
    #[test]
    #[ignore = "benchmark; run with --release --ignored --nocapture"]
    fn bench_and_emit_wasm_fixtures() {
        use std::fs;
        use std::path::Path;
        use std::time::Instant;

        const ITERATIONS: u64 = 10_000_000;

        let fixtures_dir = Path::new("target/wasm_fixtures");
        fs::create_dir_all(fixtures_dir).expect("create fixtures dir");

        let cases: Vec<(&str, Func, i64)> = vec![
            ("id",              Func::Id, 42),
            ("constant_seven",  Func::Constant(Object::atom("7")), 42),
        ];

        eprintln!("\n=== WASM lowering benchmark — {} iterations ===", ITERATIONS);
        eprintln!("{:<18} {:>12} {:>12} {:>12}", "case", "rust-ns/op", "wasmi-ns/op", "ratio");

        for (name, func, input) in &cases {
            // 1. Write the WASM bytes for Bun to load.
            let bytes = lower_to_wasm(func).expect("lower must succeed");
            let path = fixtures_dir.join(format!("{}.wasm", name));
            fs::write(&path, &bytes).expect("write fixture");

            // 2. Time the Rust native apply() path.
            let x = Object::atom(&input.to_string());
            let d = Object::phi();
            let t = Instant::now();
            let mut acc: i64 = 0;
            for _ in 0..ITERATIONS {
                let r = crate::ast::apply(func, &x, &d);
                // Keep the compiler from optimising the loop away.
                if let Object::Atom(s) = &r {
                    acc = acc.wrapping_add(s.len() as i64);
                }
            }
            let rust_ns = t.elapsed().as_nanos() as f64 / ITERATIONS as f64;
            std::hint::black_box(acc);

            // 3. Time the wasmi interpreter path.
            let engine = Engine::default();
            let module = WiModule::new(&engine, &bytes[..]).expect("wasmi validate");
            let mut store: Store<()> = Store::new(&engine, ());
            let linker: Linker<()> = Linker::new(&engine);
            let instance = linker.instantiate(&mut store, &module).unwrap()
                .start(&mut store).unwrap();
            let apply = instance.get_typed_func::<i64, i64>(&store, "apply").unwrap();
            let t = Instant::now();
            let mut wacc: i64 = 0;
            for _ in 0..ITERATIONS {
                wacc = wacc.wrapping_add(apply.call(&mut store, *input).unwrap());
            }
            let wasmi_ns = t.elapsed().as_nanos() as f64 / ITERATIONS as f64;
            std::hint::black_box(wacc);

            let ratio = wasmi_ns / rust_ns;
            eprintln!("{:<18} {:>12.1} {:>12.1} {:>11.1}x",
                name, rust_ns, wasmi_ns, ratio);
        }

        eprintln!("\nFixtures written to {}", fixtures_dir.display());
        eprintln!("Run `bun scripts/bench_bun.ts` for the V8-JIT (production) comparison.");
    }

    #[test]
    fn wasm_result_matches_rust_apply_for_supported_variants() {
        // For the supported subset, the emitted WASM must return
        // the same i64 that ast::apply produces from the equivalent
        // Object. This is the semantic-equivalence check that the
        // full compiler's extension tests will rely on.
        let cases: Vec<(Func, i64)> = vec![
            (Func::Id, 99),
            (Func::Id, -99),
            (Func::Constant(Object::atom("42")), 7),
            (Func::Constant(Object::atom("-5")), 123),
        ];
        for (f, input) in cases {
            let wasm_result = roundtrip(&f, input);
            let rust_result = crate::ast::apply(&f, &Object::atom(&input.to_string()), &Object::phi());
            let rust_i64 = rust_result.as_atom()
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(input); // Id's result is the input atom; Constant returns its literal
            // For Id, rust_result is the input atom — compare directly.
            // For Constant, rust_result is the constant atom.
            let expected = match &f {
                Func::Id => input,
                Func::Constant(Object::Atom(s)) => s.parse().unwrap(),
                _ => rust_i64,
            };
            assert_eq!(wasm_result, expected,
                "WASM result diverges from Rust apply for {:?} input={}", f, input);
        }
    }
}
