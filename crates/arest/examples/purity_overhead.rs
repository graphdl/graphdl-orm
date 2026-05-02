// Purity-overhead sandbox bench.
//
// Measures the cost of staying close to pure CONS / Func combinators
// vs. taking the current HashMap-backed shortcuts. Two legs:
//
//   1. **Microbench** — same K-cell state in three backings; N lookups by
//      name; report ns/op.
//        A. Object::Seq           — pure CONS scan (fetch loops the list).
//        B. Object::Map           — HashMap-backed cell store.
//        C. Eager typed index     — Map state + cell_index_from_state
//           precomputed once, then HashMap::get on that.
//
//   2. **End-to-end** — load the bundled metamodel corpus, parse once,
//      then time `compile_to_defs_state` over the parsed state in two
//      shapes:
//        A. as-parsed (Map-backed; current production path).
//        B. coerced to Object::Seq before compile so every internal
//           fetch_or_phi inside the compiler hits the CONS slow path.
//
// The microbench isolates the per-primitive overhead exactly. The
// end-to-end shows whether that overhead actually matters once it's
// amortised across a real compile cycle.
//
// Run with `cargo run --release --example purity_overhead`.

use std::time::Instant;

use arest::ast::{self, Object};
use arest::compile::{compile_to_defs_state, cell_index_from_state};
use arest::parse_forml2::parse_to_state;

const MICROBENCH_K: &[usize] = &[16, 64, 256, 1024];
const MICROBENCH_LOOKUPS: usize = 50_000;
const MICROBENCH_WARMUP: usize = 5_000;
const E2E_ITERS: usize = 20;
const E2E_WARMUP: usize = 3;

fn main() {
    println!("== Purity-overhead bench ==");
    println!();
    microbench();
    println!();
    end_to_end();
}

// ── Microbench ──────────────────────────────────────────────────────

fn microbench() {
    println!("-- Microbench: cell-fetch by name, {} lookups --", MICROBENCH_LOOKUPS);
    println!();
    println!("  K  |     CONS (Seq)    |   HashMap (Map)   |   eager typed     | CONS/Map | Eager/Map");
    println!("-----+-------------------+-------------------+-------------------+----------+----------");

    for &k in MICROBENCH_K {
        let names: Vec<String> = (0..k).map(|i| format!("cell_{:04}", i)).collect();
        let map_state = build_map_state(&names);
        let seq_state = force_seq_backed(&map_state);
        let typed_index = cell_index_from_state(&map_state);
        // Microbench probes simple cells, not the typed index's named
        // collections. To make the eager-typed leg comparable we use its
        // `nouns` HashMap, which is the canonical "I already paid the
        // index cost — now lookups are HashMap::get" shape.
        let typed_nouns = &typed_index.nouns;

        // Build a typed-fetchable state for the eager leg: pretend each
        // microbench cell is a Noun named cell_NNNN by registering them
        // through the same name namespace the typed index keys on.
        let typed_fallback_map = build_named_map(&names);
        let typed_fallback_index = cell_index_from_state(&typed_fallback_map);

        // Choose a deterministic sequence of lookup names (round-robin
        // through the K cell names so every lookup hits).
        let lookup_seq: Vec<&str> = (0..MICROBENCH_LOOKUPS)
            .map(|i| names[i % k].as_str())
            .collect();

        // Warmup
        for n in lookup_seq.iter().take(MICROBENCH_WARMUP) {
            let _ = ast::fetch(n, &seq_state);
            let _ = ast::fetch(n, &map_state);
            let _ = typed_nouns.get(*n);
            let _ = typed_fallback_index.nouns.get(*n);
        }

        let cons_ns = time_ns(|| {
            let mut acc = 0usize;
            for n in &lookup_seq {
                let r = ast::fetch(n, &seq_state);
                if !matches!(r, Object::Bottom) { acc += 1; }
            }
            std::hint::black_box(acc);
        });
        let map_ns = time_ns(|| {
            let mut acc = 0usize;
            for n in &lookup_seq {
                let r = ast::fetch(n, &map_state);
                if !matches!(r, Object::Bottom) { acc += 1; }
            }
            std::hint::black_box(acc);
        });
        let eager_ns = time_ns(|| {
            let mut acc = 0usize;
            for n in &lookup_seq {
                if typed_fallback_index.nouns.contains_key(*n) { acc += 1; }
            }
            std::hint::black_box(acc);
        });

        let cons_per = cons_ns as f64 / MICROBENCH_LOOKUPS as f64;
        let map_per = map_ns as f64 / MICROBENCH_LOOKUPS as f64;
        let eager_per = eager_ns as f64 / MICROBENCH_LOOKUPS as f64;
        println!(
            "{:>4} | {:>8.1} ns/op    | {:>8.1} ns/op    | {:>8.1} ns/op    | {:>5.1}x   | {:>5.2}x",
            k, cons_per, map_per, eager_per,
            cons_per / map_per.max(1e-9),
            eager_per / map_per.max(1e-9),
        );
    }
}

fn build_map_state(names: &[String]) -> Object {
    // Build via cell_push from Object::phi() (which yields a Seq-backed
    // store) then coerce to Map via merge_states (which returns Map).
    // Direct Object::Map construction would need hashbrown::HashMap, an
    // internal type the example crate doesn't see — going through the
    // public API keeps the example dep-free.
    let mut state = Object::phi();
    for n in names {
        state = ast::cell_push(n, Object::seq(vec![Object::atom(n)]), &state);
    }
    ast::merge_states(&Object::phi(), &state)
}

fn build_named_map(names: &[String]) -> Object {
    // Build a Noun cell containing one fact per name so the typed index
    // populates its `nouns` HashMap with K entries, mirroring the
    // microbench cell count for an apples-to-apples eager comparison.
    let mut state = Object::phi();
    for n in names {
        let fact = Object::seq(vec![
            Object::seq(vec![Object::atom("name"), Object::atom(n)]),
            Object::seq(vec![Object::atom("objectType"), Object::atom("entity")]),
        ]);
        state = ast::cell_push("Noun", fact, &state);
    }
    state
}

fn force_seq_backed(map_state: &Object) -> Object {
    let mut out = Object::phi();
    for (name, contents) in ast::cells_iter(map_state) {
        out = ast::store(name, contents.clone(), &out);
    }
    out
}

// ── End-to-end ──────────────────────────────────────────────────────

fn end_to_end() {
    println!("-- End-to-end: parse + compile, {} iterations --", E2E_ITERS);
    let corpus = arest::metamodel_corpus();
    println!("  metamodel corpus: {} bytes", corpus.len());

    let parsed_map = match parse_to_state(&corpus) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("parse_to_state failed: {e}");
            return;
        }
    };
    let cells_in_map = ast::cells_iter(&parsed_map).len();
    let parsed_seq = force_seq_backed(&parsed_map);
    let cells_in_seq = ast::cells_iter(&parsed_seq).len();
    println!(
        "  parsed state: {} cells (Map), {} cells (coerced Seq)",
        cells_in_map, cells_in_seq,
    );
    println!();

    // Warmup
    for _ in 0..E2E_WARMUP {
        let _ = compile_to_defs_state(&parsed_map);
        let _ = compile_to_defs_state(&parsed_seq);
    }

    let map_total = time_ns(|| {
        for _ in 0..E2E_ITERS {
            let defs = compile_to_defs_state(&parsed_map);
            std::hint::black_box(defs);
        }
    });
    let seq_total = time_ns(|| {
        for _ in 0..E2E_ITERS {
            let defs = compile_to_defs_state(&parsed_seq);
            std::hint::black_box(defs);
        }
    });

    let map_per = map_total as f64 / E2E_ITERS as f64 / 1e6;
    let seq_per = seq_total as f64 / E2E_ITERS as f64 / 1e6;
    println!(
        "  Map-backed compile: {:>7.2} ms/iter ({} iters, {:.2} ms total)",
        map_per, E2E_ITERS, map_total as f64 / 1e6,
    );
    println!(
        "  Seq-backed compile: {:>7.2} ms/iter ({} iters, {:.2} ms total)",
        seq_per, E2E_ITERS, seq_total as f64 / 1e6,
    );
    println!(
        "  Seq/Map ratio:      {:>7.2}x  (cost of pure CONS path inside the compiler)",
        seq_per / map_per.max(1e-9),
    );
}

// ── Helpers ─────────────────────────────────────────────────────────

fn time_ns<F: FnOnce()>(f: F) -> u128 {
    let t = Instant::now();
    f();
    t.elapsed().as_nanos()
}
